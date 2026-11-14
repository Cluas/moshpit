//! `list-clients` rows as the mosh sidecar reads them (`TmuxClientRow.swift`):
//! which client is a `-CC` sidecar, which pid and session it has, and whether
//! its name is plain enough to put, unquoted, into `switch-client -c`.

use crate::format::fields;

/// The `-F` format the rows are listed with. tmux 3.6a prints
/// `0|45533|$1|/dev/ttys026` for a terminal on `$1`, `1|45613|$0|client-45613`
/// for a `-CC` client without a tty. The session is its id (a name may hold
/// the separator and goes stale on a rename); the client name is the rest of
/// the line.
pub const CLIENT_ROW_FORMAT: &str =
    "#{client_control_mode}|#{client_pid}|#{session_id}|#{client_name}";

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRow {
    /// A `-CC` client: a sidecar, never a screen anyone looks at.
    pub is_control: bool,
    /// `None` when the field is not a plain decimal number.
    pub pid: Option<i64>,
    pub session_id: String,
    pub name: String,
}

/// A client by pid AND name: either alone outlives the client it named (the
/// next terminal on a tty is listed under that tty, and pids are recycled).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientIdentity {
    pub pid: i64,
    pub name: String,
}

impl ClientRow {
    /// `TmuxClientRow.init?(_:)`.
    pub fn parse(line: &str) -> Option<Self> {
        let parts = fields(line, b'|', 3, false);
        if parts.len() != 4 || !(parts[0] == "0" || parts[0] == "1") || parts[3].is_empty() {
            return None;
        }
        Some(Self {
            is_control: parts[0] == "1",
            pid: decimal(parts[1]),
            session_id: parts[2].to_owned(),
            name: parts[3].to_owned(),
        })
    }

    /// Who this client is, when tmux says it plainly enough to act on.
    pub fn identity(&self) -> Option<ClientIdentity> {
        let pid = self.pid?;
        is_plain_name(&self.name).then(|| ClientIdentity {
            pid,
            name: self.name.clone(),
        })
    }
}

fn decimal(field: &str) -> Option<i64> {
    let bytes = field.as_bytes();
    if !(1..=10).contains(&bytes.len()) || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    field.parse().ok()
}

/// A name that stands unquoted in a command: tmux names a client after its
/// tty (`/dev/pts/3`) or `client-<pid>`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // TmuxClientRowTests
    #[test]
    fn real_rows() {
        let terminal = ClientRow::parse("0|45533|$1|/dev/ttys026").unwrap();
        assert!(!terminal.is_control);
        assert_eq!(terminal.session_id, "$1");
        assert_eq!(
            terminal.identity(),
            Some(ClientIdentity {
                pid: 45533,
                name: "/dev/ttys026".into()
            })
        );
        let control = ClientRow::parse("1|45613|$0|client-45613").unwrap();
        assert!(control.is_control);
        assert_eq!(control.identity().unwrap().name, "client-45613");
        assert_eq!(
            ClientRow::parse("0|7|$3|/dev/pts/12")
                .unwrap()
                .identity()
                .unwrap()
                .name,
            "/dev/pts/12"
        );
    }

    #[test]
    fn unplain_rows_count_but_are_nobody() {
        for line in [
            "0|a|b|/dev/ttys026",
            "0||$1|/dev/ttys026",
            "0|-5|$1|/dev/ttys026",
            "0|１２|$1|/dev/ttys026",
            "0|12|$1|/dev/tty s",
            "0|12|$1|/dev/tty;x",
        ] {
            let row = ClientRow::parse(line).unwrap_or_else(|| panic!("{line:?}"));
            assert!(!row.is_control);
            assert_eq!(row.identity(), None, "{line:?}");
        }
    }

    #[test]
    fn not_rows_at_all() {
        for line in [
            "",
            "0",
            "0|1|$1",
            "2|1|$1|x",
            "0|1|$1|",
            "zsh: no such file",
        ] {
            assert_eq!(ClientRow::parse(line), None, "{line:?}");
        }
    }
}
