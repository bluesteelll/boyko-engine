//! Phase 13 §6.3 — `Local<T>` requires `T: Default` (Decision B1).
//!
//! `NoDefault` has no `Default` impl, so `Local<'_, NoDefault>` fails the
//! `SystemParam` bound `T: Send + Sync + Default + 'static`. Building a system
//! whose parameter list contains it must be rejected at compile time.

use boyko_ecs::ecs::core::system::{IntoSystem, Local};

// No `#[derive(Default)]` and no manual `impl Default` — the load-bearing
// omission for this compile-fail case. (`Send + Sync + 'static` are all
// satisfied by this empty type, isolating the missing `Default`.)
struct NoDefault;

fn main() {
    // `into_system` requires every parameter to implement `SystemParam`;
    // `Local<NoDefault>`'s impl is gated on `NoDefault: Default`, which is
    // unsatisfied.
    let _sys = IntoSystem::into_system(|_: Local<NoDefault>| {});
}
