//! `boyko_reflect` — build-gated runtime reflection for the EDITOR, absent from the
//! shipped game.
//!
//! # The gating mechanism (CORE D1/D2 — inherited from `docs/REFLECTION-ANALYSIS.md` §0/§2)
//!
//! "Reflection only in a Debug build" is NOT a compiler property: `cfg(debug_assertions)`
//! cannot control whether a crate is in the dependency graph, and it is on for every plain
//! `cargo build`. This crate is therefore an OPTIONAL dependency behind a Cargo feature
//! that every consumer names exactly `reflect`
//! (`boyko-reflect = { path = "…", optional = true }`, `reflect = ["dep:boyko-reflect"]`).
//! Feature off ⇒ the crate is not in the consumer's resolved dependency closure at all —
//! not compiled, no rlib, no symbols. That absence is DEMONSTRATED by gates
//! (`docs/REFLECTION-PLAN-GATES.md` G1/G2/G3), never asserted.
//!
//! # The directional rule (the invariant that keeps "editor-only" honest)
//!
//! **`boyko_serialize` and every shipping crate must not depend on `boyko_reflect`.**
//! Save/load, replication and baked prefabs are compile-time codegen (`boyko_serialize`),
//! never runtime reflection; `crates/boyko_serialize/Cargo.toml` asserts this at the
//! source (*"never `boyko_reflect` (the codegen-not-reflection invariant)"*). Engine
//! crates MAY carry a non-default `reflect` feature plus an optional edge to this crate
//! (`docs/REFLECTION-ANALYSIS.md` B.12, option (b)); what nothing may do is ENABLE it on
//! a ship path — the property that defeats feature unification is *"nothing enables it"*,
//! not *"nobody declares it"*.
//!
//! # Contents at G0
//!
//! Deliberately hollow (GATES G0): one stub that exists to be a census subject three
//! rungs before the registry does. CORE C2 replaces the stub's body, KEEPING the name and
//! the signature — the artifact census's needle B is `install_type_info` precisely
//! because the name survives that replacement (GATES D5).

// Unused until CORE C2 imports `MAX_COMPONENTS` (CORE D5: imported, never redeclared).
// Anchored at the skeleton rung so the dependency graph this crate is allowed is fixed
// from the first commit (CORE D18: `boyko_ecs` and `std` only).
use boyko_ecs as _;

/// Opaque placeholder for the reflection type descriptor.
///
/// Replaced by CORE C3's real `TypeInfo` (name, layout, kind, `&'static [FieldInfo]`,
/// accessors). Deliberately not constructible outside this crate: at G0 nothing can
/// install one, so nothing can observe a half-built registry.
#[non_exhaustive]
pub struct TypeInfo;

/// Install `T`'s `TypeInfo` under its dense `ComponentId` index.
///
/// **G0 stub — deliberately hollow** (GATES G0): it exists so the ship-absence artifact
/// census (GATES G3) has a plain-`fn` subject (needle B, GATES D5) three rungs before the
/// real registry lands. CORE C2 replaces this body with the write-once
/// `[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]` install (first-writer-wins, bounds
/// discipline copied from `install_bind_accessor`), keeping this exact name and this
/// exact signature — `component_id: usize` per the in-tree installer convention
/// (CORE D6: the derive calls installers as `…::component_id().0`).
#[inline(never)]
pub fn install_type_info(component_id: usize, info: &'static TypeInfo) {
    // Hollow on purpose; see the doc comment. The bindings keep the signature honest
    // without inventing a body C2 would only delete.
    let _ = (component_id, info);
}
