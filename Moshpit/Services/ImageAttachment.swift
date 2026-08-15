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

        let hasAlpha: Bool
        switch image.alphaInfo {
        case .none, .noneSkipFirst, .noneSkipLast:
            hasAlpha = false
        default:
            hasAlpha = true
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

    /// The text pasted at the prompt for uploaded paths: each path
    /// single-quoted (a home directory can contain a space), padded with
    /// spaces so it lands cleanly between whatever the user already typed.
    /// Bare paths, not `@`-mentions — `@` triggers Claude Code's completion
    /// menu when typed blind, and a bare path reads the same to every agent.
    static func insertText(forRemotePaths paths: [String]) -> String {
        guard !paths.isEmpty else { return "" }
        let quoted = paths.map { path in
            "'" + path.replacingOccurrences(of: "'", with: "'\\''") + "'"
        }
        return " " + quoted.joined(separator: " ") + " "
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
    }

    private(set) var phase: Phase = .processing
    private(set) var attachments: [Attachment] = []

    @ObservationIgnored private let uploader: any FileUploader
    @ObservationIgnored private var work: Task<Void, Never>?

    init(uploader: any FileUploader) {
        self.uploader = uploader
    }

    /// Kick off processing + upload for the picked images. UI state flows
    /// through `phase`; the caller only comes back for `insertText()`.
    func start(pickedData: [Data]) {
        work = Task { [weak self] in
            await self?.run(pickedData: pickedData)
        }
    }

    func cancel() {
        work?.cancel()
    }

    /// The paste payload — nil until every upload landed.
    func insertText() -> String? {
        guard phase == .ready else { return nil }
        let paths = attachments.compactMap(\.remotePath)
        guard !paths.isEmpty else { return nil }
        return ImageAttachmentPipeline.insertText(forRemotePaths: paths)
    }

    private func run(pickedData: [Data]) async {
        do {
            // Process off the main actor — ImageIO decode of a 48MP photo is
            // real work — then upload sequentially so progress reads honestly
            // and a failure names one file, not a pile.
            var processed: [ImageAttachmentPipeline.Processed] = []
            for raw in pickedData {
                try Task.checkCancellation()
                let item = try await Task.detached(priority: .userInitiated) {
                    try ImageAttachmentPipeline.process(raw)
                }.value
                processed.append(item)
            }
            attachments = processed.map { item in
                Attachment(thumbnail: UIImage(data: item.data)?
                               .preparingThumbnail(of: CGSize(width: 88, height: 88)),
                           byteCount: item.data.count)
            }

            let totalBytes = max(1, processed.reduce(0) { $0 + $1.data.count })
            var doneBytes = 0
            phase = .uploading(0)
            for (index, item) in processed.enumerated() {
                try Task.checkCancellation()
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
        } catch is CancellationError {
            // The overlay is already gone; nothing to show.
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }
}
