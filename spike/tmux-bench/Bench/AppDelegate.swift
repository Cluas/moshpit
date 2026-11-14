// The tmux parser bench's host: one window showing the report the bench
// also prints to stdout (scripts/device.sh reads it through devicectl's
// console). The bench starts at launch.

import UIKit

final class ReportViewController: UIViewController {
    private let text = UITextView()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        text.backgroundColor = .black
        text.textColor = .white
        text.isEditable = false
        text.font = .monospacedSystemFont(ofSize: 10, weight: .regular)
        text.frame = view.bounds
        text.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(text)
        text.text = "running…"
        Task.detached(priority: .userInitiated) {
            let report = await TmuxBench.run()
            await MainActor.run { self.text.text = report }
            if ProcessInfo.processInfo.arguments.contains("-MOSHPIT_BENCH_EXIT") {
                exit(0)
            }
        }
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: windowScene)
        window.rootViewController = ReportViewController()
        window.makeKeyAndVisible()
        self.window = window
    }
}

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        configurationForConnecting connectingSceneSession: UISceneSession,
        options: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: "Default", sessionRole: connectingSceneSession.role)
        configuration.delegateClass = SceneDelegate.self
        return configuration
    }
}
