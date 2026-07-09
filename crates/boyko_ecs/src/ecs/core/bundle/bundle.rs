//! The sealed `Bundle` trait + coalesced per-impl static payload.
//!
//! Step 1 of the Phase 8.5 Static Bundle Cache rewrite — see
//! `docs/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md` §4.1 (`Bundle` trait shape),
//! §4.4 (`BundleStaticInfo` O3 coalescing) and §2.3 (invariants SBC1..SBC9).
//!
//! # Scope of this file
//!
//! 1. [`sealed::BundleSealed`] — the module-private supertrait that prevents
//!    downstream crates (and even the rest of `boyko_ecs`) from writing
//!    manual `impl Bundle for Foo` blocks. Only `#[derive(Bundle)]` (Step 4)
//!    is permitted to mint `impl sealed::BundleSealed for Foo {}`. This is
//!    the **SBC1** enforcement mechanism — see §2.3.
//!
//! 2. [`BundleStaticInfo`] — the **O3 acceptance** struct that coalesces the
//!    per-impl `BundleTypeId` and the canonical-sorted `&'static
//!    [ComponentId]` into a single payload stored behind one per-impl
//!    `OnceLock`. Saves one cache line + one Acquire load on the cached hot
//!    path versus the Round 1 design that used two separate `OnceLock`s
//!    (see §4.4 / §6.1 / Decision SBC-D5).
//!
//! 3. [`Bundle`] — the four-method trait that `#[derive(Bundle)]` will
//!    implement in Step 4. Only `static_info` has no default body — the
//!    other three methods delegate through it (cheap inline single load).
//!
//! # What Step 1 does NOT do
//!
//! - It does not delete `bundle_impls.rs` — that is Step 2. After this
//!   rewrite lands, the tuple impls in `bundle_impls.rs` will fail to
//!   compile because they do not satisfy the new `sealed::BundleSealed`
//!   supertrait. The breakage is **intentional and expected** per plan §9
//!   Step 1 acceptance — Step 2 removes the file outright.
//! - It does not implement `Bundle::cached_archetype_id` for any concrete
//!   type — the derive macro at Step 4 generates per-impl bodies that
//!   call into `EcsMaster::bundle_archetype_id_for::<Self>()` (added in
//!   Step 3).
//! - It does not add the per-`EcsMaster` `bundle_archetype_cache` field —
//!   that is Step 3.

use crate::ecs::core::bundle::bundle_type_registry::BundleTypeId;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Maximum component count for a derived `Bundle`. Mirrors the macro's
/// `MAX_BUNDLE_ARITY` and the sibling stack collectors in
/// `spawn_batch_command.rs` / `spawn_at_command.rs` / `migration_helpers.rs`.
/// Sizes the fixed-width [`BundleColumnPtrs`] / [`ColumnPtr`] stack arrays so
/// the per-batch typed-write resolve never allocates.
pub const MAX_BUNDLE_ARITY: usize = 16;

/// Phase D4b (Q2): arity ceiling at or below which `#[derive(Bundle)]` emits a
/// const-unrolled [`Bundle::write_row_typed`] (`HAS_TYPED_WRITE = true`).
///
/// Initialised to [`MAX_BUNDLE_ARITY`] so every derived bundle takes the typed
/// path; the retained byte path
/// ([`Bundle::for_each_data_component_bytes`]) is the fallback for any future
/// lowering of this threshold (a high-arity unrolled body that bloats the spawn
/// I-cache). The proc-macro mirrors the literal `16` and cross-checks it
/// against this constant via a `const _: () = assert!(...)` (the macro cannot
/// import `boyko_ecs`).
pub const MAX_TYPED_WRITE_ARITY: usize = 16;

// CONFIRM / D4b cross-check: the `#[derive(Bundle)]` proc-macro cannot import
// `boyko_ecs`, so it mirrors the literal `16` for the `HAS_TYPED_WRITE`
// threshold and the `write_row_typed` perm indexing. This assert pins the two
// in lock-step — bumping `MAX_TYPED_WRITE_ARITY` here without updating the macro
// (or vice versa) fails the build.
const _: () = assert!(MAX_TYPED_WRITE_ARITY == 16);
const _: () = assert!(MAX_TYPED_WRITE_ARITY <= MAX_BUNDLE_ARITY);

/// One resolved destination column for the typed batch-write path (Decision 4).
///
/// Built **once per batch** by
/// `ComponentPoolBundle::resolve_column_ptrs`
/// under a single `&mut` borrow of the pool bundle that is then dropped before
/// the row loop begins (W2 single-provenance). The row loop holds only the raw
/// `base` pointer and the const-known `stride`; it never re-borrows the
/// archetype/pool bundle (the 14a-F2 / 9.3c cached-pointer-then-reborrow
/// antidote).
///
/// # Fields
///
/// * `base` — write-capable provenance for the column's data sub-region; the
///   pool's write-once, vm-reservation-stable base (Phase X.I).
/// * `stride` — `component_layout.size()` of the column's registered type. A
///   typed store at row `r` targets `base.add(r * stride)`.
///
/// # Debug-only invariant carriers
///
/// * `committed_rows` (Q1) — the pool's committed-row ceiling at resolve time;
///   `write_row_typed` asserts `row < committed_rows` per store, restoring the
///   guard the byte path has at `ComponentPool::write_at_unchecked_initialized`.
/// * `comp_id` (W3) — the column's [`ComponentId`]; `write_row_typed` asserts
///   `Tk::component_id() == comp_id` at IDENTITY level (two same-size,
///   different-type components must never silently swap columns).
#[derive(Clone, Copy)]
pub struct ColumnPtr {
    /// Write-capable base of the column's data sub-region (W2 single
    /// provenance). Cast to `*mut Tk` at the store site.
    base: *mut u8,

    /// `size_of::<Tk>()` for the column's registered type — the per-row stride.
    /// The typed store at the derive site uses the **compile-time** constant
    /// `size_of::<Tk>()` directly (a fixed-width store — the whole point of D4),
    /// so this runtime copy is read only by the debug stride-parity assert
    /// (`column_stride`). Retained as a field per the architect's `ColumnPtr`
    /// spec and to keep the resolved column self-describing.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    stride: usize,

    /// Q1: the column's committed-row ceiling at resolve time, asserted by
    /// `write_row_typed` (`row < committed_rows`).
    #[cfg(debug_assertions)]
    committed_rows: usize,

    /// W3: the column's [`ComponentId`] for the per-field IDENTITY assert.
    #[cfg(debug_assertions)]
    comp_id: ComponentId,
}

impl ColumnPtr {
    /// Constructs a resolved column. `pub(crate)` so only the pool bundle's
    /// `resolve_column_ptrs` (which owns the provenance) can mint one.
    #[inline]
    pub(crate) fn new(
        base: *mut u8,
        stride: usize,
        #[cfg(debug_assertions)] committed_rows: usize,
        #[cfg(debug_assertions)] comp_id: ComponentId,
    ) -> Self {
        Self {
            base,
            stride,
            #[cfg(debug_assertions)]
            committed_rows,
            #[cfg(debug_assertions)]
            comp_id,
        }
    }
}

/// Stack-local per-batch resolution of every DATA column for the typed
/// batch-write path (Decision 4).
///
/// Built once per batch by
/// `ComponentPoolBundle::resolve_column_ptrs`;
/// lives entirely within `SpawnBatchCommand::apply`'s frame and is consumed
/// without any intervening `&mut EcsMaster` / `&mut Archetype` reborrow (M3 TB
/// contract). The derive-emitted [`Bundle::write_row_typed`] reads it per row.
///
/// # Field semantics
///
/// * `col[..data_len]` — resolved columns in canonical **DATA-column** order
///   (the canonical `pool_ids` filtered to non-ZST, identical to the byte
///   path's `data_pool_ids` order).
/// * `perm[k]` — for declaration field `k`, the canonical data-column slot to
///   store into (`u8::MAX` = ZST field, skipped). Built correct-by-construction
///   from the canonical `data_pool_ids` (W3).
/// * `data_len` — number of populated `col` entries (== non-ZST pool columns).
pub struct BundleColumnPtrs {
    /// Resolved columns, `col[..data_len]` valid, canonical data-column order.
    col: [ColumnPtr; MAX_BUNDLE_ARITY],

    /// `perm[k]` = canonical data-column slot for declaration field `k`;
    /// `u8::MAX` marks a ZST field (skipped by `write_row_typed`).
    perm: [u8; MAX_BUNDLE_ARITY],

    /// Count of populated `col` entries.
    data_len: usize,
}

impl BundleColumnPtrs {
    /// Sentinel `perm` entry for a ZST / skipped declaration field.
    pub const PERM_SKIP: u8 = u8::MAX;

    /// Creates an empty, zeroed scratch suitable for repeated
    /// `ComponentPoolBundle::resolve_column_ptrs`
    /// fills. `base`s are null and `data_len == 0` until a resolve runs.
    #[inline]
    pub fn new() -> Self {
        Self {
            col: [ColumnPtr {
                base: core::ptr::null_mut(),
                stride: 0,
                #[cfg(debug_assertions)]
                committed_rows: 0,
                #[cfg(debug_assertions)]
                comp_id: ComponentId(0),
            }; MAX_BUNDLE_ARITY],
            perm: [Self::PERM_SKIP; MAX_BUNDLE_ARITY],
            data_len: 0,
        }
    }

    /// Number of populated DATA columns (== non-ZST pool columns).
    #[inline]
    pub fn data_len(&self) -> usize {
        self.data_len
    }

    /// Writer used by `resolve_column_ptrs` to populate data-column slot `i`'s
    /// resolved base. `pub(crate)` — only the pool bundle resolves.
    #[inline]
    pub(crate) fn set_column_base(&mut self, i: usize, col: ColumnPtr) {
        debug_assert!(i < MAX_BUNDLE_ARITY);
        self.col[i] = col;
    }

    /// Sets the populated-column count after a resolve pass.
    #[inline]
    pub(crate) fn set_data_len(&mut self, len: usize) {
        debug_assert!(len <= MAX_BUNDLE_ARITY);
        self.data_len = len;
    }

    /// Copies the caller-built `perm` (declaration field → canonical data-column
    /// slot, [`Self::PERM_SKIP`] for a ZST field) into this scratch. Any field
    /// beyond `perm.len()` keeps the all-skip default.
    #[inline]
    pub(crate) fn set_perm_from(&mut self, perm: &[u8]) {
        debug_assert!(perm.len() <= MAX_BUNDLE_ARITY);
        self.perm = [Self::PERM_SKIP; MAX_BUNDLE_ARITY];
        self.perm[..perm.len()].copy_from_slice(perm);
    }

    /// The canonical data-column slot for declaration field `k`
    /// ([`Self::PERM_SKIP`] for a ZST field). Read by the derive-emitted
    /// `write_row_typed`.
    #[inline]
    pub fn perm(&self, k: usize) -> u8 {
        debug_assert!(k < MAX_BUNDLE_ARITY);
        self.perm[k]
    }

    /// The write-capable base of data column `slot`. Read by the
    /// derive-emitted `write_row_typed`; the caller guarantees `slot` came
    /// from [`Self::perm`] (`< data_len`).
    #[inline]
    pub fn column_base(&self, slot: usize) -> *mut u8 {
        debug_assert!(slot < self.data_len, "column_base: slot out of range");
        self.col[slot].base
    }

    /// The committed-row ceiling of data column `slot` (debug only, Q1).
    #[cfg(debug_assertions)]
    #[inline]
    pub fn column_committed_rows(&self, slot: usize) -> usize {
        debug_assert!(slot < self.data_len);
        self.col[slot].committed_rows
    }

    /// The [`ComponentId`] of data column `slot` (debug only, W3 identity
    /// check).
    #[cfg(debug_assertions)]
    #[inline]
    pub fn column_comp_id(&self, slot: usize) -> ComponentId {
        debug_assert!(slot < self.data_len);
        self.col[slot].comp_id
    }

    /// The const-known stride of data column `slot` (debug only — the typed
    /// store uses `size_of::<Tk>()` directly).
    #[cfg(debug_assertions)]
    #[inline]
    pub fn column_stride(&self, slot: usize) -> usize {
        debug_assert!(slot < self.data_len);
        self.col[slot].stride
    }
}

impl Default for BundleColumnPtrs {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Seal module — the trait inside is referenced only as a supertrait bound
/// on [`Bundle`]. Because the trait is `pub` inside a non-`pub` module, no
/// external crate (and no other module inside `boyko_ecs` outside this
/// file) can name it, and therefore no other code can write
/// `impl sealed::BundleSealed for Foo`. The derive macro (Step 4) sidesteps
/// this by emitting the impl from inside the same crate via the macro's
/// generated path `$crate::ecs::core::bundle::bundle::sealed::BundleSealed`.
///
/// SBC1 enforcement (§2.3).
#[doc(hidden)]
pub mod sealed {
    /// Hidden supertrait. See parent module docs for the SBC1 rationale.
    ///
    /// **Visibility note**: `pub` (not `pub(crate)`) so that the
    /// `#[derive(Bundle)]` macro in `boyko_macros` can emit
    /// `impl ::boyko_ecs::ecs::core::bundle::bundle::sealed::BundleSealed for X {}`
    /// into downstream user crates (including this crate's own integration
    /// tests, which are external to `boyko_ecs` from the linker's
    /// perspective). The `#[doc(hidden)]` attribute keeps it out of the
    /// public rustdoc surface — the seal is a social/discoverability
    /// boundary (Bevy uses the same pattern for its `Bundle` seal). Users
    /// who manually `impl BundleSealed for MyType` to bypass the seal are
    /// violating documented invariant SBC1 at their own risk; the derive
    /// macro is the only blessed path.
    pub trait BundleSealed {}
}

/// Coalesced per-`Bundle`-impl static payload — **O3 acceptance** (§4.4 /
/// Decision SBC-D5).
///
/// Each `#[derive(Bundle)]`-generated impl owns exactly one per-impl
/// `static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();`. The first
/// caller of any of [`Bundle::static_info`] / [`Bundle::component_ids`] /
/// [`Bundle::bundle_type_id`] runs the init closure, which:
///
/// 1. Mints a fresh [`BundleTypeId`] from the process-global counter via
///    [`crate::ecs::core::bundle::bundle_type_registry::register_new`].
/// 2. Collects the bundle's `[ComponentId; N]` in declaration order, sorts
///    it ascending by `ComponentId.0` (canonical order, **B1**), and leaks
///    the boxed array to obtain a `&'static [ComponentId]`.
/// 3. Returns the populated `BundleStaticInfo`.
///
/// After init, every call observes the cached payload in a single Acquire
/// load (~2 ns on the hot path). Both fields share the same cache line —
/// reading `type_id` immediately after `component_ids` (or vice versa) is
/// free.
///
/// # Layout
///
/// `#[repr(C)]` to pin the field order: `type_id` first (8 B on 64-bit) so
/// that the slice fat-pointer that follows lands on its natural 8-byte
/// alignment without padding. The struct fits in 24 bytes — comfortably
/// under one cache line.
///
/// # `Send + Sync` (SBC2, §7.5)
///
/// Both fields are trivially shareable across threads:
///
/// - [`BundleTypeId`] is `#[repr(transparent)]` over `usize` (an integer).
/// - The component-id slice is `&'static [ComponentId]` — immutable shared
///   data, leaked into process-static storage at init time.
///
/// The unsafe `Send`/`Sync` impls below are redundant with the auto trait
/// inference but document the intent at the type level. Mirrors Bevy's
/// pattern on its analogous `BundleInfo`.
#[derive(Debug)]
#[repr(C)]
pub struct BundleStaticInfo {
    /// Process-global [`BundleTypeId`] for this Bundle impl. Indexes into
    /// `EcsMaster::bundle_archetype_cache` (added in Step 3).
    pub type_id: BundleTypeId,

    /// Canonical-sorted (ascending by `ComponentId.0`) `&'static`
    /// component-id slice. Leaked exactly once per Bundle type per process
    /// — bounded by `MAX_BUNDLE_TYPES × MAX_BUNDLE_ARITY × 8 B` (≤ 64 KB
    /// pathological, ~2.4 KB typical — see SBC8 / §10.4).
    pub component_ids: &'static [ComponentId],
}

// SAFETY (SBC2, §7.5):
//   - `BundleTypeId` is `#[repr(transparent)]` over `usize`; integers are
//     trivially `Send`.
//   - `&'static [ComponentId]` is an immutable shared slice into leaked
//     static memory; aliased reads from many threads are sound.
//   The explicit impl mirrors Bevy's `BundleInfo` documentation pattern;
//   the auto trait would also infer `Send` here.
unsafe impl Send for BundleStaticInfo {}

// SAFETY (SBC2, §7.5):
//   - Same composition as `Send`: integer payload + `&'static` immutable
//     slice. No interior mutability, no thread-local state.
unsafe impl Sync for BundleStaticInfo {}

/// A group of components to insert together when spawning an entity.
///
/// **Sealed** — the only path to producing a `Bundle` impl is through
/// `#[derive(Bundle)]` (Step 4). Manual `impl Bundle for Foo` blocks fail
/// to compile because the [`sealed::BundleSealed`] supertrait lives in a
/// private module that downstream code cannot name.
///
/// # Invariants
///
/// The derive macro is responsible for upholding the full invariant set;
/// these are the contracts that the rest of the ECS relies on at call
/// sites (`SpawnAtCommand::apply` / `InsertCommand::apply` — Phase 11
/// renamed `SpawnCommand::apply`):
///
/// * **B1** — [`component_ids`](Self::component_ids) returns a canonical
///   ascending slice (sorted by `ComponentId.0`). Sort happens once per
///   type at `OnceLock` init inside the generated `static_info` body
///   (§6.1).
/// * **B2** — [`for_each_component_bytes`](Self::for_each_component_bytes)
///   emits components in the **same** canonical order as
///   [`component_ids`](Self::component_ids). The derive macro emits a
///   pointer-based sort over `(ComponentId, *const u8, usize)` triples to
///   sidestep error E0521 — see §6.3 mandatory codegen template.
/// * **B3 (Phase 12.5 SBO-UNPIN)** — `Bundle: Send + Sync + Unpin + 'static`.
///   The `Unpin` supertrait is load-bearing for the deferred-command path:
///   `CommandQueue::push<C>` uses `ptr::write_unaligned` / `ptr::read_unaligned`
///   for bitwise byte-copy through the queue's byte arena, which is sound
///   only if the bundle has no self-references (`Unpin` ⇔ no `PhantomPinned`).
///   All `#[derive(Bundle)]` outputs are `Unpin` by default because every
///   bundle field is a `Component`-derived type with no `PhantomPinned`.
///   Manual `impl Bundle for Foo` is blocked by [`sealed::BundleSealed`].
/// * **B4** — On callback panic mid-iteration, components that have not
///   yet been emitted **leak** unconditionally: every destructured field
///   is wrapped in `ManuallyDrop<T>` BEFORE any callback runs. No
///   double-drop with archetype-side ownership (§6.3 SAFETY clause iv).
/// * **SBC1..SBC9** — see §2.3 of the plan for the full list.
///
/// # Calling convention summary
///
/// | Method | Hot-path cost | Caller context |
/// |--------|--------------|----------------|
/// | [`static_info`](Self::static_info) | ~2 ns (cached) / ~80 ns (cold first call) | every other trait method delegates here |
/// | [`component_ids`](Self::component_ids) | ~2 ns | `SpawnAtCommand::apply` (arity probe) |
/// | [`bundle_type_id`](Self::bundle_type_id) | ~2 ns | `cached_archetype_id` internal |
/// | [`cached_archetype_id`](Self::cached_archetype_id) | ~3 ns (cached) / ~1 µs (cold first call per (B, world)) | `SpawnAtCommand::apply` only |
/// | [`for_each_component_bytes`](Self::for_each_component_bytes) | ~10 ns + N × ~40 ns memcpy | `SpawnAtCommand::apply` / `InsertCommand::apply` |
pub trait Bundle: sealed::BundleSealed + Send + Sync + Unpin + 'static {
    /// Returns the per-impl coalesced [`BundleStaticInfo`] (O3 acceptance,
    /// §4.4).
    ///
    /// Cached path: a single `OnceLock::get` Acquire load on the per-impl
    /// `static INFO: OnceLock<BundleStaticInfo>` that the derive macro
    /// emits — roughly 2 ns on x86_64.
    ///
    /// Cold path (first call per Bundle type per process): runs the
    /// macro-generated init closure which mints the [`BundleTypeId`],
    /// collects + sorts + leaks the component-id slice, and CAS-installs
    /// the result. Roughly 80 ns; happens at most once per Bundle type
    /// across the whole process per the `OnceLock::get_or_init` contract
    /// (§7.3).
    ///
    /// This is the **only** trait method without a default body — the
    /// other three delegate through it. The derive macro is required to
    /// generate the per-impl `static` and `get_or_init` plumbing.
    fn static_info() -> &'static BundleStaticInfo;

    /// Returns the canonical-sorted `&'static [ComponentId]` (**B1**,
    /// SBC3).
    ///
    /// Default impl reads `static_info().component_ids`. Single Acquire
    /// load on the cached path.
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        Self::static_info().component_ids
    }

    /// Returns the process-global [`BundleTypeId`] for this Bundle type
    /// (**SBC2**).
    ///
    /// Default impl reads `static_info().type_id`. The id is minted on the
    /// first call to `static_info` and immutable thereafter — equal across
    /// threads and equal across `EcsMaster` instances.
    #[inline]
    fn bundle_type_id() -> BundleTypeId {
        Self::static_info().type_id
    }

    /// Resolves (and caches) the [`ArchetypeId`] that holds entities
    /// spawned from this Bundle in the given `world` (**SBC4**).
    ///
    /// **Caller contract**: this is intended to be called **only** from
    /// `SpawnAtCommand::apply` / `InsertCommand::apply` (Phase 11
    /// renamed `SpawnCommand::apply`), which hold `&mut EcsMaster` for
    /// the duration of the deferred-command flush. User code does not call
    /// this directly — `Commands::spawn<B>(bundle)` enqueues a
    /// `SpawnAtCommand<B>` that resolves lazily on flush.
    ///
    /// **Cost (§6.2)**:
    ///
    /// - Cached path (`(BundleTypeId, EcsMaster)` warm): ~3 ns. Two L1d
    ///   loads — the per-impl `BundleStaticInfo` slot + the per-world
    ///   boxed-array slot.
    /// - Cold path (first spawn of this bundle in this world): ~1.0-1.2 µs
    ///   dominated by `ArchetypeMaster::get_or_create_archetype` (mask
    ///   compute + register).
    ///
    /// No default body — the derive macro at Step 4 generates a per-impl
    /// stub that calls `world.bundle_archetype_id_for::<Self>()` (the
    /// `pub(crate)` helper added in Step 3).
    fn cached_archetype_id(world: &mut EcsMaster) -> ArchetypeId;

    /// Invokes `f` once per component in canonical order (**B2**), passing
    /// `(ComponentId, &[u8])`.
    ///
    /// The byte slice borrows from the bundle's stack frame for the
    /// duration of the callback chain. No transmute; no per-spawn
    /// allocation. `SpawnAtCommand::apply` / `InsertCommand::apply` use
    /// this to collect component byte slices into a fixed-size stack
    /// array, then hand them to `EcsMaster::create_entity_at` (or the
    /// migration helpers) for memcpy.
    ///
    /// # Panic safety (**B4**, §6.3 SAFETY clause iv)
    ///
    /// If `f` panics on iteration `i < N`, components `i..N` **leak**
    /// unconditionally: every destructured field is wrapped in
    /// `ManuallyDrop<T>` BEFORE any callback runs, so their `Drop` impls
    /// never execute regardless of panic state. Components `0..i` have
    /// already been transferred via the callback into the archetype slot
    /// (the archetype is now the logical owner of those bytes).
    ///
    /// "Leak on panic" is the documented safety/ergonomics trade-off: a
    /// leak is preferable to a double-drop UB.
    ///
    /// No default body — the derive macro at Step 4 generates a per-impl
    /// body following the §6.3 mandatory codegen template (pointer-based
    /// intermediate to sidestep E0521, ManuallyDrop-upfront for **B4**,
    /// sort by `ComponentId.0` for **B1**).
    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, f: F);

    /// Decision 4 (D4b): `true` iff `#[derive(Bundle)]` emitted a
    /// const-unrolled [`Self::write_row_typed`] for this bundle.
    ///
    /// `false` by default so hand-written `Bundle` impls (the Phase-19
    /// hierarchy newtypes, the `assert_impl_all!` pin stub, internal test
    /// stubs) inherit the retained byte path
    /// ([`Self::for_each_data_component_bytes`]) and are never asked for a
    /// typed write. The derive overrides it to
    /// `(n_fields <= MAX_TYPED_WRITE_ARITY)`. `SpawnBatchCommand::apply`
    /// branches on `if const { B::HAS_TYPED_WRITE }`, so for `false` bundles
    /// the typed arm — including the `write_row_typed` call — is compiled out
    /// (statically uncallable, never force-monomorphized; CONFIRM-1).
    const HAS_TYPED_WRITE: bool = false;

    /// Decision 4 (W3) — fills `out_perm[k]` for each **declaration** field
    /// `k` with the canonical data-column slot it writes
    /// ([`BundleColumnPtrs::PERM_SKIP`] for a ZST field), built **once per
    /// batch** from the canonical `data_component_ids` (the non-ZST
    /// `ComponentId`s in the same canonical order as
    /// [`Self::component_ids`] / `data_pool_ids`).
    ///
    /// `perm` is correct-by-construction: each non-ZST field's slot is the
    /// position of its `ComponentId` within `data_component_ids`, so two
    /// same-size, different-type components can never alias the wrong column
    /// (the IDENTITY guarantee the byte path enforces per-row at
    /// `spawn_batch_command.rs`). The default body is `unreachable!()` (never
    /// monomorphized — gated by `if const { HAS_TYPED_WRITE }` like
    /// [`Self::write_row_typed`]); the derive overrides it.
    ///
    /// `out_perm` must be at least the declaration-field count long; entries
    /// beyond the bundle's arity are left untouched (the caller pre-fills
    /// [`BundleColumnPtrs::PERM_SKIP`]).
    fn write_row_perm(data_component_ids: &[ComponentId], out_perm: &mut [u8])
    where
        Self: Sized,
    {
        let _ = (data_component_ids, out_perm);
        unreachable!(
            "Bundle::write_row_perm default body is statically uncallable: \
             only reached behind `if const {{ B::HAS_TYPED_WRITE }}`"
        )
    }

    /// Decision 4 — relocates every component of `self` into its destination
    /// column at `row`, using **fixed-width** `ptr::write::<Tk>` stores (no
    /// per-row sort, no runtime-length memcpy).
    ///
    /// The default body is `unreachable!()` and is never *executed*: it is
    /// codegen'd as the dead arm of `if const { false }` and eliminated, so it
    /// can never panic at runtime. `SpawnBatchCommand::apply` only reaches this
    /// method through `if const { B::HAS_TYPED_WRITE }`, which is `false` for every bundle
    /// that does not override it (CONFIRM-1). The derive overrides it with a
    /// declaration-field-unrolled body where each field `k`:
    ///
    /// * skips at compile time if `size_of::<Tk>() == 0` (ZST const-folds out);
    /// * otherwise `ptr::write::<Tk>(dst.column_base(dst.perm(k)).add(row *
    ///   size_of::<Tk>()) as *mut Tk, ManuallyDrop::take(&mut field_k))`.
    ///
    /// # Safety
    ///
    /// The caller must guarantee:
    ///
    /// 1. **Provenance / aliasing (W2, M3):** `dst` was built by
    ///    `ComponentPoolBundle::resolve_column_ptrs`
    ///    under a single `&mut` borrow of the pool bundle that has **ended**
    ///    before this call. Each `dst.column_base(..)` carries write-capable
    ///    provenance for the column's data sub-region and is **not** aliased by
    ///    any live reference. The caller must not re-borrow the archetype /
    ///    pool bundle between resolving `dst` and the last `write_row_typed`
    ///    call. This is sound on the `SpawnBatchCommand` path because its
    ///    `I: ... + 'static` bound forbids the iterator from capturing any
    ///    borrow of world/archetype/pools, and there is no TLS /
    ///    `DeferredEcsMaster` / hook / observer re-entry in this command, so
    ///    `iter.next()` cannot invalidate the resolved bases mid-loop. D4
    ///    NARROWS the TB surface versus the byte path (which re-borrows
    ///    `component_pools_mut()` every row).
    /// 2. **In-bounds (Q1):** `row < committed_rows` for every column the
    ///    bundle writes (the caller pre-grew capacity via
    ///    `Archetype::reserve_capacity`). Debug builds assert it per store.
    /// 3. **Column identity (W3):** `dst.perm(k)` selects the column whose
    ///    `ComponentId` equals `Tk::component_id()`. Debug builds assert it at
    ///    IDENTITY level per field.
    /// 4. **Disjoint, uninit, aligned slots:** the target slot is uninit (the
    ///    caller commits + tick-stamps the row only *after* the whole write
    ///    loop, B4), and `base.add(row * size_of::<Tk>())` is `Tk`-aligned by
    ///    the column-base alignment contract.
    ///
    /// # Panic safety (B4 / O1)
    ///
    /// Each field is relocated via `ManuallyDrop::take` (a bitwise move that
    /// suppresses the source `Drop`); `ptr::write` / `ManuallyDrop::take` are
    /// panic-free (no user code runs during the move). The only panic source
    /// on the batch path is `iter.next()` **between** rows — never inside
    /// `write_row_typed`. A completed row's fields are logically owned by the
    /// archetype (drop suppressed at the source); a not-yet-written row never
    /// had `write_row_typed` called → no double-drop, no half-row exposed
    /// (commit happens post-loop).
    unsafe fn write_row_typed(self, dst: &BundleColumnPtrs, row: usize)
    where
        Self: Sized,
    {
        let _ = (dst, row);
        unreachable!(
            "Bundle::write_row_typed default body is statically uncallable: \
             SpawnBatchCommand::apply gates it behind `if const \
             {{ B::HAS_TYPED_WRITE }}`, which is false for this bundle"
        )
    }

    /// Phase 22.1 D-E — invokes `f` once per **non-ZST** component in
    /// canonical order, eliding zero-size (ZST tag) columns from the
    /// per-row byte-copy walk.
    ///
    /// The spawn-batch write loop consumes this together with a per-batch
    /// compacted `data_pool_ids` (canonical `pool_ids` filtered by
    /// `layout.size() != 0`) so that a ZST tag column costs **zero**
    /// per-row instructions. Tick stamping (`Added<Tag>`) still happens
    /// for ALL columns via `fill_ticks_batch` — that walk is independent
    /// of this method.
    ///
    /// # Default body
    ///
    /// Forwards to [`Self::for_each_component_bytes`] and drops the
    /// empty-slice callbacks at runtime. This keeps hand-written `Bundle`
    /// impls (the Phase-19 hierarchy newtypes, internal test stubs)
    /// correct without per-impl codegen — they are off the gated bench and
    /// carry no ZST columns anyway. The derive macro **overrides** this
    /// with a const-filtered emission where ZST fields fold out at
    /// monomorphisation *before* the canonical sort.
    ///
    /// # Panic safety
    ///
    /// Identical to [`Self::for_each_component_bytes`] (**B4**): the
    /// callback observes only `ComponentId`s whose byte slice is non-empty;
    /// ManuallyDrop ownership transfer is handled inside the underlying
    /// emission.
    #[inline]
    fn for_each_data_component_bytes<F: FnMut(ComponentId, &[u8])>(self, mut f: F)
    where
        Self: Sized,
    {
        self.for_each_component_bytes(|id, bytes| {
            if !bytes.is_empty() {
                f(id, bytes);
            }
        });
    }
}
