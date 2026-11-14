import MessageUI
import MoshpitKit
import SwiftUI
import UIKit

/// Settings › Send Feedback: the SwiftUI page (``FeedbackView``), or the NSH
/// one while its switch is on (``NSHScreen/feedback``). With the switch off
/// the SwiftUI page is all there is: no NSH code runs.
struct FeedbackPage: View {
    var body: some View {
        if Self.showsNSH {
            FeedbackNSHPage()
        } else {
            FeedbackView()
        }
    }

    private static var showsNSH: Bool {
        #if DEBUG
        if CaptureScreen.requested == .feedbackNSH { return NSHSurface.available }
        #endif
        return NSHScreen.feedback.isOn
    }
}

/// Settings › Send Feedback drawn by NSH (`ui/src/feedback.rs`) in the sheet's
/// own stack. The text is typed on the NSH page and kept by the host
/// (``FeedbackModel``) with the two switches; sending stays the host's, as
/// it touches the system: the mail is assembled as the SwiftUI page does and
/// handed to the Mail compose sheet, else a `mailto:` handler, else the
/// address is shown to copy. If NSH cannot run here, the SwiftUI page shows.
struct FeedbackNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var model = FeedbackModel()
    @State private var failed = false
    /// A prepared mail waiting on the Mail compose sheet.
    @State private var composing: FeedbackCompose?

    var body: some View {
        if failed {
            FeedbackView()
        } else {
            NavigationStack {
                NSHPage(
                    screen: .feedback,
                    attach: { model.attach($0) },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("Feedback")
            }
            .sheet(item: $composing) { request in
                MailComposer(mail: request.mail) { result in
                    composing = nil
                    // Sent: done. Saved as a draft or cancelled: the text
                    // is still here to edit.
                    if result == .sent { dismiss() }
                }
                .ignoresSafeArea()
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "cancel":
            dismiss()
        case "text":
            model.message = message["value"] as? String ?? ""
        case "set":
            if let key = message["key"] as? String, let on = message["value"] as? Bool { model.set(key, on) }
        case "send":
            if let text = message["message"] as? String { model.message = text }
            model.send(
                compose: { composing = FeedbackCompose(mail: $0) },
                openMailto: { url, done in openURL(url) { done($0) } })
        case "copy-address":
            model.copyAddress()
        case "github-issue":
            // The redesign's Feedback page (§6.12) has the issue link too.
            openURL(FeedbackChannels.newIssueURL)
        default:
            break
        }
    }
}

/// A prepared mail for the compose sheet.
struct FeedbackCompose: Identifiable {
    let id = UUID()
    let mail: FeedbackMail
}

/// What the NSH Feedback page shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule): the text typed so far, the two
/// switches, and the send's progress; the message that carries it
/// (`{"op": "feedback", …}`, `ui/src/feedback.rs`'s `Message`).
@MainActor
final class FeedbackModel {
    /// The text typed so far, as the page last told it.
    var message = ""
    private(set) var includeEnvironment = true
    private(set) var attachLog = false
    /// A send is being prepared (the log read): Send waits.
    private(set) var isPreparing = false
    /// Neither Mail nor a `mailto:` handler took it.
    private(set) var showAddress = false
    private(set) var copied = false
    private weak var surface: NSHSurface?

    /// Whether this device can send with Mail (a test's stand-in).
    var canSendMail: () -> Bool = { MFMailComposeViewController.canSendMail() }
    /// The recent log, when it is attached (a test's stand-in).
    var transcript: () async -> String? = { await DiagnosticsLog.transcript() }
    /// Where Copy Address puts the address (a test's stand-in).
    var copy: (String) -> Void = { UIPasteboard.general.string = $0 }

    /// `surface` shows the page from now on, starting with the text typed so
    /// far.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        post(withText: true)
    }

    /// The user flipped switch `key`.
    func set(_ key: String, _ on: Bool) {
        switch key {
        case "environment": includeEnvironment = on
        case "log": attachLog = on
        default: return
        }
        post()
    }

    /// Assembles the mail (reading the log off the main actor when it is
    /// attached), then hands it to Mail, else to a `mailto:` handler, else
    /// shows the address: the SwiftUI page's `send()`. A second Send while
    /// one is prepared does nothing.
    func send(compose: @escaping (FeedbackMail) -> Void,
              openMailto: @escaping (URL, @escaping (Bool) -> Void) -> Void) {
        guard !isPreparing else { return }
        isPreparing = true
        post()
        Task {
            let log = attachLog ? await transcript() : nil
            let mail = FeedbackMail(
                message: message,
                environment: includeEnvironment ? .current() : nil,
                log: log,
                version: FeedbackView.versionNumber)
            isPreparing = false
            if canSendMail() {
                compose(mail)
            } else if let url = mail.mailtoURL {
                openMailto(url) { [weak self] accepted in
                    guard !accepted, let self else { return }
                    self.showAddress = true
                    self.post()
                }
            } else {
                showAddress = true
            }
            post()
        }
    }

    /// Copy Address.
    func copyAddress() {
        copy(FeedbackMail.recipient)
        copied = true
        post()
    }

    private func post(withText: Bool = false) {
        surface?.post(Self.message(
            text: withText ? message : nil, environment: includeEnvironment, log: attachLog,
            preparing: isPreparing, showAddress: showAddress, copied: copied))
    }

    /// The screen's message; `text` only when the page should take it.
    nonisolated static func message(text: String?, environment: Bool, log: Bool, preparing: Bool,
                                    showAddress: Bool, copied: Bool) -> [String: Any] {
        var message: [String: Any] = [
            "op": "feedback",
            "environment": environment,
            "log": log,
            "preparing": preparing,
            "showAddress": showAddress,
            "copied": copied,
            "recipient": FeedbackMail.recipient,
        ]
        if let text { message["message"] = text }
        return message
    }
}
