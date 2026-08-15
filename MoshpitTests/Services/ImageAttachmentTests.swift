import Foundation
import ImageIO
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import Moshpit

/// `ImageAttachmentPipeline` — the pure half of the attach-image flow.
///
/// Everything a picked image must be by the time it leaves the phone: small
/// enough for a cellular link, stripped of EXIF/GPS (a screenshot's path to a
/// shared server must not carry the photo library's location data), named so
/// it can't collide or leak the local filename, and quoted so a paste can't
/// break at a space in someone's home directory.
@Suite("ImageAttachmentPipeline")
struct ImageAttachmentTests {

    // MARK: Fixtures

    /// Renders a flat-colour image and encodes it, optionally with a GPS
    /// dictionary — the metadata the pipeline exists to remove.
    private func makeImageData(width: Int, height: Int,
                               opaque: Bool,
                               clearRegion: Bool? = nil,
                               gps: Bool = false) -> Data {
        let format = UIGraphicsImageRendererFormat()
        format.opaque = opaque
        format.scale = 1
        let size = CGSize(width: width, height: height)
        let image = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            UIColor.systemTeal.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            if clearRegion ?? !opaque {
                ctx.cgContext.clear(CGRect(x: 0, y: 0, width: width / 2, height: height / 2))
            }
        }
        guard let cgImage = image.cgImage else { fatalError("fixture render failed") }

        let type: UTType = opaque ? .jpeg : .png
        let out = NSMutableData()
        let destination = CGImageDestinationCreateWithData(
            out, type.identifier as CFString, 1, nil)!
        var properties: [CFString: Any] = [:]
        if gps {
            properties[kCGImagePropertyGPSDictionary] = [
                kCGImagePropertyGPSLatitude: 31.2304,
                kCGImagePropertyGPSLongitude: 121.4737,
            ]
        }
        CGImageDestinationAddImage(destination, cgImage, properties as CFDictionary)
        CGImageDestinationFinalize(destination)
        return out as Data
    }

    private func properties(of data: Data) -> [CFString: Any] {
        let source = CGImageSourceCreateWithData(data as CFData, nil)!
        return CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as! [CFString: Any]
    }

    private let fixedDate = Date(timeIntervalSince1970: 1_755_231_633) // 2026-08-15 04:20:33 UTC

    // MARK: Scaling

    @Test("Oversized image scales to the 2048 long side, keeping aspect")
    func scalesDownLongSide() throws {
        let raw = makeImageData(width: 3000, height: 1500, opaque: true)
        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(max(processed.pixelWidth, processed.pixelHeight) == 2048)
        // 2:1 input stays 2:1 (±1px for integer rounding).
        #expect(abs(processed.pixelWidth - processed.pixelHeight * 2) <= 2)
    }

    @Test("Small image is not upscaled")
    func keepsSmallImage() throws {
        let raw = makeImageData(width: 500, height: 300, opaque: true)
        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(processed.pixelWidth == 500)
        #expect(processed.pixelHeight == 300)
    }

    // MARK: Metadata

    @Test("GPS metadata is stripped by the re-encode")
    func stripsGPS() throws {
        let raw = makeImageData(width: 800, height: 600, opaque: true, gps: true)
        // Sanity: the fixture really carries GPS before processing.
        #expect(properties(of: raw)[kCGImagePropertyGPSDictionary] != nil)

        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(properties(of: processed.data)[kCGImagePropertyGPSDictionary] == nil)
    }

    // MARK: Format choice

    @Test("Opaque input becomes JPEG")
    func opaqueBecomesJPEG() throws {
        let raw = makeImageData(width: 400, height: 400, opaque: true)
        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(processed.filename.hasSuffix(".jpg"))
        #expect((properties(of: processed.data)[kCGImagePropertyHasAlpha] as? Bool) != true)
    }

    @Test("Transparency keeps PNG")
    func alphaKeepsPNG() throws {
        let raw = makeImageData(width: 400, height: 400, opaque: false)
        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(processed.filename.hasSuffix(".png"))
    }

    @Test("An alpha channel with no transparent pixels becomes JPEG")
    func opaqueAlphaChannelBecomesJPEG() throws {
        // RGBA format, every pixel fully opaque — the simulator-screenshot /
        // camera-pipeline shape that used to ship megabytes of needless PNG.
        let raw = makeImageData(width: 400, height: 400, opaque: false, clearRegion: false)
        #expect(properties(of: raw)[kCGImagePropertyHasAlpha] as? Bool == true)
        let processed = try ImageAttachmentPipeline.process(raw, date: fixedDate, suffix: "a3f2")
        #expect(processed.filename.hasSuffix(".jpg"))
    }

    @Test("Undecodable bytes throw rather than upload garbage")
    func rejectsGarbage() {
        #expect(throws: ImageAttachmentPipeline.PipelineError.self) {
            _ = try ImageAttachmentPipeline.process(
                Data("not an image".utf8), date: fixedDate, suffix: "a3f2")
        }
    }

    // MARK: Naming

    @Test("Filename is date-time-suffix, no local name leakage")
    func filenameShape() {
        let name = ImageAttachmentPipeline.filename(date: fixedDate, suffix: "a3f2", ext: "jpg")
        // Local wall-clock rendering of the fixed instant — assert shape, not
        // the timezone-dependent digits.
        #expect(name.wholeMatch(of: /IMG-\d{8}-\d{6}-a3f2\.jpg/) != nil)
    }

    @Test("Random suffix is 4 hex chars")
    func suffixShape() {
        for _ in 0..<20 {
            #expect(ImageAttachmentPipeline.randomSuffix()
                .wholeMatch(of: /[0-9a-f]{4}/) != nil)
        }
    }

    // MARK: Paste text

    @Test("Clean paths stay bare — quotes break agents' path auto-attach")
    func insertTextBareWhenClean() {
        let text = ImageAttachmentPipeline.insertText(
            forRemotePaths: ["/home/dev/.moshpit/uploads/IMG-1.jpg"])
        #expect(text == " /home/dev/.moshpit/uploads/IMG-1.jpg ")
    }

    @Test("A path with a space gets the shell-safe quoted form")
    func insertTextQuotesSpaces() {
        let text = ImageAttachmentPipeline.insertText(
            forRemotePaths: ["/home/dev user/.moshpit/uploads/IMG-1.jpg"])
        #expect(text == " '/home/dev user/.moshpit/uploads/IMG-1.jpg' ")
    }

    @Test("A quote inside a path is shell-escaped")
    func insertTextEscapesQuote() {
        let text = ImageAttachmentPipeline.insertText(forRemotePaths: ["/a/it's.jpg"])
        #expect(text == #" '/a/it'\''s.jpg' "#)
    }

    @Test("Multiple paths join with spaces; none yields empty")
    func insertTextJoins() {
        let text = ImageAttachmentPipeline.insertText(forRemotePaths: ["/a/1.jpg", "/a/2.png"])
        #expect(text == " /a/1.jpg /a/2.png ")
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: []) == "")
    }

    // MARK: Agent-aware insert styles

    @Test("Agent classification: bare for claude/codex/goose, @ for gemini/qwen, /add for aider")
    func insertStyleClassification() {
        #expect(ImageInsertStyle.forAgent(nil) == .barePath)
        #expect(ImageInsertStyle.forAgent("zsh") == .barePath)
        #expect(ImageInsertStyle.forAgent("claude") == .barePath)
        #expect(ImageInsertStyle.forAgent("2.1.227") == .barePath)   // claude's version-title comm
        #expect(ImageInsertStyle.forAgent("codex") == .barePath)
        #expect(ImageInsertStyle.forAgent("goose") == .barePath)
        #expect(ImageInsertStyle.forAgent("gemini") == .atMention)
        #expect(ImageInsertStyle.forAgent("qwen") == .atMention)
        #expect(ImageInsertStyle.forAgent("aider") == .aiderAdd)
    }

    @Test("@-mention style prefixes clean paths and falls back on quoted ones")
    func atMentionStyle() {
        #expect(ImageAttachmentPipeline.insertText(
            forRemotePaths: ["/a/1.jpg"], style: .atMention) == " @/a/1.jpg ")
        // A path needing quotes can't be an @-token — shell-safe fallback.
        #expect(ImageAttachmentPipeline.insertText(
            forRemotePaths: ["/a b/1.jpg"], style: .atMention) == " '/a b/1.jpg' ")
    }

    @Test("aider style leads with /add and no padding")
    func aiderStyle() {
        #expect(ImageAttachmentPipeline.insertText(
            forRemotePaths: ["/a/1.jpg", "/a/2.png"], style: .aiderAdd)
            == "/add /a/1.jpg /a/2.png")
    }

    // MARK: Session log

    @Test("Upload log numbers sequentially from 1 and keeps order")
    func uploadLogNumbers() {
        var log = ImageUploadLog()
        let first = log.record(remotePath: "/u/.moshpit/uploads/IMG-a.jpg")
        let second = log.record(remotePath: "/u/.moshpit/uploads/IMG-b.png")
        #expect(first.number == 1)
        #expect(second.number == 2)
        #expect(log.entries.map(\.number) == [1, 2])
        #expect(second.filename == "IMG-b.png")
    }

    @Test("Clipboard type preference starts with originals, ends with TIFF")
    func clipboardTypeOrder() {
        #expect(ClipboardImageReader.preferredTypes.first == "public.heic")
        #expect(ClipboardImageReader.preferredTypes.last == "public.tiff")
    }

    // MARK: Exec-channel fallback commands

    @Test("Exec upload plan pins the exact command strings")
    func execUploadPlan() {
        let plan = ExecUploadCommands.plan(home: "/home/dev", filename: "IMG-1.jpg")
        #expect(plan.finalPath == "/home/dev/.moshpit/uploads/IMG-1.jpg")
        #expect(plan.begin ==
            "mkdir -p '/home/dev/.moshpit/uploads' && chmod 700 '/home/dev/.moshpit' '/home/dev/.moshpit/uploads' && rm -f '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part'")
        #expect(plan.append("QUJD") ==
            "printf %s 'QUJD' >> '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part'")
        #expect(plan.finish.contains("base64 -d <"))
        #expect(plan.finish.contains("|| base64 -D <"))
        #expect(plan.finish.contains("chmod 600 '/home/dev/.moshpit/uploads/IMG-1.jpg'"))
        #expect(plan.finish.hasSuffix("echo MOSHPIT_UPLOAD_OK"))
    }

    @Test("Exec upload quoting survives a home with a space and a quote")
    func execUploadQuoting() {
        let plan = ExecUploadCommands.plan(home: "/Users/dev o'brien", filename: "IMG-1.jpg")
        #expect(plan.begin.contains(#"'/Users/dev o'\''brien/.moshpit/uploads'"#))
        #expect(plan.finalPath == "/Users/dev o'brien/.moshpit/uploads/IMG-1.jpg")
    }

    @Test("Retention sweep command is pinned exactly")
    @MainActor
    func cleanupCommand() {
        #expect(SessionHub.ActiveSession.uploadCleanupCommand(days: 7) ==
            #"find "$HOME/.moshpit/uploads" -type f -mtime +7 -delete 2>/dev/null || true"#)
    }

    @Test("Chunk size stays inside a conservative argument budget")
    func execUploadChunkBudget() {
        // 48KB raw → 64KB base64 → plus the printf wrapper, safely under the
        // ~128KB floor of ARG_MAX-style limits seen on small sshd targets.
        #expect(ExecUploadCommands.chunkBytes == 48 * 1024)
        let encoded = (ExecUploadCommands.chunkBytes + 2) / 3 * 4
        #expect(encoded < 128 * 1024)
    }
}
