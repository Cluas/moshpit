import MoshpitFFI
import SwiftUI
import UIKit

// The core view's look, paste and predictive echo, in the app's terms
// (``TerminalTheme``, ``MoshPrediction``); its place behind the app's seam is
// CoreTerminalView+Surface.

extension CoreTerminalView {
    /// The app's terminal theme: its 16 colours, foreground, background and
    /// cursor into the core; the user's cursor (Settings) over whatever the
    /// remote asks for.
    func apply(theme: TerminalTheme, cursorShape: CursorShape, cursorColorId: String, cursorBlink: Bool,
               appTheme: AppTheme = AppThemeCatalog.current) {
        func hex(_ s: String) -> UInt32 {
            UInt32(s.trimmingCharacters(in: CharacterSet(charactersIn: "# ")), radix: 16) ?? 0
        }
        let slots = TerminalTheme.ANSISlot.allCases
        let ansi = slots.map { hex(theme.ansiHex($0)) } + slots.map { hex(theme.brightHex($0)) }
        let background = hex(theme.backgroundHex), foreground = hex(theme.foregroundHex)
        let changed = appliedPalette.map { $0.0 != ansi || $0.1 != background || $0.2 != foreground } ?? true
        if changed, let term {
            ansi.withUnsafeBufferPointer {
                mp_term_set_palette(term, $0.baseAddress!, foreground, background, hex(theme.cursorHex))
            }
            appliedPalette = (ansi, background, foreground)
            backgroundColor = UIColor(cgColor: coreCGColor(background))
        }
        var s = style
        s.background = background
        s.foreground = foreground
        s.cursorColor = UIColor(TerminalCursor.color(forId: cursorColorId, theme: appTheme)).coreRGB
        s.cursor = switch cursorShape {
        case .block: .block
        case .underline: .underline
        case .bar: .bar
        }
        s.cursorBlink = cursorBlink
        style = s
        takeFrame()
    }
}

extension CoreTerminalView {
    /// What a paste of `text` sends (V30); nil: nothing.
    func pasteBytes(_ text: String, force: Bool = false) -> Data? {
        guard let term else { return nil }
        let b = text.withCString { mp_term_paste(term, $0, force) }
        defer { mp_bytes_free(b) }
        guard let p = b.ptr, b.len > 0 else { return nil }
        return Data(bytes: p, count: b.len)
    }
}

// mosh's predictive echo, in the core (crates/term/src/predict.rs): laid
// over the frames, never written into the grid the server diffs against.
extension CoreTerminalView {
    func predictAttach(_ engine: MoshPrediction?) {
        guard let term else { return }
        let mode: UInt8 = switch engine?.mode {
        case .adaptive?: 1
        case .always?: 2
        case .experimental?: 3
        default: 0
        }
        mp_term_predict_attach(term, mode, engine?.trail ?? false)
        if let engine { mp_term_predict_srtt(term, engine.srttMs) }
        takeFrame()
    }

    /// Keystrokes on their way to the server: their likely echo shows now.
    /// The batch for ``predictSent(batch:userState:)``, nil when none.
    func predictInput(_ data: Data, srtt: Double) -> Int? {
        guard let term else { return nil }
        mp_term_predict_srtt(term, srtt)
        let batch = data.withUnsafeBytes { raw in
            mp_term_predict_input(term, raw.baseAddress?.assumingMemoryBound(to: UInt8.self), raw.count)
        }
        takeFrame()
        return batch == 0 ? nil : Int(batch)
    }

    func predictSent(batch: Int, userState: UInt64?) {
        guard let term else { return }
        mp_term_predict_sent(term, UInt64(batch), userState ?? 0, userState != nil)
    }

    func predictEchoAck(_ ack: UInt64) {
        guard let term else { return }
        mp_term_predict_echo_ack(term, ack)
    }

    func predictReset() {
        guard let term else { return }
        mp_term_predict_reset(term)
        takeFrame()
    }

    var predictNeedsTick: Bool {
        guard let term else { return false }
        return mp_term_predict_needs_tick(term)
    }

    func predictTick(srtt: Double) {
        guard let term else { return }
        mp_term_predict_srtt(term, srtt)
        if mp_term_predict_tick(term) { takeFrame() }
    }
}
