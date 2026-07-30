import Foundation
import Testing
@testable import Ringdown

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

    private func freshStore() -> ShortcutStore {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        return ShortcutStore(defaults: defaults)
    }

    @Test("seeds a lean default toolbar: esc, tab, interrupt, clear, paste, arrows")
    func defaultToolbar() throws {
        let store = freshStore()
        let labels = store.toolbar.map(\.chipLabel)
        let allBuiltin = store.available.allSatisfy(\.isBuiltin)
        // The handful of keys actually reached for constantly. Ctrl and the
        // scroll thumb start outside the bar (available to add back), along
        // with everything else that used to be a default (^D, ^R, ⌃End, ⇧Tab).
        #expect(store.toolbarCount == 6)
        #expect(labels == ["esc", "tab", "^C", "^L", "paste", "✛"])
        #expect(store.toolbar.last?.kind == .dpad)
        #expect(store.available.contains { $0.kind == .ctrl })
        #expect(store.available.contains { $0.kind == .scroll })
        #expect(allBuiltin)
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
        defaults.set(data, forKey: "ringdown.shortcuts.v1")

        let store = ShortcutStore(defaults: defaults)
        // dpad joins the toolbar (its own current default); ctrl is injected
        // too but — like its own current default — starts outside the bar.
        #expect(store.toolbar.first?.kind == .dpad)
        #expect(store.shortcuts.contains { $0.kind == .ctrl })
        #expect(store.available.contains { $0.kind == .ctrl })
        #expect(store.toolbar.contains { $0.chipLabel == "esc" })
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

        let onWork = store.toolbar(forHost: "work", inTmux: false).contains { $0.chipLabel == "⌘B" }
        let onOther = store.toolbar(forHost: "rednote", inTmux: false).contains { $0.chipLabel == "⌘B" }
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
        let inTmux = store.toolbar(forHost: nil, inTmux: true).contains { $0.chipLabel == "clm" }
        let outsideTmux = store.toolbar(forHost: nil, inTmux: false).contains { $0.chipLabel == "clm" }
        #expect(inTmux == true)
        #expect(outsideTmux == false)
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
        defaults.set(try! JSONEncoder().encode(pruned), forKey: "ringdown.shortcuts.v1")

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
        defaults.set(try! JSONEncoder().encode(pruned), forKey: "ringdown.shortcuts.v1")

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
