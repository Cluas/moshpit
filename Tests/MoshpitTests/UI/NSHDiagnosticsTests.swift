import Foundation
import Testing
import UIKit
@testable import Moshpit

/// B366: a page that showed its bar and nothing else on Canary 116, fixed
/// by a restart, and nothing anywhere said why. What the app now keeps of
/// an NSH screen that fails: the core's warnings and errors in `Log.ui`,
/// refused posts once per op, the screen's error boundary's events, a look
/// at each screen a moment after it runs, all of it in a ring that outlives
/// the launch; and a page's screen that starts from the page's state
/// whether or not `onAppear` came first.
@Suite("NSH diagnostics (B366)")
@MainActor
struct NSHDiagnosticsTests {

    // MARK: The ring that outlives a launch

    private static func trailURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("nsh-trail-\(UUID().uuidString).json")
    }

    @Test("The trail keeps the newest lines, each with the launch that wrote it")
    func trailKeepsTheNewest() {
        let url = Self.trailURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let trail = NSHTrail(url: url, capacity: 3, launch: "a")
        let start = Date(timeIntervalSince1970: 1_800_000_000)
        for n in 0..<5 {
            trail.record("line \(n)", now: start.addingTimeInterval(Double(n)))
        }
        let lines = trail.read()
        #expect(lines.map(\.text) == ["line 2", "line 3", "line 4"])
        #expect(lines.allSatisfy { $0.launch == "a" })
    }

    @Test("A later launch reads what the one before wrote, and not its own")
    func earlierLaunchesOnly() {
        let url = Self.trailURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let now = Date()
        let before = NSHTrail(url: url, launch: "before")
        before.record("NSH settings: the page posted it nothing", now: now.addingTimeInterval(-60))
        before.record("too old", now: now.addingTimeInterval(-3 * 86_400))
        // The restart.
        let after = NSHTrail(url: url, launch: "after")
        after.record("this launch's own", now: now)
        let earlier = after.earlier(since: now.addingTimeInterval(-NSHDiagnostics.earlierWindow))
        #expect(earlier.map(\.text) == ["NSH settings: the page posted it nothing"])
        #expect(after.read().count == 3, "every line is kept")
    }

    @Test("No App Group, no trail: nothing is written and nothing breaks")
    func noTrail() {
        let trail = NSHTrail(url: nil)
        trail.record("nowhere")
        #expect(trail.read().isEmpty)
    }

    // MARK: Refused posts

    @Test("A refused op is logged the first time only")
    func refusedOnce() {
        var log = NSHPostLog()
        // #expect can't call a mutating member on its captured value; ask first.
        let firstSettings = log.first("settings")
        let againSettings = log.first("settings")
        let firstReveal = log.first("reveal")
        #expect(firstSettings)
        #expect(!againSettings)
        #expect(firstReveal, "another op is its own")
    }

    @Test("A post's op, from a message or its JSON")
    func postOps() {
        #expect(NSHPostLog.op(of: ["op": "home", "x": 1]) == "home")
        #expect(NSHPostLog.op(of: ["x": 1]) == "?")
        #expect(NSHPostLog.op(ofJSON: #"{"op":"trees","trees":[]}"#) == "trees")
        #expect(NSHPostLog.op(ofJSON: "not json") == "?")
        #expect(NSHPostLog.op(ofJSON: "[1]") == "?")
    }

    @Test("A message JSON can't write names the values that can't be written")
    func unwritableKeys() {
        let missing: String? = nil
        let message: [String: Any] = [
            "op": "settings",
            "fontSize": Double.nan,
            "voiceModel": missing as Any,
            "when": Date(),
            "fine": ["a": 1],
        ]
        #expect(!JSONSerialization.isValidJSONObject(message))
        // A nil Optional boxed in Any bridges to NSNull, which JSON writes as
        // null; only the NaN and the Date can't be written.
        #expect(NSHPostLog.unwritable(message) == ["fontSize", "when"])
        #expect(NSHPostLog.unwritable(["op": "home", "n": 1]).isEmpty)
    }

    // MARK: The error boundary's events

    @Test("The boundary's failed and retry events are read; anything else is the page's")
    func boundaryEvents() {
        let failed = NSHBoundaryEvent(["ev": "failed", "component": "HomeScreen", "message": "index out of range"])
        #expect(failed == .failed(component: "HomeScreen", message: "index out of range"))
        let bare = NSHBoundaryEvent(["ev": "failed"])
        #expect(bare == .failed(component: "?", message: ""))
        #expect(NSHBoundaryEvent(["ev": "retry"]) == .retry)
        #expect(NSHBoundaryEvent(["ev": "back"]) == nil)
        #expect(NSHBoundaryEvent(["op": "retry"]) == nil)
    }

    @Test("A surface takes its boundary's events and hands the page the rest")
    func surfaceRoutesBoundaryEvents() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .keyPicker)
        defer { done() }
        var retries = 0
        surface.onRetry = { retries += 1 }
        surface.receive(["ev": "failed", "component": "KeyPickerScreen", "message": "boom"])
        surface.receive(["ev": "retry"])
        surface.receive(["ev": "cancel"])
        #expect(retries == 1)
        let event = try #require(await heard.next())
        #expect(event["ev"] as? String == "cancel", "the page hears only its own")
    }

    // MARK: The core's log

    @Test("A forwarded record of the core reads as its level, target and message")
    func rustLines() {
        #expect(NSHDiagnostics.rustLine(level: 1, target: "js", message: "HomeScreen failed: boom")
            == "core [js] error: HomeScreen failed: boom")
        #expect(NSHDiagnostics.rustLine(level: 2, target: "moshpit", message: "home: no channel to hear the host on")
            == "core [moshpit] warning: home: no channel to hear the host on")
    }

    // MARK: The look after a screen runs

    @Test("Each way a page ends up as its bar alone reads differently")
    func verdicts() {
        typealias C = NSHWatch.Counts
        #expect(NSHWatch.verdict(C(heard: true, posted: 2, applied: 2, lost: 0)) == .healthy)
        #expect(NSHWatch.verdict(C(heard: false, posted: 1, applied: 0, lost: 0)) == .neverListened)
        #expect(NSHWatch.verdict(C(heard: true, posted: 0, applied: 0, lost: 0)) == .nothingPosted)
        #expect(NSHWatch.verdict(C(heard: true, posted: 1, applied: 1, lost: 3)) == .stoppedListening)
        #expect(NSHWatch.verdict(C(heard: true, posted: 3, applied: 1, lost: 0)) == .behind)
        let line = NSHWatch.line(screen: "settings", counts: C(heard: true, posted: 0, applied: 0, lost: 0))
        #expect(line == "NSH settings: the page posted it nothing 3 s after it ran (heard true, posted 0, applied 0, lost 0)")
    }

    @Test("A running screen's counts come from the core: refused posts aren't counted")
    func countsFromTheCore() throws {
        let (surface, _, done) = try SettingsNSHTests.surface(running: .keyPicker)
        defer { done() }
        #expect(surface.screenName == "key-picker")
        let start = try #require(surface.counts)
        #expect(start.posted == 0)
        #expect(surface.post(KeyPickerModel.message(KeyPickerSnapshot(keys: [], selected: nil))))
        #expect(!surface.post(["op": "nope"]), "refused")
        #expect(!surface.post(["op": "keys", "bad": Double.nan]), "not JSON")
        let after = try #require(surface.counts)
        #expect(after.posted == 1)
        #expect(after.lost == 0)
    }

    // MARK: A screen starts from the page's state

    @Test("Attached with the page's state, the screen is posted it even before onAppear")
    func attachFromCurrentState() throws {
        let (surface, _, done) = try SettingsNSHTests.surface(running: .keyPicker)
        defer { done() }
        let model = KeyPickerModel()
        model.attach(surface)
        let before = try #require(surface.counts)
        #expect(before.posted == 0, "the old attach had nothing to post before onAppear")
        model.attach(surface, current: KeyPickerSnapshot(keys: [], selected: nil))
        let after = try #require(surface.counts)
        #expect(after.posted == 1)
        #expect(model.snapshot == KeyPickerSnapshot(keys: [], selected: nil))
    }

    @Test("A state the model already has is newer than the page's captured one, and stays")
    func attachKeepsTheModelsState() throws {
        let (surface, _, done) = try SettingsNSHTests.surface(running: .keyPicker)
        defer { done() }
        let model = KeyPickerModel()
        let newer = KeyPickerSnapshot(keys: [], selected: UUID())
        model.update(newer)
        model.attach(surface, current: KeyPickerSnapshot(keys: [], selected: nil))
        #expect(model.snapshot == newer)
        let counts = try #require(surface.counts)
        #expect(counts.posted == 1, "the model's state, posted once")
    }

    @Test("Home's model attached with the record posts it, and keeps it for the next surface")
    func homeAttachFromCurrentRecord() throws {
        let (surface, _, done) = try SettingsNSHTests.surface(running: .home)
        defer { done() }
        let model = HomeNSHModel()
        let record = HomeRecord(message: #"{"connections":[],"op":"home","version":"v1"}"#)
        model.attach(surface, current: record)
        let counts = try #require(surface.counts)
        #expect(counts.posted == 1)
        #expect(model.record == record)
    }
}
