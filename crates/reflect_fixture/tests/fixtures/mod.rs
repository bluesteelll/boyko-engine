//! **BOUNDARY B0 — the shared fixture set behind the id-difference harness.**
//!
//! Eight `#[component(reflect)]` types, [`touch_all`] (the one canonical minting order),
//! [`ids_by_type`] (the fixed reporting order — a *separate* array, and the reason is
//! measured at its own doc comment), and [`ids`], the committed capture both binaries are
//! checked against.
//!
//! # Why this is `tests/fixtures/mod.rs` and not `tests/fixtures.rs`
//!
//! `crates/reflect_fixture/Cargo.toml` turns off `autobins` and `autobenches` — **not
//! `autotests`**. A `tests/fixtures.rs` would therefore be auto-discovered as a test
//! target of its own: a declaration-only binary printing `running 0 tests`, which is the
//! first of the four vacuity routes this campaign has measured, and it would not be
//! `mod`-includable by the two harness binaries besides. A directory holding `mod.rs` is
//! discovered by nothing (`docs/REFLECTION-PLAN-BOUNDARY.md` D22).
//!
//! # Why every type carries an explicit `stable_name`, in ONE `#[component(…)]`
//!
//! `Component::stable_name()` defaults to `std::any::type_name::<Self>()`
//! (`crates/boyko_ecs/src/ecs/core/component/component.rs:185`), which embeds **the test
//! binary's own crate name**. Separate `tests/*.rs` files are separate crates, so a
//! defaulted key reads `boundary_roundtrip::fixtures::T` in one half of this instrument
//! and `boundary_id_reorder::fixtures::T` in the other — and the whole boundary is
//! *name-keyed*, so the two halves would not agree on a single key (D21(a)). B5 gate 2's
//! byte-identity assertion would then red on a **key** difference having nothing to do
//! with ids, which is the exact confound that gate exists to exclude.
//!
//! It is also one attribute and not two: `#[component(reflect)]` written beside
//! `#[component(stable_name = "…")]` is `error: duplicate #[component(...)] attribute;
//! combine all hooks into one` (D21(b)).
//!
//! # Why `StrFixture` is not here
//!
//! B0 originally landed `StrFixture { s: String }`. A `#[component(reflect)]` item is
//! classified **at declaration**, so declaring it reds — `error[E0277]: the trait bound
//! `String: Reflect` is not satisfied` — and it takes this whole module, and therefore
//! *both* harness binaries, with it. `String: Reflect` is CORE C11; the type is B7's
//! (D20). Adding `#[reflect(skip)]` to make it compile is the trap, not the remedy: it
//! plants B7's own subject in the wrong shape six rungs early, where nothing observes it.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_macros::Component;

pub mod ids;

// ═══════════════════════════════════════ the fixture types ══════════════════════════════

/// The flat POD, and the type whose id both gates read. Three scalar kinds so B2's
/// `Prim` grammar has more than one width to get wrong.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::Pod3")]
pub struct Pod3 {
    /// `u32` lane.
    pub a: u32,
    /// `f32` lane.
    pub b: f32,
    /// `i16` lane — narrower than its neighbours, so a fixed-width encode is visible.
    pub c: i16,
}

/// The `Nested` case: a component whose field is itself a reflected component, plus a
/// trailing scalar so the descent cannot be mistaken for a whole-struct blit.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::NestPair")]
pub struct NestPair {
    /// The nested POD — B5 gate 1 descends `NestPair → Pod3`.
    pub inner: Pod3,
    /// The trailing scalar.
    pub tail: u8,
}

/// The `Array` case.
///
/// ⚠️ **A second, independent `ArrPack` exists** at
/// `crates/reflect_fixture/tests/c7_derive_bake.rs:162`, whose field is named `data`.
/// Separate `tests/*.rs` are separate crates so this is no collision, but a grep for the
/// type now returns two definitions differing only in the field name — and **B2 gate 3
/// and B5 gate 1 read `.m`**, this one.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::ArrPack")]
pub struct ArrPack {
    /// The array field. Named `m`, not `data`.
    pub m: [f32; 4],
}

/// The top-level-enum case: fieldless, `#[repr(u8)]`, discriminants **pinned** so a
/// variant inserted above another is a visible change rather than a silent renumbering.
///
/// ⚠️ **On today's tree this bakes `TypeKind::Opaque`, `fields = 0`, `enum_info = None`**
/// — MEASURED 2026-08-27, and pinned deliberately by
/// `crates/reflect_fixture/tests/reflect_pass/fieldless_repr_enum_accepted.rs`. Declaring
/// it here is fine; **B2 gate 4 and B5 gate 1's variant assertions are blocked on CORE
/// C10**, which is a dependency row the BOUNDARY plan did not carry until D26.
///
/// It is a **local** enum and not a re-export of `boyko_scene`'s `Visibility`: that crate
/// is outside this package's dependency table, which is the gate's argument (GATES D15)
/// and not a preference. The real `Visibility` is B5's dogfood half, in
/// `crates/reflect_dogfood/`.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::FixVis")]
#[repr(u8)]
pub enum FixVis {
    /// Discriminant 0, and the `Default`.
    #[default]
    Inherited = 0,
    /// Discriminant 1.
    Hidden = 1,
    /// Discriminant 2.
    Visible = 2,
}

/// The pin behind the word "pinned" above.
///
/// Written because the claim was otherwise prose, and because `Hidden`/`Visible` are
/// constructed nowhere at B0 — a fieldless enum whose non-default variants nothing names
/// is `warning: variants … are never constructed`, and the choice at that point is a
/// blanket `#[allow(dead_code)]` (a suppression) or a real assertion (this). B2 gate 4 and
/// B5 gate 1 set and re-read a variant across the wire; the discriminant each of them
/// travels as is the value below, so a renumbering here is a change to what those gates
/// compare, and this makes it a compile error rather than a silent one.
const _: () = {
    assert!(FixVis::Inherited as u8 == 0);
    assert!(FixVis::Hidden as u8 == 1);
    assert!(FixVis::Visible as u8 == 2);
};

/// The decoy: a distinct POD nothing in a correct dump ever writes.
///
/// It exists so a **mis-keyed apply corrupts something observable**. B5 gate 3 applies a
/// blob captured in the other process and asserts `Decoy` is unchanged: an id-keyed
/// implementation would either miss `Pod3` or write into whatever now holds that id, and
/// without a witness at that id a silent corruption reads as a pass.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::Decoy")]
pub struct Decoy {
    /// The witness lane.
    pub canary: u32,
    /// A second lane, so a partial write is distinguishable from a whole-struct one.
    pub tail: f32,
}

/// Half of B5 gate 4's field-permutation pair: `{ a, b }`.
///
/// ⚠️ Until D22 `PermA`/`PermB` lived only inside B5 gate 4's prose and appeared on **no
/// rung's Lands list anywhere**. They are `#[component(reflect)]` types, so this is the
/// module that holds them.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::PermA")]
pub struct PermA {
    /// First **by declaration**, and `u32`.
    pub a: u32,
    /// Second by declaration, and `f32`.
    pub b: f32,
}

/// The other half of B5 gate 4's pair: the same two fields, **declared in the opposite
/// order**, so `fields_of(PermA)[0].name != fields_of(PermB)[0].name` and a by-index
/// resolution transposes the values where a by-name one does not.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::PermB")]
pub struct PermB {
    /// First **by declaration**, and `f32`.
    pub b: f32,
    /// Second by declaration, and `u32`.
    pub a: u32,
}

/// The tuple struct, and the demonstrator D24's caveat needs.
///
/// **The caveat.** The wire is keyed by field *name* (D6/D8) and the derive names a tuple
/// struct's fields `"0"`, `"1"`, … So for a tuple struct **by-name resolution IS
/// by-position resolution**, and the refactor stability this boundary advertises —
/// *"reorder the fields, the dump still applies"* — **does not hold for them**. Swapping
/// two tuple fields of the same type silently swaps their values on apply; swapping two of
/// different types is caught by the kind check and is therefore loud, which is the only
/// part of the hazard v1 detects.
///
/// Both fields are `u32` deliberately: that is the *silent* half of the caveat, and a
/// demonstrator of the loud half would demonstrate the wrong thing.
///
/// ⚠️ Without this type the plan's only tuple struct is B5's dogfood `Name`, which an
/// owner "no" on B.13 #1 deletes outright — a caveat whose sole demonstrator an owner call
/// can delete is a caveat with no gate.
#[derive(Component, Default)]
#[component(reflect, stable_name = "reflect::fixture::PosPair")]
pub struct PosPair(
    /// Baked field name `"0"`.
    pub u32,
    /// Baked field name `"1"`.
    pub u32,
);

// ═════════════════════════════════════ the canonical order ══════════════════════════════

/// How many fixture types this module declares.
///
/// Not a hand-typed number that can drift: [`ids_by_type`] returns
/// `[usize; FIXTURE_TYPE_COUNT]` from an array literal naming every type, and
/// [`FIXTURE_NAMES`] is the same length, so declaring a ninth fixture and forgetting this
/// constant is a **compile error**, not a quiet disagreement. The reorder binary's budget
/// clause spends it.
pub const FIXTURE_TYPE_COUNT: usize = 8;

/// Mints (or re-reads) every fixture's `ComponentId`. **Statement order is minting
/// order**, and this is the only place either binary establishes it.
///
/// The two halves of the instrument must mint the same **set** in the same **order**,
/// differing only by the reorder prelude's shift. If each binary carried its own touch
/// sequence, a drift between them would look exactly like the shift the harness exists to
/// prove — so the sequence lives here, once.
///
/// It returns nothing. That is deliberate, and it is [`ids_by_type`]'s doc comment that
/// says why.
///
/// ⚠️ **This is the mutation site of B0's third RED** (the capture endpoint): move `Decoy`
/// ahead of `Pod3` here and `boundary_roundtrip.rs` reds while `boundary_id_reorder.rs`
/// stays green — the reorder prelude has already minted `Decoy` before it calls this. That
/// asymmetry is what proves the two files' assertions read two different endpoints instead
/// of one tautology.
pub fn touch_all() {
    <Pod3 as ComponentTrait>::component_id();
    <NestPair as ComponentTrait>::component_id();
    <ArrPack as ComponentTrait>::component_id();
    <FixVis as ComponentTrait>::component_id();
    <Decoy as ComponentTrait>::component_id();
    <PermA as ComponentTrait>::component_id();
    <PermB as ComponentTrait>::component_id();
    <PosPair as ComponentTrait>::component_id();
}

/// The fixture ids in a **fixed reporting order**, parallel to [`FIXTURE_NAMES`] — the
/// vector both binaries assert against.
///
/// ⚠️ **Reporting order and minting order are two different arrays ON PURPOSE, and the
/// reason is MEASURED, not stylistic.** As first written this rung had ONE function that
/// both minted and returned, from a single array literal. Element evaluation is
/// left-to-right, so that literal's *position* and its *evaluation order* move together:
/// the ids it returns are `[0, 1, …, 7]` for **every permutation of itself**, and the
/// vector assertion built on it was a gate that could not fail for the exact mutation it
/// was written to catch. MEASURED 2026-08-27, `rustc 1.97.1`, on B0's third RED (`Decoy`
/// moved ahead of `Pod3`): the vector printed `[0, 1, 2, 3, 4, 5, 6, 7]` and **passed**,
/// and only the separate `Pod3`-by-name assertion reded. Splitting the two arrays is what
/// makes a reordering of `touch_all` observable as a *permutation* of this vector.
///
/// It calls [`touch_all`] itself rather than trusting a caller to have called it, so the
/// reporting order can never quietly become the minting order by someone dropping a call.
/// `component_id()` is a once-per-type mint, so the repeat is a no-op.
pub fn ids_by_type() -> [usize; FIXTURE_TYPE_COUNT] {
    touch_all();
    [
        <Pod3 as ComponentTrait>::component_id().0,
        <NestPair as ComponentTrait>::component_id().0,
        <ArrPack as ComponentTrait>::component_id().0,
        <FixVis as ComponentTrait>::component_id().0,
        <Decoy as ComponentTrait>::component_id().0,
        <PermA as ComponentTrait>::component_id().0,
        <PermB as ComponentTrait>::component_id().0,
        <PosPair as ComponentTrait>::component_id().0,
    ]
}

/// The fixture names in [`ids_by_type`]'s reporting order, so a red prints *which* slot
/// moved instead of two anonymous integer vectors.
pub const FIXTURE_NAMES: [&str; FIXTURE_TYPE_COUNT] =
    ["Pod3", "NestPair", "ArrPack", "FixVis", "Decoy", "PermA", "PermB", "PosPair"];
