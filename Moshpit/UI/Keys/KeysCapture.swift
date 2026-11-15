#if DEBUG
import SwiftUI

/// A capture run of the keys pages (DEBUG only): the keys they show, held in
/// memory and never written (`-MOSHPIT_SEED_KEYS demo`, or `empty` for
/// none), and the sheet or card a `-MOSHPIT_SEED_SCREEN` asks for over SSH
/// Keys.
enum KeysCapture {
    /// The keys the pages show instead of the device's.
    static var seeded: Bool {
        ProcessInfo.processInfo.arguments.contains("-MOSHPIT_SEED_KEYS")
    }

    /// `-MOSHPIT_SEED_KEYS empty`: no keys at all, for the pages' empty
    /// states.
    static var seedsNone: Bool {
        let arguments = ProcessInfo.processInfo.arguments
        guard let at = arguments.firstIndex(of: "-MOSHPIT_SEED_KEYS"), at + 1 < arguments.count else {
            return false
        }
        return arguments[at + 1] == "empty"
    }

    /// A Secure Enclave key, an imported key two connections sign in with,
    /// and an RSA key nobody used since March: each kind of row.
    static let records: [SSHKeyRecord] = {
        let march = DateComponents(calendar: Calendar(identifier: .gregorian), year: 2026, month: 3, day: 9).date!
        return [
            SSHKeyRecord(
                id: UUID(uuidString: "5E000000-0000-4000-8000-000000000001")!,
                name: "iPad mini", algorithm: .seP256, comment: "moshpit",
                fingerprint: "SHA256:qK7mU3hVr1ycWJ5n0pE2aLx9TfB4gD6sZo8iNcR1yHk",
                publicKey: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTY= moshpit",
                source: .secureEnclave),
            SSHKeyRecord(
                id: UUID(uuidString: "1A000000-0000-4000-8000-000000000002")!,
                name: "work laptop", algorithm: .ed25519, comment: "dev@laptop",
                fingerprint: "SHA256:Hc4vXw9LmQ2tR7yP0aS3dF6gJ8kZ1xC5vB2nM4qW7eE",
                publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample dev@laptop",
                source: .imported),
            SSHKeyRecord(
                id: UUID(uuidString: "0D000000-0000-4000-8000-000000000003")!,
                name: "old build box", algorithm: .rsa4096, bits: 3072, comment: "",
                fingerprint: "SHA256:Zp9wQ1eR5tY7uI3oP2aS4dF6gH8jK0lL1zX3cV5bN7m",
                publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQExample",
                source: .generated, lastUsedAt: march),
        ]
    }()

    /// The connections that sign in with each of ``records``, as the rows
    /// say (B602): a capture run's connections don't name these keys.
    static let users: [UUID: [String]] = [
        UUID(uuidString: "5E000000-0000-4000-8000-000000000001")!: ["m1-pro"],
        UUID(uuidString: "1A000000-0000-4000-8000-000000000002")!: ["m1-pro", "nas"],
    ]

    /// The store a capture run's pages read: these keys, in memory.
    @MainActor static let store = SSHKeyStore.capture(seedsNone ? [] : records)

    /// Puts ``store`` in the environment of a capture run that seeds keys.
    struct Store: ViewModifier {
        func body(content: Content) -> some View {
            if KeysCapture.seeded {
                content.environment(KeysCapture.store)
            } else {
                content
            }
        }
    }

    /// The key whose delete card a capture run shows: the first imported
    /// key.
    static func deleteRequested(in snapshot: SSHKeysSnapshot) -> UUID? {
        guard CaptureScreen.requested == .sshKeyDelete else { return nil }
        return snapshot.imported.first?.id ?? snapshot.device.first?.id
    }

    /// Add Key (in its first method, or Import) or Select Key over SSH Keys,
    /// once the keys' sheet is up.
    struct Present: ViewModifier {
        @Environment(SSHKeyStore.self) private var keyStore
        @State private var showAddKey = false
        @State private var showPicker = false
        @State private var picked: UUID?

        func body(content: Content) -> some View {
            content
                .task {
                    guard let screen = CaptureScreen.requested else { return }
                    try? await Task.sleep(for: .milliseconds(800))
                    showAddKey = [.addKey, .addKeyImport].contains(screen)
                    if screen == .keyPicker {
                        picked = keyStore.keys.dropFirst().first?.id
                        showPicker = true
                    }
                }
                .appSheet(isPresented: $showAddKey) {
                    AddKeyNSHPage(method: CaptureScreen.requested == .addKeyImport ? .importKey : .generate)
                }
                .appSheet(isPresented: $showPicker) {
                    KeyPickerNSHPage(keys: keyStore.keys, selectedKeyId: $picked)
                }
        }
    }
}
#endif
