import Foundation
import Testing
@testable import Moshpit

/// Schema drift against every list store. Each one used to decode its array
/// in one piece and take a failure as an empty list — then save that empty
/// list over the user's entries at the next change (the shortcut store
/// didn't even wait for a change: finding "nothing", it seeded the built-ins
/// and wrote them on the spot). ConnectionStoreTests covers the connection
/// list; this suite covers the reader itself and the other four stores.
@Suite("stored lists — entries a build can't read")
struct StoredListTests {

    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.storedlist.\(UUID().uuidString)"
        return (UserDefaults(suiteName: name)!, name)
    }

    private static func entries(_ defaults: UserDefaults, _ key: String) throws -> [Any] {
        let data = try #require(defaults.data(forKey: key))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [Any])
    }

    private struct Item: Codable, Equatable {
        var name: String
        var count: Int
    }

    // MARK: - The reader

    @Test("each entry is decoded on its own; the rest are carried as stored")
    func readsEntryByEntry() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")
        defaults.set(Data(#"[{"name":"a","count":1},{"name":"b","count":"two"},7,{"name":"c","count":3}]"#.utf8),
                     forKey: "items")

        let contents = list.read(from: defaults)

        #expect(contents.elements == [Item(name: "a", count: 1), Item(name: "c", count: 3)])
        #expect(contents.carried.count == 2)
        #expect(contents.isList)
        #expect(!contents.isComplete)

        // Written back: what was read, then what wasn't, as it was.
        let data = try #require(list.encode(contents.elements, carrying: contents.carried))
        let written = try #require(try JSONSerialization.jsonObject(with: data) as? [Any])
        #expect(written.count == 4)
        #expect((written[2] as? [String: Any])?["count"] as? String == "two")
        #expect(written[3] as? Int == 7)
    }

    @Test("nothing stored is a complete, empty read that needs no backup")
    func nothingStoredIsComplete() {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")

        let contents = list.read(from: defaults)

        #expect(contents.elements.isEmpty && contents.isComplete && contents.stored == nil)
        #expect(list.secure(contents, in: defaults))
        #expect(defaults.object(forKey: list.backupKey) == nil)
    }

    @Test("a value that isn't a list is kept in the backup, once per distinct value")
    func nonListIsBackedUp() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")
        defaults.set(Data(#"{"version":2,"items":[]}"#.utf8), forKey: "items")

        let contents = list.read(from: defaults)
        #expect(!contents.isList && contents.elements.isEmpty)
        #expect(list.secure(contents, in: defaults))
        #expect(list.secure(list.read(from: defaults), in: defaults))
        #expect(defaults.array(forKey: list.backupKey)?.count == 1)

        // A plist value that isn't even Data is kept the same way.
        defaults.set(["a", "b"], forKey: "items")
        #expect(list.secure(list.read(from: defaults), in: defaults))
        #expect(defaults.array(forKey: list.backupKey)?.count == 2)
    }

    @Test("a list with skips is backed up once; later skips ride along in the list itself")
    func skippedListBackedUpOnce() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")
        let first = Data(#"[{"name":"a","count":1},{"name":"b"}]"#.utf8)
        defaults.set(first, forKey: "items")
        #expect(list.secure(list.read(from: defaults), in: defaults))

        defaults.set(Data(#"[{"name":"a","count":1},{"name":"b"},{"name":"c"}]"#.utf8), forKey: "items")
        #expect(list.secure(list.read(from: defaults), in: defaults))

        let backups = try #require(defaults.array(forKey: list.backupKey))
        #expect(backups.count == 1)
        #expect(backups.first as? Data == first)
    }

    /// B154: JSONSerialization reads -1e400 as -infinity, and writing the
    /// entry back out — the reader's next step — raised an Objective-C
    /// exception: the test process died here, as the app did on every
    /// launch while such a value sat in a list (even in a field no build
    /// reads). Out of range for either sign, as moshpit-store has it: the
    /// value is no list, copied to the backup, nothing written over it.
    @Test("a number out of Double's range makes the value no list, and nothing crashes")
    func negativeOverflowIsNoList() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")
        for number in ["-1e400", "-1.8e308", "1e400"] {
            let stored = Data(#"[{"name":"a","count":1},{"name":"b","count":2,"unread":\#(number)}]"#.utf8)
            defaults.set(stored, forKey: "items")

            let contents = list.read(from: defaults)

            #expect(!contents.isList, "\(number)")
            #expect(contents.elements.isEmpty && contents.carried.isEmpty, "\(number)")
            #expect(list.secure(contents, in: defaults), "\(number)")
            #expect(defaults.array(forKey: list.backupKey)?.contains { $0 as? Data == stored } == true,
                    "\(number)")
        }
        // Small numbers stay what they were: one that rounds to zero reads.
        defaults.set(Data(#"[{"name":"a","count":1,"tiny":1e-400}]"#.utf8), forKey: "items")
        #expect(list.read(from: defaults).elements == [Item(name: "a", count: 1)])
    }

    @Test("a connection holding -1e400 loads as no connections instead of crashing the launch")
    func connectionStoreSurvivesNegativeOverflow() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let stored = #"[{"id":"0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1","name":"work","host":"box.example.com","port":22,"username":"dev","authMethod":"password","connectionProtocol":"ssh","sshPort":22,"moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-01T09:00:00Z","useTmux":true,"keepAliveInterval":-1e400}]"#
        defaults.set(Data(stored.utf8), forKey: "moshpit.connections")

        let store = ConnectionStore(defaults: defaults)

        #expect(store.connections.isEmpty)
        #expect(defaults.data(forKey: "moshpit.connections") == Data(stored.utf8), "the value was written over")
        #expect(defaults.array(forKey: "moshpit.connections.backup")?.first as? Data == Data(stored.utf8))
    }

    /// B155: JSONSerialization drops a leading U+FEFF from every string it
    /// parses, keys included, raw or escaped — and the reader parsed every
    /// entry through it, as the save that carries entries did.
    @Test("a leading U+FEFF survives the read and a save that carries entries")
    func byteOrderMarkSurvives() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let list = StoredList<Item>(key: "items")
        let stored = "[{\"name\":\"\u{FEFF}a\",\"count\":1},{\"name\":\"\\uFEFFb\",\"count\":2},"
            + "{\"\u{FEFF}name\":\"\u{FEFF}\u{FEFF}c\"}]"
        defaults.set(Data(stored.utf8), forKey: "items")

        let contents = list.read(from: defaults)

        #expect(contents.elements == [Item(name: "\u{FEFF}a", count: 1), Item(name: "\u{FEFF}b", count: 2)])
        let carried = try #require(contents.carried.first as? [String: Any])
        #expect(carried["\u{FEFF}name"] as? String == "\u{FEFF}\u{FEFF}c")
        let data = try #require(list.encode(contents.elements, carrying: contents.carried))
        defaults.set(data, forKey: "items")
        let back = list.read(from: defaults)
        #expect(back.elements == contents.elements)
        #expect((back.carried.first as? [String: Any])?["\u{FEFF}name"] as? String == "\u{FEFF}\u{FEFF}c")
        // Nothing else changes: no U+FEFF at all reads as it always did.
        defaults.set(Data(#"[{"name":"plain\u00e9","count":3}]"#.utf8), forKey: "items")
        #expect(list.read(from: defaults).elements == [Item(name: "plainé", count: 3)])
    }

    @Test("a connection whose host starts with U+FEFF keeps it through a launch and a save")
    func connectionKeepsByteOrderMark() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let stored = #"[{"id":"0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1","name":"\#u{FEFF}work","host":"\#u{FEFF}box.example.com","port":22,"username":"dev","authMethod":"password","connectionProtocol":"ssh","sshPort":22,"moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-01T09:00:00Z","useTmux":true}]"#
        defaults.set(Data(stored.utf8), forKey: "moshpit.connections")

        let store = ConnectionStore(defaults: defaults)
        let connection = try #require(store.connections.first)
        #expect(connection.host == "\u{FEFF}box.example.com" && connection.name == "\u{FEFF}work")
        store.update(connection)

        // The bytes, not JSONSerialization's reading of them: it drops the mark.
        let bytes = try #require(defaults.data(forKey: "moshpit.connections"))
        #expect(bytes.range(of: Data("\"\u{FEFF}box.example.com\"".utf8)) != nil)
        #expect(ConnectionStore(defaults: defaults).connections.first?.host == "\u{FEFF}box.example.com")
    }

    // MARK: - SSH keys

    @Test("an SSH key record this build can't decode is kept, not dropped with the rest")
    func sshKeyRecordSurvives() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.sshkeys.v1"
        let (scratch, scratchSuite) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchSuite) }
        SSHKeyStore(defaults: scratch).add(SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .imported))
        var stored = try Self.entries(scratch, key)
        // A newer build's algorithm: this build's enum has no such case.
        var newer = try #require(stored.first as? [String: Any])
        newer["name"] = "yubikey"
        newer["id"] = UUID().uuidString
        newer["algorithm"] = "ED448-sk"
        stored.append(newer)
        defaults.set(try JSONSerialization.data(withJSONObject: stored), forKey: key)

        let store = SSHKeyStore(defaults: defaults)
        #expect(store.keys.map(\.name) == ["laptop"], "the old load read both as nothing")
        store.add(SSHKeyRecord(name: "phone", algorithm: .seP256, source: .secureEnclave))

        let names = try Self.entries(defaults, key).compactMap { ($0 as? [String: Any])?["name"] as? String }
        #expect(names.sorted() == ["laptop", "phone", "yubikey"],
                "the old store saved [phone] over both keys")
        #expect(SSHKeyStore(defaults: defaults).keys.map(\.name) == ["laptop", "phone"])
    }

    // MARK: - Shortcuts

    /// A set as a newer build saved it: the user's own chip next to one of a
    /// kind this build doesn't know.
    private static func shortcutSetWithUnknownKind() throws -> Data {
        let (scratch, suite) = makeDefaults()
        defer { scratch.removePersistentDomain(forName: suite) }
        var deploy = TerminalShortcut()
        deploy.kind = .command
        deploy.payload = "make deploy"
        deploy.chipLabel = "deploy"
        deploy.summary = "Deploy"
        ShortcutStore(defaults: scratch).add(deploy)
        var stored = try entries(scratch, "moshpit.shortcuts.v1")
        var macro = try #require(stored.last as? [String: Any])
        macro["id"] = UUID().uuidString
        macro["kind"] = "macro"
        macro["chipLabel"] = "mac"
        macro["summary"] = "A newer build's macro"
        stored.append(macro)
        return try JSONSerialization.data(withJSONObject: stored)
    }

    @Test("a chip kind this build doesn't know no longer reseeds the bar over custom chips")
    func unknownChipKindKeepsCustomChips() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.shortcuts.v1"
        let original = try Self.shortcutSetWithUnknownKind()
        defaults.set(original, forKey: key)

        let store = ShortcutStore(defaults: defaults)

        #expect(store.custom.map(\.summary) == ["Deploy"],
                "the old load seeded the built-ins and wrote them over this set at launch")
        #expect(!store.toolbar.isEmpty)
        let kinds = try Self.entries(defaults, key).compactMap { ($0 as? [String: Any])?["kind"] as? String }
        #expect(kinds.contains("macro"))

        // A change saves the unknown chip back, after the rest.
        store.removeFromToolbar(id: try #require(store.custom.first).id)
        let after = try Self.entries(defaults, key)
        #expect((after.last as? [String: Any])?["kind"] as? String == "macro")
        #expect(ShortcutStore(defaults: defaults).custom.map(\.summary) == ["Deploy"])
    }

    @Test("a set this build can't read at all shows the built-ins and writes nothing until a change")
    func whollyUnreadableSetIsNotReseeded() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.shortcuts.v1"
        let unreadable = Data(#"[{"kind":"macro","summary":"x"},{"kind":"macro","summary":"y"}]"#.utf8)
        defaults.set(unreadable, forKey: key)

        let store = ShortcutStore(defaults: defaults)

        #expect(!store.toolbar.isEmpty, "the bar still works")
        #expect(defaults.data(forKey: key) == unreadable, "nothing written over it at launch")
        // The first real change saves the built-ins AND the two it can't read.
        store.removeFromToolbar(id: try #require(store.toolbar.first).id)
        let summaries = try Self.entries(defaults, key).compactMap { ($0 as? [String: Any])?["summary"] as? String }
        #expect(summaries.contains("x") && summaries.contains("y"))
    }

    @Test("a built-in stored in a shape this build can't read isn't injected a second time")
    func unreadableBuiltinIsNotDuplicated() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.shortcuts.v1"
        let (scratch, scratchSuite) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchSuite) }
        var deploy = TerminalShortcut()
        deploy.kind = .command
        deploy.payload = "make deploy"
        deploy.chipLabel = "deploy"
        deploy.summary = "Deploy"
        ShortcutStore(defaults: scratch).add(deploy)   // the shipped set, plus one of the user's
        var stored = try Self.entries(scratch, key)
        // The paste chip, re-saved by a newer build with a field this build
        // can't decode (a modifier it has no case for).
        let pasteIndex = try #require(stored.firstIndex { ($0 as? [String: Any])?["kind"] as? String == "paste" })
        var paste = try #require(stored[pasteIndex] as? [String: Any])
        paste["modifiers"] = ["hyper"]
        stored[pasteIndex] = paste
        defaults.set(try JSONSerialization.data(withJSONObject: stored), forKey: key)

        let store = ShortcutStore(defaults: defaults)
        store.removeFromToolbar(id: try #require(store.toolbar.first).id)

        #expect(store.custom.map(\.summary) == ["Deploy"], "the old load wiped the set for the built-ins")
        let pastes = try Self.entries(defaults, key).filter { ($0 as? [String: Any])?["kind"] as? String == "paste" }
        #expect(pastes.count == 1, "the migration would otherwise inject a second paste chip")
    }

    // MARK: - Themes

    @Test("a custom terminal theme this build can't decode is kept")
    func terminalThemeSurvives() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.settings.customThemes"
        let (scratch, scratchSuite) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchSuite) }
        let writer = ThemeStore(defaults: scratch)
        writer.save(writer.duplicate(.fallback))
        var stored = try Self.entries(scratch, key)
        stored.append("a theme from a build that stores them as strings")
        defaults.set(try JSONSerialization.data(withJSONObject: stored), forKey: key)

        let store = ThemeStore(defaults: defaults)
        #expect(store.customThemes.count == 1, "the old load read both as nothing")
        store.save(store.makeDraft())

        let after = try Self.entries(defaults, key)
        #expect(after.count == 3, "the old store saved the draft alone")
        #expect(after.contains { $0 as? String == "a theme from a build that stores them as strings" })
        #expect(ThemeStore(defaults: defaults).customThemes.count == 2)
    }

    @Test("B160: a theme stored without an id keeps the one it is given, written once the read is trusted")
    func terminalThemeWithoutAnIDKeepsIt() throws {
        let key = "moshpit.settings.customThemes"
        let stored = ##"[{"name":"Paper","background":"#FFFFFF","foreground":"#111111"},{"id":"custom-1","name":"Ink","background":"#000000"}]"##

        // Before the first unlock: shown, never written back.
        let (sealed, sealedSuite) = Self.makeDefaults()
        defer { sealed.removePersistentDomain(forName: sealedSuite) }
        sealed.set(Data(stored.utf8), forKey: key)
        let early = ThemeStore(defaults: sealed, readiness: DefaultsReadiness { false })
        #expect(early.customThemes.count == 2)
        #expect(sealed.data(forKey: key) == Data(stored.utf8), "an untrusted read was written back")

        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(Data(stored.utf8), forKey: key)
        let first = ThemeStore(defaults: defaults)
        let paper = try #require(first.customThemes.first).id
        #expect(paper.hasPrefix("custom-"))
        #expect(first.customThemes.last?.id == "custom-1")
        for _ in 0..<3 {
            let again = ThemeStore(defaults: defaults)
            #expect(again.customThemes.first?.id == paper, "a new id at the next launch")
            #expect(again.theme(id: paper).name == "Paper")
        }
    }

    @Test("a custom app theme this build can't decode is kept")
    func appThemeSurvives() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "moshpit.settings.customAppThemes"
        let stored = #"[{"id":"app-1","name":"Mine","accent":"3366FF"},{"id":"app-2","name":"Gradient","stops":["FF0000","0000FF"]}]"#
        defaults.set(Data(stored.utf8), forKey: key)

        let store = AppThemeStore(defaults: defaults)
        #expect(store.customThemes.map(\.id) == ["app-1"], "the old load read both as nothing")
        store.save(AppTheme.custom(id: "app-3", name: "Other", accentHex: "FF3366"))

        let ids = try Self.entries(defaults, key).compactMap { ($0 as? [String: Any])?["id"] as? String }
        #expect(ids.sorted() == ["app-1", "app-2", "app-3"], "the old store saved [app-1, app-3]")
    }
}
