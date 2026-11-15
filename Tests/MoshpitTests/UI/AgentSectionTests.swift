import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit
import MoshpitKit

/// The shared meaning of a state — one mapping for the sheets, the Agents
/// section and the Island.
@Suite("Agent signal")
struct AgentSignalTests {

    @Test("Hook strings map to signals; anything else is silence")
    func mapping() {
        #expect(AgentSignal("attention") == .attention)
        #expect(AgentSignal("working") == .working)
        #expect(AgentSignal("done") == .done)
        #expect(AgentSignal("idle") == nil)
        #expect(AgentSignal(nil) == nil)
        #expect(AgentSignal("something new") == nil)
    }

    @Test("Ranking puts the human-blocking state first")
    func ranking() {
        #expect(AgentSignal.attention.rank < AgentSignal.working.rank)
        #expect(AgentSignal.working.rank < AgentSignal.done.rank)
    }

    /// Aggregate dots must keep showing exactly what they shipped showing:
    /// attention beats working, and `done` doesn't light a window up.
    @Test("Aggregate preserves the sheets' existing behaviour")
    func aggregate() {
        #expect(AgentSignal.aggregate([.working, .attention]) == .attention)
        #expect(AgentSignal.aggregate([.working, nil]) == .working)
        #expect(AgentSignal.aggregate([.done]) == nil)
        #expect(AgentSignal.aggregate([nil, nil]) == nil)
        #expect(AgentSignal.aggregate([]) == nil)
    }

    @Test("Colours are the shared palette's, so a dot means one thing everywhere")
    func colours() {
        #expect(AgentSignal.attention.color == AgentPalette.attention)
        #expect(AgentSignal.working.color == AgentPalette.working)
        #expect(AgentSignal.done.color == AgentPalette.done)
        // The palette's amber IS the app's warn tone — pinned so the two
        // can't drift apart silently.
        #expect(AgentPalette.attention == Ink.warn)
        #expect(AgentPalette.done == Ink.success)
    }

    @Test("The widgets' state colours read on a light lock screen and stay as they were on a dark one (R18 D)")
    func stateColoursAdapt() {
        var light = EnvironmentValues()
        light.colorScheme = .light
        var dark = EnvironmentValues()
        dark.colorScheme = .dark
        /// `color` resolved in `environment`, as 8-bit sRGB.
        func bytes(_ color: Color, _ environment: EnvironmentValues) -> [Int] {
            let resolved = color.resolve(in: environment)
            var (r, g, b, a): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
            UIColor(cgColor: resolved.cgColor).getRed(&r, green: &g, blue: &b, alpha: &a)
            return [r, g, b].map { Int(($0 * 255).rounded()) }
        }
        /// WCAG's contrast of `rgb` on white.
        func onWhite(_ rgb: [Int]) -> Double {
            func channel(_ byte: Int) -> Double {
                let c = Double(byte) / 255
                return c <= 0.040_45 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
            }
            let luminance = 0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
            return 1.05 / (luminance + 0.05)
        }
        for (name, adaptive, onDark, onLight) in [
            ("working", AgentPalette.workingAdaptive, AgentPalette.working, AgentPalette.workingOnLight),
            ("attention", AgentPalette.attentionAdaptive, AgentPalette.attention, AgentPalette.attentionOnLight),
            ("done", AgentPalette.doneAdaptive, AgentPalette.done, AgentPalette.doneOnLight),
        ] {
            #expect(bytes(adaptive, dark) == bytes(onDark, dark), "\(name): the dark side is the island's")
            #expect(bytes(adaptive, light) == bytes(onLight, light), "\(name): the light side")
            #expect(onWhite(bytes(adaptive, light)) >= 4.5, "\(name) reads on white")
        }
    }

    /// The regression this palette exists to prevent: "working" once followed
    /// the user-customizable theme accent, so an amber-ish theme made working
    /// and needs-you the same colour — and the island (fixed teal) disagreed
    /// with the app on every theme.
    @Test("State colours are fixed — none of them is the theme accent")
    func themeIndependent() {
        for signal in AgentSignal.allCases {
            #expect(signal.color != Ink.accent)
        }
    }
}
