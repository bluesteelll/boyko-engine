//! **GK-2 (Gaia) — the entity-keyed change-tick API is blind to DENSE storage.**
//! RED BY DESIGN; not fixed here.
//!
//! # The class, and why this file sits beside `ke1_or_dense_blindness.rs`
//!
//! One mechanism recurs across the kernel: *code that resolves a per-archetype
//! `ComponentPool` by `ComponentId` without first screening the storage kind*.
//! A `StorageKind::Dense` (or `Bitset`) id owns no such pool — it is excluded
//! from every archetype signature — so the resolve yields `None`, and the site
//! then consumes that `None` as if it were an ANSWER.
//!
//! Aether rung R0 / backlog KE1 fixed one instance (`Or`'s dense arms). These
//! two functions are a second, in `crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs`:
//!
//! * `EcsMaster::any_changed_since` — `let Some(pool) = archetype.component_pools().get_pool(id)
//!   else { continue; }`. A dense bind source contributes nothing to the scan, so
//!   the gate is **never true**.
//! * `EcsMaster::get_component_changed_tick` — `let pool = pools.get_pool(component_id)?;`.
//!   Returns **`None` forever** for a dense component.
//!
//! Both are the change-gated UI data-bind path's primitives
//! (`boyko_ui` `binding/bind_system.rs` — `any_changed_since` at the discovery
//! gate, `get_component_changed_tick` at the per-bind gate, where `None` is
//! consumed as "source is gone, skip this bind"). A `boyko_ui` binding whose
//! source is a dense component therefore **never updates**, and nothing is
//! logged.
//!
//! # Why the second function matters on its own
//!
//! Gaia's GK-2 row names `any_changed_since`. `get_component_changed_tick` is the
//! same defect in the same file and is **not** named there — it is recorded here
//! because a fix scoped to the named function would leave the per-bind gate
//! broken while the discovery gate started firing, which is worse than either
//! end being broken alone.
//!
//! The sharpest evidence that this is a real miss rather than a design choice:
//! **Follow-up #14 already remediated exactly this class in exactly this file.**
//! `get_component` / `get_component_mut` / `has_component` / `get_component_raw`
//! / `set_component_raw` each grew a `component_registry::storage_kind(...) ==
//! StorageKind::Dense` branch that routes to the `DenseStore`
//! (`component_api.rs` — the screens at the heads of those functions; gated by
//! `tests/dense_direct_component_access.rs`). The two tick readers sit between
//! those screened functions and did not get one.
//!
//! # Why these tests are `#[ignore]`d rather than fixed
//!
//! The remedy is Gaia's **GK-2**, which its plan gives its own design pass:
//! routing the tick read through `DenseStore`'s per-slot ticks vs refusing a
//! dense bind source at bake time are different products, and the second also
//! decides whether `Bitset` (which has no ticks at all) is refused or reported.
//! `boyko_scene`'s dirty scan (`propagation.rs`, `get_component_changed_tick` on
//! `Transform` / `ChildOf`) is a second consumer whose behaviour a fix changes,
//! so this is not a kernel-local call. Landing one here would pre-empt it.
//!
//! A companion doc fix belongs to that pass, per Gaia's own note: the
//! `any_changed_since` doc comment claims the scan is bounded to hosting
//! archetypes, which is false for exactly this case.
//!
//! Run them with:
//! `cargo test -p boyko-ecs --test gk2_change_tick_api_dense_blindness -- --ignored`

use std::sync::Arc;

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

/// DENSE payload — owns a global `DenseStore` and NO per-archetype pool.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct GDense760 {
    x: u32,
}

/// TABLE payload — the differential control. Every assertion below is written
/// as "the dense answer must equal the table answer", so a world where nothing
/// was ever changed cannot make these tests pass vacuously.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct GTable761 {
    x: u32,
}

#[derive(Bundle)]
struct GBoth {
    t: GTable761,
    d: GDense760,
}

/// Spawns one entity carrying BOTH payloads and returns its handle.
fn spawn_both(world: &mut EcsMaster) -> Entity {
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(GBoth {
            t: GTable761 { x: 1 },
            d: GDense760 { x: 1 },
        })
        .id()
    })
}

/// Runs one frame that writes BOTH payloads through `Mut<T>` (the deref that
/// bumps a changed tick on either storage kind).
fn frame_writing_both(world: &mut EcsMaster) {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut qt: Query<Mut<GTable761>>, mut qd: Query<Mut<GDense760>>| {
        for mut t in &mut qt {
            t.x = t.x.wrapping_add(1);
        }
        for mut d in &mut qd {
            d.x = d.x.wrapping_add(1);
        }
    });
    let mut schedule = builder.build(&mut *world);
    schedule.run(&mut *world);
}

/// Floor: the TABLE half of every assertion below must be TRUE / `Some`. Not
/// ignored — if this reds, the two ignored tests are measuring an empty world
/// rather than the dense blindness.
#[test]
fn table_control_reports_the_change() {
    let mut world = EcsMaster::new();
    let e = spawn_both(&mut world);
    frame_writing_both(&mut world);
    let now = world.current_tick();

    assert!(
        world.any_changed_since(&[GTable761::component_id()], Tick::ZERO, now),
        "control: any_changed_since must see a written TABLE component"
    );
    assert!(
        world
            .get_component_changed_tick(e, GTable761::component_id())
            .is_some(),
        "control: get_component_changed_tick must resolve a live TABLE row"
    );
}

/// `any_changed_since` must answer the same for a dense component as for a
/// table one. It answers `false` — the arm is skipped, not evaluated.
///
/// Hand oracle: `true` (the dense component WAS written this frame, and the
/// table control proves the window is right). Observed today: `false`.
#[test]
#[ignore = "deferred: Gaia GK-2 owns the fix for the entity-keyed change-tick \
            API over dense/bitset storage (route through DenseStore ticks vs \
            refuse a dense bind source at bake). RED BY DESIGN until GK-2's \
            design pass lands; see this file's header"]
fn any_changed_since_sees_a_dense_component() {
    let mut world = EcsMaster::new();
    let _e = spawn_both(&mut world);
    frame_writing_both(&mut world);
    let now = world.current_tick();

    assert!(
        world.any_changed_since(&[GDense760::component_id()], Tick::ZERO, now),
        "any_changed_since must see a written DENSE component. `false` is the \
         GK-2 defect: a dense id owns no per-archetype pool, so `get_pool(id)` \
         is None and the `else {{ continue }}` skips the arm — a boyko_ui \
         binding over a dense source never updates"
    );
}

/// `get_component_changed_tick` must resolve a live dense member's tick. It
/// returns `None`, which every caller reads as "the source is gone".
///
/// Hand oracle: `Some(_)`. Observed today: `None`.
#[test]
#[ignore = "deferred: Gaia GK-2 owns the fix for the entity-keyed change-tick \
            API over dense/bitset storage (route through DenseStore ticks vs \
            refuse a dense bind source at bake). RED BY DESIGN until GK-2's \
            design pass lands; see this file's header. NOTE: GK-2's row names \
            only any_changed_since — this second site is recorded by Aether R0's \
            census"]
fn get_component_changed_tick_resolves_a_dense_member() {
    let mut world = EcsMaster::new();
    let e = spawn_both(&mut world);
    frame_writing_both(&mut world);

    assert!(
        world
            .get_component_changed_tick(e, GDense760::component_id())
            .is_some(),
        "get_component_changed_tick must resolve a live DENSE member. `None` is \
         the GK-2 defect on its SECOND site: callers (boyko_ui \
         binding/bind_system.rs, boyko_scene propagation.rs) read None as \
         'source despawned, skip this bind'"
    );
}
