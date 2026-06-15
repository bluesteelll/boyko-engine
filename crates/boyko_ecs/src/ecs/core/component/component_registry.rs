//! Global registry of component layouts.
//!
//! # Component ID assignment
//!
//! Each distinct type `T` implementing [`Component`](crate::ecs::core::component::component::Component)
//! is assigned a unique [`ComponentId`] the first time `T::component_id()` is
//! called in the current process. The assignment is lazy, lock-free on the
//! cached read path, and stable for the lifetime of the process — but **not**
//! stable across processes or across runs of the same binary if the order of
//! first calls differs.
//!
//! # Startup warm-up contract
//!
//! Code that ingests `ComponentId`s from external sources (network, save
//! files, scripts, etc.) MUST warm up the registry by calling
//! `T::component_id()` for every component type `T` it expects to receive,
//! *before* the first external ID arrives. Without warm-up, an incoming
//! id `i` may refer to type `A` in this process but type `B` in a peer
//! process — IDs are assigned in first-call order.
//!
//! Recommended pattern: at engine startup, call `<T as Component>::component_id()`
//! for every component type that will be serialized, in a deterministic order.
//!
//! # Collision detection
//!
//! Every `set` call site ([`register_new`] and [`register_layout`]) checks
//! the slot before declaring success. If the slot is already occupied by a
//! *different* type than the one being registered, the call panics in both
//! debug and release builds, naming both types. This catches accidental
//! ID-space overlaps between the production counter and the test escape
//! hatch immediately.
//!
//! # Threading
//!
//! All registry operations are safe to call from any thread. The global
//! `NEXT_ID` counter uses `Relaxed` ordering (uniqueness is sufficient;
//! cross-thread happens-before is provided by `OnceLock::set` / `get`).
//! Per-slot `OnceLock`s provide acquire/release synchronization of the
//! `ComponentLayout` payload.

use std::alloc::Layout;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::hooks::{ComponentHooks, HooksError};
use crate::ecs::identifiers::primitives::ComponentId;

/// Maximum number of components supported by the ECS system.
pub const MAX_COMPONENTS: usize = 512;

/// Type-erased drop function pointer for a component type `T`.
///
/// Stored as [`ComponentLayout::drop_fn`] for types where
/// `mem::needs_drop::<T>()` is true. Invoked by [`ComponentPool`] on
/// `swap_remove`, `pop`, `set_component`, `set_component_typed`, and during
/// `Drop`.
///
/// # Safety
/// The caller must guarantee:
/// - `ptr` points at a properly-aligned, fully-initialized instance of `T`.
/// - Per the Rust Reference §"Type Layout — Size and Alignment",
///   `size_of::<T>()` is always a multiple of `align_of::<T>()` for every
///   `Sized T`, so an offset of `i * size_of::<T>()` from a base aligned to
///   `align_of::<T>()` preserves alignment.
/// - `ptr` is not aliased and the value will not be read or dropped again
///   after this call.
///
/// [`ComponentPool`]: crate::ecs::memory::component_pool::ComponentPool
pub type DropFn = unsafe fn(*mut u8);

/// Type-erased drop glue for `T`.
///
/// Stored as `ComponentLayout::drop_fn` when `mem::needs_drop::<T>()` is true.
///
/// # Safety
/// See [`DropFn`] contract above.
#[inline]
pub(crate) unsafe fn drop_in_place_glue<T: 'static>(ptr: *mut u8) {
    // SAFETY: caller upholds the DropFn contract: ptr is aligned, initialized,
    // exclusively owned, and not accessed again after this call.
    unsafe { core::ptr::drop_in_place::<T>(ptr.cast::<T>()) }
}

/// Holds layout information for a specific component type.
///
/// Filled in by [`register_new`] or [`register_layout`]. Each entry is written
/// exactly once via `OnceLock::set` and read lock-free via `OnceLock::get`.
/// Fixes audit findings M-002 / C-002 / Q-010.
///
/// Field order is cache-line friendly: hot fields (size, alignment, drop_fn)
/// at lower offsets, cold fields (type_name, type_id) at higher offsets.
/// Total: 56 B — fits in one 64 B cache line.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ComponentLayout {
    /// Size in bytes — hot (read on every memcpy). Offset 0..8.
    pub size: usize,
    /// Alignment requirement — hot. Offset 8..16.
    pub alignment: usize,
    /// Drop function pointer; `None` iff `!mem::needs_drop::<T>()` — hot
    /// (read on swap_remove/pop/set_component/Drop). Niche-optimized to 8 B.
    /// Offset 16..24.
    pub drop_fn: Option<DropFn>,
    /// Cold: type name for diagnostics. Offset 24..40.
    pub type_name: &'static str,
    /// Cold: TypeId for runtime type validation in debug. Offset 40..56.
    pub type_id: TypeId,
}

// Phase 14a TRIPWIRE 2 (plan §1-W2): the 56 B / one-cache-line guarantee that
// `Q5` relies on (hooks live in the parallel cold `HOOKS` table, NOT inline in
// `ComponentLayout`). Previously documented only in the doc comment above; now
// a hard compile-time assertion so a future field addition trips here.
// `ComponentLayout` embeds `&'static str`, `core::alloc::Layout` (two `usize`),
// an `Option<unsafe fn>`, and a `TypeId`; all are pointer-width, so the 56-byte
// size encodes the 64-bit ABI. Gated to 64-bit (the engine's supported
// platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ComponentLayout>() == 56);

impl ComponentLayout {
    /// Creates a new `ComponentLayout` with static information about type `T`.
    ///
    /// The `if needs_drop::<T>()` branch is const-folded per monomorphization.
    #[inline]
    pub fn new_static<T: 'static>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            alignment: std::mem::align_of::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(drop_in_place_glue::<T> as DropFn)
            } else {
                None
            },
            type_name: std::any::type_name::<T>(),
            type_id: TypeId::of::<T>(),
        }
    }

    /// Phase 22 D2: `true` iff the component is zero-sized (a tag).
    ///
    /// Tag-ness is a property of the registered layout, not of the derive —
    /// manual `Component` impls and `PhantomData` wrappers qualify equally.
    #[inline]
    pub const fn is_zst(&self) -> bool {
        self.size == 0
    }

    /// Phase 22 (D3): layout for a runtime-minted **dynamic tag**.
    ///
    /// Size 0, alignment 1, no drop glue; `type_id` is the private
    /// [`DynamicTagMarker`] sentinel, shared by EVERY dynamic tag. Because the
    /// sentinel is uninhabited and private it can never collide with a user
    /// type, and the typed-pool `debug_assert(component_type_id ==
    /// TypeId::of::<T>())` correctly rejects typed access to dynamic-tag ids.
    ///
    /// `name` is the interned tag name (leaked once per unique tag by the
    /// [`TAG_NAMES`] mint path) — it doubles as `type_name` for diagnostics.
    pub fn new_dynamic_tag(name: &'static str) -> Self {
        Self {
            size: 0,
            alignment: 1,
            drop_fn: None,
            type_name: name,
            type_id: TypeId::of::<DynamicTagMarker>(),
        }
    }

    /// Returns a memory layout object for this component.
    #[inline]
    pub fn layout(&self) -> Layout {
        // SAFETY: size/alignment originated from `size_of::<T>()` /
        // `align_of::<T>()` for some `T: 'static`. Those are valid by the
        // language definition — alignment is a power of two and size fits in
        // `isize::MAX` (otherwise `T` would not have a layout).
        unsafe { Layout::from_size_align_unchecked(self.size, self.alignment) }
    }
}

/// Phase 22 (D3): private uninhabited sentinel — the `TypeId` of every dynamic
/// tag. Can never collide with a user type (it is unnameable outside this
/// module); typed-pool debug guards therefore correctly reject typed access to
/// dynamic-tag ids.
///
/// Because all dynamic tags share this one `TypeId`, idempotency for dynamic
/// mints is NAME-keyed (the [`TAG_NAMES`] intern), never TypeId-keyed —
/// [`register_new`]'s same-TypeId idempotent arm would alias two distinct tag
/// names to one id (plan O2).
enum DynamicTagMarker {}

/// Filter-only dynamic-tag handle (Phase 22 D3). `repr(transparent)` over
/// [`ComponentId`]: zero-cost, but type-distinct so data-fetch APIs cannot
/// accept it (a dynamic tag has no data by definition).
///
/// # One-way bridge (W3)
///
/// `TagId → ComponentId` is public — [`TagId::component_id`] and the
/// `From<TagId> for ComponentId` impl — because the id-keyed surfaces a
/// dynamic tag needs downstream ([`register_hooks_by_id`],
/// `EcsMaster::add_observer`) take [`ComponentId`]. The reverse direction has
/// NO constructor: a `TagId` is a proof that the id was minted as a size-0
/// dynamic tag, and only the [`TAG_NAMES`] mint path can issue one.
///
/// # Identity stability
///
/// Like every [`ComponentId`], the numeric value is first-call-order
/// process-unstable; the **name** is the stable serialization key
/// (`tag_by_name`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TagId(pub(crate) ComponentId);

impl TagId {
    /// Bridges to the shared [`ComponentId`] space (Phase 22 W3) — the
    /// vocabulary of the id-keyed hook/observer surfaces
    /// ([`register_hooks_by_id`], `EcsMaster::add_observer`).
    #[inline]
    pub const fn component_id(self) -> ComponentId {
        self.0
    }
}

impl From<TagId> for ComponentId {
    #[inline]
    fn from(tag: TagId) -> Self {
        tag.component_id()
    }
}

/// Filter-only enable-tag handle (EnableTag plan, D5). `repr(transparent)` over
/// [`ComponentId`]: zero-cost, but type-distinct so data-fetch APIs cannot
/// accept it (an enable tag has no `ComponentPool` by construction — D6).
///
/// An enable tag uses the **bitset** storage backend ([`StorageKind::Bitset`]):
/// its id is filtered out of every archetype signature and toggled with a
/// single per-row bit instead of triggering a migration. It is minted via
/// [`try_register_enable_tag_by_name`], which sets the id's
/// [`STORAGE_KIND`] to [`StorageKind::Bitset`].
///
/// # One-way bridge
///
/// `EnableTagId → ComponentId` is public — [`EnableTagId::component_id`] and the
/// `From<EnableTagId> for ComponentId` impl — because the id-keyed enable
/// surfaces (`enable_id` / `disable_id` / `is_enabled_id`, `with_enabled`) take
/// a [`ComponentId`]. The reverse direction has NO constructor: an
/// `EnableTagId` is a proof that the id was minted as a bitset enable tag, and
/// only the mint path can issue one.
///
/// # Identity stability
///
/// Like every [`ComponentId`], the numeric value is first-call-order
/// process-unstable; the **name** is the stable serialization key.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnableTagId(pub(crate) ComponentId);

impl EnableTagId {
    /// Bridges to the shared [`ComponentId`] space — the vocabulary of the
    /// id-keyed enable/disable surfaces and the `with_enabled` query terms.
    #[inline]
    pub const fn component_id(self) -> ComponentId {
        self.0
    }
}

impl From<EnableTagId> for ComponentId {
    #[inline]
    fn from(tag: EnableTagId) -> Self {
        tag.component_id()
    }
}

/// Static storage for component layouts. Each slot is independent and
/// initialized at most once via `OnceLock::set`. Read path is a single
/// acquire-load + branch — no `Mutex`, no `static mut`, no data race.
static LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Monotonic counter for component IDs minted via [`register_new`].
/// Test code that needs explicit IDs uses [`register_layout`] and bypasses
/// this counter — collisions between the two paths are detected per-slot.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Phase 14a (plan Q5 / §2.4) — parallel cold table of per-component lifecycle
/// hooks. Touched ONLY during archetype construction (to OR-compute the
/// archetype's [`ArchetypeFlags`](crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags),
/// plan §4.6) and at hook-registration time — never on the hot read path. Kept
/// parallel to `LAYOUTS` rather than inlined into `ComponentLayout` so the
/// latter stays at 56 B / one cache line (TRIPWIRE 2).
///
/// `[OnceLock<ComponentHooks>; MAX_COMPONENTS]` requires `ComponentHooks: Send
/// + Sync`, which holds automatically (fn-pointer-only fields — plan §8 O1).
/// Mirrors the `LAYOUTS` declaration exactly.
static HOOKS: [OnceLock<ComponentHooks>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Returns the registered hooks for `component_id`, or `None` when no hooks
/// were installed for that component.
///
/// Cold: read only during archetype construction (flag OR-compute) and by the
/// Wave-4 `trigger_on_*` dispatch fns — never on the per-frame hot path. One
/// acquire-load + branch, mirroring [`get_layout`].
#[inline]
pub fn get_hooks(component_id: usize) -> Option<&'static ComponentHooks> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    HOOKS[component_id].get()
}

/// Installs `C`'s lifecycle hooks into `HOOKS[component_id]` (Phase 14a /
/// REG). Builds a [`ComponentHooks`] via [`Component::register_hooks`] and
/// writes it once via `OnceLock::set`, mirroring [`register_new`]'s
/// write-once discipline.
///
/// Called from the derive-generated registration path (Wave 5) atomically with
/// ID assignment, before the component can appear in any archetype (so the
/// archetype-construction flag compute reads a populated slot). A second install
/// for the same id is a silent no-op (the slot is write-once); registering
/// different hooks for a component already present in a live archetype is the
/// staleness hazard the Wave-5 release scan guards against (plan §1-W3 / Q-A5).
///
/// The derive `component_id()` calls this ONLY when `C::HAS_HOOKS` is true
/// (const-gated, Change 1 of the Wave-5 soundness fix). A plain
/// `#[derive(Component)]` therefore leaves the slot UNSET, which reads as "no
/// hooks" everywhere downstream — and, crucially, leaves the slot free for the
/// runtime [`ComponentHooksBuilder`] to commit via `OnceLock::set`. Derive and
/// the runtime builder are mutually exclusive per type (the XOR contract).
#[inline]
pub fn install_hooks<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let mut hooks = ComponentHooks::default();
    C::register_hooks(&mut hooks);
    // Write-once; a same-id re-install is a silent no-op (the first writer
    // wins, matching `register_new`'s idempotent-slot semantics).
    let _ = HOOKS[component_id].set(hooks);
}

/// Commits `hooks` into `HOOKS[component_id]` via `OnceLock::set`, returning
/// `true` on success and `false` if the slot was already populated (Phase 14a
/// runtime builder / REG §6.3).
///
/// This is the runtime [`ComponentHooksBuilder`]'s sole commit path. Because
/// derive and runtime registration are mutually exclusive per type (the XOR
/// contract — see [`install_hooks`]), the builder only ever reaches an UNSET
/// slot in correct programs, so `set` succeeds. A `false` return means a
/// derive-hooked type (or a hand-`impl Component` with an inconsistent
/// `HAS_HOOKS`) slipped past the eager `register_component_hooks` collision
/// check; the builder turns that into a panic (defense in depth).
///
/// `OnceLock::set` provides the acquire/release synchronization of the payload,
/// mirroring [`install_hooks`] / [`register_new`]. No `unsafe`, no in-place
/// re-write: writing through a pointer derived from `OnceLock::get`'s shared
/// `&'static ComponentHooks` would be UB (read-only provenance), which is why
/// the previous in-place `overwrite_hooks` was removed.
pub(crate) fn try_set_hooks(component_id: usize, hooks: ComponentHooks) -> bool {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return false;
    }
    HOOKS[component_id].set(hooks).is_ok()
}

/// Storage backend selected for a component id (EnableTag plan, D5).
///
/// The default for every id is [`Table`](StorageKind::Table) — the standard
/// signature-fragmenting, per-archetype `ComponentPool` storage. An id minted
/// as an **enable tag** (`#[component(storage = "bitset")]` or
/// [`try_register_enable_tag_by_name`]) is [`Bitset`](StorageKind::Bitset):
/// filtered out of every archetype signature, no `ComponentPool`, toggled with
/// a single per-row bit.
///
/// `#[repr(u8)]` with explicit discriminants so the value round-trips losslessly
/// through the parallel cold [`STORAGE_KIND`] `AtomicU8` table. The discriminant
/// space is intentionally extensible (D7: a future relationship kind = 2).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageKind {
    /// Standard signature storage: the id is part of the archetype signature
    /// and backed by a per-archetype `ComponentPool`.
    Table = 0,
    /// Enable-bit storage: the id is NOT in any archetype signature and has no
    /// `ComponentPool`; presence is a per-row bit in an `EnableColumn`.
    Bitset = 1,
}

/// EnableTag plan (D5) — parallel cold table of per-component storage backends,
/// one [`AtomicU8`] per `ComponentId`. Mirrors the [`HOOKS`] / [`LAYOUTS`]
/// declarations rather than adding a sixth [`ComponentLayout`] field, so
/// `ComponentLayout` stays pinned at 56 B / one cache line (TRIPWIRE 2).
///
/// Touched ONLY at registration time (write-once via [`set_storage_kind`]) and
/// at archetype construction (one `Relaxed` load per id to decide signature
/// membership) — never on the per-frame hot read path.
///
/// `Relaxed` is sufficient: the kind is a registration-time, write-once datum
/// with no payload published through it. A `#[derive(Component)]` registration
/// (or the runtime mint) sets the kind atomically with id assignment, before
/// the component can appear in any archetype, so the archetype-construction read
/// observes a settled value. The default `0` reads back as
/// [`StorageKind::Table`] for every id that was never explicitly classified.
static STORAGE_KIND: [AtomicU8; MAX_COMPONENTS] =
    [const { AtomicU8::new(StorageKind::Table as u8) }; MAX_COMPONENTS];

/// Returns the storage backend registered for `component_id` (EnableTag plan,
/// D5). Defaults to [`StorageKind::Table`] for any id never classified via
/// [`set_storage_kind`].
///
/// Cold: read at archetype construction (signature-membership decision), never
/// on the per-frame hot path. One `Relaxed` load + branch.
///
/// # Panics
///
/// Never. An out-of-range id reads back as [`StorageKind::Table`] (the safe
/// default) after a debug assertion, mirroring [`get_hooks`]'s bounds discipline.
#[inline]
pub fn storage_kind(component_id: usize) -> StorageKind {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return StorageKind::Table;
    }
    match STORAGE_KIND[component_id].load(Ordering::Relaxed) {
        x if x == StorageKind::Bitset as u8 => StorageKind::Bitset,
        // 0 (the default) and any future-but-unknown discriminant fall back to
        // the safe signature default — `set_storage_kind` is the only writer
        // and only ever stores a valid discriminant.
        _ => StorageKind::Table,
    }
}

/// Classifies `component_id`'s storage backend (EnableTag plan, D5).
/// **Write-once**: the first classification of an id wins for the process
/// lifetime, mirroring the [`LAYOUTS`] / [`HOOKS`] write-once discipline.
///
/// Called from the registration path (the bitset mint and the derive's
/// `storage = "bitset"` arm) atomically with id assignment, before the
/// component can enter any archetype. A non-`Bitset` id is left at the table
/// default and never needs an explicit call.
///
/// # Panics (debug only)
///
/// A debug assertion fires if an id is RE-classified to a *different* kind —
/// reclassification is an invariant violation (the same id cannot be both a
/// table component and an enable tag). In release the first writer's kind is
/// preserved (the store is skipped) so a buggy double-classify degrades to the
/// initial decision rather than silently corrupting live archetype layouts.
// Step 4/5 (Wave 2) wired live consumers: the archetype-construction signature
// filter (`Archetype::create_by_ids` / `register_component_inplace`) reads
// `storage_kind`, and `try_register_enable_tag_by_name` (now live via
// `EcsMaster::register_enable_tag`) writes through this. The Step-1 forward-seam
// `#[allow(dead_code)]` is therefore removed. (The derive's `storage = "bitset"`
// arm — Step 10 — is the remaining future writer.)
pub(crate) fn set_storage_kind(component_id: usize, kind: StorageKind) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    let current = STORAGE_KIND[component_id].load(Ordering::Relaxed);
    if current == kind as u8 {
        // Idempotent same-kind (re)classification — no-op.
        return;
    }
    debug_assert!(
        current == StorageKind::Table as u8,
        "set_storage_kind: ComponentId {} already classified as {:?}, refused to \
         reclassify as {:?} (storage kind is write-once)",
        component_id,
        storage_kind(component_id),
        kind
    );
    if current == StorageKind::Table as u8 {
        STORAGE_KIND[component_id].store(kind as u8, Ordering::Relaxed);
    }
}

/// Installs the storage backend for a derived component into the cold
/// `STORAGE_KIND` table from the type's compile-time [`Component::STORAGE_IS_BITSET`]
/// const (EnableTag plan, D5 — the `#[component(storage = "bitset")]` derive arm,
/// Wave 5 Step 10).
///
/// This is the **public** counterpart of the `pub(crate)` [`set_storage_kind`]:
/// `#[derive(Component)]` expands into downstream crates, where the `pub(crate)`
/// writer is unreachable, so the derive's `component_id()` install path calls
/// this `pub` wrapper instead — exactly mirroring how it calls [`install_hooks`]
/// for `C::HAS_HOOKS`. The derive emits this call ONLY when
/// `C::STORAGE_IS_BITSET` is `true` (const-gated), so a plain
/// `#[derive(Component)]` const-folds it away and the id stays at the
/// [`StorageKind::Table`] default — zero cost for non-tag components.
///
/// Write-once and idempotent through [`set_storage_kind`]: it runs once per
/// type per process (behind the `component_id()` `OnceLock`), atomically with id
/// assignment and therefore before the id can enter any archetype.
///
/// # Panics
///
/// In debug, if `component_id` is reclassified to a different kind (see
/// [`set_storage_kind`]). The XOR-by-construction discipline (a single
/// `STORAGE_IS_BITSET` const per type) makes this unreachable for derived types.
#[inline]
pub fn install_storage_kind<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    // The derive const-gates this call on `C::STORAGE_IS_BITSET`, so in practice
    // the branch is always taken; the explicit test keeps the wrapper correct if
    // a hand-`impl Component` calls it with the table default.
    if C::STORAGE_IS_BITSET {
        set_storage_kind(component_id, StorageKind::Bitset);
    }
}

/// Name-keyed enable-tag mint (EnableTag plan, D5). Mirrors
/// [`try_register_tag_by_name`] — idempotent per name, returns `None` when the
/// shared `MAX_COMPONENTS` budget is exhausted and `name` was never minted —
/// but additionally classifies the minted id as [`StorageKind::Bitset`] so it
/// is filtered out of every archetype signature.
///
/// A name already minted as a *non-bitset* dynamic tag is returned as-is in
/// debug only after the same-kind reclassification assertion fires (storage
/// kind is write-once); the two name spaces are otherwise independent
/// (`tag_by_name` vs the enable-tag name).
///
/// Lookup, capacity check, mint, and name-intern run under the shared
/// name-intern lock (inside [`try_register_tag_by_name`]), so two threads racing
/// the same name cannot double-mint. The kind classification
/// ([`set_storage_kind`]) runs AFTER that lock is released — this is sound
/// because storage kind is write-once-idempotent and registration completes
/// before any archetype can read the kind (the same registration-before-
/// construction ordering the `STORAGE_KIND` table relies on). See
/// [`try_register_tag_by_name`] for the leak bound (identical; no extra alloc).
// Step 5 (Wave 2) wired the live caller: `EcsMaster::register_enable_tag` /
// `try_register_enable_tag` delegate here, mirroring how `EcsMaster::register_tag`
// delegates to `try_register_tag_by_name`. The Step-1 forward-seam
// `#[allow(dead_code)]` is therefore removed.
pub(crate) fn try_register_enable_tag_by_name(name: &str) -> Option<EnableTagId> {
    let tag = try_register_tag_by_name(name)?;
    // Classify the minted id as bitset storage. Write-once: a re-mint of the
    // same name re-runs this with the already-bitset id, which is the idempotent
    // same-kind no-op above.
    set_storage_kind(tag.0.0, StorageKind::Bitset);
    Some(EnableTagId(tag.0))
}

/// Phase 21 (H1) — process-global "ever placed in ANY archetype" bitmask, one
/// bit per `ComponentId`. 512 components = 8 × `AtomicU64`.
///
/// Set at the archetype-creation funnels (`ArchetypeMaster::create_archetype`
/// and the `add_existing_archetype` bypass — the same two sites that seed
/// observer flags) for every component id in the new archetype; read by
/// `EcsMaster::register_component_hooks`'s staleness gate.
///
/// # Why process-global, not per-world
///
/// The `HOOKS` table above is process-global: hooks registered through any
/// world apply to the component type in EVERY world. A per-world staleness
/// scan therefore had a hole (audit H1): world A could register hooks for `C`
/// while world B already had `C` live in an archetype whose `ArchetypeFlags`
/// were OR-computed without the hook bit — the hook was then silently skipped
/// in B. This global mirrors the global scope of `HOOKS` itself, so the gate
/// matches the table it protects. Like `LAYOUTS` / `HOOKS`, this is
/// metadata-class global state — it never references world-owned storage.
///
/// Bits are never cleared (not even by `EcsMaster::clear()`): a cleared world
/// proves nothing about OTHER worlds, and the `HOOKS` slot is write-once for
/// the process lifetime anyway.
static EVER_ARCHETYPED: [AtomicU64; MAX_COMPONENTS.div_ceil(64)] =
    [const { AtomicU64::new(0) }; MAX_COMPONENTS.div_ceil(64)];

/// Marks `component_id` as having been placed in at least one archetype of at
/// least one world (Phase 21 H1). Called from the archetype-creation funnels;
/// cold (archetype creation only), idempotent.
///
/// Relaxed ordering: the bit feeds a config-time courtesy panic in
/// `register_component_hooks`, not a soundness fence — no payload is
/// published through it, and a same-thread `&mut`-sequenced caller (the only
/// realistic registration pattern) observes its own prior `fetch_or` anyway.
#[inline]
pub(crate) fn mark_ever_archetyped(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    EVER_ARCHETYPED[component_id / 64].fetch_or(1u64 << (component_id % 64), Ordering::Relaxed);
}

/// Returns `true` if `component_id` was ever placed in any archetype of any
/// world in this process (Phase 21 H1). See [`EVER_ARCHETYPED`] for ordering
/// and scope rationale.
#[inline]
pub(crate) fn was_ever_archetyped(component_id: usize) -> bool {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return false;
    }
    EVER_ARCHETYPED[component_id / 64].load(Ordering::Relaxed) & (1u64 << (component_id % 64)) != 0
}

/// Id-keyed hook registration (Phase 22 D8) — the single entry point into the
/// write-once `HOOKS` table for ids that have no Rust type to name (dynamic
/// tags), and the delegation target of the typed runtime path
/// ([`ComponentHooksBuilder`](crate::ecs::core::component::hooks::builder::ComponentHooksBuilder)'s
/// commit).
///
/// # Errors
///
/// - [`HooksError::AlreadyArchetyped`] — `component_id` was already placed in
///   an archetype of some world in this process (Phase-21 H1 staleness gate).
///   An archetype's `ArchetypeFlags` hook bits are OR-computed once at
///   creation; hooks registered after the fact would silently never fire
///   there. Without this gate the NATURAL dynamic-tag call order
///   `register_tag → add_tag → register_hooks_by_id` would compile, succeed,
///   and lie. **Contract: mint → register hooks → first attach.**
/// - [`HooksError::AlreadyRegistered`] — the slot is already populated
///   (write-once semantics, unchanged from the typed paths).
///
/// # Panics
///
/// If `component_id.0 >= MAX_COMPONENTS` — an out-of-range id is a caller bug
/// (every id minted by this registry is in range), mirroring
/// [`register_layout`]'s release-active bound assert.
pub fn register_hooks_by_id(
    component_id: ComponentId,
    hooks: ComponentHooks,
) -> Result<(), HooksError> {
    assert!(
        component_id.0 < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id.0,
        MAX_COMPONENTS
    );
    // H1 staleness gate first: even an unoccupied slot is rejected once the id
    // has been archetyped — the flags of that archetype are already frozen.
    if was_ever_archetyped(component_id.0) {
        return Err(HooksError::AlreadyArchetyped { component_id });
    }
    if !try_set_hooks(component_id.0, hooks) {
        return Err(HooksError::AlreadyRegistered { component_id });
    }
    Ok(())
}

/// Allocates a fresh `ComponentId` from the global counter and stores
/// `ComponentLayout::new_static::<T>()` in the corresponding `LAYOUTS` slot.
///
/// Production path: called from `#[derive(Component)]`-generated
/// `T::component_id()` via a per-monomorphization `OnceLock`. Each concrete
/// `T` gets exactly one ID across the process lifetime, regardless of how
/// many threads call `T::component_id()` concurrently.
///
/// # Panics
/// - If `NEXT_ID` reaches `MAX_COMPONENTS`.
/// - If the slot at the minted index is already occupied by a *different*
///   type — see module-level "Collision detection" docs.
pub fn register_new<T: 'static>() -> usize {
    let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < MAX_COMPONENTS,
        "ComponentRegistry exhausted: NEXT_ID reached {}, MAX_COMPONENTS = {}",
        raw,
        MAX_COMPONENTS
    );
    let layout = ComponentLayout::new_static::<T>();
    match LAYOUTS[raw].set(layout) {
        Ok(()) => raw,
        Err(_) => {
            let existing = LAYOUTS[raw]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id == TypeId::of::<T>() {
                raw
            } else {
                panic!(
                    "ComponentId {} occupied by type {}, refused to register {}",
                    raw,
                    existing.type_name,
                    std::any::type_name::<T>()
                )
            }
        }
    }
}

/// Phase 22 (D3): process-global name → id intern for dynamic tags. COLD:
/// touched at mint/lookup only (setup time) — never on the per-frame hot path.
///
/// `Mutex + HashMap` is justified per the Phase-12.5 `QueryTypeId`-intern
/// precedent; one concrete global (not a generic-fn-body static) avoids the
/// monomorphization-collapse trap. Capacity + idempotency are atomic under
/// this lock; idempotency is NAME-keyed, never TypeId-keyed (all dynamic tags
/// share [`DynamicTagMarker`]'s TypeId — plan O2). Names are leaked once per
/// successfully minted unique tag (bounded ≤ [`MAX_COMPONENTS`], the #53
/// bounded-leak precedent).
static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>> = OnceLock::new();

/// Lazily initializes and returns the [`TAG_NAMES`] intern table.
fn tag_names() -> &'static Mutex<HashMap<Box<str>, TagId>> {
    TAG_NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Name-keyed dynamic-tag mint (Phase 22 D3 mint protocol). Idempotent per
/// name: a present name returns its existing [`TagId`] (success), even after
/// the budget is exhausted. `None` means the shared `MAX_COMPONENTS` budget is
/// exhausted and `name` was never minted.
///
/// The whole decision — lookup, capacity check, mint, intern — runs under the
/// [`TAG_NAMES`] lock, so two threads racing the same name cannot double-mint.
///
/// # Leak bound
///
/// The name is leaked (`Box::leak`) only on the mint path, after the
/// early-return budget check. One residual window exists: a concurrent typed
/// [`register_new`] (whose `fetch_add` is not under this lock) can claim the
/// last slot between our budget peek and the CAS in [`try_register_dynamic`],
/// leaking one name copy. Past exhaustion `NEXT_ID` never decreases, so every
/// later failing call returns at the peek, allocation-free — the leak is
/// bounded by the number of distinct names racing the exact exhaustion
/// boundary, in practice zero.
pub(crate) fn try_register_tag_by_name(name: &str) -> Option<TagId> {
    let mut names = tag_names()
        .lock()
        .expect("invariant: TAG_NAMES lock poisoned only after a registry-invariant panic (dynamic_slot_occupied_panic under the guard) — process already condemned");
    if let Some(&tag) = names.get(name) {
        return Some(tag);
    }
    if NEXT_ID.load(Ordering::Relaxed) >= MAX_COMPONENTS {
        return None;
    }
    let leaked: &'static str = Box::leak(Box::<str>::from(name));
    let id = try_register_dynamic(ComponentLayout::new_dynamic_tag(leaked))?;
    let tag = TagId(id);
    names.insert(Box::from(name), tag);
    Some(tag)
}

/// Cold name → [`TagId`] lookup (Phase 22 D3). `None` if `name` was never
/// successfully minted. Never mints.
pub(crate) fn tag_by_name(name: &str) -> Option<TagId> {
    tag_names()
        .lock()
        .expect("invariant: TAG_NAMES lock poisoned only after a registry-invariant panic (dynamic_slot_occupied_panic under the guard) — process already condemned")
        .get(name)
        .copied()
}

/// Crate-internal fallible mint (Phase 22 D3): allocates a fresh
/// [`ComponentId`] via a bounded CAS on the shared [`NEXT_ID`] and stores
/// `layout` in the slot. Returns `None` at the `MAX_COMPONENTS` ceiling — it
/// does NOT inherit [`register_new`]'s release exhaustion assert (dynamic
/// mints are user-data-driven; the panic is opt-in via `register_tag`).
///
/// Coexistence with [`register_new`]'s `fetch_add` is sound: a concurrent
/// `fetch_add` merely makes the CAS retry on the fresh value; the CAS path
/// never pushes `NEXT_ID` past `MAX_COMPONENTS`.
///
/// # Panics
///
/// If the freshly CAS-minted slot is already occupied (plan O2). That is an
/// invariant violation — reachable only via the test-only [`register_layout`]
/// slot-pinning escape hatch overlapping the production counter — and it MUST
/// panic rather than take [`register_new`]'s same-TypeId idempotent return:
/// every dynamic tag shares [`DynamicTagMarker`]'s TypeId, so the idempotent
/// arm would alias two distinct tag names to one id.
pub(crate) fn try_register_dynamic(layout: ComponentLayout) -> Option<ComponentId> {
    let mut current = NEXT_ID.load(Ordering::Relaxed);
    loop {
        if current >= MAX_COMPONENTS {
            return None;
        }
        // Relaxed on success AND failure: this CAS only provides uniqueness of
        // the minted index. The `OnceLock::set` below remains the
        // acquire/release synchronization point publishing the layout payload
        // — the exact contract `register_new`'s Relaxed `fetch_add` documents
        // (module-level "Threading" notes).
        match NEXT_ID.compare_exchange_weak(
            current,
            current + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
    if LAYOUTS[current].set(layout).is_err() {
        dynamic_slot_occupied_panic(current);
    }
    Some(ComponentId(current))
}

/// Cold panic site for [`try_register_dynamic`]'s slot-occupied invariant
/// violation (plan O2). Kept out of line so the mint body stays compact.
#[cold]
#[inline(never)]
fn dynamic_slot_occupied_panic(component_id: usize) -> ! {
    let existing = LAYOUTS[component_id]
        .get()
        .map(|l| l.type_name)
        .unwrap_or("<unknown>");
    panic!(
        "try_register_dynamic: freshly minted ComponentId {component_id} is already \
         occupied by type {existing} — a slot was pinned out-of-band (test-only \
         `register_layout` overlapping the production counter). Refusing the \
         same-TypeId idempotent return: dynamic tags share one sentinel TypeId, so \
         it would alias two distinct tag names to one id."
    );
}

/// Test-only escape hatch: registers `T` under an explicit `component_id`.
///
/// Production code must not call this — use `T::component_id()` (which goes
/// through [`register_new`]). Tests use this to install components under
/// known, fixed IDs without depending on `NEXT_ID`'s value.
///
/// # Panics
/// - If `component_id >= MAX_COMPONENTS`.
/// - If the slot is already occupied by a *different* type. Same-type
///   re-registration is silently idempotent.
#[doc(hidden)]
pub fn register_layout<T: 'static>(component_id: usize) {
    assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let layout = ComponentLayout::new_static::<T>();
    match LAYOUTS[component_id].set(layout) {
        Ok(()) => {}
        Err(_) => {
            let existing = LAYOUTS[component_id]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id != TypeId::of::<T>() {
                panic!(
                    "ComponentId {} occupied by type {}, refused to register {}",
                    component_id,
                    existing.type_name,
                    std::any::type_name::<T>()
                )
            }
            // Same type — silent no-op (idempotent).
        }
    }
}

/// Test-only: overrides the process-global `NEXT_ID` counter.
///
/// Lets a test pin where the next [`register_new`]-minted [`ComponentId`]
/// lands — e.g. driving `NEXT_ID` to `MAX_COMPONENTS - 1` to exercise the
/// exhaustion panic, or parking it in a high range so a test's
/// `register_new` calls do not collide with low fixed ids used elsewhere.
///
/// # Test isolation only
///
/// `NEXT_ID` is process-global and shared by every test in the binary (which
/// run concurrently by default). This helper does NOT reset the per-slot
/// `LAYOUTS` `OnceLock`s — those are write-once for the process lifetime and
/// cannot be cleared. Callers must therefore choose a target value whose slot
/// range is not already populated by another test, exactly as the existing
/// fixed-id partitioning convention requires. Use sparingly and prefer giving
/// the test its own private id range.
// Preventive capability (C-3): added for the tester to write a
// `register_new_exhaustion_panics` test; no current in-crate caller, so the
// dead-code lint is expected and silenced rather than papered over by a fake
// use.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn set_next_id_for_test(value: usize) {
    NEXT_ID.store(value, Ordering::Relaxed);
}

/// Test-only: reads the current value of the process-global `NEXT_ID` counter
/// (the id the next [`register_new`] call would mint).
///
/// Companion to [`set_next_id_for_test`] — lets a test snapshot the counter so
/// it can restore it afterwards, or assert on the number of ids minted. See
/// that function's note on test isolation.
#[cfg(test)]
#[allow(dead_code)] // Preventive capability (C-3); see `set_next_id_for_test`.
pub(crate) fn next_id_for_test() -> usize {
    NEXT_ID.load(Ordering::Relaxed)
}

/// Reverse-lookup: returns `true` if any registered component slot in
/// `LAYOUTS` carries the given `TypeId`.
///
/// Used by the resource registry (`register_new` in `resource_registry`) to
/// enforce the Component-vs-Resource exclusivity invariant (M6): a single
/// Rust type may not be registered as both a `Component` and a `Resource`.
///
/// # Cost
/// O(MAX_COMPONENTS) — a single scan of the global `OnceLock` table. Called
/// only during registration (cold path) and never on the hot per-frame
/// system loop.
///
/// # Threading
/// Safe to call concurrently. Each `OnceLock::get` is an acquire-load; the
/// scan observes a monotonically growing set of populated slots.
pub fn is_type_registered_as_component(type_id: TypeId) -> bool {
    for slot in LAYOUTS.iter() {
        if let Some(info) = slot.get()
            && info.type_id == type_id
        {
            return true;
        }
    }
    false
}

/// Retrieves layout information for a component by its ID.
/// Returns `None` if the component hasn't been registered yet.
#[inline]
pub fn get_layout(component_id: usize) -> Option<&'static ComponentLayout> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );

    if component_id >= MAX_COMPONENTS {
        return None;
    }

    LAYOUTS[component_id].get()
}

/// Optimized function to get the size of a component by ID.
#[inline]
pub fn get_component_size(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.size)
}

/// Optimized function to get the alignment of a component by ID.
#[inline]
pub fn get_component_alignment(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.alignment)
}

/// Creates a memory layout for a component without going through `ComponentLayout`.
#[inline]
pub fn get_component_memory_layout(component_id: usize) -> Option<Layout> {
    Some(get_layout(component_id)?.layout())
}

/// Ultra-fast access to component size when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
///   Violating either yields UB.
#[inline]
pub unsafe fn get_component_size_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().size }
}

/// Ultra-fast access to component alignment when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
///   Violating either yields UB.
#[inline]
pub unsafe fn get_component_alignment_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().alignment }
}

/// Ultra-fast access to component layout when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
///   Violating either yields UB.
#[inline]
pub unsafe fn get_layout_unchecked(component_id: usize) -> &'static ComponentLayout {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked() }
}

/// Ultra-fast access to component memory layout when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
///   Violating either yields UB.
#[inline]
pub unsafe fn get_component_memory_layout_unchecked(component_id: usize) -> Layout {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    let layout = unsafe { get_layout_unchecked(component_id) };
    // SAFETY: size/alignment come from a registered `ComponentLayout`, valid by construction.
    unsafe { Layout::from_size_align_unchecked(layout.size, layout.alignment) }
}

/// Ultra-fast access to component type ID when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
///   Violating either yields UB.
#[inline]
pub unsafe fn get_component_type_id_unchecked(component_id: usize) -> TypeId {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    unsafe { get_layout_unchecked(component_id).type_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use large IDs in this module to avoid colliding with other test modules
    // that register components under low IDs (0-10). OnceLock slots are global
    // and persist for the lifetime of the test binary.
    const TEST_BASE: usize = 450;

    // --- register_layout + get_layout ---

    #[test]
    fn register_layout_then_get_returns_matching_fields() {
        let id = TEST_BASE;
        register_layout::<u32>(id);
        let layout = get_layout(id).expect("layout must be present after register");
        assert_eq!(layout.size, std::mem::size_of::<u32>(), "size must match u32");
        assert_eq!(
            layout.alignment,
            std::mem::align_of::<u32>(),
            "alignment must match u32"
        );
        assert_eq!(
            layout.type_id,
            TypeId::of::<u32>(),
            "type_id must match TypeId::of::<u32>()"
        );
    }

    #[test]
    fn get_layout_unregistered_returns_none() {
        // ID 499 is unused by any other test in this crate.
        assert!(
            get_layout(499).is_none(),
            "unregistered component must return None"
        );
    }

    #[test]
    fn register_layout_idempotent_same_type() {
        // Registering the same type twice must keep the first registration.
        let id = TEST_BASE + 1;
        register_layout::<u64>(id);
        register_layout::<u64>(id); // second call — must be silent no-op
        let layout = get_layout(id).expect("slot must remain populated");
        assert_eq!(layout.size, std::mem::size_of::<u64>());
    }

    // ----- NEW TESTS: Phase 1b C-003 / M-015 -----

    // Slot allocation map for new tests (must not collide with existing tests):
    //   TEST_BASE+0 = 450 → u32 (register_layout_then_get_returns_matching_fields)
    //   TEST_BASE+1 = 451 → u64 (register_layout_idempotent_same_type)
    //   TEST_BASE+2 = 452 → (unused by old tests; reserved for collision tests below)
    //   TEST_BASE+3 = 453 → f32
    //   TEST_BASE+4 = 454 → f64
    //   TEST_BASE+5 = 455 → f32
    //   TEST_BASE+6 = 456 → u128
    //   TEST_BASE+7..+11 = 457..461 → new tests below
    //   499, 498 → already claimed by get_layout / get_component_size unregistered tests

    // Distinct local struct types for collision tests — defined at module scope so
    // that `TypeId::of::<T>()` is unambiguous across parallel test threads.
    #[repr(C)] struct ColTypeA(u32);
    #[repr(C)] struct ColTypeB(u64);
    // A third type to test register_new distinctness independently.
    #[repr(C)] struct RegNewTypeA(u8);
    #[repr(C)] struct RegNewTypeB(u16);

    /// register_new<TypeA> and register_new<TypeB> must return different IDs.
    ///
    /// This test calls register_new directly (bypassing the macro-generated OnceLock).
    /// Since register_new uses fetch_add, each call is guaranteed a unique slot.
    #[test]
    fn register_new_assigns_distinct_ids_for_distinct_types() {
        let id_a = register_new::<RegNewTypeA>();
        let id_b = register_new::<RegNewTypeB>();
        assert_ne!(
            id_a,
            id_b,
            "register_new must assign different IDs to different types (got id_a={id_a}, id_b={id_b})"
        );
        // Verify both slots are populated with the correct type.
        assert_eq!(
            get_layout(id_a).expect("slot for RegNewTypeA must be populated").type_id,
            TypeId::of::<RegNewTypeA>(),
            "layout at id_a must carry RegNewTypeA type_id"
        );
        assert_eq!(
            get_layout(id_b).expect("slot for RegNewTypeB must be populated").type_id,
            TypeId::of::<RegNewTypeB>(),
            "layout at id_b must carry RegNewTypeB type_id"
        );
    }

    /// register_new<T> collision-idempotent branch: if the slot that NEXT_ID picks
    /// is already occupied by the same type (e.g., pre-populated via register_layout
    /// or by a concurrent thread), the call returns the existing ID without panic.
    ///
    /// We manufacture this by:
    ///   1. Pre-registering ColTypeA under a known slot (TEST_BASE+7) via register_layout.
    ///   2. Loading that slot from LAYOUTS directly via get_layout — verifying it is set.
    ///   3. Calling register_new::<ColTypeA>() again (a second monomorphization call).
    ///      Because NEXT_ID has already advanced past TEST_BASE+7 in the process,
    ///      the second call gets a fresh slot — it does NOT hit the collision branch.
    ///
    /// LIMITATION: The collision-idempotent branch in register_new (the Err arm of
    /// OnceLock::set where existing.type_id == TypeId::of::<T>()) can only be triggered
    /// if NEXT_ID happens to pick a slot that was pre-populated for the same type via
    /// register_layout. Since NEXT_ID is private and not resettable from tests, we
    /// cannot manufacture this scenario deterministically. The branch is exercised
    /// indirectly through the integration test `derive_component_emits_lazy_id` (concurrent
    /// first-call scenario). See also: test_next_id_exhaustion_not_testable note below.
    ///
    /// What we CAN test here: register_new<T> followed by register_new<T> registers T
    /// under TWO different slots (since fetch_add is monotone), both returning Some(T).
    #[test]
    fn register_new_second_call_for_same_type_occupies_new_slot() {
        // Both calls return distinct IDs because fetch_add always advances.
        let id1 = register_new::<ColTypeA>();
        let id2 = register_new::<ColTypeA>();
        // The IDs differ — fetch_add is strictly monotone.
        assert_ne!(
            id1,
            id2,
            "direct register_new calls are not idempotent — each call mints a new slot \
             (idempotency is provided by the macro-generated OnceLock wrapper)"
        );
        // Both slots hold ColTypeA.
        assert_eq!(
            get_layout(id1).expect("first slot must be populated").type_id,
            TypeId::of::<ColTypeA>(),
            "first slot must hold ColTypeA"
        );
        assert_eq!(
            get_layout(id2).expect("second slot must be populated").type_id,
            TypeId::of::<ColTypeA>(),
            "second slot must also hold ColTypeA (two separate registrations)"
        );
    }

    /// Collision detection in register_layout: registering a different type in an
    /// already-occupied slot must panic with a message naming both types.
    ///
    /// Slot 462 is reserved for this test. The panic expected substring matches the
    /// format string: "ComponentId {} occupied by type {}, refused to register {}".
    #[test]
    #[should_panic(expected = "occupied by type")]
    fn register_layout_collision_with_different_type_panics() {
        const COLLISION_SLOT: usize = 462;
        // First registration occupies the slot.
        register_layout::<ColTypeA>(COLLISION_SLOT);
        // Second registration with a different type must panic.
        register_layout::<ColTypeB>(COLLISION_SLOT);
    }

    /// Collision detection idempotent path: registering the SAME type twice under the
    /// same explicit slot must be a silent no-op (no panic, slot remains valid).
    ///
    /// Slot 465 is reserved for this test.
    #[test]
    fn register_layout_collision_with_same_type_is_silent_noop() {
        const IDEMPOTENT_SLOT: usize = 465;
        register_layout::<ColTypeA>(IDEMPOTENT_SLOT);
        register_layout::<ColTypeA>(IDEMPOTENT_SLOT); // second call — must be silent
        let layout = get_layout(IDEMPOTENT_SLOT)
            .expect("slot must remain populated after idempotent re-registration");
        assert_eq!(
            layout.type_id,
            TypeId::of::<ColTypeA>(),
            "slot type_id must remain ColTypeA after silent no-op"
        );
        assert_eq!(
            layout.size,
            std::mem::size_of::<ColTypeA>(),
            "slot size must remain correct after silent no-op"
        );
    }

    /// register_layout panics when component_id == MAX_COMPONENTS (out-of-range by one).
    ///
    /// The assert! in register_layout fires in both debug and release. Expected message
    /// matches: "Component ID {} exceeds maximum allowed ({})".
    ///
    /// Note: the developer left a test `register_layout_at_max_components_boundary_panics`
    /// that already covers this via catch_unwind. This companion test uses #[should_panic]
    /// with a tighter expected substring to lock in the panic message format.
    #[test]
    #[should_panic(expected = "exceeds maximum allowed")]
    fn register_layout_at_max_components_panics_with_expected_message() {
        register_layout::<u8>(MAX_COMPONENTS);
    }

    // NOTE: a `register_new` exhaustion test (driving NEXT_ID to MAX_COMPONENTS)
    // is now enabled by the `#[cfg(test)] set_next_id_for_test` / `next_id_for_test`
    // accessors above. The tester owns writing `register_new_exhaustion_panics`;
    // because NEXT_ID is process-global and shared across concurrently-running
    // tests, such a test must snapshot+restore the counter (or run serially) so it
    // does not perturb sibling tests' id assignments.

    // ----- END NEW TESTS -----

    #[test]
    fn register_layout_at_max_components_boundary_panics() {
        // MAX_COMPONENTS is out-of-range (valid indices: 0..MAX_COMPONENTS-1).
        // register_layout now asserts (not debug_assert) — always panics OOB.
        let result = std::panic::catch_unwind(|| {
            register_layout::<u32>(MAX_COMPONENTS);
        });
        assert!(result.is_err(), "out-of-range register_layout must panic");
    }

    #[test]
    fn get_layout_at_index_zero_is_none_before_register() {
        // Index 0 is a valid index but we never register a component there
        // in this test module. It may have been registered by another module —
        // we can only assert that get_layout does not panic on index 0.
        let _ = get_layout(0); // must not panic regardless of value
    }

    // --- get_component_size / get_component_alignment ---

    #[test]
    fn get_component_size_matches_registered_layout() {
        let id = TEST_BASE + 3;
        register_layout::<f32>(id);
        assert_eq!(
            get_component_size(id),
            Some(std::mem::size_of::<f32>()),
            "get_component_size must agree with size_of::<f32>()"
        );
    }

    #[test]
    fn get_component_alignment_matches_registered_layout() {
        let id = TEST_BASE + 4;
        register_layout::<f64>(id);
        assert_eq!(
            get_component_alignment(id),
            Some(std::mem::align_of::<f64>()),
            "get_component_alignment must agree with align_of::<f64>()"
        );
    }

    #[test]
    fn get_component_size_unregistered_returns_none() {
        assert!(
            get_component_size(498).is_none(),
            "unregistered component must return None from get_component_size"
        );
    }

    // --- get_layout_unchecked (unsafe hot path) ---

    #[test]
    fn get_layout_unchecked_after_register_returns_correct_size() {
        let id = TEST_BASE + 5;
        register_layout::<f32>(id);
        // SAFETY: `id` is < MAX_COMPONENTS and `register_layout` was just called.
        let layout = unsafe { get_layout_unchecked(id) };
        assert_eq!(
            layout.size,
            std::mem::size_of::<f32>(),
            "unchecked accessor must return the registered layout"
        );
    }

    // --- get_component_memory_layout ---

    #[test]
    fn get_component_memory_layout_produces_valid_layout() {
        let id = TEST_BASE + 6;
        register_layout::<u128>(id);
        let mem_layout =
            get_component_memory_layout(id).expect("must return Some after register");
        assert_eq!(mem_layout.size(), std::mem::size_of::<u128>());
        assert_eq!(mem_layout.align(), std::mem::align_of::<u128>());
    }

    // --- EnableTag plan, Step 1: STORAGE_KIND + EnableTagId bridge ---
    //
    // Dedicated id range 470-475 for the direct storage-kind classification
    // tests. STORAGE_KIND is process-global write-once like LAYOUTS, so each id
    // here must be unique across the whole test binary (the fixed-id
    // partitioning convention). These ids do NOT overlap the layout tests'
    // 450-465 / 498-499 range.

    #[test]
    fn storage_kind_defaults_to_table() {
        // An id never classified reads back as the table default.
        assert_eq!(
            storage_kind(470),
            StorageKind::Table,
            "unclassified id must default to StorageKind::Table"
        );
    }

    #[test]
    fn set_storage_kind_round_trips_bitset() {
        // 404-407 reserved for these storage-kind tests; the prior 471-474
        // collided with par_chunk's COMP_W7B(471)/COMP_W7POS(472) data ids —
        // marking them Bitset made C1 filter them out of the signature, so
        // par_chunk's create_entity failed in the shared lib-test process.
        set_storage_kind(404, StorageKind::Bitset);
        assert_eq!(
            storage_kind(404),
            StorageKind::Bitset,
            "set_storage_kind(Bitset) must round-trip through storage_kind"
        );
    }

    #[test]
    fn set_storage_kind_same_kind_is_idempotent() {
        // Two identical classifications are a silent no-op (no panic).
        set_storage_kind(405, StorageKind::Bitset);
        set_storage_kind(405, StorageKind::Bitset);
        assert_eq!(storage_kind(405), StorageKind::Bitset);
    }

    #[test]
    fn set_storage_kind_explicit_table_round_trips() {
        // Explicitly classifying Table on a fresh id is a same-kind no-op and
        // leaves the default in place.
        set_storage_kind(406, StorageKind::Table);
        assert_eq!(storage_kind(406), StorageKind::Table);
    }

    /// Write-once enforcement: reclassifying an id to a DIFFERENT kind must trip
    /// the debug assertion. Runs only in debug builds (where `debug_assert!`
    /// is active) — release skips the store and preserves the first kind.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "already classified")]
    fn set_storage_kind_reclassify_different_kind_panics_in_debug() {
        set_storage_kind(407, StorageKind::Bitset);
        // Reclassifying to a different kind is an invariant violation.
        set_storage_kind(407, StorageKind::Table);
    }

    #[test]
    fn enable_tag_id_bridges_to_component_id_round_trip() {
        let cid = ComponentId(475);
        let tag = EnableTagId(cid);
        assert_eq!(
            tag.component_id(),
            cid,
            "EnableTagId::component_id must return the wrapped ComponentId"
        );
        let via_from: ComponentId = tag.into();
        assert_eq!(
            via_from, cid,
            "From<EnableTagId> for ComponentId must round-trip"
        );
    }

    #[test]
    fn register_enable_tag_mints_and_sets_kind_bitset() {
        // Dynamic mint allocates a fresh id from NEXT_ID (no fixed-id collision)
        // and classifies it as bitset storage.
        let tag = try_register_enable_tag_by_name("enable_tag_step1_mint")
            .expect("budget must not be exhausted in this test binary");
        assert_eq!(
            storage_kind(tag.component_id().0),
            StorageKind::Bitset,
            "a minted enable tag must be classified as StorageKind::Bitset"
        );
    }

    #[test]
    fn register_enable_tag_is_idempotent_per_name() {
        let a = try_register_enable_tag_by_name("enable_tag_step1_idem")
            .expect("first mint must succeed");
        let b = try_register_enable_tag_by_name("enable_tag_step1_idem")
            .expect("second mint of the same name must succeed");
        assert_eq!(
            a, b,
            "an enable tag mint is idempotent per name (same id returned)"
        );
        assert_eq!(
            storage_kind(a.component_id().0),
            StorageKind::Bitset,
            "the re-minted id stays bitset (write-once same-kind no-op)"
        );
    }
}
