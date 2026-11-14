import MoshpitKit
import SwiftUI
import UIKit

/// The terminal screen drawn by NSH (`ui/src/terminal`), in place of
/// ``TerminalScreen``'s SwiftUI chrome behind the switch
/// `MOSHPIT_NSH_TERMINAL` (``NSHScreen/terminal``), read once when the
/// screen is made.
///
/// ``TerminalScreen`` stays the owner of everything that is not drawn: the
/// session and its lifecycle (connect, reconnect, the protocol switch,
/// leaving), the person's keyboard intent, the sheets, popovers, cards and
/// pickers it presents, the hardware keyboard's commands. It reads what the
/// chrome shows from its session as its SwiftUI body does
/// (``TerminalNSHChrome``) and hands it to this page; the page posts it to
/// the screen when it changes, and turns what the screen tells it into
/// ``TerminalNSHIntent``s for the screen to perform. The terminal itself is
/// the session's, shown by the page's ``TerminalNSHStage`` in the slot the
/// screen lays out.
///
/// G5's three rules (``NSHPage``): the page ignores every safe area, the
/// keyboard's included — the screen rides the keyboard itself, the
/// shortcut bar on the keyboard's own curve; SwiftUI's bar is hidden; a
/// rebuild hands over (the model and the stage live with the screen, a
/// surface made again starts from them).
struct TerminalNSHPage<Anchors: View>: View {
    let model: TerminalNSHModel
    let chrome: TerminalNSHChrome
    let bar: TerminalNSHBar
    let diagnostics: TerminalNSHDiagnostics
    /// The panels' controllers, while each is up.
    let dictation: VoiceDictationController?
    let attachment: ImageAttachmentController?
    let preview: RemotePreviewModel?
    /// What the slot shows, and how the keyboard and a sheet's hold treat it.
    let target: TerminalNSHStage.Target?
    let focus: TerminalFocusPolicy
    let geometryHeld: Bool
    let appearance: TerminalNSHStage.Appearance
    let onPaste: (String) -> Void
    let perform: (TerminalNSHIntent) -> Void
    var onFailure: () -> Void = {}
    /// The host's popovers and their anchors, laid over the page where the
    /// screen says the pieces they hang from are (``TerminalNSHModel/anchors``).
    @ViewBuilder var anchors: () -> Anchors

    var body: some View {
        ZStack(alignment: .topLeading) {
            NSHPage(
                screen: .terminal,
                params: ["terminal": terminalBackground],
                paramsKey: terminalBackground,
                prepare: { surface in model.prepare(surface) },
                attach: { surface in model.attach(surface, chrome: chrome, bar: bar, diagnostics: diagnostics) },
                onMessage: { message in
                    guard let intent = model.handle(message) else { return }
                    perform(intent)
                },
                onFailure: onFailure)
            anchors()
        }
        .ignoresSafeArea(.all)
        .toolbar(.hidden, for: .navigationBar)
        .onAppear { update() }
        // The changed values themselves, never the view's captured ones: an
        // action can run with the previous body's `self`, and posted the
        // chrome one change late (the veil stayed up after the reconnect).
        .onChange(of: chrome) { _, new in model.update(new) }
        .onChange(of: bar) { _, new in model.update(new) }
        .onChange(of: diagnostics) { _, new in model.update(new) }
        .onChange(of: dictation.map(ObjectIdentifier.init)) { model.watch(dictation: dictation) }
        .onChange(of: attachment.map(ObjectIdentifier.init)) { model.watch(attachment: attachment) }
        .onChange(of: preview.map(ObjectIdentifier.init)) { model.watch(preview: preview) }
        // A protocol switch's swap is at once (`chrome.still`): the two
        // change in one value, so the swap reads the switch it belongs to.
        .onChange(of: StageShown(target: target, instant: chrome.still)) { _, new in
            model.stage.show(new.target, instant: new.instant)
        }
        .onChange(of: focus) { _, new in model.stage.apply(focus: new, geometryHeld: geometryHeld) }
        .onChange(of: geometryHeld) { _, new in model.stage.apply(focus: focus, geometryHeld: new) }
        .onChange(of: appearance) { _, new in model.stage.apply(appearance: new) }
    }

    /// The terminal theme's background (`#RRGGBB`): the screen's chrome is
    /// light on a light one (`ui/src/screen.rs`, `Params.terminal`), and
    /// runs again when it changes.
    private var terminalBackground: String {
        NSHScreenParams.hex(appearance.theme.background)
    }

    private func update() {
        model.stage.onPaste = onPaste
        model.stage.apply(appearance: appearance)
        model.stage.apply(focus: focus, geometryHeld: geometryHeld)
        model.stage.show(target, instant: chrome.still)
        model.update(chrome)
        model.update(bar)
        model.update(diagnostics)
        model.watch(dictation: dictation)
        model.watch(attachment: attachment)
        model.watch(preview: preview)
    }
}

// MARK: - The model

/// What the NSH terminal page keeps across a surface made again: the
/// stage with the terminal, what the screen was last told, where the bar's
/// pieces are.
@MainActor @Observable
final class TerminalNSHModel {
    let stage = TerminalNSHStage()
    @ObservationIgnored private weak var surface: NSHSurface?
    @ObservationIgnored private(set) var chrome: TerminalNSHChrome?
    @ObservationIgnored private(set) var bar: TerminalNSHBar?
    @ObservationIgnored private(set) var diagnostics: TerminalNSHDiagnostics?
    /// Hangs the page's action sheets from the piece they belong to.
    @ObservationIgnored private let dialogAnchor = NSHDialogAnchor()
    /// Starts what the surface moves with the keyboard (the shortcut bar)
    /// when the keyboard starts, not when its notification came. Off for
    /// one launch with `-MOSHPIT_NSH_KEYBOARD_ALIGN 0` (to measure NSH's own).
    @ObservationIgnored private(set) var keyboardAlign: NSHKeyboardAlign?
    /// The panels' controllers, and what each panel was last told.
    @ObservationIgnored private weak var dictation: VoiceDictationController?
    @ObservationIgnored private weak var attachment: ImageAttachmentController?
    @ObservationIgnored private weak var preview: RemotePreviewModel?
    @ObservationIgnored private var panels: [String: TerminalNSHPanel] = [:]
    @ObservationIgnored private var level: Float?

    /// Where the screen's breadcrumb and transport pill are, in the page's
    /// points: the host's popovers hang from them.
    struct Anchors: Equatable {
        var breadcrumb: CGRect?
        var pill: CGRect?
    }
    private(set) var anchors = Anchors()

    /// The surface is made, the screen about to run on it: the host's
    /// components first.
    func prepare(_ surface: NSHSurface) {
        TerminalNSHComponents.register(on: surface, stage: stage)
        TerminalNSHBottomComponents.register(on: surface) { [weak self] id in
            self?.attachment?.attachments.first { "\($0.id)" == id }?.thumbnail
        }
    }

    #if DEBUG
    /// The page on screen, for the debug director.
    static weak var current: TerminalNSHModel?
    #endif

    /// `surface` shows the terminal from now on: everything as it is.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        let key = "MOSHPIT_NSH_KEYBOARD_ALIGN"
        let off = UserDefaults.standard.object(forKey: key) != nil && !UserDefaults.standard.bool(forKey: key)
        if !off { keyboardAlign = NSHKeyboardAlign(surface: surface) }
        #if DEBUG
        Self.current = self
        #endif
        if let chrome { surface.post(chrome.message) }
        if let bar { surface.post(bar.message) }
        if let diagnostics { surface.post(diagnostics.message) }
        for (op, panel) in panels.sorted(by: { $0.key < $1.key }) {
            surface.post(TerminalNSHPanel.message(op, panel))
        }
        if let level { surface.post(TerminalNSHPanel.level(level)) }
    }

    /// ``attach(_:)``, the model filled from the chrome, bar and diagnostics
    /// the page's body has now where it has none (B366): the screen starts
    /// from the page's state whether or not `onAppear` came first. What the
    /// model has is newer (`onChange` keeps it: the veil) and stays.
    func attach(_ surface: NSHSurface, chrome: TerminalNSHChrome, bar: TerminalNSHBar,
                diagnostics: TerminalNSHDiagnostics) {
        self.surface = nil
        if self.chrome == nil { update(chrome) }
        if self.bar == nil { update(bar) }
        if self.diagnostics == nil { update(diagnostics) }
        attach(surface)
    }

    func update(_ chrome: TerminalNSHChrome) {
        guard chrome != self.chrome else { return }
        self.chrome = chrome
        surface?.post(chrome.message)
    }

    /// Posts `message` (one of the screen's) to it, if it runs.
    @discardableResult
    func post(_ message: [String: Any]) -> Bool {
        surface?.post(message) ?? false
    }

    func update(_ bar: TerminalNSHBar) {
        guard bar != self.bar else { return }
        self.bar = bar
        surface?.post(bar.message)
    }

    func update(_ diagnostics: TerminalNSHDiagnostics) {
        guard diagnostics != self.diagnostics else { return }
        self.diagnostics = diagnostics
        surface?.post(diagnostics.message)
    }

    // MARK: The panels

    /// Posts panel `op`'s state when it changed (nil: the panel goes).
    private func show(_ op: String, _ panel: TerminalNSHPanel?) {
        guard panels[op] != panel else { return }
        panels[op] = panel
        surface?.post(TerminalNSHPanel.message(op, panel))
    }

    /// Which controllers' observation chains are current: a chain left by a
    /// controller since replaced stops at its next change.
    @ObservationIgnored private var chains: [String: Int] = [:]

    private func chain(_ op: String) -> Int {
        let next = (chains[op] ?? 0) + 1
        chains[op] = next
        return next
    }

    func watch(dictation controller: VoiceDictationController?) {
        if controller === dictation, (controller == nil) == (panels["dictation"] == nil) { return }
        dictation = controller
        level = nil
        observeDictation(chain("dictation"))
        observeLevel(chain("level"))
    }

    /// The voice panel follows its controller (Observation, re-armed on
    /// every change), its level on its own: the meter moves many times a
    /// second, the panel does not.
    private func observeDictation(_ id: Int) {
        guard chains["dictation"] == id else { return }
        guard let controller = dictation else { return show("dictation", nil) }
        let panel = withObservationTracking {
            TerminalNSHPanel.dictation(controller)
        } onChange: { [weak self] in
            DispatchQueue.main.async { self?.observeDictation(id) }
        }
        show("dictation", panel)
    }

    private func observeLevel(_ id: Int) {
        guard chains["level"] == id, let controller = dictation else { return }
        let level = withObservationTracking {
            controller.level
        } onChange: { [weak self] in
            DispatchQueue.main.async { self?.observeLevel(id) }
        }
        guard level != self.level else { return }
        self.level = level
        surface?.post(TerminalNSHPanel.level(level))
    }

    func watch(attachment controller: ImageAttachmentController?) {
        if controller === attachment, (controller == nil) == (panels["attachment"] == nil) { return }
        attachment = controller
        observeAttachment(chain("attachment"))
    }

    private func observeAttachment(_ id: Int) {
        guard chains["attachment"] == id else { return }
        guard let controller = attachment else { return show("attachment", nil) }
        let panel = withObservationTracking {
            TerminalNSHPanel.attachment(controller)
        } onChange: { [weak self] in
            DispatchQueue.main.async { self?.observeAttachment(id) }
        }
        show("attachment", panel)
    }

    func watch(preview model: RemotePreviewModel?) {
        if model === preview, (model == nil) == (panels["preview"] == nil) { return }
        preview = model
        observePreview(chain("preview"))
    }

    private func observePreview(_ id: Int) {
        guard chains["preview"] == id else { return }
        guard let model = preview else { return show("preview", nil) }
        let panel = withObservationTracking {
            TerminalNSHPanel.preview(model)
        } onChange: { [weak self] in
            DispatchQueue.main.async { self?.observePreview(id) }
        }
        show("preview", panel)
    }

    /// What the screen told the host: the stage's and the anchors' are
    /// kept here, the rest is the screen's to perform.
    func handle(_ message: [String: Any]) -> TerminalNSHIntent? {
        switch message["ev"] as? String {
        case "handover":
            stage.handover()
            return nil
        case "dialog-anchor":
            // The pill's question: on the iPad its popover hangs from the
            // pill, as SwiftUI's confirmationDialog does.
            if message["name"] as? String == "pill", let surface, let pill = anchors.pill {
                dialogAnchor.arm(surface: surface, rect: pill)
            }
            return nil
        case "anchor":
            guard let name = message["name"] as? String,
                  let rect = (message["rect"] as? [Any])?.compactMap(NSHHostComponents.number),
                  rect.count == 4 else { return nil }
            let frame = CGRect(x: rect[0], y: rect[1], width: rect[2], height: rect[3])
            switch name {
            case "breadcrumb": if anchors.breadcrumb != frame { anchors.breadcrumb = frame }
            case "pill": if anchors.pill != frame { anchors.pill = frame }
            default: break
            }
            return nil
        default:
            guard let intent = TerminalNSHIntent(message) else {
                Log.ui.error("terminal: no intent in \(String(describing: message["ev"]), privacy: .public)")
                return nil
            }
            return intent
        }
    }
}

// MARK: - The intents

/// What the NSH terminal asks for (`ui/src/terminal/mod.rs`'s `Intent`).
enum TerminalNSHIntent: Equatable {
    enum Crumb: String { case session, window, pane }
    enum Panel: String { case cancel, insert, send, settings, retry, confirm }

    /// `transport`: the pill's switch, confirmed on the page's action sheet.
    case back, sidebar, transport, diagnostics, diagnosticsDismiss, repaint
    case crumb(Crumb)
    /// ⌘1–⌘9: the n-th window in the picker's order.
    case window(Int)
    case takeOver, create, install
    case bannerAction, bannerDismiss, noticeAction, openLink
    case chip(String)
    case arrow(String)
    case scroll(String)
    case keyboard, mic, image, imagePaste, imageCamera
    case imageSent(String)
    case dictation(Panel)
    case attachment(Panel)
    case preview(Panel)

    init?(_ message: [String: Any]) {
        guard let ev = message["ev"] as? String else { return nil }
        let panel = (message["action"] as? String).flatMap(Panel.init(rawValue:))
        switch ev {
        case "back": self = .back
        case "sidebar": self = .sidebar
        case "switch-transport": self = .transport
        case "diagnostics": self = .diagnostics
        case "diagnostics-dismiss": self = .diagnosticsDismiss
        case "repaint": self = .repaint
        case "crumb":
            guard let crumb = (message["crumb"] as? String).flatMap(Crumb.init(rawValue:)) else { return nil }
            self = .crumb(crumb)
        case "window":
            guard let number = (message["number"] as? NSNumber)?.intValue, (1...9).contains(number) else { return nil }
            self = .window(number)
        case "take-over": self = .takeOver
        case "create": self = .create
        case "install": self = .install
        case "banner-action": self = .bannerAction
        case "banner-dismiss": self = .bannerDismiss
        case "notice-action": self = .noticeAction
        case "open-link": self = .openLink
        case "chip":
            guard let id = message["id"] as? String else { return nil }
            self = .chip(id)
        case "arrow", "scroll":
            guard let dir = message["dir"] as? String,
                  ["up", "down", "left", "right"].contains(dir) else { return nil }
            self = ev == "arrow" ? .arrow(dir) : .scroll(dir)
        case "keyboard": self = .keyboard
        case "mic": self = .mic
        case "image": self = .image
        case "image-paste": self = .imagePaste
        case "image-camera": self = .imageCamera
        case "image-sent":
            guard let id = message["id"] as? String else { return nil }
            self = .imageSent(id)
        case "dictation":
            guard let panel else { return nil }
            self = .dictation(panel)
        case "attachment":
            guard let panel else { return nil }
            self = .attachment(panel)
        case "preview":
            guard let panel else { return nil }
            self = .preview(panel)
        default: return nil
        }
    }
}

// MARK: - mosh's counters

/// mosh's live counters while the page shows them, hung from the pill
/// (`ui/src/terminal/model.rs`'s `Diagnostics`): `{"op": "diagnostics", …}`.
struct TerminalNSHDiagnostics: Equatable {
    var shown = false
    var counters: MoshDiagnostics?

    var message: [String: Any] {
        var message: [String: Any] = ["op": "diagnostics", "shown": shown]
        guard shown else { return message }
        if let d = counters {
            message["counters"] = [
                "datagrams": d.datagramsReceived,
                "applied": d.appliedHostNum,
                "parseFails": d.parseFailures,
                "gaps": d.gapEvents,
                "sent": d.packetsSent,
                "user": d.lastUserNum,
                "acked": d.serverAckedUserNum,
                "channel": d.channelState,
                "held": d.inputHeld,
            ] as [String: Any]
        } else {
            message["counters"] = NSNull()
        }
        return message
    }
}

// MARK: - What the chrome shows

/// Everything the NSH terminal draws but the shortcut bar and the panels,
/// as ``TerminalScreen`` reads it from its session (`ui/src/terminal/
/// model.rs`'s `Chrome`): `{"op": "chrome", …}`.
/// What the page's stage shows and whether the change is a protocol
/// switch's (at once): one value, so the two never come apart.
struct StageShown: Equatable {
    var target: TerminalNSHStage.Target?
    var instant: Bool
}

struct TerminalNSHChrome: Equatable {
    var name = ""
    var user = ""
    var host = ""
    var port = 22
    var mosh = false
    var mux: Multiplexer = .none
    var sidebar = false
    var pill: TransportPillKind = .ssh
    var state: TransportConnState = .connecting
    var crumbs: Crumbs?
    var cover = Cover()
    var content: Content = .none
    var empty: Empty?
    var banner: Banner?
    var notice: Notice?
    var link: String?
    var compact = false
    /// A protocol switch is under way: nothing on the page moves
    /// (`ui/src/terminal/model.rs`'s `Chrome.still`).
    var still = false

    struct Crumbs: Equatable {
        var plan: BreadcrumbPlan
        var sessionIcon: String
        var windowIcon: String
        /// The live tree's (else the remembered one, dimmed and inert).
        var live: Bool
    }

    enum CoverKind: String { case none, poster, veil, follow }

    struct Cover: Equatable {
        var kind: CoverKind = .none
        var state: TransportConnState = .connecting
        var ended = false
        var owner: String?
        var touches = false
    }

    enum Content: String { case none, terminal, blank, empty }

    struct Empty: Equatable {
        var mux: Multiplexer
        var missing: Bool
        var quiet: Bool
        /// The host has the answer already (the binary known missing, the
        /// server said it has no session or no server): the screen shows it
        /// at once instead of waiting out the attach's seconds first (the
        /// SwiftUI view's `startsSettled`).
        var settled = false
    }

    enum Banner: Equatable {
        case returnPath
        case roam(srtt: Double?, history: [Double])
        case degrade(DegradeNotice.Missing)
    }

    struct Notice: Equatable {
        var text: String
        var action: String?
    }

    var message: [String: Any] {
        var message: [String: Any] = [
            "op": "chrome",
            "name": name,
            "user": user,
            "host": host,
            "port": port,
            "mosh": mosh,
            "mux": mux.rawValue,
            "sidebar": sidebar,
            "pill": Self.pill(pill),
            "state": Self.state(state),
            "cover": coverJSON,
            "content": content.rawValue,
            "compact": compact,
            "still": still,
        ]
        if let crumbs { message["crumbs"] = Self.json(crumbs) }
        if let empty {
            let vocabulary = empty.mux.vocabulary
            message["empty"] = [
                "mux": empty.mux.rawValue,
                "missing": empty.missing,
                "quiet": empty.quiet,
                "settled": empty.settled,
                "session": vocabulary.session,
                "sessionLower": vocabulary.sessionLower,
                "sessionsLower": vocabulary.sessionPluralLower,
            ] as [String: Any]
        }
        if let banner { message["banner"] = Self.json(banner) }
        if let notice {
            var json: [String: Any] = ["text": notice.text]
            if let action = notice.action { json["action"] = action }
            message["notice"] = json
        }
        if let link { message["link"] = link }
        return message
    }

    private var coverJSON: [String: Any] {
        var json: [String: Any] = [
            "kind": cover.kind.rawValue,
            "state": Self.state(cover.state),
            "ended": cover.ended,
            "touches": cover.touches,
        ]
        if let owner = cover.owner { json["owner"] = owner }
        return json
    }

    static func pill(_ kind: TransportPillKind) -> String {
        switch kind {
        case .ssh: "ssh"
        case .mosh: "mosh"
        case .moshRoaming: "moshRoaming"
        }
    }

    static func state(_ state: TransportConnState) -> String {
        switch state {
        case .live: "live"
        case .connecting: "connecting"
        case .reconnecting: "reconnecting"
        case .offline: "offline"
        }
    }

    static func json(_ crumbs: Crumbs) -> [String: Any] {
        let plan = crumbs.plan
        var json: [String: Any] = [
            "session": plan.sessionTitle,
            "squeezable": plan.sessionSqueezable,
            "window": plan.windowTitle,
            "sessionIcon": crumbs.sessionIcon,
            "windowIcon": crumbs.windowIcon,
            "live": crumbs.live,
        ]
        if let tree = plan.tree { json["tree"] = tree.json }
        if let pane = plan.paneTitle {
            json["pane"] = pane
            // What VoiceOver reads: the title and what the dot means.
            json["paneLabel"] = plan.paneSignal.map { "\(pane), \($0.label)" } ?? pane
        }
        if let signal = plan.paneSignal {
            json["signal"] = switch signal {
            case .attention: "attention"
            case .working: "working"
            case .done: "done"
            }
        }
        return json
    }

    static func json(_ banner: Banner) -> [String: Any] {
        switch banner {
        case .returnPath:
            return ["kind": "returnPath"]
        case .roam(let srtt, let history):
            var json: [String: Any] = ["kind": "roam", "history": history]
            if let srtt { json["srtt"] = srtt }
            return json
        case .degrade(let missing):
            let name = switch missing {
            case .tmux: "tmux"
            case .herdr: "herdr"
            case .moshServer: "moshServer"
            }
            return ["kind": "degrade", "missing": name]
        }
    }
}
