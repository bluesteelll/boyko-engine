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
//! * [`type_info`] — [`TypeInfo`] / [`FieldInfo`] and the descriptors they point at,
//!   plus [`validate`]'s coherence rules (CORE C3). C2's opaque placeholder is gone:
//!   the descriptor is now constructible by any consumer, because C7's derive must
//!   bake one from a downstream crate.
//! * [`prim`] — the monomorphic `get_*`/`set_*` accessor library a `Prim` field's
//!   fn-pointer slots are filled from, carrying the **release** kind check (CORE C4)
//!   and the read-shared/write-raw asymmetry both halves of it depend on.

pub mod prim;
pub mod registry;
pub mod scalar;
pub mod type_info;

pub use registry::{install_type_info, type_info_of};
pub use scalar::{Scalar, ScalarKind};
pub use type_info::{
    ArrayInfo, EnumInfo, EnumRepr, FieldInfo, Problem, TypeInfo, TypeKind, ValueKind, VariantInfo,
    Violation, validate,
};
