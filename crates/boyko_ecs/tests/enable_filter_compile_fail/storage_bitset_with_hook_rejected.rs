// EnableTag D5 (Step 10 hardening A): a `#[component(storage = "bitset")]` tag
// combined with a structural lifecycle hook is a compile error. An enable-bit
// tag has no `ComponentPool`, so the hook (on_add/on_insert/on_replace/on_remove)
// could never fire — accepting it would install a dead hook (compile-but-lie).
// The derive fails loud, telling the user to drop the hook or the bitset storage.

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn some_fn(_world: DeferredEcsMaster<'_>, _ctx: HookContext) {}

#[derive(Component)]
#[component(storage = "bitset", on_add = some_fn)]
struct Bad;

fn main() {}
