import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit
import MoshpitKit

/// The Agents section exists to answer "who is waiting on me" at a glance, so
/// the ordering IS the feature. These drive the pure builder rather than the
/// view, which is why it was split out.
@Suite("Agents section")
@MainActor
struct AgentSectionTests {

    private func snapshot() -> TmuxSnapshot {
        var snap = TmuxSnapshot()
        snap.sessions["w1"] = SessionInfo(id: "w1", name: "moshpit", isAttached: true)
        snap.sessions["w2"] = SessionInfo(id: "w2", name: "sample-app")
        snap.windows["w1:t1"] = WindowInfo(id: "w1:t1", sessionId: "w1", name: "fix-scroll", index: 1)
        snap.windows["w2:t1"] = WindowInfo(id: "w2:t1", sessionId: "w2", name: "main", index: 1)
        snap.panes["w1:p1"] = PaneInfo(id: "w1:p1", windowId: "w1:t1", index: 1, command: "zsh")
        snap.panes["w1:p2"] = PaneInfo(id: "w1:p2", windowId: "w1:t1", index: 2)
        snap.panes["w2:p1"] = PaneInfo(id: "w2:p1", windowId: "w2:t1", index: 1)
        snap.activePaneId = "w1:p1"
        return snap
    }

    @Test("Whoever is waiting on a human comes first")
    func attentionSortsFirst() {
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: [
                "w1:p1": AgentHook(state: "working", agent: "Codex"),
                "w2:p1": AgentHook(state: "attention", agent: "Claude Code"),
                "w1:p2": AgentHook(state: "done", agent: "Droid"),
            ])
        #expect(entries.map(\.signal) == [.attention, .working, .done])
        #expect(entries.first?.name == "Claude Code")
    }

    @Test("Unknown-state panes never appear — nothing lights up without cause")
    func quietPanesAreOmitted() {
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: [
                "w1:p1": AgentHook(state: nil, agent: "Claude Code"),
                "w1:p2": AgentHook(state: "working", agent: "Codex"),
            ])
        #expect(entries.map(\.paneId) == ["w1:p2"])
    }

    /// herdr 0.8+ says `idle` by name for an agent it recognises. That agent
    /// is real — you can hand it work — so it gets a quiet row at the bottom,
    /// not silence. An idle stamp with NO name is any shell; those stay out.
    @Test("A named idle agent is a row — last, unlit, with no duration")
    func namedIdleAppearsLast() {
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: [
                "w1:p1": AgentHook(state: "idle", agent: "claude"),
                "w1:p2": AgentHook(state: "idle"),
                "w2:p1": AgentHook(state: "done", agent: "Codex"),
            ],
            since: ["w1:p1": Date(timeIntervalSinceNow: -300)])
        #expect(entries.map(\.paneId) == ["w2:p1", "w1:p1"])
        #expect(entries.last?.signal == nil)
        #expect(entries.last?.since == nil)
    }

    @Test("The hook's own stamp beats the monitor's first-seen clock")
    func sincePrecedence() {
        let stamped = Date(timeIntervalSince1970: 1000)
        let observed = Date(timeIntervalSince1970: 2000)
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working", agent: "claude", since: stamped)],
            since: ["w1:p1": observed])
        #expect(entries.first?.since == stamped)

        let unstamped = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working", agent: "claude")],
            since: ["w1:p1": observed])
        #expect(unstamped.first?.since == observed)
    }

    /// Minutes at the finest: the poll behind the number is up to 8s coarse,
    /// so seconds would be false precision (the design doc's own argument).
    @Test("Elapsed labels: now / minutes / hours, never seconds")
    func elapsedLabels() {
        let now = Date(timeIntervalSince1970: 10_000)
        #expect(ConnectionCard.elapsedLabel(since: nil, now: now) == nil)
        #expect(ConnectionCard.elapsedLabel(since: now.addingTimeInterval(-30), now: now) == "now")
        #expect(ConnectionCard.elapsedLabel(since: now.addingTimeInterval(-134), now: now) == "2m")
        #expect(ConnectionCard.elapsedLabel(since: now.addingTimeInterval(-4_320), now: now) == "1h 12m")
    }

    @Test("Location reads session · window, which is what makes a row findable")
    func locationIsHumanReadable() {
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working", agent: "Claude Code")])
        #expect(entries.first?.location == "moshpit · fix-scroll")
    }

    /// herdr 0.7.3 reports no agent name for a pane it hasn't detected, and no
    /// command either — the row still has to say something useful.
    @Test("Name falls back to the command, then to a generic label")
    func nameFallsBack() {
        let fromCommand = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["w1:p1": AgentHook(state: "working")])
        #expect(fromCommand.first?.name == "zsh")

        let noCommand = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["w1:p2": AgentHook(state: "working")])
        #expect(noCommand.first?.name == "agent")
    }

    @Test("A pane the snapshot no longer knows about is dropped, not crashed on")
    func staleHookIgnored() {
        let entries = ConnectionCard.agentEntries(
            snapshot: snapshot(),
            hooks: ["ghost:p9": AgentHook(state: "attention", agent: "Ghost")])
        #expect(entries.isEmpty)
    }

    @Test("Order is stable across polls when two agents share a state")
    func orderIsStable() {
        let hooks = [
            "w2:p1": AgentHook(state: "working", agent: "Codex"),
            "w1:p1": AgentHook(state: "working", agent: "Codex"),
        ]
        let first = ConnectionCard.agentEntries(snapshot: snapshot(), hooks: hooks)
        let second = ConnectionCard.agentEntries(snapshot: snapshot(), hooks: hooks)
        // Dictionaries have no order; without the tiebreak the section would
        // reshuffle every 2 seconds.
        #expect(first.map(\.paneId) == second.map(\.paneId))
        #expect(first.map(\.paneId) == ["w1:p1", "w2:p1"])
    }
}

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
