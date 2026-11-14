import CryptoKit
import ImageIO
import PhotosUI
import QuickLook
import SwiftUI
import UIKit

// MARK: - Reaching the host

/// How a preview reaches the host: the session's file-transfer SSH (the one
/// uploads ride — already authenticated, no Face ID), dialled on demand when
/// the session has none; on the Rust core, the session's own connection,
/// lent (W14b).
struct RemoteFileAccess {
    /// Keeps one connection's downloads apart from another's in the cache.
    let scope: String
    let channel: () async throws -> any HostConnection
}

// MARK: - Local copies

/// Downloaded previews live in the temp directory, which iOS reclaims on its
/// own. One folder per (connection, path, size, mtime): a chart the agent
/// regenerated is fetched fresh, reopening an unchanged one is instant, and
/// the file keeps its own name so QuickLook titles and types it correctly.
enum RemotePreviewCache {
    static var root: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("RemotePreview", isDirectory: true)
    }

    static func localURL(for entry: RemoteFileEntry, scope: String) -> URL {
        let stamp = "\(scope)\n\(entry.path)\n\(entry.size ?? 0)\n\(entry.modified?.timeIntervalSince1970 ?? 0)"
        let digest = SHA256.hash(data: Data(stamp.utf8)).prefix(10)
            .map { String(format: "%02x", $0) }.joined()
        return root.appendingPathComponent(digest, isDirectory: true).appendingPathComponent(entry.name)
    }

    /// A copy known to match the host's file. Without a size or a time (the
    /// exec fallback's stat has no time) staleness can't be told, so those
    /// are always fetched again.
    /// A copy whose length isn't the host file's is not a match either: a
    /// download that stopped short must never be served as the picture.
    static func cached(_ entry: RemoteFileEntry, scope: String) -> URL? {
        _ = swept
        guard entry.modified != nil else { return nil }
        let url = localURL(for: entry, scope: scope)
        guard let length = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? UInt64
        else { return nil }
        if let size = entry.size, size != length { return nil }
        return url
    }

    /// Fetch `entry` (or reuse a matching copy). Written under a name of
    /// its own beside the final one, then renamed into place: a cancelled
    /// download never passes for a cached one, and leaves nothing behind.
    ///
    /// Fix (B461): every fetch of a file shared one `.partial` and moved it
    /// over the final name. A second fetch of the same file while the first
    /// was still landing — the tap on a folder tile whose thumbnail is
    /// loading — cut the first's bytes off (the core creates its destination
    /// afresh), and whichever finished last deleted the copy the other had
    /// just handed out, then failed its own move: the tap said the host
    /// couldn't be reached, and the tile opened to nothing after that. Each
    /// fetch now downloads under its own name, and `rename(2)` puts it over
    /// the final one in one step, so that name never points at nothing or
    /// at half a file.
    static func fetch(_ entry: RemoteFileEntry, files: RemoteFileAccess,
                      progress: @escaping @Sendable (Double) -> Void) async throws -> URL {
        if let hit = cached(entry, scope: files.scope) { return hit }
        let final = localURL(for: entry, scope: files.scope)
        let fm = FileManager.default
        let folder = final.deletingLastPathComponent()
        try fm.createDirectory(at: folder, withIntermediateDirectories: true)
        let partial = folder.appendingPathComponent(".partial-\(UUID().uuidString)")
        do {
            try await files.channel().downloadRemoteFile(entry.path, to: partial, progress: progress)
            guard Darwin.rename(partial.path, final.path) == 0 else {
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
        } catch {
            try? fm.removeItem(at: partial)
            throw error
        }
        return final
    }

    /// Drop copies older than a day, once per run — the temp directory is
    /// reclaimed eventually, but a day of agent charts is plenty to keep.
    /// Done before the first copy is handed out (`cached`), never after: a
    /// copy served earlier in the run — on screen, in the share sheet — is
    /// never deleted under it (B461).
    private static let swept: Void = {
        let fm = FileManager.default
        let cutoff = Date().addingTimeInterval(-24 * 3600)
        let folders = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for folder in folders {
            let modified = (try? folder.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified < cutoff { try? fm.removeItem(at: folder) }
        }
    }()

    /// A small thumbnail straight from the file, without decoding it whole.
    static func thumbnail(at url: URL, maxPixels: Int = 320) -> UIImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: image)
    }

    static func message(for error: Error) -> String {
        let underlying = (error as? SSHError).flatMap { error -> Error? in
            if case .underlying(let inner) = error { return inner }
            return nil
        }
        // The Rust session's lent connection says what happened in the
        // app's words: reconnecting, not connected, not answering, denied.
        if let lent = error as? HostConnectionError, let words = lent.errorDescription {
            return words
        }
        switch underlying as? SSHSession.RemoteEditError {
        case .nameTaken?:
            return String(localized: "Something with that name is already there.")
        case .failed?:
            return String(localized: "The host didn't allow it. Check that you can write to this folder.")
        case nil:
            break
        }
        switch underlying as? SSHSession.RemoteReadError {
        case .notFound?:
            return String(localized: "This file is no longer on the host.")
        case .denied?:
            return HostConnectionError.readDenied.errorDescription
                ?? String(localized: "The host doesn't let this account read it.")
        case .tooLargeWithoutSFTP?:
            return String(localized: "This host has SFTP turned off, and the file is too large to send without it.")
        case nil:
            return String(localized: "Couldn't reach the host. Check the connection and try again.")
        }
    }
}

// MARK: - One tapped path

/// What a tap on a host path is doing: finding out what it is, fetching a
/// file (or asking first, if it's big), and handing the copy to QuickLook —
/// or, for a folder, handing over to the folder sheet.
@Observable
@MainActor
final class RemotePreviewModel: Identifiable {
    enum Phase: Equatable {
        case looking
        case confirm(RemoteFileEntry)
        case downloading(RemoteFileEntry, Double)
        case ready(URL)
        case folder(RemoteFileEntry)
        /// A video opened for streaming: the player takes it from here.
        case stream(RemoteFileEntry, RemoteFileHandle)
        case failed(String)

        static func == (a: Phase, b: Phase) -> Bool {
            switch (a, b) {
            case (.looking, .looking): return true
            case let (.confirm(x), .confirm(y)): return x == y
            case let (.downloading(x, p), .downloading(y, q)): return x == y && p == q
            case let (.ready(x), .ready(y)): return x == y
            case let (.folder(x), .folder(y)): return x == y
            case let (.stream(x, f), .stream(y, g)): return x == y && f === g
            case let (.failed(x), .failed(y)): return x == y
            default: return false
            }
        }
    }

    /// Past this, a file is only fetched once the user says so.
    static let confirmAbove: UInt64 = 50 * 1024 * 1024

    let id = UUID()
    /// Empty until `locate` has said where to look.
    private(set) var path: String
    let files: RemoteFileAccess
    private(set) var phase: Phase = .looking
    @ObservationIgnored private var work: Task<Void, Never>?
    /// Finds the path first — the Files key's working directory.
    @ObservationIgnored private let locate: (() async throws -> String)?

    var name: String {
        path.isEmpty ? String(localized: "Browse files") : (path as NSString).lastPathComponent
    }

    /// The card shows while there's something to wait for or answer; a
    /// fetched file belongs to QuickLook and a folder to its sheet.
    var showsHUD: Bool {
        switch phase {
        case .ready, .folder, .stream: return false
        default: return true
        }
    }

    init(path: String, files: RemoteFileAccess) {
        self.path = path
        self.files = files
        self.locate = nil
    }

    /// A path found only once started (`locate`), shown as "Browse files"
    /// until then.
    init(files: RemoteFileAccess, locate: @escaping () async throws -> String) {
        self.path = ""
        self.files = files
        self.locate = locate
    }

    func start() {
        work = Task { [weak self] in
            guard let self else { return }
            do {
                if let locate = self.locate { self.path = try await locate() }
                let channel = try await self.files.channel()
                let entry = try await channel.remoteStat(self.path)
                if entry.isDirectory {
                    self.phase = .folder(entry)
                } else if entry.kind == .video, let source = try? await channel.openRemoteFile(entry.path) {
                    // Streamed whatever its size — nothing to wait for or
                    // confirm. A host without SFTP downloads it below.
                    try Task.checkCancellation()
                    self.phase = .stream(entry, source)
                } else if (entry.size ?? 0) > Self.confirmAbove {
                    self.phase = .confirm(entry)
                } else {
                    try await self.download(entry)
                }
            } catch is CancellationError {
            } catch {
                self.phase = .failed(RemotePreviewCache.message(for: error))
            }
        }
    }

    func confirm() {
        guard case .confirm(let entry) = phase else { return }
        work = Task { [weak self] in
            guard let self else { return }
            do {
                try await self.download(entry)
            } catch is CancellationError {
            } catch {
                self.phase = .failed(RemotePreviewCache.message(for: error))
            }
        }
    }

    func cancel() {
        work?.cancel()
        work = nil
    }

    private func download(_ entry: RemoteFileEntry) async throws {
        phase = .downloading(entry, 0)
        let url = try await RemotePreviewCache.fetch(entry, files: files) { [weak self] fraction in
            Task { @MainActor in
                guard let self, case .downloading(let current, _) = self.phase else { return }
                self.phase = .downloading(current, fraction)
            }
        }
        try Task.checkCancellation()
        phase = .ready(url)
    }
}

/// The card that rides above the shortcut bar while a tapped path is being
/// looked up or fetched — the image-attachment overlay's shape, so the two
/// read as the same kind of thing.
struct RemotePreviewHUD: View {
    let model: RemotePreviewModel
    let onDismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            HStack(spacing: 10) {
                Image(systemName: RemoteFileIcon.symbol(for: PreviewableFile.kind(of: model.name)))
                    .font(.system(size: 15))
                    .foregroundStyle(Ink.secondary)
                    .frame(width: 22)
                Text(model.name)
                    .font(Face.text(13, .medium))
                    .foregroundStyle(Ink.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            if case .failed(let message) = model.phase {
                Text(message)
                    .font(Face.text(13))
                    .foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if case .confirm(let entry) = model.phase {
                Text("This file is \(Self.size(entry.size)). Download it to preview?")
                    .font(Face.text(13))
                    .foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            actions
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
        .background(Ink.modalBG.opacity(0.97), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1))
        .padding(.horizontal, 8)
        .padding(.bottom, 6)
        .accessibilityIdentifier("remote-preview-hud")
    }

    @ViewBuilder
    private var header: some View {
        HStack(spacing: 8) {
            switch model.phase {
            case .looking, .ready, .folder, .stream:
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "OPENING…"), tint: Ink.secondary)
            case .downloading(_, let fraction):
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "DOWNLOADING \(Int(fraction * 100))%"), tint: Ink.accent)
            case .confirm:
                Image(systemName: "arrow.down.circle")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.accent)
                statusLabel(String(localized: "LARGE FILE"), tint: Ink.accent)
            case .failed:
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.warn)
                statusLabel(String(localized: "CAN'T OPEN"), tint: Ink.warn)
            }
            Spacer(minLength: 0)
            if case .downloading(let entry, _) = model.phase, entry.size != nil {
                Text(Self.size(entry.size))
                    .font(Face.mono(9, .medium))
                    .foregroundStyle(Ink.tertiary)
            }
        }
    }

    @ViewBuilder
    private var actions: some View {
        HStack(spacing: 8) {
            Spacer(minLength: 0)
            switch model.phase {
            case .confirm:
                Button("Cancel") { onDismiss() }
                    .buttonStyle(.bordered)
                Button("Download") {
                    Haptics.tap()
                    model.confirm()
                }
                .buttonStyle(.borderedProminent)
                .tint(Ink.accent)
            case .failed:
                Button("OK") { onDismiss() }
                    .buttonStyle(.bordered)
            default:
                Button("Cancel") { onDismiss() }
                    .buttonStyle(.bordered)
            }
        }
        .font(Face.text(13, .semibold))
        .controlSize(.small)
    }

    private func statusLabel(_ text: String, tint: Color) -> some View {
        Text(text)
            .font(Face.mono(10, .semibold))
            .kerning(1.2)
            .foregroundStyle(tint)
            .lineLimit(1)
    }

    static func size(_ bytes: UInt64?) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes ?? 0), countStyle: .file)
    }
}

enum RemoteFileIcon {
    static func symbol(for kind: PreviewableFile.Kind?) -> String {
        switch kind {
        case .image?: return "photo"
        case .video?: return "film"
        case .audio?: return "waveform"
        case .pdf?: return "doc.richtext"
        case .document?: return "doc"
        case .text?: return "doc.text"
        case nil: return "folder"
        }
    }
}

// MARK: - A folder

/// One folder on the host, as the folder sheet shows it: images and video
/// as a thumbnail grid on top, subfolders and the other files as a list
/// below, each newest first, thumbnails filled in as they arrive. Opening a
/// file fetches it and hands it to QuickLook together with every file
/// already fetched here, so swiping moves between them.
///
/// The folder's rules are the shared model's (`crates/app/src/remote_folder.rs`,
/// ``RemoteFolderHandle``, the one Android runs): what it shows, which
/// thumbnails to fetch, names checked before the round trip, progress taken
/// only while under way, each failure numbered (B286). This class does the
/// I/O and the platform's part (the connection, thumbnails, QuickLook, the
/// share sheet, the pickers), tells the model what happened, and mirrors its
/// state for the views to observe.
@Observable
@MainActor
final class RemoteFolderModel {
    let folder: RemoteFileEntry
    let files: RemoteFileAccess
    @ObservationIgnored private let rules: RemoteFolderHandle
    /// What the folder shows, newest first (the model's).
    private(set) var entries: [RemoteFileEntry] = []
    /// The thumbnail grid: images and video, newest first.
    private(set) var media: [RemoteFileEntry] = []
    /// The list under the grid: subfolders, then every other file.
    private(set) var rows: [RemoteFileEntry] = []
    private(set) var loading = true
    private(set) var failure: String?
    private(set) var thumbnails: [String: UIImage] = [:]
    /// Running times of the videos whose covers are in, by path.
    private(set) var durations: [String: Double] = [:]
    /// Files the user opened that are still on their way, by path.
    private(set) var opening: [String: Double] = [:]
    /// Copies fetched so far (thumbnails included), by path.
    private(set) var local: [String: URL] = [:]
    var previewURL: URL?
    /// An upload under way, as a fraction — shown above the grid.
    private(set) var uploading: Double?
    private var openFailureState: FolderFailureRecord?
    private var editFailureState: FolderFailureRecord?

    /// Why the file just tapped couldn't be opened (the grid stays up).
    var openFailure: String? {
        get { openFailureState?.message }
        set {
            rules.setOpenFailure(message: newValue)
            sync()
        }
    }

    /// The number of the failure in ``openFailure``: new for each one set
    /// anew and never reused in this process, so the NSH sheet says the same
    /// words failing again (B286); 0 when none stands.
    var openFailureId: Int { Int(openFailureState?.id ?? 0) }

    /// A create / rename / delete / upload that didn't go through.
    var editFailure: EditFailure? {
        get { editFailureState.map { EditFailure(title: $0.title, message: $0.message) } }
        set {
            rules.setEditFailure(title: newValue?.title, message: newValue?.message)
            sync()
        }
    }

    /// ``editFailure``'s number, as ``openFailureId`` is ``openFailure``'s.
    var editFailureId: Int { Int(editFailureState?.id ?? 0) }

    struct EditFailure {
        let title: String
        let message: String
    }

    static let thumbnailWorkers = 3

    init(folder: RemoteFileEntry, files: RemoteFileAccess) {
        self.folder = folder
        self.files = files
        rules = RemoteFolderHandle(path: folder.path)
    }

    /// The model's state, mirrored for the views.
    private func sync() {
        let state = rules.state()
        let shown = state.entries.map(RemoteFileEntry.init)
        if shown != entries { entries = shown }
        let grid = state.media.map(RemoteFileEntry.init)
        if grid != media { media = grid }
        let list = state.rows.map(RemoteFileEntry.init)
        if list != rows { rows = list }
        if state.loading != loading { loading = state.loading }
        if state.failure != failure { failure = state.failure }
        if state.opening != opening { opening = state.opening }
        if state.uploading != uploading { uploading = state.uploading }
        if state.openFailure != openFailureState { openFailureState = state.openFailure }
        if state.editFailure != editFailureState { editFailureState = state.editFailure }
    }

    /// The copies QuickLook can swipe between, in the order the sheet shows them.
    var previewItems: [URL] { (media + rows).compactMap { local[$0.path] } }

    func load() async {
        rules.loadStarted()
        sync()
        do {
            let listing = try await files.channel().listRemoteDirectory(folder.path)
            rules.listed(listing: listing.map(RemoteFileRecord.init))
        } catch {
            rules.listFailed(why: RemotePreviewCache.message(for: error))
        }
        sync()
        await fillThumbnails()
    }

    /// Media still waiting for a thumbnail; the workers take from the front.
    @ObservationIgnored private var thumbnailQueue: [RemoteFileEntry] = []

    private func fillThumbnails() async {
        thumbnailQueue = rules.thumbnailQueue().map(RemoteFileEntry.init)
        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<Self.thumbnailWorkers {
                group.addTask { @MainActor [weak self] in
                    while let self, !self.thumbnailQueue.isEmpty, !Task.isCancelled {
                        let entry = self.thumbnailQueue.removeFirst()
                        if entry.kind == .video {
                            // A frame and the running time, streamed — the
                            // file itself stays on the host.
                            guard let cover = await RemoteVideoCover.make(entry, files: self.files) else { continue }
                            self.thumbnails[entry.path] = cover.image
                            self.durations[entry.path] = cover.seconds
                            if let seconds = cover.seconds {
                                self.rules.coverArrived(path: entry.path, seconds: seconds)
                            } else {
                                self.rules.thumbnailArrived(path: entry.path)
                            }
                            continue
                        }
                        guard let url = try? await RemotePreviewCache.fetch(entry, files: self.files,
                                                                            progress: { _ in }) else { continue }
                        self.local[entry.path] = url
                        self.thumbnails[entry.path] = RemotePreviewCache.thumbnail(at: url)
                        self.rules.thumbnailArrived(path: entry.path)
                    }
                }
            }
        }
    }

    func open(_ entry: RemoteFileEntry) async {
        if let url = local[entry.path] {
            previewURL = url
            return
        }
        if entry.kind == .video {
            rules.openStarted(path: entry.path)
            sync()
            let source = try? await files.channel().openRemoteFile(entry.path)
            rules.openFinished(path: entry.path)
            sync()
            if let source {
                // Streamed in the system player; a host without SFTP falls
                // through to the download.
                RemoteVideoPlayer.present(entry, source: source, files: files, onClose: {})
                return
            }
        }
        if let url = await localCopy(of: entry) { previewURL = url }
    }

    /// Fetches `entry` (or reuses the copy already here) and hands it to the
    /// share sheet — another app, or another person. A video is downloaded
    /// whole for this: a share needs the file, not a stream.
    func share(_ entry: RemoteFileEntry) async {
        guard let url = await localCopy(of: entry) else { return }
        RemoteShareSheet.present(url)
    }

    /// The phone-side copy of `entry`, fetched with its progress on the tile;
    /// nil (and the failure shown) when it couldn't be.
    private func localCopy(of entry: RemoteFileEntry) async -> URL? {
        if let url = local[entry.path] { return url }
        rules.openStarted(path: entry.path)
        sync()
        defer {
            rules.openFinished(path: entry.path)
            sync()
        }
        do {
            let url = try await RemotePreviewCache.fetch(entry, files: files) { [weak self] fraction in
                Task { @MainActor in
                    // The model drops it once the fetch is over.
                    self?.rules.openProgress(path: entry.path, fraction: fraction)
                    self?.sync()
                }
            }
            local[entry.path] = url
            return url
        } catch is CancellationError {
        } catch {
            openFailure = RemotePreviewCache.message(for: error)
        }
        return nil
    }

    // MARK: Changing the folder

    /// Files picked in the Files app, uploaded under their own names (a
    /// taken name gets Finder's " 2").
    func upload(files urls: [URL]) async {
        for url in urls {
            let scoped = url.startAccessingSecurityScopedResource()
            await upload(local: url, named: url.lastPathComponent)
            if scoped { url.stopAccessingSecurityScopedResource() }
        }
        await load()
    }

    /// Photos go through the same pipeline as an attached image — re-encoded
    /// so no location or other metadata leaves the phone, and named
    /// `IMG-<time>-<rand>` rather than after the library entry.
    func upload(photos items: [PhotosPickerItem]) async {
        for item in items {
            guard let raw = try? await item.loadTransferable(type: Data.self),
                  let processed = await Task.detached(operation: { try? ImageAttachmentPipeline.process(raw) }).value
            else {
                editFailure = EditFailure(title: String(localized: "Couldn't upload"),
                                          message: String(localized: "Couldn't read that photo."))
                continue
            }
            let staged = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            do {
                try processed.data.write(to: staged)
                await upload(local: staged, named: processed.filename)
            } catch {
                editFailure = EditFailure(title: String(localized: "Couldn't upload"),
                                          message: String(localized: "Couldn't read that photo."))
            }
            try? FileManager.default.removeItem(at: staged)
        }
        await load()
    }

    private func upload(local: URL, named name: String) async {
        // Never over a name there, nor one just sent (the model keeps them).
        let target = rules.uploadTarget(name: name)
        rules.uploadStarted()
        sync()
        defer {
            rules.uploadFinished()
            sync()
        }
        do {
            try await files.channel().uploadRemoteFile(from: local, to: target) { [weak self] fraction in
                Task { @MainActor in
                    self?.rules.uploadProgress(fraction: fraction)
                    self?.sync()
                }
            }
        } catch is CancellationError {
        } catch {
            editFailure = EditFailure(title: String(localized: "Couldn't upload"),
                                      message: RemotePreviewCache.message(for: error))
        }
    }

    func createFolder(named raw: String) async {
        let title = String(localized: "Couldn't create the folder")
        guard let path = target(rules.newFolderTarget(raw: raw), title: title) else { return }
        await change(title: title) { try await $0.createRemoteDirectory(path) }
    }

    func rename(_ entry: RemoteFileEntry, to raw: String) async {
        let title = String(localized: "Couldn't rename")
        guard let path = target(rules.renameTarget(entry: RemoteFileRecord(entry), raw: raw), title: title)
        else { return }
        await change(title: title) { try await $0.renameRemoteItem(entry.path, to: path) }
    }

    func delete(_ entry: RemoteFileEntry) async {
        await change(title: String(localized: "Couldn't delete")) {
            try await $0.deleteRemoteItem(entry)
        }
    }

    /// Where a typed name goes: nil when the model refused it — said when the
    /// name is taken, before the round trip; nothing to do otherwise.
    private func target(_ target: FolderNameTarget, title: String) -> String? {
        switch target {
        case .path(let path):
            return path
        case .refused(.taken):
            editFailure = EditFailure(title: title,
                                      message: String(localized: "Something with that name is already there."))
            return nil
        case .refused(.ignored):
            return nil
        }
    }

    /// Run one change against the host, then re-list.
    private func change(title: String, _ work: @escaping (any HostConnection) async throws -> Void) async {
        do {
            try await work(files.channel())
        } catch is CancellationError {
            return
        } catch {
            editFailure = EditFailure(title: title, message: RemotePreviewCache.message(for: error))
        }
        await load()
    }
}

/// The sheet a tapped folder path (or the Files key) opens.
struct RemoteFolderSheet: View {
    let files: RemoteFileAccess
    /// Where the stack starts — the folder it opened on, or one the title
    /// menu climbed out to.
    @State private var root: RemoteFileEntry
    @State private var trail: [RemoteFileEntry] = []
    /// Half height first: a swipe down closes it from there, and the
    /// terminal stays in view. Full height is a drag up.
    @State private var detent: PresentationDetent = .medium

    init(folder: RemoteFileEntry, files: RemoteFileAccess) {
        self.files = files
        _root = State(initialValue: folder)
    }

    var body: some View {
        NavigationStack(path: $trail) {
            RemoteFolderView(folder: root, files: files, onClimb: climb)
                .id(root.path)
                .navigationDestination(for: RemoteFileEntry.self) { entry in
                    RemoteFolderView(folder: entry, files: files, onClimb: climb)
                }
        }
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .sheetSurface()
        // Pictures and films are seen on dark, whatever the app's appearance
        // (R18: the media preview stays dark).
        .preferredColorScheme(.dark)
        .accessibilityIdentifier("remote-folder-sheet")
    }

    /// Start over at an enclosing folder picked from the title menu.
    private func climb(to path: String) {
        trail = []
        root = RemoteFileEntry(path: path, isDirectory: true, size: nil, modified: nil)
    }
}

struct RemoteFolderView: View {
    @State private var model: RemoteFolderModel
    @State private var showPhotos = false
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var showFiles = false
    @State private var showNewFolder = false
    @State private var newFolderName = ""
    @State private var renameTarget: RemoteFileEntry?
    @State private var renameDraft = ""
    @State private var deleteTarget: RemoteFileEntry?

    /// Re-roots the sheet at an enclosing folder.
    let onClimb: (String) -> Void

    init(folder: RemoteFileEntry, files: RemoteFileAccess, onClimb: @escaping (String) -> Void) {
        _model = State(initialValue: RemoteFolderModel(folder: folder, files: files))
        self.onClimb = onClimb
    }

    private let columns = [GridItem(.adaptive(minimum: 100, maximum: 180), spacing: 10)]

    var body: some View {
        @Bindable var model = model
        ScrollView {
            if let fraction = model.uploading {
                uploadStrip(fraction)
                    .padding(.horizontal, 14)
                    .padding(.top, 10)
            }
            if model.loading && model.entries.isEmpty {
                ProgressView()
                    .tint(Ink.accent)
                    .padding(.top, 40)
            } else if let failure = model.failure, model.entries.isEmpty {
                emptyState(title: String(localized: "CAN'T OPEN"), detail: failure)
            } else if model.entries.isEmpty {
                emptyState(title: String(localized: "NOTHING TO PREVIEW"),
                           detail: String(localized: "Only images, video, audio, PDFs and documents are shown here."))
            } else {
                VStack(alignment: .leading, spacing: 18) {
                    if !model.media.isEmpty {
                        LazyVGrid(columns: columns, spacing: 14) {
                            ForEach(model.media) { mediaTile($0) }
                        }
                    }
                    if !model.rows.isEmpty {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(model.rows.enumerated()), id: \.element.id) { index, entry in
                                if index > 0 {
                                    Rectangle().fill(Ink.cardDivider).frame(height: 1).padding(.leading, 58)
                                }
                                row(entry)
                            }
                        }
                        .background(Ink.groupRaised, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .strokeBorder(Ink.cardBorder, lineWidth: 1))
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
            }
        }
        .navigationTitle(model.folder.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { addMenu }
        }
        .toolbarTitleMenu { climbMenu }
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
        .alert("New Folder", isPresented: $showNewFolder) {
            TextField("Name", text: $newFolderName)
            Button("Cancel", role: .cancel) {}
            Button("Create") {
                let name = newFolderName
                Task { await model.createFolder(named: name) }
            }
        }
        .confirmationDialog(Text("Delete \(deleteTarget?.name ?? "")?"), isPresented: Binding(
            get: { deleteTarget != nil }, set: { if !$0 { deleteTarget = nil } }
        ), titleVisibility: .visible, presenting: deleteTarget) { entry in
            Button("Delete", role: .destructive) {
                Task { await model.delete(entry) }
            }
            Button("Cancel", role: .cancel) {}
        } message: { entry in
            if entry.isDirectory {
                Text("“\(entry.name)” and everything in it will be deleted from the host.")
            } else {
                Text("“\(entry.name)” will be deleted from the host.")
            }
        }
        .task { await model.load() }
        .refreshable { await model.load() }
        .quickLookPreview($model.previewURL, in: model.previewItems)
        .alert(String(localized: "CAN'T OPEN"), isPresented: Binding(
            get: { model.openFailure != nil }, set: { if !$0 { model.openFailure = nil } }
        )) {
            Button("OK", role: .cancel) { model.openFailure = nil }
        } message: {
            Text(model.openFailure ?? "")
        }
        .background {
            // Separate hosts for the remaining alerts: one view, one alert.
            Color.clear
                .alert("Rename", isPresented: Binding(
                    get: { renameTarget != nil }, set: { if !$0 { renameTarget = nil } }
                )) {
                    TextField("Name", text: $renameDraft)
                    Button("Cancel", role: .cancel) { renameTarget = nil }
                    Button("Rename") {
                        if let target = renameTarget {
                            let name = renameDraft
                            Task { await model.rename(target, to: name) }
                        }
                        renameTarget = nil
                    }
                }
            Color.clear
                .alert(model.editFailure?.title ?? "", isPresented: Binding(
                    get: { model.editFailure != nil }, set: { if !$0 { model.editFailure = nil } }
                )) {
                    Button("OK", role: .cancel) { model.editFailure = nil }
                } message: {
                    Text(model.editFailure?.message ?? "")
                }
        }
    }

    /// The + in the corner: what can be added to this folder.
    private var addMenu: some View {
        Menu {
            Button { showPhotos = true } label: {
                Label("Upload Photos", systemImage: "photo.on.rectangle")
            }
            Button { showFiles = true } label: {
                Label("Upload Files", systemImage: "doc.badge.plus")
            }
            Button {
                newFolderName = ""
                showNewFolder = true
            } label: {
                Label("New Folder", systemImage: "folder.badge.plus")
            }
        } label: {
            Label("Add", systemImage: "plus")
                .labelStyle(.iconOnly)
                .foregroundStyle(Ink.accent)
        }
        .disabled(model.uploading != nil)
        .accessibilityIdentifier("remote-folder-add")
    }

    /// The title's menu: the full path, and every folder above it — the
    /// way out of a working directory when the files are somewhere else.
    @ViewBuilder
    private var climbMenu: some View {
        Section(model.folder.path) {
            ForEach(RemoteFileEntry.enclosingFolders(of: model.folder.path), id: \.self) { path in
                Button { onClimb(path) } label: {
                    Label(path == "/" ? "/" : (path as NSString).lastPathComponent, systemImage: "folder")
                }
            }
        }
    }

    /// The long-press menu on a tile or a row.
    @ViewBuilder
    private func actions(for entry: RemoteFileEntry) -> some View {
        if !entry.isDirectory {
            Button {
                Task { await model.open(entry) }
            } label: {
                Label("Open", systemImage: "eye")
            }
            Button {
                Task { await model.share(entry) }
            } label: {
                Label("Share", systemImage: "square.and.arrow.up")
            }
        }
        Button {
            renameDraft = entry.name
            renameTarget = entry
        } label: {
            Label("Rename", systemImage: "pencil")
        }
        Button(role: .destructive) {
            deleteTarget = entry
        } label: {
            Label("Delete", systemImage: "trash")
        }
    }

    private func uploadStrip(_ fraction: Double) -> some View {
        HStack(spacing: 10) {
            Text(String(localized: "UPLOADING \(Int(fraction * 100))%"))
                .font(Face.mono(10, .semibold))
                .kerning(1.2)
                .foregroundStyle(Ink.accent)
                .fixedSize()
            ProgressView(value: fraction)
                .tint(Ink.accent)
        }
        .padding(.horizontal, 2)
    }

    /// An image or video: a square thumbnail, cropped to fill — the base
    /// shape sets the size and the picture only overlays it, so a wide image
    /// can never push the tile (or its neighbours) wider.
    private func mediaTile(_ entry: RemoteFileEntry) -> some View {
        Button {
            Haptics.tap()
            Task { await model.open(entry) }
        } label: {
            VStack(alignment: .leading, spacing: 5) {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Ink.groupRaised)
                    .aspectRatio(1, contentMode: .fit)
                    .overlay {
                        if let image = model.thumbnails[entry.path] {
                            Image(uiImage: image)
                                .resizable()
                                .scaledToFill()
                        } else {
                            Image(systemName: RemoteFileIcon.symbol(for: entry.kind))
                                .font(.system(size: 24, weight: .light))
                                .foregroundStyle(Ink.tertiary)
                        }
                    }
                    .overlay(alignment: .bottomLeading) {
                        if entry.kind == .video {
                            Image(systemName: "play.fill")
                                .font(.system(size: 10, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(6)
                                .background(.black.opacity(0.55), in: Circle())
                                .padding(6)
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if let seconds = model.durations[entry.path] {
                            Text(RemoteVideoCover.label(seconds))
                                .font(Face.mono(10, .semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 5)
                                .padding(.vertical, 2)
                                .background(.black.opacity(0.55), in: Capsule())
                                .padding(6)
                        }
                    }
                    .overlay {
                        if let fraction = model.opening[entry.path] {
                            ZStack {
                                Color.black.opacity(0.45)
                                ProgressView(value: fraction)
                                    .progressViewStyle(.circular)
                                    .tint(Ink.accent)
                            }
                        }
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .strokeBorder(Ink.cardBorder, lineWidth: 1))
                Text(entry.name)
                    .font(Face.text(12, .medium))
                    .foregroundStyle(Ink.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(meta(entry))
                    .font(Face.mono(9))
                    .foregroundStyle(Ink.tertiary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { actions(for: entry) }
        .accessibilityElement(children: .combine)
    }

    /// A subfolder or a document: one compact row. A folder opens in place;
    /// a file downloads (progress in its icon) and opens in QuickLook.
    @ViewBuilder
    private func row(_ entry: RemoteFileEntry) -> some View {
        Group {
            if entry.isDirectory {
                NavigationLink(value: entry) { rowBody(entry) }
                    .buttonStyle(.plain)
            } else {
                Button {
                    Haptics.tap()
                    Task { await model.open(entry) }
                } label: { rowBody(entry) }
                .buttonStyle(.plain)
            }
        }
        .contextMenu { actions(for: entry) }
    }

    private func rowBody(_ entry: RemoteFileEntry) -> some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Ink.neutralFill)
                if let fraction = model.opening[entry.path] {
                    ProgressView(value: fraction)
                        .progressViewStyle(.circular)
                        .controlSize(.small)
                        .tint(Ink.accent)
                } else {
                    Image(systemName: RemoteFileIcon.symbol(for: entry.kind))
                        .font(.system(size: 15, weight: .regular))
                        .foregroundStyle(entry.isDirectory ? Ink.accent : Ink.secondary)
                }
            }
            .frame(width: 34, height: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.name)
                    .font(Face.text(14, .medium))
                    .foregroundStyle(Ink.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(meta(entry))
                    .font(Face.mono(9))
                    .foregroundStyle(Ink.tertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            if entry.isDirectory {
                Image(systemName: "chevron.right")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Ink.tertiary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private func meta(_ entry: RemoteFileEntry) -> String {
        var parts: [String] = []
        if let modified = entry.modified {
            parts.append(modified.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated)))
        }
        if !entry.isDirectory, let size = entry.size {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file))
        }
        return parts.joined(separator: " · ")
    }

    private func emptyState(title: String, detail: String) -> some View {
        VStack(spacing: 8) {
            Text(title)
                .font(Face.mono(10, .semibold))
                .kerning(1.2)
                .foregroundStyle(Ink.tertiary)
            Text(detail)
                .font(Face.text(13))
                .foregroundStyle(Ink.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 32)
        .padding(.top, 48)
        .frame(maxWidth: .infinity)
    }
}
