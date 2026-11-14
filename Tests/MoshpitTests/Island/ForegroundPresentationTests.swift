import Testing
import UserNotifications
import MoshpitKit

/// B315: in the foreground, a `.passive` card — the waiting card redrawn
/// around a second agent, or after one was answered; a short turn's finish —
/// bannered like a new prompt, on every redraw. Only the moments that
/// interrupt in the background (`.timeSensitive`, `.active`) banner in the
/// foreground, and not over the pane they are about.
@Suite("Foreground presentation of agent cards")
struct ForegroundPresentationTests {

    @Test("a passive update to a card goes to the list, never a banner (B315)")
    func passiveNeverBanners() {
        let options = AgentNotificationHandler.foregroundPresentation(level: .passive, paneVisible: false)
        #expect(!options.contains(.banner), "a passive redraw bannered in the foreground")
        #expect(!options.contains(.sound))
        #expect(options.contains(.list))
    }

    @Test("a new prompt or a long finish banners, except over its own pane")
    func edgesBanner() {
        for level in [UNNotificationInterruptionLevel.timeSensitive, .active] {
            #expect(AgentNotificationHandler.foregroundPresentation(level: level, paneVisible: false)
                    == [.banner, .sound, .list])
            #expect(AgentNotificationHandler.foregroundPresentation(level: level, paneVisible: true)
                    == [.list])
        }
    }
}
