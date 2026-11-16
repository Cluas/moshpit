import Foundation
import Testing
@testable import Moshpit

/// Row titles say each thing once — the tree fix for herdr's default names,
/// where every tab is named its own number and every workspace its cwd.
@Suite("Tree display titles")
struct TreeDisplayTitleTests {

    @Test("A named window keeps the index: name form")
    func namedWindow() {
        let window = WindowInfo(id: "w1:t2", sessionId: "w1", name: "logs", index: 2)
        #expect(window.displayTitle(Multiplexer.tmux.words) == "2: logs")
        #expect(window.displayTitle(Multiplexer.herdr.words) == "2: logs")
    }

    @Test("herdr's default tab name — its own number — reads as 'Tab N', not '1: 1'")
    func numberNamedWindow() {
        let window = WindowInfo(id: "w1:t1", sessionId: "w1", name: "1", index: 1)
        #expect(window.displayTitle(Multiplexer.herdr.words) == "Tab 1")
        #expect(window.displayTitle(Multiplexer.tmux.words) == "Window 1")
    }

    @Test("An empty name gets the same floor")
    func emptyNamedWindow() {
        let window = WindowInfo(id: "w1:t3", sessionId: "w1", name: "", index: 3)
        #expect(window.displayTitle(Multiplexer.herdr.words) == "Tab 3")
    }

    @Test("Duplicate session names carry their id; unique ones stay bare")
    func sessionDisambiguation() {
        var snap = TmuxSnapshot()
        snap.sessions["w1"] = SessionInfo(id: "w1", name: "~")
        snap.sessions["w2"] = SessionInfo(id: "w2", name: "~")
        snap.sessions["w3"] = SessionInfo(id: "w3", name: "moshpit")
        #expect(snap.sessionDisplayName(snap.sessions["w1"]!) == "~ · w1")
        #expect(snap.sessionDisplayName(snap.sessions["w2"]!) == "~ · w2")
        #expect(snap.sessionDisplayName(snap.sessions["w3"]!) == "moshpit")
    }
}
