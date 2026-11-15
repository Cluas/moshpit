import Foundation

extension DictationTranscript {
    /// Whitespace and sentence punctuation — what a final pass adds or
    /// moves without changing what is said. `-`, `/`, `_`, `~` and the like
    /// are not here: in a command they are the command.
    private static let unsaid = CharacterSet.whitespacesAndNewlines
        .union(CharacterSet(charactersIn: ".,!?;:'\"…，。！？；：、“”‘’「」"))

    /// `a` and `b` say the same, ignoring case, spacing and sentence
    /// punctuation ("git status" and "Git status." do; "rm -rf /tmp" and
    /// "rm -rf /" don't) (B676).
    static func sameWords(_ a: String, _ b: String) -> Bool {
        func words(_ text: String) -> String {
            String(String.UnicodeScalarView(text.lowercased().unicodeScalars.filter { !unsaid.contains($0) }))
        }
        return words(a) == words(b)
    }
}
