// MARK: - tmux clients, as the mosh sidecar lists them

/// One row of the mosh sidecar's `list-clients -F '\(format)'`.
///
/// The session is its id — `$<digits>`, never renamed — because a session
/// NAME may hold the separator (`a|b`) and goes stale on a rename. The
/// client name goes last: it is the rest of the line, whatever it holds.
struct TmuxClientRow: Equatable {
    /// tmux 3.6a prints `0|45533|$1|/dev/ttys026` for a terminal on session
    /// `$1` and `1|45613|$0|client-45613` for a `-CC` client without a tty
    /// (verified on a private server).
    static let format = "#{client_control_mode}|#{client_pid}|#{session_id}|#{client_name}"

    /// A `-CC` client: a sidecar like this app's own, never a screen anyone
    /// looks at.
    let isControl: Bool
    /// nil when the field is not a plain decimal number.
    let pid: Int?
    let sessionId: String
    let name: String

    init?(_ line: String) {
        let fields = line.tmuxFields(separator: "|", maxSplits: 3, omittingEmptySubsequences: false)
        guard fields.count == 4, fields[0] == "0" || fields[0] == "1", !fields[3].isEmpty else { return nil }
        isControl = fields[0] == "1"
        pid = Self.decimal(fields[1])
        sessionId = fields[2]
        name = fields[3]
    }

    /// Who this client is, when tmux says it plainly enough to act on: a
    /// pid, and a name that can stand unquoted in `switch-client -c`. tmux
    /// names a client after its tty (`/dev/pts/3`, `/dev/ttys026`), or
    /// `client-<pid>` without one.
    var identity: TmuxClientIdentity? {
        guard let pid, Self.isPlainName(name) else { return nil }
        return TmuxClientIdentity(pid: pid, name: name)
    }

    private static func decimal(_ field: String) -> Int? {
        guard (1...10).contains(field.utf8.count),
              field.utf8.allSatisfy({ (0x30...0x39).contains($0) }) else { return nil }
        return Int(field)
    }

    private static func isPlainName(_ name: String) -> Bool {
        !name.isEmpty && name.utf8.allSatisfy { byte in
            switch byte {
            case UInt8(ascii: "a")...UInt8(ascii: "z"), UInt8(ascii: "A")...UInt8(ascii: "Z"),
                 UInt8(ascii: "0")...UInt8(ascii: "9"),
                 UInt8(ascii: "/"), UInt8(ascii: "."), UInt8(ascii: "_"), UInt8(ascii: "-"):
                return true
            default:
                return false
            }
        }
    }
}

/// A tmux client by process id AND name. Either alone outlives the client
/// it named: the next terminal to open a tty is listed under that tty's
/// path, and pids are recycled.
struct TmuxClientIdentity: Hashable, Sendable {
    let pid: Int
    let name: String
}
