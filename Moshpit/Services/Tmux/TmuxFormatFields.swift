// MARK: - tmux format fields

extension String {
    /// The fields of one line of tmux `-F` format output, cut at every
    /// `separator` BYTE.
    ///
    /// Same arguments as `split(separator:maxSplits:omittingEmptySubsequences:)`,
    /// and for ASCII text the same fields — but that split runs through
    /// String's Character view, where the text AROUND a separator decides
    /// whether it is one. A space or `|` followed by a combining mark, a ZWJ,
    /// a variation selector or a spacing mark is ONE Character together with
    /// it (and so is one preceded by a prepend mark), so a field starting like
    /// that fused with the field before it: a session, window or pane named
    /// "\u{301}x" left its `list-*` row one field short, and the row was
    /// skipped — gone from the snapshot as if tmux had never listed it. Every
    /// separator in these formats is ASCII, so the line is cut on its UTF-8
    /// bytes instead, the rule the control-protocol parser follows too (see
    /// "Protocol bytes" in TmuxControlClient.swift).
    ///
    /// A cut at an ASCII byte of valid UTF-8 leaves valid UTF-8 on both
    /// sides, so every field decodes to exactly the text tmux printed. With
    /// `maxSplits`, the last field is the rest of the line, separators and
    /// all — which is how a format that puts the name last keeps
    /// "my project" whole.
    ///
    /// - Precondition: `separator` is ASCII.
    func tmuxFields(separator: Unicode.Scalar, maxSplits: Int = .max,
                    omittingEmptySubsequences: Bool = true) -> [String] {
        utf8.split(separator: UInt8(ascii: separator), maxSplits: maxSplits,
                   omittingEmptySubsequences: omittingEmptySubsequences)
            .map { String(decoding: $0, as: UTF8.self) }
    }
}
