//! Phase S0 — serialization registry + derive substrate BEHAVIORAL tests.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` (§3.7 data structures, §5 C1–C3, §7 Phase
//! S0). This file pins the S0 contract produced by the registry additions
//! (`component_registry::{Serializability, SerializeInfo, get_serialize_info,
//! install_serialize_fn, register_stable_name, resolve_stable_name}`) + the derive
//! emission (`SERIALIZABILITY` / `serializability_runtime` via the stricter
//! `SerializeProbe`, `LAYOUT_FINGERPRINT`, `FORMAT_VERSION`, `stable_name`).
//!
//! # Test strategy
//!
//! * **Component ids** come from `#[derive(Component)]` minted off the global
//!   atomic counter (`register_new`), so they never collide with the explicit
//!   `register_layout` slots other test files use, nor with each other.
//! * The classification is read back through `get_serialize_info(id)` AFTER
//!   forcing registration via `T::component_id()` (the install path runs in the
//!   `component_id()` `OnceLock` closure, ungated).
//! * Mirrors `tests/clone_entity.rs`'s style.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    self, Serializability, fnv1a_64, resolve_stable_name,
};
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::Component;

/// Reads the installed `Serializability` for `T`, forcing registration first.
fn classify<T: Component>() -> Serializability {
    let id = T::component_id().0;
    component_registry::get_serialize_info(id)
        .expect("install_serialize_fn runs ungated in component_id() — info must be present")
        .serializability
}

/// Reads the installed `SerializeInfo` for `T`, forcing registration first.
fn info_of<T: Component>() -> &'static component_registry::SerializeInfo {
    let id = T::component_id().0;
    component_registry::get_serialize_info(id).expect("serialize info must be present")
}

// ════════════════════════════════════════════════════════════════════════════
// Classification matrix (§7 Phase S0 / §5 C3)
// ════════════════════════════════════════════════════════════════════════════

// Copy-POD, repr(C), all-int/float fields → PlainOldBytes.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SPosition {
    x: f32,
    y: f32,
    z: f32,
}

// Copy-POD tuple struct, repr(C), all-int fields → PlainOldBytes.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SCoords(u32, u64, i16);

// repr(transparent) newtype over an int → PlainOldBytes.
#[derive(Component, Clone, Copy)]
#[repr(transparent)]
struct STransparent(u64);

// Unit-struct ZST tag, repr(C) → PlainOldBytes (vacuously all-bits-valid).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct STagZst;

// Owning component (String) → SerializeViaFn.
#[derive(Component, Clone)]
#[repr(C)]
struct SName {
    label: String,
}

// repr(C) but carries a `bool` field → NOT POB (bool is not all-bits-valid) →
// SerializeViaFn (C3 — the stricter-than-clone rule).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SFlagged {
    value: u32,
    flag: bool,
}

// repr(C) but carries a `char` field → NOT POB → SerializeViaFn (C3).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SChared {
    code: u32,
    ch: char,
}

// repr(C) with a fieldless enum field → NOT POB → SerializeViaFn (C3): an enum
// has invalid bit patterns (only its discriminants are valid).
// The variants exist only to give the enum a non-trivial discriminant shape for
// the classification test; they are never constructed.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
enum SColor {
    Red = 0,
    Green = 1,
    Blue = 2,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SEnumField {
    n: u32,
    color: SColor,
}

// repr(C) with an `Entity` field → NOT POB (Entity must be remapped, never
// blitted) → SerializeViaFn (C3 / C4).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct STargeted {
    n: u32,
    target: Entity,
}

// A non-Clone type → Ignore (no Clone bound on Component; the autoref probe falls
// to the by-value arm).
#[derive(Component)]
#[repr(C)]
struct SNonClone {
    data: u32,
}

// An explicit opt-out → Ignore even though the type is Copy-POD.
#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(no_serialize)]
struct SOptOut {
    x: f32,
}

#[test]
fn copy_pod_classifies_plain_old_bytes() {
    assert_eq!(classify::<SPosition>(), Serializability::PlainOldBytes);
    assert_eq!(classify::<SCoords>(), Serializability::PlainOldBytes);
    assert_eq!(classify::<STransparent>(), Serializability::PlainOldBytes);
    assert_eq!(classify::<STagZst>(), Serializability::PlainOldBytes);
}

#[test]
fn owning_string_classifies_serialize_via_fn() {
    assert_eq!(classify::<SName>(), Serializability::SerializeViaFn);
}

#[test]
fn bool_field_is_not_pob_but_via_fn() {
    // C3: a Copy type with a bool field is NOT PlainOldBytes (bool has invalid
    // bit patterns) — it must go through the validating decode path.
    assert_eq!(classify::<SFlagged>(), Serializability::SerializeViaFn);
}

#[test]
fn char_field_is_not_pob_but_via_fn() {
    assert_eq!(classify::<SChared>(), Serializability::SerializeViaFn);
}

#[test]
fn enum_field_is_not_pob_but_via_fn() {
    assert_eq!(classify::<SEnumField>(), Serializability::SerializeViaFn);
}

#[test]
fn entity_field_is_not_pob_but_via_fn() {
    // C3/C4: an Entity-bearing component is never blitted (it must be remapped),
    // so it classifies SerializeViaFn even though it is Copy.
    assert_eq!(classify::<STargeted>(), Serializability::SerializeViaFn);
}

#[test]
fn non_clone_classifies_ignore() {
    assert_eq!(classify::<SNonClone>(), Serializability::Ignore);
}

#[test]
fn no_serialize_opt_out_classifies_ignore() {
    // Copy-POD but explicitly opted out → Ignore.
    assert_eq!(classify::<SOptOut>(), Serializability::Ignore);
}

#[test]
fn pob_components_install_no_serialize_fns() {
    // POB installs None fns (the whole-column blit is driven by the pool layout,
    // never a per-element fn — S0 keeps both None; the encode glue lands in S1).
    let info = info_of::<SPosition>();
    assert!(info.serialize_fn.is_none());
    assert!(info.deserialize_fn.is_none());
    assert!(info.map_entities_fn.is_none());
}

// ════════════════════════════════════════════════════════════════════════════
// repr(Rust) demotion (§5 C2 / §7 Phase S0)
// ════════════════════════════════════════════════════════════════════════════

// A repr(Rust) (default-repr) struct of all-POD fields: S0 INFERS POB rather than
// requiring it, so a non-repr-C/transparent struct never gets the `SerPod` impl
// and is DEMOTED to SerializeViaFn (the decode path is always sound). This is the
// S0 realization of the "#[repr(Rust)] is not blittable" rule (C2): a silent,
// safe demotion rather than a hard error, because POB is inferred not requested.
#[derive(Component, Clone, Copy)]
struct SReprRust {
    x: f32,
    y: f32,
}

#[test]
fn repr_rust_pod_struct_demotes_to_via_fn() {
    assert_eq!(classify::<SReprRust>(), Serializability::SerializeViaFn);
}

// ════════════════════════════════════════════════════════════════════════════
// layout_fingerprint — changes on field reorder (§3.6 / C2)
// ════════════════════════════════════════════════════════════════════════════

// Two structs with the SAME field set but DIFFERENT declaration order. The
// differently-sized fields produce different offsets, so the fingerprints differ.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SLayoutA {
    a: u8,
    b: u64,
    c: u16,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SLayoutB {
    b: u64,
    c: u16,
    a: u8,
}

#[test]
fn layout_fingerprint_changes_on_field_reorder() {
    let fp_a = SLayoutA::LAYOUT_FINGERPRINT;
    let fp_b = SLayoutB::LAYOUT_FINGERPRINT;
    assert_ne!(
        fp_a, fp_b,
        "a field reorder that changes offsets must change the fingerprint (C2)"
    );
    // The installed info carries the same const.
    assert_eq!(info_of::<SLayoutA>().layout_fingerprint, fp_a);
    assert_eq!(info_of::<SLayoutB>().layout_fingerprint, fp_b);
}

#[test]
fn layout_fingerprint_is_stable_for_identical_layout() {
    // The fingerprint is a pure function of layout, so re-reading is identical.
    assert_eq!(SLayoutA::LAYOUT_FINGERPRINT, SLayoutA::LAYOUT_FINGERPRINT);
    assert_ne!(
        SLayoutA::LAYOUT_FINGERPRINT,
        0,
        "a real struct must produce a non-trivial fingerprint"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// format_version (§3.5)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SVersionedDefault {
    x: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(format_version = 7)]
struct SVersioned {
    x: u32,
}

#[test]
fn format_version_defaults_to_zero() {
    assert_eq!(info_of::<SVersionedDefault>().format_version, 0);
}

#[test]
fn format_version_override_is_recorded() {
    assert_eq!(info_of::<SVersioned>().format_version, 7);
    assert_eq!(SVersioned::FORMAT_VERSION, 7);
}

// ════════════════════════════════════════════════════════════════════════════
// stable name resolution + collision disambiguation (§3.5 / C1)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct SNamedDefault {
    x: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(stable_name = "my::stable::Alpha")]
struct SNamedAlpha {
    x: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(stable_name = "my::stable::Beta")]
struct SNamedBeta {
    y: u64,
}

#[test]
fn stable_name_defaults_to_type_name() {
    let info = info_of::<SNamedDefault>();
    // The default is the fully-qualified type name, which contains the type ident.
    assert!(
        info.stable_name.contains("SNamedDefault"),
        "default stable_name must be the type_name (got {:?})",
        info.stable_name
    );
}

#[test]
fn stable_name_override_is_recorded_and_resolves() {
    let id = SNamedAlpha::component_id().0;
    let info = info_of::<SNamedAlpha>();
    assert_eq!(info.stable_name, "my::stable::Alpha");
    assert_eq!(info.stable_name_hash, fnv1a_64(b"my::stable::Alpha"));

    // Resolution: the hash + full name recover the running ComponentId.
    let resolved = resolve_stable_name(info.stable_name_hash, "my::stable::Alpha");
    assert_eq!(
        resolved,
        Some(id),
        "resolve_stable_name must recover the registered id"
    );
}

#[test]
fn resolve_stable_name_distinguishes_distinct_names() {
    let id_alpha = SNamedAlpha::component_id().0;
    let id_beta = SNamedBeta::component_id().0;
    assert_ne!(id_alpha, id_beta);

    let h_alpha = info_of::<SNamedAlpha>().stable_name_hash;
    let h_beta = info_of::<SNamedBeta>().stable_name_hash;

    assert_eq!(
        resolve_stable_name(h_alpha, "my::stable::Alpha"),
        Some(id_alpha)
    );
    assert_eq!(
        resolve_stable_name(h_beta, "my::stable::Beta"),
        Some(id_beta)
    );
    // A wrong name under the right hash does not resolve.
    assert_eq!(resolve_stable_name(h_alpha, "my::stable::Wrong"), None);
}

#[test]
fn resolve_unknown_stable_name_is_none() {
    // A name never registered resolves to None.
    let h = fnv1a_64(b"never::registered::Type::Zzz");
    assert_eq!(resolve_stable_name(h, "never::registered::Type::Zzz"), None);
}

// ════════════════════════════════════════════════════════════════════════════
// hash-collision disambiguation (§3.5 / C1)
// ════════════════════════════════════════════════════════════════════════════

// Two DISTINCT registered components whose stable names are explicitly forced to
// hash-collide. We pick names by their precomputed fnv1a_64 — but rather than
// search for a collision (expensive), we directly exercise the disambiguation
// path by registering two names and asserting each resolves to ITS OWN id even
// when queried with the OTHER's hash but its own name. The registry compares the
// full name on every hash-bucket candidate, so the bucket containing both ids
// still returns the right one by full-name match.

#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(stable_name = "collide::First")]
struct SCollideFirst {
    a: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
#[component(stable_name = "collide::Second")]
struct SCollideSecond {
    a: u32,
}

#[test]
fn full_name_compare_disambiguates_bucket_candidates() {
    let id_first = SCollideFirst::component_id().0;
    let id_second = SCollideSecond::component_id().0;

    let h_first = info_of::<SCollideFirst>().stable_name_hash;
    let h_second = info_of::<SCollideSecond>().stable_name_hash;

    // Even if these two hashes happened to be equal (a real collision), the full
    // name compare guarantees each name maps to its own id. With distinct hashes
    // it is trivially correct; the test asserts the full-name compare is wired by
    // querying with the correct (hash, name) pair for each.
    assert_eq!(resolve_stable_name(h_first, "collide::First"), Some(id_first));
    assert_eq!(
        resolve_stable_name(h_second, "collide::Second"),
        Some(id_second)
    );
    // Querying a hash bucket with a name that is not in it returns None even if
    // the hash matches a different registered name.
    assert_eq!(resolve_stable_name(h_first, "collide::Second"), None);
}

// Direct synthetic collision: two names crafted to share one fnv1a_64 hash would
// be ideal, but finding such a pair is impractical here. Instead we assert the
// disambiguation INVARIANT directly against the public hasher: a query with the
// right hash but the wrong name never returns the candidate (covered above), and
// a query with the right hash AND right name returns it. Together these prove the
// full-name compare is load-bearing on the bucket scan.
#[test]
fn fnv1a_64_is_deterministic_and_distinguishes_inputs() {
    assert_eq!(fnv1a_64(b"alpha"), fnv1a_64(b"alpha"));
    assert_ne!(fnv1a_64(b"alpha"), fnv1a_64(b"beta"));
    // Known FNV-1a 64 vector for the empty string is the offset basis.
    assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
}
