import Foundation

/// mosh's predictive local echo for one terminal, as the terminal core runs
/// it (crates/term/src/predict.rs, a port of mosh's
/// `Overlay::PredictionEngine`): which mode, whether unconfirmed guesses are
/// drawn dim, and the transport's round trip. The guesses themselves — their
/// epochs, their judging against the server's echo ack, the overlay laid
/// over the frames and never written into the grid the server diffs against
/// — are the core's.
///
/// One object per mosh transport, shared by the session's coordinator and
/// its terminal: the round trip is written as the transport measures it and
/// read at every keystroke and tick.
///
/// It was the Swift engine itself, drawing into SwiftTerm's cells; that
/// went with SwiftTerm (batch B). Main thread only.
final class MoshPrediction {
    let mode: PredictMode
    /// Dim unconfirmed guesses on a slow link (mosh's "flagging").
    let trail: Bool
    /// The transport's smoothed round trip, ms. 0 = not measured yet, which
    /// counts as slow (mosh starts its SRTT at one second).
    var srttMs: Double = 0

    init(mode: PredictMode, trail: Bool) {
        self.mode = mode
        self.trail = trail
    }
}
