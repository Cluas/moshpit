import SwiftUI
import UIKit

/// One action in a ``SwipeTrayRow``: a tinted square with a symbol. The
/// title is its accessibility label; the long-press menu carries the words.
struct SwipeTrayAction: Identifiable {
    let id: String
    let title: String
    let systemImage: String
    let tint: Color
    let action: () -> Void
}

/// Swipe-to-reveal actions drawn INSIDE the row, on the card's own surface.
///
/// `List.swipeActions` slides the whole cell — background included — and
/// puts its buttons on the list background behind it. On a card built from
/// rows that tears the card: the swiped row (worst of all the head) slides
/// away from the rest, leaving a slab of page behind three system pills
/// (真机 2026-09-16). Here the row's background stays where it is; only the
/// content moves, and the actions surface in the space it vacates, in the
/// card's own button vocabulary (the `+` / `↻` / `↗` squares).
///
/// Mechanics: a horizontal-only UIKit pan on the row (``HorizontalPanGesture``).
/// UIKit rather than a SwiftUI `DragGesture` because a vertical drag starting
/// on a row must still scroll the list, and a `DragGesture` at high priority
/// claims both directions — the list stopped scrolling under every tray row
/// (rig, 2026-09-16) — while at normal priority the row's button fires after
/// the swipe. The recognizer fails a vertical movement before it begins, so
/// the list's own pan takes it; when it does begin, UIKit cancels the touch
/// to the content, so the row's button does not fire, and a plain tap never
/// moves far enough to begin it. One row per card is open at a time
/// (`openRow`); tapping an open row's content closes it rather than acting.
struct SwipeTrayRow<Content: View>: View {
    enum Style {
        /// The compact tree rows: 28pt squares.
        case tree
        /// The card head: 40pt squares, the row being three times as tall.
        case head

        var buttonSize: CGFloat { self == .head ? 40 : 28 }
        var iconSize: CGFloat { self == .head ? 15 : 12 }
        var cornerRadius: CGFloat { self == .head ? 11 : 8 }
        var spacing: CGFloat { self == .head ? 12 : 8 }
        var edgePadding: CGFloat { self == .head ? 16 : 6 }
    }

    let id: String
    @Binding var openRow: String?
    let actions: [SwipeTrayAction]
    var style: Style = .tree
    @ViewBuilder let content: () -> Content

    @State private var dragTranslation: CGFloat = 0

    private var isOpen: Bool { openRow == id }

    private var trayWidth: CGFloat {
        guard !actions.isEmpty else { return 0 }
        return CGFloat(actions.count) * style.buttonSize
            + CGFloat(actions.count - 1) * style.spacing
            + style.edgePadding * 2
    }

    /// Where the content sits: closed at 0, open at -trayWidth, the live
    /// drag added on top. A little overdrag past open, none past closed.
    private var offset: CGFloat {
        let base: CGFloat = isOpen ? -trayWidth : 0
        return min(0, max(-trayWidth - 14, base + dragTranslation))
    }

    private var progress: CGFloat {
        trayWidth > 0 ? min(1, -offset / trayWidth) : 0
    }

    var body: some View {
        ZStack(alignment: .trailing) {
            content()
                .offset(x: offset)
                .overlay {
                    if isOpen {
                        // Tapping the content of an open row closes it.
                        Color.clear
                            .contentShape(Rectangle())
                            .onTapGesture { close() }
                    }
                }
                // On the content, not the stack: the tray's buttons are
                // siblings, so a pan over them would swallow their taps.
                .gesture(pan)
            if offset < 0 {
                // Above the content, so its buttons take the tap whatever
                // the slid content's hit area claims; masked to the space
                // the content has vacated, so it never paints over content
                // mid-drag.
                tray
                    .frame(width: -offset, alignment: .trailing)
                    .clipped()
            }
        }
        .clipped()
        .onChange(of: openRow) { _, now in
            // Another row opened, or the card closed us: settle any leftover
            // translation so the next drag starts from rest.
            if now != id { dragTranslation = 0 }
        }
    }

    private var tray: some View {
        HStack(spacing: style.spacing) {
            ForEach(actions) { action in
                Button {
                    close()
                    action.action()
                } label: {
                    Image(systemName: action.systemImage)
                        .font(.system(size: style.iconSize, weight: .semibold))
                        .foregroundStyle(action.tint)
                        .frame(width: style.buttonSize, height: style.buttonSize)
                        .background(action.tint.opacity(0.14),
                                    in: RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous)
                            .strokeBorder(action.tint.opacity(0.32), lineWidth: 1))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(action.title)
            }
        }
        .padding(.horizontal, style.edgePadding)
        .opacity(0.35 + 0.65 * progress)
        .scaleEffect(0.88 + 0.12 * progress, anchor: .trailing)
    }

    private var pan: HorizontalPanGesture {
        HorizontalPanGesture(isEnabled: !actions.isEmpty) { translation in
            if let open = openRow, open != id {
                withAnimation(.snappy(duration: 0.22)) { openRow = nil }
            }
            dragTranslation = translation
        } onEnded: { translation, velocity in
            // Where the content would come to rest if the finger's velocity
            // carried it a little further: past halfway means open.
            let base: CGFloat = isOpen ? -trayWidth : 0
            let projected = base + translation + velocity * 0.1
            let shouldOpen = -projected > trayWidth / 2
            if shouldOpen, !isOpen { Haptics.select() }
            withAnimation(.snappy(duration: 0.25)) {
                openRow = shouldOpen ? id : (openRow == id ? nil : openRow)
                dragTranslation = 0
            }
        }
    }

    private func close() {
        withAnimation(.snappy(duration: 0.22)) {
            if openRow == id { openRow = nil }
            dragTranslation = 0
        }
    }
}

/// A pan that begins only for a horizontal movement. A vertical one fails
/// while the recognizer is still `possible`, before UIKit's own 10pt
/// hysteresis would begin it, so the list's pan (an ancestor, vertical only)
/// takes the touch and scrolls; a horizontal one fails the list's pan
/// instead and begins ours. Translation and velocity are the x components,
/// in points; `onEnded` is also called for a cancelled pan, with zero
/// velocity.
struct HorizontalPanGesture: UIGestureRecognizerRepresentable {
    var isEnabled = true
    let onChanged: (CGFloat) -> Void
    let onEnded: (CGFloat, CGFloat) -> Void

    func makeUIGestureRecognizer(context: Context) -> HorizontalPanRecognizer {
        let recognizer = HorizontalPanRecognizer()
        recognizer.maximumNumberOfTouches = 1
        recognizer.isEnabled = isEnabled
        return recognizer
    }

    func updateUIGestureRecognizer(_ recognizer: HorizontalPanRecognizer, context: Context) {
        recognizer.isEnabled = isEnabled
    }

    func handleUIGestureRecognizerAction(_ recognizer: HorizontalPanRecognizer, context: Context) {
        let translation = context.converter.translation(in: .local)?.x
            ?? recognizer.translation(in: recognizer.view).x
        switch recognizer.state {
        case .began, .changed:
            onChanged(translation)
        case .ended:
            let velocity = context.converter.velocity(in: .local)?.x
                ?? recognizer.velocity(in: recognizer.view).x
            onEnded(translation, velocity)
        case .cancelled:
            onEnded(translation, 0)
        default:
            break
        }
    }
}

final class HorizontalPanRecognizer: UIPanGestureRecognizer {
    private var start: CGPoint?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        if start == nil { start = touches.first?.location(in: view) }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible, let start, let point = touches.first?.location(in: view) {
            let dx = point.x - start.x
            let dy = point.y - start.y
            if hypot(dx, dy) >= 8, abs(dy) > abs(dx) {
                state = .failed
                return
            }
        }
        super.touchesMoved(touches, with: event)
    }

    override func reset() {
        super.reset()
        start = nil
    }
}
