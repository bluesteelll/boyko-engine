//! `reflect_dogfood` — the reflection campaign's REAL-ENGINE-TYPES acceptance package.
//!
//! Deliberately empty at GATES G0 except its manifest: the manifest IS the deliverable —
//! it is the workspace's one leaf umbrella over the engine crates' non-default `reflect`
//! features (`boyko-scene/reflect`, `boyko-render/reflect`), and G0 gate 2's
//! `cargo check -p reflect-dogfood --all-targets --features reflect` is the first
//! compile that proves an engine crate's `reflect` feature resolves at all (GATES D3).
//!
//! `docs/REFLECTION-PLAN-ECS.md` EG8 fills this package with the acceptance test against
//! the real `Transform` / `Name` / `Visibility` / `GpuTransform3D` / `EmitterActive`.
//! It is deliberately NOT the package the CI Miri row names — it reaches
//! `boyko_render` → `boyko_rhi_vulkan`, and Miri cannot execute FFI (GATES D15).
