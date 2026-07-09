//! Boyko Engine — main binary.
//!
//! The engine itself lives in `crates/boyko_ecs`. This `main.rs` is a thin
//! placeholder so that `cargo build` produces a binary target. Real entry
//! points for testing and examples should go to `crates/boyko_ecs/tests/`
//! and `crates/boyko_ecs/benches/`.
//!
//! The previous content of this file (a hand-written test script written
//! against the old `ComponentPool<T>` API from the `master` branch) is
//! preserved in `archive/legacy_main.rs.txt` for reference. It will not
//! compile against the current type-erased `ComponentPool` API and should
//! be rewritten on top of `EcsMaster` when integration tests are added.

fn main() {
    println!("boyko-engine binary placeholder. See `crates/boyko_ecs` for the library.");
}
