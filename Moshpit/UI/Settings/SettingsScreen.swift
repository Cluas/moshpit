import SwiftUI
import UIKit
import MoshpitKit

/// Settings: SwiftUI's stack around the NSH hub (``SettingsNSHPage``,
/// `ui/src/settings.rs`). The hub's rows ask for a page (``open(_:)``),
/// pushed here, or a sheet, presented here; Set up a host routes through
/// ``go(_:)``. The SwiftUI form it used to fall back to is gone (D5).
struct SettingsScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL

    /// The saved connections, connected or not: Set up a host picks one
    /// (``HostSetupRoute``).
    var hosts: [SettingsHost] = []
    /// A connection's live session, which its Host Setup runs over.
    var session: (UUID) -> SessionHub.ActiveSession? = { _ in nil }
    /// Connect a saved connection and open it (Home closes Settings first).
    var onConnect: (UUID) -> Void = { _ in }

    /// Some connection is live: Set up a host can inspect a host now.
    private var liveSession: Bool { hosts.contains(where: \.live) }

    #if DEBUG
    @State private var showShortcuts = CaptureScreen.requested.map(CaptureScreen.opensShortcuts.contains) ?? false
    @State private var showKeys = CaptureScreen.requested.map(CaptureScreen.opensSSHKeys.contains) ?? false
    // The galleries and the other pushed pages are pushed, not presented, so
    // a capture run pushes them through these rather than tapping a row it
    // has to find first. Set once the sheet is on screen (see the .task
    // below): a push requested while the sheet is still presenting is
    // dropped by SwiftUI, and the capture run then filed a picture of Home
    // under the gallery's name.
    @State private var pushTheme = false
    /// A page of Settings' stack a capture run pushes
    /// (``CaptureScreen/pushedInSettings``).
    @State private var pushCapture: CaptureScreen?
    #else
    @State private var showShortcuts = false
    @State private var showKeys = false
    @State private var pushTheme = false
    #endif
    /// The host Set up a host opened, over its live session.
    @State private var settingUp: HostSetupTarget?
    /// A connection picked that isn't connected: asked to connect first.
    @State private var asking: SettingsHost?
    @State private var showFeedback = false
    /// The hub's screen could not run here: the page says so.
    @State private var nshFailed = false
    /// A page the NSH page's rows asked for, pushed on Settings' stack.
    @State private var pushed: SettingsPage?
    /// A page of the NSH hub (Appearance, Terminal…), pushed on the stack.
    @State private var pushedPart: SettingsPart?
    /// Whether the MIIT filing line is shown; resolved once per appearance from
    /// the App Store storefront (see MainlandStorefront).
    @State private var showsFilingNumber = false

    var body: some View {
        if nshFailed {
            // The hub's screen could not run: the page says so, with
            // Cancel to close Settings.
            NSHSheetFailed { nshFailed = false }
        } else {
            stack
        }
    }

    /// Settings' stack: the NSH hub at its root, the pages its rows ask
    /// for pushed on it, the sheets presented over it. The hub draws its
    /// own bar; the stack's title is what a pushed page's back button says.
    private var stack: some View {
        NavigationStack {
            SettingsNSHPage(
                liveSession: liveSession,
                showsFilingNumber: showsFilingNumber,
                onOpen: open,
                onDone: { dismiss() },
                onFailure: { nshFailed = true })
            .navigationDestination(isPresented: $pushTheme) { ThemeGalleryNSHPage() }
            .navigationDestination(item: $pushed) { page in page.page }
            .navigationDestination(item: $pushedPart) { part in
                SettingsNSHPage(
                    part: part,
                    liveSession: liveSession,
                    showsFilingNumber: showsFilingNumber,
                    onOpen: open,
                    onDone: { dismiss() },
                    hosts: hosts,
                    onSetup: { go(HostSetupRoute.choosing($0, in: hosts)) })
            }
            #if DEBUG
            .navigationDestination(item: $pushCapture) { screen in screen.settingsPage }
            #endif
            .task {
                #if DEBUG
                if CaptureScreen.requested == .theme {
                    try? await Task.sleep(for: .milliseconds(600))
                    pushTheme = true
                }
                if let screen = CaptureScreen.requested, CaptureScreen.pushedInSettings.contains(screen) {
                    try? await Task.sleep(for: .milliseconds(600))
                    pushCapture = screen
                }
                if CaptureScreen.requested == .feedback {
                    try? await Task.sleep(for: .milliseconds(600))
                    showFeedback = true
                }
                if let part = SettingsPart.requested {
                    try? await Task.sleep(for: .milliseconds(600))
                    pushedPart = part
                }
                #endif
                showsFilingNumber = await MainlandStorefront.isMainland()
            }
            .navigationTitle("Settings")
        }
        .appSheet(isPresented: $showShortcuts) { ShortcutsPage() }
        .appSheet(isPresented: $showKeys) { SSHKeysPage() }
        .appSheet(item: $settingUp) { target in
            HostSetupPage(connection: target.connection, testPane: target.testPane)
        }
        // Setup runs over a live session: a connection that isn't connected
        // is offered Connect, which opens it.
        .alert(Text("\(asking?.name ?? "") isn't connected"),
               isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }),
               presenting: asking) { host in
            Button("Connect") { onConnect(host.id) }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text("Setup runs on the host over a live session. Connect first, then set it up here.")
        }
        .appSheet(isPresented: $showFeedback) { FeedbackNSHPage() }
    }

    /// Set up a host, where `route` goes: the list pushed, the host's setup
    /// presented over its live session, or Connect asked first.
    private func go(_ route: HostSetupRoute?) {
        switch route {
        case .list?:
            pushedPart = .hosts
        case .setup(let id)?:
            // Live when the route was chosen; asked to connect if it dropped.
            guard let live = session(id) else {
                asking = hosts.first { $0.id == id }
                return
            }
            settingUp = HostSetupTarget(connection: live.connection,
                                        testPane: live.tmuxControl?.snapshot.activePaneId)
        case .connect(let id)?:
            asking = hosts.first { $0.id == id }
        case nil:
            break
        }
    }

    /// What a row of the NSH page asked for (`ui/src/settings.rs`'s `Page`):
    /// a page pushed on the stack, a sheet, or a link.
    private func open(_ page: String) {
        if let part = SettingsPart(rawValue: page) {
            pushedPart = part
            return
        }
        if let pushed = SettingsPage(rawValue: page) {
            self.pushed = pushed
            return
        }
        switch page {
        case "shortcuts": showShortcuts = true
        case "ssh-keys": showKeys = true
        case "host-setup":
            go(HostSetupRoute.entering(hosts))
        case "feedback": showFeedback = true
        case "github-issue": openURL(FeedbackChannels.newIssueURL)
        case "filing": openURL(Self.icpFilingURL)
        case "ios-settings":
            if let url = IOSAlertPermissions.shared.settingsURL { openURL(url) }
        default: Log.ui.error("settings: no page \(page, privacy: .public)")
        }
    }

    /// 工信部 APP 备案号 for Moshpit (filed 2026-09 through Tencent Cloud under the
    /// cluas.cn domain). The website carries the sibling site filing number; the
    /// store listing carries this one. Changing the distribution certificate or
    /// the bundle id means amending the filing, not just this string.
    static let icpFilingNumber = "浙ICP备2025212603号-2A"
    static let icpFilingURL = URL(string: "https://beian.miit.gov.cn/")!

    /// Build identity for the footer: marketing version + build number from the
    /// bundle, plus the `MoshpitBuildStamp` (git SHA · build time) injected into
    /// the product's Info.plist by scripts/release/build-ipa.sh. Xcode/simulator builds
    /// carry no stamp → "dev". A "+" after the SHA means uncommitted changes.
    static let versionLine: String = {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        let stamp = info?["MoshpitBuildStamp"] as? String ?? "dev"
        return "Moshpit \(version) (\(build)) · \(stamp)"
    }()
}

#if DEBUG
extension CaptureScreen {
    /// The page a capture run pushes in Settings' stack
    /// (``pushedInSettings``).
    @ViewBuilder
    var settingsPage: some View {
        switch self {
        case .appIcon: AppIconNSHPage()
        case .recentLog: RecentLogNSHPage()
        case .accent, .accentEditor, .accentDelete: AccentNSHPage()
        case .voiceEngine: VoiceEngineNSHPage()
        case .voiceLanguage: VoiceLanguageNSHPage()
        case .whisperModel: WhisperModelNSHPage()
        case .themeEditor, .themeDelete: ThemeGalleryNSHPage()
        default: EmptyView()
        }
    }
}
#endif

/// The host Settings' Set up a host opened: its connection, and the tmux
/// pane the hooks test fires into.
struct HostSetupTarget: Identifiable {
    let connection: ServerConnection
    let testPane: String?
    var id: UUID { connection.id }
}
