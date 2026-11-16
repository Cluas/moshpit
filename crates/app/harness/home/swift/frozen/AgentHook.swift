import Foundation

struct AgentHook: Equatable, Sendable {
    var state: String?
    var agent: String?
    var since: Date?
    /// What is actually RUNNING in the pane right now — polled fresh every
    /// round, unlike the discovery snapshot's copy, which can be minutes stale.
    /// The monitor uses it to spot stamps whose agent has since died: hooks
    /// only ever write the options, and a killed agent fires no final hook, so
    /// `attention` freezes at the moment of death unless something asks what
    /// the pane is doing NOW.
    var command: String?
    var title: String?
}
