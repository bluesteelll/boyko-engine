//! Transient, Copy-only scratch storage (audit Stage-0 enabler).
//!
//! [`ScratchColumn<T>`] is a `ComponentPool`-backed, POD-only scratch column
//! with the SAME BuildView/SolveView type-split as the committed
//! [`DenseStore`](crate::ecs::core::component::dense::DenseStore). It lets the
//! solver move its scratch (gather buffers, per-element solver state) off
//! `std::Vec` onto the engine's OWN storage, with a row-ptr-only parallel access
//! (no whole-buffer reborrow) — the structural fix for the SP4 colored-solve
//! data race.
//!
//! Two view types (the SP4 structural fix):
//! [`ScratchBuildView`] (`!Send`, the single-threaded refill / whole-buffer
//! surface) and [`ScratchSolveView`] (`Copy + Send + Sync`, per-element typed
//! `row_ptr(i) -> *mut T` only — no whole-buffer `&mut [T]` path).
//!
//! Stage 0 lands the primitive in ISOLATION. Routing the physics solver onto it
//! (Stage P1) does NOT touch this module.

pub mod scratch_column;
pub mod views;

pub use scratch_column::ScratchColumn;
pub use views::{ScratchBuildView, ScratchSolveView};
