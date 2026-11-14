import CoreGraphics
import CoreText
import MoshpitFFI
import UIKit

// What a core terminal view keeps between frames and how its cells measure.
// The core (crates/term) parses on its own thread and hands the view frames:
// copies of the rows that changed since the last one, every cell's colours
// resolved. The view keeps the rows it was given (``CoreTermRows``) and
// draws from them: the Metal canvas re-encodes only the rows a frame
// changed, the CoreGraphics canvas invalidates only those rows.

/// A cell grid's metrics in points for one font, on the device's pixel grid
/// (the same box ``TerminalCellGeometry/measuredCell(font:scale:)`` gives,
/// so the app's grid arithmetic and the view's agree).
struct CoreTermMetrics: Equatable {
    let font: UIFont
    let bold: UIFont
    let italic: UIFont
    let boldItalic: UIFont
    let scale: CGFloat
    /// The cell, in points (whole pixels).
    let width: CGFloat
    let height: CGFloat
    /// From the cell's top to the baseline, in points (whole pixels).
    let ascent: CGFloat
    /// The descender, in points.
    let descent: CGFloat

    init(font: UIFont, scale: CGFloat) {
        let scale = max(scale, 1)
        self.font = font
        self.scale = scale
        func variant(_ traits: UIFontDescriptor.SymbolicTraits) -> UIFont {
            let base = font.fontDescriptor
            guard let d = base.withSymbolicTraits(base.symbolicTraits.union(traits)) else { return font }
            // Keep the CJK cascade and the ligatures off (TerminalFont).
            var attrs = d.fontAttributes
            for key in [UIFontDescriptor.AttributeName.cascadeList, .featureSettings] {
                if let v = base.fontAttributes[key] { attrs[key] = v }
            }
            return UIFont(descriptor: UIFontDescriptor(fontAttributes: attrs), size: font.pointSize)
        }
        bold = variant(.traitBold)
        italic = variant(.traitItalic)
        boldItalic = variant([.traitBold, .traitItalic])
        let cell = TerminalCellGeometry.measuredCell(font: font, scale: scale)
        width = cell.width
        height = cell.height
        let ct = font as CTFont
        let up = CTFontGetAscent(ct)
        let down = CTFontGetDescent(ct)
        let gap = CTFontGetLeading(ct)
        // The line box is the cell; the leading splits above and below.
        ascent = ((up + gap / 2) * scale).rounded() / scale
        descent = down
    }

    func font(bold isBold: Bool, italic isItalic: Bool) -> UIFont {
        switch (isBold, isItalic) {
        case (true, true): boldItalic
        case (true, false): bold
        case (false, true): italic
        default: font
        }
    }

    /// Whole pixels.
    var widthPx: Int { Int((width * scale).rounded()) }
    var heightPx: Int { Int((height * scale).rounded()) }
    var ascentPx: Int { Int((ascent * scale).rounded()) }

    static func == (a: CoreTermMetrics, b: CoreTermMetrics) -> Bool {
        a.font.fontName == b.font.fontName && a.font.pointSize == b.font.pointSize && a.scale == b.scale
    }
}

/// A cell flag (moshpit_term.h's MP_CELL_*) is set.
@inline(__always) func coreHas(_ cell: MpDrawCell, _ flag: Int) -> Bool { Int(cell.flags) & flag != 0 }

extension UIColor {
    /// 0xRRGGBB.
    var coreRGB: UInt32 {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        getRed(&r, green: &g, blue: &b, alpha: &a)
        func byte(_ v: CGFloat) -> UInt32 { UInt32(max(0, min(1, v)) * 255 + 0.5) }
        return byte(r) << 16 | byte(g) << 8 | byte(b)
    }
}

func coreCGColor(_ rgb: UInt32, alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255, alpha: alpha)
}

/// The look the view adds to the core's colours: the user's cursor (shape,
/// blink and colour are the user's, whatever DECSCUSR or OSC 12 the remote
/// sends — the SwiftTerm path enforces the same), the selection and search
/// tints, what lies outside the grid.
struct CoreTermStyle: Equatable {
    enum Cursor: Equatable { case block, underline, bar }
    var background: UInt32 = 0x000000
    var foreground: UInt32 = 0xE5E5E5
    var cursorColor: UInt32 = 0x5FE3D8
    var cursor: Cursor = .block
    var cursorBlink = true
    var selection: UInt32 = 0x264F78
    var match: UInt32 = 0x7A5F00
    /// What lies outside the grid (the margins its columns and rows leave),
    /// when it isn't ``background`` (B495): the colour the grid shows at
    /// its edges, which a remote can make other than the theme's.
    var ground: UInt32?

    /// The colour outside the grid.
    var outside: UInt32 { ground ?? background }

    /// A cell's background as drawn: the selection and a search match tint it.
    @inline(__always) func background(of cell: MpDrawCell) -> UInt32 {
        coreHas(cell, Int(MP_CELL_SELECTED)) ? selection : coreHas(cell, Int(MP_CELL_MATCH)) ? match : cell.bg
    }
}

/// The rows a view shows, kept between frames: a frame carries only what
/// changed, the view keeps the rest.
final class CoreTermRows {
    struct Row {
        var cells: [MpDrawCell] = []
        var extra: [UInt32] = []
        var wrapped = false
        /// Images over the row's cells; `id` keys ``CoreTermRows/images``.
        var images: [(run: MpImageRun, id: UInt64)] = []
    }

    /// A decoded image: RGBA, 8 bits a channel.
    struct Image {
        let width: Int
        let height: Int
        let rgba: Data
    }

    private(set) var rows: [Row] = []
    private(set) var cols = 0
    /// The grid's rows (``rows`` may hold one more: the row under it).
    private(set) var gridRows = 0
    private(set) var info = MpFrameInfo()
    /// Decoded images by id, as long as a row shows them.
    private(set) var images: [UInt64: Image] = [:]
    /// Bytes of the decoded images kept (debug probes).
    var imageBytes: Int { images.values.reduce(0) { $0 + $1.rgba.count } }
    /// Bumped whenever the grid's shape changes (every row is new).
    private(set) var shape = 0

    /// Takes a frame in: the rows drawn last move by the frame's shift (the
    /// view or the screen scrolled), then the rows it carries replace theirs.
    /// The rows it changed (every row after a resize), and the shift.
    @discardableResult
    func apply(_ frame: OpaquePointer) -> (changed: [Int], shift: Int) {
        let info = mp_frame_info(frame)
        self.info = info
        // A view scrolled back by the pixel shows the row under the
        // viewport too: the core sends it.
        let wanted = Int(info.rows) + (info.display_offset > 0 ? 1 : 0)
        var reshaped = false
        if wanted != rows.count || Int(info.cols) != cols {
            rows = Array(repeating: Row(), count: wanted)
            cols = Int(info.cols)
            gridRows = Int(info.rows)
            shape += 1
            reshaped = true
        }
        var shift = 0
        // Rows with images that went (scrolled out, or replaced below): the
        // decoded images only they showed are let go after.
        var hadImages = false
        if !reshaped, !info.full, info.shift != 0, abs(Int(info.shift)) < rows.count {
            shift = Int(info.shift)
            // The rows the shift drops held images too: without this, an
            // image that scrolled away by a shift stayed decoded in the view
            // for good (14.4 MB a pane on the iPad, B297).
            let gone = shift > 0 ? rows.prefix(shift) : rows.suffix(-shift)
            if gone.contains(where: { !$0.images.isEmpty }) { hadImages = true }
            if shift > 0 {
                rows.removeFirst(shift)
                rows.append(contentsOf: Array(repeating: Row(), count: shift))
            } else {
                rows.removeLast(-shift)
                rows.insert(contentsOf: Array(repeating: Row(), count: -shift), at: 0)
            }
        }
        var ids: [UInt64] = []
        var img = MpImage(id: 0, width: 0, height: 0, rgba: nil, len: 0)
        for i in 0..<info.images where mp_frame_image(frame, i, &img) {
            ids.append(img.id)
            if images[img.id] == nil, let rgba = img.rgba, img.len >= Int(img.width) * Int(img.height) * 4 {
                images[img.id] = Image(width: Int(img.width), height: Int(img.height),
                                       rgba: Data(bytes: rgba, count: img.len))
            }
        }
        var changed: [Int] = []
        changed.reserveCapacity(Int(info.changed))
        var r = MpFrameRow(row: 0, wrapped: false, cells: nil, ncells: 0, extra: nil, nextra: 0, images: nil, nimages: 0)
        for i in 0..<info.changed where mp_frame_row(frame, i, &r) {
            let index = Int(r.row)
            guard index < rows.count else { continue }
            if !rows[index].images.isEmpty { hadImages = true }
            rows[index].cells = Array(UnsafeBufferPointer(start: r.cells, count: Int(r.ncells)))
            rows[index].extra = r.nextra > 0 ? Array(UnsafeBufferPointer(start: r.extra, count: Int(r.nextra))) : []
            rows[index].wrapped = r.wrapped
            rows[index].images = r.nimages == 0 ? [] : UnsafeBufferPointer(start: r.images, count: Int(r.nimages)).compactMap { run in
                let k = Int(run.image) - 1
                return k >= 0 && k < ids.count ? (run, ids[k]) : nil
            }
            changed.append(index)
        }
        // Let go of images no row shows any more.
        if (hadImages || !images.isEmpty) && (reshaped || info.full || hadImages) {
            var shown = Set<UInt64>()
            for row in rows { for (_, id) in row.images { shown.insert(id) } }
            if shown.count != images.count { images = images.filter { shown.contains($0.key) } }
        }
        return (changed, shift)
    }

    /// A cell's text: its first scalar and the ones joined to it.
    func text(of cell: MpDrawCell, in row: Row) -> String {
        var s = String.UnicodeScalarView()
        if let c = Unicode.Scalar(cell.ch) { s.append(c) }
        if cell.extra > 0 {
            let start = Int(cell.extra_at)
            for k in start..<start + Int(cell.extra) where k < row.extra.count {
                if let c = Unicode.Scalar(row.extra[k]) { s.append(c) }
            }
        }
        return String(s)
    }

    /// A blank grid of `cols` × `rows` (before the first frame).
    func reset(cols: Int, rows count: Int, background: UInt32) {
        var blank = MpDrawCell()
        blank.ch = 0x20
        blank.bg = background
        blank.fg = background
        blank.underline = UInt32.max
        rows = Array(repeating: Row(cells: Array(repeating: blank, count: cols)), count: count)
        self.cols = cols
        gridRows = count
        shape += 1
    }
}

/// Draws rows of a ``CoreTermRows`` with CoreGraphics and CoreText into a
/// context whose origin is the top left: the fallback canvas, and the
/// keyboard cover's still picture.
struct CoreTermCGRenderer {
    let metrics: CoreTermMetrics
    var style: CoreTermStyle

    private func drawText(_ ctx: CGContext, _ string: String, font: UIFont, color: UInt32, x: CGFloat,
                          baseline: CGFloat, span: CGFloat) {
        let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: UIColor(cgColor: coreCGColor(color))]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: string, attributes: attrs))
        let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
        ctx.saveGState()
        ctx.textPosition = CGPoint(x: 0, y: 0)
        ctx.translateBy(x: x, y: baseline)
        // A glyph wider than its cells (a fallback font's emoji or symbol)
        // is fitted to them, its shape kept, as the Metal atlas does (B472).
        if let fit = CoreTermGlyphFit.fit(advance: width, ink: CoreTermGlyphFit.ink(of: line, in: ctx), span: span,
                                          ascent: metrics.ascent, descent: metrics.descent,
                                          scalar: string.unicodeScalars.first?.value ?? 0) {
            fit.apply(to: ctx, flipped: true)
        }
        CTLineDraw(line, ctx)
        ctx.restoreGState()
    }

    /// The row's image runs: each draws its texture rectangle over its cells
    /// (below the text when its z is negative).
    func drawImages(_ row: CoreTermRows.Row, of rows: CoreTermRows, y: CGFloat, in ctx: CGContext, above: Bool,
                    cache: inout [UInt64: CGImage]) {
        let w = metrics.width, h = metrics.height
        for (run, id) in row.images where (run.z >= 0) == above {
            if cache[id] == nil, let image = rows.images[id],
               let provider = CGDataProvider(data: image.rgba as CFData),
               let cg = CGImage(width: image.width, height: image.height, bitsPerComponent: 8, bitsPerPixel: 32,
                                bytesPerRow: image.width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                                provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) {
                cache[id] = cg
            }
            guard let image = cache[id] else { continue }
            let iw = CGFloat(image.width), ih = CGFloat(image.height)
            let crop = CGRect(x: CGFloat(run.left) * iw, y: CGFloat(run.top) * ih,
                              width: CGFloat(run.right - run.left) * iw,
                              height: CGFloat(run.bottom - run.top) * ih).integral
            guard let part = image.cropping(to: crop) else { continue }
            let pxPerCell = CGFloat(run.right - run.left) * iw / CGFloat(max(run.cols, 1))
            let scale = pxPerCell > 0 ? w / pxPerCell : 1
            let dest = CGRect(x: CGFloat(run.col) * w, y: y, width: CGFloat(run.cols) * w,
                              height: min(h, CGFloat(run.bottom - run.top) * ih * scale))
            ctx.saveGState()
            ctx.translateBy(x: 0, y: dest.minY + dest.height)
            ctx.scaleBy(x: 1, y: -1)
            ctx.draw(part, in: CGRect(x: dest.minX, y: 0, width: dest.width, height: dest.height))
            ctx.restoreGState()
        }
    }

    /// Draws row `index` (background, text, decorations, the cursor) at
    /// `y` (points from the top).
    func draw(row index: Int, of rows: CoreTermRows, y: CGFloat, cursorShown: Bool, in ctx: CGContext,
              cache: inout [UInt64: CGImage]) {
        guard index < rows.rows.count else { return }
        let row = rows.rows[index]
        let w = metrics.width, h = metrics.height
        let baseline = y + metrics.ascent
        ctx.saveGState()
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)

        // The cursor's cell, drawn as a block: its colours swap.
        let cursor = rows.info.cursor
        let blockAt: Int? = cursorShown && style.cursor == .block && cursor.visible && Int(cursor.row) == index
            ? Int(cursor.col) : nil

        var col = 0
        while col < row.cells.count {
            let colour = style.background(of: row.cells[col])
            var end = col + 1
            while end < row.cells.count, style.background(of: row.cells[end]) == colour { end += 1 }
            ctx.setFillColor(coreCGColor(colour))
            ctx.fill(CGRect(x: CGFloat(col) * w, y: y, width: CGFloat(end - col) * w, height: h))
            col = end
        }
        if let c = blockAt, c < row.cells.count {
            let wide = coreHas(row.cells[c], Int(MP_CELL_WIDE))
            ctx.setFillColor(coreCGColor(style.cursorColor))
            ctx.fill(CGRect(x: CGFloat(c) * w, y: y, width: (wide ? 2 : 1) * w, height: h))
        }

        drawImages(row, of: rows, y: y, in: ctx, above: false, cache: &cache)

        col = 0
        while col < row.cells.count {
            let cell = row.cells[col]
            if coreHas(cell, Int(MP_CELL_SPACER)) || (cell.ch == 0x20 && cell.extra == 0) || cell.ch == 0 {
                col += 1
                continue
            }
            let font = metrics.font(bold: coreHas(cell, Int(MP_CELL_BOLD)), italic: coreHas(cell, Int(MP_CELL_ITALIC)))
            let fg = blockAt == col ? style.background(of: cell) : cell.fg
            let wide = coreHas(cell, Int(MP_CELL_WIDE))
            drawText(ctx, rows.text(of: cell, in: row), font: font, color: fg, x: CGFloat(col) * w,
                     baseline: baseline, span: (wide ? 2 : 1) * w)
            col += 1
        }

        drawImages(row, of: rows, y: y, in: ctx, above: true, cache: &cache)

        let line = max(1 / metrics.scale, (metrics.scale >= 3 ? 1 : 0.5))
        for (c, cell) in row.cells.enumerated() where Int(cell.flags) & 0x20FC != 0 {
            let x = CGFloat(c) * w
            let color = coreCGColor(cell.underline == UInt32.max ? cell.fg : cell.underline)
            ctx.setFillColor(color)
            let uy = baseline + max(1, metrics.descent / 2)
            if coreHas(cell, Int(MP_CELL_UNDERLINE)) || coreHas(cell, Int(MP_CELL_LINK)) {
                ctx.fill(CGRect(x: x, y: uy, width: w, height: line))
            }
            if coreHas(cell, Int(MP_CELL_DOUBLE_UNDERLINE)) {
                ctx.fill(CGRect(x: x, y: uy, width: w, height: line))
                ctx.fill(CGRect(x: x, y: uy + 2 * line, width: w, height: line))
            }
            if coreHas(cell, Int(MP_CELL_UNDERCURL)) {
                ctx.setStrokeColor(color)
                ctx.setLineWidth(line)
                ctx.move(to: CGPoint(x: x, y: uy))
                ctx.addCurve(to: CGPoint(x: x + w, y: uy), control1: CGPoint(x: x + w / 3, y: uy - 2),
                             control2: CGPoint(x: x + 2 * w / 3, y: uy + 2))
                ctx.strokePath()
            }
            if coreHas(cell, Int(MP_CELL_DOTTED_UNDERLINE)) || coreHas(cell, Int(MP_CELL_DASHED_UNDERLINE)) {
                let dash: CGFloat = coreHas(cell, Int(MP_CELL_DOTTED_UNDERLINE)) ? 1 : 3
                var dx: CGFloat = 0
                while dx < w {
                    ctx.fill(CGRect(x: x + dx, y: uy, width: min(dash, w - dx), height: line))
                    dx += dash + 2
                }
            }
            if coreHas(cell, Int(MP_CELL_STRIKEOUT)) {
                ctx.fill(CGRect(x: x, y: y + (h / 2).rounded(), width: w, height: line))
            }
        }

        if cursorShown, cursor.visible, Int(cursor.row) == index, style.cursor != .block {
            let x = CGFloat(cursor.col) * w
            let wide = Int(cursor.col) < row.cells.count && coreHas(row.cells[Int(cursor.col)], Int(MP_CELL_WIDE))
            ctx.setFillColor(coreCGColor(style.cursorColor))
            let thick = max(2 / metrics.scale, 1)
            switch style.cursor {
            case .underline: ctx.fill(CGRect(x: x, y: y + h - 2 * thick, width: (wide ? 2 : 1) * w, height: 2 * thick))
            case .bar: ctx.fill(CGRect(x: x, y: y, width: 2 * thick, height: h))
            case .block: break
            }
        }
        ctx.restoreGState()
    }

    /// The whole grid as a picture (the keyboard cover).
    func image(of rows: CoreTermRows, size: CGSize, xOffset: CGFloat = 0, yOffset: CGFloat,
               cursorShown: Bool) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = metrics.scale
        format.opaque = true
        var cache: [UInt64: CGImage] = [:]
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            let ctx = context.cgContext
            ctx.setFillColor(coreCGColor(style.outside))
            ctx.fill(CGRect(origin: .zero, size: size))
            // Where the view's grid starts across it (B473).
            ctx.translateBy(x: xOffset, y: 0)
            for r in 0..<rows.rows.count {
                draw(row: r, of: rows, y: CGFloat(r) * metrics.height - yOffset, cursorShown: cursorShown,
                     in: ctx, cache: &cache)
            }
        }
    }
}
