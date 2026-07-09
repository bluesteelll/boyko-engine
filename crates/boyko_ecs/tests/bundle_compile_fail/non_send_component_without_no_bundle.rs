// Phase 22 D7 — a `!Send` / `!Sync` component derived WITHOUT
// `#[component(no_bundle)]` must fail: the derive's single-component Bundle
// emission imposes `Bundle`'s `Send + Sync + Unpin` supertrait obligations.
//
// TWO diagnostics are expected, and their relative order is NOT guaranteed
// across rustc versions (see the driver header in bundle_compile_fail.rs):
//
//   1. The named const-assert E0277 — the readable, comment-bearing anchor:
//      `_boyko_component_as_bundle_requires_send_sync_unpin`.
//   2. The impl-level supertrait E0277 from `impl Bundle for ExoticComponent`
//      (supertrait obligations on a concrete impl cannot be silenced).
//
// The fix the user is steered to: `#[component(no_bundle)]` (the type stays a
// full Component, it just is not spawnable as a bare bundle).

use std::rc::Rc;

use boyko_macros::Component;

#[derive(Component)]
struct ExoticComponent {
    shared: Rc<u32>,
}

fn main() {}
