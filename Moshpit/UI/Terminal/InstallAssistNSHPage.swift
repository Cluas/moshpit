import MoshpitKit
import SwiftUI
import UIKit

/// What Install Assist offers for a session missing tmux, mosh or herdr:
/// the packages, the command for the host's package manager, and when the
/// re-check has found them — the core's plan (`crates/hostcare`'s
/// `AssistPlan`, which Android runs too), for ``InstallAssistNSHPage``.
@MainActor
struct InstallAssistPlan {
    let session: SessionHub.ActiveSession
    /// From `DegradeNotice.packages`: `["tmux"]`, `["mosh"]` or `["herdr"]`.
    let packages: [String]

    private var tmux: Bool { session.connection.multiplexer == .tmux }
    private var manager: String? { session.capabilities?.packageManager?.rawValue }
    private var value: AssistPlanValue {
        installAssistPlan(packages: packages, tmux: tmux, packageManager: manager)
    }

    var isHerdr: Bool { value.herdr }
    /// What to install, from the CONNECTION rather than the notice's list.
    var installPackages: [String] { value.packages }
    var packageManager: PackageManager? { session.capabilities?.packageManager }
    var command: String? { value.command }
    /// herdr's installer drops the binary in `~/.local/bin`: say Moshpit
    /// looks there.
    var showsNote: Bool { value.showsNote }

    enum Recheck: String, Equatable { case resolved, stillMissing, noChannel }

    /// Probes the host again: resolved once everything the packages cover
    /// is there.
    func recheck() async -> Recheck {
        let caps = await session.recheckCapabilities()
        let answer = installAssistRecheck(packages: packages, tmux: tmux, packageManager: manager,
                                          hasTmux: caps?.hasTmux, hasMoshServer: caps?.hasMoshServer,
                                          hasHerdr: caps?.hasHerdr)
        return Recheck(rawValue: answer) ?? .noChannel
    }

    /// The NSH sheet's message (`{"op": "install", …}`), the core's.
    func message(rechecking: Bool, result: Recheck?, copied: Bool) -> String {
        installAssistMessage(packages: packages, tmux: tmux, packageManager: manager,
                             rechecking: rechecking, result: result?.rawValue, copied: copied)
    }

    /// Types `command` into the live shell, with the newline that runs it:
    /// sudo's prompt and the output stay in view behind the sheet.
    func run(_ command: String) {
        if let data = (command + "\r").data(using: .utf8) {
            session.sendInput(data)
        }
    }
}

/// Install Assist drawn by NSH (`ui/src/terminal/install.rs`) in its sheet
/// (medium and large, the grabber, the screen's background): the message is
/// the core's, from the plan and the sheet's own state (a re-check running,
/// its result, the command copied), posted as it changes; what the person asks runs
/// the plan. If its screen can't run, the sheet says so (``NSHPageFailed``).
struct InstallAssistNSHPage: View {
    let session: SessionHub.ActiveSession
    let packages: [String]
    let onReconnect: () -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var model = InstallAssistNSHModel()
    @State private var failed = false
    @State private var rechecking = false
    @State private var result: InstallAssistPlan.Recheck?
    @State private var copied = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let state = plan.message(rechecking: rechecking, result: result, copied: copied)
            NSHPage(
                screen: .installAssist,
                attach: { model.attach($0, current: state) },
                onEvent: { if case .installAssist(let event) = $0 { handle(event) } },
                onFailure: { failed = true })
                .ignoresSafeArea(.all)
                .onAppear { model.update(state) }
                .onChange(of: state) { _, new in model.update(new) }
                .background(Ink.pageBG)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
    }

    private var plan: InstallAssistPlan { InstallAssistPlan(session: session, packages: packages) }

    private func handle(_ event: InstallAssistEvent) {
        switch event {
        case .done:
            dismiss()
        case .run:
            if let command = plan.command { plan.run(command) }
            dismiss()
        case .copy:
            if let command = plan.command {
                UIPasteboard.general.string = command
                copied = true
            }
        case .recheck:
            guard !rechecking else { return }
            rechecking = true
            result = nil
            let plan = plan
            Task {
                let found = await plan.recheck()
                rechecking = false
                result = found
            }
        case .reconnect:
            onReconnect()
            dismiss()
        }
    }
}

/// What the NSH Install Assist was last told, so a surface made again
/// starts from it.
@MainActor
final class InstallAssistNSHModel {
    private weak var surface: NSHSurface?
    private var message: String?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let message { surface.post(json: message) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's state as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current message: String) {
        if self.message == nil {
            self.surface = nil
            update(message)
        }
        attach(surface)
    }

    func update(_ message: String) {
        guard message != self.message else { return }
        self.message = message
        surface?.post(json: message)
    }
}
