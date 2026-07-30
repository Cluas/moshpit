import Foundation
import Testing
@testable import Ringdown

/// Vibe Island T1 control surface — the keystrokes each lock-screen verb sends
/// into the agent's tmux pane. These are tuned for Claude Code's permission
/// prompt (Enter accepts the highlighted "Yes", Esc cancels) and reused by both
/// the notification actions and the Live Activity buttons, so they must stay
/// exact.
@Suite("AgentAction keystrokes")
struct AgentControlTests {

    @Test("allow sends a bare Enter (accept the highlighted option)")
    func allow() {
        #expect(AgentAction.allow.bytes() == Data([0x0d]))
        // Text is irrelevant for allow/deny — they're fixed keystrokes.
        #expect(AgentAction.allow.bytes(text: "ignored") == Data([0x0d]))
    }

    @Test("deny sends Esc (cancel / decline)")
    func deny() {
        #expect(AgentAction.deny.bytes() == Data([0x1b]))
        #expect(AgentAction.deny.bytes(text: "ignored") == Data([0x1b]))
    }

    @Test("reply sends the typed answer terminated by Enter")
    func reply() {
        #expect(AgentAction.reply.bytes(text: "use option 2") == Data("use option 2\r".utf8))
        #expect(AgentAction.reply.bytes(text: "y") == Data("y\r".utf8))
    }

    @Test("an empty / missing reply still submits a lone Enter")
    func emptyReply() {
        #expect(AgentAction.reply.bytes(text: "") == Data([0x0d]))
        #expect(AgentAction.reply.bytes() == Data([0x0d]))
    }

    @Test("interrupt sends Ctrl-C (stop the running agent)")
    func interrupt() {
        #expect(AgentAction.interrupt.bytes() == Data([0x03]))
        #expect(AgentAction.interrupt.bytes(text: "ignored") == Data([0x03]))
    }

    @Test("raw values are the stable notification-action / intent identifiers")
    func rawValues() {
        #expect(AgentAction.allow.rawValue == "allow")
        #expect(AgentAction.deny.rawValue == "deny")
        #expect(AgentAction.reply.rawValue == "reply")
        #expect(AgentAction.interrupt.rawValue == "interrupt")
        #expect(AgentAction(rawValue: "allow") == .allow)
        #expect(AgentAction(rawValue: "interrupt") == .interrupt)
        #expect(AgentAction(rawValue: "bogus") == nil)
    }
}
