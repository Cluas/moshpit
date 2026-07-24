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
//   Moshi/App/IconFiles/AppIcon60x60@{2x,3x}.png              (primary, Signal Room)
//   Moshi/App/IconFiles/AppIcon-<Theme>60x60@{2x,3x}.png      (alternates)
//   Moshi/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png  (1024, primary/marketing)

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
    let accentPressed: (UInt8, UInt8, UInt8)
}

let themes: [IconTheme] = [
    IconTheme(fileTag: nil, accent: (0x6C, 0x6B, 0xEF), accentPressed: (0x56, 0x52, 0xD6)),           // Signal Room (primary)
    IconTheme(fileTag: "MoshiClassic", accent: (0x53, 0xDC, 0xC9), accentPressed: (0x3E, 0xBF, 0xA9)),
    IconTheme(fileTag: "TerminalGreen", accent: (0x2F, 0xA8, 0x71), accentPressed: (0x25, 0x86, 0x5A)),
    IconTheme(fileTag: "AmberConsole", accent: (0xC9, 0x8A, 0x2E), accentPressed: (0xA6, 0x6F, 0x1F)),
]

func color(_ rgb: (UInt8, UInt8, UInt8), alpha: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat(rgb.0) / 255, green: CGFloat(rgb.1) / 255, blue: CGFloat(rgb.2) / 255, alpha: alpha)
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

    // 1. Diagonal gradient background, accent (top-left) -> accentPressed
    //    (bottom-right) — monochromatic-per-theme so each icon reads as a
    //    distinct color at a glance, not just a recolored detail.
    let gradientColors = [color(theme.accent), color(theme.accentPressed)] as CFArray
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

    // 2. Inner terminal panel — dark rounded rect, ~78% of the canvas.
    let panelInset = size * 0.11
    let panelRect = rect.insetBy(dx: panelInset, dy: panelInset)
    let panelPath = CGPath(roundedRect: panelRect, cornerWidth: size * 0.14, cornerHeight: size * 0.14, transform: nil)
    ctx.setFillColor(color((0x05, 0x05, 0x07), alpha: 0.92))
    ctx.addPath(panelPath)
    ctx.fillPath()

    // 3. Beacon glyph: a light source (filled dot) with two signal waves
    //    radiating outward — same motif as BeaconMark.swift, translated to
    //    raw CoreGraphics. Both arcs share the dot's center so they read as
    //    concentric; sized off `panelRect.width` (not the full canvas) and
    //    centered within it so nothing crosses the panel's rounded edge.
    let midY = size * 0.5
    let pw = panelRect.width
    let dotRadius = pw * 0.08
    let innerWaveRadius = pw * 0.17
    let outerWaveRadius = pw * 0.28
    let totalWidth = dotRadius + outerWaveRadius
    let leftMargin = (pw - totalWidth) / 2
    let originX = panelRect.minX + leftMargin + dotRadius

    let waveStart = CGFloat(-55.0 * .pi / 180)
    let waveEnd = CGFloat(55.0 * .pi / 180)

    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.5))
    ctx.setLineWidth(pw * 0.038)
    ctx.setLineCap(.round)
    ctx.beginPath()
    ctx.addArc(center: CGPoint(x: originX, y: midY), radius: outerWaveRadius,
               startAngle: waveStart, endAngle: waveEnd, clockwise: false)
    ctx.strokePath()

    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.92))
    ctx.setLineWidth(pw * 0.045)
    ctx.setLineCap(.round)
    ctx.beginPath()
    ctx.addArc(center: CGPoint(x: originX, y: midY), radius: innerWaveRadius,
               startAngle: waveStart, endAngle: waveEnd, clockwise: false)
    ctx.strokePath()

    // Accent-colored dot last, on top of the wave arcs' inner ends.
    ctx.setFillColor(color(theme.accent))
    ctx.fillEllipse(in: CGRect(x: originX - dotRadius, y: midY - dotRadius,
                                width: dotRadius * 2, height: dotRadius * 2))

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

let iconFilesDir = "Moshi/App/IconFiles"
let appIconSetDir = "Moshi/App/Assets.xcassets/AppIcon.appiconset"
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
