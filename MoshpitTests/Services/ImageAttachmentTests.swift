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
                               gps: Bool = false) -> Data {
        let format = UIGraphicsImageRendererFormat()
        format.opaque = opaque
        format.scale = 1
        let size = CGSize(width: width, height: height)
        let image = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            UIColor.systemTeal.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            if !opaque {
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

    @Test("Paths are single-quoted and space-padded")
    func insertTextQuoting() {
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
        #expect(text == " '/a/1.jpg' '/a/2.png' ")
        #expect(ImageAttachmentPipeline.insertText(forRemotePaths: []) == "")
    }
}
