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
//! # Contents
//!
//! * [`scalar`] — `Scalar` / `ScalarKind`, the 16-byte POD value cell (CORE C1).
//! * [`registry`] — the dense `ComponentId`-indexed `REFLECT` table behind
//!   [`install_type_info`] / [`type_info_of`] (CORE C2). `install_type_info` keeps
//!   G0's stub name and signature — the artifact census's needle B is that name
//!   precisely because it survives the replacement (GATES D5).

pub mod registry;
pub mod scalar;

pub use registry::{install_type_info, type_info_of};
pub use scalar::{Scalar, ScalarKind};

/// Opaque placeholder for the reflection type descriptor.
///
/// Replaced by CORE C3's real `TypeInfo` (name, layout, kind, `&'static [FieldInfo]`,
/// accessors). Deliberately not constructible outside this crate: until C3, nothing
/// downstream can install one, so nothing can observe a half-built registry — which is
/// also why the C2 registry gates live in `registry`'s own unit-test module rather
/// than in `tests/`.
#[non_exhaustive]
pub struct TypeInfo;
