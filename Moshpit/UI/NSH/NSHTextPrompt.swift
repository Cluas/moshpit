import MoshpitKit
import UIKit

/// The question with a line of text for its answer that an NSH screen asks
/// through its host (`ui/src/terminal/prompt.rs`): NSH's alerts have no
/// text field, so the screen says what to ask (``TextPromptQuestion``,
/// every word already in the app's language, in the event of the screen
/// that asks) and the host asks it with UIKit's alert, as SwiftUI's
/// `.alert` with a `TextField` does, then answers the screen
/// (``PageMessage/prompted(id:text:)``, no text: Cancel).
@MainActor
enum NSHTextPrompt {
    /// The screen's answer to prompt `id`.
    nonisolated static func answer(id: UInt32, text: String?) -> PageMessage {
        .prompted(id: id, text: text)
    }

    /// Asks `question` over the screen `view` is on; `answer` hears the
    /// screen's message (once).
    static func present(_ question: TextPromptQuestion, from view: UIView,
                        answer: @escaping (PageMessage) -> Void) {
        guard let presenter = presenter(for: view) else {
            Log.ui.error("prompt: no screen to ask on")
            return answer(Self.answer(id: question.id, text: nil))
        }
        let alert = UIAlertController(title: question.title, message: question.message,
                                      preferredStyle: .alert)
        // The appearance of the page that asks (R18): the terminal's colour
        // scheme's from the terminal, the app's from any other page.
        alert.overrideUserInterfaceStyle = view.traitCollection.userInterfaceStyle
        // No preferred action: iOS 26 fills a preferred action's capsule
        // from a colour the alert's tint does not reach (seen on device: a
        // light blue fill under purple words), and no public API sets its
        // label, so the pair can't be made to read. Without one, Create is
        // words in the tint on the alert's own platter, as Cancel is words
        // in the label colour; the tint is the accent made readable on
        // that platter in either appearance (4.5:1, the tokens'
        // `readable_on`, ``InkContrast/alertTint(_:)``).
        alert.view.tintColor = InkContrast.alertTint(UIColor(AppThemeCatalog.current.accent))
        // Answered once, by a button or by Return.
        var answered = false
        let finish = { (text: String?) in
            guard !answered else { return }
            answered = true
            answer(Self.answer(id: question.id, text: text))
        }
        alert.addTextField { field in
            field.placeholder = question.placeholder
            field.text = question.text
            field.autocorrectionType = .no
            field.autocapitalizationType = .none
            field.accessibilityIdentifier = "nsh-prompt-field"
            field.returnKeyType = .done
            // Return confirms, as the preferred action did.
            field.addAction(UIAction { [weak field] _ in
                guard let field else { return }
                finish(field.text ?? "")
                presenter.dismiss(animated: true)
            }, for: .primaryActionTriggered)
        }
        alert.addAction(UIAlertAction(title: question.cancel, style: .cancel) { _ in
            finish(nil)
        })
        alert.addAction(UIAlertAction(title: question.confirm, style: .default) { [weak alert] _ in
            finish(alert?.textFields?.first?.text ?? "")
        })
        presenter.present(alert, animated: true)
    }

    /// The controller on top of `view`'s window, which presents.
    static func presenter(for view: UIView) -> UIViewController? {
        var top = view.window?.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top
    }
}

extension TextPromptQuestion {
    /// What `message` asks (`{"ev": "prompt", "id", "title", "message",
    /// "placeholder", "text", "confirm", "cancel"}`), for a screen whose
    /// events don't cross the page bridge yet (the terminal's); nil when it
    /// is not a prompt.
    init?(_ message: [String: Any]) {
        guard message["ev"] as? String == "prompt",
              let id = (message["id"] as? NSNumber).flatMap({ UInt32(exactly: $0.int64Value) }),
              let title = message["title"] as? String,
              let confirm = message["confirm"] as? String,
              let cancel = message["cancel"] as? String else { return nil }
        self.init(id: id, title: title, message: message["message"] as? String,
                  placeholder: message["placeholder"] as? String ?? "", text: message["text"] as? String ?? "",
                  confirm: confirm, cancel: cancel)
    }
}
