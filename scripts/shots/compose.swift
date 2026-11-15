// The App Store images: each raw shot under its headline, on the app's dark
// background, at the size the store takes (1320 × 2868 for the iPhone,
// 2064 × 2752 for the iPad, 2752 × 2064 for a landscape iPad shot). Run on the screenshot workflow's macOS runner
// after scripts/shots/ci-capture.sh, and anywhere else with Xcode's Swift:
//
//   swift scripts/shots/compose.swift <device> <raw> <out> [--external <dir>] [locale…]
//
//   <device>    iphone | ipad
//   <raw>       the capture's shots: <raw>/<locale>/<shot>.png
//   <out>       the store images: <out>/<locale>/<nn>-<shot>.png
//   --external  shots taken elsewhere (the lock screen and the Dynamic
//               Island, on a phone): <dir>/<locale>/<shot>.png, else
//               <dir>/<shot>.png for every locale. One found here is used
//               in place of the capture's.
//   locale      the captions' locales to compose (default: every file in
//               scripts/shots/captions/)
//
// The headlines are scripts/shots/captions/<locale>.json: for each device,
// the shots in the store's order, each with its headline. A headline is set
// in the system's bold face for the locale's language (CoreText picks the
// script's own: PingFang, Hiragino, Apple SD Gothic Neo), centred, on at
// most two lines, smaller only when two lines at the full size can't hold
// it. The shot sits under it, scaled to the room left, with the device's
// rounded corners and a hairline edge so a dark screen holds its shape on
// the dark ground. No device frame.
//
// Deterministic: the same shots and headlines on the same macOS give the
// same bytes (no dates, no metadata). A shot missing from both <raw> and
// --external fails the run, after the rest are written.

import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct Layout {
    var canvas: CGSize
    /// Left and right of the headline.
    var side: CGFloat
    /// Above the headline.
    var top: CGFloat
    /// The headline's size, and the least it may shrink to.
    var font: CGFloat
    var minFont: CGFloat
    /// Between the headline and the shot, and under the shot.
    var gap: CGFloat
    var bottom: CGFloat
    /// The shot's corners, as a share of its drawn width.
    var corner: CGFloat

    static let iphone = Layout(canvas: CGSize(width: 1320, height: 2868), side: 110, top: 190,
                               font: 104, minFont: 72, gap: 96, bottom: 110, corner: 0.118)
    static let ipad = Layout(canvas: CGSize(width: 2064, height: 2752), side: 200, top: 160,
                             font: 112, minFont: 80, gap: 96, bottom: 120, corner: 0.035)
}

/// The app's dark screen (ui/src/ui/tokens.rs: background, dark) and the
/// headline's ink.
let ground = CGColor(srgbRed: 0x06 / 255, green: 0x07 / 255, blue: 0x0A / 255, alpha: 1)
let ink = CGColor(srgbRed: 0xF4 / 255, green: 0xF4 / 255, blue: 0xF6 / 255, alpha: 1)
let edge = CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.14)

struct Caption: Decodable {
    var shot: String
    var title: String
}

struct Captions: Decodable {
    /// The language the headlines are set in (a BCP 47 tag: CoreText's
    /// cascade for its script).
    var language: String
    var iphone: [Caption]
    var ipad: [Caption]
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("✘ \(message)\n".utf8))
    exit(1)
}

// MARK: The arguments

var args = Array(CommandLine.arguments.dropFirst())
var external: URL?
if let i = args.firstIndex(of: "--external") {
    guard i + 1 < args.count else { fail("--external needs a folder") }
    external = URL(fileURLWithPath: args[i + 1])
    args.removeSubrange(i...(i + 1))
}
guard args.count >= 3 else {
    fail("usage: compose.swift <iphone|ipad> <raw> <out> [--external <dir>] [locale…]")
}
let device = args[0]
let raw = URL(fileURLWithPath: args[1])
let out = URL(fileURLWithPath: args[2])
let layout: Layout
switch device {
case "iphone": layout = .iphone
case "ipad": layout = .ipad
default: fail("device: iphone or ipad, not \(device)")
}
let script = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
let captionsDir = script.deletingLastPathComponent().appendingPathComponent("captions")
var locales = Array(args.dropFirst(3))
if locales.isEmpty {
    let files = (try? FileManager.default.contentsOfDirectory(atPath: captionsDir.path)) ?? []
    locales = files.filter { $0.hasSuffix(".json") }.map { String($0.dropLast(5)) }.sorted()
}
guard !locales.isEmpty else { fail("no captions in \(captionsDir.path)") }

// MARK: Drawing

func loadImage(_ url: URL) -> CGImage? {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(source, 0, nil)
}

/// The headline set at `size`: its frame in `width`, and how many lines.
func setHeadline(_ title: String, language: String, size: CGFloat, width: CGFloat)
    -> (framesetter: CTFramesetter, height: CGFloat, lines: Int) {
    let font = CTFontCreateUIFontForLanguage(.emphasizedSystem, size, language as CFString)
        ?? CTFontCreateWithName("Helvetica-Bold" as CFString, size, nil)
    var alignment = CTTextAlignment.center
    var spacing = size * 0.12
    let style = withUnsafePointer(to: &alignment) { alignment in
        withUnsafePointer(to: &spacing) { spacing in
            let settings = [
                CTParagraphStyleSetting(spec: .alignment, valueSize: MemoryLayout<CTTextAlignment>.size,
                                        value: alignment),
                CTParagraphStyleSetting(spec: .lineSpacingAdjustment, valueSize: MemoryLayout<CGFloat>.size,
                                        value: spacing),
            ]
            return CTParagraphStyleCreate(settings, settings.count)
        }
    }
    let attributes: [CFString: Any] = [
        kCTFontAttributeName: font,
        kCTForegroundColorAttributeName: ink,
        kCTParagraphStyleAttributeName: style,
        // The language's own forms (Han glyphs differ between zh and ja).
        kCTLanguageAttributeName: language as CFString,
    ]
    let text = CFAttributedStringCreate(nil, title as CFString, attributes as CFDictionary)!
    let framesetter = CTFramesetterCreateWithAttributedString(text)
    let fit = CTFramesetterSuggestFrameSizeWithConstraints(
        framesetter, CFRange(location: 0, length: 0), nil,
        CGSize(width: width, height: .greatestFiniteMagnitude), nil)
    let path = CGPath(rect: CGRect(x: 0, y: 0, width: width, height: ceil(fit.height) + 2), transform: nil)
    let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: 0, length: 0), path, nil)
    let lines = CFArrayGetCount(CTFrameGetLines(frame))
    return (framesetter, ceil(fit.height), lines)
}

/// Where a headline breaks. One that fits on a line stays on it. One
/// with a line break of its own (`\n` in its caption) breaks there. A
/// Chinese or Japanese one, whose words a line may split anywhere, breaks
/// after the comma or stop nearest its middle ("…，"), else the
/// enumeration comma ("…、"), or, with none, shrinks to one line. Any other is wrapped by CoreText at the width that
/// keeps its lines as even as they go (no word left alone on the last).
/// Then it shrinks, to `minFont` at most, until it has no more lines than
/// it should.
func fitHeadline(_ title: String, language: String, width: CGFloat)
    -> (text: String, font: CGFloat, width: CGFloat) {
    var font = layout.font
    func lines(_ text: String, _ size: CGFloat, _ w: CGFloat) -> Int {
        setHeadline(text, language: language, size: size, width: w).lines
    }
    func shrink(_ text: String, to most: Int, _ w: CGFloat) {
        while lines(text, font, w) > most && font > layout.minFont { font -= 2 }
    }
    if title.contains("\n") {
        let text = title.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
            .joined(separator: "\n")
        shrink(text, to: text.components(separatedBy: "\n").count, width)
        return (text, font, width)
    }
    if lines(title, font, width) == 1 { return (title, font, width) }
    let cjk = ["zh", "ja"].contains { language.hasPrefix($0) }
    if cjk {
        // A comma or a stop ends a clause; an enumeration comma (、) only
        // an item: the clause's end first.
        let strong: Set<Character> = ["，", "。", "：", "；", "！", "？", ","]
        let weak: Set<Character> = ["、"]
        let chars = Array(title)
        let middle = Double(chars.count) / 2
        func nearest(_ marks: Set<Character>) -> Int? {
            chars.indices.filter { marks.contains(chars[$0]) && $0 < chars.count - 1 }
                .min { abs(Double($0 + 1) - middle) < abs(Double($1 + 1) - middle) }
        }
        let cut = nearest(strong) ?? nearest(weak)
        guard let cut else {
            shrink(title, to: 1, width)
            return (title, font, width)
        }
        let text = String(chars[...cut]) + "\n"
            + String(chars[(cut + 1)...]).trimmingCharacters(in: .whitespaces)
        shrink(text, to: 2, width)
        return (text, font, width)
    }
    shrink(title, to: 2, width)
    let count = lines(title, font, width)
    // The narrowest width that keeps that many lines: the lines as even
    // as the words let them be.
    var (low, high) = (width / 2, width)
    while high - low > 4 {
        let mid = (low + high) / 2
        if lines(title, font, mid) > count { low = mid } else { high = mid }
    }
    return (title, font, ceil(high))
}

/// The store's size for `shot`: the device's, turned for a landscape iPad
/// shot (2752 × 2064, which the store takes beside portrait ones).
func canvas(for shot: CGImage) -> CGSize {
    guard device == "ipad", shot.width > shot.height else { return layout.canvas }
    return CGSize(width: layout.canvas.height, height: layout.canvas.width)
}

func compose(shot: CGImage, title: String, language: String, to url: URL) -> Bool {
    let size = canvas(for: shot)
    guard let context = CGContext(
        data: nil, width: Int(size.width), height: Int(size.height), bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return false }
    context.setFillColor(ground)
    context.fill(CGRect(origin: .zero, size: size))

    let fitted = fitHeadline(title, language: language, width: size.width - 2 * layout.side)
    let set = setHeadline(fitted.text, language: language, size: fitted.font, width: fitted.width)
    if set.lines > 2 {
        FileHandle.standardError.write(Data("  ! \(url.lastPathComponent): \(set.lines) lines at \(Int(fitted.font)) pt\n".utf8))
    }
    // CoreGraphics counts from the bottom.
    let textTop = size.height - layout.top
    let textRect = CGRect(x: ((size.width - fitted.width) / 2).rounded(), y: textTop - set.height,
                          width: fitted.width, height: set.height + 2)
    let frame = CTFramesetterCreateFrame(set.framesetter, CFRange(location: 0, length: 0),
                                         CGPath(rect: textRect, transform: nil), nil)
    CTFrameDraw(frame, context)

    // The shot: the room left, its aspect kept, centred across.
    let roomTop = textTop - set.height - layout.gap
    let room = CGSize(width: size.width - 2 * layout.side * 0.7, height: roomTop - layout.bottom)
    let scale = min(room.width / CGFloat(shot.width), room.height / CGFloat(shot.height))
    let drawn = CGSize(width: (CGFloat(shot.width) * scale).rounded(),
                       height: (CGFloat(shot.height) * scale).rounded())
    let rect = CGRect(x: ((size.width - drawn.width) / 2).rounded(), y: roomTop - drawn.height,
                      width: drawn.width, height: drawn.height)
    let radius = (drawn.width * layout.corner).rounded()
    let outline = CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
    context.saveGState()
    context.addPath(outline)
    context.clip()
    context.interpolationQuality = .high
    context.draw(shot, in: rect)
    context.restoreGState()
    context.addPath(outline)
    context.setStrokeColor(edge)
    context.setLineWidth(3)
    context.strokePath()

    guard let image = context.makeImage(),
          let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { return false }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

// MARK: The run

var missing: [String] = []
for locale in locales {
    let file = captionsDir.appendingPathComponent("\(locale).json")
    guard let data = try? Data(contentsOf: file) else { fail("no captions for \(locale): \(file.path)") }
    let captions: Captions
    do { captions = try JSONDecoder().decode(Captions.self, from: data) } catch {
        fail("\(file.lastPathComponent): \(error)")
    }
    let shots = device == "iphone" ? captions.iphone : captions.ipad
    let folder = out.appendingPathComponent(locale)
    try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    for (index, caption) in shots.enumerated() {
        let candidates = [
            external?.appendingPathComponent(locale).appendingPathComponent("\(caption.shot).png"),
            external?.appendingPathComponent("\(caption.shot).png"),
            raw.appendingPathComponent(locale).appendingPathComponent("\(caption.shot).png"),
        ].compactMap { $0 }
        guard let source = candidates.first(where: { FileManager.default.fileExists(atPath: $0.path) }),
              let shot = loadImage(source) else {
            missing.append("\(locale)/\(caption.shot).png")
            continue
        }
        // The store's order first; a shot id's own number gives way to it.
        let bare = caption.shot.replacingOccurrences(of: #"^\d+-"#, with: "", options: .regularExpression)
        let name = String(format: "%02d-%@.png", index + 1, bare)
        let target = folder.appendingPathComponent(name)
        guard compose(shot: shot, title: caption.title, language: captions.language, to: target) else {
            fail("couldn't write \(target.path)")
        }
        let from = source.path.hasPrefix(raw.path) ? "" : "  (external)"
        let size = canvas(for: shot)
        print("  ✓ \(locale)/\(name) \(Int(size.width))x\(Int(size.height))\(from)")
    }
}
if !missing.isEmpty {
    fail("no shot for: \(missing.joined(separator: ", "))")
}
