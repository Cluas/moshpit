import MessageUI
import SwiftUI
import UIKit

/// Settings → Feedback. A note to support@cluas.eu.org, composed in the
/// user's own Mail app: the compose sheet opens pre-filled and the mail
/// leaves only when they tap Send there. With no Mail account, the same text
/// goes to whatever handles `mailto:`; failing that, the address is shown to
/// copy. The build/device facts and the recent log are opt-in switches the
/// user can see — a bug report without them is guesswork, but including them
/// has to be their choice, and nothing here is sent on its own.
struct FeedbackView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var message = ""
    @State private var includeEnvironment = true
    @State private var attachLog = false
    @State private var isPreparing = false
    /// A prepared mail waiting on the Mail compose sheet.
    @State private var composing: ComposeRequest?
    /// Neither Mail nor a `mailto:` handler took it: show the address instead.
    @State private var showAddress = false
    @State private var copied = false
    @FocusState private var editing: Bool

    private struct ComposeRequest: Identifiable {
        let id = UUID()
        let mail: FeedbackMail
    }

    var body: some View {
        NavigationStack {
            Form {
                FormGroup(title: "MESSAGE") {
                    TextField("What happened, or what would help?", text: $message, axis: .vertical)
                        .lineLimit(4...12)
                        .font(.body)
                        .foregroundStyle(Ink.primary)
                        .focused($editing)
                        .accessibilityIdentifier("feedback-message")
                }
                FormGroup(
                    title: "INCLUDE",
                    footer: "The log is the app’s own from the last 30 minutes — the same lines as Recent Log. Read it there first if anything in it should stay private."
                ) {
                    ToggleRow(label: "Device and app info",
                              subtitle: "Version, system, device model, language",
                              identifier: "feedback-environment",
                              isOn: $includeEnvironment)
                    ToggleRow(label: "Recent log",
                              identifier: "feedback-log",
                              isOn: $attachLog)
                }
                if showAddress {
                    FormGroup(footer: "Mail isn’t set up on this device. Copy the address and write from anywhere.") {
                        HStack(spacing: 10) {
                            Text(verbatim: FeedbackMail.recipient)
                                .font(Face.mono(14))
                                .foregroundStyle(Ink.primary)
                                .textSelection(.enabled)
                            Spacer()
                            Button(copied ? "Copied" : "Copy Address") {
                                UIPasteboard.general.string = FeedbackMail.recipient
                                copied = true
                            }
                            .foregroundStyle(Ink.accent)
                            .accessibilityIdentifier("feedback-copy-address")
                        }
                    }
                }
            }
            .moshpitForm()
            .navigationTitle("Feedback")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Send") { send() }
                        .foregroundStyle(Ink.accent).fontWeight(.semibold)
                        .disabled(isPreparing)
                        .accessibilityIdentifier("feedback-send")
                }
            }
        }
        .preferredColorScheme(.dark)
        .sheet(item: $composing) { request in
            MailComposer(mail: request.mail) { result in
                composing = nil
                // Sent: the feedback screen's job is done. Saved as a draft or
                // cancelled: stay, the text is still here to edit.
                if result == .sent { dismiss() }
            }
            .ignoresSafeArea()
        }
        .onAppear { editing = true }
    }

    /// Assemble the mail (reading the log off the main actor when attached),
    /// then hand it to Mail, else to a `mailto:` handler, else show the
    /// address. `isPreparing` keeps a second tap from composing twice.
    private func send() {
        guard !isPreparing else { return }
        isPreparing = true
        editing = false
        Task {
            let log = attachLog ? await DiagnosticsLog.transcript() : nil
            let mail = FeedbackMail(
                message: message,
                environment: includeEnvironment ? .current() : nil,
                log: log,
                version: Self.versionNumber)
            isPreparing = false
            if MFMailComposeViewController.canSendMail() {
                composing = ComposeRequest(mail: mail)
            } else if let url = mail.mailtoURL {
                openURL(url) { accepted in
                    if !accepted { showAddress = true }
                }
            } else {
                showAddress = true
            }
        }
    }

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
/// `FeedbackMail`, the log as a text attachment, one callback on finish.
private struct MailComposer: UIViewControllerRepresentable {
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
