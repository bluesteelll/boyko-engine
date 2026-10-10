//! Entity-cloning sub-registry (Feature 3 — `EntityCloner`).
//!
//! Split out of the former single-file `component_registry` (pure mechanical
//! move — every item keeps its exact `component_registry::…` path via the
//! `pub use clone::*` re-export in the parent `mod.rs`). Holds the per-component
//! clone metadata (`CLONE` / `MAP_ENTITIES`), the relationship clone-remap /
//! relink tables, the autoref clone-classification probes, and the owning clone
//! glue. Reaches into the core registry (parent module) only for the shared
//! `MAX_COMPONENTS` bound.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::ecs::core::component::component::Component;

use super::MAX_COMPONENTS;

// ═════════════════════════════════════════════════════════════════════════════
// Entity cloning (Feature 3 — `EntityCloner`). Two parallel cold tables mirroring
// the HOOKS / STORAGE_KIND / REQUIRES blocks above (D1): the 56 B `ComponentLayout`
// hot record stays pinned (TRIPWIRE 2). Touched ONLY at registration time
// (write-once `install_clone_fn`, one cold `OnceLock::set` per type) and at the
// explicit `clone_*` materialization path — NEVER on the per-frame hot path
// (spawn / iter / schedule). A program that never clones is byte-identical: every
// reader below (`get_clone_info` / `get_map_entities_fn`) is called only from
// `core::clone`. 0%-gate (grep-proof obligation): the `get_*` callers ⊆ `clone/`.
// ═════════════════════════════════════════════════════════════════════════════

/// Clone one component instance: read the live value at `src`, produce a clone at
/// `dst`. A bare `unsafe fn(*const u8, *mut u8)` (mirror of [`DropFn`](super::DropFn)) — no
/// `Box<dyn>`, no `Arc<dyn Fn>`. Installed ONLY for [`Cloneability::CloneViaFn`]
/// components (O2: a [`Cloneability::TriviallyCopyable`] component installs `None`
/// and is byte-copied whole-column from the pool layout, never through this
/// pointer).
///
/// # Safety (caller-guaranteed at the single materialization call site)
/// - `src` points at a live, initialized value of THIS `ComponentId`'s type `C`,
///   aligned to `align_of::<C>()`, readable for `size_of::<C>()` bytes.
/// - `dst` points at writable, **uninitialized** space of `>= size_of::<C>()`
///   bytes, aligned to `align_of::<C>()`.
/// - `src` and `dst` do NOT overlap (distinct pool rows / archetypes).
/// - On a normal return `dst` holds an initialized `C` written exactly once; on a
///   panic `dst` is left uninitialized and the caller's rollback guard must NOT
///   drop it (W5 partial-row contract).
pub type CloneFn = unsafe fn(src: *const u8, dst: *mut u8);

/// Remap the `Entity` field(s) inside a component at `dst` using the source→clone
/// map produced by a deep clone (D5). Installed ONLY for `ChildOf` in v1 (every
/// other id leaves its slot unset, so a deep clone remaps `ChildOf` and leaves all
/// other entity references verbatim — the documented v1 boundary).
///
/// # Safety (caller-guaranteed at the deep-clone remap call site)
/// - `dst` points at a live, initialized value of THIS `ComponentId`'s type.
/// - The map outlives the call and is not aliased mutably.
pub type MapEntitiesFn = unsafe fn(
    dst: *mut u8,
    map: &crate::ecs::core::clone::map::EntityCloneMap,
);

/// Per-component clone classification (D3 / O2). Drives the materialization
/// branch on its own — the fn-ptr is redundant for the trivially-copyable case.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cloneability {
    /// `Copy` with NO `Entity` field — `clone_fn` is `None`; the materialization
    /// batch-copies the whole column slice via `copy_nonoverlapping` driven by the
    /// pool layout (the -45 % Bevy win, reuses `write_at`). O2 collapse.
    TriviallyCopyable = 0,
    /// `Clone` (owning) OR `Copy`-with-`Entity` — must run the per-element
    /// `clone_fn`. A `Copy`-with-`Entity` component is classified here (NOT
    /// `TriviallyCopyable`) so the `ChildOf` deep-clone remap (D5) can run.
    CloneViaFn = 1,
    /// Not `Clone`, or `#[component(no_clone)]` — `clone_fn` is `None`. The cloner
    /// skips it (opt-out / non-strict) or rejects (strict). The backward-compatible
    /// default for every existing non-`Clone` component (the 0%-gate: no `Clone`
    /// bound on `Component`).
    Ignore = 2,
}

/// Cold per-component clone metadata (D1). 16 B POD (niche-packed `Option<fn-ptr>`
/// 8 B + `Cloneability` 1 B + pad). Lives in the parallel `CLONE` table, NOT in
/// `ComponentLayout` (keeps TRIPWIRE 2's 56 B). `Copy + Send + Sync` (fn-ptr +
/// enum only), like `ComponentHooks`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CloneInfo {
    /// `Some(clone_via_clone::<C>)` ONLY for [`Cloneability::CloneViaFn`]; `None`
    /// for [`Cloneability::TriviallyCopyable`] (O2 batch path) and
    /// [`Cloneability::Ignore`].
    pub clone_fn: Option<CloneFn>,
    /// The branch discriminator. Drives batch-vs-fn-ptr-vs-skip on its own.
    pub cloneability: Cloneability,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<CloneInfo>() == 16);

/// Feature 3 — parallel cold table of per-component clone metadata. Touched ONLY
/// when a clone executes (never on spawn/iter/schedule). Mirrors `HOOKS`.
static CLONE: [OnceLock<CloneInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Feature 3 — parallel cold table of per-component entity-remap fns. v1: ONLY
/// `ChildOf` installs a `Some(child_of_map_entities)` (hand-written, in
/// `hierarchy/mod.rs`); every other id is never set, so a deep clone remaps
/// `ChildOf` and leaves all other entity refs verbatim (D5 boundary). Separate
/// table (not a `CloneInfo` field) so the common no-remap component leaves it
/// unset — one fewer write at registration.
static MAP_ENTITIES: [OnceLock<MapEntitiesFn>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Returns the registered clone metadata for `component_id`, or `None` when no
/// clone fn was installed (a non-derived / hand-written impl that never opted in).
///
/// Cold: read ONLY from `core::clone` materialization — never on the per-frame hot
/// path (the 0%-gate). One acquire-load + branch, mirroring [`get_hooks`](super::get_hooks).
#[inline]
pub fn get_clone_info(component_id: usize) -> Option<&'static CloneInfo> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    CLONE[component_id].get()
}

/// Returns the entity-remap fn for `component_id` (v1: `Some` only for `ChildOf`),
/// or `None`. Cold: read ONLY from the `core::clone` deep-clone remap pass.
#[inline]
pub fn get_map_entities_fn(component_id: usize) -> Option<MapEntitiesFn> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    MAP_ENTITIES[component_id].get().copied()
}

/// Installs `C`'s clone metadata into `CLONE[component_id]` (Feature 3). Builds a
/// [`CloneInfo`] from the type's compile-time [`Component::CLONE_BEHAVIOR`] +
/// [`Component::clone_fn`] and writes it once via `OnceLock::set`, mirroring
/// [`install_hooks`](super::install_hooks) / [`install_required`](super::install_required).
///
/// **PUBLIC** (the derive expands into downstream crates where `pub(crate)` is
/// unreachable — the same rationale as [`install_storage_kind`](super::install_storage_kind)).
/// Called from the
/// derive's `component_id()` closure **UNGATED** (unlike `install_hooks`): the
/// 0%-gate is preserved because the write is one cold `OnceLock::set` per type per
/// process, behind the existing `component_id()` `OnceLock`, and never on a
/// per-frame path. Ungating it means the clone path never has to special-case a
/// missing entry — a plain non-cloneable component installs `Cloneability::Ignore`
/// with `clone_fn = None` (the trait defaults), which reads as "skip / reject"
/// everywhere downstream.
#[inline]
pub fn install_clone_fn<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let info = CloneInfo {
        clone_fn: C::clone_fn(),
        // Read the METHOD (not the const): the derive overrides `clone_behavior()`
        // with the autoref-probe result (a const cannot run autoref). Hand-written
        // impls (e.g. `ChildOf`) default the method to their `CLONE_BEHAVIOR` const.
        cloneability: C::clone_behavior(),
    };
    // Write-once; a same-id re-install is a silent no-op (first writer wins).
    let _ = CLONE[component_id].set(info);
}

/// Id-keyed entity-remap install (the hand-written `ChildOf` path only in v1,
/// Feature 3 D5). Mirrors [`install_clone_fn`]'s write-once discipline.
pub(crate) fn install_map_entities_fn(component_id: usize, f: MapEntitiesFn) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let _ = MAP_ENTITIES[component_id].set(f);
}

/// Installs the GENERIC clone-direction foreign-key remap for a relationship
/// SOURCE `R` (BUG-RELATIONS-CLONE-1). The deep-clone remap pass reads this fn via
/// [`get_map_entities_fn`] and applies it to every cloned `R` so its FK is rewritten
/// from the original target to the cloned one (the generalized `child_of_map_entities`
/// hand-mirror).
///
/// **PUBLIC** for the same reason as [`install_clone_fn`]: a `#[derive(Component)]`
/// for a relationship source expands into a downstream crate where the raw
/// [`install_map_entities_fn`] setter (`pub(crate)`) is unreachable. This thin
/// wrapper monomorphizes
/// [`relationship_clone_map_entities`](crate::ecs::core::relationship::relationship_clone_map_entities)
/// for `R` and installs it
/// through the same write-once setter — so the clone path reads the SAME remap fn
/// whether the relation is the hand-mirrored `ChildOf` or a derived one. The serialize
/// (load) direction stays a separate slot (`SerializeInfo::map_entities_fn`), since the
/// two directions take different map types (`EntityCloneMap` vs `LoadEntityMap`).
///
/// One cold `OnceLock::set` per relation source type per process (the 0%-gate);
/// never touched on a per-frame path.
#[inline]
pub fn install_relationship_clone_remap<R: crate::ecs::core::relationship::Relationship>(
    component_id: usize,
) {
    install_map_entities_fn(
        component_id,
        crate::ecs::core::relationship::relationship_clone_map_entities::<R> as MapEntitiesFn,
    );
    install_relationship_relink(
        component_id,
        crate::ecs::core::relationship::relationship_clone_relink::<R>
            as crate::ecs::core::relationship::RelationshipRelinkFn,
    );
}

/// Parallel cold table of per-relationship-source clone-relink fns
/// (BUG-RELATIONS-CLONE-1). The deep-clone remap pass (in `ComponentId` space) reads
/// this via [`get_relationship_relink_fn`] to rebuild a clone's (cloner-denied)
/// reverse index after its foreign key was remapped — the type-erased generalization
/// of `link_child`. Set only for relationship SOURCES (`Likes`, `ChildOf`, …); a
/// target / plain component leaves it unset. Mirrors the [`MAP_ENTITIES`] table.
static RELATIONSHIP_LINK: [OnceLock<crate::ecs::core::relationship::RelationshipRelinkFn>;
    MAX_COMPONENTS] = [const { OnceLock::new() }; MAX_COMPONENTS];

/// Id-keyed relationship-relink install (BUG-RELATIONS-CLONE-1). Write-once,
/// mirroring [`install_map_entities_fn`]. Called from
/// [`install_relationship_clone_remap`] so a source installs its clone-remap and its
/// relink together (the deep clone needs both).
pub(crate) fn install_relationship_relink(
    component_id: usize,
    f: crate::ecs::core::relationship::RelationshipRelinkFn,
) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let _ = RELATIONSHIP_LINK[component_id].set(f);
}

/// Returns the relationship-relink fn for `component_id` (BUG-RELATIONS-CLONE-1),
/// `Some` only for a relationship source that installed one. Cold: read only from the
/// deep-clone remap pass.
#[inline]
pub fn get_relationship_relink_fn(
    component_id: usize,
) -> Option<crate::ecs::core::relationship::RelationshipRelinkFn> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    RELATIONSHIP_LINK[component_id].get().copied()
}

/// Parallel cold table marking each `ComponentId` that is a `RelationshipTarget`
/// reverse-index collection (BUG-RELATIONS-CLONE-1). One [`AtomicBool`] per id,
/// mirroring the [`STORAGE_KIND`] table shape.
///
/// The deep-clone id-selection (`select_clone_ids`) denies EVERY id with this flag
/// set: a reverse index is never byte-copied — it is rebuilt when the source FKs are
/// cloned and their link hooks fire. `Children` and every derived `RelationshipTarget`
/// (`LikedBy`, …) set it, so the generic clone-deny is one shared predicate rather
/// than a literal `Children` special-case.
///
/// Touched ONLY at registration time (write-once via [`set_relationship_target`]) and
/// from the cold clone id-selection — never on the per-frame hot path. `Relaxed` is
/// sufficient: it is a registration-time, write-once datum with no payload published
/// through it, settled (atomically with id assignment) before the component can appear
/// in any archetype that a clone would walk. The default `false` reads back for every
/// id that is not a relationship target.
static RELATIONSHIP_TARGET: [AtomicBool; MAX_COMPONENTS] =
    [const { AtomicBool::new(false) }; MAX_COMPONENTS];

/// Returns `true` iff `component_id` is a `RelationshipTarget` reverse-index
/// collection (BUG-RELATIONS-CLONE-1). Defaults to `false` for any id never flagged
/// via [`set_relationship_target`].
///
/// Cold: read only from the clone id-selection (`select_clone_ids`), never on the
/// per-frame hot path. One `Relaxed` load.
#[inline]
pub fn is_relationship_target(component_id: usize) -> bool {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return false;
    }
    RELATIONSHIP_TARGET[component_id].load(Ordering::Relaxed)
}

/// Flags `component_id` as a `RelationshipTarget` reverse index (BUG-RELATIONS-CLONE-1).
/// Idempotent write-once: called from the `RelationshipTarget` registration path (the
/// derive's `component_id()` for `LikedBy`-style targets, and the in-crate `Children`
/// hand-mirror) atomically with id assignment, before the component can enter any
/// archetype a clone would walk.
///
/// **PUBLIC** for the same reason as [`install_clone_fn`]: the derive expands into
/// downstream crates where `pub(crate)` is unreachable.
#[inline]
pub fn set_relationship_target(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    RELATIONSHIP_TARGET[component_id].store(true, Ordering::Relaxed);
}

// ── Autoref clone-classification probes (Feature 3, derive support) ─────────────
//
// The `#[derive(Component)]` macro cannot resolve a type to ask "is it Copy? is it
// Clone?". These three zero-sized probe arms use AUTOREF SPECIALIZATION to pick the
// right `Cloneability` + `clone_fn` at the type level, reflection-free.
//
// Each arm is a TRAIT with an `&self` method, impl'd for a different reference depth
// of `CloneProbe`. Given a call expression with ENOUGH leading refs, Rust method
// resolution picks the applicable arm whose `&self` receiver is reached with the
// FEWEST autorefs from the call expression — i.e. the impl on the type with the MOST
// refs in `Self` wins (the dtolnay autoref-specialization rule: the more-specific,
// more-ref'd impl is preferred). So priority is by depth: MORE refs in `Self` =
// HIGHER priority.
//
//   * `TriviallyCopyableArm for &&CloneProbe<C, true>` (gated `C: Copy` AND
//     `TRIVIAL == true`) — the MOST-ref'd `Self`, HIGHEST priority. A
//     `Copy`-no-`Entity` type wins here → `TriviallyCopyable` / `None` (the O2 batch
//     path). For a `Copy`-WITH-`Entity` type the macro passes `TRIVIAL == false`, so
//     this arm's `Self` (`&&CloneProbe<C, true>`) does NOT structurally match and the
//     type falls through to the `Clone` arm (no hard error — the const mismatch
//     removes the arm as a candidate rather than failing a bound).
//   * `CloneViaFnArm for &CloneProbe<C, TRIVIAL>` (gated `C: Clone`) — middle
//     priority. A `Clone`-not-`Copy` type, OR a `Copy`-with-`Entity` type (forced
//     here via `TRIVIAL == false`), lands here because `Copy ⊆ Clone` →
//     `CloneViaFn` / `Some(clone_via_clone::<C>)` (so the `ChildOf` deep-clone remap,
//     D5, can run).
//   * `CloneIgnoreArm for CloneProbe<C, TRIVIAL>` (no bound) — the by-value `Self`,
//     LOWEST priority. A non-`Clone` type reaches it → `Ignore` / `None`.
//
// The call site (`derive` codegen) invokes the probe through THREE refs
// (`(&&&probe).method()`): with three leading refs and `&self` arms, all three
// receiver depths are reachable, and the resolver selects the highest-priority
// APPLICABLE arm. Because the unbounded `Ignore` arm sits at the by-value (least
// specific) `Self`, it never shadows the bounded `Copy` / `Clone` arms; because the
// `Copy` arm is gated by BOTH `C: Copy` and the `TRIVIAL == true` const, a non-`Copy`
// or `Copy`-with-`Entity` type cleanly falls to the next arm with no bound-failure
// hard error. Resolution is exactly `Copy → Trivial` / `Clone → ViaFn` /
// `neither → Ignore`.

/// Probe wrapper for autoref clone classification (see module note). `TRIVIAL` is
/// the macro's "no `Entity` field" syntactic flag: when `false` the `Copy` arm is
/// suppressed so a `Copy`-with-`Entity` type falls to `CloneViaFn`.
#[doc(hidden)]
pub struct CloneProbe<C, const TRIVIAL: bool>(pub core::marker::PhantomData<C>);

impl<C, const TRIVIAL: bool> CloneProbe<C, TRIVIAL> {
    /// Constructs the probe (called by derive-generated code only).
    #[doc(hidden)]
    #[inline]
    pub const fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

/// By-value-`Self` fallback arm (no bound): a non-`Clone` type is `Ignore`. The
/// LEAST-specific `Self` (fewest refs), so it has the LOWEST priority and only wins
/// when neither the `Copy` arm nor the `Clone` arm applies.
#[doc(hidden)]
pub trait CloneIgnoreArm {
    #[doc(hidden)]
    fn clone_behavior(&self) -> Cloneability;
    #[doc(hidden)]
    fn clone_fn_ptr(&self) -> Option<CloneFn>;
}

impl<C, const TRIVIAL: bool> CloneIgnoreArm for CloneProbe<C, TRIVIAL> {
    #[inline]
    fn clone_behavior(&self) -> Cloneability {
        Cloneability::Ignore
    }

    #[inline]
    fn clone_fn_ptr(&self) -> Option<CloneFn> {
        None
    }
}

/// `&`-`Self` arm gated `C: Clone` (middle priority): an owning (or
/// `Copy`-with-`Entity`, forced via `TRIVIAL = false`) type is `CloneViaFn`. More
/// specific than the by-value `Ignore` fallback, less specific than the `&&`-`Self`
/// `Copy` arm.
#[doc(hidden)]
pub trait CloneViaFnArm {
    #[doc(hidden)]
    fn clone_behavior(&self) -> Cloneability;
    #[doc(hidden)]
    fn clone_fn_ptr(&self) -> Option<CloneFn>;
}

impl<C: Clone + 'static, const TRIVIAL: bool> CloneViaFnArm for &CloneProbe<C, TRIVIAL> {
    #[inline]
    fn clone_behavior(&self) -> Cloneability {
        Cloneability::CloneViaFn
    }

    #[inline]
    fn clone_fn_ptr(&self) -> Option<CloneFn> {
        Some(clone_via_clone::<C> as CloneFn)
    }
}

/// `&&`-`Self` arm gated `C: Copy` AND `TRIVIAL == true` (the MOST specific `Self`,
/// HIGHEST priority): a `Copy`-no-`Entity` type is `TriviallyCopyable` (O2 batch
/// path, `clone_fn` None). Being most-specific it wins for a `Copy + TRIVIAL == true`
/// type before the `&`-`Self` `Clone` arm (`Copy ⊆ Clone`) can match; a
/// `Copy`-with-`Entity` type carries `TRIVIAL == false`, so this arm's `Self` does
/// not match and the type falls cleanly to the `Clone` arm.
#[doc(hidden)]
pub trait TriviallyCopyableArm {
    #[doc(hidden)]
    fn clone_behavior(&self) -> Cloneability;
    #[doc(hidden)]
    fn clone_fn_ptr(&self) -> Option<CloneFn>;
}

impl<C: Copy + 'static> TriviallyCopyableArm for &&CloneProbe<C, true> {
    #[inline]
    fn clone_behavior(&self) -> Cloneability {
        Cloneability::TriviallyCopyable
    }

    #[inline]
    fn clone_fn_ptr(&self) -> Option<CloneFn> {
        None
    }
}

/// Owning clone glue (Feature 3 D2 / O2): read `&C`, call `Clone::clone`, write
/// the result into the uninit `dst`. The single monomorphized free fn the derive
/// installs for a [`Cloneability::CloneViaFn`] component — no vtable, no `Arc<dyn>`.
///
/// # W7 — cannot reach world state
/// This fn receives ONLY `*const u8` / `*mut u8`; it has no `DeferredEcsMaster` /
/// world view. So even though arbitrary user `Clone::clone` code runs while
/// `&mut Archetype` / `&mut pool` reborrows are live in the materialization loop,
/// it CANNOT reach world state and CANNOT create the F2 protected-tag conflict.
///
/// # Safety
///
/// The caller must uphold the [`CloneFn`] contract:
/// - `src` is a live, aligned, initialized `C` (established at the call site via
///   `unit_ptr`'s contract); we form a shared `&C` for the `.clone()` call only,
///   and the source row is read-only during materialization (no `&mut C` aliases).
/// - `dst` is uninit, aligned space for one `C`; `ptr::write` initializes it
///   WITHOUT dropping the (uninitialized) prior contents.
/// - `src` and `dst` are disjoint (distinct pool rows / archetypes).
pub unsafe fn clone_via_clone<C: Clone>(src: *const u8, dst: *mut u8) {
    // SAFETY: `src` is a valid, live, aligned, initialized `C` (CloneFn contract,
    // established at the materialization call site by `unit_ptr`). The shared `&C`
    // lives only across the `.clone()` call; the source row is not mutated during
    // materialization. `dst` is uninit aligned space for one `C`; `write` does not
    // drop the uninit destination. `src` and `dst` are disjoint allocations.
    unsafe {
        let value: &C = &*src.cast::<C>();
        let cloned: C = value.clone();
        dst.cast::<C>().write(cloned);
    }
}
