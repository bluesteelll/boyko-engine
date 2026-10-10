//! Phase 22 Wave 1C — positive tests for the `#[derive(Component)]`
//! single-component `Bundle` emission, the `#[component(no_bundle)]` opt-out,
//! and `Commands::spawn_empty` (plan D5/D7).
//!
//! Component ids are minted dynamically via the derive (`register_new`) — no
//! pinned slots, no collision with the slot-pinned suites.
//!
//! The compile-fail half of the D7 contract (double derive, `!Send` without
//! `no_bundle`) lives in the trybuild suite (`tests/bundle_compile_fail/`).

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

#[derive(Component)]
struct Solo(u32);

#[derive(Component)]
struct Extra(u32);

/// `!Send + !Sync` payload — compiles ONLY because of the `no_bundle` opt-out
/// (the trybuild case `non_send_component_without_no_bundle.rs` pins the
/// failure without it).
#[derive(Component)]
#[component(no_bundle)]
struct ExoticRc(Rc<u32>);

/// Ordinary type opting out of the Bundle emission: stays a full `Component`,
/// usable inside a `#[derive(Bundle)]` wrapper, just not spawnable bare.
#[derive(Component)]
#[component(no_bundle)]
struct PlainOptOut(u32);

#[derive(Bundle)]
struct PlainOptOutBundle {
    inner: PlainOptOut,
}

/// Runs one deferred-command system and returns the `Entity` it captured
/// (the `Arc<Mutex<_>>` capture idiom from `phase11_entity_commands.rs` —
/// system closures must be `Send`).
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

// ── D7: derive(Component) emits a single-component Bundle ────────────────────

#[test]
fn derived_component_is_a_single_component_bundle() {
    // The emitted impl exposes exactly one id — the component's own (B1 is
    // trivially canonical for one element).
    let ids = <Solo as Bundle>::component_ids();
    assert_eq!(ids.len(), 1, "single-component bundle exposes exactly 1 id");
    assert_eq!(ids[0], Solo::component_id());

    // OnceLock cache contract (SBC2/SBC3): stable &'static payload.
    let a = <Solo as Bundle>::static_info();
    let b = <Solo as Bundle>::static_info();
    assert!(std::ptr::eq(a, b), "static_info() must be cached per type");

    // Distinct component types mint distinct BundleTypeIds.
    assert_ne!(
        <Solo as Bundle>::bundle_type_id(),
        <Extra as Bundle>::bundle_type_id(),
        "each derived component owns its own BundleTypeId (SBC2)"
    );
}

#[test]
fn spawn_bare_component_end_to_end() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| cmds.spawn(Solo(7)).id());

    assert_eq!(ecs.entity_count(), 1, "bare-component spawn creates one entity");
    assert!(ecs.has_entity(entity));
    let solo = ecs.get_component::<Solo>(entity).expect("Solo present");
    assert_eq!(solo.0, 7, "component bytes survived the bundle byte-erasure");
}

#[test]
fn spawn_bare_component_then_insert_bare_component() {
    let mut ecs = EcsMaster::new();
    let entity =
        run_capturing(&mut ecs, |cmds| cmds.spawn(Solo(1)).insert(Extra(2)).id());

    assert_eq!(ecs.entity_count(), 1);
    assert_eq!(ecs.get_component::<Solo>(entity).expect("Solo present").0, 1);
    assert_eq!(
        ecs.get_component::<Extra>(entity).expect("Extra present").0,
        2,
        "bare-component insert migrates through the same audited machinery"
    );
}

// ── D7: #[component(no_bundle)] opt-out ──────────────────────────────────────

#[test]
fn no_bundle_type_is_still_a_full_component() {
    // The exotic (!Send) type compiles under the opt-out and mints a real
    // ComponentId; the derive's inherent layout constants stay intact.
    let id = ExoticRc::component_id();
    assert_eq!(ExoticRc::component_id(), id, "id is stable (OnceLock)");
    assert_eq!(ExoticRc::mem_size(), std::mem::size_of::<ExoticRc>());
    assert_eq!(ExoticRc::alignment(), std::mem::align_of::<ExoticRc>());
    // The payload itself stays usable (and keeps the field read).
    assert_eq!(*ExoticRc(Rc::new(3)).0, 3);
}

#[test]
fn no_bundle_type_composes_with_derive_bundle_wrapper() {
    // A no_bundle component remains spawnable through an explicit
    // #[derive(Bundle)] wrapper — the opt-out removes only the bare-spawn
    // emission, not component-hood.
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(PlainOptOutBundle { inner: PlainOptOut(5) }).id()
    });

    assert_eq!(
        ecs.get_component::<PlainOptOut>(entity).expect("present").0,
        5
    );
}

// ── M1: the MAX_BUNDLE_ARITY (16) ceiling, exactly-at-boundary ───────────────
//
// The derive rejects > 16 fields at expansion time (the trybuild case
// `over_max_arity.rs` pins the message); 16 is the LARGEST accepted bundle.
// The runtime stack collectors (`migration_helpers::migrate_entity_insert`'s
// `bundle_ids[MAX_BUNDLE_ARITY]` / `bundle_added[MAX_BUNDLE_ARITY]`, guarded
// by `debug_assert!(bundle_id_count < MAX_BUNDLE_ARITY)`) reach exactly full
// occupancy only through a 16-component insert — covered below.

macro_rules! def_arity_components {
    ($($ty:ident),+ $(,)?) => {
        $(
            #[derive(Component)]
            struct $ty(u32);
        )+
    };
}

def_arity_components!(
    A01, A02, A03, A04, A05, A06, A07, A08, A09, A10, A11, A12, A13, A14, A15, A16,
);

/// Exactly at the ceiling: 16 fields == `MAX_BUNDLE_ARITY`.
#[derive(Bundle)]
struct SixteenFieldBundle {
    f01: A01,
    f02: A02,
    f03: A03,
    f04: A04,
    f05: A05,
    f06: A06,
    f07: A07,
    f08: A08,
    f09: A09,
    f10: A10,
    f11: A11,
    f12: A12,
    f13: A13,
    f14: A14,
    f15: A15,
    f16: A16,
}

fn make_sixteen(base: u32) -> SixteenFieldBundle {
    SixteenFieldBundle {
        f01: A01(base + 1),
        f02: A02(base + 2),
        f03: A03(base + 3),
        f04: A04(base + 4),
        f05: A05(base + 5),
        f06: A06(base + 6),
        f07: A07(base + 7),
        f08: A08(base + 8),
        f09: A09(base + 9),
        f10: A10(base + 10),
        f11: A11(base + 11),
        f12: A12(base + 12),
        f13: A13(base + 13),
        f14: A14(base + 14),
        f15: A15(base + 15),
        f16: A16(base + 16),
    }
}

/// Asserts all 16 components landed with their `make_sixteen(base)` payloads.
fn assert_sixteen(ecs: &EcsMaster, entity: Entity, base: u32) {
    macro_rules! check {
        ($($idx:literal => $ty:ident),+ $(,)?) => {
            $(
                assert_eq!(
                    ecs.get_component::<$ty>(entity)
                        .unwrap_or_else(|| panic!(
                            "{} missing after a 16-component bundle landed",
                            stringify!($ty)
                        ))
                        .0,
                    base + $idx,
                    "{} carries the wrong payload",
                    stringify!($ty)
                );
            )+
        };
    }
    check!(
        1 => A01, 2 => A02, 3 => A03, 4 => A04, 5 => A05, 6 => A06, 7 => A07,
        8 => A08, 9 => A09, 10 => A10, 11 => A11, 12 => A12, 13 => A13,
        14 => A14, 15 => A15, 16 => A16,
    );
}

#[test]
fn sixteen_field_bundle_spawns_at_the_arity_ceiling() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| cmds.spawn(make_sixteen(100)).id());

    assert_eq!(ecs.entity_count(), 1, "one entity from the at-ceiling spawn");
    assert_sixteen(&ecs, entity, 100);
}

#[test]
fn sixteen_field_bundle_inserts_through_migration_at_the_arity_ceiling() {
    // Spawn with a 17th, unrelated component first so the insert is a real
    // migration (target = source ∪ 16 new ids ≠ source) — this drives
    // `migrate_entity_insert`'s bundle-id collectors to exactly
    // `MAX_BUNDLE_ARITY` entries, the boundary the debug_assert guards.
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| {
        cmds.spawn(Solo(7)).insert(make_sixteen(200)).id()
    });

    assert_eq!(ecs.entity_count(), 1);
    assert_eq!(
        ecs.get_component::<Solo>(entity).expect("Solo retained").0,
        7,
        "the retained component survives the at-ceiling migration"
    );
    assert_sixteen(&ecs, entity, 200);
}

// ── D5: Commands::spawn_empty ────────────────────────────────────────────────

#[test]
fn spawn_empty_creates_componentless_entity() {
    let mut ecs = EcsMaster::new();
    let entity = run_capturing(&mut ecs, |cmds| cmds.spawn_empty().id());

    assert_eq!(ecs.entity_count(), 1, "empty spawn creates one live entity");
    assert!(ecs.has_entity(entity), "the componentless entity is live");
    assert!(
        ecs.get_component::<Solo>(entity).is_none(),
        "a componentless entity carries no components"
    );
}

#[test]
fn spawn_empty_warm_path_mints_distinct_live_entities() {
    // Two empty spawns land in the SAME (cached) empty archetype; both
    // entities stay live and distinct — the despawn-identity regression
    // proper is Wave 1A's; here we pin the spawn/identity half reachable
    // through the public Commands surface.
    let mut ecs = EcsMaster::new();
    let pair: Arc<Mutex<Option<(Entity, Entity)>>> = Arc::new(Mutex::new(None));

    let probe = Arc::clone(&pair);
    ecs.run_system(move |mut cmds: Commands| {
        let a = cmds.spawn_empty().id();
        let b = cmds.spawn_empty().id();
        *probe.lock().expect("not poisoned") = Some((a, b));
    });

    let (a, b) = pair
        .lock()
        .expect("not poisoned")
        .take()
        .expect("spawns ran");
    assert_ne!(a, b, "two empty spawns mint distinct entities");
    assert_eq!(ecs.entity_count(), 2);
    assert!(ecs.has_entity(a));
    assert!(ecs.has_entity(b));
}

#[test]
fn spawn_empty_then_insert_component() {
    // Natural user flow: reserve an empty entity, attach data later. The
    // insert migrates from the empty archetype (zero retained columns)
    // through the existing generic insert machinery.
    let mut ecs = EcsMaster::new();
    let entity =
        run_capturing(&mut ecs, |cmds| cmds.spawn_empty().insert(Solo(9)).id());

    assert_eq!(ecs.entity_count(), 1);
    assert_eq!(
        ecs.get_component::<Solo>(entity).expect("Solo present").0,
        9,
        "insert after spawn_empty attaches the component"
    );
}
