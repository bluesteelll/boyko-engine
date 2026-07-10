//! Rung A0 — the host-only, ECS-native asset kernel core.
//!
//! Storage + typed handles + loading, laid out as first-class kernel
//! resources: [`Assets<T>`] is a per-asset-type [`Resource`](crate::ecs::core::resources::resource::Resource)
//! table addressed by the 8-byte [`Handle<T>`]. [`AssetServer`] is the
//! path→handle intern.
//!
//! # Scope — host-only, render-agnostic
//!
//! This module wires nothing a renderer reads: no device, no upload, no
//! GPU-resident table. GPU residency (a `Material` table, textures,
//! bindless indices) lands in `boyko_render` at later rungs — the kernel
//! core here cannot depend on `boyko_render` / `boyko_rhi_vulkan`, by
//! design.
//!
//! [`AssetLoader::decode`](loader::AssetLoader::decode) is the pure-CPU
//! decode half of loading (bytes → [`Asset::Cpu`]); it is `Send`-bound so a
//! later rung can dispatch it across the threadpool. The GPU-upload half is
//! dispatcher-serial by design and does not exist at this rung.
//!
//! # Storage — the shipped `DenseStore` recipe (asset-streaming plan F1)
//!
//! `Assets<T>` is now a store-owned, standalone
//! [`ComponentPool`](crate::ecs::memory::component_pool::ComponentPool)
//! (`ComponentPool::new(id, reserve_rows)` directly — no archetype) plus an
//! occupancy `LiveBitmap` and a LIFO free-list — the identical recipe
//! `DenseStore` already ships for dense components. This **retracts** the
//! former "a hand-rolled `SlotColumn` over a raw byte column is unsound"
//! rationale: `DenseStore` already performs an occupancy-tracked,
//! exactly-once drop over exactly this kind of standalone pool, so
//! `Assets<T>` reuses it rather than a safe `Vec<Slot<T>>` slotmap. See
//! [`assets`]'s module doc and [`backing`] for the full design.
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
pub mod backing;
pub mod error;
pub mod handle;
pub mod loader;
pub mod server;
pub mod staging;

pub use asset::{Asset, AssetLoadState};
pub use assets::{Assets, RetireTicket};
pub use backing::{AssetBacking, register_asset_layout};
pub use error::AssetError;
pub use handle::Handle;
pub use loader::{AssetLoader, HasLoaders, LoaderEntry};
pub use server::AssetServer;
pub use staging::{AssetStaging, Staged};
