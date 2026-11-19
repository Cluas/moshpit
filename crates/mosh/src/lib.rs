//! The mosh client protocol for Moshpit's Rust core: a port of the app's
//! Swift mosh client (`Moshpit/Services/Mosh/`) without the parts that need
//! a platform.
//!
//! - [`ocb`]: AES-128-OCB3 (RFC 7253), the datagram cipher.
//! - [`crypto`]: mosh's datagram framing around it (`crypto.cc`): the 8-byte
//!   wire nonce, direction bit, sequence number and the two timestamps.
//! - [`compress`]: the zlib (RFC 1950) wrapper mosh puts around every
//!   transport instruction, inflated by zlib's rules (`inflate`).
//! - [`proto`] and [`wire`]: the hand-written protobuf of mosh protocol 2
//!   (transport instructions, user events, host output) and the transport
//!   fragment framing with its reassembly.
//! - [`bootstrap`]: the `mosh-server new` command line and the parsing of
//!   what it prints (`MOSH CONNECT`, key, pid).
//! - [`transport`]: the State Synchronization Protocol client as a pure
//!   state machine. Datagrams, user input and a caller-supplied clock go in;
//!   datagrams to send, host output and timer decisions come out. No sockets:
//!   the UDP loop lives in the platform layer.
//!
//! Predictive echo (`MoshPrediction.swift`) is not here: it writes the
//! terminal's cells directly and stays in Swift on iOS for now.
//!
//! Where this port deliberately differed from the Swift implementation it is
//! marked `Fix (<id>)` in the source; `harness/divergences.json` lists every
//! fix with the Swift location, the symptom, the trigger and a minimal input.
//! The Swift client took every fix but `excerpt-unicode` (each entry names
//! its Swift commit) before it was deleted (E2); this crate is the only mosh
//! client now.

pub mod bootstrap;
pub mod compress;
pub mod crypto;
#[doc(hidden)]
pub mod harness;
mod inflate;
pub mod ocb;
pub mod proto;
pub mod screen;
pub mod transport;
pub mod wire;

pub use bootstrap::{BootstrapError, Credentials};
pub use crypto::{Direction, Message, OpenError, Session};
pub use ocb::{Ocb3, OcbError};
pub use transport::{ChannelState, Diagnostics, Event, HostFrame, Transport, TransportConfig};
pub use wire::{
    Fragment, FragmentAssembler, HostInstruction, HostMessage, Instruction, UserEvent, WireError,
};
