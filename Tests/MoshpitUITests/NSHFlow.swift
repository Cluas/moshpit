import XCTest

/// What every NSH flow drives the app with: elements found by their
/// accessibility id, waited for, and tapped, never a coordinate.
///
/// Every NSH element is a real `UIView` whose `accessibilityIdentifier` is
/// its `test_id` (nsh-platform-ios), so the ids the Rust UI tests press
/// headlessly (`ui/tests`, `press("id:…")`) are the ids these flows tap on a
/// simulator. Two things differ from the headless runs:
///
/// - An element that is itself an accessibility element (a pressable row,
///   a switch) hides the ids inside it. Tap and read the outermost id; its
///   label and value carry what is inside (a row's trailing value, a
///   switch's state).
/// - A field's text input is `<field id>-input` (a field's own id names its
///   card); a secret input is a native `UITextField` with the id it was
///   given (`connection-form-password`).
///
/// The screenshot run's `ShotsUITest` uses the same helpers: one way to
/// reach a screen, so a shot and a flow can never disagree about where a
/// row is. The flows are `NSH*FlowUITest`; each says whether it runs on
/// CI's simulator or needs a host.
enum NSHFlow {
    /// No banners, no host care, no notification prompt: a fresh launch
    /// whose first screen is the one the flow asked for.
    static let quiet = [
        "-MOSHPIT_SEED_QUIET", "1", "-MOSHPIT_AUTOCARE_OFF",
        "-moshpit.settings.notifications", "<false/>",
        "-moshpit.settings.liveActivity", "<false/>",
    ]

    /// How long a screen may take to come up on a loaded CI runner.
    static let timeout: TimeInterval = 15

    /// The app, launched quiet. `reset` empties what a flow could find left
    /// over from the last one (connections, custom themes and accents, the
    /// theme and icon picks): flows that persist something across a
    /// relaunch launch the second time without it.
    @MainActor
    static func launch(_ extra: [String] = [], reset: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += quiet
        if reset { app.launchArguments.append("-MOSHPIT_RESET") }
        app.launchArguments += extra
        app.launch()
        return app
    }

    /// `app` closed and opened again the way a person does it, to find what
    /// it stored: sent to the background first, then ended, then launched
    /// without the reset. `terminate()` on an app still in front kills it
    /// on the spot. UserDefaults writes asynchronously (`synchronize()` is
    /// documented as waiting for those pending writes), so a setting
    /// changed a moment before can be lost, and a relaunch reads the old
    /// value. CI's run on 863f81c did (B820).
    @MainActor
    static func relaunch(_ app: XCUIApplication, _ extra: [String] = [],
                         file: StaticString = #filePath, line: UInt = #line) -> XCUIApplication {
        XCUIDevice.shared.press(.home)
        // Suspended once it has nothing left to finish; in the background
        // at least.
        let left = app.wait(for: .runningBackgroundSuspended, timeout: 5)
            || app.state == .runningBackground
        XCTAssertTrue(left, "the app never left the foreground (state \(app.state.rawValue))",
                      file: file, line: line)
        app.terminate()
        return launch(extra, reset: false)
    }

    /// The seeded session's launch arguments, from the test runner's
    /// environment (`TEST_RUNNER_MOSHPIT_SEED_*` given to xcodebuild arrives
    /// here without the prefix): the local rig's or a staged host's. `nil`
    /// when there is none, and a flow that needs one skips.
    static func seededSession(_ env: [String: String] = ProcessInfo.processInfo.environment) -> [String]? {
        guard let user = env["MOSHPIT_SEED_USER"], let key = env["MOSHPIT_SEED_KEY_B64"] else { return nil }
        var args = ["-MOSHPIT_SEED_USER", user, "-MOSHPIT_SEED_KEY_B64", key]
        for name in ["HOST", "PORT", "TRUST_FP", "NAME", "MUX", "TMUX_BIN"] {
            if let value = env["MOSHPIT_SEED_\(name)"] { args += ["-MOSHPIT_SEED_\(name)", value] }
        }
        return args
    }
}

/// Which App Store storefront the app believes it was bought in (the
/// mainland's carries the filing number in Settings).
enum MainlandStorefrontArgs {
    static let mainland = "-MOSHPIT_STOREFRONT"
    static let code = "CHN"
}

extension XCUIApplication {
    /// The element whose accessibility id is `id`, whatever its type.
    func nsh(_ id: String) -> XCUIElement {
        descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The first element whose id is one of `ids` (a card is
    /// `home-card-<name>` once connected, `connection-card-<name>` while
    /// only saved).
    func nsh(anyOf ids: [String]) -> XCUIElement {
        descendants(matching: .any).matching(NSPredicate(format: "identifier IN %@", ids)).firstMatch
    }

    /// Waits for `id` and fails the flow, naming it, when it never comes.
    @MainActor
    @discardableResult
    func waitNSH(_ id: String, timeout: TimeInterval = NSHFlow.timeout,
                 _ message: String? = nil,
                 file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let element = nsh(id)
        XCTAssertTrue(element.waitForExistence(timeout: timeout),
                      message ?? "\(id) never appeared", file: file, line: line)
        return element
    }

    /// Waits for `id`, scrolls it on screen when a list holds it below the
    /// fold, and taps it.
    @MainActor
    func tapNSH(_ id: String, timeout: TimeInterval = NSHFlow.timeout,
                file: StaticString = #filePath, line: UInt = #line) {
        let element = waitNSH(id, timeout: timeout, file: file, line: line)
        if !element.isHittable { reveal(element) }
        XCTAssertTrue(element.isHittable, "\(id) is there but not on screen", file: file, line: line)
        element.tap()
    }

    /// Back on an NSH page: its own bar's back button (the system bar is
    /// hidden under NSH pages).
    @MainActor
    func backNSH(file: StaticString = #filePath, line: UInt = #line) {
        tapNSH("nav-bar-back", file: file, line: line)
    }

    /// Types `text` into the field `field` (its input, `<field>-input`).
    @MainActor
    func typeNSH(_ field: String, _ text: String,
                 file: StaticString = #filePath, line: UInt = #line) {
        let input = "\(field)-input"
        tapNSH(input, file: file, line: line)
        nsh(input).typeText(text)
    }

    /// Settings, from Home's bar, up on its hub.
    @MainActor
    func openSettings(file: StaticString = #filePath, line: UInt = #line) {
        tapNSH("home-settings", file: file, line: line)
        waitNSH("settings", file: file, line: line)
    }

    /// One of Settings' parts, from its hub (`appearance`, `terminal`,
    /// `connections`, `notifications`, `voice`).
    @MainActor
    func openSettingsPart(_ part: String, file: StaticString = #filePath, line: UInt = #line) {
        tapNSH("settings-open-\(part)", file: file, line: line)
        waitNSH("nav-bar-back", file: file, line: line)
    }
}

extension XCUIElement {
    /// A switch's or a checkable row's state, as NSH reports it in the
    /// element's value ("checked" / "not checked"); `nil` when it reports
    /// none.
    var isOn: Bool? {
        guard let value = value as? String else { return nil }
        if value.contains("not checked") { return false }
        if value.contains("checked") { return true }
        switch value {
        case "1", "On", "on": return true
        case "0", "Off", "off": return false
        default: return nil
        }
    }

    /// Everything a screen reader hears of it: its label and its value.
    var spoken: String {
        [label, value as? String ?? ""].filter { !$0.isEmpty }.joined(separator: ", ")
    }
}
