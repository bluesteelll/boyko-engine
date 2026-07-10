//! [`Assets<T>`] — the world-global, per-asset-type storage table (rung A0).

use crate::ecs::core::asset::asset::{Asset, AssetLoadState};
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::asset::slot::Slot;
use crate::ecs::core::resources::resource::{NonSendResource, Resource};
use crate::ecs::core::resources::resource_type_registry::resource_id_for;
use crate::ecs::identifiers::primitives::ResourceId;

/// A world-global, per-asset-type storage table — registered as a
/// [`Resource`] via [`resource_id_for`] (the same generic-registry pattern
/// [`State<S>`](crate::ecs::core::state::State) uses; see that type's doc for
/// the rust#22991 rationale for NOT using a per-impl `static`).
///
/// # Storage
///
/// Four parallel `Vec`s indexed by the same `usize` row:
/// - `records` — the row's value ([`Slot::Occupied`]) or its vacancy marker
///   ([`Slot::Vacant`]); Rust's own `Drop` only ever touches `Occupied`
///   payloads, so removal and table teardown are correct with ZERO `unsafe`.
/// - `generations` — bumped on [`remove`](Self::remove), so a [`Handle`]
///   minted before a row was freed is rejected after reuse.
/// - `free` — the LIFO stack of vacated row indices `add` reuses before
///   ever growing `records`.
/// - `states` — the row's [`AssetLoadState`], read independently of the
///   value (a `Failed` row's `T` may be a placeholder, not a usable asset).
///
/// `live` is a running count (bumped/decremented in `add`/`remove`) so
/// [`len`](Self::len) is O(1) — it feeds the future material-table clamp,
/// which cannot afford an O(n) scan per frame.
///
/// # Render-carrier reuse caveat
///
/// See [`Handle`]'s doc: reusing a freed slot is unsound for
/// render-referenced assets until a later rung carries the generation into
/// the render path. `remove` is implemented and tested at the host level
/// (this rung's scope), but treat render-visible tables as append-only
/// until that rung lands.
pub struct Assets<T: Asset> {
    records: Vec<Slot<T>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    states: Vec<AssetLoadState>,
    live: usize,
    dirty_gen: u64,
}

impl<T: Asset> Assets<T> {
    /// Creates an empty table with `cap` rows pre-reserved (no rows
    /// initialized — this only avoids the first few `Vec` growth copies).
    #[inline]
    pub fn with_reserved(cap: usize) -> Self {
        Self {
            records: Vec::with_capacity(cap),
            generations: Vec::with_capacity(cap),
            free: Vec::new(),
            states: Vec::with_capacity(cap),
            live: 0,
            dirty_gen: 0,
        }
    }

    /// Inserts `value`, reusing a freed row (LIFO) if one exists, otherwise
    /// appending a fresh row. Returns the [`Handle`] addressing it.
    pub fn add(&mut self, value: T) -> Handle<T> {
        if let Some(reused) = self.free.pop() {
            let idx = reused as usize;

            // Debug-only integrity cross-check: `records[idx]`'s intrusive
            // `next_free` echo (captured by `remove` when this row was
            // vacated) must equal the flat `free` stack's new head now that
            // `idx` itself has been popped off — see `Slot`'s doc. This is a
            // stack-invariant tautology under correct push/pop discipline,
            // so it costs nothing to hold; it exists to catch a future bug
            // that mutates `free` out of band.
            if let Slot::Vacant { next_free } = &self.records[idx] {
                debug_assert_eq!(
                    *next_free,
                    self.free.last().copied().unwrap_or(u32::MAX),
                    "invariant: free-list corruption — row {idx}'s intrusive \
                     `next_free` echo must match `free`'s new head after reuse"
                );
            } else {
                debug_assert!(
                    false,
                    "invariant: a row index popped off `free` must be Vacant"
                );
            }

            self.records[idx] = Slot::Occupied(value);
            self.states[idx] = AssetLoadState::Loaded;
            self.live += 1;
            // The generation was already bumped by `remove` when this row
            // was vacated — reuse it as-is so the freed `Handle` (whose
            // generation is now stale) is rejected.
            return Handle::new(reused, self.generations[idx]);
        }

        let idx = self.records.len();
        debug_assert!(
            idx < u32::MAX as usize,
            "invariant: Assets<T> row count exceeds u32 range (the planned \
             render carrier is a 32-bit index)"
        );
        self.records.push(Slot::Occupied(value));
        self.generations.push(0);
        self.states.push(AssetLoadState::Loaded);
        self.live += 1;
        Handle::new(idx as u32, 0)
    }

    /// Returns a shared reference to the value `handle` addresses, or `None`
    /// if `handle` is out of range, stale (generation mismatch), or the row
    /// is vacant.
    #[inline]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let idx = self.resolve_index(handle)?;
        match &self.records[idx] {
            Slot::Occupied(value) => Some(value),
            Slot::Vacant { .. } => None,
        }
    }

    /// Mutable variant of [`get`](Self::get). Bumps [`dirty_gen`](Self::dirty_gen)
    /// on every call that resolves to a live row — callers holding the
    /// returned `&mut T` are assumed to mutate it.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let idx = self.resolve_index(handle)?;
        match &mut self.records[idx] {
            Slot::Occupied(value) => {
                self.dirty_gen += 1;
                Some(value)
            }
            Slot::Vacant { .. } => None,
        }
    }

    /// Frees the row `handle` addresses, returning its value.
    ///
    /// Bumps the row's generation so the just-freed `handle` (and any other
    /// copy of it) is rejected by [`get`](Self::get) / [`get_mut`](Self::get_mut)
    /// / [`contains`](Self::contains) from this point on, including after
    /// the row is reused by a future [`add`](Self::add). See [`Handle`]'s
    /// doc for the render-carrier caveat before calling this on a
    /// render-referenced asset.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let idx = self.resolve_index(handle)?;

        // Mirrors the current free-list head into this row's own `Vacant`
        // link — a debug-only cross-check `add` reads back on reuse; see
        // `Slot`'s doc.
        let next_free = self.free.last().copied().unwrap_or(u32::MAX);
        let slot = std::mem::replace(&mut self.records[idx], Slot::Vacant { next_free });
        let Slot::Occupied(value) = slot else {
            // `resolve_index` matched `handle.generation()` against
            // `self.generations[idx]`; a matching generation can only occur
            // while the row is `Occupied` (this very function is the only
            // place a generation is bumped, and it is bumped in the same
            // step the row is vacated) — so this arm is unreachable absent
            // a bug in that invariant.
            unreachable!("invariant: generation match implies an Occupied slot")
        };

        self.generations[idx] = self.generations[idx].wrapping_add(1);
        self.free.push(idx as u32);
        self.live -= 1;
        Some(value)
    }

    /// `true` if `handle` resolves to a live row.
    #[inline]
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.resolve_index(handle).is_some()
    }

    /// Number of live (`Occupied`) rows. O(1) — backed by a running counter,
    /// not a scan.
    #[inline]
    pub fn len(&self) -> usize {
        self.live
    }

    /// `true` if there are no live rows.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// The slot-row count INCLUDING freed holes — `records.len()`, the high-water mark
    /// of every row index ever minted. O(1).
    ///
    /// This is the size an INDEX-ADDRESSED GPU mirror (e.g. a material/mesh device
    /// table keyed by `Handle::index()`) must allocate: [`len`](Self::len) is the LIVE
    /// count, but a still-live [`Handle`]'s `index()` can exceed `len() - 1` once a
    /// hole exists (some OTHER row was freed without being reused) — sizing a mirror
    /// buffer by `len()` and then writing at `handle.index()` is an out-of-bounds write
    /// the moment a hole exists. `high_water()` never shrinks below the max index ever
    /// minted, even across `remove`.
    #[inline]
    pub fn high_water(&self) -> usize {
        self.records.len()
    }

    /// Resolves a dense row `index` DIRECTLY, bypassing the generation check —
    /// the sanctioned way a render carrier that stores only a raw `u32` index
    /// (no generation in hand, e.g. a GPU-mirror table or a bucketed gather
    /// keyed by dense mesh id) resolves back to its record. Returns `None` if
    /// `index` is out of range or the row is vacant.
    ///
    /// # Append-only caveat
    ///
    /// Sound ONLY under the append-only usage [`Handle`]'s own doc already
    /// documents for a render-visible table: once the row at `index` is freed
    /// and reused by [`add`](Self::add), this call cannot distinguish the old
    /// occupant from the new one (no generation is consulted here) — the same
    /// P1-3 caveat governs any render carrier that stores a bare index.
    /// Callers that never [`remove`](Self::remove) a live render-referenced
    /// handle (the only supported usage today) are safe.
    #[inline]
    pub fn get_by_index(&self, index: u32) -> Option<&T> {
        match self.records.get(index as usize)? {
            Slot::Occupied(value) => Some(value),
            Slot::Vacant { .. } => None,
        }
    }

    /// Returns the [`AssetLoadState`] of the row `handle` addresses, or
    /// `None` if `handle` is out of range or stale.
    ///
    /// Deliberately does NOT require the row to be `Occupied`: a `Failed`
    /// row's value may be a placeholder, but its state is still readable.
    #[inline]
    pub fn state(&self, handle: Handle<T>) -> Option<AssetLoadState> {
        let idx = self.resolve_index(handle)?;
        Some(self.states[idx])
    }

    /// Monotonically increasing counter bumped by every [`get_mut`](Self::get_mut)
    /// call that resolves to a live row. Consumers (a future GPU-upload pass)
    /// compare this against their own last-seen value to decide whether a
    /// re-upload is needed, mirroring the light/material staging dirty-gens
    /// documented on `boyko_render`'s registries.
    #[inline]
    pub fn dirty_gen(&self) -> u64 {
        self.dirty_gen
    }

    /// Iterates every live `(Handle<T>, &T)` pair, skipping vacant rows.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> + '_ {
        self.records
            .iter()
            .enumerate()
            .filter_map(move |(idx, slot)| match slot {
                Slot::Occupied(value) => Some((Handle::new(idx as u32, self.generations[idx]), value)),
                Slot::Vacant { .. } => None,
            })
    }

    /// Validates `handle` against the current row count + generation,
    /// returning the row index on success. Does NOT check occupancy — the
    /// generation invariant (bumped exactly once per free/reuse cycle)
    /// guarantees a matching generation implies `Occupied`.
    #[inline]
    fn resolve_index(&self, handle: Handle<T>) -> Option<usize> {
        let idx = handle.index() as usize;
        if idx >= self.generations.len() || self.generations[idx] != handle.generation() {
            return None;
        }
        Some(idx)
    }
}

impl<T: Asset> Default for Assets<T> {
    /// An empty table with no rows reserved.
    fn default() -> Self {
        Self::with_reserved(0)
    }
}

impl<T: Asset + Send + Sync> Resource for Assets<T> {
    // The `ResourceId` is minted through the `TypeId`-keyed process-global
    // registry, NOT a `static ID: OnceLock<_>` in this generic body: such a
    // static collapses across monomorphisations (rust#22991), aliasing e.g.
    // `Assets<Mesh>` and `Assets<Material>` onto the SAME resource slot. See
    // `resources::resource_type_registry` and `State<S>`'s identical impl.
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<Assets<T>>()
    }
}

// Asset-system rung A2 (the "Resource residency" flavor — e.g. `Assets<MeshGpu>`,
// whose records own device buffers): a `!Send` asset record (`T: !Send`) cannot
// satisfy the `Resource` impl above (it requires `T: Send + Sync`), so its
// `Assets<T>` table must be registered through the separate NonSend slab instead.
//
// `NonSendResource` carries no `Send`/`Sync` bound at all (`T: 'static` only,
// already implied by `Asset: 'static + Sized`), so this impl is unconditional —
// ANY `Assets<T>` may be registered as a NonSend resource, regardless of whether
// `T` also happens to be `Send + Sync` (in which case it may ALSO be registered as
// a plain `Resource`, at the caller's choice; the two slabs are independent, so
// there is no ambiguity — a type satisfying both traits is not itself inserted
// into both slabs unless a caller explicitly calls both `insert_resource` and
// `insert_non_send_resource`).
//
// This impl must live HERE, not in a downstream crate: `Assets<T>` and
// `NonSendResource` are both defined in this crate, so a downstream crate (e.g.
// `boyko_render`, implementing this for `Assets<MeshGpu>`) would hit the orphan
// rule (E0117) — neither `Assets` nor `NonSendResource` is local there, and
// `Assets` is not a "fundamental" type (unlike `&`/`&mut`/`Box`), so nesting a
// local `T` inside it does not satisfy coherence.
impl<T: Asset> NonSendResource for Assets<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal concrete `Asset` for the unit/proptest suite below. `Cpu`
    /// is never exercised at A0 (no loader dispatch yet) — it just needs to
    /// satisfy the `Send` bound.
    impl Asset for u64 {
        type Cpu = u64;
    }

    /// Stale-handle rejection across a full add → remove → reuse cycle
    /// (plan §A0 unit: generational stale-handle rejection).
    #[test]
    fn stale_handle_rejected_after_remove_and_reuse() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let old = assets.add(111);
        assert_eq!(assets.remove(old), Some(111));

        assert_eq!(assets.get(old), None, "removed handle must not resolve");
        assert!(
            assets.get_mut(old).is_none(),
            "removed handle must not resolve mutably"
        );
        assert!(!assets.contains(old), "removed handle must report absent");

        let fresh = assets.add(222);
        assert_eq!(
            fresh.index(),
            old.index(),
            "the freed row must be reused (same index)"
        );
        assert_eq!(
            fresh.generation(),
            old.generation() + 1,
            "reuse must bump the generation by exactly one"
        );

        assert_eq!(assets.get(fresh), Some(&222));
        assert_eq!(
            assets.get(old),
            None,
            "the OLD handle must still fail after the row was reused"
        );
        assert!(!assets.contains(old));
    }

    /// The free list is LIFO and rows are correctly reused without ever
    /// producing two live handles for the same row (plan §A0 unit:
    /// free-list LIFO reuse).
    #[test]
    fn free_list_reuses_lifo_without_duplication() {
        let mut assets = Assets::<u64>::with_reserved(8);
        let handles: Vec<_> = (0..5u64).map(|v| assets.add(v)).collect();
        assert_eq!(assets.len(), 5);

        // Remove rows 1 and 3 (in that order) — LIFO reuse must hand back
        // row 3 first, then row 1.
        assert_eq!(assets.remove(handles[1]), Some(1));
        assert_eq!(assets.remove(handles[3]), Some(3));
        assert_eq!(assets.len(), 3);

        let reuse_a = assets.add(301);
        assert_eq!(
            reuse_a.index(),
            handles[3].index(),
            "LIFO: the most recently freed row is reused first"
        );
        let reuse_b = assets.add(302);
        assert_eq!(
            reuse_b.index(),
            handles[1].index(),
            "LIFO: the second-most recently freed row is reused second"
        );

        assert_eq!(assets.len(), 5, "len() must reflect exactly the 5 live rows");
        // No duplicate live row: every original still-live handle plus both
        // reused handles must resolve to distinct, correct values.
        assert_eq!(assets.get(handles[0]), Some(&0));
        assert_eq!(assets.get(handles[2]), Some(&2));
        assert_eq!(assets.get(handles[4]), Some(&4));
        assert_eq!(assets.get(reuse_a), Some(&301));
        assert_eq!(assets.get(reuse_b), Some(&302));
    }

    /// `iter()` visits exactly the `Occupied` rows, in row order (plan §A0
    /// unit: `iter()` visits exactly the Occupied slots).
    #[test]
    fn iter_visits_only_occupied_rows() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h0 = assets.add(10);
        let h1 = assets.add(20);
        let h2 = assets.add(30);
        assets.remove(h1);

        let visited: Vec<(Handle<u64>, u64)> = assets.iter().map(|(h, v)| (h, *v)).collect();
        assert_eq!(
            visited,
            vec![(h0, 10), (h2, 30)],
            "iter() must skip the vacated row and preserve the rest in order"
        );
    }

    /// `get_by_index` resolves a dense row bypassing generation, returns `None`
    /// out of range, and correctly stops resolving a freed row while a still-live
    /// higher index remains resolvable (plan A2 unit: the render-carrier indexed
    /// accessor added for `MeshHandle`-shaped consumers).
    #[test]
    fn get_by_index_resolves_dense_row_bypassing_generation() {
        let mut assets = Assets::<u64>::with_reserved(4);
        let h0 = assets.add(10);
        let h1 = assets.add(20);

        assert_eq!(assets.get_by_index(h1.index()), Some(&20), "index 1 is the 2nd row");
        assert_eq!(assets.get_by_index(5), None, "an out-of-range index must be None");

        assets.remove(h0);
        assert_eq!(
            assets.get_by_index(h0.index()),
            None,
            "a freed row must resolve to None even though the index is in range"
        );
        assert_eq!(
            assets.get_by_index(h1.index()),
            Some(&20),
            "a still-live higher index remains resolvable after a lower row is freed"
        );
    }

    /// `len()` / `is_empty()` track live rows through interleaved add/remove.
    #[test]
    fn len_and_is_empty_track_live_count() {
        let mut assets = Assets::<u64>::with_reserved(2);
        assert!(assets.is_empty());
        let h = assets.add(1);
        assert_eq!(assets.len(), 1);
        assert!(!assets.is_empty());
        assets.remove(h);
        assert_eq!(assets.len(), 0);
        assert!(assets.is_empty());
    }

    /// `get_mut` bumps `dirty_gen`; `get` does not.
    #[test]
    fn get_mut_bumps_dirty_gen_get_does_not() {
        let mut assets = Assets::<u64>::with_reserved(1);
        let h = assets.add(7);
        assert_eq!(assets.dirty_gen(), 0);
        let _ = assets.get(h);
        assert_eq!(assets.dirty_gen(), 0, "get() must not bump dirty_gen");
        *assets.get_mut(h).expect("h is live") += 1;
        assert_eq!(assets.dirty_gen(), 1);
        assert_eq!(assets.get(h), Some(&8));
    }

    /// proptest oracle: a random sequence of add/remove/get against a model
    /// `HashMap<Handle<u64>, u64>` (plan §A0 proptest). `Assets` must never
    /// return a stale value, never resolve to the wrong row, and its live
    /// count must always match the model's.
    mod oracle {
        use std::collections::HashMap;

        use proptest::prelude::*;

        use super::*;

        #[derive(Clone, Debug)]
        enum Op {
            Add(u64),
            RemoveAt(usize),
            GetAt(usize),
        }

        fn op_strategy() -> impl Strategy<Value = Op> {
            prop_oneof![
                any::<u64>().prop_map(Op::Add),
                any::<usize>().prop_map(Op::RemoveAt),
                any::<usize>().prop_map(Op::GetAt),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]
            #[test]
            fn assets_matches_hashmap_oracle(ops in proptest::collection::vec(op_strategy(), 1..300)) {
                let mut assets = Assets::<u64>::with_reserved(16);
                let mut oracle: HashMap<Handle<u64>, u64> = HashMap::new();
                // Every handle ever minted, including ones later removed —
                // lets `RemoveAt`/`GetAt` also exercise already-stale handles.
                let mut minted: Vec<Handle<u64>> = Vec::new();

                for op in ops {
                    match op {
                        Op::Add(value) => {
                            let handle = assets.add(value);
                            oracle.insert(handle, value);
                            minted.push(handle);
                        }
                        Op::RemoveAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let real = assets.remove(handle);
                                let model = oracle.remove(&handle);
                                prop_assert_eq!(real, model, "remove() must match the oracle exactly");
                            }
                        }
                        Op::GetAt(pick) => {
                            if let Some(&handle) = minted.get(pick % minted.len().max(1)) {
                                let real = assets.get(handle).copied();
                                let model = oracle.get(&handle).copied();
                                prop_assert_eq!(real, model, "get() must match the oracle exactly (never a stale or wrong value)");
                            }
                        }
                    }
                    prop_assert_eq!(assets.len(), oracle.len(), "live count must always match the oracle");
                }
            }
        }
    }
}
