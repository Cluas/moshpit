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

extension XCUIApplication {
    /// Scroll until `element` is on screen and returns whether it got there.
    ///
    /// The app's forms are `Form`/`List`, which materialise rows lazily: a row
    /// below the fold is not in the accessibility tree at all, so `.exists`
    /// on it is false until the list has scrolled it into view. Swipes up
    /// first (most targets sit further down a form), then back down.
    @discardableResult
    func reveal(_ element: XCUIElement, up: Int = 6, down: Int = 8) -> Bool {
        if element.exists && element.isHittable { return true }
        for _ in 0..<up {
            swipeUp()
            if element.exists && element.isHittable { return true }
        }
        for _ in 0..<down {
            swipeDown()
            if element.exists && element.isHittable { return true }
        }
        return element.exists && element.isHittable
    }
}
