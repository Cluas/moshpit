import MoshpitFFI
import MoshpitKit
import UIKit

// Touch, the pointer and the edit menu on the core view, with the rules the
// SwiftTerm path learned (TerminalScrollGesture, docs/PATCHES.md):
//
// - one one-finger pan for both axes, locked to the first ~12 pt: vertical
//   scrolls (the owner routes it: wheel to a mouse app, tmux copy-mode,
//   herdr, or this view's own history by the pixel), horizontal switches
//   panes; the lift coasts at the drag's speed, each refresh's travel
//   routed as the drag's was (B521: the core's physics, paced and bounded
//   for a wheel or the host); a touch stops a coast, and isn't a tap;
// - a tap is a tap: it opens a link without taking the keyboard (patches
//   4, 13), raises the keyboard only on the cursor's own rows, clicks for a
//   program that wants the mouse, dismisses a selection (20); it never
//   fires on the lift of a long press;
// - a long press selects the word under the finger and slides from it
//   (17, 56); double and triple taps select a word and a line; finger-sized
//   handles drag a selection's ends to the nearest edge between cells,
//   the last column included (16, 55); the edit menu shows without the
//   keyboard (16);
// - the pointer is an I-beam (48) and its drag selects (49); a trackpad's
//   two-finger scroll scrolls; a pinch sets the font size.

/// What a gesture in progress keeps.
final class CoreTermGestureState {
    enum Axis { case vertical, horizontal }
    var axis: Axis?
    var pinchBase: CGFloat = 9
    weak var tap: UITapGestureRecognizer?
    weak var longPress: UILongPressGestureRecognizer?
    /// A selection is showing (the core has one).
    var selecting = false
    /// A handle is being dragged (the selection keeps its other end).
    var draggingHandle = false
    /// The coast after a lift (B521), and when it last moved.
    var coast: CADisplayLink?
    var coastAt: CFTimeInterval = 0
    /// The touch that stopped a coast: no tap of it (B521).
    weak var coastStopper: UITouch?
}

/// The coast's refresh, without the link keeping the view.
private final class CoreTermCoastTarget: NSObject {
    private weak var view: CoreTerminalView?
    init(_ view: CoreTerminalView) { self.view = view }
    @objc func tick(_ link: CADisplayLink) {
        MainActor.assumeIsolated {
            guard let view else { return link.invalidate() }
            view.coastTick(link)
        }
    }
}

/// A selection handle: a bar the height of a row and a knob, in a
/// finger-sized box (fork patch 16).
final class CoreTermHandle: UIView {
    let isStart: Bool
    private let bar = UIView()
    private let knob = UIView()

    init(start: Bool, color: UIColor) {
        isStart = start
        super.init(frame: .zero)
        backgroundColor = .clear
        bar.backgroundColor = color
        knob.backgroundColor = color
        knob.layer.cornerRadius = 5
        addSubview(bar)
        addSubview(knob)
        accessibilityIdentifier = start ? "core-terminal-handle-start" : "core-terminal-handle-end"
    }

    required init?(coder: NSCoder) { fatalError("CoreTermHandle is made in code") }

    /// Places the handle at the edge `x` of a row from `y` to `y + height`.
    func place(x: CGFloat, y: CGFloat, height: CGFloat) {
        let box: CGFloat = 44
        frame = CGRect(x: x - box / 2, y: (isStart ? y - 10 : y) - (box - height - 10) / 2,
                       width: box, height: box)
        let barY = isStart ? 10 + (box - height - 10) / 2 : (box - height - 10) / 2
        bar.frame = CGRect(x: box / 2 - 1, y: barY, width: 2, height: height)
        knob.frame = CGRect(x: box / 2 - 5, y: isStart ? barY - 10 : barY + height, width: 10, height: 10)
    }
}

extension CoreTerminalView: UIGestureRecognizerDelegate, UIEditMenuInteractionDelegate, UIPointerInteractionDelegate {
    func installGestures() {
        let pan = UIPanGestureRecognizer(target: self, action: #selector(handlePan(_:)))
        pan.maximumNumberOfTouches = 1
        pan.cancelsTouchesInView = false
        pan.allowedScrollTypesMask = .all
        pan.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue)]
        pan.delegate = self
        addGestureRecognizer(pan)

        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(handlePinch(_:)))
        pinch.cancelsTouchesInView = false
        pinch.delegate = self
        addGestureRecognizer(pinch)

        let triple = UITapGestureRecognizer(target: self, action: #selector(handleTripleTap(_:)))
        triple.numberOfTapsRequired = 3
        triple.delegate = self
        addGestureRecognizer(triple)

        let double = UITapGestureRecognizer(target: self, action: #selector(handleDoubleTap(_:)))
        double.numberOfTapsRequired = 2
        double.delegate = self
        addGestureRecognizer(double)

        let tap = UITapGestureRecognizer(target: self, action: #selector(handleTap(_:)))
        tap.cancelsTouchesInView = false
        tap.delegate = self
        tap.require(toFail: double)
        tap.require(toFail: triple)
        addGestureRecognizer(tap)
        gestures.tap = tap

        let long = UILongPressGestureRecognizer(target: self, action: #selector(handleLongPress(_:)))
        long.minimumPressDuration = 0.5
        long.delegate = self
        addGestureRecognizer(long)
        gestures.longPress = long

        // The pointer's drag (a Mac, an iPad's trackpad) selects.
        let pointerDrag = UIPanGestureRecognizer(target: self, action: #selector(handlePointerDrag(_:)))
        pointerDrag.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        pointerDrag.allowedScrollTypesMask = []
        pointerDrag.delegate = self
        addGestureRecognizer(pointerDrag)

        addInteraction(editMenu)
        addInteraction(UIPointerInteraction(delegate: self))
    }

    // MARK: Scrolling and switching

    @objc private func handlePan(_ g: UIPanGestureRecognizer) {
        switch g.state {
        case .began:
            gestures.axis = nil
            surfaceDelegate?.surfaceScrollWillBegin(self)
        case .changed:
            let t = g.translation(in: self)
            if gestures.axis == nil, abs(t.x) >= 12 || abs(t.y) >= 12 {
                gestures.axis = abs(t.x) > abs(t.y) ? .horizontal : .vertical
            }
            guard gestures.axis == .vertical else { return }
            // Every move, wherever it goes: the lift's speed (B521).
            if let term { mp_term_fling_track(term, Double(t.y * metrics.scale)) }
            _ = surfaceDelegate?.surface(self, scrollPixels: t.y)
            g.setTranslation(.zero, in: self)
        case .ended:
            switch gestures.axis {
            case .horizontal:
                let dx = g.translation(in: self).x
                if abs(dx) >= 40, surfaceDelegate?.surfaceCanSwitch == true {
                    Haptics.select()
                    surfaceDelegate?.surface(self, switchForward: dx < 0)
                }
            case .vertical:
                // The lift coasts at the drag's speed; the scroll ends when
                // the coast does (B521).
                let elsewhere = surfaceDelegate?.surfaceScrollsElsewhere == true
                if let term, mp_term_fling_start(term, elsewhere) {
                    startCoast()
                } else {
                    surfaceDelegate?.surfaceScrollDidEnd(self)
                }
            case nil:
                break
            }
            gestures.axis = nil
        default:
            if gestures.axis == .vertical { surfaceDelegate?.surfaceScrollDidEnd(self) }
            gestures.axis = nil
        }
    }

    // MARK: The coast (B521)

    private func startCoast() {
        gestures.coast?.invalidate()
        let link = CADisplayLink(target: CoreTermCoastTarget(self), selector: #selector(CoreTermCoastTarget.tick(_:)))
        link.add(to: .main, forMode: .common)
        gestures.coast = link
        gestures.coastAt = CACurrentMediaTime()
    }

    /// One refresh of the coast: the core's travel for it, routed as a
    /// drag's (the history, a wheel, the host's rows). Typing, a resize or
    /// the live screen stop it in the core.
    fileprivate func coastTick(_ link: CADisplayLink) {
        guard let term else { return endCoast() }
        let now = link.timestamp
        let dt = max(0, now - gestures.coastAt)
        gestures.coastAt = now
        var dy: Double = 0
        let going = mp_term_fling_step(term, dt, &dy)
        if dy != 0 {
            _ = surfaceDelegate?.surface(self, scrollPixels: CGFloat(dy) / metrics.scale)
        }
        if !going { endCoast() }
    }

    /// The coast is over: the scroll ends (the dock near the live screen,
    /// the host's end of a scroll).
    private func endCoast() {
        guard let link = gestures.coast else { return }
        link.invalidate()
        gestures.coast = nil
        surfaceDelegate?.surfaceScrollDidEnd(self)
    }

    /// A touch on a coast stops it; that touch is no tap (B521).
    private func touchStopsCoast(_ touch: UITouch) {
        guard gestures.coast != nil else { return }
        if let term { _ = mp_term_fling_stop(term) }
        gestures.coastStopper = touch
        endCoast()
    }

    @objc private func handlePinch(_ g: UIPinchGestureRecognizer) {
        switch g.state {
        case .began:
            gestures.pinchBase = font.pointSize
        case .changed:
            let target = min(max(gestures.pinchBase * g.scale, 8), 32)
            if abs(target - font.pointSize) >= 0.5 { apply(font: font.withSize(target)) }
        case .ended, .cancelled, .failed:
            surfaceDelegate?.surface(self, fontSizeCommitted: Double(font.pointSize))
        default:
            break
        }
    }

    // MARK: Taps

    /// What a tap means is the core's call (V12–V14): dismiss a selection,
    /// open a link without taking the keyboard, or click — and take the
    /// keyboard on the cursor's own rows.
    @objc private func handleTap(_ g: UITapGestureRecognizer) {
        guard g.state == .ended, let term else { return }
        let cell = self.cell(at: g.location(in: self))
        let tap = mp_term_tap(term, UInt16(clamping: cell.row), UInt16(clamping: cell.col))
        switch tap.kind {
        case 1:
            clearSelection()
        case 2:
            if let p = tap.link {
                let link = String(cString: p)
                mp_string_free(p)
                Log.input.info("core term: link tap")
                surfaceDelegate?.surface(self, openLink: link)
            }
        default:
            if tap.keyboard, !isFirstResponder { _ = becomeFirstResponder() }
            surfaceDelegate?.surface(self, click: cell.col, row: cell.row)
        }
    }

    @objc private func handleDoubleTap(_ g: UITapGestureRecognizer) {
        guard g.state == .ended else { return }
        let cell = self.cell(at: g.location(in: self))
        select(kind: 1, row: cell.row, col: cell.col)
        showMenu()
    }

    @objc private func handleTripleTap(_ g: UITapGestureRecognizer) {
        guard g.state == .ended else { return }
        let cell = self.cell(at: g.location(in: self))
        select(kind: 2, row: cell.row, col: cell.col)
        showMenu()
    }

    @objc private func handleLongPress(_ g: UILongPressGestureRecognizer) {
        guard let term else { return }
        let cell = self.cell(at: g.location(in: self))
        switch g.state {
        case .began:
            // The link or the word under this press, whatever was selected
            // before (V15, patch 56).
            Haptics.select()
            select(kind: 4, row: cell.row, col: cell.col)
        case .changed:
            mp_term_select_extend(term, UInt16(clamping: cell.row), UInt16(clamping: cell.col), false)
            takeFrame()
        case .ended:
            showMenu()
        default:
            break
        }
    }

    @objc private func handlePointerDrag(_ g: UIPanGestureRecognizer) {
        guard let term else { return }
        let point = g.location(in: self)
        switch g.state {
        case .began:
            let start = g.location(in: self) - g.translation(in: self)
            let e = edge(at: start)
            mp_term_select_start(term, 0, UInt16(clamping: e.row), UInt16(clamping: e.col), e.after)
            gestures.selecting = true
            fallthrough
        case .changed:
            let e = edge(at: point)
            mp_term_select_extend(term, UInt16(clamping: e.row), UInt16(clamping: e.col), e.after)
            takeFrame()
        case .ended:
            if !hasSelection { clearSelection() }
        default:
            break
        }
    }

    // MARK: Links

    /// The link under a viewport cell: an OSC 8 link the program declared,
    /// else a URL the core detected (V34), else a path the host confirmed
    /// (V43).
    func link(at cell: (col: Int, row: Int)) -> String? {
        guard let term, let p = mp_term_link_at(term, UInt16(clamping: cell.row), UInt16(clamping: cell.col))
        else { return nil }
        defer { mp_string_free(p) }
        return String(cString: p)
    }

    // MARK: The selection

    var hasSelection: Bool {
        guard let term else { return false }
        var b = MpSelectionBounds()
        return mp_term_selection_bounds(term, &b)
    }

    /// `kind`: 0 cells, 1 word, 2 line, 4 semantic (the link, else the
    /// word).
    func select(kind: UInt8, row: Int, col: Int) {
        guard let term else { return }
        mp_term_select_start(term, kind, UInt16(clamping: row), UInt16(clamping: col), false)
        gestures.selecting = true
        takeFrame()
        positionHandles()
    }

    func clearSelection() {
        guard let term else { return }
        mp_term_select_clear(term)
        gestures.selecting = false
        editMenu.dismissMenu()
        handles?.start.removeFromSuperview()
        handles?.end.removeFromSuperview()
        handles = nil
        takeFrame()
    }

    var selectedText: String? {
        guard let term, let p = mp_term_selection_text(term) else { return nil }
        defer { mp_string_free(p) }
        return String(cString: p)
    }

    /// The handles at the selection's ends; gone with the selection. No
    /// handles on a Mac: the pointer selects (patch 49).
    func positionHandles() {
        guard let term else { return }
        var b = MpSelectionBounds()
        guard mp_term_selection_bounds(term, &b) else {
            if gestures.selecting { gestures.selecting = false }
            handles?.start.removeFromSuperview()
            handles?.end.removeFromSuperview()
            handles = nil
            return
        }
        gestures.selecting = true
        #if targetEnvironment(macCatalyst)
        return
        #else
        if handles == nil {
            let color = UIColor(cgColor: coreCGColor(style.cursorColor))
            let start = CoreTermHandle(start: true, color: color)
            let end = CoreTermHandle(start: false, color: color)
            for h in [start, end] {
                let pan = UIPanGestureRecognizer(target: self, action: #selector(handleHandleDrag(_:)))
                pan.maximumNumberOfTouches = 1
                h.addGestureRecognizer(pan)
                addSubview(h)
            }
            handles = (start, end)
        }
        let endCol = min(Int(b.end_col), max(0, rows.cols - 1))
        handles?.start.place(x: x(ofColumn: Int(b.start_col)), y: y(ofRow: Int(b.start_row)),
                             height: metrics.height)
        handles?.end.place(x: x(ofColumn: Int(endCol) + 1), y: y(ofRow: Int(b.end_row)),
                           height: metrics.height)
        #endif
    }

    @objc private func handleHandleDrag(_ g: UIPanGestureRecognizer) {
        guard let term, let handle = g.view as? CoreTermHandle else { return }
        switch g.state {
        case .began:
            // The other end stays where it is, on screen or not (B477: it
            // was re-anchored through the viewport, and the drag refused
            // when it lay outside it), cell by cell from here.
            guard mp_term_select_keep_end(term, handle.isStart) else { return }
            editMenu.dismissMenu()
            gestures.draggingHandle = true
            fallthrough
        case .changed:
            guard gestures.draggingHandle else { return }
            // The handle's edge, a row's height above the finger for the
            // start handle (the finger hides the text under it).
            var point = g.location(in: self)
            if handle.isStart { point.y -= metrics.height / 2 }
            let e = edge(at: point)
            mp_term_select_extend(term, UInt16(clamping: e.row), UInt16(clamping: e.col), e.after)
            takeFrame()
            positionHandles()
        case .ended, .cancelled:
            gestures.draggingHandle = false
            showMenu()
        default:
            break
        }
    }

    // MARK: The edit menu

    func showMenu() {
        guard let term else { return }
        var b = MpSelectionBounds()
        let rect: CGRect
        if mp_term_selection_bounds(term, &b) {
            let top = y(ofRow: Int(b.start_row)), bottom = y(ofRow: Int(b.end_row)) + metrics.height
            rect = CGRect(x: 0, y: max(0, top), width: bounds.width, height: max(metrics.height, bottom - top))
        } else {
            rect = cursorCellRect()
        }
        let config = UIEditMenuConfiguration(identifier: nil, sourcePoint: CGPoint(x: rect.midX, y: rect.minY))
        editMenu.presentEditMenu(with: config)
    }

    func editMenuInteraction(_ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration,
                             suggestedActions: [UIMenuElement]) -> UIMenu? {
        withFind(suggestedActions)
    }

    func editMenuInteraction(_ interaction: UIEditMenuInteraction,
                             targetRectFor configuration: UIEditMenuConfiguration) -> CGRect {
        guard let term else { return cursorCellRect() }
        var b = MpSelectionBounds()
        guard mp_term_selection_bounds(term, &b) else { return cursorCellRect() }
        let top = y(ofRow: Int(b.start_row)), bottom = y(ofRow: Int(b.end_row)) + metrics.height
        return CGRect(x: 0, y: top, width: bounds.width, height: bottom - top).intersection(bounds)
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        switch action {
        case #selector(copy(_:)): return hasSelection
        case #selector(paste(_:)): return UIPasteboard.general.hasStrings
        case #selector(find(_:)), #selector(findNext(_:)), #selector(findPrevious(_:)): return finder != nil
        case #selector(useSelectionForFind(_:)): return finder != nil && hasSelection
        default: return false
        }
    }

    override func copy(_ sender: Any?) {
        guard let text = selectedText else { return }
        UIPasteboard.general.string = text
        clearSelection()
    }

    override func paste(_ sender: Any?) {
        // The pasteboard can take a while: off the main thread.
        Task { [weak self] in
            let text = await Task.detached { UIPasteboard.general.string }.value
            guard let self, let text, !text.isEmpty else { return }
            self.scrollToBottom()
            self.surfaceDelegate?.surface(self, paste: text)
        }
    }

    // MARK: The pointer

    func pointerInteraction(_ interaction: UIPointerInteraction, regionFor request: UIPointerRegionRequest,
                            defaultRegion: UIPointerRegion) -> UIPointerRegion? {
        UIPointerRegion(rect: bounds)
    }

    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        UIPointerStyle(shape: .verticalBeam(length: metrics.height), constrainedAxes: .vertical)
    }

    // MARK: UIGestureRecognizerDelegate

    override func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        if let pan = g as? UIPanGestureRecognizer, pan.maximumNumberOfTouches == 1, pan.view === self,
           pan.allowedTouchTypes.contains(NSNumber(value: UITouch.TouchType.direct.rawValue)) {
            // A live selection: its handles own the drag.
            if gestures.selecting { return false }
            let canScroll = surfaceDelegate?.surfaceScrollsElsewhere == true || termModes.mouse != 0 || historyLines > 0
            return canScroll || surfaceDelegate?.surfaceCanSwitch == true
        }
        return true
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        // A touch on a coast stops it; taps don't see that touch (B521). The
        // pan and the long press do: a new drag, a selection.
        touchStopsCoast(touch)
        if g is UITapGestureRecognizer, gestures.coastStopper === touch { return false }
        return true
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        // A held-then-lifted finger is a tap too: it must not dismiss what
        // the long press just selected.
        if g is UITapGestureRecognizer, other is UILongPressGestureRecognizer { return false }
        if g is UILongPressGestureRecognizer, other is UITapGestureRecognizer { return false }
        return true
    }
}

private func - (a: CGPoint, b: CGPoint) -> CGPoint { CGPoint(x: a.x - b.x, y: a.y - b.y) }
