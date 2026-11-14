//! What a host can do for the installer, from one round trip
//! (`HostFacts`, the output of [`crate::commands::PREFLIGHT`]).

use std::collections::{BTreeMap, BTreeSet};

use moshpit_seal::foundation::text;

/// The preflight's answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    pub home: String,
    pub shell: String,
    pub uname: String,
    /// Tool name → present.
    pub tools: BTreeMap<String, bool>,
    /// Agent id → found on the host (its config directory, or its CLI on
    /// PATH), from the preflight's `agent.<id>` lines (B374).
    pub agents: BTreeMap<String, bool>,
    /// Which agents' hooks have run here ([`crate::commands::AGENTS_PROBE`],
    /// the marks the stamp script leaves), once asked; `None` until then.
    pub marks: Option<AgentFacts>,
}

/// The coding agents on a host, as [`crate::commands::AGENTS_PROBE`] finds
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentFacts {
    /// The agents whose hooks have run here (their stamp left its mark).
    pub seen: BTreeSet<String>,
}

impl AgentFacts {
    /// Read the probe's `key=value` lines; anything else is ignored.
    pub fn parse(output: &str) -> Self {
        let mut facts = Self::default();
        for line in output.lines() {
            let Some((key, value)) = line.trim_end_matches('\r').split_once('=') else {
                continue;
            };
            if key.trim() == "seen" {
                facts.seen = value.split_whitespace().map(str::to_owned).collect();
            }
        }
        facts
    }

    /// Whether `agent`'s hooks have run here.
    pub fn seen(&self, agent: &str) -> bool {
        self.seen.contains(agent)
    }
}

impl HostFacts {
    /// Whether the host has agent `id` (its config directory or its CLI).
    pub fn has_agent(&self, id: &str) -> bool {
        self.agents.get(id) == Some(&true)
    }

    pub fn has_openssl(&self) -> bool {
        self.tools.get("openssl") == Some(&true)
    }

    pub fn has_curl(&self) -> bool {
        self.tools.get("curl") == Some(&true)
    }

    pub fn has_base64(&self) -> bool {
        self.tools.get("base64") == Some(&true)
    }

    /// Optional: without it the stamp script still reports state, no title.
    pub fn has_jq(&self) -> bool {
        self.tools.get("jq") == Some(&true)
    }

    /// Everything the push path needs; `base64` because it is how the
    /// installer lands files at all.
    pub fn can_push(&self) -> bool {
        self.has_openssl() && self.has_curl() && self.has_base64()
    }

    /// What is missing for pushes to work, in this order.
    pub fn missing_for_push(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if !self.has_openssl() {
            missing.push("openssl".to_owned());
        }
        if !self.has_curl() {
            missing.push("curl".to_owned());
        }
        if !self.has_base64() {
            missing.push("base64".to_owned());
        }
        missing
    }

    /// Read the preflight's `key=value` lines (`HostFacts.parse`): lines and
    /// the `=` found by Swift `Character`, the value trimmed of
    /// `CharacterSet.whitespaces`; `home`, `shell` and `uname` are facts,
    /// `agent.<id>` an agent (B374), any other key a tool, present when its
    /// value is exactly `yes`.
    ///
    /// Fix (facts-crlf): Swift splits on the `Character` `"\n"`, which a
    /// CR LF pair is not (`InstallManifest.swift:184`). Output with CR LF
    /// line ends reads as one line: `home` swallows everything after the
    /// first `=`, no tool is found, and a host with openssl, curl and base64
    /// is refused as missing all three. Here a CR LF ends a line too.
    pub fn parse(output: &str) -> Self {
        let mut facts = Self::default();
        for line in text::split_characters(output, |c| c == "\n" || c == "\r\n") {
            let Some((key, value)) = split_once_character(line, "=") else {
                continue;
            };
            let value = text::trim_inline_spaces(value).to_owned();
            match key {
                "home" => facts.home = value,
                "shell" => facts.shell = value,
                "uname" => facts.uname = value,
                _ if key.starts_with("agent.") => {
                    facts
                        .agents
                        .insert(key["agent.".len()..].to_owned(), value == "yes");
                }
                _ => {
                    facts.tools.insert(key.to_owned(), value == "yes");
                }
            }
        }
        facts
    }
}

/// `text` split at the first `Character` equal to `separator`.
fn split_once_character<'a>(text: &'a str, separator: &str) -> Option<(&'a str, &'a str)> {
    let mut at = 0;
    for character in text::characters(text) {
        if character == separator {
            return Some((&text[..at], &text[at + character.len()..]));
        }
        at += character.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TOOLS: &str = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes";

    /// `HostInstallerTests.preflight`.
    #[test]
    fn preflight_parses_every_fact() {
        let facts = HostFacts::parse(ALL_TOOLS);
        assert_eq!(facts.home, "/Users/dev");
        assert_eq!(facts.uname, "Darwin");
        assert_eq!(facts.shell, "/bin/zsh");
        assert!(facts.can_push());
        assert!(facts.has_jq());
        assert!(facts.missing_for_push().is_empty());
    }

    /// `HostInstallerTests.missingTools`.
    #[test]
    fn a_host_missing_openssl_says_so() {
        let facts = HostFacts::parse("home=/home/x\ncurl=yes\nbase64=yes\nopenssl=no");
        assert!(!facts.can_push());
        assert_eq!(facts.missing_for_push(), ["openssl"]);
        assert_eq!(
            HostFacts::parse("").missing_for_push(),
            ["openssl", "curl", "base64"]
        );
    }

    #[test]
    fn values_are_trimmed_and_only_yes_is_yes() {
        let facts = HostFacts::parse(
            "home= /h \u{A0}\n\njq= yes\ncurl=YES\nnot a fact\nbase64=yes=no\n=yes",
        );
        assert_eq!(facts.home, "/h");
        assert!(facts.has_jq());
        assert!(!facts.has_curl());
        assert!(!facts.has_base64());
        assert_eq!(facts.tools.get(""), Some(&true));
    }

    /// B374: the preflight's `agent.<id>` lines are agents, not tools (the
    /// setup screen shows every tool as a chip).
    #[test]
    fn agent_lines_are_agents_not_tools() {
        let facts = HostFacts::parse(&format!(
            "{ALL_TOOLS}\nagent.claude=no\nagent.codex=yes\nagent.factory= yes"
        ));
        assert!(facts.has_agent("codex"));
        assert!(facts.has_agent("factory"));
        assert!(!facts.has_agent("claude"));
        assert!(!facts.has_agent("gemini"));
        assert!(facts.tools.keys().all(|k| !k.starts_with("agent.")));
        assert_eq!(facts.tools, HostFacts::parse(ALL_TOOLS).tools);
    }

    /// Fix (facts-crlf).
    #[test]
    fn cr_lf_line_ends_read_like_lf() {
        let facts = HostFacts::parse(&ALL_TOOLS.replace('\n', "\r\n"));
        assert_eq!(facts, HostFacts::parse(ALL_TOOLS));
        assert!(facts.can_push());
    }
}
