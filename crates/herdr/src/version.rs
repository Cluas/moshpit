//! What `herdr status` says about the two ends, and which side of a version
//! skew is older: `HerdrVersionReport` in
//! `Moshpit/Services/Herdr/HerdrSnapshot.swift`.

use std::cmp::Ordering;

use crate::text;

/// The `herdr` the app's commands resolve to (the client) and the server
/// actually running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReport {
    pub client: String,
    pub server: String,
}

/// The skew in one banner-sized line; which side is older decides the
/// remedy it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionNotice {
    /// A stale copy on the host's PATH; it needs updating there, and a
    /// restart with it would downgrade the server.
    ClientOlder { client: String, server: String },
    /// A stale server; the restart button fixes it.
    ServerOlder { client: String, server: String },
}

impl VersionNotice {
    /// The app's English text (its string catalog's source).
    pub fn english(&self) -> String {
        match self {
            VersionNotice::ClientOlder { client, server } => {
                format!(
                    "herdr on the host's PATH is {client}, its server {server} — update that copy"
                )
            }
            VersionNotice::ServerOlder { client, server } => {
                format!("The host's herdr server is {server}, older than its herdr {client}")
            }
        }
    }
}

impl VersionReport {
    /// Parse `herdr status`, the shape 0.8.0 and 0.9.5 both print:
    ///
    /// ```text
    /// client:
    ///   version: 0.8.0
    ///   …
    /// server:
    ///   status: running
    ///   version: 0.9.5
    /// ```
    ///
    /// `None` unless both versions are there: a stopped server has none, and
    /// a herdr without `status` prints something else entirely.
    pub fn parse(output: &str) -> Option<VersionReport> {
        let mut section = "";
        let mut client: Option<&str> = None;
        let mut server: Option<&str> = None;
        for raw in text::components_by_newlines(output) {
            let line = text::trim_whitespaces(raw);
            if line.is_empty() {
                continue;
            }
            // Section headers sit at column 0; their fields are indented.
            if raw
                .chars()
                .next()
                .is_some_and(|first| !first.is_whitespace())
            {
                section = line.strip_suffix(':').unwrap_or("");
                continue;
            }
            let Some(value) = line.strip_prefix("version:") else {
                continue;
            };
            let value = text::trim_whitespaces(value);
            let slot = match section {
                "client" => &mut client,
                "server" => &mut server,
                _ => continue,
            };
            if slot.is_none() && !value.is_empty() {
                *slot = Some(value);
            }
        }
        Some(VersionReport {
            client: client?.to_owned(),
            server: server?.to_owned(),
        })
    }

    pub fn mismatched(&self) -> bool {
        compare(&self.client, &self.server) != Ordering::Equal
    }

    /// Restarting the server with the client would downgrade it.
    pub fn client_is_older(&self) -> bool {
        compare(&self.client, &self.server) == Ordering::Less
    }

    pub fn notice(&self) -> VersionNotice {
        let (client, server) = (self.client.clone(), self.server.clone());
        if self.client_is_older() {
            VersionNotice::ClientOlder { client, server }
        } else {
            VersionNotice::ServerOlder { client, server }
        }
    }
}

/// Dotted-number order: "0.10.0" > "0.9.5", a missing component is 0, and a
/// suffix ("0.9.5-preview") compares by its leading digits alone.
///
/// Fix (version-digits): the Swift version took a component's leading
/// Characters with a numeric value and parsed them as an `Int`, so a digit
/// run followed by another numeral ("1٣", "5½") or a combining mark, or one
/// past `Int64`, read as 0. Here a component is its leading ASCII digits,
/// compared as a number of any length.
pub fn compare(a: &str, b: &str) -> Ordering {
    fn parts(version: &str) -> Vec<&str> {
        version
            .split('.')
            .filter(|part| !part.is_empty())
            .map(|part| text::ascii_digits(part).trim_start_matches('0'))
            .collect()
    }
    let (left, right) = (parts(a), parts(b));
    for index in 0..left.len().max(right.len()) {
        let l = left.get(index).copied().unwrap_or("");
        let r = right.get(index).copied().unwrap_or("");
        let order = l.len().cmp(&r.len()).then_with(|| l.cmp(r));
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}
