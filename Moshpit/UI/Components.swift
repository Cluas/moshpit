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
    static func warning() { UINotificationFeedbackGenerator().notificationOccurred(.warning) }
}

// MARK: - Form group (iOS inset-grouped list, prototype §3.2)

/// Section container: ALL-CAPS title, rounded group with hairline-separated
/// rows, optional grey helper footnote below.
struct FormGroup<Content: View>: View {
    var title: LocalizedStringKey?
    var titleSuffix: LocalizedStringKey?
    var footer: LocalizedStringKey?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let title {
                HStack(spacing: 4) {
                    // Mono section labels — part of Moshpit's terminal voice.
                    Text(title)
                        .font(Face.mono(10.5))
                        .kerning(0.9)
                        .foregroundStyle(Ink.sectionTitle)
                    if let titleSuffix {
                        Text(titleSuffix)
                            .font(Face.text(11))
                            .foregroundStyle(Color.white.opacity(0.36))
                    }
                }
                .padding(EdgeInsets(top: 20, leading: 2, bottom: 8, trailing: 2))
            }
            _VariadicGroupRows(content: content)
                .padding(.horizontal, Metrics.groupInset)
                .background(Ink.group)
                .clipShape(RoundedRectangle(cornerRadius: Metrics.groupRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.groupRadius, style: .continuous)
                        .strokeBorder(Ink.groupBorder, lineWidth: 1)
                )
                .shadow(color: Ink.insetShadow.opacity(0.42), radius: 18, y: 10)
            if let footer {
                Text(footer)
                    .font(Face.text(12))
                    .foregroundStyle(Ink.tertiary)
                    .lineSpacing(3)
                    .padding(EdgeInsets(top: 8, leading: 16, bottom: 0, trailing: 16))
            }
        }
    }
}

/// Interleaves hairline separators between the group's rows.
private struct _VariadicGroupRows<Content: View>: View {
    let content: Content
    var body: some View {
        Group(subviews: content) { subviews in
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(subviews.enumerated()), id: \.offset) { index, subview in
                    subview
                    if index < subviews.count - 1 {
                        Rectangle().fill(Ink.hairline).frame(height: 1)
                    }
                }
            }
        }
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
                SecureField("", text: $text, prompt: prompt)
            } else if multiline {
                TextField("", text: $text, prompt: prompt, axis: .vertical)
                    .lineLimit(1...4)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            } else {
                TextField("", text: $text, prompt: prompt)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            }
        }
        .font(mono ? Face.mono(14) : Face.text(14))
        .foregroundStyle(Ink.primary)
        .multilineTextAlignment(alignment)
        .frame(minHeight: Metrics.cellMinHeight)
    }

    private var prompt: Text {
        Text(placeholder).font(Face.text(14)).foregroundStyle(Ink.placeholder)
    }
}

/// Label + fixed (non-interactive) trailing value.
struct ValueRow: View {
    let label: LocalizedStringKey
    /// Dynamic, pre-formatted display value (kept `String` on purpose).
    let value: String
    var valueMono: Bool = false

    var body: some View {
        HStack(spacing: 10) {
            Text(label).font(Face.text(14)).foregroundStyle(Ink.primary)
            Spacer()
            Text(value)
                .font(valueMono ? Face.mono(14) : Face.text(14))
                .foregroundStyle(Ink.fixedValue)
        }
        .frame(minHeight: Metrics.cellMinHeight)
    }
}

/// Label (+ optional subtitle) + trailing dimmed value + mini chevron. Tappable.
struct ChevronRow: View {
    let label: LocalizedStringKey
    var subtitle: LocalizedStringKey?
    /// Dynamic, pre-formatted display value (kept `String` on purpose).
    var value: String?
    var action: () -> Void = {}

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(label).font(Face.text(14)).foregroundStyle(Ink.primary)
                    if let subtitle {
                        Text(subtitle)
                            .font(Face.text(11))
                            .foregroundStyle(Ink.meta)
                            .lineSpacing(2)
                            .multilineTextAlignment(.leading)
                    }
                }
                Spacer()
                if let value {
                    Text(value).font(Face.text(14)).foregroundStyle(Ink.meta)
                }
                MiniChevron()
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// Title (+ optional subtitle) + trailing switch.
struct ToggleRow: View {
    let label: LocalizedStringKey
    var subtitle: LocalizedStringKey?
    @Binding var isOn: Bool

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                    Text(label).font(Face.text(14)).foregroundStyle(Ink.primary)
                if let subtitle {
                    Text(subtitle)
                        .font(Face.text(11))
                        .foregroundStyle(Ink.meta)
                        .lineSpacing(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            Toggle("", isOn: $isOn)
                .labelsHidden()
                .tint(Ink.accent)
                .scaleEffect(0.82, anchor: .trailing)
        }
        .frame(minHeight: Metrics.cellMinHeight)
        .padding(.vertical, 4)
    }
}

struct MiniChevron: View {
    var color: Color = Ink.meta
    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(color)
    }
}

// MARK: - Segmented control (prototype styling, supports glyph leading)

struct SegItem<Value: Hashable>: Identifiable {
    let value: Value
    let label: LocalizedStringKey
    var systemImage: String?
    var trailingBadge: LocalizedStringKey?
    var id: Value { value }
}

struct PillSegmentedControl<Value: Hashable>: View {
    let items: [SegItem<Value>]
    @Binding var selection: Value

    var body: some View {
        HStack(spacing: 2) {
            ForEach(items) { item in
                let active = item.value == selection
                Button {
                    withAnimation(.easeOut(duration: 0.15)) { selection = item.value }
                } label: {
                    HStack(spacing: 5) {
                        if let glyph = item.systemImage {
                            Image(systemName: glyph)
                                .font(.system(size: 12, weight: .medium))
                                .opacity(active ? 1 : 0.55)
                        }
                        Text(item.label).font(Face.text(12, .medium))
                        if let badge = item.trailingBadge {
                            Text(badge)
                                .font(Face.mono(8.5, .bold))
                                .kerning(0.7)
                                .padding(EdgeInsets(top: 1, leading: 4, bottom: 1, trailing: 4))
                                .background(
                                    active ? Ink.accent.opacity(0.22) : Ink.signal.opacity(0.16),
                                    in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                                .foregroundStyle(active ? Ink.accent : Ink.signalSoft)
                        }
                    }
                    .foregroundStyle(active ? Ink.primary : Ink.secondary)
                    .frame(maxWidth: .infinity, minHeight: 30)
                    .background(
                        active ? Ink.segActive : .clear,
                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                    .overlay {
                        if active {
                            RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                .strokeBorder(Ink.accent.opacity(0.20), lineWidth: 1)
                        }
                    }
                }
                .buttonStyle(.plain)
            }
        }
        .padding(2)
        .background(Ink.segTrack, in: RoundedRectangle(cornerRadius: Metrics.controlRadius + 2, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.controlRadius + 2, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1)
        )
        .padding(.vertical, 8)
    }
}

// MARK: - Status pills (§3.10)

enum TransportPillKind {
    case mosh, moshRoaming, ssh

    var label: String {
        switch self {
        case .mosh: return "MOSH"
        case .moshRoaming: return "MOSH · roaming"
        case .ssh: return "SSH"
        }
    }
    var bg: Color {
        switch self {
        case .mosh: return Ink.moshPillBG
        case .moshRoaming: return Ink.roamPillBG
        case .ssh: return Ink.sshPillBG
        }
    }
    var fg: Color {
        switch self {
        case .mosh: return Ink.moshPillText
        case .moshRoaming: return Ink.roamPillText
        case .ssh: return Ink.sshPillText
        }
    }
    var border: Color? {
        switch self {
        case .mosh: return Ink.moshPillBorder
        case .moshRoaming: return Ink.roamPillBorder
        // Explicit .clear (not nil): skips the shared fallback stroke so this
        // reads as a soft, borderless chip — like the breadcrumb crumbs it
        // sits beside — instead of a second outlined box next to theirs.
        case .ssh: return Color.clear
        }
    }
    var dotColor: Color {
        switch self {
        case .mosh: return Ink.mosh
        case .moshRoaming: return Ink.warn
        case .ssh: return Ink.success
        }
    }
}

/// Live connection state, surfaced on the transport pill so a dropped/
/// reconnecting session is never silent.
enum TransportConnState { case live, connecting, reconnecting, offline }

struct TransportPill: View {
    let kind: TransportPillKind
    var connState: TransportConnState = .live
    @State private var pulsing = false

    private var dotColor: Color {
        switch connState {
        case .live: return kind.dotColor
        case .connecting, .reconnecting: return Ink.warn
        case .offline: return Ink.danger
        }
    }

    private var label: String {
        switch connState {
        case .live: return kind.label
        case .connecting: return kind.label
        case .reconnecting: return String(localized: "reconnecting")
        case .offline: return String(localized: "offline")
        }
    }

    private var animates: Bool {
        connState == .connecting || connState == .reconnecting || kind == .moshRoaming
    }

    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(dotColor)
                .frame(width: 6, height: 6)
                .shadow(color: dotColor.opacity(0.7), radius: 4)
                .scaleEffect(animates && pulsing ? 1.6 : 1)
                .opacity(animates && pulsing ? 0.55 : 1)
            Text(label)
                .font(Face.mono(10, .bold))
                .kerning(0.45)
                .foregroundStyle(kind.fg)
        }
        .padding(EdgeInsets(top: 7, leading: 10, bottom: 7, trailing: 10))
        .background(kind.bg, in: Capsule(style: .continuous))
        .overlay {
            Capsule(style: .continuous)
                .strokeBorder(kind.border ?? Color.white.opacity(0.09), lineWidth: 1)
        }
        .onAppear {
            guard animates else { return }
            withAnimation(Motion.roamPulse) {
                pulsing = true
            }
        }
        .onChange(of: animates) { _, on in
            if on {
                withAnimation(Motion.roamPulse) { pulsing = true }
            } else {
                withAnimation(.default) { pulsing = false }
            }
        }
    }
}

// MARK: - Shortcut chip (§3.5)

struct ShortcutChip: View {
    let label: String
    var custom: Bool = false
    var size: CGFloat = 11

    var body: some View {
        Text(label)
            .font(Face.mono(size, .semibold))
            .kerning(0.22)
            .foregroundStyle(custom ? Ink.customChipText : .white)
            .padding(EdgeInsets(top: 5, leading: 7, bottom: 5, trailing: 7))
            .frame(minWidth: 36)
            .background(
                custom ? Ink.customChipBG : Ink.chipNeutralBG,
                in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(custom ? Ink.customChipBorder : Ink.groupBorder, lineWidth: 1)
            )
    }
}

// MARK: - Host chip multiselect (§3.6)

struct HostChipsRow: View {
    let hosts: [String]
    @Binding var selected: Set<String>

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(hosts, id: \.self) { host in
                let on = selected.contains(host)
                Button {
                    if on { selected.remove(host) } else { selected.insert(host) }
                } label: {
                    HStack(spacing: 3) {
                        Text(on ? "−" : "+")
                            .font(Face.mono(14))
                            .foregroundStyle(on ? Ink.hostChipOnText : Ink.accent)
                        Text(host)
                            .font(Face.mono(11.5))
                            .foregroundStyle(on ? Ink.hostChipOnText : Color.white.opacity(0.72))
                    }
                    .padding(EdgeInsets(top: 6, leading: 10, bottom: 6, trailing: 10))
                    .background(
                        on ? Ink.hostChipOnBG : Color.white.opacity(0.055),
                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous).strokeBorder(
                            on ? Ink.hostChipOnBorder : Ink.faintFill, lineWidth: 1))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(EdgeInsets(top: 10, leading: 0, bottom: 8, trailing: 0))
    }
}

/// Minimal wrapping flow layout for chip rows.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        return CGSize(width: width, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                x = bounds.minX
                y += rowHeight + spacing
                rowHeight = 0
            }
            subview.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
    }
}

// MARK: - Color swatch selectors (§3.6)

struct ColorSwatchRow: View {
    let colors: [(id: String, color: Color)]
    @Binding var selection: String
    var diameter: CGFloat = 22

    var body: some View {
        HStack(spacing: 8) {
            ForEach(colors, id: \.id) { entry in
                let on = entry.id == selection
                Button {
                    selection = entry.id
                } label: {
                    Circle()
                        .fill(entry.color)
                        .frame(width: diameter, height: diameter)
                        .overlay(Circle().strokeBorder(Color.white.opacity(0.10), lineWidth: 1))
                        .overlay {
                            if on {
                                Circle().strokeBorder(Color.white.opacity(0.85), lineWidth: 2)
                                    .padding(-2)
                            }
                        }
                        .padding(2)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

// MARK: - Warn box (§3.7)

struct WarnBox: View {
    let title: LocalizedStringKey
    let body_: LocalizedStringKey

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 15))
                .foregroundStyle(Ink.warn)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(Face.text(12, .semibold)).foregroundStyle(Ink.warnBoxTitle)
                Text(body_)
                    .font(Face.text(11))
                    .foregroundStyle(Color.white.opacity(0.55))
                    .lineSpacing(2)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Ink.warn.opacity(0.10), in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.warn.opacity(0.22), lineWidth: 1))
        .padding(.vertical, 10)
    }
}

// MARK: - Strength meter (§3.8)

struct StrengthMeter: View {
    /// 0…1
    let strength: Double

    private var label: LocalizedStringKey {
        switch strength {
        case ..<0.34: return "WEAK"
        case ..<0.67: return "FAIR"
        default: return "STRONG"
        }
    }

    var body: some View {
        HStack(spacing: 10) {
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Ink.faintFill)
                    Capsule()
                        .fill(Ink.strengthFill)
                        .frame(width: geo.size.width * strength)
                        .shadow(color: Ink.mosh.opacity(0.6), radius: 4)
                }
            }
            .frame(height: 4)
            Text(label)
                .font(Face.mono(10.5))
                .kerning(0.84)
                .foregroundStyle(Ink.strengthLabel)
        }
        .padding(EdgeInsets(top: 10, leading: 0, bottom: 8, trailing: 0))
    }
}

// MARK: - Key hint chip (sheet footers)

struct KeyHintChip: View {
    let label: String
    var body: some View {
        Text(label)
            .font(Face.mono(11))
            .foregroundStyle(Ink.keyChipText)
            .padding(EdgeInsets(top: 2, leading: 5, bottom: 2, trailing: 5))
            .background(Ink.keyChipBG, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .strokeBorder(Ink.groupBorder, lineWidth: 1)
            )
    }
}

// MARK: - Compact bottom sheet scaffolding (§3.3)

/// Shared chrome for the tmux Windows / Sessions / Select Pane sheets:
/// grabber is provided by the system sheet; we render title row, content,
/// and footer hint bar. Present with `.presentationDetents` + dark background.
struct CompactSheet<Content: View>: View {
    let title: LocalizedStringKey
    var onPlus: (() -> Void)?
    let footerHint: LocalizedStringKey
    /// Keyboard glyph chips (⌃, b s, …) — never localized.
    let footerKeys: [String]
    @ViewBuilder var content: Content

    /// Natural height of (title + content + footer); drives a `.height` detent
    /// so the sheet hugs its content instead of always filling 68%.
    @State private var contentHeight: CGFloat = 0

    /// Hard cap (prototype §3.3: bottom sheet max-height 68%).
    private var maxHeight: CGFloat { UIScreen.main.bounds.height * Metrics.sheetMaxFraction }

    /// Resolved sheet height: content height (+ grabber room) clamped to the cap.
    private var sheetHeight: CGFloat {
        let natural = contentHeight + 28   // grabber + top breathing room
        return max(180, min(natural, maxHeight))
    }

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                Text(title)
                    .font(Face.display(17, .semibold))
                    .foregroundStyle(Ink.primary)
                if let onPlus {
                    HStack {
                        Spacer()
                        Button(action: onPlus) {
                            Image(systemName: "plus")
                                .font(.system(size: 14, weight: .semibold))
                                .foregroundStyle(Ink.accent)
                                .frame(width: 32, height: 32)
                                .background(Ink.accent.opacity(0.11), in: Circle())
                                .overlay(Circle().strokeBorder(Ink.accent.opacity(0.24), lineWidth: 1))
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(EdgeInsets(top: 14, leading: 14, bottom: 12, trailing: 14))

            ScrollView {
                content
                    .padding(.horizontal, 14)
            }
            // The ScrollView only needs to scroll when content exceeds the cap;
            // otherwise the .height detent already hugs it.
            .scrollBounceBehavior(.basedOnSize)

            footer
        }
        // Measure the natural laid-out height off-screen so the detent can
        // shrink to fit. A hidden copy avoids feeding the measurement back
        // into the visible ScrollView's flexible height.
        .background {
            VStack(spacing: 0) {
                Color.clear.frame(height: 50)            // title row ≈ 50
                content
                footerMetrics
            }
            .fixedSize(horizontal: false, vertical: true)
            .hidden()
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { contentHeight = $0 }
        }
        .presentationDetents([.height(sheetHeight)])
        .presentationDragIndicator(.visible)
        .presentationBackground(Ink.sheet)
        .presentationCornerRadius(Metrics.sheetRadius)
        .preferredColorScheme(.dark)
    }

    private var footer: some View {
        VStack(spacing: 0) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
            HStack {
                Text(footerHint)
                    .font(Face.mono(11, .medium))
                    .foregroundStyle(Ink.meta)
                Spacer()
                HStack(spacing: 4) {
                    ForEach(footerKeys, id: \.self) { KeyHintChip(label: $0) }
                }
            }
            .padding(EdgeInsets(top: 14, leading: 14, bottom: 12, trailing: 14))
        }
        .padding(.top, 8)
    }

    /// Same footprint as `footer`, for measurement only.
    private var footerMetrics: some View {
        HStack { Text(footerHint).font(Face.mono(11)); Spacer() }
            .padding(EdgeInsets(top: 23, leading: 14, bottom: 12, trailing: 14))
    }
}

/// Row used inside Windows / Sessions sheets.
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
    let action: () -> Void

    init(_ title: LocalizedStringKey, kind: Kind = .secondary, action: @escaping () -> Void) {
        self.title = title
        self.kind = kind
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
    }

    private var fg: Color {
        switch button.kind {
        case .secondary: return Ink.secondary
        case .primary: return Color(hex: "CFCEFF")
        case .danger: return Color(hex: "FFC4C4")
        }
    }
    private var bg: Color {
        switch button.kind {
        case .secondary: return Color.white.opacity(0.06)
        case .primary: return Ink.accent.opacity(0.16)
        case .danger: return Ink.danger.opacity(0.15)
        }
    }
    private var border: Color {
        switch button.kind {
        case .secondary: return Color.white.opacity(0.10)
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
        .background(Color.white.opacity(0.045), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
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
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Color.white.opacity(0.10), lineWidth: 1))
        .shadow(color: .black.opacity(0.55), radius: 30, y: 18)
    }
}

/// Scrim + centred card with a scale/fade transition. Not dismissed by tapping
/// the scrim — modals here demand a real decision.
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
        .preferredColorScheme(.dark)
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

/// The little count chip that rides next to a section title — one shape for
/// CONNECTIONS / AGENTS / WORKSPACES (and any future section) so every header
/// counts in the same voice.
struct CountBadge: View {
    let count: Int

    var body: some View {
        Text("\(count)")
            .font(Face.mono(10, .bold))
            .foregroundStyle(Ink.meta)
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(Color.white.opacity(0.06),
                        in: RoundedRectangle(cornerRadius: 4, style: .continuous))
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
            TextField(placeholder, text: $text)
                .font(Face.mono(14))
                .foregroundStyle(Ink.primary)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .submitLabel(.done)
                .onSubmit(onConfirm)
                .focused($focused)
                .padding(EdgeInsets(top: 12, leading: 13, bottom: 12, trailing: 13))
                .background(Color.white.opacity(0.045),
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
                MonoCodeBlock(label: "Verify on server", text: "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub")
            }
        }
    }
}

struct SheetListRow: View {
    let icon: String
    let name: String
    let meta: String
    let isActive: Bool
    /// Agent activity dot (shared `AgentPalette`): teal = working, amber =
    /// needs you, nil = no agent signal. Answers "which window needs me"
    /// right in the picker instead of forcing a guess by name.
    var statusColor: Color? = nil
    /// What the dot MEANS, for people who can't lean on its colour — read by
    /// VoiceOver and appended to the row's label ("Claude Code, needs you").
    /// A colour with no words is a signal only some users receive.
    var statusLabel: String? = nil
    /// Small mono text before the dot — the Agents section's "for how long".
    var trailing: String? = nil
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: icon)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(isActive ? Ink.accent : Ink.secondary)
                    .frame(width: 30, height: 30)
                    .background(
                        isActive ? Ink.accent.opacity(0.12) : Ink.neutralFill,
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                VStack(alignment: .leading, spacing: 1) {
                    Text(name).font(Face.text(14, .semibold)).foregroundStyle(Ink.primary)
                    Text(meta).font(Face.mono(11)).foregroundStyle(Ink.meta)
                }
                Spacer()
                if let trailing {
                    Text(trailing)
                        .font(Face.mono(11))
                        .foregroundStyle(Ink.meta)
                }
                if let statusColor {
                    Circle()
                        .fill(statusColor)
                        .frame(width: 8, height: 8)
                        .shadow(color: statusColor.opacity(0.8), radius: 4)
                }
                if isActive {
                    Image(systemName: "checkmark")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Ink.accent)
                        .frame(width: 18)
                }
            }
            .padding(EdgeInsets(top: 7, leading: 8, bottom: 7, trailing: 10))
            .frame(minHeight: 48)
            .background(
                isActive ? Ink.rowActiveBG : .clear,
                in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
            .overlay {
                if isActive {
                    RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                        .strokeBorder(Ink.accent.opacity(0.24), lineWidth: 1)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.vertical, 1)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilitySummary)
    }

    /// "name, meta, needs you, 2m, active" — the dot and the trailing time as
    /// words, in reading order.
    private var accessibilitySummary: String {
        var parts = [name, meta]
        if let statusLabel { parts.append(statusLabel) }
        if let trailing, trailing != statusLabel { parts.append(trailing) }
        if isActive { parts.append(String(localized: "active")) }
        return parts.filter { !$0.isEmpty }.joined(separator: ", ")
    }
}
