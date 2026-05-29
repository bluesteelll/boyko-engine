//! `ComponentHooksBuilder` — the runtime hook-registration builder (Phase 14a,
//! plan §6.3 / REG).
//!
//! Returned by [`EcsMaster::register_component_hooks`], it offers four chainable
//! setters (`on_add` / `on_insert` / `on_replace` / `on_remove`) and commits the
//! accumulated [`ComponentHooks`] into the cold `HOOKS` table when the builder is
//! dropped (or [`finish`](ComponentHooksBuilder::finish)ed explicitly).
//!
//! # Why no `on_despawn`
//!
//! The entity-level despawn hook (distinct from `on_remove`) is deferred to
//! Phase 14b — the [`ComponentHooks`] struct has no `on_despawn` field in 14a, so
//! the builder exposes only the four supported kinds.
//!
//! # Derive XOR runtime (mutually exclusive)
//!
//! A component declares its hooks via EITHER the `#[component(...)]` derive
//! attribute OR this runtime builder — never both. This is the sound,
//! zero-`unsafe` registration path: each `HOOKS` slot is written exactly once
//! via `OnceLock::set`.
//!
//! [`EcsMaster::register_component_hooks`] forces `C::component_id()` first.
//! For a type carrying `#[component(...)]`, `component_id()` installs the derive
//! hooks into the slot, and `register_component_hooks` then **panics eagerly**
//! (it sees `C::HAS_HOOKS == true`) — you must pick one mechanism. For a plain
//! `#[derive(Component)]` (`HAS_HOOKS == false`) the slot stays unset, so the
//! builder seeds its accumulator from [`ComponentHooks::default`] and commits
//! the result via
//! [`try_set_hooks`](crate::ecs::core::component::component_registry::try_set_hooks)
//! (`OnceLock::set`). There is nothing to merge — a derive-hooked type never
//! reaches the builder's commit in correct programs.

use core::marker::PhantomData;

use crate::ecs::core::component::component_registry;
use crate::ecs::core::component::hooks::{ComponentHooks, HookFn};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// Chainable builder that writes a component's lifecycle hooks into the cold
/// `HOOKS` table on drop (Phase 14a, plan §6.3).
///
/// Each setter sets the corresponding [`ComponentHooks`] field; the accumulated
/// value is committed via `OnceLock::set` when the builder goes out of scope.
/// Holding the builder borrows `&mut EcsMaster` for `'a`, so the world cannot be
/// used until the builder is dropped.
///
/// Derive and the runtime builder are mutually exclusive per type (the XOR
/// contract — see the module docs): the accumulator therefore seeds from
/// [`ComponentHooks::default`], never from a pre-installed derive value.
#[must_use = "the builder commits hooks on drop; bind it or chain a setter"]
pub struct ComponentHooksBuilder<'a> {
    /// The component the hooks are being registered for.
    component_id: usize,
    /// Accumulator, seeded from [`ComponentHooks::default`] (all-`None`). The
    /// XOR contract guarantees a derive-hooked type never reaches the builder,
    /// so there is no pre-installed value to merge.
    hooks: ComponentHooks,
    /// Ties the builder to the `&mut EcsMaster` borrow that minted it, so the
    /// world is exclusively borrowed for the builder's lifetime.
    _marker: PhantomData<&'a mut EcsMaster>,
}

impl<'a> ComponentHooksBuilder<'a> {
    /// Creates a builder for `component_id`, seeding the accumulator from
    /// [`ComponentHooks::default`] (all-`None`).
    ///
    /// The accumulator is NOT seeded from `get_hooks`: the derive-XOR-runtime
    /// contract guarantees a derive-hooked type never reaches the builder
    /// (`register_component_hooks` panics first), so there is nothing to merge.
    ///
    /// `pub(crate)`: minted only by [`EcsMaster::register_component_hooks`],
    /// which establishes the exclusive-world / staleness preconditions.
    #[inline]
    pub(crate) fn new(component_id: usize) -> Self {
        Self {
            component_id,
            hooks: ComponentHooks::default(),
            _marker: PhantomData,
        }
    }

    /// Sets the `on_add` hook (fired after the component becomes newly present
    /// on an entity). Chainable.
    #[inline]
    pub fn on_add(mut self, f: HookFn) -> Self {
        self.hooks.on_add = Some(f);
        self
    }

    /// Sets the `on_insert` hook (fired after the component is inserted, newly
    /// or via a bundle insert). Chainable.
    #[inline]
    pub fn on_insert(mut self, f: HookFn) -> Self {
        self.hooks.on_insert = Some(f);
        self
    }

    /// Sets the `on_replace` hook (fired before an existing value is
    /// overwritten or the component leaves; the view reads the OLD value).
    /// Chainable.
    #[inline]
    pub fn on_replace(mut self, f: HookFn) -> Self {
        self.hooks.on_replace = Some(f);
        self
    }

    /// Sets the `on_remove` hook (fired before the component is removed; the
    /// view reads the dying value). Chainable.
    #[inline]
    pub fn on_remove(mut self, f: HookFn) -> Self {
        self.hooks.on_remove = Some(f);
        self
    }

    /// Commits the accumulated hooks explicitly, consuming the builder.
    ///
    /// Equivalent to dropping the builder (the commit also runs in `Drop`); call
    /// this when you want the write to happen at a precise point or to make the
    /// commit visible at the call site.
    #[inline]
    pub fn finish(self) {
        // The actual commit is in `Drop`; consuming `self` here runs it.
        drop(self);
    }
}

impl Drop for ComponentHooksBuilder<'_> {
    #[inline]
    fn drop(&mut self) {
        // Safe commit via `OnceLock::set` (no `unsafe`). In correct programs the
        // slot is unset (derive XOR runtime — `register_component_hooks` panics
        // eagerly for a derive-hooked type), so `set` succeeds. A `false` return
        // means the slot was already populated — a derive-vs-runtime collision
        // that slipped past the eager check (only reachable via a hand-`impl
        // Component` with an inconsistent `HAS_HOOKS`). Panic as defense in depth.
        if !component_registry::try_set_hooks(self.component_id, self.hooks) {
            hooks_already_installed_panic(self.component_id);
        }
    }
}

/// Cold panic site for the derive-vs-runtime hook collision (a `try_set_hooks`
/// commit that finds the slot already populated). `#[cold] #[inline(never)]`
/// keeps the wording and the unwind machinery out of the `Drop` body.
#[cold]
#[inline(never)]
fn hooks_already_installed_panic(component_id: usize) -> ! {
    let type_name = component_registry::get_layout(component_id)
        .map(|l| l.type_name)
        .unwrap_or("<unknown>");
    panic!(
        "register_component_hooks::<{type_name}>(): {type_name} already has hooks \
         installed (derive `#[component(...)]` and the runtime builder are mutually \
         exclusive — use one)."
    );
}
