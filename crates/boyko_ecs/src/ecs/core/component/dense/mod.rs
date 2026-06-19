//! Dense (non-fragmenting) component storage (Dense plan, D1).
//!
//! One [`DenseStore`] per dense `ComponentId` holds every instance of that type
//! across all archetypes in ONE contiguous column, keyed by `EntityId`.
//! Deletion is tombstone + free-list (live slots never move), giving the
//! determinism contract the colored physics solver depends on.
//!
//! Access is split into two view types (the SP4 structural fix):
//! [`DenseBuildView`] (`!Send`, the structural / whole-buffer surface) and
//! [`DenseSolveView`] (`Copy + Send + Sync`, per-slot `row_ptr` only — no
//! whole-buffer `&mut [T]` path).
//!
//! D1 lands the data structure + views + structural ops in ISOLATION. Routing
//! the engine's spawn/insert/remove (D2), query integration (D3), ticks + serde
//! (D4), and the physics consumer (Stage P) land later.

pub mod dense_registry;
pub mod dense_store;
pub(crate) mod live_bitmap;
pub mod views;

pub use dense_registry::DenseRegistry;
pub use dense_store::DenseStore;
pub use views::{DenseBuildView, DenseSolveView};
