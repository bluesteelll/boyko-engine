// Phase 14a — a duplicate hook key within one `#[component(...)]` attribute is
// a compile error (each kind may be set at most once).
//
// Expected diagnostic: "duplicate #[component(...)] key; each hook may be set
// at most once".

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn a(_w: DeferredEcsMaster<'_>, _c: HookContext) {}
unsafe fn b(_w: DeferredEcsMaster<'_>, _c: HookContext) {}

#[derive(Component)]
#[component(on_add = a, on_add = b)]
#[repr(C)]
struct Bad(u32);

fn main() {}
