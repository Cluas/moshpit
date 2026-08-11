import SwiftUI
import UIKit

/// Screen 2 — Settings. APPEARANCE / DISPLAY / CURSOR / BEHAVIOR /
/// MOSH·ROAMING / NOTIFICATIONS / VOICE INPUT groups, exactly per prototype.
struct SettingsScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @Environment(ShortcutStore.self) private var shortcutStore
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ThemeStore.self) private var themes
    /// Saved hosts — read only to decide whether the tmux hooks installer is
    /// worth offering when nothing is connected (see `hooksApplicable`).
    /// Comes through the same holder the shortcut editor uses; `ConnectionStore`
    /// itself is not in the environment.
    @Environment(ConnectionStoreHolder.self) private var connectionHolder

    /// The foreground live session, if any — used by the agent-hooks installer
    /// so "Run in terminal" / "Re-check" can run against a real shell. nil →
    /// the installer shows command + Copy only (its `.noChannel` state).
    var liveSession: SessionHub.ActiveSession?

    #if DEBUG
    @State private var showShortcuts = CaptureScreen.requested == .shortcuts
        || CaptureScreen.requested == .shortcutLibrary
    @State private var showKeys = CaptureScreen.requested == .sshKeys
    #else
    @State private var showShortcuts = false
    @State private var showKeys = false
    #endif
    @State private var showServerBinaryEditor = false
    @State private var showUDPEditor = false
    @State private var showNotifInfo = false
    @State private var showHooksInstall = false

    /// True when the session this sheet would act on runs herdr, whose agent
    /// status needs no host-side hooks at all.
    private var liveSessionUsesHerdr: Bool {
        liveSession?.connection.multiplexer == .herdr
    }

    /// Whether the tmux hooks installer is worth offering at all.
    ///
    /// The optional chain above makes "nothing connected" read as "not herdr",
    /// so a herdr-only user opening Settings from Home was offered an
    /// installer whose own screen ends with "connect to a host with tmux" —
    /// the app telling them to install tmux for something herdr gives free.
    /// With no live session, ask their saved hosts instead.
    private var hooksApplicable: Bool {
        if let liveSession { return liveSession.connection.multiplexer != .herdr }
        return connectionHolder.store.connections.contains { $0.multiplexer != .herdr }
    }

    var body: some View {
        @Bindable var settings = settings

        NavigationStack {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        appearanceGroup(settings: $settings)
                        displayGroup(settings: $settings)
                        cursorGroup(settings: $settings)

                        FormGroup(title: "BEHAVIOR") {
                            ToggleRow(
                                label: "Keep Connections Alive",
                                subtitle: "Send keepalive pings so the server does not drop idle connections",
                                isOn: $settings.keepConnectionsAlive)
                        }

                        moshGroup(settings: $settings)

                        FormGroup(title: "KEYBOARD · KEYS") {
                            ChevronRow(
                                label: "Shortcuts",
                                value: "\(shortcutStore.toolbarCount)/\(ShortcutStore.toolbarLimit)"
                            ) { showShortcuts = true }
                            ChevronRow(
                                label: "SSH Keys",
                                value: String(localized: "\(keyStore.keys.count) keys")
                            ) { showKeys = true }
                        }

                        FormGroup(
                            title: "NOTIFICATIONS",
                            footer: "Moshpit watches the active session for agent activity and posts a local alert when your agent needs attention — natively on herdr, via the bell and hooks on tmux."
                        ) {
                            ToggleRow(
                                label: "Notifications",
                                subtitle: "Alert when an agent needs you",
                                isOn: $settings.notificationsEnabled)
                            ToggleRow(
                                label: "Live Activity",
                                subtitle: "Show agent session status in the Dynamic Island",
                                isOn: $settings.liveActivityEnabled)
                            ToggleRow(
                                label: "Alert sound",
                                subtitle: "Play a sound when the agent needs you",
                                isOn: $settings.attentionSoundEnabled)
                            ToggleRow(
                                label: "Show detail on lock screen",
                                subtitle: "Display what the agent is running/asking — off keeps it private",
                                isOn: $settings.lockScreenDetailEnabled)
                            ChevronRow(label: "How notifications work") { showNotifInfo = true }
                            // Hidden on herdr: it reports agent status itself,
                            // so there is nothing to install — offering the
                            // installer would send the user to set up hooks
                            // that would never be read.
                            if hooksApplicable {
                                ChevronRow(label: "Install agent hooks") { showHooksInstall = true }
                            }
                        }

                        voiceGroup(settings: $settings)

                        // Build identity — long-press to copy (for bug reports).
                        Text(Self.versionLine)
                            .font(Face.mono(11))
                            .foregroundStyle(Ink.meta)
                            .frame(maxWidth: .infinity)
                            .padding(.top, 20)
                            .contextMenu {
                                Button("Copy") { UIPasteboard.general.string = Self.versionLine }
                            }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 44)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                }
            }
        }
        .preferredColorScheme(.dark)
        .sheet(isPresented: $showShortcuts) { ShortcutsView() }
        .sheet(isPresented: $showKeys) { SSHKeysView() }
        .sheet(isPresented: $showServerBinaryEditor) {
            MoshServerBinaryEditor(settings: settings)
        }
        .sheet(isPresented: $showUDPEditor) {
            UDPRangeEditor(settings: settings)
        }
        .sheet(isPresented: $showNotifInfo) {
            NotificationInfoView()
        }
        .sheet(isPresented: $showHooksInstall) {
            IslandHooksInstallView(session: liveSession)
        }
    }

    /// Build identity for the footer: marketing version + build number from the
    /// bundle, plus the `MoshpitBuildStamp` (git SHA · build time) injected into
    /// the product's Info.plist by scripts/build-ipa.sh. Xcode/simulator builds
    /// carry no stamp → "dev". A "+" after the SHA means uncommitted changes.
    static let versionLine: String = {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        let stamp = info?["MoshpitBuildStamp"] as? String ?? "dev"
        return "Moshpit \(version) (\(build)) · \(stamp)"
    }()

    // MARK: APPEARANCE

    /// App chrome: accent color and app icon, each its own screen. They used to
    /// be one list where picking a color also swapped the icon; they are
    /// separate because the icon cannot follow a custom accent (iOS only
    /// switches to icons bundled at build time), and because wanting a green
    /// accent is not the same as wanting the green icon.
    @ViewBuilder
    private func appearanceGroup(settings: Bindable<AppSettings>) -> some View {
        let theme = AppThemeCatalog.theme(for: settings.wrappedValue.appThemeId)
        let icon = AppIconCatalog.option(for: settings.wrappedValue.appIconId)

        FormGroup(
            title: "APPEARANCE",
            footer: "The accent color tints the app's controls and highlights. The home-screen icon is a separate choice. Both are separate from the terminal color scheme (Display → Theme, below)."
        ) {
            NavigationLink {
                AccentGalleryView()
            } label: {
                HStack(spacing: 10) {
                    Text("Accent").font(Face.text(14)).foregroundStyle(Ink.primary)
                    Spacer()
                    Circle()
                        .fill(theme.accent)
                        .frame(width: 20, height: 20)
                        .overlay(Circle().strokeBorder(Color.white.opacity(0.16), lineWidth: 1))
                    Text(theme.name).font(Face.text(14)).foregroundStyle(Ink.meta).lineLimit(1)
                    MiniChevron()
                }
                .frame(minHeight: Metrics.cellMinHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            NavigationLink {
                AppIconGalleryView()
            } label: {
                HStack(spacing: 10) {
                    Text("App Icon").font(Face.text(14)).foregroundStyle(Ink.primary)
                    Spacer()
                    AppIconThumb(option: icon, side: 24)
                    Text(icon.name).font(Face.text(14)).foregroundStyle(Ink.meta).lineLimit(1)
                    MiniChevron()
                }
                .frame(minHeight: Metrics.cellMinHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: DISPLAY

    @ViewBuilder
    private func displayGroup(settings: Bindable<AppSettings>) -> some View {
        let theme = themes.theme(id: settings.wrappedValue.themeId)

        FormGroup(title: "DISPLAY") {
            Menu {
                ForEach(TerminalFont.families, id: \.id) { family in
                    Button(family.label) { settings.wrappedValue.fontName = family.id }
                }
            } label: {
                HStack(spacing: 10) {
                    Text("Font").font(Face.text(14)).foregroundStyle(Ink.primary)
                    Spacer()
                    Text(TerminalFont.label(for: settings.wrappedValue.fontName))
                        .font(Face.text(14)).foregroundStyle(Ink.meta)
                    MiniChevron()
                }
                .frame(minHeight: Metrics.cellMinHeight)
                .contentShape(Rectangle())
            }
            ValueRow(label: "Font Size", value: String(localized: "\(Int(settings.wrappedValue.fontSize))pt"))
            VStack(spacing: 10) {
                Slider(value: settings.fontSize, in: 8...18, step: 1)
                    .tint(Ink.accent)
                Text("ABCdef 012 ~/ssh $")
                    .font(Face.mono(settings.wrappedValue.fontSize * 0.85))
                    .foregroundStyle(Ink.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(EdgeInsets(top: 8, leading: 10, bottom: 8, trailing: 10))
                    .background(Ink.terminalBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                            .strokeBorder(Ink.groupBorder, lineWidth: 1))
            }
            .padding(.vertical, 8)
            // Themes get their own screen rather than a menu: a list of names
            // can't answer "what will my terminal look like", which is the only
            // question being asked here.
            NavigationLink {
                ThemeGalleryView()
            } label: {
                HStack(spacing: 10) {
                    Text("Theme").font(Face.text(14)).foregroundStyle(Ink.primary)
                    Spacer()
                    ThemeSwatchStrip(theme: theme)
                    Text(theme.name).font(Face.text(14)).foregroundStyle(Ink.meta).lineLimit(1)
                    MiniChevron()
                }
                .frame(minHeight: Metrics.cellMinHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: CURSOR

    @ViewBuilder
    private func cursorGroup(settings: Bindable<AppSettings>) -> some View {
        FormGroup(
            title: "CURSOR",
            footer: "Shape and color apply to all SSH / mosh sessions. With trail on, characters that mosh's predictive echo shows ahead of the server are marked with a translucent trail until confirmed."
        ) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Shape").font(Face.text(14)).foregroundStyle(Ink.primary)
                Text("Applies to all sessions").font(Face.text(11)).foregroundStyle(Ink.meta)
                PillSegmentedControl(
                    items: [
                        SegItem(value: CursorShape.block, label: "Block", systemImage: "rectangle.fill"),
                        SegItem(value: CursorShape.bar, label: "Bar", systemImage: "poweron"),
                        SegItem(value: CursorShape.underline, label: "Underline", systemImage: "underline"),
                    ],
                    selection: settings.cursorShape)
            }
            .padding(.top, 8)

            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Color").font(Face.text(14)).foregroundStyle(Ink.primary)
                    Text("Current session · switches to amber while roaming")
                        .font(Face.text(11)).foregroundStyle(Ink.meta)
                }
                Spacer()
                ColorSwatchRow(
                    colors: [
                        ("teal", Ink.mosh),
                        ("green", Ink.cursorGreen),
                        ("white", Color.white.opacity(0.86)),
                        ("accent", Ink.accent),
                    ],
                    selection: settings.cursorColorId)
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .padding(.vertical, 4)

            ToggleRow(label: "Blink", subtitle: "1.1s cadence, follows the iOS system default", isOn: settings.cursorBlink)
            ToggleRow(
                label: "Trail on predict",
                subtitle: "Leaves a signal trail behind the cursor while mosh predicts ahead of the server",
                isOn: settings.trailOnPredict)

            CursorPreview(
                shape: settings.wrappedValue.cursorShape,
                colorId: settings.wrappedValue.cursorColorId,
                blink: settings.wrappedValue.cursorBlink,
                trail: settings.wrappedValue.trailOnPredict)
        }
    }

    // MARK: VOICE INPUT

    /// What the picked dictation language is called, for the Language row's
    /// value slot. The two engines keep separate language settings, so this
    /// reads whichever one is live. "" reads as Automatic / Auto-detect, not
    /// the resolved language — the picker's own detail line spells that out.
    private func voiceLanguageName(_ settings: AppSettings) -> String {
        if settings.voiceEngine == .whisper {
            return WhisperLanguageCatalog.displayName(for: settings.whisperLanguage)
        }
        let id = settings.voiceInputLocaleId
        guard !id.isEmpty else { return String(localized: "Automatic") }
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }

    /// The Model row's value: the model that would actually be used, or a
    /// prompt when there is none. Never shows a stored-but-deleted variant.
    private func voiceModelName(_ settings: AppSettings) -> String {
        guard let variant = WhisperModelStore.resolvedVariant(preferring: settings.whisperModelId) else {
            return String(localized: "None")
        }
        return WhisperModelStore.displayName(for: variant)
    }

    @ViewBuilder
    private func voiceGroup(settings: Bindable<AppSettings>) -> some View {
        FormGroup(
            title: "VOICE INPUT",
            footer: "Dictate commands and prompts from the mic key on the terminal bar. Speech is transcribed entirely on-device — your voice never leaves this device, and nothing is typed until you tap Insert."
        ) {
            ToggleRow(
                label: "Enable Voice Input",
                subtitle: "Adds a mic key to the terminal bar",
                isOn: settings.voiceInputEnabled)
            if settings.wrappedValue.voiceInputEnabled {
                NavigationLink {
                    VoiceEngineView()
                } label: {
                    settingsValueRow(
                        title: String(localized: "Recognition"),
                        value: settings.wrappedValue.voiceEngine.displayName)
                }
                .buttonStyle(.plain)

                if settings.wrappedValue.voiceEngine == .whisper {
                    NavigationLink {
                        WhisperModelView()
                    } label: {
                        settingsValueRow(
                            title: String(localized: "Model"),
                            value: voiceModelName(settings.wrappedValue))
                    }
                    .buttonStyle(.plain)
                }

                NavigationLink {
                    VoiceLanguageView()
                } label: {
                    settingsValueRow(
                        title: String(localized: "Language"),
                        value: voiceLanguageName(settings.wrappedValue))
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func settingsValueRow(title: String, value: String) -> some View {
        HStack(spacing: 10) {
            Text(title).font(Face.text(14)).foregroundStyle(Ink.primary)
            Spacer()
            Text(value)
                .font(Face.text(14)).foregroundStyle(Ink.meta).lineLimit(1)
            MiniChevron()
        }
        .frame(minHeight: Metrics.cellMinHeight)
        .contentShape(Rectangle())
    }

    // MARK: MOSH · ROAMING

    @ViewBuilder
    private func moshGroup(settings: Bindable<AppSettings>) -> some View {
        FormGroup(
            title: "MOSH · ROAMING",
            footer: "Mosh runs over UDP and survives IP changes. If your server is behind a strict firewall, open the port range above outbound from your iPhone."
        ) {
            ToggleRow(
                label: "Mosh by default",
                subtitle: "Wrap new SSH hosts with mosh-server on connect",
                isOn: settings.moshByDefault)
            Menu {
                ForEach(PredictMode.allCases, id: \.self) { mode in
                    Button(mode.label) { settings.wrappedValue.predictMode = mode }
                }
            } label: {
                HStack(spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Predictive Echo").font(Face.text(14)).foregroundStyle(Ink.primary)
                        Text("Show typed characters locally before server confirms")
                            .font(Face.text(11)).foregroundStyle(Ink.meta)
                    }
                    Spacer()
                    Text(settings.wrappedValue.predictMode.label)
                        .font(Face.text(14)).foregroundStyle(Ink.meta)
                    MiniChevron()
                }
                .frame(minHeight: Metrics.cellMinHeight)
                .padding(.vertical, 4)
                .contentShape(Rectangle())
            }
            ChevronRow(label: "Server binary",
                       value: settings.wrappedValue.moshServerBinary) { showServerBinaryEditor = true }
            ChevronRow(
                label: "UDP port range",
                value: "\(settings.wrappedValue.udpRangeStart) – \(settings.wrappedValue.udpRangeEnd)"
            ) { showUDPEditor = true }
        }
    }
}

// MARK: - Notification info

/// Honest explainer for how Moshpit's notifications work — they are local and
/// bell-driven, not a remote push service.
struct NotificationInfoView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        row("bell.fill", Ink.warn, "Bell = attention",
                            "When a tmux pane rings the terminal bell (BEL) — which Claude Code and most CLIs emit when they finish or need input — Moshpit posts a local notification and flips the Vibe Island to “needs attention.”")
                        row("bubbles.and.sparkles.fill", Ink.accent, "Live Activity",
                            "While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane.")
                        row("bolt.horizontal.fill", Ink.mosh, "Local, not push",
                            "Alerts are generated on-device from the live session — there’s no cloud push server. They fire while Moshpit is in the foreground or recently backgrounded; a fully suspended app won’t poll. iOS will ask for notification permission the first time you connect with Notifications on.")
                    }
                    .padding(20)
                }
            }
            .navigationTitle("How notifications work")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }.foregroundStyle(Ink.accent)
                }
            }
        }
        .preferredColorScheme(.dark)
    }

    private func row(_ icon: String, _ tint: Color, _ title: LocalizedStringKey, _ body: LocalizedStringKey) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 18))
                .foregroundStyle(tint)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(Face.text(15, .semibold)).foregroundStyle(Ink.primary)
                Text(body).font(Face.text(13)).foregroundStyle(Ink.secondary).lineSpacing(2)
            }
        }
    }
}

// MARK: - Mosh server-binary editor

struct MoshServerBinaryEditor: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var settings: AppSettings
    @State private var path = ""

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    FormGroup(
                        title: "MOSH SERVER PATH",
                        footer: "The mosh-server executable on the remote host. Override if it isn't on PATH (e.g. /opt/homebrew/bin/mosh-server)."
                    ) {
                        FieldRow(placeholder: "/usr/local/bin/mosh-server", text: $path, mono: true)
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.top, 8)
                }
            }
            .navigationTitle("Server Binary")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        let trimmed = path.trimmingCharacters(in: .whitespaces)
                        if !trimmed.isEmpty { settings.moshServerBinary = trimmed }
                        dismiss()
                    }.foregroundStyle(Ink.accent).fontWeight(.semibold)
                }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear { path = settings.moshServerBinary }
    }
}

// MARK: - UDP port-range editor

struct UDPRangeEditor: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var settings: AppSettings
    @State private var start = ""
    @State private var end = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    FormGroup(
                        title: "UDP PORT RANGE",
                        footer: "mosh binds one UDP port in this range per session. Open it outbound from your iPhone and inbound on the server (default 60000–61000)."
                    ) {
                        HStack {
                            Text("From").font(Face.text(14)).foregroundStyle(Ink.primary)
                            Spacer()
                            TextField("60000", text: $start)
                                .keyboardType(.numberPad)
                                .multilineTextAlignment(.trailing)
                                .font(Face.mono(14)).foregroundStyle(Ink.fixedValue)
                                .frame(width: 100)
                        }
                        .frame(minHeight: Metrics.cellMinHeight)
                        HStack {
                            Text("To").font(Face.text(14)).foregroundStyle(Ink.primary)
                            Spacer()
                            TextField("61000", text: $end)
                                .keyboardType(.numberPad)
                                .multilineTextAlignment(.trailing)
                                .font(Face.mono(14)).foregroundStyle(Ink.fixedValue)
                                .frame(width: 100)
                        }
                        .frame(minHeight: Metrics.cellMinHeight)
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.top, 8)
                }
            }
            .navigationTitle("UDP Port Range")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.foregroundStyle(Ink.accent)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).foregroundStyle(Ink.accent).fontWeight(.semibold)
                }
            }
            .moshpitCard(isPresented: Binding(
                get: { error != nil }, set: { if !$0 { error = nil } }
            )) {
                MoshpitNoticeCard(title: "Invalid range", message: error ?? "") {
                    error = nil
                }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear {
            start = String(settings.udpRangeStart)
            end = String(settings.udpRangeEnd)
        }
    }

    private func save() {
        switch UDPPortRange.validate(from: start, to: end) {
        case .success(let (s, e)):
            settings.udpRangeStart = s
            settings.udpRangeEnd = e
            dismiss()
        case .failure(.nonNumeric):
            error = String(localized: "Enter numeric ports.")
        case .failure(.outOfBounds):
            error = String(localized: "Ports must be 1–65535.")
        case .failure(.inverted):
            error = String(localized: "From must be ≤ To.")
        }
    }
}

// MARK: - Cursor preview box

struct CursorPreview: View {
    let shape: CursorShape
    let colorId: String
    let blink: Bool
    let trail: Bool

    @State private var visible = true

    private var cursorColor: Color {
        switch colorId {
        case "green": return Ink.cursorGreen
        case "white": return Color.white.opacity(0.86)
        case "accent": return Ink.accent
        default: return Ink.mosh
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 0) {
                prompt("trade", dim: false)
                cursor(.block, active: shape == .block)
            }
            HStack(spacing: 0) {
                prompt("git push", dim: true)
                cursor(.bar, active: shape == .bar)
            }
            HStack(spacing: 0) {
                prompt("ls -la", dim: true)
                cursor(.underline, active: shape == .underline)
            }
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Ink.terminalBG, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1))
        .padding(.vertical, 10)
        .onAppear {
            guard blink else { return }
            withAnimation(.easeInOut(duration: 0.55).repeatForever(autoreverses: true)) {
                visible = false
            }
        }
    }

    private func prompt(_ cmd: String, dim: Bool) -> some View {
        HStack(spacing: 6) {
            Text("➜").font(Face.mono(11)).foregroundStyle(Ink.promptGreen.opacity(dim ? 0.5 : 1))
                Text(cmd).font(Face.mono(11))
                .foregroundStyle(dim ? Ink.termMuted : Ink.primary)
        }
    }

    @ViewBuilder
    private func cursor(_ kind: CursorShape, active: Bool) -> some View {
        let color = cursorColor
        let opacity = blink && active ? (visible ? 1.0 : 0.2) : 1.0
        HStack(spacing: 0) {
            if trail && active {
                LinearGradient(
                    colors: [color.opacity(0), color.opacity(0.55)],
                    startPoint: .leading, endPoint: .trailing)
                    .frame(width: 18, height: kind == .underline ? 2 : 12)
            }
            Group {
                switch kind {
                case .block: Rectangle().frame(width: 7, height: 12)
                case .bar: Rectangle().frame(width: 2, height: 12)
                case .underline: Rectangle().frame(width: 8, height: 2)
                }
            }
            .foregroundStyle(color)
            .opacity(opacity)
        }
        .padding(.leading, 2)
        .opacity(active ? 1 : 0.6)
    }
}
