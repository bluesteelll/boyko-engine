//! Rung A0 — the host-only, ECS-native asset kernel core.
//!
//! Storage + typed handles + loading, laid out as first-class kernel
//! resources: [`Assets<T>`] is a per-asset-type [`Resource`](crate::ecs::core::resources::resource::Resource)
//! table (the same Principle-0 precedent `MeshRegistry`/`MaterialRegistry`
//! document in `boyko_render` — assets are integer-indexed, never
//! pointer-addressed, so a plain `Vec`-backed table is correct), addressed
//! by the 8-byte [`Handle<T>`]. [`AssetServer`] is the path→handle intern.
//!
//! # Scope — host-only, render-agnostic
//!
//! This module wires nothing a renderer reads: no device, no upload, no
//! GPU-resident table. GPU residency (a `Material` table, textures,
//! bindless indices) lands in `boyko_render` at later rungs (A1/A2) — the
//! kernel core here cannot depend on `boyko_render` / `boyko_rhi_vulkan`,
//! by design.
//!
//! [`AssetLoader::decode`](loader::AssetLoader::decode) is the pure-CPU
//! decode half of loading (bytes → [`Asset::Cpu`]); it is `Send`-bound so a
//! later rung (A5) can dispatch it across the threadpool. The GPU-upload
//! half is dispatcher-serial by design and does not exist at this rung.
//!
//! # Storage — a safe `Vec`-slotmap, not a new VA primitive
//!
//! [`Slot<T>`](slot::Slot) is a plain `Occupied(T)` / `Vacant { .. }` enum
//! row — NOT a hand-rolled `VmColumn`-style column. A bespoke `SlotColumn`
//! primitive was rejected for this rung: dropping a `T: !Copy` through a raw
//! byte column reintroduces double-free / drop-uninit UB, whereas
//! `VmColumn` is sound only because it never drops. The enum row lets
//! Rust's own `Drop` do the right thing (only `Occupied` payloads drop) with
//! ZERO `unsafe`.
//!
//! # Render-carrier reuse caveat
//!
//! [`Assets::remove`](assets::Assets::remove) and slot reuse are implemented
//! and tested at the host level, but the render carrier planned for later
//! rungs stores only a 16-bit index (no generation) — see [`Handle`]'s doc.
//! Treat render-visible tables as append-only/live-forever until a later
//! rung carries the generation (or a remap) into the render path.

// Module name mirrors the public `Asset` trait's file; the parent module
// `asset` is the subsystem namespace (same accepted pattern as `state::state`
// / `resources::resources`).
#[allow(clippy::module_inception)]
pub mod asset;
pub mod assets;
pub mod error;
pub mod handle;
pub mod loader;
pub mod server;
pub(crate) mod slot;

pub use asset::{Asset, AssetLoadState};
pub use assets::Assets;
pub use error::AssetError;
pub use handle::Handle;
pub use loader::AssetLoader;
pub use server::AssetServer;
