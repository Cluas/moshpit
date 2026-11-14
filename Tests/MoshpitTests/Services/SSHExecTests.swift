import Citadel
import Foundation
import Testing
@testable import Moshpit

/// B136: a command is answered only once it is known to have finished — an
/// exit status of 0. Citadel's `executeCommand` couldn't tell 0 from no
/// status at all: a channel closed without one (the server dropping it) or
/// with `exit-signal` (the command killed) handed back what had arrived as
/// the answer. These run `SSHSession.executeCommand` over the app's real
/// connect path against an SSH server in the test process.
@Suite("Commands end with an exit status, or fail", .serialized)
struct SSHExecTests {

    private static let execs: [String: LoopbackSSHServer.Exec] = [
        "echo hi": .exits(output: "hi\n", status: 0),
        "fails": .exits(output: "x", status: 3),
        "drops": .closes(output: "partial"),
        "killed": .killed(output: "partial", signal: "KILL"),
        "both": .interleaved(stdout: "a", stderr: "b", more: "c"),
        "gone": .dropsConnection(output: "partial"),
    ]

    private static func session(_ server: LoopbackSSHServer) async throws -> SSHSession {
        let connection = ServerConnection(name: "t", host: "127.0.0.1", port: server.port, username: "me",
                                          authMethod: .password, keychainRef: nil)
        return try await SSHService(keychain: .inMemory(), hostKeyValidator: .inMemory())
            .connect(connection, overrideSecret: "right", onUnknownHost: { _, _, _ in true })
    }

    /// What `command` returned, or the error's description.
    private static func outcome(_ session: SSHSession, _ command: String) async -> String {
        do {
            return "ok " + String(decoding: try await session.executeCommand(command), as: UTF8.self)
        } catch SSHError.underlying(let error) {
            return "error \(error)"
        } catch {
            return "error \(error)"
        }
    }

    @Test("Exit status 0: the output, stdout and stderr as they came")
    func exitZero() async throws {
        let server = try await LoopbackSSHServer.start(.init(execs: Self.execs))
        let session = try await Self.session(server)
        #expect(await Self.outcome(session, "echo hi") == "ok hi\n")
        #expect(await Self.outcome(session, "both") == "ok abc")
        await session.close()
        await server.stop()
    }

    /// B138: sshd runs an exec request as `<login shell> -c <command>`; a
    /// fish login shell refused the app's sh — `$?` above all, which every
    /// herdr command ends with. The request is one word for any shell to
    /// hand sh.
    @Test("Every command goes out as sh -c and one word any login shell reads alike")
    func requestIsPortable() async throws {
        let requests = LoopbackSSHServer.RequestLog()
        let script = #"herdr api snapshot; echo "__moshpit_rc=$?" 'a\'"#
        var options = LoopbackSSHServer.Options(execs: Self.execs)
        options.execs[script] = .exits(output: "ok", status: 0)
        options.requests = requests
        let server = try await LoopbackSSHServer.start(options)
        let session = try await Self.session(server)
        #expect(await Self.outcome(session, script) == "ok ok")
        let request = try #require(requests.all.last)
        #expect(request == #"sh -c 'herdr api snapshot; echo "__moshpit_rc=$?" '\''a'\\''\'''"#)
        #expect(PosixShell.unquoteAsFish(String(request.dropFirst("sh -c ".count))) == script)
        await session.close()
        await server.stop()
    }

    @Test("A non-zero status is a failure, as it was")
    func nonZero() async throws {
        let server = try await LoopbackSSHServer.start(.init(execs: Self.execs))
        let session = try await Self.session(server)
        #expect(await Self.outcome(session, "fails")
                == "error \(SSHExec.Failure.exited(status: 3, output: Data("x".utf8)))")
        await session.close()
        await server.stop()
    }

    /// B240: on a host without SFTP the folder sheet's edits go over their
    /// exec forms (`RemoteFileEdits`), which say "taken" and exit 1. Since
    /// B136 a non-zero status throws, the marker went unread and the person
    /// heard "couldn't reach the host". The reply is read whatever the
    /// status; one with no marker is the host not allowing it.
    @Test("An edit's exec form that says taken and exits 1 is a taken name, not an unreachable host")
    func editTakenWithStatusOne() async throws {
        let execs: [String: LoopbackSSHServer.Exec] = [
            RemoteFileEdits.makeDirectory("/work/made"): .exits(output: RemoteFileEdits.takenMarker + "\n", status: 1),
            RemoteFileEdits.rename("/work/a", to: "/work/b"): .exits(output: "mv: Permission denied\n", status: 1),
            RemoteFileEdits.makeDirectory("/work/new"): .exits(output: RemoteFileEdits.okMarker + "\n", status: 0),
            RemoteFileEdits.upload(to: "/work/up.txt").begin: .exits(output: RemoteFileEdits.takenMarker + "\n", status: 1),
        ]
        let server = try await LoopbackSSHServer.start(.init(execs: execs))
        let session = try await Self.session(server)
        func said(_ edit: () async throws -> Void) async -> String {
            do {
                try await edit()
                return "done"
            } catch {
                return RemotePreviewCache.message(for: error)
            }
        }
        #expect(await said { try await session.createRemoteDirectory("/work/made") }
                == String(localized: "Something with that name is already there."))
        #expect(await said { try await session.renameRemoteItem("/work/a", to: "/work/b") }
                == String(localized: "The host didn't allow it. Check that you can write to this folder."))
        #expect(await said { try await session.createRemoteDirectory("/work/new") } == "done")
        // A file picked in Files, onto a name that's there: the upload's exec
        // form says "taken" the same way.
        let local = FileManager.default.temporaryDirectory.appendingPathComponent("b240-\(UUID().uuidString).txt")
        try Data("x".utf8).write(to: local)
        defer { try? FileManager.default.removeItem(at: local) }
        #expect(await said { try await session.uploadRemoteFile(from: local, to: "/work/up.txt") }
                == String(localized: "Something with that name is already there."))
        await session.close()
        await server.stop()
    }

    /// B241: SFTP failing on a path is not "no SFTP". A server that says the
    /// path isn't there, or isn't this account's to read, is said so; the
    /// exec form (whose answer showed as a generic error) is only for the
    /// rest: no subsystem, a dropped channel, a call it doesn't support.
    @Test("SFTP's no such file and permission denied are said, not retried over exec")
    func sftpSaysWhy() {
        #expect(SSHSession.sftpRefusal(.noSuchFile) == .notFound)
        #expect(SSHSession.sftpRefusal(.permissionDenied) == .denied)
        for other: SFTPStatusCode? in [nil, .failure, .unsupportedOperation, .connectionLost, .noConnection, .badMessage] {
            #expect(SSHSession.sftpRefusal(other) == nil)
        }
        #expect(SSHSession.sftpStatusCode(SSHError.sessionClosed) == nil)
        #expect(RemotePreviewCache.message(for: SSHError.underlying(SSHSession.RemoteReadError.denied))
                == HostConnectionError.readDenied.errorDescription)
        #expect(RemotePreviewCache.message(for: SSHError.underlying(SSHSession.RemoteReadError.notFound))
                == String(localized: "This file is no longer on the host."))
    }

    @Test("A channel closed without an exit status, or killed by a signal: a failure, not what it printed")
    func cutShort() async throws {
        let server = try await LoopbackSSHServer.start(.init(execs: Self.execs))
        let session = try await Self.session(server)
        #expect(await Self.outcome(session, "drops") == "error \(SSHExec.Failure.endedWithoutStatus)")
        #expect(await Self.outcome(session, "killed") == "error \(SSHExec.Failure.killed(signal: "KILL"))")
        // The connection is fine: the next command answers.
        #expect(await Self.outcome(session, "echo hi") == "ok hi\n")
        await session.close()
        await server.stop()
    }

    @Test("A connection that goes mid-command fails the command")
    func connectionGone() async throws {
        let server = try await LoopbackSSHServer.start(.init(execs: Self.execs))
        let session = try await Self.session(server)
        let outcome = await Self.outcome(session, "gone")
        #expect(outcome.hasPrefix("error "), "\(outcome)")
        await session.close()
        await server.stop()
    }
}
