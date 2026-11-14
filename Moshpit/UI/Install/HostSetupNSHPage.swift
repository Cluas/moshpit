import SwiftUI

/// Where a Host Setup screen's page lives: the core's machine
/// (``HostCare``, `crates/hostcare`), or in a DEBUG demo one answering from
/// a canned host (``HostSetupDemo``). The page reads its message from it
/// whenever it changes, and hands it what the person asks for.
@MainActor
protocol HostSetupSource: AnyObject {
    /// Open a page on `connection`'s host (and read the host); its id.
    func openSetup(_ connection: ServerConnection, testPane: String?) -> UInt64
    func closeSetup(_ page: UInt64)
    /// `{"op": "host-setup", …}` for `page` in the app's language. Read in
    /// a view's body, it is observed: it changes as the page does.
    func setupMessage(_ page: UInt64) -> String?
    /// What the page told the host (`{"ev": …}`).
    func setupEvent(_ page: UInt64, _ ev: String, agent: String?)
}

extension HostCare: HostSetupSource {}

/// Settings › Set up this host, drawn by NSH (`ui/src/host_setup.rs`): the
/// agent hooks and the push pairing on the connected host, each with the
/// proof that it works. Everything it says and does is the core's
/// (``HostSetupSource``); this page posts the message the core wrote and
/// passes on the buttons. Its header is the page's own: no navigation bar.
/// If NSH can't run here, what the phone knows offline shows instead
/// (``OfflineHostInfoView``).
struct HostSetupPage: View {
    let connection: ServerConnection
    /// The tmux pane to fire the hook self-test into, when there is one.
    let testPane: String?
    var source: any HostSetupSource = HostCare.shared

    var body: some View {
        HostSetupNSHPage(connection: connection, testPane: testPane, source: source)
    }
}

struct HostSetupNSHPage: View {
    let connection: ServerConnection
    let testPane: String?
    let source: any HostSetupSource

    @Environment(\.dismiss) private var dismiss
    @State private var failed = false
    @State private var screen = HostSetupScreenModel()
    @State private var page: UInt64?

    var body: some View {
        if failed {
            OfflineHostInfoView()
        } else {
            let message = page.flatMap { source.setupMessage($0) }
            NSHPage(
                screen: .hostSetup,
                attach: { screen.attach($0, current: message) },
                onMessage: handle,
                onFailure: { failed = true })
                .nshPage()
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .onAppear {
                    if page == nil { page = source.openSetup(connection, testPane: testPane) }
                    screen.update(message)
                }
                .onChange(of: message) { _, new in screen.update(new) }
                .onDisappear {
                    if let page { source.closeSetup(page) }
                }
        }
    }

    /// What the screen tells the host: the page's buttons, to the core.
    private func handle(_ message: [String: Any]) {
        guard let ev = message["ev"] as? String else { return }
        if ev == "done" {
            dismiss()
            return
        }
        guard let page else { return }
        source.setupEvent(page, ev, agent: message["id"] as? String)
    }
}

/// What the NSH Set up this host page was last told, so a surface made
/// again (``NSHPage``'s third rule) starts from it.
@MainActor
final class HostSetupScreenModel {
    private(set) var message: String?
    private weak var surface: NSHSurface?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let message { surface.post(json: message) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's setup message as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current message: String?) {
        if self.message == nil {
            self.surface = nil
            update(message)
        }
        attach(surface)
    }

    func update(_ message: String?) {
        guard let message, message != self.message else { return }
        self.message = message
        surface?.post(json: message)
    }
}
