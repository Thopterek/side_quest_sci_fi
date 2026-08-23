//! Store worker for builds without the `db` feature — wasm, and
//! `--no-default-features`. Same API as [`crate::db::worker`], backed by
//! whatever [`VaultStore`](crate::core::store::VaultStore) it is handed.
//!
//! The real worker lives in `db::worker` and is written against the trait, not
//! against PostgreSQL, so this module is a re-export rather than a second
//! implementation.

#[path = "db/worker.rs"]
mod worker_impl;

pub use worker_impl::{StoreHandle, StoreRequest, StoreUpdate};
