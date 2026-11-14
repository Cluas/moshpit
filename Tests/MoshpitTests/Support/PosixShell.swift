import Foundation
#if canImport(Darwin)
import Darwin
#endif

/// A command line run through a real `/bin/sh -c`, for tests that prove a
/// command means what it says to an actual shell rather than to a model of
/// one.
///
/// Where the platform lets a test spawn a process — macOS, the simulator.
/// A device's sandbox does not, so there ``available`` is false and the
/// tests that need it are skipped; the model checks beside them still run.
enum PosixShell {

    struct Result {
        /// The raw wait status (`WEXITSTATUS` is `status >> 8 & 0xff`).
        var status: Int32
        /// stdout and stderr, merged.
        var output: Data

        var exitCode: Int32 { (status >> 8) & 0xFF }
        var text: String { String(decoding: output, as: UTF8.self) }
    }

    /// Whether `/bin/sh` can be spawned here at all.
    static let available: Bool = run("printf ok")?.text == "ok"

    /// A model of the shell for where a real one can't run: reads one quoted
    /// word back the way a POSIX shell does — a single-quoted run literal, a
    /// backslash outside quotes taking the next character as it is. nil for
    /// anything else: a character outside the quotes (one the shell would
    /// parse) or an unterminated quote. By scalar, which for `'` and `\` is
    /// by byte: in UTF-8 they are never part of another character.
    static func unquote(_ word: String) -> String? {
        var value = String.UnicodeScalarView()
        var quoted = false
        var scalars = word.unicodeScalars.makeIterator()
        while let scalar = scalars.next() {
            if quoted {
                if scalar == "'" { quoted = false } else { value.append(scalar) }
            } else if scalar == "'" {
                quoted = true
            } else if scalar == "\\" {
                guard let next = scalars.next() else { return nil }
                value.append(next)
            } else {
                return nil
            }
        }
        return quoted ? nil : String(value)
    }

    /// The script of `sh -c <one word>` — what ``Shell/portable(_:)`` sends —
    /// as a POSIX shell hands it to sh; nil for anything else.
    static func shPayload(_ line: String) -> String? {
        guard line.hasPrefix("sh -c ") else { return nil }
        return unquote(String(line.dropFirst(6)))
    }

    /// A model of fish reading one quoted word (B138): inside single quotes
    /// `\'` is a quote and `\\` a backslash — every other character, a lone
    /// backslash included, is itself; outside them only `\'` and `\\` are
    /// taken (a word ``Shell/quote(_:)`` makes holds no other). nil for
    /// anything else: a character outside the quotes, a quote left open.
    /// fish 3.7.1 and 4.6.0 read words this way (checked in containers; see
    /// moshpit-ssh's harness/shells.py for the five real shells).
    static func unquoteAsFish(_ word: String) -> String? {
        var value = String.UnicodeScalarView()
        var quoted = false
        var scalars = Array(word.unicodeScalars)[...]
        while let scalar = scalars.popFirst() {
            if quoted {
                if scalar == "'" {
                    quoted = false
                } else if scalar == "\\", let next = scalars.first, next == "'" || next == "\\" {
                    value.append(scalars.removeFirst())
                } else {
                    value.append(scalar)
                }
            } else if scalar == "'" {
                quoted = true
            } else if scalar == "\\", let next = scalars.popFirst(), next == "'" || next == "\\" {
                value.append(next)
            } else {
                return nil
            }
        }
        return quoted ? nil : String(value)
    }

    /// A command line's words as they stand, split at the blanks outside
    /// quotes (a backslash outside them keeps the next character in its
    /// word), for reading each with ``unquote(_:)`` and
    /// ``unquoteAsFish(_:)``.
    static func rawWords(_ line: String) -> [String] {
        var words: [String] = []
        var word = String.UnicodeScalarView()
        var quoted = false
        var scalars = Array(line.unicodeScalars)[...]
        while let scalar = scalars.popFirst() {
            if quoted {
                word.append(scalar)
                if scalar == "'" { quoted = false }
            } else if scalar == "'" {
                word.append(scalar)
                quoted = true
            } else if scalar == "\\", let next = scalars.popFirst() {
                word.append(scalar)
                word.append(next)
            } else if scalar == " " {
                if !word.isEmpty { words.append(String(word)) }
                word = String.UnicodeScalarView()
            } else {
                word.append(scalar)
            }
        }
        if !word.isEmpty { words.append(String(word)) }
        return words
    }

    /// Run `command` as `<shell> -c command`. The environment is only what is
    /// passed (plus a plain PATH and the C locale unless overridden), so
    /// nothing from the test host leaks into what the shell does. nil when
    /// the shell couldn't be spawned.
    static func run(_ command: String, shell: String = "/bin/sh",
                    environment: [String: String] = [:]) -> Result? {
        var env = ["PATH": "/usr/bin:/bin", "LC_ALL": "C"]
        env.merge(environment) { _, new in new }
        return spawn(shell, ["-c", command], environment: env)
    }

    /// The shells on this machine a quoting test should satisfy besides
    /// `/bin/sh`, by path.
    static var otherShells: [String] {
        ["/bin/dash", "/bin/bash", "/bin/zsh", "/bin/ksh", "/opt/homebrew/bin/bash",
         "/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish"]
            .filter { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// A scratch directory of the test's own, removed by the caller.
    static func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("posix-shell-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// An executable script at `directory/name`.
    static func writeExecutable(_ script: String, named name: String, in directory: URL) throws {
        let url = directory.appendingPathComponent(name)
        try Data(script.utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
    }

    private static func spawn(_ path: String, _ arguments: [String],
                              environment: [String: String]) -> Result? {
        var fds: [Int32] = [-1, -1]
        guard pipe(&fds) == 0 else { return nil }
        var actions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&actions)
        defer { posix_spawn_file_actions_destroy(&actions) }
        posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_adddup2(&actions, fds[1], 1)
        posix_spawn_file_actions_adddup2(&actions, fds[1], 2)
        posix_spawn_file_actions_addclose(&actions, fds[0])
        posix_spawn_file_actions_addclose(&actions, fds[1])

        // C strings, as execve takes them: an argument holding a NUL ends
        // there, exactly as it would for a real command line.
        var argv: [UnsafeMutablePointer<CChar>?] = ([path] + arguments).map { strdup($0) } + [nil]
        var envp: [UnsafeMutablePointer<CChar>?] = environment.map { strdup("\($0.key)=\($0.value)") } + [nil]
        defer {
            argv.forEach { free($0) }
            envp.forEach { free($0) }
        }

        var pid: pid_t = 0
        let spawned = posix_spawn(&pid, path, &actions, nil, &argv, &envp)
        close(fds[1])
        guard spawned == 0 else {
            close(fds[0])
            return nil
        }
        var output = Data()
        var buffer = [UInt8](repeating: 0, count: 65_536)
        while true {
            let count = read(fds[0], &buffer, buffer.count)
            if count > 0 {
                output.append(buffer, count: count)
            } else if count < 0, errno == EINTR {
                continue
            } else {
                break
            }
        }
        close(fds[0])
        var status: Int32 = 0
        while waitpid(pid, &status, 0) == -1, errno == EINTR {}
        return Result(status: status, output: output)
    }
}
