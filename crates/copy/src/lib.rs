//! The rules Moshpit's Chinese copy keeps, as data, and the one function
//! that holds a text to them. `tests/catalogs.rs` runs every string the app
//! ships (the string catalogs, the extra-locale tables, the NSH screens'
//! translations, the notification card's Rust copy) through [`check`], so
//! `cargo test --workspace` fails on a word the owner banned or a term the
//! word list renders some other way.
//!
//! Where the rules come from:
//! - The owner's: Chinese is written as Chinese, never word for word; in the
//!   app "agent" is 智能体; Chinese never uses 钩子, 中继, 密封, 封装 or 您.
//! - The word list of the UI redesign (`docs/nsh/UI-DESIGN.md` §9, en /
//!   zh-Hans): one rendering per concept, so a screen never says 中转服务器
//!   where another says 推送服务.
//! - zh-Hant follows where the same words apply: the banned words in their
//!   traditional forms, and the §9 concepts with Taiwan's words
//!   (`scripts/gen/locales/STYLE.md`): 智能體, 狀態腳本, 推播服務, 正在連線….
//!   It is written in traditional characters, never a copy of the zh-Hans
//!   string, so a common simplified-only character is a violation there.
//!
//! A rule here is a decision about copy. Change one by changing the copy
//! with it: fix the generator's inputs (`scripts/gen/gen_xcstrings.py`,
//! `scripts/gen/locales/*.json`), regenerate, and the test says what is left.

/// Which Chinese a text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Script {
    /// zh-Hans, the word list's own language.
    Hans,
    /// zh-Hant (Taiwan).
    Hant,
}

impl Script {
    /// The catalogs' name for it.
    pub fn code(self) -> &'static str {
        match self {
            Self::Hans => "zh-Hans",
            Self::Hant => "zh-Hant",
        }
    }
}

/// One way a text breaks the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The rule's name: `banned`, `label`, or a term's [`Term::name`].
    pub rule: &'static str,
    /// What is wrong, for a person to read.
    pub detail: String,
}

/// Words no Chinese text uses, in either script.
pub const BANNED: &[&str] = &["钩子", "鉤子", "中继", "中繼", "密封", "封装", "封裝", "您"];

/// How one script renders a term.
#[derive(Debug, Clone, Copy)]
pub struct Rendering {
    /// The text says one of these (ASCII matched without case). Empty: no
    /// requirement, the English may be rendered by sense.
    pub must: &'static [&'static str],
    /// The text says none of these.
    pub never: &'static [&'static str],
    /// English words the text must not leave untranslated (whole words,
    /// without case; a word after `/`, like Codex's `/hooks` command or a
    /// path, is a name and allowed).
    pub never_en: &'static [&'static str],
}

impl Rendering {
    const FREE: Self = Self {
        must: &[],
        never: &[],
        never_en: &[],
    };
}

/// A concept of the word list, named in a text's English.
#[derive(Debug, Clone, Copy)]
pub struct Term {
    pub name: &'static str,
    /// The English words or phrases that name it: whole words, without case;
    /// one after `/` is a command or a path and does not count.
    pub en: &'static [&'static str],
    pub hans: Rendering,
    pub hant: Rendering,
}

/// The word list's concepts that a sentence can name (§9).
pub const TERMS: &[Term] = &[
    // 编码智能体: agent → 智能体, 智能體 in zh-Hant.
    Term {
        name: "agent",
        en: &["agent", "agents"],
        hans: Rendering {
            must: &["智能体"],
            never: &["代理"],
            never_en: &["agent", "agents"],
        },
        hant: Rendering {
            must: &["智能體"],
            never: &["代理"],
            never_en: &["agent", "agents"],
        },
    },
    // 状态脚本（hooks）: what the hooks are for, not what they are called.
    Term {
        name: "status scripts",
        en: &["hook", "hooks"],
        hans: Rendering {
            must: &["状态脚本"],
            never: &[],
            never_en: &["hook", "hooks"],
        },
        hant: Rendering {
            must: &["狀態腳本"],
            never: &[],
            never_en: &["hook", "hooks"],
        },
    },
    // 推送服务（relay）.
    Term {
        name: "push service",
        en: &["relay", "relays"],
        hans: Rendering {
            must: &["推送服务"],
            never: &["中转"],
            never_en: &["relay"],
        },
        hant: Rendering {
            must: &["推播服務"],
            never: &["中轉"],
            never_en: &["relay"],
        },
    },
    // 加密（sealed）.
    Term {
        name: "encrypted",
        en: &["seal", "sealed", "seals"],
        hans: Rendering {
            must: &["加密"],
            ..Rendering::FREE
        },
        hant: Rendering {
            must: &["加密"],
            ..Rendering::FREE
        },
    },
    // 在等你 (Needs you): the state, the same words in both scripts.
    Term {
        name: "needs you",
        en: &["needs you", "need you", "needs-you"],
        hans: Rendering {
            must: &["在等你"],
            never: &["需要你"],
            never_en: &[],
        },
        hant: Rendering {
            must: &["在等你"],
            never: &["需要你"],
            never_en: &[],
        },
    },
    // 快捷键栏 (the shortcut bar, which the English still calls the toolbar
    // or the terminal bar).
    Term {
        name: "shortcut bar",
        en: &["toolbar", "shortcut bar", "shortcuts bar", "terminal bar"],
        hans: Rendering {
            never: &["工具栏", "快捷栏"],
            ..Rendering::FREE
        },
        hant: Rendering {
            never: &["工具列", "快捷列"],
            ..Rendering::FREE
        },
    },
    // tmux and herdr's nouns: 会话 / 窗口 / 窗格, 工作区 / 标签页 / 窗格,
    // and 主机 for a machine.
    Term {
        name: "pane",
        en: &["pane", "panes"],
        hans: Rendering {
            must: &["窗格"],
            ..Rendering::FREE
        },
        hant: Rendering {
            must: &["窗格"],
            ..Rendering::FREE
        },
    },
    Term {
        name: "workspace",
        en: &["workspace", "workspaces"],
        hans: Rendering {
            must: &["工作区"],
            ..Rendering::FREE
        },
        hant: Rendering {
            must: &["工作區"],
            ..Rendering::FREE
        },
    },
    Term {
        name: "host",
        en: &["host", "hosts"],
        hans: Rendering {
            must: &["主机"],
            never: &["宿主"],
            never_en: &[],
        },
        hant: Rendering {
            must: &["主機"],
            never: &["宿主"],
            never_en: &[],
        },
    },
];

/// Whole texts the word list fixes: an English text that is exactly the
/// first reads exactly the second in zh-Hans and the third in zh-Hant (the
/// same concept in Taiwan's words: 儲存, 拷貝, 閒置, 連線). Case matters:
/// "Done" is the button (完成), "done" the agent's state (已完成).
pub const LABELS: &[(&str, &str, &str)] = &[
    ("Connect", "连接", "連線"),
    ("Disconnect", "断开", "中斷連線"),
    ("Reconnect", "重新连接", "重新連線"),
    ("Online", "在线", "上線"),
    ("online", "在线", "上線"),
    // The Home card's state chip for a connection that is up.
    ("live", "在线", "上線"),
    ("Offline", "离线", "離線"),
    ("offline", "离线", "離線"),
    ("Connecting", "正在连接", "正在連線"),
    ("Connecting…", "正在连接…", "正在連線…"),
    // The Home card's state chip for a connection on its way up.
    ("linking", "正在连接", "正在連線"),
    ("Reconnecting", "正在重连", "正在重新連線"),
    ("Reconnecting…", "正在重连…", "正在重新連線…"),
    ("Cancel", "取消", "取消"),
    ("Save", "保存", "儲存"),
    ("Done", "完成", "完成"),
    ("Delete", "删除", "刪除"),
    ("Copied", "已复制", "已拷貝"),
    ("Working", "工作中", "工作中"),
    ("working", "工作中", "工作中"),
    ("Idle", "空闲", "閒置"),
    ("idle", "空闲", "閒置"),
    ("done", "已完成", "已完成"),
    ("Needs you", "在等你", "在等你"),
    ("needs you", "在等你", "在等你"),
];

/// Common characters that exist only in simplified Chinese (their
/// traditional forms differ, and Taiwan never writes them). One in a zh-Hant
/// text means it was copied from zh-Hans, not written for Taiwan.
/// Characters both scripts share (端, 窗, 屏, 云, 于, 里, 台…) are not here.
pub const SIMPLIFIED_ONLY: &str = "这连网设钥务为时们个么来发对说还没会开关进应动页点击键码删输认证\
    错误载览转显线闲写脚体级种选项号让给经过现实话语读请远运边双节样历区变复杂当备处将构报态机\
    户档归账盘电视图续断换装签间长门问闭阅题颜风飞马东车专业丢两严乐习书买乱争亏产亲仅从仪价众\
    优传伤伪启终测试录条师储贴钮简获权无尽纸触验着帮记忆识别编辑类链缓络协议状递达隐软拟虚灵岛\
    锁声听";

/// Every rule `zh` (a rendering of `en` in `script`) breaks.
pub fn check(en: &str, zh: &str, script: Script) -> Vec<Violation> {
    let mut found = Vec::new();
    for word in BANNED {
        if zh.contains(word) {
            found.push(Violation {
                rule: "banned",
                detail: format!("uses {word}"),
            });
        }
    }
    let lower = zh.to_lowercase();
    for term in TERMS {
        let Some(named) = term.en.iter().find(|word| has_word(en, word)) else {
            continue;
        };
        let rendering = match script {
            Script::Hans => &term.hans,
            Script::Hant => &term.hant,
        };
        if !rendering.must.is_empty()
            && !rendering
                .must
                .iter()
                .any(|word| lower.contains(&word.to_lowercase()))
        {
            found.push(Violation {
                rule: term.name,
                detail: format!(
                    "the English says {named:?}; the Chinese needs {}",
                    rendering.must.join(" / ")
                ),
            });
        }
        for word in rendering.never {
            if zh.contains(word) {
                found.push(Violation {
                    rule: term.name,
                    detail: format!("says {word} for {named:?}"),
                });
            }
        }
        for word in rendering.never_en {
            if has_word(zh, word) {
                found.push(Violation {
                    rule: term.name,
                    detail: format!("leaves {word:?} in English"),
                });
            }
        }
    }
    if let Some((_, hans, hant)) = LABELS.iter().find(|(english, ..)| *english == en) {
        let want = match script {
            Script::Hans => hans,
            Script::Hant => hant,
        };
        if zh != *want {
            found.push(Violation {
                rule: "label",
                detail: format!("{en:?} is {want} in the word list"),
            });
        }
    }
    if script == Script::Hant {
        let simplified: String = zh
            .chars()
            .filter(|c| SIMPLIFIED_ONLY.contains(*c))
            .collect();
        if !simplified.is_empty() {
            found.push(Violation {
                rule: "simplified",
                detail: format!("simplified-only characters in zh-Hant: {simplified}"),
            });
        }
    }
    found
}

/// Does `text` hold `word` (ASCII, matched without case) as a whole word?
/// A match glued to a letter, a digit or `_` on either side is part of
/// another word ("Magenta"); one after `/`, `.` or `-` is part of a path, a
/// command or a name (`/hooks`, `~/.moshpit/hooks`, `ssh-agent`).
pub fn has_word(text: &str, word: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let word = word.to_ascii_lowercase();
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(|c| is_word(c) || matches!(c, '/' | '.' | '-'))
            && !after.is_some_and(|c| is_word(c) || c == '-')
    })
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(en: &str, zh: &str, script: Script) -> Vec<&'static str> {
        check(en, zh, script).into_iter().map(|v| v.rule).collect()
    }

    #[test]
    fn a_banned_word_is_found_in_either_script() {
        assert_eq!(rules("Thanks", "谢谢您", Script::Hans), ["banned"]);
        assert_eq!(rules("Relay", "中繼", Script::Hant)[0], "banned");
        assert!(rules("Thanks", "谢谢你", Script::Hans).is_empty());
    }

    #[test]
    fn agent_is_zhinengti_and_never_proxy() {
        assert!(rules("Send to agent", "发给智能体", Script::Hans).is_empty());
        assert_eq!(
            rules("Send to agent", "发给代理", Script::Hans),
            ["agent", "agent"]
        );
        assert_eq!(
            rules("Pick an agent", "选一个 agent", Script::Hans),
            ["agent", "agent"]
        );
        assert!(rules("Pick an agent", "選一個智能體", Script::Hant).is_empty());
        assert_eq!(
            rules("Pick an agent", "選一個代理", Script::Hant),
            ["agent", "agent"]
        );
        assert_eq!(
            rules("Pick an agent", "選一個 agent", Script::Hant),
            ["agent", "agent"]
        );
    }

    #[test]
    fn a_word_inside_another_or_after_a_slash_is_not_the_term() {
        assert!(rules("Magenta", "品红色", Script::Hans).is_empty());
        assert!(!has_word("Run /hooks inside Codex", "hooks"));
        assert!(!has_word("ssh-agent", "agent"));
        assert!(has_word("the agent.", "agent"));
        assert!(has_word("report needs-you", "needs-you"));
    }

    #[test]
    fn hooks_are_status_scripts_and_a_command_keeps_its_name() {
        let en = "Codex won't run a newly added hook until you trust it. Run /hooks inside Codex.";
        assert!(
            rules(
                en,
                "新的状态脚本要你信任后才会运行。在 Codex 里运行 /hooks。",
                Script::Hans
            )
            .is_empty()
        );
        assert_eq!(
            rules(
                en,
                "新的 hook 要你信任后才会运行。在 Codex 里运行 /hooks。",
                Script::Hans
            ),
            ["status scripts", "status scripts"]
        );
    }

    #[test]
    fn a_label_is_matched_whole_and_with_case() {
        assert!(rules("Done", "完成", Script::Hans).is_empty());
        assert!(rules("done", "已完成", Script::Hans).is_empty());
        assert_eq!(rules("done", "完成", Script::Hans), ["label"]);
        assert_eq!(rules("Disconnect", "断开连接", Script::Hans), ["label"]);
        assert!(rules("Disconnect", "中斷連線", Script::Hant).is_empty());
        assert_eq!(rules("linking", "連線中", Script::Hant), ["label"]);
    }

    #[test]
    fn zh_hant_is_written_in_traditional_characters() {
        assert!(rules("Settings", "設定", Script::Hant).is_empty());
        assert_eq!(rules("Settings", "设定", Script::Hant), ["simplified"]);
        assert!(
            rules("Settings", "设置", Script::Hans).is_empty(),
            "zh-Hant only"
        );
        assert!(
            SIMPLIFIED_ONLY.chars().all(|c| !c.is_whitespace()),
            "the line breaks are escaped"
        );
    }
}
