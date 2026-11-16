import Foundation
import Testing
@testable import Moshpit

// The bar's rules are the Rust store's (`crates/store/src/shortcuts.rs`,
// whose own tests hold them); these run them through the app's Swift store
// and value type, on the device, as the views do.

extension TerminalShortcut {
    /// Every byte a tap writes, in one piece (nil: none of its own) — the
    /// Rust store's stages, joined.
    func encodedBytes() -> Data? {
        let stages = encodedStages()
        return stages.isEmpty ? nil : stages.reduce(Data(), +)
    }

    /// A tap makes more than one write.
    var sendsDiscreteKeys: Bool { encodedStages().count > 1 }
}

/// `shortcuts` in the stored form (a kind 1.0.8 reads, the real one in
/// `shortcutKind`), as the store writes it — for seeding a set the store
/// has not seen.
func storedShortcutSet(_ shortcuts: [TerminalShortcut]) -> Data {
    let objects: [[String: Any]] = shortcuts.map { s in
        [
            "id": s.id.uuidString, "kind": s.kind.olderBuildsName.rawValue, "shortcutKind": s.kind.rawValue,
            "modifiers": s.modifiers.sorted().map(\.rawValue), "key": s.key, "payload": s.payload,
            "appendReturn": s.appendReturn, "repeatOnHold": s.repeatOnHold, "chipLabel": s.chipLabel,
            "summary": s.summary, "colorId": s.colorId, "scopeHosts": s.scopeHosts.sorted(),
            "onlyInTmux": s.onlyInTmux, "isBuiltin": s.isBuiltin, "inToolbar": s.inToolbar,
        ]
    }
    return try! JSONSerialization.data(withJSONObject: objects)
}

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

    @Test("the bulk-clear keys encode to their readline control bytes")
    func killLineKeys() throws {
        // ^U kills to line start, ^K to line end — one keystroke each, versus
        // holding backspace for one tmux round trip PER character.
        #expect(combo([.ctrl], "u").encodedBytes() == Data([0x15]))
        #expect(combo([.ctrl], "k").encodedBytes() == Data([0x0B]))
        let builtins = ShortcutStore.builtins
        #expect(builtins.contains { $0.chipLabel == "^U" })
        #expect(builtins.contains { $0.chipLabel == "^K" })
    }

    @Test("Return encodes to CR")
    func returnKey() throws {
        #expect(combo([], "return").encodedBytes() == Data([0x0D]))
        #expect(combo([], "enter").encodedBytes() == Data([0x0D]))
    }

    @Test("The Claude Code two-step sends Tab then Return, in that order")
    func tabThenEnter() throws {
        // Accepting a suggested prompt is Tab followed by Return, and the
        // order is the whole point — so one chip can do both.
        let sc = try #require(ShortcutStore.builtins.first { $0.chipLabel == "⇥⏎" })
        #expect(sc.kind == .text)
        #expect(sc.encodedBytes() == Data([0x09, 0x0D]))
        // Must NOT append its own CR on top — that would submit twice.
        #expect(sc.appendReturn == false)
        #expect(sc.isBuiltin)
        // …and as TWO writes. Reported as "tab + enter only did the tab":
        // arriving in one read, the Return reads as pasted text and lands as a
        // newline in the input box instead of submitting.
        #expect(sc.encodedStages() == [Data([0x09]), Data([0x0D])])
    }

    @Test("only control-byte payloads split; text stays one write")
    func onlyKeySequencesSplit() {
        func text(_ payload: String) -> TerminalShortcut {
            var sc = TerminalShortcut()
            sc.kind = .text
            sc.payload = payload
            return sc
        }
        // Typing a command IS one string to the shell — splitting it would put
        // 120ms between every character.
        #expect(text("git status").encodedStages() == [Data("git status".utf8)])
        #expect(text("git status").sendsDiscreteKeys == false)
        // A lone key has nothing to pace against.
        #expect(text(#"\r"#).encodedStages() == [Data([0x0D])])
        // Mixed content is text with a submit on the end, not a key run.
        #expect(text(#"deploy\r"#).sendsDiscreteKeys == false)
        // Escape-then-key runs split like ⇥⏎ does.
        #expect(text(#"\e\r"#).encodedStages() == [Data([0x1B]), Data([0x0D])])
    }

    @Test("ctrl (like dpad/scroll) has no bytes of its own — resolved by the caller")
    func ctrlKindEncodesNothing() throws {
        var sc = TerminalShortcut()
        sc.kind = .ctrl
        #expect(sc.encodedBytes() == nil)
    }

    @Test("modifiers sort ⌃⌥⇧⌘, the order macOS writes them")
    func modifierOrder() {
        // Pins the order now that `<` reads it off an exhaustive switch instead
        // of searching a lookup table with a force-unwrapped firstIndex.
        let scrambled: [ShortcutModifier] = [.cmd, .shift, .alt, .ctrl]
        let expected: [ShortcutModifier] = [.ctrl, .alt, .shift, .cmd]
        #expect(scrambled.sorted() == expected)
        #expect(ShortcutModifier.allCases.sorted() == expected)
    }

    @Test("the arrow cluster has no bytes of its own — its zones route through onArrow")
    func arrowsKindEncodesNothing() throws {
        var sc = TerminalShortcut()
        sc.kind = .arrows
        // Deliberately nil rather than a CSI: the bar's `onArrow` encodes for
        // the remote's *current* cursor-key mode, and a fixed sequence here
        // would break history search in application-cursor-mode shells.
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

    @Test("seeds a lean default toolbar: esc, tab, interrupt, paste, arrows, mic, img, files")
    func defaultToolbar() throws {
        let store = freshStore()
        let labels = store.toolbar.map(\.chipLabel)
        let allBuiltin = store.available.allSatisfy(\.isBuiltin)
        // The handful of keys actually reached for constantly. Ctrl and the
        // scroll thumb start outside the bar (available to add back), along
        // with everything else that used to be a default (^D, ^R, ⌃End, ⇧Tab).
        // The mic trails the row: it used to be pinned outside the scroll at
        // the trailing edge, so this is where it already appeared. The image
        // chip trails even that — same "attach something" family as the mic,
        // added to the defaults with the M1 image-attach feature. The files
        // chip trails the image: the way back out to what an agent wrote.
        #expect(store.toolbarCount == 8)
        #expect(labels == ["esc", "tab", "^C", "paste", "✛", "mic", "img", "files"])
        #expect(store.toolbar.last?.kind == .files)
        #expect(store.available.contains { $0.kind == .ctrl })
        #expect(store.available.contains { $0.kind == .scroll })
        #expect(allBuiltin)
    }

    @Test("the core editing keys always fit a phone-width row")
    func defaultToolbarCoreKeysFitOnScreen() throws {
        // The contract CHANGED when the image chip joined the defaults: the
        // full default row now deliberately exceeds a 402pt phone, because
        // every honest alternative was worse — no default chip is evictable
        // the way ^L was (each is a feature's only always-visible door), and
        // shaving chip metrics couldn't recover the 52pt a chip costs. What
        // made the overflow acceptable is the bar's edge behaviour: a crisply
        // hard-clipped chip that pans into reach (the old edge FADE sat mid-
        // glyph at rest and read as a rendering bug — user report, 2026-08-16).
        //
        // The invariant that remains load-bearing: the CORE editing keys —
        // everything up to and including the D-pad — must sit fully inside
        // the row at rest, so nothing you type/steer with ever needs a
        // scroll. Trailing feature chips (mic, img, files) are allowed past the
        // edge. Widths mirror the layout: chips 46pt (D-pad 42), 6pt gaps,
        // 8pt leading inset; the pinned keyboard toggle occupies 54pt
        // (4 + 42 + 8) of a 402pt screen.
        let store = freshStore()
        let dpad = try #require(store.toolbar.firstIndex { $0.kind == .dpad })
        let core = store.toolbar[...dpad]
        let width = core.reduce(CGFloat(8)) { total, shortcut in
            total + (shortcut.kind == .dpad ? 42 : 46) + 6
        }
        #expect(width <= 402 - 54, "core keys overflow: \(width)pt")
        // And the feature chips are where the contract expects them — at the
        // trailing edge, where clipping only ever hides a re-openable door.
        #expect(store.toolbar.last?.kind == .files)
    }

    @Test("the tap-first cluster ships available but out of the bar")
    func tapArrowsAvailableNotDefault() throws {
        let store = freshStore()
        // The joystick keeps the default slot: one chip for four directions,
        // where the cluster's four zones cost ~2.5 and overflow a phone row.
        // The cluster is the answer to "a tap can't carry a direction", so it
        // has to be reachable — just not at the cost of the default layout.
        let cluster = try #require(store.shortcuts.first { $0.kind == .arrows })
        #expect(!cluster.inToolbar)
        #expect(store.available.contains { $0.kind == .arrows })
        #expect(store.toolbar.contains { $0.kind == .dpad })
        // Distinct summaries, because summary is the identity the builtin
        // catch-all migration keys on — two "Arrow keys" would make it inject
        // duplicates for anyone upgrading.
        #expect(cluster.summary != store.toolbar.first { $0.kind == .dpad }?.summary)
    }

    @Test("all four arrows exist as individual chips, outside the bar")
    func individualArrowBuiltinsExist() throws {
        let store = freshStore()
        // ← and → had no chip at all before the cluster landed — the joystick
        // was the only way to move a cursor sideways, which is the press-and-
        // hold complaint in its purest form.
        for key in ["up", "down", "left", "right"] {
            let sc = try #require(store.shortcuts.first { $0.kind == .keyCombo && $0.key == key },
                                  "no builtin chip for \(key)")
            #expect(sc.repeatOnHold, "\(key) should hold-to-repeat like a hardware key")
            #expect(!sc.inToolbar, "the cluster covers all four; single chips stay available")
        }
    }

    @Test("a builtin the user put in the bar is still there on the next launch")
    func userPlacedBuiltinSurvivesRelaunch() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        // ↑ on its own is the chip the builtins' comment expects people to
        // add. A pre-1.0 upgrade step used to take it back out of the bar on
        // every load.
        let first = ShortcutStore(defaults: defaults)
        let up = try #require(first.available.first { $0.kind == .keyCombo && $0.key == "up" })
        first.addToToolbar(id: up.id)

        let relaunched = ShortcutStore(defaults: defaults)
        #expect(relaunched.toolbar.contains { $0.id == up.id })
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
        defaults.set(storedShortcutSet(full), forKey: "moshpit.shortcuts.v1")

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

    /// B233: AVAILABLE listed the built-ins alone, so a custom shortcut taken
    /// off the bar — or saved while the bar was full — could never come back,
    /// while the editor's footnote said it stays there.
    @Test("a custom shortcut taken off the bar stays available and can go back on")
    func customShortcutComesBack() throws {
        let store = freshStore()
        var mine = TerminalShortcut()
        mine.chipLabel = "gs"
        mine.kind = .text
        mine.payload = "git status"
        store.add(mine)
        let id = try #require(store.toolbar.first { $0.chipLabel == "gs" }?.id)
        store.removeFromToolbar(id: id)
        #expect(!store.toolbar.contains { $0.id == id })
        #expect(store.available.contains { $0.id == id }, "a custom shortcut off the bar has no way back")
        #expect(store.addToToolbar(id: id))
        #expect(store.toolbar.contains { $0.id == id })
        #expect(!store.available.contains { $0.id == id })
    }

    @Test("with twelve on the bar, ＋ adds nothing and says it can't; one saved then waits under AVAILABLE")
    func fullBarSaysSo() throws {
        let store = freshStore()
        while !store.toolbarIsFull {
            var filler = TerminalShortcut()
            filler.chipLabel = "f\(store.toolbarCount)"
            filler.kind = .text
            filler.payload = "x"
            store.add(filler)
        }
        #expect(store.toolbarCount == ShortcutStore.toolbarLimit)
        var late = TerminalShortcut()
        late.chipLabel = "late"
        late.kind = .text
        late.payload = "y"
        // B252: the store says it didn't go on the bar, for the editor to say.
        #expect(!store.add(late), "saved off a full bar, and said it went on")
        let saved = try #require(store.shortcuts.first { $0.chipLabel == "late" })
        #expect(!saved.inToolbar)
        #expect(store.available.contains { $0.id == saved.id }, "saved while the bar was full: nowhere to be found")
        #expect(!store.addToToolbar(id: saved.id), "a ＋ on a full bar did nothing and said nothing")
        #expect(store.toolbarCount == ShortcutStore.toolbarLimit)
        // A slot frees: it goes on.
        store.removeFromToolbar(id: store.toolbar[0].id)
        #expect(store.addToToolbar(id: saved.id))
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
        defaults.set(storedShortcutSet(pruned), forKey: "moshpit.shortcuts.v1")

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
        defaults.set(storedShortcutSet(pruned), forKey: "moshpit.shortcuts.v1")

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

/// The sealed launch (see ConnectionStoreTests), for the one store that used
/// to WRITE when it found nothing: it seeded the built-ins and persisted them,
/// and over a file that was merely still encrypted that erased every custom
/// chip for good.
@Suite("ShortcutStore on a sealed launch")
struct ShortcutStoreSealedLaunchTests {
    /// The store's key, duplicated so the test can look at the disk directly.
    private static let storageKey = "moshpit.shortcuts.v1"

    private final class Phone { var unlocked = false }

    @Test("built-ins are shown, not written, over a file that could not be read")
    func sealedLaunchDoesNotSeedOverCustomChips() throws {
        let suite = "test.shortcuts.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }

        // What a previous, trusted run left on disk: one custom chip.
        let scratchName = "test.shortcuts.\(UUID().uuidString)"
        let scratch = UserDefaults(suiteName: scratchName)!
        defer { scratch.removePersistentDomain(forName: scratchName) }
        var deploy = TerminalShortcut()
        deploy.kind = .text
        deploy.payload = "make deploy"
        deploy.chipLabel = "deploy"
        deploy.summary = "Deploy"
        ShortcutStore(defaults: scratch).add(deploy)
        let payload = try #require(scratch.data(forKey: Self.storageKey))

        let phone = Phone()
        let store = ShortcutStore(defaults: defaults, protectedData: { phone.unlocked })
        // Usable — the bar has its shipped chips — but nothing reached the disk.
        #expect(!store.toolbar.isEmpty)
        #expect(!store.isAuthoritative)
        #expect(defaults.data(forKey: Self.storageKey) == nil,
                "seeding the built-ins over a sealed file is how custom chips were lost")

        // The file becomes readable; the app comes forward.
        defaults.set(payload, forKey: Self.storageKey)
        phone.unlocked = true
        store.reloadIfNeeded()
        #expect(store.custom.map(\.summary) == ["Deploy"])
        #expect(store.isAuthoritative)
    }
}

/// `TerminalShortcut` as 1.0.8 declares it (8c10490a), field for field, with
/// the kinds it knows — no `files`. 1.0.8 reads the stored set the way
/// `decodeAsOlderBuild` does, the whole array in one decode, and took a set
/// that failed it as nothing: it seeded its built-ins and saved them over
/// the set.
private struct OlderBuildShortcut: Codable, Equatable {
    enum Kind: String, Codable {
        case keyCombo, text, command, dpad, arrows, paste, scroll, ctrl, mic, image
    }

    var id = UUID()
    var kind: Kind = .keyCombo
    var modifiers: Set<ShortcutModifier> = []
    var key: String = ""
    var payload: String = ""
    var appendReturn: Bool = false
    var repeatOnHold: Bool = false
    var chipLabel: String = ""
    var summary: String = ""
    var colorId: String = "gray"
    var scopeHosts: Set<String> = []
    var onlyInTmux: Bool = false
    var isBuiltin: Bool = false
    var inToolbar: Bool = true
}

/// B153: since the files chip, every set the app saves holds it, and it was
/// stored as `"kind":"files"` — a kind 1.0.8 can't decode, so a downgrade
/// (TestFlight keeps 1.0.8 a tap away) erased every custom chip. `kind` now
/// holds a kind 1.0.8 knows and the real one rides in `shortcutKind`, as
/// moshpit-store writes it (Fix (files-downgrade)).
@Suite("Shortcut sets across builds")
struct ShortcutDowngradeTests {
    private static let storageKey = "moshpit.shortcuts.v1"

    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.shortcuts.downgrade.\(UUID().uuidString)"
        return (UserDefaults(suiteName: name)!, name)
    }

    private static func decodeAsOlderBuild(_ data: Data) throws -> [OlderBuildShortcut] {
        try JSONDecoder().decode([OlderBuildShortcut].self, from: data)
    }

    private static func objects(_ data: Data) throws -> [[String: Any]] {
        try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
    }

    @Test("a set the app saves decodes in 1.0.8, custom chips and all")
    func olderBuildReadsTheSet() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = ShortcutStore(defaults: defaults)
        var mine = TerminalShortcut()
        mine.kind = .command
        mine.chipLabel = "GP"
        mine.payload = "git push"
        store.add(mine)
        let data = try #require(defaults.data(forKey: Self.storageKey))

        // The old encoding failed right here: "files" is no kind 1.0.8 has.
        let older = try Self.decodeAsOlderBuild(data)

        #expect(older.count == store.shortcuts.count)
        #expect(older.contains { $0.chipLabel == "GP" && $0.kind == .command && $0.payload == "git push" })
        // The files chip is an inert key combo there: no key, no payload.
        let files = try #require(older.first { $0.summary == "Browse files" })
        #expect(files.kind == .keyCombo && files.key.isEmpty && files.payload.isEmpty && files.isBuiltin)
    }

    /// What a fresh store writes over `defaults`.
    private static func written(_ defaults: UserDefaults) throws -> Data {
        _ = ShortcutStore(defaults: defaults)
        return try #require(defaults.data(forKey: storageKey))
    }

    @Test("every chip carries its real kind beside one every build reads")
    func storedShape() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let builtins = ShortcutStore.builtins
        let objects = try Self.objects(try Self.written(defaults))
        #expect(objects.count == builtins.count)
        for (object, shortcut) in zip(objects, builtins) {
            #expect(object["shortcutKind"] as? String == shortcut.kind.rawValue)
            let expected = shortcut.kind == .files ? "keyCombo" : shortcut.kind.rawValue
            #expect(object["kind"] as? String == expected, "\(shortcut.summary)")
        }
    }

    @Test("this build reads its own set back with the files chip intact")
    func roundTrip() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        _ = try Self.written(defaults)
        let back = ShortcutStore(defaults: defaults).shortcuts
        #expect(back.map(\.summary) == ShortcutStore.builtins.map(\.summary))
        #expect(back.contains { $0.kind == .files })
    }

    @Test("a set 1.0.8 saved back finds the files chip again by its summary")
    func olderBuildsSaveIsRecovered() throws {
        let (defaults, suite) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        let older = try Self.decodeAsOlderBuild(try Self.written(defaults))
        defaults.set(try JSONEncoder().encode(older), forKey: Self.storageKey)

        let store = ShortcutStore(defaults: defaults)

        let files = store.shortcuts.filter { $0.summary == "Browse files" }
        #expect(files.count == 1, "the chip was injected a second time")
        #expect(files.first?.kind == .files)
        // An inert key combo of the user's own is not taken for it.
        let (mine, mineSuite) = Self.makeDefaults()
        defer { mine.removePersistentDomain(forName: mineSuite) }
        var custom = OlderBuildShortcut()
        custom.summary = "Browse files"
        mine.set(try JSONEncoder().encode([custom]), forKey: Self.storageKey)
        #expect(ShortcutStore(defaults: mine).shortcuts.first { $0.id == custom.id }?.kind == .keyCombo)
    }

    @Test("the files chip as earlier builds stored it still reads; an unknown real kind falls back")
    func earlierShapes() throws {
        var files = ShortcutStore.builtins.first { $0.kind == .files }!
        files.id = UUID()
        var object = try #require(try Self.objects(storedShortcutSet([files])).first)
        func read(_ object: [String: Any]) throws -> ShortcutKind? {
            let (defaults, suite) = Self.makeDefaults()
            defer { defaults.removePersistentDomain(forName: suite) }
            defaults.set(try JSONSerialization.data(withJSONObject: [object]), forKey: Self.storageKey)
            return ShortcutStore(defaults: defaults).shortcuts.first { $0.id == files.id }?.kind
        }
        object["kind"] = "files"
        object.removeValue(forKey: "shortcutKind")
        #expect(try read(object) == .files)

        object["kind"] = "text"
        object["shortcutKind"] = "a kind from a newer build"
        #expect(try read(object) == .text)
    }
}
