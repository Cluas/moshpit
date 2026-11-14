import MoshpitFFI
import MoshpitKit
import UIKit

/// A native view an NSH screen places with `native("name")`: it fills the
/// box the screen lays out; props arrive as JSON values (`String`,
/// `NSNumber`, `[Any]`, `[String: Any]`, `NSNull`).
@MainActor
protocol NSHComponent: AnyObject {
    var view: UIView { get }
    func setProp(_ name: String, _ value: Any?)
    func command(_ name: String, _ args: Any?)
    /// The element went away; the component is released right after.
    func destroy()
}

extension NSHComponent {
    func command(_ name: String, _ args: Any?) {}
    func destroy() {}
}

/// What a component tells its screen: events and its own size. Thread-safe.
final class NSHComponentContext: @unchecked Sendable {
    private let handle: OpaquePointer

    fileprivate init(_ handle: OpaquePointer) {
        self.handle = handle
    }

    deinit {
        mp_ui_component_context_free(handle)
    }

    func emit(_ event: String, _ data: Any? = nil) {
        guard let data else { return mp_ui_component_emit(handle, event, nil) }
        guard JSONSerialization.isValidJSONObject([data]),
              let json = try? JSONSerialization.data(withJSONObject: data, options: .fragmentsAllowed)
        else {
            return Log.ui.error("NSH component event \(event, privacy: .public): data is not JSON")
        }
        String(decoding: json, as: UTF8.self).withCString { mp_ui_component_emit(handle, event, $0) }
    }

    func setSize(_ size: CGSize) {
        mp_ui_component_set_size(handle, Double(size.width), Double(size.height))
    }
}

/// An NSH screen as a `UIView` (`ui/` in the Rust workspace, through the C ABI
/// of `ui/include/moshpit_ui.h` in the app's one Rust library), modelled on
/// spikes G1 and G5's host view (`spike/ios/Spike/SpikeAppView.swift`).
///
/// One surface runs one screen (``run(screen:params:)``) and has its own
/// channel with it: ``post(_:)`` hands the screen the data it shows, in order;
/// ``onMessage`` hears what the screen tells the host (`{"ev": …}`), on the
/// main thread. The host's own native components (``NSHHostComponents``) are
/// registered on every surface. Dropping the surface stops the screen.
@MainActor
final class NSHSurface: UIView {
    private let surface: Handle
    /// What the engine thread's handler reaches: the surface, weakly.
    private let inbox: Inbox

    /// Hears the screen, on the main thread: everything but the events of
    /// its error boundary (`failed`, `retry`), which the surface takes
    /// (``receive(_:)``).
    var onMessage: (([String: Any]) -> Void)?

    /// The screen's Retry, under a body that failed: run it again (B366).
    var onRetry: (() -> Void)?

    /// The screen that runs, as ``run(screen:params:)`` named it: what the
    /// surface's log lines say.
    private(set) var screenName: String?

    /// The ops of posts already logged as refused, once each (B366).
    private var refusedPosts = NSHPostLog()

    /// Called once, the first time the surface is in a window: where the
    /// host can measure what the screen stands in for
    /// (``navigationBarHeight``).
    var onWindow: ((NSHSurface) -> Void)?

    /// Whether this build's Rust library can host NSH screens: it is linked
    /// and is the one the app was compiled against (a stale
    /// libmoshpit_ffi.a would be misread, not just fail).
    nonisolated static let available = mp_ui_abi_version() == UInt32(MP_UI_ABI_VERSION)

    /// A surface, empty until a screen runs; nil when this build cannot host
    /// NSH or NSH failed to start (the reason is in the log).
    static func make(frame: CGRect = .zero) -> NSHSurface? {
        if available {
            NSHDiagnostics.forwardRustLog()
            NSHDiagnostics.prepareTrail()
        }
        guard available, let surface = mp_ui_surface_create(frame) else {
            NSHDiagnostics.error("NSH surface not made (library available: \(available))")
            return nil
        }
        return NSHSurface(surface: surface, frame: frame)
    }

    private init(surface: OpaquePointer, frame: CGRect) {
        self.surface = Handle(pointer: surface)
        inbox = Inbox()
        super.init(frame: frame)
        inbox.surface = self
        if let inner = mp_ui_surface_view(surface) {
            inner.frame = bounds
            inner.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            addSubview(inner)
        }
        let context = Unmanaged.passRetained(inbox).toOpaque()
        mp_ui_surface_set_handler(surface, { context, json in
            // NSH's engine thread: copy the message, hop to the main thread.
            let inbox = Unmanaged<Inbox>.fromOpaque(context!).takeUnretainedValue()
            let data = Data(bytes: json, count: strlen(json))
            guard let message = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
            DispatchQueue.main.async {
                MainActor.assumeIsolated { inbox.surface?.receive(message) }
            }
        }, context)
        NSHHostComponents.register(on: self)
    }

    required init?(coder: NSCoder) {
        fatalError("NSHSurface is made in code")
    }

    deinit {
        // On the main thread, where a view goes away: no call reaches the
        // handler once it is cleared, then its context can go, then the
        // screen stops.
        let handle = surface, inbox = inbox
        let teardown: @Sendable () -> Void = {
            mp_ui_surface_set_handler(handle.pointer, nil, nil)
            Unmanaged.passUnretained(inbox).release()
            mp_ui_surface_destroy(handle.pointer)
        }
        if Thread.isMainThread { teardown() } else { DispatchQueue.main.async(execute: teardown) }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil, let onWindow else { return }
        self.onWindow = nil
        onWindow(self)
    }

    /// The height of the navigation bar of the stack the surface is a page
    /// of (hidden for an NSH page, which draws its own: it measures the same
    /// as shown): 56 pt in a sheet, 50 pt on an iPad screen, 44 on an
    /// iPhone's.
    var navigationBarHeight: CGFloat? {
        var responder: UIResponder? = self
        while let next = responder?.next {
            if let controller = next as? UIViewController, let navigation = controller.navigationController {
                // A bar hidden from the start (an NSH page at the root of a
                // sheet's stack) was never laid out where it shows, and
                // reports the screen's 50 or 44 pt: a sheet's bar is 56.
                if navigation.isNavigationBarHidden, Self.inSheet(navigation) {
                    return Self.sheetBarHeight
                }
                let height = navigation.navigationBar.frame.height
                return height > 0 ? height : nil
            }
            responder = next
        }
        return nil
    }

    /// The height of a navigation bar in a sheet (iPad and iPhone alike).
    static let sheetBarHeight: CGFloat = 56

    /// Whether `controller` is in a sheet: the controller its stack hangs
    /// from is presented as one.
    private static func inSheet(_ controller: UIViewController) -> Bool {
        var top = controller
        while let parent = top.parent { top = parent }
        return top.presentingViewController != nil && top.sheetPresentationController != nil
    }

    /// The title of the page under the surface's in its navigation stack,
    /// which a pushed page's back button shows (SwiftUI's `navigationTitle`
    /// of that page); nil at the stack's root or outside one.
    var backTitle: String? {
        var responder: UIResponder? = self
        while let next = responder?.next {
            if let controller = next as? UIViewController, let navigation = controller.navigationController {
                let stack = navigation.viewControllers
                // The page is the stack's controller that holds this one.
                guard let index = stack.firstIndex(where: { controller.isDescendant(of: $0) }), index > 0
                else { return nil }
                let previous = stack[index - 1].navigationItem
                return previous.backButtonTitle ?? previous.title
            }
            responder = next
        }
        return nil
    }

    /// Runs screen `screen` (`"recent-log"`) with `params` (the app's
    /// language and theme: ``NSHScreenParams``); register components first.
    /// false for an unknown screen or params it cannot read. Run again, it
    /// replaces what ran (Retry).
    @discardableResult
    func run(screen: String, params: [String: Any]) -> Bool {
        screenName = screen
        refusedPosts = NSHPostLog()
        guard let json = Self.json(params) else {
            NSHDiagnostics.error("NSH \(screen): params are not JSON (\(NSHPostLog.unwritable(params).joined(separator: ", ")))")
            return false
        }
        return json.withCString { mp_ui_surface_run(surface.pointer, screen, $0) }
    }

    /// Posts `message` (`{"op": …}`) to the running screen, after what was
    /// posted before. false when it is not one of the screen's messages, or
    /// not JSON: logged once per op (B366), the core's line before it saying
    /// why the screen refused it.
    @discardableResult
    func post(_ message: [String: Any]) -> Bool {
        guard let json = Self.json(message) else {
            let op = NSHPostLog.op(of: message)
            if refusedPosts.first(op) {
                let keys = NSHPostLog.unwritable(message).joined(separator: ", ")
                NSHDiagnostics.error("NSH \(screenName ?? "-"): post \(op) is not JSON (\(keys))")
            }
            return false
        }
        return post(json: json)
    }

    /// Posts a message the core wrote as JSON already (`{"op": …}`).
    @discardableResult
    func post(json: String) -> Bool {
        let posted = json.withCString { mp_ui_surface_post(surface.pointer, $0) }
        if !posted {
            let op = NSHPostLog.op(ofJSON: json)
            if refusedPosts.first(op) {
                NSHDiagnostics.error("NSH \(screenName ?? "-"): post \(op) refused")
            }
        }
        return posted
    }

    /// What became of the posts to the running screen (``NSHWatch``); nil
    /// when the library can't say.
    var counts: NSHWatch.Counts? {
        var counts = MpUiSurfaceCounts()
        guard mp_ui_surface_counts(surface.pointer, &counts) else { return nil }
        return NSHWatch.Counts(heard: counts.heard, posted: counts.posted, applied: counts.applied,
                               lost: counts.lost)
    }

    /// What the screen told: its error boundary's events are the surface's
    /// (logged; Retry runs it again), the rest the page's.
    func receive(_ message: [String: Any]) {
        switch NSHBoundaryEvent(message) {
        case .failed(let component, let why)?:
            NSHDiagnostics.error("NSH \(screenName ?? "-"): \(component) failed: \(why)")
        case .retry?:
            Log.ui.notice("NSH \(self.screenName ?? "-", privacy: .public): Retry")
            onRetry?()
        case nil:
            onMessage?(message)
        }
    }

    /// Registers native component `name`: `make` makes one for every element
    /// created from now on.
    func register(component name: String, _ make: @escaping @MainActor (NSHComponentContext) -> NSHComponent) {
        let factory = Unmanaged.passRetained(FactoryBox(make)).toOpaque()
        var callbacks = MpUiComponentCallbacks(
            create: { factory, context in
                let box = Unmanaged<FactoryBox>.fromOpaque(factory!).takeUnretainedValue()
                let component = MainActor.assumeIsolated { box.make(NSHComponentContext(context)) }
                return Unmanaged.passRetained(ComponentBox(component)).toOpaque()
            },
            view: { component in
                let box = Unmanaged<ComponentBox>.fromOpaque(component).takeUnretainedValue()
                return MainActor.assumeIsolated { box.component.view }
            },
            set_prop: { component, name, json in
                let box = Unmanaged<ComponentBox>.fromOpaque(component).takeUnretainedValue()
                let name = String(cString: name)
                let value = jsonObject(json)
                MainActor.assumeIsolated { box.component.setProp(name, value) }
            },
            command: { component, name, json in
                let box = Unmanaged<ComponentBox>.fromOpaque(component).takeUnretainedValue()
                let name = String(cString: name)
                let args = jsonObject(json)
                MainActor.assumeIsolated { box.component.command(name, args) }
            },
            destroy: { component in
                let box = Unmanaged<ComponentBox>.fromOpaque(component)
                MainActor.assumeIsolated { box.takeUnretainedValue().component.destroy() }
                box.release()
            },
            release_factory: { factory in
                Unmanaged<FactoryBox>.fromOpaque(factory!).release()
            }
        )
        if !mp_ui_surface_register_component(surface.pointer, name, factory, &callbacks) {
            Unmanaged<FactoryBox>.fromOpaque(factory).release()
            Log.ui.error("NSH component \(name, privacy: .public) not registered")
        }
    }

    /// Whether `message` is one of screen `screen`'s, as ``post(_:)`` checks
    /// it, with no surface.
    nonisolated static func check(screen: String, message: [String: Any]) -> Bool {
        guard let json = json(message) else { return false }
        return check(screen: screen, json: json)
    }

    /// ``check(screen:message:)`` of a message the core wrote as JSON.
    nonisolated static func check(screen: String, json: String) -> Bool {
        json.withCString { mp_ui_check_message(screen, $0) }
    }

    private nonisolated static func json(_ object: [String: Any]) -> String? {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    /// The Rust surface (only ever used on the main thread).
    private struct Handle: @unchecked Sendable {
        let pointer: OpaquePointer
    }

    /// What the engine thread's handler holds: the surface, weakly, read on
    /// the main thread.
    private final class Inbox: @unchecked Sendable {
        @MainActor weak var surface: NSHSurface?
    }

    private final class FactoryBox {
        let make: @MainActor (NSHComponentContext) -> NSHComponent
        init(_ make: @escaping @MainActor (NSHComponentContext) -> NSHComponent) { self.make = make }
    }

    private final class ComponentBox {
        let component: NSHComponent
        init(_ component: NSHComponent) { self.component = component }
    }
}

private func jsonObject(_ json: UnsafePointer<CChar>?) -> Any? {
    guard let json else { return nil }
    let data = Data(bytes: json, count: strlen(json))
    return try? JSONSerialization.jsonObject(with: data, options: .fragmentsAllowed)
}

private extension UIViewController {
    /// Whether this controller is `ancestor` or one of its children's.
    func isDescendant(of ancestor: UIViewController) -> Bool {
        var controller: UIViewController? = self
        while let current = controller {
            if current === ancestor { return true }
            controller = current.parent
        }
        return false
    }
}
