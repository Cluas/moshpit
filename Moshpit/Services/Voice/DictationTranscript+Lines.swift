import Foundation

extension DictationTranscript {
    /// `text` with every line break, and the spaces around it, made one
    /// space (B672).
    ///
    /// Apple's dictation model writes a spoken "new line" as a real line
    /// break ("first line new line second line" → "First line.\nSecond
    /// line"). A pane that hasn't asked for bracketed pastes — a plain shell
    /// prompt, a REPL, older bash — takes a pasted line break as Return, so
    /// Insert, the button that promises to type and not run, ran the first
    /// half of the dictation. The overlay shows the transcript after this, so
    /// what is inserted is what was on screen.
    static func singleLine(_ text: String) -> String {
        guard text.rangeOfCharacter(from: .newlines) != nil else { return text }
        return text.replacingOccurrences(
            of: #"[ \t]*[\r\n\x{0B}\x{0C}\x{85}\x{2028}\x{2029}]+[ \t]*"#,
            with: " ", options: .regularExpression)
    }
}
