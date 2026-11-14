import Foundation
import MoshpitKit
import Observation
import UIKit

/// What a session on the Rust core hears: its events, on the main actor, in
/// the order the core produced them.
@MainActor
protocol RustSessionListener: AnyObject {
    func handle(_ event: SessionEventKind)
}

/// The app's one registry of Rust sessions (crates/ffi `CoreSessions`) and
/// what the app tells it about itself: the foreground, the network, the
/// terminal on screen. Made the first time a connection starts on the Rust
/// core; an app that never turns the switch on never makes it (and never
/// starts the core's threads).
///
/// Events arrive in batches on the main queue (``RustEventSink``) and go to
/// the session they name, in order; the registry-wide ones (a flush's
/// answer) are handled here.
@MainActor
final class RustCore {
    /// The registry, if one was made.
    private(set) static var current: RustCore?

    /// The registry, made on first use.
    static var shared: RustCore {
        if let current { return current }
        let core = RustCore()
        current = core
        return core
    }

    let sessions: CoreSessionsProtocol
    let credentials: RustCredentialHost
    /// What a view that reads the registry's views observes (Home's record,
    /// ``HomeNSHPage``): it changes after each batch of the sessions'
    /// events, once their listeners have taken them.
    let changes = RustCoreChanges()
    private var listeners: [UInt64: WeakListener] = [:]
    private var flushes: [UInt64: () -> Void] = [:]
    private var pathWatch: NetworkPathWatch?
    private var pathKick: Task<Void, Never>?
    private var unlockWatch: NSObjectProtocol?
    private(set) var isForeground = true

    private struct WeakListener {
        weak var listener: RustSessionListener?
    }

    private convenience init() {
        let protectedData = ProtectedDataFlag()
        let credentials = RustCredentialHost(protectedData: { protectedData.isAvailable })
        var receive: (@MainActor ([SessionEvent]) -> Void)?
        let sink = RustEventSink { events in receive?(events) }
        let label = UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        let sessions = CoreSessions(
            credentials: credentials,
            keychain: RustKeychainHost(),
            preferences: RustPreferencesHost(protectedData: protectedData),
            events: sink,
            deviceLabel: label)
        self.init(sessions: sessions, credentials: credentials)
        receive = { [weak self] events in self?.deliver(events) }
        isForeground = UIApplication.shared.applicationState != .background
        if !isForeground { sessions.setForeground(foreground: false) }
        noteResumeInPlace()
        startPathWatch()
        watchUnlocks(on: .default)
        Log.ssh.info("rust core: session registry up (\(label, privacy: .public))")
    }

    /// The device's unlock, heard on `center`, told to the core: a session
    /// whose keychain read failed while the device was locked dials again
    /// once it isn't (B201, B211). Once per registry; a test posts on its
    /// own center.
    func watchUnlocks(on center: NotificationCenter) {
        guard unlockWatch == nil else { return }
        unlockWatch = center.addObserver(
            forName: UIApplication.protectedDataDidBecomeAvailableNotification,
            object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.sessions.deviceUnlocked() }
        }
    }

    /// Over any registry: tests drive the routing with a fake.
    init(sessions: CoreSessionsProtocol, credentials: RustCredentialHost) {
        self.sessions = sessions
        self.credentials = credentials
    }

    // MARK: Sessions

    func listen(_ session: UInt64, _ listener: RustSessionListener) {
        listeners[session] = WeakListener(listener: listener)
    }

    func forget(_ session: UInt64) {
        listeners[session] = nil
    }

    /// One batch from the core, in order.
    func deliver(_ events: [SessionEvent]) {
        var heard = false
        for event in events {
            if event.session == 0 {
                if case .flushed(let token) = event.kind {
                    flushes.removeValue(forKey: token)?()
                }
                continue
            }
            heard = true
            listeners[event.session]?.listener?.handle(event.kind)
        }
        if heard { changes.batches &+= 1 }
    }

    // MARK: The app

    /// scenePhase: the foreground (`true`) or the background. Leaving hands
    /// tmux's window pins back, and the process is held awake (a background
    /// assertion) until the sessions say the release went out — or the
    /// core's 5 s bound passes.
    func setForeground(_ foreground: Bool) {
        guard foreground != isForeground else { return }
        isForeground = foreground
        sessions.setForeground(foreground: foreground)
        noteResumeInPlace()
        guard !foreground else { return }
        let assertion = BackgroundAssertion(name: "moshpit.rust-background-teardown")
        flush { assertion.end() }
    }

    /// What the core last decided of resume in place.
    private var resumeInPlace: ResumeInPlaceDecision?

    /// The core's decision on resume in place, logged when it changes (B365):
    /// it reads ``ResumeInPlaceSwitch`` again around every move to the
    /// background and back, and a device's log then says which way back its
    /// sessions take, and why.
    private func noteResumeInPlace() {
        let decision = sessions.resumeInPlace()
        guard decision != resumeInPlace else { return }
        resumeInPlace = decision
        Log.ssh.notice("rust core: resume in place \(decision.on ? "on" : "off", privacy: .public) (\(ResumeInPlaceSwitch.words(decision.source), privacy: .public))")
    }

    /// Calls `done` once every session has written what it was asked to
    /// send so far (bounded by the core).
    func flush(_ done: @escaping () -> Void) {
        let token = sessions.flush()
        flushes[token] = done
    }

    /// The terminal on screen changed (`nil`: none, Home).
    func setVisible(_ session: UInt64?) {
        sessions.setVisible(session: session)
    }

    /// The session whose terminal is on screen (the hub's
    /// `visibleSession`), kept even before a registry exists: a first Rust
    /// session is set visible before it starts.
    private(set) static weak var visibleSession: SessionHub.ActiveSession?

    /// Only the Rust session on screen takes its tmux window back when the
    /// app returns; leaving the terminal hands it back.
    static func visibleSessionChanged(_ session: SessionHub.ActiveSession?) {
        visibleSession = session
        current?.setVisible(session?.rust?.id)
        #if DEBUG
        RustSessionSeed.sessionBecameVisible(session)
        RustSideProbe.sessionBecameVisible(session)
        #endif
    }

    /// Both monitors the Swift hub watches — the default path and `.other`
    /// (a split-tunnel VPN such as Tailscale comes up without moving the
    /// default path). Only acted on in the foreground: the app can't hold a
    /// socket in the background, and the foreground return resumes anyway.
    /// A burst (a tunnel coming up) is told once; the core settles it again.
    private func startPathWatch() {
        let watch = NetworkPathWatch { [weak self] in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.networkPathChanged() }
            }
        }
        watch.start()
        pathWatch = watch
    }

    private func networkPathChanged() {
        guard isForeground else { return }
        pathKick?.cancel()
        pathKick = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled, let self else { return }
            Log.ssh.debug("rust core: network path changed")
            self.sessions.networkChanged()
        }
    }
}

/// ``RustCore/changes``.
@MainActor
@Observable
final class RustCoreChanges {
    /// How many batches of session events the registry has delivered.
    fileprivate(set) var batches: UInt64 = 0
}
