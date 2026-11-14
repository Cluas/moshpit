//! The coding agents Moshpit registers hooks for, and editing their configs
//! (`HookAgent`, `AgentHookConfig`): config text in, config text out.
//!
//! The JSON family (Claude Code, Gemini CLI, Qwen Code, Qoder, Factory,
//! CodeBuddy) is merged the app's way — read and rewritten through
//! `JSONSerialization`, pretty, keys sorted in its collation, slashes
//! unescaped, a newline at the end — so the file a user ends up with is the
//! one the app would have written. Codex's TOML gets a fenced block.

use moshpit_seal::foundation::json::{self, Dialect, Object, Style, Value};
use moshpit_seal::foundation::text;

/// How an agent's config is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    ClaudeJson,
    CodexToml,
    /// Claude Code's hooks with no `hooks` key around them: the file is the
    /// event map (Factory's `~/.factory/hooks.json`, B385).
    EventMapJson,
}

/// A coding agent Moshpit can register hooks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HookAgent {
    /// Stamp argument and `@moshpit_agent` label.
    pub id: &'static str,
    pub display_name: &'static str,
    /// Shell expression, expanded on the host.
    pub config_path: &'static str,
    pub format: Format,
    /// The names its CLI goes by on a host's PATH: the preflight counts
    /// the agent as there when one is found, or its config directory is.
    pub commands: &'static [&'static str],
    /// A file of hooks the agent reads instead of its config's `hooks` key
    /// once it exists (B385): droid reads `~/.factory/hooks.json`, and
    /// `settings.json`'s hooks only while that file is absent.
    pub hooks_file: Option<&'static str>,
}

impl HookAgent {
    /// The roster, Claude Code first.
    pub const ALL: [Self; 7] = [
        Self {
            id: "claude",
            display_name: "Claude Code",
            config_path: "$HOME/.claude/settings.json",
            format: Format::ClaudeJson,
            commands: &["claude"],
            hooks_file: None,
        },
        Self {
            id: "codex",
            display_name: "Codex",
            config_path: "$HOME/.codex/config.toml",
            format: Format::CodexToml,
            commands: &["codex"],
            hooks_file: None,
        },
        Self {
            id: "gemini",
            display_name: "Gemini CLI",
            config_path: "$HOME/.gemini/settings.json",
            format: Format::ClaudeJson,
            commands: &["gemini"],
            hooks_file: None,
        },
        Self {
            id: "qwen",
            display_name: "Qwen Code",
            config_path: "$HOME/.qwen/settings.json",
            format: Format::ClaudeJson,
            commands: &["qwen"],
            hooks_file: None,
        },
        Self {
            id: "qoder",
            display_name: "Qoder",
            config_path: "$HOME/.qoder/settings.json",
            format: Format::ClaudeJson,
            commands: &["qodercli", "qoder"],
            hooks_file: None,
        },
        Self {
            id: "factory",
            display_name: "Factory",
            config_path: "$HOME/.factory/settings.json",
            format: Format::ClaudeJson,
            commands: &["droid", "factory"],
            hooks_file: Some("$HOME/.factory/hooks.json"),
        },
        Self {
            id: "codebuddy",
            display_name: "CodeBuddy",
            config_path: "$HOME/.codebuddy/settings.json",
            format: Format::ClaudeJson,
            commands: &["codebuddy", "cbc"],
            hooks_file: None,
        },
    ];

    pub fn named(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.id == id)
    }

    /// The directory under `$HOME` its config lives in (`.claude`).
    pub fn config_dir(&self) -> &'static str {
        let rest = self.config_path.trim_start_matches("$HOME/");
        rest.split('/').next().unwrap_or(rest)
    }

    /// The config location as a person reads it.
    pub fn config_hint(&self) -> String {
        self.config_path.replace("$HOME", "~")
    }

    /// How the file at `path` holds this agent's hooks: its hooks file's
    /// own shape when `path` is that file (B385), else its config's.
    pub fn format_at(&self, path: &str) -> Format {
        match self.hooks_file {
            Some(file) if file == path => Format::EventMapJson,
            _ => self.format,
        }
    }

    /// The events Moshpit registers in this agent's config, the state each
    /// stamps, and whether it takes a `matcher`, in the order they're
    /// written.
    pub fn events(&self) -> &'static [(&'static str, &'static str, bool)] {
        match (self.format, self.id) {
            (Format::CodexToml, _) => &CODEX_EVENTS,
            (_, "gemini") => &GEMINI_EVENTS,
            _ => &EVENTS,
        }
    }

    /// A step the user has to take before these hooks will run, if there is
    /// one: Codex will not run a newly added hook until it is trusted.
    pub fn trust_step(&self) -> Option<&'static str> {
        match self.format {
            Format::ClaudeJson | Format::EventMapJson => None,
            Format::CodexToml => Some(
                "Codex won't run a newly added hook until you trust it. Run /hooks inside Codex on this host and trust \
                 the four Moshpit entries \u{2014} until you do, nothing will be sent.",
            ),
        }
    }
}

/// The four events Moshpit registers, the state each stamps, and whether it
/// takes a `matcher` (only the tool-scoped two do).
pub const EVENTS: [(&str, &str, bool); 4] = [
    ("UserPromptSubmit", "working", false),
    ("PreToolUse", "working", true),
    ("Notification", "attention", true),
    ("Stop", "done", false),
];

/// Gemini CLI's four (B381). It takes no Claude Code name but
/// `Notification`: an unknown event is skipped with a warning
/// (`hookRegistry.ts`), so the Claude names registered nothing but the
/// attention hook. Its own names, as `gemini hooks migrate --from-claude`
/// maps them (`migrate.ts`): `UserPromptSubmit` → `BeforeAgent`,
/// `PreToolUse` → `BeforeTool`, `Stop` → `AfterAgent`. Its `Notification`
/// fires for tool permission only, and `"*"` matches every tool.
pub const GEMINI_EVENTS: [(&str, &str, bool); 4] = [
    ("BeforeAgent", "working", false),
    ("BeforeTool", "working", true),
    ("Notification", "attention", true),
    ("AfterAgent", "done", false),
];

/// Codex's four, as its fenced block lists them: no `Notification` event,
/// its attention signal is `PermissionRequest`; TOML tables take no matcher.
pub const CODEX_EVENTS: [(&str, &str, bool); 4] = [
    ("UserPromptSubmit", "working", false),
    ("PreToolUse", "working", false),
    ("Stop", "done", false),
    ("PermissionRequest", "attention", false),
];

/// The substring that marks a hook command as Moshpit's.
pub const MARKER: &str = "moshpit-stamp.sh";

/// How a registered hook invokes the stamp script.
pub fn command(state: &str, agent: &str) -> String {
    format!("sh ~/.moshpit/moshpit-stamp.sh {state} {agent}")
}

/// Why a config was not edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    UnparseableJson(String),
    NotAnObject,
}

/// `JSONSerialization`'s description of every parse error.
pub const PARSE_ERROR: &str =
    "The data couldn\u{2019}t be read because it isn\u{2019}t in the correct format.";

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnparseableJson(detail) => write!(
                f,
                "That config file isn't valid JSON ({detail}). Fix it on the host, or move it aside and Moshpit will \
                 write a fresh one."
            ),
            Self::NotAnObject => {
                f.write_str("That config file's top level isn't a JSON object, so Moshpit can't merge hooks into it.")
            }
        }
    }
}

impl std::error::Error for Failure {}

// MARK: - JSON family

/// A hook entry Moshpit registered: an object whose command mentions the
/// stamp script.
fn is_our_entry(entry: &Value) -> bool {
    entry
        .as_object()
        .and_then(|e| e.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|command| text::contains_characters(command, MARKER))
}

/// `group` without Moshpit's entries: unchanged when it holds none of them,
/// `None` when nothing else was in it.
///
/// Fix (hooks-shared-group, B356): the Swift original drops every group
/// one of whose entries runs the stamp script (`AgentHookConfig.isMoshpit`),
/// whole. A hook the person added beside ours in the same group went with
/// it, on every refresh and on removal — and that is where an agent's own
/// hooks editor puts one: Claude Code's `/hooks` adds a hook to the group
/// that already has its matcher, and the events that take none
/// (`UserPromptSubmit`, `Stop`) have only ours to add to. Now only our
/// entries go, and a group only once nothing of the person's is left.
fn without_ours(group: &Value) -> Option<Value> {
    let Some(object) = group.as_object() else {
        return Some(group.clone());
    };
    let Some(Value::Array(entries)) = object.get("hooks") else {
        return Some(group.clone());
    };
    if !entries.iter().any(is_our_entry) {
        return Some(group.clone());
    }
    let kept: Vec<Value> = entries
        .iter()
        .filter(|entry| !is_our_entry(entry))
        .cloned()
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mut object = object.clone();
    object.insert("hooks", Value::Array(kept));
    Some(Value::Object(object))
}

/// `existing` made to hold `root`, changing no more of the person's file
/// than that takes; written whole, as Foundation would, when it can't be
/// (B375, [`crate::jsonedit`]).
fn written(existing: &str, root: &Object) -> String {
    crate::jsonedit::splice(existing, &Value::Object(root.clone()))
        .unwrap_or_else(|| serialise(root))
}

fn serialise(root: &Object) -> String {
    let style = Style {
        pretty: true,
        sorted_keys: true,
        escape_slashes: false,
    };
    format!(
        "{}\n",
        json::write_serialization(&Value::Object(root.clone()), style)
    )
}

/// Merge Moshpit's hook groups into a Claude-format settings file
/// (`AgentHookConfig.mergedJSON`). Idempotent: our entries are taken out
/// (a group left empty goes, B356) before our group is appended. Every other key and every hook the user added
/// is kept; the file comes back with sorted keys, as the app writes it.
///
/// Fix (hooks-array-elements): Swift reads an event's groups as
/// `[[String: Any]]` (`AgentHookConfig.swift:145`), so an array holding one
/// thing that is not an object reads as empty and the user's groups beside
/// it are dropped. Here only ours go; anything else in the array stays.
pub fn merged_json(existing: &str, agent: &str) -> Result<String, Failure> {
    merged_json_in(existing, agent, true)
}

/// [`merged_json`] for a file whose top level holds the hooks (`wrapped`
/// false: Factory's `hooks.json`, B385) or their `hooks` key.
fn merged_json_in(existing: &str, agent: &str, wrapped: bool) -> Result<String, Failure> {
    let trimmed = text::trim_white_space(existing);
    let mut root = Object::new();
    if !trimmed.is_empty() {
        match json::parse(
            trimmed.as_bytes(),
            Dialect::Serialization {
                fragments_allowed: true,
            },
        ) {
            Err(_) => return Err(Failure::UnparseableJson(PARSE_ERROR.to_owned())),
            Ok(Value::Object(object)) => root = object,
            Ok(_) => return Err(Failure::NotAnObject),
        }
    }
    let mut hooks = match (wrapped, root.get("hooks")) {
        (false, _) => root.clone(),
        (true, Some(Value::Object(hooks))) => hooks.clone(),
        (true, _) => Object::new(),
    };
    let events = HookAgent::named(agent).map_or(&EVENTS[..], |a| a.events());
    // Ours come out of every other event too: an install this build
    // replaces may have registered under names it no longer uses (Gemini
    // CLI's took Claude Code's, B381). An event left with nothing goes, as
    // in removal.
    let others: Vec<String> = hooks
        .keys()
        .filter(|key| !events.iter().any(|(event, _, _)| event == key))
        .map(str::to_owned)
        .collect();
    for key in others {
        let Some(Value::Array(groups)) = hooks.get(&key) else {
            continue;
        };
        let kept: Vec<Value> = groups.iter().filter_map(without_ours).collect();
        if kept == *groups {
            continue;
        }
        if kept.is_empty() {
            hooks.remove(&key);
        } else {
            hooks.insert(key, Value::Array(kept));
        }
    }
    for &(event, state, matched) in events {
        let mut groups: Vec<Value> = match hooks.get(event) {
            Some(Value::Array(groups)) => groups.iter().filter_map(without_ours).collect(),
            _ => Vec::new(),
        };
        let mut group = Object::new();
        let mut entry = Object::new();
        entry.insert("type", Value::String("command".to_owned()));
        entry.insert("command", Value::String(command(state, agent)));
        group.insert("hooks", Value::Array(vec![Value::Object(entry)]));
        if matched {
            group.insert("matcher", Value::String("*".to_owned()));
        }
        groups.push(Value::Object(group));
        hooks.insert(event, Value::Array(groups));
    }
    if !wrapped {
        // Factory's hooks.json is the person's file too (B375).
        return Ok(written(existing, &hooks));
    }
    root.insert("hooks", Value::Object(hooks));
    Ok(written(existing, &root))
}

/// Take Moshpit's hook entries out, leaving everything else
/// (`AgentHookConfig.withoutMoshpitJSON`): a group left empty goes (B356),
/// an event left with no group goes, and so does `hooks` when nothing is
/// left in it. An empty config is returned as it came.
///
/// Fix (hooks-array-elements): Swift leaves an event whose array holds
/// anything but objects untouched (`AgentHookConfig.swift:171`), our groups
/// included, so uninstalling leaves the hooks running; here they go.
pub fn without_moshpit_json(existing: &str) -> Result<String, Failure> {
    without_moshpit_json_in(existing, true)
}

/// [`without_moshpit_json`] for a file whose top level holds the hooks
/// (`wrapped` false, B385) or their `hooks` key.
fn without_moshpit_json_in(existing: &str, wrapped: bool) -> Result<String, Failure> {
    let trimmed = text::trim_white_space(existing);
    if trimmed.is_empty() {
        return Ok(existing.to_owned());
    }
    let Ok(Value::Object(root)) = json::parse(
        trimmed.as_bytes(),
        Dialect::Serialization {
            fragments_allowed: false,
        },
    ) else {
        return Err(Failure::UnparseableJson(
            "could not be read for removal".to_owned(),
        ));
    };
    let mut out = root.clone();
    let mut hooks = if wrapped {
        let Some(Value::Object(hooks)) = root.get("hooks").cloned() else {
            return Ok(written(existing, &out));
        };
        hooks
    } else {
        root.clone()
    };
    let events: Vec<String> = hooks.keys().map(str::to_owned).collect();
    for event in events {
        let Some(Value::Array(groups)) = hooks.get(&event) else {
            continue;
        };
        let kept: Vec<Value> = groups.iter().filter_map(without_ours).collect();
        if kept.is_empty() {
            hooks.remove(&event);
        } else {
            hooks.insert(event, Value::Array(kept));
        }
    }
    if !wrapped {
        // Factory's hooks.json is the person's file too (B375).
        return Ok(written(existing, &hooks));
    }
    if hooks.is_empty() {
        out.remove("hooks");
    } else {
        out.insert("hooks", Value::Object(hooks));
    }
    Ok(written(existing, &out))
}

// MARK: - Codex TOML

pub const TOML_BEGIN: &str = "# >>> moshpit vibe island >>>";
pub const TOML_END: &str = "# <<< moshpit vibe island <<<";

/// Append (or replace) Moshpit's fenced block in a Codex config
/// (`AgentHookConfig.mergedTOML`). Codex has no `Notification` event; its
/// attention signal is `PermissionRequest`.
pub fn merged_toml(existing: &str, agent: &str) -> String {
    let mut body = without_moshpit_toml(existing);
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    if !body.is_empty() {
        body.push('\n');
    }
    let mut block = format!("{TOML_BEGIN}\n");
    let table = |event: &str, state: &str| {
        format!(
            "[[hooks.{event}]]\n[[hooks.{event}.hooks]]\ntype = \"command\"\ncommand = \"{}\"\n",
            command(state, agent)
        )
    };
    for (event, state, _) in CODEX_EVENTS {
        block.push_str(&table(event, state));
    }
    block.push_str(TOML_END);
    block.push('\n');
    body + &block
}

/// Strip Moshpit's fenced block and nothing else
/// (`AgentHookConfig.withoutMoshpitTOML`); trailing blank lines go too.
///
/// Fix (toml-crlf): Swift splits the file on the `Character` `"\n"`, which a
/// CR LF pair is not (`AgentHookConfig.swift:235`): once an editor has
/// turned our block's line ends into CR LF its fences are never found, the
/// next install appends a second block and uninstall removes neither. Here
/// lines end at LF and a fence is recognised with or without the CR. Files
/// without a CR come out byte for byte as the app writes them.
pub fn without_moshpit_toml(existing: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in existing.split('\n') {
        let fence = text::trim_inline_spaces(line.strip_suffix('\r').unwrap_or(line));
        if fence == TOML_BEGIN {
            inside = true;
            continue;
        }
        if fence == TOML_END {
            inside = false;
            continue;
        }
        if !inside {
            kept.push(line);
        }
    }
    while kept.last().is_some_and(|line| {
        text::trim_inline_spaces(line.strip_suffix('\r').unwrap_or(line)).is_empty()
    }) {
        kept.pop();
    }
    let body = kept.join("\n");
    if body.is_empty() {
        String::new()
    } else {
        body + "\n"
    }
}

/// Moshpit's hook entries in an agent's config as it stands: each one's
/// event and command, in the file's order — what the Host Setup page's
/// test runs (B380). A config that won't parse says why.
pub fn registered(existing: &str, agent: &HookAgent) -> Result<Vec<(String, String)>, Failure> {
    registered_at(existing, agent, agent.config_path)
}

/// [`registered`] in the file at `path`, read in its shape
/// ([`HookAgent::format_at`]).
pub fn registered_at(
    existing: &str,
    agent: &HookAgent,
    path: &str,
) -> Result<Vec<(String, String)>, Failure> {
    match agent.format_at(path) {
        Format::ClaudeJson => registered_json(existing, true),
        Format::EventMapJson => registered_json(existing, false),
        Format::CodexToml => Ok(registered_toml(existing)),
    }
}

fn registered_json(existing: &str, wrapped: bool) -> Result<Vec<(String, String)>, Failure> {
    let trimmed = text::trim_white_space(existing);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let root = match json::parse(
        trimmed.as_bytes(),
        Dialect::Serialization {
            fragments_allowed: true,
        },
    ) {
        Err(_) => return Err(Failure::UnparseableJson(PARSE_ERROR.to_owned())),
        Ok(Value::Object(object)) => object,
        Ok(_) => return Err(Failure::NotAnObject),
    };
    let mut found = Vec::new();
    let hooks = if wrapped {
        let Some(Value::Object(hooks)) = root.get("hooks") else {
            return Ok(found);
        };
        hooks
    } else {
        &root
    };
    for (event, groups) in hooks.iter() {
        for group in groups.as_array().unwrap_or_default() {
            let entries = group
                .as_object()
                .and_then(|g| g.get("hooks"))
                .and_then(Value::as_array)
                .unwrap_or_default();
            for entry in entries.iter().filter(|e| is_our_entry(e)) {
                if let Some(command) = entry
                    .as_object()
                    .and_then(|e| e.get("command"))
                    .and_then(Value::as_str)
                {
                    found.push((event.to_owned(), command.to_owned()));
                }
            }
        }
    }
    Ok(found)
}

/// Inside our fences: each `[[hooks.<Event>]]` table's `command = "…"`.
fn registered_toml(existing: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut inside = false;
    let mut event: Option<&str> = None;
    for line in existing.split('\n') {
        let line = text::trim_inline_spaces(line.strip_suffix('\r').unwrap_or(line));
        if line == TOML_BEGIN {
            inside = true;
            continue;
        }
        if line == TOML_END {
            inside = false;
            event = None;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(table) = line
            .strip_prefix("[[hooks.")
            .and_then(|rest| rest.strip_suffix("]]"))
        {
            if !table.contains('.') {
                event = Some(table);
            }
        } else if let (Some(event), Some(value)) = (event, line.strip_prefix("command = \""))
            && let Some(command) = value.strip_suffix('"')
            && text::contains_characters(command, MARKER)
        {
            found.push((event.to_owned(), command.to_owned()));
        }
    }
    found
}

/// Merge for whichever format this agent uses.
pub fn merged(existing: &str, agent: &HookAgent) -> Result<String, Failure> {
    merged_at(existing, agent, agent.config_path)
}

/// [`merged`] into the file at `path`, in its shape (B385).
pub fn merged_at(existing: &str, agent: &HookAgent, path: &str) -> Result<String, Failure> {
    match agent.format_at(path) {
        Format::ClaudeJson => merged_json(existing, agent.id),
        Format::EventMapJson => merged_json_in(existing, agent.id, false),
        Format::CodexToml => Ok(merged_toml(existing, agent.id)),
    }
}

/// Remove for whichever format this agent uses.
pub fn removed(existing: &str, agent: &HookAgent) -> Result<String, Failure> {
    removed_at(existing, agent, agent.config_path)
}

/// [`removed`] from the file at `path`, in its shape (B385).
pub fn removed_at(existing: &str, agent: &HookAgent, path: &str) -> Result<String, Failure> {
    match agent.format_at(path) {
        Format::ClaudeJson => without_moshpit_json(existing),
        Format::EventMapJson => without_moshpit_json_in(existing, false),
        Format::CodexToml => Ok(without_moshpit_toml(existing)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(text: &str) -> Object {
        match json::parse(text.as_bytes(), Dialect::Decoder).unwrap() {
            Value::Object(object) => object,
            other => panic!("not an object: {other:?}"),
        }
    }

    fn groups(text: &str, event: &str) -> Vec<Value> {
        object(text)
            .get("hooks")
            .and_then(Value::as_object)
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .map(<[Value]>::to_vec)
            .unwrap_or_default()
    }

    fn commands(text: &str, event: &str) -> Vec<String> {
        groups(text, event)
            .iter()
            .flat_map(|g| {
                g.as_object()
                    .and_then(|g| g.get("hooks"))
                    .and_then(Value::as_array)
                    .map(<[Value]>::to_vec)
                    .unwrap_or_default()
            })
            .filter_map(|e| {
                e.as_object()
                    .and_then(|e| e.get("command"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    // MARK: - AgentHookConfigTests

    #[test]
    fn a_fresh_config_gets_all_four_events_with_the_right_states() {
        let out = merged_json("", "claude").unwrap();
        assert_eq!(
            commands(&out, "UserPromptSubmit"),
            ["sh ~/.moshpit/moshpit-stamp.sh working claude"]
        );
        assert_eq!(
            commands(&out, "PreToolUse"),
            ["sh ~/.moshpit/moshpit-stamp.sh working claude"]
        );
        assert_eq!(
            commands(&out, "Notification"),
            ["sh ~/.moshpit/moshpit-stamp.sh attention claude"]
        );
        assert_eq!(
            commands(&out, "Stop"),
            ["sh ~/.moshpit/moshpit-stamp.sh done claude"]
        );
    }

    #[test]
    fn only_the_tool_scoped_events_carry_a_matcher() {
        let out = merged_json("{}", "claude").unwrap();
        let matcher = |event: &str| {
            groups(&out, event)[0]
                .as_object()
                .unwrap()
                .get("matcher")
                .cloned()
        };
        assert_eq!(matcher("PreToolUse"), Some(Value::String("*".into())));
        assert_eq!(matcher("Notification"), Some(Value::String("*".into())));
        assert_eq!(matcher("UserPromptSubmit"), None);
        assert_eq!(matcher("Stop"), None);
    }

    #[test]
    fn everything_else_in_the_file_survives_including_the_users_own_hooks() {
        let existing = r#"{
          "model": "opus",
          "permissions": {"allow": ["Bash(git*)"]},
          "hooks": {
            "PreToolUse": [
              {"matcher": "Bash", "hooks": [{"type": "command", "command": "my-own-audit.sh"}]}
            ],
            "SessionStart": [
              {"hooks": [{"type": "command", "command": "greet.sh"}]}
            ]
          }
        }"#;
        let out = merged_json(existing, "claude").unwrap();
        let root = object(&out);
        assert_eq!(root.get("model").and_then(Value::as_str), Some("opus"));
        assert!(root.get("permissions").is_some());
        assert!(commands(&out, "PreToolUse").contains(&"my-own-audit.sh".to_owned()));
        assert!(
            commands(&out, "PreToolUse")
                .contains(&"sh ~/.moshpit/moshpit-stamp.sh working claude".to_owned())
        );
        assert_eq!(commands(&out, "SessionStart"), ["greet.sh"]);
    }

    #[test]
    fn re_running_replaces_our_hooks_instead_of_stacking_them() {
        let once = merged_json("{}", "claude").unwrap();
        let twice = merged_json(&once, "claude").unwrap();
        assert_eq!(once, twice);
        assert_eq!(commands(&twice, "PreToolUse").len(), 1);
        let switched = merged_json(&once, "codex").unwrap();
        assert_eq!(
            commands(&switched, "Stop"),
            ["sh ~/.moshpit/moshpit-stamp.sh done codex"]
        );
    }

    #[test]
    fn a_config_that_will_not_parse_is_refused_never_overwritten() {
        assert!(merged_json("{\"model\": }", "claude").is_err());
        let error = merged_json("{oops", "claude").unwrap_err();
        assert!(error.to_string().contains("valid JSON"));
        assert_eq!(
            error.to_string(),
            "That config file isn't valid JSON (The data couldn\u{2019}t be read because it isn\u{2019}t in the correct \
             format.). Fix it on the host, or move it aside and Moshpit will write a fresh one."
        );
    }

    #[test]
    fn a_top_level_that_is_not_an_object_is_refused() {
        assert_eq!(merged_json("[1,2,3]", "claude"), Err(Failure::NotAnObject));
        assert_eq!(merged_json("7", "claude"), Err(Failure::NotAnObject));
    }

    #[test]
    fn removal_takes_ours_out_and_leaves_theirs_alone() {
        let installed = merged_json(
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "mine.sh"}]}]}}"#,
            "claude",
        )
        .unwrap();
        let out = without_moshpit_json(&installed).unwrap();
        assert_eq!(commands(&out, "PreToolUse"), ["mine.sh"]);
        let hooks = object(&out)
            .get("hooks")
            .and_then(Value::as_object)
            .cloned()
            .unwrap();
        assert!(hooks.get("Stop").is_none());
        assert!(hooks.get("Notification").is_none());
    }

    /// B356: a hook the person added to one of our groups — where Claude
    /// Code's `/hooks` puts a `Stop` or `UserPromptSubmit` hook, those
    /// events having no matcher to tell groups apart — went with the group
    /// on every refresh and on removal. Only our entries go now.
    #[test]
    fn a_hook_added_beside_ours_in_our_group_survives_refresh_and_removal() {
        let mut installed = object(&merged_json("{}", "claude").unwrap());
        let mut hooks = installed
            .get("hooks")
            .and_then(Value::as_object)
            .cloned()
            .unwrap();
        for event in ["Stop", "PreToolUse"] {
            let mut groups = hooks.get(event).and_then(Value::as_array).unwrap().to_vec();
            let mut group = groups[0].as_object().cloned().unwrap();
            let mut entries = group
                .get("hooks")
                .and_then(Value::as_array)
                .unwrap()
                .to_vec();
            let mut mine = Object::new();
            mine.insert("type", Value::String("command".into()));
            mine.insert("command", Value::String(format!("notify-{event}.sh")));
            entries.push(Value::Object(mine));
            group.insert("hooks", Value::Array(entries));
            groups[0] = Value::Object(group);
            hooks.insert(event, Value::Array(groups));
        }
        installed.insert("hooks", Value::Object(hooks));
        let edited = serialise(&installed);
        assert_eq!(
            commands(&edited, "Stop").len(),
            2,
            "the person's hook is in our group"
        );

        let refreshed = merged_json(&edited, "claude").unwrap();
        assert_eq!(
            commands(&refreshed, "Stop"),
            [
                "notify-Stop.sh",
                "sh ~/.moshpit/moshpit-stamp.sh done claude"
            ]
        );
        assert_eq!(
            commands(&refreshed, "PreToolUse"),
            [
                "notify-PreToolUse.sh",
                "sh ~/.moshpit/moshpit-stamp.sh working claude"
            ]
        );
        // Its group keeps its matcher; a refresh of the refresh is the same.
        assert_eq!(
            groups(&refreshed, "PreToolUse")[0]
                .as_object()
                .and_then(|g| g.get("matcher"))
                .and_then(Value::as_str),
            Some("*")
        );
        assert_eq!(merged_json(&refreshed, "claude").unwrap(), refreshed);

        let removed = without_moshpit_json(&edited).unwrap();
        assert_eq!(commands(&removed, "Stop"), ["notify-Stop.sh"]);
        assert_eq!(commands(&removed, "PreToolUse"), ["notify-PreToolUse.sh"]);
        assert!(groups(&removed, "Notification").is_empty());
        assert_eq!(without_moshpit_json(&removed).unwrap(), removed);
    }

    #[test]
    fn removing_from_a_config_that_only_ever_had_our_hooks_leaves_no_hooks_key() {
        let installed = merged_json(r#"{"model":"opus"}"#, "claude").unwrap();
        let out = without_moshpit_json(&installed).unwrap();
        let root = object(&out);
        assert!(root.get("hooks").is_none());
        assert_eq!(root.get("model").and_then(Value::as_str), Some("opus"));
    }

    /// A re-install changes nothing; the person's keys stay in their
    /// order (B375: no longer sorted).
    #[test]
    fn a_re_install_produces_no_diff_and_keeps_the_keys_in_order() {
        let a = merged_json("{\"b\":1,\"a\":2}\n", "claude").unwrap();
        assert_eq!(merged_json(&a, "claude").unwrap(), a);
        assert!(a.starts_with("{\"b\":1,\"a\":2,\"hooks\":"), "{a}");
        assert!(a.ends_with("}\n"));
        assert!(!a.contains("\\/"));
    }

    /// B375: refreshing hooks already there changes nothing in the file,
    /// whichever order the earlier write put our entries' keys in.
    #[test]
    fn a_refresh_over_our_own_hooks_changes_nothing() {
        let installed = merged_json("", "claude").unwrap();
        assert_eq!(merged_json(&installed, "claude").unwrap(), installed);
        let theirs = "{\n  \"model\": \"opus\"\n}\n";
        let once = merged_json(theirs, "claude").unwrap();
        assert_eq!(merged_json(&once, "claude").unwrap(), once);
    }

    /// A config that isn't there yet is written as the app always wrote
    /// one.
    #[test]
    fn a_new_config_is_the_apps() {
        assert_eq!(
            merged_json("", "claude").unwrap(),
            "{\n  \"hooks\" : {\n    \"Notification\" : [\n      {\n        \"hooks\" : [\n          {\n            \
             \"command\" : \"sh ~/.moshpit/moshpit-stamp.sh attention claude\",\n            \"type\" : \"command\"\n          \
             }\n        ],\n        \"matcher\" : \"*\"\n      }\n    ],\n    \"PreToolUse\" : [\n      {\n        \"hooks\" \
             : [\n          {\n            \"command\" : \"sh ~/.moshpit/moshpit-stamp.sh working claude\",\n            \
             \"type\" : \"command\"\n          }\n        ],\n        \"matcher\" : \"*\"\n      }\n    ],\n    \"Stop\" : [\n      \
             {\n        \"hooks\" : [\n          {\n            \"command\" : \"sh ~/.moshpit/moshpit-stamp.sh done claude\",\n            \
             \"type\" : \"command\"\n          }\n        ]\n      }\n    ],\n    \"UserPromptSubmit\" : [\n      {\n        \
             \"hooks\" : [\n          {\n            \"command\" : \"sh ~/.moshpit/moshpit-stamp.sh working claude\",\n            \
             \"type\" : \"command\"\n          }\n        ]\n      }\n    ]\n  }\n}\n"
        );
    }

    /// B375: a config the person wrote keeps their layout: only Moshpit's
    /// part is written, in the file's own style (here, all on one line).
    #[test]
    fn a_config_keeps_its_layout_and_only_ours_is_added() {
        let merged = merged_json(r#"{"model":"opus"}"#, "claude").unwrap();
        assert_eq!(
            merged,
            "{\"model\":\"opus\",\"hooks\":{\"UserPromptSubmit\":[{\"hooks\":[{\"type\":\"command\",\
             \"command\":\"sh ~/.moshpit/moshpit-stamp.sh working claude\"}]}],\"PreToolUse\":[{\"hooks\":[{\"type\":\
             \"command\",\"command\":\"sh ~/.moshpit/moshpit-stamp.sh working claude\"}],\"matcher\":\"*\"}],\
             \"Notification\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"sh ~/.moshpit/moshpit-stamp.sh attention \
             claude\"}],\"matcher\":\"*\"}],\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\
             \"sh ~/.moshpit/moshpit-stamp.sh done claude\"}]}]}}"
        );
        assert_eq!(
            without_moshpit_json(&merged).unwrap(),
            r#"{"model":"opus"}"#
        );
        let pretty = "{\n    \"model\": \"opus\",\n    \"env\": {\"A\": \"1\"}\n}\n";
        let merged = merged_json(pretty, "claude").unwrap();
        assert!(merged.starts_with("{\n    \"model\": \"opus\",\n    \"env\": {\"A\": \"1\"},\n    \"hooks\": {\n        \""), "{merged}");
        assert_eq!(without_moshpit_json(&merged).unwrap(), pretty);
    }

    /// Fix (hooks-array-elements).
    #[test]
    fn an_array_with_something_odd_in_it_keeps_the_users_groups() {
        let existing =
            r#"{"hooks":{"Stop":["note",{"hooks":[{"type":"command","command":"mine.sh"}]}]}}"#;
        let merged = merged_json(existing, "claude").unwrap();
        assert_eq!(
            commands(&merged, "Stop"),
            ["mine.sh", "sh ~/.moshpit/moshpit-stamp.sh done claude"]
        );
        assert_eq!(groups(&merged, "Stop")[0], Value::String("note".into()));
        let removed = without_moshpit_json(&merged).unwrap();
        assert_eq!(commands(&removed, "Stop"), ["mine.sh"]);
    }

    // MARK: - Codex TOML

    #[test]
    fn the_codex_block_is_fenced_and_uses_permission_request_for_attention() {
        let out = merged_toml("", "codex");
        assert!(out.contains(TOML_BEGIN));
        assert!(out.contains(TOML_END));
        assert!(out.contains("[[hooks.PermissionRequest]]"));
        assert!(out.contains(r#"command = "sh ~/.moshpit/moshpit-stamp.sh attention codex""#));
        assert!(out.contains(r#"command = "sh ~/.moshpit/moshpit-stamp.sh done codex""#));
        assert!(!out.contains("[[hooks.Notification]]"));
        assert!(out.starts_with("# >>> moshpit vibe island >>>\n[[hooks.UserPromptSubmit]]\n[[hooks.UserPromptSubmit.hooks]]\ntype = \"command\"\ncommand = \"sh ~/.moshpit/moshpit-stamp.sh working codex\"\n[[hooks.PreToolUse]]\n"));
        assert!(out.ends_with("command = \"sh ~/.moshpit/moshpit-stamp.sh attention codex\"\n# <<< moshpit vibe island <<<\n"));
    }

    #[test]
    fn the_users_toml_is_preserved_above_our_block_and_re_running_replaces_it() {
        let existing = "model = \"gpt-5\"\n\n[tools]\nweb_search = true";
        let once = merged_toml(existing, "codex");
        assert!(once.starts_with("model = \"gpt-5\""));
        assert!(once.contains("web_search = true\n\n# >>> moshpit"));
        let twice = merged_toml(&once, "codex");
        assert_eq!(twice, once);
        assert_eq!(twice.matches(TOML_BEGIN).count(), 1);
    }

    #[test]
    fn removing_the_block_restores_the_file() {
        let existing = "model = \"gpt-5\"\n";
        let installed = merged_toml(existing, "codex");
        assert_eq!(without_moshpit_toml(&installed), existing);
        assert_eq!(without_moshpit_toml(existing), existing);
    }

    /// Fix (toml-crlf).
    #[test]
    fn fences_with_cr_lf_line_ends_are_still_found() {
        let installed = merged_toml("model = \"gpt-5\"\n", "codex").replace('\n', "\r\n");
        assert_eq!(
            merged_toml(&installed, "codex").matches(TOML_BEGIN).count(),
            1
        );
        assert_eq!(without_moshpit_toml(&installed), "model = \"gpt-5\"\r\n");
    }

    /// B381: Gemini CLI skips every Claude Code name but Notification, so
    /// its hooks go under its own; a refresh of an older registration takes
    /// ours out of the names it no longer uses, and leaves the person's.
    #[test]
    fn gemini_clis_hooks_go_under_its_own_names_and_old_ones_come_out() {
        let gemini = HookAgent::named("gemini").unwrap();
        let fresh = merged("", &gemini).unwrap();
        for event in ["BeforeAgent", "BeforeTool", "Notification", "AfterAgent"] {
            assert_eq!(commands(&fresh, event).len(), 1, "{event}: {fresh}");
        }
        assert!(!fresh.contains("UserPromptSubmit") && !fresh.contains("\"Stop\""));
        assert_eq!(
            commands(&fresh, "BeforeAgent"),
            ["sh ~/.moshpit/moshpit-stamp.sh working gemini"]
        );
        // An older build's registration, with a hook of the person's beside
        // ours under Stop.
        let older = merged_json("", "claude")
            .unwrap()
            .replace(" claude", " gemini")
            .replace(
                "\"type\" : \"command\"\n          }\n        ]\n      }\n    ],\n    \"UserPromptSubmit\"",
                "\"type\" : \"command\"\n          },\n          {\n            \"command\" : \"say done\",\n            \"type\" : \"command\"\n          }\n        ]\n      }\n    ],\n    \"UserPromptSubmit\"",
            );
        assert!(older.contains("say done"), "{older}");
        let refreshed = merged(&older, &gemini).unwrap();
        assert!(!refreshed.contains("UserPromptSubmit"), "{refreshed}");
        assert!(!refreshed.contains("PreToolUse"), "{refreshed}");
        assert_eq!(commands(&refreshed, "Stop"), ["say done"], "{refreshed}");
        assert_eq!(commands(&refreshed, "AfterAgent").len(), 1);
    }

    /// B380: what the page's test reads back is ours, by event, in either
    /// format.
    #[test]
    fn the_registration_reads_back_by_event() {
        let claude = HookAgent::named("claude").unwrap();
        let config = merged(
            "{\"hooks\":{\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"say\"}]}]}}",
            &claude,
        )
        .unwrap();
        let found = registered(&config, &claude).unwrap();
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.contains(&(
            "Stop".to_owned(),
            "sh ~/.moshpit/moshpit-stamp.sh done claude".to_owned()
        )));
        let codex = HookAgent::named("codex").unwrap();
        let toml = merged("model = \"o3\"\n", &codex).unwrap();
        let found = registered(&toml, &codex).unwrap();
        let events: Vec<&str> = found.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(
            events,
            [
                "UserPromptSubmit",
                "PreToolUse",
                "Stop",
                "PermissionRequest"
            ]
        );
        assert!(registered("{not json", &claude).is_err());
        assert!(registered("", &claude).unwrap().is_empty());
    }

    /// B385: Factory's hooks file is the event map itself, no `hooks` key:
    /// ours go in beside the person's, come out leaving theirs, and read
    /// back.
    #[test]
    fn factorys_hooks_file_holds_the_events_at_its_top_level() {
        let factory = HookAgent::named("factory").unwrap();
        let file = "$HOME/.factory/hooks.json";
        assert_eq!(factory.format_at(file), Format::EventMapJson);
        assert_eq!(factory.format_at(factory.config_path), Format::ClaudeJson);
        let theirs = "{\"PreToolUse\":[{\"matcher\":\"Execute\",\"hooks\":[{\"type\":\"command\",\"command\":\"audit\"}]}]}";
        let into_file = merged_at(theirs, &factory, file).unwrap();
        let root = object(&into_file);
        assert!(root.get("hooks").is_none(), "{into_file}");
        for event in ["UserPromptSubmit", "PreToolUse", "Notification", "Stop"] {
            assert!(root.get(event).is_some(), "{event}: {into_file}");
        }
        assert!(
            into_file.contains("\"audit\""),
            "the person's hook: {into_file}"
        );
        assert_eq!(registered_at(&into_file, &factory, file).unwrap().len(), 4);
        let removed = removed_at(&into_file, &factory, file).unwrap();
        assert!(!removed.contains(MARKER), "{removed}");
        assert!(removed.contains("\"audit\""), "{removed}");
        // Its settings.json is the Claude Code shape, as before.
        assert_eq!(
            merged_at("", &factory, factory.config_path).unwrap(),
            merged("", &factory).unwrap()
        );
    }

    #[test]
    fn codex_declares_its_trust_step_claude_has_none() {
        let step = HookAgent::named("codex").unwrap().trust_step().unwrap();
        assert!(step.contains("/hooks"));
        assert_eq!(HookAgent::named("claude").unwrap().trust_step(), None);
        assert_eq!(HookAgent::named("gemini").unwrap().trust_step(), None);
    }

    #[test]
    fn the_agent_roster_covers_the_formats_and_keeps_claude_first() {
        assert_eq!(HookAgent::ALL[0].id, "claude");
        assert_eq!(HookAgent::named("codex").unwrap().format, Format::CodexToml);
        assert_eq!(
            HookAgent::named("gemini").unwrap().format,
            Format::ClaudeJson
        );
        assert_eq!(HookAgent::named("nope"), None);
        assert_eq!(
            HookAgent::named("claude").unwrap().config_hint(),
            "~/.claude/settings.json"
        );
    }

    /// Fix (negative-overflow): Swift crashes writing `-1e400` back.
    #[test]
    fn a_config_with_a_negative_overflow_is_refused_not_a_crash() {
        assert!(matches!(
            merged_json(r#"{"x":-1e400}"#, "claude"),
            Err(Failure::UnparseableJson(_))
        ));
        assert!(matches!(
            without_moshpit_json(r#"{"x":-1e400}"#),
            Err(Failure::UnparseableJson(_))
        ));
    }

    // The app's B106 and B110 tests, moved with the Swift they tested.

    #[test]
    fn out_of_range_either_sign_anywhere_in_the_file_is_refused_as_unreadable() {
        for config in [
            r#"{"x":-1e400}"#,
            r#"{"x":1e400}"#,
            r#"{"model":"opus","hooks":{"Stop":[{"n":-1.8e308}]}}"#,
            r#"[{"x":-1e999}]"#,
            "-1e400",
        ] {
            assert!(
                matches!(
                    merged_json(config, "claude"),
                    Err(Failure::UnparseableJson(_))
                ),
                "{config}"
            );
        }
        assert!(matches!(
            without_moshpit_json(r#"{"x":-1e400,"hooks":{}}"#),
            Err(Failure::UnparseableJson(_))
        ));
    }

    #[test]
    fn large_numbers_in_range_and_true_and_false_still_go_through() {
        let merged = merged_json(r#"{"big":-1e300,"flag":true,"off":false}"#, "claude").unwrap();
        let root = object(&merged);
        assert_eq!(
            root.get("big").and_then(Value::as_swift_double),
            Some(-1e300)
        );
        assert_eq!(root.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(root.get("off"), Some(&Value::Bool(false)));
        let removed = without_moshpit_json(&merged).unwrap();
        assert_eq!(
            object(&removed).get("big").and_then(Value::as_swift_double),
            Some(-1e300)
        );
    }

    #[test]
    fn merging_keeps_the_users_groups_and_the_odd_elements_in_place_before_ours() {
        let existing = r#"{"hooks":{"Stop":["note",{"hooks":[{"type":"command","command":"mine.sh"}]},7,null,[1]]}}"#;
        let merged = merged_json(existing, "claude").unwrap();
        let stop = groups(&merged, "Stop");
        assert_eq!(stop.len(), 6);
        assert_eq!(stop[0], Value::String("note".into()));
        assert_eq!(stop[2].as_swift_int(), Some(7));
        assert_eq!(stop[3], Value::Null);
        assert_eq!(
            commands(&merged, "Stop"),
            ["mine.sh", "sh ~/.moshpit/moshpit-stamp.sh done claude"]
        );
    }

    #[test]
    fn merging_again_replaces_ours_and_removing_takes_ours_out_and_leaves_the_rest() {
        let existing =
            r#"{"hooks":{"Stop":["note",{"hooks":[{"type":"command","command":"mine.sh"}]}]}}"#;
        let once = merged_json(existing, "claude").unwrap();
        let twice = merged_json(&once, "claude").unwrap();
        assert_eq!(groups(&twice, "Stop").len(), 3, "ours stacked");
        let removed = without_moshpit_json(&twice).unwrap();
        assert!(
            !removed.contains(MARKER),
            "uninstall left Moshpit's hooks running"
        );
        let stop = groups(&removed, "Stop");
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0], Value::String("note".into()));
        assert_eq!(commands(&removed, "Stop"), ["mine.sh"]);
    }
}
