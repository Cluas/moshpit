// How the harness reads a case's shortcut JSON (`short` cases' add and
// update, `bytes` cases): the app's `TerminalShortcut` decoder as it was
// until b85f84d1 moved the stored form into the Rust store
// (crates/store/src/shortcuts.rs, `TerminalShortcut::from_json`, which the
// harness proved reads the same): its keys and `init(from:)`, verbatim
// from b85f84d1^'s Moshpit/Models/TerminalShortcut.swift (the encoding
// half, and the conformance, were on the type). The app decodes no shortcut in
// Swift any more; this only turns a case's input into the value the app's
// store takes, as the Rust side's `from_json` does.

import Foundation

extension TerminalShortcut: Decodable {
    /// Every key 1.0.8's synthesized `Codable` requires, and `shortcutKind`.
    private enum CodingKeys: String, CodingKey {
        case id, kind, shortcutKind, modifiers, key, payload, appendReturn, repeatOnHold,
             chipLabel, summary, colorId, scopeHosts, onlyInTmux, isBuiltin, inToolbar
    }

    /// The summary that names the files built-in for good (see `builtins`).
    private static let filesSummary = "Browse files"

    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        self.init()
        id = try fields.decode(UUID.self, forKey: .id)
        let stored = try fields.decode(ShortcutKind.self, forKey: .kind)
        modifiers = try fields.decode(Set<ShortcutModifier>.self, forKey: .modifiers)
        key = try fields.decode(String.self, forKey: .key)
        payload = try fields.decode(String.self, forKey: .payload)
        appendReturn = try fields.decode(Bool.self, forKey: .appendReturn)
        repeatOnHold = try fields.decode(Bool.self, forKey: .repeatOnHold)
        chipLabel = try fields.decode(String.self, forKey: .chipLabel)
        summary = try fields.decode(String.self, forKey: .summary)
        colorId = try fields.decode(String.self, forKey: .colorId)
        scopeHosts = try fields.decode(Set<String>.self, forKey: .scopeHosts)
        onlyInTmux = try fields.decode(Bool.self, forKey: .onlyInTmux)
        isBuiltin = try fields.decode(Bool.self, forKey: .isBuiltin)
        inToolbar = try fields.decode(Bool.self, forKey: .inToolbar)
        // The real kind when this build knows it; anything else there (a
        // kind from a newer build) falls back to `kind`, as 1.0.8 reads it.
        let raw: String? = (try? fields.decodeIfPresent(String.self, forKey: .shortcutKind)) ?? nil
        let real = raw.flatMap(ShortcutKind.init(rawValue:))
        // A set an older build saved back has no real kind: its files
        // built-in is the inert key combo it was given, named by its summary.
        let recovered = stored == .keyCombo && isBuiltin && key.isEmpty && payload.isEmpty
            && summary == Self.filesSummary
        kind = real ?? (recovered ? .files : stored)
    }
}
