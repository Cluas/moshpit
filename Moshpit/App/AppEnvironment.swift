import SwiftUI

/// Every app-wide `@Observable` the view tree reads through
/// `@Environment(X.self)`, bundled so a presentation boundary can hand all
/// of them to what it presents in one go.
///
/// Why this exists: when the iOS app runs on a Mac as "Designed for iPad",
/// SwiftUI builds the content of `.sheet` / `.fullScreenCover` / `.popover`
/// in a hosting controller whose environment does NOT inherit from the
/// presenting view. iPhone and iPad inherit fine; the Mac path has been
/// broken for years (Apple Developer Forums thread 738201). The first
/// `@Environment(AppSettings.self)` inside the sheet then traps with
/// "No Observable object of type AppSettings found" — build 394 died on
/// every Mac tester's first click on Settings or "+" (TestFlight crash
/// feedback, 2026-09-08, macOS 15.3.1). The `appSheet` family below wraps
/// the stock modifiers and re-injects the bundle into the presented tree,
/// so the lookup succeeds whether or not the host inherited anything.
struct AppEnvironment {
    let settings: AppSettings
    let metrics: SessionMetricsRegistry
    let shortcuts: ShortcutStore
    let sshKeys: SSHKeyStore
    let themes: ThemeStore
    let appThemes: AppThemeStore
    let monitor: AgentActivityMonitor
    let connectionHolder: ConnectionStoreHolder
    let keychainHolder: KeychainServiceHolder
    let router: DeepLinkRouter
}

private struct AppEnvironmentKey: EnvironmentKey {
    static let defaultValue: AppEnvironment? = nil
}

extension EnvironmentValues {
    /// The bundle rides along in the environment so a presenting view — which
    /// IS inside the main hierarchy and can see it — can pass it across the
    /// presentation boundary to a tree that may not.
    var appEnvironment: AppEnvironment? {
        get { self[AppEnvironmentKey.self] }
        set { self[AppEnvironmentKey.self] = newValue }
    }
}

extension View {
    /// Inject every app-wide object plus the bundle itself. Applied once at
    /// the root by `MoshpitApp`, and again on every presented tree by the
    /// `appSheet` family. The tint rides along for the same reason: it is
    /// environment-carried, and a host that drops the environment would
    /// otherwise paint the sheet's controls in the system default.
    func appEnvironment(_ env: AppEnvironment) -> some View {
        self
            .tint(Ink.accent)
            .environment(env.settings)
            .environment(env.metrics)
            .environment(env.shortcuts)
            .environment(env.sshKeys)
            .environment(env.themes)
            .environment(env.appThemes)
            .environment(env.monitor)
            .environment(env.connectionHolder)
            .environment(env.keychainHolder)
            .environment(env.router)
            .environment(\.appEnvironment, env)
    }

    /// Re-inject if the presenting view had the bundle. It won't in the DEBUG
    /// demo seams, which inject `AppSettings` alone; those keep stock behavior.
    @ViewBuilder
    fileprivate func reinjecting(_ env: AppEnvironment?) -> some View {
        if let env { appEnvironment(env) } else { self }
    }

    // MARK: Presentation wrappers — same signatures as the stock modifiers.

    /// `.sheet(isPresented:onDismiss:content:)` that survives the Mac host.
    func appSheet<Content: View>(
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        modifier(AppSheet(isPresented: isPresented, onDismiss: onDismiss, sheet: content))
    }

    /// `.sheet(item:onDismiss:content:)` that survives the Mac host.
    func appSheet<Item: Identifiable, Content: View>(
        item: Binding<Item?>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping (Item) -> Content
    ) -> some View {
        modifier(AppItemSheet(item: item, onDismiss: onDismiss, sheet: content))
    }

    /// `.fullScreenCover(isPresented:onDismiss:content:)` that survives the Mac host.
    func appFullScreenCover<Content: View>(
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        modifier(AppFullScreenCover(isPresented: isPresented, onDismiss: onDismiss, cover: content))
    }

    /// `.popover(isPresented:attachmentAnchor:arrowEdge:content:)` that survives the Mac host.
    func appPopover<Content: View>(
        isPresented: Binding<Bool>,
        attachmentAnchor: PopoverAttachmentAnchor = .rect(.bounds),
        arrowEdge: Edge? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        modifier(AppPopover(isPresented: isPresented, attachmentAnchor: attachmentAnchor,
                            arrowEdge: arrowEdge, popover: content))
    }
}

// The modifiers read the bundle from THEIR environment — the presenting
// view's, where it exists — and capture it into the presentation closure.

private struct AppSheet<Sheet: View>: ViewModifier {
    @Environment(\.appEnvironment) private var env
    let isPresented: Binding<Bool>
    let onDismiss: (() -> Void)?
    let sheet: () -> Sheet

    func body(content: Content) -> some View {
        content.sheet(isPresented: isPresented, onDismiss: onDismiss) {
            sheet().reinjecting(env)
        }
    }
}

private struct AppItemSheet<Item: Identifiable, Sheet: View>: ViewModifier {
    @Environment(\.appEnvironment) private var env
    let item: Binding<Item?>
    let onDismiss: (() -> Void)?
    let sheet: (Item) -> Sheet

    func body(content: Content) -> some View {
        content.sheet(item: item, onDismiss: onDismiss) { value in
            sheet(value).reinjecting(env)
        }
    }
}

private struct AppFullScreenCover<Cover: View>: ViewModifier {
    @Environment(\.appEnvironment) private var env
    let isPresented: Binding<Bool>
    let onDismiss: (() -> Void)?
    let cover: () -> Cover

    func body(content: Content) -> some View {
        content.fullScreenCover(isPresented: isPresented, onDismiss: onDismiss) {
            cover().reinjecting(env)
        }
    }
}

private struct AppPopover<Popover: View>: ViewModifier {
    @Environment(\.appEnvironment) private var env
    let isPresented: Binding<Bool>
    let attachmentAnchor: PopoverAttachmentAnchor
    let arrowEdge: Edge?
    let popover: () -> Popover

    func body(content: Content) -> some View {
        content.popover(isPresented: isPresented, attachmentAnchor: attachmentAnchor,
                        arrowEdge: arrowEdge) {
            popover().reinjecting(env)
        }
    }
}
