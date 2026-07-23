#!/usr/bin/env swift
// Renders alternate app icon PNGs for each non-primary AppTheme, using plain
// CoreGraphics + ImageIO (no UIKit/AppKit/SwiftUI needed) so it runs as a
// standalone `swift` script. Mirrors MoshiMark's composition (diagonal
// gradient panel, slash cut, terminal cursor glyph) but full-bleed —
// iOS applies its own corner mask to app icons, so no rounding here.
//
// Usage: swift scripts/generate-theme-icons.swift
// Output: Moshi/App/IconFiles/AppIcon-<Theme><size>@<scale>x.png

import CoreGraphics
import ImageIO
import Foundation
#if canImport(UniformTypeIdentifiers)
import UniformTypeIdentifiers
#endif

struct IconTheme {
    let fileTag: String   // matches AppTheme.iconName minus "AppIcon-"
    let accent: (UInt8, UInt8, UInt8)
    let accentPressed: (UInt8, UInt8, UInt8)
}

let themes: [IconTheme] = [
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

    // 3. Prompt glyph: a ">" chevron + a solid cursor bar, same silhouette
    //    as MoshiMark's terminal-prompt motif. Every measurement is relative
    //    to `panelRect` (not the full canvas) and the whole glyph run is
    //    sized to ~70% of the panel width so nothing crosses the panel's
    //    rounded edge, whatever `panelInset` happens to be.
    let midY = size * 0.5
    let pw = panelRect.width
    let chevronW = pw * 0.22
    let chevronH = pw * 0.28
    let gap1 = pw * 0.08
    let barW = pw * 0.24
    let gap2 = pw * 0.07
    let cursorW = pw * 0.09
    let cursorH = pw * 0.32
    let barH = pw * 0.075
    let runStartX = panelRect.minX + pw * 0.15   // (pw*0.15 both sides; run = 0.70*pw)

    let chevronX = runStartX
    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.94))
    ctx.setLineWidth(pw * 0.075)
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.beginPath()
    ctx.move(to: CGPoint(x: chevronX, y: midY + chevronH / 2))
    ctx.addLine(to: CGPoint(x: chevronX + chevronW, y: midY))
    ctx.addLine(to: CGPoint(x: chevronX, y: midY - chevronH / 2))
    ctx.strokePath()

    let barX = chevronX + chevronW + gap1
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.94))
    ctx.fill(CGRect(x: barX, y: midY - barH / 2, width: barW, height: barH))

    // 4. Accent-colored cursor block, right of the dash — echoes MoshiMark's
    //    small solid accent rectangle so the glyph isn't monochrome-on-color.
    let cursorX = barX + barW + gap2
    ctx.setFillColor(color(theme.accent))
    ctx.fill(CGRect(x: cursorX, y: midY - cursorH / 2, width: cursorW, height: cursorH))

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

let outDir = "Moshi/App/IconFiles"
try? FileManager.default.createDirectory(atPath: outDir, withIntermediateDirectories: true)

// 60pt @2x/@3x = 120px/180px, matching the existing primary AppIcon60x60
// convention already declared in project.yml.
let sizes: [(scale: String, px: Int)] = [("@2x", 120), ("@3x", 180)]

for theme in themes {
    for size in sizes {
        guard let image = renderIcon(theme: theme, pixelSize: size.px) else {
            FileHandle.standardError.write("Render failed for \(theme.fileTag) @\(size.px)\n".data(using: .utf8)!)
            continue
        }
        writePNG(image, to: "\(outDir)/AppIcon-\(theme.fileTag)60x60\(size.scale).png")
    }
}
