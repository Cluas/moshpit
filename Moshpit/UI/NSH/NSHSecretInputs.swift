import UIKit

/// The host's own fields for what an NSH screen must never hold — a
/// private key, a passphrase, a password (NSH-GAPS #11): the screen lays
/// them out with `native(…)`, and what is typed goes to the host's form
/// (`changed`) and nowhere else, never the engine or the channel. A screen
/// that needs to know whether one holds anything (to enable Save) gets a
/// `filled` event (a Bool, `filled(text)`) each time that changes, and only
/// that. Each keystroke reaches the form as the field's whole text, on the
/// main thread as UIKit hands it over: none is lost, whatever the screen
/// does meanwhile. Each says when it takes the keyboard (`focus`) and lets
/// it go (`blur`), which NSH cannot see in a host's view: the `ui` layer's
/// secret field rings itself while it has it.

/// A private key's editor (`moshpit/key-pem`, `moshpit/connection-pem`): a
/// `UITextView` as SwiftUI's `TextEditor` is (mono, no autocorrection),
/// growing with the key. Props `fontSize`, `textColor`, `tint`,
/// `identifier`, `label`. Reports the height its text needs.
@MainActor
final class NSHSecretEditor: NSObject, NSHComponent, UITextViewDelegate {
    private let container = LayoutView()
    private let textView = UITextView()
    /// What the editor tells its screen (none outside a surface: a test's).
    private let context: NSHComponentContext?
    private let changed: (String) -> Void
    private let filled: ((String) -> Bool)?
    /// What the screen was last told (or knew from its form) of `filled`.
    private var told: Bool?
    private var reported: CGFloat?
    var view: UIView { container }

    init(text: String, context: NSHComponentContext?, filled: ((String) -> Bool)? = nil,
         changed: @escaping (String) -> Void) {
        self.context = context
        self.changed = changed
        self.filled = filled
        super.init()
        textView.backgroundColor = .clear
        textView.isScrollEnabled = false
        textView.autocorrectionType = .no
        textView.autocapitalizationType = .none
        textView.spellCheckingType = .no
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        // The keyboard of the page's appearance (R18).
        textView.keyboardAppearance = .default
        textView.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        textView.text = text
        told = filled?(text)
        textView.delegate = self
        container.addSubview(textView)
        container.onLayout = { [weak self] in self?.layout() }
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "identifier":
            textView.accessibilityIdentifier = value as? String
        case "label":
            textView.accessibilityLabel = value as? String
        case "fontSize":
            let size = CGFloat(NSHHostComponents.number(value) ?? 11)
            textView.font = .monospacedSystemFont(ofSize: size, weight: .regular)
            report()
        case "textColor":
            textView.textColor = NSHHostComponents.color(value)
        case "tint":
            textView.tintColor = NSHHostComponents.color(value)
        default:
            break
        }
    }

    /// Shows `text`, which its form already holds (a file picked for it).
    func show(_ text: String) {
        textView.text = text
        tell()
        report()
    }

    /// `text` pasted in, as UIKit hands a paste to the view and its delegate.
    func paste(_ text: String) {
        textView.text = text
        textViewDidChange(textView)
    }

    func textViewDidBeginEditing(_ textView: UITextView) {
        context?.emit("focus")
    }

    func textViewDidEndEditing(_ textView: UITextView) {
        context?.emit("blur")
    }

    func textViewDidChange(_ textView: UITextView) {
        changed(textView.text)
        tell()
        report()
    }

    private func tell() {
        guard let filled else { return }
        let now = filled(textView.text)
        guard now != told else { return }
        told = now
        context?.emit("filled", now)
    }

    private func layout() {
        textView.frame = container.bounds
        report()
    }

    /// The height the text needs at the editor's width.
    private func report() {
        let width = container.bounds.width
        guard width > 0 else { return }
        let height = textView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        guard reported.map({ abs($0 - height) > 0.25 }) ?? true else { return }
        reported = height
        context?.setSize(CGSize(width: width, height: height))
    }
}

/// A secret on one line (`moshpit/key-passphrase`,
/// `moshpit/connection-password`): a secure `UITextField` as SwiftUI's
/// `SecureField` is. Props `placeholder`, `placeholderColor`, `fontSize`,
/// `textColor`, `tint`, `identifier`.
@MainActor
final class NSHSecretField: NSHComponent {
    private let field = UITextField()
    private let context: NSHComponentContext?
    private let changed: (String) -> Void
    private let filled: ((String) -> Bool)?
    private var told: Bool?
    private var placeholder = ""
    private var placeholderColor = UIColor.placeholderText
    var view: UIView { field }

    init(text: String, context: NSHComponentContext?, filled: ((String) -> Bool)? = nil,
         changed: @escaping (String) -> Void) {
        self.context = context
        self.changed = changed
        self.filled = filled
        field.isSecureTextEntry = true
        field.autocorrectionType = .no
        field.autocapitalizationType = .none
        // The keyboard of the page's appearance (R18).
        field.keyboardAppearance = .default
        field.font = .preferredFont(forTextStyle: .body)
        field.text = text
        told = filled?(text)
        field.addAction(UIAction { [weak self] _ in self?.edited() }, for: .editingChanged)
        field.addAction(UIAction { [weak self] _ in self?.context?.emit("focus") }, for: .editingDidBegin)
        field.addAction(UIAction { [weak self] _ in self?.context?.emit("blur") }, for: .editingDidEnd)
    }

    private func edited() {
        let text = field.text ?? ""
        changed(text)
        guard let filled else { return }
        let now = filled(text)
        guard now != told else { return }
        told = now
        context?.emit("filled", now)
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "identifier":
            field.accessibilityIdentifier = value as? String
        case "placeholder":
            placeholder = value as? String ?? ""
            showPlaceholder()
        case "placeholderColor":
            placeholderColor = NSHHostComponents.color(value) ?? .placeholderText
            showPlaceholder()
        case "fontSize":
            field.font = .systemFont(ofSize: CGFloat(NSHHostComponents.number(value) ?? 17))
        case "textColor":
            field.textColor = NSHHostComponents.color(value)
        case "tint":
            field.tintColor = NSHHostComponents.color(value)
        case "alignment":
            // `trailing` beside a label (B319), `leading` under one.
            field.textAlignment = value as? String == "trailing" ? .right : .natural
        default:
            break
        }
    }

    private func showPlaceholder() {
        field.attributedPlaceholder = NSAttributedString(
            string: placeholder, attributes: [.foregroundColor: placeholderColor])
    }
}

/// A view that says when it is laid out.
final class LayoutView: UIView {
    var onLayout: (() -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        onLayout?()
    }
}
