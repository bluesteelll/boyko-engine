//! **Gaia ballot F4 — what "loaded" means. The five insert-path mechanisms the
//! load path does NOT run, measured one by one.** RED BY DESIGN; nothing fixed here.
//!
//! # The claim under test
//!
//! [`docs/gaia/CAMPAIGN.md`](../../../docs/gaia/CAMPAIGN.md) §Owner ballots, row
//! **F4**, widened on 2026-08-29 from "hooks" to *every* insert-path mechanism:
//!
//! > (i) hooks (the loader runs none: reverse indexes absent, asset refcounts at 0);
//! > (ii) **the `requires` closure — MEASURED, not inferred: nothing on the load path
//! > adds a component the file omitted … so `query<(&Health, &Regen)>` silently skips
//! > level-authored entities while gameplay-spawned ones match**; (iii) flag initial
//! > state (bitset ids are filtered out of the loaded signature); (iv) relation
//! > reverse index; (v) asset refcounts.
//!
//! and its recommended disposition demands *"a census [that] must enumerate all five
//! mechanisms, not hooks alone"*. This file is that census, executable. Each
//! mechanism gets its own test, so a partial fix cannot turn the row green.
//!
//! **Do not fix any of this here.** F4 is an open owner ballot with three options —
//! a specified post-load fixup pass (a), the loader firing attach hooks (b), or
//! forbidding hook-dependent components in baked assets (c) — and the fix *is* the
//! answer to it. Kernel request **GK-3** is the carrier.
//!
//! # Result of the census (2026-08-30, this checkout)
//!
//! All five reproduce. Two findings the ballot's own wording does not carry:
//!
//! 1. **(v) is not an independent mechanism — it is (i) applied to the asset
//!    carriers.** `Assets::inc_ref`'s own doc names its only driver: *"the carrier
//!    ATTACH event … a `MeshHandle`/`MaterialHandle` `on_insert` hook pushes `+1`"*
//!    (`boyko_ecs` `asset/assets.rs`). No hook fires, so no `+1` is pushed. The
//!    ballot lists five mechanisms; four are independent and the fifth is a
//!    consequence of the first. That matters to option (b): firing attach hooks
//!    closes (i), (iv) and (v) at once, while (ii) and (iii) survive it untouched.
//! 2. **(ii) is narrower than "the load path has no `requires` support", and the
//!    narrower statement is the load-bearing one.** The load path *does* resolve
//!    require-ctors — `boyko_serialize`'s `construct_or_exclude` calls
//!    `boyko_ecs::ecs::core::serialize::required_ctor_in_set` and emits a
//!    `LoadColumn::Construct`. But it is reached only from `classify_column`, which
//!    runs **per column descriptor already present in the file**. So a component the
//!    file mentions with no data IS constructed, and a component the file never
//!    mentions is NEVER added. A Gaia bake authored from text produces exactly the
//!    second shape. The control test below pins both halves so no later reader
//!    "fixes" a mechanism that already works.
//!
//! # Why (ii) drives `load_archetype` directly
//!
//! `save_world` requires a live world, and a live world that carries `F4Health`
//! necessarily carries the `#[require]`d `F4Regen` (the spawn path added it), so a
//! self-save can never produce a file that omits it. The omission is what a **baker**
//! produces, and `load_archetype` is the seam a baker's loader writes through — it is
//! `pub` in `boyko_ecs::ecs::core::serialize` precisely so the file-format parser can
//! drive it, and `boyko_serialize::load::load_one_archetype` reaches it by the same
//! call. Driving it with the required id absent from `ids` is the faithful model of a
//! baked file, not a shortcut around the parser.
//!
//! Run the deferred half with:
//! `cargo test -p boyko-serialize --test gaia_f4_load_path_fixups -- --ignored`

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure, and
// the hook counters are process-global because a hook is a bare `unsafe fn` with no
// user payload. Integration test — compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::serialize::{LoadColumn, LoadEntityMap, load_archetype};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Bundle, Component, Relationship, RelationshipTarget};

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

const SEQ: Ordering = Ordering::SeqCst;

/// (i) + (v): how many times the attach hook fired. `on_insert` is the hook the
/// asset carriers use (`Assets::inc_ref`'s doc names it), so this counter stands in
/// for the `+1` a `MeshHandle` would have pushed.
static F4_ATTACH_FIRES: AtomicU32 = AtomicU32::new(0);

/// Serializes every test in this file. `F4_ATTACH_FIRES` is a PROCESS-GLOBAL (a
/// component hook is a bare `unsafe fn` with no user payload, so there is nowhere
/// else to put the counter), and the default test runner is multi-threaded — two
/// tests that each spawn an `F4Carrier` otherwise both see the other's fires.
/// MEASURED: without this the control read 2 where it asserts 1, under
/// `--all-targets` and not under `--test-threads=1`, which is exactly the shape of
/// bug that hides until CI runs it. The file-static `Mutex<()>` is this repo's
/// established pattern for a test that arms a process-global (`p4_bind.rs` names it
/// for the counting allocator and the watch-poll counters).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Takes the file lock and zeroes the hook counter. Poison is recovered rather than
/// propagated: a test that already failed must not convert every sibling into a
/// second, misleading failure.
fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    let g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    F4_ATTACH_FIRES.store(0, SEQ);
    g
}

/// The attach hook. Bare `unsafe fn` — the shape the derive accepts.
///
/// # Safety
/// Hook ABI: the engine calls this with a valid deferred world view. Touches no raw
/// pointer of its own.
unsafe fn f4_on_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    F4_ATTACH_FIRES.fetch_add(1, SEQ);
}

/// (i) / (v) — an attach-hook-bearing carrier, shaped like `MeshHandle(u32)`: a POB
/// integer whose `on_insert` hook is the ONLY thing that ever raises the asset's
/// refcount.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[component(on_insert = f4_on_insert)]
#[repr(C)]
struct F4Carrier {
    slot: u32,
}

/// (ii) — the required component. `query<(&F4Health, &F4Regen)>` is the query the
/// ballot names.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct F4Regen {
    per_sec: u32,
}

/// (ii) — the requirer.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[require(F4Regen)]
#[repr(C)]
struct F4Health {
    hp: u32,
}

/// (iii) — a `flag`: `StorageKind::Bitset`, filtered out of every archetype
/// signature and therefore out of the saved one too.
#[derive(Component, Default, Clone, Copy)]
#[component(storage = "bitset")]
#[repr(C)]
struct F4Flag;

/// **G1's `BindText` probe, structural twin.** Field-for-field the shape of
/// `boyko_ui`'s `BindText`: an `Entity`, a `ComponentId`, two `u8` field ids and a
/// `#[repr(u8)]` enum. `crates/boyko_ui/tests/g1_bind_record_serializability_probe.rs`
/// measured the real type's registry row —
/// `class=SerializeViaFn, serialize_fn=false, deserialize_fn=false,
/// map_entities_fn=false` — and this twin exists so the CONSEQUENCE of that row can
/// be measured on the load path, which `boyko_ui` cannot reach (it takes no
/// `boyko_serialize` dependency, and adding one for a probe is not this rung's call).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct F4BindRecordTwin {
    source: Entity,
    comp: boyko_ecs::ecs::identifiers::primitives::ComponentId,
    field: u8,
    field2: u8,
    template: F4Template,
}

/// The `TemplateId` stand-in — a `#[repr(u8)]` enum, one of the two fields that
/// disqualify the real `BindText` from the blit path.
#[repr(u8)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum F4Template {
    #[default]
    Value = 0,
    Ratio = 1,
}

#[derive(Bundle)]
struct F4BindRecordBundle {
    t: F4Tag,
    b: F4BindRecordTwin,
}

/// A plain POB anchor so every entity below has at least one blittable column and
/// is materialized on load.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct F4Tag {
    id: u32,
}

/// (iv) — the relation SOURCE. `Clone, Copy` so the derive classifies the FK
/// `SerializeViaFn` and the serialize-direction remap is emitted (the shape
/// `relations_serialize_remap.rs` establishes).
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = F4LikedBy)]
struct F4Likes(pub Entity);

/// (iv) — the relation TARGET: the reverse index, maintained ONLY by the source's
/// link hook.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = F4Likes, linked_despawn, retain_empty)]
struct F4LikedBy(Vec<Entity>);

#[derive(Bundle)]
struct F4CarrierBundle {
    t: F4Tag,
    c: F4Carrier,
}

#[derive(Bundle)]
struct F4HealthBundle {
    t: F4Tag,
    h: F4Health,
}

// NOTE: there is deliberately no `F4FlagBundle`. A `storage = "bitset"` component
// cannot be spawned through a bundle at all — `#[derive(Component)]` suppresses the
// single-component `Bundle` emission for it (`boyko_macros` `component.rs`, the
// `hooks.no_bundle || hooks.storage_bitset || hooks.storage_dense` gate), and putting
// one in a user `#[derive(Bundle)]` struct compiles but PANICS at spawn. That panic
// is a separate finding with its own red test —
// `crates/boyko_ecs/tests/gaia_storage_blindness.rs` — and is not what this file
// measures. Flags are raised out of band, via `EcsMaster::enable`, which is the
// sanctioned path (`clone_entity.rs` test 8 uses the same one).

#[derive(Bundle)]
struct F4LikesBundle {
    t: F4Tag,
    l: F4Likes,
}

// ─────────────────────────────── harness ────────────────────────────────

fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

fn load_into_fresh(bytes: &[u8]) -> EcsMaster {
    let mut dst = EcsMaster::new();
    load_world(&mut dst, bytes, LoadEntityPolicy::Remap).expect("load");
    dst
}

/// Spawns `bundle` through `Commands` — the gameplay path, which runs every fixup.
fn spawn<B: boyko_ecs::ecs::core::bundle::bundle::Bundle + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let b = Mutex::new(Some(bundle));
    world.run_system(move |mut cmds: Commands| {
        let b = b.lock().unwrap().take().expect("spawn closure runs once");
        *probe.lock().unwrap() = Some(cmds.spawn(b).id());
    });
    sink.lock().unwrap().expect("spawned handle")
}

/// The entity carrying `F4Tag(id)` in `world` (ids are fresh after a load).
fn entity_with_tag(world: &EcsMaster, id: u32) -> Entity {
    world
        .iter_entities()
        .find(|&e| world.get_component::<F4Tag>(e).is_some_and(|t| t.id == id))
        .expect("an entity carrying the tag")
}

/// Counts `(&F4Health, &F4Regen)` matches — the ballot's own query, run for real.
fn health_and_regen_matches(world: &mut EcsMaster) -> usize {
    let n = Arc::new(Mutex::new(0usize));
    let probe = Arc::clone(&n);
    world.run_system(move |q: Query<(&F4Health, &F4Regen)>| {
        *probe.lock().unwrap() = q.into_iter().count();
    });
    *n.lock().unwrap()
}

// ───────────────────────────── the controls ─────────────────────────────

/// FLOOR for (i), (iii), (v). The SPAWN path applies every fixup. Not ignored: if
/// this reds, the ignored tests below are measuring a broken harness rather than the
/// load path, and their failure would carry no information.
#[test]
fn control_the_spawn_path_applies_every_fixup() {
    let _guard = exclusive();
    let mut world = EcsMaster::new();

    let carrier = spawn(&mut world, F4CarrierBundle { t: F4Tag { id: 1 }, c: F4Carrier { slot: 7 } });
    assert_eq!(
        F4_ATTACH_FIRES.load(SEQ),
        1,
        "control (i)/(v): the spawn path fires the attach hook exactly once — this is \
         the `+1` an Assets::inc_ref would have received"
    );
    assert!(world.get_component::<F4Carrier>(carrier).is_some());

    let hp = spawn(&mut world, F4HealthBundle { t: F4Tag { id: 2 }, h: F4Health { hp: 50 } });
    assert!(
        world.get_component::<F4Regen>(hp).is_some(),
        "control (ii): the spawn path's `requires` closure adds F4Regen, which the \
         bundle never mentioned"
    );

    let flagged = spawn(&mut world, F4CarrierBundle { t: F4Tag { id: 3 }, c: F4Carrier { slot: 0 } });
    world.enable::<F4Flag>(flagged);
    assert!(
        world.is_enabled::<F4Flag>(flagged),
        "control (iii): the flag bit is raised in the live world before any save"
    );

    assert_eq!(
        health_and_regen_matches(&mut world),
        1,
        "control (ii): the ballot's own query matches the gameplay-spawned entity"
    );
}

/// FLOOR for (ii), and an honesty guard against over-claiming. The load path DOES
/// resolve require-ctors — but only for a column the file already mentions.
///
/// This is a real `save_world` → `load_world` round trip: the saved world's entity
/// carries `F4Regen` (the spawn path added it), so the file HAS an `F4Regen` column
/// and the loaded entity gets it back. Not ignored, because a later reader must not
/// "repair" a mechanism that already works. What (ii) is actually about is the file
/// that never mentions the column at all — the test below.
#[test]
fn control_a_round_trip_preserves_a_required_component_the_file_carries() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let _ = spawn(&mut src, F4HealthBundle { t: F4Tag { id: 9 }, h: F4Health { hp: 50 } });
    let bytes = save(&src);

    let mut dst = load_into_fresh(&bytes);
    let e = entity_with_tag(&dst, 9);
    assert!(
        dst.get_component::<F4Regen>(e).is_some(),
        "a self-save carries the F4Regen COLUMN, so the round trip restores it. F4(ii) \
         is NOT 'the loader ignores requires' — it is 'the loader never adds a \
         component the file omitted', which only a BAKED file can exhibit"
    );
    assert_eq!(health_and_regen_matches(&mut dst), 1);
}

// ──────────────────── (i) hooks — the loader fires none ────────────────────

/// F4(i). A loaded entity's attach hook must fire.
///
/// Hand oracle: 1 (the control proves the identical spawn fires exactly one).
/// Observed today: 0 — `load_archetype` writes pool rows and commits them; it
/// contains no hook dispatch at all (`grep -n hook load_writer.rs` returns nothing).
#[test]
#[ignore = "deferred: Gaia ballot F4(i) — the load path runs no attach hooks. The fix \
            IS the answer to an open three-option ballot (post-load fixup pass / loader \
            fires hooks / forbid hook-dependent components in baked assets), carried by \
            kernel request GK-3. RED BY DESIGN; see this file's header"]
fn load_path_fires_attach_hooks() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let _ = spawn(&mut src, F4CarrierBundle { t: F4Tag { id: 11 }, c: F4Carrier { slot: 7 } });
    let bytes = save(&src);

    // Zero the counter AFTER the save: the spawn above fired the hook once, and
    // that fire is not what this test measures. Without this line the assertion
    // below reads the SPAWN's fire and passes vacuously — measured, not theorised.
    F4_ATTACH_FIRES.store(0, SEQ);

    let dst = load_into_fresh(&bytes);
    let e = entity_with_tag(&dst, 11);
    assert!(
        dst.get_component::<F4Carrier>(e).is_some(),
        "precondition: the carrier column round-tripped, so the hook had something to \
         fire on"
    );

    assert_eq!(
        F4_ATTACH_FIRES.load(SEQ),
        1,
        "the load path must fire the attach hook for a loaded carrier. 0 is F4(i): \
         load_archetype writes and commits pool rows with no hook dispatch anywhere in \
         it, so every hook-maintained invariant of a loaded scene is simply absent"
    );
}

// ────────── (v) asset refcounts — (i)'s consequence, measured separately ──────────

/// F4(v). The asset refcount a loaded carrier owes its asset.
///
/// Measured through the attach-hook counter rather than a refcount reader, because
/// `Assets<T>` exposes no public refcount getter — and because the counter measures
/// exactly the quantity at issue: `Assets::inc_ref`'s doc names its ONLY driver as
/// "a `MeshHandle`/`MaterialHandle` `on_insert` hook pushes `+1`". Zero fires means
/// zero `+1`s means the asset sits at refcount 0.
///
/// Consequence, recorded because this repo has already been bitten by it: an asset at
/// refcount 0 is retirable, so a mass despawn elsewhere can retire a loaded scene's
/// mesh mid-game and the scene silently stops drawing.
///
/// **This test is deliberately NOT merged into (i).** It is listed as a separate
/// mechanism by the ballot, and separating them is what makes visible that option (b)
/// — the loader fires attach hooks — closes (i), (iv) and (v) together while leaving
/// (ii) and (iii) untouched.
#[test]
#[ignore = "deferred: Gaia ballot F4(v) — a loaded carrier contributes no asset \
            refcount, because the only mechanism that contributes one is the attach \
            hook of F4(i). Same ballot, same GK-3 carrier. RED BY DESIGN; see this \
            file's header"]
fn load_path_contributes_asset_refcounts() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let _ = spawn(&mut src, F4CarrierBundle { t: F4Tag { id: 12 }, c: F4Carrier { slot: 3 } });
    let bytes = save(&src);

    // Zero the counter AFTER the save: the spawn above fired the hook once, and
    // that fire is not what this test measures. Without this line the assertion
    // below reads the SPAWN's fire and passes vacuously — measured, not theorised.
    F4_ATTACH_FIRES.store(0, SEQ);

    let _dst = load_into_fresh(&bytes);

    assert!(
        F4_ATTACH_FIRES.load(SEQ) > 0,
        "a loaded asset carrier must contribute its `+1`. 0 is F4(v): every mesh and \
         material of a loaded scene sits at refcount 0 and is retirable mid-game"
    );
}

// ───────── (ii) the requires closure — nothing adds an omitted component ─────────

/// F4(ii), the ballot's headline: *"`query<(&Health, &Regen)>` silently skips
/// level-authored entities while gameplay-spawned ones match"*, asserted as exactly
/// that — one entity from each path, one query over both.
///
/// The baked file is modelled by driving `load_archetype` with `F4Regen` absent from
/// `ids` (see this file's header for why that is faithful and not a shortcut).
///
/// Hand oracle: 2. Observed today: 1 — the gameplay entity matches, the loaded one
/// does not, and nothing anywhere says so.
#[test]
#[ignore = "deferred: Gaia ballot F4(ii) — nothing on the load path adds a component \
            the file omitted, and RequiredCtor being an `unsafe fn(*mut u8)` makes the \
            ctor form unbakeable in principle. The fix IS the answer to the open \
            ballot; carried by GK-3. RED BY DESIGN; see this file's header"]
fn load_path_applies_the_requires_closure() {
    let _guard = exclusive();
    let mut world = EcsMaster::new();

    // The gameplay half: spawned, so `requires` ran.
    let _gameplay = spawn(&mut world, F4HealthBundle { t: F4Tag { id: 21 }, h: F4Health { hp: 40 } });

    // The level-authored half: one archetype written straight through the loader's
    // own writer, with F4Health and F4Tag present and F4Regen ABSENT — the shape a
    // Gaia bake of `entity { F4Tag id=22  F4Health hp=40 }` produces.
    let tag_bytes = 22u32.to_le_bytes();
    let health_bytes = 40u32.to_le_bytes();
    let mut ids = [F4Tag::component_id(), F4Health::component_id()];
    ids.sort_by_key(|c| c.0);
    let mut columns: Vec<LoadColumn<'_>> = Vec::new();
    for id in ids {
        if id == F4Tag::component_id() {
            columns.push(LoadColumn::Blit { component_id: id, bytes: &tag_bytes });
        } else {
            columns.push(LoadColumn::Blit { component_id: id, bytes: &health_bytes });
        }
    }
    let mut map = LoadEntityMap::new();
    let written = load_archetype(&mut world, &ids, &columns, &[EntityId(1000)], &mut map)
        .expect("the writer accepts a well-formed single-row archetype");
    assert_eq!(written, 1, "precondition: exactly one level-authored row landed");

    let authored = entity_with_tag(&world, 22);
    assert!(
        world.get_component::<F4Health>(authored).is_some(),
        "precondition: the level-authored entity really carries F4Health, so the \
         `requires` edge from F4Health to F4Regen is live for it"
    );

    assert_eq!(
        health_and_regen_matches(&mut world),
        2,
        "the ballot's own query must match BOTH the gameplay-spawned entity and the \
         level-authored one. 1 is F4(ii): the load path's only requires mechanism \
         (`construct_or_exclude` → `required_ctor_in_set`) is reached per COLUMN \
         DESCRIPTOR PRESENT IN THE FILE, so a component the file never mentions is \
         never added — and the query answers a plausible wrong set in silence"
    );
}

// ───────────── (iii) flag initial state — bitset ids never reach the file ─────────────

/// F4(iii). A `flag` set on a saved entity must be set on the loaded one.
///
/// Hand oracle: `true`. Observed today: `false` — and this one fails on BOTH sides of
/// the round trip, which the ballot's wording ("bitset ids are filtered out of the
/// loaded signature") states for only the load side:
///
/// * SAVE: a bitset id is excluded from every archetype signature
///   (`Archetype::filtered_signature_mask`), so the saver never emits a column for it
///   — the bit is not in the file at all.
/// * LOAD: `load_one_archetype`'s pass-1 W1 hardening explicitly skips a
///   bitset-classified column (`bitset_skipped`), so even a forged file cannot carry
///   one in.
///
/// So flag state is not "lost on load"; it is **not representable in the format**.
/// That is a strictly larger hole than the ballot row states, and it bears directly
/// on Gaia ballot **F9** (the flags carrier: a per-entity enable-bit region in the
/// format plus one spelling, or a coded refusal) — F9 cannot be answered "the format
/// already carries it".
#[test]
#[ignore = "deferred: Gaia ballot F4(iii) — flag/bitset state is not representable in \
            the save format, on the SAVE side as well as the load side. Bears on ballot \
            F9 (the flags carrier), which is also open. RED BY DESIGN; see this file's \
            header"]
fn load_path_restores_flag_initial_state() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let flagged = spawn(&mut src, F4CarrierBundle { t: F4Tag { id: 31 }, c: F4Carrier { slot: 0 } });
    src.enable::<F4Flag>(flagged);
    assert!(
        src.is_enabled::<F4Flag>(flagged),
        "precondition: the flag is set before the save"
    );
    let bytes = save(&src);

    let dst = load_into_fresh(&bytes);
    let e = entity_with_tag(&dst, 31);
    assert!(
        dst.is_enabled::<F4Flag>(e),
        "a loaded entity must carry the flag its saved twin carried. `false` is \
         F4(iii): the bitset id is filtered out of the archetype signature, so the \
         saver never emits it and the loader would skip it if it were there"
    );
}

// ───────────── (iv) relation reverse index — never rebuilt on load ─────────────

/// F4(iv). A loaded relation's TARGET must list its loaded source.
///
/// The forward FK survives — `relations_serialize_remap.rs` is the standing gate that
/// it is remapped rather than left dangling. What does not survive is the REVERSE
/// index, because it is maintained solely by the source's link hook, and F4(i) is
/// that no hook runs. So after a load the graph is half-present: `F4Likes` answers
/// correctly, `F4LikedBy` is empty, and any traversal that walks target→sources
/// silently returns nothing.
///
/// Hand oracle: the loaded target's `F4LikedBy` contains the loaded source.
/// Observed today: it is empty. The in-tree comment at
/// `relations_serialize_remap.rs` (the note above its final `let _ = &dst.get_component::<LikedBy>(...)`)
/// already records the same fact in prose without asserting it — this test asserts it.
#[test]
#[ignore = "deferred: Gaia ballot F4(iv) — the relation reverse index is never rebuilt \
            on load, because it is maintained by the source's link hook and F4(i) is \
            that the loader fires none. Same ballot, same GK-3 carrier. RED BY DESIGN; \
            see this file's header"]
fn load_path_rebuilds_the_relation_reverse_index() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let target = spawn(&mut src, F4CarrierBundle { t: F4Tag { id: 41 }, c: F4Carrier { slot: 0 } });
    let source = spawn(&mut src, F4LikesBundle { t: F4Tag { id: 42 }, l: F4Likes(target) });
    assert!(
        src.get_component::<F4LikedBy>(target)
            .is_some_and(|r| r.0.contains(&source)),
        "precondition: the SPAWN path's link hook built the reverse index"
    );

    let bytes = save(&src);
    let dst = load_into_fresh(&bytes);
    let loaded_target = entity_with_tag(&dst, 41);
    let loaded_source = entity_with_tag(&dst, 42);

    assert!(
        dst.get_component::<F4Likes>(loaded_source)
            .is_some_and(|l| l.0 == loaded_target),
        "precondition: the forward FK round-tripped and was remapped (the standing \
         gate relations_serialize_remap.rs covers this half)"
    );
    assert!(
        dst.get_component::<F4LikedBy>(loaded_target)
            .is_some_and(|r| r.0.contains(&loaded_source)),
        "the loaded target's reverse index must list the loaded source. An empty (or \
         absent) F4LikedBy is F4(iv): the graph loads half-present — target→sources \
         traversal silently returns nothing while source→target answers correctly"
    );
}

// ───── G1's BindText probe, second half: what the classification COSTS ─────

/// **The consequence of `BindText`'s measured class, on the load path.**
///
/// `crates/boyko_ui/tests/g1_bind_record_serializability_probe.rs` measured the real
/// `BindText` registry row as `SerializeViaFn` with `serialize_fn=false`,
/// `deserialize_fn=false`, `map_entities_fn=false`. A `SerializeViaFn` class with no
/// installed codec is not a neutral fact — it selects a specific branch:
/// `classify_column`'s `if d.byte_len == 0 || !rt.has_decoder { construct_or_exclude(..) }`,
/// which with no require-ctor yields `ColumnPlan::ExcludeDefaulted` — the column is
/// **omitted from the loaded archetype and counted as `types_defaulted`**.
///
/// This test measures that on a structural twin and reports the answer. It is NOT
/// `#[ignore]`d: it pins today's behaviour rather than asserting a fix, exactly like
/// the `boyko_ui` probe it completes. What it hands G1 is a fact the gate needs
/// before it can be written: **`bake_one(binding) == save_world(hand-built world)`
/// byte-for-byte is, for a bind record today, an equality between two EMPTY column
/// sets** — the hand-built world does not save the record either. A byte-identity
/// gate over a component neither side emits is a gate that cannot fail, which is the
/// class this corpus removes on sight. G1's gate must therefore either pick a
/// component that actually round-trips, or install the codec first.
#[test]
fn g1_probe_a_bind_record_twin_does_not_survive_a_round_trip() {
    let _guard = exclusive();
    let mut src = EcsMaster::new();
    let target = spawn(&mut src, F4CarrierBundle { t: F4Tag { id: 51 }, c: F4Carrier { slot: 0 } });
    let holder = spawn(
        &mut src,
        F4BindRecordBundle {
            t: F4Tag { id: 52 },
            b: F4BindRecordTwin {
                source: target,
                comp: F4Tag::component_id(),
                field: 0,
                field2: 1,
                template: F4Template::Ratio,
            },
        },
    );
    assert!(
        src.get_component::<F4BindRecordTwin>(holder).is_some(),
        "precondition: the twin is present before the save"
    );

    let bytes = save(&src);
    let dst = load_into_fresh(&bytes);
    let loaded_holder = entity_with_tag(&dst, 52);

    let survived = dst.get_component::<F4BindRecordTwin>(loaded_holder).is_some();
    println!(
        "G1 probe — a BindText-shaped record (Entity + repr(u8) enum) survives a \
         save/load round trip: {survived}"
    );
    assert!(
        !survived,
        "MEASURED BEHAVIOUR CHANGED. This test pins that a bind-record-shaped \
         component is SILENTLY EXCLUDED by the loader today (SerializeViaFn with no \
         installed codec → classify_column → construct_or_exclude → \
         ExcludeDefaulted). If it now survives, a codec was installed and G1's \
         byte-for-byte gate has become writable over this shape — update the gate and \
         this pin together"
    );
}

