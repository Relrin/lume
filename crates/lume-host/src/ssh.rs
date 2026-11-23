//! The system's OpenSSH client, for whoever else has to run it: where it is, which
//! words it may be handed, and what its end says.
//!
//! A session started over SSH runs the client in its terminal. Something that runs it
//! out of sight, to carry a connection of its own, finds and reads it the same way.

use std::path::PathBuf;

use lume_proto::ProtoError;

pub use crate::ssh_exit::{Tail, classify};

/// The OpenSSH client of this machine: the system's own where there is one, else the
/// first on `PATH`. Nothing when the machine has none.
pub fn client() -> Option<PathBuf> {
    crate::launch::ssh_client_here()
}

/// Refuses a word that names a machine or an account to the client if the client would
/// not take it as a name: nothing at all, an option, or something with control
/// characters in it. `what` says what the word is, for the refusal.
pub fn check_word(what: &str, word: &str) -> Result<(), ProtoError> {
    crate::launch::check_word(what, word)
}
