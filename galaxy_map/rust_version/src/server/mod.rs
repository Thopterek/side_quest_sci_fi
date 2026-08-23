//! The backend service.
//!
//! Exists because the desktop client talking straight to PostgreSQL does not
//! survive a second operator. Three problems, all of them real:
//!
//! 1. **Lost updates.** The client wrote every dossier column on every save, so
//!    two people editing different fields of one system would revert each other.
//!    Writes here are field level and version checked.
//! 2. **A shared singleton `settings` row.** Whoever clicked last decided what
//!    everybody had selected. Settings are now per user.
//! 3. **Credentials everywhere.** Every desktop needed the database password and
//!    a route to port 5432. Only this service does now.
//!
//! It also caps connections: a pool of N regardless of how many clients attach,
//! and change notification over SSE so clients see each other's work without
//! polling.

pub mod api;
pub mod auth;
pub mod listen;
pub mod repo;

pub use api::{router, AppState};
pub use auth::{AuthError, Caller};
pub use crate::core::patch::{PlanetRecordPatch, RecordPatch};
pub use repo::{Repo, RepoError, VaultSnapshot, VersionedSystem};
