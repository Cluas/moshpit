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

// MARK: - Form group (a Form section)

/// Section container for the app's forms. A thin wrapper over `Section`, so
/// call sites keep their `FormGroup(title:footer:)` shape while the rows get
/// the system's inset-grouped chrome: row insets, separators, header and
/// footer typography, Dynamic Type, and the highlight on tap. Must sit inside
/// a `Form` or `List` (see `moshpitForm()`).
struct FormGroup<Content: View>: View {
    var title: LocalizedStringKey?
    var titleSuffix: LocalizedStringKey?
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
                SectionKicker(title: title, suffix: titleSuffix)
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
    var suffix: LocalizedStringKey?
    /// A count riding beside the title as a chip, the way CONNECTIONS · 1
    /// has always read on the home screen.
    var count: Int?

    var body: some View {
        HStack(spacing: 7) {
            Text(title)
                .font(Face.mono(10.5, .semibold))
                .kerning(1.2)
                .foregroundStyle(Ink.sectionTitle)
                .textCase(.uppercase)
            if let count {
                CountBadge(count: count)
            }
            if let suffix {
                // The "(optional)" aside reads as an aside only in its own case.
                Text(suffix)
                    .font(Face.text(11))
                    .foregroundStyle(Ink.meta)
                    .textCase(nil)
            }
        }
        .padding(.leading, 2)
    }
}

extension View {
    /// The app's Form/List chrome: the system inset-grouped layout on the
    /// theme's screen colour instead of the system grouped background.
    func moshpitForm() -> some View {
        self
            .scrollContentBackground(.hidden)
            .background(Ink.screenBG)
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
    /// `.pickerStyle(.segmented)` exposes nothing else.
    @MainActor static func installAppearance() {
        let seg = UISegmentedControl.appearance()
        seg.backgroundColor = UIColor(Ink.segTrack)
        seg.selectedSegmentTintColor = UIColor(Ink.segActive)
        seg.setTitleTextAttributes([.foregroundColor: UIColor(Ink.secondary)], for: .normal)
        seg.setTitleTextAttributes([.foregroundColor: UIColor(Ink.primary)], for: .selected)
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

/// Label + a short numeric field (port, range bound). Right-aligned in a
/// fixed column beside the label; at accessibility sizes `LabeledContent`
/// stacks the field under the label, where a fixed right-aligned column
/// would read as a random indent, so the field goes leading and full width.
struct NumberFieldRow: View {
    let label: LocalizedStringKey
    let placeholder: LocalizedStringKey
    @Binding var text: String
    var mono: Bool = false
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        let stacked = dynamicTypeSize.isAccessibilitySize
        LabeledContent {
            TextField(placeholder, text: $text)
                .keyboardType(.numberPad)
                .font(mono ? .system(.body, design: .monospaced) : .body)
                .foregroundStyle(Ink.fixedValue)
                .multilineTextAlignment(stacked ? .leading : .trailing)
                .frame(width: stacked ? nil : 100)
        } label: {
            Text(label).font(.body).foregroundStyle(Ink.primary)
        }
    }
}

/// Label + fixed (non-interactive) trailing value.
struct ValueRow: View {
    let label: LocalizedStringKey
    /// Dynamic, pre-formatted display value (kept `String` on purpose).
    let value: String
    var valueMono: Bool = false

    var body: some View {
        LabeledContent {
            Text(value)
                .font(valueMono ? .system(.body, design: .monospaced) : .body)
                .foregroundStyle(Ink.fixedValue)
        } label: {
            Text(label).font(.body).foregroundStyle(Ink.primary)
        }
    }
}

/// Label (+ optional subtitle) + trailing dimmed value + chevron. Tappable —
/// the row highlights like any list row, and opens a sheet or editor.
struct ChevronRow: View {
    let label: LocalizedStringKey
    var subtitle: LocalizedStringKey?
    /// Dynamic, pre-formatted display value (kept `String` on purpose).
    var value: String?
    var action: () -> Void = {}
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                // At accessibility sizes the value drops under the label, the
                // way `LabeledContent` does, instead of truncating to "Signal R…".
                let stacked = dynamicTypeSize.isAccessibilitySize
                let layout = stacked
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: 4))
                    : AnyLayout(HStackLayout(spacing: 10))
                layout {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(label).font(.body).foregroundStyle(Ink.primary)
                        if let subtitle {
                            Text(subtitle)
                                .font(.footnote)
                                .foregroundStyle(Ink.meta)
                                .multilineTextAlignment(.leading)
                        }
                    }
                    if !stacked { Spacer(minLength: 8) }
                    if let value {
                        Text(value)
                            .font(.body)
                            .foregroundStyle(Ink.meta)
                            .lineLimit(stacked ? nil : 1)
                            .multilineTextAlignment(.leading)
                    }
                }
                if stacked { Spacer(minLength: 8) }
                MiniChevron()
            }
        }
    }
}

/// Title (+ optional subtitle) + trailing switch.
///
/// Laid out by hand rather than as `Toggle(label)`: SwiftUI folds a labelled
/// toggle into one accessibility element, which hides the title from
/// `staticTexts` queries the UI tests (and VoiceOver's rotor) rely on.
struct ToggleRow: View {
    let label: LocalizedStringKey
    var subtitle: LocalizedStringKey?
    /// Stable handle for UI tests. The row's Toggle carries an EMPTY label
    /// (the visible text is our own views), so to XCUI it is an anonymous
    /// switch — unaddressable without this.
    var identifier: String?
    @Binding var isOn: Bool

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(label).font(.body).foregroundStyle(Ink.primary)
                if let subtitle {
                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(Ink.meta)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            Toggle("", isOn: $isOn)
                .labelsHidden()
                .tint(Ink.accent)
                .accessibilityLabel(Text(label))
                .accessibilityIdentifier(identifier ?? "")
        }
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

/// The system's indicator for a row that opens a menu in place (the one a
/// `.menu` Picker draws), for rows built on `Menu` because their labels carry
/// more than a title.
struct MenuChevron: View {
    var body: some View {
        Image(systemName: "chevron.up.chevron.down")
            .font(.caption2.weight(.semibold))
            .foregroundStyle(Ink.meta)
    }
}

// MARK: - Segmented control

struct SegItem<Value: Hashable>: Identifiable {
    let value: Value
    let label: LocalizedStringKey
    /// Kept for call-site compatibility; the system segmented control shows
    /// titles only, so glyph and badge are not drawn.
    var systemImage: String?
    var trailingBadge: LocalizedStringKey?
    var id: Value { value }
}

/// The system segmented control. Segments are titles only — the same control
/// iOS Settings uses — so it reads as a control, not a custom pill strip.
struct PillSegmentedControl<Value: Hashable>: View {
    let items: [SegItem<Value>]
    @Binding var selection: Value

    var body: some View {
        Picker(selection: $selection) {
            ForEach(items) { item in
                Text(item.label).tag(item.value)
            }
        } label: {
            EmptyView()
        }
        .pickerStyle(.segmented)
        .labelsHidden()
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

extension TransportConnState {
    /// The one colour for this state, for every surface that shows it.
    ///
    /// There used to be three independent mappings — the pill painted
    /// connecting *and* reconnecting amber, ``TerminalConnectingView`` painted
    /// them accent and blue, and the home card's row painted both amber with a
    /// "WAIT" caption. One event, three colours, and on the connecting screen
    /// the reconnect visibly changed hue partway through. A state that means one
    /// thing has to look like one thing, so the mapping lives here and the
    /// surfaces read it.
    ///
    /// Amber is deliberately not in this list. It is the app's "an agent needs
    /// you" colour, and spending it on a transport hiccup is what made a
    /// reconnect shout as loudly as an agent waiting on a human.
    /// `nil` for ``live``: a healthy session's colour belongs to its transport
    /// (SSH and mosh read differently on the pill), so callers keep their own.
    var transientTint: Color? {
        switch self {
        // The user's own accent: a first connect is the app doing what it was
        // asked, not a fault.
        case .connecting: return Ink.accent
        // Fixed transport blue, theme or no theme — the same hue the mosh roam
        // banner uses, because a reconnect is that same story: the line is
        // moving, not gone.
        case .reconnecting: return Ink.signal
        // Nothing is being attempted. This is the only one that earns red.
        case .offline: return Ink.danger
        case .live: return nil
        }
    }
}

struct TransportPill: View {
    let kind: TransportPillKind
    var connState: TransportConnState = .live
    @State private var pulsing = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The state the LABEL shows — which deliberately lags the state the dot
    /// shows. The dot recolors and pulses the instant anything changes (a
    /// dropped session is never silent), but swapping the pill's TEXT is a
    /// layout event: "tmux" → "reconnecting" more than doubles the pill and
    /// shoves the whole header. Health flaps routinely bounce through
    /// reconnecting and back within a second, and every bounce used to slam a
    /// long word in and out of the layout. Now a transient state must STAND
    /// for a beat before it earns words; recovery snaps back immediately.
    @State private var labelState: TransportConnState = .live
    @State private var labelDebounce: Task<Void, Never>?

    private var dotColor: Color {
        connState.transientTint ?? kind.dotColor
    }

    private var label: String {
        switch labelState {
        // Reconnecting deliberately keeps the transport word. The word
        // "reconnecting" more than doubled the pill and shoved the whole
        // header, for a state that routinely lasts under a second — the story
        // is told by MOTION instead: the dot pulses in transport blue and the
        // capsule's border breathes the same colour. Only `offline`, the state
        // where nothing is being attempted, still earns a word.
        case .live, .connecting, .reconnecting: return kind.label
        case .offline: return String(localized: "offline")
        }
    }

    private func adoptLabelState(_ new: TransportConnState) {
        labelDebounce?.cancel()
        switch new {
        case .live, .connecting:
            // Good news and no-text states apply at once.
            withAnimation(.snappy(duration: 0.25)) { labelState = new }
        case .reconnecting, .offline:
            labelDebounce = Task {
                try? await Task.sleep(for: .milliseconds(800))
                guard !Task.isCancelled else { return }
                withAnimation(.snappy(duration: 0.25)) { labelState = new }
            }
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
                .lineLimit(1)
                .fixedSize()
        }
        .padding(EdgeInsets(top: 7, leading: 10, bottom: 7, trailing: 10))
        .background(kind.bg, in: Capsule(style: .continuous))
        .overlay {
            Capsule(style: .continuous)
                .strokeBorder(
                    connState.transientTint.map { $0.opacity(pulsing ? 0.7 : 0.25) }
                        ?? kind.border ?? Color.white.opacity(0.09),
                    lineWidth: 1)
        }
        // The width change itself animates, so when "offline" does earn its
        // place the pill grows instead of teleporting.
        .animation(.snappy(duration: 0.25), value: label)
        .onChange(of: connState, initial: true) { _, new in
            adoptLabelState(new)
        }
        .onAppear {
            guard animates, !reduceMotion else { return }
            withAnimation(Motion.roamPulse) {
                pulsing = true
            }
        }
        .onChange(of: animates) { _, on in
            if on, !reduceMotion {
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
            .lineLimit(1)
            .fixedSize()
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

/// Shared chrome for the tmux Windows / Sessions / Select Pane sheets: a
/// system sheet at medium/large detents, an inline title with the ＋ in the
/// bar, an inset-grouped `List` for the rows, and the keyboard hints as the
/// section's footer. iOS 26 gives the sheet its Liquid Glass surface on its
/// own — setting a presentation background would take that away, so the
/// dark fill is only applied where there is no glass to lose (iOS 18).
struct CompactSheet<Content: View>: View {
    let title: LocalizedStringKey
    var onPlus: (() -> Void)?
    let footerHint: LocalizedStringKey
    /// Keyboard glyph chips (⌃, b s, …) — never localized.
    let footerKeys: [String]
    @ViewBuilder var content: Content

    var body: some View {
        NavigationStack {
            List {
                Section {
                    content
                        .moshpitRows()
                } footer: {
                    HStack(alignment: .firstTextBaseline) {
                        Text(footerHint)
                            .moshpitFooter()
                        Spacer()
                        HStack(spacing: 4) {
                            ForEach(footerKeys, id: \.self) { KeyHintChip(label: $0) }
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if let onPlus {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(action: onPlus) {
                            Label("New", systemImage: "plus")
                        }
                        .foregroundStyle(Ink.accent)
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .sheetSurface()
        .preferredColorScheme(.dark)
    }
}

extension View {
    /// The tmux sheets' surface: nothing on iOS 26 (the system sheet is
    /// already glass, and any explicit background replaces it with paint),
    /// the app's dark sheet fill on iOS 18 where the default is flat.
    @ViewBuilder func sheetSurface() -> some View {
        if #available(iOS 26.0, *) {
            self
        } else {
            self.presentationBackground(Ink.sheet)
        }
    }

    /// Chrome capsules and circles in the terminal's top bar: Liquid Glass on
    /// iOS 26, the bar material with a hairline below it.
    @ViewBuilder func glassChrome<S: InsettableShape>(in shape: S) -> some View {
        if #available(iOS 26.0, *) {
            self.glassEffect(.regular.interactive(), in: shape)
        } else {
            self.background(.bar, in: shape)
                .overlay(shape.strokeBorder(Ink.groupBorder, lineWidth: 1))
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
    var statusColor: Color?
    /// What the dot MEANS, for people who can't lean on its colour — read by
    /// VoiceOver and appended to the row's label ("Claude Code, needs you").
    /// A colour with no words is a signal only some users receive.
    var statusLabel: String?
    /// Small mono text before the dot — the Agents section's "for how long".
    var trailing: String?
    var action: () -> Void

    /// A plain list row: the current item is said by the checkmark, the way
    /// every system picker says it, not by filling the whole row in accent.
    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(isActive ? Ink.accent : Ink.secondary)
                    .frame(width: 28)
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(.body).foregroundStyle(Ink.primary)
                    if !meta.isEmpty {
                        Text(meta)
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(Ink.meta)
                    }
                }
                Spacer(minLength: 8)
                if let trailing {
                    Text(trailing)
                        .font(.system(.footnote, design: .monospaced))
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
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Ink.accent)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // A pointer on the iPad lights the row the way it lights a list cell.
        .hoverEffect(.highlight)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilitySummary)
        .accessibilityAddTraits(isActive ? .isSelected : [])
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
