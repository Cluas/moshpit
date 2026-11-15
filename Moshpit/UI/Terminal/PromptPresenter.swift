import SwiftUI
import UIKit
import UserNotifications

// What a connect asks the person — a host key, a key's passphrase, the
// server's sign-in question — presented by the app, not by the page that
// started the session (B632).
//
// Home presented the questions of the sessions it had started, until their
// first connect; the terminal presented its own session's. A reconnect's
// question (an OTP, every reconnect since B563; a changed host key) while
// the person was on Home with that terminal closed had no presenter: the
// dial waited on the person, the session read "reconnecting", nothing
// asked. A question under another sheet (Install Assist's reconnect) could
// not show either: a view covered by a sheet presents nothing.
//
// Now every session's question goes through `PromptCenter`, and one
// presenter shows it in a window of its own above the app's
// (`PromptWindowController`), over whatever is frontmost, a sheet included.
// While the app is in the background, a local notification says a
// connection waits on the person (`PromptNotifier`, posted by the session's
// driver as the question comes); opening the app shows the question. What
// is presented is E6c's: the NSH host-key sheet and the system's alerts
// (`connectPrompts(hostKey:passphrase:question:)`).

/// The questions every session asks now, one of each kind at a time, in a
/// stable order (the connection's id), so one waits behind another.
@MainActor
struct PromptCenter {
    let sessions: () -> [SessionHub.ActiveSession]

    init(hub: SessionHub) {
        sessions = { Array(hub.sessions.values) }
    }

    init(sessions: @escaping () -> [SessionHub.ActiveSession]) {
        self.sessions = sessions
    }

    private var asking: [SessionHub.ActiveSession] {
        sessions().sorted { $0.connection.id.uuidString < $1.connection.id.uuidString }
    }

    var hostKey: TerminalViewModel.HostKeyPrompt? {
        asking.lazy.compactMap { $0.viewModel.hostKeyPrompt }.first
    }

    var passphrase: TerminalViewModel.KeyPassphrasePrompt? {
        asking.lazy.compactMap { $0.viewModel.keyPassphrasePrompt }.first
    }

    var question: TerminalViewModel.SignInQuestion? {
        asking.lazy.compactMap { $0.viewModel.signInQuestion }.first
    }

    /// Whether anything waits on the person.
    var pending: Bool {
        hostKey != nil || passphrase != nil || question != nil
    }
}

/// A question waiting while the app is in the background, as its local
/// notification.
struct PromptNotice: Equatable {
    let id: String
    let body: String

    enum Kind: String {
        case hostKey, passphrase, question
    }

    /// The notification's id for connection `connection`'s `kind` of question.
    static func id(_ connection: UUID, _ kind: Kind) -> String {
        "moshpit.prompt.\(connection.uuidString).\(kind.rawValue)"
    }

    static func hostKey(connection: UUID, host: String, changed: Bool) -> PromptNotice {
        PromptNotice(
            id: id(connection, .hostKey),
            body: changed
                ? String(localized: "\(host)'s host key has changed. Open Moshpit to decide.")
                : String(localized: "Confirm \(host)'s host key to finish connecting."))
    }

    static func passphrase(connection: UUID, host: String) -> PromptNotice {
        PromptNotice(
            id: id(connection, .passphrase),
            body: String(localized: "Enter the passphrase of the key for \(host) to finish connecting."))
    }

    static func question(connection: UUID, host: String) -> PromptNotice {
        PromptNotice(
            id: id(connection, .question),
            body: String(localized: "\(host) asks a question before you can sign in. Open Moshpit to answer."))
    }
}

/// The background notifications of questions waiting (``PromptNotice``):
/// posted as a question comes while the app is in the background, taken
/// back when it is answered or withdrawn, and all taken back when the app
/// comes forward, where the questions show themselves. Opening the
/// notification opens the app, and so the question.
@MainActor
final class PromptNotifier {
    static let shared = PromptNotifier()

    private var posted: Set<String> = []
    private let background: () -> Bool
    private let add: (UNNotificationRequest) -> Void
    private let remove: ([String]) -> Void

    init(background: @escaping () -> Bool = { UIApplication.shared.applicationState != .active },
         add: @escaping (UNNotificationRequest) -> Void = { UNUserNotificationCenter.current().add($0) },
         remove: @escaping ([String]) -> Void = { ids in
             UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ids)
             UNUserNotificationCenter.current().removePendingNotificationRequests(withIdentifiers: ids)
         }) {
        self.background = background
        self.add = add
        self.remove = remove
    }

    /// A question came: said, if the app can't show it now.
    func asked(_ notice: PromptNotice) {
        guard background(), !posted.contains(notice.id) else { return }
        let content = UNMutableNotificationContent()
        content.body = notice.body
        content.sound = .default
        content.interruptionLevel = .timeSensitive
        add(UNNotificationRequest(identifier: notice.id, content: content, trigger: nil))
        posted.insert(notice.id)
    }

    /// The question was answered or withdrawn.
    func answered(_ id: String) {
        guard posted.remove(id) != nil else { return }
        remove([id])
    }

    /// The app came forward: the questions show themselves.
    func foreground() {
        guard !posted.isEmpty else { return }
        remove(Array(posted))
        posted.removeAll()
    }

    var postedIds: Set<String> { posted }
}

/// Shows the prompt window while a question waits, from the app's own
/// window (always on screen while the app is). Hung off the main scene.
struct PromptWatcher: ViewModifier {
    let center: PromptCenter

    func body(content: Content) -> some View {
        content
            .onChange(of: center.pending, initial: true) { _, pending in
                if pending {
                    PromptWindowController.shared.show()
                } else {
                    PromptWindowController.shared.hideWhenIdle()
                }
            }
    }
}

/// The presenter: E6c's sheet and alerts, fed by ``PromptCenter``, in the
/// prompt window — only while it is up, so nothing is presented from a
/// hidden window.
struct PromptOverlay: View {
    let center: PromptCenter
    @Environment(AppSettings.self) private var settings

    var body: some View {
        let up = PromptWindowController.shared.visible
        Color.clear
            .ignoresSafeArea()
            .background(WindowAppearance(style: settings.appearance.interfaceStyle))
            .connectPrompts(
                hostKey: { up ? center.hostKey : nil },
                passphrase: { up ? center.passphrase : nil },
                question: { up ? center.question : nil })
    }
}

/// The prompt window: above the app's own, so a question shows over any
/// sheet; hidden while nothing is asked. A touch on its empty background
/// goes to the window under it.
final class PromptWindow: UIWindow {
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === rootViewController?.view ? nil : hit
    }
}

/// The one prompt window, in the scene the app runs in.
@MainActor
@Observable
final class PromptWindowController {
    static let shared = PromptWindowController()

    /// Above the app's windows and the sheets presented in them, under the
    /// system's alert level.
    static let level = UIWindow.Level.alert - 1

    /// Whether the window is up: what ``PromptOverlay`` presents from.
    private(set) var visible = false

    @ObservationIgnored private var window: PromptWindow?
    /// A question waits: shown as soon as the window is made, if it came
    /// first.
    @ObservationIgnored private var wanted = false

    /// Makes the window in `scene` once, with `root` as what it shows.
    func attach(to scene: UIWindowScene, root: () -> AnyView) {
        if let window {
            if window.windowScene !== scene, scene.activationState == .foregroundActive {
                window.windowScene = scene
            }
            return
        }
        let window = PromptWindow(windowScene: scene)
        window.windowLevel = Self.level
        window.backgroundColor = .clear
        let host = UIHostingController(rootView: root())
        host.view.backgroundColor = .clear
        window.rootViewController = host
        window.isHidden = true
        self.window = window
        if wanted { show() }
    }

    /// Up, and the key window, so a question's field gets the keyboard.
    func show() {
        wanted = true
        guard let window, window.isHidden else { return }
        window.isHidden = false
        window.makeKey()
        visible = true
    }

    /// Hidden once nothing is presented from it (an alert or the sheet may
    /// still be on its way out), the key window handed back to the app's.
    func hideWhenIdle() {
        wanted = false
        guard let window, !window.isHidden else { return }
        if window.rootViewController?.presentedViewController != nil {
            Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(300))
                // A question that came meanwhile keeps it up.
                if !self.wanted { self.hideWhenIdle() }
            }
            return
        }
        visible = false
        window.isHidden = true
        window.windowScene?.windows.first { $0 !== window && !$0.isHidden }?.makeKey()
    }
}

/// Puts the prompt window in the scene of the view it sits in.
struct PromptWindowAnchor: UIViewRepresentable {
    let root: () -> AnyView

    func makeUIView(context: Context) -> Anchor { Anchor(root: root) }
    func updateUIView(_ view: Anchor, context: Context) {}

    final class Anchor: UIView {
        let root: () -> AnyView

        init(root: @escaping () -> AnyView) {
            self.root = root
            super.init(frame: .zero)
            isUserInteractionEnabled = false
            isHidden = true
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            guard let scene = window?.windowScene, !(window is PromptWindow) else { return }
            PromptWindowController.shared.attach(to: scene, root: root)
        }
    }
}
