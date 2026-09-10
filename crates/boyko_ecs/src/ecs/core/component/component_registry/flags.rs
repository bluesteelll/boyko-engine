//! Initial-flag-state sub-registry (KE10 — the component `flags (…)` group).
//!
//! The **enable-bit twin of [`REQUIRES_DIRECT`]**: one write-once cold table
//! recording, per component, the enable tags that component wants set (or left
//! clear) the moment it is attached to an entity. Structured deliberately as a
//! narrowing of the `required` precedent next door, and the two narrowings are
//! the whole design:
//!
//! | | `required` | `flags` (this module) |
//! |---|---|---|
//! | direct table | `REQUIRES_DIRECT` | [`FLAGS_DIRECT`] |
//! | transitive closure | `REQUIRES_ALL` + memoized DFS + a cycle break | **none** |
//! | payload | a [`RequiredCtor`] that writes BYTES | an `initial: bool` — a BIT |
//!
//! **No transitive closure.** A required component pulls its own requires, so
//! the closure exists and needs a DFS, a `BUILDING` cycle stack and a W1
//! conflict rule. A flag pulls nothing: it is one bit at one `(archetype, row)`
//! address, and a flag is a `StorageKind::Bitset` id, which owns no attach
//! behaviour of its own — nothing can hang off it. So there is no `FLAGS_ALL`,
//! no DFS, no cycle break, and no `BUILDING` thread-local. A `#[require]`-style
//! cycle is not representable here, which is why this file is a third the size
//! of [`required`](super::required).
//!
//! **No ctor.** `RequiredCtor` exists because a required component must be
//! MATERIALIZED — someone has to write a value of the right type into an
//! uninitialized pool slot, which is a type-erased `unsafe fn(*mut u8)`. A flag
//! has no bytes: the whole state is `on` / `off`, so the entry carries a plain
//! `bool` and the application path is one `EnableColumn::set`. There is
//! therefore nothing `unsafe` in this module at all.
//!
//! **What IS kept from the precedent**, and why:
//!
//! * The id is stored as an **UNCALLED resolver** ([`FlagIdFn`]), exactly as
//!   BUG-REQ-CYCLE-1 requires of `REQUIRES_DIRECT`. [`install_flags`] runs
//!   inside the OWNER's `component_id()` `OnceLock::get_or_init`; calling
//!   `Flag::component_id()` there would re-enter that mid-init `OnceLock` on the
//!   same thread and deadlock if the flag's own registration ever reached back.
//!   The absence of a closure removes the *cycle* hazard, not the *re-entrancy*
//!   one, so the resolver stays uncalled until the attach path invokes it.
//! * The write-once `OnceLock` table with a first-writer-wins re-install, the
//!   `MAX_COMPONENTS`-bounded leak, and the `HAS_FLAGS` const gate on the
//!   install call — the 0%-gate. A component that declares no `flags (…)` group
//!   leaves its slot UNSET, which reads as an empty slice everywhere
//!   downstream, and the derive emits no install call at all.
//!
//! [`REQUIRES_DIRECT`]: super::required
//! [`RequiredCtor`]: super::RequiredCtor

use std::sync::OnceLock;

use crate::ecs::core::component::component::{Component, FlagsBuilder};
use crate::ecs::identifiers::primitives::ComponentId;

use super::MAX_COMPONENTS;

/// A capture-free resolver returning a flag's [`ComponentId`]. Stored UNCALLED
/// in [`FlagDirectEntry`] for the reason [`RequiredIdFn`](super::RequiredIdFn)
/// is: [`install_flags`] runs inside the declaring component's own
/// `component_id()` `OnceLock` init, and resolving the flag's id there could
/// re-enter that init on the same thread. The id is resolved LAZILY on the
/// attach path, well outside any `component_id` init.
pub type FlagIdFn = fn() -> ComponentId;

/// One DIRECT `flags (X = on | off)` declaration.
///
/// 16 B POD: an 8 B [`FlagIdFn`] resolver plus a `bool` and its padding.
/// `#[repr(C)]` pins the field order; `Copy` so the slice plumbing never invokes
/// drop glue. The `bool` is where the `required` twin carries a
/// [`RequiredCtor`](super::RequiredCtor) — a bit, not bytes.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FlagDirectEntry {
    /// The flag's id resolver, called lazily on the attach path.
    pub id_fn: FlagIdFn,
    /// The state the bit is set to when the declaring component is attached.
    /// `false` is NOT a no-op: it clears a bit an earlier attach may have set.
    pub initial: bool,
}

// Mirrors `required.rs`'s size pins. Gated to 64-bit (the engine's supported
// platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<FlagDirectEntry>() == 16);

/// KE10 D1: write-once table of each component's DIRECT `flags (…)`
/// declarations. Mirrors `REQUIRES_DIRECT`'s declaration; populated by
/// [`install_flags`] from the derive-generated registration path, gated on
/// [`Component::HAS_FLAGS`].
///
/// There is deliberately no `FLAGS_ALL` companion — see the module docs.
static FLAGS_DIRECT: [OnceLock<&'static [FlagDirectEntry]>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Installs `C`'s DIRECT `flags (…)` declarations into
/// `FLAGS_DIRECT[component_id]`. Builds the entry slice via
/// [`Component::register_flags`] and leaks it once (`&'static`), mirroring
/// [`install_required`](super::install_required)'s write-once discipline.
///
/// Called from the derive-generated `component_id()` ONLY when
/// `C::HAS_FLAGS` is true (const-gated, like `install_required`): a component
/// declaring no flags leaves the slot UNSET, which reads as "no initial flag
/// states" everywhere downstream — the 0%-gate. The leak is bounded by
/// `MAX_COMPONENTS` (one slice per declaring component per process).
#[inline]
pub fn install_flags<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let mut builder = FlagsBuilder::new();
    C::register_flags(&mut builder);
    let leaked: &'static [FlagDirectEntry] = Box::leak(builder.into_entries());
    // Write-once; a same-id re-install is a silent no-op (first writer wins,
    // matching `install_required` / `install_hooks` / `register_new`).
    let _ = FLAGS_DIRECT[component_id].set(leaked);
}

/// Installs a flag declaration set for `component_id` WITHOUT a `Component`
/// bound (the runtime twin of [`install_flags`], mirroring
/// `try_set_hooks`/`register_hooks_by_id`).
///
/// KE10 ships the kernel half ahead of its author-facing producer: the Aether
/// `flags (…)` group and the matching derive key are rung R3 work, and the value
/// vocabulary is still on owner ballot AB-13. Without this entry point the table
/// would be unreachable and therefore untestable — the "a gate that cannot fail"
/// shape, one level down. Returns `false` if the slot was already written (the
/// table is write-once per id, exactly like `HOOKS`).
///
/// Cold: registration-time only.
#[cold]
pub fn try_set_flags_direct(component_id: usize, entries: &[FlagDirectEntry]) -> bool {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return false;
    }
    let leaked: &'static [FlagDirectEntry] = Box::leak(entries.to_vec().into_boxed_slice());
    FLAGS_DIRECT[component_id].set(leaked).is_ok()
}

/// Returns the DIRECT `flags (…)` declarations for `component_id`, or an empty
/// slice when the component declared none.
///
/// Cold: read at archetype construction (to OR the
/// [`ArchetypeFlags::FLAGS_ON_ATTACH`] gate bit) and on the attach path behind
/// that gate — never on the per-frame read path.
///
/// [`ArchetypeFlags::FLAGS_ON_ATTACH`]: crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags::FLAGS_ON_ATTACH
#[inline]
pub fn flags_direct_for(component_id: usize) -> &'static [FlagDirectEntry] {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return &[];
    }
    FLAGS_DIRECT[component_id].get().copied().unwrap_or(&[])
}

/// `true` iff `component_id` declares at least one initial flag state.
///
/// The one-shot predicate `ArchetypeFlags::insert_from_flags` uses to decide
/// whether to raise the archetype's gate bit. One `OnceLock` read + an
/// `is_empty` — no allocation, no resolver call.
#[inline]
pub fn declares_flags(component_id: usize) -> bool {
    !flags_direct_for(component_id).is_empty()
}
