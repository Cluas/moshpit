import Foundation

/// The two notification preferences a PUSHED notification must honor, readable
/// from the notification service extension.
///
/// The app's settings live in `UserDefaults.standard`, which is a per-process
/// container the extension cannot see — and for months that gap was a broken
/// promise: Settings said "Show detail on lock screen … off keeps it private"
/// and the LOCAL surfaces obeyed, while a pushed notification rendered the
/// command line anyway, because the only process awake when it landed had no
/// way to read the switch. This mirror closes the gap the same way every other
/// app↔extension fact travels (`PushPairingStore`, `PushStanding`,
/// `PushDiagnostics`): through the App Group.
///
/// `AppSettings` writes through on every change and once at launch; the
/// extension only ever reads. Defaults are `true` on both because that is what
/// the switches themselves default to — a mirror that has never been written
/// (fresh install, extension racing first launch) must behave like the
/// untouched settings screen, not like a stricter one.
public enum PushPrefs {
    public static let suite = PushPairingStore.appGroup
    public static let detailKey = "moshpit.push.showDetail"
    public static let soundKey = "moshpit.push.sound"

    public struct Values: Equatable {
        /// Render what the agent is running/asking (the hook title) in the
        /// notification body. Off: the body still names where ("m1-pro · pit"),
        /// never what.
        public var showDetail: Bool
        /// Whether an alert that has earned a sound (an attention edge, a long
        /// turn finishing) may actually play one.
        public var sound: Bool

        public static let `default` = Values(showDetail: true, sound: true)

        public init(showDetail: Bool, sound: Bool) {
            self.showDetail = showDetail; self.sound = sound
        }
    }

    public static func read(suiteName: String = PushPrefs.suite) -> Values {
        guard let d = UserDefaults(suiteName: suiteName) else { return .default }
        return Values(showDetail: d.object(forKey: detailKey) as? Bool ?? true,
                      sound: d.object(forKey: soundKey) as? Bool ?? true)
    }

    public static func write(showDetail: Bool, sound: Bool,
                             suiteName: String = PushPrefs.suite) {
        guard let d = UserDefaults(suiteName: suiteName) else { return }
        d.set(showDetail, forKey: detailKey)
        d.set(sound, forKey: soundKey)
    }
}
