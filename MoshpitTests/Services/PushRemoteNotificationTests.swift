import Foundation
import Testing
import UserNotifications
@testable import Moshpit

/// What the notification service extension does with a push: open the envelope,
/// then make the result indistinguishable from a local notification.
///
/// That equivalence is the whole reason the push path needed no new control code
/// — `AgentNotificationHandler` reads two `userInfo` keys and a category, so
/// getting those three right is what makes lock-screen Allow/Deny work on a
/// notification that arrived while the app was dead.
@Suite("Push notification rendering")
struct PushRemoteNotificationTests {

    static let secret = PushSealedBoxTests.secret
    static let otherSecret = String(repeating: "ab", count: 32)

    static func payload(_ status: PushSealedBox.Status,
                        secret: String = secret) throws -> [AnyHashable: Any] {
        let data = try JSONEncoder().encode(status)
        let sealed = try PushSealedBox.seal(data, secretHex: secret)
        return [
            PushRemoteNotification.envelopeKey: [
                "v": sealed.v, "iv": sealed.iv, "ct": sealed.ct, "mac": sealed.mac,
            ],
        ]
    }

    static func status(state: String = "attention",
                       title: String? = "Bash: rm -rf build",
                       agent: String? = "claude",
                       sess: String? = "work",
                       ts: Int? = nil) -> PushSealedBox.Status {
        PushSealedBox.Status(conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301",
                             host: "m1-pro", sess: sess, pane: "%3", agent: agent,
                             state: state, title: title,
                             // NOW by default. Age no longer changes what is
                             // rendered — nothing is actionable at any age — but
                             // a frozen timestamp would still be a fixture
                             // asserting yesterday. The frozen vector lives in
                             // PushSealedBoxTests, which never renders.
                             ts: ts ?? Int(Date().timeIntervalSince1970))
    }

    // MARK: - Extracting the envelope

    @Test("the envelope is read out of an APNs payload")
    func envelopeExtraction() throws {
        let info = try Self.payload(Self.status())
        let envelope = PushRemoteNotification.envelope(in: info)
        #expect(envelope?.v == 1)
        #expect(envelope?.ct.isEmpty == false)
    }

    @Test("a notification that is not ours yields no envelope")
    func foreignPayloads() {
        #expect(PushRemoteNotification.envelope(in: [:]) == nil)
        // A local Moshpit notification carries these keys and no envelope.
        #expect(PushRemoteNotification.envelope(in: [
            AgentNotifications.connectionKey: UUID().uuidString,
            AgentNotifications.paneKey: "%3",
        ]) == nil)
        // A partial envelope must not decode into something half-valid.
        #expect(PushRemoteNotification.envelope(in: ["mp": ["v": 1, "iv": "00"]]) == nil)
    }

    // MARK: - Trying the device's secrets

    @Test("each stored secret is tried until one authenticates")
    func triesEverySecret() throws {
        let info = try Self.payload(Self.status(), secret: Self.otherSecret)
        let envelope = try #require(PushRemoteNotification.envelope(in: info))

        // Wrong key only: nothing.
        #expect(PushRemoteNotification.open(envelope, secrets: [Self.secret]) == nil)
        // The right key anywhere in the list: found. A MAC failure here is the
        // NORMAL case ("not this host"), never an error to surface.
        let found = PushRemoteNotification.open(
            envelope, secrets: [Self.secret, Self.otherSecret])
        #expect(found?.host == "m1-pro")
        #expect(PushRemoteNotification.open(envelope, secrets: []) == nil)
    }

    // MARK: - Rendering

    @Test("an attention push reads like its local twin and routes like one")
    func attentionRendering() throws {
        let content = UNMutableNotificationContent()
        content.title = "An agent needs you"        // the relay's translated fallback
        content.body = "Open Moshpit to see what it is asking."
        content.categoryIdentifier = "something.the.relay.did.not.send"

        PushRemoteNotification.apply(Self.status(), to: content)

        #expect(content.title == "claude")
        #expect(content.body == "Bash: rm -rf build — m1-pro · work")
        // The two keys that make the existing Allow/Deny path work.
        #expect(content.userInfo[AgentNotifications.connectionKey] as? String
                == "3F2504E0-4F89-11D3-9A0C-0305E82C3301")
        #expect(content.userInfo[AgentNotifications.paneKey] as? String == "%3")
        // No category. Moshpit's notifications carry no action buttons at all
        // any more — the lock screen tells you and takes you there.
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("a done push is marked with the glyph, not a translated sentence")
    func doneRendering() throws {
        let content = UNMutableNotificationContent()
        PushRemoteNotification.apply(Self.status(state: "done", title: nil), to: content)
        // No English is composed here on purpose: String(localized:) in an
        // extension resolves against the EXTENSION's bundle, so a sentence built
        // here would ship untranslated.
        #expect(content.title == "✓ claude")
        #expect(content.body == "m1-pro · work")
        #expect(content.categoryIdentifier.isEmpty)
    }

    @Test("no notification carries an action, whatever its state or age")
    func nothingIsActionable() throws {
        // These assertions have now been written four ways, which is the record
        // worth keeping. First they claimed a push was equivalent to a local
        // notification — right about the code, wrong about the world, because
        // delivery needed a session that is gone by definition. Then: no pushed
        // notification can ever be actionable. Then: actionable again, through a
        // relay that had the host press the key. Now: nothing is actionable,
        // anywhere, and not for a delivery reason.
        //
        // Allow and Deny were a BLIND keystroke. Answering an agent's permission
        // request from a lock screen meant approving something you had not read,
        // in an app whose whole value is that you can read it. The operation did
        // not belong; the three delivery failures were the symptom.
        for state in ["attention", "done", "working"] {
            for age in [TimeInterval(0), -30, -3600] {
                let content = UNMutableNotificationContent()
                content.categoryIdentifier = "left.over.from.somewhere"
                PushRemoteNotification.apply(
                    Self.status(state: state, ts: Int(Date().timeIntervalSince1970 + age)),
                    to: content)
                #expect(content.categoryIdentifier.isEmpty,
                        "\(state) @\(Int(age))s came back actionable")
                // The ids survive — they are what makes a TAP land on the pane.
                #expect(content.userInfo[AgentNotifications.paneKey] as? String == "%3")
            }
        }
    }

    @Test("a missing hook title keeps the fallback title and shows the location")
    func noTitle() throws {
        let content = UNMutableNotificationContent()
        content.title = "An agent needs you"
        PushRemoteNotification.apply(Self.status(title: "   "), to: content)
        #expect(content.title == "claude")
        #expect(content.body == "m1-pro · work")
    }

    @Test("tmux's default numeric session name is not shown as a location")
    func numericSessionIsNotALocation() {
        // Seen on a real lock screen as "mac-mini.lan · 0", which reads as a
        // broken counter. No test caught it because it was technically correct.
        #expect(PushRemoteNotification.location(Self.status(sess: "0")) == "m1-pro")
        #expect(PushRemoteNotification.location(Self.status(sess: "12")) == "m1-pro")
        #expect(PushRemoteNotification.location(Self.status(sess: "  ")) == "m1-pro")
        // A name that says something still says it.
        #expect(PushRemoteNotification.location(Self.status(sess: "work")) == "m1-pro · work")
        #expect(PushRemoteNotification.location(Self.status(sess: "api-2")) == "m1-pro · api-2")
    }

    @Test("a session-less host renders as just the host")
    func noSession() throws {
        let content = UNMutableNotificationContent()
        PushRemoteNotification.apply(Self.status(sess: nil), to: content)
        #expect(content.body == "Bash: rm -rf build — m1-pro")
        #expect(PushRemoteNotification.location(Self.status(sess: "")) == "m1-pro")
    }

    @Test("an agent-less status falls back to the host as the title")
    func noAgent() throws {
        let content = UNMutableNotificationContent()
        PushRemoteNotification.apply(Self.status(agent: nil, sess: nil), to: content)
        #expect(content.title == "m1-pro")
    }

    @Test("existing userInfo is preserved, not replaced")
    func userInfoMerge() throws {
        let content = UNMutableNotificationContent()
        content.userInfo = ["aps": ["category": "x"], "keep": 1]
        PushRemoteNotification.apply(Self.status(), to: content)
        #expect(content.userInfo["keep"] as? Int == 1)
        #expect(content.userInfo["aps"] != nil)
    }
}

/// The last inch of the pairing proof: recognising the self-test when it lands.
///
/// The whole rest of the chain — sender, relay, APNs, extension — can be
/// perfect, and if this returns nil the sheet still times out with "the host
/// sent one, but this screen never saw it arrive". That is not hypothetical:
/// the recognition was deleted once, in a rewrite of the delegate that was
/// about something else entirely, and no test noticed because none pinned it.
@Suite("Self-test recognition")
struct SelfTestRecognitionTests {

    private func selfTestInfo(nonce: String) -> [AnyHashable: Any] {
        let content = UNMutableNotificationContent()
        PushRemoteNotification.apply(
            PushSealedBox.Status(conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301",
                                 host: "m1-pro", sess: nil, pane: "%0",
                                 agent: PushRemoteNotification.selfTestAgent,
                                 state: "done", title: nonce,
                                 ts: Int(Date().timeIntervalSince1970)),
            to: content)
        return content.userInfo
    }

    @Test("a self-test push yields its nonce")
    func recognised() {
        // Built through the REAL rendering path, not a hand-rolled dictionary:
        // if apply() ever renames a userInfo key, this must break with it.
        let nonce = AgentNotificationHandler.selfTestNonce(in: selfTestInfo(nonce: "selftest-abc123"))
        #expect(nonce == "selftest-abc123")
    }

    @Test("ordinary notifications are not self-tests")
    func ordinaryIsNot() {
        let content = UNMutableNotificationContent()
        PushRemoteNotification.apply(
            PushSealedBox.Status(conn: "c", host: "m1", sess: nil, pane: "%1",
                                 agent: "claude", state: "attention", title: "may I",
                                 ts: Int(Date().timeIntervalSince1970)),
            to: content)
        #expect(AgentNotificationHandler.selfTestNonce(in: content.userInfo) == nil)
        #expect(AgentNotificationHandler.selfTestNonce(in: [:]) == nil)
    }
}
