// Phase 14a — more than one `#[component(...)]` attribute on the same item is a
// compile error (combine all hooks into one attribute).
//
// Expected diagnostic: "duplicate #[component(...)] attribute; combine all
// hooks into one".

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn a(_w: DeferredEcsMaster<'_>, _c: HookContext) {}
unsafe fn b(_w: DeferredEcsMaster<'_>, _c: HookContext) {}

#[derive(Component)]
#[component(on_add = a)]
#[component(on_remove = b)]
#[repr(C)]
struct Bad(u32);

fn main() {}
