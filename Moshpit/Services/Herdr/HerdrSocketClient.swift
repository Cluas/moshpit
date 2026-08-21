import Foundation

/// One push from an `events.subscribe` subscription.
///
/// The envelope is `{"event":"<kind>","data":{…}}` — note there is NO id and
/// NO sequence number (herdr's event stream cannot be replayed; on reconnect
/// the client re-snapshots instead — see docs/design/roaming-transport.md).
/// `raw` is the full line so consumers that need the payload can decode it
/// with the tolerant `[String: Any]` machinery `HerdrSnapshot` already uses.
struct HerdrSocketEvent: Equatable {
    /// The `event` field, e.g. `pane_agent_status_changed`, `tab_created`.
    /// Snake_case on the wire (subscription REQUESTS use dotted names —
    /// `pane.agent_status_changed` — the asymmetry is herdr's, not ours).
    let kind: String
    let raw: String
}

enum HerdrSocketError: Error, Equatable {
    /// The server answered with `{"id":…,"error":{code,message}}`.
    case server(code: String, message: String)
    /// The transport ended (EOF / teardown) while the request was pending.
    /// herdr also CLOSES the connection after an `invalid_request`, so every
    /// in-flight request fails with this rather than hanging forever.
    case connectionClosed
    /// No reply within the deadline. Distinct from `connectionClosed`: the
    /// pipe may still be alive but wedged (the classic half-open SSH after
    /// suspension), and the caller's remedy differs (rebuild vs surface).
    case timeout
}

/// Speaks herdr's JSON socket API over any byte pipe.
///
/// Phase 0 of docs/design/roaming-transport.md: the client is deliberately
/// transport-agnostic — whoever owns the connection feeds inbound bytes via
/// ``feed(_:)`` (the `TmuxControlClient.feed` pattern) and provides a `write`
/// closure at init. Today that pipe is an SSH channel pumping to
/// `~/.config/herdr/herdr.sock`; Phase 1 swaps in a roaming-bridge channel
/// without touching this file.
///
/// Wire facts this encodes (verified against herdr 0.8.0 / protocol 19,
/// captures in MoshpitTests/Fixtures/herdr-socket-*.json*):
///   - Requests are one-line JSON `{"id","method","params"}`. `id` MUST be a
///     string (an integer id is silently ignored — no error, no reply) and
///     `params` MUST be present even when empty.
///   - Success is `{"id","result":{…}}`; failure `{"id","error":{code,message}}`.
///     A request the server cannot even parse comes back with `"id":""` and
///     the server then closes the connection.
///   - Subscription pushes are `{"event","data"}` lines interleaved with
///     responses on the same pipe.
///   - Subscribing replays the CURRENT state as synthetic created/focused
///     events before live ones — a subscribe IS a full bootstrap, which is
///     what makes reconnect cheap.
actor HerdrSocketClient {

    // MARK: Output

    /// Subscription pushes, in arrival order. One consumer (the controller).
    nonisolated let events: AsyncStream<HerdrSocketEvent>
    private let eventContinuation: AsyncStream<HerdrSocketEvent>.Continuation

    // MARK: Wiring

    private let write: @Sendable (Data) async throws -> Void
    private let requestTimeout: Duration

    /// Same cap and rationale as `HerdrFrameParser`: the pipe may ride a PTY
    /// login shell, so tolerate banners/prompts/noise between JSON lines, and
    /// never let a newline-free flood grow unbounded.
    private static let maxBufferBytes = 16 * 1024 * 1024
    private var buffer = Data()

    /// Resolved when the transport's pump announces it is connected — see
    /// ``HerdrPushBoot/readyMarker``. Until then anything written to the pipe
    /// may be going to a login shell that eats it.
    private var pumpReady = false
    private var pumpWaiters: [CheckedContinuation<Bool, Never>] = []

    private var nextId = 1
    private var pending: [String: CheckedContinuation<String, Error>] = [:]
    private var closed = false

    init(requestTimeout: Duration = .seconds(10),
         write: @escaping @Sendable (Data) async throws -> Void) {
        self.write = write
        self.requestTimeout = requestTimeout
        var cont: AsyncStream<HerdrSocketEvent>.Continuation!
        self.events = AsyncStream(bufferingPolicy: .unbounded) { cont = $0 }
        self.eventContinuation = cont
    }

    // MARK: Inbound

    /// Feed raw transport bytes. Complete lines are routed; partial lines are
    /// buffered across calls, and non-JSON lines (shell noise on a PTY pipe)
    /// are skipped rather than treated as corruption.
    func feed(_ data: Data) {
        buffer.append(data)
        if buffer.count > Self.maxBufferBytes { buffer.removeAll(keepingCapacity: false) }
        while let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            route(String(decoding: line, as: UTF8.self))
        }
    }

    /// The transport ended. Fails every in-flight request and finishes the
    /// event stream — callers decide whether to rebuild the pipe.
    func finishInput() {
        guard !closed else { return }
        closed = true
        failPumpWaiters()
        for (_, continuation) in pending {
            continuation.resume(throwing: HerdrSocketError.connectionClosed)
        }
        pending.removeAll()
        eventContinuation.finish()
    }

    /// Wait for the pump's readiness marker. `false` on timeout — the caller
    /// treats that as "this transport never came up" rather than writing into
    /// a pipe nobody is reading.
    func waitForPump(timeout: Duration) async -> Bool {
        if pumpReady { return true }
        let deadline = Task { [weak self] in
            try? await Task.sleep(for: timeout)
            await self?.failPumpWaiters()
        }
        defer { deadline.cancel() }
        return await withCheckedContinuation { continuation in
            pumpWaiters.append(continuation)
        }
    }

    private func failPumpWaiters() {
        let waiters = pumpWaiters
        pumpWaiters.removeAll()
        for waiter in waiters { waiter.resume(returning: false) }
    }

    private func notePumpReady() {
        guard !pumpReady else { return }
        pumpReady = true
        let waiters = pumpWaiters
        pumpWaiters.removeAll()
        for waiter in waiters { waiter.resume(returning: true) }
    }

    private func route(_ line: String) {
        // At the END of a line, never merely `contains`: the boot line carries
        // the marker inside its own python source and the shell echoes that
        // whole line back before `stty -echo` takes effect, so a loose match
        // would take our own echo as proof the pump is up — the exact failure
        // this handshake exists to prevent. Not line-EQUAL either: zsh emits
        // a window-title escape (`ESC k stty ESC \`) with no newline after
        // it, so the marker arrives glued to that prefix (observed on a real
        // login shell). The echoed command line ends in `")'`, never in the
        // marker, so the suffix is unambiguous.
        if line.trimmingCharacters(in: .whitespacesAndNewlines)
            .hasSuffix(HerdrPushBoot.readyMarker) {
            notePumpReady()
            return
        }
        guard let object = HerdrSnapshot.firstJSONObject(in: line) else { return }
        if let kind = object["event"] as? String {
            eventContinuation.yield(HerdrSocketEvent(kind: kind, raw: line))
            return
        }
        guard let id = object["id"] as? String else { return }
        if id.isEmpty {
            // Server-level parse failure: it doesn't know which request this
            // was, and it hangs up right after. Fail everything in flight so
            // no caller waits out a timeout on a connection that's gone.
            let error = Self.serverError(from: object)
                ?? HerdrSocketError.server(code: "invalid_request", message: line)
            for (_, continuation) in pending { continuation.resume(throwing: error) }
            pending.removeAll()
            return
        }
        guard let continuation = pending.removeValue(forKey: id) else { return }
        if let error = Self.serverError(from: object) {
            continuation.resume(throwing: error)
        } else {
            continuation.resume(returning: line)
        }
    }

    private static func serverError(from object: [String: Any]) -> HerdrSocketError? {
        guard let error = object["error"] as? [String: Any] else { return nil }
        return .server(code: error["code"] as? String ?? "unknown",
                       message: error["message"] as? String ?? "")
    }

    // MARK: Requests

    /// Send one request and await its reply line (the FULL envelope, so
    /// existing decoders like `HerdrSnapshot.decode` consume it unchanged).
    func request<P: Encodable>(_ method: String, params: P) async throws -> String {
        guard !closed else { throw HerdrSocketError.connectionClosed }
        let id = "mp\(nextId)"
        nextId += 1

        let envelope = RequestEnvelope(id: id, method: method, params: params)
        let data = try JSONEncoder().encode(envelope) + Data([UInt8(ascii: "\n")])

        // Deadline watchdog: on expiry the continuation is failed and the id
        // abandoned (a late reply routes to nobody, which is correct — the
        // caller already moved on).
        let deadline = Task { [requestTimeout] in
            try? await Task.sleep(for: requestTimeout)
            await self.expire(id: id)
        }
        defer { deadline.cancel() }

        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            Task {
                do { try await write(data) }
                catch { await self.fail(id: id, with: HerdrSocketError.connectionClosed) }
            }
        }
    }

    /// `request` with no parameters (`params` is mandatory on the wire even
    /// when empty — omitting it gets the silent-ignore treatment).
    func request(_ method: String) async throws -> String {
        try await request(method, params: EmptyParams())
    }

    /// Subscribe to event kinds. Global kinds take no filter; the per-pane
    /// kinds (`pane.agent_status_changed`, `pane.scroll_changed`,
    /// `pane.output_matched`) REJECT a bare subscription — they require a
    /// `pane_id`, so pass those through `paneScoped`.
    func subscribe(_ kinds: [String], paneScoped: [(kind: String, paneId: String)] = []) async throws {
        var subscriptions = kinds.map { Subscription(type: $0, paneId: nil) }
        subscriptions += paneScoped.map { Subscription(type: $0.kind, paneId: $0.paneId) }
        _ = try await request("events.subscribe", params: SubscribeParams(subscriptions: subscriptions))
    }

    private func expire(id: String) {
        fail(id: id, with: HerdrSocketError.timeout)
    }

    private func fail(id: String, with error: HerdrSocketError) {
        guard let continuation = pending.removeValue(forKey: id) else { return }
        continuation.resume(throwing: error)
    }

    // MARK: Wire shapes

    private struct RequestEnvelope<P: Encodable>: Encodable {
        let id: String
        let method: String
        let params: P
    }

    struct EmptyParams: Encodable {}

    struct Subscription: Encodable {
        let type: String
        let paneId: String?
        enum CodingKeys: String, CodingKey {
            case type
            case paneId = "pane_id"
        }
        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(type, forKey: .type)
            // Omit rather than null: the schema validates per-variant and a
            // spurious `pane_id: null` on a global kind is a gamble.
            if let paneId { try container.encode(paneId, forKey: .paneId) }
        }
    }

    private struct SubscribeParams: Encodable {
        let subscriptions: [Subscription]
    }
}
