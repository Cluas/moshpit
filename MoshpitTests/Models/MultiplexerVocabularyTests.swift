import Foundation
import Testing
@testable import Moshpit

/// The sheets are generic over the controller, so the words and keys they
/// print come from here. The keys are the part that must not drift.
@Suite("Multiplexer vocabulary")
struct MultiplexerVocabularyTests {

    @Test("Each multiplexer names its own levels")
    func nouns() {
        #expect(Multiplexer.tmux.vocabulary.session == "Session")
        #expect(Multiplexer.tmux.vocabulary.window == "Window")
        #expect(Multiplexer.herdr.vocabulary.session == "Workspace")
        #expect(Multiplexer.herdr.vocabulary.window == "Tab")
    }

    /// The bug this whole type exists for: the Select Pane sheet printed
    /// tmux's `⌃ b q` for every connection. In tmux that lists panes; in herdr
    /// it DETACHES. The app was telling herdr users to press the key that
    /// drops their session.
    @Test("herdr never advertises the detach key as a pane shortcut")
    func detachKeyIsNeverSuggested() {
        let herdr = Multiplexer.herdr.vocabulary
        let allKeys = herdr.sessionKeys + herdr.windowKeys + herdr.paneKeys
        #expect(!allKeys.contains("b q"))
        // tmux's own hint is still correct for tmux.
        #expect(Multiplexer.tmux.vocabulary.paneKeys.contains("b q"))
    }

    @Test("herdr's hints match its actual bindings")
    func herdrKeysMatchHerdr() {
        let herdr = Multiplexer.herdr.vocabulary
        // Per herdr's keyboard docs: workspaces `prefix+w`, tabs step with
        // `prefix+n` / `prefix+p`, split right `prefix+v`.
        #expect(herdr.sessionKeys.contains("b w"))
        #expect(herdr.windowKeys.contains("b n/p"))
        #expect(herdr.paneKeys.contains("b v"))
        // tmux's number-jump doesn't exist in herdr.
        #expect(!herdr.windowKeys.contains("1-9"))
    }

    @Test("tmux keeps exactly the hints it had before")
    func tmuxUnchanged() {
        let tmux = Multiplexer.tmux.vocabulary
        #expect(tmux.sessionKeys == ["⌃", "b s"])
        #expect(tmux.windowKeys == ["⌃", "1-9"])
        #expect(tmux.paneKeys == ["⌃", "b q"])
    }

    /// "Kill Workspace" is a tmux idiom stapled to a herdr noun — herdr's own
    /// CLI is `workspace close` / `tab close` / `pane close`.
    @Test("Each multiplexer destroys things in its own words")
    func killVerbs() {
        #expect(Multiplexer.tmux.vocabulary.killVerb == "Kill")
        #expect(Multiplexer.herdr.vocabulary.killVerb == "Close")
    }

    /// The icons are part of the vocabulary for the same reason the nouns
    /// are: `macwindow` — literal window chrome — next to the word "Tab" tells
    /// a herdr user the app was built for something else.
    @Test("Each multiplexer draws its own levels")
    func icons() {
        #expect(Multiplexer.tmux.vocabulary.windowIcon == "macwindow")
        #expect(Multiplexer.herdr.vocabulary.windowIcon != "macwindow")
        #expect(Multiplexer.herdr.vocabulary.sessionIcon != Multiplexer.tmux.vocabulary.sessionIcon)
        // Every icon has to be a real symbol name, not an empty string.
        for vocab in [MultiplexerVocabulary.tmux, .herdr] {
            #expect(!vocab.sessionIcon.isEmpty)
            #expect(!vocab.windowIcon.isEmpty)
        }
    }

    @Test("Plain-shell connections fall back to something printable")
    func noneHasVocabulary() {
        #expect(!Multiplexer.none.vocabulary.session.isEmpty)
    }
}

/// Row titles say each thing once — the tree fix for herdr's default names,
/// where every tab is named its own number and every workspace its cwd.
@Suite("Tree display titles")
struct TreeDisplayTitleTests {

    @Test("A named window keeps the index: name form")
    func namedWindow() {
        let window = WindowInfo(id: "w1:t2", sessionId: "w1", name: "logs", index: 2)
        #expect(window.displayTitle(.tmux) == "2: logs")
        #expect(window.displayTitle(.herdr) == "2: logs")
    }

    @Test("herdr's default tab name — its own number — reads as 'Tab N', not '1: 1'")
    func numberNamedWindow() {
        let window = WindowInfo(id: "w1:t1", sessionId: "w1", name: "1", index: 1)
        #expect(window.displayTitle(.herdr) == "Tab 1")
        #expect(window.displayTitle(.tmux) == "Window 1")
    }

    @Test("An empty name gets the same floor")
    func emptyNamedWindow() {
        let window = WindowInfo(id: "w1:t3", sessionId: "w1", name: "", index: 3)
        #expect(window.displayTitle(.herdr) == "Tab 3")
    }

    @Test("Duplicate session names carry their id; unique ones stay bare")
    func sessionDisambiguation() {
        var snap = TmuxSnapshot()
        snap.sessions["w1"] = SessionInfo(id: "w1", name: "~")
        snap.sessions["w2"] = SessionInfo(id: "w2", name: "~")
        snap.sessions["w3"] = SessionInfo(id: "w3", name: "moshi")
        #expect(snap.sessionDisplayName(snap.sessions["w1"]!) == "~ · w1")
        #expect(snap.sessionDisplayName(snap.sessions["w2"]!) == "~ · w2")
        #expect(snap.sessionDisplayName(snap.sessions["w3"]!) == "moshi")
    }
}
