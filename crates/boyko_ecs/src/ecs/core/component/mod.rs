pub mod component_mask;
// Module name mirrors the public `Component` trait; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod component;
pub mod component_registry;
pub mod component_pool_bundle;
// Dense (non-fragmenting) storage (Dense plan, D1). Lands in isolation: the
// `DenseStore` + views + structural ops are not yet wired into
// Commands/Query/hooks/serde (D2–D4), so the surface is legitimately unused
// outside its own tests until those stages land.
#[allow(dead_code)]
pub mod dense;
pub mod hooks;
pub mod observers;
// Transient Copy-only scratch storage (audit Stage-0 enabler). Lands in
// isolation: the `ScratchColumn` + views are not yet consumed by the physics
// solver (Stage P1), so the surface is legitimately unused outside its own
// tests until that stage lands.
#[allow(dead_code)]
pub mod scratch;
// Wave-1 foundation: the paged enable-bit storage + presence oracle land ahead
// of their consumers (archetype wiring in Wave 2, toggle API + migration in
// Wave 3). Until those waves wire the call sites, the storage surface is
// legitimately unused; the allow is removed when the consumers land.
#[allow(dead_code)]
pub(crate) mod enable;
