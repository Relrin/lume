//! The `lume` console command.
//!
//! Placeholder: for now it only prints its version. It will start the app, or signal a
//! running one, and host the `ctl` subcommands.

fn main() {
    println!("lume {}", env!("CARGO_PKG_VERSION"));
}
