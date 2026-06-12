// Phase 22 D7 — deriving BOTH `Component` and `Bundle` on the same type must
// fail: `#[derive(Component)]` now itself emits `impl Bundle for Self` (the
// single-component bundle), so the explicit `#[derive(Bundle)]` collides with
// it (E0119 duplicate `Bundle` / `BundleSealed` impls). The documented escape
// hatch is `#[component(no_bundle)]`, which suppresses the Component derive's
// Bundle emission and frees the type to be a multi-field bundle.

use boyko_macros::{Bundle, Component};

#[derive(Component)]
struct Inner(u32);

#[derive(Component, Bundle)]
struct DoubleDerive {
    inner: Inner,
}

fn main() {}
