import MessageUI
import SwiftUI

extension FeedbackMail {
    /// "1.0.3 (398)" — the subject's version, without the build stamp the
    /// footer line carries (the environment block includes the full line).
    static let versionNumber: String = {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(version) (\(build))"
    }()
}

extension FeedbackMail.Environment {
    /// The running build and device, read once per send.
    @MainActor static func current() -> Self {
        var systemInfo = utsname()
        uname(&systemInfo)
        let machine = withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: Int(_SYS_NAMELEN)) { String(cString: $0) }
        }
        let device = UIDevice.current
        return .init(
            app: SettingsScreen.versionLine,
            system: "\(device.systemName) \(device.systemVersion)",
            device: machine,
            language: Locale.preferredLanguages.first ?? Locale.current.identifier)
    }
}

/// `MFMailComposeViewController` in SwiftUI clothes: pre-filled from a
/// `FeedbackMail`, the log as a text attachment, one callback on finish —
/// what the NSH Send Feedback page presents.
struct MailComposer: UIViewControllerRepresentable {
    let mail: FeedbackMail
    let onFinish: (MFMailComposeResult) -> Void

    func makeUIViewController(context: Context) -> MFMailComposeViewController {
        let controller = MFMailComposeViewController()
        controller.mailComposeDelegate = context.coordinator
        controller.setToRecipients([FeedbackMail.recipient])
        controller.setSubject(mail.subject)
        controller.setMessageBody(mail.body, isHTML: false)
        if let log = mail.log, let data = log.data(using: .utf8) {
            controller.addAttachmentData(data, mimeType: "text/plain", fileName: FeedbackMail.logFilename)
        }
        return controller
    }

    func updateUIViewController(_ uiViewController: MFMailComposeViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(onFinish: onFinish) }

    final class Coordinator: NSObject, MFMailComposeViewControllerDelegate {
        let onFinish: (MFMailComposeResult) -> Void
        init(onFinish: @escaping (MFMailComposeResult) -> Void) { self.onFinish = onFinish }

        func mailComposeController(_ controller: MFMailComposeViewController,
                                   didFinishWith result: MFMailComposeResult, error: Error?) {
            onFinish(result)
        }
    }
}
