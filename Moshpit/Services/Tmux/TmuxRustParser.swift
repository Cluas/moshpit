#if canImport(MoshpitFFI)
import Foundation
import MoshpitFFI

// MARK: - TmuxRustParser

/// The Rust tmux control-mode parser (crates/tmux), through the C ABI of the
/// app's Rust library (crates/ffi, `mp_tmux_parser_*` in
/// crates/ffi/include/moshpit_ffi.h): what a ``TmuxControlClient`` runs in
/// place of its own parser when ``TmuxParserBackend`` says `.rust`.
///
/// One per client, used only on the client's actor: the Rust parser is not
/// thread-safe, and it calls back synchronously during a feed.
final class TmuxRustParser {
    fileprivate let handle: OpaquePointer

    /// nil when the linked library is not the one this app was compiled
    /// against (see ``TmuxParserBackend/rustAvailable``).
    init?() {
        guard TmuxParserBackend.rustAvailable else { return nil }
        handle = mp_tmux_parser_new()
    }

    deinit {
        mp_tmux_parser_free(handle)
    }

    func reset() {
        mp_tmux_parser_reset(handle)
    }

    /// Why the parser last panicked, if it did.
    fileprivate var panicMessage: String {
        mp_tmux_parser_panic_message(handle).string
    }
}

extension TmuxParserBackend {
    /// Whether this build can run the Rust parser: the library is linked and
    /// is the one the app was compiled against. A stale libmoshpit_ffi.a
    /// would be misread, not just fail, so it is refused.
    static let rustAvailable = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)
}

// MARK: - Feeding

extension TmuxControlClient {
    /// ``feed(_:)`` through the Rust parser. Every event arrives through
    /// ``rustEventSink`` before `mp_tmux_parser_feed` returns, still on this
    /// actor, and fires the same callback the Swift parser would.
    func feedRust(_ parser: TmuxRustParser, _ data: Data) {
        let context = Unmanaged.passUnretained(self).toOpaque()
        let result = data.withUnsafeBytes { raw in
            mp_tmux_parser_feed(parser.handle,
                                raw.baseAddress?.assumingMemoryBound(to: UInt8.self), raw.count,
                                rustEventSink, context)
        }
        guard result != .ok else { return }
        let reason = result == .panicked
            ? "Rust tmux parser panicked: \(parser.panicMessage)"
            : "Rust tmux parser refused its input (result \(result.rawValue))"
        rustParserFailed(reason: reason)
    }

    /// The Rust parser failed — a bug, caught at the C boundary rather than
    /// taking the app down. Carry on with the Swift parser, keep new clients
    /// off the Rust one for the rest of the process, and tell the controller
    /// to start the control session over: the chunk the parser failed on is
    /// gone, and with it maybe the end of a reply some command waits for.
    func rustParserFailed(reason: String,
                          latch: TmuxParserBackend.Latch = TmuxParserBackend.rustFailure) {
        latch.trip()
        dropRustParser()
        onProtocolError?(.malformedLine(reason))
        onParserFailure?(reason)
    }

    /// One Rust event, as the Swift parser would have reported it.
    fileprivate func dispatchRust(_ event: MpTmuxEvent) {
        switch event.kind {
        case .paneOutput:
            onPaneOutput?(event.first.string, event.second.data)
        case .layoutChange:
            onLayoutChange?(event.first.string, event.second.string, event.flag)
        case .windowAdd:
            onWindowAdd?(event.first.string)
        case .windowClose:
            onWindowClose?(event.first.string)
        case .windowRenamed:
            onWindowRenamed?(event.first.string, event.second.string)
        case .sessionChanged:
            onSessionChanged?(event.first.string, event.second.string)
        case .sessionRenamed:
            onSessionRenamed?(event.first.string, event.second.string)
        case .sessionWindowChanged:
            onSessionWindowChanged?(event.first.string, event.second.string)
        case .sessionsChanged:
            onSessionsChanged?()
        case .unlinkedWindowAdd:
            onUnlinkedWindowAdd?(event.first.string)
        case .unlinkedWindowClose:
            onUnlinkedWindowClose?(event.first.string)
        case .unlinkedWindowRenamed:
            onUnlinkedWindowRenamed?(event.first.string, event.second.string)
        case .clientSessionChanged:
            onClientSessionChanged?(event.first.string, event.second.string, event.third.string)
        case .activePaneChanged:
            onActivePaneChanged?(event.first.string, event.second.string)
        case .pause:
            onPause?(event.first.string)
        case .continue:
            onContinue?(event.first.string)
        case .clientDetached:
            onClientDetached?(event.first.string)
        case .exit:
            onExit?(event.flag ? event.first.string : nil)
        case .commandResponse:
            guard let onCommandResponse else { return }
            let lines = UnsafeBufferPointer(start: event.lines, count: event.line_count)
            onCommandResponse(TmuxCommandResponse(
                commandId: Int(event.command_id),
                commandNum: Int(event.command_num),
                isError: event.flag,
                lines: lines.map(\.string),
                fromClient: event.from_client))
        case .protocolError:
            onProtocolError?(.malformedLine(event.first.string))
        @unknown default:
            // A kind newer than this app: impossible while the ABI version
            // matches, and never worth a crash.
            onProtocolError?(.malformedLine("unknown Rust tmux event kind \(event.kind.rawValue)"))
        }
    }
}

/// Receives the Rust parser's events. `mp_tmux_parser_feed` calls it before
/// it returns, so on the actor running ``TmuxControlClient/feedRust(_:_:)``,
/// whose client `context` is.
private let rustEventSink: MpTmuxEventSink = { context, event in
    guard let context else { return }
    let client = Unmanaged<TmuxControlClient>.fromOpaque(context).takeUnretainedValue()
    client.assumeIsolated { $0.dispatchRust(event.pointee) }
}

// MARK: - Bytes

private extension MpBytes {
    /// The bytes as a String; the ABI promises UTF-8 for every text field.
    var string: String {
        String(decoding: UnsafeBufferPointer(start: ptr, count: len), as: UTF8.self)
    }

    /// A copy of the bytes.
    var data: Data {
        guard let ptr, len > 0 else { return Data() }
        return Data(bytes: ptr, count: len)
    }
}
#endif
