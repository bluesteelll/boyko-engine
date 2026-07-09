//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

// ── Ref<'w, T> (Phase 10 Wave C Step 11 — read-only with ticks) ─────────────

/// Read-only access to component `T` with attached change-detection info
/// (plan §2.5 REF1-REF4 / §6.1).
///
/// Compared to `&T` (no tick exposure) and `Changed<T>` (forces a filter
/// through the type system), `Ref<T>` is the "I want to read the tick
/// without filtering" path. Use it when the system needs to know whether
/// `T` was added or changed and to read the underlying value in the same
/// pass.
///
/// # Boundary semantics ([`Ref::is_added`] / [`Ref::is_changed`])
///
/// Per plan §6.2 / §6.2-bis (Round 2 O1) — both predicates use the inclusive
/// lower-bound trick (`last_run - 1`) so a self-write within the current
/// system reports as changed. The match formula is
/// `tick.is_newer_than(last_run - 1, this_run)`, equivalent to
/// `tick >= last_run` under bounded ages. See [`Tick::is_newer_than`] for
/// the precise wrapping semantics.
pub struct Ref<'w, T: Component> {
    /// Borrowed view into the component slot.
    pub(crate) value: &'w T,
    /// Snapshot of the row's `added` tick at fetch time.
    pub(crate) added: Tick,
    /// Snapshot of the row's `changed` tick at fetch time.
    pub(crate) changed: Tick,
    /// System's `last_run` snapshot.
    pub(crate) last_run: Tick,
    /// System's `this_run` snapshot.
    pub(crate) this_run: Tick,
}

impl<'w, T: Component> Ref<'w, T> {
    /// Returns `true` if `T` was inserted into this row since the system's
    /// `last_run` tick.
    ///
    /// Uses the inclusive-lower-bound trick (plan §6.2 / Round 2 O1):
    /// `last_run - 1` is passed as `is_newer_than`'s exclusive lower bound,
    /// promoting it to inclusive. Equivalent to `added >= last_run` under
    /// the bounded-age discipline of plan §9.3.
    #[inline]
    pub fn is_added(&self) -> bool {
        self.added
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }

    /// Returns `true` if `T` was added OR mutated in this row since the
    /// system's `last_run` tick.
    ///
    /// Same inclusive semantic as [`Self::is_added`] (plan §6.2-bis): a
    /// self-write within the current frame reports as changed.
    #[inline]
    pub fn is_changed(&self) -> bool {
        self.changed
            .is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)
    }
}

impl<'w, T: Component> std::ops::Deref for Ref<'w, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

/// Per-system state for `Ref<T>: QueryData`. Identical to [`ReadState`] —
/// only the trait-impl side differs (the Fetch carries tick column bases).
#[derive(Clone, Copy)]
pub struct RefState<T: Component> {
    pub(crate) id: ComponentId,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

/// Per-archetype fetch scratch for `Ref<T>: QueryData`.
///
/// Caches:
/// * `value_base` — the component column for `T` (cast from `column.ptr`).
/// * `added_base` / `changed_base` — the tick column bases returned by
///   `Archetype::tick_column_base`.
/// * `last_run` / `this_run` — the system's tick snapshot captured at
///   `set_table_*` time so the per-row hot loop pays no indirection.
///
/// All fields are populated by `set_table_*` before any `fetch` call; the
/// tick columns are write-once sub-regions of the pool's own
/// `VmReservation`, so their base addresses are stable for the pool's
/// lifetime (Phase X.I vm-reservation stability).
pub struct RefFetch<'w, T: Component> {
    pub(crate) value_base: *const T,
    pub(crate) added_base: *const UnsafeCell<Tick>,
    pub(crate) changed_base: *const UnsafeCell<Tick>,
    pub(crate) last_run: Tick,
    pub(crate) this_run: Tick,
    pub(crate) _marker: PhantomData<&'w T>,
}

impl<T: Component> Clone for RefFetch<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for RefFetch<'_, T> {}

// SAFETY (QD1-QD4):
//   - QD1: `state.id` is `T::component_id()`; `init_access` declares a read.
//   - QD2: `init_fetch` produces NULL pointers; `set_table_*` overwrites
//     them before any `fetch` call.
//   - QD3: every cached pointer is scoped to `'w` via `PhantomData<&'w T>`.
//   - QD4: read-only data — both `set_table_*` methods produce identical
//     behaviour (the mutable variant delegates to the read-only path).
unsafe impl<T: Component> QueryData for Ref<'_, T> {
    type State = RefState<T>;
    type Fetch<'w> = RefFetch<'w, T>;
    type Item<'w> = Ref<'w, T>;
    const IS_READ_ONLY: bool = true;
    // Phase 12.5 Track B NCD2: `Ref<T>` exposes per-row tick info; the
    // dispatcher MUST forward `meta` so `set_table_*` can copy
    // `last_run` / `this_run` into the Fetch.
    const NEEDS_CHANGE_DETECTION: bool = true;
    // EnableTag C2: `Ref<T>` is a real data component (positive include bit).
    const HAS_DATA_COMPONENT: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        RefState {
            id: T::component_id(),
            _marker: PhantomData,
        }
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        access_set
            .add_component_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
        mask.contains(state.id)
    }

    #[inline]
    fn aggregate_include(state: &Self::State, include: &mut ComponentMask) {
        include.set(state.id);
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        RefFetch {
            value_base: std::ptr::null(),
            added_base: std::ptr::null(),
            changed_base: std::ptr::null(),
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
            _marker: PhantomData,
        }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): `archetype` is a live `*const Archetype` for `'w`
        //   (caller contract); `columns` at offset 0 per Phase 7 D4;
        //   `state.id.0 < MAX_COMPONENTS` by construction of the cached id.
        let column = unsafe { (*archetype).columns.get_unchecked(state.id.0) };
        debug_assert!(!column.ptr.is_null(), "QD2: column was unexpectedly null");
        fetch.value_base = column.ptr as *const T;

        // SAFETY (STORE3): shared reborrow of the archetype is sound; the
        //   sparse map read does not produce write provenance. `archetype`
        //   contains `state.id` per QD1 / archetype matching, so
        //   `tick_column_base` returns `Some`.
        let archetype_ref: &Archetype = unsafe { &*archetype };
        let (added_base, changed_base) = archetype_ref
            .tick_column_base(state.id)
            .expect("QD1: matched archetype must contain T's pool");
        fetch.added_base = added_base;
        fetch.changed_base = changed_base;

        fetch.last_run = meta.last_run();
        fetch.this_run = meta.this_run();
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // For `Ref<T>`, the mutable cursor variant degrades to the same
        // read-only setup; no write provenance is consumed.
        // SAFETY (QD3, QD4): same conditions as `set_table_readonly` with
        //   the strictly-stronger caller guarantee that `archetype` carries
        //   write-capable provenance.
        unsafe { Self::set_table_readonly(fetch, state, archetype as *const _, meta) }
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_table_readonly_no_meta<'w>(
        _fetch: &mut Self::Fetch<'w>,
        _state: &Self::State,
        _archetype: *const Archetype,
    ) {
        // NCD5 backstop: dispatcher's NCD6 const-fold must route Ref<T>
        // through the meta-bearing path. Reaching here means a contributor
        // broke the dispatch contract.
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
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (QD2, QD3, STORE3):
        //   - `set_table_*` was called before `fetch` (caller contract), so
        //     every base pointer is non-null and points at the active
        //     archetype's column for `T`.
        //   - `row < entity_count` (caller contract).
        //   - The returned `Ref<'w, T>` lifetime is tied to `'w` via
        //     `PhantomData<&'w T>` in `RefFetch`.
        //   - STORE3: the tick reads through `UnsafeCell::get()` need no
        //     concurrent writer of this `(archetype, T)` slot. The guarantee is
        //     named by the construction origin: on the scheduler/query path
        //     (the only origin that builds a read-only `Ref`), Phase 9 SCH3 —
        //     the conflict graph — grants exclusive access; the system-less
        //     `&mut World` path (which `EcsMaster::get_component_mut` uses for
        //     `Mut`) would supply whole-world exclusivity instead. `Tick` is
        //     `Copy`.
        unsafe {
            let value = &*fetch.value_base.add(row);
            let added = *(*fetch.added_base.add(row)).get();
            let changed = *(*fetch.changed_base.add(row)).get();
            Ref {
                value,
                added,
                changed,
                last_run: fetch.last_run,
                this_run: fetch.this_run,
            }
        }
    }
}

// SAFETY: `Ref<T>::IS_READ_ONLY = true`; the impl never writes.
unsafe impl<T: Component> ReadOnlyQueryData for Ref<'_, T> {}

