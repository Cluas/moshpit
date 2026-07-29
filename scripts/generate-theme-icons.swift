#!/usr/bin/env swift
// Renders app icon PNGs for every AppTheme (primary Signal Room + the 3
// alternates), using plain CoreGraphics + ImageIO (no UIKit/AppKit/SwiftUI
// needed) so it runs as a standalone `swift` script. Mirrors BeaconMark's
// composition (diagonal gradient panel, slash cut, dot + radiating signal
// waves) but full-bleed — iOS applies its own corner mask to app icons, so
// no rounding here.
//
// Usage: swift scripts/generate-theme-icons.swift
// Output:
//   Beacon/App/IconFiles/AppIcon60x60@{2x,3x}.png              (primary, Signal Room)
//   Beacon/App/IconFiles/AppIcon-<Theme>60x60@{2x,3x}.png      (alternates)
//   Beacon/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png  (1024, primary/marketing)

import CoreGraphics
import ImageIO
import Foundation
#if canImport(UniformTypeIdentifiers)
import UniformTypeIdentifiers
#endif

struct IconTheme {
    /// nil fileTag = primary icon (no "-<Theme>" suffix, matches AppTheme.iconName == nil).
    let fileTag: String?
    let accent: (UInt8, UInt8, UInt8)
}

let themes: [IconTheme] = [
    IconTheme(fileTag: nil, accent: (0x6C, 0x6B, 0xEF)),           // Signal Room (primary)
    IconTheme(fileTag: "BeaconClassic", accent: (0x53, 0xDC, 0xC9)),
    IconTheme(fileTag: "TerminalGreen", accent: (0x2F, 0xA8, 0x71)),
    IconTheme(fileTag: "AmberConsole", accent: (0xC9, 0x8A, 0x2E)),
]

func color(_ rgb: (UInt8, UInt8, UInt8), alpha: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat(rgb.0) / 255, green: CGFloat(rgb.1) / 255, blue: CGFloat(rgb.2) / 255, alpha: alpha)
}

/// Blends `rgb` toward `target` by `t` (0 = unchanged, 1 = `target`).
func mix(_ rgb: (UInt8, UInt8, UInt8), toward target: (UInt8, UInt8, UInt8), by t: CGFloat) -> (UInt8, UInt8, UInt8) {
    func lerp(_ a: UInt8, _ b: UInt8) -> UInt8 {
        UInt8(max(0, min(255, CGFloat(a) + (CGFloat(b) - CGFloat(a)) * t)))
    }
    return (lerp(rgb.0, target.0), lerp(rgb.1, target.1), lerp(rgb.2, target.2))
}

func renderIcon(theme: IconTheme, pixelSize: Int) -> CGImage? {
    let size = CGFloat(pixelSize)
    guard let ctx = CGContext(
        data: nil, width: pixelSize, height: pixelSize,
        bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }

    let rect = CGRect(x: 0, y: 0, width: size, height: size)

    // 1. Full-bleed near-black background — the dark "terminal panel" from
    //    the original design, just stretched edge-to-edge instead of inset
    //    (the inset used to leave a distracting colored ring around it). A
    //    faint diagonal falloff toward the theme accent keeps some depth
    //    without turning the whole icon into a saturated color block.
    let deepShadow = mix(theme.accent, toward: (0, 0, 0), by: 0.93)
    let deepHighlight = mix(theme.accent, toward: (0, 0, 0), by: 0.82)
    let gradientColors = [color(deepHighlight), color(deepShadow)] as CFArray
    if let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: gradientColors, locations: [0, 1]) {
        ctx.saveGState()
        ctx.addRect(rect)
        ctx.clip()
        ctx.drawLinearGradient(
            gradient,
            start: CGPoint(x: 0, y: size),
            end: CGPoint(x: size, y: 0),
            options: [])
        ctx.restoreGState()
    }

    // 2. Ringdown glyph: a handset (shallow arc + two solid end beads) hanging
    //    ABOVE its cradle, which is drawn as a filled terminal cursor block —
    //    off-hook forever (the line never closes) and a shell waiting for you.
    //    Same motif as RingdownMark.swift, translated to raw CoreGraphics.
    //
    //    Geometry is authored once in the mark's 24×24 design grid and mapped
    //    here, so this and the SwiftUI mark cannot drift: `u` converts design
    //    units to pixels, `px`/`py` place a design point (with py flipping the
    //    axis, since CoreGraphics' origin is bottom-left and the grid's is
    //    top-left like SVG).
    let contentInset = size * 0.075
    let contentRect = rect.insetBy(dx: contentInset, dy: contentInset)
    let pw = contentRect.width
    /// Design units → pixels.
    func u(_ v: CGFloat) -> CGFloat { pw * v / 24 }
    func px(_ v: CGFloat) -> CGFloat { contentRect.minX + u(v) }
    /// Design-grid Y (top-left origin) → CoreGraphics Y (bottom-left origin).
    func py(_ v: CGFloat) -> CGFloat { contentRect.minY + u(24 - v) }

    // The whole glyph sits 0.8u high of the grid's centre so its optical
    // centre (arc crown → cursor baseline) lands on the canvas centre.
    let lift: CGFloat = -0.4
    let beadY = 11.8 + lift
    // Arc through (5.5, beadY) and (18.5, beadY): the 13u chord fixes the
    // centre √(r² − 6.5²) below the beads. r is only just over the 6.5
    // minimum, which is what makes the curve deep enough to read as a handset
    // rather than a pair of headphones.
    let arcR: CGFloat = 6.62
    let arcDrop = (arcR * arcR - 6.5 * 6.5).squareRoot()
    let arcCenter = CGPoint(x: px(12), y: py(beadY + arcDrop))
    let toEnd = atan2(arcDrop, CGFloat(6.5))     // right bead
    let toStart = CGFloat.pi - toEnd             // left bead

    // Handset: near-white so it reads as the physical object, with the theme
    // accent reserved for the live cursor below.
    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.94))
    ctx.setLineWidth(u(2))
    ctx.setLineCap(.round)
    ctx.beginPath()
    ctx.addArc(center: arcCenter, radius: u(arcR),
               startAngle: toStart, endAngle: toEnd, clockwise: true)
    ctx.strokePath()

    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.94))
    for beadX in [CGFloat(5.5), CGFloat(18.5)] {
        ctx.fillEllipse(in: CGRect(x: px(beadX) - u(2), y: py(beadY) - u(2),
                                   width: u(4), height: u(4)))
    }

    // Cradle-as-cursor: the accent-colored block the handset never returns to,
    // glowing like a live prompt.
    ctx.saveGState()
    ctx.setShadow(offset: .zero, blur: u(2.6), color: color(theme.accent, alpha: 0.8))
    ctx.setFillColor(color(theme.accent))
    let cursor = CGRect(x: px(9.5), y: py(18.9 + lift),
                        width: u(5), height: u(3))
    ctx.addPath(CGPath(roundedRect: cursor, cornerWidth: u(0.8), cornerHeight: u(0.8),
                       transform: nil))
    ctx.fillPath()
    ctx.restoreGState()

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

let iconFilesDir = "Beacon/App/IconFiles"
let appIconSetDir = "Beacon/App/Assets.xcassets/AppIcon.appiconset"
try? FileManager.default.createDirectory(atPath: iconFilesDir, withIntermediateDirectories: true)

// 60pt @2x/@3x = 120px/180px, matching the existing AppIcon60x60 convention
// already declared in project.yml.
let sizes: [(scale: String, px: Int)] = [("@2x", 120), ("@3x", 180)]

for theme in themes {
    let baseName = theme.fileTag.map { "AppIcon-\($0)" } ?? "AppIcon"
    for size in sizes {
        guard let image = renderIcon(theme: theme, pixelSize: size.px) else {
            FileHandle.standardError.write("Render failed for \(baseName) @\(size.px)\n".data(using: .utf8)!)
            continue
        }
        writePNG(image, to: "\(iconFilesDir)/\(baseName)60x60\(size.scale).png")
    }

    // Primary theme also gets the 1024 marketing/App-Store icon used by the
    // Assets.xcassets appiconset (ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon).
    if theme.fileTag == nil {
        if let image1024 = renderIcon(theme: theme, pixelSize: 1024) {
            writePNG(image1024, to: "\(appIconSetDir)/AppIcon.png")
        }
    }
}
