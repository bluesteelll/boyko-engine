// KM2 — `on_despawn` is now a valid `#[component(...)]` key, so the Phase-14a
// "deferred to Phase 14b" refusal (and the fixture that pinned it) is gone. What
// replaces it is the refusal KM2 *creates*: `on_despawn` joins the four other
// lifecycle hooks in the `storage = "bitset"` refusal, on exactly their ground —
// a bitset enable tag is excluded from every archetype signature, so
// `ArchetypeFlags` never ORs its hook bits in and the hook can never fire.
//
// Without this fixture the extension is untested and the combination would be a
// compile-but-lie: an accepted key wired to something that never runs.
//
// Expected diagnostic: "#[component(storage = \"bitset\")] cannot combine with
// lifecycle hooks (on_add/on_insert/on_replace/on_remove/on_despawn) ..."

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_macros::Component;

unsafe fn h(_w: DeferredEcsMaster<'_>, _c: HookContext) {}

#[derive(Component)]
#[component(storage = "bitset", on_despawn = h)]
struct Bad;

fn main() {}
