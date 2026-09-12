import AppIntents
import MoshpitKit

/// Tapping the "switch agent" affordance — cycles which agent the Dynamic Island
/// shows as the headline, so several concurrent agents are all reachable. Runs
/// in the app process (nil cycler in the widget process).
///
/// Compiled into BOTH the app and the MoshpitIsland target by path (see
/// project.yml), the way Xcode's own widget template does it: an App Intent is
/// an entry point of the bundle that declares it — the widget's Button needs
/// the type, the app is where `perform()` runs. It is the one piece of shared
/// code deliberately kept out of MoshpitKit: App Intents metadata is extracted
/// per target, and a package would need the AppIntentsPackage plumbing to get
/// the same result.
struct AgentCycleIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Switch agent"
    static var isDiscoverable: Bool = false

    func perform() async throws -> some IntentResult {
        await AgentControlBridge.shared.cycleHeadline()
        return .result()
    }
}
