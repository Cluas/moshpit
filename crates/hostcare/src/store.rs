//! The phone's pairings, as the machine reaches them: the one thing it
//! touches without asking through an output — a local file, read and
//! written in place (on iOS the App Group's `push-pairings.json`, which the
//! notification service reads; its platform keeps the file's protection).

use std::collections::HashMap;

use moshpit_seal::{PushPairing, PushPairingFile, Uuid};

/// The pairings file's three operations, each keeping
/// `PushPairingFile`'s rules (read an entry at a time, carry what can't be
/// read, never write over a file that can't be read, read back after).
pub trait PairingStore: Send {
    /// The pairings this build can read, in stored order.
    fn read(&self) -> Vec<PushPairing>;
    /// Replace (or add) one connection's pairing; `true` once it reads back.
    fn upsert(&mut self, pairing: &PushPairing) -> bool;
    /// Forget one connection's pairing; `true` once nothing for it is left
    /// (also when there was nothing).
    fn remove(&mut self, connection: Uuid) -> bool;

    fn pairing(&self, connection: Uuid) -> Option<PushPairing> {
        self.read()
            .into_iter()
            .find(|p| p.connection_id == connection)
    }
}

/// The file itself (Android, tests, a host without its own store).
impl PairingStore for PushPairingFile {
    fn read(&self) -> Vec<PushPairing> {
        PushPairingFile::read(self)
    }

    fn upsert(&mut self, pairing: &PushPairing) -> bool {
        PushPairingFile::upsert(self, pairing)
    }

    fn remove(&mut self, connection: Uuid) -> bool {
        PushPairingFile::remove(self, connection)
    }
}

/// Pairings in memory, for tests: in insertion order, with a switch that
/// makes every write fail.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    pub pairings: Vec<PushPairing>,
    pub refuse_writes: bool,
    /// How many writes each connection had.
    pub writes: HashMap<Uuid, usize>,
}

impl PairingStore for MemoryStore {
    fn read(&self) -> Vec<PushPairing> {
        self.pairings.clone()
    }

    fn upsert(&mut self, pairing: &PushPairing) -> bool {
        if self.refuse_writes {
            return false;
        }
        *self.writes.entry(pairing.connection_id).or_default() += 1;
        match self
            .pairings
            .iter_mut()
            .find(|p| p.connection_id == pairing.connection_id)
        {
            Some(slot) => *slot = pairing.clone(),
            None => self.pairings.push(pairing.clone()),
        }
        true
    }

    fn remove(&mut self, connection: Uuid) -> bool {
        if self.refuse_writes {
            return false;
        }
        self.pairings.retain(|p| p.connection_id != connection);
        true
    }
}
