import Foundation

/// What the features beside a terminal need from the host — the remote
/// preview and its folder sheet, uploads, the path probe, host maintenance:
/// commands on exec channels, and files over SFTP with exec forms for a host
/// that has SFTP turned off.
///
/// The Rust session's own connection stands behind it, lent
/// (``RustSideChannel`` and the ``BorrowedHost`` it hands each call, W14b):
/// the session's SSH, mosh + tmux's control connection, or the one side
/// connection plain mosh keeps. No second sign-in, no second Face ID; when
/// the session is reconnecting or gone, a call says so
/// (``HostConnectionError``) instead of dialling.
///
/// The rules it keeps: a command is a POSIX sh script run as
/// `sh -c` whatever the login shell (B138), its words quoted by the one
/// scalar-safe quoter (`Shell.quote`, B113), and it succeeds only with exit
/// status 0 (B136).
protocol HostConnection: FileUploader {
    /// Runs `command` on an exec channel of its own; its output, once it
    /// exited with status 0. Anything else throws.
    func executeCommand(_ command: String) async throws -> Data

    /// What `path` is: file or folder, size, modification time (none from a
    /// host without SFTP).
    func remoteStat(_ path: String) async throws -> RemoteFileEntry

    /// `path` opened for reads at any offset — what streaming playback pulls
    /// ranges through. SFTP only: a caller falls back to a download.
    func openRemoteFile(_ path: String) async throws -> RemoteFileHandle

    /// The login directory, where SFTP and exec channels start.
    func remoteHomeDirectory() async throws -> String

    /// The entries of the folder at `path`, unfiltered and unsorted.
    func listRemoteDirectory(_ path: String) async throws -> [RemoteFileEntry]

    /// Copies the file at `path` into `destination` (created or replaced).
    func downloadRemoteFile(_ path: String, to destination: URL,
                            progress: (@Sendable (Double) -> Void)?) async throws

    func createRemoteDirectory(_ path: String) async throws

    /// Never over something already there.
    func renameRemoteItem(_ path: String, to newPath: String) async throws

    /// A file, or a folder with everything in it.
    func deleteRemoteItem(_ entry: RemoteFileEntry) async throws

    /// Streams `local` to `remotePath`, which must not exist yet.
    func uploadRemoteFile(from local: URL, to remotePath: String,
                          progress: (@Sendable (Double) -> Void)?) async throws
}

/// Why the session's lent connection couldn't do what was asked, worded for
/// the person where it reaches one (the preview card, the folder sheet, the
/// image overlay).
enum HostConnectionError: Error, Equatable, LocalizedError, CustomStringConvertible {
    /// The session is reconnecting, and wasn't back within the wait.
    case reconnecting
    /// The session isn't connected and isn't on its way back (failed, ended,
    /// closed): the person has to reconnect it.
    case notConnected
    /// The connection didn't answer, or dropped mid-way, while the session
    /// still counted it as up.
    case notAnswering
    case notFound
    /// The host won't let this account read it.
    case readDenied
    /// The host won't let this account change it.
    case writeDenied
    /// Something with that name is already there.
    case nameTaken
    /// The delete would take the home folder, or a folder it is in (B816).
    case homeFolder
    /// The host has SFTP turned off and the file is past what the exec form
    /// moves (`RemoteFileCommands.execDownloadLimit`).
    case tooLargeWithoutSFTP
    /// A command exited with this status (B136).
    case exited(status: Int)
    /// A command's channel closed without an exit status: what came is cut
    /// short (B136).
    case endedWithoutStatus
    /// Anything else; the text is for the log.
    case failed(String)

    var errorDescription: String? {
        switch self {
        case .reconnecting:
            return String(localized: "Moshpit is reconnecting to the host. Try again in a moment.")
        case .notConnected:
            return String(localized: "This session isn't connected to the host. Reconnect, then try again.")
        case .notAnswering:
            return String(localized: "The host isn't answering on this session's connection. Try again in a moment.")
        case .notFound:
            return String(localized: "This file is no longer on the host.")
        case .readDenied:
            return String(localized: "The host doesn't let this account read it.")
        case .writeDenied:
            return String(localized: "The host didn't allow it. Check that you can write to this folder.")
        case .nameTaken:
            return String(localized: "Something with that name is already there.")
        case .homeFolder:
            return String(localized: "Moshpit won't delete your home folder, or a folder it's in.")
        case .tooLargeWithoutSFTP:
            return String(localized: "This host has SFTP turned off, and the file is too large to send without it.")
        case .exited, .endedWithoutStatus, .failed:
            return String(localized: "Couldn't reach the host. Check the connection and try again.")
        }
    }

    /// For the log.
    var description: String {
        switch self {
        case .reconnecting: return "the session is reconnecting"
        case .notConnected: return "the session is not connected"
        case .notAnswering: return "the session's connection isn't answering"
        case .notFound: return "not found"
        case .readDenied: return "read permission denied"
        case .writeDenied: return "write permission denied"
        case .nameTaken: return "name taken"
        case .homeFolder: return "the home folder, or a folder holding it"
        case .tooLargeWithoutSFTP: return "too large without SFTP"
        case .exited(let status): return "command exited with status \(status)"
        case .endedWithoutStatus: return "command's channel closed without an exit status"
        case .failed(let log): return log
        }
    }

    /// Whether the connection, not the request, is what failed: worth one
    /// more try on the session's next connection.
    var isConnectionLost: Bool {
        switch self {
        case .reconnecting, .notConnected, .notAnswering: return true
        default: return false
        }
    }
}
