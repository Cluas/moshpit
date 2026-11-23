import MoshpitKit
import PhotosUI
import SwiftUI
import UIKit

/// The sheet a tapped folder path (or the Files key) opens, drawn by NSH
/// (`ui/src/terminal/folder.rs`): half height first, full height a drag
/// up, the grabber; each folder an NSH page of its own, pushed in SwiftUI's
/// stack so the edge swipe goes back. If its screen can't run, the sheet
/// says so (``NSHPageFailed``).
struct RemoteFolderNSHSheet: View {
    let files: RemoteFileAccess
    @State private var root: RemoteFileEntry
    @State private var trail: [RemoteFileEntry] = []
    @State private var detent: PresentationDetent = .medium
    @State private var failed = false

    init(folder: RemoteFileEntry, files: RemoteFileAccess) {
        self.files = files
        _root = State(initialValue: folder)
    }

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack(path: $trail) {
                RemoteFolderNSHPage(folder: root, files: files, pushed: false, open: open, climb: climb,
                                    onFailure: { failed = true })
                    .id(root.path)
                    .navigationDestination(for: RemoteFileEntry.self) { entry in
                        RemoteFolderNSHPage(folder: entry, files: files, pushed: true, open: open, climb: climb,
                                            onFailure: { failed = true })
                    }
            }
            .presentationDetents([.medium, .large], selection: $detent)
            .presentationDragIndicator(.visible)
            .sheetSurface()
        }
    }

    /// A subfolder: its own page.
    private func open(_ folder: RemoteFileEntry) {
        trail.append(folder)
    }

    /// Start over at an enclosing folder picked from the title's menu.
    private func climb(to path: String) {
        trail = []
        root = RemoteFileEntry(path: path, isDirectory: true, size: nil, modified: nil)
    }
}

/// One folder on the NSH page: the model is ``RemoteFolderModel`` (the
/// listing, thumbnails, downloads, uploads, edits), posted as it changes
/// (``RemoteFolderFacts``); the pickers, QuickLook and the share sheet stay
/// the host's (UIKit's, presented as the SwiftUI page presents them).
struct RemoteFolderNSHPage: View {
    let pushed: Bool
    let open: (RemoteFileEntry) -> Void
    let climb: (String) -> Void
    let onFailure: () -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var model: RemoteFolderModel
    @State private var page = RemoteFolderNSHModel()
    @State private var menus = NSHContextMenus()
    @State private var showPhotos = false
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var showFiles = false

    init(folder: RemoteFileEntry, files: RemoteFileAccess, pushed: Bool,
         open: @escaping (RemoteFileEntry) -> Void, climb: @escaping (String) -> Void,
         onFailure: @escaping () -> Void) {
        _model = State(initialValue: RemoteFolderModel(folder: folder, files: files))
        self.pushed = pushed
        self.open = open
        self.climb = climb
        self.onFailure = onFailure
    }

    var body: some View {
        let folder = self.model
        @Bindable var model = model
        let facts = RemoteFolderFacts(model: folder, pushed: pushed)
        NSHPage(
            screen: .remoteFolder,
            prepare: { surface in
                surface.register(component: "moshpit/remote-thumbnail") { _ in
                    RemoteThumbnailComponent(lookup: { folder.thumbnails[$0] })
                }
            },
            attach: { surface in
                page.attach(surface, current: facts)
                menus.install(on: surface)
            },
            onEvent: { if case .remoteFolder(let event) = $0 { handle(event) } },
            onFailure: onFailure)
            .nshPage()
            .onAppear { show(facts) }
            .onChange(of: facts) { _, new in show(new) }
            .task { await model.load() }
            .photosPicker(isPresented: $showPhotos, selection: $photoItems, maxSelectionCount: 20, matching: .images)
            .onChange(of: photoItems) { _, items in
                guard !items.isEmpty else { return }
                photoItems = []
                Task { await model.upload(photos: items) }
            }
            .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
                guard case .success(let urls) = result, !urls.isEmpty else { return }
                Task { await model.upload(files: urls) }
            }
            .remotePreview($model.previewURL, in: model.previewItems)
    }

    private func show(_ facts: RemoteFolderFacts) {
        page.update(facts)
        menus.menus = RemoteFolderNSHModel.menus(for: facts) { [weak page] action, path in
            page?.post(.remoteFolderAct(action: action, path: path))
        }
        menus.cornerRadius = Dictionary(uniqueKeysWithValues: facts.media.map {
            ("remote-folder-item-\($0.path)", CGFloat(10))
        })
    }

    private func entry(_ path: String) -> RemoteFileEntry? {
        model.entries.first { $0.path == path }
    }

    private func handle(_ event: RemoteFolderEvent) {
        switch event {
        case .open(let path):
            guard let entry = entry(path) else { return }
            Haptics.tap()
            if entry.isDirectory { open(entry) } else { Task { await model.open(entry) } }
        case .share(let path):
            guard let entry = entry(path) else { return }
            Task { await model.share(entry) }
        case .back:
            dismiss()
        case .climb(let path):
            climb(path)
        case .add(.photos):
            showPhotos = true
        case .add(.files):
            showFiles = true
        case .newFolder(let name):
            Task { await model.createFolder(named: name) }
        case .rename(let path, let name):
            guard let entry = entry(path) else { return }
            Task { await model.rename(entry, to: name) }
        case .delete(let path):
            guard let entry = entry(path) else { return }
            Task { await model.delete(entry) }
        case .refresh:
            Task { await model.load() }
        case .dismiss(.open):
            model.openFailure = nil
        case .dismiss(.edit):
            model.editFailure = nil
        case .prompt(let question):
            guard let surface = page.surface else { return }
            NSHTextPrompt.present(question, from: surface) { [weak page] answer in page?.post(answer) }
        }
    }
}

/// What the NSH folder page shows (``RemoteFolderPageState``, the screen's
/// own through the page bridge), each entry said in the system's words.
struct RemoteFolderFacts: Equatable {
    struct Entry: Equatable {
        var path: String
        var name: String
        var dir: Bool
        var video: Bool
        var symbol: String
        var meta: String
        var thumb: Bool
        var duration: String?
        var opening: Double?
    }

    var name = ""
    var path = ""
    var parents: [String] = []
    var pushed = false
    var loading = false
    var failure: String?
    var uploading: Double?
    var media: [Entry] = []
    var rows: [Entry] = []
    var openFailure: String?
    var editFailure: RemoteFolderModel.EditFailure?
    /// The failures' numbers (``RemoteFolderModel/openFailureId``): the
    /// screen says each number once.
    var openFailureId = 0
    var editFailureId = 0

    init() {}

    @MainActor
    init(model: RemoteFolderModel, pushed: Bool) {
        name = model.folder.name
        path = model.folder.path
        parents = RemoteFileEntry.enclosingFolders(of: model.folder.path)
        self.pushed = pushed
        loading = model.loading
        failure = model.failure
        uploading = model.uploading
        let entry = { (e: RemoteFileEntry) in
            Entry(path: e.path, name: e.name, dir: e.isDirectory, video: e.kind == .video,
                  symbol: RemoteFileIcon.symbol(for: e.kind), meta: Self.meta(e),
                  thumb: model.thumbnails[e.path] != nil,
                  duration: model.durations[e.path].map(RemoteVideoCover.label),
                  opening: model.opening[e.path])
        }
        media = model.media.map(entry)
        rows = model.rows.map(entry)
        openFailure = model.openFailure
        editFailure = model.editFailure
        openFailureId = model.openFailureId
        editFailureId = model.editFailureId
    }

    /// When it changed and how big it is.
    static func meta(_ entry: RemoteFileEntry) -> String {
        var parts: [String] = []
        if let modified = entry.modified {
            parts.append(modified.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated)))
        }
        if !entry.isDirectory, let size = entry.size {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file))
        }
        return parts.joined(separator: " · ")
    }

    /// The folder as the page bridge carries it.
    var page: RemoteFolderPageState {
        RemoteFolderPageState(
            name: name, path: path, parents: parents, pushed: pushed, loading: loading, failure: failure,
            uploading: uploading, media: media.map(Self.item), rows: rows.map(Self.item),
            openFailure: openFailure,
            editFailure: editFailure.map { RemoteFolderFailure(title: $0.title, message: $0.message) },
            openFailureId: UInt64(clamping: openFailureId), editFailureId: UInt64(clamping: editFailureId))
    }

    private static func item(_ entry: Entry) -> RemoteFolderItem {
        RemoteFolderItem(path: entry.path, name: entry.name, dir: entry.dir, video: entry.video,
                         symbol: entry.symbol, meta: entry.meta, thumb: entry.thumb,
                         duration: entry.duration, opening: entry.opening)
    }
}

extension RemoteFolderModel.EditFailure: Equatable {
    static func == (a: Self, b: Self) -> Bool { a.title == b.title && a.message == b.message }
}

/// What the NSH folder page was last told, and its long-press menus.
@MainActor
final class RemoteFolderNSHModel {
    private(set) weak var surface: NSHSurface?
    private var facts: RemoteFolderFacts?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let facts { surface.post(.remoteFolder(state: facts.page)) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's folder as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current facts: RemoteFolderFacts) {
        if self.facts == nil {
            self.surface = nil
            update(facts)
        }
        attach(surface)
    }

    func update(_ facts: RemoteFolderFacts) {
        guard facts != self.facts else { return }
        self.facts = facts
        surface?.post(.remoteFolder(state: facts.page))
    }

    @discardableResult
    func post(_ message: PageMessage) -> Bool {
        surface?.post(message) ?? false
    }

    /// A tile's or a row's menu, by its
    /// identifier: Open and Share for a file, Rename, Delete. A pick goes
    /// back to the page (`perform(action, path)`), which asks where it asks.
    static func menus(for facts: RemoteFolderFacts,
                      perform: @escaping (RemoteFolderAction, String) -> Void) -> [String: () -> [UIMenuElement]] {
        var menus: [String: () -> [UIMenuElement]] = [:]
        for entry in facts.media + facts.rows {
            let path = entry.path
            let dir = entry.dir
            menus["remote-folder-item-\(path)"] = {
                var items: [UIMenuElement] = []
                if !dir {
                    items.append(UIAction(title: String(localized: "Open"), image: UIImage(systemName: "eye")) { _ in
                        perform(.open, path)
                    })
                    items.append(UIAction(title: String(localized: "Share"),
                                          image: UIImage(systemName: "square.and.arrow.up")) { _ in
                        perform(.share, path)
                    })
                }
                items.append(UIAction(title: String(localized: "Rename"), image: UIImage(systemName: "pencil")) { _ in
                    perform(.rename, path)
                })
                items.append(UIAction(title: String(localized: "Delete"), image: UIImage(systemName: "trash"),
                                      attributes: .destructive) { _ in
                    perform(.delete, path)
                })
                return items
            }
        }
        return menus
    }
}

/// `moshpit/remote-thumbnail`: a file's thumbnail, cropped to fill its box
/// (`.scaledToFill()` under a clip).
@MainActor
private final class RemoteThumbnailComponent: NSHComponent {
    private let imageView = UIImageView()
    private let lookup: (String) -> UIImage?
    var view: UIView { imageView }

    init(lookup: @escaping (String) -> UIImage?) {
        self.lookup = lookup
        imageView.contentMode = .scaleAspectFill
        imageView.clipsToBounds = true
        imageView.isUserInteractionEnabled = false
    }

    func setProp(_ name: String, _ value: Any?) {
        guard name == "path", let path = value as? String else { return }
        imageView.image = lookup(path)
    }
}
