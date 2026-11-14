import Foundation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// Drives the notification service extension's own entry point.
///
/// This is as close to a real push as this repo can get without an APNs key and
/// a physical phone: a `UNNotificationRequest` shaped exactly like the one the
/// relay causes iOS to deliver, handed to the same `didReceive` the system calls,
/// reading secrets from the same App Group store the extension reads on device.
/// What it cannot cover is APNs delivery itself, and therefore the extension
/// actually being launched while the screen is locked.
///
/// `didReceive` answers synchronously in every path — there is no network in it —
/// so the tests collect the delivered content from a plain closure. If it ever
/// grows async work, these need a continuation instead.
@Suite("Push notification service extension", .serialized, .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
struct PushNotificationServiceTests {

    static let secret = PushSealedBoxTests.secret
    static let conn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"

    /// Install one pairing for the duration of a test, restoring whatever the
    /// container held. The store is real shared state — the extension has no
    /// injection seam, because on device it has no one to inject it.
    private func withPairing<T>(secretHex: String = secret, label: String = "m1-pro",
                                _ body: () throws -> T) throws -> T {
        let saved = PushPairingStore.read()
        defer { PushPairingStore.write(saved) }
        PushPairingStore.write([
            PushPairing(connectionId: UUID(uuidString: Self.conn)!,
                        hostLabel: label, secretHex: secretHex,
                        sendToken: String(repeating: "0", count: 64),
                        relayURL: "https://push.example.org", createdAt: Date()),
        ])
        return try body()
    }

    /// A request in the shape the relay produces: translated fallback text, the
    /// attention category, and the sealed envelope under `mp`.
    private func request(state: String = "attention",
                         title: String? = "Bash: rm -rf build",
                         dur: Int? = nil, ts: Int = Int(Date().timeIntervalSince1970),
                         sealedWith key: String = secret,
                         naming conn: String = conn,
                         agent: String = "claude") throws -> UNNotificationRequest {
        let status = PushSealedBox.Status(conn: conn, host: "m1-pro", sess: "work",
                                         pane: "%3", agent: agent, state: state,
                                         title: title,
                                         // See PushRemoteNotificationTests: a
                                         // frozen ts would silently exercise the
                                         // stale-attention path.
                                         ts: ts, dur: dur)
        let sealed = try PushSealedBox.seal(try JSONEncoder().encode(status), secretHex: key)
        let content = UNMutableNotificationContent()
        content.title = "An agent needs you"
        content.body = "Open Moshpit to see what it is asking."
        // No category, matching what the relay actually sends: an unopened push
        // must not offer buttons that have nowhere to send a keystroke.
        content.userInfo = [
            "aps": ["mutable-content": 1],
            PushRemoteNotification.envelopeKey: [
                "v": sealed.v, "iv": sealed.iv, "ct": sealed.ct, "mac": sealed.mac,
            ],
        ]
        // iOS uses apns-collapse-id as the delivered notification's identifier,
        // which the relay sets to the app's own local identifier for that pane.
        return UNNotificationRequest(identifier: "moshpit.\(state).\(Self.conn).%3",
                                     content: content, trigger: nil)
    }

    private func deliver(_ request: UNNotificationRequest) -> UNNotificationContent? {
        var delivered: UNNotificationContent?
        NotificationService().didReceive(request) { delivered = $0 }
        return delivered
    }

    @Test("the extension opens the envelope and rewrites the notification")
    func opensAndRewrites() throws {
        let delivered = try withPairing { deliver(try request()) }
        let content = try #require(delivered)
        #expect(content.title == "claude needs you")
        #expect(content.body == "Bash: rm -rf build — m1-pro · work")
        // The two keys the lock-screen Allow/Deny path reads. Without them a
        // pushed notification would look right and do nothing.
        #expect(content.userInfo[AgentNotifications.connectionKey] as? String == Self.conn)
        #expect(content.userInfo[AgentNotifications.paneKey] as? String == "%3")
        // No category, decrypted or not: these notifications have no actions.
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("a done push comes through as finished, not as a question")
    func donePush() throws {
        let delivered = try withPairing { deliver(try request(state: "done", title: nil)) }
        let content = try #require(delivered)
        #expect(content.title == "✓ claude finished")
        #expect(content.categoryIdentifier.isEmpty)
    }

    /// The extension wrote its cards in its own process's languages — the
    /// system's — while the app writes in the language picked for Moshpit
    /// (Settings → Moshpit → Language). With the two apart, a pushed card and
    /// the app's own copy of the same event read in different languages, and
    /// the lock screen showed whichever landed last. The app records its
    /// language in the App Group (AppLanguage); the extension writes in it.
    /// Run on a device in English: Moshpit set to Chinese must give Chinese.
    @Test("a pushed card is written in the language Moshpit shows, not the system's")
    func pushedCardFollowsTheAppsLanguage() throws {
        let saved = AppLanguage.stored()
        defer {
            if let saved { AppLanguage.record(saved) }
            else { UserDefaults(suiteName: PushPrefs.suite)?.removeObject(forKey: AppLanguage.key) }
        }
        AppLanguage.record("zh-Hans")

        let done = try #require(try withPairing { deliver(try request(state: "done", title: nil)) })
        #expect(done.title == "✓ claude 已完成", "the system's language, not Moshpit's: \(done.title)")
        let asking = try #require(try withPairing { deliver(try request()) })
        #expect(asking.title == "claude 在等你", "the system's language, not Moshpit's: \(asking.title)")
    }

    /// A long turn's finish reaches the phone twice when the app is awake:
    /// this push, and the app's own card from its poll. Whichever is first
    /// rings; the other only updates the card (PushFinishes).
    @Test("a long finish the app already rang arrives silent; one it didn't, rings")
    func longFinishRingsOnce() throws {
        let finishes = try #require(PushFinishes.appGroupFile)
        let saved = try? Data(contentsOf: finishes.url)
        defer {
            if let saved { try? saved.write(to: finishes.url) }
            else { try? FileManager.default.removeItem(at: finishes.url) }
        }
        let ended = Int(Date().timeIntervalSince1970)
        // The sound switch on, as it ships; put back as it was after.
        let prefs = PushPrefs.read()
        PushPrefs.write(showDetail: prefs.showDetail, sound: true)
        defer { PushPrefs.write(showDetail: prefs.showDetail, sound: prefs.sound) }

        // Nobody rang this turn yet: the push does.
        let first = try withPairing { deliver(try request(state: "done", title: nil, dur: 600, ts: ended)) }
        #expect(try #require(first).sound != nil)

        // The app rang the next turn's finish before its push arrived.
        #expect(PushFinishes.noteRing(conn: Self.conn, pane: "%3", endedAt: ended + 900))
        let second = try withPairing {
            deliver(try request(state: "done", title: nil, dur: 600, ts: ended + 901))
        }
        let content = try #require(second)
        #expect(content.sound == nil, "one turn's finish rang twice")
        #expect(content.interruptionLevel == .passive)
        #expect(content.title.hasPrefix("✓ claude finished"))
    }

    @Test("the extension names the connection what the phone calls it")
    func pairingLabelNamesThePlace() throws {
        // The envelope carries the host's `hostname`; the pairing it points at
        // carries the label the user typed when saving the connection. The
        // card uses the latter, like the local card does.
        let delivered = try withPairing(label: "mac-mini") { deliver(try request()) }
        let content = try #require(delivered)
        #expect(content.body == "Bash: rm -rf build — mac-mini · work")
    }

    @Test("a push this device has no key for keeps the translated fallback")
    func unknownSecretFallsBack() throws {
        // The normal cause is a host paired to a different install — not an
        // error, and not something to tell the user about in a notification.
        let delivered = try withPairing {
            deliver(try request(sealedWith: String(repeating: "ab", count: 32)))
        }
        let content = try #require(delivered)
        #expect(content.title == "An agent needs you")
        #expect(content.userInfo[AgentNotifications.connectionKey] == nil)
    }

    /// B435: a host holds one pairing's key and writes the connection id
    /// into what it seals. Sealed with this pairing's key but naming another
    /// connection, the push read as that connection's: its name, its waiting
    /// set, a tap into its pane. It keeps the fallback, as if unopened.
    @Test("a push sealed with one connection's key, naming another, keeps the fallback")
    func misnamedPushFallsBack() throws {
        let other = "9B2C1D3E-0000-4000-8000-00000000000A"
        let delivered = try withPairing { deliver(try request(naming: other)) }
        let content = try #require(delivered)
        #expect(content.title == "An agent needs you")
        #expect(content.userInfo[AgentNotifications.connectionKey] == nil)
        #expect(PushStanding.standing(conn: other).isEmpty, "recorded as the other connection's")
    }

    /// B439: a push self-test named the pane it ran outside of (`%0`, where
    /// an agent often waits), and its "done" cleared that pane's waiting
    /// entry: the question left the set and rang again as a new one. A
    /// self-test leaves the set as it found it.
    @Test("a push self-test leaves the waiting set as it found it")
    func selfTestLeavesTheWaitingSet() throws {
        defer { PushStanding.clear(conn: Self.conn, pane: "%3") }
        let asked = Int(Date().timeIntervalSince1970)
        PushStanding.noteStanding(conn: Self.conn,
                                  entry: .init(pane: "%3", agent: "claude", since: asked, recordedAt: Date()))
        let delivered = try withPairing {
            deliver(try request(state: "done", title: "selftest-ab12",
                                agent: PushRemoteNotification.selfTestAgent))
        }
        _ = try #require(delivered)
        #expect(PushStanding.standing(conn: Self.conn).map(\.pane) == ["%3"])
    }

    @Test("with nothing paired the fallback arrives, and its buttons are gone")
    func noPairings() throws {
        let saved = PushPairingStore.read()
        defer { PushPairingStore.write(saved) }
        PushPairingStore.write([])
        let content = try #require(deliver(try request()))
        #expect(content.title == "An agent needs you")
        // This assertion used to demand the OPPOSITE, and was wrong. Allow and
        // Deny need the connection and pane ids that live inside the envelope
        // this path just failed to open, so the buttons would have had nowhere
        // to send a keystroke — a tap that does nothing while the user believes
        // they approved, which is the single worst outcome this feature has.
        // The body tap still opens the app, which is what the fallback text asks
        // for. (Found in review by a peer session, 2026-08-24.)
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("an attention old enough to be stale keeps its text and loses its buttons")
    func staleAttentionIsNotActionable() throws {
        let content = UNMutableNotificationContent()
        // `ts` is inside the sealed envelope, so it is the one timestamp a
        // compromised relay cannot move. Replaying a day-old approval prompt
        // must not put a blind Enter under the user's thumb.
        let old = PushSealedBox.Status(
            conn: Self.conn, host: "m1-pro", sess: "work", pane: "%3", agent: "claude",
            state: "attention", title: "Bash: rm -rf build",
            ts: Int(Date().timeIntervalSince1970) - 86_400)
        PushRemoteNotification.apply(old, to: content)

        #expect(content.title == "claude needs you")
        #expect(content.body.contains("rm -rf build"), "the user should still learn it was asked")
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("a replayed done loses its Reply, because Reply types into a live pane")
    func staleDoneIsNotActionable() throws {
        let content = UNMutableNotificationContent()
        // done's only action is Reply, which sends the user's text plus Enter
        // into the pane. Less dangerous than a blind Enter — they are actively
        // composing — but a replayed done still aims that text at whatever the
        // pane holds now, and the hour bounding it was otherwise enforced only by
        // the relay, which is the party this design does not trust.
        let old = PushSealedBox.Status(
            conn: Self.conn, host: "m1-pro", sess: nil, pane: "%3", agent: "claude",
            state: "done", title: nil,
            ts: Int(Date().timeIntervalSince1970) - 3 * 60 * 60)
        PushRemoteNotification.apply(old, to: content)
        #expect(content.title == "✓ claude finished")
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("an attention notification is a signal, not a control")
    func attentionCarriesNoActions() throws {
        // Named for what it now is. The previous name — "keeps its buttons" —
        // outlived two opposite behaviours and at one point asserted the reverse
        // of what it claimed, which is why the assertion and the age sweep live
        // together here.
        for seconds in [TimeInterval(5), 600, 3600] {
            let content = UNMutableNotificationContent()
            let status = PushSealedBox.Status(
                conn: Self.conn, host: "m1-pro", sess: nil, pane: "%3", agent: "claude",
                state: "attention", title: "may I",
                ts: Int(Date().timeIntervalSince1970 - seconds))
            PushRemoteNotification.apply(status, to: content)
            #expect(content.categoryIdentifier.isEmpty, "actionable at \(Int(seconds))s old")
            // What it DOES carry: the pane, so a tap opens the right thing.
            #expect(content.userInfo[AgentNotifications.paneKey] as? String == "%3")
        }
    }

    @Test("a notification the extension could not open routes nowhere")
    func undecryptedCarriesNoTarget() throws {
        // The fallback a user sees when no key opened the envelope: generic text
        // and no pane. Worth pinning, because the delegate's only job now is to
        // resolve a pane from `userInfo` — and it must find nothing here rather
        // than something wrong.
        let pending = UNMutableNotificationContent()
        #expect(PushRemoteNotification.envelope(in: pending.userInfo) == nil)
        #expect(pending.userInfo[AgentNotifications.paneKey] == nil)
    }

    @Test("a notification with no envelope passes through untouched")
    func foreignNotification() throws {
        let content = UNMutableNotificationContent()
        content.title = "Something else"
        let delivered = try #require(deliver(UNNotificationRequest(
            identifier: "x", content: content, trigger: nil)))
        #expect(delivered.title == "Something else")
    }

    @Test("expiring before finishing still delivers the best attempt")
    func timeoutDeliversFallback() throws {
        // iOS calls this when the extension runs out of time. It must hand over
        // the content it has, or the user sees nothing at all.
        var count = 0
        let service = NotificationService()
        let saved = PushPairingStore.read()
        defer { PushPairingStore.write(saved) }
        PushPairingStore.write([])
        service.didReceive(try request()) { _ in count += 1 }
        service.serviceExtensionTimeWillExpire()
        #expect(count == 2)
    }
}

@MainActor
@Suite("Push diagnostics bridge", .serialized, .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
struct PushDiagnosticsTests {

    private func withCleanRing(_ body: () -> Void) {
        let saved = PushDiagnostics.read()
        defer {
            PushDiagnostics.clear()
            for line in saved { PushDiagnostics.record(line.text, now: line.at) }
        }
        PushDiagnostics.clear()
        body()
    }

    @Test("the extension's lines survive into a place the app can read")
    func recordsAndReads() {
        withCleanRing {
            PushDiagnostics.record("woke for a push")
            PushDiagnostics.record("opened a done push from m1-pro")
            let lines = PushDiagnostics.read()
            #expect(lines.count == 2)
            #expect(lines.last?.text.contains("m1-pro") == true)
        }
    }

    @Test("the ring is bounded, so a busy night cannot grow without limit")
    func bounded() {
        withCleanRing {
            for i in 0..<(PushDiagnostics.capacity + 20) {
                PushDiagnostics.record("line \(i)")
            }
            let lines = PushDiagnostics.read()
            #expect(lines.count == PushDiagnostics.capacity)
            // Oldest dropped, newest kept — the recent thing is what is wanted.
            #expect(lines.last?.text == "line \(PushDiagnostics.capacity + 19)")
        }
    }

    @Test("only lines inside the screen's window are offered")
    func windowed() {
        withCleanRing {
            PushDiagnostics.record("ancient", now: Date().addingTimeInterval(-3600))
            PushDiagnostics.record("recent")
            let recent = PushDiagnostics.recent(since: Date().addingTimeInterval(-600))
            #expect(recent.count == 1)
            #expect(recent.first?.text == "recent")
        }
    }

    @Test("a failure to decrypt records the count that names the cause")
    func failureRecordsSecretCount() throws {
        withCleanRing {
            let saved = PushPairingStore.read()
            defer { PushPairingStore.write(saved) }
            PushPairingStore.write([])
            _ = deliver(try! request())
            let text = PushDiagnostics.read().map(\.text).joined(separator: "\n")
            // 0 means "this process could not reach the store"; any other number
            // means "the host is paired to a different install". Without the
            // number the two are indistinguishable, which is why it is in here.
            #expect(text.contains("(0 available)"))
        }
    }

    private func deliver(_ request: UNNotificationRequest) -> UNNotificationContent? {
        var delivered: UNNotificationContent?
        NotificationService().didReceive(request) { delivered = $0 }
        return delivered
    }

    private func request() throws -> UNNotificationRequest {
        let status = PushSealedBox.Status(
            conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301", host: "m1-pro", sess: nil,
            pane: "%3", agent: "claude", state: "attention", title: "t",
            ts: Int(Date().timeIntervalSince1970))
        let sealed = try PushSealedBox.seal(try JSONEncoder().encode(status),
                                           secretHex: PushSealedBoxTests.secret)
        let content = UNMutableNotificationContent()
        content.userInfo = [PushRemoteNotification.envelopeKey:
            ["v": sealed.v, "iv": sealed.iv, "ct": sealed.ct, "mac": sealed.mac]]
        return UNNotificationRequest(identifier: "x", content: content, trigger: nil)
    }
}
