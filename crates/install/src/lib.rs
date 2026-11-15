//! What Moshpit puts on a dev host, as pure logic: a port of the app's
//! `Moshpit/Services/Install/*` and `HostCapabilities` for the Rust core. No
//! SSH here — commands go through [`installer::HostChannel`], one method a
//! platform implements over its exec channel.
//!
//! - [`scripts`]: the stamp and push scripts and their digests — the bytes
//!   that decide whether a user's host needs a reinstall, so byte for byte
//!   the app's.
//! - [`commands`]: every command the installer sends.
//! - [`manifest`]: `~/.moshpit/manifest.json` and the components it records.
//! - [`hooks`]: the seven agents and their configs, merged and unmerged.
//! - [`installer`]: installing, verifying, proving and removing; the push
//!   conf; the host-care decisions.
//! - [`drift`]: what the host actually holds against what its manifest
//!   says, looked at in one command (B315).
//! - [`facts`], [`capabilities`]: what a host can do, from the probes'
//!   output.
//!
//! `harness/equivalence.py` compiles the app's unchanged Swift sources and
//! holds every generated script, conf, config, manifest and command to the
//! Swift bytes, over every agent and option combination. Where the port
//! deliberately differs it is marked `Fix (<id>)` and listed in
//! `harness/divergences.json`.

pub mod capabilities;
pub mod commands;
pub mod drift;
pub mod facts;
#[doc(hidden)]
pub mod harness;
pub mod hooks;
pub mod installer;
pub mod jsonedit;
pub mod manifest;
pub mod scripts;

pub use capabilities::{HostCapabilities, Multiplexer, PackageManager};
pub use facts::{AgentFacts, HostFacts};
pub use hooks::HookAgent;
pub use installer::{
    HostChannel, HostInstaller, InstallReport, InstallState, StepFailure, push_conf, push_conf_as,
};
pub use manifest::{InstallComponent, InstallManifest};
