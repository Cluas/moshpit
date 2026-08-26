import Foundation
import SwiftUI
import Testing
@testable import Moshpit

/// How tall the lock-screen Live Activity actually is.
///
/// iOS gives a Live Activity a fixed slice of the Lock Screen and clips whatever
/// does not fit. It reports nothing: no warning, no log, no callback. The only
/// evidence is a photograph of a phone with the card sliced off at both ends —
/// which is how this was found, after the layout had been "budgeted" twice in
/// comments by eye and shipped over-height anyway.
///
/// So the budget stops being a comment and becomes an assertion. `ImageRenderer`
/// lays the real view out at the real width and reports the height it wanted.
@MainActor
@Suite("Lock screen activity height")
struct LockScreenHeightTests {

    /// What iOS gives the card, measured off the failing screenshot rather than
    /// taken from a document: on an iPhone 16 Pro (402pt wide) the visible card
    /// spanned ~158pt before the top and bottom were cut. Apple's guidance for
    /// the Lock Screen presentation is the same 160.
    static let budget: CGFloat = 160

    /// The card runs nearly the full screen width, inset a little each side.
    static let width: CGFloat = 360

    /// A distinct pane per call, so no two fixtures collide.
    ///
    /// The first version of this built `id` from the agent's NAME, and a test
    /// with two agents both called "claude" — the realistic case, two Claude
    /// panes — handed `ForEach` two identical ids and got a height that matched
    /// no layout at all (220pt where the same shape measures 152). Real ids are
    /// "<connectionUUID>:<paneId>" and cannot collide; the fixture could, and a
    /// fixture that can collide makes every number in this file suspect.
    private static let paneCounter = Counter()
    final class Counter: @unchecked Sendable {
        private var n = 0
        func next() -> Int { n += 1; return n }
    }

    static func agent(_ name: String, state: AgentActivityAttributes.AgentState,
                      detail: String? = nil) -> AgentActivityAttributes.Agent {
        let pane = "%\(paneCounter.next())"
        return AgentActivityAttributes.Agent(
            id: "\(UUID().uuidString):\(pane)", connectionId: UUID().uuidString, paneId: pane,
            command: name,
            // The real thing from the failing screenshot, not "host · 1:pane" —
            // a location that renders short would quietly measure a card the
            // user never sees.
            location: "work · 0 · 5: oci",
            detail: detail, state: state,
            startedAt: Date().addingTimeInterval(-90))
    }

    static func height(agents: [AgentActivityAttributes.Agent]) -> CGFloat {
        let state = AgentActivityAttributes.ContentState(
            agents: agents,
            workingCount: agents.filter { $0.state == .working }.count,
            attentionCount: agents.filter { $0.state == .attention }.count,
            headlineDeepLink: nil)
        let renderer = ImageRenderer(
            content: LockScreenView(state: state).frame(width: width))
        renderer.scale = 1
        return renderer.uiImage?.size.height ?? .infinity
    }

    @Test("more agents than fit do not make the card taller")
    func growthIsBounded() {
        // The assertion that matters most, and the one a budget check alone
        // cannot make. Three separate "fits the budget" tests can all pass on
        // numbers that merely happen to land under 160 while the layout is
        // still unbounded — the next agent, or the next slightly taller row,
        // walks it back over the edge in silence.
        //
        // Equal heights prove the cap is doing the work.
        var agents = [Self.agent("claude", state: .attention,
                                 detail: "Claude is waiting for your input")]
        for i in 1...19 { agents.append(Self.agent("agent-\(i)", state: .working)) }

        // Compared AT the cap, taken from the view rather than written here: the
        // first version of this compared two agents against ten, which differed
        // for the honest reason that two is below the cap. Reading the constant
        // means the assertion survives someone changing it.
        let atCap = 1 + LockScreenView.maxTrailingRows
        let full = Self.height(agents: Array(agents.prefix(atCap)))
        let many = Self.height(agents: agents)
        #expect(full == many,
                "\(atCap) agents render \(Int(full))pt and twenty render \(Int(many))pt — the row count is not capped")
        #expect(many <= Self.budget)
    }

    @Test("the headline agent still says what it is asking")
    func headlineKeepsItsDetail() {
        // What replaced the controls test. The card's job is now purely to tell
        // you — which agent, and what it wants — so the detail line is the thing
        // that must not get optimised away for height. It is also the line that
        // was actually falling off the clipped card, while the buttons that
        // caused the clipping stayed on screen.
        let withDetail = Self.height(agents: [
            Self.agent("claude", state: .attention,
                       detail: "Bash: rm -rf build — may I run this?"),
        ])
        let without = Self.height(agents: [Self.agent("claude", state: .attention)])
        #expect(withDetail > without,
                "the detail line is not being rendered (\(Int(withDetail))pt vs \(Int(without))pt)")
        #expect(withDetail <= Self.budget)
    }

    @Test("one agent asking for approval fits the card")
    func oneAgentFits() {
        // The commonest case by far, and the one the whole feature exists for.
        let h = Self.height(agents: [
            Self.agent("claude", state: .attention, detail: "Claude is waiting for your input"),
        ])
        #expect(h <= Self.budget,
                "a single prompt renders \(Int(h))pt into a \(Int(Self.budget))pt card — iOS clips the difference silently")
    }

    @Test("three agents still fit, because that is what a real phone had")
    func threeAgentsFit() {
        // The reported state: one asking, two more running. The card came back
        // clipped at BOTH ends, which is what an over-height activity looks
        // like — the system centres what it cannot fit. Measured 367pt then;
        // 152 now, most of it won by removing the buttons rather than by
        // squeezing the text that tells you what is going on.
        let h = Self.height(agents: [
            Self.agent("claude", state: .attention, detail: "Claude is waiting for your input"),
            Self.agent("claude", state: .attention),
            Self.agent("codex", state: .working),
        ])
        #expect(h <= Self.budget,
                "three agents render \(Int(h))pt into a \(Int(Self.budget))pt card")
    }

    @Test("the card does not grow without bound as agents pile up")
    func manyAgentsAreCapped() {
        // Ten agents is not a design target, but "renders 400pt and gets sliced"
        // is not a behaviour either. Whatever the layout does with more agents
        // than fit, it has to stay inside the card.
        let many = (1...10).map { Self.agent("agent-\($0)", state: .working) }
        let h = Self.height(agents: many)
        #expect(h <= Self.budget, "ten agents render \(Int(h))pt")
    }

    @Test("the measurement itself is not vacuous")
    func rendererActuallyRenders() {
        // If ImageRenderer returned nil or zero, every assertion above would
        // pass while measuring nothing at all.
        let h = Self.height(agents: [Self.agent("claude", state: .attention)])
        #expect(h > 20, "the renderer produced \(h)pt — it is not laying the view out")
        #expect(h.isFinite)
    }
}
