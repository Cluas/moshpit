import CoreText
import Metal
import MoshpitFFI
import MoshpitKit
import QuartzCore
import UIKit

// The Metal canvas of a core terminal view. One instance per cell — its
// background, its glyph from a shared atlas, its colours — kept in a
// CPU-side array that a frame updates only for the rows it changed (B20:
// an echoed key re-encodes one row, not the screen); the GPU then draws the
// whole grid from it in four instanced passes, which costs it next to
// nothing. Decorations (underlines, strikeout, a beam or underline cursor)
// are rectangles per row, images (kitty, sixel, iTerm2) textured quads per
// run of cells. Glyphs are rasterised once by CoreText into an atlas shared
// by every view with the same font, size and scale: alpha coverage for
// text (tinted by the cell's colour in the shader), colour for emoji.

/// The device, queue and pipelines, made once. `nil` when this device has
/// no Metal or the shaders do not compile: views fall back to CoreGraphics.
final class CoreTermGPU {
    let device: MTLDevice
    let queue: MTLCommandQueue
    let background: MTLRenderPipelineState
    let glyphs: MTLRenderPipelineState
    let rects: MTLRenderPipelineState
    let images: MTLRenderPipelineState
    let sampler: MTLSamplerState
    let linear: MTLSamplerState

    static let pixelFormat = MTLPixelFormat.bgra8Unorm

    @MainActor static let shared: CoreTermGPU? = {
        let start = CACurrentMediaTime()
        let gpu = CoreTermGPU()
        Log.ui.info("core term: Metal \(gpu != nil ? "ready" : "unavailable", privacy: .public) in \(Int((CACurrentMediaTime() - start) * 1000), privacy: .public) ms")
        return gpu
    }()

    private init?() {
        guard let device = MTLCreateSystemDefaultDevice(), let queue = device.makeCommandQueue() else { return nil }
        let library: MTLLibrary
        do {
            library = try device.makeLibrary(source: Self.shaders, options: nil)
        } catch {
            Log.ui.error("core term: shaders: \(String(describing: error), privacy: .public)")
            return nil
        }
        func pipeline(_ vertex: String, _ fragment: String) -> MTLRenderPipelineState? {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = library.makeFunction(name: vertex)
            d.fragmentFunction = library.makeFunction(name: fragment)
            let a = d.colorAttachments[0]!
            a.pixelFormat = Self.pixelFormat
            a.isBlendingEnabled = true
            a.rgbBlendOperation = .add
            a.alphaBlendOperation = .add
            a.sourceRGBBlendFactor = .one
            a.sourceAlphaBlendFactor = .one
            a.destinationRGBBlendFactor = .oneMinusSourceAlpha
            a.destinationAlphaBlendFactor = .oneMinusSourceAlpha
            return try? device.makeRenderPipelineState(descriptor: d)
        }
        guard let bg = pipeline("cell_bg_vertex", "color_fragment"),
              let glyphs = pipeline("cell_glyph_vertex", "glyph_fragment"),
              let rects = pipeline("rect_vertex", "color_fragment"),
              let images = pipeline("image_vertex", "image_fragment") else { return nil }
        let s = MTLSamplerDescriptor()
        s.minFilter = .nearest
        s.magFilter = .nearest
        guard let sampler = device.makeSamplerState(descriptor: s) else { return nil }
        let l = MTLSamplerDescriptor()
        l.minFilter = .linear
        l.magFilter = .linear
        guard let linear = device.makeSamplerState(descriptor: l) else { return nil }
        self.device = device
        self.queue = queue
        self.background = bg
        self.glyphs = glyphs
        self.rects = rects
        self.images = images
        self.sampler = sampler
        self.linear = linear
    }

    /// Per cell, eight words: position (col | row << 16), the glyph's atlas
    /// x | y << 16 and w | h << 16 (w 0: none), its offset from the cell's
    /// top left (x | y << 16, signed), foreground, background (0xRRGGBB),
    /// flags (1: a colour glyph), padding.
    static let shaders = """
    #include <metal_stdlib>
    using namespace metal;

    struct Uniforms {
        float2 viewport;   // pixels
        float2 cell;       // pixels
        float2 mono;       // atlas sizes
        float2 color;
        float yOffset;     // pixels, added to every row
        float cols;        // the grid's columns: an instance's cell is its index
    };

    struct Cell { uint pos; uint gxy; uint gwh; uint off; uint fg; uint bg; uint flags; uint pad; };
    struct Rect { float4 frame; uint color; uint pad0; uint pad1; uint pad2; };
    struct Image { float4 frame; float4 uv; };

    struct VOut {
        float4 position [[position]];
        float4 color;
        float2 uv;
        uint flags [[flat]];
    };

    static float4 rgb(uint v) {
        return float4(float((v >> 16) & 0xff) / 255.0, float((v >> 8) & 0xff) / 255.0, float(v & 0xff) / 255.0, 1.0);
    }

    static float4 clip(float2 p, float2 viewport) {
        return float4(p.x / viewport.x * 2.0 - 1.0, 1.0 - p.y / viewport.y * 2.0, 0.0, 1.0);
    }

    static float2 corner(uint vid) { return float2(float(vid & 1), float(vid >> 1)); }

    vertex VOut cell_bg_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                               constant Uniforms& u [[buffer(0)]], const device Cell* cells [[buffer(1)]]) {
        Cell c = cells[iid];
        uint n = uint(u.cols);
        float2 origin = float2(float(iid % n), float(iid / n)) * u.cell;
        origin.y += u.yOffset;
        VOut out;
        out.position = clip(origin + corner(vid) * u.cell, u.viewport);
        out.color = rgb(c.bg);
        out.uv = float2(0);
        out.flags = 0;
        return out;
    }

    vertex VOut cell_glyph_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                                  constant Uniforms& u [[buffer(0)]], const device Cell* cells [[buffer(1)]]) {
        Cell c = cells[iid];
        float2 size = float2(float(c.gwh & 0xffff), float(c.gwh >> 16));
        float2 atlas = float2(float(c.gxy & 0xffff), float(c.gxy >> 16));
        float2 offset = float2(float(short(c.off & 0xffff)), float(short(c.off >> 16)));
        uint n = uint(u.cols);
        float2 origin = float2(float(iid % n), float(iid / n)) * u.cell + offset;
        origin.y += u.yOffset;
        float2 k = corner(vid);
        VOut out;
        // No glyph: every corner on one point, nothing drawn.
        out.position = clip(origin + k * size, u.viewport);
        float2 sizeOf = (c.flags & 1) != 0 ? u.color : u.mono;
        out.uv = (atlas + k * size) / sizeOf;
        out.color = rgb(c.fg);
        out.flags = c.flags;
        return out;
    }

    fragment float4 color_fragment(VOut in [[stage_in]]) {
        return float4(in.color.rgb * in.color.a, in.color.a);
    }

    fragment float4 glyph_fragment(VOut in [[stage_in]], texture2d<float> mono [[texture(0)]],
                                   texture2d<float> color [[texture(1)]], sampler s [[sampler(0)]]) {
        if ((in.flags & 1) != 0) {
            return color.sample(s, in.uv);  // premultiplied
        }
        float a = mono.sample(s, in.uv).r;
        return float4(in.color.rgb * a, a);
    }

    vertex VOut rect_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                            constant Uniforms& u [[buffer(0)]], const device Rect* rects [[buffer(1)]]) {
        Rect r = rects[iid];
        float2 origin = r.frame.xy;
        origin.y += u.yOffset;
        VOut out;
        out.position = clip(origin + corner(vid) * r.frame.zw, u.viewport);
        out.color = rgb(r.color);
        out.uv = float2(0);
        out.flags = 0;
        return out;
    }

    vertex VOut image_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                             constant Uniforms& u [[buffer(0)]], const device Image* images [[buffer(1)]]) {
        Image i = images[iid];
        float2 origin = i.frame.xy;
        origin.y += u.yOffset;
        float2 k = corner(vid);
        VOut out;
        out.position = clip(origin + k * i.frame.zw, u.viewport);
        out.uv = i.uv.xy + k * (i.uv.zw - i.uv.xy);
        out.color = float4(1);
        out.flags = 0;
        return out;
    }

    fragment float4 image_fragment(VOut in [[stage_in]], texture2d<float> image [[texture(0)]], sampler s [[sampler(0)]]) {
        float4 c = image.sample(s, in.uv);
        return float4(c.rgb * c.a, c.a);
    }
    """
}

/// Where a glyph lives in the atlas, and where its box sits relative to its
/// cell's top left (pixels).
struct CoreTermGlyph {
    var x: UInt16 = 0, y: UInt16 = 0, w: UInt16 = 0, h: UInt16 = 0
    var dx: Int16 = 0, dy: Int16 = 0
    var color = false
    static let none = CoreTermGlyph()
    /// Not looked up yet (no glyph is that wide).
    static let unknown = CoreTermGlyph(w: .max)
}

/// A glyph drawn as the atlas draws it and cut to its ink, not in the atlas
/// yet: what any thread can make (CoreText into a bitmap of its own) for the
/// main thread to put in.
struct CoreTermInk: Sendable {
    let pixels: [UInt8]
    let w: Int, h: Int
    /// The ink's box relative to the cell's top left (pixels).
    let dx: Int, dy: Int
    let color: Bool
}

/// The glyphs of one font, size and scale, rasterised once and shared by
/// every view drawing with them. Shelf-packed; a full atlas grows (the
/// glyphs keep their places), and past the largest size starts over
/// (``generation`` tells views to re-encode their cells).
@MainActor
final class CoreTermAtlas {
    let metrics: CoreTermMetrics
    private let device: MTLDevice
    private(set) var mono: MTLTexture
    private(set) var color: MTLTexture
    private(set) var generation = 0
    private var glyphs: [Key: CoreTermGlyph] = [:]
    /// Single-scalar glyphs by `scalar << 8 | style`: a full-screen frame
    /// looks up thousands of cells, and hashing an Int beats a String.
    private var scalarGlyphs: [UInt64: CoreTermGlyph] = [:]
    /// ASCII's glyphs by `scalar << 3 | style`, ``CoreTermGlyph/unknown``
    /// until looked up: most cells of most frames, found without hashing
    /// (a debug build's dictionary cost a full-screen frame milliseconds
    /// on the main thread).
    private let ascii: UnsafeMutablePointer<CoreTermGlyph> = {
        let p = UnsafeMutablePointer<CoreTermGlyph>.allocate(capacity: 128 * 8)
        p.initialize(repeating: .unknown, count: 128 * 8)
        return p
    }()
    private var monoShelves = Shelves()
    private var colorShelves = Shelves()
    /// How many glyphs were rasterised (the prewarm's and the misses').
    private(set) var rasterised = 0
    /// The common CJK glyphs drawn ahead of time (``prewarmCommon``): how
    /// many went in, how long from the start to the last, the longest a
    /// main-thread turn of it took (ms); nil until done.
    private(set) var commonWarm: (glyphs: Int, ms: Double, longestTurnMs: Double)?
    private var commonInstalled = 0
    private var commonLongestTurn = 0.0
    /// The atlas made last: the one a view in use draws with (pinch zoom
    /// walks through sizes; only the one it stops at warms the common CJK).
    private static weak var latest: CoreTermAtlas?

    private struct Key: Hashable {
        let text: String
        let style: UInt8
    }

    private struct Shelves {
        struct Shelf { var y: Int; var height: Int; var x: Int }
        var shelves: [Shelf] = []
        var bottom = 0

        mutating func place(_ w: Int, _ h: Int, size: Int) -> (Int, Int)? {
            let pad = 1
            for i in shelves.indices where shelves[i].height >= h && shelves[i].height <= h + h / 3 + 2
                && shelves[i].x + w + pad <= size {
                let x = shelves[i].x
                shelves[i].x += w + pad
                return (x, shelves[i].y)
            }
            guard bottom + h + pad <= size, w + pad <= size else { return nil }
            shelves.append(Shelf(y: bottom, height: h, x: w + pad))
            defer { bottom += h + pad }
            return (0, bottom)
        }
    }

    private static let maxSize = 4096
    private static var atlases: [String: CoreTermAtlas] = [:]

    /// The atlas for `metrics`, made (and warmed with ASCII) the first time.
    static func shared(for metrics: CoreTermMetrics, device: MTLDevice) -> CoreTermAtlas? {
        let key = "\(metrics.font.fontName)|\(metrics.font.pointSize)|\(metrics.scale)"
        if let a = atlases[key] { return a }
        guard let a = CoreTermAtlas(metrics: metrics, device: device) else { return nil }
        atlases[key] = a
        // Atlases of fonts no longer in use: pinch zoom walks through sizes.
        if atlases.count > 6 { atlases = atlases.filter { $0.key == key } }
        a.prewarm()
        latest = a
        // The common CJK a second later, if this is still the atlas in use:
        // never in the way of the first frame, never for a size a pinch
        // passed through.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak a] in
            MainActor.assumeIsolated {
                guard let a, a === latest else { return }
                var hanzi = CoreTermCommonCJK.hanziWarmed
                #if DEBUG
                // The director's A/B: `-MOSHPIT_CJK_PREWARM n` warms the n
                // most frequent (0: nothing).
                if let n = UserDefaults.standard.string(forKey: "MOSHPIT_CJK_PREWARM").flatMap(Int.init) {
                    guard n > 0 else { return }
                    hanzi = n
                }
                #endif
                a.prewarmCommon(hanzi: hanzi)
            }
        }
        return a
    }

    private init?(metrics: CoreTermMetrics, device: MTLDevice) {
        self.metrics = metrics
        self.device = device
        guard let m = Self.texture(device, .r8Unorm, 1024), let c = Self.texture(device, .bgra8Unorm, 512) else { return nil }
        mono = m
        color = c
    }

    deinit { ascii.deallocate() }

    private static func texture(_ device: MTLDevice, _ format: MTLPixelFormat, _ size: Int) -> MTLTexture? {
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: format, width: size, height: size, mipmapped: false)
        d.usage = .shaderRead
        d.storageMode = .shared
        return device.makeTexture(descriptor: d)
    }

    /// ASCII in the regular and bold faces: the first screen never waits on
    /// CoreText (spike G1: new glyphs on a first frame cost 30–37 ms).
    func prewarm() {
        let start = CACurrentMediaTime()
        for style: UInt8 in [0, 1] {
            for v in 0x21...0x7E {
                _ = glyph(String(UnicodeScalar(UInt8(v))), style: style)
            }
        }
        Log.ui.info("core term: atlas warmed \(self.rasterised, privacy: .public) glyphs in \(Int((CACurrentMediaTime() - start) * 1000), privacy: .public) ms")
        Self.warmFallbacks(fontName: metrics.font.fontName, size: metrics.font.pointSize)
    }

    private static var fallbacksWarmed = false

    /// The fallback fonts a terminal meets first after ASCII (CJK, kana,
    /// Hangul, emoji) are loaded off the main thread, once a process:
    /// CoreText loading one for a frame's first such glyph cost a burst's
    /// frame 20 to 40 ms of the main thread on an iPad mini, a missed
    /// refresh or two (docs/nsh/TERMINAL-VIEW.md §5.6). The glyphs are
    /// still rasterised into the atlas when first drawn.
    private static func warmFallbacks(fontName: String, size: CGFloat) {
        guard !fallbacksWarmed else { return }
        fallbacksWarmed = true
        DispatchQueue.global(qos: .utility).async {
            let font = CTFontCreateWithName(fontName as CFString, size, nil)
            guard let ctx = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 64,
                                      space: CGColorSpaceCreateDeviceGray(),
                                      bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return }
            for text in ["中文字", "日本語かなカナ", "한국어", "😀"] {
                let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [.font: font]))
                ctx.textPosition = CGPoint(x: 0, y: 16)
                CTLineDraw(line, ctx)
            }
        }
    }

    /// The CJK a terminal shows most (``CoreTermCommonCJK``: the `hanzi`
    /// most frequent Chinese characters, CJK punctuation, full-width forms,
    /// kana) drawn on a utility queue and put into the atlas on the main
    /// thread 32 at a time: a first screen of Chinese finds most of its
    /// glyphs there instead of rasterising them all in one frame (about 50
    /// ms of the main thread on an iPad mini, TERMINAL-VIEW.md §5.6). A
    /// glyph outside the set is rasterised when first drawn, as before.
    private func prewarmCommon(hanzi: Int) {
        let metrics = self.metrics
        let start = CACurrentMediaTime()
        let wanted = (CoreTermCommonCJK.hanzi.prefix(hanzi) + CoreTermCommonCJK.wide).map { (String($0), UInt8(4)) }
            + CoreTermCommonCJK.narrow.map { (String($0), UInt8(0)) }
        DispatchQueue.global(qos: .utility).async { [weak self] in
            var batch: [(text: String, style: UInt8, ink: CoreTermInk?)] = []
            func send(last: Bool) {
                let ready = batch
                batch = []
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.installCommon(ready, start: start, last: last) }
                }
            }
            for (text, style) in wanted {
                batch.append((text, style, Self.ink(text, style: style, metrics: metrics)))
                if batch.count == 32 { send(last: false) }
            }
            send(last: true)
        }
    }

    private func installCommon(_ batch: [(text: String, style: UInt8, ink: CoreTermInk?)], start: CFTimeInterval,
                               last: Bool) {
        let turn = CACurrentMediaTime()
        for (text, style, ink) in batch {
            let key = Key(text: text, style: style)
            // Drawn meanwhile (on screen before its turn came).
            guard glyphs[key] == nil else { continue }
            let g = ink.map(install) ?? .none
            glyphs[key] = g
            if let s = text.unicodeScalars.first, text.unicodeScalars.count == 1 {
                scalarGlyphs[UInt64(s.value) << 8 | UInt64(style)] = g
            }
            commonInstalled += 1
        }
        commonLongestTurn = max(commonLongestTurn, (CACurrentMediaTime() - turn) * 1000)
        guard last else { return }
        let ms = (CACurrentMediaTime() - start) * 1000
        commonWarm = (commonInstalled, ms, commonLongestTurn)
        Log.ui.info("core term: atlas warmed \(self.commonInstalled, privacy: .public) common CJK glyphs in \(Int(ms), privacy: .public) ms, longest main-thread turn \(String(format: "%.1f", self.commonLongestTurn), privacy: .public) ms, mono atlas \(self.mono.width, privacy: .public) px")
    }

    /// For the debug probes: the atlas's size and what it warmed.
    var summary: String {
        let warm = commonWarm.map {
            String(format: "%d common CJK in %.0f ms, longest main-thread turn %.1f ms", $0.glyphs, $0.ms, $0.longestTurnMs)
        } ?? "common CJK not warmed yet"
        return "atlas mono \(mono.width) px, colour \(color.width) px, \(glyphs.count) glyphs; \(warm)"
    }

    /// A glyph of one scalar (most cells).
    func glyph(scalar: UInt32, style: UInt8) -> CoreTermGlyph {
        let small = scalar < 128 && style < 8
        let at = Int(scalar) << 3 | Int(style)
        if small {
            let g = ascii[at]
            if g.w != CoreTermGlyph.unknown.w { return g }
        }
        let key = UInt64(scalar) << 8 | UInt64(style)
        if let g = scalarGlyphs[key] {
            if small { ascii[at] = g }
            return g
        }
        guard let s = Unicode.Scalar(scalar) else { return .none }
        let g = glyph(String(Character(s)), style: style)
        scalarGlyphs[key] = g
        if small { ascii[at] = g }
        return g
    }

    /// `style`: 1 bold, 2 italic, 4 wide (two cells).
    func glyph(_ text: String, style: UInt8) -> CoreTermGlyph {
        let key = Key(text: text, style: style)
        if let g = glyphs[key] { return g }
        let g = rasterise(text, style: style)
        glyphs[key] = g
        return g
    }

    private func rasterise(_ text: String, style: UInt8) -> CoreTermGlyph {
        rasterised += 1
        guard let ink = Self.ink(text, style: style, metrics: metrics) else { return .none }
        return install(ink)
    }

    /// `text` drawn as the atlas draws it, cut to its ink; nil when it has
    /// none. Any thread.
    nonisolated static func ink(_ text: String, style: UInt8, metrics m: CoreTermMetrics) -> CoreTermInk? {
        let span = (style & 4 != 0 ? 2 : 1) * m.widthPx
        let pad = Int((CGFloat(m.heightPx) * 0.35).rounded(.up))
        let w = span + 2 * pad, h = m.heightPx + 2 * pad
        let font = m.font(bold: style & 1 != 0, italic: style & 2 != 0)
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [
            .font: font, .foregroundColor: UIColor.white,
        ]))
        // A colour glyph (emoji) anywhere in the line: rasterise in colour.
        var isColor = false
        if let runs = CTLineGetGlyphRuns(line) as? [CTRun] {
            for run in runs {
                let attrs = CTRunGetAttributes(run) as NSDictionary
                if let f = attrs[kCTFontAttributeName] {
                    // swiftlint:disable:next force_cast
                    let ct = f as! CTFont
                    if CTFontGetSymbolicTraits(ct).contains(.traitColorGlyphs) { isColor = true }
                }
            }
        }
        let bytesPerPixel = isColor ? 4 : 1
        var data = [UInt8](repeating: 0, count: w * h * bytesPerPixel)
        // The ink's box, found as it is drawn (through a pointer: a debug
        // build's checked subscripts cost a glyph a millisecond here).
        var minX = w, minY = h, maxX = -1, maxY = -1
        let drawn: Bool = data.withUnsafeMutableBytes { raw -> Bool in
            let ctx: CGContext?
            if isColor {
                ctx = CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8,
                                bytesPerRow: w * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
            } else {
                ctx = CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8,
                                bytesPerRow: w, space: CGColorSpaceCreateDeviceGray(),
                                bitmapInfo: CGImageAlphaInfo.none.rawValue)
            }
            guard let ctx, let px = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return false }
            ctx.setShouldSmoothFonts(false)
            ctx.setAllowsFontSmoothing(false)
            ctx.setShouldAntialias(true)
            ctx.setShouldSubpixelPositionFonts(false)
            ctx.setShouldSubpixelQuantizeFonts(false)
            // Baseline `pad + ascent` pixels from the top (CoreGraphics
            // counts from the bottom).
            ctx.translateBy(x: CGFloat(pad), y: CGFloat(h - pad - m.ascentPx))
            ctx.scaleBy(x: m.scale, y: m.scale)
            let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            let spanPt = CGFloat(span) / m.scale
            ctx.textPosition = .zero
            // Wider than its cells: fitted to them, its shape kept (B472).
            if let fit = CoreTermGlyphFit.fit(advance: width, ink: CoreTermGlyphFit.ink(of: line, in: ctx), span: spanPt,
                                              ascent: m.ascent, descent: m.descent,
                                              scalar: text.unicodeScalars.first?.value ?? 0) {
                fit.apply(to: ctx)
            }
            CTLineDraw(line, ctx)
            let alpha = isColor ? 3 : 0
            for y in 0..<h {
                let row = px + y * w * bytesPerPixel + alpha
                for x in 0..<w where row[x * bytesPerPixel] != 0 {
                    if x < minX { minX = x }
                    if x > maxX { maxX = x }
                    if y < minY { minY = y }
                    if y > maxY { maxY = y }
                }
            }
            return true
        }
        guard drawn, maxX >= 0 else { return nil }
        let gw = maxX - minX + 1, gh = maxY - minY + 1
        var trimmed = [UInt8](repeating: 0, count: gw * gh * bytesPerPixel)
        for y in 0..<gh {
            let src = ((minY + y) * w + minX) * bytesPerPixel
            let dst = y * gw * bytesPerPixel
            trimmed[dst..<dst + gw * bytesPerPixel] = data[src..<src + gw * bytesPerPixel]
        }
        return CoreTermInk(pixels: trimmed, w: gw, h: gh, dx: minX - pad, dy: minY - pad, color: isColor)
    }

    /// `ink` into the atlas.
    private func install(_ ink: CoreTermInk) -> CoreTermGlyph {
        guard let (ax, ay) = place(ink.w, ink.h, color: ink.color) else { return .none }
        let texture = ink.color ? color : mono
        let bytesPerPixel = ink.color ? 4 : 1
        ink.pixels.withUnsafeBytes {
            texture.replace(region: MTLRegionMake2D(ax, ay, ink.w, ink.h), mipmapLevel: 0,
                            withBytes: $0.baseAddress!, bytesPerRow: ink.w * bytesPerPixel)
        }
        return CoreTermGlyph(x: UInt16(ax), y: UInt16(ay), w: UInt16(ink.w), h: UInt16(ink.h),
                             dx: Int16(ink.dx), dy: Int16(ink.dy), color: ink.color)
    }

    /// A place in the atlas; grows it (or, at the largest size, starts over)
    /// when full.
    private func place(_ w: Int, _ h: Int, color isColor: Bool) -> (Int, Int)? {
        for _ in 0..<4 {
            let size = (isColor ? color : mono).width
            if isColor, let p = colorShelves.place(w, h, size: size) { return p }
            if !isColor, let p = monoShelves.place(w, h, size: size) { return p }
            if size < Self.maxSize {
                grow(color: isColor, to: size * 2)
            } else {
                startOver()
            }
        }
        return nil
    }

    private func grow(color isColor: Bool, to size: Int) {
        let old = isColor ? color : mono
        guard let new = Self.texture(device, old.pixelFormat, size) else { return }
        // Copy what is there (shared storage: through the CPU).
        let bpp = isColor ? 4 : 1
        var bytes = [UInt8](repeating: 0, count: old.width * old.height * bpp)
        bytes.withUnsafeMutableBytes {
            old.getBytes($0.baseAddress!, bytesPerRow: old.width * bpp,
                         from: MTLRegionMake2D(0, 0, old.width, old.height), mipmapLevel: 0)
        }
        bytes.withUnsafeBytes {
            new.replace(region: MTLRegionMake2D(0, 0, old.width, old.height), mipmapLevel: 0,
                        withBytes: $0.baseAddress!, bytesPerRow: old.width * bpp)
        }
        if isColor { color = new } else { mono = new }
        Log.ui.info("core term: atlas \(isColor ? "colour" : "mono", privacy: .public) grew to \(size, privacy: .public)")
    }

    private func startOver() {
        glyphs.removeAll()
        scalarGlyphs.removeAll()
        ascii.update(repeating: .unknown, count: 128 * 8)
        monoShelves = Shelves()
        colorShelves = Shelves()
        generation += 1
        Log.ui.info("core term: atlas full, starting over (generation \(self.generation, privacy: .public))")
    }
}

/// The view's Metal-drawn picture of a ``CoreTermRows``.
@MainActor
final class CoreTermMetalCanvas: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer } // swiftlint:disable:this force_cast

    let gpu: CoreTermGPU
    private(set) var atlas: CoreTermAtlas
    private var atlasGeneration: Int
    var style: CoreTermStyle
    /// The rows' instance words (8 per cell), rows after rows.
    private var words: [UInt32] = []
    private var cols = 0
    private var rowCount = 0
    private var shape = -1
    /// Decorations per row, as Rect structs (8 words each).
    private var rects: [[Float]] = []
    /// Image quads per row: id, frame (px), uv.
    private var imageRuns: [[(id: UInt64, frame: SIMD4<Float>, uv: SIMD4<Float>, above: Bool)]] = []
    private var textures: [UInt64: MTLTexture] = [:]
    /// The images' textures kept: how many, and their bytes (debug probes).
    var textureMemory: (count: Int, bytes: Int) {
        (textures.count, textures.values.reduce(0) { $0 + $1.allocatedSize })
    }
    /// The pixel offset rows are drawn at (negative: up).
    var yOffset: CGFloat = 0
    /// Whether the cursor shows (blink off: not).
    var cursorShown = true
    /// For measurements: how long the last encode took (ms) and how many
    /// rows it encoded.
    private(set) var lastEncodeRows = 0

    init?(metrics: CoreTermMetrics, style: CoreTermStyle) {
        guard let gpu = CoreTermGPU.shared, let atlas = CoreTermAtlas.shared(for: metrics, device: gpu.device) else {
            return nil
        }
        self.gpu = gpu
        self.atlas = atlas
        self.atlasGeneration = atlas.generation
        self.style = style
        super.init(frame: .zero)
        isOpaque = true
        isUserInteractionEnabled = false
        metalLayer.device = gpu.device
        metalLayer.pixelFormat = CoreTermGPU.pixelFormat
        metalLayer.framebufferOnly = false
        metalLayer.isOpaque = true
        metalLayer.maximumDrawableCount = 3
        metalLayer.allowsNextDrawableTimeout = true
        metalLayer.contentsScale = metrics.scale
    }

    required init?(coder: NSCoder) { fatalError("CoreTermMetalCanvas is made in code") }

    var metrics: CoreTermMetrics { atlas.metrics }

    /// A new font: a new atlas, every row re-encoded.
    func setMetrics(_ metrics: CoreTermMetrics) {
        guard metrics != atlas.metrics, let a = CoreTermAtlas.shared(for: metrics, device: gpu.device) else { return }
        atlas = a
        atlasGeneration = a.generation
        metalLayer.contentsScale = metrics.scale
        shape = -1
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        fitDrawables()
    }

    /// The drawables' size the view's bounds call for, in whole pixels (so
    /// the size set and the size read back compare equal: a render checks
    /// them every frame).
    var fittedSize: CGSize {
        CGSize(width: max(1, (bounds.width * metrics.scale).rounded()),
               height: max(1, (bounds.height * metrics.scale).rounded()))
    }

    /// The drawables' size now, in pixels (diagnostics, tests).
    var drawableSize: CGSize { metalLayer.drawableSize }

    /// The drawables were let go (``releaseDrawables``) and not made again:
    /// whatever the sizes say, the next fit makes them (B386).
    private(set) var released = false

    /// Whether the next drawable will be the view's size, and made since
    /// the drawables were last let go.
    var drawablesFit: Bool { !released && metalLayer.drawableSize == fittedSize }

    /// Drawables the size of the view: made again after a release even when
    /// the sizes already agree (B386).
    func fitDrawables() {
        let size = fittedSize
        guard released || metalLayer.drawableSize != size else { return }
        metalLayer.drawableSize = size
        released = false
    }

    /// A view off screen lets its drawables go (two of the screen's size,
    /// 22 MB a pane on an iPad mini, kept by every tmux pane once shown,
    /// B297): the layer's picture and its pool, and its images' textures
    /// (B737: a full-screen image's is as large again, and every pane
    /// shown kept its own for good). Back in a window it draws again
    /// (``fitDrawables``, then every row encoded, the textures with them)
    /// before it shows.
    func releaseDrawables() {
        metalLayer.contents = nil
        metalLayer.drawableSize = CGSize(width: 1, height: 1)
        textures.removeAll()
        released = true
    }

    /// Why the last ``render(rows:completion:)`` drew nothing, if it did
    /// not: what the log says when the view can't present (B386).
    private(set) var lastFailure: String?

    /// The GPU's last refusal of a frame (a command buffer that ended in an
    /// error: the app in the background, a fault), taken once read.
    private let gpuFailure = CoreTermGPUFailure()

    /// The GPU's last refusal of a frame since this was last asked, if any.
    func takeGPUFailure() -> String? { gpuFailure.take() }

    /// Whether the last render found no drawable free: `nextDrawable()`
    /// held the main thread its full second and gave nothing (B754).
    var foundNoDrawable: Bool { lastFailure?.hasPrefix("no drawable") == true }

    /// Drawables taken from this layer and not heard of since (B754).
    private let taken = CoreTermTakenDrawables()

    /// The drawables of this layer taken within the last `within` seconds
    /// and not heard of since: one heard of no longer (a drawable the
    /// system let go without a word, B445) stops counting.
    func drawablesHeld(now: CFTimeInterval, within: CFTimeInterval) -> Int {
        taken.held(now: now, within: within)
    }

    /// When a drawable last came (`CACurrentMediaTime`), 0 before any.
    var lastAcquiredAt: CFTimeInterval { taken.lastTakenAt }

    /// How many drawables the layer has.
    var drawableCount: Int { metalLayer.maximumDrawableCount }

    // MARK: Encoding

    /// Re-encodes the rows `changed` of `rows` (every row when the grid's
    /// shape changed, the atlas started over, or `all`).
    func update(_ rows: CoreTermRows, changed: [Int], all: Bool, shift: Int) {
        // The rows drawn last move with the content (the view or the
        // screen scrolled): their instances, decorations and images too.
        if shift != 0, rows.shape == shape, rows.cols == cols, rows.rows.count == rowCount, abs(shift) < rowCount {
            let n = abs(shift) * cols * 8
            if shift > 0 {
                words.removeFirst(n)
                words.append(contentsOf: repeatElement(0, count: n))
                rects.removeFirst(shift)
                rects.append(contentsOf: Array(repeating: [], count: shift))
                imageRuns.removeFirst(shift)
                imageRuns.append(contentsOf: Array(repeating: [], count: shift))
            } else {
                words.removeLast(n)
                words.insert(contentsOf: repeatElement(0, count: n), at: 0)
                rects.removeLast(-shift)
                rects.insert(contentsOf: Array(repeating: [], count: -shift), at: 0)
                imageRuns.removeLast(-shift)
                imageRuns.insert(contentsOf: Array(repeating: [], count: -shift), at: 0)
            }
        }
        if atlas.generation != atlasGeneration {
            atlasGeneration = atlas.generation
            shape = -1
        }
        var list = changed
        if rows.shape != shape || rows.cols != cols || rows.rows.count != rowCount || all {
            shape = rows.shape
            cols = rows.cols
            rowCount = rows.rows.count
            words = Array(repeating: 0, count: cols * rowCount * 8)
            rects = Array(repeating: [], count: rowCount)
            imageRuns = Array(repeating: [], count: rowCount)
            list = Array(0..<rowCount)
        }
        for r in list where r < rowCount { encode(row: r, of: rows) }
        lastEncodeRows = list.count
        // Textures no row draws any more.
        if !textures.isEmpty {
            let shown = Set(imageRuns.joined().map(\.id))
            if shown.count != textures.count { textures = textures.filter { shown.contains($0.key) } }
        }
    }

    private func encode(row r: Int, of rows: CoreTermRows) {
        let row = rows.rows[r]
        let m = metrics
        let cw = Float(m.widthPx), ch = Float(m.heightPx)
        let cursor = rows.info.cursor
        let cursorHere = cursorShown && cursor.visible && Int(cursor.row) == r
        let style = self.style
        let atlas = self.atlas
        let cols = self.cols
        let block = cursorHere && style.cursor == .block ? Int(cursor.col) : -1
        let blockWide = block >= 0 && block < row.cells.count && coreHas(row.cells[block], Int(MP_CELL_WIDE))
        var decorations: [Float] = []
        let n = min(cols, row.cells.count)
        // A full-screen frame is thousands of cells on the main thread:
        // the words and the cells through pointers, the style and the atlas
        // read once (a debug build's checked subscripts and property reads
        // per word were most of a frame's cost).
        words.withUnsafeMutableBufferPointer { words in
            row.cells.withUnsafeBufferPointer { cells in
                guard let out = words.baseAddress else { return }
                let src = cells.baseAddress
                var w = out + r * cols * 8
                for c in 0..<cols {
                    defer { w += 8 }
                    guard c < n, let src else {
                        w[0] = UInt32(c) | UInt32(r) << 16
                        w[1] = 0; w[2] = 0; w[3] = 0
                        w[4] = style.foreground; w[5] = style.background
                        w[6] = 0; w[7] = 0
                        continue
                    }
                    let cell = src[c]
                    let flags = Int(cell.flags)
                    var bg = flags & Int(MP_CELL_SELECTED) != 0 ? style.selection
                        : flags & Int(MP_CELL_MATCH) != 0 ? style.match
                        : cell.bg == style.blended ? style.background : cell.bg
                    var fg = cell.fg
                    if c == block || (blockWide && c == block + 1) {
                        fg = bg
                        bg = style.cursorColor
                    }
                    var g = CoreTermGlyph.none
                    if flags & Int(MP_CELL_SPACER) == 0, cell.ch != 0, !(cell.ch == 0x20 && cell.extra == 0) {
                        var s: UInt8 = 0
                        if flags & Int(MP_CELL_BOLD) != 0 { s |= 1 }
                        if flags & Int(MP_CELL_ITALIC) != 0 { s |= 2 }
                        if flags & Int(MP_CELL_WIDE) != 0 { s |= 4 }
                        g = cell.extra == 0 ? atlas.glyph(scalar: cell.ch, style: s)
                            : atlas.glyph(rows.text(of: cell, in: row), style: s)
                        if atlas.generation != atlasGeneration {
                            // The atlas started over under this row:
                            // everything encoded before points at glyphs
                            // that are gone.
                            atlasGeneration = atlas.generation
                            shape = -1
                            g = cell.extra == 0 ? atlas.glyph(scalar: cell.ch, style: s)
                                : atlas.glyph(rows.text(of: cell, in: row), style: s)
                        }
                    }
                    w[0] = UInt32(c) | UInt32(r) << 16
                    w[1] = UInt32(g.x) | UInt32(g.y) << 16
                    w[2] = UInt32(g.w) | UInt32(g.h) << 16
                    w[3] = UInt32(UInt16(bitPattern: g.dx)) | UInt32(UInt16(bitPattern: g.dy)) << 16
                    w[4] = fg
                    w[5] = bg
                    w[6] = g.color ? 1 : 0
                    w[7] = 0
                    guard flags & 0x20FC != 0 else { continue }
                    decorate(cell, col: c, fg: fg, into: &decorations)
                }
            }
        }
        if cursorHere, style.cursor != .block {
            let c = Int(cursor.col)
            let wide = c < row.cells.count && coreHas(row.cells[c], Int(MP_CELL_WIDE))
            let x = Float(c) * cw, y: Float = 0
            let thick = max(2, (Float(m.scale) * 1.5).rounded())
            let color = Float(bitPattern: style.cursorColor)
            switch style.cursor {
            case .bar: decorations += [x, y, thick, ch, color, 0, 0, 0]
            case .underline: decorations += [x, y + ch - thick, (wide ? 2 : 1) * cw, thick, color, 0, 0, 0]
            case .block: break
            }
        }
        rects[r] = decorations
        // Images.
        var runs: [(id: UInt64, frame: SIMD4<Float>, uv: SIMD4<Float>, above: Bool)] = []
        for (run, id) in row.images {
            guard let image = rows.images[id] else { continue }
            if textures[id] == nil { textures[id] = makeTexture(image) }
            let ih = Float(image.height), iw = Float(image.width)
            let pxPerCell = (run.right - run.left) * iw / Float(max(run.cols, 1))
            let k = pxPerCell > 0 ? cw / pxPerCell : 1
            let height = min(ch, (run.bottom - run.top) * ih * k)
            runs.append((id, SIMD4(Float(run.col) * cw, 0, Float(run.cols) * cw, height),
                         SIMD4(run.left, run.top, run.right, run.bottom), run.z >= 0))
        }
        imageRuns[r] = runs
    }

    /// A cell's underlines and strikeout, relative to its row (rows move
    /// when the content scrolls).
    private func decorate(_ cell: MpDrawCell, col c: Int, fg: UInt32, into decorations: inout [Float]) {
        let m = metrics
        let cw = Float(m.widthPx), ch = Float(m.heightPx)
        let color = cell.underline == UInt32.max ? fg : cell.underline
        let x = Float(c) * cw, y: Float = 0
        let thick = max(1, (Float(m.scale) * 0.75).rounded())
        let uy = y + Float(m.ascentPx) + max(1, Float(m.descent * m.scale / 2).rounded())
        func rect(_ rx: Float, _ ry: Float, _ rw: Float, _ rh: Float) {
            decorations += [rx, ry, rw, rh, Float(bitPattern: color), 0, 0, 0]
        }
        if coreHas(cell, Int(MP_CELL_UNDERLINE)) || coreHas(cell, Int(MP_CELL_LINK)) { rect(x, uy, cw, thick) }
        if coreHas(cell, Int(MP_CELL_DOUBLE_UNDERLINE)) {
            rect(x, uy, cw, thick)
            rect(x, uy + 2 * thick, cw, thick)
        }
        if coreHas(cell, Int(MP_CELL_UNDERCURL)) {
            let steps = max(4, Int(cw / 2))
            let amp = max(1, thick * 1.5)
            for k in 0..<steps {
                let t = Float(k) / Float(steps)
                rect(x + t * cw, uy + amp * sin(t * 2 * .pi), cw / Float(steps) + 0.5, thick)
            }
        }
        if coreHas(cell, Int(MP_CELL_DOTTED_UNDERLINE)) || coreHas(cell, Int(MP_CELL_DASHED_UNDERLINE)) {
            let dash = coreHas(cell, Int(MP_CELL_DOTTED_UNDERLINE)) ? thick : 3 * thick
            var dx: Float = 0
            while dx < cw {
                rect(x + dx, uy, min(dash, cw - dx), thick)
                dx += dash + 2 * thick
            }
        }
        if coreHas(cell, Int(MP_CELL_STRIKEOUT)) { rect(x, y + (ch / 2).rounded(), cw, thick) }
    }

    private func makeTexture(_ image: CoreTermRows.Image) -> MTLTexture? {
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm, width: image.width,
                                                         height: image.height, mipmapped: false)
        d.usage = .shaderRead
        d.storageMode = .shared
        guard let t = gpu.device.makeTexture(descriptor: d) else { return nil }
        image.rgba.withUnsafeBytes {
            t.replace(region: MTLRegionMake2D(0, 0, image.width, image.height), mipmapLevel: 0,
                      withBytes: $0.baseAddress!, bytesPerRow: image.width * 4)
        }
        return t
    }

    // MARK: Drawing

    /// Draws into the layer's next drawable; false when none was free.
    /// `completion` runs once the frame's fate is known: true when the
    /// picture reached the screen, false when the layer dropped it (its
    /// presentation time is zero) or the GPU refused it (B445).
    @discardableResult
    func render(rows: CoreTermRows, completion: (@Sendable (Bool) -> Void)?) -> Bool {
        guard bounds.width > 0, bounds.height > 0 else {
            lastFailure = "no size"
            return false
        }
        // Never into drawables let go or of another size: a frame the size
        // of one pixel shows as the background colour all over (B386).
        if !drawablesFit { fitDrawables() }
        guard let drawable = metalLayer.nextDrawable() else {
            lastFailure = "no drawable free (\(Int(metalLayer.drawableSize.width))×\(Int(metalLayer.drawableSize.height)))"
            return false
        }
        guard let buffer = gpu.queue.makeCommandBuffer() else {
            lastFailure = "no command buffer"
            return false
        }
        lastFailure = nil
        encodePass(into: drawable.texture, buffer: buffer, rows: rows)
        // Held until its fate is known (B754: the pacing counts them).
        let ledger = taken, ticket = ledger.take(at: CACurrentMediaTime())
        let fate = CoreTermFrameFate { shown in
            ledger.giveBack(ticket)
            completion?(shown)
        }
        #if !targetEnvironment(simulator)
        drawable.addPresentedHandler { shown in fate.resolve(shown: shown.presentedTime > 0) }
        #endif
        let failure = gpuFailure
        buffer.addCompletedHandler { buffer in
            if buffer.status == .error {
                failure.note(buffer.error.map { "\($0)" } ?? "error")
                fate.resolve(shown: false)
            }
            #if targetEnvironment(simulator)
            // The simulator's SDK has no addPresentedHandler (iPhoneSimulator's
            // MTLDrawable.h doesn't declare it), so there the command buffer
            // tells the fate: done without error counts as shown. After an
            // error it was told above, and only the first word counts. The
            // fate timeout (CoreTermPresentation.fateTimeout) stands as on a
            // device.
            fate.resolve(shown: true)
            #endif
        }
        buffer.present(drawable)
        buffer.commit()
        return true
    }

    /// Draws into a texture of its own and reads it back: BGRA rows, for
    /// render-and-compare tests and pixel checks on a device.
    func renderOffscreen(rows: CoreTermRows, size: CGSize) -> (pixels: [UInt8], width: Int, height: Int)? {
        let w = Int(size.width * metrics.scale), h = Int(size.height * metrics.scale)
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: CoreTermGPU.pixelFormat, width: w, height: h,
                                                         mipmapped: false)
        d.usage = [.renderTarget, .shaderRead]
        d.storageMode = .shared
        guard let target = gpu.device.makeTexture(descriptor: d), let buffer = gpu.queue.makeCommandBuffer() else {
            return nil
        }
        encodePass(into: target, buffer: buffer, rows: rows, viewport: CGSize(width: w, height: h))
        buffer.commit()
        buffer.waitUntilCompleted()
        var pixels = [UInt8](repeating: 0, count: w * h * 4)
        pixels.withUnsafeMutableBytes {
            target.getBytes($0.baseAddress!, bytesPerRow: w * 4, from: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0)
        }
        return (pixels, w, h)
    }

    private func encodePass(into texture: MTLTexture, buffer: MTLCommandBuffer, rows: CoreTermRows,
                            viewport: CGSize? = nil) {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = texture
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        // What lies outside the grid (B495).
        let bg = style.outside
        pass.colorAttachments[0].clearColor = MTLClearColor(red: Double((bg >> 16) & 0xFF) / 255,
                                                            green: Double((bg >> 8) & 0xFF) / 255,
                                                            blue: Double(bg & 0xFF) / 255, alpha: 1)
        guard let enc = buffer.makeRenderCommandEncoder(descriptor: pass) else { return }
        let m = metrics
        let vp = viewport ?? CGSize(width: texture.width, height: texture.height)
        var uniforms: [Float] = [
            Float(vp.width), Float(vp.height),
            Float(m.widthPx), Float(m.heightPx),
            Float(atlas.mono.width), Float(atlas.mono.height),
            Float(atlas.color.width), Float(atlas.color.height),
            Float((yOffset * m.scale).rounded()), Float(max(cols, 1)),
        ]
        enc.setVertexBytes(&uniforms, length: uniforms.count * 4, index: 0)
        let cells = cols * rowCount
        if cells > 0, !words.isEmpty {
            let instances = words.withUnsafeBytes { gpu.device.makeBuffer(bytes: $0.baseAddress!, length: $0.count) }
            if let instances {
                enc.setRenderPipelineState(gpu.background)
                enc.setVertexBuffer(instances, offset: 0, index: 1)
                enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: cells)
                drawImages(enc, above: false)
                enc.setRenderPipelineState(gpu.glyphs)
                enc.setVertexBuffer(instances, offset: 0, index: 1)
                enc.setFragmentTexture(atlas.mono, index: 0)
                enc.setFragmentTexture(atlas.color, index: 1)
                enc.setFragmentSamplerState(gpu.sampler, index: 0)
                enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: cells)
                drawImages(enc, above: true)
            }
        }
        // Decorations at their rows (kept relative to them).
        let rowPx = Float(m.heightPx)
        var flat: [Float] = []
        for (r, list) in rects.enumerated() where !list.isEmpty {
            var k = 0
            while k + 8 <= list.count {
                flat += [list[k], list[k + 1] + Float(r) * rowPx, list[k + 2], list[k + 3],
                         list[k + 4], list[k + 5], list[k + 6], list[k + 7]]
                k += 8
            }
        }
        if !flat.isEmpty {
            let count = flat.count / 8
            flat.withUnsafeBytes { raw in
                if let b = gpu.device.makeBuffer(bytes: raw.baseAddress!, length: raw.count) {
                    enc.setRenderPipelineState(gpu.rects)
                    enc.setVertexBuffer(b, offset: 0, index: 1)
                    enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: count)
                }
            }
        }
        enc.endEncoding()
    }

    /// The images below the text (kitty's negative z) or above it.
    private func drawImages(_ enc: MTLRenderCommandEncoder, above: Bool) {
        var byId: [UInt64: [Float]] = [:]
        let rowPx = Float(metrics.heightPx)
        for (r, row) in imageRuns.enumerated() {
            for run in row where run.above == above {
                byId[run.id, default: []] += [run.frame.x, run.frame.y + Float(r) * rowPx, run.frame.z, run.frame.w,
                                              run.uv.x, run.uv.y, run.uv.z, run.uv.w]
            }
        }
        guard !byId.isEmpty else { return }
        enc.setRenderPipelineState(gpu.images)
        enc.setFragmentSamplerState(gpu.linear, index: 0)
        for (id, quads) in byId {
            guard let texture = textures[id] else { continue }
            quads.withUnsafeBytes { raw in
                guard let b = gpu.device.makeBuffer(bytes: raw.baseAddress!, length: raw.count) else { return }
                enc.setVertexBuffer(b, offset: 0, index: 1)
                enc.setFragmentTexture(texture, index: 0)
                enc.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: quads.count / 8)
            }
        }
    }
}

/// The GPU's last refusal of a frame, noted on Metal's completion thread
/// and taken on the main one (B386).
final class CoreTermGPUFailure: @unchecked Sendable {
    private let lock = NSLock()
    private var last: String?

    func note(_ failure: String) {
        lock.lock()
        last = failure
        lock.unlock()
    }

    func take() -> String? {
        lock.lock()
        defer { last = nil; lock.unlock() }
        return last
    }
}
