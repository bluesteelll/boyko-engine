//! `aether-tests` — the Aether DSL's integration surface (Decision A2's third crate).
//!
//! Deliberately an EMPTY library: everything lives in `tests/`, where `aether!` blocks compile
//! against the real `boyko_ecs` + `boyko_macros` + `boyko_render` and drive a real `EcsMaster`.
//! This crate exists so those tests have a Cargo home whose dependency set includes the engine
//! while the language crates (`aether-lang`, `aether`) stay engine-free (the tokens-not-deps rule).
//!
//! `boyko_render` is the third engine dependency (rung A5): `material` emits
//! `::boyko_render::Material` tokens, so the §8 R4 anti-drift gate for that construct lives here
//! — see the `Cargo.toml` note for what that pulls in.
