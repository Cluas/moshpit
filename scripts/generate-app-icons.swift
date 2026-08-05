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
// The handset-over-cursor geometry mirrors `RingdownGlyph.Metrics` in
// Ringdown/UI/Brand/RingdownMark.swift — keep the two in step.
//
// Usage: swift scripts/generate-app-icons.swift
// Output:
//   Ringdown/App/IconFiles/AppIcon60x60@{2x,3x}.png              (primary)
//   Ringdown/App/IconFiles/AppIcon-<Tag>60x60@{2x,3x}.png        (alternates)
//   Ringdown/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png  (1024 marketing)

import CoreGraphics
import ImageIO
import Foundation
#if canImport(UniformTypeIdentifiers)
import UniformTypeIdentifiers
#endif

typealias RGB = (UInt8, UInt8, UInt8)

/// Which figure to draw. Distinct compositions, not recolors — the picker is
/// meant to offer real alternatives.
enum Pattern {
    /// Handset (arc + two beads) hanging above a filled cursor block.
    case handset
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

let specs: [IconSpec] = [
    // Primary — the app's own identity.
    IconSpec(fileTag: nil, pattern: .handset, ink: nearWhite, glow: violet, bg: darkBG(violet)),
    IconSpec(fileTag: "Teal", pattern: .handset, ink: nearWhite, glow: teal, bg: darkBG(teal)),
    IconSpec(fileTag: "Green", pattern: .handset, ink: nearWhite, glow: green, bg: darkBG(green)),
    IconSpec(fileTag: "Amber", pattern: .handset, ink: nearWhite, glow: amber, bg: darkBG(amber)),
    // Inverted: the same figure, read as ink on paper.
    IconSpec(fileTag: "Daylight", pattern: .handset, ink: (0x1A, 0x1C, 0x24), glow: violet,
             bg: ((0xF2, 0xF4, 0xF8), (0xD8, 0xDD, 0xE8))),
    // No color at all — the one that pairs with any custom accent.
    IconSpec(fileTag: "Mono", pattern: .handset, ink: (0xFF, 0xFF, 0xFF), glow: (0xFF, 0xFF, 0xFF),
             bg: ((0x0B, 0x0B, 0x0D), (0x00, 0x00, 0x00))),
    // Different figures.
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

/// Handset: shallow arc with two solid end beads, over a filled cursor block.
func drawHandset(_ ctx: CGContext, _ g: Grid, spec: IconSpec) {
    let lift: CGFloat = -0.4
    let beadY = 11.8 + lift
    let halfChord: CGFloat = 6.5
    // r only just over the 6.5 minimum for a 13u chord — that near-minimum
    // radius is what makes the curve deep enough to read as a handset rather
    // than a pair of headphones.
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
    case .handset: drawHandset(ctx, g, spec: spec)
    case .cursor:  drawCursor(ctx, g, spec: spec)
    case .hail:    drawHail(ctx, g, spec: spec)
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

let iconFilesDir = "Ringdown/App/IconFiles"
let appIconSetDir = "Ringdown/App/Assets.xcassets/AppIcon.appiconset"
try? FileManager.default.createDirectory(atPath: iconFilesDir, withIntermediateDirectories: true)

// 60pt @2x/@3x = 120px/180px, matching the AppIcon60x60 convention declared in
// project.yml.
let sizes: [(scale: String, px: Int)] = [("@2x", 120), ("@3x", 180)]

for spec in specs {
    let baseName = spec.fileTag.map { "AppIcon-\($0)" } ?? "AppIcon"
    for size in sizes {
        guard let image = renderIcon(spec: spec, pixelSize: size.px) else {
            FileHandle.standardError.write("Render failed for \(baseName) @\(size.px)\n".data(using: .utf8)!)
            continue
        }
        writePNG(image, to: "\(iconFilesDir)/\(baseName)60x60\(size.scale).png")
    }
    // The primary icon also supplies the 1024 marketing asset, flattened so the
    // App Store will accept it.
    if spec.fileTag == nil,
       let marketing = renderIcon(spec: spec, pixelSize: 1024),
       let opaque = flattened(marketing) {
        writePNG(opaque, to: "\(appIconSetDir)/AppIcon.png")
    }
}
