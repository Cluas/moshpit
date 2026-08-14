import Foundation

/// Result of starting `mosh-server` over SSH.
struct MoshCredentials: Equatable {
    let host: String
    /// `UInt16`, not `Int`, so a value that cannot be a port cannot reach the
    /// socket layer at all. It used to be `Int` straight out of `Int(parts[2])`
    /// with no range check, and `MoshTransport.init` narrowed it with
    /// `UInt16(udpPort)` — which **traps** on anything negative or above 65535.
    /// A remote printing `MOSH CONNECT 70000 …` crashed the app at connect time.
    ///
    /// Zero is a different case and worth not confusing with that one:
    /// `NWEndpoint.Port(rawValue: 0)` is `Optional(0)`, not nil, so port 0 never
    /// crashed — it just cannot connect. It is rejected at parse time anyway, so
    /// the failure reads as "the host reported an unusable port" instead of an
    /// opaque socket error minutes later.
    let udpPort: UInt16
    /// 16-byte AES-128 session key.
    let key: Data
}

/// Starts `mosh-server` on the remote host over an existing SSH session and
/// parses the `MOSH CONNECT <port> <key>` handshake line it prints before
/// daemonizing (mosh-server.cc).
enum MoshBootstrap {

    enum BootstrapError: Error, CustomStringConvertible {
        case noConnectLine(String)
        /// The connect line's port field parsed as a number but is not a port.
        /// Deliberately separate from ``noConnectLine``: "the host said 0" and
        /// "the host never said anything" are different diagnoses, and this one
        /// used to be a crash rather than any diagnosis at all.
        case unusablePort(Int)
        case badKey

        var description: String {
            switch self {
            case .noConnectLine(let out):
                return "mosh-server did not print a MOSH CONNECT line. Output: \(Self.sanitizedExcerpt(of: out))"
            case .unusablePort(let port):
                // An Int needs no sanitizing — it cannot carry the control
                // sequences `sanitizedExcerpt` exists to defang.
                return "mosh-server reported UDP port \(port), which is not a usable port"
            case .badKey:
                return "mosh-server returned a malformed session key"
            }
        }

        /// Produces a short, safe-to-display excerpt of raw remote command output
        /// for embedding in an error message.
        ///
        /// The associated output here is arbitrary text emitted by a remote host we
        /// do not control. Splatting it verbatim into an error string is a hygiene
        /// risk: that string later flows into user-facing alerts, `os_log`, and
        /// potentially crash-reporting/analytics. A misbehaving or hostile remote
        /// could embed ANSI/terminal escape sequences (cursor moves, screen clears,
        /// title-setting, hyperlink OSC codes) that would then execute against
        /// whatever terminal or log viewer renders the message, or could pad the
        /// output with kilobytes of junk to bloat logs. Today's failure path does
        /// not leak the session key (the key only appears on the success path, which
        /// never reaches this branch), but treating remote-controlled bytes as inert
        /// display text is the wrong default regardless.
        ///
        /// So we: (1) cap the excerpt at a length that is still useful for diagnosing
        /// a genuine connection failure (a login banner, an `mosh-server: not found`
        /// message, a permission error), and (2) replace every non-printable control
        /// character with a visible `\u{XXXX}` escape so nothing can act on the
        /// rendering context.
        static func sanitizedExcerpt(of output: String) -> String {
            let maxLength = 100
            var truncated = String(output.prefix(maxLength))
            let wasTruncated = output.count > maxLength

            var escaped = ""
            escaped.reserveCapacity(truncated.count)
            for scalar in truncated.unicodeScalars {
                // Keep ordinary printable characters (including regular spaces) as-is;
                // escape C0/C1 controls and other non-printables so they render inert.
                if scalar == " " || (!scalar.properties.isDefaultIgnorableCodePoint
                    && !CharacterSet.controlCharacters.contains(scalar)
                    && !CharacterSet.illegalCharacters.contains(scalar)) {
                    escaped.unicodeScalars.append(scalar)
                } else {
                    escaped += String(format: "\\u{%04X}", scalar.value)
                }
            }
            truncated = escaped

            if truncated.isEmpty {
                return "<empty>"
            }
            return wasTruncated ? "\(truncated)…" : truncated
        }
    }

    /// - Parameters:
    ///   - session: an authenticated SSH session to the target host.
    ///   - host: the host to send mosh UDP datagrams to (the SSH host).
    ///   - serverBinary: path/name of mosh-server (default `mosh-server`).
    ///   - locale: forwarded as `LANG` so the remote PTY is UTF-8.
    ///   - portRangeStart/portRangeEnd: the connection's configured UDP port
    ///     range, forwarded as `-p start:end` so a user whose firewall only
    ///     opens a specific range actually gets a port inside it.
    static func start(over session: SSHSession,
                      host: String,
                      serverBinary: String = "mosh-server",
                      locale: String = "en_US.UTF-8",
                      portRangeStart: Int? = nil,
                      portRangeEnd: Int? = nil) async throws -> MoshCredentials {
        let command = command(serverBinary: serverBinary, locale: locale,
                              portRangeStart: portRangeStart, portRangeEnd: portRangeEnd)
        let output = String(decoding: try await session.executeCommand(command), as: UTF8.self)
        return try parse(output: output, host: host)
    }

    /// Builds the remote command line. Exposed for tests.
    ///
    /// `new -s` binds the UDP socket to the server-side IP of the SSH
    /// connection (so datagrams traverse the same path). `-c 256` =
    /// 256-color. The server prints MOSH CONNECT then forks into the bg.
    /// A configured port range rides along as `-p` (mosh-server accepts
    /// `PORT` or `PORT:PORT2`). Degenerate values degrade instead of
    /// producing a command that errors out remotely: an unusable start
    /// (non-positive / > 65535) drops `-p` entirely (mosh-server's default
    /// 60000–61000), while an unusable *end* keeps the valid start as a
    /// single-port pin (start == end is exactly that intent in the UI).
    /// A bare binary name (no `/`) additionally gets the probe's PATH
    /// extension prepended: the exec channel is a non-interactive shell, so
    /// Homebrew-installed servers (`/opt/homebrew/bin` etc.) are otherwise
    /// invisible here even though `probeCapabilities` — which searches those
    /// dirs — just reported the host as mosh-capable. An explicit path means
    /// the user vouches for it; leave it untouched.
    static func command(serverBinary: String,
                        locale: String,
                        portRangeStart: Int?,
                        portRangeEnd: Int?) -> String {
        var cmd = "\(serverBinary) new -s -c 256"
        if !serverBinary.contains("/") {
            cmd = "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" " + cmd
        }
        if let start = portRangeStart, (1...65535).contains(start) {
            if let end = portRangeEnd, (start...65535).contains(end), end > start {
                cmd += " -p \(start):\(end)"
            } else {
                cmd += " -p \(start)"
            }
        }
        cmd += " -l LANG=\(locale)"
        return cmd
    }

    /// Parses the `MOSH CONNECT <port> <base64key>` line. Exposed for tests.
    static func parse(output: String, host: String) throws -> MoshCredentials {
        // `\r\n` is a single Swift grapheme, so match on `isNewline` (handles
        // CR, LF, and CRLF) rather than comparing to "\n"/"\r" characters.
        guard let line = output
            .split(whereSeparator: \.isNewline)
            .first(where: { $0.hasPrefix("MOSH CONNECT ") })
        else { throw BootstrapError.noConnectLine(output) }

        let parts = line.split(separator: " ")
        guard parts.count >= 4, let port = Int(parts[2]) else {
            throw BootstrapError.noConnectLine(output)
        }
        // `Int(parts[2])` only rejects non-numbers, and "numeric but not a port"
        // is reachable without a hostile server: a login banner or stderr
        // interleaving with mosh-server's own line moves which token lands in
        // `parts[2]`, and a PID, timestamp or version number parses just fine.
        //
        // Out of range is the crash: `MoshTransport.init` narrowed this with
        // `UInt16(udpPort)`, which traps. Catching it here turns a crash at
        // connect time into the recoverable error the caller already degrades on
        // (fall back to plain SSH). Zero is folded in for tidiness rather than
        // safety — it cannot connect, so failing now beats failing opaquely.
        guard let udpPort = UInt16(exactly: port), udpPort != 0 else {
            throw BootstrapError.unusablePort(port)
        }
        let key = try decodeKey(String(parts[3]))
        return MoshCredentials(host: host, udpPort: udpPort, key: key)
    }

    /// mosh prints the key as 22 unpadded base64 chars = 16 bytes.
    private static func decodeKey(_ token: String) throws -> Data {
        var b64 = token
        while b64.count % 4 != 0 { b64.append("=") }
        guard let key = Data(base64Encoded: b64), key.count == OCB3.keyLength else {
            throw BootstrapError.badKey
        }
        return key
    }
}
