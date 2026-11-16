import Foundation
import MoshpitKit

extension AppLanguage {
    /// Record the language Moshpit shows (``AppLanguage/record(_:suiteName:)``)
    /// — only once the defaults can be trusted. Returns whether it did.
    ///
    /// The record is a write into the App Group's preferences, which are
    /// sealed until the first unlock just as the app's are. A write into a
    /// domain that reads as empty can replace the file with only that key:
    /// the push switches the notification service reads, the widget's
    /// snapshot, gone with it. So it waits for the readiness the stores wait
    /// for (the core's: the sentinel the stores write after their first
    /// trusted read, or `protectedData`), and the app asks again whenever it
    /// comes forward.
    @MainActor @discardableResult
    static func recordIfTrusted(protectedData: @escaping () -> Bool = { ProtectedDataFlag.app.isAvailable },
                                defaults: UserDefaults = .standard,
                                language: String = AppLanguage.own,
                                suiteName: String = AppGroup.id) -> Bool {
        let host = RustPreferencesHost(defaults: defaults, available: protectedData)
        guard storeDefaultsReadable(host: host, mode: .system) else { return false }
        record(language, suiteName: suiteName)
        return true
    }
}
