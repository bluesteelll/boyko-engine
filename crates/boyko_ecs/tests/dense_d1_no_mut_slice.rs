//! Dense plan D1 — SP4 un-typeability proof.
//!
//! Two parts:
//!  1. `static_assertions`: `DenseSolveView` is `Send + Sync` (the parallel
//!     primitive) and `DenseBuildView` is `!Send` (the structural surface stays
//!     single-threaded). Compile-time, in the normal build.
//!  2. trybuild compile-fail cases under `tests/dense_d1_no_mut_slice/`: a
//!     `DenseSolveView` cannot produce a whole-buffer `&mut [T]` — the SP4
//!     reborrow is un-typeable. Each `.rs` is compiled in isolation against its
//!     `.stderr` baseline.
//!
//! Regenerate baselines after a rustc point release:
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo +stable-x86_64-pc-windows-gnu test -p boyko-ecs --test dense_d1_no_mut_slice
//! ```

use boyko_ecs::ecs::core::component::dense::{DenseBuildView, DenseSolveView};
use static_assertions::{assert_impl_all, assert_not_impl_any};

// SP4 fix part 1 — the auto-trait split:
//  * the solve view is the parallel primitive → Send + Sync.
assert_impl_all!(DenseSolveView<'static>: Send, Sync, Copy);
//  * the build view is the single-threaded structural surface → !Send / !Sync.
assert_not_impl_any!(DenseBuildView<'static>: Send, Sync);

// SP4 fix part 2 — the un-typeable whole-buffer reborrow (trybuild).
//
// Gated behind `#[cfg(not(miri))]`: trybuild's driver is not wired under Miri
// (mirrors the other `*_compile_fail.rs` harnesses).
#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/dense_d1_no_mut_slice/*.rs");
}
