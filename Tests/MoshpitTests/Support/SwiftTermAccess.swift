import SwiftTerm
import UIKit
@testable import Moshpit

// The SwiftTerm path's tests reach SwiftTerm's view behind the app's seam
// (TerminalSurface) the way the adapter does (UI/Terminal/SwiftTerm/); app
// code never does.

extension SwiftTerminalView.Coordinator {
    /// SwiftTerm's view of the coordinator's terminal, when SwiftTerm draws it.
    var terminalView: TerminalView? { swiftTermView }

    /// Makes `view` this coordinator's terminal, wrapped as a surface (its
    /// delegate and gestures), as a controller's mint does.
    @MainActor
    func attach(to view: TerminalView) {
        attach(SwiftTermSurface.of(view))
    }

    /// The SwiftTerm terminal's surface.
    @MainActor
    var swiftTermSurface: SwiftTermSurface? { surface as? SwiftTermSurface }
}

extension TmuxSessionController {
    /// A pane's terminal as SwiftTerm's view (the Swift controller's panes
    /// always are; the Rust one's when the core switch is off).
    func terminalView(for paneId: String) -> TerminalView {
        guard let surface = surface(for: paneId) as? SwiftTermSurface else {
            fatalError("pane \(paneId) is not drawn by SwiftTerm")
        }
        return surface.terminalView
    }
}

extension TerminalHostContainer {
    /// The hosted terminal as SwiftTerm's view.
    var terminalView: TerminalView? { (surface as? SwiftTermSurface)?.terminalView }

    /// Hosts SwiftTerm's `view` (wrapped as a surface when it is not one
    /// yet).
    func host(_ view: TerminalView) {
        host(SwiftTermSurface.of(view))
    }
}

private var keptKey: UInt8 = 0

extension SwiftTermSurface {
    /// The surface `view` already is (its delegate), or a new one kept with
    /// the view, so a container and a coordinator a test gives the same
    /// view share one surface.
    @MainActor
    static func of(_ view: TerminalView) -> SwiftTermSurface {
        if let surface = view.terminalDelegate as? SwiftTermSurface { return surface }
        let surface = SwiftTermSurface(wrapping: view)
        objc_setAssociatedObject(view, &keptKey, surface, .OBJC_ASSOCIATION_RETAIN)
        return surface
    }
}
