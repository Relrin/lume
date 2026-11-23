//! The application's ids for a server's blocks.
//!
//! The application names a session by a number. A Rex server names a block by a long
//! text, and a block belongs to a session of the server's. A block is given a number the
//! first time it is seen and keeps it for as long as its backend lives, so a view that
//! holds the number finds the same block after a reconnect.
//!
//! The numbers of two backends are not the same ones: whoever is given a number finds
//! one terminal by it, whichever server runs it.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};

use lume_proto::SessionId;

/// A block of a server, by the server's names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BlockRef {
    pub session_id: String,
    pub block_id: String,
}

/// How many bits of a number count a backend's blocks. The bits above them are the
/// backend's own.
const COUNT_BITS: u32 = 32;

pub(crate) struct Ids {
    by_block: HashMap<String, SessionId>,
    by_id: HashMap<SessionId, BlockRef>,
    last: u64,
}

impl Default for Ids {
    fn default() -> Self {
        // The keys of a hasher are the one thing at random the standard library gives.
        let own = RandomState::new().hash_one(0_u8);
        Self {
            by_block: HashMap::new(),
            by_id: HashMap::new(),
            last: (own >> COUNT_BITS) << COUNT_BITS,
        }
    }
}

impl Ids {
    /// The number of a block, given now if it had none. The session it is in is noted
    /// each time, since a block can be moved to another.
    pub(crate) fn of(&mut self, session_id: &str, block_id: &str) -> SessionId {
        let id = match self.by_block.get(block_id) {
            Some(id) => *id,
            None => {
                self.last += 1;
                // Counting up from a number never reaches zero.
                let id = SessionId::new(self.last).expect("a count upwards is not zero");
                self.by_block.insert(block_id.to_owned(), id);
                id
            }
        };
        self.by_id.insert(
            id,
            BlockRef {
                session_id: session_id.to_owned(),
                block_id: block_id.to_owned(),
            },
        );
        id
    }

    /// The number of a block that was given one.
    pub(crate) fn known(&self, block_id: &str) -> Option<SessionId> {
        self.by_block.get(block_id).copied()
    }

    /// The block a number names, if it names one.
    pub(crate) fn block(&self, id: SessionId) -> Option<&BlockRef> {
        self.by_id.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_backends_give_numbers_of_their_own() {
        let first = Ids::default().of("session:1", "block:a");
        let second = Ids::default().of("session:1", "block:a");
        assert_ne!(first, second);
    }

    #[test]
    fn a_block_keeps_its_number_and_is_found_by_it() {
        let mut ids = Ids::default();
        let a = ids.of("session:1", "block:a");
        let b = ids.of("session:1", "block:b");
        assert_ne!(a, b);
        assert_eq!(ids.of("session:1", "block:a"), a);
        assert_eq!(ids.block(b).unwrap().block_id, "block:b");

        // Moved to another session, it is the same block.
        assert_eq!(ids.of("session:2", "block:a"), a);
        assert_eq!(ids.block(a).unwrap().session_id, "session:2");
    }
}
