import Foundation

/// What the in-app feedback screen hands to Mail: recipient, subject, body,
/// and an optional log attachment. Pure so the assembly can be pinned by
/// tests; the screen only decides which pieces the user switched on.
///
/// Nothing here leaves the phone by itself. The mail is composed in the
/// user's own Mail app (or handed to whatever handles `mailto:`), and goes
/// out only when they tap Send there — which is what keeps the App Store
/// privacy label at "Data Not Collected".
struct FeedbackMail: Equatable {
    static let recipient = "support@cluas.eu.org"

    /// Device and build facts a bug report is useless without. Kept as a
    /// struct so a test can pass fixed values instead of reading the device.
    struct Environment: Equatable {
        /// `SettingsScreen.versionLine`: "Moshpit 1.0.3 (398) · <stamp>".
        var app: String
        /// "iOS 26.2", "iPadOS 26.2".
        var system: String
        /// Hardware identifier, e.g. "iPhone18,2" — the model name a crash log
        /// would show, not the marketing name.
        var device: String
        /// The language the UI runs in, so a report in Chinese about a Chinese
        /// screen is read as such.
        var language: String

        var lines: [String] {
            ["App: \(app)", "System: \(system)", "Device: \(device)", "Language: \(language)"]
        }
    }

    let subject: String
    let body: String
    /// The last stretch of the app's own log, when the user chose to attach it.
    let log: String?

    static let logFilename = "moshpit-log.txt"
    /// `mailto:` cannot carry an attachment, so the log rides inline there —
    /// and a URL has to stay a URL, so only its tail goes.
    static let inlineLogLimit = 6_000

    /// - Parameters:
    ///   - message: what the user typed; whitespace-trimmed, may be empty.
    ///   - environment: device facts, nil when the user switched them off.
    ///   - log: recent log text, nil when the user did not attach it.
    ///   - version: the marketing version for the subject, e.g. "1.0.3 (398)".
    init(message: String, environment: Environment?, log: String?, version: String) {
        subject = "Moshpit feedback · \(version)"
        var parts: [String] = []
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        parts.append(text.isEmpty ? "" : text)
        if let environment {
            parts.append(environment.lines.joined(separator: "\n"))
        }
        body = parts.filter { !$0.isEmpty }.joined(separator: "\n\n") + "\n"
        let trimmedLog = log?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.log = (trimmedLog?.isEmpty ?? true) ? nil : trimmedLog
    }

    /// The `mailto:` fallback for a phone without Mail configured. The log
    /// is appended inline, tail-truncated to keep the URL within what other
    /// mail apps accept.
    var mailtoURL: URL? {
        var text = body
        if let log {
            let tail = log.count > Self.inlineLogLimit
                ? "…" + String(log.suffix(Self.inlineLogLimit))
                : log
            text += "\n--- log ---\n" + tail + "\n"
        }
        var components = URLComponents()
        components.scheme = "mailto"
        components.path = Self.recipient
        components.queryItems = [
            URLQueryItem(name: "subject", value: subject),
            URLQueryItem(name: "body", value: text),
        ]
        // URLComponents leaves "+" alone; a mail client decoding
        // application/x-www-form-urlencoded would read it as a space.
        components.percentEncodedQuery = components.percentEncodedQuery?
            .replacingOccurrences(of: "+", with: "%2B")
        return components.url
    }
}
