//! Command lines any login shell parses the same way (B138): `sh`, dash,
//! bash, zsh — and fish, which the others' rules don't fit.
//!
//! sshd runs an exec request as `<login shell> -c <command>`. Moshpit's
//! commands are POSIX sh; fish refuses `$?` and `$$` ("$? is not the exit
//! status. In fish, please use $status."), has no `while … do … done`, and
//! inside single quotes takes `\'` and `\\` as escapes, so a quoted value
//! ending in `\` never closes its quote (fish 3.7.1 and 4.6.0). So every
//! exec request goes out as `sh -c <payload>` ([`portable`]), and every
//! quoted word keeps `'` and `\` outside its quotes ([`quote`]) — the one
//! form that reads the same in all five. `harness/shells.py` runs every
//! command line the app builds through all of them.
//!
//! The app's own copies: `Shell.quote` and `Shell.portable`
//! (`Moshpit/Services/Shell.swift`); moshpit-herdr's and moshpit-install's
//! `quote` are this one too.

/// `value` as one shell word, whatever it holds, in any login shell: wrapped
/// in single quotes, with each `'` closed, escaped and reopened (`'\''`)
/// and each `\` likewise (`'\\'`). Inside the quotes nothing is left that
/// any of the five reads specially — not `$`, a backtick, a newline or a
/// glob in POSIX shells; not `\` in fish. By `char`: `'` and `\` are one
/// byte of UTF-8 and never part of another character.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str(r"'\''"),
            '\\' => out.push_str(r"'\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `script`, a POSIX sh script, as a command line any login shell runs as
/// that script: `sh -c <script quoted>`. The login shell parses one word;
/// sh parses the rest.
pub fn portable(script: &str) -> String {
    format!("sh -c {}", quote(script))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_keeps_quotes_and_backslashes_outside() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(r"a\"), r"'a'\\''");
        assert_eq!(quote(r"\'"), r"''\\''\'''");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn portable_is_one_word_for_the_login_shell() {
        assert_eq!(
            portable(r#"herdr x; echo "rc=$?""#),
            r#"sh -c 'herdr x; echo "rc=$?"'"#
        );
    }
}
