//! `Bundle` — a group of components to insert together when spawning an entity.
//!
//! Phase 8d Step 6 — see `docs/PHASE-8CD-INTOSYSTEM-COMMANDS-PLAN.md` §12.
//!
//! The `Bundle` trait describes a heterogeneous tuple of `Component` values
//! that the deferred-command path (`Commands::spawn(...)`) memcpy's into an
//! archetype slot at flush time. The trait's contract avoids per-spawn
//! allocations and is panic-safe via the `ManuallyDrop`-upfront pattern
//! (invariant **B4**).
//!
//! Implemented for arities `(C,)`, `(A, B)`, `(A, B, C)`, `(A, B, C, D)`
//! in Phase 8d; higher arities deferred to Phase 9.

#[allow(clippy::module_inception)]
pub mod bundle;
pub mod bundle_column_cache;
pub mod bundle_type_registry;

pub use bundle::{Bundle, BundleStaticInfo};
pub use bundle_column_cache::{BundleColumnCache, BundleColumnRecord};
pub use bundle_type_registry::{BundleTypeId, MAX_BUNDLE_TYPES};
