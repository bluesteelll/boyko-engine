//! `aether-tests` — the Aether DSL's integration surface (Decision A2's third crate).
//!
//! Deliberately an EMPTY library: everything lives in `tests/`, where `aether!` blocks compile
//! against the real `boyko_ecs` + `boyko_macros` and drive a real `EcsMaster`. This crate exists
//! so those tests have a Cargo home whose dependency set includes the engine while the language
//! crates (`aether-lang`, `aether`) stay engine-free (the tokens-not-deps rule).
