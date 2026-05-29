// Phase 14a — an unknown `#[component(...)]` key is a compile error.
//
// Expected diagnostic: "unknown #[component(...)] key; valid keys: on_add,
// on_insert, on_replace, on_remove".

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn h(_w: DeferredEcsMaster<'_>, _c: HookContext) {}

#[derive(Component)]
#[component(bogus = h)]
#[repr(C)]
struct Bad(u32);

fn main() {}
