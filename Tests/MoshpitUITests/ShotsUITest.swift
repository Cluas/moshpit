import XCTest

/// The store and README screenshots a tap reaches, or that must wait for a
/// page to be there rather than for a fixed time: Home with its agents, the
/// agent's pane, the tmux window picker. Run by the screenshot workflow
/// only (`scripts/shots/ci-capture.sh`, which hands the runner its locales
/// and seeds as `TEST_RUNNER_SHOTS_*` and the staged host's
/// `TEST_RUNNER_MOSHPIT_SEED_*`); it skips anywhere else, CI's tests
/// included.
///
/// Every shot waits on its page's id (the same ids the NSH flows tap,
/// ``NSHFlow``), then a short settle for the page's own animation, then
/// keeps the screen as an attachment named `shot~<locale>~<shot>`, which
/// the workflow exports out of the result bundle as `<locale>/<shot>.png`.
final class ShotsUITest: XCTestCase {
    /// A store locale, the app language it runs in and its region.
    struct Locale {
        let name: String
        let language: String
        let region: String
    }

    private static var env: [String: String] { ProcessInfo.processInfo.environment }

    /// `SHOTS_LOCALES`: `name=language=region` words, e.g.
    /// `zh-Hans=zh-Hans=zh_CN en-US=en=en_US`.
    private static var locales: [Locale] {
        (env["SHOTS_LOCALES"] ?? "").split(separator: " ").compactMap { word in
            let parts = word.split(separator: "=").map(String.init)
            guard parts.count == 3 else { return nil }
            return Locale(name: parts[0], language: parts[1], region: parts[2])
        }
    }

    /// `SHOTS_ARGS_JSON`: the launch arguments every shot takes (quiet, the
    /// pet, no first-run hints), as the simulator shots take them.
    private static var common: [String] {
        guard let json = env["SHOTS_ARGS_JSON"], let data = json.data(using: .utf8),
              let args = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
        return args
    }

    override func setUpWithError() throws {
        continueAfterFailure = true
        guard !Self.locales.isEmpty else {
            throw XCTSkip("screenshot run only (SHOTS_LOCALES in the runner's environment)")
        }
    }

    @MainActor
    private func launch(_ locale: Locale, _ extra: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(\(locale.language))", "-AppleLocale", locale.region]
            + Self.common + extra
        app.launch()
        return app
    }

    /// Lets the page's own animation finish once its id is there.
    private func settle(_ seconds: TimeInterval = 1.5) {
        Thread.sleep(forTimeInterval: seconds)
    }

    @MainActor
    private func keep(_ locale: Locale, _ shot: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "shot~\(locale.name)~\(shot)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// Home: the staged host connected, its agents listed (one waiting on
    /// the person, so the Needs You group is there), the companion on its
    /// card.
    @MainActor
    func test01HomeAgents() throws {
        guard let fixture = Self.env["SHOTS_HOME_FIXTURE"] else { throw XCTSkip("no SHOTS_HOME_FIXTURE") }
        for locale in Self.locales {
            let app = launch(locale, ["-MOSHPIT_SEED_HOME_FIXTURE", fixture])
            let waiting = app.nsh("home-needs-you")
            if !waiting.waitForExistence(timeout: 20) {
                // Home lists the agents of panes a terminal has attached to
                // (dry run 2: connected, stamped, and no agents on Home): open
                // devbox's terminal from its card, as a person would, and
                // come back.
                let head = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'devbox'")).firstMatch
                if head.waitForExistence(timeout: 30) { head.tap() }
                let crumb = app.nsh("terminal-crumb-window")
                if crumb.waitForExistence(timeout: 30), waitEnabled(crumb, 30) {
                    app.tapNSH("terminal-back")
                }
            }
            if waiting.waitForExistence(timeout: 30) {
                settle(2)
                keep(locale, "01-home")
            } else {
                XCTFail("\(locale.name): Home never listed an agent waiting")
            }
            app.terminate()
        }
    }

    /// The agent's pane: the staged Claude Code turn asking for an edit, the
    /// session live (its window segment answers).
    @MainActor
    func test02AgentPane() throws {
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let app = launch(locale, session)
            let window = app.nsh("terminal-crumb-window")
            if window.waitForExistence(timeout: 60) && waitEnabled(window, 30) {
                settle(2)
                keep(locale, "02-agent-pane")
            } else {
                XCTFail("\(locale.name): the seeded session never went live")
            }
            app.terminate()
        }
    }

    /// tmux's windows: the picker the breadcrumb's window segment opens,
    /// each row with its agent's state.
    @MainActor
    func test04TmuxWindows() throws {
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let app = launch(locale, session)
            let window = app.nsh("terminal-crumb-window")
            guard window.waitForExistence(timeout: 60), waitEnabled(window, 30) else {
                XCTFail("\(locale.name): the seeded session never went live")
                app.terminate()
                continue
            }
            window.tap()
            if app.nsh("mux-picker-rows").waitForExistence(timeout: 15) {
                settle()
                keep(locale, "04-tmux-windows")
            } else {
                XCTFail("\(locale.name): the window picker never listed its windows")
            }
            app.terminate()
        }
    }

    /// The pane with the software keyboard and the key bar up, raised by
    /// the key bar's own keyboard key and shot once iOS's keyboard is
    /// there (a clock missed it in both Chinese locales: the CJK keyboard
    /// takes longer). iPhone only: the iPad simulator raises no software
    /// keyboard.
    @MainActor
    func test06Keyboard() throws {
        guard UIDevice.current.userInterfaceIdiom == .phone else { throw XCTSkip("iPhone only") }
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let app = launch(locale, session)
            let window = app.nsh("terminal-crumb-window")
            guard window.waitForExistence(timeout: 60), waitEnabled(window, 30) else {
                XCTFail("\(locale.name): the seeded session never went live")
                app.terminate()
                continue
            }
            app.tapNSH("keyboard-toggle")
            if app.keyboards.firstMatch.waitForExistence(timeout: 15) {
                settle(2)
                keep(locale, "06-keyboard")
            } else {
                XCTFail("\(locale.name): the keyboard never came up")
            }
            app.terminate()
        }
    }

    /// Waits for `element` to be enabled (a breadcrumb segment is inert
    /// until the session is live).
    private func waitEnabled(_ element: XCUIElement, _ timeout: TimeInterval) -> Bool {
        let enabled = expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: element)
        return XCTWaiter().wait(for: [enabled], timeout: timeout) == .completed
    }
}
