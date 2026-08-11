import Foundation
import Testing
@testable import Moshpit

@Suite("TerminalShortcut encoding")
struct TerminalShortcutEncodingTests {

    private func combo(_ mods: Set<ShortcutModifier>, _ key: String) -> TerminalShortcut {
        var sc = TerminalShortcut()
        sc.kind = .keyCombo
        sc.modifiers = mods
        sc.key = key
        return sc
    }

    @Test("named keys encode to their control bytes")
    func namedKeys() throws {
        #expect(combo([], "esc").encodedBytes() == Data([0x1B]))
        #expect(combo([], "tab").encodedBytes() == Data([0x09]))
        #expect(combo([], "return").encodedBytes() == Data([0x0D]))
        #expect(combo([], "space").encodedBytes() == Data([0x20]))
    }

    @Test("arrow keys encode to CSI sequences")
    func arrows() throws {
        #expect(combo([], "up").encodedBytes() == Data([0x1B, 0x5B, 0x41]))
        #expect(combo([], "down").encodedBytes() == Data([0x1B, 0x5B, 0x42]))
        #expect(combo([], "right").encodedBytes() == Data([0x1B, 0x5B, 0x43]))
        #expect(combo([], "left").encodedBytes() == Data([0x1B, 0x5B, 0x44]))
    }

    @Test("ctrl+letter maps to the C0 control byte")
    func ctrlLetters() throws {
        #expect(combo([.ctrl], "c").encodedBytes() == Data([0x03]))  // ^C SIGINT
        #expect(combo([.ctrl], "d").encodedBytes() == Data([0x04]))  // ^D EOF
        #expect(combo([.ctrl], "r").encodedBytes() == Data([0x12]))  // ^R rev-search
        #expect(combo([.ctrl], "l").encodedBytes() == Data([0x0C]))  // ^L clear
        #expect(combo([.ctrl], "a").encodedBytes() == Data([0x01]))
    }

    @Test("alt prefixes the byte with ESC (meta)")
    func altMeta() throws {
        #expect(combo([.alt], "b").encodedBytes() == Data([0x1B, 0x62]))
    }

    @Test("text shortcut sends literal bytes; appendReturn adds CR")
    func textKind() throws {
        var sc = TerminalShortcut()
        sc.kind = .text
        sc.payload = "git status"
        #expect(sc.encodedBytes() == Data("git status".utf8))
        sc.appendReturn = true
        #expect(sc.encodedBytes() == Data("git status\r".utf8))
    }

    @Test("command shortcut always terminates with CR")
    func commandKind() throws {
        var sc = TerminalShortcut()
        sc.kind = .command
        sc.payload = "claude --resume"
        #expect(sc.encodedBytes() == Data("claude --resume\r".utf8))
    }

    @Test("escape sequences in payloads are unescaped")
    func escapes() throws {
        var sc = TerminalShortcut()
        sc.kind = .text
        sc.payload = #"a\tb\r\e"#
        #expect(sc.encodedBytes() == Data([0x61, 0x09, 0x62, 0x0D, 0x1B]))
    }

    @Test("ctrl (like dpad/scroll) has no bytes of its own — resolved by the caller")
    func ctrlKindEncodesNothing() throws {
        var sc = TerminalShortcut()
        sc.kind = .ctrl
        #expect(sc.encodedBytes() == nil)
    }
}

@Suite("ShortcutStore")
struct ShortcutStoreTests {

    /// A persisted set as it looked before the mic became a chip: today's
    /// builtins minus the mic, with ^L put back in the bar where it was.
    ///
    /// Reconstructed rather than taken from `builtins` as-is — ^L now ships
    /// out of the bar, so a plain filter would seed a toolbar that already
    /// lacks it and every assertion about the migration would pass without
    /// the migration doing anything.
    private static func preMicDefaults() -> [TerminalShortcut] {
        ShortcutStore.builtins
            .filter { $0.kind != .mic }
            .map { shortcut in
                var restored = shortcut
                if restored.summary == "Clear screen" { restored.inToolbar = true }
                return restored
            }
    }

    private func freshStore() -> ShortcutStore {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        return ShortcutStore(defaults: defaults)
    }

    @Test("seeds a lean default toolbar: esc, tab, interrupt, paste, arrows, mic")
    func defaultToolbar() throws {
        let store = freshStore()
        let labels = store.toolbar.map(\.chipLabel)
        let allBuiltin = store.available.allSatisfy(\.isBuiltin)
        // The handful of keys actually reached for constantly. Ctrl and the
        // scroll thumb start outside the bar (available to add back), along
        // with everything else that used to be a default (^D, ^R, ⌃End, ⇧Tab).
        // The mic trails the row: it used to be pinned outside the scroll at
        // the trailing edge, so this is where it already appeared.
        #expect(store.toolbarCount == 6)
        #expect(labels == ["esc", "tab", "^C", "paste", "✛", "mic"])
        #expect(store.toolbar.last?.kind == .mic)
        #expect(store.available.contains { $0.kind == .ctrl })
        #expect(store.available.contains { $0.kind == .scroll })
        #expect(allBuiltin)
    }

    @Test("the default bar stays within a phone-width row")
    func defaultToolbarFitsOnScreen() throws {
        // Measured on a 402pt iPhone: chips are 46pt (the D-pad 42) with 6pt
        // gaps and an 8pt leading inset, and the pinned keyboard toggle takes
        // the last 50pt. Seven chips overflowed by ~10pt, which is what a
        // half-cut chip at the right edge looks like. Recomputed here so a
        // future default can't silently reintroduce it.
        let store = freshStore()
        let width = store.toolbar.reduce(CGFloat(8)) { total, shortcut in
            total + (shortcut.kind == .dpad ? 42 : 46) + 6
        }
        #expect(width <= 402 - 50, "default toolbar overflows: \(width)pt")
    }

    @Test("migration injects ctrl + the D-pad for a persisted set that predates them")
    func migratesDpad() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        // Seed an old persisted set with one builtin and NO D-pad/ctrl.
        var esc = TerminalShortcut()
        esc.kind = .keyCombo; esc.key = "esc"; esc.chipLabel = "esc"
        esc.isBuiltin = true; esc.inToolbar = true
        let data = try JSONEncoder().encode([esc])
        defaults.set(data, forKey: "moshpit.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        // dpad joins the toolbar (its own current default); ctrl is injected
        // too but — like its own current default — starts outside the bar.
        #expect(store.toolbar.first?.kind == .dpad)
        #expect(store.shortcuts.contains { $0.kind == .ctrl })
        #expect(store.available.contains { $0.kind == .ctrl })
        #expect(store.toolbar.contains { $0.chipLabel == "esc" })
    }

    @Test("migration moves the mic into the bar for a set that predates the chip")
    func migratesMic() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        defaults.set(try JSONEncoder().encode(Self.preMicDefaults()),
                     forKey: "moshpit.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        // It has to arrive IN the bar: the key was always visible before, and
        // an upgrade that quietly hides it reads as the feature being removed.
        #expect(store.toolbar.contains { $0.kind == .mic })
        // At the trailing edge, which is where the pinned key already sat — so
        // no existing chip shifts under the user's thumb.
        #expect(store.toolbar.last?.kind == .mic)
        // And it makes room for itself rather than overflowing the row.
        #expect(!store.toolbar.contains { $0.chipLabel == "^L" })
        #expect(store.available.contains { $0.chipLabel == "^L" })
    }

    @Test("a bar that already grew a mic still gets its slot freed")
    func clearScreenLeavesEvenWhenMicAlreadyPresent() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        // The intermediate state: mic already injected, ^L still in the bar,
        // seven chips wide. Reachable by anyone who ran a build between the
        // two changes, and the reason the ^L step can't hang off the mic
        // injection — that branch never runs again once a mic exists.
        var previous = Self.preMicDefaults()
        var mic = try #require(ShortcutStore.builtins.first { $0.kind == .mic })
        mic.inToolbar = true
        previous.append(mic)
        defaults.set(try JSONEncoder().encode(previous), forKey: "moshpit.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        #expect(store.toolbarCount == 6)
        #expect(!store.toolbar.contains { $0.chipLabel == "^L" })
        #expect(store.toolbar.last?.kind == .mic)
    }

    @Test("a customized toolbar keeps every key the user put in it")
    func micMigrationSparesCustomizedBars() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        // Same era as the migration above, but the user rearranged it: ^L
        // moved to the front and a custom chip added. Freeing a slot by
        // deleting a shortcut someone deliberately placed is worse than a row
        // they have to drag, so the ^L removal must not fire here.
        var previous = Self.preMicDefaults()
        if let clear = previous.firstIndex(where: { $0.summary == "Clear screen" }) {
            previous.insert(previous.remove(at: clear), at: 0)
        }
        var custom = TerminalShortcut()
        custom.kind = .command; custom.payload = "claude"; custom.chipLabel = "clm"
        custom.isBuiltin = false; custom.inToolbar = true
        previous.append(custom)
        defaults.set(try JSONEncoder().encode(previous), forKey: "moshpit.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        #expect(store.toolbar.contains { $0.chipLabel == "^L" })
        #expect(store.toolbar.contains { $0.chipLabel == "clm" })
        #expect(store.toolbar.contains { $0.kind == .mic })
    }

    @Test("a full toolbar keeps the mic out of the bar rather than over-filling it")
    func micRespectsToolbarCap() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        // 12 chips already in the bar — the cap — and no mic.
        let full = (0 ..< ShortcutStore.toolbarLimit).map { i -> TerminalShortcut in
            var sc = TerminalShortcut()
            sc.kind = .keyCombo; sc.key = "\(i)"; sc.chipLabel = "k\(i)"
            sc.isBuiltin = false; sc.inToolbar = true
            return sc
        }
        defaults.set(try JSONEncoder().encode(full), forKey: "moshpit.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        #expect(store.toolbarCount == ShortcutStore.toolbarLimit)
        #expect(!store.toolbar.contains { $0.kind == .mic })
        // Present but parked, so the editor can offer it rather than it
        // vanishing without trace.
        #expect(store.shortcuts.contains { $0.kind == .mic })
    }

    @Test("toolbar is capped at 12 slots")
    func toolbarCap() throws {
        let store = freshStore()
        // Add customs until full; the 13th should not enter the toolbar.
        for i in 0..<10 {
            var sc = TerminalShortcut()
            sc.chipLabel = "c\(i)"
            sc.kind = .text
            sc.payload = "x"
            store.add(sc)
        }
        let overflow = store.custom.filter { !$0.inToolbar }
        #expect(store.toolbarCount == ShortcutStore.toolbarLimit)
        #expect(overflow.isEmpty == false, "shortcuts beyond 12 stay out of the toolbar")
    }

    @Test("remove takes a builtin out of the toolbar but keeps it available")
    func removeBuiltin() throws {
        let store = freshStore()
        let esc = store.toolbar.first { $0.chipLabel == "esc" }!
        store.removeFromToolbar(id: esc.id)
        let inToolbar = store.toolbar.contains { $0.chipLabel == "esc" }
        let inAvailable = store.available.contains { $0.chipLabel == "esc" }
        #expect(!inToolbar)
        #expect(inAvailable)
    }

    @Test("scope filter hides host-scoped shortcuts on other hosts")
    func scopeFilter() throws {
        let store = freshStore()
        var scoped = TerminalShortcut()
        scoped.chipLabel = "⌘B"
        scoped.kind = .command
        scoped.payload = "tmux"
        scoped.scopeHosts = ["work"]
        store.add(scoped)

        let onWork = store.toolbar(forHost: "work", inMultiplexer: false).contains { $0.chipLabel == "⌘B" }
        let onOther = store.toolbar(forHost: "rednote", inMultiplexer: false).contains { $0.chipLabel == "⌘B" }
        #expect(onWork == true)
        #expect(onOther == false)
    }

    @Test("only-in-tmux shortcuts are hidden outside tmux")
    func tmuxOnlyFilter() throws {
        let store = freshStore()
        var sc = TerminalShortcut()
        sc.chipLabel = "clm"
        sc.kind = .command
        sc.payload = "claude"
        sc.onlyInTmux = true
        store.add(sc)
        let inMultiplexer = store.toolbar(forHost: nil, inMultiplexer: true).contains { $0.chipLabel == "clm" }
        let outsideMultiplexer = store.toolbar(forHost: nil, inMultiplexer: false).contains { $0.chipLabel == "clm" }
        #expect(inMultiplexer == true)
        #expect(outsideMultiplexer == false)
    }

    @Test("named CSI keys encode with xterm modifiers (⌃End jumps Claude Code to the live end)")
    func namedKeyEncoding() {
        func bytes(mods: Set<ShortcutModifier> = [], key: String) -> [UInt8] {
            var sc = TerminalShortcut()
            sc.kind = .keyCombo
            sc.modifiers = mods
            sc.key = key
            return sc.encodedBytes().map(Array.init) ?? []
        }
        #expect(bytes(mods: [.ctrl], key: "end") == Array("\u{1B}[1;5F".utf8))
        #expect(bytes(key: "end") == Array("\u{1B}[F".utf8))
        #expect(bytes(key: "home") == Array("\u{1B}[H".utf8))
        #expect(bytes(mods: [.shift], key: "tab") == Array("\u{1B}[Z".utf8))
        #expect(bytes(key: "pgup") == Array("\u{1B}[5~".utf8))
        #expect(bytes(mods: [.ctrl], key: "pgdn") == Array("\u{1B}[6;5~".utf8))
        // Plain arrows keep their historic encoding; ctrl adds word-jump.
        #expect(bytes(key: "up") == Array("\u{1B}[A".utf8))
        #expect(bytes(mods: [.ctrl], key: "left") == Array("\u{1B}[1;5D".utf8))
    }

    @Test("new builtins are injected into a persisted pre-existing set")
    func builtinInjection() {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)

        // Persist an OLD set that predates ⌃End/⇧Tab (drop them + the
        // available-list named keys), bypassing the store so no migration
        // runs yet. Key mirrors ShortcutStore.storageKey.
        let pruned = ShortcutStore.builtins.filter {
            !["Jump to end", "Back-tab / toggle mode", "End", "Home",
              "Page up", "Page down"].contains($0.summary)
        }
        defaults.set(try! JSONEncoder().encode(pruned), forKey: "moshpit.shortcuts.v1")

        // A fresh store over the same defaults must inject the new builtins.
        // "Jump to end" isn't one of the current 6 default-bar items, so the
        // injected copy starts outside the toolbar, like its own current default.
        let migrated = ShortcutStore(defaults: defaults)
        #expect(migrated.shortcuts.contains { $0.summary == "Jump to end" && !$0.inToolbar })
        #expect(migrated.shortcuts.contains { $0.summary == "Back-tab / toggle mode" })
        #expect(migrated.shortcuts.contains { $0.summary == "Page down" })
    }

    @Test("migration injects ctrl for a set that already has everything else, outside the toolbar")
    func migratesCtrlAheadOfExistingSet() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)

        // A realistic pre-existing set: every current builtin except ctrl
        // (ctrl postdates it), bypassing the store so no migration runs yet.
        let pruned = ShortcutStore.builtins.filter { $0.kind != .ctrl }
        defaults.set(try! JSONEncoder().encode(pruned), forKey: "moshpit.shortcuts.v1")

        // ctrl is a builtin now, like any other — it's added for a set that
        // predates it, but (like its own current default) starts outside the
        // toolbar rather than force-inserted ahead of everything else.
        let migrated = ShortcutStore(defaults: defaults)
        #expect(migrated.shortcuts.contains { $0.kind == .ctrl })
        #expect(migrated.available.contains { $0.kind == .ctrl })
        #expect(!migrated.toolbar.contains { $0.kind == .ctrl })
    }

    @Test("custom shortcuts persist across store instances")
    func persistence() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)

        let writer = ShortcutStore(defaults: defaults)
        var sc = TerminalShortcut()
        sc.chipLabel = "GP"
        sc.kind = .command
        sc.payload = "git push"
        writer.add(sc)

        let reader = ShortcutStore(defaults: defaults)
        let persisted = reader.custom.contains { $0.chipLabel == "GP" }
        #expect(persisted)
    }
}
