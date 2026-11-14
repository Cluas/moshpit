import QuartzCore
import UIKit

/// Starts what an NSH surface moves with the software keyboard when the
/// keyboard itself starts to move.
///
/// NSH animates a layout the keyboard changes (`mb_keyboard`) on the
/// keyboard's own curve and duration, and starts it — however late its
/// batch arrives — at the moment UIKit posted the keyboard's notification
/// (`KeyboardMotion::aligned`, its animations' `beginTime` back-dated to
/// then). The keyboard's own animation begins when the transaction that
/// adds it commits, after the work the notification set off (the first
/// responder, the input views, the terminal's layout): on the iPad mini
/// some 30 ms later going up, so NSH's shortcut bar ran that far ahead of
/// the keyboard, up to 100 pt off its curve mid-way (W11g's measure).
///
/// Armed by the keyboard's notification, this watches the surface for a
/// moment with a run-loop observer ordered before Core Animation's commit,
/// and gives each animation NSH back-dated to the notification the
/// keyboard's own begin time instead: the one its host view's animation
/// has once committed, or the commit they share when they go together.
/// NSH upstream should do this itself (align `LayoutCurve::Keyboard` to the
/// keyboard's animation, not to the notification).
@MainActor
final class NSHKeyboardAlign {
    private weak var surface: UIView?
    private var observer: CFRunLoopObserver?
    private var tokens: [NSObjectProtocol] = []
    /// When the keyboard's notification came (NSH's back-dated start).
    private var posted: CFTimeInterval = 0
    private var deadline: CFTimeInterval = 0
    /// What was moved already (layer, key).
    private var moved: Set<String> = []
    /// How many animations were re-timed, and by how much, for the director.
    private(set) var shifts: [Double] = []

    /// How long after the notification NSH's batch may come.
    private static let window: CFTimeInterval = 0.6

    init(surface: UIView) {
        self.surface = surface
        let center = NotificationCenter.default
        for name in [UIResponder.keyboardWillChangeFrameNotification, UIResponder.keyboardWillHideNotification] {
            tokens.append(center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
                // Posted on the main thread, as NSH's own observer hears it.
                MainActor.assumeIsolated { self?.arm() }
            })
        }
    }

    deinit {
        for token in tokens { NotificationCenter.default.removeObserver(token) }
        if let observer { CFRunLoopObserverInvalidate(observer) }
    }

    private func arm() {
        posted = CACurrentMediaTime()
        deadline = posted + Self.window
        moved = []
        guard observer == nil else { return }
        // After UIKit's layout, before Core Animation's commit (2000000).
        let observer = CFRunLoopObserverCreateWithHandler(
            nil, CFRunLoopActivity.beforeWaiting.rawValue, true, 1_999_000
        ) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.check() }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        self.observer = observer
    }

    private func disarm() {
        if let observer { CFRunLoopObserverInvalidate(observer) }
        observer = nil
    }

    private func check() {
        guard let surface, CACurrentMediaTime() < deadline else { return disarm() }
        let keyboard = Self.keyboardBegin()
        Self.walk(surface.layer) { layer in
            for key in layer.animationKeys() ?? [] {
                let id = "\(ObjectIdentifier(layer).hashValue).\(key)"
                guard !moved.contains(id), let animation = layer.animation(forKey: key) else { continue }
                // NSH's: begun at the notification (in this layer's time).
                let notified = layer.convertTime(posted, from: nil)
                guard animation.beginTime > 0, abs(animation.beginTime - notified) < 0.004 else { continue }
                moved.insert(id)
                let copy = animation.copy() as! CAAnimation
                switch keyboard {
                case .committed(let begin):
                    let local = layer.convertTime(begin, from: nil)
                    guard local > animation.beginTime else { continue }
                    copy.beginTime = local
                    shifts.append((begin - posted) * 1000)
                case .pending:
                    // Committed together with the keyboard's: its begin.
                    copy.beginTime = 0
                    shifts.append((CACurrentMediaTime() - posted) * 1000)
                case .none:
                    continue
                }
                copy.fillMode = .backwards
                layer.add(copy, forKey: key)
            }
        }
    }

    private enum KeyboardBegin {
        case committed(CFTimeInterval)
        case pending
    }

    /// When the keyboard's animation begins: committed (in global time),
    /// or going with this transaction.
    private static func keyboardBegin() -> KeyboardBegin? {
        guard let host = keyboardHost() else { return nil }
        for key in host.layer.animationKeys() ?? [] {
            guard let animation = host.layer.animation(forKey: key) else { continue }
            if animation.beginTime == 0 { return .pending }
            return .committed(host.layer.convertTime(animation.beginTime, to: nil))
        }
        return nil
    }

    /// The keyboard's host view in its window: its frame moves with it.
    static func keyboardHost() -> UIView? {
        for scene in UIApplication.shared.connectedScenes {
            guard let scene = scene as? UIWindowScene else { continue }
            for window in scene.windows {
                let name = String(describing: type(of: window))
                guard name.contains("Keyboard") || name.contains("TextEffects") else { continue }
                if let host = find("InputSetHostView", in: window), host.bounds.height > 50 { return host }
            }
        }
        return nil
    }

    private static func find(_ name: String, in view: UIView) -> UIView? {
        if String(describing: type(of: view)).contains(name) { return view }
        for sub in view.subviews {
            if let found = find(name, in: sub) { return found }
        }
        return nil
    }

    private static func walk(_ layer: CALayer, _ visit: (CALayer) -> Void) {
        visit(layer)
        for sub in layer.sublayers ?? [] { walk(sub, visit) }
    }
}
