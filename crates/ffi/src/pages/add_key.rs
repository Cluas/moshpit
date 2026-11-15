//! Add Key (`ui/src/add_key.rs`): the form, never a key (the private key and
//! its passphrase stay in the host's own views). The screen's types are
//! named for its own module (`AddKey`, `Method`, `Pem`), so they cross as
//! prefixed records converted into them; its small enums (how the key is
//! made, what it is) as their ids.

use moshpit_ui::add_key::{AddKey, Algorithm, Method, Pem};
use serde_json::Value;

use super::{PageError, choice};

/// The form, as the host has it (`AddKey`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AddKeyPageState {
    /// `generate`, `import` or `hardware`.
    pub method: String,
    /// `ed25519` or `rsa`.
    pub algorithm: String,
    pub require_face_id: bool,
    pub secure_enclave: bool,
    pub pem: AddKeyPem,
    /// The text fields, as the host has them: taken when `fields` moves.
    pub name: String,
    pub comment: String,
    pub public_line: String,
    pub fields: u64,
    /// Generate or Import can be pressed.
    pub can_submit: bool,
    /// A key is being made or imported.
    pub working: bool,
    /// The new key's fingerprint, once there is one.
    pub fingerprint: Option<String>,
    /// Why the key could not be made.
    pub error: Option<String>,
    /// Import Again (B781): into a key's own record, which keeps its name.
    pub again: bool,
}

/// What the host's private key editor holds, as far as the page may know
/// (`Pem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AddKeyPem {
    /// Nothing pasted yet.
    pub empty: bool,
    /// The pasted key is behind a passphrase.
    pub encrypted: bool,
}

/// One of the form's two switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddKeySwitch {
    RequireFaceId,
    SecureEnclave,
}

/// The field a picked file fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddKeyFile {
    PrivateKey,
    PublicKey,
}

/// What Add Key tells the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AddKeyEvent {
    /// Generate or Import (`generate`, `import`).
    Method {
        id: String,
    },
    /// `ed25519` or `rsa`.
    Algorithm {
        id: String,
    },
    Set {
        key: AddKeySwitch,
        on: bool,
    },
    Name {
        text: String,
    },
    Comment {
        text: String,
    },
    PublicLine {
        text: String,
    },
    /// Import File…: the host's document picker.
    ImportFile {
        target: AddKeyFile,
    },
    Submit,
    Cancel,
    /// The error alert was read.
    DismissError,
}

impl TryFrom<AddKeyPageState> for AddKey {
    type Error = PageError;

    fn try_from(s: AddKeyPageState) -> Result<Self, PageError> {
        Ok(AddKey {
            method: choice::<Method>("method", &s.method)?,
            algorithm: choice::<Algorithm>("algorithm", &s.algorithm)?,
            require_face_id: s.require_face_id,
            secure_enclave: s.secure_enclave,
            pem: Pem {
                empty: s.pem.empty,
                encrypted: s.pem.encrypted,
            },
            name: s.name,
            comment: s.comment,
            public_line: s.public_line,
            fields: s.fields,
            can_submit: s.can_submit,
            working: s.working,
            fingerprint: s.fingerprint,
            error: s.error,
            again: s.again,
        })
    }
}

impl AddKeyEvent {
    /// What Add Key told, read as it writes it; `None` for anything else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let text = || value["value"].as_str().map(str::to_owned);
        Some(match value["ev"].as_str()? {
            "method" => {
                let id = text()?;
                choice::<Method>("method", &id).ok()?;
                AddKeyEvent::Method { id }
            }
            "algorithm" => {
                let id = text()?;
                choice::<Algorithm>("algorithm", &id).ok()?;
                AddKeyEvent::Algorithm { id }
            }
            "set" => AddKeyEvent::Set {
                key: match value["key"].as_str()? {
                    "requireFaceID" => AddKeySwitch::RequireFaceId,
                    "secureEnclave" => AddKeySwitch::SecureEnclave,
                    _ => return None,
                },
                on: value["value"].as_bool()?,
            },
            "name" => AddKeyEvent::Name { text: text()? },
            "comment" => AddKeyEvent::Comment { text: text()? },
            "public-line" => AddKeyEvent::PublicLine { text: text()? },
            "import-file" => AddKeyEvent::ImportFile {
                target: match value["target"].as_str()? {
                    "private" => AddKeyFile::PrivateKey,
                    "public" => AddKeyFile::PublicKey,
                    _ => return None,
                },
            },
            "submit" => AddKeyEvent::Submit,
            "cancel" => AddKeyEvent::Cancel,
            "dismiss-error" => AddKeyEvent::DismissError,
            _ => return None,
        })
    }
}
