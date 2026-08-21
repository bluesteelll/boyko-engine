//! `reflect_dogfood` — the reflection campaign's REAL-ENGINE-TYPES acceptance package.
//!
//! Empty at GATES G0 except its manifest: the manifest IS the deliverable — it is the
//! workspace's one leaf umbrella over the engine crates' non-default `reflect` features
//! (`boyko-scene/reflect`, `boyko-render/reflect`), and G0 gate 2's
//! `cargo check -p reflect-dogfood --all-targets --features reflect` is the first
//! compile that proves an engine crate's `reflect` feature resolves at all (GATES D3).
//!
//! `docs/REFLECTION-PLAN-ECS.md` EG8 fills this package with the acceptance test against
//! the real `Transform` / `Name` / `Visibility` / `GpuTransform3D` / `EmitterActive`.
//! It is deliberately NOT the package the CI Miri row names — it reaches
//! `boyko_render` → `boyko_rhi_vulkan`, and Miri cannot execute FFI (GATES D15).
//!
//! # What CORE C7 added, and why it could not live anywhere else
//!
//! [`address`] carries two annotated types and two `#[inline(never)]` readers of their
//! descriptors. They exist so that **`tests/c7_cross_crate_address.rs` can read one
//! type's `TYPE_INFO` address from two different crates** — the defining one (through
//! these readers) and a consuming one (its own `<T as Reflect>::TYPE_INFO`).
//!
//! That subject is not constructible in `reflect_fixture`: it has no `src/lib.rs`, so
//! every annotated type it owns is a private item of one integration-test binary and
//! *every* read of the descriptor happens in the crate that defined it. This package is
//! the only one in the workspace that has a `lib.rs`, a `boyko-macros` edge, and a
//! `reflect` feature at once — see [`address`] for the property and what breaks without
//! it.

#[cfg(feature = "reflect")]
pub mod address;
