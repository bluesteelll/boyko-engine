//! Phase 4 FIX-2 / FSC-C1 + Phase 5 C2 — the set-level residency reject + GPU
//! stamp on the **LIVE** archetype-mint funnel, exercised through the PUBLIC
//! `EcsMaster` facade (`get_or_create_archetype`), not the dead
//! `Archetype::create_by_ids`.
//!
//! `EcsMaster::get_or_create_archetype` → `ArchetypeMaster::get_or_create_archetype`
//! → `create_archetype` → `add_archetype_from_components_fallible` (the funnel
//! that loops `register_component_inplace`). Before FIX-2 the set-level reject
//! lived only in `create_by_ids` (zero non-test callers); the live path silently
//! built a corrupt mixed-residency archetype. FIX-2 added the reject after the
//! per-component walk on this path.
//!
//! Phase 5 C2: the semantic is now `GPU_RESIDENT ⇔ all-components-Gpu` — a `Gpu`
//! component alongside ANY non-Gpu component (`Cpu` OR `CpuPinned`) rejects (was
//! the narrower `Gpu + CpuPinned`-only reject). The reject string is now
//! `"must be GPU-pure"`.
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
// Phase 5 C2 — a SECOND Gpu id so a GPU-PURE multi-component archetype can be
// spawned through the public live funnel (GPU_RESIDENT ⇔ all-components-Gpu).
const LIVE_GPU_B: ComponentId = ComponentId(353);

/// A `Gpu` + `CpuPinned` mix spawned through the PUBLIC live funnel rejects
/// loudly (release-present panic). Phase 5 C2: the reject string is now
/// `"must be GPU-pure"` (the conflict is `Gpu` + any non-Gpu).
#[test]
#[should_panic(expected = "must be GPU-pure")]
fn live_facade_mixed_gpu_and_cpu_pinned_rejects() {
    component_registry::register_layout::<u32>(LIVE_GPU.0);
    component_registry::register_layout::<u32>(LIVE_PINNED.0);
    classify_component_residency(LIVE_GPU.0, ResidencyKind::Gpu);
    classify_component_residency(LIVE_PINNED.0, ResidencyKind::CpuPinned);

    let mut world = EcsMaster::new();
    // Real spawn through the public facade → the live mint funnel.
    let _ = world.get_or_create_archetype(&[LIVE_GPU, LIVE_PINNED]);
}

/// Phase 5 C2: a `Gpu` + ordinary `Cpu` mix spawned through the PUBLIC live
/// funnel now ALSO rejects (GPU-resident ⇒ all-Gpu) — it was previously
/// permitted. A mixed archetype would let the blanket query-skip silently drop a
/// `Query<&CpuComp>` over it.
#[test]
#[should_panic(expected = "must be GPU-pure")]
fn live_facade_mixed_gpu_and_cpu_rejects() {
    component_registry::register_layout::<u32>(LIVE_GPU.0);
    component_registry::register_layout::<u32>(LIVE_CPU.0);
    classify_component_residency(LIVE_GPU.0, ResidencyKind::Gpu);
    // LIVE_CPU stays at the default Cpu — the mixed signature.

    let mut world = EcsMaster::new();
    let _ = world.get_or_create_archetype(&[LIVE_GPU, LIVE_CPU]);
}

/// A GPU-pure real spawn through the public facade succeeds and the live funnel
/// stamps the GPU bit. Phase 5 C2: the signature must be all-Gpu (a `Gpu + Cpu`
/// mix now rejects, covered above), so this uses two Gpu ids. Here we only prove
/// the pure-Gpu signature does NOT trip the reject (it builds + dedups stably);
/// the stamp itself is asserted in the in-crate unit tests.
#[test]
fn live_facade_pure_gpu_builds_without_reject() {
    component_registry::register_layout::<u32>(LIVE_GPU.0);
    component_registry::register_layout::<u32>(LIVE_GPU_B.0);
    classify_component_residency(LIVE_GPU.0, ResidencyKind::Gpu);
    classify_component_residency(LIVE_GPU_B.0, ResidencyKind::Gpu);

    let mut world = EcsMaster::new();
    let id = world.get_or_create_archetype(&[LIVE_GPU, LIVE_GPU_B]);
    // Dedup is stable: re-requesting the same signature returns the same id.
    let id_again = world.get_or_create_archetype(&[LIVE_GPU, LIVE_GPU_B]);
    assert_eq!(id, id_again, "a GPU-pure signature must build and dedup stably");
}

/// A `Cpu`-only real spawn through the public facade never trips the reject —
/// the common-case path is unaffected (the 0%-gate).
#[test]
fn live_facade_cpu_only_builds_without_reject() {
    component_registry::register_layout::<u32>(LIVE_CPU.0);

    let mut world = EcsMaster::new();
    let _ = world.get_or_create_archetype(&[LIVE_CPU]);
}
