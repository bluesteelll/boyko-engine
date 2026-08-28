//! **A1 gate 8 — MIRI over the fused tick and the deferred reap**
//! (`docs/UI-PLAN-ANIMATION-A1.md` A1 gate 8; `docs/UI-PLAN-ANIMATION-DECISIONS.md` AD5).
//!
//! Like the crate's other Miri files this is an ORDINARY test binary — it is NOT
//! `#![cfg(miri)]`, so it runs natively as well and a `cargo test` filter for it
//! can never print the `running 0 tests` that a cfg-gated file prints while
//! exiting 0. It drives the systems via `EcsMaster::run_system` rather than a
//! parallel `Schedule`, because the threadpool worker-join is impractically slow
//! under Miri (the `miri_layout.rs` precedent).
//!
//! # What gate 8 must exercise, and what it must NOT
//!
//! A1's original reason for this gate named a hazard AD5's DEFERRED reap
//! **designs out** — "dense insert/remove during a frame that also iterates the
//! store". Miri passing over the shipped shape says nothing at all about that,
//! which is the classic gate whose subject stops being observable at the moment
//! the rung succeeds. The three LIVE subjects are:
//!
//! * **(i)** the retained [`UiTweenScratch`] reused across frames, with no stale
//!   `(EntityId, ComponentId)` surviving a despawn — [`retained_scratch_survives_a_despawn`];
//! * **(ii)** a **remove-then-insert of the same channel on the same entity
//!   within one frame** — the reap frees the slot and a `start_tween_*` reuses
//!   it, which is where a stale per-slot tick would leak
//!   ([`a_reaped_slot_is_reused_with_fresh_ticks`]);
//! * **(iii)** the tick's `AnyOf` fetch over FOUR dense stores at once
//!   ([`the_anyof_fetch_spans_four_dense_stores`]).
//!
//! Run (windows-gnu nightly, Tree-Borrows via `.cargo/config.toml`):
//!   `RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu cargo miri test -p boyko-ui
//!   --test miri_a1_tween`

use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::*;

use boyko_ui::animation::{
    start_tween_offset, start_tween_opacity, start_tween_scale, start_tween_tint, ui_clock_tick,
    ui_tween_reap, ui_visual_tick, UiClock, UiTweenScratch,
};
use boyko_ui::components::{
    EasingId, TweenOffset, TweenOpacity, TweenScale, TweenTint, UiVisual,
};

const FRAME: Duration = Duration::from_millis(100);

fn a1_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    world
}

/// One frame, threadpool-free: advance `Time`, then the three systems in order.
///
/// `run_system` rebuilds each system per call, so no `Changed` window is carried
/// — which costs nothing here, because neither `ui_visual_tick` nor
/// `ui_tween_reap` reads a change filter. The tick's write still stamps the
/// sink's tick; only a READER of that tick would need `Schedule::run`.
fn frame(world: &mut EcsMaster, dt: Duration) {
    world.resource_mut::<Time>().advance_with(dt);
    world.run_system(ui_clock_tick);
    world.run_system(ui_visual_tick);
    world.run_system(ui_tween_reap);
}

fn spawn_node(world: &mut EcsMaster) -> Entity {
    world.run_system(|mut cmds: Commands| cmds.spawn(UiVisual::IDENTITY).id())
}

fn live_count(world: &EcsMaster, id: ComponentId) -> usize {
    world.dense_registry().store(id).map_or(0, |s| s.live_count())
}

/// **(i)** The retained completion list is drained every frame, and an entity
/// despawned after its tween completed leaves nothing behind for a later frame
/// to replay.
///
/// # The drain is the GATE; the survival below is SMOKE
///
/// The `pending() == 0` assertions own red mutation 8 (drop the `done.clear()`
/// in `ui_tween_reap`) and are what this leg gates.
///
/// The survival half — the second entity's channel still live after three
/// frames — **cannot red, and is kept as a smoke assertion only.** MEASURED
/// 2026-08-27, two independent reasons, either of which alone is sufficient:
/// a despawn removes the entity's dense rows ITSELF (a channel's live count went
/// 1 → 0 across a despawn, no reap involved), so a replayed stale pair finds no
/// slot; and this kernel does not recycle `EntityId` at all, so there is no id
/// under which that could be undone. No `live_count` assertion over a stale
/// replay can fail while both hold. The property that actually protects the
/// scratch's generation-free key is carried by
/// [`entity_ids_are_not_recycled_today`], which is where it can fail.
#[test]
fn retained_scratch_survives_a_despawn() {
    let mut world = a1_world();
    let a = spawn_node(&mut world);
    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, a, 0x0000_0000, 0xFFFF_FFFF, 100.0, EasingId::LINEAR, 0);
    });

    frame(&mut world, FRAME); // completes + reaps
    assert_eq!(live_count(&world, TweenTint::component_id()), 0);
    assert_eq!(world.resource::<UiTweenScratch>().pending(), 0, "drained in its own frame");

    world.run_system(move |mut cmds: Commands| cmds.despawn(a));

    // This kernel hands out fresh ids (measured); see
    // `entity_ids_are_not_recycled_today`, which is what makes the bare key safe
    // and which reds if that changes.
    let b = spawn_node(&mut world);
    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, b, 0x0000_0000, 0xFFFF_FFFF, 10_000.0, EasingId::LINEAR, 0);
    });
    for _ in 0..3 {
        frame(&mut world, FRAME);
        assert_eq!(world.resource::<UiTweenScratch>().pending(), 0);
    }
    // The queued pair named `TweenTint` (the channel `a` completed above), so the
    // watched store must be `TweenTint` too — watching `TweenOpacity` here was
    // looking at a store no stale pair could ever have named.
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "smoke: the long tween on the channel the stale pair NAMED is untouched"
    );
    assert!(
        world.get_component::<TweenTint>(b).is_some(),
        "smoke: and it is on `b` — the pair's id, had it been replayed onto a recycled slot, is \
         the one this row would have lost"
    );
}

/// **The property A1's generation-free scratch key rests on.**
///
/// Not a Miri subject — it lives here because this is the file whose leg (i)
/// depends on it. If this ever reds, `UiTweenScratch`'s bare `EntityId` key stops
/// being safe on the day it reds, and `ui_tween_reap` must carry the generation.
///
/// # This is a KERNEL-PROPERTY gate, and it owns no mutation ON PURPOSE
///
/// Do not "fix" it as a gate with no red. Its red IS the fact changing: `Entity`
/// already carries a `generation: u32` and `Entity::increment_generation` is
/// documented "used to detect stale handles", so the kernel is BUILT for
/// recycling and merely does not do it yet. The day allocation starts reusing
/// freed ids, this goes red and names the consequence, instead of
/// `ui_tween_reap` silently removing an unrelated entity's channel —
/// `DenseStore::remove` takes a bare `EntityId` and performs NO liveness check
/// (MEASURED: it removed a live, unrelated row and returned `true`).
#[test]
fn entity_ids_are_not_recycled_today() {
    let mut world = a1_world();

    let first: Vec<Entity> = (0..6).map(|_| spawn_node(&mut world)).collect();
    let first_ids: Vec<_> = first.iter().map(|e| e.id()).collect();

    for &e in &first[..4] {
        world.run_system(move |mut cmds: Commands| cmds.despawn(e));
    }

    let second: Vec<Entity> = (0..6).map(|_| spawn_node(&mut world)).collect();
    let second_ids: Vec<_> = second.iter().map(|e| e.id()).collect();

    let recycled: Vec<_> =
        second_ids.iter().filter(|id| first_ids.contains(id)).copied().collect();

    assert!(
        recycled.is_empty(),
        "an EntityId handed out before was handed out AGAIN ({recycled:?}; first batch \
         {first_ids:?}, second batch {second_ids:?}). `UiTweenScratch`'s key is a bare EntityId \
         with no generation, and `DenseStore::remove` does no liveness check — the moment ids are \
         recycled, a completion pair replayed across a despawn removes an UNRELATED entity's \
         channel. `ui_tween_reap` must carry `Entity` (generation included) from here on."
    );
}

/// **(ii)** A reaped dense slot reused by a `start_tween_*` in the same frame
/// carries the NEW value and no residue of the old one.
///
/// `DenseStore::remove` tombstones the slot and pushes it onto the free list;
/// the next `insert` pops that same slot and re-stamps both ticks. This drives
/// exactly that sequence inside one frame — the reap, then the insert, then the
/// tick reading it — which is where a stale per-slot tick or a stale byte range
/// would show.
#[test]
fn a_reaped_slot_is_reused_with_fresh_ticks() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 100.0, EasingId::LINEAR, 0);
    });

    // The frame that completes it: tick pushes, reap removes, slot goes free.
    world.resource_mut::<Time>().advance_with(FRAME);
    world.run_system(ui_clock_tick);
    world.run_system(ui_visual_tick);
    world.run_system(ui_tween_reap);
    assert_eq!(live_count(&world, TweenTint::component_id()), 0);

    // …and, in the SAME frame, a new tween on the same entity + same channel.
    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0xFF00_0000, 0xFF00_00FF, 1000.0, EasingId::LINEAR, 0);
    });
    let row = *world.get_component::<TweenTint>(e).expect("the reused slot holds the new row");
    assert_eq!(row.from, 0xFF00_0000, "the reused slot carries the NEW value");
    assert_eq!(row.elapsed, 0.0, "and no residue of the old phase");

    frame(&mut world, FRAME);
    assert_eq!(
        live_count(&world, TweenTint::component_id()),
        1,
        "the new tween is running, not swept by the frame that reaped its predecessor"
    );
}

/// **(iii)** The tick's `AnyOf` fetch resolves FOUR dense stores in one query and
/// composes all four payloads into one sink.
#[test]
fn the_anyof_fetch_spans_four_dense_stores() {
    let mut world = a1_world();
    let e = spawn_node(&mut world);
    world.run_system(move |mut cmds: Commands| {
        start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, 200.0, EasingId::LINEAR, 0);
        start_tween_opacity(&mut cmds, e, 0.0, 1.0, 200.0, EasingId::LINEAR, 0);
        start_tween_offset(&mut cmds, e, [0.0, 0.0], [-8.0, 4.0], 200.0, EasingId::LINEAR, 0);
        start_tween_scale(&mut cmds, e, [1.0, 1.0], [2.0, 2.0], 200.0, EasingId::LINEAR, 0);
    });
    for id in [
        TweenTint::component_id(),
        TweenOpacity::component_id(),
        TweenOffset::component_id(),
        TweenScale::component_id(),
    ] {
        assert_eq!(live_count(&world, id), 1, "all four channels are live");
    }

    frame(&mut world, FRAME);
    let v = *world.get_component::<UiVisual>(e).expect("sink");
    assert!(v.opacity > 0.0 && v.opacity < 1.0, "opacity is mid-tween");
    assert!(v.offset_px[0] < 0.0, "offset is mid-tween");
    assert!(v.scale[0] > 1.0, "scale is mid-tween");
    assert_ne!(v.tint_mul, 0xFFFF_FFFF, "tint is mid-tween");

    frame(&mut world, FRAME);
    for id in [
        TweenTint::component_id(),
        TweenOpacity::component_id(),
        TweenOffset::component_id(),
        TweenScale::component_id(),
    ] {
        assert_eq!(live_count(&world, id), 0, "and all four completed and were reaped together");
    }
    let v = *world.get_component::<UiVisual>(e).expect("sink");
    assert_eq!(v.opacity, 1.0);
    assert_eq!(v.offset_px, [-8.0, 4.0]);
    assert_eq!(v.scale, [2.0, 2.0]);
    assert_eq!(v.tint_mul, 0xFFFF_FFFF);
}
