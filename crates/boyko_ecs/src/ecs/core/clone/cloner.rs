//! `EntityCloner` + the typestate builder (Feature 3, D4).
//!
//! A builder-configured, reusable cloner. The headline default is opt-out (clone
//! every cloneable component, Bevy `clone_and_spawn` parity); [`EntityCloner::only`]
//! switches to opt-in. The cloner holds NO world borrow — it is pure config
//! (`Send + Sync`, reusable across frames/threads); execution is single-threaded
//! on `&mut EcsMaster`.
//!
//! The which-components filter is a [`ComponentMask`] (the in-crate 512-bit set —
//! the "BitSet512" the plan refers to), NOT a `HashMap`: dense, branch-predictable
//! (one `contains` bit-test per component), already in the crate.

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::identifiers::primitives::ComponentId;

/// Zero-sized typestate marker: an **opt-out** builder (`deny`-only). The default
/// shape — clones every cloneable component minus the denied ids.
pub struct OptOut;

/// Zero-sized typestate marker: an **opt-in** builder (`allow`-only). Clones ONLY
/// the explicitly-allowed ids (that also have a clone fn).
pub struct OptIn;

/// Shallow (default) vs deep-over-`ChildOf` clone policy (D5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LinkedClonePolicy {
    /// `ChildOf(parent)` is copied verbatim → the clone shares the source's parent
    /// (becomes a sibling). `Children` is always cloner-denied (a shallow clone has
    /// no children). DEFAULT.
    Shallow,
    /// Recursively clone the `ChildOf` subtree; clones re-parent to the cloned
    /// root. Reuses the Phase-19 `Children` reverse index (D5).
    Deep,
}

/// The which-components filter (D4). `Copy` (`ComponentMask` is `Copy`), so the
/// whole [`EntityCloner`] is `Copy`-cheap to clone for the deferred command path.
#[derive(Clone, Copy)]
pub(crate) enum CloneFilter {
    /// Opt-out: clone every component that has a clone fn, MINUS `deny`.
    All { deny: ComponentMask },
    /// Opt-in: clone ONLY ids in `allow` (that also have a clone fn).
    Only { allow: ComponentMask },
}

/// Configuration for a clone operation (D4). Built once via [`EntityCloner::new`]
/// / [`EntityCloner::only`], reused across many clones. Holds no world borrow.
///
/// # EnableTag state is NOT cloned (v1)
///
/// An EnableTag (`#[component(storage = "bitset")]`) has no `ComponentPool`; its
/// enable/disable bit is **not** carried through a v1 clone. A bitset id is skipped
/// during materialization, so the clone lands in an archetype without the tag (the
/// clone is still valid — it just lacks the tag). Preserving the enable-state is a
/// v1.1 follow-up. See the [`clone` module docs](crate::ecs::core::clone).
#[derive(Clone, Copy)]
pub struct EntityCloner {
    pub(crate) filter: CloneFilter,
    pub(crate) linked: LinkedClonePolicy,
    /// Whether materialization fires `on_add` / `on_insert` hooks + observers on
    /// the new entity. Default: `true` (Bevy parity; reuses the existing gated fire
    /// sites for free — 0% when the archetype has no hooks). `fire_hooks(false)`
    /// is the bulk-clone escape hatch.
    pub(crate) fire_hooks: bool,
    /// Strict mode: panic if the source has a component with no clone fn
    /// ([`Cloneability::Ignore`](crate::ecs::core::component::component_registry::Cloneability::Ignore))
    /// instead of silently skipping it. Default: `false` (skip, Bevy parity).
    pub(crate) strict: bool,
    /// Reset change-detection ticks to current (default) vs preserve the source's
    /// ticks. Default: `false` (reset — a clone is "added now"; `Added` / `Changed`
    /// fire the frame it is cloned).
    pub(crate) preserve_ticks: bool,
}

impl EntityCloner {
    /// Opt-out cloner: clones all cloneable components. The headline default —
    /// shallow, fires hooks.
    // Builder entry point (Bevy `EntityCloner::build_opt_out` parity): `new`
    // intentionally returns the typestate BUILDER, not `Self`. The cloner is
    // finalized via `.build()`.
    #[allow(clippy::new_ret_no_self)]
    #[inline]
    pub fn new() -> EntityClonerBuilder<OptOut> {
        EntityClonerBuilder {
            cloner: EntityCloner {
                filter: CloneFilter::All {
                    deny: ComponentMask::new(),
                },
                linked: LinkedClonePolicy::Shallow,
                fire_hooks: true,
                strict: false,
                preserve_ticks: false,
            },
            _mode: core::marker::PhantomData,
        }
    }

    /// Opt-in cloner: clones ONLY explicitly-allowed components.
    #[inline]
    pub fn only() -> EntityClonerBuilder<OptIn> {
        EntityClonerBuilder {
            cloner: EntityCloner {
                filter: CloneFilter::Only {
                    allow: ComponentMask::new(),
                },
                linked: LinkedClonePolicy::Shallow,
                fire_hooks: true,
                strict: false,
                preserve_ticks: false,
            },
            _mode: core::marker::PhantomData,
        }
    }

    /// Returns the default opt-out cloner already built (shallow, fires hooks,
    /// non-strict, reset ticks) — the configuration `EcsMaster::clone_and_spawn`
    /// uses. Convenience for the materialization entry points.
    #[inline]
    pub(crate) fn default_built() -> EntityCloner {
        EntityCloner::new().build()
    }

    /// `true` iff this cloner deep-clones the `ChildOf` subtree.
    #[inline]
    pub(crate) fn is_deep(&self) -> bool {
        matches!(self.linked, LinkedClonePolicy::Deep)
    }

    /// Forces this cloner to the shallow policy, preserving every other setting.
    /// Used by the deep-clone walk to clone each node WITHOUT re-entering the
    /// subtree recursion (the walk drives the recursion explicitly).
    #[inline]
    pub(crate) fn force_shallow(&mut self) {
        self.linked = LinkedClonePolicy::Shallow;
    }

    /// Decides whether `id` should be cloned given the source carries it and the
    /// filter's allow/deny rule. Does NOT consider `Children` denial or clone-fn
    /// presence — the caller applies those separately (C2 require-closure builds
    /// from this predicate, then the require-closure overrides a denied required).
    #[inline]
    pub(crate) fn filter_allows(&self, id: ComponentId) -> bool {
        match &self.filter {
            CloneFilter::All { deny } => !deny.contains(id),
            CloneFilter::Only { allow } => allow.contains(id),
        }
    }
}

impl Default for EntityCloner {
    /// The headline opt-out, shallow, fire-hooks configuration (matches
    /// `EntityCloner::new().build()`).
    #[inline]
    fn default() -> Self {
        EntityCloner::default_built()
    }
}

/// Typestate builder for [`EntityCloner`] (D4). `Mode` (`OptOut` / `OptIn`)
/// enforces at compile time that `allow` is only callable on an opt-in builder and
/// `deny` on an opt-out builder — a zero-cost mirror of Bevy's
/// `build_opt_in` / `build_opt_out` split.
pub struct EntityClonerBuilder<Mode> {
    cloner: EntityCloner,
    _mode: core::marker::PhantomData<Mode>,
}

impl<Mode> EntityClonerBuilder<Mode> {
    /// Shallow (`false`, default) vs deep-over-`ChildOf` (`true`).
    #[inline]
    pub fn linked(mut self, deep: bool) -> Self {
        self.cloner.linked = if deep {
            LinkedClonePolicy::Deep
        } else {
            LinkedClonePolicy::Shallow
        };
        self
    }

    /// Whether to fire `on_add` / `on_insert` on the clone (default `true`).
    #[inline]
    pub fn fire_hooks(mut self, fire: bool) -> Self {
        self.cloner.fire_hooks = fire;
        self
    }

    /// Strict mode: panic on an `Ignore` (non-cloneable) source component instead
    /// of skipping it (default `false`).
    #[inline]
    pub fn strict(mut self, strict: bool) -> Self {
        self.cloner.strict = strict;
        self
    }

    /// Preserve the source's change-detection ticks instead of resetting to the
    /// current tick (default `false` → reset).
    #[inline]
    pub fn preserve_ticks(mut self, preserve: bool) -> Self {
        self.cloner.preserve_ticks = preserve;
        self
    }

    /// Finalizes the configuration.
    #[inline]
    pub fn build(self) -> EntityCloner {
        self.cloner
    }
}

impl EntityClonerBuilder<OptOut> {
    /// Excludes component `C` from the clone (opt-out only — typestate-enforced).
    #[inline]
    pub fn deny<C: Component>(self) -> Self {
        self.deny_id(C::component_id())
    }

    /// Excludes the component `id` from the clone (opt-out only).
    #[inline]
    pub fn deny_id(mut self, id: ComponentId) -> Self {
        if let CloneFilter::All { deny } = &mut self.cloner.filter {
            deny.set(id);
        }
        self
    }
}

impl EntityClonerBuilder<OptIn> {
    /// Includes component `C` in the clone (opt-in only — typestate-enforced).
    #[inline]
    pub fn allow<C: Component>(self) -> Self {
        self.allow_id(C::component_id())
    }

    /// Includes the component `id` in the clone (opt-in only).
    #[inline]
    pub fn allow_id(mut self, id: ComponentId) -> Self {
        if let CloneFilter::Only { allow } = &mut self.cloner.filter {
            allow.set(id);
        }
        self
    }
}
