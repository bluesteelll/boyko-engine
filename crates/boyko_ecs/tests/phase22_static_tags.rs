//! Phase 22 (Tags) — Wave 2C: STATIC ZST-tag end-to-end integration suite
//! (plan D1/D2/D7/D8, Wave 2 step 9).
//!
//! Exercises `#[derive(Component)] struct Tag;` (size-0, tick-only pool,
//! single-component Bundle emission from Waves 0-1) through the PUBLIC API
//! only: spawn / insert / remove / `With` / `Without` / `&Tag` / `Mut<Tag>` /
//! `Added` / `Changed` / hooks / observers / bundles-with-tags / hierarchy.
//! Dynamic `TagId` terms are out of scope here (they live in
//! `phase22_tags.rs` and the Wave-2B driver suite).
//!
//! # Isolation strategy
//!
//! - Every test owns its component types (derive-minted ids — no pinned
//!   slots, no collision with the slot-pinned suites) and its own
//!   `EcsMaster`, so concurrently-running tests never observe one another.
//! - Hook/observer fns are bare `unsafe fn` pointers (cannot capture), so
//!   those tests use module-level `static AtomicUsize` counters scoped to
//!   test-private component types (the Phase 14a/14b pattern).
//! - Frame-semantics tests use `Arc` probes (no shared statics), so no
//!   test-wide mutex is needed.
//!
//! # Frame semantics relied upon (pinned by Phase 10 / Bug #56)
//!
//! - A row spawned BEFORE a schedule's first frame is seen by `Added`/
//!   `Changed` on frame 1 (the first window spans the spawn tick).
//! - A DEFERRED structural change (commands issued in frame N) stamps at the
//!   frame-N apply-window tick and is observed exactly once, in frame N+1.
//! - A same-frame `Mut` write IS observed by a later-ordered reader in the
//!   same frame (W/R conflict creates the ordering edge; registration order
//!   breaks the tie — `phase10_change_detection.rs` test 2).
//! - A direct-API `get_component_mut` write between frames stamps at the
//!   current (post-apply-window) tick and is observed once, next frame.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query, With, Without};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ────────────────────────────────────────────────────────────────────────────

/// Runs one deferred-command system and returns the `Entity` it captured
/// (the `Arc<Mutex<_>>` capture idiom — system closures must be `Send`).
fn run_capturing(
    ecs: &mut EcsMaster,
    f: impl Fn(&mut Commands) -> Entity + Send + Sync + 'static,
) -> Entity {
    let slot: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&slot);
    ecs.run_system(move |mut cmds: Commands| {
        *probe.lock().expect("not poisoned") = Some(f(&mut cmds));
    });
    let captured = slot.lock().expect("not poisoned").take();
    captured.expect("the system ran and captured an entity")
}

// ════════════════════════════════════════════════════════════════════════════
// Section 1 — spawn: single-tag spawn (commands + direct), mixed bundles
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[derive(Clone, Copy)]
struct PlayerTag;

#[test]
fn spawn_single_tag_via_commands() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| cmds.spawn(PlayerTag).id());

    assert_eq!(ecs.entity_count(), 1, "single-tag spawn creates one entity");
    assert!(ecs.has_entity(entity), "the tag-only entity is live");
    assert!(
        ecs.has_component(entity, PlayerTag::component_id()),
        "the entity carries the tag's ComponentId bit"
    );
    // D4: a ZST read materializes from the dangling base — a valid &PlayerTag.
    let tag_ref = ecs
        .get_component::<PlayerTag>(entity)
        .expect("get_component::<ZST tag> returns Some for a tagged entity");
    let _materialized: PlayerTag = *tag_ref;
}

#[test]
fn spawn_single_tag_direct_equivalents() {
    let mut ecs = EcsMaster::new();
    let cid = PlayerTag::component_id();

    // All three resolution paths agree on the single-tag archetype.
    let arch_via_bundle = ecs.bundle_archetype_id_for::<PlayerTag>();
    let arch_via_ids = ecs.get_or_create_archetype(&[cid]);
    assert_eq!(
        arch_via_bundle, arch_via_ids,
        "the derive-emitted single-component Bundle resolves to the id-built archetype"
    );

    // Direct typed spawn.
    let e1 = ecs.spawn_one(arch_via_ids, PlayerTag).expect("spawn_one(ZST tag)");
    // Direct raw spawn: a ZST contributes a 0-length byte slice.
    let e2 = ecs
        .create_entity(arch_via_ids, &[(cid, &[])])
        .expect("create_entity with an empty byte payload for a ZST tag");

    assert_eq!(ecs.entity_count(), 2);
    for e in [e1, e2] {
        assert!(ecs.has_component(e, cid), "direct-spawned entity carries the tag");
        assert_eq!(
            ecs.get_entity_archetype_id(e),
            Some(arch_via_ids),
            "direct spawns land in the single-tag archetype"
        );
        assert!(
            ecs.get_component::<PlayerTag>(e).is_some(),
            "ZST tag readable through get_component on the direct path"
        );
    }
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct MixPos {
    x: u64,
    y: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct MixHp {
    hp: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct MixTagA;

#[derive(Component)]
#[derive(Clone, Copy)]
struct MixTagB;

#[derive(Bundle)]
struct MixBundle {
    pos: MixPos,
    hp: MixHp,
    a: MixTagA,
    b: MixTagB,
}

fn mix_bundle(seed: u64) -> MixBundle {
    MixBundle {
        pos: MixPos { x: seed, y: seed.wrapping_mul(3) },
        hp: MixHp { hp: seed.wrapping_add(0xDEAD_BEEF) },
        a: MixTagA,
        b: MixTagB,
    }
}

fn assert_mix_payload(ecs: &EcsMaster, e: Entity, seed: u64) {
    assert_eq!(
        ecs.get_component::<MixPos>(e).expect("MixPos present"),
        &MixPos { x: seed, y: seed.wrapping_mul(3) },
        "MixPos bytes intact around the stride-0 columns"
    );
    assert_eq!(
        ecs.get_component::<MixHp>(e).expect("MixHp present"),
        &MixHp { hp: seed.wrapping_add(0xDEAD_BEEF) },
        "MixHp bytes intact around the stride-0 columns"
    );
}

#[test]
fn spawn_mixed_bundle_with_data_and_two_tags() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| cmds.spawn(mix_bundle(7)).id());

    assert_eq!(ecs.entity_count(), 1);
    assert_mix_payload(&ecs, entity, 7);
    assert!(ecs.has_component(entity, MixTagA::component_id()), "tag A attached");
    assert!(ecs.has_component(entity, MixTagB::component_id()), "tag B attached");
}

#[test]
fn spawn_mixed_direct_equivalents() {
    let mut ecs = EcsMaster::new();

    // Direct bundle path: spawn_batch (the EcsMaster-side bundle spawner).
    // Range<u32> (not u64): spawn_batch requires an ExactSizeIterator.
    let entities = ecs
        .spawn_batch((0..3u32).map(|i| mix_bundle(100 + u64::from(i))))
        .expect("spawn_batch of a data+2-tags bundle");
    assert_eq!(entities.len(), 3);
    for (i, &e) in entities.iter().enumerate() {
        assert_mix_payload(&ecs, e, 100 + i as u64);
        assert!(ecs.has_component(e, MixTagA::component_id()));
        assert!(ecs.has_component(e, MixTagB::component_id()));
    }

    // Direct typed pair path: spawn_two with a ZST second component.
    let arch = ecs.get_or_create_archetype(&[MixPos::component_id(), MixTagA::component_id()]);
    let e = ecs
        .spawn_two(arch, MixPos { x: 5, y: 6 }, MixTagA)
        .expect("spawn_two(data, ZST tag)");
    assert_eq!(
        ecs.get_component::<MixPos>(e).expect("MixPos present"),
        &MixPos { x: 5, y: 6 }
    );
    assert!(ecs.has_component(e, MixTagA::component_id()));
    assert_eq!(ecs.entity_count(), 4);
}

// ════════════════════════════════════════════════════════════════════════════
// Section 2 — insert/remove: tag attach/detach migrations preserve data
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct KeepA {
    a: u64,
    b: u32,
    c: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct KeepB {
    d: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct AttachTag;

#[derive(Bundle)]
struct KeepBundle {
    a: KeepA,
    b: KeepB,
}

/// Distinctive bit patterns so any byte disturbance by the stride-0 machinery
/// during the row move is visible in the equality check.
const KEEP_A: KeepA = KeepA { a: 0xA5A5_5A5A_DEAD_BEEF, b: 0xC0FF_EE00, c: 0x1234_5678 };
const KEEP_B: KeepB = KeepB { d: 0x0F0F_F0F0_CAFE_BABE };

#[test]
fn insert_then_remove_tag_preserves_component_data() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(KeepBundle { a: KEEP_A, b: KEEP_B }).id()
    });
    let arch_before = ecs.get_entity_archetype_id(entity).expect("live");

    // Attach: {KeepA, KeepB} -> {KeepA, KeepB, AttachTag} (a real migration).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(AttachTag);
    });
    let arch_tagged = ecs.get_entity_archetype_id(entity).expect("live");
    assert_ne!(arch_before, arch_tagged, "tag attach migrates to a new archetype");
    assert!(ecs.has_component(entity, AttachTag::component_id()), "tag attached");
    assert_eq!(
        ecs.get_component::<KeepA>(entity).expect("KeepA survived attach"),
        &KEEP_A,
        "KeepA must survive the tag-attach migration byte-for-byte"
    );
    assert_eq!(
        ecs.get_component::<KeepB>(entity).expect("KeepB survived attach"),
        &KEEP_B,
        "KeepB must survive the tag-attach migration byte-for-byte"
    );

    // Detach: back to the original archetype, data still intact.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).remove::<AttachTag>();
    });
    assert_eq!(
        ecs.get_entity_archetype_id(entity),
        Some(arch_before),
        "tag detach returns the entity to its original archetype"
    );
    assert!(
        !ecs.has_component(entity, AttachTag::component_id()),
        "tag detached"
    );
    assert_eq!(
        ecs.get_component::<KeepA>(entity).expect("KeepA survived detach"),
        &KEEP_A,
        "KeepA must survive the tag-detach migration byte-for-byte"
    );
    assert_eq!(
        ecs.get_component::<KeepB>(entity).expect("KeepB survived detach"),
        &KEEP_B,
        "KeepB must survive the tag-detach migration byte-for-byte"
    );
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct RiPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct RiTag;

#[test]
fn tag_reinsert_in_place_keeps_archetype_and_data() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[RiPayload::component_id(), RiTag::component_id()]);
    let entity = ecs
        .spawn_two(arch, RiPayload { v: 41 }, RiTag)
        .expect("spawn data+tag");

    // Re-inserting a present tag: merged target == source ⇒ in-place fast
    // path (D8) — no archetype move, neighboring data untouched.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(RiTag);
    });

    assert_eq!(
        ecs.get_entity_archetype_id(entity),
        Some(arch),
        "re-inserting a present tag is in-place (no migration)"
    );
    assert_eq!(
        ecs.get_component::<RiPayload>(entity).expect("payload present"),
        &RiPayload { v: 41 },
        "payload untouched by the in-place tag re-insert"
    );
    assert!(ecs.has_component(entity, RiTag::component_id()), "tag still present");
}

// ════════════════════════════════════════════════════════════════════════════
// Section 3 — queries: With/Without, &Tag as QueryData, Mut<Tag>
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct QPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct QTag;

/// Mixed fixture: 3 tagged (payload 1,2,3) + 2 untagged (payload 10,20).
fn spawn_mixed_population(ecs: &mut EcsMaster) {
    let arch_tagged =
        ecs.get_or_create_archetype(&[QPayload::component_id(), QTag::component_id()]);
    let arch_plain = ecs.get_or_create_archetype(&[QPayload::component_id()]);
    for v in [1u64, 2, 3] {
        ecs.spawn_two(arch_tagged, QPayload { v }, QTag).expect("spawn tagged");
    }
    for v in [10u64, 20] {
        ecs.spawn_one(arch_plain, QPayload { v }).expect("spawn untagged");
    }
}

#[test]
fn with_without_filters_count_mixed_population() {
    let mut ecs = EcsMaster::new();
    spawn_mixed_population(&mut ecs);

    let all: u64 = ecs.query::<&QPayload, ()>().iter().map(|p| p.v).sum();
    assert_eq!(all, 36, "unfiltered query sees all 5 entities (1+2+3+10+20)");

    let tagged: u64 = ecs.query::<&QPayload, With<QTag>>().iter().map(|p| p.v).sum();
    assert_eq!(tagged, 6, "With<QTag> sees exactly the 3 tagged entities (1+2+3)");

    let untagged: u64 = ecs.query::<&QPayload, Without<QTag>>().iter().map(|p| p.v).sum();
    assert_eq!(untagged, 30, "Without<QTag> sees exactly the 2 untagged entities (10+20)");
}

#[test]
fn tag_as_query_data_materializes_zst() {
    let mut ecs = EcsMaster::new();
    spawn_mixed_population(&mut ecs);

    // `&QTag` as the data term: yields one materialized ZST ref per tagged row.
    let mut count = 0usize;
    for t in ecs.query::<&QTag, ()>().iter() {
        let _materialized: QTag = *t; // a real &QTag from the dangling base (D4)
        count += 1;
    }
    assert_eq!(count, 3, "&Tag as QueryData yields exactly the tagged rows");

    // Tuple data: the ZST term must not disturb the sibling data fetch.
    let sum: u64 = ecs
        .query::<(&QPayload, &QTag), ()>()
        .iter()
        .map(|(p, t)| {
            let _: QTag = *t;
            p.v
        })
        .sum();
    assert_eq!(sum, 6, "(&Payload, &Tag) fetches payloads of tagged rows only");
}

#[test]
fn scheduled_query_with_without_filters() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    spawn_mixed_population(&mut ecs);

    let with_sum = Arc::new(AtomicUsize::new(0));
    let without_sum = Arc::new(AtomicUsize::new(0));
    let with_probe = Arc::clone(&with_sum);
    let without_probe = Arc::clone(&without_sum);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |q: Query<&QPayload, With<QTag>>| {
        for p in &q {
            with_probe.fetch_add(p.v as usize, SEQ);
        }
    });
    builder.add_system(move |q: Query<&QPayload, Without<QTag>>| {
        for p in &q {
            without_probe.fetch_add(p.v as usize, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);
    schedule.run(&mut ecs);

    assert_eq!(with_sum.load(SEQ), 6, "scheduled With<QTag> sums the tagged payloads");
    assert_eq!(without_sum.load(SEQ), 30, "scheduled Without<QTag> sums the untagged payloads");
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct GPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct GTag;

#[test]
fn mut_tag_deref_guard_stamps_changed_tick() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[GPayload::component_id(), GTag::component_id()]);
    ecs.spawn_two(arch, GPayload { v: 1 }, GTag).expect("spawn");

    let should_write = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    let changed_seen = Arc::new(AtomicUsize::new(0));
    let should_write_w = Arc::clone(&should_write);
    let writes_w = Arc::clone(&writes);
    let changed_probe = Arc::clone(&changed_seen);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer FIRST: W on GTag. The reader below reads GTag ⇒ W/R conflict ⇒
    // ordering edge writer→reader (registration order; phase10 test-2 pattern).
    builder.add_system(move |mut q: Query<Mut<GTag>>| {
        if should_write_w.load(SEQ) {
            for mut t in &mut q {
                *t = GTag; // DerefMut through the guard stamps changed_tick
                writes_w.fetch_add(1, SEQ);
            }
        }
    });
    builder.add_system(move |q: Query<(&GPayload, &GTag), Changed<GTag>>| {
        for _ in &q {
            changed_probe.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // Frame 1 — fresh spawn lies in the first window.
    schedule.run(&mut ecs);
    assert_eq!(changed_seen.load(SEQ), 1, "frame 1: fresh tag row matches Changed<GTag>");

    // Frame 2 — idle.
    changed_seen.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(changed_seen.load(SEQ), 0, "frame 2: idle, Changed<GTag> yields zero");

    // Frame 3 — writer stamps via Mut<GTag> deref; reader (ordered after) sees it.
    should_write.store(true, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(writes.load(SEQ), 1, "frame 3: writer stamped exactly one row");
    assert_eq!(
        changed_seen.load(SEQ),
        1,
        "frame 3: Mut<Tag> deref-mut stamped the changed tick; same-frame reader sees it"
    );

    // Frame 4 — no further writes.
    should_write.store(false, SEQ);
    changed_seen.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(changed_seen.load(SEQ), 0, "frame 4: the stamp is observed exactly once");
}

// ════════════════════════════════════════════════════════════════════════════
// Section 4 — change detection: Added / Changed / swap-lockstep
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[derive(Clone, Copy)]
struct SpTag;

#[test]
fn added_tag_on_spawn_seen_exactly_once() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[SpTag::component_id()]);
    ecs.spawn_one(arch, SpTag).expect("spawn tag-only");

    let matches = Arc::new(AtomicUsize::new(0));
    let probe = Arc::clone(&matches);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |q: Query<&SpTag, Added<SpTag>>| {
        for _ in &q {
            probe.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    schedule.run(&mut ecs);
    assert_eq!(
        matches.load(SEQ),
        1,
        "frame 1: Added<Tag> matches the freshly spawned tag-only entity"
    );

    matches.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frame 2: Added<Tag> no longer matches");
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct AtPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct AtTag;

#[test]
fn added_tag_on_fresh_attach_seen_exactly_once_next_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[AtPayload::component_id()]);
    let entity = ecs.spawn_one(arch, AtPayload { v: 9 }).expect("spawn untagged");

    let should_attach = Arc::new(AtomicBool::new(false));
    let matches = Arc::new(AtomicUsize::new(0));
    let attach_probe = Arc::clone(&should_attach);
    let match_probe = Arc::clone(&matches);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut cmds: Commands| {
        if attach_probe.load(SEQ) {
            cmds.entity(entity).insert(AtTag);
        }
    });
    builder.add_system(move |q: Query<&AtPayload, Added<AtTag>>| {
        for _ in &q {
            match_probe.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // Frames 1-2 — entity is untagged; the {AtPayload, AtTag} archetype is unmatched.
    schedule.run(&mut ecs);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frames 1-2: no tag, Added<AtTag> never matches");

    // Frame 3 — the attach command is ISSUED; it applies at the frame-3
    // apply window, AFTER the reader ran ⇒ still unseen this frame.
    should_attach.store(true, SEQ);
    schedule.run(&mut ecs);
    should_attach.store(false, SEQ);
    assert_eq!(
        matches.load(SEQ),
        0,
        "frame 3: the deferred attach applies at the apply window — not visible in-frame"
    );

    // Frame 4 — the apply-window stamp lies in this frame's window: exactly once.
    schedule.run(&mut ecs);
    assert_eq!(
        matches.load(SEQ),
        1,
        "frame 4: Added<AtTag> sees the deferred fresh attach exactly once (Bug-#56 contract)"
    );

    // Frame 5 — never again.
    matches.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frame 5: the attach is not re-observed");
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct RrPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct RrTag;

#[test]
fn changed_tag_on_reinsert_seen_exactly_once_next_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let arch = ecs.get_or_create_archetype(&[RrPayload::component_id(), RrTag::component_id()]);
    let entity = ecs.spawn_two(arch, RrPayload { v: 3 }, RrTag).expect("spawn tagged");

    let should_reinsert = Arc::new(AtomicBool::new(false));
    let matches = Arc::new(AtomicUsize::new(0));
    let reinsert_probe = Arc::clone(&should_reinsert);
    let match_probe = Arc::clone(&matches);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut cmds: Commands| {
        if reinsert_probe.load(SEQ) {
            // In-place re-insert of the present tag (D8): stamps changed_tick.
            cmds.entity(entity).insert(RrTag);
        }
    });
    builder.add_system(move |q: Query<&RrPayload, Changed<RrTag>>| {
        for _ in &q {
            match_probe.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // Frame 1 — fresh spawn matches; frame 2 — idle.
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 1, "frame 1: fresh spawn matches Changed<RrTag>");
    matches.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frame 2: idle");

    // Frame 3 — re-insert issued (applies at the apply window).
    should_reinsert.store(true, SEQ);
    schedule.run(&mut ecs);
    should_reinsert.store(false, SEQ);
    assert_eq!(matches.load(SEQ), 0, "frame 3: deferred re-insert not visible in-frame");

    // Frame 4 — the in-place replace's changed stamp is seen exactly once.
    schedule.run(&mut ecs);
    assert_eq!(
        matches.load(SEQ),
        1,
        "frame 4: Changed<RrTag> sees the in-place tag re-insert exactly once"
    );
    assert_eq!(
        ecs.get_entity_archetype_id(entity),
        Some(arch),
        "the re-insert stayed in place (no migration)"
    );

    // Frame 5 — never again.
    matches.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frame 5: the re-insert is not re-observed");
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct SwapPayload {
    v: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct SwapTag;

/// The swap-lockstep pin: tag tick columns must move WITH the row when a
/// NEIGHBORING entity's swap_remove relocates it. If the stride-0 machinery
/// desynced ticks from rows, the survivor would carry the victim's stale
/// changed tick and the reader would miss (or mis-attribute) the stamp.
#[test]
fn tag_ticks_survive_neighbor_swap_remove() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let arch =
        ecs.get_or_create_archetype(&[SwapPayload::component_id(), SwapTag::component_id()]);
    // Row 0 = victim, row 1 = survivor.
    let victim = ecs.spawn_two(arch, SwapPayload { v: 1111 }, SwapTag).expect("spawn A");
    let survivor = ecs.spawn_two(arch, SwapPayload { v: 2222 }, SwapTag).expect("spawn B");

    let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |q: Query<(&SwapPayload, &SwapTag), Changed<SwapTag>>| {
        for (p, _) in &q {
            probe.lock().expect("probe lock").push(p.v);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // Frame 1 — both fresh rows match; frame 2 — idle.
    schedule.run(&mut ecs);
    {
        let mut s = seen.lock().expect("probe lock");
        assert_eq!(s.len(), 2, "frame 1: both fresh rows match Changed<SwapTag>");
        s.clear();
    }
    schedule.run(&mut ecs);
    assert!(seen.lock().expect("probe lock").is_empty(), "frame 2: idle");

    // Between frames: stamp the SURVIVOR's tag tick through the direct-API
    // guard (stamps at the current post-frame tick), then swap_remove the
    // victim — the survivor's row AND its ticks must relocate in lockstep.
    {
        let mut guard = ecs
            .get_component_mut::<SwapTag>(survivor)
            .expect("survivor is live and tagged");
        *guard = SwapTag; // deref-mut stamp
    }
    assert!(ecs.delete_entity(victim), "victim despawned (swap_remove moves the survivor)");
    assert!(ecs.has_entity(survivor), "survivor still live after the neighbor swap");
    assert_eq!(
        ecs.get_component::<SwapPayload>(survivor).expect("survivor payload"),
        &SwapPayload { v: 2222 },
        "survivor's data moved correctly by swap_remove"
    );

    // Frame 3 — exactly the survivor's stamp is observed, attributed to ITS row.
    schedule.run(&mut ecs);
    {
        let mut s = seen.lock().expect("probe lock");
        assert_eq!(
            s.as_slice(),
            &[2222],
            "frame 3: the survivor's changed stamp survived the neighbor swap_remove \
             (tick column moved in lockstep with the row)"
        );
        s.clear();
    }

    // Frame 4 — observed exactly once.
    schedule.run(&mut ecs);
    assert!(
        seen.lock().expect("probe lock").is_empty(),
        "frame 4: the stamp is not re-observed"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Section 5 — hooks + observers on static tags (all four structural sites)
// ════════════════════════════════════════════════════════════════════════════

static HK_ADD: AtomicUsize = AtomicUsize::new(0);
static HK_INSERT: AtomicUsize = AtomicUsize::new(0);
static HK_REPLACE: AtomicUsize = AtomicUsize::new(0);
static HK_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn hk_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    HK_ADD.fetch_add(1, SEQ);
}
unsafe fn hk_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    HK_INSERT.fetch_add(1, SEQ);
}
unsafe fn hk_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    HK_REPLACE.fetch_add(1, SEQ);
}
unsafe fn hk_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    HK_REMOVE.fetch_add(1, SEQ);
}

/// ZST tag with all four derive hooks (`#[component(...)]` on a unit struct).
#[derive(Component)]
#[component(on_add = hk_add, on_insert = hk_insert, on_replace = hk_replace, on_remove = hk_remove)]
#[derive(Clone, Copy)]
struct HkTag;

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct HkPayload(u64);

#[test]
fn hooks_fire_on_all_four_static_tag_sites() {
    let mut ecs = EcsMaster::new();

    // Site 1 — DIRECT spawn-with-tag: on_add + on_insert.
    let arch_tag = ecs.get_or_create_archetype(&[HkTag::component_id()]);
    let tag_only = ecs.spawn_one(arch_tag, HkTag).expect("spawn tag-only");
    assert_eq!(HK_ADD.load(SEQ), 1, "direct tag spawn fires on_add once");
    assert_eq!(HK_INSERT.load(SEQ), 1, "direct tag spawn fires on_insert once");

    // Site 2 — DEFERRED insert-tag onto a data entity (migration): on_add + on_insert.
    let arch_data = ecs.get_or_create_archetype(&[HkPayload::component_id()]);
    let data_entity = ecs.spawn_one(arch_data, HkPayload(7)).expect("spawn data");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(data_entity).insert(HkTag);
    });
    assert!(ecs.has_component(data_entity, HkTag::component_id()), "tag attached");
    assert_eq!(HK_ADD.load(SEQ), 2, "tag-attach migration fires on_add");
    assert_eq!(HK_INSERT.load(SEQ), 2, "tag-attach migration fires on_insert");
    assert_eq!(HK_REPLACE.load(SEQ), 0, "no replace yet");

    // Site 3 — DEFERRED remove-tag (migration): on_replace + on_remove.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(data_entity).remove::<HkTag>();
    });
    assert!(!ecs.has_component(data_entity, HkTag::component_id()), "tag detached");
    assert_eq!(HK_REPLACE.load(SEQ), 1, "tag remove fires on_replace");
    assert_eq!(HK_REMOVE.load(SEQ), 1, "tag remove fires on_remove");

    // Site 4 — despawn-of-tagged: on_replace + on_remove for the dying tag.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(tag_only).despawn();
    });
    assert!(!ecs.has_entity(tag_only), "tagged entity despawned");
    assert_eq!(HK_REPLACE.load(SEQ), 2, "despawn of a tagged entity fires on_replace");
    assert_eq!(HK_REMOVE.load(SEQ), 2, "despawn of a tagged entity fires on_remove");
    assert_eq!(HK_ADD.load(SEQ), 2, "remove/despawn never fire on_add");
    assert_eq!(HK_INSERT.load(SEQ), 2, "remove/despawn never fire on_insert");
}

static OB_ADD: AtomicUsize = AtomicUsize::new(0);
static OB_INSERT: AtomicUsize = AtomicUsize::new(0);
static OB_REPLACE: AtomicUsize = AtomicUsize::new(0);
static OB_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn ob_add(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Add);
    OB_ADD.fetch_add(1, SEQ);
}
unsafe fn ob_insert(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Insert);
    OB_INSERT.fetch_add(1, SEQ);
}
unsafe fn ob_replace(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Replace);
    OB_REPLACE.fetch_add(1, SEQ);
}
unsafe fn ob_remove(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Remove);
    OB_REMOVE.fetch_add(1, SEQ);
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct ObTag;

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct ObPayload(u64);

#[test]
fn observers_fire_on_all_four_static_tag_sites() {
    let mut ecs = EcsMaster::new();
    // Runtime registration through the id-keyed surface — the tag's
    // ComponentId is the same currency dynamic tags use (D8 uniformity).
    let cid = ObTag::component_id();
    ecs.add_observer(ObserverKind::Add, cid, ob_add);
    ecs.add_observer(ObserverKind::Insert, cid, ob_insert);
    ecs.add_observer(ObserverKind::Replace, cid, ob_replace);
    ecs.add_observer(ObserverKind::Remove, cid, ob_remove);

    // Site 1 — DEFERRED spawn-with-tag (the historically under-wired path).
    let tag_only = run_capturing(&mut ecs, |cmds| cmds.spawn(ObTag).id());
    assert_eq!(OB_ADD.load(SEQ), 1, "deferred tag spawn fires the Add observer");
    assert_eq!(OB_INSERT.load(SEQ), 1, "deferred tag spawn fires the Insert observer");

    // Site 2 — DEFERRED insert-tag migration.
    let arch_data = ecs.get_or_create_archetype(&[ObPayload::component_id()]);
    let data_entity = ecs.spawn_one(arch_data, ObPayload(5)).expect("spawn data");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(data_entity).insert(ObTag);
    });
    assert_eq!(OB_ADD.load(SEQ), 2, "tag-attach migration fires the Add observer");
    assert_eq!(OB_INSERT.load(SEQ), 2, "tag-attach migration fires the Insert observer");
    assert_eq!(OB_REPLACE.load(SEQ), 0, "no replace yet");

    // Site 3 — DEFERRED remove-tag migration.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(data_entity).remove::<ObTag>();
    });
    assert_eq!(OB_REPLACE.load(SEQ), 1, "tag remove fires the Replace observer");
    assert_eq!(OB_REMOVE.load(SEQ), 1, "tag remove fires the Remove observer");

    // Site 4 — DIRECT despawn-of-tagged.
    assert!(ecs.delete_entity(tag_only), "despawn the tag-only entity");
    assert_eq!(OB_REPLACE.load(SEQ), 2, "despawn of tagged fires the Replace observer");
    assert_eq!(OB_REMOVE.load(SEQ), 2, "despawn of tagged fires the Remove observer");
    assert_eq!(OB_ADD.load(SEQ), 2, "remove/despawn never fire Add");
    assert_eq!(OB_INSERT.load(SEQ), 2, "remove/despawn never fire Insert");
}

static BOTH_HOOK: AtomicUsize = AtomicUsize::new(0);
static BOTH_OBS: AtomicUsize = AtomicUsize::new(0);

unsafe fn both_hook(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    BOTH_HOOK.fetch_add(1, SEQ);
}
unsafe fn both_obs(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    BOTH_OBS.fetch_add(1, SEQ);
}

/// Derive hook + runtime observer on the SAME ZST tag: both must fire on a
/// single tag spawn (the 14b layering — observer after hook — applies to
/// tags unchanged).
#[derive(Component)]
#[component(on_add = both_hook)]
#[derive(Clone, Copy)]
struct BothTag;

#[test]
fn hook_and_observer_both_fire_on_tag_spawn() {
    let mut ecs = EcsMaster::new();
    ecs.add_observer(ObserverKind::Add, BothTag::component_id(), both_obs);

    let _e = run_capturing(&mut ecs, |cmds| cmds.spawn(BothTag).id());

    assert_eq!(BOTH_HOOK.load(SEQ), 1, "the derive on_add hook fires for the tag spawn");
    assert_eq!(BOTH_OBS.load(SEQ), 1, "the runtime Add observer fires for the tag spawn");
}

// ════════════════════════════════════════════════════════════════════════════
// Section 6 — bundles with tags through the static bundle cache
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct CchPos {
    x: u64,
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct CchTagA;

#[derive(Component)]
#[derive(Clone, Copy)]
struct CchTagB;

#[derive(Bundle)]
struct CchBundle {
    pos: CchPos,
    a: CchTagA,
    b: CchTagB,
}

#[test]
fn repeat_spawn_same_tag_bundle_hits_same_archetype() {
    let mut ecs = EcsMaster::new();

    // The static cache surface itself: one BundleTypeId, a stable resolution.
    assert_eq!(<CchBundle as Bundle>::component_ids().len(), 3);
    let arch_first = ecs.bundle_archetype_id_for::<CchBundle>();
    let arch_second = ecs.bundle_archetype_id_for::<CchBundle>();
    assert_eq!(
        arch_first, arch_second,
        "warm path: the same BundleTypeId resolves to the same archetype"
    );

    // Two separate deferred spawns of the same bundle land in that archetype.
    let e1 = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(CchBundle { pos: CchPos { x: 1 }, a: CchTagA, b: CchTagB }).id()
    });
    let e2 = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(CchBundle { pos: CchPos { x: 2 }, a: CchTagA, b: CchTagB }).id()
    });
    assert_eq!(ecs.get_entity_archetype_id(e1), Some(arch_first));
    assert_eq!(
        ecs.get_entity_archetype_id(e1),
        ecs.get_entity_archetype_id(e2),
        "repeat spawns of the same tag-bearing bundle share one archetype (cache warm path)"
    );
    assert_eq!(ecs.get_component::<CchPos>(e1).expect("pos"), &CchPos { x: 1 });
    assert_eq!(ecs.get_component::<CchPos>(e2).expect("pos"), &CchPos { x: 2 });
}

#[derive(Component)]
#[derive(Clone, Copy)]
struct OnlyA;

#[derive(Component)]
#[derive(Clone, Copy)]
struct OnlyB;

#[derive(Bundle)]
struct TagsOnlyBundle {
    a: OnlyA,
    b: OnlyB,
}

#[test]
fn bundle_containing_only_tags_spawns_and_queries() {
    let mut ecs = EcsMaster::new();
    let e1 = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(TagsOnlyBundle { a: OnlyA, b: OnlyB }).id()
    });

    assert!(ecs.has_component(e1, OnlyA::component_id()), "tag A present");
    assert!(ecs.has_component(e1, OnlyB::component_id()), "tag B present");

    // The all-ZST archetype is queryable: &OnlyA data + With<OnlyB> filter.
    let count = ecs.query::<&OnlyA, With<OnlyB>>().iter().count();
    assert_eq!(count, 1, "the tags-only entity matches (&OnlyA, With<OnlyB>)");

    // Warm path holds for the all-tag bundle too.
    let e2 = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(TagsOnlyBundle { a: OnlyA, b: OnlyB }).id()
    });
    assert_eq!(
        ecs.get_entity_archetype_id(e1),
        ecs.get_entity_archetype_id(e2),
        "repeat tags-only spawns share one archetype"
    );
    assert_eq!(ecs.query::<&OnlyA, With<OnlyB>>().iter().count(), 2);
}

// ════════════════════════════════════════════════════════════════════════════
// Section 7 — hierarchy interplay: tag-only parents/children, cascade despawn
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[derive(Clone, Copy)]
struct NodeTag;

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct KidPayload {
    v: u64,
}

#[derive(Bundle)]
struct KidBundle {
    p: KidPayload,
    t: NodeTag,
}

#[test]
fn tag_only_parent_child_link() {
    let mut ecs = EcsMaster::new();
    let parent = run_capturing(&mut ecs, |cmds| cmds.spawn(NodeTag).id());
    let child = run_capturing(&mut ecs, |cmds| cmds.spawn(NodeTag).id());

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    // Both directions of the hierarchy invariant hold for tag-only entities.
    assert_eq!(
        ecs.get_component::<ChildOf>(child).map(|c| c.0),
        Some(parent),
        "tag-only child's ChildOf points at the tag-only parent"
    );
    let kids = ecs
        .get_component::<Children>(parent)
        .expect("tag-only parent gained a Children collection");
    assert_eq!(kids.as_slice(), &[child], "parent.Children holds exactly the child");

    // The ChildOf/Children migrations did not disturb the tag columns.
    assert!(ecs.has_component(parent, NodeTag::component_id()), "parent keeps its tag");
    assert!(ecs.has_component(child, NodeTag::component_id()), "child keeps its tag");
}

#[test]
fn cascade_despawn_of_tagged_children() {
    let mut ecs = EcsMaster::new();
    let parent = run_capturing(&mut ecs, |cmds| cmds.spawn(NodeTag).id());
    // One tag-only child + one mixed (data + tag) child.
    let child_tag_only = run_capturing(&mut ecs, |cmds| cmds.spawn(NodeTag).id());
    let child_mixed = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(KidBundle { p: KidPayload { v: 77 }, t: NodeTag }).id()
    });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child_tag_only);
        cmds.entity(parent).add_child(child_mixed);
    });
    assert_eq!(
        ecs.get_component::<Children>(parent)
            .expect("parent has Children")
            .as_slice()
            .len(),
        2,
        "both tagged children linked"
    );

    // Default-recursive despawn of the tag-only root cascades to BOTH children.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    assert!(!ecs.has_entity(parent), "tag-only parent despawned");
    assert!(!ecs.has_entity(child_tag_only), "tag-only child cascaded");
    assert!(!ecs.has_entity(child_mixed), "mixed (data+tag) child cascaded");
    assert_eq!(ecs.entity_count(), 0, "no entity survives the cascade");
}
