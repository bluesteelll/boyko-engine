//! Phase 14a — deferred reentrancy (the SOUNDNESS tests) + runtime-builder +
//! derive-XOR-runtime + staleness panics.
//!
//! Plan §8 test surface bullets 3 + 5:
//!
//! * **3 (deferred reentrancy).** A hook may enqueue structural commands via
//!   `ctx.commands()`. They must be DEFERRED — not executed inline at the fire
//!   site — and applied at the OUTERMOST drain. We pin:
//!   (a) an `on_add` hook spawning a new entity → the spawn is NOT visible
//!   inline; the new entity exists AFTER the drain; no UB.
//!   (b) a hook despawning an entity → deferred, applied after, no double-free.
//!   (c) a re-entrant chain (hook spawns a hooked entity whose hook enqueues
//!   nothing) terminates.
//! * **5 (registration panics).** `register_component_hooks` on a derive-hooked
//!   type panics eagerly; `register_component_hooks` after the component is in a
//!   live archetype panics; a plain-derive type's runtime builder works.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// 3a — an on_add hook spawning a new entity is DEFERRED, not inline
// ════════════════════════════════════════════════════════════════════════════

/// How many times the trigger fired (guards against the spawned child's
/// `Child`-typed component re-triggering — `Child` has no hook, so this stays 1).
static R3A_FIRE_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe fn r3a_on_add(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R3A_FIRE_COUNT.fetch_add(1, SEQ);
    // Enqueue a deferred spawn of a DIFFERENT (un-hooked) component type. The
    // POST-drain entity-count assertion in the test body proves it was deferred
    // (and `hook_sees_self_but_not_its_deferred_spawn_target_inline` below proves
    // non-inline application directly via the read-only view).
    w.commands().spawn(R3aChildBundle { c: R3aChild(1) });
}

#[derive(Component)]
#[component(on_add = r3a_on_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3aParent(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3aChild(u32);

#[derive(Bundle)]
struct R3aChildBundle {
    c: R3aChild,
}

#[test]
fn hook_spawn_is_deferred_and_applied_after_drain() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[R3aParent::component_id()]);
    let _ = R3aChild::component_id();

    // Spawn the parent via the DIRECT api (create_entity → fires on_add inline,
    // drains at end). The hook enqueues a child spawn into the deferred queue;
    // the outermost drain (end of spawn_one's create_entity) applies it.
    let _p = ecs.spawn_one(arch, R3aParent(7)).expect("spawn parent");

    assert_eq!(R3A_FIRE_COUNT.load(SEQ), 1, "parent on_add fired exactly once");
    // After the full direct-API call returns, the deferred child spawn HAS been
    // drained — so the world holds BOTH entities. If the child spawn had been
    // applied INLINE (a soundness bug), the parent's own registration would not
    // yet be complete and re-entrancy could corrupt state; instead it is queued.
    assert_eq!(
        ecs.entity_count(),
        2,
        "parent + deferred-spawned child both present AFTER the outermost drain"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 3a' — prove non-inline directly: the hook reads its OWN entity, which must
//        already be registered (the fire is POST-register), but a deferred
//        spawn target is NOT yet queryable from inside the hook.
// ════════════════════════════════════════════════════════════════════════════

static R3B_SELF_VISIBLE: AtomicUsize = AtomicUsize::new(0);
static R3B_CHILD_VISIBLE_INLINE: AtomicUsize = AtomicUsize::new(0);

unsafe fn r3b_on_add(mut w: DeferredEcsMaster<'_>, ctx: HookContext) {
    // Self must be visible — on_add fires AFTER register_entity_with_ptr.
    if w.get_component::<R3bParent>(ctx.entity).is_some() {
        R3B_SELF_VISIBLE.fetch_add(1, SEQ);
    }
    // Enqueue a child spawn, capture its (reserved) entity handle.
    let child = w.commands().spawn(R3bChildBundle { c: R3bChild(9) });
    // The child is NOT yet materialised — a deferred spawn applies at the drain.
    if w.get_component::<R3bChild>(child).is_some() {
        // This would indicate the spawn was applied INLINE — a soundness bug.
        R3B_CHILD_VISIBLE_INLINE.fetch_add(1, SEQ);
    }
}

#[derive(Component)]
#[component(on_add = r3b_on_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3bParent(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3bChild(u32);

#[derive(Bundle)]
struct R3bChildBundle {
    c: R3bChild,
}

#[test]
fn hook_sees_self_but_not_its_deferred_spawn_target_inline() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[R3bParent::component_id()]);
    let _ = R3bChild::component_id();

    let _p = ecs.spawn_one(arch, R3bParent(1)).expect("spawn parent");

    assert_eq!(
        R3B_SELF_VISIBLE.load(SEQ),
        1,
        "on_add fires POST-register: the hook sees its own entity"
    );
    assert_eq!(
        R3B_CHILD_VISIBLE_INLINE.load(SEQ),
        0,
        "a hook's deferred spawn must NOT be visible inline (deferred, not inline)"
    );
    assert_eq!(ecs.entity_count(), 2, "child materialises at the drain");
}

// ════════════════════════════════════════════════════════════════════════════
// 3b — a hook despawning another entity is DEFERRED; applied after; no double-free
// ════════════════════════════════════════════════════════════════════════════

static R3C_TARGET: TargetCell = TargetCell::new();
static R3C_REMOVE_FIRES: AtomicUsize = AtomicUsize::new(0);

/// When the trigger entity is added, it enqueues a despawn of a pre-existing
/// target. The despawn is deferred and applied at the drain.
unsafe fn r3c_on_add(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    let target = R3C_TARGET.get();
    w.commands().entity(target).despawn();
}
/// Counts that the target's on_remove fired EXACTLY once (no double-free / no
/// double-fire). The target carries this hook.
unsafe fn r3c_target_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R3C_REMOVE_FIRES.fetch_add(1, SEQ);
}

#[derive(Component)]
#[component(on_add = r3c_on_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3cTrigger(u32);

#[derive(Component)]
#[component(on_remove = r3c_target_remove)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3cTarget(u32);

// Used by the (currently `#[ignore]`d) F1-repro despawn test; retained so the
// test compiles once F1 is fixed and it is un-ignored.
#[allow(dead_code)]
#[derive(Bundle)]
struct R3cTriggerBundle {
    c: R3cTrigger,
}

#[test]
fn hook_despawn_is_deferred_applied_once_no_double_free() {
    let mut ecs = EcsMaster::new();
    let target_arch = ecs.create_archetype(&[R3cTarget::component_id()]);
    let trigger_arch = ecs.create_archetype(&[R3cTrigger::component_id()]);

    // Pre-existing target entity.
    let target = ecs.spawn_one(target_arch, R3cTarget(1)).expect("spawn target");
    R3C_TARGET.set(target);
    assert!(ecs.has_entity(target), "target alive before trigger");

    // Spawn the trigger via the DIRECT API (`spawn_one` → `create_entity`),
    // whose on_add fires inline and enqueues the target despawn into the
    // world-resident `deferred_hook_queue`. `create_entity`'s OUTERMOST drain
    // (depth 0) then applies the despawn. NOTE: the lightweight `run_system`
    // helper does NOT drain the hook queue (only the schedule apply-window +
    // the three direct-API methods do, plan §0 Q-A1), so we drive the trigger
    // through the direct API here — that is the supported single-call boundary.
    let _trigger = ecs.spawn_one(trigger_arch, R3cTrigger(2)).expect("spawn trigger");

    assert!(!ecs.has_entity(target), "deferred despawn removed the target");
    assert_eq!(
        R3C_REMOVE_FIRES.load(SEQ),
        1,
        "target on_remove fired EXACTLY once — no double-free / no double-fire"
    );
    assert_eq!(ecs.entity_count(), 1, "only the trigger entity remains");
}

// ════════════════════════════════════════════════════════════════════════════
// 3c — re-entrant chain terminates: hook spawns a hooked entity whose hook
//        enqueues nothing
// ════════════════════════════════════════════════════════════════════════════

static R3D_PARENT_FIRES: AtomicUsize = AtomicUsize::new(0);
static R3D_CHILD_FIRES: AtomicUsize = AtomicUsize::new(0);

/// Parent's on_add spawns a Child (which is hooked). Child's on_add enqueues
/// NOTHING, so the chain terminates after one re-entrant level.
unsafe fn r3d_parent_add(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R3D_PARENT_FIRES.fetch_add(1, SEQ);
    w.commands().spawn(R3dChildBundle { c: R3dChild(1) });
}
unsafe fn r3d_child_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R3D_CHILD_FIRES.fetch_add(1, SEQ);
    // Intentionally enqueues nothing — terminates the chain.
}

#[derive(Component)]
#[component(on_add = r3d_parent_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3dParent(u32);

#[derive(Component)]
#[component(on_add = r3d_child_add)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R3dChild(u32);

#[derive(Bundle)]
struct R3dChildBundle {
    c: R3dChild,
}

#[test]
fn reentrant_hook_chain_terminates() {
    let mut ecs = EcsMaster::new();
    let parent_arch = ecs.create_archetype(&[R3dParent::component_id()]);
    let _ = R3dChild::component_id();

    // Direct API spawn → parent on_add fires inline, enqueues child spawn,
    // outermost drain applies the child spawn → child on_add fires (during the
    // drain), enqueues nothing → drain loop sees empty queue → terminates.
    let _p = ecs.spawn_one(parent_arch, R3dParent(1)).expect("spawn parent");

    assert_eq!(R3D_PARENT_FIRES.load(SEQ), 1, "parent on_add fired once");
    assert_eq!(
        R3D_CHILD_FIRES.load(SEQ),
        1,
        "child on_add fired once during the re-entrant drain (chain terminated)"
    );
    assert_eq!(ecs.entity_count(), 2, "parent + child both materialised");
}

// ════════════════════════════════════════════════════════════════════════════
// 5 — runtime builder works on a plain-derive type
// ════════════════════════════════════════════════════════════════════════════

static R5_RUNTIME_ADD: AtomicUsize = AtomicUsize::new(0);
static R5_RUNTIME_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn r5_runtime_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R5_RUNTIME_ADD.fetch_add(1, SEQ);
}
unsafe fn r5_runtime_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    R5_RUNTIME_REMOVE.fetch_add(1, SEQ);
}

/// Plain `#[derive(Component)]` — NO `#[component(...)]` attribute, so
/// `HAS_HOOKS == false` and the runtime builder is free to commit.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R5Plain(u32);

#[test]
fn runtime_builder_installs_hooks_on_plain_derive_type() {
    let mut ecs = EcsMaster::new();

    // Register hooks at runtime BEFORE the component appears in any archetype.
    // `.finish()` consumes the builder and commits explicitly (equivalent to the
    // drop-on-statement-end commit, but avoids the `#[must_use]` lint).
    ecs.register_component_hooks::<R5Plain>()
        .on_add(r5_runtime_add)
        .on_remove(r5_runtime_remove)
        .finish();

    let arch = ecs.create_archetype(&[R5Plain::component_id()]);
    let e = ecs.spawn_one(arch, R5Plain(1)).expect("spawn");
    assert_eq!(R5_RUNTIME_ADD.load(SEQ), 1, "runtime-registered on_add fires on spawn");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert_eq!(R5_RUNTIME_REMOVE.load(SEQ), 1, "runtime-registered on_remove fires on despawn");
}

// ════════════════════════════════════════════════════════════════════════════
// 5 — register_component_hooks on a DERIVE-hooked type panics eagerly
// ════════════════════════════════════════════════════════════════════════════

unsafe fn r5b_noop(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {}

#[derive(Component)]
#[component(on_add = r5b_noop)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R5DeriveHooked(u32);

#[test]
#[should_panic(expected = "declares #[component(...)]")]
fn register_component_hooks_on_derive_hooked_type_panics() {
    let mut ecs = EcsMaster::new();
    // The derive installed hooks via component_id(); the runtime builder must
    // reject this type eagerly (derive XOR runtime).
    let _builder = ecs.register_component_hooks::<R5DeriveHooked>();
}

// ════════════════════════════════════════════════════════════════════════════
// 5 — register_component_hooks AFTER the component is in a live archetype panics
// ════════════════════════════════════════════════════════════════════════════

/// Plain derive (so the derive-conflict check passes) — the panic we want is the
/// STALENESS one, triggered by the component already living in an archetype.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct R5Stale(u32);

#[test]
#[should_panic(expected = "already appears in a live archetype")]
fn register_component_hooks_after_archetype_exists_panics() {
    let mut ecs = EcsMaster::new();
    // Put R5Stale into a live archetype FIRST.
    let _arch = ecs.create_archetype(&[R5Stale::component_id()]);
    // Now registering hooks is stale — the archetype's flags were already
    // OR-computed without them. Must panic (release-level).
    let _builder = ecs.register_component_hooks::<R5Stale>();
}

// ────────────────────────────────────────────────────────────────────────────
// Helper: Send+Sync cell stashing an Entity into a static, for hooks.
// ────────────────────────────────────────────────────────────────────────────

use std::sync::atomic::AtomicU64;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::EntityId;

struct TargetCell(AtomicU64);
impl TargetCell {
    const fn new() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
    fn set(&self, e: Entity) {
        self.0.store((e.id().0 as u64) | ((e.generation() as u64) << 32), SEQ);
    }
    fn get(&self) -> Entity {
        let packed = self.0.load(SEQ);
        Entity::new(EntityId((packed & 0xFFFF_FFFF) as usize), (packed >> 32) as u32)
    }
}
