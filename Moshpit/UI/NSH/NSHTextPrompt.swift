import MoshpitKit
import UIKit

/// The question with a line of text for its answer that an NSH screen asks
/// through its host (`ui/src/terminal/prompt.rs`): NSH's alerts have no
/// text field, so the screen says what to ask (`{"ev": "prompt", "id",
/// "title", "message", "placeholder", "text", "confirm", "cancel"}`, every
/// word already in the app's language) and the host asks it with UIKit's
/// alert, as SwiftUI's `.alert` with a `TextField` does, then answers the
/// screen with `{"op": "prompted", "id", "text": … | null}` (null: Cancel).
@MainActor
enum NSHTextPrompt {
    /// What `message` asks, or nil when it is not a prompt.
    struct Question: Equatable {
        var id: Int
        var title: String
        var message: String?
        var placeholder: String
        var text: String
        var confirm: String
        var cancel: String

        init?(_ message: [String: Any]) {
            guard message["ev"] as? String == "prompt",
                  let id = (message["id"] as? NSNumber)?.intValue,
                  let title = message["title"] as? String,
                  let confirm = message["confirm"] as? String,
                  let cancel = message["cancel"] as? String else { return nil }
            self.id = id
            self.title = title
            self.message = message["message"] as? String
            placeholder = message["placeholder"] as? String ?? ""
            text = message["text"] as? String ?? ""
            self.confirm = confirm
            self.cancel = cancel
        }
    }

    /// The screen's answer to prompt `id`.
    nonisolated static func answer(id: Int, text: String?) -> [String: Any] {
        ["op": "prompted", "id": id, "text": text.map { $0 as Any } ?? NSNull()]
    }

    /// Asks `question` over the screen `view` is on; `answer` hears the
    /// screen's message (once).
    static func present(_ question: Question, from view: UIView,
                        answer: @escaping ([String: Any]) -> Void) {
        guard let presenter = presenter(for: view) else {
            Log.ui.error("prompt: no screen to ask on")
            return answer(Self.answer(id: question.id, text: nil))
        }
        let alert = UIAlertController(title: question.title, message: question.message,
                                      preferredStyle: .alert)
        // The appearance of the page that asks (R18): the terminal's colour
        // scheme's from the terminal, the app's from any other page.
        alert.overrideUserInterfaceStyle = view.traitCollection.userInterfaceStyle
        alert.addTextField { field in
            field.placeholder = question.placeholder
            field.text = question.text
            field.autocorrectionType = .no
            field.autocapitalizationType = .none
            field.accessibilityIdentifier = "nsh-prompt-field"
        }
        alert.addAction(UIAlertAction(title: question.cancel, style: .cancel) { _ in
            answer(Self.answer(id: question.id, text: nil))
        })
        let confirm = UIAlertAction(title: question.confirm, style: .default) { [weak alert] _ in
            answer(Self.answer(id: question.id, text: alert?.textFields?.first?.text ?? ""))
        }
        alert.addAction(confirm)
        alert.preferredAction = confirm
        presenter.present(alert, animated: true)
    }

    /// The controller on top of `view`'s window, which presents.
    static func presenter(for view: UIView) -> UIViewController? {
        var top = view.window?.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top
    }
}
