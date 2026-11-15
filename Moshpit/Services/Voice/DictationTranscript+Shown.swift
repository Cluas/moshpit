import Foundation

extension DictationTranscript {
    /// The overlay's two pieces — the settled words, then the guess drawn
    /// quieter — such that putting them side by side gives exactly what
    /// ``join(_:_:)`` gives, i.e. what Insert types (B673): the guess
    /// carries whatever space the join puts before it.
    static func shownPieces(_ finalized: String, _ volatile: String) -> (finalized: String, volatile: String) {
        let joined = join(finalized, volatile)
        return (finalized, String(joined.dropFirst(finalized.count)))
    }
}
