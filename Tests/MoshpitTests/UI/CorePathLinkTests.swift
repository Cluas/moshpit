import Foundation
import MoshpitFFI
import Testing
import UIKit
@testable import Moshpit

/// T3: host paths as links and OSC 7 on the core's view (TERMINAL-VIEW.md
/// V43, V44) — what the SwiftTerm path's PathLinkDetector and its
/// `hostCurrentDirectoryUpdate` do, so the switch can default on without
/// them regressing.
@Suite("Core view: host paths as links, OSC 7", .serialized)
@MainActor
struct CorePathLinkTests {
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    /// The core's view in a window (the pass runs only for one on screen),
    /// attached to a coordinator as the session's is.
    private func makeCore(cols: Int = 40, rows: Int = 6) -> (CoreTerminalView, SwiftTerminalView.Coordinator, UIWindow) {
        let surface = TerminalSurfaces.make(.core, font: Self.font, grid: TerminalGrid(cols: cols, rows: rows))
        let core = surface as! CoreTerminalView
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(surface)
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
        window.frame = CGRect(x: 0, y: 0, width: 600, height: 800)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(core)
        window.isHidden = false
        return (core, coordinator, window)
    }

    /// Waits (letting the resolver's tasks and the pass's timers run) until
    /// `done` holds.
    private func wait(_ timeout: TimeInterval = 5, _ done: () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return done()
    }

    /// What the host is asked, and what it has.
    final class Host: @unchecked Sendable {
        var asked: [String] = []
        var files: [String: String] = [:]
        func probe(_ batch: [TerminalPathScanner.Candidate]) -> [Int: PathProbe.Hit] {
            asked += batch.map(\.token)
            var hits: [Int: PathProbe.Hit] = [:]
            for (i, c) in batch.enumerated() {
                if let path = files[c.token] { hits[i] = PathProbe.Hit(path: path, isDirectory: false) }
            }
            return hits
        }
    }

    private func flags(_ core: CoreTerminalView, row: Int, col: Int) -> Int {
        guard row < core.rows.rows.count, col < core.rows.rows[row].cells.count else { return 0 }
        return Int(core.rows.rows[row].cells[col].flags)
    }

    @Test("a path the host confirmed is a link on the core's view, and opens the host's file")
    func confirmedPathIsALink() async throws {
        let (core, coordinator, window) = makeCore()
        defer { window.isHidden = true }
        let host = Host()
        host.files = ["out/a.png": "/r/out/a.png"]
        coordinator.pathLinks = RemotePathLinks(workingDirectory: { "/r" }, probe: { _, batch in host.probe(batch) })
        var opened: [String] = []
        coordinator.onOpenRemotePath = { opened.append($0) }
        coordinator.feed(data: Data("see out/a.png and out/b.png\r\n".utf8))

        let expected = RemotePathLink.url(for: "/r/out/a.png")
        #expect(await wait { core.link(at: (col: 4, row: 0)) == expected }, "never became a link")
        #expect(host.asked.contains("out/a.png"))
        #expect(host.asked.contains("out/b.png"))
        #expect(await wait { flags(core, row: 0, col: 4) & Int(MP_CELL_LINK) != 0 }, "not drawn as a link")
        #expect(flags(core, row: 0, col: 12) & Int(MP_CELL_LINK) != 0, "its last cell")
        #expect(flags(core, row: 0, col: 3) & Int(MP_CELL_LINK) == 0)
        #expect(flags(core, row: 0, col: 13) & Int(MP_CELL_LINK) == 0)
        #expect(core.link(at: (col: 18, row: 0)) == nil, "the host found no out/b.png")

        // The tap's intent, then the coordinator's open: the app's preview.
        let term = try #require(core.term)
        let tap = mp_term_tap(term, 0, 6)
        #expect(tap.kind == 2)
        let link = tap.link.map { p in defer { mp_string_free(p) }; return String(cString: p) }
        #expect(link == expected)
        coordinator.openLink(try #require(link))
        #expect(await wait { opened == ["/r/out/a.png"] })
    }

    @Test("a terminal off screen asks the host nothing; on screen it does")
    func offScreenAsksNothing() async {
        let (core, coordinator, window) = makeCore()
        defer { window.isHidden = true }
        core.removeFromSuperview()
        let host = Host()
        coordinator.pathLinks = RemotePathLinks(workingDirectory: { "/r" }, probe: { _, batch in host.probe(batch) })
        coordinator.feed(data: Data("out/a.png\r\n".utf8))
        _ = await wait(0.8) { false }
        #expect(host.asked.isEmpty, "a hidden pane's paths cost the host a probe")
        window.rootViewController?.view.addSubview(core)
        #expect(await wait { !host.asked.isEmpty }, "coming on screen runs a pass")
    }

    @Test("without a resolver no path is a link and the host is never asked")
    func noResolverNoLinks() async {
        let (core, coordinator, window) = makeCore()
        defer { window.isHidden = true }
        coordinator.feed(data: Data("out/a.png\r\n".utf8))
        #expect(core.settle { $0.rowText(0) == "out/a.png" })
        guard let term = core.term else { return }
        #expect(CoreTerminalView.pathCandidates(mp_term_path_candidates(term)).isEmpty)
    }

    @Test("OSC 7 reaches the session as the URL the program reported")
    func osc7() async {
        let (_, coordinator, window) = makeCore()
        defer { window.isHidden = true }
        var dirs: [String?] = []
        coordinator.onHostDirectoryChange = { dirs.append($0) }
        coordinator.feed(data: Data("\u{1b}]7;file://box/tmp/a%20b\u{07}x".utf8))
        #expect(await wait { !dirs.isEmpty })
        #expect(dirs == ["file://box/tmp/a%20b"])
        #expect(dirs.first.flatMap { $0 }.flatMap(RemotePathLink.path(from:)) == "/tmp/a b",
                "the session's reading of it")
    }

    /// The core's scanner (crates/term/src/paths.rs) against the app's
    /// (`TerminalPathScanner`, the SwiftTerm path's) on the same lines: the
    /// two views offer the host the same candidates.
    @Test("the core offers the host what the app's scanner finds", arguments: [
        "  3. 对比图：qchar/out/GripLab.png（准备、放网、平抽）。规则见 CLAUDE.md 第 3 步和 qchar/README.md「体型怎么挑」。",
        "saved to out/a.png.",
        "**out/plot.png** and (charts/b.jpg), `c.gif`",
        "保存到out/a.png了",
        "see src/main.swift:12 and out/a.png:3",
        "/Users/me/Desktop/shot.PNG ~/Downloads/clip.mov ./a.pdf ../b.md",
        "README Makefile notes.txt",
        "outputs in out/plots and ~/Desktop/charts/ v1.2/ src/main.swift 10/18 1/2 /config",
        "改稿：episodes/nolift/改稿建议.md。",
        "见 out/羽毛球/a.png 和 out/图/，",
        "在 out/里面有图",
        "for you: 成品/西游-写实版-粗剪.mp4. It's",
        "见 写实版-粗剪.mp4。",
        "输出到成品/a.png",
        "规则见CLAUDE.md",
        "a~b/c.png me@example.com",
        "已经输出到成品/粗剪.mp4。",
    ])
    func scannerParity(line: String) {
        let t = mp_term_new(120, 2, 0)
        defer { mp_term_close(t) }
        guard let t else { Issue.record("no core"); return }
        let kinds = PreviewableFile.extensions.sorted().joined(separator: ",")
        kinds.withCString { mp_term_set_path_extensions(t, $0) }
        let bytes = Array((line + "\r\n").utf8)
        bytes.withUnsafeBufferPointer { mp_term_feed(t, $0.baseAddress!, $0.count) }
        let core = CoreTerminalView.pathCandidates(mp_term_path_candidates(t)).map { "\($0.directoryOnly ? "D" : "F")\($0.token)" }
        var app: [String] = []
        for c in TerminalPathScanner.scan(line) {
            let key = "\(c.directoryOnly ? "D" : "F")\(c.token)"
            if !app.contains(key) { app.append(key) }
        }
        #expect(core == app)
    }
}
