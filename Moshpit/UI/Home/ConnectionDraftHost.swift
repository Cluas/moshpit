import Foundation

// The connection editor's draft is the shared Rust model's
// (`crates/app/src/connection_form.rs`, through `crates/ffi/src/app.rs`),
// the one Android runs too: ``ConnectionDraft`` is its record, and its
// rules — the ports, Save, the credential a save points the connection at
// (B294, B298) — are Rust functions. The iOS form, NSH's
// (``ConnectionFormNSHPage``), drives it.
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
    /// `secretMissing`: its own password or pasted key isn't on this device
    /// (B782, ``ConnectionStore/secretMissing``) — the draft keeps nothing,
    /// and its field says whose secret is gone and takes the keyboard.
    init(_ c: ServerConnection, secretMissing: Bool = false) {
        // A Swift connection's id is a UUID, which is all the model can
        // refuse.
        self = (try? connectionDraftOf(connection: ConnectionRecord(c)))
            ?? connectionDraftNew(moshByDefault: c.connectionProtocol == .mosh)
        if secretMissing {
            self = connectionDraftSecretMissing(draft: self, name: c.displayName)
        }
    }

    /// The draft of the New Jump Host form, opened on top of another form.
    init(newJumpHostWith settings: AppSettings) {
        self = connectionDraftNewJumpHost(moshByDefault: settings.moshByDefault)
    }

    /// The draft of saved connection `c` opened on top of another form
    /// (Edit under Connect via).
    init(jumpHost c: ServerConnection) {
        self = (try? connectionDraftOfJumpHost(connection: ConnectionRecord(c)))
            ?? connectionDraftNewJumpHost(moshByDefault: c.connectionProtocol == .mosh)
    }

    /// The managed key picked from SSH Keys; nil: paste a PEM.
    var selectedKeyId: UUID? {
        get { selectedKey.flatMap(UUID.init(uuidString:)) }
        set { selectedKey = newValue?.uuidString }
    }

    /// Save's rule: a name, a host, ports that are ports, and something to
    /// sign in with, `keys` being SSH Keys' now.
    func canSave(keys: [SSHKeyRecord], secrets: ConnectionSecrets) -> Bool {
        connectionDraftCanSave(draft: self, given: secrets.given, keys: keys.map(KeyRecord.init))
    }

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
    /// A connection now pointing at another host or port (B328) has its
    /// push pairing forgotten (`forgetPairing`, ``HostCare`` by default): it
    /// belongs to the box it pointed at before, which would go on pushing
    /// under it. A phone that can't let go saves nothing
    /// (``PushPairingForgetFailure``). It comes after the keychain write
    /// (B534): a write that failed had already lost the pairing. A move's
    /// new secret goes to a fresh item (the plan's), so a failed forget is
    /// undone by deleting that item, and the record kept still signs in
    /// with its own.
    @MainActor
    @discardableResult
    func save(existing: ServerConnection?, store: ConnectionStore, keychain: KeychainService,
              settings: AppSettings, keys: [SSHKeyRecord], secrets: ConnectionSecrets,
              release: (String) async -> Void = { _ in },
              forgetPairing: ((UUID) throws -> Void)? = nil) async throws -> UUID {
        // The record as saved now, not as the form opened it: one deleted
        // meanwhile is refused (B536), where the update used to change
        // nothing and the form closed as saved.
        let existing = existing.flatMap { store.saved($0.id) }
        let plan = try connectionDraftSavePlan(
            draft: self, existing: existing.map(ConnectionRecord.init),
            fresh: ConnectionRecord(ServerConnection()), keys: keys.map(KeyRecord.init),
            given: secrets.given, udpStart: Int64(settings.udpRangeStart),
            udpEnd: Int64(settings.udpRangeEnd), newRef: storeNewKeychainRef())
        guard let connection = ServerConnection(plan.connection) else {
            throw ConnectionSaveRefusal.Invalid(why: "not a UUID: \(plan.connection.id)")
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
        if plan.moved {
            do {
                try (forgetPairing ?? { try HostCare.shared.hostChanged($0) })(connection.id)
            } catch {
                // The fresh item nothing names yet goes with the save.
                if plan.write != nil, let ref = connection.keychainRef, ref != existing?.keychainRef {
                    try? await keychain.delete(forRef: ref)
                }
                throw error
            }
            // The store's update forgets what was learned about the old
            // box (B431).
        }
        // Drop any cached (possibly stale) credential so the new one is used.
        await SSHService.shared.clearCachedSecret(for: connection.id)
        if plan.added {
            store.add(connection)
        } else {
            store.update(connection)
        }
        // Save asked for a secret of its own or a managed key (B782): it
        // has one now.
        if missing != nil {
            store.secretFound(connection.id)
        }
        if let released = plan.released {
            await release(released)
        }
        return connection.id
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
        case .Gone:
            String(localized: "This connection was deleted while you were editing it.")
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
    /// (`moshpit_ssh::keyfile::storable`), Android's too.
    static func storable(_ pasted: String) -> String {
        sshKeyStorable(pasted: pasted)
    }
}
