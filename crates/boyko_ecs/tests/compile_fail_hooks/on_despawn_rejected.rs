// Phase 14a — `on_despawn` is deliberately removed from 14a (deferred to 14b).
// The derive macro emits a clear, dedicated error rather than letting it fall
// into the generic "unknown key" branch.
//
// Expected diagnostic: "on_despawn is not supported in this version (deferred
// to Phase 14b); valid keys: on_add, on_insert, on_replace, on_remove".

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn h(_w: DeferredEcsMaster<'_>, _c: HookContext) {}

#[derive(Component)]
#[component(on_despawn = h)]
#[repr(C)]
struct Bad(u32);

fn main() {}
