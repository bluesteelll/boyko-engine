//! EnableTag Step 9 — behavioral tests for the deferred toggle commands, the
//! dynamic `with_enabled` / `without_enabled` per-row query terms, and the
//! `QueryView::get` / `get_mut` / `single` enable handling (Decision D2/D3,
//! C3-r5 / C3-r7-c).
//!
//! Lives OUT-OF-CRATE so every assertion proves public reachability of the
//! Step-9 surface (`EntityCommands::enable`/`disable`/`enable_id`/`disable_id`,
//! `Query`/`QueryView::with_enabled`/`without_enabled`, and the point-lookup
//! enable filtering).
//!
//! # Fixture note
//!
//! Enable tags are minted via the public `EcsMaster::register_enable_tag`,
//! which classifies the id as `StorageKind::Bitset` (the public path — the
//! `set_storage_kind` classifier is crate-private). For the typed
//! `Enabled<T>` filter a hand-written `Component` impl returns the
//! bitset-classified id of a registered enable tag, so the typed filter sees a
//! genuine bitset storage kind without the derive's `storage = "bitset"`
//! emission (Wave 5). Ids are lazy-minted (Step 7 proved lazy-mint is
//! collision-proof in the shared lib-test process).

use std::sync::OnceLock;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::EnableTagId;
use boyko_ecs::ecs::core::iters::query::Enabled;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::prelude::{Commands, EcsMaster, Entity, Query};
use boyko_macros::Component;

// ── Component fixtures ───────────────────────────────────────────────────────

/// Real data component the queries read; lazy-minted via the derive.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Payload {
    v: u32,
}

/// A typed enable tag. Its id is the bitset-classified id of a registered
/// enable tag (set up once via `prime_typed_tag`), so `Enabled<TypedFlag>` /
/// `Disabled<TypedFlag>` see a genuine `StorageKind::Bitset`. Hand-written
/// `Component` impl (the derive's `boyko_ecs::..` paths are out of scope here,
/// and `storage = "bitset"` emission is a Wave-5 concern).
#[repr(C)]
struct TypedFlag;

static TYPED_FLAG_ID: OnceLock<ComponentId> = OnceLock::new();

impl Component for TypedFlag {
    fn component_id() -> ComponentId {
        *TYPED_FLAG_ID.get().expect("call prime_typed_tag() first")
    }
}

/// Mints the bitset enable tag backing [`TypedFlag`] (idempotent across the
/// shared test process via the `OnceLock`).
fn prime_typed_tag(ecs: &mut EcsMaster) {
    let tag = ecs.register_enable_tag("step9_typed_flag");
    let _ = TYPED_FLAG_ID.set(tag.component_id());
}

// ── Spawn helper (direct create path) ────────────────────────────────────────

fn spawn(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32) -> Entity {
    let bytes = v.to_ne_bytes();
    ecs.create_entity(arch, &[(Payload::component_id(), &bytes)])
        .expect("create_entity must succeed on the direct path")
}

fn payload_archetype(ecs: &mut EcsMaster) -> ArchetypeId {
    ecs.create_archetype(&[Payload::component_id()])
}

// ── 1. Deferred toggle applies at the apply window ───────────────────────────

#[test]
fn deferred_enable_applies_at_window() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step9_deferred_apply");
    let arch = payload_archetype(&mut ecs);
    let e = spawn(&mut ecs, arch, 1);

    assert!(!ecs.is_enabled_id(e, tag), "starts disabled");

    // Enqueue a deferred enable; it must NOT take effect until the apply window.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).enable_id(tag);
    });
    assert!(ecs.is_enabled_id(e, tag), "deferred enable applied at the window");

    // A deferred disable round-trips.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).disable_id(tag);
    });
    assert!(!ecs.is_enabled_id(e, tag), "deferred disable applied at the window");
}

#[test]
fn deferred_enable_typed_and_chained() {
    let mut ecs = EcsMaster::new();
    prime_typed_tag(&mut ecs);
    let arch = payload_archetype(&mut ecs);
    let e = spawn(&mut ecs, arch, 1);

    assert!(!ecs.is_enabled::<TypedFlag>(e), "starts disabled");
    // Typed deferred enable, chained off `entity`.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).enable::<TypedFlag>();
    });
    assert!(ecs.is_enabled::<TypedFlag>(e), "typed deferred enable applied");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).disable::<TypedFlag>();
    });
    assert!(!ecs.is_enabled::<TypedFlag>(e), "typed deferred disable applied");
}

// ── 2. T-INTERLEAVE: despawn racing a deferred toggle in one frame ───────────

#[test]
fn t_interleave_despawn_races_deferred_toggle_is_noop() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step9_interleave");
    let arch = payload_archetype(&mut ecs);
    let victim = spawn(&mut ecs, arch, 1);
    let survivor = spawn(&mut ecs, arch, 2);

    // In one apply window: despawn `victim` AND enqueue an enable on `victim`.
    // The toggle must be a silent no-op (dead entity) and must NOT corrupt the
    // survivor that swaps into the vacated row. The enable on the survivor
    // resolves to its live (post-swap) row.
    let enable_gen_before = ecs.archetype_master().enable_generation();
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).despawn();
        cmds.entity(victim).enable_id(tag); // races the despawn — no-op
        cmds.entity(survivor).enable_id(tag); // applies to the survivor's live row
    });

    assert!(!ecs.is_enabled_id(victim, tag), "dead victim's toggle is a no-op");
    assert!(
        ecs.is_enabled_id(survivor, tag),
        "survivor's enable applied at its live post-swap row"
    );
    assert!(
        ecs.archetype_master().enable_generation() > enable_gen_before,
        "a real column alloc happened for the survivor toggle"
    );
}

// ── 3. Dynamic with_enabled / without_enabled filter per-row ─────────────────

/// Builds 4 `Payload` rows in one archetype; enables `tag` on rows 0 and 2.
fn mixed_fixture(name: &str) -> (EcsMaster, EnableTagId, [Entity; 4]) {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag(name);
    let arch = payload_archetype(&mut ecs);
    let e = [
        spawn(&mut ecs, arch, 0),
        spawn(&mut ecs, arch, 1),
        spawn(&mut ecs, arch, 2),
        spawn(&mut ecs, arch, 3),
    ];
    ecs.enable_id(e[0], tag);
    ecs.enable_id(e[2], tag);
    (ecs, tag, e)
}

#[test]
fn dynamic_with_enabled_yields_only_enabled_rows() {
    let (mut ecs, tag, _e) = mixed_fixture("step9_dyn_with");
    let sum = ecs.run_closure_once(move |q: Query<&Payload>| {
        let mut s = 0u32;
        for p in q.with_enabled(tag).iter() {
            s += p.v;
        }
        s
    });
    // Rows 0 (v=0) and 2 (v=2) are enabled ⇒ 0 + 2 = 2.
    assert_eq!(sum, 2, "with_enabled visits only the enabled rows");
}

#[test]
fn dynamic_without_enabled_yields_complement() {
    let (mut ecs, tag, _e) = mixed_fixture("step9_dyn_without");
    let sum = ecs.run_closure_once(move |q: Query<&Payload>| {
        let mut s = 0u32;
        for p in q.without_enabled(tag).iter() {
            s += p.v;
        }
        s
    });
    // Rows 1 (v=1) and 3 (v=3) are disabled (clear bit).
    assert_eq!(sum, 1 + 3, "without_enabled visits only the disabled rows");
}

#[test]
fn dynamic_with_enabled_iter_mut() {
    let (mut ecs, tag, e) = mixed_fixture("step9_dyn_with_mut");
    // Double every enabled row's payload via iter_mut + with_enabled.
    // `with_enabled(self)` consumes the query and returns it with the term set;
    // `.iter_mut()` then needs `&mut self` on that owned value.
    ecs.run_system(move |q: Query<&mut Payload>| {
        for p in q.with_enabled(tag).iter_mut() {
            p.v *= 10;
        }
    });
    // Enabled rows 0 (0*10=0) and 2 (2*10=20) changed; disabled rows untouched.
    assert_eq!(ecs.get_component::<Payload>(e[0]).unwrap().v, 0);
    assert_eq!(ecs.get_component::<Payload>(e[2]).unwrap().v, 20);
    assert_eq!(ecs.get_component::<Payload>(e[1]).unwrap().v, 1, "disabled untouched");
    assert_eq!(ecs.get_component::<Payload>(e[3]).unwrap().v, 3, "disabled untouched");
}

// ── 4. >MAX_ENABLE_TERMS dynamic terms = loud panic ──────────────────────────

#[test]
#[should_panic(expected = "MAX_ENABLE_TERMS")]
fn dynamic_enable_terms_overflow_panics() {
    let mut ecs = EcsMaster::new();
    // Mint MAX_ENABLE_TERMS + 1 distinct tags.
    let mut tags = Vec::new();
    for i in 0..(boyko_ecs::ecs::constants::MAX_ENABLE_TERMS + 1) {
        tags.push(ecs.register_enable_tag(&format!("step9_overflow_{i}")));
    }
    let _ = payload_archetype(&mut ecs);
    ecs.run_closure_once(move |q: Query<&Payload>| {
        let mut q = q;
        // Past the cap, push_with must loudly panic at setup time.
        for &t in &tags {
            q = q.with_enabled(t);
        }
        0u32
    });
}

// ── 5. QueryView::get / get_mut honor typed Enabled<T> (C3 compile-but-lie) ───

#[test]
fn get_on_disabled_entity_returns_none_typed() {
    let mut ecs = EcsMaster::new();
    prime_typed_tag(&mut ecs);
    let arch = ecs.create_archetype(&[Payload::component_id()]);
    let enabled = spawn(&mut ecs, arch, 7);
    let disabled = spawn(&mut ecs, arch, 8);
    ecs.enable::<TypedFlag>(enabled);

    let view = ecs.query::<&Payload, Enabled<TypedFlag>>();
    assert!(
        view.get(enabled).is_some(),
        "get() yields the enabled entity"
    );
    assert!(
        view.get(disabled).is_none(),
        "get() on a disabled entity must return None (typed Enabled<T> honored)"
    );
}

#[test]
fn get_mut_on_disabled_entity_returns_none_typed() {
    let mut ecs = EcsMaster::new();
    prime_typed_tag(&mut ecs);
    let arch = ecs.create_archetype(&[Payload::component_id()]);
    let enabled = spawn(&mut ecs, arch, 7);
    let disabled = spawn(&mut ecs, arch, 8);
    ecs.enable::<TypedFlag>(enabled);

    let mut view = ecs.query::<&mut Payload, Enabled<TypedFlag>>();
    assert!(view.get_mut(enabled).is_some(), "get_mut yields the enabled entity");
    assert!(
        view.get_mut(disabled).is_none(),
        "get_mut on a disabled entity must return None (typed Enabled<T> honored)"
    );
}

#[test]
fn get_honors_dynamic_with_enabled() {
    let (mut ecs, tag, e) = mixed_fixture("step9_get_dyn");
    let view = ecs.query::<&Payload, ()>().with_enabled(tag);
    assert!(view.get(e[0]).is_some(), "enabled row visible to get()");
    assert!(
        view.get(e[1]).is_none(),
        "disabled row filtered by dynamic with_enabled at get()"
    );
}

// ── 6. single over a world where the only match is disabled → empty/panic ─────

#[test]
#[should_panic(expected = "zero rows")]
fn single_over_only_disabled_match_is_empty() {
    let mut ecs = EcsMaster::new();
    prime_typed_tag(&mut ecs);
    let arch = ecs.create_archetype(&[Payload::component_id()]);
    let only = spawn(&mut ecs, arch, 42);
    // `only` is the sole entity, but it is DISABLED — `single` routes through
    // `iter`, which filters it out, so `single` panics with "zero rows".
    let _ = only;

    let view = ecs.query::<&Payload, Enabled<TypedFlag>>();
    let _ = view.single();
}

#[test]
fn single_yields_the_one_enabled_match() {
    let mut ecs = EcsMaster::new();
    prime_typed_tag(&mut ecs);
    let arch = ecs.create_archetype(&[Payload::component_id()]);
    let a = spawn(&mut ecs, arch, 10);
    let _b = spawn(&mut ecs, arch, 20);
    // Enable only `a` — `single` (via iter) must yield exactly `a`.
    ecs.enable::<TypedFlag>(a);

    let view = ecs.query::<&Payload, Enabled<TypedFlag>>();
    assert_eq!(view.single().v, 10, "single yields the lone enabled match");
}

// ── 7. 0%-gate behavioral: a no-enable-term query is unchanged ───────────────

#[test]
fn no_enable_term_query_iterates_all_rows() {
    // Structural gate: the dynamic per-row test is unreachable when no term is
    // set (`EnableTerms::is_empty()` short-circuits before the resolve / per-row
    // loop in both cursors). This behavioral check confirms a no-term query
    // still visits every row even when an enable column EXISTS in the archetype.
    let (mut ecs, _tag, _e) = mixed_fixture("step9_zero_gate");
    let (count, sum) = ecs.run_closure_once(|q: Query<&Payload>| {
        let mut c = 0usize;
        let mut s = 0u32;
        for p in &q {
            c += 1;
            s += p.v;
        }
        (c, s)
    });
    assert_eq!(count, 4, "no-enable-term query visits every row (0%-gate)");
    // Rows v = 0,1,2,3 ⇒ sum 6.
    assert_eq!(sum, 6, "no-enable-term query sums every row");
}

#[test]
fn no_enable_term_get_is_unfiltered() {
    // A QueryView with no enable term / no enable filter must return every live
    // matched entity from get(), including ones whose (unrelated) enable bit is
    // set — proving the point-lookup enable test is gated off.
    let (mut ecs, _tag, e) = mixed_fixture("step9_zero_gate_get");
    let view = ecs.query::<&Payload, ()>();
    for &ent in &e {
        assert!(view.get(ent).is_some(), "no-enable-term get yields every entity");
    }
}
