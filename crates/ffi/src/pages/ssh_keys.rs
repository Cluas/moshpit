//! Settings › SSH Keys (`ui/src/ssh_keys.rs`): metadata only, never a key.
//! The screen's types are named for its own module (`Key`, `Keys`,
//! `Notice`, `Avatar`), too generic for the app's, so they cross as
//! prefixed records converted into them.

use moshpit_ui::ssh_keys::{Avatar, Key, Keys, Notice};
use serde_json::Value;

/// What the SSH Keys page shows (`Keys`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshKeysPageState {
    /// The Secure Enclave's keys.
    pub device: Vec<SshKeyRow>,
    /// The others.
    pub imported: Vec<SshKeyRow>,
    /// The key the delete card asks about, while it shows.
    pub deleting: Option<String>,
    /// Why a key wasn't deleted, while the card saying so shows.
    pub notice: Option<SshKeysNotice>,
}

/// One key's metadata (`Key`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshKeyRow {
    pub id: String,
    pub name: String,
    /// The algorithm badge (`SE · P256`, `ED25519`, `RSA-4096`).
    pub badge: String,
    pub avatar: SshKeyAvatar,
    /// Unused for 90 days.
    pub stale: bool,
    /// `SHA256:…` (empty: none shown).
    pub fingerprint: String,
    /// The hosts it is bound to.
    pub hosts: Vec<String>,
    /// The month and year it was last used, for a stale key.
    pub unused_since: Option<String>,
    /// Whether it has a public key to copy or share.
    pub has_public_key: bool,
    /// The kind of key it is when the app can't sign in with it (B780).
    pub refused: Option<String>,
}

/// A key's avatar (`Avatar`): its kind, as its gradient tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SshKeyAvatar {
    SecureEnclave,
    Hardware,
    /// Imported or generated.
    Imported,
    /// An RSA key unused for 90 days.
    Gray,
}

/// A key that wasn't deleted, and why, in the host's words (`Notice`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshKeysNotice {
    pub title: String,
    pub message: String,
}

/// How Add Key opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SshKeyAddMethod {
    Generate,
    Import,
}

/// What the SSH Keys page tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SshKeysEvent {
    Done,
    /// The bar's + (Generate), or an empty group's button.
    Add {
        method: SshKeyAddMethod,
    },
    /// A key's action sheet.
    CopyPublicKey {
        id: String,
    },
    CopyFingerprint {
        id: String,
    },
    SharePublicKey {
        id: String,
    },
    /// Delete Key: the host checks the key can go and names it.
    ConfirmDelete {
        id: String,
    },
    /// The delete alert's Cancel.
    CancelDelete,
    /// The delete alert's Delete.
    Delete {
        id: String,
    },
    /// The notice's OK.
    DismissNotice,
}

impl From<SshKeysPageState> for Keys {
    fn from(s: SshKeysPageState) -> Self {
        Keys {
            device: s.device.into_iter().map(Into::into).collect(),
            imported: s.imported.into_iter().map(Into::into).collect(),
            deleting: s.deleting,
            notice: s.notice.map(|n| Notice {
                title: n.title,
                message: n.message,
            }),
        }
    }
}

impl From<SshKeyRow> for Key {
    fn from(k: SshKeyRow) -> Self {
        Key {
            id: k.id,
            name: k.name,
            badge: k.badge,
            avatar: k.avatar.into(),
            stale: k.stale,
            fingerprint: k.fingerprint,
            hosts: k.hosts,
            unused_since: k.unused_since,
            has_public_key: k.has_public_key,
            refused: k.refused,
        }
    }
}

impl From<SshKeyAvatar> for Avatar {
    fn from(avatar: SshKeyAvatar) -> Self {
        match avatar {
            SshKeyAvatar::SecureEnclave => Avatar::SecureEnclave,
            SshKeyAvatar::Hardware => Avatar::Hardware,
            SshKeyAvatar::Imported => Avatar::Imported,
            SshKeyAvatar::Gray => Avatar::Gray,
        }
    }
}

impl SshKeysEvent {
    /// What the SSH Keys page told, read as it writes it; `None` for
    /// anything else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let id = || value["id"].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "done" => SshKeysEvent::Done,
            "add" => SshKeysEvent::Add {
                method: match value.get("method").map(|m| m.as_str()) {
                    None | Some(Some("generate")) => SshKeyAddMethod::Generate,
                    Some(Some("import")) => SshKeyAddMethod::Import,
                    Some(_) => return None,
                },
            },
            "copy-public-key" => SshKeysEvent::CopyPublicKey { id: id()? },
            "copy-fingerprint" => SshKeysEvent::CopyFingerprint { id: id()? },
            "share-public-key" => SshKeysEvent::SharePublicKey { id: id()? },
            "confirm-delete" => SshKeysEvent::ConfirmDelete { id: id()? },
            "cancel-delete" => SshKeysEvent::CancelDelete,
            "delete" => SshKeysEvent::Delete { id: id()? },
            "dismiss-notice" => SshKeysEvent::DismissNotice,
            _ => return None,
        })
    }
}
