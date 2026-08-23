//! Everything that is not pixels.
//!
//! This module has no rendering, windowing or HTTP dependency, which is what
//! lets the astronomy be tested on its own: `cargo test` exercises the
//! coordinate transforms, the habitable-zone maths, the camera projection, the
//! orrery layout, the vault semantics and the archive parser without ever
//! opening a window.

pub mod astro;
pub mod camera;
#[cfg(feature = "auth")]
pub mod grant;
pub mod index;
pub mod model;
pub mod nasa;
pub mod patch;
pub mod orrery;
pub mod seed;
pub mod store;
pub mod vault;

pub use astro::{DistanceMode, OrbitScale, Vec3};
pub use camera::Camera;
#[cfg(feature = "auth")]
pub use grant::{Capability, Grant, Scope};
pub use index::VaultIndex;
pub use patch::{PlanetRecordPatch, RecordPatch};
pub use model::{Arm, Planet, PlanetRecord, Record, Source, System};
pub use store::{Settings, Snapshot, StoreError, VaultStore};
pub use vault::Vault;
