import Foundation
import MoshpitKit
import Observation
import Security
import UIKit

/// The companion pet's host side (moshpit-design `docs/nsh/PET-DESIGN.md`
/// §15–16): the core's ``PetMachine`` decides everything, this supplies what
/// only the platform has — 16 bytes of `SecRandomCopyBytes` at the hatch,
/// the clock and the local hour, a device id, the preferences, and iCloud
/// key-value storage — and tells the pages when to post again
/// (``revision``).
///
/// iCloud: without the app's `com.apple.developer.ubiquity-kvstore-identifier`
/// entitlement the store syncs nothing, and the pet stays this device's.
@MainActor
@Observable
final class CompanionPet {
    static let shared = CompanionPet()

    /// Bumped whenever what the screens or the island show changed.
    private(set) var revision: UInt64 = 0

    /// Home is on screen (not under a pushed page): where the egg is tapped
    /// and hatches.
    @ObservationIgnored var onHome = false {
        didSet { if onHome != oldValue { logStatus() } }
    }
    @ObservationIgnored private var lastStatus = ""

    @ObservationIgnored let machine: PetMachine
    @ObservationIgnored private let cloud = UbiquitousPetCloud()
    @ObservationIgnored private var cloudWatch: NSObjectProtocol?

    private init() {
        let protectedData = ProtectedDataFlag()
        machine = PetMachine(preferences: RustPreferencesHost(protectedData: protectedData),
                             deviceId: Self.deviceId())
        // Rust calls it on any thread (a session's events, iCloud): it
        // only hops to the main actor.
        machine.setListener(listener: PetChangeListener {
            Task { @MainActor in
                CompanionPet.shared.revision &+= 1
                CompanionPet.shared.logStatus()
            }
        })
        machine.setCloud(cloud: cloud)
        cloudWatch = NotificationCenter.default.addObserver(
            forName: NSUbiquitousKeyValueStore.didChangeExternallyNotification,
            object: NSUbiquitousKeyValueStore.default, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.machine.cloudChanged() }
        }
        // The first sync settles it: the egg may hatch from then on, and a
        // pet on another device is adopted rather than a second hatched.
        if FileManager.default.ubiquityIdentityToken == nil {
            machine.cloudUnavailable()
        } else {
            NSUbiquitousKeyValueStore.default.synchronize()
            machine.cloudChanged()
        }
        syncIcon()
    }

    /// `{"op": "pet", …}`, for Home and Settings.
    var message: String { machine.message() }

    /// The Live Activity's stills for the agents' headline state.
    func island(state: String) -> PetIsland? { machine.island(state: state) }

    /// The screen asked to hatch the egg (it's ready, iCloud has settled):
    /// a secure seed, the time, the local hour.
    func hatch() {
        var seed = Data(count: 16)
        let status = seed.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress!) }
        guard status == errSecSuccess else { return Log.ui.error("pet: no random bytes (\(status))") }
        let hour = UInt8(Calendar.current.component(.hour, from: Date()))
        _ = machine.hatch(seed: seed, now: Self.now, localHour: hour)
    }

    /// A tap on the egg on Home: the first wobbles it, the second cracks
    /// it, the third hatches it (once iCloud has settled).
    func tapEgg() { _ = machine.tapEgg() }

    /// Home shows the pet: it may grow.
    func seen() { _ = machine.seen(now: Self.now) }

    /// The app came forward (B865): what a launch before the first unlock
    /// read as nothing — the pet shown as an egg — is read again.
    func reloadIfNeeded() { machine.reloadIfNeeded() }

    /// The egg's and the hatch's state, logged when it changes, so a device
    /// report can be read from the syslog (`pet: egg connections=… taps=…`).
    func logStatus() {
        let line = "\(machine.status()) on_home=\(onHome)"
        guard line != lastStatus else { return }
        lastStatus = line
        Log.ui.notice("pet: \(line, privacy: .public)")
    }

    func rename(_ name: String) { machine.rename(name: name, now: Self.now) }

    /// The app icon iOS shows and whether it was picked by hand (B668):
    /// Home's card offers the pet's icon from it.
    func syncIcon(_ settings: AppSettings = .shared) {
        machine.setIcon(current: AppIconModel().selectedID(settings),
                        byHand: settings.appIconPickedByHand)
    }

    /// A 「换」 under way: another answer waits for it.
    @ObservationIgnored private var switchingIcon = false

    /// The person answered Home's icon card (B668): 「换」 switches to the
    /// pet's icon through the gallery's own call. Never switched otherwise.
    func answerIcon(switch change: Bool, settings: AppSettings = .shared) {
        guard !switchingIcon else { return }
        switchingIcon = change
        Self.answer(
            switch: change, offered: Self.offeredIcon(machine.message()),
            select: { id, done in
                Log.ui.notice("pet: switching the app icon to \(id, privacy: .public)")
                AppIconModel().select(id, settings: settings, byHand: false, done: done)
            },
            answered: { [weak self] in self?.machine.iconAnswered() },
            synced: { [weak self] in
                self?.switchingIcon = false
                self?.syncIcon(settings)
            })
    }

    /// What an answer to Home's icon card does: 「先不换」 is the answer at
    /// once, this look not asked about again; 「换」 is the answer once iOS
    /// has switched (`select`, then `synced` either way).
    ///
    /// B608: it was the answer whatever iOS said. A switch iOS refused (the
    /// app not in front, an icon missing from the bundle) took the card
    /// away, left the icon as it was, said nothing on Home, and the look
    /// was never offered again. Refused, the card stays to be tried again,
    /// and the log says so.
    @MainActor
    static func answer(switch change: Bool, offered: String?,
                       select: (String, @escaping @MainActor (Bool) -> Void) -> Void,
                       answered: @escaping @MainActor () -> Void,
                       synced: @escaping @MainActor () -> Void) {
        guard change, let id = offered else {
            answered()
            if change { synced() }
            return
        }
        select(id) { switched in
            if switched {
                answered()
            } else {
                Log.ui.error("pet: iOS didn't switch the app icon to \(id, privacy: .public); the card asks again")
            }
            synced()
        }
    }

    /// The icon Home's card offers, from the screens' message.
    nonisolated static func offeredIcon(_ message: String) -> String? {
        guard let data = message.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let icon = object["icon"] as? [String: Any]
        else { return nil }
        return icon["id"] as? String
    }

    /// Settings › Appearance: 「让它说话」.
    func setTalk(_ on: Bool) { machine.setTalk(talk: on) }

    /// Settings › Appearance: the style (`minimal`, `pixel`) and the switch.
    func choose(style: String, shown: Bool) { machine.setChoice(style: style, shown: shown) }

    static var now: Int64 { Int64(Date().timeIntervalSince1970) }

    /// A stable id for this device: its tasks count under it.
    ///
    /// Kept only once the preferences can be believed (B866): a launch
    /// before the first unlock reads them as empty, and a write then can
    /// leave the app's preferences holding this one key (B603) — every
    /// connection and setting gone. Read sealed, the id is this launch's
    /// alone, and the next trusted launch keeps one.
    static func deviceId(_ defaults: UserDefaults = .standard,
                         protectedData: @escaping () -> Bool = { ProtectedDataFlag.app.isAvailable }) -> String {
        let key = "moshpit.pet.device"
        if let id = defaults.string(forKey: key) { return id }
        let id = UUID().uuidString
        let host = RustPreferencesHost(defaults: defaults, available: protectedData)
        if storeDefaultsReadable(host: host, mode: .system) {
            defaults.set(id, forKey: key)
        }
        return id
    }
}

/// iCloud key-value storage under `moshpit.pet.v1`.
final class UbiquitousPetCloud: PetCloudHost, @unchecked Sendable {
    private let key = "moshpit.pet.v1"

    func get() -> Data? { NSUbiquitousKeyValueStore.default.data(forKey: key) }

    func put(record: Data) {
        NSUbiquitousKeyValueStore.default.set(record, forKey: key)
        NSUbiquitousKeyValueStore.default.synchronize()
    }
}

/// Hears the core say something changed.
final class PetChangeListener: PetListener, @unchecked Sendable {
    private let onChange: @Sendable () -> Void
    init(_ onChange: @escaping @Sendable () -> Void) { self.onChange = onChange }
    func changed() { onChange() }
}
