import Foundation
import Testing
import UIKit
@testable import Moshpit

/// mosh's predictive echo, as the app hands it to the terminal core: one
/// ``MoshPrediction`` per transport — its mode, its trail and the round
/// trip — shared by the session's coordinator and its terminal. The guesses
/// are the core's (crates/term/src/predict.rs, and the core view's
/// `predictionsShowOverTheFrame`); the Swift engine that drew into
/// SwiftTerm's cells went with SwiftTerm (batch B).
@Suite("MoshPrediction")
@MainActor
struct MoshPredictionTests {
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private func terminal() -> (CoreTerminalView, SwiftTerminalView.Coordinator) {
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 20, rows: 3))
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(surface)
        return (surface as! CoreTerminalView, coordinator)
    }

    @Test("the round trip measured after attaching is the one the terminal guesses with")
    func roundTripIsShared() {
        let (view, coordinator) = terminal()
        let engine = MoshPrediction(mode: .adaptive, trail: true)
        coordinator.attachPrediction(engine)
        #expect(view.prediction === engine)
        #expect(engine.srttMs == 0, "not measured yet: counts as slow")
        coordinator.prediction?.srttMs = 320
        #expect(view.prediction?.srttMs == 320)
        #expect(view.prediction?.mode == .adaptive)
        #expect(view.prediction?.trail == true)
    }

    @Test("with no prediction attached a keystroke guesses nothing")
    func noPredictionNoGuess() {
        let (view, coordinator) = terminal()
        #expect(coordinator.predict(Data("a".utf8)) == nil)
        coordinator.attachPrediction(MoshPrediction(mode: .always, trail: false))
        coordinator.attachPrediction(nil)
        #expect(view.prediction == nil)
        #expect(coordinator.predict(Data("a".utf8)) == nil)
    }

    /// B531: "adaptive" (and nothing, and a value this build doesn't know)
    /// is as Settings says; a connection pinning Adaptive is
    /// "pinnedAdaptive". The Rust session's prediction reads it so.
    @Test("a connection's own predictive echo, or Settings' (B531)")
    func predictOverride() {
        var connection = ServerConnection()
        let cases: [(String?, PredictMode?)] = [
            (nil, nil), ("adaptive", nil), ("turbo", nil), ("pinnedAdaptive", .adaptive),
            ("always", .always), ("never", .never), ("experimental", .experimental),
        ]
        for (raw, own) in cases {
            connection.predictModeRaw = raw
            #expect(connection.predictOverride == own, "\(raw ?? "nil")")
        }
    }
}
