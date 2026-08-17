import XCTest

extension XCTestCase {
    /// Long-press `element` until `revealed` shows up — RETRIED, because on a
    /// loaded CI runner a synthesized long-press occasionally lands while the
    /// row is still settling and the context menu simply never opens. The
    /// press is the flaky half, not the wait: the same test went green at
    /// 17:39 and red at 17:53 on identical code (2026-08-16) before every
    /// context-menu site was routed through here.
    func pressForMenu(_ element: XCUIElement,
                      until revealed: XCUIElement,
                      attempts: Int = 3,
                      _ message: String = "context menu should appear") {
        for attempt in 1...attempts {
            element.press(forDuration: 1.3)
            if revealed.waitForExistence(timeout: 3) { return }
            XCTAssertTrue(attempt < attempts, message)
        }
    }
}
