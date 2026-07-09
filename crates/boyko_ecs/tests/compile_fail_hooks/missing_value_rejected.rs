// Phase 14a — a hook key without its `= <path>` value is a compile error.
// `meta.value()` (which consumes the `=`) fails when there is no `=`.
//
// Expected diagnostic: a syn parse error at the `on_add` key — "expected `=`".

use boyko_macros::Component;

#[derive(Component)]
#[component(on_add)]
#[repr(C)]
struct Bad(u32);

fn main() {}
