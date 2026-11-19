//! Lume's plugins: what they are, how they are installed, and the order they load in.
//!
//! A plugin is a folder (its package) with a manifest, `plugin.toml`, and whatever files
//! the manifest names: the plugin's code, themes, pictures. Installing one copies the
//! package into the user's plugins folder, works out a hash over every file in it, shows
//! the user what the plugin may do, and writes all of that to a lock file. Whenever the
//! plugin is loaded the hash is worked out again; a package that changed since it was
//! installed is not used.
//!
//! Nothing here runs a plugin's code. This crate knows what is installed and what each
//! plugin asks for and adds.

pub mod assets;
pub mod broker;
pub mod contributions;
pub mod hash;
pub mod id;
pub mod lock;
pub mod manifest;
pub mod motion;
pub mod order;
pub mod package;
pub mod permissions;
pub mod placement;
pub mod places;
pub mod shader;
pub mod store;

pub use manifest::{MANIFEST_FILE, Manifest, Problem};
pub use store::{Installed, Prepared, Store, StoreError};
