//! ScratchColumn — SP4 un-typeability proof (mirrors `dense_d1_no_mut_slice`).
//!
//! Two parts:
//!  1. `static_assertions`: `ScratchSolveView` is `Send + Sync + Copy` (the
//!     parallel primitive) and `ScratchBuildView` is `!Send` / `!Sync` (the
//!     refill surface stays single-threaded). Compile-time, in the normal build.
//!  2. trybuild compile-fail cases under `tests/scratch_no_mut_slice/`: a
//!     `ScratchSolveView` cannot produce a whole-buffer `&mut [T]` — the SP4
//!     reborrow is un-typeable. Each `.rs` is compiled in isolation against its
//!     `.stderr` baseline.
//!
//! Regenerate baselines after a rustc point release:
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo +stable-x86_64-pc-windows-gnu test -p boyko-ecs --test scratch_no_mut_slice
//! ```

use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchSolveView};
use static_assertions::{assert_impl_all, assert_not_impl_any};

// SP4 fix part 1 — the auto-trait split:
//  * the solve view is the parallel primitive → Send + Sync + Copy.
assert_impl_all!(ScratchSolveView<'static, f32>: Send, Sync, Copy);
//  * the build view is the single-threaded refill surface → !Send / !Sync.
assert_not_impl_any!(ScratchBuildView<'static, f32>: Send, Sync);

// SP4 fix part 2 — the un-typeable whole-buffer reborrow (trybuild).
//
// Gated behind `#[cfg(not(miri))]`: trybuild's driver is not wired under Miri
// (mirrors the dense + other `*_compile_fail.rs` harnesses).
#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/scratch_no_mut_slice/*.rs");
}
