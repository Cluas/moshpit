import MoshpitFFI
import MoshpitKit
import UIKit

// A hardware keyboard on the core view. Text keys go through the text input
// system (an input method may take them); the keys a terminal needs that
// produce no text — arrows, Home/End, Page Up/Down, forward delete, Escape,
// F1–F24, Control and Option chords, Shift-Tab — are encoded by the core as the remote
// program asked (DECCKM, the kitty keyboard protocol) and repeated while
// held at the platform's timing (fork patch 52). ⌘ chords are left to the
// responder chain, so the app's menu commands fire once (W11h). Under the
// kitty protocol a key that was pressed is released, also when UIKit takes
// the press back (B237, patches 61–62).

extension CoreTerminalView {
    /// `UIKeyModifierFlags` as the core's `MP_MOD_*`.
    static func mods(_ flags: UIKeyModifierFlags) -> UInt8 {
        var m: UInt8 = 0
        if flags.contains(.shift) { m |= UInt8(MP_MOD_SHIFT) }
        if flags.contains(.alternate) { m |= UInt8(MP_MOD_ALT) }
        if flags.contains(.control) { m |= UInt8(MP_MOD_CTRL) }
        if flags.contains(.command) { m |= UInt8(MP_MOD_SUPER) }
        if flags.contains(.alphaShift) { m |= UInt8(MP_MOD_CAPS_LOCK) }
        return m
    }

    /// The core's key for a key that makes no text; nil for the others.
    static func functionalKey(_ usage: UIKeyboardHIDUsage) -> UInt32? {
        switch usage {
        case .keyboardUpArrow: UInt32(MP_KEY_UP)
        case .keyboardDownArrow: UInt32(MP_KEY_DOWN)
        case .keyboardLeftArrow: UInt32(MP_KEY_LEFT)
        case .keyboardRightArrow: UInt32(MP_KEY_RIGHT)
        case .keyboardHome: UInt32(MP_KEY_HOME)
        case .keyboardEnd: UInt32(MP_KEY_END)
        case .keyboardPageUp: UInt32(MP_KEY_PAGE_UP)
        case .keyboardPageDown: UInt32(MP_KEY_PAGE_DOWN)
        case .keyboardDeleteForward: UInt32(MP_KEY_DELETE)
        case .keyboardInsert: UInt32(MP_KEY_INSERT)
        case .keyboardEscape: UInt32(MP_KEY_ESCAPE)
        case .keyboardF1: UInt32(MP_KEY_FUNCTION) + 1
        case .keyboardF2: UInt32(MP_KEY_FUNCTION) + 2
        case .keyboardF3: UInt32(MP_KEY_FUNCTION) + 3
        case .keyboardF4: UInt32(MP_KEY_FUNCTION) + 4
        case .keyboardF5: UInt32(MP_KEY_FUNCTION) + 5
        case .keyboardF6: UInt32(MP_KEY_FUNCTION) + 6
        case .keyboardF7: UInt32(MP_KEY_FUNCTION) + 7
        case .keyboardF8: UInt32(MP_KEY_FUNCTION) + 8
        case .keyboardF9: UInt32(MP_KEY_FUNCTION) + 9
        case .keyboardF10: UInt32(MP_KEY_FUNCTION) + 10
        case .keyboardF11: UInt32(MP_KEY_FUNCTION) + 11
        case .keyboardF12: UInt32(MP_KEY_FUNCTION) + 12
        default: nil
        }
    }

    /// The modifier keys themselves (only the kitty protocol's "report all
    /// keys" sends them).
    static func modifierKey(_ usage: UIKeyboardHIDUsage) -> UInt32? {
        switch usage {
        case .keyboardLeftShift: UInt32(MP_KEY_LEFT_SHIFT)
        case .keyboardRightShift: UInt32(MP_KEY_RIGHT_SHIFT)
        case .keyboardLeftControl: UInt32(MP_KEY_LEFT_CONTROL)
        case .keyboardRightControl: UInt32(MP_KEY_RIGHT_CONTROL)
        case .keyboardLeftAlt: UInt32(MP_KEY_LEFT_ALT)
        case .keyboardRightAlt: UInt32(MP_KEY_RIGHT_ALT)
        case .keyboardLeftGUI: UInt32(MP_KEY_LEFT_SUPER)
        case .keyboardRightGUI: UInt32(MP_KEY_RIGHT_SUPER)
        case .keyboardCapsLock: UInt32(MP_KEY_CAPS_LOCK)
        default: nil
        }
    }

    /// What an Option chord sends: Option is Meta (B413), as the SwiftTerm
    /// fork's `optionAsMetaKey` had it — the key encoded with Alt (ESC and
    /// the key; kitty's encoding under its protocol), not the layout's
    /// ∫ ƒ ≥ or a dead key's composition. Option-Return is Meta-Enter
    /// (Claude Code's newline), Option-Delete Meta-Backspace (readline's
    /// word delete), Option-←/→ readline's word moves (ESC b, ESC f) outside
    /// the kitty protocol. Nil for a key that is none of these (the other
    /// arrows and function keys keep their own encoding, with Alt).
    static func metaKey(_ usage: UIKeyboardHIDUsage, characters: String, flags: UIKeyModifierFlags,
                        kitty: UInt8) -> (key: UInt32, scalar: UInt32, mods: UInt8)? {
        guard flags.contains(.alternate), flags.isDisjoint(with: [.command, .control]) else { return nil }
        let mods = Self.mods(flags)
        let alt = UInt8(MP_MOD_ALT)
        switch usage {
        case .keyboardLeftArrow, .keyboardRightArrow:
            guard kitty == 0, mods & ~UInt8(MP_MOD_CAPS_LOCK) == alt else { return nil }
            let letter: Character = usage == .keyboardLeftArrow ? "b" : "f"
            return (UInt32(MP_KEY_CHAR), letter.unicodeScalars.first!.value, alt)
        case .keyboardReturnOrEnter, .keypadEnter:
            return (UInt32(MP_KEY_ENTER), 0, mods)
        case .keyboardDeleteOrBackspace:
            return (UInt32(MP_KEY_BACKSPACE), 0, mods)
        default:
            break
        }
        let scalars = characters.unicodeScalars
        guard scalars.count == 1, let c = scalars.first, c.value >= 0x20, c.value != 0x7f,
              !(0xF700...0xF8FF).contains(c.value) else { return nil }
        return (UInt32(MP_KEY_CHAR), c.value, mods)
    }

    /// How long a held key waits, then how often it repeats: a Mac's own
    /// settings on Catalyst, read at every press; iPadOS gives an app none
    /// (0.4 s, 0.1 s) — fork patch 52.
    static func keyRepeatTiming() -> (delay: TimeInterval, interval: TimeInterval) {
        #if targetEnvironment(macCatalyst)
        func appKit(_ name: String) -> TimeInterval? {
            let selector = NSSelectorFromString(name)
            guard let event = NSClassFromString("NSEvent"), let method = class_getClassMethod(event, selector) else {
                return nil
            }
            typealias Getter = @convention(c) (AnyClass, Selector) -> Double
            let value = unsafeBitCast(method_getImplementation(method), to: Getter.self)(event, selector)
            return value > 0 && value < 5 ? value : nil
        }
        if let delay = appKit("keyRepeatDelay"), let interval = appKit("keyRepeatInterval") { return (delay, interval) }
        #endif
        return (0.4, 0.1)
    }

    func startKeyRepeat(_ body: @escaping (_ count: UInt16) -> Void) {
        keyRepeat?.invalidate()
        let timing = Self.keyRepeatTiming()
        var count: UInt16 = 1
        let timer = Timer(fire: Date(timeIntervalSinceNow: timing.delay), interval: timing.interval, repeats: true) { _ in
            count = count &+ 1
            MainActor.assumeIsolated { body(max(count, 2)) }
        }
        RunLoop.main.add(timer, forMode: .common)
        keyRepeat = timer
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        // An input method composing takes every key.
        guard imeState.marked_start < 0 else { return super.pressesBegan(presses, with: event) }
        var unhandled = Set<UIPress>()
        let kitty = termModes.kitty_flags
        for press in presses {
            guard let key = press.key else { unhandled.insert(press); continue }
            if key.keyCode == .keyboardLeftGUI || key.keyCode == .keyboardRightGUI { commandKeys.insert(key.keyCode) }
            // ⌘ chords are the app's (its menu, copy and paste): once each.
            if key.modifierFlags.contains(.command) || !commandKeys.isEmpty {
                unhandled.insert(press)
                continue
            }
            let mods = Self.mods(key.modifierFlags)
            if let modifier = Self.modifierKey(key.keyCode) {
                if kitty & 8 != 0 {
                    sendKey(modifier, mods: mods)
                    keysDown[key.keyCode] = (modifier, 0, mods)
                }
                unhandled.insert(press)
                continue
            }
            // Option chords: Meta (B413).
            if let meta = Self.metaKey(key.keyCode, characters: key.charactersIgnoringModifiers,
                                       flags: key.modifierFlags, kitty: kitty) {
                sendKey(meta.key, scalar: meta.scalar, mods: meta.mods)
                if kitty != 0 { keysDown[key.keyCode] = (meta.key, meta.scalar, meta.mods) }
                startKeyRepeat { [weak self] n in
                    self?.sendKey(meta.key, scalar: meta.scalar, mods: meta.mods, repeat: n)
                }
                continue
            }
            if let code = Self.functionalKey(key.keyCode) {
                // Page Up/Down page the local history over a shell, as a
                // terminal's own scrollback does; a program in application
                // cursor mode gets them.
                if code == UInt32(MP_KEY_PAGE_UP) || code == UInt32(MP_KEY_PAGE_DOWN), !termModes.app_cursor,
                   !termModes.alt_screen, kitty == 0, mods == 0 {
                    scrollLines((code == UInt32(MP_KEY_PAGE_UP) ? 1 : -1) * max(1, grid.rows - 1))
                    continue
                }
                sendKey(code, mods: mods)
                if kitty != 0 { keysDown[key.keyCode] = (code, 0, mods) }
                if code != UInt32(MP_KEY_ESCAPE) {
                    startKeyRepeat { [weak self] n in self?.sendKey(code, mods: mods, repeat: n) }
                }
                continue
            }
            // Shift-Tab: nothing the text system sends.
            if key.keyCode == .keyboardTab, key.modifierFlags.contains(.shift) {
                sendKey(UInt32(MP_KEY_TAB), mods: mods)
                continue
            }
            // Control chords: the control codes (or kitty's encoding).
            if key.modifierFlags.contains(.control), let c = key.charactersIgnoringModifiers.unicodeScalars.first {
                let isBackspace = key.keyCode == .keyboardDeleteOrBackspace
                let code = isBackspace ? UInt32(MP_KEY_BACKSPACE) : UInt32(MP_KEY_CHAR)
                let scalar = isBackspace ? 0 : Character(c).lowercased().unicodeScalars.first?.value ?? c.value
                sendKey(code, scalar: scalar, mods: mods)
                if kitty != 0 { keysDown[key.keyCode] = (code, scalar, mods) }
                startKeyRepeat { [weak self] n in self?.sendKey(code, scalar: scalar, mods: mods, repeat: n) }
                continue
            }
            // A text key under the kitty protocol: its text comes through the
            // text system; its release is sent from here.
            if kitty & 2 != 0, let c = key.charactersIgnoringModifiers.unicodeScalars.first, c.value >= 0x20 {
                keysDown[key.keyCode] = (UInt32(MP_KEY_CHAR), c.value, mods)
            }
            #if targetEnvironment(macCatalyst)
            repeatHeldTextKey(key)
            #endif
            unhandled.insert(press)
        }
        if !unhandled.isEmpty { super.pressesBegan(unhandled, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        keysUp(presses)
        super.pressesEnded(presses, with: event)
    }

    /// A press UIKit took back (the app lost the keyboard with the key down,
    /// the system claimed the key): it ends here too — no repeat goes on, no
    /// ⌘ stays held (patch 61), the kitty program hears the release (62).
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        keysUp(presses)
        super.pressesCancelled(presses, with: event)
    }

    private func keysUp(_ presses: Set<UIPress>) {
        keyRepeat?.invalidate()
        keyRepeat = nil
        for press in presses {
            guard let key = press.key else { continue }
            commandKeys.remove(key.keyCode)
            if let down = keysDown.removeValue(forKey: key.keyCode), termModes.kitty_flags & 2 != 0 {
                sendKey(down.key, scalar: down.scalar, mods: down.mods, down: false)
            }
        }
    }

    #if targetEnvironment(macCatalyst)
    /// A held letter repeats on a Mac at the Mac's timing (the system
    /// repeats text only for its own text views), unless a composition
    /// starts (patch 52).
    private func repeatHeldTextKey(_ key: UIKey) {
        let text = key.characters
        guard imeState.marked_start < 0, key.modifierFlags.isDisjoint(with: [.command, .control, .alternate]), !text.isEmpty,
              text.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x7f && !(0xF700...0xF8FF).contains($0.value) })
        else { return }
        startKeyRepeat { [weak self] _ in
            guard let self else { return }
            if self.imeState.marked_start >= 0 {
                self.keyRepeat?.invalidate()
                self.keyRepeat = nil
                return
            }
            self.insertText(text)
        }
    }

    /// On a Mac the text system takes Escape ("stop editing") and Tab and
    /// Shift-Tab (focus) before the view sees them: commands of the view's
    /// own, ahead of the system's (patches 50, 51).
    override var keyCommands: [UIKeyCommand]? {
        let commands = [
            UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(coreEscape)),
            UIKeyCommand(input: "\t", modifierFlags: [], action: #selector(coreTab)),
            UIKeyCommand(input: "\t", modifierFlags: .shift, action: #selector(coreBacktab)),
        ]
        for c in commands { c.wantsPriorityOverSystemBehavior = true }
        return commands
    }

    @objc private func coreEscape() { sendKey(UInt32(MP_KEY_ESCAPE), mods: 0) }
    @objc private func coreTab() { sendKey(UInt32(MP_KEY_TAB), mods: 0) }
    @objc private func coreBacktab() { sendKey(UInt32(MP_KEY_TAB), mods: UInt8(MP_MOD_SHIFT)) }
    #endif
}
