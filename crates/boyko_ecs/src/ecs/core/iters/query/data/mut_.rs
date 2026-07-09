//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

// ── Mut<'w, T> (Phase 10 Wave C Step 11 — write with deref guard) ──────────

/// Mutable access to component `T` with a deref guard that bumps the row's
/// `changed_tick` on first `DerefMut` (plan §2.5 MUT1-MUT8 / §6.2 / §3 Q6).
///
/// Compared to `&mut T` (no change tracking), `Mut<T>` is the path that
/// participates in change detection. Any `DerefMut` call counts as
/// "changed" — the Bevy deref-bump semantic (plan §3 Q6 adopted). Use
/// [`Self::set_if_neq`] or [`Self::bypass_change_detection`] to opt into
/// stricter behaviour.
///
/// # Boundary semantics ([`Self::is_added`] / [`Self::is_changed`])
///
/// Inclusive lower-bound semantic (plan §6.2 / Round 2 O1 + Round 3 O1):
/// a self-write within the same system reports as changed.
///
/// # Once-only deref guard
///
/// The first `deref_mut()` call on a given `Mut<T>` writes `this_run` to
/// the row's `changed_tick`. Subsequent calls within the same guard
/// instance skip the write (`deref_mut_called` flag). This is a
/// micro-optimisation: even if the compiler does not elide the duplicate
/// store the cost is one extra u32 store per call, and the semantic is
/// identical.
pub struct Mut<'w, T: Component> {
    /// Borrowed view into the component slot.
    pub(crate) value: &'w mut T,
    /// Snapshot of the row's `added` tick at fetch time.
    pub(crate) added: Tick,
    /// Pointer to the row's `changed_tick` slot. The write target for the
    /// deref guard. Stable for the pool's lifetime (write-once tick
    /// sub-region of the pool's own `VmReservation` — Phase X.I).
    pub(crate) changed_tick: *const UnsafeCell<Tick>,
    /// System's `last_run` snapshot.
    pub(crate) last_run: Tick,
    /// System's `this_run` snapshot.
    pub(crate) this_run: Tick,
    /// Has `deref_mut` already bumped the changed tick this guard?
    /// Skips duplicate stores on repeated `deref_mut()` calls.
    pub(crate) deref_mut_called: bool,
}

impl<'w, T: Component> Mut<'w, T> {
    /// Returns `true` if `T` was inserted into this row since the system's
    /// `last_run` tick (inclusive lower bound; plan §6.2 / Round 2 O1).
    #[inline]
    pub fn is_added(&self) -> bool {
        self.added
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Returns `true` if `T` was added OR mutated in this row since the
    /// system's `last_run` tick.
    ///
    /// Reads the current `changed_tick` slot (per-row); a self-write earlier
    /// in this system reports as changed thanks to the inclusive-lower-bound
    /// trick (plan §6.2-bis worked proof).
    #[inline]
    pub fn is_changed(&self) -> bool {
        // SAFETY (STORE3): `changed_tick` is a live `UnsafeCell<Tick>` slot for
        //   the row (set by `set_table_mut` on the query path, or by
        //   `EcsMaster::get_component_mut` on the direct path). Exclusivity of
        //   this read rests on the `Mut`'s construction origin: on the
        //   scheduler/query path, Phase 9 SCH3 (the conflict graph) grants
        //   exclusive `(archetype, T)` access; on the system-less
        //   `EcsMaster::get_component_mut` path, `&mut World` whole-world
        //   exclusivity (the method borrows the entire `EcsMaster` for the
        //   `Mut`'s lifetime). Both independently guarantee no concurrent
        //   reader/writer of this row's tick. `Tick` is `Copy`.
        let tick: Tick = unsafe { *(*self.changed_tick).get() };
        tick.is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Sets the value only if it differs from the current one (per
    /// `PartialEq`). Avoids triggering `Changed<T>` when the new value
    /// equals the old.
    ///
    /// Returns `true` if the write happened, `false` otherwise.
    pub fn set_if_neq(&mut self, new_value: T) -> bool
    where
        T: PartialEq,
    {
        if *self.value != new_value {
            *self.value = new_value;
            // The assignment above writes THROUGH the inner `&mut T`
            // (`self.value`) directly — it does NOT route through
            // `Mut::deref_mut`, so the deref-bump guard never fired. Bump the
            // changed tick manually here.
            if !self.deref_mut_called {
                self.deref_mut_called = true;
                // SAFETY (STORE3, plan §2.5 MUT4): exclusivity of this
                //   `changed_tick` write rests on the `Mut`'s construction
                //   origin — on the scheduler/query path, Phase 9 SCH3 (the
                //   conflict graph) grants exclusive `(archetype, T)` access (the
                //   system declared a write to `T`); on the system-less
                //   `EcsMaster::get_component_mut` path, `&mut World` whole-world
                //   exclusivity. Both independently guarantee no concurrent
                //   reader/writer of this row's tick. `Tick` is `Copy`.
                unsafe {
                    *(*self.changed_tick).get() = self.this_run;
                }
            }
            true
        } else {
            false
        }
    }

    /// Returns `&mut T` without bumping the changed tick. Breaks
    /// `Changed<T>` for this row until the next legitimate write — use
    /// sparingly (plan §2.5 MUT5).
    #[inline]
    pub fn bypass_change_detection(&mut self) -> &mut T {
        self.value
    }
}

impl<'w, T: Component> std::ops::Deref for Mut<'w, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'w, T: Component> std::ops::DerefMut for Mut<'w, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        if !self.deref_mut_called {
            self.deref_mut_called = true;
            // SAFETY (STORE3, plan §2.5 MUT3):
            //   - `self.changed_tick` is a live `UnsafeCell<Tick>` slot for the
            //     cached row (set by `set_table_mut` on the query path, or by
            //     `EcsMaster::get_component_mut` on the direct path); the
            //     slot lives in a write-once tick sub-region of the pool's
            //     own `VmReservation`, address-stable for the pool's
            //     lifetime (Phase X.I).
            //   - Exclusivity of this write is named by the construction origin:
            //     on the scheduler/query path, Phase 9 SCH3 (the conflict graph)
            //     guarantees the system holding `Mut<T>` has exclusive access to
            //     this `(archetype, T)` slot — no concurrent reader/writer of
            //     this tick exists, and per-row adjacent writes from sibling
            //     `par_iter` chunks ride disjoint memory locations (Round 2 C3 —
            //     distinct `UnsafeCell<u32>`s); on the system-less
            //     `EcsMaster::get_component_mut` path, `&mut World` whole-world
            //     exclusivity supplies the same guarantee. `Tick` is `Copy`.
            unsafe {
                *(*self.changed_tick).get() = self.this_run;
            }
        }
        self.value
    }
}

/// Per-system state for `Mut<T>: QueryData`. Same shape as [`WriteState`].
#[derive(Clone, Copy)]
pub struct MutState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

/// Per-archetype fetch scratch for `Mut<T>: QueryData`.
///
/// Caches the write-capable component column base plus both tick column
/// bases (so `fetch` can produce a `Mut<T>` carrying the per-row
/// `changed_tick` pointer for the deref guard). Tick base lifetimes ride
/// the vm-reservation address stability (write-once sub-regions of the
/// pool's own reservation — Phase X.I).
pub struct MutFetch<'w, T: Component> {
    pub(crate) value_base: *mut T,
    pub(crate) added_base: *const UnsafeCell<Tick>,
    pub(crate) changed_base: *const UnsafeCell<Tick>,
    pub(crate) last_run: Tick,
    pub(crate) this_run: Tick,
    /// Dense plan D4: the global `DenseStore` for a DENSE `T`, resolved once by
    /// `resolve_dense`. NULL for a TABLE `T` (the field is never read — the dense
    /// arm const-folds out, the 0%-gate) or when no dense store exists yet.
    pub(crate) dense: *const crate::ecs::core::component::dense::DenseStore,
    /// Dense plan D4: the current archetype's `entity_ids` column base, cached by
    /// `set_table_mut` for the per-row gather (`entity = entity_ids[row]`). NULL
    /// for a TABLE `T`.
    pub(crate) entity_ids: *const crate::ecs::identifiers::primitives::EntityId,
    pub(crate) _marker: PhantomData<&'w mut T>,
}

impl<T: Component> Clone for MutFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for MutFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a write.
//   - QD2: `set_table_mut` overwrites the bases; `set_table_readonly`
//     panics (QD4 runtime backstop — same as `&mut T`).
//   - QD3: lifetime bound by `PhantomData<&'w mut T>` in `MutFetch`.
//   - QD4: `set_table_readonly` is forbidden (the trait gate on
//     `Query::iter()` requires `D: ReadOnlyQueryData`, which `Mut<T>` does
//     not implement).
unsafe impl<T: Component> QueryData for Mut<'_, T> {
    type State = MutState<T>;
    type Fetch<'w> = MutFetch<'w, T>;
    type Item<'w> = Mut<'w, T>;
    const IS_READ_ONLY: bool = false;
    // Phase 12.5 Track B NCD2: `Mut<T>` exposes per-row tick info via the
    // deref guard and `is_added`/`is_changed`; the dispatcher MUST forward
    // `meta`.
    const NEEDS_CHANGE_DETECTION: bool = true;
    // EnableTag C2: `Mut<T>` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;
    // Dense plan D4: dense-ness is a compile-time property of `T`.
    const HAS_DENSE: bool = T::STORAGE_IS_DENSE;
    // Dense plan D4: a dense `Mut<T>` is a dense INCLUDE term (seeds candidates).
    const HAS_DENSE_INCLUDE: bool = T::STORAGE_IS_DENSE;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        MutState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_write(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        // Dense is signature-excluded (mirrors `&mut T`): the mask never carries
        // its bit, so a dense include must NOT gate at the archetype level. Per-row
        // membership is the `e2s` oracle (D4).
        if const { T::STORAGE_IS_DENSE } {
            return true;
        }
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        // Dense contributes NO include bit (signature-excluded); candidates are
        // seeded from `arch_presence` (D4 seed wiring, via `dense_include_candidates`).
        if const { T::STORAGE_IS_DENSE } {
            return;
        }
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        MutFetch {
            value_base: std::ptr::null_mut(),
            added_base: std::ptr::null(),
            changed_base: std::ptr::null(),
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
            dense: std::ptr::null(),
            entity_ids: std::ptr::null(),
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Gated by `const { Self::HAS_DENSE }` at the cursor — never emitted for a
        // TABLE `T` (the 0%-gate). For a dense `T`, cache the global `DenseStore`
        // pointer (the write-through target's owner; its tick sub-regions back the
        // deref guard's changed-tick bump).
        // SAFETY (resolve_dense contract): `world` upholds the access contract; the
        //   `DenseStore` lives in the address-stable `DenseRegistry` slot array,
        //   valid for `'w`; the pointer is confined to the `'w`-scoped `Fetch`.
        let registry = unsafe { world.world().dense_registry() };
        fetch.dense = match registry.store(state.id) {
            Some(store) => store as *const _,
            None => std::ptr::null(),
        };
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // QD4: read-only cursor on `Mut<T>` is forbidden by the trait gate
        // `D: ReadOnlyQueryData` on `Query::iter()`. Reaching here means a
        // custom `QueryData` impl falsely claimed `ReadOnlyQueryData` for
        // a type containing `Mut<T>`. Panic loudly.
        panic!(
            "QD4 violation: set_table_readonly called for Mut<T> (T = {}). \
             Did a custom QueryData impl falsely claim ReadOnlyQueryData?",
            std::any::type_name::<T>()
        );
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        if const { T::STORAGE_IS_DENSE } {
            // Dense `T`: no archetype column / tick column. Cache `entity_ids` for
            // the per-row gather; the write target + tick sub-regions come from the
            // resolved `DenseStore` in `fetch` (D4). The deref guard's changed-tick
            // bump and `is_added`/`is_changed` read the dense column's per-slot
            // ticks, indexed by SLOT (not row).
            // SAFETY (QD3): `archetype` is live for `'w`; reading the immutable
            //   `entity_ids` slice base needs only shared access.
            let arch_ref: &Archetype = unsafe { &*archetype };
            fetch.entity_ids = arch_ref.entity_ids_slice().as_ptr();
            fetch.last_run = meta.last_run();
            fetch.this_run = meta.this_run();
            return;
        }
        // SAFETY (QD1, QD3): `archetype` carries write-capable provenance
        //   (caller minted it via `archetype_ptr_mut`); `columns` at offset
        //   0 per Phase 7 D4; `state.id.0 < MAX_COMPONENTS`.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.value_base = column.ptr as *mut T;

        // SAFETY (STORE3): shared reborrow for the sparse-map read; no
        //   write-capable provenance is needed for the tick column lookup
        //   (the per-row write goes through `UnsafeCell::get()`, separately
        //   ridden by the access-set declaration).
        let archetype_ref: &Archetype = unsafe { &*archetype };
        let (added_base, changed_base) = archetype_ref
            .tick_column_base(state.id)
            .expect("QD1: matched archetype must contain T's pool");
        fetch.added_base = added_base;
        fetch.changed_base = changed_base;

        fetch.last_run = meta.last_run();
        fetch.this_run = meta.this_run();
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop: dispatcher's NCD6 const-fold must route Mut<T>
        // through the meta-bearing path (and the read-only cursor is
        // additionally gated out by `D: ReadOnlyQueryData` on `iter`).
        panic!(
            "NCD violation: set_table_readonly_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_mut_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *mut Archetype,
    ) {
        panic!(
            "NCD violation: set_table_mut_no_meta called for {} \
             (NEEDS_CHANGE_DETECTION = true).",
            std::any::type_name::<Self>()
        );
    }

    #[inline]
    unsafe fn dense_row_passes<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if const { T::STORAGE_IS_DENSE } {
            // SAFETY (D4): `entity_ids` cached by `set_table_mut`; `row <
            //   entity_count`. NULL `dense` ⟹ no member ⟹ skip.
            if fetch.dense.is_null() {
                return false;
            }
            let entity = unsafe { *fetch.entity_ids.add(row) };
            let store = unsafe { &*fetch.dense };
            return store.slot_of(entity).is_some();
        }
        true
    }

    #[inline]
    fn dense_include_candidates(
        state: &Self::State,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
        out: &mut crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet,
    ) {
        // See `&mut T::dense_include_candidates` — OR-in the dense store's
        // `arch_presence`. Gated by `const { Self::HAS_DENSE_INCLUDE }`.
        if const { T::STORAGE_IS_DENSE }
            && let Some(store) = registry.store(state.id)
        {
            store.arch_presence().for_each_set_bit(|a| out.insert(a));
        }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        if const { T::STORAGE_IS_DENSE } {
            // Dense `Mut`: row → entity → slot → row_ptr + per-slot tick pointers.
            // `dense_row_passes` proved the slot is live before this call. The
            // returned `Mut` carries the slot's `changed_tick` pointer (into the
            // dense column's changed sub-region) so `deref_mut` / `set_if_neq` bump
            // the dense slot's changed tick exactly like the archetypal path.
            // SAFETY (D4): `entity_ids`/`dense` cached; `row < entity_count`; the
            //   entity is a live member, so `slot < column.count()` and
            //   `row_ptr(slot)` is a live, stride-aligned WRITE-capable pointer into
            //   the address-stable column. The dense conflict node (Decision 6) +
            //   the `&mut`-cursor discipline serialise the column, so no other writer
            //   aliases this slot. `added_ticks_ptr`/`changed_ticks_ptr` are the
            //   column's address-stable per-slot tick bases; `slot < count <=
            //   committed_rows`, so `[slot]` is in the committed prefix. `Tick` is
            //   `Copy`. `'w` ties to the world borrow.
            unsafe {
                let entity = *fetch.entity_ids.add(row);
                let store = &*fetch.dense;
                let slot = store
                    .slot_of(entity)
                    .expect("invariant: dense_row_passes proved the slot is live")
                    as usize;
                let ptr = store.solve_view().row_ptr(slot);
                let value = &mut *(ptr as *mut T);
                let added = *(*store.added_ticks_ptr().add(slot)).get();
                let changed_tick = store.changed_ticks_ptr().add(slot);
                return Mut {
                    value,
                    added,
                    changed_tick,
                    last_run: fetch.last_run,
                    this_run: fetch.this_run,
                    deref_mut_called: false,
                };
            }
        }
        // SAFETY (QD2, QD3, STORE3):
        //   - `set_table_mut` was called before `fetch` (caller contract);
        //     every base pointer is live and non-null.
        //   - `row < entity_count` (caller contract).
        //   - STORE3: exclusivity of the `&mut *value_base.add(row)` reborrow
        //     and the tick access is named by the construction origin. This
        //     `fetch` is the scheduler/query origin, where Phase 9 SCH3 (the
        //     conflict graph) grants exclusive `(archetype, T)` access (no
        //     aliasing reader/writer). The `EcsMaster::get_component_mut` direct
        //     origin builds its `Mut` separately, resting on `&mut World`
        //     whole-world exclusivity. The returned `Mut<'w, T>` lifetime is
        //     tied to `'w` via `PhantomData<&'w mut T>` in `MutFetch`.
        //   - The tick reads through `UnsafeCell::get()` are sound for the same
        //     reason; `Tick` is `Copy`.
        //   - `changed_tick` pointer is captured as `*const UnsafeCell<Tick>`
        //     for the row; the deref guard writes through `UnsafeCell::get()`
        //     (per-row distinct memory location — Round 2 C3).
        unsafe {
            let value = &mut *fetch.value_base.add(row);
            let added = *(*fetch.added_base.add(row)).get();
            let changed_tick = fetch.changed_base.add(row);
            Mut {
                value,
                added,
                changed_tick,
                last_run: fetch.last_run,
                this_run: fetch.this_run,
                deref_mut_called: false,
            }
        }
    }
}

// NOTE: No `ReadOnlyQueryData for Mut<T>` impl — `Mut<T>` writes (deref guard).

