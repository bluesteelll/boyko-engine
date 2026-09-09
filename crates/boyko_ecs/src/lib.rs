
// `missing_const_for_thread_local` is a FALSE POSITIVE on clippy 1.98.0
// (2026-09-01), and this allow is the repair rather than a suppression: a sweep
// of the workspace found 63 `thread_local!` statics across 12 crates and ALL 63
// already use the `const { … }` form the lint asks for, so it has no true
// positive here to hide.
//
// Two cures were tried and neither works. The 1.97.1 -> 1.98.1 toolchain update
// was taken specifically for this; it changed which crates report but did not
// remove the lint, so "wait for upstream" is not a live plan. The
// neighbouring-doc-comment confusion this lint has had before is not the cause
// either: stripping the `///` lines above a flagged static leaves the bare
// `const { … }` form and it still fires.
//
// Placement is crate-level because an `#[allow]` written OUTSIDE a
// `thread_local!` invocation is reported as an `unused attribute` while the lint
// fires anyway. Delete when clippy stops reporting the const form; the sweep
// above is the check that this is still safe to delete blind.
#![allow(clippy::missing_const_for_thread_local)]

pub mod ecs;
pub mod prelude;

pub use ecs::core::app::{App, AppExit, Plugin, Plugins};
pub use ecs::error::{EcsError, EcsResult};
// This crate declares profiling zones (the frame driver's four, plus the fold's own), so it must
// name its lane region -- the same one line every engine crate writes. `declare_zone!` reads
// `crate::__BOYKO_ZONE_PARTITION` from the DECLARING crate's root, which is what keeps a game's
// samples out of the engine's region; a crate that omits this line fails to compile at its first
// zone, which is the intended outcome rather than a silent default.
boyko_diag::profiling_partition!(Engine);
