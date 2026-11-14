// store-swift-rust's stores: the app's store classes on its Rust-store
// adapters (Moshpit/Services/Rust/RustStores.swift, unchanged) over the
// Rust library — what the app runs with the MOSHPIT_RUST_STORE switch on.
// The preferences reach the Rust store through the app's own
// RustPreferencesHost and the keychain through its RustKeychainHost (both
// cut verbatim out of RustCoreHosts.swift by build-swift.sh), over the
// case's UserDefaults suite and test keychain; the sealed-launch rule asks
// the case's phone. main.swift makes every store through these functions;
// swift/Stores.swift has the same ones on the Swift stores (store-swift).
//
// One command besides `run`:
//
//   store-swift-rust pairings <seed> <count> <adapter-files-out> <swift-files-out>
//
// pushes generated pairings through the app's pairings adapter
// (RustPushPairings) and through the Swift PushPairingFile, each reading the
// other's files (a disagreement fails the run), and writes each file the
// adapter left and the one the Swift left (hex, one per line, in
// store-swift-108's `pairings` input format): equivalence.py hands both to
// 1.0.8, which must read the two alike.

import Foundation
import MoshpitKit

/// The app's preferences host over `defaults`, readable while `phone` (nil:
/// always) is unlocked.
func preferences(_ defaults: UserDefaults, _ phone: Phone?) -> RustPreferencesHost {
    RustPreferencesHost(defaults: defaults, protectedData: ProtectedDataFlag { phone?.unlocked ?? true })
}

func makeConnectionStore(_ defaults: UserDefaults, _ phone: Phone) -> ConnectionStore {
    ConnectionStore(backing: RustConnectionStore(preferences: preferences(defaults, phone), mode: .system))
}

func makeKeyStore(_ defaults: UserDefaults, _ phone: Phone) -> SSHKeyStore {
    SSHKeyStore(backing: RustSSHKeyStore(preferences: preferences(defaults, phone), mode: .system))
}

func makeShortcutStore(_ defaults: UserDefaults, _ phone: Phone) -> ShortcutStore {
    ShortcutStore(backing: RustShortcutStore(preferences: preferences(defaults, phone), mode: .system))
}

/// `phone` nil: always readable (the export op's scratch store).
func makeThemeStore(_ defaults: UserDefaults, _ phone: Phone?) -> ThemeStore {
    ThemeStore(backing: RustThemeStore(preferences: preferences(defaults, phone), mode: .system,
                                       untitled: String(localized: "Untitled")))
}

func makeAppThemeStore(_ defaults: UserDefaults, _ phone: Phone) -> AppThemeStore {
    AppThemeStore(backing: RustAppThemeStore(preferences: preferences(defaults, phone), mode: .system,
                                             untitled: String(localized: "Untitled")))
}

/// The settings mirror their two switches into the App Group's suite, as
/// the app's do (RustStoreSwitch.swift).
func makeSettings(_ defaults: UserDefaults, _ phone: Phone) -> AppSettings {
    AppSettings(defaults: defaults,
                backing: RustSettings(preferences: preferences(defaults, phone),
                                      mirror: preferences(groupSuite, nil), mode: .system, debugBuild: false))
}

/// The keychain item through the app's keychain host over the test
/// keychain. No unlock probe, as the Swift store here has none: "no item"
/// is believed (the sealed device is the `hosts` cases' kclock).
func makeKnownHosts(_ keychain: TestKeychain) -> KnownHostsStore {
    RustKnownHostsStore(keychain: RustKeychainHost(backend: keychain, launchTrust: nil),
                        preferences: preferences(.standard, nil))
}

func readinessCheck(_ defaults: UserDefaults, _ phone: Phone) -> Bool {
    storeDefaultsReadable(host: preferences(defaults, phone), mode: .system)
}

func readinessMark(_ defaults: UserDefaults, _ phone: Phone) {
    storeMarkDefaultsReadable(host: preferences(defaults, phone))
}

// The app makes its two singletons where it reads its storage switch
// (Moshpit/Services/Rust/RustStoreSwitch.swift, not compiled here);
// UI/AppTheme.swift names them. Nothing here reads them.
extension AppSettings {
    static let shared = AppSettings(readiness: .system)
}

extension AppThemeStore {
    static let shared = AppThemeStore(readiness: .system)
}

// MARK: - Pairings

/// A tiny deterministic generator (xorshift64*), so a seed names a run.
struct Draw {
    var state: UInt64
    init(_ seed: UInt64) { state = seed &* 0x9E37_79B9_7F4A_7C15 | 1 }
    mutating func next() -> UInt64 {
        state ^= state >> 12; state ^= state << 25; state ^= state >> 27
        return state &* 0x2545_F491_4F6C_DD1D
    }
    mutating func below(_ n: Int) -> Int { Int(next() % UInt64(n)) }
    mutating func pick<T>(_ items: [T]) -> T { items[below(items.count)] }
}

// Not a label that starts with U+FEFF: moshpit-seal keeps it, the Swift
// file drops it as JSONSerialization does (B264, harness/divergences.json
// "pairing-label-bom"). One later in the label is kept by both.
let pairingTexts = ["", "box", "b\u{FEFF}ox", "büro · 研究所", "a\"b\\c/d", "🧪 pi", "x y", "\u{0301}e", "tab\there"]

func pairing(_ draw: inout Draw, id: UUID) -> PushPairing {
    let dates: [Double] = [0, 784_000_000, 784_000_000.25, -1.5, 1e9 + 0.125, 812_345_678.987654]
    var p = PushPairing(connectionId: id, hostLabel: draw.pick(pairingTexts),
                        secretHex: String(repeating: draw.pick(["ab", "00", "f1"]), count: 32),
                        sendToken: draw.pick(["", "tok.en", "a/b+c=="]),
                        relayURL: draw.pick(["https://relay.example.com", "http://127.0.0.1:8080", ""]),
                        createdAt: Date(timeIntervalSinceReferenceDate: draw.pick(dates)))
    if draw.below(2) == 0 { p.apnsToken = draw.pick(["", "ABCDEF0123", "abcdef0123"]) }
    if draw.below(2) == 0 { p.apnsEnv = draw.pick(["production", "sandbox"]) }
    if draw.below(2) == 0 { p.sendTokenIssuedAt = Date(timeIntervalSinceReferenceDate: draw.pick(dates)) }
    return p
}

func runOtherCommand(_ args: [String]) -> Bool {
    guard args.count == 6, args[1] == "pairings", let seed = UInt64(args[2]), let count = Int(args[3]) else {
        return false
    }
    var draw = Draw(seed)
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("w5b-pairings-\(ProcessInfo.processInfo.processIdentifier)")
    try? FileManager.default.removeItem(at: dir)
    defer { try? FileManager.default.removeItem(at: dir) }
    var files: [String] = []
    var swiftFiles: [String] = []
    var failures = 0
    for n in 0..<count {
        let ours = dir.appendingPathComponent("\(n)/push-pairings.json")
        let theirs = dir.appendingPathComponent("\(n)/swift/push-pairings.json")
        let adapter = RustPushPairings(url: ours)
        let swift = PushPairingFile(url: theirs)
        let ids = (0..<1 + draw.below(4)).map { _ in UUID() }
        // The same operations on both: upserts, a re-pair, a forget, maybe a
        // whole write.
        var ops: [(String, PushPairing?, UUID?)] = []
        for id in ids { ops.append(("upsert", pairing(&draw, id: id), nil)) }
        if draw.below(2) == 0 { ops.append(("upsert", pairing(&draw, id: draw.pick(ids)), nil)) }
        if draw.below(2) == 0 { ops.append(("remove", nil, draw.pick(ids))) }
        if draw.below(4) == 0 { ops.append(("remove", nil, UUID())) }
        for (op, p, id) in ops {
            let a: Bool, b: Bool
            if op == "upsert" {
                a = adapter.upsert(p!)
                b = swift.upsert(p!)
            } else {
                a = adapter.remove(connectionId: id!)
                b = swift.remove(connectionId: id!)
            }
            if a != b {
                failures += 1
                FileHandle.standardError.write(Data("pairings \(n): \(op) answered \(a) vs Swift \(b)\n".utf8))
            }
        }
        if draw.below(4) == 0 {
            let all = ids.map { pairing(&draw, id: $0) }
            if adapter.write(all) != swift.write(all) {
                failures += 1
                FileHandle.standardError.write(Data("pairings \(n): write answered differently\n".utf8))
            }
        }
        // Each side reads the other's file as its own.
        let mine = adapter.read(), yours = swift.read()
        let swiftReadsMine = PushPairingFile(url: ours).read()
        let iReadYours = RustPushPairings(url: theirs).read()
        if mine != yours || swiftReadsMine != mine || iReadYours != yours {
            failures += 1
            FileHandle.standardError.write(Data("pairings \(n): the two sides read differently\n".utf8))
        }
        let bytes = (try? Data(contentsOf: ours)) ?? Data()
        let swiftBytes = (try? Data(contentsOf: theirs)) ?? Data()
        files.append(bytes.isEmpty ? "~" : hex([UInt8](bytes)))
        swiftFiles.append(swiftBytes.isEmpty ? "~" : hex([UInt8](swiftBytes)))
    }
    try! (files.joined(separator: "\n") + "\n").write(toFile: args[4], atomically: true, encoding: .utf8)
    try! (swiftFiles.joined(separator: "\n") + "\n").write(toFile: args[5], atomically: true, encoding: .utf8)
    if failures > 0 {
        FileHandle.standardError.write(Data("\(failures) pairing disagreement(s)\n".utf8))
        exit(1)
    }
    return true
}
