import XCTest

/// The App Store screenshots, shot on CI's simulators against the staged
/// hosts (scripts/shots/stage-host.sh): what each store image shows under
/// its headline (scripts/shots/captions/, composed by
/// scripts/shots/compose.swift). Run by the screenshot workflow only
/// (`scripts/shots/ci-capture.sh`, which hands the runner its locales and
/// seeds as `TEST_RUNNER_SHOTS_*` and the staged host's
/// `TEST_RUNNER_MOSHPIT_SEED_*`); it skips anywhere else, CI's tests
/// included.
///
/// The iPhone's eight, in the store's order: the lock screen with an
/// agent's question and the Live Activity (01), the pane that asked (02),
/// Home's Needs You across three hosts (03), the Dynamic Island expanded
/// (04), the pane overview (05), a file the agent wrote, previewed (06),
/// the companion talking (07), the key bar with the keyboard up (08). The
/// iPad's two: the overview (panes) and Home beside a terminal
/// (home-terminal).
///
/// Every shot waits on its page's id, and a terminal shot on the session
/// being live with nothing over it (no connecting poster, no reconnect
/// veil), then a short settle for the page's own animation; then it keeps
/// the screen as an attachment named `shot~<locale>~<shot>`, which the
/// workflow exports out of the result bundle as `<locale>/<shot>.png`. The
/// test methods run in name order, each over every locale; the two that
/// leave the app (the lock screen, the island) run last.
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

    private static func json(_ name: String) -> [String] {
        guard let json = env[name], let data = json.data(using: .utf8),
              let args = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
        return args
    }

    /// `SHOTS_ARGS_JSON`: what every shot takes (the companion hatched, no
    /// first-run hints, no host care).
    private static var common: [String] { json("SHOTS_ARGS_JSON") }

    /// `SHOTS_QUIET_JSON`: no notifications and no Live Activity, for the
    /// shots inside the app (a simulator's banner never goes by itself).
    private static var quiet: [String] { json("SHOTS_QUIET_JSON") }

    /// What the lock screen and the island take instead of quiet: the
    /// question announced at once and every time (no 30 s grace, no
    /// episode remembered from the locale before), with its words on the
    /// lock screen; the Live Activity on.
    private static let loud = [
        "-MOSHPIT_ANNOUNCE_GRACE", "0", "-island.announcedEpisodes", "<dict/>",
        "-moshpit.settings.notifications", "<true/>", "-moshpit.settings.liveActivity", "<true/>",
        "-moshpit.settings.lockScreenDetail", "<true/>", "-moshpit.settings.attentionSound", "<false/>",
    ]

    @MainActor private static var phone: Bool { UIDevice.current.userInterfaceIdiom == .phone }

    /// A terminal shot: the staged session, its text larger than the
    /// default 9 pt on the phone (too small to read on a store page, dry
    /// run 4): `size`, a size the Settings stepper offers. The staged turn
    /// is wrapped to read at 13 pt's 54 columns.
    @MainActor
    private static func terminal(_ session: [String], size: Int = 11) -> [String] {
        session + (phone ? ["-moshpit.settings.fontSize", "\(size)"] : [])
    }

    override func setUpWithError() throws {
        continueAfterFailure = true
        guard !Self.locales.isEmpty else {
            throw XCTSkip("screenshot run only (SHOTS_LOCALES in the runner's environment)")
        }
    }

    @MainActor
    private func launch(_ locale: Locale, _ extra: [String], quiet: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(\(locale.language))", "-AppleLocale", locale.region]
            + Self.common + (quiet ? Self.quiet : Self.loud) + extra
        app.launch()
        return app
    }

    /// Home with the three staged hosts, each connecting.
    private var home: [String]? {
        Self.env["SHOTS_HOME_FIXTURE"].map { ["-MOSHPIT_SEED_HOME_FIXTURE", $0] }
    }

    /// Lets the page's own animation finish once its id is there.
    private func settle(_ seconds: TimeInterval = 1.5) {
        Thread.sleep(forTimeInterval: seconds)
    }

    /// Waits for the middle of the screen to show something light: a
    /// QuickLook preview's frame comes up black before its file (B844: 06
    /// was kept black in every locale), and what it draws lives in another
    /// process, out of the app's accessibility tree, so the screen itself
    /// is what can tell.
    @MainActor
    private func waitForLight(_ timeout: TimeInterval) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        repeat {
            if Self.lightShare(XCUIScreen.main.screenshot().image) > 0.05 { return true }
            Thread.sleep(forTimeInterval: 1)
        } while Date() < end
        return false
    }

    /// The share of the screen's middle (the centre 60 % each way) that is
    /// light, read from a 48 × 48 copy of it.
    private static func lightShare(_ image: UIImage) -> CGFloat {
        guard let cg = image.cgImage else { return 0 }
        let crop = CGRect(x: CGFloat(cg.width) * 0.2, y: CGFloat(cg.height) * 0.2,
                          width: CGFloat(cg.width) * 0.6, height: CGFloat(cg.height) * 0.6)
        guard let middle = cg.cropping(to: crop.integral) else { return 0 }
        let side = 48
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            context.draw(middle, in: CGRect(x: 0, y: 0, width: side, height: side))
            return true
        }
        guard drawn else { return 0 }
        var light = 0
        for i in stride(from: 0, to: pixels.count, by: 4) where
            Int(pixels[i]) + Int(pixels[i + 1]) + Int(pixels[i + 2]) > 3 * 200 {
            light += 1
        }
        return CGFloat(light) / CGFloat(side * side)
    }

    @MainActor
    private func keep(_ locale: Locale, _ shot: String) {
        let screen = XCUIScreen.main.screenshot()
        // The screen comes in the device's own portrait, a landscape page
        // on its side (dry run 4's iPad split): turned back to how it shows.
        let landscape = XCUIDevice.shared.orientation.isLandscape
        let attachment = landscape && screen.image.size.width < screen.image.size.height
            ? XCTAttachment(image: Self.turned(screen.image))
            : XCTAttachment(screenshot: screen)
        attachment.name = "shot~\(locale.name)~\(shot)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// `image` a quarter turn back (counterclockwise), at its own scale.
    private static func turned(_ image: UIImage) -> UIImage {
        let size = CGSize(width: image.size.height, height: image.size.width)
        let format = UIGraphicsImageRendererFormat()
        format.scale = image.scale
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            context.cgContext.translateBy(x: 0, y: size.height)
            context.cgContext.rotate(by: -.pi / 2)
            image.draw(at: .zero)
        }
    }

    /// A shot that didn't come: the screen as it was and what the app's
    /// accessibility tree held, for the next run's fix (`debug~` beside
    /// the shots; the tree in the log).
    @MainActor
    private func explain(_ app: XCUIApplication, _ locale: Locale, _ shot: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = "debug~\(locale.name)~\(shot)"
        attachment.lifetime = .keepAlways
        add(attachment)
        print("SHOTS debug \(locale.name) \(shot):\n\(app.debugDescription.prefix(8000))")
    }

    /// The first of `ids` to show within `timeout`, if one does.
    @MainActor
    private func first(_ app: XCUIApplication, of ids: [String], within timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if ids.contains(where: { app.nsh($0).exists }) { return true }
            Thread.sleep(forTimeInterval: 0.5)
        } while Date() < deadline
        return false
    }

    // MARK: Waiting

    /// Waits for `element` to be enabled (a breadcrumb segment is inert
    /// until the session is live).
    private func waitEnabled(_ element: XCUIElement, _ timeout: TimeInterval) -> Bool {
        let enabled = expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: element)
        return XCTWaiter().wait(for: [enabled], timeout: timeout) == .completed
    }

    /// The terminal live with nothing over it: the window segment answers
    /// (a live, attached tree), and the reconnect veil isn't there (the
    /// tree can be live a moment before the first frame is painted). The
    /// connecting poster is gone from the app, so there's no wait for it.
    @MainActor
    private func waitLive(_ app: XCUIApplication, _ locale: Locale) -> Bool {
        let window = app.nsh("terminal-crumb-window")
        guard window.waitForExistence(timeout: 60), waitEnabled(window, 30) else {
            XCTFail("\(locale.name): the seeded session never went live")
            return false
        }
        if !app.nsh("terminal-reconnect-veil").waitForNonExistence(timeout: 20) {
            XCTFail("\(locale.name): terminal-reconnect-veil is still up")
            return false
        }
        settle(0.8)
        return true
    }

    /// Home listing its agents, someone among them waiting (B620: a cold
    /// Home should, on its own; if it doesn't, the log says so and the test
    /// does what a person would: opens devbox's terminal and comes back).
    @MainActor
    private func waitNeedsYou(_ app: XCUIApplication, _ locale: Locale) -> Bool {
        let waiting = app.nsh("home-needs-you")
        let cold = waiting.waitForExistence(timeout: 45)
        print("SHOTS B620 \(locale.name): Home listed the waiting agent without a terminal: \(cold)")
        if !cold {
            let head = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'devbox'")).firstMatch
            if head.waitForExistence(timeout: 30) { head.tap() }
            let crumb = app.nsh("terminal-crumb-window")
            if crumb.waitForExistence(timeout: 30), waitEnabled(crumb, 30) {
                app.tapNSH("terminal-back")
            }
        }
        guard waiting.waitForExistence(timeout: 30) else {
            XCTFail("\(locale.name): Home never listed an agent waiting")
            return false
        }
        // The pet's offer of its app icon (B668), if the seed's answer
        // didn't hold: Not Yet, so it isn't over the shot.
        let notYet = app.nsh("home-icon-not-yet")
        if notYet.exists {
            print("SHOTS \(locale.name): Home offered the pet's icon; answered Not Yet")
            notYet.tap()
            settle(1)
        }
        return true
    }

    // MARK: Inside the app

    /// 02: the pane that asked: Claude Code's permission prompt, the key
    /// bar under it to answer with.
    @MainActor
    func test02Pane() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            // Twice: a UI query that timed out (ko and pt-BR, dry run 4)
            // took the shot with it.
            for _ in 0..<2 {
                let app = launch(locale, Self.terminal(session, size: 13))
                defer { app.terminate() }
                if waitLive(app, locale) {
                    settle(1.2)
                    keep(locale, "02-pane")
                    break
                }
            }
        }
    }

    /// 03: Home's Needs You over three hosts' agents, waiting, at work and
    /// done, the companion perched.
    @MainActor
    func test03Home() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        guard let home else { throw XCTSkip("no SHOTS_HOME_FIXTURE") }
        for locale in Self.locales {
            let app = launch(locale, home)
            if waitNeedsYou(app, locale) {
                settle(2)
                keep(locale, "03-home")
            }
            app.terminate()
        }
    }

    /// 05 (and the iPad's panes): the pane overview, every window of the
    /// session as a card, the split one with its three panes. A tap on the
    /// window segment opens it once the tree is live.
    @MainActor
    func test05Overview() throws {
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let app = launch(locale, Self.terminal(session))
            guard waitLive(app, locale) else { app.terminate(); continue }
            let shot = Self.phone ? "05-overview" : "panes"
            // The page, its bar's Done or its summary: whichever the tree
            // shows (dry run 4 waited on the page's own id and saw nothing).
            let page = ["terminal-overview", "terminal-overview-done", "terminal-overview-summary",
                        "terminal-overview-scroll"]
            // Each pane visited once, so the overview has a picture of each
            // (it draws the panes the app has shown; dry run 5's build and
            // review cards were empty): the sideways swipe walks the
            // session's ten panes and back to the one that asked.
            let area = app.nsh("terminal-pane-area")
            for _ in 0..<9 { area.swipeLeft(); settle(1.2) }
            for _ in 0..<9 { area.swipeRight(); settle(0.8) }
            app.nsh("terminal-crumb-window").tap()
            // Dry run 5: the overview opens (its debug shot shows it) but
            // isn't in the accessibility tree yet, so its ids can't be
            // waited on. Kept after its opening's time either way; the log
            // says which.
            let seen = first(app, of: page, within: 4)
            print("SHOTS overview \(locale.name): \(seen ? "its ids showed" : "kept without its ids (not in the accessibility tree)")")
            settle(1.5)
            keep(locale, shot)
            app.terminate()
        }
    }

    /// The middle of `count` columns from `column` across a row of the pane,
    /// as a fraction of the row, and the row's columns. A row's element is
    /// the terminal view's whole width; its cells are the app's own ("W" of
    /// the system monospaced font at `size`, in whole pixels:
    /// TerminalCellGeometry.measuredCell), the grid centred in the view to
    /// the pixel (CoreTerminalView.gridOrigin). B817: a guess of 40 columns
    /// put the tap two columns past the path on a row of 62.
    @MainActor
    private static func middle(ofColumns column: Int, _ count: Int, in row: XCUIElement,
                               of app: XCUIApplication, size: CGFloat) -> (CGFloat, Int) {
        // The screen's pixels a point: a screenshot's against the app's frame.
        let pixels = XCUIScreen.main.screenshot().image.cgImage.map { CGFloat(max($0.width, $0.height)) } ?? 0
        let points = max(app.frame.width, app.frame.height)
        let scale = pixels > 0 && points > 0 ? max((pixels / points).rounded(), 1) : 3
        let font = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        let cell = ceil(("W" as NSString).size(withAttributes: [.font: font]).width * scale) / scale
        let width = row.frame.width
        guard cell > 0, width > cell else { return (0.3, 0) }
        let columns = Int(width / cell)
        let spare = width - CGFloat(columns) * cell
        let origin = min(spare, ((spare / 2) * scale).rounded() / scale)
        let x = (origin + (CGFloat(column) + CGFloat(count) / 2) * cell) / width
        return (min(max(x, 0.02), 0.98), columns)
    }

    /// 06: the chart the agent wrote, opened from its path in the pane
    /// (a link once the host confirmed the file), previewed. If the tap
    /// misses the link, the Files chip's folder sheet reaches it.
    @MainActor
    func test06Preview() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let size = 11
            let app = launch(locale, Self.terminal(session, size: size))
            guard waitLive(app, locale) else { app.terminate(); continue }
            // The path is a link once the host's probe confirmed it.
            settle(4)
            // QuickLook: its title is the file's name (the pane's row says
            // more around it), or its own view.
            let preview = app.descendants(matching: .any).matching(NSPredicate(
                format: "label == 'latency.png' OR identifier == 'QLPreviewControllerView'")).firstMatch
            let row = app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'bench/latency.png'")).firstMatch
            if row.waitForExistence(timeout: 10) {
                // The path starts after "     Wrote " (a column a
                // character): aim at its middle.
                let path = (row.label as NSString).range(of: "bench/latency.png")
                let (x, columns) = Self.middle(ofColumns: path.location, path.length, in: row, of: app,
                                                   size: CGFloat(size))
                XCTContext.runActivity(named: "Tap the path, columns \(path.location)–\(path.location + path.length - 1) of \(columns), at x \(String(format: "%.3f", x))") { _ in
                    row.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.5)).tap()
                }
            }
            if !preview.waitForExistence(timeout: 20) {
                // The folder sheet: the pane's folder, then bench/.
                app.tapNSH("shortcut-files")
                let bench = app.descendants(matching: .any)
                    .matching(NSPredicate(format: "identifier ENDSWITH '/bench'")).firstMatch
                if bench.waitForExistence(timeout: 20) { bench.tap() }
                let file = app.descendants(matching: .any)
                    .matching(NSPredicate(format: "identifier ENDSWITH '/latency.png'")).firstMatch
                if file.waitForExistence(timeout: 20) { file.tap() }
            }
            if preview.waitForExistence(timeout: 30) {
                // The frame comes before the picture (dry run 5, B844 kept
                // it black): the chart itself, white, on the screen.
                guard waitForLight(20) else {
                    explain(app, locale, "06-preview")
                    XCTFail("\(locale.name): the preview opened and never showed the chart")
                    app.terminate()
                    continue
                }
                settle(1)
                keep(locale, "06-preview")
            } else {
                explain(app, locale, "06-preview")
                XCTFail("\(locale.name): the chart never opened")
            }
            app.terminate()
        }
    }

    /// 07: the companion itself: its card (Settings › Appearance › the
    /// companion), the hatched and grown pet large, its name, species and
    /// stage, how it grew. Home showed it small on a card, the same picture
    /// as 03 (dry run 4).
    @MainActor
    func test07Pet() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        for locale in Self.locales {
            let app = launch(locale, ["-MOSHPIT_SEED_SCREEN", "settings",
                                      "-MOSHPIT_SEED_SETTINGS_PAGE", "companion"])
            if app.nsh("settings-pet-hero").waitForExistence(timeout: 30) {
                settle(1.5)
                keep(locale, "07-pet")
            } else {
                explain(app, locale, "07-pet")
                XCTFail("\(locale.name): the companion's card never came")
            }
            app.terminate()
        }
    }

    /// 08: the key bar with the keyboard up, raised by the bar's own
    /// keyboard key and shot once iOS's keyboard is there (a clock missed
    /// it in both Chinese locales: the CJK keyboard takes longer). The bar
    /// carries the image and voice keys by default. iPhone only: the iPad
    /// simulator raises no software keyboard.
    @MainActor
    func test08Keyboard() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        guard let session = NSHFlow.seededSession() else { throw XCTSkip("no staged host") }
        for locale in Self.locales {
            let app = launch(locale, Self.terminal(session, size: 13))
            guard waitLive(app, locale) else { app.terminate(); continue }
            app.tapNSH("keyboard-toggle")
            if app.keyboards.firstMatch.waitForExistence(timeout: 15) {
                settle(1)
                // A reply going in, so the shot isn't 02's again.
                // The pane's tty echoes and nothing reads it, so each locale's
                // text piled up on the staged screen (and under 02's shot).
                // Ctrl-U (kill line) wipes the pending input and its echo.
                app.typeText("\u{15}add a test for the jitter cap")
                settle(2)
                keep(locale, "08-keyboard")
                app.typeText("\u{15}")
            } else {
                XCTFail("\(locale.name): the keyboard never came up")
            }
            app.terminate()
        }
    }

    /// The iPad's Home beside a terminal: the split view in landscape (in
    /// portrait the terminal takes the screen and Home only overlays it),
    /// devbox's terminal opened from its card.
    @MainActor
    func test10HomeAndTerminal() throws {
        guard !Self.phone else { throw XCTSkip("iPad only") }
        guard let home else { throw XCTSkip("no SHOTS_HOME_FIXTURE") }
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        for locale in Self.locales {
            let app = launch(locale, home)
            guard waitNeedsYou(app, locale) else { app.terminate(); continue }
            let head = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'devbox'")).firstMatch
            if head.waitForExistence(timeout: 30) { head.tap() }
            if waitLive(app, locale) {
                // The sidebar back beside it, if opening hid it.
                if !app.nsh("home-needs-you").exists { app.nsh("terminal-sidebar").tap() }
                settle(1.5)
                // B843: the terminal's page beside the sidebar, not under it
                // (it was the window's whole width, its crumb and text behind
                // the sidebar, the pane black).
                let crumb = app.nsh("terminal-crumb-window").frame, sidebar = app.nsh("home-needs-you").frame
                XCTAssertGreaterThan(crumb.minX, sidebar.maxX,
                                     "\(locale.name): the terminal's crumb \(crumb) is under the sidebar \(sidebar)")
                keep(locale, "home-terminal")
            }
            app.terminate()
        }
    }

    // MARK: Outside the app (last: they leave it)

    /// SpringBoard: the lock screen and the Dynamic Island.
    @MainActor private var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    /// The system's notification permission, allowed when it asks (the
    /// first launch on a fresh simulator): its second button, whatever the
    /// language says.
    @MainActor
    private func allowNotifications() {
        let alert = springboard.alerts.firstMatch
        if alert.waitForExistence(timeout: 5), alert.buttons.count > 1 {
            alert.buttons.element(boundBy: 1).tap()
        }
    }

    /// Home's agents loaded, the question announced and the Live Activity
    /// up, then the app sent to the background.
    @MainActor
    private func announceAndLeave(_ locale: Locale) -> XCUIApplication? {
        guard let home else { return nil }
        let app = launch(locale, home, quiet: false)
        allowNotifications()
        guard waitNeedsYou(app, locale) else { app.terminate(); return nil }
        // The announcement's sweep runs every 2 s; the island's state with
        // it.
        settle(5)
        XCUIDevice.shared.press(.home)
        settle(2)
        return app
    }

    /// 04: the Dynamic Island, expanded (a long press on it from the home
    /// screen), its agents and their timers.
    @MainActor
    func test90Island() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        for locale in Self.locales {
            guard let app = announceAndLeave(locale) else { throw XCTSkip("no SHOTS_HOME_FIXTURE") }
            // A banner from the announcement may sit on the island first.
            settle(5)
            springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.025))
                .press(forDuration: 1.2)
            settle(1.2)
            keep(locale, "04-island")
            print("SHOTS island \(locale.name): kept; check it shows the expanded island")
            app.terminate()
        }
    }

    /// 01: the lock screen: the agent's question as a notification and
    /// the Live Activity under the clock.
    @MainActor
    func test91Lock() throws {
        guard Self.phone else { throw XCTSkip("iPhone only") }
        for locale in Self.locales {
            guard let app = announceAndLeave(locale) else { throw XCTSkip("no SHOTS_HOME_FIXTURE") }
            XCUIDevice.shared.perform(NSSelectorFromString("pressLockButton"))
            settle(1.5)
            // The lock button darkens the screen; the home button wakes it
            // on the lock screen.
            XCUIDevice.shared.press(.home)
            settle(2.5)
            // The first time, the lock screen asks whether Moshpit may show
            // Live Activities there (dry run 4): its second button allows.
            let ask = springboard.buttons.matching(NSPredicate(format: "label == 'Allow'")).firstMatch
            if ask.exists {
                ask.tap()
                settle(2)
            }
            keep(locale, "01-lock")
            print("SHOTS lock \(locale.name): kept; check it shows the notification and the Live Activity")
            // Unlock (no passcode on a simulator) for the next locale.
            XCUIDevice.shared.press(.home)
            settle(1)
            springboard.swipeUp()
            settle(1)
            app.terminate()
        }
    }
}
