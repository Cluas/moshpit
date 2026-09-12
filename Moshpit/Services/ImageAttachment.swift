import Foundation
import ImageIO
import Observation
import UIKit
import UniformTypeIdentifiers

// MARK: - Pipeline (pure, testable)

/// Prepares a picked image for the remote host: re-encode, downscale, strip
/// metadata, name it. Pure functions over bytes — the tests feed fixture
/// images straight in.
enum ImageAttachmentPipeline {

    /// Long side above which an image is downscaled. Screenshots carry text,
    /// so this stays generous: 2048 keeps UI text legible to an agent while
    /// remaining cellular-friendly (Claude downsamples past ~1568 anyway, so
    /// anything bigger is bytes the model never sees).
    static let maxPixelSize = 2048
    static let jpegQuality: CGFloat = 0.85

    struct Processed {
        let data: Data
        let filename: String
        /// Post-processing pixel size, for tests and the overlay caption.
        let pixelWidth: Int
        let pixelHeight: Int
    }

    enum PipelineError: Error {
        case undecodable
        case encodingFailed
    }

    /// Re-encode `raw` (HEIC/PNG/JPEG/…) for upload.
    ///
    /// Everything goes through one decode → thumbnail → encode pass, even
    /// images already small enough: the re-encode is what guarantees EXIF/GPS
    /// never leaves the phone (no metadata is copied to the destination), and
    /// the thumbnail's `WithTransform` bakes the EXIF orientation into the
    /// pixels so stripping it can't sideways a photo. Alpha keeps PNG;
    /// everything else becomes JPEG.
    ///
    /// `date`/`suffix` are injectable so tests get deterministic names.
    static func process(_ raw: Data,
                        date: Date = Date(),
                        suffix: String = Self.randomSuffix()) throws -> Processed {
        guard let source = CGImageSourceCreateWithData(raw as CFData, nil) else {
            throw PipelineError.undecodable
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            throw PipelineError.undecodable
        }

        // An alpha CHANNEL is not transparency: simulator screenshots and
        // some camera pipelines hand over fully-opaque RGBA, and treating
        // the channel as the signal shipped a 4.6MB PNG where an ~800KB
        // JPEG carried the same pixels. Only images with at least one
        // actually-transparent pixel keep PNG.
        let hasAlpha: Bool
        switch image.alphaInfo {
        case .none, .noneSkipFirst, .noneSkipLast:
            hasAlpha = false
        default:
            hasAlpha = hasTransparentPixels(image)
        }

        let type: UTType = hasAlpha ? .png : .jpeg
        let encoded = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            encoded, type.identifier as CFString, 1, nil) else {
            throw PipelineError.encodingFailed
        }
        // Only the compression quality — deliberately NO metadata dictionary,
        // which is the strip: EXIF, GPS, maker notes all stay on the phone.
        let encodeOptions: [CFString: Any] = hasAlpha
            ? [:]
            : [kCGImageDestinationLossyCompressionQuality: jpegQuality]
        CGImageDestinationAddImage(destination, image, encodeOptions as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw PipelineError.encodingFailed
        }

        return Processed(
            data: encoded as Data,
            filename: filename(date: date, suffix: suffix, ext: hasAlpha ? "png" : "jpg"),
            pixelWidth: image.width,
            pixelHeight: image.height)
    }

    /// Whether any pixel is actually transparent, decided from the alpha
    /// bytes themselves — the image redrawn into an RGBA context at full
    /// thumbnail resolution (≤2048², a ~16MB pass) so a small transparent
    /// region can't be averaged away by downsampling. Threshold below 250
    /// rather than 255: resampling bleeds a whisker of coverage into edge
    /// pixels of fully-opaque images.
    private static func hasTransparentPixels(_ image: CGImage) -> Bool {
        let width = image.width, height = image.height
        guard width > 0, height > 0,
              let context = CGContext(
                  data: nil, width: width, height: height,
                  bitsPerComponent: 8, bytesPerRow: width * 4,
                  space: CGColorSpaceCreateDeviceRGB(),
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return true }   // can't inspect → keep PNG, the lossless err
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let data = context.data else { return true }
        let pixels = data.bindMemory(to: UInt8.self, capacity: width * height * 4)
        var index = 3   // RGBA — alpha is every fourth byte
        let end = width * height * 4
        while index < end {
            if pixels[index] < 250 { return true }
            index += 4
        }
        return false
    }

    /// `IMG-20260815-142033-a3f2.jpg` — sortable, collision-free, and free of
    /// the local photo-library name (which can carry a place or person).
    static func filename(date: Date, suffix: String, ext: String) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        return "IMG-\(formatter.string(from: date))-\(suffix).\(ext)"
    }

    static func randomSuffix() -> String {
        String(format: "%04x", UInt16.random(in: .min ... .max))
    }

    /// The text pasted at the prompt for uploaded paths.
    ///
    /// The default is a bare path padded with spaces: it reads the same to
    /// every agent, Codex's pasted-image-path detector auto-attaches it
    /// (quotes break that detector, so quoting happens ONLY when a path
    /// actually carries a space or quote — our filenames never do), and `@`
    /// typed blind would trigger Claude Code's completion menu. The other
    /// styles exist because two agent families genuinely differ — see
    /// ``ImageInsertStyle``.
    static func insertText(forRemotePaths paths: [String],
                           style: ImageInsertStyle = .barePath) -> String {
        guard !paths.isEmpty else { return "" }
        switch style {
        case .barePath:
            return " " + paths.map(Self.quotedIfNeeded).joined(separator: " ") + " "
        case .atMention:
            // @-parsers read to whitespace and do NOT strip quotes, so a
            // path that needs quoting can't be @-mentioned at all — those
            // fall back to the shell-safe bare form (the model may still
            // read it; a mis-parsed @-token helps nobody).
            let rendered = paths.map { path in
                needsQuoting(path) ? Self.quotedIfNeeded(path) : "@" + path
            }
            return " " + rendered.joined(separator: " ") + " "
        case .aiderAdd:
            // aider attaches only through its /add command, which must lead
            // the message — no padding. Insert leaves it for review; Send's
            // Return submits it, same trust split as everywhere else.
            return "/add " + paths.map(Self.quotedIfNeeded).joined(separator: " ")
        }
    }

    private static func needsQuoting(_ path: String) -> Bool {
        path.contains(where: { $0 == " " || $0 == "'" || $0 == "\"" })
    }

    private static func quotedIfNeeded(_ path: String) -> String {
        guard needsQuoting(path) else { return path }
        return "'" + path.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

// MARK: - Agent-aware insert format

/// How the active pane's agent best receives an image path — from the
/// 2026-08 cross-agent survey (the design doc carries the full matrix):
///
/// - `barePath`: Claude Code reads it with its Read tool, Codex's TUI
///   auto-attaches a pasted image path as native `[image N]`, Goose has
///   `read_image`. The universal default, also for plain shells.
/// - `atMention`: Gemini CLI and Qwen Code only read files
///   deterministically through `@path` (a bare path is left to the model's
///   discretion there).
/// - `aiderAdd`: aider attaches images ONLY via its `/add` command — bare
///   paths and @-mentions are ignored.
enum ImageInsertStyle: Equatable {
    case barePath
    case atMention
    case aiderAdd

    /// Classify from the pane's agent signal: the hook-stamped
    /// `@moshpit_agent` when Vibe Island hooks are installed, else the
    /// pane's foreground command. nil / unrecognized → the universal
    /// default.
    static func forAgent(_ agent: String?) -> ImageInsertStyle {
        guard var name = agent?.lowercased(), !name.isEmpty else { return .barePath }
        // Claude Code sets a process title that starts with its version —
        // a bare semver comm is claude (same normalization as the Home
        // screen's pane rows).
        if name.range(of: #"^\d+\.\d+\.\d+$"#, options: .regularExpression) != nil {
            name = "claude"
        }
        if name.contains("gemini") || name.contains("qwen") { return .atMention }
        if name.contains("aider") { return .aiderAdd }
        return .barePath
    }
}

// MARK: - Session upload log

/// Numbered record of every image this session has uploaded, so a picture can
/// be re-referenced (#1, #2, …) and re-inserted without re-uploading — the
/// terminal-side equivalent of Claude Code's `[Image #N]` mental model, which
/// itself can't exist over SSH (its placeholder only triggers on a LOCAL
/// clipboard paste; remote reads go through file paths).
struct ImageUploadLog: Equatable {
    struct Entry: Identifiable, Equatable {
        let id: UUID
        /// 1-based session number — stable once assigned.
        let number: Int
        let remotePath: String
        var filename: String { (remotePath as NSString).lastPathComponent }
    }

    private(set) var entries: [Entry] = []

    @discardableResult
    mutating func record(remotePath: String) -> Entry {
        let entry = Entry(id: UUID(), number: entries.count + 1, remotePath: remotePath)
        entries.append(entry)
        return entry
    }
}

// MARK: - Clipboard reading

/// Raw image bytes off the pasteboard, originals preferred over re-encodes.
enum ClipboardImageReader {
    /// Preference order per item: HEIC and PNG are what screenshots and
    /// camera copies actually carry; TIFF is the lowest-fidelity fallback
    /// UIKit synthesizes.
    static let preferredTypes = [UTType.heic, UTType.png, UTType.jpeg, UTType.tiff]
        .map(\.identifier)

    /// One Data per clipboard item that carries an image. Synchronous
    /// pasteboard reads can block for over a second (cross-app paste prompt +
    /// pasteboardd round trip) — call OFF the main thread.
    static func read(from pasteboard: UIPasteboard = .general) -> [Data] {
        var out: [Data] = []
        for index in 0..<pasteboard.numberOfItems {
            let item = IndexSet(integer: index)
            for type in preferredTypes {
                if let data = pasteboard.data(forPasteboardType: type, inItemSet: item)?.first {
                    out.append(data)
                    break
                }
            }
        }
        return out
    }
}

// MARK: - Upload transport seam

/// What the attachment flow needs from a transport: one call that lands bytes
/// in the app's remote uploads directory and returns the absolute path.
/// `SSHSession` provides the real SFTP implementation; tests provide a fake.
protocol FileUploader: Sendable {
    func uploadToUploadsDirectory(
        _ data: Data,
        named filename: String,
        progress: (@Sendable (Double) -> Void)?) async throws -> String
}

extension SSHSession: FileUploader {}

// MARK: - Controller

/// Owns one picker-to-paste attachment flow: process the picked images,
/// upload them over the session's SSH channel, and hand back the paste text.
/// Mirrors `VoiceDictationController`'s compose-then-commit shape — nothing
/// reaches the PTY until the user says Insert or Send.
@MainActor
@Observable
final class ImageAttachmentController {

    enum Phase: Equatable {
        case processing
        /// Overall fraction across every file (bytes-weighted).
        case uploading(Double)
        /// Everything landed; `insertText()` is ready.
        case ready
        case failed(String)
    }

    struct Attachment: Identifiable {
        let id = UUID()
        var thumbnail: UIImage?
        var byteCount = 0
        var remotePath: String?
        /// 1-based number in the session's upload log, assigned at `.ready`.
        var sessionNumber: Int?
    }

    private(set) var phase: Phase = .processing
    private(set) var attachments: [Attachment] = []

    /// Called once when every upload has landed, with the remote paths in
    /// attachment order; returns the session numbers the host assigned
    /// (empty to skip numbering). Lets the session's upload log own the
    /// counter while the overlay still shows #N on each thumbnail.
    @ObservationIgnored var onReady: (([String]) -> [Int])?

    /// Resolves the transport when the flow actually needs it — async
    /// because a pure-mosh session dials an SSH connection on demand (the
    /// session has none to reuse). A throw here lands in `.failed` with the
    /// transport's own user-facing message.
    @ObservationIgnored private let uploaderProvider: () async throws -> any FileUploader
    @ObservationIgnored private var work: Task<Void, Never>?
    /// Processed images held for retry — a failed upload shouldn't re-run
    /// the pipeline, and MUSTN'T re-upload the files that already landed.
    @ObservationIgnored private var processed: [ImageAttachmentPipeline.Processed] = []

    init(uploaderProvider: @escaping () async throws -> any FileUploader) {
        self.uploaderProvider = uploaderProvider
    }

    /// Kick off processing + upload for the picked images. UI state flows
    /// through `phase`; the caller only comes back for `insertText()`.
    func start(pickedData: [Data]) {
        work = Task { [weak self] in
            await self?.run(pickedData: pickedData)
        }
    }

    /// Re-attempt after `.failed`: transport re-acquired (the failure may
    /// have BEEN the transport), pipeline skipped, already-landed files kept.
    func retry() {
        guard case .failed = phase else { return }
        phase = .uploading(0)
        work = Task { [weak self] in
            await self?.upload()
        }
    }

    func cancel() {
        work?.cancel()
    }

    /// The paste payload — nil until every upload landed. `style` follows
    /// the active pane's agent (see `ImageInsertStyle`).
    func insertText(style: ImageInsertStyle = .barePath) -> String? {
        guard phase == .ready else { return nil }
        let paths = attachments.compactMap(\.remotePath)
        guard !paths.isEmpty else { return nil }
        return ImageAttachmentPipeline.insertText(forRemotePaths: paths, style: style)
    }

    private func run(pickedData: [Data]) async {
        do {
            // Process off the main actor — ImageIO decode of a 48MP photo is
            // real work — then upload sequentially so progress reads honestly
            // and a failure names one file, not a pile.
            var items: [ImageAttachmentPipeline.Processed] = []
            for raw in pickedData {
                try Task.checkCancellation()
                let item = try await Task.detached(priority: .userInitiated) {
                    try ImageAttachmentPipeline.process(raw)
                }.value
                items.append(item)
            }
            processed = items
            attachments = items.map { item in
                Attachment(thumbnail: UIImage(data: item.data)?
                               .preparingThumbnail(of: CGSize(width: 88, height: 88)),
                           byteCount: item.data.count)
            }
        } catch is CancellationError {
            return  // the overlay is already gone; nothing to show
        } catch {
            phase = .failed(error.localizedDescription)
            return
        }
        await upload()
    }

    /// The upload leg, shared by the first run and every retry. Skips
    /// attachments that already carry a remote path.
    private func upload() async {
        do {
            let uploader = try await uploaderProvider()
            let totalBytes = max(1, processed.reduce(0) { $0 + $1.data.count })
            var doneBytes = 0
            phase = .uploading(0)
            for (index, item) in processed.enumerated() {
                try Task.checkCancellation()
                if attachments[index].remotePath != nil {
                    doneBytes += item.data.count
                    phase = .uploading(Double(doneBytes) / Double(totalBytes))
                    continue
                }
                let base = doneBytes
                let path = try await uploader.uploadToUploadsDirectory(
                    item.data, named: item.filename,
                    progress: { [weak self] fraction in
                        let overall = (Double(base) + fraction * Double(item.data.count))
                            / Double(totalBytes)
                        Task { @MainActor [weak self] in
                            // Never regress past a later chunk's report — the
                            // hops can land out of order.
                            if case .uploading(let shown) = self?.phase ?? .processing,
                               overall > shown {
                                self?.phase = .uploading(overall)
                            }
                        }
                    })
                attachments[index].remotePath = path
                doneBytes += item.data.count
                phase = .uploading(Double(doneBytes) / Double(totalBytes))
            }
            phase = .ready
            let paths = attachments.compactMap(\.remotePath)
            if let numbers = onReady?(paths), numbers.count == attachments.count {
                for (index, number) in numbers.enumerated() {
                    attachments[index].sessionNumber = number
                }
            }
        } catch is CancellationError {
            // The overlay is already gone; nothing to show.
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }
}
