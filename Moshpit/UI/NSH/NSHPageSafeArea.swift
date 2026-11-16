import SwiftUI

/// Which safe areas a page NSH draws lays itself out in (B843).
///
/// A page NSH draws takes the whole screen and keeps its pieces out of the
/// status bar, the home indicator and the keyboard itself (``NSHPage``'s
/// rule 1): it ignores every safe area. Except beside the iPad's sidebar:
/// iOS 26's split view floats the sidebar over the detail column and says
/// so as a leading safe-area inset, and a page ignoring it was laid out the
/// window's full width, its leading third under the sidebar. The terminal
/// there showed the end of its crumb and a black pane (its text sat under
/// the sidebar), in every locale's iPad shot. In the split view's detail
/// column (``SwiftUI/EnvironmentValues/nshKeepsSideInsets``) a page keeps
/// out of the sides' safe area and still ignores the top's and the
/// bottom's.
struct NSHPageSafeArea: ViewModifier {
    @Environment(\.nshKeepsSideInsets) private var keepsSides

    func body(content: Content) -> some View {
        content.ignoresSafeArea(.all, edges: Self.ignored(keepsSides: keepsSides))
    }

    /// The edges whose safe area a page ignores.
    static func ignored(keepsSides: Bool) -> Edge.Set {
        keepsSides ? .vertical : .all
    }
}

private struct NSHKeepsSideInsetsKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Pages NSH draws here keep out of the leading and trailing safe area
    /// (``NSHPageSafeArea``): the iPad split view's detail column.
    var nshKeepsSideInsets: Bool {
        get { self[NSHKeepsSideInsetsKey.self] }
        set { self[NSHKeepsSideInsetsKey.self] = newValue }
    }
}
