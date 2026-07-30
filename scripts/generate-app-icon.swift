#!/usr/bin/env swift
//
// generate-app-icon.swift — Generate pixel-art Ringdown app icon
//
// Usage: swift scripts/generate-app-icon.swift
// Output: Ringdown/App/Assets.xcassets/AppIcon.appiconset/icon_1024.png
//

import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let size = 1024
let pixelSize = 64  // Each "pixel" in the art is 64x64 real pixels (16x16 grid)
let gridSize = size / pixelSize  // 16

// Color palette
func color(_ hex: UInt32) -> (CGFloat, CGFloat, CGFloat) {
    let r = CGFloat((hex >> 16) & 0xFF) / 255.0
    let g = CGFloat((hex >> 8) & 0xFF) / 255.0
    let b = CGFloat(hex & 0xFF) / 255.0
    return (r, g, b)
}

let bg = color(0x1A1B26)       // Tokyo Night background
let termGreen = color(0x9ECE6A) // Terminal green
let termCyan = color(0x7DCFFF)  // Cyan accent
let termPurple = color(0xBB9AF7) // Purple for "V"
let termBlue = color(0x7AA2F7)  // Blue
let dark = color(0x15161E)      // Darker bg
let border = color(0x414868)    // Border color

// 16x16 pixel art grid (each value is a color index)
// Design: Terminal window with ">_" prompt and "V" logo
// 0=bg, 1=dark, 2=green, 3=cyan, 4=purple, 5=blue, 6=border
let art: [[Int]] = [
    [0,0,0,1,1,1,1,1,1,1,1,1,1,0,0,0],  // top border
    [0,0,1,6,6,6,6,6,6,6,6,6,6,1,0,0],
    [0,1,6,3,5,2,6,1,1,1,1,1,1,6,1,0],  // title bar (colored dots)
    [0,1,6,6,6,6,6,6,6,6,6,6,6,6,1,0],
    [0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0],  // separator
    [0,1,1,0,0,0,0,0,0,0,0,0,0,1,1,0],  // terminal body
    [0,1,1,0,2,0,4,0,4,0,0,0,0,1,1,0],  // > V
    [0,1,1,0,2,0,0,4,0,0,0,0,0,1,1,0],  // > V (bottom)
    [0,1,1,0,0,0,0,0,0,0,0,0,0,1,1,0],
    [0,1,1,0,3,3,0,5,5,5,0,5,0,1,1,0],  // $ ssh
    [0,1,1,0,0,0,0,0,0,0,0,0,0,1,1,0],
    [0,1,1,0,2,0,3,3,3,0,0,0,0,1,1,0],  // > ___  (cursor)
    [0,1,1,0,0,0,0,0,0,0,0,0,0,1,1,0],
    [0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0],  // bottom border
    [0,0,1,1,1,1,1,1,1,1,1,1,1,1,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
]

let colors: [(CGFloat, CGFloat, CGFloat)] = [bg, dark, termGreen, termCyan, termPurple, termBlue, border]

// Create CGContext
let colorSpace = CGColorSpaceCreateDeviceRGB()
guard let ctx = CGContext(
    data: nil,
    width: size,
    height: size,
    bitsPerComponent: 8,
    bytesPerRow: size * 4,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    print("Failed to create context")
    exit(1)
}

// Fill background with rounded rect feel
let (bgR, bgG, bgB) = bg
ctx.setFillColor(red: bgR, green: bgG, blue: bgB, alpha: 1.0)
ctx.fill(CGRect(x: 0, y: 0, width: size, height: size))

// Draw pixel art
for row in 0..<gridSize {
    for col in 0..<gridSize {
        let colorIndex = art[row][col]
        let (r, g, b) = colors[colorIndex]
        ctx.setFillColor(red: r, green: g, blue: b, alpha: 1.0)
        ctx.fill(CGRect(
            x: col * pixelSize,
            y: row * pixelSize,
            width: pixelSize,
            height: pixelSize
        ))
    }
}

// Save as PNG
guard let image = ctx.makeImage() else {
    print("Failed to create image")
    exit(1)
}

let outputDir = "Ringdown/App/Assets.xcassets/AppIcon.appiconset"
let outputPath = "\(outputDir)/icon_1024.png"

// Ensure directory exists
try? FileManager.default.createDirectory(atPath: outputDir, withIntermediateDirectories: true)

guard let dest = CGImageDestinationCreateWithURL(
    URL(fileURLWithPath: outputPath) as CFURL,
    "public.png" as CFString,
    1, nil
) else {
    print("Failed to create image destination")
    exit(1)
}

CGImageDestinationAddImage(dest, image, nil)
if CGImageDestinationFinalize(dest) {
    print("App icon generated: \(outputPath)")
} else {
    print("Failed to write image")
    exit(1)
}

// Update Contents.json to reference the icon
let contentsJson = """
{
  "images" : [
    {
      "filename" : "icon_1024.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    }
  ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
"""

try? contentsJson.write(toFile: "\(outputDir)/Contents.json", atomically: true, encoding: .utf8)
print("Contents.json updated")
