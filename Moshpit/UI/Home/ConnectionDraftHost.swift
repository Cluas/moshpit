import Foundation

// The connection editor's draft is the shared Rust model's
// (`crates/app/src/connection_form.rs`, through `crates/ffi/src/app.rs`),
// the one Android runs too: ``ConnectionDraft`` is its record, and its
// rules — the ports, Save, the credential a save points the connection at
// (B294, B298) — are Rust functions. Both iOS forms drive it: the SwiftUI
// one (``AddConnectionView``) and the NSH one (``ConnectionFormNSHPage``).
//
// What stays here is the host's: the password and the pasted key
// (``ConnectionSecrets``), which never reach Rust — the model hears only
// whether each holds one — and the keychain and the store a save writes.

extension ConnectionDraft {
    /// A new connection's draft: everything at its default, mosh as the
    /// "Mosh by default" setting says (B256).
    init(newWith settings: AppSettings) {
        self = connectionDraftNew(moshByDefault: settings.moshByDefault)
    }

    /// The draft of a saved connection, as the editor opens it.
    init(_ c: ServerConnection) {
        // A Swift connection's id is a UUID, which is all the model can
        // refuse.
        self = (try? connectionDraftOf(connection: ConnectionRecord(c)))
            ?? connectionDraftNew(moshByDefault: c.connectionProtocol == .mosh)
    }

    var authMethod: AuthMethod {
        get { auth == .key ? .key : .password }
        set { auth = newValue == .key ? .key : .password }
    }

    /// The managed key picked from SSH Keys; nil: paste a PEM.
    var selectedKeyId: UUID? {
        get { selectedKey.flatMap(UUID.init(uuidString:)) }
        set { selectedKey = newValue?.uuidString }
    }

    var predictMode: PredictMode {
        get { PredictMode(rawValue: predictModeRaw) ?? .adaptive }
        set { predictModeRaw = newValue.rawValue }
    }

    var multiplexer: Multiplexer {
        get { Multiplexer(rawValue: multiplexerRaw) ?? .tmux }
        set { multiplexerRaw = newValue.rawValue }
    }

    /// Save's rule: a name, a host, ports that are ports, and something to
    /// sign in with, `keys` being SSH Keys' now.
    func canSave(keys: [SSHKeyRecord], secrets: ConnectionSecrets) -> Bool {
        connectionDraftCanSave(draft: self, given: secrets.given, keys: keys.map(KeyRecord.init))
    }

    /// The port field holds no port (the form says so under it).
    var portIsWrong: Bool { connectionDraftPortIsWrong(draft: self) }

    /// The proxy's port field holds no port, while the proxy is on.
    var proxyPortIsWrong: Bool { connectionDraftProxyPortIsWrong(draft: self) }

    /// The connection edited keeps a key that saving with none picked or
    /// pasted keeps.
    var keepsStoredKey: Bool { connectionDraftKeepsStoredKey(draft: self) }

    /// The connection edited keeps a password of its own.
    var keepsStoredPassword: Bool { connectionDraftKeepsStoredPassword(draft: self) }

    /// `connection` with the draft's fields (the ones saving writes).
    func applied(to connection: ServerConnection, settings: AppSettings) -> ServerConnection {
        let record = try? connectionDraftApplied(
            draft: self, connection: ConnectionRecord(connection),
            udpStart: Int64(settings.udpRangeStart), udpEnd: Int64(settings.udpRangeEnd))
        return record.flatMap(ServerConnection.init) ?? connection
    }

    /// Saves the draft over `existing` (or as a new connection): the model
    /// decides (``connectionDraftSavePlan(draft:existing:fresh:keys:given:udpStart:udpEnd:newRef:)``),
    /// then the secret it names goes to the keychain and the connection to
    /// `store`. Throws ``ConnectionSaveRefusal`` or, when the keychain kept
    /// nothing, ``ConnectionKeychainFailure``, with nothing saved. Once
    /// saved, the item the connection owned and no longer names (it signs
    /// in with a managed key now, B325) goes to `release`.
    ///
    /// A connection now pointing at another host or port (B328) first has
    /// its push pairing forgotten (`forgetPairing`, ``HostCare`` by
    /// default): it belongs to the box it pointed at before, which would go
    /// on pushing under it. That is the one step that can fail, so it goes
    /// first, and a phone that can't let go saves nothing
    /// (``PushPairingForgetFailure``).
    @MainActor
    func save(existing: ServerConnection?, store: ConnectionStore, keychain: KeychainService,
              settings: AppSettings, keys: [SSHKeyRecord], secrets: ConnectionSecrets,
              release: (String) async -> Void = { _ in },
              forgetPairing: ((UUID) throws -> Void)? = nil) async throws {
        let plan = try connectionDraftSavePlan(
            draft: self, existing: existing.map(ConnectionRecord.init),
            fresh: ConnectionRecord(ServerConnection()), keys: keys.map(KeyRecord.init),
            given: secrets.given, udpStart: Int64(settings.udpRangeStart),
            udpEnd: Int64(settings.udpRangeEnd), newRef: keychain.generateRef())
        guard let connection = ServerConnection(plan.connection) else {
            throw ConnectionSaveRefusal.Invalid(why: "not a UUID: \(plan.connection.id)")
        }
        if plan.moved {
            try (forgetPairing ?? { try HostCare.shared.hostChanged($0) })(connection.id)
        }
        if let write = plan.write, let ref = connection.keychainRef {
            switch write {
            case .password:
                do {
                    try await keychain.savePassword(secrets.password, forRef: ref, requireBiometry: false)
                } catch {
                    throw ConnectionKeychainFailure(secret: .password, error: error)
                }
            case .key:
                do {
                    try await keychain.savePrivateKey(ConnectionSecrets.storable(secrets.privateKeyPEM),
                                                      forRef: ref, requireBiometry: false)
                } catch {
                    throw ConnectionKeychainFailure(secret: .key, error: error)
                }
            }
        }
        // Drop any cached (possibly stale) credential so the new one is used.
        await SSHService.shared.clearCachedSecret(for: connection.id)
        if plan.added {
            store.add(connection)
        } else {
            store.update(connection)
        }
        if let released = plan.released {
            await release(released)
        }
    }
}

/// A save the keychain refused: the secret it was writing (the connection
/// was not saved), and what the keychain threw. Saving again may work (the
/// keychain was locked or busy).
struct ConnectionKeychainFailure: Error, CustomStringConvertible {
    enum Secret: String { case password, key }
    let secret: Secret
    let error: Error

    var description: String { String(describing: error) }
}

extension ConnectionSaveRefusal: CustomStringConvertible {
    public var description: String {
        switch self {
        case .NewSecretNeeded:
            String(localized: "To change how this connection signs in, enter the new password or key.")
        case .Incomplete:
            String(localized: "Could not save")
        case .Invalid(let why):
            why
        }
    }
}

/// The password and the pasted private key the connection editor holds:
/// the host's, typed into its own fields and written to the keychain by it.
/// The shared model hears only ``given``.
struct ConnectionSecrets: Equatable {
    var password = ""
    var privateKeyPEM = ""

    /// What the model may know: whether there is a password (white space
    /// is one), and whether the paste holds a key (more than white space).
    var given: SecretsGiven {
        SecretsGiven(password: !password.isEmpty, key: Self.holdsKey(privateKeyPEM))
    }

    /// Whether `pem` holds a pasted key: more than white space.
    static func holdsKey(_ pem: String) -> Bool {
        !pem.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// A pasted private key in the one form the connect path signs with — an
    /// unencrypted openssh-key-v1 file — when it reads without a passphrase:
    /// Windows line endings, indented lines, line breaks lost on the way, a
    /// PEM key (B133). A key behind a passphrase, or text that isn't a key,
    /// is kept as pasted: the connect asks for the passphrase once, or says
    /// what is wrong with it. The Rust key reader's rule
    /// (`moshpit_ssh::keyfile::storable`, W15a), Android's too.
    static func storable(_ pasted: String) -> String {
        sshKeyStorable(pasted: pasted)
    }
}
