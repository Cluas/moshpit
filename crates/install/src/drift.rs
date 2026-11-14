//! What the host actually holds, against what its manifest says (B315).
//!
//! The manifest is the app's own record, written when something lands. A
//! file taken away or edited by hand afterwards left it saying the file was
//! there and current: the push conf deleted, the sender or the stamp
//! script removed, the hooks taken out of an agent's config — and nothing
//! ever looked again, so pushes stopped without a word (`status` and the
//! pairing's digest compare were the manifest's alone).
//!
//! One command looks: a digest for each of Moshpit's files the manifest
//! records (the two scripts, this device's push conf), and whether each
//! agent config it names still calls the stamp script. [`reconcile`] makes
//! the manifest say what was found, so every decision made on it — the
//! statuses, care's refresh, the pairing's rewrite — sees the host as it
//! is:
//!
//! - a file that isn't there is dropped from the manifest (it reads as
//!   absent, and is installed again where care would install it);
//! - a file with other content is recorded with that content's digest (a
//!   script reads as stale, a conf as not this phone's);
//! - an agent config without Moshpit's hooks drops its registration (the
//!   person may have taken them out: care treats the host as one without
//!   hooks — it asks, it doesn't put them back);
//! - what couldn't be told (no checksum tool, no answer for a line) is
//!   left as the manifest has it.
//!
//! The command reads and never writes; what was found is only written to
//! the host's manifest when something is installed there next.

use crate::commands;
use crate::hooks::{self, HookAgent};
use crate::installer::InstallState;
use crate::manifest::{InstallComponent, InstallManifest};

/// What a registration's recorded digest says once a look found it isn't
/// what this build registers ([`Drift::HooksOutdated`]):
/// [`InstallState::status`](crate::installer::InstallState::status) reads
/// it as stale. A registration's digest is otherwise only a record of the
/// config written; its status is its presence.
pub const OUTDATED_REGISTRATION: &str = "outdated-registration";

/// What is looked at for one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A file of Moshpit's: its digest, or `-` when it isn't there.
    File,
    /// An agent's config: whether it still calls the stamp script.
    Hooks,
}

/// One thing to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub component: InstallComponent,
    pub path: String,
    pub kind: Kind,
}

/// What the host turned out to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// The manifest records it; the host doesn't have it.
    Missing(InstallComponent),
    /// There, with other content than recorded (`found`: its digest).
    Edited {
        component: InstallComponent,
        found: String,
    },
    /// The agent's config no longer calls the stamp script.
    HooksGone { agent: String },
    /// The agent's config calls it, but not under every event this build
    /// registers for the agent: an older build's registration (B381).
    HooksOutdated { agent: String },
}

/// The checks for `manifest`: the scripts, `conn`'s push conf, and each
/// agent's registration whose config path is one a command may name.
pub fn checks(manifest: &InstallManifest, conn: Option<&str>) -> Vec<Check> {
    let mut out = Vec::new();
    let mut file = |component: InstallComponent| {
        if manifest.get(&component).is_some()
            && let Some(path) = component.path()
        {
            out.push(Check {
                component,
                path,
                kind: Kind::File,
            });
        }
    };
    file(InstallComponent::Stamp);
    file(InstallComponent::Sender);
    if let Some(conn) = conn {
        // Every recorded pairing of the connection: the legacy conf and each
        // install's (B443).
        for pairing in manifest.pairings_of(conn) {
            file(pairing);
        }
    }
    for agent in HookAgent::ALL {
        let component = InstallComponent::Hooks {
            agent: agent.id.to_owned(),
        };
        let Some(recorded) = manifest.get(&component) else {
            continue;
        };
        let path = recorded
            .config_path
            .clone()
            .unwrap_or_else(|| agent.config_path.to_owned());
        if commands::is_safe_path(&path) {
            out.push(Check {
                component,
                path,
                kind: Kind::Hooks,
            });
        }
    }
    out
}

/// The one command that looks at every check, answering `<index>=<value>`
/// a line each, in order; `None` with nothing to look at.
pub fn command(checks: &[Check]) -> Option<String> {
    if checks.is_empty() {
        return None;
    }
    let lines: Vec<String> = checks
        .iter()
        .enumerate()
        .map(|(i, check)| {
            let path = &check.path;
            let value = match check.kind {
                Kind::File => format!(
                    "if [ -f \"{path}\" ]; then {}; else echo -; fi",
                    commands::sha256(path)
                ),
                Kind::Hooks => {
                    let events = registered_events(&check.component)
                        .iter()
                        .map(|needle| format!("grep -qF '{needle}' \"{path}\""))
                        .collect::<Vec<_>>()
                        .join(" && ");
                    // Ours in a config whose hooks the agent no longer
                    // reads, its hooks file being there (B385): out of
                    // date, to be registered where it reads them.
                    let ignored = shadowing_file(&check.component, path)
                        .map(|file| format!("[ -f \"{file}\" ]; then echo old; elif "))
                        .unwrap_or_default();
                    format!(
                        "if grep -qF '{}' \"{path}\" 2>/dev/null; then if {ignored}{events}; then echo yes; \
                         else echo old; fi; else echo no; fi",
                        hooks::MARKER
                    )
                }
            };
            format!("printf '%s=%s\\n' {i} \"$({value})\"")
        })
        .collect();
    Some(lines.join("; "))
}

/// What an agent's config holds for each event this build registers for
/// it, as text a `grep -F` finds: the event's key (`"BeforeAgent"`) in
/// the JSON family, its table (`[[hooks.Stop]]`) in Codex's TOML. None of
/// it holds a quote of the shell's.
fn registered_events(component: &InstallComponent) -> Vec<String> {
    let InstallComponent::Hooks { agent } = component else {
        return Vec::new();
    };
    let Some(agent) = HookAgent::named(agent) else {
        return Vec::new();
    };
    agent
        .events()
        .iter()
        .map(|(event, _, _)| match agent.format {
            hooks::Format::ClaudeJson | hooks::Format::EventMapJson => format!("\"{event}\""),
            hooks::Format::CodexToml => format!("[[hooks.{event}]]"),
        })
        .collect()
}

/// The hooks file that makes the agent ignore the hooks in `path`, if it
/// has one and `path` isn't it (Factory's `hooks.json`, B385).
fn shadowing_file(component: &InstallComponent, path: &str) -> Option<&'static str> {
    let InstallComponent::Hooks { agent } = component else {
        return None;
    };
    HookAgent::named(agent)?
        .hooks_file
        .filter(|file| *file != path)
}

/// `state` with its manifest saying what `output` (the answer to
/// [`command`]) found, and what differed.
pub fn reconcile(
    state: &InstallState,
    checks: &[Check],
    output: &str,
) -> (InstallState, Vec<Drift>) {
    let mut manifest = state.manifest.clone();
    let mut drifts = Vec::new();
    let answers: std::collections::HashMap<usize, &str> = output
        .lines()
        .filter_map(|line| {
            let (index, value) = line.trim_end_matches('\r').split_once('=')?;
            Some((index.trim().parse().ok()?, value.trim()))
        })
        .collect();
    for (i, check) in checks.iter().enumerate() {
        let Some(&value) = answers.get(&i) else {
            continue;
        };
        let Some(recorded) = manifest.get(&check.component).cloned() else {
            continue;
        };
        match check.kind {
            Kind::File if value == "-" => {
                manifest.set(&check.component, None);
                drifts.push(Drift::Missing(check.component.clone()));
            }
            Kind::File => {
                let found = value.to_ascii_lowercase();
                let is_digest = found.len() == 64 && found.bytes().all(|b| b.is_ascii_hexdigit());
                if is_digest && recorded.digest.as_deref() != Some(found.as_str()) {
                    let mut edited = recorded;
                    edited.digest = Some(found.clone());
                    manifest.set(&check.component, Some(edited));
                    drifts.push(Drift::Edited {
                        component: check.component.clone(),
                        found,
                    });
                }
            }
            Kind::Hooks if value == "no" => {
                manifest.set(&check.component, None);
                if let InstallComponent::Hooks { agent } = &check.component {
                    drifts.push(Drift::HooksGone {
                        agent: agent.clone(),
                    });
                }
            }
            // Out of date: read as stale, so care registers it again the
            // way this build does (B381).
            Kind::Hooks if value == "old" => {
                let mut outdated = recorded;
                outdated.digest = Some(OUTDATED_REGISTRATION.to_owned());
                manifest.set(&check.component, Some(outdated));
                if let InstallComponent::Hooks { agent } = &check.component {
                    drifts.push(Drift::HooksOutdated {
                        agent: agent.clone(),
                    });
                }
            }
            // As this build registers it: a mark an earlier look left is
            // gone.
            Kind::Hooks if value == "yes" => {
                if recorded.digest.as_deref() == Some(OUTDATED_REGISTRATION) {
                    let mut current = recorded;
                    current.digest = None;
                    manifest.set(&check.component, Some(current));
                }
            }
            Kind::Hooks => {}
        }
    }
    (
        InstallState {
            facts: state.facts.clone(),
            manifest,
        },
        drifts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::HostFacts;
    use crate::manifest::Component;
    use crate::scripts;
    use moshpit_seal::Date;

    const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

    fn manifest() -> InstallManifest {
        let now = Date::from_unix(1_762_800_000.0);
        let mut m = InstallManifest::empty();
        for component in [InstallComponent::Stamp, InstallComponent::Sender] {
            m.set(
                &component,
                Some(Component {
                    digest: scripts::digest(&component),
                    ..Component::new(now)
                }),
            );
        }
        m.set(
            &InstallComponent::Pairing { conn: CONN.into(), install: None },
            Some(Component {
                digest: Some("c".repeat(64)),
                connection_id: Some(CONN.into()),
                ..Component::new(now)
            }),
        );
        m.set(
            &InstallComponent::Hooks {
                agent: "claude".into(),
            },
            Some(Component {
                config_path: Some("$HOME/.claude/settings.json".into()),
                ..Component::new(now)
            }),
        );
        m
    }

    fn state(m: InstallManifest) -> InstallState {
        InstallState {
            facts: HostFacts::parse("home=/h"),
            manifest: m,
        }
    }

    #[test]
    fn one_command_looks_at_what_the_manifest_records_and_nothing_else() {
        let looked = checks(&manifest(), Some(CONN));
        let paths: Vec<&str> = looked.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "$HOME/.moshpit/moshpit-stamp.sh",
                "$HOME/.moshpit/moshpit-push.sh",
                &format!("$HOME/.moshpit/push.d/{CONN}.conf"),
                "$HOME/.claude/settings.json",
            ]
        );
        assert!(
            checks(&manifest(), Some("another-device")).len() == 3,
            "only this device's conf"
        );
        assert!(command(&[]).is_none());
        let text = command(&looked).unwrap();
        assert!(!text.contains('\n'), "one line");
        assert!(
            !text.contains("base64 -d >") && !text.contains("rm "),
            "it only reads"
        );
    }

    #[test]
    fn a_path_from_the_manifest_that_isnt_safe_is_not_looked_at() {
        let mut m = manifest();
        let mut hooks = m
            .get(&InstallComponent::Hooks {
                agent: "claude".into(),
            })
            .cloned()
            .unwrap();
        hooks.config_path = Some("$(touch x)".into());
        m.set(
            &InstallComponent::Hooks {
                agent: "claude".into(),
            },
            Some(hooks),
        );
        assert!(checks(&m, None).iter().all(|c| c.kind == Kind::File));
    }

    #[test]
    fn what_is_missing_goes_what_is_edited_is_recorded_as_found() {
        let looked = checks(&manifest(), Some(CONN));
        let edited = "e".repeat(64);
        let output = format!("0=-\r\n1={edited}\n2=-\n3=no\n");
        let (after, drifts) = reconcile(&state(manifest()), &looked, &output);
        assert_eq!(
            after.status(&InstallComponent::Stamp),
            crate::installer::ComponentStatus::Absent
        );
        assert_eq!(
            after.status(&InstallComponent::Sender),
            crate::installer::ComponentStatus::Stale {
                installed: edited.clone()
            }
        );
        assert!(after.manifest.pairing_entry(CONN).is_none());
        assert!(crate::installer::installed_hook_agents(&after.manifest).is_empty());
        assert_eq!(drifts.len(), 4);
        assert!(drifts.contains(&Drift::HooksGone {
            agent: "claude".into()
        }));
    }

    /// B381: a registration that calls the stamp script but lacks one of
    /// this build's event names reads as out of date (care registers it
    /// again); once it has them all again it reads current.
    #[test]
    fn a_registration_from_an_older_build_reads_out_of_date() {
        let looked = checks(&manifest(), Some(CONN));
        let hooks = InstallComponent::Hooks {
            agent: "claude".into(),
        };
        let text = command(&looked).unwrap();
        for event in ["UserPromptSubmit", "PreToolUse", "Notification", "Stop"] {
            assert!(text.contains(&format!("grep -qF '\"{event}\"'")), "{text}");
        }
        let (old, drifts) = reconcile(&state(manifest()), &looked, "3=old\n");
        assert!(matches!(
            old.hooks_status("claude"),
            crate::installer::ComponentStatus::Stale { .. }
        ));
        assert!(drifts.contains(&Drift::HooksOutdated {
            agent: "claude".into()
        }));
        let (again, _) = reconcile(&old, &looked, "3=yes\n");
        assert_eq!(
            again.status(&hooks),
            crate::installer::ComponentStatus::Current
        );
    }

    /// B385: Factory's registration in its settings.json reads out of date
    /// once its hooks file is there (droid reads that one instead); one in
    /// the hooks file is checked as it is.
    #[test]
    fn factorys_settings_registration_is_out_of_date_beside_its_hooks_file() {
        let mut m = manifest();
        let factory = InstallComponent::Hooks {
            agent: "factory".into(),
        };
        let at = |path: &str| Component {
            config_path: Some(path.into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        };
        m.set(&factory, Some(at("$HOME/.factory/settings.json")));
        let text = command(&checks(&m, None)).unwrap();
        assert!(
            text.contains(
                "if grep -qF 'moshpit-stamp.sh' \"$HOME/.factory/settings.json\" 2>/dev/null; then \
                 if [ -f \"$HOME/.factory/hooks.json\" ]; then echo old; elif "
            ),
            "{text}"
        );
        m.set(&factory, Some(at("$HOME/.factory/hooks.json")));
        let text = command(&checks(&m, None)).unwrap();
        assert!(
            !text.contains("[ -f \"$HOME/.factory/hooks.json\" ]; then echo old"),
            "{text}"
        );
        assert!(text.contains("grep -qF 'moshpit-stamp.sh' \"$HOME/.factory/hooks.json\""));
    }

    #[test]
    fn what_matches_or_couldnt_be_told_is_left_as_recorded() {
        let looked = checks(&manifest(), Some(CONN));
        let stamp = scripts::digest(&InstallComponent::Stamp).unwrap();
        let sender = scripts::digest(&InstallComponent::Sender)
            .unwrap()
            .to_uppercase();
        // No checksum tool for the conf (empty); line 3 never answered.
        let output = format!("0={stamp}\n1={sender}\n2=\ngarbage\n");
        let (after, drifts) = reconcile(&state(manifest()), &looked, &output);
        assert_eq!(after.manifest, manifest());
        assert!(drifts.is_empty(), "{drifts:?}");
    }
}
