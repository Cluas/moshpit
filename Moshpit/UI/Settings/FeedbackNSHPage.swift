import MessageUI
import MoshpitKit
import SwiftUI
import UIKit

/// Settings › Send Feedback drawn by NSH (`ui/src/feedback.rs`) in the sheet's
/// own stack. The text is typed on the NSH page and kept by the host
/// (``FeedbackModel``) with the two switches; sending stays the host's, as
/// it touches the system: the mail is assembled as the SwiftUI page does and
/// handed to the Mail compose sheet, else a `mailto:` handler, else the
/// address is shown to copy. If its screen can't run, the page says so.
struct FeedbackNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var model = FeedbackModel()
    @State private var failed = false
    /// A prepared mail waiting on the Mail compose sheet.
    @State private var composing: FeedbackCompose?

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack {
                NSHPage(
                    screen: .feedback,
                    attach: { model.attach($0) },
                    onEvent: { if case .feedback(let event) = $0 { handle(event) } },
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
    private func handle(_ event: FeedbackEvent) {
        switch event {
        case .cancel:
            dismiss()
        case .text(let value):
            model.message = value
        case .set(let key, let on):
            model.set(key, on)
        case .send(let text):
            model.message = text
            model.send(
                compose: { composing = FeedbackCompose(mail: $0) },
                openMailto: { url, done in openURL(url) { done($0) } })
        case .copyAddress:
            model.copyAddress()
        case .githubIssue:
            // The redesign's Feedback page (§6.12) has the issue link too.
            openURL(FeedbackChannels.newIssueURL)
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
/// switches, and the send's progress; the state that carries it
/// (``FeedbackState``, `ui/src/feedback.rs`'s own, through the page bridge).
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
    func set(_ key: FeedbackSwitch, _ on: Bool) {
        switch key {
        case .environment: includeEnvironment = on
        case .log: attachLog = on
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
                version: FeedbackMail.versionNumber)
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
        surface?.post(.feedback(state: state(withText: withText)))
    }

    /// What the page shows; the text only when the page should take it.
    func state(withText: Bool = false) -> FeedbackState {
        FeedbackState(
            message: withText ? message : nil, environment: includeEnvironment, log: attachLog,
            preparing: isPreparing, showAddress: showAddress, copied: copied,
            recipient: FeedbackMail.recipient)
    }
}
