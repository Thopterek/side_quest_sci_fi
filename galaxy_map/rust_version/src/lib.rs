//! Parallax — a vault for star systems.
//!
//! Obsidian's shape applied to astronomy: a system is a note, `#tags` are
//! filters, `[[Wikilinks]]` are edges, and the 3D cube is the graph view.
//!
//! The crate is split so that the astronomy can be tested without a window:
//!
//! * [`core`] — coordinates, habitable zones, the camera, orrery layout, the
//!   vault, and the NASA archive parser. No egui, no I/O, fully unit tested.
//! * [`ui`] — egui rendering, and the one module that performs HTTP.
//!
//! Run `cargo test` to exercise `core` on its own.

pub mod core;

#[cfg(feature = "db")]
pub mod db;

/// The multi-user backend. See [`server`] for why it exists.
#[cfg(feature = "server")]
pub mod server;

/// HTTP-backed store, so the face needs no database credentials.
#[cfg(feature = "client")]
pub mod client;

/// The store worker, for builds without a database. See [`db_stub`].
#[cfg(all(feature = "gui", not(feature = "db")))]
pub mod db_stub;

#[cfg(feature = "gui")]
pub mod app;
#[cfg(feature = "gui")]
pub mod ui;
