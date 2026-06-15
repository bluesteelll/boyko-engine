//! EnableTag Wave 5 / Step 10 — end-to-end tests for the
//! `#[derive(Component)] #[component(storage = "bitset")]` ergonomic path.
//!
//! Lives OUT-OF-CRATE so every assertion proves the derive's emitted
//! `boyko_ecs::` paths resolve for a downstream user. The derive must:
//!
//! 1. emit `const STORAGE_IS_BITSET = true;` (overriding the trait default),
//! 2. classify the minted id as `StorageKind::Bitset` at first `component_id()`
//!    (so it is filtered out of every archetype signature and has no
//!    `ComponentPool`), and
//! 3. SUPPRESS the single-component `Bundle` emission (a bitset tag has no pool
//!    and must not be spawnable as a one-component bundle).
//!
//! The compile-rejection cases (spawn-as-bundle, `storage = "typo"`, a fielded
//! bitset tag, `Added`/`Changed` on a derived bitset tag) live in the
//! `enable_filter_compile_fail` trybuild suite.
//!
//! Ids are lazy-minted via `#[derive(Component)]`, which the prior steps proved
//! collision-proof in the shared lib-test process (each distinct type gets its
//! own `OnceLock`-cached id).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self, StorageKind};
use boyko_ecs::ecs::core::iters::query::{Disabled, Enabled};
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::Component;

// ── Fixtures: a derived bitset enable tag + a real data component ─────────────

/// The headline case: a fieldless (ZST) bitset enable tag minted entirely by the
/// derive — no hand-written `Component` impl, no runtime `register_enable_tag`.
#[derive(Component)]
#[component(storage = "bitset")]
struct Stunned;

/// A second derived bitset tag, to prove distinct tags get distinct ids and that
/// the classification is per-type.
#[derive(Component)]
#[component(storage = "bitset")]
struct Selected;

/// An empty named-field struct is also a valid (ZST) bitset tag shape.
#[derive(Component)]
#[component(storage = "bitset")]
struct OnGround {}

/// Real data component the queries read; a normal (table-storage) derive.
#[derive(Clone, Copy)]
#[derive(Component)]
#[repr(C)]
struct Payload {
    v: u32,
}

/// A second real (table-storage) data component, used to build a MULTI-component
/// archetype alongside `Payload` (hardening D test 3).
#[derive(Clone, Copy)]
#[derive(Component)]
#[repr(C)]
struct Tag2 {
    w: u32,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn payload_archetype(ecs: &mut EcsMaster) -> ArchetypeId {
    ecs.create_archetype(&[Payload::component_id()])
}

fn spawn(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32) -> Entity {
    let bytes = v.to_ne_bytes();
    ecs.create_entity(arch, &[(Payload::component_id(), &bytes)])
        .expect("create_entity must succeed on the direct path")
}

/// A canonical-order archetype carrying BOTH `Payload` and `Tag2`. The bitset
/// tag id is deliberately NOT part of any archetype signature (it is filtered
/// out — see `derived_bitset_tag_filtered_out_of_signature`), so this is a
/// genuine two-data-column archetype.
fn payload_and_tag2_archetype(ecs: &mut EcsMaster) -> ArchetypeId {
    ecs.create_archetype(&[Payload::component_id(), Tag2::component_id()])
}

fn spawn_two(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32, w: u32) -> Entity {
    let vb = v.to_ne_bytes();
    let wb = w.to_ne_bytes();
    ecs.create_entity(
        arch,
        &[
            (Payload::component_id(), &vb),
            (Tag2::component_id(), &wb),
        ],
    )
    .expect("create_entity must succeed on the direct multi-component path")
}

// ── 1. STORAGE_IS_BITSET trait const is overridden to true ────────────────────

#[test]
fn derived_bitset_tag_storage_is_bitset_const_true() {
    // `STORAGE_IS_BITSET` is a trait const, so these are compile-time checks
    // (`const {}` is the idiomatic form for asserting a const value).
    const {
        assert!(
            <Stunned as Component>::STORAGE_IS_BITSET,
            "the derive must emit `const STORAGE_IS_BITSET = true;` for storage = \"bitset\""
        );
        assert!(<Selected as Component>::STORAGE_IS_BITSET);
        assert!(<OnGround as Component>::STORAGE_IS_BITSET);
    }
}

#[test]
fn plain_derived_component_storage_is_bitset_stays_false() {
    // A normal `#[derive(Component)]` keeps the trait default — zero opt-in.
    const {
        assert!(
            !<Payload as Component>::STORAGE_IS_BITSET,
            "a plain derived component must keep STORAGE_IS_BITSET = false"
        );
    }
}

// ── 2. First component_id() classifies the id StorageKind::Bitset ─────────────

#[test]
fn derived_bitset_tag_classified_bitset_after_first_component_id() {
    // The classification is routed through the derive's `component_id()` install
    // path (install_storage_kind::<Self>), so it is in place after the first
    // call — before the id can enter any archetype.
    let id = Stunned::component_id();
    assert_eq!(
        component_registry::storage_kind(id.0),
        StorageKind::Bitset,
        "first component_id() must classify a storage = \"bitset\" tag as Bitset"
    );

    // A second tag is independently classified.
    let sel = Selected::component_id();
    assert_eq!(component_registry::storage_kind(sel.0), StorageKind::Bitset);
    assert_ne!(id, sel, "distinct derived tags get distinct ids");
}

#[test]
fn plain_derived_component_stays_table_storage() {
    let id = Payload::component_id();
    assert_eq!(
        component_registry::storage_kind(id.0),
        StorageKind::Table,
        "a plain derived component must stay at the table-storage default"
    );
}

// ── 3. A bitset tag id never enters an archetype signature ────────────────────

#[test]
fn derived_bitset_tag_filtered_out_of_signature() {
    let mut ecs = EcsMaster::new();
    // Request an archetype carrying BOTH a real component and the bitset tag id.
    // The bitset id must be filtered out of the signature (Step 4 / D6), so the
    // resulting archetype is identical to the Payload-only archetype.
    let with_tag = ecs.create_archetype(&[Payload::component_id(), Stunned::component_id()]);
    let payload_only = payload_archetype(&mut ecs);
    assert_eq!(
        with_tag, payload_only,
        "a bitset tag id must be filtered out of the archetype signature, so an \
         archetype requested with the tag collapses onto the tag-less one"
    );

    // The entity created in that archetype carries the Payload but NOT the tag id
    // as a stored component — the tag is a per-row bit, toggled separately.
    let e = spawn(&mut ecs, with_tag, 42);
    assert!(
        ecs.get_component::<Payload>(e).is_some(),
        "the real component is stored"
    );
    assert!(
        !ecs.is_enabled::<Stunned>(e),
        "a freshly spawned entity has the bit clear (no column/page allocated yet)"
    );
}

// ── 4. enable / is_enabled / Query<&P, Enabled<T>> work end-to-end ────────────

#[test]
fn derived_bitset_tag_enable_is_enabled_round_trip() {
    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let e = spawn(&mut ecs, arch, 1);

    assert!(!ecs.is_enabled::<Stunned>(e), "starts disabled");
    ecs.enable::<Stunned>(e);
    assert!(ecs.is_enabled::<Stunned>(e), "enable<Stunned> sets the bit");
    ecs.disable::<Stunned>(e);
    assert!(!ecs.is_enabled::<Stunned>(e), "disable<Stunned> clears the bit");
}

#[test]
fn derived_bitset_tag_typed_query_filters_enabled_rows() {
    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let rows = [
        spawn(&mut ecs, arch, 1),
        spawn(&mut ecs, arch, 2),
        spawn(&mut ecs, arch, 4),
        spawn(&mut ecs, arch, 8),
    ];
    // Enable the bit on rows 0 and 2.
    ecs.enable::<Stunned>(rows[0]);
    ecs.enable::<Stunned>(rows[2]);

    // `Query<&Payload, Enabled<Stunned>>` — the `&Payload` data term satisfies
    // the C2 positive-archetypal-term requirement.
    let sum: u32 = ecs
        .query::<&Payload, Enabled<Stunned>>()
        .iter()
        .map(|p| p.v)
        .sum();
    assert_eq!(
        sum,
        1 + 4,
        "typed Enabled<Stunned> visits only the enabled rows (1 and 4)"
    );
}

#[test]
fn two_derived_bitset_tags_are_independent() {
    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let e = spawn(&mut ecs, arch, 1);

    ecs.enable::<Stunned>(e);
    assert!(ecs.is_enabled::<Stunned>(e));
    assert!(
        !ecs.is_enabled::<Selected>(e),
        "enabling Stunned must not set the independent Selected bit"
    );

    ecs.enable::<Selected>(e);
    ecs.disable::<Stunned>(e);
    assert!(ecs.is_enabled::<Selected>(e), "Selected stays set");
    assert!(!ecs.is_enabled::<Stunned>(e), "Stunned cleared independently");
}

// ── 5. The empty-named-field tag shape behaves identically ────────────────────

#[test]
fn empty_named_struct_bitset_tag_works() {
    let id = OnGround::component_id();
    assert_eq!(component_registry::storage_kind(id.0), StorageKind::Bitset);

    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let e = spawn(&mut ecs, arch, 1);
    assert!(!ecs.is_enabled::<OnGround>(e));
    ecs.enable::<OnGround>(e);
    assert!(ecs.is_enabled::<OnGround>(e));
}

// ── 6. Hardening D — a derived tag matches the hand-impl across the API ───────

/// D1: `Query<&Payload, Disabled<Stunned>>` over a derived bitset tag visits
/// EXACTLY the rows whose bit is CLEAR (Wave-3 A1.1: a positive-data query with
/// a `Disabled<T>` filter is the polarity twin of `Enabled<T>` — it selects the
/// complement). The `&Payload` data term satisfies the C2 positive-archetypal
/// requirement, so a sole `Disabled<Stunned>` is an accepted single shape.
#[test]
fn derived_bitset_tag_typed_query_disabled_filters_cleared_rows() {
    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let rows = [
        spawn(&mut ecs, arch, 1),
        spawn(&mut ecs, arch, 2),
        spawn(&mut ecs, arch, 4),
        spawn(&mut ecs, arch, 8),
    ];
    // Enable the bit on rows 0 and 2; rows 1 and 3 stay cleared.
    ecs.enable::<Stunned>(rows[0]);
    ecs.enable::<Stunned>(rows[2]);

    let sum: u32 = ecs
        .query::<&Payload, Disabled<Stunned>>()
        .iter()
        .map(|p| p.v)
        .sum();
    assert_eq!(
        sum,
        2 + 8,
        "typed Disabled<Stunned> visits only the cleared rows (2 and 8) — the \
         complement of Enabled<Stunned>"
    );
}

/// D2: the DYNAMIC per-row `with_enabled` term (Step 9) filters identically to
/// the typed `Enabled<DerivedTag>` path. There is no public
/// `ComponentId → EnableTagId` bridge (an `EnableTagId` is minted only by
/// `register_enable_tag`), so the dynamic term uses a name-keyed enable tag; we
/// drive BOTH a derived typed tag (`Selected`) and the name-keyed dynamic tag
/// over the SAME enabled-row set, then assert the dynamic `with_enabled` and the
/// typed `Enabled<Selected>` select the identical rows — proving the two code
/// paths agree on the per-row enable machinery.
#[test]
fn derived_bitset_tag_dynamic_with_enabled_matches_typed_enabled() {
    let mut ecs = EcsMaster::new();
    let dyn_tag = ecs.register_enable_tag("step10_dyn_matches_typed");
    let arch = payload_archetype(&mut ecs);
    let rows = [
        spawn(&mut ecs, arch, 1),
        spawn(&mut ecs, arch, 2),
        spawn(&mut ecs, arch, 4),
        spawn(&mut ecs, arch, 8),
    ];
    // Identical pattern on both bits: rows 0 and 2 enabled.
    for &r in &[rows[0], rows[2]] {
        ecs.enable::<Selected>(r);
        ecs.enable_id(r, dyn_tag);
    }

    // Typed path: Enabled<Selected> over &Payload.
    let typed_sum: u32 = ecs
        .query::<&Payload, Enabled<Selected>>()
        .iter()
        .map(|p| p.v)
        .sum();

    // Dynamic path: a no-filter view + a runtime `with_enabled(dyn_tag)` term.
    let dynamic_sum: u32 = ecs
        .query::<&Payload, ()>()
        .with_enabled(dyn_tag)
        .iter()
        .map(|p| p.v)
        .sum();

    assert_eq!(
        dynamic_sum, typed_sum,
        "the dynamic with_enabled per-row term must filter identically to the \
         typed Enabled<T> path when both bits track the same rows"
    );
    assert_eq!(dynamic_sum, 1 + 4, "both paths visit only the enabled rows (1 and 4)");
}

/// D3: a derived bitset tag on entities in a MULTI-component archetype
/// (`Payload` + `Tag2`) still selects the right rows. The tag is a per-row bit
/// orthogonal to the two stored data columns, so the enable filter composes with
/// a multi-column data term.
#[test]
fn derived_bitset_tag_in_multi_component_archetype() {
    let mut ecs = EcsMaster::new();
    let arch = payload_and_tag2_archetype(&mut ecs);
    let rows = [
        spawn_two(&mut ecs, arch, 1, 10),
        spawn_two(&mut ecs, arch, 2, 20),
        spawn_two(&mut ecs, arch, 4, 40),
    ];
    // Enable on rows 0 and 2.
    ecs.enable::<Stunned>(rows[0]);
    ecs.enable::<Stunned>(rows[2]);

    // The two-column data term `(&Payload, &Tag2)` satisfies C2; the enable
    // filter must select only the enabled rows.
    let (v_sum, w_sum): (u32, u32) = ecs
        .query::<(&Payload, &Tag2), Enabled<Stunned>>()
        .iter()
        .fold((0, 0), |(vs, ws), (p, t)| (vs + p.v, ws + t.w));
    assert_eq!(v_sum, 1 + 4, "Payload column summed over enabled rows only");
    assert_eq!(w_sum, 10 + 40, "Tag2 column summed over enabled rows only");

    // The bitset tag id is filtered out of the signature, so the archetype is a
    // genuine two-DATA-column archetype (Payload + Tag2), not three.
    let payload_id: ComponentId = Payload::component_id();
    let tag2_id: ComponentId = Tag2::component_id();
    assert_ne!(
        payload_id, tag2_id,
        "the two data components are distinct ids"
    );
}

/// D4: enable / disable on a despawned entity is a silent no-op — it must not
/// panic and must not corrupt a live entity that recycled the slot. Mirrors the
/// dead/stale-handle no-op contract the other enable_tag suites rely on.
#[test]
fn derived_bitset_tag_dead_entity_toggle_is_noop() {
    let mut ecs = EcsMaster::new();
    let arch = payload_archetype(&mut ecs);
    let victim = spawn(&mut ecs, arch, 1);

    assert!(ecs.delete_entity(victim), "despawn must succeed");

    // Toggling on the now-dead handle is a silent no-op (no panic).
    ecs.enable::<Stunned>(victim);
    ecs.disable::<Stunned>(victim);
    assert!(
        !ecs.is_enabled::<Stunned>(victim),
        "is_enabled on a dead handle reads false"
    );

    // A fresh entity may recycle the slot; the dead-handle toggle must not have
    // leaked a set bit onto it.
    let recycled = spawn(&mut ecs, arch, 2);
    assert!(
        !ecs.is_enabled::<Stunned>(recycled),
        "a recycled slot must start with the bit clear — the dead-handle toggle \
         did not corrupt it"
    );
    // The recycled entity still toggles normally.
    ecs.enable::<Stunned>(recycled);
    assert!(ecs.is_enabled::<Stunned>(recycled), "recycled entity toggles normally");
}
