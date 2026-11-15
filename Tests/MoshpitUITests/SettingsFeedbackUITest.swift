import XCTest

/// Settings › Send Feedback on the NSH page: the GitHub way in is offered
/// beside mail, and its row reads as Chinese written for Chinese readers,
/// not a translation. Runs on CI's simulator (no Mail account there, so the
/// page also says mail isn't set up).
final class SettingsFeedbackUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    private func openFeedback(languages: [String] = []) -> XCUIApplication {
        var extra: [String] = []
        if !languages.isEmpty {
            extra += ["-AppleLanguages", "(\(languages.joined(separator: ", ")))"]
        }
        let app = NSHFlow.launch(extra)
        app.openSettings()
        app.tapNSH("settings-feedback")
        app.waitNSH("feedback")
        return app
    }

    @MainActor
    private func keep(_ app: XCUIApplication, as name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    @MainActor
    func testFeedbackOffersGitHubBesideMail() throws {
        let app = openFeedback()
        let issue = app.waitNSH("feedback-github-issue")
        XCTAssertTrue(issue.spoken.contains("Open an issue on GitHub"), issue.spoken)
        keep(app, as: "settings-feedback-en")
    }

    @MainActor
    func testFeedbackReadsNativelyInSimplifiedChinese() throws {
        let app = openFeedback(languages: ["zh-Hans"])
        let issue = app.waitNSH("feedback-github-issue")
        XCTAssertTrue(issue.spoken.contains("在 GitHub 上提 issue"), issue.spoken)
        keep(app, as: "settings-feedback-zh-Hans")
    }
}
