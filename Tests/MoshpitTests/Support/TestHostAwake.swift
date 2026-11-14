import Foundation
import Testing
import UIKit

/// The test bundle's principal class (`NSPrincipalClass`, project.yml):
/// XCTest makes one as it loads the bundle, before any test runs.
///
/// It keeps the test host's screen on for the run. A full run on the iPad
/// takes six minutes; the device's auto-lock does not care that tests are
/// running, and once the screen sleeps the host's scene leaves the
/// foreground — the display link every NSH surface draws on stops, and each
/// real-surface test after that waits out its ceiling for a view that is
/// never mounted, while the tests around it pass. The runner's lock check
/// can't see it: this iPad has no passcode, so it reads "not locked" with
/// the screen off.
@objc(MoshpitTestHostAwake)
final class TestHostAwake: NSObject {
    override init() {
        super.init()
        DispatchQueue.main.async {
            UIApplication.shared.isIdleTimerDisabled = true
        }
    }
}

/// How long a test waits for a real NSH surface to show something, or for
/// its screen to say something: a ceiling, not a window — every wait returns
/// the moment its condition holds. Five seconds was a window, and on a busy
/// device it was tight.
enum NSHTestWait {
    static let ceiling: TimeInterval = 30
}

@Suite("Test host")
@MainActor
struct TestHostAwakeTests {
    @Test("the test host keeps its screen on for the run")
    func screenStaysOn() async {
        for _ in 0..<50 where !UIApplication.shared.isIdleTimerDisabled {
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(UIApplication.shared.isIdleTimerDisabled,
                "the principal class never ran: the screen can sleep mid-run")
    }
}
