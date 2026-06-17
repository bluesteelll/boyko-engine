//! Phase 4 FIX-2 / FSC-C1 — the set-level residency reject + GPU stamp on the
//! **LIVE** archetype-mint funnel, exercised through the PUBLIC `EcsMaster`
//! facade (`get_or_create_archetype`), not the dead `Archetype::create_by_ids`.
//!
//! `EcsMaster::get_or_create_archetype` → `ArchetypeMaster::get_or_create_archetype`
//! → `create_archetype` → `add_archetype_from_components_fallible` (the funnel
//! that loops `register_component_inplace`). Before FIX-2 the set-level
//! `saw_gpu && saw_cpu_pinned` reject lived only in `create_by_ids` (zero
//! non-test callers); the live path silently built a corrupt mixed-residency
//! archetype. FIX-2 added the reject after the per-component walk on this path.
//!
//! Disjoint component-id range 350-359 — the global write-once `RESIDENCY_CLASS`
//! table must not collide with the in-crate residency tests (330-349).

use boyko_ecs::ecs::core::component::component_registry::{
    self, ResidencyKind, classify_component_residency,
};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

const LIVE_GPU: ComponentId = ComponentId(350);
const LIVE_CPU: ComponentId = ComponentId(351);
const LIVE_PINNED: ComponentId = ComponentId(352);

/// A `Gpu` + `CpuPinned` mix spawned through the PUBLIC live funnel rejects
/// loudly (release-present panic) — the FIX-2 reject is on the live path.
#[test]
#[should_panic(expected = "residency conflict")]
fn live_facade_mixed_gpu_and_cpu_pinned_rejects() {
    component_registry::register_layout::<u32>(LIVE_GPU.0);
    component_registry::register_layout::<u32>(LIVE_PINNED.0);
    classify_component_residency(LIVE_GPU.0, ResidencyKind::Gpu);
    classify_component_residency(LIVE_PINNED.0, ResidencyKind::CpuPinned);

    let mut world = EcsMaster::new();
    // Real spawn through the public facade → the live mint funnel.
    let _ = world.get_or_create_archetype(&[LIVE_GPU, LIVE_PINNED]);
}

/// A pure-`Gpu` real spawn through the public facade succeeds and the live
/// funnel stamps the GPU bit (read back via the `pub(crate)`-adjacent
/// `archetype_master().get_archetype()` accessor + `is_gpu_resident`). The
/// stamp itself is asserted in the in-crate unit tests; here we only prove the
/// pure-Gpu signature does NOT trip the reject (it builds successfully).
#[test]
fn live_facade_pure_gpu_builds_without_reject() {
    component_registry::register_layout::<u32>(LIVE_GPU.0);
    component_registry::register_layout::<u32>(LIVE_CPU.0);
    classify_component_residency(LIVE_GPU.0, ResidencyKind::Gpu);
    // LIVE_CPU stays at the default Cpu — a Cpu component is compatible with a
    // Gpu signature (it becomes a device column in Phase 5).

    let mut world = EcsMaster::new();
    let id = world.get_or_create_archetype(&[LIVE_GPU, LIVE_CPU]);
    // Dedup is stable: re-requesting the same signature returns the same id.
    let id_again = world.get_or_create_archetype(&[LIVE_GPU, LIVE_CPU]);
    assert_eq!(id, id_again, "a Gpu signature must build and dedup stably");
}

/// A `Cpu`-only real spawn through the public facade never trips the reject —
/// the common-case path is unaffected (the 0%-gate).
#[test]
fn live_facade_cpu_only_builds_without_reject() {
    component_registry::register_layout::<u32>(LIVE_CPU.0);

    let mut world = EcsMaster::new();
    let _ = world.get_or_create_archetype(&[LIVE_CPU]);
}
