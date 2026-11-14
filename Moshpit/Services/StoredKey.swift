import Foundation

/// A private key as the keychain holds it, against the one form every
/// connect path signs with: an unencrypted openssh-key-v1 file.
///
/// Import has stored keys in that form since B29. Keys imported before were
/// stored as pasted, and so is a key pasted into a connection's own form: a
/// PEM key (PKCS #1, SEC1, PKCS #8), one behind a passphrase, or one with
/// Windows line endings or indented lines imported fine and then failed every
/// connection. ``assess(_:passphrase:)`` tells those apart from a key that
/// works, and turns them into the form that does — with the reader import
/// uses (`moshpit_ssh::keyfile`), so a key is read the same way whether it
/// is imported today or was years ago.
enum StoredKey {
    enum Assessment: Equatable {
        /// Every connect path reads it as stored (1.0.8's too).
        case usable
        /// The same key as the file every connect path reads. Already
        /// checked: it reads back to the same public key.
        case converted(String)
        /// It is behind a passphrase, which the user has to give once.
        case needsPassphrase
    }

    /// What to do with `stored` — decrypted with `passphrase` when it needs
    /// one. Throws an ``SSHKeyImportError`` for a key no form of which can
    /// sign in (a security key, DSA, a damaged file), saying why, and
    /// `.wrongPassphrase` for a passphrase that doesn't unlock it.
    static func assess(_ stored: String, passphrase: String? = nil) throws -> Assessment {
        do {
            switch try sshKeyAssess(stored: stored, passphrase: passphrase) {
            case .usable: return .usable
            case .converted(let openssh): return .converted(openssh)
            case .needsPassphrase: return .needsPassphrase
            }
        } catch let failure as KeyImportFailure {
            throw SSHKeyImportError(failure)
        }
    }

    /// The kind of key a saved key is, by name ("DSA"), when it is one no
    /// connect path can sign in with — judged from its public key line
    /// alone, which is all the key picker can see without unlocking the key.
    /// Nil when it signs in, and when the line doesn't say (an old import
    /// saved without its public half): such a key says why at connect.
    static func refusedType(ofPublicKeyLine line: String) -> String? {
        sshKeyRefusedType(publicKeyLine: line)
    }
}
