import SwiftUI
import UIKit

// The NSH terminal's bottom: the shortcut bar's state (`{"op": "bar", …}`)
// and the panels that ride above it — voice input, an image upload, a host
// file's preview — read from their controllers, and the host components
// they place.

/// The shortcut bar (`ui/src/terminal/model.rs`'s `Bar`).
struct TerminalNSHBar: Equatable {
    struct Chip: Equatable {
        var id: String
        var kind: String
        var label: String
    }

    struct Sent: Equatable {
        var id: String
        var number: Int
        var filename: String
    }

    var chips: [Chip] = []
    var keyboardDown = true
    var ctrlArmed = false
    var voice = false
    var micActive = false
    var clipboardHasImage = false
    var camera = false
    var sent: [Sent] = []

    /// A shortcut's chip, as the bar draws its kind.
    static func chip(_ shortcut: TerminalShortcut) -> Chip {
        let kind = switch shortcut.kind {
        case .keyCombo: "key"
        case .text, .command: "text"
        case .ctrl: "ctrl"
        case .paste: "paste"
        case .files: "files"
        case .image: "image"
        case .mic: "mic"
        case .arrows: "arrows"
        case .dpad: "dpad"
        case .scroll: "scroll"
        }
        return Chip(id: shortcut.id.uuidString, kind: kind, label: shortcut.chipLabel)
    }

    var message: [String: Any] {
        [
            "op": "bar",
            "chips": chips.map { ["id": $0.id, "kind": $0.kind, "label": $0.label] },
            "keyboardDown": keyboardDown,
            "ctrlArmed": ctrlArmed,
            "voice": voice,
            "micActive": micActive,
            "image": [
                "clipboard": clipboardHasImage,
                "camera": camera,
                "sent": sent.map { ["id": $0.id, "number": $0.number, "filename": $0.filename] as [String: Any] },
            ] as [String: Any],
        ]
    }
}

/// What a panel shows, in its message's shape: compared to post only a
/// change.
struct TerminalNSHPanel: Equatable {
    let json: String
    let object: [String: Any]

    init(_ object: [String: Any]) {
        self.object = object
        let data = (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])) ?? Data()
        json = String(decoding: data, as: UTF8.self)
    }

    static func == (a: TerminalNSHPanel, b: TerminalNSHPanel) -> Bool { a.json == b.json }

    // MARK: Voice

    /// `{"op": "dictation", "panel": …}` of `controller`'s state (its level
    /// goes on its own: ``level(_:)``).
    @MainActor
    static func dictation(_ controller: VoiceDictationController) -> TerminalNSHPanel {
        let phase: [String: Any]
        switch controller.phase {
        case .idle, .starting: phase = ["phase": "starting"]
        case .downloadingModel(let progress): phase = ["phase": "downloading", "progress": progress]
        case .loadingModel: phase = ["phase": "loading"]
        case .listening: phase = ["phase": "listening"]
        case .finishing: phase = ["phase": "finishing"]
        case .interrupted: phase = ["phase": "interrupted"]
        case .failed(let failure):
            // A permission was denied: Settings is where it is given back.
            let denied = failure == .microphoneDenied || failure == .speechRecognitionDenied
            phase = ["phase": "failed", "message": failure.message, "settings": denied]
        }
        // The engine's chip says nothing over a denied microphone.
        let engine = controller.phase == .failed(.microphoneDenied) ? "" : controller.engineLabel
        // Side by side, the two are what Insert types (B673).
        let shown = DictationTranscript.shownPieces(controller.finalizedText, controller.volatileText)
        return TerminalNSHPanel([
            "phase": phase,
            "engine": engine,
            "finalized": shown.finalized,
            "volatile": shown.volatile,
            "insertable": !controller.transcript.isEmpty || controller.phase == .listening,
        ])
    }

    /// `{"op": "level", …}`.
    static func level(_ level: Float) -> [String: Any] {
        ["op": "level", "level": Double(min(max(level, 0), 1))]
    }

    // MARK: Images

    @MainActor
    static func attachment(_ controller: ImageAttachmentController) -> TerminalNSHPanel {
        let phase: [String: Any] = switch controller.phase {
        case .processing: ["phase": "processing"]
        case .uploading(let fraction): ["phase": "uploading", "fraction": fraction]
        case .ready: ["phase": "ready"]
        case .failed(let message): ["phase": "failed", "message": message]
        }
        let bytes = controller.attachments.reduce(0) { $0 + $1.byteCount }
        var object: [String: Any] = [
            "phase": phase,
            "images": controller.attachments.map { attachment -> [String: Any] in
                var image: [String: Any] = [
                    // The thumbnail's key: changes once it is made.
                    "id": attachment.thumbnail == nil ? "\(attachment.id)" : "\(attachment.id)#thumb",
                    "landed": attachment.remotePath != nil,
                ]
                if let number = attachment.sessionNumber { image["number"] = number }
                return image
            },
        ]
        if bytes > 0 {
            object["size"] = ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
        }
        return TerminalNSHPanel(object)
    }

    // MARK: A host file

    /// The preview panel while it shows (`RemotePreviewModel.showsHUD`).
    @MainActor
    static func preview(_ model: RemotePreviewModel) -> TerminalNSHPanel? {
        guard model.showsHUD else { return nil }
        let phase: [String: Any]
        switch model.phase {
        case .looking, .ready, .folder, .stream:
            phase = ["phase": "opening"]
        case .downloading(let entry, let fraction):
            var downloading: [String: Any] = ["phase": "downloading", "fraction": fraction]
            if entry.size != nil { downloading["size"] = RemotePreviewHUD.size(entry.size) }
            phase = downloading
        case .confirm(let entry):
            phase = ["phase": "confirm", "size": RemotePreviewHUD.size(entry.size)]
        case .failed(let message):
            phase = ["phase": "failed", "message": message]
        }
        return TerminalNSHPanel([
            "phase": phase,
            "name": model.name,
            "symbol": RemoteFileIcon.symbol(for: PreviewableFile.kind(of: model.name)),
        ])
    }

    /// The panel's message (`nil`: none shows).
    static func message(_ op: String, _ panel: TerminalNSHPanel?) -> [String: Any] {
        ["op": op, "panel": panel?.object ?? NSNull()]
    }
}

// MARK: - The host components of the bottom

/// What the bottom places that NSH draws no equivalent of:
///
/// - `moshpit/press-menu`: a clear button over a drawn chip: a tap is its
///   primary action (event `press`), a long press opens the system's menu
///   (event `select`, the item's id) — SwiftUI's `Menu(primaryAction:)`.
///   Props `items` (`[{id, title, image?, section?}]`: items with a
///   `section` are grouped under its title), `identifier`, `label`.
/// - `moshpit/thumbnail`: an image being uploaded, 44 pt, cropped to fill,
///   rounded 8 with a hairline (a photo symbol until its thumbnail is
///   made). Prop `image` (the panel's key for it).
/// - `moshpit/sparkline`: the roam banner's round trips (`Sparkline`).
///   Props `samples`, `color`, `lineWidth`.
@MainActor
enum TerminalNSHBottomComponents {
    static func register(on surface: NSHSurface, thumbnails: @escaping @MainActor (String) -> UIImage?) {
        surface.register(component: "moshpit/press-menu") { PressMenuComponent(context: $0) }
        surface.register(component: "moshpit/thumbnail") { _ in ThumbnailComponent(lookup: thumbnails) }
        surface.register(component: "moshpit/sparkline") { _ in SparklineComponent() }
    }
}

/// `moshpit/press-menu`.
@MainActor
private final class PressMenuComponent: NSHComponent {
    private let button = UIButton(type: .custom)
    private let context: NSHComponentContext
    var view: UIView { button }

    init(context: NSHComponentContext) {
        self.context = context
        button.backgroundColor = .clear
        button.showsMenuAsPrimaryAction = false
        button.addAction(UIAction { [context] _ in context.emit("press") }, for: .primaryActionTriggered)
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "identifier": button.accessibilityIdentifier = value as? String
        case "label": button.accessibilityLabel = value as? String
        case "items":
            let items = (value as? [Any] ?? []).compactMap { $0 as? [String: Any] }
            button.menu = items.isEmpty ? nil : Self.menu(items, context: context)
        default: break
        }
    }

    static func menu(_ items: [[String: Any]], context: NSHComponentContext) -> UIMenu {
        var loose: [UIMenuElement] = []
        var sections: [(String, [UIMenuElement])] = []
        for item in items {
            guard let id = item["id"] as? String, let title = item["title"] as? String else { continue }
            let image = (item["image"] as? String).flatMap { UIImage(systemName: $0) }
            let action = UIAction(title: title, image: image) { _ in context.emit("select", id) }
            if let section = item["section"] as? String {
                if let index = sections.firstIndex(where: { $0.0 == section }) {
                    sections[index].1.append(action)
                } else {
                    sections.append((section, [action]))
                }
            } else {
                loose.append(action)
            }
        }
        return UIMenu(children: loose + sections.map {
            UIMenu(title: $0.0, options: .displayInline, children: $0.1)
        })
    }
}

/// `moshpit/thumbnail`.
@MainActor
private final class ThumbnailComponent: NSHComponent {
    private let image = UIImageView()
    private let placeholder = UIImageView(image: UIImage(systemName: "photo",
        withConfiguration: UIImage.SymbolConfiguration(pointSize: 16)))
    private let lookup: @MainActor (String) -> UIImage?
    var view: UIView { image }

    init(lookup: @escaping @MainActor (String) -> UIImage?) {
        self.lookup = lookup
        image.contentMode = .scaleAspectFill
        image.clipsToBounds = true
        image.layer.cornerRadius = 8
        image.layer.cornerCurve = .continuous
        image.layer.borderWidth = 1
        image.layer.borderColor = UIColor(white: 1, alpha: 0.075).cgColor
        image.isAccessibilityElement = false
        placeholder.tintColor = UIColor(Ink.tertiary)
        placeholder.contentMode = .center
        placeholder.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        image.addSubview(placeholder)
    }

    func setProp(_ name: String, _ value: Any?) {
        guard name == "image", let key = value as? String else { return }
        let id = key.components(separatedBy: "#").first ?? key
        image.image = lookup(id)
        placeholder.isHidden = image.image != nil
        placeholder.frame = image.bounds
    }
}

/// `moshpit/sparkline`.
@MainActor
private final class SparklineComponent: NSHComponent {
    private final class Line: UIView {
        override class var layerClass: AnyClass { CAShapeLayer.self }
        var shape: CAShapeLayer { layer as! CAShapeLayer }
        var samples: [Double] = [] { didSet { setNeedsLayout() } }

        override func layoutSubviews() {
            super.layoutSubviews()
            shape.path = Sparkline(samples: samples).path(in: bounds).cgPath
        }
    }

    private let line = Line()
    var view: UIView { line }

    init() {
        line.isUserInteractionEnabled = false
        line.backgroundColor = .clear
        line.shape.fillColor = nil
        line.shape.lineWidth = 1.4
        line.shape.contentsScale = UIScreen.main.scale
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "samples": line.samples = (value as? [Any] ?? []).compactMap(NSHHostComponents.number)
        case "color": line.shape.strokeColor = NSHHostComponents.color(value)?.cgColor
        case "lineWidth": line.shape.lineWidth = CGFloat(NSHHostComponents.number(value) ?? 1.4)
        default: break
        }
    }
}
