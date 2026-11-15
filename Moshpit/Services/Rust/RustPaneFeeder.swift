import Foundation
import MoshpitKit
import MoshpitFFI
import QuartzCore

/// One pane's byte channel from the Rust core into the app's terminal
/// coordinator (crates/stream; the G1/G5 spike's glue, PaneTerminal.swift,
/// moved onto ``CoreTerminalHost/Coordinator``).
///
/// The core pushes the host's bytes into the channel as it reads them; the
/// channel calls the waker at most once until the queue is drained, on a
/// core thread, and the waker makes one hop to the main queue. There the
/// feeder reads the queue in order and feeds the coordinator — always on the
/// main thread (B21, B63: the coordinator and SwiftTerm's parser are not
/// thread-safe) — until it is drained or the turn's budget is spent; the
/// rest goes at the next display frame (a burst never keeps a frame from
/// coming). Nothing is dropped and nothing reorders: one queue, one reader.
///
/// What the pane's view must do between two bytes (cover, reveal, a held
/// resize) arrives as a mark in the same queue, read where it was put
/// (`mp_stream_read_marked`), and goes to ``onMark`` before the bytes after
/// it are fed.
///
/// Input goes the other way synchronously on the calling thread
/// (`mp_stream_write`, `mp_stream_resize`): one call, in order, no hop.
@MainActor
final class RustPaneFeeder {
    let stream: UInt64
    private weak var coordinator: CoreTerminalHost.Coordinator?
    /// A mark read from the channel, before the bytes after it are fed.
    var onMark: ((MpStreamMark) -> Void)?
    /// When bytes were last fed: the scroll routing's "is this pane
    /// streaming" (a primary-screen mouse app that repaints parks a wheel).
    private(set) var lastFedAt: CFTimeInterval = 0
    private(set) var bytesFed: UInt64 = 0
    /// Bytes written into the channel so far: the offsets mosh's echo
    /// acknowledgement is told in (the core counts the same bytes).
    private(set) var written: UInt64 = 0

    private var buffer = [UInt8](repeating: 0, count: 32 * 1024)
    private var pacer: CADisplayLink?
    /// Released by ``detach()``, or by the deinit of a feeder nobody
    /// detached (a channel's waker must never outlive what it points at).
    nonisolated(unsafe) private var waker: Unmanaged<WakerBox>?
    private static let budget: CFTimeInterval = 0.004

    /// Starts reading `stream` into `coordinator`; whatever is queued
    /// already is read right away.
    init(stream: UInt64, coordinator: CoreTerminalHost.Coordinator) {
        self.stream = stream
        self.coordinator = coordinator
        let box = Unmanaged.passRetained(WakerBox(self))
        waker = box
        if !mp_stream_attach(stream, rustPaneWake, box.toOpaque()) {
            box.release()
            waker = nil
            Log.ssh.error("rust: stream \(stream, privacy: .public) is gone")
        }
    }

    deinit {
        if let waker {
            mp_stream_detach(stream)
            waker.release()
        }
    }

    /// Stops reading: the waker is not called again once this returns.
    func detach() {
        pacer?.invalidate()
        pacer = nil
        guard let waker else { return }
        mp_stream_detach(stream)
        waker.release()
        self.waker = nil
    }

    // MARK: Output

    /// Reads the channel until it is drained or the turn's budget is spent.
    func drain() {
        guard waker != nil else { return }
        let start = CACurrentMediaTime()
        let cap = buffer.count
        var mark = MpStreamMark()
        var marked = false
        while true {
            let id = stream
            let n = buffer.withUnsafeMutableBufferPointer {
                mp_stream_read_marked(id, $0.baseAddress, cap, &mark, &marked)
            }
            if n > 0 {
                let bytes = Data(buffer[0..<n])
                lastFedAt = CACurrentMediaTime()
                bytesFed += UInt64(n)
                coordinator?.feed(data: bytes)
            }
            if marked {
                onMark?(mark)
                continue
            }
            if n < cap {
                pacer?.isPaused = true
                return
            }
            if CACurrentMediaTime() - start >= Self.budget {
                resumePacer()
                return
            }
        }
    }

    private func resumePacer() {
        if pacer == nil {
            let link = CADisplayLink(target: PacerTarget(self), selector: #selector(PacerTarget.frame(_:)))
            link.add(to: .main, forMode: .common)
            pacer = link
        }
        pacer?.isPaused = false
    }

    // MARK: Input

    /// Keystrokes, a paste, the terminal's replies: appended to the
    /// channel's input in order. False once the core's end is gone.
    @discardableResult
    func write(_ data: Data) -> Bool {
        let stream = self.stream
        let taken = data.withUnsafeBytes { raw in
            mp_stream_write(stream, raw.baseAddress?.assumingMemoryBound(to: UInt8.self), raw.count)
        }
        if taken { written += UInt64(data.count) }
        return taken
    }

    /// A paste, after the bytes written so far and ahead of any written
    /// after it: the core sends it as one paste bracketed by the pane's real
    /// mode, in order with the keystrokes and held with them while a return
    /// from the background waits for its answer (B361). Not stream bytes:
    /// `written` doesn't count it, as the core doesn't. False once the
    /// core's end is gone.
    @discardableResult
    func paste(_ text: String) -> Bool {
        let stream = self.stream
        var text = text
        return text.withUTF8 { raw in
            mp_stream_paste(stream, raw.baseAddress, raw.count)
        }
    }

    /// The shortcut bar's keys, after the bytes written so far: keystrokes
    /// while the session is live, never kept for a reconnect to deliver
    /// later (B476). Stream bytes all the same: `written` counts them, as
    /// the core does. False once the core's end is gone.
    @discardableResult
    func writeBar(_ data: Data) -> Bool {
        let stream = self.stream
        let taken = data.withUnsafeBytes { raw in
            mp_stream_bar(stream, raw.baseAddress?.assumingMemoryBound(to: UInt8.self), raw.count)
        }
        if taken { written += UInt64(data.count) }
        return taken
    }

    /// The view's grid, after the bytes written so far.
    @discardableResult
    func resize(cols: Int, rows: Int) -> Bool {
        mp_stream_resize(stream, UInt16(clamping: cols), UInt16(clamping: rows))
    }
}

/// What the waker holds: the feeder, weakly (a stream may outlive it until
/// the detach).
private final class WakerBox {
    weak var feeder: RustPaneFeeder?
    init(_ feeder: RustPaneFeeder) { self.feeder = feeder }
}

/// The channel's waker: on a core thread, at most once until drained; one
/// hop to the main queue (FIFO, so wakes and the events around them keep
/// their order there).
private let rustPaneWake: MpStreamWaker = { ctx in
    guard let ctx else { return }
    let box = Unmanaged<WakerBox>.fromOpaque(ctx).takeUnretainedValue()
    DispatchQueue.main.async {
        MainActor.assumeIsolated { box.feeder?.drain() }
    }
}

/// The pacer's target: a display link holds its target strongly.
private final class PacerTarget: NSObject {
    private weak var feeder: RustPaneFeeder?
    init(_ feeder: RustPaneFeeder) { self.feeder = feeder }
    @objc func frame(_ link: CADisplayLink) {
        MainActor.assumeIsolated { feeder?.drain() }
    }
}
