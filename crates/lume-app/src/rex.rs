//! The Rex servers the application shows sessions of.
//!
//! This is the one part of the application that knows Rex servers as such. To the rest,
//! a server that is connected is one more backend in the router, and a session of it on
//! show is a layout that a server owns.

mod asking;
mod attempt;
pub mod commands;
mod hosts;
pub mod installed;
mod programs;
mod remembered;
pub mod start;

pub use asking::{Asking, Attempt};
pub use hosts::{GIVEN_UP, Hosts};
