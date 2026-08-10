#!/usr/bin/env swift
// Renders every bundled app icon: several designs, each with its own fixed
// colorway. Plain CoreGraphics + ImageIO (no UIKit/AppKit/SwiftUI) so it runs
// as a standalone `swift` script.
//
// Icons are deliberately INDEPENDENT of the app's accent color. iOS alternate
// icons must be declared in Info.plist and bundled as PNGs at build time —
// `setAlternateIconName` can only pick from those, and nothing can generate an
// icon at runtime. So rather than pretend the icon tracks a (possibly custom)
// accent, the icon is its own choice from this gallery and each entry bakes in
// the colors it wants.
//
// The crowd-surf geometry mirrors `MoshpitGlyph.Metrics` in
// Moshpit/UI/Brand/MoshpitMark.swift — keep the two in step.
//
// Usage: swift scripts/generate-app-icons.swift
// Output:
//   Moshpit/App/IconFiles/AppIcon60x60@{2x,3x}.png              (primary)
//   Moshpit/App/IconFiles/AppIcon-<Tag>60x60@{2x,3x}.png        (alternates)
//   Moshpit/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png  (1024 marketing)

import CoreGraphics
import CoreText
import ImageIO
import Foundation
#if canImport(UniformTypeIdentifiers)
import UniformTypeIdentifiers
#endif

typealias RGB = (UInt8, UInt8, UInt8)

/// Which figure to draw. Distinct compositions, not recolors — the picker is
/// meant to offer real alternatives.
enum Pattern {
    /// A glowing cursor crowd-surfing a row of three grounded blocks.
    case pit
    /// The heritage mark: a handset over its cursor cradle. A ringdown
    /// circuit has no dial — the line is already up. Kept as an easter egg.
    case handset
    /// ":wq" — the one vim command everybody knows.
    case wq
    /// A "⌃b" keycap — the tmux prefix, muscle memory made visible.
    case prefixKey
    /// A little house whose door is a glowing cursor block — there's no
    /// place like 127.0.0.1.
    case localhost
    /// "NO CARRIER" in CRT amber over faint scanlines — how a line died
    /// before mosh existed.
    case noCarrier
    /// The cursor block alone, oversized.
    case cursor
    /// A prompt chevron with two hail arcs — "❯))".
    case hail
}

struct IconSpec {
    /// nil = primary icon (file `AppIcon60x60`, and `CFBundlePrimaryIcon`).
    let fileTag: String?
    let pattern: Pattern
    /// The figure's main color (handset strokes / chevron).
    let ink: RGB
    /// The highlight color (cursor block, arcs) — glows.
    let glow: RGB
    /// Background gradient endpoints, top-left → bottom-right.
    let bg: (RGB, RGB)
}

func color(_ rgb: RGB, alpha: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat(rgb.0) / 255, green: CGFloat(rgb.1) / 255, blue: CGFloat(rgb.2) / 255, alpha: alpha)
}

/// Blends `rgb` toward `target` by `t` (0 = unchanged, 1 = `target`).
func mix(_ rgb: RGB, toward target: RGB, by t: CGFloat) -> RGB {
    func lerp(_ a: UInt8, _ b: UInt8) -> UInt8 {
        UInt8(max(0, min(255, CGFloat(a) + (CGFloat(b) - CGFloat(a)) * t)))
    }
    return (lerp(rgb.0, target.0), lerp(rgb.1, target.1), lerp(rgb.2, target.2))
}

let violet: RGB = (0x6C, 0x6B, 0xEF)
let teal: RGB   = (0x53, 0xDC, 0xC9)
let green: RGB  = (0x2F, 0xA8, 0x71)
let amber: RGB  = (0xC9, 0x8A, 0x2E)
let nearWhite: RGB = (0xF4, 0xF6, 0xFA)

/// Near-black backdrop carrying a faint wash of `tint`, matching what the
/// in-app mark sits on.
func darkBG(_ tint: RGB) -> (RGB, RGB) {
    (mix(tint, toward: (0, 0, 0), by: 0.82), mix(tint, toward: (0, 0, 0), by: 0.93))
}

// Every alternate is a DIFFERENT figure with its own colorway — an easter-egg
// gallery, not recolors of one mark. Each egg carries one of the accents, so
// the color range survives the redesign.
let specs: [IconSpec] = [
    // Primary — the app's own identity.
    IconSpec(fileTag: nil, pattern: .pit, ink: nearWhite, glow: violet, bg: darkBG(violet)),
    // The heritage mark, restored as an egg.
    IconSpec(fileTag: "Ringdown", pattern: .handset, ink: nearWhite, glow: violet, bg: darkBG(violet)),
    IconSpec(fileTag: "Wq", pattern: .wq, ink: nearWhite, glow: green, bg: darkBG(green)),
    IconSpec(fileTag: "Prefix", pattern: .prefixKey, ink: nearWhite, glow: teal, bg: darkBG(teal)),
    // The one light tile — home reads as daylight.
    IconSpec(fileTag: "Localhost", pattern: .localhost, ink: (0x1A, 0x1C, 0x24), glow: violet,
             bg: ((0xF2, 0xF4, 0xF8), (0xD8, 0xDD, 0xE8))),
    IconSpec(fileTag: "NoCarrier", pattern: .noCarrier, ink: nearWhite, glow: amber, bg: darkBG(amber)),
    // Different figures from the original set — already distinct, kept.
    IconSpec(fileTag: "Cursor", pattern: .cursor, ink: nearWhite, glow: violet, bg: darkBG(violet)),
    IconSpec(fileTag: "Hail", pattern: .hail, ink: nearWhite, glow: teal, bg: darkBG(teal)),
]

// MARK: - Figures
//
// All geometry is authored in the mark's 24×24 design grid; `u`/`px`/`py` map it
// into pixels (`py` flips the axis, since CoreGraphics' origin is bottom-left
// while the grid's is top-left, like SVG).

struct Grid {
    let rect: CGRect
    var side: CGFloat { rect.width }
    func u(_ v: CGFloat) -> CGFloat { side * v / 24 }
    func px(_ v: CGFloat) -> CGFloat { rect.minX + u(v) }
    func py(_ v: CGFloat) -> CGFloat { rect.minY + u(24 - v) }
}

/// The cursor block, glowing. `rect` is in design units with a top-left origin.
func drawCursorBlock(_ ctx: CGContext, _ g: Grid, spec: IconSpec, rect: CGRect) {
    ctx.saveGState()
    ctx.setShadow(offset: .zero, blur: g.u(2.6), color: color(spec.glow, alpha: 0.8))
    ctx.setFillColor(color(spec.glow))
    let px = CGRect(x: g.px(rect.minX), y: g.py(rect.maxY),
                    width: g.u(rect.width), height: g.u(rect.height))
    ctx.addPath(CGPath(roundedRect: px, cornerWidth: g.u(0.8), cornerHeight: g.u(0.8),
                       transform: nil))
    ctx.fillPath()
    ctx.restoreGState()
}

/// A rotated rounded block. `rect` is in design units (top-left origin);
/// positive `tilt` lifts the RIGHT edge — this CG context's y grows upward, so
/// the sign here is the opposite of the SwiftUI view's `rotationEffect`.
func drawBlock(_ ctx: CGContext, _ g: Grid, rect: CGRect, tilt: CGFloat = 0,
               fill: CGColor, glow: CGColor? = nil, corner: CGFloat) {
    ctx.saveGState()
    ctx.translateBy(x: g.px(rect.midX), y: g.py(rect.midY))
    if tilt != 0 { ctx.rotate(by: tilt * .pi / 180) }
    if let glow { ctx.setShadow(offset: .zero, blur: g.u(2.6), color: glow) }
    ctx.setFillColor(fill)
    let w = g.u(rect.width), h = g.u(rect.height)
    ctx.addPath(CGPath(roundedRect: CGRect(x: -w / 2, y: -h / 2, width: w, height: h),
                       cornerWidth: g.u(corner), cornerHeight: g.u(corner), transform: nil))
    ctx.fillPath()
    ctx.restoreGState()
}

/// Crowd-surf: a glowing cursor block carried over a row of three grounded ink
/// blocks. Geometry mirrors `MoshpitGlyph.Metrics`.
func drawPit(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    for x in [3.6, 9.8, 16.0] as [CGFloat] {
        drawBlock(ctx, g, rect: CGRect(x: x, y: 15.6, width: 4.4, height: 2.8),
                  fill: color(spec.ink, alpha: 0.9), corner: 0.8)
    }
    drawBlock(ctx, g, rect: CGRect(x: 8.4, y: 8.0, width: 6.8, height: 4.0), tilt: 12,
              fill: color(spec.glow), glow: color(spec.glow, alpha: 0.8), corner: 1.1)
}

/// Centered monospace text (Menlo-Bold via CoreText), optionally glowing.
/// `size` and `center` are in design units.
func drawMonoText(_ ctx: CGContext, _ g: Grid, text: String, size: CGFloat,
                  center: CGPoint, fill: CGColor, glow: CGColor? = nil) {
    let font = CTFontCreateWithName("Menlo-Bold" as CFString, g.u(size), nil)
    let attrs: [CFString: Any] = [
        kCTFontAttributeName: font,
        kCTForegroundColorAttributeName: fill,
    ]
    guard let astr = CFAttributedStringCreate(
        kCFAllocatorDefault, text as CFString, attrs as CFDictionary) else { return }
    let line = CTLineCreateWithAttributedString(astr)
    ctx.saveGState()
    if let glow { ctx.setShadow(offset: .zero, blur: g.u(2.0), color: glow) }
    // CTLineGetImageBounds measures relative to the CURRENT text position —
    // zero it first, or the second string on a tile inherits the first one's
    // offset and gets shoved off-canvas.
    ctx.textPosition = .zero
    let bounds = CTLineGetImageBounds(line, ctx)
    ctx.textPosition = CGPoint(x: g.px(center.x) - bounds.width / 2 - bounds.minX,
                               y: g.py(center.y) - bounds.height / 2 - bounds.minY)
    CTLineDraw(line, ctx)
    ctx.restoreGState()
}

/// Ringdown: the heritage handset — a shallow arc with two end beads over a
/// glowing cursor cradle. Geometry preserved verbatim from the pre-rebrand
/// mark so the egg IS the original, not a tribute.
func drawHandset(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    let lift: CGFloat = -0.4
    let beadY = 11.8 + lift
    let halfChord: CGFloat = 6.5
    let arcR: CGFloat = 6.62
    let drop = (arcR * arcR - halfChord * halfChord).squareRoot()
    let center = CGPoint(x: g.px(12), y: g.py(beadY + drop))
    let toEnd = atan2(drop, halfChord)
    let toStart = CGFloat.pi - toEnd

    ctx.setStrokeColor(color(spec.ink, alpha: 0.96))
    ctx.setLineWidth(g.u(2))
    ctx.setLineCap(.round)
    ctx.beginPath()
    ctx.addArc(center: center, radius: g.u(arcR),
               startAngle: toStart, endAngle: toEnd, clockwise: true)
    ctx.strokePath()

    ctx.setFillColor(color(spec.ink, alpha: 0.96))
    for beadX in [12 - halfChord, 12 + halfChord] {
        ctx.fillEllipse(in: CGRect(x: g.px(beadX) - g.u(2), y: g.py(beadY) - g.u(2),
                                   width: g.u(4), height: g.u(4)))
    }

    drawCursorBlock(ctx, g, spec: spec,
                    rect: CGRect(x: 9.5, y: 15.9 + lift, width: 5, height: 3))
}

/// ":wq" — write and quit. Nothing else on the tile; the joke needs no help.
func drawWq(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    drawMonoText(ctx, g, text: ":wq", size: 9,
                 center: CGPoint(x: 12, y: 12),
                 fill: color(spec.glow), glow: color(spec.glow, alpha: 0.75))
}

/// "⌃b" on a keycap — drawn like the shortcut bar's keycaps, one level up.
func drawPrefixKey(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    let cap = CGRect(x: g.px(5.2), y: g.py(17.2), width: g.u(13.6), height: g.u(10.4))
    let path = CGPath(roundedRect: cap, cornerWidth: g.u(2.4), cornerHeight: g.u(2.4),
                      transform: nil)
    ctx.setFillColor(color(spec.ink, alpha: 0.10))
    ctx.addPath(path)
    ctx.fillPath()
    ctx.setStrokeColor(color(spec.ink, alpha: 0.30))
    ctx.setLineWidth(g.u(0.9))
    ctx.addPath(path)
    ctx.strokePath()

    drawMonoText(ctx, g, text: "⌃b", size: 6,
                 center: CGPoint(x: 12, y: 12),
                 fill: color(spec.glow), glow: color(spec.glow, alpha: 0.7))
}

/// A house whose door is a live cursor block — no place like 127.0.0.1.
func drawLocalhost(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    ctx.setStrokeColor(color(spec.ink, alpha: 0.92))
    ctx.setLineWidth(g.u(2))
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.beginPath()
    ctx.move(to: CGPoint(x: g.px(5), y: g.py(18)))
    ctx.addLine(to: CGPoint(x: g.px(5), y: g.py(11)))
    ctx.addLine(to: CGPoint(x: g.px(12), y: g.py(5.6)))
    ctx.addLine(to: CGPoint(x: g.px(19), y: g.py(11)))
    ctx.addLine(to: CGPoint(x: g.px(19), y: g.py(18)))
    ctx.addLine(to: CGPoint(x: g.px(5), y: g.py(18)))
    ctx.strokePath()

    // The door: same glowing cursor block every figure in the family carries.
    drawCursorBlock(ctx, g, spec: spec,
                    rect: CGRect(x: 10, y: 13.2, width: 4, height: 4.8))
}

/// "NO CARRIER" over CRT scanlines — the amber goodbye mosh made obsolete.
func drawNoCarrier(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    // Scanlines across the full tile (drawn inside the grid's inset is fine —
    // the falloff bg already darkens the edges).
    ctx.setFillColor(CGColor(red: 0, green: 0, blue: 0, alpha: 0.16))
    var y: CGFloat = 0
    while y < 24 {
        ctx.fill(CGRect(x: g.rect.minX - g.u(2), y: g.py(y),
                        width: g.side + g.u(4), height: g.u(0.5)))
        y += 1.7
    }

    drawMonoText(ctx, g, text: "NO", size: 6.4,
                 center: CGPoint(x: 12, y: 9.4),
                 fill: color(spec.glow), glow: color(spec.glow, alpha: 0.75))
    drawMonoText(ctx, g, text: "CARRIER", size: 3.6,
                 center: CGPoint(x: 12, y: 14.6),
                 fill: color(spec.glow), glow: color(spec.glow, alpha: 0.6))
}

/// Oversized cursor block, centred — the terminal reduced to its heartbeat.
func drawCursor(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    drawCursorBlock(ctx, g, spec: spec, rect: CGRect(x: 6, y: 8.5, width: 12, height: 7))
}

/// "❯))" — a prompt chevron with two hail arcs radiating from it.
func drawHail(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.setLineWidth(g.u(2.2))

    ctx.setStrokeColor(color(spec.ink, alpha: 0.96))
    ctx.beginPath()
    ctx.move(to: CGPoint(x: g.px(4.5), y: g.py(6)))
    ctx.addLine(to: CGPoint(x: g.px(11), y: g.py(12)))
    ctx.addLine(to: CGPoint(x: g.px(4.5), y: g.py(18)))
    ctx.strokePath()

    // Two concentric arcs opening rightward from the chevron's tip.
    for (radius, alpha) in [(CGFloat(5.5), CGFloat(1.0)), (CGFloat(10), CGFloat(0.7))] {
        ctx.saveGState()
        ctx.setShadow(offset: .zero, blur: g.u(2), color: color(spec.glow, alpha: 0.5))
        ctx.setStrokeColor(color(spec.glow, alpha: alpha))
        ctx.beginPath()
        ctx.addArc(center: CGPoint(x: g.px(8.5), y: g.py(12)), radius: g.u(radius),
                   startAngle: -CGFloat.pi / 4, endAngle: CGFloat.pi / 4, clockwise: false)
        ctx.strokePath()
        ctx.restoreGState()
    }
}

// MARK: - Render

func renderIcon(spec: IconSpec, pixelSize: Int) -> CGImage? {
    let size = CGFloat(pixelSize)
    guard let ctx = CGContext(
        data: nil, width: pixelSize, height: pixelSize,
        bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }

    let rect = CGRect(x: 0, y: 0, width: size, height: size)

    // Full-bleed background: iOS applies its own corner mask, so no rounding
    // here. A diagonal falloff keeps some depth without turning the icon into a
    // flat color block.
    let gradientColors = [color(spec.bg.0), color(spec.bg.1)] as CFArray
    if let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(),
                                 colors: gradientColors, locations: [0, 1]) {
        ctx.saveGState()
        ctx.addRect(rect)
        ctx.clip()
        ctx.drawLinearGradient(gradient,
                               start: CGPoint(x: 0, y: size),
                               end: CGPoint(x: size, y: 0),
                               options: [])
        ctx.restoreGState()
    }

    let g = Grid(rect: rect.insetBy(dx: size * 0.075, dy: size * 0.075))
    switch spec.pattern {
    case .pit:       drawPit(ctx, g, spec: spec)
    case .handset:   drawHandset(ctx, g, spec: spec)
    case .wq:        drawWq(ctx, g, spec: spec)
    case .prefixKey: drawPrefixKey(ctx, g, spec: spec)
    case .localhost: drawLocalhost(ctx, g, spec: spec)
    case .noCarrier: drawNoCarrier(ctx, g, spec: spec)
    case .cursor:    drawCursor(ctx, g, spec: spec)
    case .hail:      drawHail(ctx, g, spec: spec)
    }

    return ctx.makeImage()
}

/// Redraws an image into a context that has no alpha channel at all.
///
/// The App Store's 1024 marketing icon is rejected on upload (ITMS-90717) if
/// its PNG carries an alpha channel — even a fully opaque one, which is exactly
/// what the `premultipliedLast` render context above produces. Every icon here
/// is full-bleed, so this changes no pixel; it only changes what the PNG file
/// declares about itself. The home-screen `IconFiles` PNGs are left alone: iOS
/// composites those itself and never complains.
func flattened(_ image: CGImage) -> CGImage? {
    guard let ctx = CGContext(
        data: nil, width: image.width, height: image.height,
        bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { return nil }
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    return ctx.makeImage()
}

func writePNG(_ image: CGImage, to path: String) {
    let url = URL(fileURLWithPath: path) as CFURL
    let type: CFString
    if #available(macOS 11.0, *) {
        type = UTType.png.identifier as CFString
    } else {
        type = "public.png" as CFString
    }
    guard let dest = CGImageDestinationCreateWithURL(url, type, 1, nil) else {
        FileHandle.standardError.write("Failed to create PNG destination for \(path)\n".data(using: .utf8)!)
        return
    }
    CGImageDestinationAddImage(dest, image, nil)
    if CGImageDestinationFinalize(dest) {
        print("Wrote \(path)")
    } else {
        FileHandle.standardError.write("Failed to write \(path)\n".data(using: .utf8)!)
    }
}

let iconFilesDir = "Moshpit/App/IconFiles"
let appIconSetDir = "Moshpit/App/Assets.xcassets/AppIcon.appiconset"
try? FileManager.default.createDirectory(atPath: iconFilesDir, withIntermediateDirectories: true)

// 60pt @2x/@3x = 120px/180px (iPhone), plus 76pt @2x = 152px and
// 83.5pt @2x = 167px (iPad). The app ships for both device families
// ("1,2"), and App Store validation flags every alternate icon that lacks
// its iPad sizes (ITMS-90892) — the files must exist here AND be listed in
// project.yml's CFBundleIcons~ipad block.
//
// The iPad files MUST carry the `~ipad` device suffix in the FILENAME
// (`…76x76@2x~ipad.png`): the validator resolves loose icon files by the
// QA1686 naming convention, and correctly-sized files without the suffix
// still warn ITMS-90892 on upload — verified the hard way on build 284.
let variants: [(pointName: String, scale: String, px: Int, device: String)] = [
    ("60x60", "@2x", 120, ""), ("60x60", "@3x", 180, ""),
    ("76x76", "@2x", 152, "~ipad"), ("83.5x83.5", "@2x", 167, "~ipad"),
]

for spec in specs {
    let baseName = spec.fileTag.map { "AppIcon-\($0)" } ?? "AppIcon"
    for variant in variants {
        guard let image = renderIcon(spec: spec, pixelSize: variant.px) else {
            FileHandle.standardError.write("Render failed for \(baseName) @\(variant.px)\n".data(using: .utf8)!)
            continue
        }
        writePNG(image, to: "\(iconFilesDir)/\(baseName)\(variant.pointName)\(variant.scale)\(variant.device).png")
    }
    // The primary icon also supplies the 1024 marketing asset, flattened so the
    // App Store will accept it.
    if spec.fileTag == nil,
       let marketing = renderIcon(spec: spec, pixelSize: 1024),
       let opaque = flattened(marketing) {
        writePNG(opaque, to: "\(appIconSetDir)/AppIcon.png")
    }
}
