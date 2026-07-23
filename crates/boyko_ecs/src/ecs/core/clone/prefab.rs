//! Prefab — a source-independent frozen template (std-lib S7, REDO).
//!
//! A [`Prefab`] is a captured `ChildOf` subtree built on the **audited clone
//! machinery** (`materialize`/`deep` — `clone_fn` per component, never a SerPod
//! byte-blit), so it round-trips EVERY `Clone` component including the non-`SerPod`
//! `Transform`/`GlobalTransform`. Capture once into an owned, type-erased byte
//! image; [`EcsMaster::instantiate`] it N times, each instance an independent
//! deep-clone-equivalent tree. The frozen template OWNS its bytes, so it survives
//! the source entity (and its whole subtree) being despawned.
//!
//! # Why clone-based, not blit-based
//!
//! The reverted S7 design captured only `SerPod` (plain-old-bytes) components, so
//! it SILENTLY DROPPED `Transform` (not `SerPod`) — every spatial gate failed. This
//! design routes each component through its registered `clone_fn`
//! ([`Cloneability::CloneViaFn`]) or a layout-driven memcpy
//! ([`Cloneability::TriviallyCopyable`]), exactly as the live clone path does, so it
//! is correct for all `Clone` components by construction. It also reuses
//! `deep::link_child` (Children rebuild) and `deep::remap_clone_child_of`
//! (ChildOf remap) VERBATIM rather than re-implementing them (Principle 0).
//!
//! # Cold capture-time structure (NOT a parallel hot-path data system)
//!
//! The owned template image ([`RawBlob`] + `Vec`s) is a **cold, capture-time**
//! structure: built once under `&mut EcsMaster`, read-only during instantiate. It is
//! NOT a per-frame side store of entity data — durable entity data still lives in
//! the kernel `ComponentPool`s; the prefab is an inert frozen snapshot used at
//! spawn-time only. This is the legitimate "transient/template" exception to
//! Principle 0, documented as such.
//!
//! # v1 boundary (parity with `clone_subtree`)
//!
//! * The instance ROOT is **detached** (no `ChildOf`): a frozen template has no live
//!   external parent to preserve across time/despawn (Decision 5). The caller
//!   parents the returned root as it wishes.
//! * Internal `ChildOf` is remapped to the fresh instance parents; `Children` is
//!   rebuilt via `link_child` (never byte-stored).
//! * Non-`ChildOf` `#[entities]` refs are kept **verbatim** (only `ChildOf` installs
//!   a `MapEntitiesFn` in v1 — the same boundary `clone_subtree` documents).
//! * Instance change-detection ticks are **reset** to the instantiate-time tick
//!   (instances are "Added now"); `cloner.preserve_ticks` is ignored by the prefab
//!   path (capture-time ticks are stale by instantiate). See [`EcsMaster::instantiate`].
//! * Dense / bitset (EnableTag) memberships are **not** captured (a divergence from
//!   `clone_subtree`, which re-materializes dense): a prefab of a dense-physics-body
//!   entity instantiates WITHOUT the dense membership. A capture-time
//!   `debug_assert!`-anchored diagnostic flags the divergence so it is never silent
//!   (Decision MINOR-1). Carrying dense is a v1.1 follow-up.

use std::alloc::{self, Layout};

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::clone::cloner::EntityCloner;
use crate::ecs::core::clone::deep::{
    link_child, remap_clone_child_of, remap_relink_generic_relations,
};
use crate::ecs::core::clone::map::EntityCloneMap;
use crate::ecs::core::clone::materialize::{
    CloneColumnSrc, CloneRowGuard, MAX_CLONE_COLUMNS, select_clone_ids, write_clone_column,
};
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, Cloneability, DropFn};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::{ChildOf, Children};
use crate::ecs::core::relationship::EvictionSuppressGuard;
use crate::ecs::identifiers::primitives::{ComponentId, EntityId};

/// Sentinel for "no parent" (the template root) in [`PrefabNode::parent`].
const PREFAB_NODE_NONE: u32 = u32::MAX;

/// How a captured component's value is reproduced into an instance row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    /// `Cloneability::TriviallyCopyable` — capture memcpys the source row bytes into
    /// the blob; instantiate memcpys blob→row (no `clone_fn`).
    CopyBytes,
    /// `Cloneability::CloneViaFn` — capture runs `clone_fn(src_row, blob_slot)`
    /// (the blob slot holds a LIVE owned `C`); instantiate runs
    /// `clone_fn(blob_slot, fresh_row)` (re-clone, leaving the blob `C` intact).
    CloneFnBytes,
    /// A require-closure id absent from the source — no blob bytes; instantiate
    /// reconstructs via the registry `required_ctor_for` (parity with materialize).
    Reconstruct,
}

/// One captured component record (flat, grouped per node via [`PrefabNode`]'s
/// `[comp_start, comp_start + comp_len)`).
#[derive(Clone, Copy)]
struct PrefabComponent {
    /// The component id (process-global; matches across `EcsMaster` instances).
    id: ComponentId,
    /// How instantiate reproduces this value.
    kind: CaptureKind,
    /// Byte offset into [`RawBlob`] (valid for `CopyBytes`/`CloneFnBytes`; unused for
    /// `Reconstruct`).
    blob_off: u32,
    /// Component size in bytes (clone_fn / memcpy / drop length).
    size: u32,
    /// Drop glue for the blob value, `None` iff `!needs_drop::<C>()` (so a `Copy`
    /// component is never dropped). `Reconstruct` records `None` (no blob value).
    drop_fn: Option<DropFn>,
}

/// One captured node of the template subtree (BFS order; node 0 = root).
struct PrefabNode {
    /// Node index of this node's ChildOf parent WITHIN the template, or
    /// [`PREFAB_NODE_NONE`] for the root / a parent outside the captured subtree.
    parent: u32,
    /// Index into [`Prefab::components`] of this node's first component.
    comp_start: u32,
    /// Number of components for this node.
    comp_len: u32,
    /// The SOURCE parent `Entity` recorded at capture (this node's `ChildOf`
    /// target), used at instantiate to populate the source→instance
    /// [`EntityCloneMap`] so [`remap_clone_child_of`] works UNCHANGED (Decision 5).
    /// Only meaningful when `has_src_parent`.
    src_parent: Entity,
    /// Whether `src_parent` is set (the root has no captured `ChildOf` — Decision 5,
    /// and a top-level captured node may have an external parent dropped at capture).
    has_src_parent: bool,
    /// This node's OWN source `Entity` recorded at capture. Used at instantiate to
    /// populate the per-node source→instance [`EntityCloneMap`] (`src_entity` →
    /// instance) so [`remap_relink_generic_relations`] remaps every in-subtree generic
    /// relation FK to its instance and relinks the reverse index — fixing the prefab
    /// half-edge bug (FK present, target's reverse collection missing the instance).
    /// A pure value key (never dereferenced against the world at instantiate), so the
    /// prefab stays source-independent: it only matches the verbatim-copied FK target
    /// values, which came from the same capture-time source entities.
    src_entity: Entity,
}

/// One owned, max-aligned, growable byte region holding all captured component
/// values. NOT a `Vec<u8>` (a `Vec`'s alignment is 1; component values need their
/// natural alignment). Hand-managed via the global allocator: alloc/realloc on
/// grow, dealloc in [`Prefab::drop`] after every live value has been dropped.
struct RawBlob {
    /// Allocation base; null when `cap == 0`.
    ptr: *mut u8,
    /// Bytes used (the bump cursor).
    len: usize,
    /// Bytes allocated.
    cap: usize,
    /// Current allocation alignment (the max component alignment seen so far). A
    /// power of two; 1 when empty.
    align: usize,
}

impl RawBlob {
    /// An empty blob (no allocation).
    #[inline]
    fn new() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            cap: 0,
            align: 1,
        }
    }

    /// Reserves a slot of `size` bytes aligned to `align`, growing the allocation if
    /// needed, and returns the byte offset of the slot (the bytes are uninitialized
    /// — the caller writes them via memcpy or `clone_fn`). The returned offset
    /// satisfies `base.add(off)` aligned to `align`.
    ///
    /// On grow, if the new max alignment exceeds the current allocation alignment the
    /// region is RE-ALLOCATED with the larger alignment (relocating live values by
    /// byte copy — sound: a Rust value is always trivially relocatable, and the
    /// derived pointers are stored as OFFSETS, recomputed from the fresh base —
    /// Decision MAJOR-2). The grow happens BEFORE the slot pointer is taken, so no
    /// `*mut u8` into the old allocation is ever held across a `realloc`.
    fn push_aligned(&mut self, size: usize, align: usize) -> u32 {
        debug_assert!(align.is_power_of_two(), "component align must be power-of-two");
        // Padded start offset for this slot.
        let off = self.len.next_multiple_of(align);
        let needed = off + size;
        let new_align = self.align.max(align);
        // Grow when more capacity is needed, the alignment increased, OR the blob has
        // never allocated (cap == 0). The last case forces a non-null base even for an
        // all-ZST (size-0) capture, so `slot_ptr` NEVER returns null — `clone_fn` /
        // `copy_nonoverlapping` / `from_raw_parts` then always see a valid (non-null,
        // aligned) pointer, even for a zero-byte slot (a `copy_nonoverlapping(_, _, 0)`
        // into null would otherwise be UB).
        if needed > self.cap || new_align > self.align || self.ptr.is_null() {
            self.grow(needed.max(1), new_align);
        }
        debug_assert!(off + size <= self.cap, "blob slot out of allocation");
        self.len = off + size;
        debug_assert!(
            (self.ptr as usize + off).is_multiple_of(align),
            "blob slot misaligned"
        );
        off as u32
    }

    /// Grows the allocation to at least `min_cap` bytes with alignment `new_align`,
    /// preserving the existing `len` bytes. `new_align >= self.align`.
    #[cold]
    fn grow(&mut self, min_cap: usize, new_align: usize) {
        // Double-or-min growth (amortized O(1) appends), aligned up to `new_align` so
        // the allocation size is a multiple of the alignment (a `Layout` requirement
        // satisfied automatically here, but kept explicit).
        let mut new_cap = (self.cap * 2).max(min_cap).max(64);
        new_cap = new_cap.next_multiple_of(new_align);

        // SAFETY (Decision MAJOR-2 / unsafe #5):
        //   * `new_align` is a power of two (max of two power-of-two component aligns,
        //     or the initial 1), and `new_cap > 0`, so `Layout::from_size_align` is
        //     valid; we `expect` (abort on the impossible overflow case).
        //   * When the alignment is unchanged AND `self.ptr` is non-null, `realloc`
        //     preserves the existing `len` bytes and may relocate; the relocation is
        //     a byte copy of trivially-relocatable Rust values, and every derived
        //     pointer is recomputed from the fresh base via stored OFFSETS — no stale
        //     pointer survives.
        //   * When the alignment INCREASES, `realloc` cannot honor a stricter align,
        //     so we `alloc` a fresh region, byte-copy the live `len` bytes, and free
        //     the old one (same trivial-relocation soundness).
        //   * On allocation failure we call `handle_alloc_error` (abort), never return
        //     a null `ptr` for a non-zero cap.
        unsafe {
            let new_layout = Layout::from_size_align(new_cap, new_align)
                .expect("invariant: blob layout (power-of-two align, in-range size)");
            let new_ptr = if self.ptr.is_null() {
                alloc::alloc(new_layout)
            } else if new_align == self.align {
                let old_layout = Layout::from_size_align(self.cap, self.align)
                    .expect("invariant: prior blob layout was valid");
                alloc::realloc(self.ptr, old_layout, new_cap)
            } else {
                // Alignment increased — allocate fresh, copy, free old.
                let fresh = alloc::alloc(new_layout);
                if fresh.is_null() {
                    alloc::handle_alloc_error(new_layout);
                }
                std::ptr::copy_nonoverlapping(self.ptr, fresh, self.len);
                let old_layout = Layout::from_size_align(self.cap, self.align)
                    .expect("invariant: prior blob layout was valid");
                alloc::dealloc(self.ptr, old_layout);
                fresh
            };
            if new_ptr.is_null() {
                alloc::handle_alloc_error(new_layout);
            }
            self.ptr = new_ptr;
            self.cap = new_cap;
            self.align = new_align;
        }
    }

    /// `*mut u8` to the slot at `off` (an uninitialized slot just reserved, or a live
    /// value written earlier). `off` came from [`Self::push_aligned`].
    ///
    /// # Safety
    /// `off < self.len` (a reserved/written offset); the returned pointer is valid
    /// for the slot's `size` bytes and aligned for the component (guaranteed by
    /// `push_aligned`). The blob outlives the use.
    #[inline]
    unsafe fn slot_ptr(&self, off: u32) -> *mut u8 {
        debug_assert!((off as usize) <= self.len, "blob offset past cursor");
        // SAFETY: `off <= len <= cap`; `ptr` is the live allocation base (non-null
        //   whenever any slot was reserved). The caller guarantees the slot's bytes
        //   are valid for its component.
        unsafe { self.ptr.add(off as usize) }
    }
}

/// A captured, source-independent, frozen template of a `ChildOf` subtree.
///
/// Built once via [`EcsMaster::capture_prefab`] / [`EcsMaster::capture_prefab_with`],
/// instantiated N times via [`EcsMaster::instantiate`]. Owns its component bytes, so
/// it survives the source entity being despawned. Each instantiate yields an
/// independent deep copy (re-runs `clone_fn` from the template) with a **detached**
/// root.
///
/// `!Send + !Sync` (v1): it owns a raw `*mut u8` blob holding arbitrary `C` values,
/// built and instantiated on the world's thread (matches the engine's `Arena`
/// `!Send`-by-design stance). The raw pointer makes it auto-`!Send`/`!Sync` already;
/// the marker is implied by the `*mut u8` field, no explicit `impl` needed.
pub struct Prefab {
    /// BFS order; index 0 is the root. `parent` is a NODE INDEX (or
    /// [`PREFAB_NODE_NONE`]). Setup-time `Vec` (cold).
    nodes: Vec<PrefabNode>,
    /// Flat per-component records, grouped per node by `[comp_start, comp_start +
    /// comp_len)`.
    components: Vec<PrefabComponent>,
    /// One owned, growable, max-aligned byte region holding every
    /// `CopyBytes`/`CloneFnBytes` value (a `CloneFnBytes` slot holds a LIVE `C`).
    blob: RawBlob,
    /// Diagnostic anchor: the captured root carried a `ChildOf` (the instance root
    /// is detached regardless — Decision 5).
    root_had_child_of: bool,
}

impl Prefab {
    /// Number of captured nodes (subtree size). The root counts as one.
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// `true` iff the captured root carried a `ChildOf` at capture time (the instance
    /// root is detached regardless — Decision 5). Diagnostic / gate anchor.
    #[inline]
    pub fn root_had_child_of(&self) -> bool {
        self.root_had_child_of
    }
}

impl Drop for Prefab {
    fn drop(&mut self) {
        // Drop each live blob value exactly once via its registered `drop_fn`
        // (Decision 7 / unsafe #6), then free the blob allocation. Instance rows own
        // their OWN clones (dropped by the world); the blob owns its one `C` per
        // `CloneFnBytes`/drop-needing slot, dropped here.
        for comp in &self.components {
            // `Reconstruct` has no blob value; a `Copy` component records `drop_fn ==
            // None`. Only drop when there is a value AND it needs dropping.
            if comp.kind == CaptureKind::Reconstruct {
                continue;
            }
            if let Some(drop_fn) = comp.drop_fn {
                // SAFETY (Decision 7 / unsafe #6):
                //   * This slot holds exactly one live, initialized `C`: capture ran
                //     `clone_fn`/memcpy into it and instantiate only ever READS it
                //     (`clone_fn` forms `&C`, never moves out), so it is still valid.
                //   * `slot_ptr(blob_off)` is aligned for `C` and valid for its
                //     `size` bytes (`push_aligned` guaranteed it).
                //   * `drop_fn` (= `drop_in_place_glue::<C>`) runs `C`'s Drop ONCE;
                //     each slot has exactly one `PrefabComponent`, so no double-drop.
                unsafe {
                    let slot = self.blob.slot_ptr(comp.blob_off);
                    drop_fn(slot);
                }
            }
        }
        if !self.blob.ptr.is_null() {
            // SAFETY: `ptr`/`cap`/`align` describe the live allocation from the last
            //   `grow`; the layout matches what was allocated; every contained value
            //   was just dropped, so freeing the raw bytes leaks/double-frees nothing.
            unsafe {
                let layout = Layout::from_size_align(self.blob.cap, self.blob.align)
                    .expect("invariant: blob layout was valid at allocation");
                alloc::dealloc(self.blob.ptr, layout);
            }
        }
    }
}

/// RAII rollback guard that OWNS the in-progress capture image (Decision MAJOR-1 /
/// unsafe #1).
///
/// Capture builds DIRECTLY into this guard's `blob` + `committed` fields. On a
/// panicking user `Clone::clone` mid-capture, `Drop` drops the ALREADY-COMMITTED
/// blob values (each via its `drop_fn`) and frees the partial allocation — but NOT
/// the in-flight slot whose `clone_fn` panicked (a `PrefabComponent` is pushed only
/// AFTER its value is fully written, so the panicking slot is never in `committed`).
/// On success capture takes ownership of `blob`/`committed` via [`Self::into_image`]
/// (which disarms), after which the finished [`Prefab`] owns the drop/free.
///
/// Owning (not `&mut`-borrowing) the fields is what makes the borrow checker accept
/// "push to `committed` while the guard can still roll it back" — the guard IS the
/// mutable owner during capture.
struct BlobGuard {
    blob: RawBlob,
    committed: Vec<PrefabComponent>,
    /// `true` until [`Self::into_image`] disarms (the finished `Prefab` then owns
    /// the drop/free).
    armed: bool,
}

impl BlobGuard {
    #[inline]
    fn new() -> Self {
        Self {
            blob: RawBlob::new(),
            committed: Vec::new(),
            armed: true,
        }
    }

    /// Disarms and yields the finished `(blob, components)` image. Called once capture
    /// completes without unwinding; after this `Drop` is a no-op (the `Prefab` owns
    /// the values).
    #[inline]
    fn into_image(mut self) -> (RawBlob, Vec<PrefabComponent>) {
        self.armed = false;
        let blob = std::mem::replace(&mut self.blob, RawBlob::new());
        let components = std::mem::take(&mut self.committed);
        (blob, components)
    }
}

impl Drop for BlobGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY (MAJOR-1 / unsafe #1 rollback):
        //   * Every entry in `committed` was pushed only AFTER its blob value was
        //     fully written (memcpy / `clone_fn` returned), so each
        //     `CloneFnBytes`/drop-needing slot holds a live `C`. The panicking slot's
        //     `PrefabComponent` was NOT pushed (capture pushes after the write), so it
        //     is excluded — no drop of an uninit slot.
        //   * Each `drop_fn` runs the value's Drop ONCE (one entry per slot).
        //   * After dropping, the partial allocation is freed with its current layout.
        for comp in self.committed.iter() {
            if comp.kind == CaptureKind::Reconstruct {
                continue;
            }
            if let Some(drop_fn) = comp.drop_fn {
                unsafe {
                    let slot = self.blob.slot_ptr(comp.blob_off);
                    drop_fn(slot);
                }
            }
        }
        if !self.blob.ptr.is_null() {
            unsafe {
                let layout = Layout::from_size_align(self.blob.cap, self.blob.align)
                    .expect("invariant: blob layout valid during capture");
                alloc::dealloc(self.blob.ptr, layout);
            }
        }
    }
}

/// Inline worklist capacity before spilling to the heap (mirrors
/// `deep.rs`'s `DEEP_WORKLIST_INLINE`).
const CAPTURE_WORKLIST_INLINE: usize = 32;

/// Captures `source` and its `ChildOf` subtree into a frozen [`Prefab`] per
/// `cloner` (S7 capture backend — Model B owned `clone_fn` image).
///
/// Reuses the deep-clone walk shape from `clone_subtree_inner` (worklist +
/// snapshot-children-by-value + dedup) but materializes each node's components into
/// owned blob bytes (via the SAME id-selection `select_clone_ids`) instead of a world
/// row. The caller has already checked `source` is alive.
pub(crate) fn capture(world: &mut EcsMaster, source: Entity, cloner: &EntityCloner) -> Prefab {
    debug_assert!(
        world.has_entity(source),
        "capture: source must be alive (caller checks has_entity)"
    );
    // Force deep semantics: a prefab is always a subtree (the per-node selection runs
    // shallow because the walk drives the recursion explicitly).
    let mut node_cloner = *cloner;
    node_cloner.force_shallow();

    let child_of_id = ChildOf::component_id();

    // MINOR-1: flag (never silently drop) a source carrying dense memberships — the
    // v1 prefab does not capture them (a divergence from `clone_subtree`).
    debug_warn_dense_divergence(world);

    let mut nodes: Vec<PrefabNode> = Vec::with_capacity(CAPTURE_WORKLIST_INLINE);
    let mut root_had_child_of = false;

    // source EntityId.0 → node index (diamond dedup + parent resolution).
    let mut node_of: boyko_utils::sparse_map::sparse_map::SparseMap<u32> =
        boyko_utils::sparse_map::sparse_map::SparseMap::with_capacity(CAPTURE_WORKLIST_INLINE);
    // Worklist of SOURCE entities still to capture (BFS, mirrors deep.rs).
    let mut worklist: Vec<Entity> = Vec::with_capacity(CAPTURE_WORKLIST_INLINE);
    worklist.push(source);

    // The rollback guard OWNS the in-progress blob + committed components (Decision
    // MAJOR-1): capture builds directly into it, so a panicking user `Clone::clone`
    // below unwinds through `BlobGuard::drop`, dropping committed values + freeing the
    // partial allocation. `guard.committed` IS the final `components` Vec.
    let mut guard = BlobGuard::new();

    let mut node_guard = 0usize;
    while let Some(src) = worklist.pop() {
        if node_of.contains(src.id().0) {
            continue; // diamond dedup
        }
        debug_assert!(
            node_guard < crate::ecs::core::clone::MAX_CLONE_SUBTREE_NODES,
            "capture: subtree node cap exceeded (a ChildOf cycle?)"
        );
        node_guard += 1;

        let node_index = nodes.len() as u32;
        node_of.insert(src.id().0, node_index);
        let is_root = node_index == 0;

        // Resolve the source's parent (ChildOf target) — recorded for the instantiate
        // remap. The root's ChildOf is dropped (detached instance root, Decision 5).
        // `src_parent` is only meaningful when `has_src_parent`; the unused-default is
        // an inert placeholder Entity.
        let placeholder = Entity::with_id(EntityId(0));
        let (src_parent, has_src_parent) = match world.get_component::<ChildOf>(src) {
            Some(child_of) if !is_root => (child_of.0, true),
            Some(_) => {
                root_had_child_of = true;
                (placeholder, false)
            }
            None => (placeholder, false),
        };

        // Resolve the source archetype pointer + row (generation-checked) the SAME way
        // `materialize_clone_into` does — re-resolved per node (W6, no caching across
        // the structural-free capture).
        let source_inland = world.entity_master.entities_inland[src.id().0];
        debug_assert!(
            !source_inland.is_null() && source_inland.generation() == src.generation(),
            "capture: source node must be alive"
        );
        let source_ptr: *mut Archetype = source_inland.archetype_ptr();
        let source_row = source_inland.unit_index() as usize;

        // Build the id set via the SHARED selector (byte-identical to the live clone;
        // exclude ChildOf for the root so the instance root is detached — Decision 5).
        let extra_excluded = if is_root { Some(child_of_id) } else { None };
        let selected = select_clone_ids(source_ptr, &node_cloner, extra_excluded);

        let comp_start = guard.committed.len() as u32;
        for &id in &selected.ids[..selected.len] {
            if selected.copy_from_source.contains(id) {
                let info = component_registry::get_clone_info(id.0)
                    .expect("invariant: copy_from_source id was classified cloneable");
                // SAFETY: `source_ptr` is live, stable slab provenance (source node is
                //   alive); the shared `&Archetype` view is scoped to reading the pool
                //   metadata + row pointer. `source_row < pool.count()` (live row).
                let (src_ptr, size, align) = unsafe {
                    let src_arch: &Archetype = &*source_ptr;
                    let src_pool = src_arch
                        .component_pools()
                        .get_pool(id)
                        .expect("invariant: copy_from_source id exists in source");
                    debug_assert!(source_row < src_pool.count(), "capture: row OOB");
                    let layout = src_pool.component_layout();
                    (src_pool.unit_ptr(source_row), layout.size(), layout.align())
                };
                // Grow the blob (BEFORE taking the slot pointer — MAJOR-2: no `*mut u8`
                // into the old allocation survives a realloc).
                let blob_off = guard.blob.push_aligned(size, align);
                let drop_fn = component_registry::get_layout(id.0).and_then(|l| l.drop_fn);
                let kind = match info.cloneability {
                    Cloneability::TriviallyCopyable => {
                        // SAFETY (unsafe #2): `src_ptr` is readable for `size` bytes
                        //   (live source row); the blob slot at `blob_off` is the
                        //   freshly-reserved uninit `size`-byte region aligned per the
                        //   component; disjoint (source pool vs owned blob).
                        unsafe {
                            let dst = guard.blob.slot_ptr(blob_off);
                            std::ptr::copy_nonoverlapping(src_ptr, dst, size);
                        }
                        CaptureKind::CopyBytes
                    }
                    Cloneability::CloneViaFn => {
                        let clone_fn = info.clone_fn.expect(
                            "invariant: CloneViaFn installs Some(clone_via_clone::<C>)",
                        );
                        // SAFETY (unsafe #1): `src_ptr` is a live, aligned, initialized
                        //   `C` (source row); the blob slot at `blob_off` is the
                        //   freshly-reserved uninit region aligned >= align_of::<C>();
                        //   disjoint (source pool vs owned blob). `clone_fn` forms `&C`,
                        //   `ptr::write`s the clone into the uninit dst (no drop of
                        //   uninit). On a `clone_fn` PANIC the dst is left uninit and
                        //   the `PrefabComponent` below is NOT pushed (the push follows
                        //   this block), so `BlobGuard` excludes this slot on unwind —
                        //   no drop of an uninit slot. Single-threaded `&mut World`.
                        unsafe {
                            let dst = guard.blob.slot_ptr(blob_off);
                            clone_fn(src_ptr, dst);
                        }
                        CaptureKind::CloneFnBytes
                    }
                    Cloneability::Ignore => {
                        unreachable!("copy_from_source only holds cloneable ids")
                    }
                };
                // Commit AFTER the value is fully written (so the panicking slot is
                // never recorded — BlobGuard MAJOR-1 contract).
                debug_assert!(
                    guard.committed.len() < u32::MAX as usize,
                    "capture: component count overflow"
                );
                guard.committed.push(PrefabComponent {
                    id,
                    kind,
                    blob_off,
                    size: size as u32,
                    drop_fn,
                });
            } else {
                // Reconstruct: a require-closure id absent from the source — no blob
                // bytes (instantiate builds it via `required_ctor_for`).
                guard.committed.push(PrefabComponent {
                    id,
                    kind: CaptureKind::Reconstruct,
                    blob_off: 0,
                    size: 0,
                    drop_fn: None,
                });
            }
        }
        let comp_len = guard.committed.len() as u32 - comp_start;

        nodes.push(PrefabNode {
            parent: PREFAB_NODE_NONE, // patched below once all nodes are indexed
            comp_start,
            comp_len,
            src_parent,
            has_src_parent,
            src_entity: src,
        });

        // W6: SNAPSHOT this node's children BY VALUE before any further push.
        let mut snapshot: Vec<Entity> = Vec::new();
        if let Some(children) = world.get_component::<Children>(src) {
            snapshot.extend_from_slice(children.as_slice());
        }
        for child in snapshot {
            if !node_of.contains(child.id().0) {
                worklist.push(child);
            }
        }
    }

    // Patch each node's `parent` NODE INDEX from its recorded source-parent entity.
    // A parent outside the captured subtree (the root's external parent) stays
    // `PREFAB_NODE_NONE`.
    for node in &mut nodes {
        if node.has_src_parent
            && let Some(&pidx) = node_of.get(node.src_parent.id().0)
        {
            node.parent = pidx;
        }
    }

    // Capture completed without unwinding — take ownership of the blob/components
    // from the rollback guard (disarms it; the finished `Prefab` now owns drop/free).
    let (blob, components) = guard.into_image();

    Prefab {
        nodes,
        components,
        blob,
        root_had_child_of,
    }
}

/// Cold MINOR-1 diagnostic: a v1 prefab captures TABLE columns only — it does NOT
/// capture dense memberships (a documented divergence from `clone_subtree`, which
/// re-materializes dense). Flagged here in debug so the divergence is never silent
/// (the prior reverted S7 failed by SILENTLY dropping components). A real warning
/// hook (log / counter) can replace the marker call later; production is a no-op.
#[cold]
#[inline(never)]
fn debug_warn_dense_divergence(world: &EcsMaster) {
    #[cfg(debug_assertions)]
    if !world.dense_registry.is_empty() {
        // Breakpoint anchor: this world has dense stores whose memberships the v1
        // prefab will NOT carry into instances (Decision MINOR-1). Not an error.
        let _divergence = "prefab v1 does not capture dense memberships";
    }
    let _ = world;
}

/// Instantiates `prefab` into `world`, returning the (detached) instance root (S7
/// instantiate backend). Each call yields an independent deep copy.
///
/// Nodes are stored so that every node's PARENT has a smaller node index (the capture
/// worklist pops a parent before pushing its children, mirroring `clone_subtree_inner`
/// — DFS/LIFO, but the parent-before-child index ordering is the load-bearing
/// invariant). Iterating in index order therefore materializes a parent before its
/// children, so `instance_of[node.parent]` is always already filled. Per node it
/// computes the target archetype from the node's component id set, reserves a row
/// under a [`CloneRowGuard`], writes each component via the SHARED
/// [`write_clone_column`] (the verbatim materialize three-branch writer, src = blob),
/// then runs the VERBATIM `remap_clone_child_of` / `link_child` pass from
/// `clone_subtree_inner`.
pub(crate) fn instantiate(world: &mut EcsMaster, prefab: &Prefab) -> Entity {
    debug_assert!(!prefab.nodes.is_empty(), "instantiate: empty prefab");
    let current_tick = world.current_tick();
    let child_of_id = ChildOf::component_id();
    let children_id = Children::component_id();

    // node index → instance Entity (BFS, so a parent is filled before its children).
    let mut instance_of: Vec<Entity> = Vec::with_capacity(prefab.nodes.len());

    for (node_index, node) in prefab.nodes.iter().enumerate() {
        let comp_start = node.comp_start as usize;
        let comp_len = node.comp_len as usize;
        let comps = &prefab.components[comp_start..comp_start + comp_len];

        // Gather the node's (already canonical-sorted at capture) id set.
        let mut ids: [ComponentId; MAX_CLONE_COLUMNS] = [ComponentId(0); MAX_CLONE_COLUMNS];
        debug_assert!(comp_len <= MAX_CLONE_COLUMNS, "instantiate: id set overflow");
        for (i, comp) in comps.iter().enumerate() {
            ids[i] = comp.id;
        }
        let target_archetype_id = world.get_or_create_archetype(&ids[..comp_len]);
        let entity = world.entity_master.allocate_entity();

        // Resolve the (possibly grown) target archetype pointer.
        let target_ptr = world
            .archetype_master_mut()
            .archetype_ptr_for(target_archetype_id)
            .expect("invariant: target archetype just resolved");

        let new_row: usize;
        {
            // SAFETY: `target_ptr` is write-capable, interior-mutable slab provenance
            //   under `&mut EcsMaster`; this reborrow is confined to the reserve call.
            new_row = unsafe {
                let target: &mut Archetype = &mut *target_ptr;
                target.reserve_capacity(1).expect(
                    "instantiate: target pool reserve ceiling exhausted (grows on demand)",
                );
                target.current_index
            };

            let mut guard = CloneRowGuard::new(target_ptr, new_row);
            for comp in comps {
                let src = match comp.kind {
                    CaptureKind::CopyBytes => {
                        // SAFETY: `comp.blob_off` is a live, written slot (capture
                        //   memcpy'd `size` bytes); `slot_ptr` is aligned + valid for
                        //   `size` bytes.
                        let src_ptr = unsafe { prefab.blob.slot_ptr(comp.blob_off) };
                        CloneColumnSrc::CopyBytes(src_ptr)
                    }
                    CaptureKind::CloneFnBytes => {
                        let clone_fn = component_registry::get_clone_info(comp.id.0)
                            .and_then(|i| i.clone_fn)
                            .expect("invariant: CloneFnBytes id has a clone_fn");
                        // SAFETY: `comp.blob_off` holds a LIVE `C` (capture's `clone_fn`
                        //   wrote it; instantiate only READS it). `clone_fn` forms `&C`
                        //   and `ptr::write`s a fresh clone into the target row, leaving
                        //   the blob `C` intact (so a later instantiate re-clones it,
                        //   and `Prefab::drop` drops it exactly once).
                        let src_ptr = unsafe { prefab.blob.slot_ptr(comp.blob_off) };
                        CloneColumnSrc::CloneFn { clone_fn, src_ptr }
                    }
                    CaptureKind::Reconstruct => {
                        // Resolve the require ctor against this node's CLONED set (the
                        // copy_from_source comps), exactly as materialize does.
                        let ctor = required_ctor_for_node(comps, comp.id).expect(
                            "invariant: a Reconstruct id came from the require closure, \
                             so a ctor exists",
                        );
                        CloneColumnSrc::Reconstruct(ctor)
                    }
                };
                // SAFETY (unsafe #3/#4): `target_ptr` is live write-capable slab
                //   provenance; `new_row < committed_rows` (reserved above); `comp.id`
                //   is a table id hosted by the target; for Copy/CloneFn the blob
                //   `src_ptr` is a live value readable for `comp.size` bytes, DISJOINT
                //   from the target row (owned blob vs pool row); `comp.size` ==
                //   target pool size. NO `&mut Archetype` is live across the (blob)
                //   `clone_fn` call (the helper drops it before), so the guard's Drop
                //   is the sole `&mut Archetype` accessor on unwind. Ticks reset to
                //   `current_tick` (instances are "Added now" — preserve_ticks ignored).
                unsafe {
                    write_clone_column(
                        target_ptr,
                        new_row,
                        comp.id,
                        comp.size as usize,
                        src,
                        current_tick,
                        current_tick,
                    );
                }
                guard.note_committed(comp.id);
            }

            // Advance archetype bookkeeping, then disarm (the materialize tail).
            // SAFETY: `target_ptr` is write-capable slab provenance; `&mut` confined;
            //   neither op can panic.
            unsafe {
                let target: &mut Archetype = &mut *target_ptr;
                target.entity_ids.push(entity.id());
                target.current_index = new_row + 1;
            }
            guard.disarm();
        }

        // Commit the entity→inland mapping LAST (W5): only after the row is fully
        // materialized is the entity mapped.
        world
            .entity_master
            .register_entity_with_ptr(entity, target_ptr, new_row as u32);

        debug_assert_eq!(
            instance_of.len(),
            node_index,
            "instantiate: BFS order — parent materialized before child"
        );
        instance_of.push(entity);
    }

    // ── Remap + link pass (VERBATIM clone_subtree_inner tail) ───────────────
    // Build the per-node source→instance map, keyed IDENTICALLY to the deep-clone map
    // (`clone_subtree_inner` inserts `src → clone` for EVERY cloned node): every captured
    // node's own source `Entity` → its instance. This is a strict superset of the prior
    // ChildOf-only map (a child's `src_parent` IS its parent node's `src_entity`, so the
    // verbatim `remap_clone_child_of` still resolves `childof.target() → parent_instance`),
    // and it additionally lets `remap_relink_generic_relations` translate any in-subtree
    // generic relation FK target to its instance (a target outside the subtree is absent
    // from the map → kept verbatim, then relinked-or-detached per the v1.1 rules).
    let Some(remap_fn) = component_registry::get_map_entities_fn(child_of_id.0) else {
        debug_assert!(false, "instantiate: ChildOf map_entities_fn not installed");
        return instance_of[0];
    };

    let mut map = EntityCloneMap::new();
    for (node_index, node) in prefab.nodes.iter().enumerate() {
        map.insert(node.src_entity, instance_of[node_index]);
    }

    // Suppress 1:1 eviction for the relink phase (Relations v1.1, C3), identical to
    // the `clone_subtree` relink guard. NOW LOAD-BEARING: the loop below routes every
    // node's generic-relation FKs through `remap_relink_generic_relations`, so an
    // instance whose FK points at an EXTERNAL `Exclusive` 1:1 target must DETACH (drop
    // its own dangling FK) instead of evicting that unrelated target's existing source.
    // No effect on the `Vec` one-to-many path (eviction never triggers) or on
    // `ChildOf`/`Children` (a `Vec` collection).
    let eviction_suppress = EvictionSuppressGuard::enter();

    for (node_index, node) in prefab.nodes.iter().enumerate() {
        let instance = instance_of[node_index];

        // ── (1) ChildOf / Children — UNCHANGED (Decision 5 + Phase-19 regression gate).
        // Skip the root / externally-parented node (no captured ChildOf to remap).
        if node.parent != PREFAB_NODE_NONE {
            let remapped_parent =
                remap_clone_child_of(world, instance, child_of_id, remap_fn, &map);
            if let Some(parent_clone) = remapped_parent
                && parent_clone != instance
            {
                link_child(world, parent_clone, instance, children_id);
            }
        }

        // ── (2) GENERIC relation FKs — fixes the prefab half-edge bug. Reuses the ONE
        // deep-clone relink body (Principle 0): remaps every NON-ChildOf relationship
        // source FK on the instance (in-subtree target → its instance via `map`,
        // external target → verbatim) and relinks the instance into the target's reverse
        // index (external `Exclusive` → detach under the eviction-suppress guard). Runs
        // for EVERY node including the root (a root instance may carry a generic FK).
        remap_relink_generic_relations(world, instance, child_of_id, &map);
    }

    // Re-enable 1:1 eviction before returning (any deferred detach removes run with
    // normal eviction semantics — they only clear FKs).
    drop(eviction_suppress);

    instance_of[0]
}

/// Resolves the require ctor for a `Reconstruct` `id` against this node's CLONED set
/// (the `copy_from_source` comps, i.e. the non-`Reconstruct` entries). Mirrors
/// `materialize`'s `required_ctor_for(cloned_set, id)` — the cloned set is exactly the
/// `CopyBytes`/`CloneFnBytes` ids of this node.
fn required_ctor_for_node(
    comps: &[PrefabComponent],
    id: ComponentId,
) -> Option<component_registry::RequiredCtor> {
    // Build the cloned set (non-Reconstruct ids) on the stack — small (≤ node arity).
    let mut cloned: [ComponentId; MAX_CLONE_COLUMNS] = [ComponentId(0); MAX_CLONE_COLUMNS];
    let mut n = 0usize;
    for comp in comps {
        if comp.kind != CaptureKind::Reconstruct {
            debug_assert!(n < MAX_CLONE_COLUMNS, "instantiate: cloned set overflow");
            cloned[n] = comp.id;
            n += 1;
        }
    }
    component_registry::required_ctor_for(&cloned[..n], id)
}
