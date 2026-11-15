import SwiftUI
import UIKit

// MARK: - Haptics

/// Lightweight haptic feedback for user actions, so a tap registers physically
/// even when the on-screen result is subtle or slightly delayed (the "did that
/// do anything?" gap). Generators are cheap to create per call.
enum Haptics {
    /// Navigation / selection change (switch session/window/pane, pick a row).
    static func select() { UISelectionFeedbackGenerator().selectionChanged() }
    /// A discrete action fired (create, send, toggle).
    static func tap(_ style: UIImpactFeedbackGenerator.FeedbackStyle = .light) {
        UIImpactFeedbackGenerator(style: style).impactOccurred()
    }
    static func success() { UINotificationFeedbackGenerator().notificationOccurred(.success) }
}

// MARK: - Form group (a Form section)

/// Section container for the app's forms. A thin wrapper over `Section`, so
/// call sites keep their `FormGroup(title:footer:)` shape while the rows get
/// the system's inset-grouped chrome: row insets, separators, header and
/// footer typography, Dynamic Type, and the highlight on tap. Must sit inside
/// a `Form` or `List` (see `moshpitForm()`).
struct FormGroup<Content: View>: View {
    var title: LocalizedStringKey?
    var footer: LocalizedStringKey?
    @ViewBuilder var content: Content

    /// A real `Section` — swipe actions, edit mode, keyboard avoidance and
    /// Dynamic Type all come from the List — wearing the app's own chrome:
    /// the mono kicker, the ink group fill, the hairline separators. Native
    /// is how it behaves, not whose grey it is painted.
    var body: some View {
        Section {
            content
                .moshpitRows()
        } header: {
            if let title {
                SectionKicker(title: title)
            }
        } footer: {
            if let footer {
                Text(footer)
                    .moshpitFooter()
            }
        }
    }
}

/// Section labels in the terminal voice: small mono caps, tracked out, dim.
/// The one typographic tell that this list belongs to Moshpit and not to
/// Settings; sized off `caption2` so it still follows the reading size.
struct SectionKicker: View {
    let title: LocalizedStringKey

    var body: some View {
        Text(title)
            .font(Face.mono(10.5, .semibold))
            .kerning(1.2)
            .foregroundStyle(Ink.sectionTitle)
            .textCase(.uppercase)
            .padding(.leading, 2)
    }
}

extension View {
    /// The app's Form/List chrome: the system inset-grouped layout on the
    /// theme's screen colour instead of the system grouped background.
    func moshpitForm() -> some View {
        self
            .scrollContentBackground(.hidden)
            // The page's paper in the light appearance, the theme's screen
            // in the dark one (R18).
            .background(Ink.pageBG)
    }

    /// Rows in the app's ink rather than the system's grey: the navy-black
    /// group fill and a hairline between rows. Apply to a Section's content.
    func moshpitRows() -> some View {
        self
            .listRowBackground(Ink.group)
            .listRowSeparatorTint(Ink.hairline)
    }

    /// Section footers: the quiet 12pt explanatory voice under a group.
    func moshpitFooter() -> some View {
        self
            .font(Face.text(12))
            .foregroundStyle(Ink.tertiary)
            .lineSpacing(3)
            .textCase(nil)
    }
}

/// UIKit-level styling for the few system controls SwiftUI leaves unstyled.
enum Chrome {
    /// The segmented control in the app's ink — dark track, raised pill — so
    /// Password / SSH Key and Block / Bar / Underline stop reading as a
    /// Settings.app control dropped into a dark screen. UIAppearance, because
    /// `.pickerStyle(.segmented)` exposes nothing else. Every segmented
    /// control takes it, the NSH pages' too, so its colours follow the
    /// appearance where the control shows (R18).
    @MainActor static func installAppearance() {
        let seg = UISegmentedControl.appearance()
        seg.backgroundColor = InkUI.segTrack
        seg.selectedSegmentTintColor = InkUI.segActive
        seg.setTitleTextAttributes([.foregroundColor: InkUI.secondary], for: .normal)
        seg.setTitleTextAttributes([.foregroundColor: InkUI.primary], for: .selected)
    }
}

// MARK: - Rows

/// Plain text-field row.
struct FieldRow: View {
    let placeholder: LocalizedStringKey
    @Binding var text: String
    var secure: Bool = false
    var mono: Bool = false
    var alignment: TextAlignment = .leading
    /// Grow with the text (up to a few lines) instead of scrolling one line —
    /// for prose fields like an agent prompt, where composing two sentences
    /// through a 14pt keyhole means editing blind.
    var multiline: Bool = false

    var body: some View {
        Group {
            if secure {
                SecureField(placeholder, text: $text)
            } else if multiline {
                TextField(placeholder, text: $text, axis: .vertical)
                    .lineLimit(1...4)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            } else {
                TextField(placeholder, text: $text)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            }
        }
        .font(mono ? .system(.body, design: .monospaced) : .body)
        .foregroundStyle(Ink.primary)
        .multilineTextAlignment(alignment)
    }
}

struct MiniChevron: View {
    var color: Color = Ink.meta
    var body: some View {
        Image(systemName: "chevron.right")
            .font(.footnote.weight(.semibold))
            .foregroundStyle(color)
    }
}

// MARK: - Status pills (§3.10)

/// What the transport pill says (``TerminalNSHPage``'s `pill`).
enum TransportPillKind {
    case mosh, moshRoaming, ssh
}

/// Live connection state, surfaced on the transport pill so a dropped/
/// reconnecting session is never silent.
enum TransportConnState { case live, connecting, reconnecting, offline }

// MARK: - Sheet surface and chrome

extension View {
    /// The tmux sheets' surface: nothing on iOS 26 (the system sheet is
    /// already glass, and any explicit background replaces it with paint),
    /// the app's sheet fill on iOS 18 where the default is flat: dark, or
    /// light under an NSH sheet in the light appearance (R18).
    @ViewBuilder func sheetSurface() -> some View {
        if #available(iOS 26.0, *) {
            self
        } else {
            self.presentationBackground(Ink.sheet)
        }
    }
}

/// A presentation that follows the size class: a popover anchored to the
/// modified view on a regular-width iPad, a sheet everywhere else. The tmux
/// pickers use it — on an iPad the whole-screen sheet for a five-row list was
/// the iPhone shape stretched, and the breadcrumb they belong to is a natural
/// anchor. A Max-size iPhone in landscape is regular too and stays a sheet:
/// the idiom check, not the size class alone, decides.
struct AdaptivePresentation<Sheet: View>: ViewModifier {
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    let isPresented: Binding<Bool>
    let sheet: () -> Sheet

    private var asPopover: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && horizontalSizeClass == .regular
    }

    @ViewBuilder
    func body(content: Content) -> some View {
        if asPopover {
            content.appPopover(isPresented: isPresented, arrowEdge: .top) {
                sheet()
                    // A List has no ideal size of its own; the popover needs one.
                    // Three rows and the footer fit; longer lists scroll.
                    .frame(width: 400, height: 380)
                    .presentationCompactAdaptation(.sheet)
            }
        } else {
            content.appSheet(isPresented: isPresented, content: sheet)
        }
    }
}

extension View {
    func adaptivePresentation<Sheet: View>(
        isPresented: Binding<Bool>, @ViewBuilder content: @escaping () -> Sheet
    ) -> some View {
        modifier(AdaptivePresentation(isPresented: isPresented, sheet: content))
    }
}

// MARK: - Moshpit modal system
//
// One dark modal language for every prompt the app raises, replacing the
// system `.alert`/`confirmationDialog` (centered title, blue tint, no room for
// a fingerprint or an option list) that read as a different app. Prototype:
// the moshpit Open Design project's `dialogs.html`.

/// Semantic severity of a modal — sets the header icon tint and the primary
/// button's colour. `violet` = an asserted action, `danger` = destructive,
/// `warn` = a safety caution, `neutral` = an informational choice.
enum ModalTone {
    case violet, danger, warn, neutral

    var tint: Color {
        switch self {
        case .violet: return Ink.accent
        case .danger: return Ink.danger
        case .warn: return Ink.warn
        case .neutral: return Ink.secondary
        }
    }
}

/// One button in a modal's action row.
struct ModalButton: Identifiable {
    enum Kind { case secondary, primary, danger }
    let id = UUID()
    let title: LocalizedStringKey
    let kind: Kind
    /// What UI tests find it by (the NSH cards' buttons carry the same).
    let identifier: String?
    let action: () -> Void

    init(_ title: LocalizedStringKey, kind: Kind = .secondary, identifier: String? = nil,
         action: @escaping () -> Void) {
        self.title = title
        self.kind = kind
        self.identifier = identifier
        self.action = action
    }
}

private struct ModalButtonView: View {
    let button: ModalButton

    var body: some View {
        Button(action: button.action) {
            Text(button.title)
                .font(Face.display(15, .semibold))
                .foregroundStyle(fg)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(bg, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 13, style: .continuous)
                    .strokeBorder(border, lineWidth: 1))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(button.identifier ?? "")
    }

    private var fg: Color {
        switch button.kind {
        case .secondary: return Ink.secondary
        case .primary: return Ink.modalPrimaryText
        case .danger: return Ink.modalDangerText
        }
    }
    private var bg: Color {
        switch button.kind {
        case .secondary: return Ink.modalButtonFill
        case .primary: return Ink.accent.opacity(0.16)
        case .danger: return Ink.danger.opacity(0.15)
        }
    }
    private var border: Color {
        switch button.kind {
        case .secondary: return Ink.modalEdge
        case .primary: return Ink.accent.opacity(0.42)
        case .danger: return Ink.danger.opacity(0.42)
        }
    }
}

/// A framed monospace block — a fingerprint, a verify command — with an
/// optional uppercase label. Selectable so the command can be copied.
struct MonoCodeBlock: View {
    var label: LocalizedStringKey?
    let text: String
    var accent: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            if let label {
                Text(label)
                    .font(Face.mono(9.5, .medium))
                    .tracking(1.2)
                    .textCase(.uppercase)
                    .foregroundStyle(Ink.meta)
            }
            Text(text)
                .font(Face.mono(11.5))
                .foregroundStyle(accent ? Ink.accent : Ink.secondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(EdgeInsets(top: 9, leading: 11, bottom: 9, trailing: 11))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Ink.neutralFill, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(Ink.hairline, lineWidth: 1))
    }
}

/// The unified modal card: a tinted icon, left-aligned title, optional message
/// and inline content (code block / text field / option list), then a button
/// row. Present it via `.moshpitCard(item:)` / `.moshpitCard(isPresented:)`.
struct MoshpitModalCard<Extra: View>: View {
    let icon: String
    var tone: ModalTone = .violet
    let title: LocalizedStringKey
    var message: Text?
    var buttons: [ModalButton]
    /// Stack the button row vertically — for long labels or a destructive
    /// primary that shouldn't sit shoulder-to-shoulder with its escape hatch.
    var stackButtons: Bool = false
    @ViewBuilder var extra: () -> Extra

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(tone.tint)
                    .frame(width: 38, height: 38)
                    .background(tone.tint.opacity(0.14), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                    .padding(.bottom, 13)

                Text(title)
                    .font(Face.display(19, .bold))
                    .foregroundStyle(Ink.primary)

                if let message {
                    message
                        .font(Face.text(13.5))
                        .foregroundStyle(Ink.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 8)
                }

                extra()
                    .padding(.top, 12)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(EdgeInsets(top: 20, leading: 20, bottom: 4, trailing: 20))

            Group {
                if stackButtons {
                    VStack(spacing: 9) { ForEach(buttons) { ModalButtonView(button: $0) } }
                } else {
                    HStack(spacing: 9) { ForEach(buttons) { ModalButtonView(button: $0) } }
                }
            }
            .padding(16)
        }
        .frame(maxWidth: 340)
        .background(Ink.modalBG, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Ink.modalEdge, lineWidth: 1))
        .shadow(color: Ink.modalShadow, radius: 30, y: 18)
    }
}

/// Scrim + centred card with a scale/fade transition. Not dismissed by tapping
/// the scrim — modals here demand a real decision. The card takes the
/// appearance of what it shows over (R18): light over the NSH Home in the
/// light appearance, dark over a page still forced dark.
private struct MoshpitCardPresenter<CardContent: View>: View {
    @ViewBuilder var card: () -> CardContent

    var body: some View {
        ZStack {
            Rectangle()
                .fill(Color.black.opacity(0.55))
                .ignoresSafeArea()
                .transition(.opacity)
            card()
                .padding(24)
                .transition(.scale(scale: 0.94).combined(with: .opacity))
        }
    }
}

extension View {
    /// Present a Moshpit modal card bound to an optional item (mirrors
    /// `.alert(item:)`). The item stays the source of truth; dismiss by
    /// clearing it inside a button action.
    func moshpitCard<Item: Identifiable, CardContent: View>(
        item: Binding<Item?>,
        @ViewBuilder card: @escaping (Item) -> CardContent
    ) -> some View {
        overlay {
            if let value = item.wrappedValue {
                MoshpitCardPresenter { card(value) }
            }
        }
        .animation(Motion.settle, value: item.wrappedValue != nil)
    }

    /// Boolean-bound variant (mirrors `.alert(isPresented:)`).
    func moshpitCard<CardContent: View>(
        isPresented: Binding<Bool>,
        @ViewBuilder card: @escaping () -> CardContent
    ) -> some View {
        overlay {
            if isPresented.wrappedValue {
                MoshpitCardPresenter { card() }
            }
        }
        .animation(Motion.settle, value: isPresented.wrappedValue)
    }
}

/// A one-decision information card — errors and notices that only need OK.
struct MoshpitNoticeCard: View {
    var icon: String = "exclamationmark.circle.fill"
    var tone: ModalTone = .danger
    let title: LocalizedStringKey
    let message: String
    var dismissLabel: LocalizedStringKey = "OK"
    let onDismiss: () -> Void

    var body: some View {
        MoshpitModalCard(
            icon: icon, tone: tone, title: title,
            message: message.isEmpty ? nil : Text(message),
            buttons: [ModalButton(dismissLabel, kind: .secondary, action: onDismiss)]
        ) { EmptyView() }
    }
}

/// A name-prompt card — the rename / create flows' single text field, focused
/// on arrival so the keyboard is already up when the card lands.
struct MoshpitInputCard: View {
    var icon: String = "character.cursor.ibeam"
    let title: LocalizedStringKey
    var message: Text?
    let placeholder: LocalizedStringKey
    @Binding var text: String
    let confirmLabel: LocalizedStringKey
    /// A field whose text is never shown (a passphrase).
    var secure = false
    /// What the field holds, for the keyboard's AutoFill (a one-time code).
    var contentType: UITextContentType?
    let onCancel: () -> Void
    let onConfirm: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        MoshpitModalCard(
            icon: icon, tone: .violet, title: title, message: message,
            buttons: [
                ModalButton("Cancel", kind: .secondary, action: onCancel),
                ModalButton(confirmLabel, kind: .primary, action: onConfirm),
            ]
        ) {
            field
                .font(Face.mono(14))
                .foregroundStyle(Ink.primary)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .textContentType(contentType)
                .submitLabel(.done)
                .onSubmit(onConfirm)
                .focused($focused)
                .padding(EdgeInsets(top: 12, leading: 13, bottom: 12, trailing: 13))
                .background(Ink.neutralFill,
                            in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 11, style: .continuous)
                    .strokeBorder(Ink.hairline, lineWidth: 1))
                .task {
                    // Give the card's scale/fade transition a beat before
                    // summoning the keyboard, or the two animations fight.
                    try? await Task.sleep(nanoseconds: 350_000_000)
                    focused = true
                }
        }
    }

    @ViewBuilder
    private var field: some View {
        if secure {
            SecureField(placeholder, text: $text)
        } else {
            TextField(placeholder, text: $text)
        }
    }
}

/// The card that asks once for the passphrase of a key stored behind one
/// (``TerminalViewModel/KeyPassphrasePrompt``), shared by the home and
/// terminal screens like ``hostKeyPromptCard(_:)``.
struct KeyPassphrasePromptCard: View {
    let prompt: TerminalViewModel.KeyPassphrasePrompt
    @State private var passphrase = ""

    var body: some View {
        MoshpitInputCard(
            icon: "key.fill",
            title: "Key Passphrase",
            message: Text(prompt.wrong
                ? String(localized: "That passphrase doesn't unlock this key.")
                : String(localized: "The key for \(prompt.host) is locked with a passphrase. Enter it once: from then on the Keychain on this device protects the key, as it does every key you import.")),
            placeholder: "Passphrase",
            text: $passphrase,
            confirmLabel: "Unlock",
            secure: true,
            onCancel: { prompt.decide(nil) },
            onConfirm: { prompt.decide(passphrase) })
    }
}

/// What the server asks at sign-in beyond the password — a one-time code
/// (``TerminalViewModel/SignInQuestion``, B563) — in its own words, shared
/// by the home and terminal screens like ``KeyPassphrasePromptCard``.
struct SignInQuestionCard: View {
    let question: TerminalViewModel.SignInQuestion
    @State private var answer = ""

    var body: some View {
        MoshpitInputCard(
            icon: "lock.rotation",
            title: "Sign In to \(question.host)",
            message: Text(verbatim: question.words),
            placeholder: "Answer",
            text: $answer,
            confirmLabel: "Send",
            secure: !question.echo,
            contentType: .oneTimeCode,
            onCancel: { question.decide(nil) },
            onConfirm: { question.decide(answer) })
    }
}

/// The shared TOFU host-key card, built for both the home and terminal screens
/// so the two presentations can't drift. `prompt.decide(_:)` clears the prompt
/// and resumes the suspended handshake.
@ViewBuilder
func hostKeyPromptCard(_ prompt: TerminalViewModel.HostKeyPrompt) -> some View {
    if let previous = prompt.previousFingerprint {
        MoshpitModalCard(
            icon: "exclamationmark.triangle.fill",
            tone: .danger,
            title: "Host Key Changed",
            message: Text("The key for \(prompt.host):\(String(prompt.port)) does **not** match what's stored here. The server may have been reinstalled — or the connection is being intercepted."),
            buttons: [
                ModalButton("Trust New Key", kind: .danger) { prompt.decide(true) },
                ModalButton("Disconnect", kind: .secondary) { prompt.decide(false) },
            ],
            stackButtons: true
        ) {
            VStack(spacing: 8) {
                MonoCodeBlock(label: "Stored", text: previous)
                MonoCodeBlock(label: "Offered now", text: prompt.fingerprint, accent: true)
            }
        }
    } else {
        MoshpitModalCard(
            icon: "lock.shield.fill",
            tone: .violet,
            title: "New Host",
            message: Text("First connection to \(prompt.host):\(String(prompt.port)). Verify this fingerprint matches the server before you trust it."),
            buttons: [
                ModalButton("Cancel", kind: .secondary) { prompt.decide(false) },
                ModalButton("Trust", kind: .primary) { prompt.decide(true) },
            ]
        ) {
            VStack(spacing: 8) {
                MonoCodeBlock(label: "SHA256 fingerprint", text: prompt.fingerprint, accent: true)
                // The key that came by ssh-rsa is an RSA key (B564).
                MonoCodeBlock(label: "Verify on server", text: prompt.olderAlgorithm == nil
                    ? "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub"
                    : "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_rsa_key.pub")
                if let older = prompt.olderAlgorithm {
                    Label {
                        Text("This server uses an older host-key algorithm (\(older), SHA-1).")
                    } icon: {
                        Image(systemName: "exclamationmark.triangle.fill")
                    }
                    .font(Face.text(13))
                    .foregroundStyle(Ink.warnText)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}
