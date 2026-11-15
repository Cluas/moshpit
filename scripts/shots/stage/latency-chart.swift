// The chart the staged Claude turn says it wrote (bench/latency.png): what
// the store's file-preview shot opens from the pane. Drawn here, the same
// every run, so no image of anyone's sits in the repo.
//
//   swift scripts/shots/stage/latency-chart.swift <out.png>

import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size = CGSize(width: 1600, height: 1000)
let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "latency.png")
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: Int(size.width), height: Int(size.height), bitsPerComponent: 8,
                    bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

func text(_ string: String, at point: CGPoint, size: CGFloat, color: CGColor, bold: Bool = false,
          align: CGFloat = 0) {
    let font = CTFontCreateUIFontForLanguage(bold ? .emphasizedSystem : .system, size, "en" as CFString)!
    let attributes: [CFString: Any] = [kCTFontAttributeName: font, kCTForegroundColorAttributeName: color]
    let line = CTLineCreateWithAttributedString(
        CFAttributedStringCreate(nil, string as CFString, attributes as CFDictionary)!)
    let width = CTLineGetTypographicBounds(line, nil, nil, nil)
    ctx.textPosition = CGPoint(x: point.x - CGFloat(width) * align, y: point.y)
    CTLineDraw(line, ctx)
}

// White page, as a plotting library leaves it.
ctx.setFillColor(color(0xFFFFFF))
ctx.fill(CGRect(origin: .zero, size: size))

let plot = CGRect(x: 150, y: 140, width: 1360, height: 660)
text("Webhook delivery latency, p99 (s)", at: CGPoint(x: 150, y: 900), size: 44, color: color(0x1F2328), bold: true)
text("1,000 replayed events against a rate-limited endpoint", at: CGPoint(x: 150, y: 852), size: 28,
     color: color(0x656D76))

// Grid and axes: 0–10 s.
let maxY: CGFloat = 10
ctx.setLineWidth(2)
for tick in stride(from: 0, through: 10, by: 2) {
    let y = plot.minY + plot.height * CGFloat(tick) / maxY
    ctx.setStrokeColor(color(0xD0D7DE, tick == 0 ? 1 : 0.6))
    ctx.move(to: CGPoint(x: plot.minX, y: y))
    ctx.addLine(to: CGPoint(x: plot.maxX, y: y))
    ctx.strokePath()
    text("\(tick)", at: CGPoint(x: plot.minX - 24, y: y - 10), size: 26, color: color(0x656D76), align: 1)
}
let minutes = 12
for minute in stride(from: 0, through: minutes, by: 2) {
    let x = plot.minX + plot.width * CGFloat(minute) / CGFloat(minutes)
    text("\(minute)m", at: CGPoint(x: x, y: plot.minY - 44), size: 26, color: color(0x656D76), align: 0.5)
}

// Before: three tries, no delay — spikes to the cap of nothing. After:
// backoff with jitter, capped at 30 s, settling under 2 s.
let before: [CGFloat] = [2.1, 6.8, 9.4, 4.2, 8.9, 9.8, 3.6, 7.7, 9.1, 5.4, 9.6, 8.2, 6.9]
let after: [CGFloat] = [2.0, 2.6, 1.9, 1.7, 2.2, 1.8, 1.6, 1.9, 1.7, 1.5, 1.8, 1.6, 1.5]
func series(_ values: [CGFloat], _ stroke: CGColor, dashed: Bool) {
    ctx.setStrokeColor(stroke)
    ctx.setLineWidth(6)
    ctx.setLineJoin(.round)
    ctx.setLineCap(.round)
    ctx.setLineDash(phase: 0, lengths: dashed ? [18, 14] : [])
    for (i, value) in values.enumerated() {
        let point = CGPoint(x: plot.minX + plot.width * CGFloat(i) / CGFloat(values.count - 1),
                            y: plot.minY + plot.height * value / maxY)
        if i == 0 { ctx.move(to: point) } else { ctx.addLine(to: point) }
    }
    ctx.strokePath()
    ctx.setLineDash(phase: 0, lengths: [])
}
series(before, color(0xCF222E), dashed: true)
series(after, color(0x1A7F37), dashed: false)

// Legend.
func key(_ label: String, _ stroke: CGColor, dashed: Bool, x: CGFloat) {
    ctx.setStrokeColor(stroke)
    ctx.setLineWidth(6)
    ctx.setLineDash(phase: 0, lengths: dashed ? [14, 10] : [])
    ctx.move(to: CGPoint(x: x, y: 70))
    ctx.addLine(to: CGPoint(x: x + 60, y: 70))
    ctx.strokePath()
    ctx.setLineDash(phase: 0, lengths: [])
    text(label, at: CGPoint(x: x + 76, y: 60), size: 28, color: color(0x1F2328))
}
key("main: 3 tries, no delay", color(0xCF222E), dashed: true, x: 150)
key("fix-webhook-retry: backoff, cap 30s", color(0x1A7F37), dashed: false, x: 620)

let image = ctx.makeImage()!
let destination = CGImageDestinationCreateWithURL(out as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, image, nil)
guard CGImageDestinationFinalize(destination) else { exit(1) }
