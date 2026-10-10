//! S2.5 entity-remap — the DENSE arm of the load-path remap pass.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 step 5 + §5 C4, and the Dense plan's
//! D4 serde section. `remap_loaded_entities` rewrites every saved `Entity`
//! reference inside an `#[entities]`-annotated component to its freshly-allocated
//! id. `entity_remap.rs` gates that for TABLE storage; this file gates it for
//! DENSE storage, which the pass reaches through a different traversal.
//!
//! # Why this file exists (the defect it was written against)
//!
//! `remap_loaded_entities` gathered its work from `archetype.component_ids()`.
//! A dense component has no per-archetype `ComponentPool` — its one column lives
//! in the world-global `DenseRegistry` — so no archetype walk can reach its value,
//! and the pass had no dense arm: the dense loader remapped each store's OWNER ids
//! (`saved_s2e` → fresh) but never the component's INTERIOR. The result was
//! SILENT — no counter moved, no diagnostic code was emitted, `load_world`
//! returned `Ok` — and the user had done everything right: the field carried
//! `#[entities]` and the registry had installed `map_entities_fn`. The two storage
//! kinds simply disagreed.
//!
//! # Three constraints this gate is built on, each earned by a measured failure
//!
//! 1. **The destination world is SEEDED before the load.** In a fresh destination
//!    the saved and loaded ids coincide, and a stale (un-remapped) id is
//!    indistinguishable from a correctly remapped one. `entity_remap.rs` guards
//!    its own anti-staleness assertion behind `if loaded != saved`, so in a fresh
//!    world that assertion silently skips; this file asserts UNCONDITIONALLY and
//!    buys the right to do so by spawning [`FILLER_COUNT`] entities first.
//! 2. **Both poles are asserted.** The remapped field must EQUAL the loaded
//!    subject AND must NOT equal the saved subject. A one-sided assertion cannot
//!    tell "remapped correctly" from "happened to collide".
//! 3. **The archetype control rides in the SAME load.** [`TableRef`] carries the
//!    same annotation in table storage and passes today. It is what proves a red
//!    here is the dense arm rather than the harness — the gate can tell its own
//!    failure from its own blindness.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

/// Entities spawned into the destination world BEFORE the load, so every loaded
/// id is offset off its saved id. The offset is what separates "remapped" from
/// "accidentally right" — see constraint 1 in the module header.
const FILLER_COUNT: u32 = 50;

// ── Test components ────────────────────────────────────────────────────────────

/// A POB table marker identifying a loaded entity by payload (loaded ids are
/// fresh, so the nodes cannot be matched by id).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Tag(u32);

/// The destination-world seed. Deliberately a component the save does NOT carry,
/// so the fillers occupy their own archetype and cannot confound the loaded rows.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Filler(u32);

/// The ARCHETYPE CONTROL: `#[entities]` in TABLE storage. This path works, and
/// asserting it in the same load is what makes a dense red diagnostic.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct TableRef {
    n: u32,
    #[entities]
    target: Entity,
}

/// The SUBJECT: the same `#[entities]` annotation in DENSE storage. An `Entity`
/// field classifies the type `SerializeViaFn`, so it round-trips through the
/// per-member dense decode path (`load_dense_store_via_fn`).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct DenseRef {
    n: u32,
    #[entities]
    target: Entity,
}

/// One holder entity carrying BOTH annotated components plus its identifying
/// `Tag` — so the control and the subject are remapped by the same pass over the
/// same load map.
#[derive(Bundle)]
struct Holder {
    tag: Tag,
    table_ref: TableRef,
    dense_ref: DenseRef,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Saves `world` into a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Finds the entity carrying `Tag(value)` (ids are fresh after a load, so nodes
/// are identified by their distinguishing payload).
fn entity_with_tag(world: &EcsMaster, value: u32) -> Entity {
    world
        .iter_entities()
        .find(|&e| world.get_component::<Tag>(e).map(|t| t.0) == Some(value))
        .unwrap_or_else(|| panic!("no entity carries Tag({value})"))
}

/// Builds the source world: a subject entity (`Tag(7)`) and a holder (`Tag(8)`)
/// whose TABLE and DENSE annotated fields both point at the subject's saved id.
/// Returns the world and the saved subject id.
fn build_source() -> (EcsMaster, Entity) {
    let mut src = EcsMaster::new();
    let arch_subject = src.get_or_create_archetype(&[Tag::component_id()]);
    let subject = src.spawn_one(arch_subject, Tag(7)).expect("spawn subject");

    src.run_system(move |mut cmds: Commands| {
        cmds.spawn(Holder {
            tag: Tag(8),
            table_ref: TableRef { n: 42, target: subject },
            dense_ref: DenseRef { n: 43, target: subject },
        });
    });

    // The source must hold what the load-side assertions claim it held, or a
    // green would prove nothing about the remap.
    let holder = entity_with_tag(&src, 8);
    assert_eq!(
        src.get_component::<TableRef>(holder).map(|r| r.target),
        Some(subject),
        "source table control points at the subject",
    );
    assert_eq!(
        src.get_component::<DenseRef>(holder).map(|r| r.target),
        Some(subject),
        "source dense subject points at the subject",
    );
    (src, subject)
}

/// A destination world with [`FILLER_COUNT`] entities already in it and every
/// loadable component registered (the W1 contract).
fn seeded_destination() -> EcsMaster {
    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = TableRef::component_id();
    let _ = DenseRef::component_id();

    dst.run_system(|mut cmds: Commands| {
        for i in 0..FILLER_COUNT {
            cmds.spawn(Filler(i));
        }
    });
    dst
}

// ════════════════════════════════════════════════════════════════════════════
// A dense component's #[entities] field remaps exactly like the table control
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dense_entities_field_remaps_like_the_table_control() {
    let (src, saved_subject) = build_source();
    let bytes = save(&src);

    let mut dst = seeded_destination();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load dense + table refs");

    let loaded_subject = entity_with_tag(&dst, 7);
    let loaded_holder = entity_with_tag(&dst, 8);

    // Constraint 1: the seed must actually have moved the ids apart, otherwise
    // the two-pole assertions below degenerate into one.
    assert_ne!(
        loaded_subject, saved_subject,
        "the {FILLER_COUNT}-entity seed must offset the loaded ids off the saved ids \
         (without it a stale id is indistinguishable from a remapped one)",
    );

    // ── The archetype control: passes today, and proves the harness is sound ──
    let table_ref = dst
        .get_component::<TableRef>(loaded_holder)
        .copied()
        .expect("loaded holder carries TableRef");
    assert_eq!(table_ref.n, 42, "the table control's payload round-trips");
    assert_eq!(
        table_ref.target, loaded_subject,
        "CONTROL: a table #[entities] field remaps to the LOADED subject",
    );
    assert_ne!(
        table_ref.target, saved_subject,
        "CONTROL: a table #[entities] field must be remapped OFF the saved id",
    );

    // ── The subject: the same annotation, dense storage ──────────────────────
    let dense_ref = dst
        .get_component::<DenseRef>(loaded_holder)
        .copied()
        .expect("loaded holder carries DenseRef");
    assert_eq!(dense_ref.n, 43, "the dense subject's payload round-trips");
    assert_eq!(
        dense_ref.target, loaded_subject,
        "a DENSE #[entities] field must remap to the LOADED subject, exactly as the \
         table control above does",
    );
    assert_ne!(
        dense_ref.target, saved_subject,
        "a DENSE #[entities] field must be remapped OFF the saved id",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The load report shows that BOTH remap arms ran (silence was half the defect)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn load_report_records_both_remap_arms() {
    let (src, _saved_subject) = build_source();
    let bytes = save(&src);

    let mut dst = seeded_destination();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // The dense store took the normal decode path — no skip, so a zero dense
    // remap count below could only mean the remap arm never ran.
    assert_eq!(report.dense_stores_loaded, 1, "the DenseRef store is restored");
    assert_eq!(report.dense_members_loaded, 1, "its one membership is restored");
    assert_eq!(report.dense_stores_skipped, 0, "no dense store is skipped");

    assert_eq!(
        report.remapped_table_rows, 1,
        "one TableRef row carried a remappable reference",
    );
    assert_eq!(
        report.remapped_dense_rows, 1,
        "one DenseRef slot carried a remappable reference — a 0 here is the \
         original defect: the pass returning Ok having visited no dense storage",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The dense counter tracks reality, not a constant: a table-only save reports 0
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn table_only_load_reports_no_dense_remap() {
    let mut src = EcsMaster::new();
    let arch_subject = src.get_or_create_archetype(&[Tag::component_id()]);
    let subject = src.spawn_one(arch_subject, Tag(7)).expect("spawn subject");
    let arch_ref = src.get_or_create_archetype(&[Tag::component_id(), TableRef::component_id()]);
    src.spawn_two(arch_ref, Tag(8), TableRef { n: 42, target: subject })
        .expect("spawn holder");

    let bytes = save(&src);

    let mut dst = seeded_destination();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    assert_eq!(report.remapped_table_rows, 1, "the table arm still ran");
    assert_eq!(
        report.remapped_dense_rows, 0,
        "a save with no dense component must report zero dense remaps — otherwise \
         the counter is a constant and proves nothing in the test above",
    );
}
