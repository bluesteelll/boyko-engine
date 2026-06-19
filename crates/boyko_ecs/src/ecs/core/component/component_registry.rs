//! Global registry of component layouts.
//!
//! # Component ID assignment
//!
//! Each distinct type `T` implementing [`Component`]
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
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ecs::core::component::component::{Component, RequiredBuilder};
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
    /// `DynamicTagMarker` sentinel, shared by EVERY dynamic tag. Because the
    /// sentinel is uninhabited and private it can never collide with a user
    /// type, and the typed-pool `debug_assert(component_type_id ==
    /// TypeId::of::<T>())` correctly rejects typed access to dynamic-tag ids.
    ///
    /// `name` is the interned tag name (leaked once per unique tag by the
    /// `TAG_NAMES` mint path) — it doubles as `type_name` for diagnostics.
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
/// dynamic tag, and only the `TAG_NAMES` mint path can issue one.
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
/// `try_register_enable_tag_by_name`, which sets the id's
/// `STORAGE_KIND` to [`StorageKind::Bitset`].
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
/// runtime `ComponentHooksBuilder` to commit via `OnceLock::set`. Derive and
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
/// `try_register_enable_tag_by_name`) is [`Bitset`](StorageKind::Bitset):
/// filtered out of every archetype signature, no `ComponentPool`, toggled with
/// a single per-row bit.
///
/// `#[repr(u8)]` with explicit discriminants so the value round-trips losslessly
/// through the parallel cold `STORAGE_KIND` `AtomicU8` table. The discriminant
/// space is intentionally extensible (2 = Dense; 3 reserved for relationships).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageKind {
    /// Standard signature storage: the id is part of the archetype signature
    /// and backed by a per-archetype `ComponentPool`.
    Table = 0,
    /// Enable-bit storage: the id is NOT in any archetype signature and has no
    /// `ComponentPool`; presence is a per-row bit in an `EnableColumn`.
    Bitset = 1,
    /// Dense (non-fragmenting) storage (Dense plan, D0): the id is NOT in any
    /// archetype signature and has NO per-archetype `ComponentPool`; instead a
    /// single global `DenseStore` column (D1) holds every instance across all
    /// archetypes. Excluded from the signature exactly like [`Bitset`], but —
    /// unlike a bitset tag — it owns a global backing store rather than no
    /// storage. Always [`ResidencyKind::Cpu`] (Dense plan W1).
    ///
    /// [`Bitset`]: StorageKind::Bitset
    Dense = 2,
}

/// Returns `true` iff `kind` is a **signature** storage backend — i.e. an id of
/// this kind participates in the archetype signature mask and owns a
/// per-archetype `ComponentPool` (Dense plan C1).
///
/// Only [`StorageKind::Table`] is a signature kind. Both [`StorageKind::Bitset`]
/// (enable tags) and [`StorageKind::Dense`] (global dense columns) are excluded
/// from every archetype signature and own no per-archetype pool, so they return
/// `false`. The single shared predicate every signature-exclude / pool-skip site
/// routes through (`if !is_signature_storage(storage_kind(id))`), so adding a
/// future non-signature kind only widens this one function.
///
/// Cold: read at archetype construction only, never on the per-frame hot path.
#[inline]
pub fn is_signature_storage(kind: StorageKind) -> bool {
    matches!(kind, StorageKind::Table)
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
/// `set_storage_kind`.
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
        // Dense plan C1 #0 (the silent-reader-fall-through fix): discriminant 2
        // MUST read back as `Dense`. Without this explicit arm a dense id falls
        // into the `_ => Table` default, silently re-enters the archetype
        // signature, and fragments archetypes with NO compile error.
        x if x == StorageKind::Dense as u8 => StorageKind::Dense,
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

/// Residency class selected for a component id (Phase 4 Seam 1, D1).
///
/// The default for every id is [`Cpu`](ResidencyKind::Cpu) — the standard
/// host-memory `ComponentPool` storage. A `boyko_render` device column type
/// classifies its id as [`Gpu`](ResidencyKind::Gpu) (the archetype becomes
/// GPU-resident); a host-pinned-for-life type classifies as
/// [`CpuPinned`](ResidencyKind::CpuPinned). A signature mixing `Gpu` and
/// `CpuPinned` is a residency conflict and is rejected loudly at archetype mint.
///
/// `#[repr(u8)]` with explicit discriminants so the value round-trips losslessly
/// through the parallel cold `RESIDENCY_CLASS` `AtomicU8` table — exactly as
/// [`StorageKind`] rides `STORAGE_KIND`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResidencyKind {
    /// Host-memory storage (the default). Compatible with any signature.
    Cpu = 0,
    /// Device-memory storage — the archetype is stamped `GPU_RESIDENT` at mint.
    Gpu = 1,
    /// Host-memory storage that must NEVER migrate to a device column. Conflicts
    /// with [`Gpu`](ResidencyKind::Gpu) in the same signature.
    CpuPinned = 2,
}

/// Phase 4 (Seam 1, D1) — parallel cold table of per-component residency
/// classes, one [`AtomicU8`] per `ComponentId`. Mirrors [`STORAGE_KIND`]
/// rather than adding a `ComponentLayout` field, so `ComponentLayout` stays
/// pinned at one cache line.
///
/// Touched ONLY at registration time (write-once via [`set_residency_class`])
/// and at archetype construction (one `Relaxed` load per id to decide the
/// `GPU_RESIDENT` stamp + the conflict scan) — never on the per-frame hot read
/// path.
///
/// `Relaxed` is sufficient (M1): the class is a registration-time, write-once
/// datum with no payload published through it. A `#[derive(Component)]`
/// registration (or the public runtime classify) sets the class atomically with
/// id assignment, before the component can appear in any archetype, so the
/// archetype-construction read observes a settled value. The default `0` reads
/// back as [`ResidencyKind::Cpu`] for every id never explicitly classified.
static RESIDENCY_CLASS: [AtomicU8; MAX_COMPONENTS] =
    [const { AtomicU8::new(ResidencyKind::Cpu as u8) }; MAX_COMPONENTS];

/// Number of 64-bit words in the [`GPU_COMPONENT_SET`] bitset
/// (`MAX_COMPONENTS / 64`).
///
/// `pub(crate)`: the raw residency-bitset word geometry is an internal detail of
/// the W1 query-construction footgun check (`QueryState::new`, same crate).
/// Downstream code that needs residency uses the public [`residency_class`] /
/// [`classify_component_residency`] surface, never the raw process-global word
/// accessor.
pub(crate) const GPU_COMPONENT_SET_WORDS: usize = MAX_COMPONENTS / 64;

/// Phase 5 W1 — process-global bitset of every `ComponentId` whose
/// [`residency_class`] is [`ResidencyKind::Gpu`], one bit per id.
///
/// Maintained write-once in [`set_residency_class`] (the same write-once
/// discipline as [`RESIDENCY_CLASS`]); read at `QueryState::new` to LOUDLY reject
/// a CPU query that NAMES a `Gpu` component in its `include` mask (a device
/// component is absent from the CPU surface, so such a query would silently match
/// nothing). `MAX_COMPONENTS / 64` words; touched only at registration time and
/// at query construction — never on the per-frame hot loop, so the collection
/// loop stays byte-identical (the 0%-gate). A CPU-only world leaves it all-zero.
///
/// `Relaxed` is sufficient (M1, same as `RESIDENCY_CLASS`): a registration-time,
/// write-once datum with no payload published through it.
///
/// FIX-6 — dual-table discipline: [`set_residency_class`] writes this bitset and
/// `RESIDENCY_CLASS` as a PAIR under the one `current == Cpu` first-touch guard,
/// so the pair inherits and depends on the SAME single-threaded,
/// write-once-at-registration-before-archetype-entry discipline already
/// documented for the registry — that is what keeps the two tables coherent.
static GPU_COMPONENT_SET: [AtomicU64; GPU_COMPONENT_SET_WORDS] =
    [const { AtomicU64::new(0) }; GPU_COMPONENT_SET_WORDS];

/// Returns the `word_index`-th 64-bit word of [`GPU_COMPONENT_SET`] (Phase 5 W1).
///
/// `word_index` covers component ids `[word_index * 64, word_index * 64 + 64)`.
/// Cold: read at query construction only (the W1 footgun check), never on the
/// per-frame hot path.
///
/// # Panics (debug only)
///
/// A debug assertion fires if `word_index >= GPU_COMPONENT_SET_WORDS`.
///
/// `pub(crate)`: the sole caller is `QueryState::new` (same crate). Exposing the
/// raw process-global GPU residency bitset word-accessor downstream is a footgun —
/// downstream code that needs residency uses the public [`residency_class`].
#[inline]
pub(crate) fn gpu_component_set_word(word_index: usize) -> u64 {
    // FIX-5: no runtime `>= WORDS` guard — the sole caller (`QueryState::new`)
    // bounds `w` to `0..GPU_COMPONENT_SET_WORDS`, so an out-of-bounds index is
    // unreachable. A silent `return 0` would make the W1 diagnostic MISS rather
    // than fail loudly; instead the `debug_assert!` catches a misuse in debug,
    // and the index access panics honestly if ever called out of bounds.
    debug_assert!(
        word_index < GPU_COMPONENT_SET_WORDS,
        "gpu_component_set_word: word_index {} >= {}",
        word_index,
        GPU_COMPONENT_SET_WORDS
    );
    GPU_COMPONENT_SET[word_index].load(Ordering::Relaxed)
}

/// Returns the residency class registered for `component_id` (Phase 4 Seam 1,
/// D1). Defaults to [`ResidencyKind::Cpu`] for any id never classified via
/// `set_residency_class`.
///
/// Cold: read at archetype construction (the `GPU_RESIDENT` stamp + conflict
/// scan), never on the per-frame hot path. One `Relaxed` load + branch.
///
/// # Panics
///
/// Never. An out-of-range id reads back as [`ResidencyKind::Cpu`] (the safe
/// default) after a debug assertion, mirroring [`storage_kind`]'s discipline.
#[inline]
pub fn residency_class(component_id: usize) -> ResidencyKind {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return ResidencyKind::Cpu;
    }
    match RESIDENCY_CLASS[component_id].load(Ordering::Relaxed) {
        x if x == ResidencyKind::Gpu as u8 => ResidencyKind::Gpu,
        x if x == ResidencyKind::CpuPinned as u8 => ResidencyKind::CpuPinned,
        // 0 (the default) and any future-but-unknown discriminant fall back to
        // the safe host default — `set_residency_class` is the only writer and
        // only ever stores a valid discriminant.
        _ => ResidencyKind::Cpu,
    }
}

/// Classifies `component_id`'s residency class (Phase 4 Seam 1, D1).
/// **Write-once**: the first classification of an id wins for the process
/// lifetime, mirroring [`set_storage_kind`]'s discipline exactly.
///
/// Called from the registration path (the derive's `install_residency_class`
/// and the public runtime [`classify_component_residency`]) atomically with id
/// assignment, before the component can enter any archetype. A `Cpu` id is left
/// at the host default and never needs an explicit call.
///
/// # Panics (debug only)
///
/// A debug assertion fires if an id is RE-classified to a *different* class —
/// reclassification is an invariant violation (the same id cannot be both a CPU
/// and a GPU component). In release the first writer's class is preserved (the
/// store is skipped) so a buggy double-classify degrades to the initial
/// decision rather than silently corrupting the residency partition (M1).
pub(crate) fn set_residency_class(component_id: usize, kind: ResidencyKind) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    let current = RESIDENCY_CLASS[component_id].load(Ordering::Relaxed);
    if current == kind as u8 {
        // Idempotent same-class (re)classification — no-op.
        return;
    }
    debug_assert!(
        current == ResidencyKind::Cpu as u8,
        "set_residency_class: ComponentId {} already classified as {:?}, refused \
         to reclassify as {:?} (residency class is write-once)",
        component_id,
        residency_class(component_id),
        kind
    );
    if current == ResidencyKind::Cpu as u8 {
        RESIDENCY_CLASS[component_id].store(kind as u8, Ordering::Relaxed);
        // Phase 5 W1: mirror a `Gpu` classification into the GPU_COMPONENT_SET
        // bitset (write-once, same discipline). Only `Gpu` raises a bit — `Cpu`
        // is the default (never reached here, the early-out above) and
        // `CpuPinned` is non-Gpu, so neither sets a bit.
        if kind == ResidencyKind::Gpu {
            let word = component_id / 64;
            let bit = component_id % 64;
            GPU_COMPONENT_SET[word].fetch_or(1u64 << bit, Ordering::Relaxed);
        }
    }
}

/// Installs the residency class for a derived component into the cold
/// `RESIDENCY_CLASS` table from the type's compile-time [`Component::RESIDENCY`]
/// const (Phase 4 Seam 1, D1 — the derive emit, mirroring
/// [`install_storage_kind`]).
///
/// This is the derive-emitted counterpart of the public runtime
/// [`classify_component_residency`]: `#[derive(Component)]` expands into
/// downstream crates where the `pub(crate)` `set_residency_class` is
/// unreachable, so the derive's `component_id()` install path calls this `pub`
/// wrapper instead — exactly mirroring how it calls [`install_clone_fn`].
///
/// The call is emitted UNGATED (one cold read of the const per type per
/// process, behind the `component_id()` `OnceLock`); a `Cpu` const (the default
/// for every existing component) short-circuits to a no-op, so a plain
/// `#[derive(Component)]` pays zero — the 0%-gate.
///
/// # Panics
///
/// In debug, if `component_id` is reclassified to a different class (see
/// `set_residency_class`). Unreachable for derived types (a single
/// `RESIDENCY` const per type).
#[inline]
pub fn install_residency_class<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    // The default `Cpu` const short-circuits, so a plain `#[derive(Component)]`
    // never touches the table (the 0%-gate); only a GPU/CpuPinned type writes.
    if C::RESIDENCY != ResidencyKind::Cpu {
        set_residency_class(component_id, C::RESIDENCY);
    }
}

/// Classifies `component_id`'s residency class at runtime (Phase 4 Seam 1, D1 —
/// Q4 public path).
///
/// The public counterpart to the derive-only [`install_residency_class`]: it
/// lets `boyko_render` (or any non-derive caller) classify a foreign component
/// id whose type it does not own the `Component` impl for. Same write-once /
/// reclassify-panic discipline as `set_residency_class`.
///
/// Must be called at registration time, before the id can enter any archetype
/// (the same ordering the `RESIDENCY_CLASS` table relies on).
///
/// # Panics (debug only)
///
/// If `component_id` is reclassified to a different class (see
/// `set_residency_class`).
#[inline]
pub fn classify_component_residency(component_id: usize, kind: ResidencyKind) {
    set_residency_class(component_id, kind);
}

/// Installs the storage backend for a derived component into the cold
/// `STORAGE_KIND` table from the type's compile-time [`Component::STORAGE_IS_BITSET`]
/// const (EnableTag plan, D5 — the `#[component(storage = "bitset")]` derive arm,
/// Wave 5 Step 10).
///
/// This is the **public** counterpart of the `pub(crate)` `set_storage_kind`:
/// `#[derive(Component)]` expands into downstream crates, where the `pub(crate)`
/// writer is unreachable, so the derive's `component_id()` install path calls
/// this `pub` wrapper instead — exactly mirroring how it calls [`install_hooks`]
/// for `C::HAS_HOOKS`. The derive emits this call ONLY when
/// `C::STORAGE_IS_BITSET` is `true` (const-gated), so a plain
/// `#[derive(Component)]` const-folds it away and the id stays at the
/// [`StorageKind::Table`] default — zero cost for non-tag components.
///
/// Write-once and idempotent through `set_storage_kind`: it runs once per
/// type per process (behind the `component_id()` `OnceLock`), atomically with id
/// assignment and therefore before the id can enter any archetype.
///
/// # Panics
///
/// In debug, if `component_id` is reclassified to a different kind (see
/// `set_storage_kind`). The XOR-by-construction discipline (a single
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

/// Installs [`StorageKind::Dense`] for a derived dense component into the cold
/// `STORAGE_KIND` table from the type's compile-time
/// [`Component::STORAGE_IS_DENSE`] const (Dense plan D0 — the
/// `#[component(storage = "dense")]` derive arm).
///
/// The dense twin of [`install_storage_kind`]: `#[derive(Component)]` expands
/// into downstream crates where the `pub(crate)` `set_storage_kind` is
/// unreachable, so the derive's `component_id()` install path calls this `pub`
/// wrapper. The derive emits the call ONLY when `C::STORAGE_IS_DENSE` is `true`
/// (const-gated), so a plain `#[derive(Component)]` const-folds it away and the
/// id stays at the [`StorageKind::Table`] default — zero cost for non-dense
/// components.
///
/// Write-once and idempotent through `set_storage_kind`: it runs once per type
/// per process (behind the `component_id()` `OnceLock`), atomically with id
/// assignment and therefore before the id can enter any archetype.
///
/// # Panics
///
/// In debug, if `component_id` is reclassified to a different kind (see
/// `set_storage_kind`), or — Dense plan W1 — if the type's residency is not
/// [`ResidencyKind::Cpu`]: dense storage is ALWAYS host-resident, so a
/// `storage = "dense"` component classified `Gpu`/`CpuPinned` is an invariant
/// violation (the derive rejects the attribute combination at compile time; this
/// guards a hand-`impl Component`). The XOR-by-construction discipline (a single
/// `STORAGE_IS_DENSE` const per type) makes reclassification unreachable for
/// derived types.
#[inline]
pub fn install_dense_storage_kind<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    // The derive const-gates this call on `C::STORAGE_IS_DENSE`, so in practice
    // the branch is always taken; the explicit test keeps the wrapper correct if
    // a hand-`impl Component` calls it with the table default.
    if C::STORAGE_IS_DENSE {
        // Dense plan W1: dense storage is ALWAYS `ResidencyKind::Cpu`. The derive
        // forces this (it never sets a non-`Cpu` `RESIDENCY` for a dense type and
        // rejects a `gpu` residency attribute at compile time), so for a derived
        // type the const is always `Cpu`. The assert catches a hand-`impl
        // Component` that wrongly pairs `STORAGE_IS_DENSE = true` with a `Gpu` /
        // `CpuPinned` `RESIDENCY`.
        debug_assert!(
            C::RESIDENCY == ResidencyKind::Cpu,
            "install_dense_storage_kind: dense storage is always Cpu-resident, but \
             ComponentId {} declares RESIDENCY = {:?} (Dense plan W1)",
            component_id,
            C::RESIDENCY
        );
        set_storage_kind(component_id, StorageKind::Dense);
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

// ═════════════════════════════════════════════════════════════════════════════
// Required components (Feature 1 — `#[require(B, C)]`). Two parallel cold tables
// mirroring the HOOKS / STORAGE_KIND blocks above (D1): the 56 B `ComponentLayout`
// hot record stays pinned (TRIPWIRE 2). Touched ONLY at registration time
// (write-once `REQUIRES_DIRECT`) and at the bundle-resolution / first-expansion
// cold path (`REQUIRES_ALL` memoized DFS) — never on the per-frame hot read path.
// ═════════════════════════════════════════════════════════════════════════════

/// Capture-free constructor for a required component (D2). Mirrors [`DropFn`]:
/// a bare `unsafe fn(*mut u8)` that writes one fully-initialized value of the
/// required component's type into `dst`. F2-immune by construction — it never
/// sees the world.
///
/// The derive lowers `#[require(B)]` to
/// `unsafe fn __require_ctor_B(dst) { dst.cast::<B>().write(B::default()) }`
/// and `#[require(C = expr)]` to `...write({ expr })` (a capture-free
/// expression only — no `Arc<dyn>` / no closure environment).
///
/// # Safety
/// The caller must guarantee:
/// - `dst` points at properly-aligned, writable, **uninitialized** memory of at
///   least `size_of::<T>()` bytes for the required component type `T` whose
///   layout matches `RequiredEntry::component_id`.
/// - `dst` is exclusively owned for the duration of the call; the value written
///   is subsequently owned by the archetype (its `drop_fn` runs on teardown).
pub type RequiredCtor = unsafe fn(dst: *mut u8);

/// A capture-free resolver that returns the required component's
/// [`ComponentId`]. Stored UNCALLED in [`RequiredDirectEntry`] so that
/// registering a `#[require]` edge does NOT eagerly invoke the required type's
/// `component_id()` (BUG-REQ-CYCLE-1): a cycle would otherwise re-enter the
/// requiring type's own `component_id()` `OnceLock::get_or_init` on the same
/// thread and deadlock. The derive emits `B::component_id` (a fn item, no
/// parentheses); the id is resolved LAZILY in `build_required_plan`, which
/// runs at archetype-expansion time — OUTSIDE any `component_id` init.
pub type RequiredIdFn = fn() -> ComponentId;

/// One DIRECT `#[require]` edge as stored in `REQUIRES_DIRECT` (D2). 16 B POD:
/// an 8 B [`RequiredIdFn`] resolver + an 8 B [`RequiredCtor`]. Distinct from
/// [`RequiredEntry`] (which carries a RESOLVED [`ComponentId`]): the direct
/// table holds the id as an UNCALLED resolver to break the registration-time
/// re-entrancy (BUG-REQ-CYCLE-1). `#[repr(C)]` pins the field order; `Copy` so
/// the slice plumbing never invokes drop glue.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct RequiredDirectEntry {
    /// The required component's id resolver, called lazily at plan-build time.
    pub id_fn: RequiredIdFn,
    /// The capture-free constructor that materializes one instance.
    pub ctor: RequiredCtor,
}

/// One transitively-resolved required component (D2). 16 B POD: an 8 B
/// [`ComponentId`] + an 8 B function pointer. `#[repr(C)]` pins the field
/// order; the type is `Copy` so the closure/slice plumbing never invokes drop
/// glue. Produced by `build_required_plan` after resolving each
/// [`RequiredDirectEntry::id_fn`].
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RequiredEntry {
    /// The required component's id.
    pub component_id: ComponentId,
    /// The capture-free constructor that materializes one instance.
    pub ctor: RequiredCtor,
}

// W3: pair the 16 B assert with a `ComponentId` companion so the 16 is
// self-documenting (it is `size_of::<ComponentId>() + size_of::<fn ptr>()`),
// guarding a future `ComponentId` widening. Gated to 64-bit (the engine's
// supported platform) — see CLAUDE.md target platform. Both the resolved entry
// (`ComponentId` + ctor) and the raw direct entry (`RequiredIdFn` + ctor) are
// two-pointer-wide.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ComponentId>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<RequiredEntry>() == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<RequiredDirectEntry>() == 16);

/// The full transitive closure of a component's required components (D3),
/// computed once and memoized in `REQUIRES_ALL`. The `entries` slice is
/// DFS-ordered (deps-before-dependent) and deduped by `component_id` (the W1
/// conflict rule resolves which ctor each id carries).
pub struct RequiredPlan {
    /// Transitive, DFS-ordered, deduped required entries. Empty for a
    /// component that declares no `#[require]` and is required by nothing it
    /// transitively pulls.
    pub entries: &'static [RequiredEntry],
}

/// Distinct error type for the required-components subsystem (W2). Surfaced via
/// a fail-loud panic at registration / first-expansion (present in release, not
/// a vanishing `debug_assert`).
#[derive(Debug)]
pub enum RequiredError {
    /// A `#[require]` cycle was detected (`A → … → A`). Memoization alone does
    /// NOT break it (the `REQUIRES_ALL` slot is `None` during recursion), so
    /// the "currently-building" stack catches the re-entry.
    Cycle {
        /// The id re-entered while already on the building stack.
        component_id: ComponentId,
    },
}

/// Required-components plan D1: write-once table of each component's DIRECT
/// `#[require]` declarations (the immediate edges only — the transitive closure
/// lives in [`REQUIRES_ALL`]). Mirrors the [`HOOKS`] declaration; populated by
/// [`install_required`] from the derive-generated registration path, gated on
/// `Component::HAS_REQUIRES`.
static REQUIRES_DIRECT: [OnceLock<&'static [RequiredDirectEntry]>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Required-components plan D3: memoized transitive closure per component. Built
/// lazily by [`build_required_plan`] on first expansion (the bundle-resolution
/// funnel), DFS over [`REQUIRES_DIRECT`]. A leaked `&'static RequiredPlan` per
/// component (bounded by `MAX_COMPONENTS`, the #53 bounded-leak precedent).
static REQUIRES_ALL: [OnceLock<&'static RequiredPlan>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

thread_local! {
    /// W2 cycle break: the "currently-building" id stack, DISTINCT from the
    /// memoized [`REQUIRES_ALL`] result. Re-entering an id already on this stack
    /// is a cycle (`build_required_plan` panics with [`RequiredError::Cycle`]).
    /// Thread-local because `build_required_plan` recurses on a single thread;
    /// the memoized result is published process-globally via `OnceLock::set`.
    static BUILDING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard for the [`BUILDING`] cycle-detection stack (MINOR robustness fix —
/// mirrors the codebase's `DeferredScopeGuard` / `CursorSync` RAII pattern).
///
/// The cycle-detection stack MUST be unwound even when [`build_required_plan`]
/// panics: `CommandQueue::apply` wraps the apply path in a `catch_unwind`, so a
/// caught [`RequiredError::Cycle`] (or any panic mid-build, e.g. a leaked-alloc
/// OOM) would otherwise leave stale ids on the stack — a later unrelated ACYCLIC
/// bundle's build could then false-positive a cycle on a re-pushed id. Popping in
/// `Drop` guarantees the stack is balanced on both the normal and the unwinding
/// path.
struct BuildingGuard {
    /// The id this guard pushed; verified by the `Drop` balance check.
    id: usize,
}

impl BuildingGuard {
    /// Pushes `component_id` onto the [`BUILDING`] stack, panicking with
    /// [`RequiredError::Cycle`] (via [`required_cycle_panic`]) if it is already
    /// present (a `#[require]` cycle). On the cycle path NO guard is created (the
    /// panic happens before the push), so the existing stack frames' guards still
    /// unwind their own ids correctly.
    #[inline]
    fn push(component_id: usize) -> Self {
        BUILDING.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&component_id) {
                required_cycle_panic(component_id);
            }
            stack.push(component_id);
        });
        Self { id: component_id }
    }
}

impl Drop for BuildingGuard {
    #[inline]
    fn drop(&mut self) {
        BUILDING.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert_eq!(
                popped,
                Some(self.id),
                "BUILDING stack imbalance in build_required_plan"
            );
        });
    }
}

/// Installs `C`'s DIRECT `#[require]` declarations into
/// `REQUIRES_DIRECT[component_id]` (D1). Builds the entry slice via
/// [`Component::register_required`] and leaks it once (`&'static`), mirroring
/// [`install_hooks`]'s write-once discipline.
///
/// Called from the derive-generated `component_id()` ONLY when
/// `C::HAS_REQUIRES` is true (const-gated, like [`install_hooks`]): a plain
/// `#[derive(Component)]` leaves the slot UNSET, which reads as "no direct
/// requires" everywhere downstream — the 0%-gate. The leak is bounded by
/// `MAX_COMPONENTS` (one slice per requiring component per process).
#[inline]
pub fn install_required<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let mut builder = RequiredBuilder::new();
    C::register_required(&mut builder);
    let leaked: &'static [RequiredDirectEntry] = Box::leak(builder.into_entries());
    // Write-once; a same-id re-install is a silent no-op (first writer wins,
    // matching `install_hooks` / `register_new`).
    let _ = REQUIRES_DIRECT[component_id].set(leaked);
}

/// Returns the DIRECT `#[require]` declarations for `component_id`, or an empty
/// slice when the component declared none. Cold (registration / first-expansion
/// only).
#[inline]
fn get_required_direct(component_id: usize) -> &'static [RequiredDirectEntry] {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return &[];
    }
    REQUIRES_DIRECT[component_id].get().copied().unwrap_or(&[])
}

/// Returns the memoized transitive required-components plan for `component_id`
/// (D3), building it on first access. The returned `entries` slice is
/// DFS-ordered (deps-before-dependent) and deduped per the W1 conflict rule.
///
/// Cold: called at the bundle-resolution funnel (archetype expansion) and by
/// Feature 2 (cloning) to reconstruct a missing required component — exposed
/// `pub(crate)` for that reuse.
///
/// # Panics
///
/// [`RequiredError::Cycle`] (fail-loud, release-active) if a `#[require]` cycle
/// is reachable from `component_id` (W2).
#[inline]
pub(crate) fn get_required_plan(component_id: usize) -> &'static RequiredPlan {
    build_required_plan(component_id)
}

/// Builds (or returns the memoized) transitive required-components plan for
/// `component_id` (D3). Memoized DFS over [`REQUIRES_DIRECT`] with the W1
/// conflict rule and the W2 cycle break.
///
/// Algorithm:
/// 1. Fast path: return the memoized `REQUIRES_ALL[id]` if present.
/// 2. Push `id` onto the thread-local `BUILDING` stack (W2). If it is already
///    present → cycle → panic with [`RequiredError::Cycle`].
/// 3. For each DIRECT required entry `e` of `id`, in declaration order: first
///    resolve `e.id_fn()` to the required `ComponentId` (BUG-REQ-CYCLE-1: this
///    is the ONLY place the required type's `component_id()` is invoked, well
///    outside any mid-init `OnceLock`); then recurse to build that id's closure
///    and merge it (deps-before-dependent), keep-first on duplicate ids (W1
///    inherited rule: first-DFS-reached ctor wins); then merge `e` itself — if
///    the resolved id is already present (pulled transitively by an earlier
///    sibling), OVERRIDE its ctor with `e`'s (a DIRECT declaration wins over an
///    inherited one, W1 direct rule); otherwise push it.
/// 4. Pop the `BUILDING` stack, leak the deduped DFS-ordered slice, and
///    memoize via `OnceLock::set` (first writer wins on a race).
///
/// Exposed `pub(crate)` so Feature 2 (cloning) can reuse the closure +
/// missing-required diff keyed on the clone target id set.
pub(crate) fn build_required_plan(component_id: usize) -> &'static RequiredPlan {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    // 1. Memoized fast path.
    if let Some(plan) = REQUIRES_ALL[component_id].get() {
        return plan;
    }

    // 2. W2 cycle break: push onto the building stack, panic on re-entry. The
    // RAII `_guard` pops `component_id` on EVERY exit path — including a panic
    // unwinding through `CommandQueue::apply`'s `catch_unwind` — so a caught
    // cycle-panic cannot leave a stale id that false-positives a later acyclic
    // build (MINOR robustness fix).
    let _guard = BuildingGuard::push(component_id);

    // 3. DFS merge. `out` accumulates the deps-before-dependent, deduped set.
    let mut out: Vec<RequiredEntry> = Vec::new();
    for &direct in get_required_direct(component_id) {
        // BUG-REQ-CYCLE-1: resolve the required id LAZILY here, NOT at
        // registration time. `id_fn()` is `B::component_id` invoked outside any
        // `component_id` `get_or_init`, so the required type fully initializes
        // without re-entering the requiring type's mid-init `OnceLock`. A
        // genuine cycle instead re-enters `build_required_plan` on the BUILDING
        // stack below → the `BuildingGuard` panic fires (release-active).
        let dep_id_cid = (direct.id_fn)();
        let entry = RequiredEntry {
            component_id: dep_id_cid,
            ctor: direct.ctor,
        };
        let dep_id = dep_id_cid.0;
        // 3a. Merge the dependency's full closure FIRST (deps-before-dependent),
        // keep-first on duplicate ids (W1 inherited rule).
        let dep_plan = build_required_plan(dep_id);
        for &dep_entry in dep_plan.entries {
            if !out.iter().any(|e| e.component_id == dep_entry.component_id) {
                out.push(dep_entry);
            }
        }
        // 3b. Merge the direct entry itself. A DIRECT declaration OVERRIDES any
        // inherited ctor for the same id (W1 direct rule); otherwise push it.
        if let Some(existing) = out.iter_mut().find(|e| e.component_id == entry.component_id) {
            existing.ctor = entry.ctor;
        } else {
            out.push(entry);
        }
    }

    // 4. The DFS for this id is complete. `_guard` pops `component_id` from the
    // BUILDING stack when it drops at the end of this function (or on unwind).

    let leaked_entries: &'static [RequiredEntry] = Box::leak(out.into_boxed_slice());
    let leaked_plan: &'static RequiredPlan = Box::leak(Box::new(RequiredPlan {
        entries: leaked_entries,
    }));
    // Write-once memoization. A concurrent racer's plan is identical (the DFS is
    // deterministic for `id`), so the loser drops its leaked plan reference and
    // reads back the winner's.
    match REQUIRES_ALL[component_id].set(leaked_plan) {
        Ok(()) => leaked_plan,
        Err(_) => REQUIRES_ALL[component_id]
            .get()
            .expect("invariant: OnceLock::set Err implies the slot is occupied"),
    }
}

/// Required components (D4): true iff ANY id in `base_ids` declares (directly or
/// transitively) at least one required component. Cold (bundle-resolution funnel
/// only). For a require-free id set this is `MAX_COMPONENTS`-bounded but in
/// practice one [`get_required_plan`] memoized read per base id, all empty — so
/// the union loop at the call site runs zero inner iterations (the 0%-gate (3)).
#[inline]
pub(crate) fn any_requires(base_ids: &[ComponentId]) -> bool {
    base_ids
        .iter()
        .any(|cid| !get_required_plan(cid.0).entries.is_empty())
}

/// Required components (D4): invokes `push` once for every transitively-required
/// component id reachable from `base_ids` that is NOT already present in
/// `base_ids` and not already pushed (deduped against an internal `seen` set
/// seeded from `base_ids`). Used by the bundle-resolution funnel
/// (`cold_register_bundle_archetype` / `merged_archetype_id`) to compute the
/// EFFECTIVE archetype id set.
///
/// Cold path only — runs once per `(bundle, world)` (spawn) or per insert
/// migration resolve; the result is cached on the Phase-8.5 slot /
/// `BundleColumnRecord`.
///
/// Present⇒skip: a required id already in `base_ids` is never pushed (the
/// explicit value wins, no overwrite — D resolved-questions "present ⇒ skip").
#[inline]
pub(crate) fn for_each_required_id_excluding<F: FnMut(ComponentId)>(
    base_ids: &[ComponentId],
    mut push: F,
) {
    // `seen` carries the base ids plus everything pushed so far, so a diamond
    // (two base ids both requiring D) emits D once (D3 / W1 dedup).
    let mut seen: Vec<ComponentId> = base_ids.to_vec();
    for &cid in base_ids {
        for entry in get_required_plan(cid.0).entries {
            if !seen.contains(&entry.component_id) {
                seen.push(entry.component_id);
                push(entry.component_id);
            }
        }
    }
}

/// Required components (Feature 1): resolves the W1-conflict-resolved
/// [`RequiredCtor`] for `target_id` within the transitive closure of `base_ids`,
/// or `None` if `target_id` is not reachable as a required component from any
/// base id. Used by the insert-path constructor pass to look up the ctor for an
/// id it decided to construct.
///
/// Returns the FIRST matching entry (the W1 first-DFS / direct-override result
/// is already baked into each base id's memoized plan, so the first base whose
/// closure contains `target_id` carries the precedence-correct ctor — the
/// `for_each_required_id_excluding` iteration order is identical).
#[inline]
pub(crate) fn required_ctor_for(
    base_ids: &[ComponentId],
    target_id: ComponentId,
) -> Option<RequiredCtor> {
    for &cid in base_ids {
        for entry in get_required_plan(cid.0).entries {
            if entry.component_id == target_id {
                return Some(entry.ctor);
            }
        }
    }
    None
}

/// Cold fail-loud panic site for the W2 cycle break. Kept out of line so
/// [`build_required_plan`]'s body stays compact.
#[cold]
#[inline(never)]
fn required_cycle_panic(component_id: usize) -> ! {
    let name = get_layout(component_id)
        .map(|l| l.type_name)
        .unwrap_or("<unregistered>");
    panic!(
        "{:?}: a #[require] cycle is reachable from ComponentId {} ({}). \
         Required-component edges must form a DAG.",
        RequiredError::Cycle {
            component_id: ComponentId(component_id),
        },
        component_id,
        name,
    )
}

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
/// `dst`. A bare `unsafe fn(*const u8, *mut u8)` (mirror of [`DropFn`]) — no
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
/// path (the 0%-gate). One acquire-load + branch, mirroring [`get_hooks`].
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
/// [`install_hooks`] / [`install_required`].
///
/// **PUBLIC** (the derive expands into downstream crates where `pub(crate)` is
/// unreachable — the same rationale as [`install_storage_kind`]). Called from the
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

// ═════════════════════════════════════════════════════════════════════════════
// Serialization (Phase S0 — registry substrate). Spec: `docs/SERIALIZATION-PLAN.md`
// §3.7 (data structures) + §5 C1–C3 + §7 Phase S0. One cold parallel table (mirror
// of `CLONE`) + a NEW stable-name → id index (C1). Touched ONLY at registration
// time (write-once `install_serialize_fn` / `register_stable_name`, one cold
// `OnceLock::set` + one `Mutex` insert per type) and from the future
// `boyko_serialize` crate (save/load — never on the per-frame spawn/iter/schedule
// path). 0%-gate (grep-proof obligation): the `get_serialize_info` /
// `resolve_stable_name` / `STABLE_NAME_INDEX` readers ⊆ `boyko_serialize`. The 56 B
// `ComponentLayout` hot record stays pinned (TRIPWIRE 2): the metadata lives in the
// parallel cold `SERIALIZE` table and the separate `STABLE_NAME_INDEX` static.
// ═════════════════════════════════════════════════════════════════════════════

/// Serialize one component instance: read the live value at `src`, append
/// position-independent bytes into `sink` (plan §3.7). A bare
/// `unsafe fn(*const u8, &mut SaveCursor)` (mirror of [`CloneFn`]) — no
/// `Box<dyn>`, no `Arc<dyn Fn>`. Installed ONLY for the
/// [`Serializability::SerializeViaFn`] encode path; a
/// [`Serializability::PlainOldBytes`] component installs `None` and is blitted
/// whole-column from the pool layout, never through this pointer (the POB fast
/// path).
///
/// # Safety (caller-guaranteed at the single save call site, S1)
/// - `src` points at a live, initialized value of THIS `ComponentId`'s type `C`,
///   aligned to `align_of::<C>()`, readable for `size_of::<C>()` bytes.
/// - `sink` is a valid, append-only cursor; the fn only appends and never reads
///   back prior bytes.
pub type SerializeFn =
    unsafe fn(src: *const u8, sink: &mut crate::ecs::core::serialize::SaveCursor<'_>);

/// Deserialize one component instance from `src` into the UNINITIALIZED `dst`
/// (one `ptr::write`, no drop of prior contents — plan §3.7). Returns `Err` on a
/// malformed stream (the caller rolls back; `dst` is left uninit — the W5
/// partial-row contract, mirroring [`CloneFn`]). Entity fields are written with
/// their SAVED ids; the separate [`LoadMapEntitiesFn`] pass remaps them.
///
/// # Safety (caller-guaranteed at the single load call site, S2)
/// - `dst` points at writable, **uninitialized** space of `>= size_of::<C>()`
///   bytes, aligned to `align_of::<C>()`.
/// - On a normal `Ok` return `dst` holds an initialized `C` written exactly once;
///   on `Err` (or a panic) `dst` is left uninitialized and the caller's rollback
///   guard must NOT drop it.
pub type DeserializeFn = unsafe fn(
    src: &mut crate::ecs::core::serialize::LoadCursor<'_>,
    dst: *mut u8,
) -> Result<(), crate::ecs::core::serialize::DecodeError>;

/// Load-direction entity remap (saved id → freshly-allocated `Entity`), plan
/// §3.7 / C4. Installed ONLY for entity-bearing components (v1: `ChildOf` and
/// explicit `#[entities]` fields); every other id leaves its slot unset, so the
/// remap pass touches only the annotated components.
///
/// Rewrites every remappable `Entity` field of the value at `dst` in place,
/// translating each SAVED id to the freshly-allocated `Entity` via `map`. Returns
/// [`DecodeError::UnmappedEntity`](crate::ecs::core::serialize::DecodeError::UnmappedEntity)
/// when a referenced saved id is absent from `map` (a dangling reference — the C4
/// loud-error path; never silently kept as a stale id).
///
/// # Safety (caller-guaranteed at the load remap call site, S2.5)
/// - `dst` points at a live, initialized value of THIS `ComponentId`'s type.
/// - `map` outlives the call and is not aliased mutably.
pub type LoadMapEntitiesFn = unsafe fn(
    dst: *mut u8,
    map: &crate::ecs::core::serialize::LoadEntityMap,
) -> Result<(), crate::ecs::core::serialize::DecodeError>;

/// Per-component serialization classification (plan §3.7 / C3). STRICTER than the
/// clone [`Cloneability`]: serialization ingests **untrusted bytes**, so the
/// blittable [`PlainOldBytes`](Serializability::PlainOldBytes) class additionally
/// requires every field to have an all-bits-valid representation. Drives the
/// blit-vs-fn-ptr-vs-skip branch on its own.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Serializability {
    /// `#[repr(C)]`/`#[repr(transparent)]` AND every field transitively in
    /// `{integers, floats, raw pointers}` — NO `bool`, `char`, enum, niche type,
    /// or `Entity`. `serialize_fn` / `deserialize_fn` are `None`; the whole column
    /// is blitted with one `copy_nonoverlapping` (the POB fast path). Strictly
    /// narrower than `Cloneability::TriviallyCopyable` (which only needs `Copy`):
    /// a `Copy` type with a `bool`/`char`/enum/niche field is NOT POB because
    /// those bytes are not all-bits-valid on an untrusted load (C3).
    PlainOldBytes = 0,
    /// Owning (`String`/`Vec`/heap) OR bit-restricted (`bool`/`char`/enum/niche)
    /// OR entity-bearing — must run the per-element `serialize_fn` /
    /// `deserialize_fn` (length-prefixed, position-independent, validates on
    /// read). The decode path validates every bit-restricted field, making the
    /// copy path sound on untrusted bytes (C3).
    SerializeViaFn = 1,
    /// Not serializable (not `Clone`, or `#[component(no_serialize)]`) —
    /// `serialize_fn` / `deserialize_fn` are `None`. The saver skips it; the
    /// loader default-constructs (via the `REQUIRES` ctor) or rejects in strict
    /// mode. The backward-compatible default for every non-opted-in component.
    Ignore = 2,
}

/// Cold per-component serialization metadata (plan §3.7). Lives in the parallel
/// `SERIALIZE` table, NOT in `ComponentLayout` (keeps TRIPWIRE 2's 56 B).
/// `Copy + Send + Sync` (fn-ptrs + POD + `&'static str`), like [`CloneInfo`].
///
/// Per plan O1, the exact size is NOT load-bearing (this is a cold record), so —
/// unlike [`CloneInfo`]'s asserted 16 B — there is no `const_assert` on this type.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SerializeInfo {
    /// `Some(serialize_via_serde::<C>)` ONLY for [`Serializability::SerializeViaFn`];
    /// `None` for [`Serializability::PlainOldBytes`] (blit path) and
    /// [`Serializability::Ignore`]. (S0 installs `None` everywhere — the encode
    /// glue lands in S1; the classification + table shape are pinned now.)
    pub serialize_fn: Option<SerializeFn>,
    /// `Some(deserialize_via_serde::<C>)` ONLY for
    /// [`Serializability::SerializeViaFn`]; `None` otherwise. (S0: `None` — see
    /// `serialize_fn`.)
    pub deserialize_fn: Option<DeserializeFn>,
    /// `Some(remap)` ONLY for entity-bearing components (v1: `ChildOf` + explicit
    /// `#[entities]`); `None` otherwise (C4). (S0: `None` — the hand-written
    /// `ChildOf` remap install lands with the loader.)
    pub map_entities_fn: Option<LoadMapEntitiesFn>,
    /// The branch discriminator. Drives blit-vs-fn-ptr-vs-skip on its own.
    pub serializability: Serializability,
    /// User-bumpable on any intentional layout/semantic change — the human-facing
    /// version gate (plan §3.5 / C2). Default `0`.
    pub format_version: u16,
    /// Derive-computed blit-validity guard (plan §3.6 / C2): a best-effort hash of
    /// `(size, align, repr, per-field offsets, field_count)`. Guards "the struct
    /// changed shape since the save"; `format_version` is the human-facing gate.
    pub layout_fingerprint: u64,
    /// The stable serialization key (plan §3.5 / C1): default = the fully-qualified
    /// type name, overridable via `#[component(stable_name = "...")]`. The on-disk
    /// type key — `ComponentId` is process-unstable, the name is the stable option.
    pub stable_name: &'static str,
    /// The 64-bit hash of [`Self::stable_name`], the `STABLE_NAME_INDEX` key.
    pub stable_name_hash: u64,
}

/// Phase S0 — parallel cold table of per-component serialization metadata (plan
/// §3.7). Touched ONLY at registration time (write-once `install_serialize_fn`)
/// and from the future `boyko_serialize` save/load path — never on
/// spawn/iter/schedule. Mirrors the [`CLONE`] declaration exactly.
static SERIALIZE: [OnceLock<SerializeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Returns the registered serialization metadata for `component_id`, or `None`
/// when no metadata was installed (a hand-written impl that never opted in).
///
/// Cold: read ONLY from the future `boyko_serialize` crate — never on the
/// per-frame hot path (the 0%-gate). One acquire-load + branch, mirroring
/// [`get_clone_info`].
#[inline]
pub fn get_serialize_info(component_id: usize) -> Option<&'static SerializeInfo> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    SERIALIZE[component_id].get()
}

/// Installs `C`'s serialization metadata into `SERIALIZE[component_id]` (Phase
/// S0). Builds a [`SerializeInfo`] from the type's compile-time serialization
/// consts ([`Component::SERIALIZABILITY`], [`Component::FORMAT_VERSION`],
/// [`Component::LAYOUT_FINGERPRINT`]) + methods ([`Component::stable_name`] /
/// [`Component::serializability_runtime`] / [`Component::serialize_fn`] /
/// [`Component::deserialize_fn`] / [`Component::map_entities_fn`]) and writes it
/// once via `OnceLock::set`, mirroring [`install_clone_fn`].
///
/// **PUBLIC** (the derive expands into downstream crates where `pub(crate)` is
/// unreachable — the same rationale as [`install_clone_fn`]). Called from the
/// derive's `component_id()` closure **UNGATED** (like `install_clone_fn`): the
/// 0%-gate is preserved because the write is one cold `OnceLock::set` per type per
/// process, behind the existing `component_id()` `OnceLock`, and never on a
/// per-frame path. Ungating it means the serialize path never has to special-case
/// a missing entry — a plain non-serializable component installs
/// `Serializability::Ignore` with `None` fns (the trait defaults).
#[inline]
pub fn install_serialize_fn<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let stable_name = C::stable_name();
    // Read the METHOD (not the const): the derive overrides
    // `serializability_runtime()` with the autoref-probe result (a const cannot run
    // autoref). Hand-written impls default the method to their `SERIALIZABILITY`
    // const.
    let serializability = C::serializability_runtime();
    // Gate the encode/decode fn-ptrs on the RUNTIME classification — the single
    // source of truth for "POB installs None" (plan §3.7). The derive's
    // `serialize_fn()` / `deserialize_fn()` return `Some(glue)` whenever every field
    // is `Wire` (it does NOT see the runtime POB/ViaFn split), so a genuinely
    // `PlainOldBytes` component (all SerPod primitives, which are also all `Wire`)
    // would otherwise install a live encoder. Only `SerializeViaFn` keeps the
    // `Some`; `PlainOldBytes` (blit path) and `Ignore` drop to `None`.
    let is_via_fn = serializability == Serializability::SerializeViaFn;
    let (serialize_fn, deserialize_fn) = if is_via_fn {
        (C::serialize_fn(), C::deserialize_fn())
    } else {
        (None, None)
    };
    let info = SerializeInfo {
        serialize_fn,
        deserialize_fn,
        map_entities_fn: C::map_entities_fn(),
        serializability,
        format_version: C::FORMAT_VERSION,
        layout_fingerprint: C::LAYOUT_FINGERPRINT,
        stable_name,
        stable_name_hash: fnv1a_64(stable_name.as_bytes()),
    };
    // Write-once; a same-id re-install is a silent no-op (first writer wins).
    let _ = SERIALIZE[component_id].set(info);
}

/// 64-bit FNV-1a hash of a byte string (the `STABLE_NAME_INDEX` keying, C1).
///
/// A `const fn` so the derive could fold it at compile time and so it is reusable
/// by the future `boyko_serialize` file-key path. FNV-1a is chosen over a
/// heavier hash because the index is COLD (registration + once-per-load-type
/// only) and collisions are explicitly disambiguated by a full-name compare in
/// [`resolve_stable_name`] — the hash only buckets candidates.
#[inline]
pub const fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// C1 — process-global stable-name → `ComponentId` index. Does NOT exist before
/// S0 (`TAG_NAMES` interns only dynamic-tag names; derived components are keyed by
/// `TypeId` in `ComponentLayout`, with no name→id reverse index). Built fresh
/// here.
///
/// Keyed by the stable-name HASH (never `TypeId` — two builds of "the same"
/// component differ in `TypeId` but must resolve to the same stable name).
/// Collisions are disambiguated by comparing the full `stable_name` string on a
/// hash hit (see [`resolve_stable_name`]); the bucket value is the list of
/// candidate `ComponentId`s that hashed equal.
///
/// COLD: touched only at registration ([`register_stable_name`]) and once per
/// file-local type at load ([`resolve_stable_name`]) — never on the per-frame hot
/// path. `Mutex + HashMap` is justified per the `TAG_NAMES` precedent (one
/// concrete global, not a generic-fn-body static — sidesteps the
/// monomorphization-collapse trap). A `Vec<usize>` (not a `SmallVec`) is the
/// bucket: `boyko_utils` ships no small-vec, and per-hash candidate counts are
/// ~1 in practice, so the allocation is negligible on this cold path.
#[allow(clippy::type_complexity)]
static STABLE_NAME_INDEX: OnceLock<Mutex<HashMap<u64, Vec<usize>>>> = OnceLock::new();

/// Lazily initializes and returns the [`STABLE_NAME_INDEX`] table.
fn stable_name_index() -> &'static Mutex<HashMap<u64, Vec<usize>>> {
    STABLE_NAME_INDEX.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `C`'s stable name in the `STABLE_NAME_INDEX` (C1). Called from the
/// derive's `component_id()` closure UNGATED (like [`install_clone_fn`]), once per
/// component per process. COLD — never a frame path.
///
/// The index maps `stable_name_hash → candidate ComponentIds`. A second
/// registration of the SAME id under the same hash is deduped (idempotent — the
/// `component_id()` `OnceLock` already guarantees one call per type, but a
/// hand-written impl calling twice stays correct). A DIFFERENT id sharing the hash
/// (a genuine collision, or two distinct types with the same stable name) is
/// appended; [`resolve_stable_name`] disambiguates by the full name.
pub fn register_stable_name<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let hash = fnv1a_64(C::stable_name().as_bytes());
    let mut index = stable_name_index()
        .lock()
        .expect("invariant: STABLE_NAME_INDEX lock poisoned only after a panic under the guard");
    let bucket = index.entry(hash).or_default();
    if !bucket.contains(&component_id) {
        bucket.push(component_id);
    }
}

/// Load-time resolution (C1): maps a saved `(stable_name_hash, stable_name)` to
/// the running process's `ComponentId`, or `None` if no registered component
/// carries that stable name. Called ONCE per file-local type at load (never per
/// entity). COLD.
///
/// On a hash hit it compares the FULL `stable_name` of each candidate to confirm —
/// a hash collision (or two distinct types hashing equal) is resolved by the
/// string compare, so only the component whose registered `stable_name` exactly
/// equals `name` is returned. `None` covers both "never registered" and "hash hit
/// but no full-name match".
pub fn resolve_stable_name(hash: u64, name: &str) -> Option<usize> {
    let index = stable_name_index()
        .lock()
        .expect("invariant: STABLE_NAME_INDEX lock poisoned only after a panic under the guard");
    let bucket = index.get(&hash)?;
    for &candidate_id in bucket {
        // Confirm the full name on the hash hit — disambiguates collisions.
        if let Some(info) = get_serialize_info(candidate_id)
            && info.stable_name == name
        {
            return Some(candidate_id);
        }
    }
    None
}

// ── Autoref serialize-classification probes (Phase S0, derive support) ──────────
//
// STRICTER than the clone `CloneProbe` (C3). The `#[derive(Component)]` macro
// cannot resolve a type to ask "is every field all-bits-valid?". These three
// zero-sized probe arms use AUTOREF SPECIALIZATION (the dtolnay rule, identical
// mechanism to `CloneProbe`) to pick the right `Serializability` at the type
// level, reflection-free — but the POB arm is gated on a NEW `Pod` marker
// (`SerPod`) that is implemented ONLY for the all-bits-valid primitives, NOT on
// the loose `C: Copy` the clone path uses. So a `Copy` type that contains a
// `bool`/`char`/enum/niche/`Entity` field does NOT satisfy `SerPod` and falls to
// the `SerializeViaFn` arm (whose `deserialize_fn` validates those fields on an
// untrusted read).
//
//   * `SerPobArm for &&SerializeProbe<C, true>` (gated `C: SerPod` AND
//     `POB_ELIGIBLE == true`) — the MOST-ref'd `Self`, HIGHEST priority. A
//     provably-all-bits-valid `#[repr(C/transparent)]` type with no
//     bool/char/enum/niche/Entity wins here → `PlainOldBytes`. The macro passes
//     `POB_ELIGIBLE == false` for a non-`repr(C)` type or one with an `Entity`
//     field, removing this arm as a candidate (a const mismatch, not a bound
//     failure — so the type cleanly falls through).
//   * `SerViaFnArm for &SerializeProbe<C, POB_ELIGIBLE>` (gated `C: Clone`) —
//     middle priority. A `Clone` type that is NOT provably-POB lands here →
//     `SerializeViaFn`.
//   * `SerIgnoreArm for SerializeProbe<C, POB_ELIGIBLE>` (no bound) — the
//     by-value `Self`, LOWEST priority. A non-`Clone` type reaches it → `Ignore`.
//
// The call site (derive codegen) invokes the probe through THREE refs
// (`(&&&probe).method()`), exactly like `CloneProbe`: the resolver selects the
// highest-priority APPLICABLE arm. The ref count MUST stay `&&&` to agree with the
// arm receiver depths below.

/// Sealed marker for the all-bits-valid primitive types that may appear in a
/// [`Serializability::PlainOldBytes`] component (C3). Implemented ONLY for
/// integers, floats, and raw pointers — every bit pattern of these is a valid
/// value, so loading one from untrusted bytes can never be UB. **Deliberately NOT
/// implemented** for `bool`, `char`, fieldless enums, or niche-optimized types
/// (`NonZeroU32`, `Option<NonNull<_>>`, …): those have invalid bit patterns, so a
/// corrupt byte would instantiate an invalid value.
///
/// The derive does NOT emit a per-struct `SerPod` impl (a conditional `impl SerPod
/// for Struct where Field: SerPod {}` is rejected by the compiler when a concrete
/// field is not `SerPod` — it eagerly evaluates the false bound rather than
/// silently dropping the impl). Instead the derive passes the struct's FIELD TUPLE
/// `(F0, F1, …)` as a TYPE PARAMETER of [`SerializeProbe`], and the
/// [`SerPobArm`] is gated `F: SerPodTuple` — a GENERIC bound on the probe arm,
/// which the autoref resolver can leave UN-matched (deferring, not erroring) when a
/// field is not `SerPod`, demoting the type to [`Serializability::SerializeViaFn`]
/// (C3). [`SerPodTuple`] proves "every element is `SerPod`" via generic tuple
/// impls, so a `bool`/`char`/enum/niche field fails the bound and the POB arm is
/// simply skipped.
///
/// # Safety
/// Implementing `SerPod` for a type asserts that EVERY bit pattern of
/// `size_of::<Self>()` bytes is a valid `Self`. The leaf impls below uphold this
/// for the language-primitive types; do NOT implement it for any type with a
/// validity invariant.
pub unsafe trait SerPod: 'static {}

// Leaf impls: all-bits-valid language primitives only.
//
// SAFETY (each): every bit pattern of these widths is a valid value of the type.
// Integers and floats have no invalid representations; a raw pointer may hold any
// address bits (validity of the POINTEE is not a property of the pointer value).
unsafe impl SerPod for u8 {}
unsafe impl SerPod for u16 {}
unsafe impl SerPod for u32 {}
unsafe impl SerPod for u64 {}
unsafe impl SerPod for u128 {}
unsafe impl SerPod for usize {}
unsafe impl SerPod for i8 {}
unsafe impl SerPod for i16 {}
unsafe impl SerPod for i32 {}
unsafe impl SerPod for i64 {}
unsafe impl SerPod for i128 {}
unsafe impl SerPod for isize {}
unsafe impl SerPod for f32 {}
unsafe impl SerPod for f64 {}
// SAFETY: a raw pointer value is any address-sized bit pattern; every such
// pattern is a valid `*const T` / `*mut T` (the pointer's VALIDITY as a
// dereferenceable address is a separate, runtime concern, not a type-validity
// invariant — so reading one from bytes is sound).
unsafe impl<T: 'static> SerPod for *const T {}
unsafe impl<T: 'static> SerPod for *mut T {}

// SAFETY: an array `[T; N]` has NO padding between or after its elements
// (`size_of::<[T; N]>() == N * size_of::<T>()`), so its bytes are exactly `N`
// contiguous `T` values laid end to end. If every bit pattern of `size_of::<T>()`
// bytes is a valid `T` (`T: SerPod`), then every bit pattern of
// `size_of::<[T; N]>()` bytes is a valid `[T; N]`. (`T: SerPod` already implies
// `T: 'static`, so `[T; N]: 'static` holds.) This is the standard `Pod`-for-arrays
// rule (cf. bytemuck/zerocopy `Pod for [T; N]`). Its absence silently demoted
// every component with an array field — `[f32; N]` transforms, vectors, the
// common case — from the whole-column `PlainOldBytes` blit to the per-row
// `SerializeViaFn` encode path (the C3 demotion), so adding it restores the fast
// path the POB design intended.
unsafe impl<T: SerPod, const N: usize> SerPod for [T; N] {}

/// "Every element of this tuple is [`SerPod`]" — the field-validity proof for the
/// [`SerPobArm`] (C3). Implemented by GENERIC tuple impls (arity 0..=16, the
/// realistic component field count), so the POB arm's `F: SerPodTuple` bound is a
/// bound on a probe TYPE PARAMETER the autoref resolver can leave un-matched
/// (deferring, not erroring) when an element is not `SerPod`. The derive passes the
/// struct's field tuple `(F0, F1, …)` as that parameter; a unit struct passes `()`
/// (vacuously `SerPodTuple` — a blittable ZST tag).
#[doc(hidden)]
pub trait SerPodTuple {}

macro_rules! impl_serpod_tuple {
    ($($name:ident),*) => {
        impl<$($name: SerPod),*> SerPodTuple for ($($name,)*) {}
    };
}

impl SerPodTuple for () {}
impl_serpod_tuple!(A);
impl_serpod_tuple!(A, B);
impl_serpod_tuple!(A, B, C);
impl_serpod_tuple!(A, B, C, D);
impl_serpod_tuple!(A, B, C, D, E);
impl_serpod_tuple!(A, B, C, D, E, F);
impl_serpod_tuple!(A, B, C, D, E, F, G);
impl_serpod_tuple!(A, B, C, D, E, F, G, H);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

/// Probe wrapper for autoref serialize classification (see module note). `Fields`
/// is the struct's field tuple `(F0, F1, …)` (the macro fills it; a unit struct
/// passes `()`); `POB_ELIGIBLE` is the macro's "`#[repr(C/transparent)]` AND no
/// `Entity` field" syntactic flag. The POB arm fires only when BOTH `POB_ELIGIBLE
/// == true` AND `Fields: SerPodTuple` (every field all-bits-valid) AND `C: Copy`;
/// otherwise the type falls to [`Serializability::SerializeViaFn`] / `Ignore`.
#[doc(hidden)]
pub struct SerializeProbe<C, Fields, const POB_ELIGIBLE: bool>(
    pub core::marker::PhantomData<(C, Fields)>,
);

impl<C, Fields, const POB_ELIGIBLE: bool> SerializeProbe<C, Fields, POB_ELIGIBLE> {
    /// Constructs the probe (called by derive-generated code only).
    #[doc(hidden)]
    #[inline]
    pub const fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

/// By-value-`Self` fallback arm (no bound): a non-`Clone` type is `Ignore`. The
/// LEAST-specific `Self` (fewest refs), LOWEST priority — wins only when neither
/// the POB arm nor the `Clone` arm applies.
#[doc(hidden)]
pub trait SerIgnoreArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C, Fields, const POB_ELIGIBLE: bool> SerIgnoreArm
    for SerializeProbe<C, Fields, POB_ELIGIBLE>
{
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::Ignore
    }
}

/// `&`-`Self` arm gated `C: Clone` (middle priority): a `Clone` type that is NOT
/// provably-POB is `SerializeViaFn`. More specific than the by-value `Ignore`
/// fallback, less specific than the `&&`-`Self` POB arm.
#[doc(hidden)]
pub trait SerViaFnArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C: Clone + 'static, Fields, const POB_ELIGIBLE: bool> SerViaFnArm
    for &SerializeProbe<C, Fields, POB_ELIGIBLE>
{
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::SerializeViaFn
    }
}

/// `&&`-`Self` arm gated `C: Copy`, `Fields: SerPodTuple`, AND `POB_ELIGIBLE ==
/// true` (the MOST specific `Self`, HIGHEST priority): a `#[repr(C/transparent)]`
/// `Copy` type with no `Entity` field and ALL fields all-bits-valid is
/// `PlainOldBytes`. Being most-specific it wins before the `&`-`Self` `Clone` arm
/// (`Copy ⊆ Clone`) can match; a type with a non-`SerPod` field fails the
/// `Fields: SerPodTuple` bound (the autoref resolver leaves this arm un-matched
/// and DEFERS to the `Clone` arm — it does NOT error, the C3 silent demotion), and
/// a non-repr-C / Entity-bearing type carries `POB_ELIGIBLE == false`.
///
/// `Copy` is required HERE (not as a `SerPod` supertrait): a POB column is
/// byte-copied whole, so the type must be `Copy`; `SerPod` stays a pure
/// all-bits-valid marker so the `Fields: SerPodTuple` field proof composes cleanly.
#[doc(hidden)]
pub trait SerPobArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C: Copy + 'static, Fields: SerPodTuple> SerPobArm for &&SerializeProbe<C, Fields, true> {
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::PlainOldBytes
    }
}

// ── Owning / bit-restricted encode glue (Phase S1.5, plan §3.1 / §3.7) ──────────
//
// The `SerializeViaFn` encode path runs a per-element `serialize_fn` /
// `deserialize_fn` that walks a component's fields through the `Wire` codec. The
// derive CANNOT emit a verbatim `field.wire_write(sink)` body unconditionally: that
// would impose `FieldTy: Wire` on EVERY derived component (a concrete inherent impl
// with an unsatisfiable bound is a hard `E0277`, not a silent skip — confirmed), so
// an existing `Clone` component with a non-`Wire` field (`Box<u32>`, `Rc<u32>`, …)
// would fail to compile. Instead the derive emits a thin, bound-free `WireBridge`
// (struct ↔ field-tuple) and the GENERIC glue below carries the `Wire` bound on the
// field tuple. The encode-fn autoref arm (`WireFnProbe`) selects the glue ptr ONLY
// when `C::Owned: WireTuple` holds (every field `Wire`) AND the type is not
// POB-eligible — otherwise it DEFERS to the `None` fallback (the graceful demotion
// the `SerPodTuple` POB gate already uses, mirroring the house style).

/// Struct ↔ field-tuple bridge the derive emits for a serializable component (plan
/// §3.7). Carries NO `Wire` bound (so it compiles for ANY plain struct, including
/// one with non-`Wire` fields, and — crucially — one that implements `Drop`) — the
/// `Wire` requirement lives on the generic [`serialize_via_wire`] /
/// [`deserialize_via_wire`] glue's `WireRefTuple` / `WireTuple` bounds, which the
/// encode-fn autoref arm checks and defers on.
///
/// The derive maps a component `struct C { f0: F0, f1: F1, … }` to:
/// - `Owned = (F0, F1, …)` and `from_owned(t) = C { f0: t.0, f1: t.1, … }` (the
///   §3.7 "fields in declaration order" read constructor);
/// - `Refs<'a> = (&'a F0, &'a F1, …)` and `as_refs(&self) = (&self.f0, …)` — a tuple
///   of **borrows**, so the write path never MOVES a field out of `self` (which a
///   `Drop` component forbids — `E0509`) and never needs `Clone`.
///
/// A unit struct maps to `Owned = ()` / `Refs<'a> = ()`.
pub trait WireBridge: Sized {
    /// The component's fields as an OWNED tuple, in declaration order — the decode
    /// target (`from_owned` rebuilds the struct from it).
    type Owned;

    /// The component's fields as a tuple of BORROWS, in declaration order — the
    /// encode source (`as_refs` produces it without moving any field, so a `Drop`
    /// component is fine). The GAT lifetime ties the borrows to `&self`.
    type Refs<'a>
    where
        Self: 'a;

    /// Borrows the component's fields into the ref tuple (the encode source). No
    /// move-out, no `Clone`, no `Wire` bound — pure field borrows.
    fn as_refs(&self) -> Self::Refs<'_>;

    /// Reconstructs the component from a decoded owned field tuple (the `C { … }`
    /// constructor, §3.7). Pure value move into the fields — no allocation, no
    /// `Wire` bound.
    fn from_owned(owned: Self::Owned) -> Self;
}

/// Owning / bit-restricted serialize glue (plan §3.1 / §3.7): read `&C`, borrow its
/// fields into the ref tuple, and write each through `WireRefTuple`. The single
/// monomorphized free fn the derive installs as the [`SerializeFn`] for a
/// [`Serializability::SerializeViaFn`] component — no vtable, no `Box<dyn>`, no
/// clone (mirrors [`clone_via_clone`]'s reach-no-world-state boundary).
///
/// # W7 — cannot reach world state
/// Receives ONLY `*const u8` / a `&mut SaveCursor`; it has no world view, so the
/// user `Wire::wire_write` code it runs cannot create the F2 protected-tag conflict
/// (same boundary as [`clone_via_clone`]).
///
/// # Safety
///
/// The caller must uphold the [`SerializeFn`] contract:
/// - `src` is a live, aligned, initialized `C` (established at the save call site by
///   the column row-pointer walk); we form a shared `&C` only.
/// - `sink` is a valid, append-only cursor; the fn only appends.
pub unsafe fn serialize_via_wire<C>(
    src: *const u8,
    sink: &mut crate::ecs::core::serialize::SaveCursor<'_>,
) where
    C: WireBridge,
    for<'a> C::Refs<'a>: crate::ecs::core::serialize::WireRefTuple,
{
    use crate::ecs::core::serialize::WireRefTuple as _;
    // SAFETY: `src` is a valid, live, aligned, initialized `C` (the `SerializeFn`
    // contract, established at the column row-pointer call site). The shared `&C`
    // lives only for the borrow + write; the source row is read-only during the save
    // (the saver never mutates the world), so no `&mut C` aliases it.
    let value = unsafe { &*src.cast::<C>() };
    value.as_refs().ref_tuple_write(sink);
}

/// Owning / bit-restricted deserialize glue (plan §3.1 / §3.7): read each field
/// through `WireTuple` in declaration order, then `ptr::write` the reconstructed
/// `C` into the UNINITIALIZED `dst`. The single monomorphized free fn the derive
/// installs as the [`DeserializeFn`] for a [`Serializability::SerializeViaFn`]
/// component.
///
/// On a malformed stream the field read fails BEFORE the `ptr::write`, so `dst` is
/// left uninitialized and the caller (S2) must not drop it — the W5 partial-row
/// contract.
///
/// # Safety
///
/// The caller must uphold the [`DeserializeFn`] contract:
/// - `dst` points at writable, **uninitialized** space of `>= size_of::<C>()`
///   bytes, aligned to `align_of::<C>()`.
/// - On `Ok`, `dst` holds an initialized `C` written exactly once (no prior value
///   is dropped); on `Err`, `dst` is left uninitialized.
pub unsafe fn deserialize_via_wire<C>(
    src: &mut crate::ecs::core::serialize::LoadCursor<'_>,
    dst: *mut u8,
) -> Result<(), crate::ecs::core::serialize::DecodeError>
where
    C: WireBridge,
    C::Owned: crate::ecs::core::serialize::WireTuple,
{
    // Read every field first; on a malformed stream this returns `Err` and `dst` is
    // never written (the value is built fully before any write). The trait method is
    // called fully-qualified, so no `use` is needed here.
    let owned = <C::Owned as crate::ecs::core::serialize::WireTuple>::tuple_read(src)?;
    let value = C::from_owned(owned);
    // SAFETY: `dst` is writable, uninitialized, aligned space for one `C` (the
    // `DeserializeFn` contract). `ptr::write` initializes it WITHOUT dropping the
    // uninitialized prior contents; `value` is moved in exactly once.
    unsafe {
        core::ptr::write(dst.cast::<C>(), value);
    }
    Ok(())
}

/// Autoref probe selecting the [`SerializeFn`] / [`DeserializeFn`] pair for a
/// component (plan §3.7 / C3 graceful demotion). The `&`-`Self` "some" arm requires
/// `C: WireBridge`, `for<'a> C::Refs<'a>: WireRefTuple`, and `C::Owned: WireTuple`
/// (every field `Wire`); the by-value "none" arm (no bound) is the fallback.
///
/// This probe does NOT key on the syntactic POB-eligibility flag: a
/// `#[repr(C)]`-but-not-all-bits-valid struct (e.g. a `String` field) is
/// `POB_ELIGIBLE == true` syntactically yet classified `SerializeViaFn` at runtime,
/// so suppressing the encoder on the syntactic flag would wrongly leave it
/// `None`. Instead the encoder is selected whenever every field is `Wire`, and
/// [`install_serialize_fn`] gates the install on the RUNTIME `Serializability`:
/// `SerializeViaFn` keeps the `Some`, while `PlainOldBytes` (blit path) / `Ignore`
/// store `None` — the single source of truth for "POB installs None".
///
/// Invoked through TWO refs (`(&&probe).serialize_fn_ptr()`): the resolver tries the
/// more-specific `&`-`Self` "some" arm first; if its bounds hold it returns
/// `Some(glue)`, otherwise it DEFERS to the by-value "none" arm (`None`) — never a
/// hard error (the §5 C3 / `SerPodTuple` graceful-demotion discipline).
#[doc(hidden)]
pub struct WireFnProbe<C>(pub core::marker::PhantomData<C>);

impl<C> WireFnProbe<C> {
    /// Constructs the probe (derive-generated code only).
    #[doc(hidden)]
    #[inline]
    pub const fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

/// By-value-`Self` fallback arm (no bound): a component with a non-`Wire` field
/// installs `None`. The LEAST-specific `Self`, LOWEST priority — wins only when the
/// `&`-`Self` "some" arm's bound does not hold.
#[doc(hidden)]
pub trait WireFnNoneArm {
    #[doc(hidden)]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn>;
    #[doc(hidden)]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn>;
}

impl<C> WireFnNoneArm for WireFnProbe<C> {
    #[inline]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn> {
        None
    }

    #[inline]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn> {
        None
    }
}

/// `&`-`Self` "some" arm gated `C: WireBridge`, `for<'a> C::Refs<'a>: WireRefTuple`,
/// `C::Owned: WireTuple` (the MORE specific `Self`, HIGHER priority): a component
/// whose every field is `Wire` installs the [`serialize_via_wire`] /
/// [`deserialize_via_wire`] glue. A type with a non-`Wire` field fails the
/// `WireRefTuple` / `WireTuple` bound and the resolver leaves this arm un-matched,
/// DEFERRING to the `None` fallback (C3 graceful demotion). A genuinely POB type's
/// fields are all SerPod primitives (which are all `Wire`), so this arm produces a
/// `Some` for it too — [`install_serialize_fn`] then drops it to `None` because the
/// runtime `Serializability` is `PlainOldBytes` (the blit path).
#[doc(hidden)]
pub trait WireFnSomeArm {
    #[doc(hidden)]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn>;
    #[doc(hidden)]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn>;
}

impl<C> WireFnSomeArm for &WireFnProbe<C>
where
    C: WireBridge + 'static,
    for<'a> C::Refs<'a>: crate::ecs::core::serialize::WireRefTuple,
    C::Owned: crate::ecs::core::serialize::WireTuple,
{
    #[inline]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn> {
        Some(serialize_via_wire::<C> as SerializeFn)
    }

    #[inline]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn> {
        Some(deserialize_via_wire::<C> as DeserializeFn)
    }
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
/// - `register_layout::<T>(component_id)` (test-only escape hatch).
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
/// - `register_layout::<T>(component_id)` (test-only escape hatch).
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
/// - `register_layout::<T>(component_id)` (test-only escape hatch).
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
/// - `register_layout::<T>(component_id)` (test-only escape hatch).
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
/// - `register_layout::<T>(component_id)` (test-only escape hatch).
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

    /// Dense plan C1 #0 — THE reader-regression test. A `Dense`-classified id
    /// MUST read back as `StorageKind::Dense`, not silently fall through to the
    /// `Table` default. Before the explicit `Dense` arm in `storage_kind`,
    /// discriminant 2 fell into `_ => Table`, re-entering the signature.
    #[test]
    fn set_storage_kind_round_trips_dense() {
        // Fixed id 352, grep-verified free in [348, 360) in the shared lib-test
        // process (disjoint from the 404-407 storage-kind block and the 320-347
        // archetype/master/query fixtures).
        set_storage_kind(352, StorageKind::Dense);
        assert_eq!(
            storage_kind(352),
            StorageKind::Dense,
            "set_storage_kind(Dense) must round-trip through storage_kind (C1 #0 \
             reader-regression: discriminant 2 must NOT fall through to Table)"
        );
    }

    /// Dense plan W1 — a dense id's residency defaults to `Cpu` (dense is ALWAYS
    /// host-resident). Classifying storage as `Dense` does not touch the
    /// residency table, so the default `Cpu` stands.
    #[test]
    fn dense_id_residency_defaults_to_cpu() {
        set_storage_kind(352, StorageKind::Dense);
        assert_eq!(
            residency_class(352),
            ResidencyKind::Cpu,
            "a dense id is always ResidencyKind::Cpu (Dense plan W1)"
        );
    }

    /// Dense plan C1 — `is_signature_storage` is the single predicate every
    /// signature-exclude site routes through. Only `Table` is a signature kind;
    /// both `Bitset` and `Dense` are excluded. (Confirms the 0%-gate: the
    /// `Table`/`Bitset` answers are identical to the pre-refactor explicit
    /// `== Bitset` comparison.)
    #[test]
    fn is_signature_storage_only_table() {
        assert!(
            is_signature_storage(StorageKind::Table),
            "Table is the only signature-storage kind"
        );
        assert!(
            !is_signature_storage(StorageKind::Bitset),
            "Bitset is excluded from the signature (unchanged behavior)"
        );
        assert!(
            !is_signature_storage(StorageKind::Dense),
            "Dense is excluded from the signature (Dense plan C1)"
        );
    }

    /// Write-once enforcement extends to `Dense`: reclassifying a `Dense` id to a
    /// different kind trips the debug assertion (same discipline as bitset).
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "already classified")]
    fn set_storage_kind_reclassify_dense_to_table_panics_in_debug() {
        set_storage_kind(353, StorageKind::Dense);
        set_storage_kind(353, StorageKind::Table);
    }

    /// Dense plan W1 — `storage = "dense"` combined with a non-`Cpu` residency is
    /// rejected. The derive forces `Cpu` (it never sets a non-`Cpu` `RESIDENCY`
    /// for a dense type and rejects a gpu residency attribute at compile time), so
    /// this guards a hand-`impl Component` that wrongly pairs
    /// `STORAGE_IS_DENSE = true` with `RESIDENCY = Gpu`. The
    /// `install_dense_storage_kind` debug assertion fires before classification.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "dense storage is always Cpu-resident")]
    fn dense_plus_gpu_residency_is_rejected_in_debug() {
        // A minimal hand-`impl Component` fixture: dense storage + (illegal) GPU
        // residency. Only `component_id()` is required; the rest defaults.
        struct DenseGpuFixture;
        impl Component for DenseGpuFixture {
            #[inline]
            fn component_id() -> ComponentId {
                // Fixed id 354 (grep-verified free in the shared lib-test
                // process); `install_dense_storage_kind` reads only the passed id
                // and the consts, so no `register_new` mint is needed.
                ComponentId(354)
            }
            const STORAGE_IS_DENSE: bool = true;
            const RESIDENCY: ResidencyKind = ResidencyKind::Gpu;
        }
        // The W1 assertion in `install_dense_storage_kind` must fire BEFORE the id
        // is classified.
        install_dense_storage_kind::<DenseGpuFixture>(354);
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

    // ----- Phase 4 Seam 1: residency classification -----
    //
    // Fixed ids 408-412 reserved for these residency tests, adjacent to the
    // storage-kind 404-407 block and disjoint from every other fixed-id range
    // (layout 450-465 / 498-499; storage 470-475).

    #[test]
    fn residency_class_defaults_to_cpu() {
        // An id never classified reads back as the host default.
        assert_eq!(
            residency_class(408),
            ResidencyKind::Cpu,
            "unclassified id must default to ResidencyKind::Cpu"
        );
    }

    #[test]
    fn set_residency_class_round_trips_gpu() {
        set_residency_class(409, ResidencyKind::Gpu);
        assert_eq!(
            residency_class(409),
            ResidencyKind::Gpu,
            "set_residency_class(Gpu) must round-trip through residency_class"
        );
    }

    #[test]
    fn set_residency_class_round_trips_cpu_pinned() {
        set_residency_class(410, ResidencyKind::CpuPinned);
        assert_eq!(
            residency_class(410),
            ResidencyKind::CpuPinned,
            "set_residency_class(CpuPinned) must round-trip through residency_class"
        );
    }

    #[test]
    fn set_residency_class_same_kind_is_idempotent() {
        // Two identical classifications are a silent no-op (no panic).
        set_residency_class(411, ResidencyKind::Gpu);
        set_residency_class(411, ResidencyKind::Gpu);
        assert_eq!(residency_class(411), ResidencyKind::Gpu);
    }

    #[test]
    fn classify_component_residency_public_path_round_trips() {
        // The public runtime entry mirrors set_residency_class for foreign ids.
        classify_component_residency(412, ResidencyKind::Gpu);
        assert_eq!(
            residency_class(412),
            ResidencyKind::Gpu,
            "classify_component_residency must round-trip through residency_class"
        );
    }

    /// Write-once enforcement: reclassifying an id to a DIFFERENT class must trip
    /// the debug assertion. Runs only in debug builds (where `debug_assert!` is
    /// active) — release skips the store and preserves the first class.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "already classified")]
    fn set_residency_class_reclassify_different_kind_panics_in_debug() {
        set_residency_class(413, ResidencyKind::Gpu);
        // Reclassifying to a different class is an invariant violation.
        set_residency_class(413, ResidencyKind::CpuPinned);
    }
}
