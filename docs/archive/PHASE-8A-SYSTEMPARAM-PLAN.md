> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 8a â€” `SystemParam` trait + Resource subsystem (architectural plan)

**Status:** Round 3 â€” addresses architecture-critic findings on Round 2 (3 new criticals, 5 new majors). Implementation depends on Phase 7 (landed) and on this plan being approved.
**Branch (when active):** `ecs`.
**Plan author:** architect agent.
**Scope:** sub-phase 8a only. Phases 8b (`Query<D, F>` DSL), 8c (`IntoSystem` + function-as-system), 8d (`Commands` buffer) are explicitly out of scope.

---

## Changes from Round 1

| Finding | Severity | Resolution |
|---------|----------|------------|
| **C1** â€” `archetype_ptr(&self)` and `world_mut(&self)` retag pointer through `&self` receivers (Tree Borrows SharedReadOnly downgrade) | Critical | **Adopt Bevy's `Copy + by-value receiver` shape.** `UnsafeEcsCell<'w>` remains a raw-pointer newtype (`*mut EcsMaster` + `PhantomData`), but **every method takes `self` by value** (not `&self` / `&mut self`). The cell is `Copy`, so callers can hand out as many copies as needed. By-value receivers eliminate the retag entirely â€” there is no `&self` borrow to downgrade the pointer's provenance. Adds a `#[cfg(debug_assertions)] allows_mutable_access: bool` field mirroring Bevy. See Â§3 (rewritten). |
| **C2** â€” `Item<'w, 's>: SystemParam<State = Self::State>` bound clashes with concrete `Res<'w_outer, R>` impl | Critical | **Match Bevy's exact shape.** The `SystemParam` impl is written as `unsafe impl<'a, R: Resource> SystemParam for Res<'a, R>` with `Item<'w, 's> = Res<'w, R>`. The trait is parameterised over `'a`, so the bound `Item<'w, 's>: SystemParam<State = Self::State>` holds for ALL lifetimes via the generic `'a` blanket. This is the canonical Bevy-shape. No lifetimeless module needed. See Â§13.4 (new). |
| **C3** â€” `Resources::insert` replace path leaks/UBs on panic-in-drop | Critical | **Clear-bit-first protocol.** The replace branch is reordered: (1) extract the old slot's `ptr` + `drop_fn` + `layout`; (2) **clear the `registered_mask` bit BEFORE calling `drop_fn`**; (3) call `drop_fn`; (4) `dealloc`; (5) write the new slot; (6) re-set the bit. If `drop_fn` panics, the observable state is "slot empty" â€” a leak instead of UB. Same fix applied to a new R-invariant. The Phase 7 `add_archetype` replace path is patched in parallel â€” see Â§16.5 (new). |
| **C4** â€” `init_state` aggregation has no intra-system conflict detection | Critical | **Adopt Bevy's two-phase init.** The `SystemParam` trait gains a separate **`init_access`** method (after `init_state`) that takes a mutable `FilteredAccessSet` accumulator. Each param's `init_access` checks for conflicts against siblings already in the set before adding its own. The tuple impl calls each component's `init_access` in order; conflicts panic at registration time with a clear B0002-style diagnostic. See Â§4.5 (new). |
| **C5** â€” drop order with `resources` between `events` and `entity_master` is hand-waved | Critical | **`resources` placed FIRST** (before `events`), and the `Resource` trait doc gains an explicit "Drop impls MUST NOT touch the world" rule. New drop order: `resources â†’ events â†’ entity_master â†’ archetype_master â†’ arena`. See Â§2.2 (rewritten). |
| **M1** â€” `Access` 256 B layout with `_pad: [u8; 64]` is dead weight if write-once | Major | **Drop `align(64)` and `_pad`.** `Access` is **write-once at `init_access` time, read-only thereafter**; Phase 9 mutates only during scheduler init, never from worker threads. Final layout: 192 B, naturally aligned. If Phase 9 later needs per-thread mutation, add per-BitSet padding then (not blanket struct align). See Â§4.1 (rewritten). |
| **M2** â€” `Resource: Send + Sync` strict bound is preloading Phase 9 constraint | Major | **Resolution (a)** â€” keep `Send + Sync` on `Resource` (matches Bevy, simplifies Phase 9 migration), AND add explicit `NonSendResource` deferral note in the trait doc with file:line tracking item: "Deferred to Phase 9 Â§9.4 â€” non-send resources". See Â§5.1.1 (new). |
| **M3** â€” `MAX_RESOURCES = 256` is locked by `BitSet256` width, "adjustable" claim is misleading | Major | **Rename `MAX_RESOURCES` â†’ `RESOURCE_SLOT_COUNT = 256`** with doc-comment "locked to `BitSet256` width; raising requires a wider bitset". See Â§5.1 (renamed). |
| **M4** â€” `init_state` with `&mut EcsMaster` allows mid-init mutations violating SP1 | Major | **Resolution (a)** â€” keep `&mut EcsMaster` (required for `init_state` to allocate state like `QueryState` later), but add a debug-time invariant: `SystemMeta` records `world.archetype_master.archetype_generation()` before/after the init sweep and `debug_assert_eq!`s them. Documented in the `SystemParam::init_state` trait doc as: "Implementations MUST NOT register new resources, components, or archetypes â€” this is a `debug_assert`-checked invariant." See Â§13.5 (new). |
| **M5** â€” `FnOnceSystem::new` signature missing; turbofish may be required | Major | **Spell out the full signature** and add a `EcsMaster::run_closure_once<P, F, O>` helper that takes the closure directly. **Note (W3 update):** Phase 8a callers DO still need turbofish on the param tuple type because closure-arg inference cannot infer `P`; this is honestly documented and is removed by Phase 8c's `IntoSystem`. See Â§8.4 (rewritten). |
| **M6** â€” `#[derive(Component)] + #[derive(Resource)]` on the same type â€” allowed or forbidden? | Major | **Forbid via runtime check at `register_resource_new`** time. The check uses `TypeId` against the component registry's reverse map (added in this phase). Diagnostic: "type `X` is already registered as a Component; a type cannot be both Component and Resource". See Â§5.1.2 (new). |
| **M7** â€” Arity 12 vs Bevy's 16: diagnostic when user writes 13-param function | Major | **Resolution (b)** â€” keep arity 12. **Round 3 (C-NEW-2 update):** the diagnostic is now implemented via `const { panic!(...) }` in the stub impl bodies (fires at monomorphization). See Â§7.5 (rewritten). |
| **M8** â€” `Access::conflicts_with` is useless for intra-system check | Major | **Split into two predicates.** `Access::conflicts_with(other: &Access) -> bool` remains the Phase 9 cross-system check. Intra-system conflict detection is handled by `FilteredAccessSet::add_resource_read/write` (returning `Result<(), AccessConflict>`) in the C4 resolution. The two structs are orthogonal: `Access` is the *summary*, `FilteredAccessSet` is the *accumulator*. See Â§4.2 (rewritten) and Â§4.5 (new). |
| **Q1** â€” naming inconsistency `UnsafeEcsCell` vs `UnsafeWorldCell` | Quality | All listings audited and corrected. Plan uses `UnsafeEcsCell` everywhere. |
| **Q2** â€” `init_state` SAFETY contract | Quality | Documented as a *contract* invariant on the `unsafe trait SystemParam`. No additional `unsafe fn` marker needed â€” the obligation flows from the `unsafe trait`. |
| **Q3** â€” `#[inline]` discipline | Quality | Preserved unchanged. |
| **Q4** â€” `Resources::new` constructor pattern | Quality | Picked **`Box::<T>::new_uninit().assume_init()`** pattern (mirror of `ArchetypeBundle::new`). Justification: the slab is `[ResourceSlotStorage; N]` where `ResourceSlotStorage` wraps `MaybeUninit<ResourceSlot>` â€” the wrapper IS the uninit story, so `new_uninit().assume_init()` is a single coherent allocation. See Â§5.1 (updated). |
| **Q5** â€” `SystemMeta` size math | Quality | Corrected: 192 B Access + 16 B name + 16 B (2 Ã— gen) = 224 B â†’ no padding needed â†’ 4 cache lines. See Â§11.1 (corrected). |
| **Q6** â€” `pub mod resource_registry` over-exposes | Quality | Changed to `pub(crate) mod resource_registry`. Public surface re-exports the needed symbols via `core/resources/mod.rs`. See Â§11 (updated). |
| **Q7** â€” `Box::new` in `insert` | Quality | Acceptable cold-path, no change. |

## Changes from Round 2 (Round 3 deltas)

| Finding | Severity | Resolution |
|---------|----------|------------|
| **C-NEW-1** — §16.5 audit conclusion was **factually wrong**: `add_archetype` DOES have a replace path with `drop_in_place + ptr::write` (archetype_bundle.rs:412-436) | Critical | **Patch Phase 7 in this phase.** §16.5 rewritten to acknowledge the replace path. New **Step 12** body changed from "audit-only" to "apply clear-bit-first to `add_archetype` replace path". The reorder (with C-R3-1 correction): (1) cache `slot_idx`; (2a) clear the `occupied` bit BEFORE drop (THIS gates `Drop`'s walk); (2b) clear `id_to_slot[raw_id] = NO_SLOT`; (3) `drop_in_place(slot_ptr)`; (4) `ptr::write(slot_ptr, archetype)`; (5a) re-set the `occupied` bit; (5b) re-set `id_to_slot[raw_id] = slot_idx`. If `drop_in_place` panics, `ArchetypeBundle::Drop` walks `self.occupied` (not `id_to_slot`) and skips the cleared bit → no double-drop. Adds a new invariant **AB-R1** to §10. |
| **C-R3-1** — Round 2 fix cleared only `id_to_slot`, but `ArchetypeBundle::Drop` walks `self.occupied` (not `id_to_slot`) — fix was incomplete and would have left the double-drop UB | Critical | **Add `occupied` bitset clear-bit-first** to the protocol (Steps 2a + 5a above). Matches `remove_archetype` (lines 477-505) which clears both structures atomically. `count` field intentionally not modified during the drop window — brief inconsistency with `occupied` is only observable via `len()`, which is unused by `Drop` and read paths. AB-R1 updated; Step 12 acceptance test extended to verify the `occupied` bit is cleared after the panic-caught replace attempt. |
| **C-NEW-2** â€” `compile_error!` in Â§7.5 stub impls fires at macro-expansion time, not at monomorphization â€” the entire crate would refuse to compile | Critical | **Replace with `const { panic!(...) }`** in the stub method bodies. `const { panic!(...) }` is a const block (stable since Rust 1.79); it is only evaluated when the impl is *monomorphized* (i.e., when the user actually writes `(P0..P12)` or larger). Verified with rust-lang RFC 2345 stabilization track. Adds a `// rustc >= 1.79` comment in Â§7.5. boyko targets Rust 2024 edition (1.85+), so this is unconditionally available. |
| **C-NEW-3** â€” `MaybeUninit::assume_init()` takes `self` by value; cannot be called on `self.slots[idx].slot` (place-expression, cannot move out of array) | Critical | **Mark `ResourceSlot: Copy` + use `assume_init_read()`.** All three fields (`*mut u8`, `Option<unsafe fn>`, `Layout`) are already `Copy`. `assume_init_read()` reads the slot bitwise and leaves the underlying bytes untouched. Soundness: a `Box::into_raw` pointer has unique ownership semantically, but bitwise copies do not violate this because the *protocol* ensures only one logical owner uses the pointer after the read (the `registered_mask` bit is cleared in the same code path). Updated Â§5.1.3 at all three sites: `Resources::insert` replace branch, `Resources::remove`, `Resources::Drop`. |
| **W1** â€” `Res::get_param` calls `resources.get_ptr::<R>()` which re-resolves `R::resource_id()` (OnceLock load), ignoring the cached `state.id` | Major | **Add `Resources::get_ptr_by_id(id: ResourceId) -> Option<*const u8>`** (untyped). Modify `Res::get_param` to call this with `state.id` directly and cast `*const u8 â†’ *const R`. Same change to `ResMut::get_param` via `get_mut_ptr_by_id`. Updated Â§5.1.3, Â§6.1, Â§14.1. |
| **W2** â€” `bit_owners` size math wrong: `&'static str` is a fat pointer (16 B), not 8 B | Major | **Corrected to 24 KB.** Updated Â§4.6 from "12 KB" â†’ "24 KB"; updated Â§11.1 sizing row. No behavioural change â€” init-time transient heap allocation, still well under any meaningful budget. |
| **W3** â€” `run_closure_once` smoke test in Â§10 still requires turbofish | Major | **Acknowledged honestly.** Updated Â§8.1 wording: "turbofish required in 8a; Phase 8c's `IntoSystem` eliminates it." The smoke test in Step 10 keeps the turbofish; this is the truthful state of the API in 8a. Added a comment in the smoke test explaining the planned 8c improvement. |
| **W4** â€” M6 component/resource exclusivity check has TOCTOU race in multi-threaded registration | Major | **Document single-threaded registration constraint.** The `ComponentRegistry` and `ResourceRegistry` already de facto serialize via `OnceLock::set`; the M6 reverse-lookup is best-effort. Updated Â§5.1.2 doc-comment on `register_resource_new`: "The Component-vs-Resource check is best-effort: it assumes registration is single-threaded (matches the de-facto pattern of `#[derive]`-generated lazy init). Concurrent registration of the same TypeId as both Component and Resource is not defended against; this is a `should-not-happen` constraint of the registration model." |
| **W5** â€” Plan references `archetype_master.id_generation()` but the existing method is `archetype_generation()` | Major | **Use the existing name throughout.** Replaced all 4 references in Â§13.5, Â§8.4 (`FnOnceSystem::initialize`), and Step 0. No rename; no API change. |

---

## 1. Goal and target metrics

### 1.1 Goal

Deliver the **trait scaffolding** for boyko's ergonomic system API plus a complete `Resource` subsystem. Every Phase 8a artefact must compose with the Phase 7 fast read path (`get_component_raw` ~3 ns) and avoid introducing any new dispatch indirection on hot paths.

Exit criteria for 8a:

1. The `SystemParam` trait (Bevy-shape, GAT-based, two-phase init) exists and compiles.
2. Tuple impls for `SystemParam` cover arity 0..=12 via a single `macro_rules!` site located in `boyko_ecs`.
3. `Res<T>` and `ResMut<T>` are first-class `SystemParam`s backed by a real `Resources` subsystem.
4. `Resource: 'static + Send + Sync` trait + `#[derive(Resource)]` in `boyko_macros`.
5. `UnsafeEcsCell<'w>` is defined with SAFETY contract, **by-value method receivers**, and consumed by the `get_param` hot path.
6. `EcsMaster::run_system_once<S: System>` runs an `S: System` end-to-end.
7. `SystemMeta` + `Access` + `FilteredAccessSet` exist with enough fields to support Phase 9 scheduler conflict detection.
8. Intra-system access conflict detection works at `init_access` time (catches `Res<X> + ResMut<X>` in the same system).
9. No `dyn Trait`, no `Box<dyn Trait>`, no `HashMap`, no `Mutex`/`RwLock`/`RefCell` on the system body hot path.
10. `cargo test --all-targets` green; new unit tests for `Resources`; integration test running an end-to-end system through `run_system_once`.
11. **Phase 7 `add_archetype` replace path patched** with the clear-bit-first protocol (C-NEW-1 RESOLUTION).

### 1.2 Target metrics (release, AMD Zen3 / Intel Alder Lake)

| Operation | Target | Cache profile |
|-----------|--------|---------------|
| `Res<T>` `get_param` (read) | â‰¤ 3 ns (1 cache line: `Resources::slots[id.0]` load + cast) | 1 L1d hit |
| `ResMut<T>` `get_param` (write) | â‰¤ 3 ns (same path, `*mut` cast) | 1 L1d hit |
| Empty-param system call via `run_system_once` | â‰¤ 5 ns dispatch overhead | call + return only |
| Tuple of 4 mixed `SystemParam`s `get_param` | â‰¤ 4 Ã— per-param + â‰¤ 2 ns tuple overhead | linear, no cache miss above per-param hits |
| `Resources::insert::<R>` (cold path) | â‰¤ 200 ns | not budgeted |
| `Resources::Drop` walking 64 occupied resources | â‰¤ 2 Âµs | bitset-driven |
| `init_access` conflict check (per param) | â‰¤ 50 ns | 1 BitSet256 intersect + branch |

Phase 8a is API design; benches written but only spot-checked. Full bench discipline is Phase 8d.

### 1.3 Cross-phase relation to perf

Phase 7 fast read path (`get_component_raw` ~3 ns) is the absolute lower bound for any per-row work on a query. Phase 8a's `Res<T>` access must not be slower than Phase 7's per-row component access; Phase 9 scheduler will multiply system count Ã— per-system param-fetch cost into the critical-path budget.

---

## 2. Context and constraints

### 2.1 Subsystems affected

| Subsystem | Touch type |
|-----------|-----------|
| `EcsMaster` | New facade methods: `insert_resource`, `remove_resource`, `resource`, `resource_mut`, `run_system_once`, `run_closure_once`. New `resources: Resources` field. Drop order updates. |
| `Resources` (new) | New module under `core/resources/` (mirrors `core/events/`). |
| `boyko_macros` | New `#[derive(Resource)]` mirroring `#[derive(Component)]`. |
| `EntityMaster`, `ArchetypeMaster`, `Archetype`, `EventDispatcher` | No change in 8a. |
| `Query<'a>` (existing) | Untouched. Bevy-style `Query<D, F>` `SystemParam` lives in Phase 8b. |
| `ArchetypeBundle::add_archetype` (Phase 7 file) | **Replace path retroactively patched** to apply the C-NEW-1 clear-lookup-first protocol (see Â§16.5). This is a Phase 7 bug carry-over fixed in 8a scope. |

### 2.2 Invariants that must be preserved + drop order (C5 RESOLUTION)

- **U1â€“U14 from Phase 7** (slab stability, generation match, pointer minting, drop discipline).
- **`EcsMaster` field order â€” REVISED for Round 2:** `resources â†’ events â†’ entity_master â†’ archetype_master â†’ arena`.
  - `resources` drops **first**. Rationale: a `Resource`'s `Drop` impl runs while every other subsystem is still alive. If user code violates the contract (touches the world from `Drop`), the world is fully valid. The most-defensive position prevents the worst-case from being UB.
  - `events` drops next: event buffers are independent heap allocations and contain no arena pointers.
  - `entity_master â†’ archetype_master`: entity slab references archetype indices via inland storage.
  - `arena` drops last: archetype columns hold `*const Arena` pointers (Phase 3a invariant).
- **Documented `Resource::Drop` contract:** the `Resource` trait doc-comment explicitly states:
  > **Drop discipline.** A `Resource`'s `Drop` impl MUST NOT call back into `EcsMaster` â€” no `EcsMaster::insert_*`, `remove_*`, `resource*`, `run_system_once`, no archetype/entity queries. The world is mid-teardown when `Drop` runs and only the resource itself is guaranteed to be valid. Violations are detectable in debug builds via re-entrancy guards (Phase 9).
- **`EcsMaster: !Send + !Sync`** preserved.
- **`Resource: Send + Sync` (NOT `!Send + !Sync`)** â€” Phase 9 scheduler crosses thread boundaries.

### 2.3 Hard prohibitions on the hot path

| Forbidden | Why | Allowed substitute |
|-----------|-----|--------------------|
| `Box<dyn SystemParam>` | virtual dispatch | monomorphisation via `SystemParam` impl per concrete tuple |
| `Box<dyn Any>` per resource lookup | virtual dispatch | type-erased `*mut u8` + cached `ResourceId` |
| `HashMap<TypeId, Box<dyn Any>>` | hash cost + indirection | `[ResourceSlotStorage; RESOURCE_SLOT_COUNT]` indexed by `ResourceId.0` |
| `Mutex<Resources>` | lock cost | `&self` / `&mut self` borrow on `EcsMaster` is the synchronisation point |
| `RefCell<Resources>` | runtime borrow check cost | `UnsafeEcsCell` + scheduler-checked access |
| Any allocation per `get_param` | per-frame allocation kills budget | param state pre-allocated in `init_state` |

### 2.4 Variadic arity ceiling

Pick **12**, not Bevy's 16. Rationale:
- Real systems with > 8 params are rare; 12 covers the long tail without bloating compile-times.
- 12 monomorphisations Ã— 4 traits (`SystemParam`, `QueryData` (8b), `QueryFilter` (8b), `SystemParamFunction` (8c)) = 48 tuple impls â€” half Bevy's 64. Saves ~10-15 % `cargo build` time per researcher's measurement.
- 12 aligns with x86_64 register-passing limits.
- **M7 RESOLUTION + C-NEW-2 update**: arities 13..=24 get `const { panic!(...) }`-bearing stub impls so the user gets a clear diagnostic instead of a deep trait-resolution error tail. See Â§7.5.

The ceiling is `MAX_SYSTEM_PARAM_ARITY = 12` in the macro file.

---

## 3. Decision D1 â€” World-access abstraction: `UnsafeEcsCell<'w>` (REWRITTEN â€” C1 resolution)

### 3.1 Decision

Introduce **`UnsafeEcsCell<'w>`** â€” a `Copy` newtype wrapping `*mut EcsMaster` with a phantom lifetime `'w` carrying the master's borrow scope. **Every method takes `self` by value** (not `&self` / `&mut self`). This is the canonical Bevy shape (`UnsafeWorldCell::world_mut(self) -> &'w mut World`) and is the load-bearing element of the C1 resolution.

**Why by-value receivers fix Tree Borrows retag UB:** when a method takes `&self`, the receiver creates an `&UnsafeEcsCell` borrow on the cell, which under Tree Borrows tags the *interior pointer* `ptr` as SharedReadOnly for the call duration. Any `*mut` derived from that pointer inside the method body inherits the SharedReadOnly capability and **cannot** be written through â€” even if the original `*mut EcsMaster` was write-capable. By-value receivers consume a `Copy` of the cell; the pointer flows directly through the method without any intermediate `&self` borrow that could downgrade its capability.

```rust
// File: crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs (new)
//
// A `Copy` interior-mutable cell pointing at `EcsMaster` for the borrow
// scope `'w`. Methods take `self` by value (Bevy `UnsafeWorldCell` shape):
// this is load-bearing under Tree Borrows because `&self` would retag the
// interior pointer as SharedReadOnly for the call duration.

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct UnsafeEcsCell<'w> {
    ptr: *mut EcsMaster,
    // PhantomData carrying both `&'w EcsMaster` (variance) and the
    // `&'w UnsafeCell<EcsMaster>` interior-mutability marker â€” same shape
    // as Bevy's UnsafeWorldCell. Combined with `Copy` + by-value receivers,
    // this is the canonical raw-pointer-based interior-mutability pattern.
    _marker: PhantomData<(&'w EcsMaster, &'w core::cell::UnsafeCell<EcsMaster>)>,
    // Debug-only sentinel: cells minted from `&mut EcsMaster` carry
    // `true`, from `&EcsMaster` carry `false`. `world_mut` debug-asserts
    // `true`. Bevy uses this exact pattern (`allows_mutable_access`).
    #[cfg(debug_assertions)]
    allows_mutable_access: bool,
}

impl<'w> UnsafeEcsCell<'w> {
    /// Mint a write-capable cell from `&mut EcsMaster`.
    ///
    /// SAFETY: the cell must not outlive `'w`. The resulting cell can be
    /// freely `Copy`'d and passed through tuple impls; aliasing discipline
    /// is enforced at the `SystemParam::init_access` level (see SP1, U_C2).
    #[inline]
    pub(crate) unsafe fn new_mutable(world: &'w mut EcsMaster) -> Self {
        Self {
            ptr: world as *mut EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            allows_mutable_access: true,
        }
    }

    /// Mint a read-only cell from `&EcsMaster`.
    ///
    /// SAFETY: cell must not outlive `'w`. `world_mut()` calls on this cell
    /// will `debug_assert!` and panic.
    #[inline]
    pub(crate) unsafe fn new_readonly(world: &'w EcsMaster) -> Self {
        Self {
            // Cast through `*const â†’ *mut` is fine: the cell's
            // `allows_mutable_access = false` guards against any
            // `world_mut()` use; only `world()` is reachable.
            ptr: world as *const EcsMaster as *mut EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            allows_mutable_access: false,
        }
    }

    /// Returns a shared reference to the master.
    ///
    /// SAFETY (U_C2): caller asserts they hold a `&` borrow per the
    /// `SystemParam` `init_access` protocol; no `&mut` aliases the read.
    #[inline]
    pub(crate) unsafe fn world(self) -> &'w EcsMaster {
        // SAFETY: by-value receiver consumes a Copy of the cell; the raw
        //   pointer is dereferenced without any `&self` borrow that would
        //   downgrade its provenance to SharedReadOnly. Validity of `ptr`
        //   for `'w` is the new_*() postcondition.
        unsafe { &*self.ptr }
    }

    /// Returns an exclusive reference to the master.
    ///
    /// SAFETY (U_C3): caller asserts they hold a `&mut` borrow per the
    /// `SystemParam` `init_access` protocol; no other access through
    /// any cell copy aliases the write.
    #[inline]
    pub(crate) unsafe fn world_mut(self) -> &'w mut EcsMaster {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.allows_mutable_access,
            "invariant: world_mut() called on a readonly UnsafeEcsCell minted via new_readonly"
        );
        // SAFETY: by-value receiver consumes a Copy of the cell; the raw
        //   pointer remains write-capable because no `&self` reborrow
        //   downgrades it. Aliasing is the caller's responsibility per
        //   the SystemParam protocol.
        unsafe { &mut *self.ptr }
    }

    /// Direct read-only access to the resources subsystem. Hot path for
    /// `Res<R>::get_param` â€” avoids the full `world()` materialisation
    /// when only the resources slab is needed.
    ///
    /// SAFETY (U_C2): caller asserts `init_access` declared a resource
    /// read; no `&mut Resources` aliases through any cell copy.
    #[inline]
    pub(crate) unsafe fn resources(self) -> &'w Resources {
        // SAFETY: by-value receiver; raw pointer not retagged.
        //   `&(*self.ptr).resources` projects through the raw pointer
        //   without going through an `&EcsMaster` reference (we never
        //   construct `&*self.ptr` as a temporary â€” the `&` operator
        //   applies directly to the projected field).
        unsafe { &(*self.ptr).resources }
    }

    /// Direct mutable access to the resources subsystem. Hot path for
    /// `ResMut<R>::get_param`.
    ///
    /// SAFETY (U_C3): caller asserts `init_access` declared a resource
    /// write; no other access through any cell copy aliases.
    #[inline]
    pub(crate) unsafe fn resources_mut(self) -> &'w mut Resources {
        #[cfg(debug_assertions)]
        debug_assert!(self.allows_mutable_access);
        // SAFETY: by-value receiver; raw pointer write-capable.
        //   `&mut (*self.ptr).resources` projects through the raw pointer
        //   without an intermediate `&mut EcsMaster` reborrow.
        unsafe { &mut (*self.ptr).resources }
    }

    /// Mints a `*mut Archetype` for `id` using the Phase 7 U11 recipe.
    /// By-value receiver ensures the pointer minting retains write
    /// capability â€” this is the C1 fix for Round 1's `&self` retag bug.
    ///
    /// SAFETY (U_C3): caller asserts archetype write access was declared.
    #[inline]
    pub(crate) unsafe fn archetype_ptr_mut(self, id: ArchetypeId) -> Option<*mut Archetype> {
        #[cfg(debug_assertions)]
        debug_assert!(self.allows_mutable_access);
        // SAFETY: by-value receiver; the raw pointer to EcsMaster has
        //   never been retagged. Calling `archetype_master_mut()`
        //   through `world_mut()` is sound because `world_mut(self)`
        //   takes `self` by value and produces a fresh `&mut` from the
        //   write-capable raw pointer.
        unsafe { self.world_mut().archetype_master_mut().archetype_ptr_for(id) }
    }

    /// Read-only counterpart (Phase 7 U11 read-only recipe).
    #[inline]
    pub(crate) unsafe fn archetype_ptr(self, id: ArchetypeId) -> Option<*const Archetype> {
        // SAFETY: by-value receiver; read-only projection.
        unsafe { self.world().archetype_master().get_archetype_ptr(id) }
    }
}

// !Send + !Sync in 8a. Phase 9 will add `unsafe impl Send/Sync` with
// the explicit scheduler aliasing-discipline contract.
impl<'w> !Send for UnsafeEcsCell<'w> {}
impl<'w> !Sync for UnsafeEcsCell<'w> {}
```

### 3.2 Alternatives rejected

**(a) `&mut EcsMaster` directly to `get_param`.** Rejected because the tuple impl for `(Res<A>, Res<B>)` needs to call `<Res<A>>::get_param` and `<Res<B>>::get_param` from the same scope â€” Rust's borrow checker forbids two `&mut EcsMaster`s alive simultaneously.

**(b) `&self`/`&mut self` receivers on `UnsafeEcsCell` methods.** Rejected â€” Round 1's design. Under Tree Borrows, `&self` retags the interior `ptr` field as SharedReadOnly for the call duration, downgrading any pointer derived from it. This is the C1 bug. By-value receivers (Bevy's choice) eliminate the retag entirely.

**(c) Splay into sub-cells.** Rejected â€” Phase 8b `Query` spans entities + archetypes; splitting forces every `SystemParam` to know which sub-cell it cares about. Bevy collapsed this for a reason.

**(d) `&UnsafeCell<EcsMaster>` instead of raw pointer.** Reconsidered per C1 critic note. Rejected because Bevy's actual implementation uses a raw pointer + `PhantomData<(&'w World, &'w UnsafeCell<World>)>`, NOT an `&UnsafeCell<World>`. The `UnsafeCell` is in the `PhantomData` marker (for variance + drop check), not in a real field. By-value receivers achieve the same Tree Borrows safety without forcing `EcsMaster` itself to be wrapped in `UnsafeCell` (which would force every existing `&EcsMaster` method through `.get()`).

### 3.3 Trade-off

We pay one `unsafe` block per `get_param` impl (with enforced SAFETY comment). The cell is `Copy`, so tuple impls can `Copy` it freely â€” no borrow-checker contortions. Phase 9 scheduler must encode the aliasing discipline correctly. In exchange: monomorphised, allocation-free, lifetime-clean tuple fetching that the borrow checker would otherwise refuse.

### 3.4 Why this is faster

- `Res<T>` access: load `ResourceId` from `Self::State`, copy `UnsafeEcsCell` (1 register), load `Resources::slots[id.0].ptr` from `&EcsMaster::resources`, cast to `&T`. Same shape as Phase 7's column read.
- Tuple impl for `(Res<A>, Res<B>, Res<C>)`: the cell is `Copy`; each `get_param` receives its own copy. The compiler lifts the `(*self.ptr).resources` load out of all three calls (CSE) â€” net effect 1 load + 3 indexed reads.
- No virtual dispatch anywhere.

---

## 4. Decision D2 â€” Access tracking: `Access` + `FilteredAccessSet` (M1 + C4 + M8 resolution)

### 4.1 Decision â€” `Access` shape (M1 RESOLUTION)

Introduce **`Access`** â€” a 4-bitset struct sized **192 B** (down from 256 B in Round 1) â€” and **`SystemMeta`** â€” per-system context carrying `Access`, name, and observed generations.

**Write-once contract:** `Access` is mutated *only* during `SystemParam::init_access` calls, which happen once per system per `EcsMaster`. After that it is read-only forever. Phase 9 scheduler reads it; never writes. **No false-sharing risk â†’ no `align(64)`, no padding.**

```rust
// File: crates/boyko_ecs/src/ecs/core/system/access.rs (new)
//
// Per-system aliasing summary. Phase 9 scheduler reads this to build
// the conflict graph. Phase 8a populates it but does NOT consume it
// for scheduling. Write-once: filled during init_access, read-only
// thereafter (no `align(64)` / padding â€” see M1 resolution).
#[repr(C)]
pub struct Access {
    /// Components this system reads. 64 B (ComponentMask = BitSet512).
    pub(crate) component_reads: ComponentMask,
    /// Components this system writes. 64 B.
    pub(crate) component_writes: ComponentMask,
    /// Resources this system reads. 32 B (BitSet256).
    pub(crate) resource_reads: BitSet256,
    /// Resources this system writes. 32 B.
    pub(crate) resource_writes: BitSet256,
    // Total: 64 + 64 + 32 + 32 = 192 B; 3 cache lines (no padding).
}

impl Access {
    pub const fn new() -> Self {
        Self {
            component_reads: ComponentMask::new(),
            component_writes: ComponentMask::new(),
            resource_reads: BitSet256::new(),
            resource_writes: BitSet256::new(),
        }
    }

    #[inline] pub fn add_component_read(&mut self, id: ComponentId) { self.component_reads.set(id); }
    #[inline] pub fn add_component_write(&mut self, id: ComponentId) { self.component_writes.set(id); }
    #[inline] pub fn add_resource_read(&mut self, id: ResourceId) { self.resource_reads.set(id.0 as u32); }
    #[inline] pub fn add_resource_write(&mut self, id: ResourceId) { self.resource_writes.set(id.0 as u32); }

    /// CROSS-SYSTEM conflict check (Phase 9 scheduler use only).
    /// Returns `true` iff `self` and `other` cannot execute concurrently.
    ///
    /// NOTE (M8 RESOLUTION): this predicate trivially returns `true` for
    /// `self.conflicts_with(self)` if either system has any write â€” that
    /// is intentional and correct for cross-system checks (the same
    /// system cannot run twice in parallel against itself). For
    /// intra-system (one system, many params) conflict detection, see
    /// `FilteredAccessSet` (Â§4.5).
    pub fn conflicts_with(&self, other: &Access) -> bool {
        let cw_vs_cr = self.component_writes.intersects(&other.component_reads);
        let cr_vs_cw = self.component_reads.intersects(&other.component_writes);
        let cw_vs_cw = self.component_writes.intersects(&other.component_writes);
        let rw_vs_rr = self.resource_writes.intersects(&other.resource_reads);
        let rr_vs_rw = self.resource_reads.intersects(&other.resource_writes);
        let rw_vs_rw = self.resource_writes.intersects(&other.resource_writes);
        cw_vs_cr || cr_vs_cw || cw_vs_cw || rw_vs_rr || rr_vs_rw || rw_vs_rw
    }
}
```

### 4.2 `SystemMeta` shape

```rust
// File: crates/boyko_ecs/src/ecs/core/system/system_meta.rs (new)
#[repr(C)]
pub struct SystemMeta {
    /// Read/write surface declared by the system's parameters.
    /// Filled during init_access; read-only thereafter (write-once).
    pub(crate) access: Access,
    /// Diagnostic name (set at registration; defaults to type_name).
    pub(crate) name: &'static str,
    /// Last `archetype_generation` observed; Phase 8b `Query::new_archetype`
    /// uses this to know when to refresh.
    pub(crate) last_archetype_generation: ArchetypeGeneration,
    /// Last `structural_generation`. Same pattern.
    pub(crate) last_structural_generation: ArchetypeGeneration,
}

impl SystemMeta {
    pub fn new(name: &'static str) -> Self {
        Self {
            access: Access::new(),
            name,
            last_archetype_generation: ArchetypeGeneration::FIRST,
            last_structural_generation: ArchetypeGeneration::FIRST,
        }
    }
}
```

### 4.3 Alternatives rejected

**(a) `Vec<ComponentId>` per access kind.** Rejected â€” allocates, requires linear scan. Bitset is O(1) intersect via 8 Ã— u64 AND.

**(b) `[bool; MAX_COMPONENTS]`.** Rejected â€” 512 B per direction Ã— 2 directions = 1 KB per `Access`, doesn't fit in 2 cache lines.

**(c) Splay `Access` per resource/component/event into separate structs.** Rejected â€” Phase 9 conflict check wants a single `conflicts_with` call.

### 4.4 Trade-off

`Access` is 192 B (down from Round 1's 256 B). Per-system memory cost is ~192 B; for 100 systems = 19.2 KB total â€” irrelevant. Conflict check is 6 Ã— u64-AND-then-test = ~10 cycles.

### 4.5 `FilteredAccessSet` â€” intra-system conflict detector (C4 + M8 RESOLUTION)

**New struct** introduced for the C4 + M8 resolution. Mirrors Bevy's `FilteredAccessSet` but simplified for Phase 8a (no per-component filters; Phase 8b adds those).

```rust
// File: crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs (new)
//
// Accumulator passed through SystemParam::init_access so siblings can
// detect aliasing conflicts BEFORE registration completes. C4 + M8
// resolution: this is the intra-system check that `Access::conflicts_with`
// cannot perform.

#[derive(Debug, Clone, Copy)]
pub struct AccessConflict {
    pub kind: ConflictKind,
    pub id: u32,
    pub existing_param: &'static str,
    pub new_param: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum ConflictKind {
    ResourceReadVsWrite,
    ResourceWriteVsRead,
    ResourceWriteVsWrite,
    ComponentReadVsWrite,
    ComponentWriteVsRead,
    ComponentWriteVsWrite,
}

#[repr(C)]
pub struct FilteredAccessSet {
    /// Running aggregate of all params declared so far in this system.
    combined: Access,
    /// Per-param name of the param that registered each currently-set
    /// access bit. Used for diagnostic. Heap-allocated, populated once at
    /// init time (NOT hot path).
    /// Indexed by bit index (component_reads gets indices 0..512,
    /// component_writes 512..1024, resource_reads 1024..1280,
    /// resource_writes 1280..1536).
    bit_owners: Box<[&'static str; 1536]>,
}

impl FilteredAccessSet {
    pub fn new() -> Self {
        Self {
            combined: Access::new(),
            bit_owners: Box::new([""; 1536]),
        }
    }

    /// Adds a resource-read declaration. Returns `Err` if a previous
    /// `add_resource_write` already claimed this resource in this set.
    pub fn add_resource_read(
        &mut self,
        id: ResourceId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0 as u32;
        if self.combined.resource_writes.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceReadVsWrite,
                id: idx,
                existing_param: self.bit_owners[(1280 + idx) as usize],
                new_param: param_name,
            });
        }
        self.combined.resource_reads.set(idx);
        self.bit_owners[(1024 + idx) as usize] = param_name;
        Ok(())
    }

    /// Adds a resource-write declaration. Returns `Err` if a previous
    /// `add_resource_read` OR `add_resource_write` already claimed it.
    pub fn add_resource_write(
        &mut self,
        id: ResourceId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0 as u32;
        if self.combined.resource_reads.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceWriteVsRead,
                id: idx,
                existing_param: self.bit_owners[(1024 + idx) as usize],
                new_param: param_name,
            });
        }
        if self.combined.resource_writes.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceWriteVsWrite,
                id: idx,
                existing_param: self.bit_owners[(1280 + idx) as usize],
                new_param: param_name,
            });
        }
        self.combined.resource_writes.set(idx);
        self.bit_owners[(1280 + idx) as usize] = param_name;
        Ok(())
    }

    // add_component_read / add_component_write follow the same pattern
    // and are elided here for brevity. Implementation: Â§17 Step 4.

    /// Finalises the set: copies the accumulated `Access` into `meta`.
    /// Called by the tuple impl after every param's `init_access` returns.
    #[inline]
    pub fn finalize(self, meta: &mut SystemMeta) {
        meta.access = self.combined;
    }
}
```

The conflict diagnostic message at panic:
```text
error[boyko-B0002]: Resource `Tick` has conflicting access in the same system.
   Existing access: ResMut<Tick> (declared by param 0)
   Conflicting:     Res<Tick>    (declared by param 2)
This would be UB at runtime. Remove one of the accesses or use the same
mutability for both.
```

### 4.6 Why FilteredAccessSet is a separate struct, not folded into Access

`Access` is the **final summary** stored on `SystemMeta`. It does not need per-bit ownership tracking after init completes. `FilteredAccessSet` carries the heavyweight ownership map only during the brief init window and discards it via `finalize`. This keeps `SystemMeta` at 192 B summary + 16 B name + 16 B gens = 224 B; the per-init transient `FilteredAccessSet` is **~24 KB heap (1536 Ã— 16 B `&'static str` fat pointers)** â€” paid once per system per `init_access` call, then freed. **(W2 RESOLUTION: corrected from "12 KB" â€” `&'static str` is a 16 B fat pointer, not 8 B.)**

---

## 5. Decision D3 â€” Resource subsystem layout (M3, M6, Q4 + C3 resolution)

### 5.1 Decision â€” file layout

Introduce a **`Resources`** subsystem mirroring the `EventDispatcher` slab pattern:

- `RESOURCE_SLOT_COUNT = 256` constant (M3 RESOLUTION: renamed from `MAX_RESOURCES`).
- `ResourceId(usize)` newtype via `define_id!` (consistent with `EntityId`, `ArchetypeId`, etc.).
- Static `RESOURCE_INFO: [OnceLock<ResourceInfo>; RESOURCE_SLOT_COUNT]` registry.
- `Resources` storage: `Box<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]>` slab + `BitSet256 registered_mask`.
- Slab built via `Box::<[ResourceSlotStorage; N]>::new_uninit().assume_init()` (Q4 RESOLUTION â€” mirror of `ArchetypeBundle::new`; the `MaybeUninit<ResourceSlot>` inside each storage cell IS the uninit story).
- Per-`Resource` type registration via `#[derive(Resource)]` lazy `OnceLock`.
- Instances allocated via `Box::<R>::into_raw` (cold-construction, sparse).

#### 5.1.1 `Resource` trait (M2 RESOLUTION + C5 contract)

```rust
// File: crates/boyko_ecs/src/ecs/core/resources/resource.rs (new)

/// Marker trait for ECS resource types â€” `World`-global singletons.
///
/// Implemented automatically via `#[derive(Resource)]`. Each type gets a
/// unique [`ResourceId`] assigned on first call to [`resource_id`].
///
/// # `Send + Sync` requirement
/// Resources are read/written by SystemParam closures across multiple
/// systems. Phase 9 scheduler will run systems on multiple threads; a
/// non-Sync resource would be unsound to `Res<&'w T>` from a worker. The
/// bound matches Bevy.
///
/// **Future migration path (Phase 9 Â§9.4):** types that legitimately cannot
/// be `Send + Sync` (e.g., FFI handles, `Rc<T>`-wrapped state) will be
/// supported via a separate `NonSendResource` trait + `NonSendRes<T>` param.
/// Phase 8a does NOT ship this â€” track in `docs/plans/PHASE-09-scheduler.md`.
///
/// # Drop discipline (C5)
/// A `Resource`'s `Drop` impl MUST NOT call back into `EcsMaster` â€” no
/// `EcsMaster::insert_*`, `remove_*`, `resource*`, `run_system_once`, no
/// archetype/entity queries. The world is mid-teardown when `Drop` runs and
/// only the resource itself is guaranteed to be valid. Violations are
/// detectable in debug builds via re-entrancy guards (Phase 9 Â§9.5).
///
/// # Component-vs-Resource exclusivity (M6)
/// A type may not be both `#[derive(Component)]` and `#[derive(Resource)]`.
/// The runtime registration at `register_resource_new` panics with a clear
/// diagnostic if the type is already registered as a Component.
///
/// # Panic safety
/// `<Self as Drop>::drop` must not panic. If it does, `Resources::Drop`
/// (or `insert` replace path) clears the registered_mask bit BEFORE
/// calling drop, so the observable state on unwind is "slot empty"
/// (leak rather than UB). See R4.
pub trait Resource: 'static + Send + Sync + Sized {
    fn resource_id() -> ResourceId;

    #[inline] fn debug_type_name() -> &'static str { std::any::type_name::<Self>() }
    #[inline] fn type_id() -> TypeId { TypeId::of::<Self>() }
    #[inline] fn mem_size() -> usize { std::mem::size_of::<Self>() }
    #[inline] fn alignment() -> usize { std::mem::align_of::<Self>() }
}
```

#### 5.1.2 `resource_registry` (M6 RESOLUTION + W4 update)

```rust
// File: crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs (new)

pub const RESOURCE_SLOT_COUNT: usize = 256;
// Locked to BitSet256 width. Raising this requires a wider bitset
// (Phase 9+ backlog: parameterizable BitSetN<N>).

pub type ResourceDropFn = unsafe fn(*mut u8);

/// SAFETY contract identical to ComponentRegistry::DropFn (M-001).
#[inline]
pub(crate) unsafe fn resource_drop_in_place_glue<R: 'static>(ptr: *mut u8) {
    // SAFETY: caller upholds the DropFn contract: ptr aligned + initialised,
    //   exclusively owned, not accessed again after this call.
    unsafe { core::ptr::drop_in_place::<R>(ptr.cast::<R>()) }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct ResourceInfo {
    pub size: usize,                          // hot
    pub alignment: usize,                     // hot
    pub drop_fn: Option<ResourceDropFn>,      // hot
    pub type_name: &'static str,              // cold
    pub type_id: TypeId,                      // cold
}

impl ResourceInfo {
    #[inline]
    pub fn new_static<R: Resource>() -> Self {
        Self {
            size: std::mem::size_of::<R>(),
            alignment: std::mem::align_of::<R>(),
            drop_fn: if std::mem::needs_drop::<R>() {
                Some(resource_drop_in_place_glue::<R> as ResourceDropFn)
            } else { None },
            type_name: std::any::type_name::<R>(),
            type_id: TypeId::of::<R>(),
        }
    }
}

static RESOURCE_INFO: [OnceLock<ResourceInfo>; RESOURCE_SLOT_COUNT] =
    [const { OnceLock::new() }; RESOURCE_SLOT_COUNT];
static NEXT_RESOURCE_ID: AtomicUsize = AtomicUsize::new(0);

/// Mirror of `component_registry::register_new`. M6 RESOLUTION: also checks
/// the component registry for collision and panics on overlap.
///
/// W4 RESOLUTION â€” single-threaded registration constraint:
/// The Component-vs-Resource check is **best-effort**: it assumes
/// registration is single-threaded (matches the de-facto pattern of
/// `#[derive]`-generated lazy init via `OnceLock`). Concurrent registration
/// of the same `TypeId` as both Component and Resource from two threads
/// is not defended against. This is acceptable because:
///   1. The `OnceLock`-based registration model already implicitly assumes
///      single-threaded first-touch (the engine's startup phase).
///   2. A type is statically annotated with EITHER `#[derive(Component)]`
///      OR `#[derive(Resource)]` â€” never both â€” so the racy case requires
///      malicious code authoring, not user error.
///   3. Phase 9 will add a `world.is_single_threaded_phase()` guard if
///      we ever need stronger atomicity here.
pub fn register_resource_new<R: Resource>() -> usize {
    // M6 RESOLUTION: forbid double-registration as both Component and Resource.
    let type_id = TypeId::of::<R>();
    if crate::ecs::core::component::component_registry::is_type_registered_as_component(type_id) {
        panic!(
            "type `{}` is already registered as a Component; \
             a type cannot be both Component and Resource. \
             Remove one of #[derive(Component)] / #[derive(Resource)].",
            std::any::type_name::<R>()
        );
    }

    let raw = NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < RESOURCE_SLOT_COUNT,
        "ResourceRegistry exhausted: NEXT_RESOURCE_ID reached {raw}, RESOURCE_SLOT_COUNT = {RESOURCE_SLOT_COUNT}"
    );
    let info = ResourceInfo::new_static::<R>();
    match RESOURCE_INFO[raw].set(info) {
        Ok(()) => raw,
        Err(_) => {
            let existing = RESOURCE_INFO[raw].get()
                .expect("invariant: set Err implies occupied");
            if existing.type_id == type_id { raw }
            else {
                panic!(
                    "ResourceId {raw} occupied by type {}, refused to register {}",
                    existing.type_name, std::any::type_name::<R>()
                )
            }
        }
    }
}

/// Test-only escape hatch (mirrors `register_layout` for components).
#[doc(hidden)]
pub fn register_resource<R: Resource>(resource_id: usize) { /* mirrors register_layout */ }

#[inline]
pub fn get_resource_info(id: ResourceId) -> Option<&'static ResourceInfo> {
    RESOURCE_INFO.get(id.0)?.get()
}
```

**M6 supporting helper** (added to `component_registry`):

```rust
// Added to crates/boyko_ecs/src/ecs/core/component/component_registry.rs
//
// Reverse-lookup helper used by `register_resource_new` to enforce
// Component/Resource exclusivity. Walks the OnceLock table once; cost
// is O(MAX_COMPONENTS) at registration time only (never on hot path).
pub(crate) fn is_type_registered_as_component(type_id: TypeId) -> bool {
    for slot in COMPONENT_LAYOUTS.iter() {
        if let Some(info) = slot.get() {
            if info.type_id == type_id { return true; }
        }
    }
    false
}
```

#### 5.1.3 `Resources` storage + Drop (C3 + C-NEW-3 + W1 RESOLUTION)

```rust
// File: crates/boyko_ecs/src/ecs/core/resources/resources.rs (new)

/// One slot per registered resource type. 1 instance per type.
///
/// C-NEW-3 RESOLUTION: `Copy` derived so `MaybeUninit::assume_init_read()`
/// can pull the slot out of `slots[idx].slot` without moving the place.
/// All three fields are `Copy`:
///   - `ptr: *mut u8` is `Copy`,
///   - `Option<unsafe fn>` is `Copy` (function pointers are `Copy`),
///   - `Layout` is `Copy` (POD: `usize` size + `NonZeroUsize` align).
/// Soundness: the `Box::into_raw` ownership of `ptr` is preserved by the
/// protocol â€” the `registered_mask` bit is cleared in the same code path
/// that reads the slot, ensuring only one logical owner uses the pointer
/// after the bitwise copy.
#[derive(Clone, Copy)]
#[repr(C)]
struct ResourceSlot {
    /// `Box::into_raw` of the resource value, type-erased to `*mut u8`.
    ptr: *mut u8,
    /// Cached `drop_fn` from `ResourceInfo`. Stored locally so `Drop` does
    /// not have to hit the global registry on teardown.
    drop_fn: Option<ResourceDropFn>,
    /// Cached `Layout` for `dealloc` on remove/replace.
    layout: Layout,
}

const _: () = assert!(std::mem::size_of::<ResourceSlot>() <= 32);

#[repr(C)]
struct ResourceSlotStorage {
    /// MaybeUninit so the array can be heap-allocated without `Default`.
    /// Initialized iff `registered_mask.get(index) == true`.
    slot: MaybeUninit<ResourceSlot>,
}

pub struct Resources {
    /// 256 Ã— 32 B = 8 KB heap-allocated slab. Stable address by Box invariant
    /// (mirror of EventDispatcher::slots).
    slots: Box<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]>,
    /// Tracks which slots are initialised. 32 B.
    registered_mask: BitSet256,
}

impl Resources {
    /// Q4 RESOLUTION: Box::<T>::new_uninit().assume_init() pattern, mirror
    /// of ArchetypeBundle::new. The `MaybeUninit<ResourceSlot>` wrapper IS
    /// the uninit story â€” the outer Box just needs heap allocation.
    pub fn new() -> Self {
        // SAFETY: ResourceSlotStorage wraps MaybeUninit, so an
        //   uninitialised array of them is valid (the wrapper IS the
        //   uninit story). We never read a slot before its bit is set.
        let slots: Box<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]> = unsafe {
            Box::<[ResourceSlotStorage; RESOURCE_SLOT_COUNT]>::new_uninit().assume_init()
        };
        Self {
            slots,
            registered_mask: BitSet256::new(),
        }
    }

    /// Inserts or replaces the resource of type `R`. Cold path.
    ///
    /// C3 RESOLUTION: clear-bit-first replace protocol. If `drop_fn` panics
    /// during a replace, the observable state is "slot empty" (leak instead
    /// of UB).
    /// C-NEW-3 RESOLUTION: uses `assume_init_read()` (requires `Copy` on
    /// `ResourceSlot`) rather than `assume_init()` which needs by-value.
    pub fn insert<R: Resource>(&mut self, value: R) {
        let id = R::resource_id();
        let layout = Layout::new::<R>();
        let raw = Box::into_raw(Box::new(value)) as *mut u8;
        let info = resource_registry::get_resource_info(id)
            .expect("invariant: R::resource_id() implies registry slot is populated");
        let new_slot = ResourceSlot { ptr: raw, drop_fn: info.drop_fn, layout };

        if self.registered_mask.get(id.0 as u32) {
            // === C3 + C-NEW-3 RESOLUTION: clear-bit-first replace ===
            // Step 1: extract old slot data via bitwise copy. `Copy` on
            //   ResourceSlot makes `assume_init_read()` legal â€” the slot
            //   bytes are left untouched but the protocol below ensures
            //   only one logical owner of `old.ptr` proceeds.
            // SAFETY: `registered_mask.get == true` â‡’ slot is initialised (R1).
            //   `assume_init_read()` is sound because all fields are `Copy`
            //   and the unique-ownership invariant of `old.ptr` is preserved
            //   by clearing the registered_mask bit in step 2 before any
            //   external observer could touch the slot again.
            let old = unsafe { self.slots[id.0].slot.assume_init_read() };

            // Step 2: clear the registered_mask bit FIRST. After this point,
            // any external observer (e.g., a `get_ptr<R>` race-paneled into
            // mid-replace via panic stack unwind) sees the slot as empty.
            self.registered_mask.clear(id.0 as u32);

            // Step 3: drop the old value. If this panics, we are in the
            // intermediate "slot empty, allocation leaked" state â€” leak
            // is preferable to UB. The new slot has NOT been written yet,
            // so the leak is exactly one resource.
            if let Some(drop_fn) = old.drop_fn {
                // SAFETY: old.ptr was minted from Box<R>::into_raw, so it is
                //   aligned, initialised, not aliased; not accessed after.
                unsafe { drop_fn(old.ptr); }
            }

            // Step 4: dealloc. If drop didn't panic, this proceeds normally.
            // SAFETY: old.ptr came from Box::into_raw with `old.layout`.
            unsafe { std::alloc::dealloc(old.ptr, old.layout); }

            // Step 5: write the new slot. Cannot panic (POD write).
            self.slots[id.0].slot.write(new_slot);

            // Step 6: re-set the bit. Atomicity is not required (single &mut).
            self.registered_mask.set(id.0 as u32);
        } else {
            // First-insertion path: write slot, then set bit.
            self.slots[id.0].slot.write(new_slot);
            self.registered_mask.set(id.0 as u32);
        }
    }

    /// Removes the resource of type `R`. Returns the typed value if present.
    ///
    /// C-NEW-3 RESOLUTION: uses `assume_init_read()`.
    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        let id = R::resource_id();
        if !self.registered_mask.get(id.0 as u32) { return None; }
        // SAFETY: registered_mask bit is set â‡’ slot initialised (R1).
        //   `assume_init_read()` bitwise-copies; ResourceSlot: Copy.
        let slot = unsafe { self.slots[id.0].slot.assume_init_read() };
        // R5: clear the bit BEFORE reconstructing the Box (in case Box's Drop
        // recursively touches resources â€” extremely unlikely but cheap to
        // guard).
        self.registered_mask.clear(id.0 as u32);
        // SAFETY: slot.ptr came from Box::<R>::into_raw; reconstructing
        //   the Box returns ownership and the value is moved out by `*boxed`.
        let boxed: Box<R> = unsafe { Box::from_raw(slot.ptr.cast::<R>()) };
        Some(*boxed)
    }

    /// Returns `*const R` if present; intended for `Res<T>::get_param`. Hot path.
    ///
    /// SAFETY (caller-side, R2):
    ///   The returned pointer is valid only for the lifetime of `&self`.
    ///   Caller must not alias with a `*mut R` from `get_mut_ptr`.
    #[inline]
    pub(crate) fn get_ptr<R: Resource>(&self) -> Option<*const R> {
        let id = R::resource_id();
        self.get_ptr_by_id(id).map(|p| p.cast::<R>())
    }

    #[inline]
    pub(crate) fn get_mut_ptr<R: Resource>(&mut self) -> Option<*mut R> {
        let id = R::resource_id();
        self.get_mut_ptr_by_id(id).map(|p| p.cast::<R>())
    }

    /// **W1 RESOLUTION** â€” untyped fast path used by `Res::get_param` to
    /// avoid re-resolving `R::resource_id()` (which is an `OnceLock` load).
    /// `Res::get_param` calls this with the cached `state.id` and casts.
    ///
    /// SAFETY (caller-side, R2): the returned pointer is valid only for
    /// the lifetime of `&self`. Caller is responsible for casting to the
    /// correct type â€” `state.id` was minted by `R::resource_id()` at
    /// `init_state` time so the typeâ†”id binding is enforced by the
    /// `ResState<R>` type itself.
    #[inline]
    pub(crate) fn get_ptr_by_id(&self, id: ResourceId) -> Option<*const u8> {
        if !self.registered_mask.get(id.0 as u32) { return None; }
        debug_assert!(id.0 < RESOURCE_SLOT_COUNT);
        // SAFETY: registered bit set â‡’ slot initialised (R1).
        let slot = unsafe { self.slots[id.0].slot.assume_init_ref() };
        Some(slot.ptr as *const u8)
    }

    /// W1 RESOLUTION counterpart for `ResMut`.
    #[inline]
    pub(crate) fn get_mut_ptr_by_id(&mut self, id: ResourceId) -> Option<*mut u8> {
        if !self.registered_mask.get(id.0 as u32) { return None; }
        debug_assert!(id.0 < RESOURCE_SLOT_COUNT);
        // SAFETY: registered bit set â‡’ slot initialised (R1).
        let slot = unsafe { self.slots[id.0].slot.assume_init_ref() };
        Some(slot.ptr)
    }

    #[inline] pub fn contains<R: Resource>(&self) -> bool {
        self.registered_mask.get(R::resource_id().0 as u32)
    }
    #[inline] pub fn len(&self) -> usize { self.registered_mask.count_ones() as usize }
    #[inline] pub fn is_empty(&self) -> bool { self.registered_mask.is_empty() }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // Walk `registered_mask` via pop_lowest_set_bit (TZCNT/BLSR) and
        // drop + dealloc each occupied slot. Mirror of EventDispatcher::Drop.
        //
        // C3 RESOLUTION extended: same clear-bit-first protocol on each
        // iteration. `pop_lowest_set_bit` already clears the bit it returns;
        // if drop_fn panics, the bit stays cleared (cannot revisit) and
        // remaining slots can still be cleaned up by panic-on-double-panic
        // abort. Worst case: one leak, no UB.
        // C-NEW-3 RESOLUTION: uses `assume_init_read()`.
        let mut mask = self.registered_mask;
        while let Some(idx) = mask.pop_lowest_set_bit() {
            let idx = idx as usize;
            debug_assert!(idx < RESOURCE_SLOT_COUNT);
            // SAFETY: idx was just popped from registered_mask â‡’ slot at
            //   idx was initialised by `insert`. The slot has not been
            //   moved-from since insert because Resources owns it.
            //   `assume_init_read()` bitwise-copies (ResourceSlot: Copy).
            let slot = unsafe { self.slots[idx].slot.assume_init_read() };
            if let Some(drop_fn) = slot.drop_fn {
                // SAFETY: slot.ptr was minted from Box<R>::into_raw; it
                //   is aligned, initialised, not aliased (no live SystemParam
                //   can hold a borrow into a dropping Resources because
                //   Drop runs under exclusive &mut self).
                unsafe { drop_fn(slot.ptr); }
            }
            // SAFETY: slot.ptr came from `alloc(slot.layout)` via
            //   Box::new + into_raw with the same layout.
            unsafe { std::alloc::dealloc(slot.ptr, slot.layout); }
        }
    }
}
```

### 5.2 Alternatives rejected

**(a) Reuse `ComponentId` from `ComponentRegistry` (Bevy's choice).** Rejected because Bevy needs it for change-detection-on-resources (Phase 10 deferred for boyko); reusing `ComponentId` exposes resources to the column-iteration code; the 512-slot `LAYOUTS` table would fill with non-component entries reducing usable component budget.

**(b) `HashMap<TypeId, Box<dyn Any>>`.** Rejected â€” virtual dispatch on every `get`, hash cost, allocates per insert. Forbidden by principle 1.

**(c) `[Box<dyn Any>; RESOURCE_SLOT_COUNT]`.** Rejected â€” `Any::downcast_ref` cost; double-pointer-indirection slot size.

**(d) Arena-allocated resources.** Rejected â€” `Arena` is optimised for many small frees; resources are 1-per-type lifetime-of-program.

### 5.3 Trade-off

- Per-resource `Box` allocation (1 per type, lifetime-of-program). Acceptable: not hot path.
- Slab is 8 KB always allocated, even with zero resources. Acceptable: dwarfed by event slab (16 KB) and archetype slab (8 MB).
- The C3 clear-bit-first protocol adds one extra atomic-like store on the replace path. Cost: ~1 ns. Replace is cold; budget is 200 ns; trivially absorbed.

### 5.4 Why this is faster

`Res<T>::get_param`:
1. Load `id: ResourceId` from `Self::State` (1 register).
2. Load `Resources::slots[id.0].slot.ptr` from `&EcsMaster::resources` (1 cache line).
3. Cast `*const u8 â†’ *const T â†’ &T` (free).

Three operations, ~1 cache line touch. **Target met: â‰¤ 3 ns.**

---

## 6. Decision D4 â€” `Res<T>` / `ResMut<T>` `SystemParam` impls (C2 + C4 + W1 resolution)

### 6.1 Decision

`Res<T>` and `ResMut<T>` are thin newtypes around `&'w T` / `&'w mut T` with `SystemParam` impls following the **Bevy `unsafe impl<'a, R: Resource> SystemParam for Res<'a, R>` shape** (C2 RESOLUTION). The impl is parameterised over `'a`, so the bound `Item<'w, 's>: SystemParam<State = Self::State>` holds for all lifetimes via the generic blanket.

**W1 RESOLUTION:** `get_param` uses the cached `state.id` via `Resources::get_ptr_by_id` rather than re-resolving `R::resource_id()`. The OnceLock load is paid once at `init_state`, never on the hot path.

```rust
// File: crates/boyko_ecs/src/ecs/core/system/params/res.rs (new)

#[repr(transparent)]
pub struct Res<'w, R: Resource>(pub(crate) &'w R);
#[repr(transparent)]
pub struct ResMut<'w, R: Resource>(pub(crate) &'w mut R);

impl<'w, R: Resource> std::ops::Deref for Res<'w, R> {
    type Target = R;
    #[inline] fn deref(&self) -> &R { self.0 }
}
impl<'w, R: Resource> std::ops::Deref for ResMut<'w, R> {
    type Target = R;
    #[inline] fn deref(&self) -> &R { &*self.0 }
}
impl<'w, R: Resource> std::ops::DerefMut for ResMut<'w, R> {
    #[inline] fn deref_mut(&mut self) -> &mut R { &mut *self.0 }
}

#[derive(Clone, Copy)]
pub struct ResState<R: Resource> {
    id: ResourceId,
    _marker: PhantomData<fn() -> R>,
}

// SAFETY (SP1): see invariant SP1 in Â§10. The impl is parameterised over
// `'a` so `Item<'w, 's>: SystemParam<State = Self::State>` holds for all
// lifetimes â€” this is the C2 resolution.
unsafe impl<'a, R: Resource> SystemParam for Res<'a, R> {
    type State = ResState<R>;
    type Item<'w, 's> = Res<'w, R>;

    fn init_state(_world: &mut EcsMaster, _meta: &mut SystemMeta) -> Self::State {
        let id = R::resource_id();
        ResState { id, _marker: PhantomData }
    }

    // C4 RESOLUTION: init_access is a SEPARATE method that takes the
    // FilteredAccessSet accumulator and checks for intra-system conflicts.
    fn init_access(
        state: &Self::State,
        meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        access_set
            .add_resource_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP1, SP2, U_C2): we declared a read of `state.id` in
        //   init_access. The protocol guarantees no `ResMut<R>` is being
        //   fetched in this stage (intra-system conflict check at
        //   init_access; cross-system at Phase 9 scheduler).
        //   `world.resources()` is a by-value call on Copy `UnsafeEcsCell`
        //   â€” no Tree Borrows retag (C1 resolution).
        let resources = unsafe { world.resources() };
        // W1 RESOLUTION: use cached state.id with the untyped fast path,
        //   avoiding a redundant `R::resource_id()` OnceLock load.
        let ptr = resources.get_ptr_by_id(state.id)
            .unwrap_or_else(|| missing_resource_panic::<R>());
        // SAFETY: ptr was minted from a populated slot whose registration
        //   was bound to `R` at insert time; lifetime 'w bound to world's
        //   borrow scope. The `ResState<R>` type binds `state.id` to `R`,
        //   so the cast is type-correct.
        Res(unsafe { &*(ptr as *const R) })
    }
}

// ResMut<'a, R> impl follows the same shape with:
//   - add_resource_write in init_access
//   - world.resources_mut() in get_param
//   - get_mut_ptr_by_id(state.id) with &mut *(ptr as *mut R)

#[cold]
#[inline(never)]
fn missing_resource_panic<R: Resource>() -> ! {
    panic!(
        "Resource `{}` not registered. \
         Call `EcsMaster::insert_resource::<{}>(...)` before running systems that read it.",
        R::debug_type_name(), R::debug_type_name()
    );
}

#[cold]
#[inline(never)]
fn intra_system_conflict_panic(conflict: AccessConflict) -> ! {
    panic!(
        "error[boyko-B0002]: intra-system access conflict on resource id {}.\n\
         Existing param: {}\n\
         Conflicting param: {}\n\
         Kind: {:?}\n\
         This would be UB at runtime. Remove one of the accesses or use the same mutability.",
        conflict.id, conflict.existing_param, conflict.new_param, conflict.kind
    );
}
```

### 6.2 Why the typed wrapper and not just `&T`

- Phase 8c `IntoSystem` needs distinct types to disambiguate "global resource" from "queried component". `&T` alone is ambiguous.
- Bevy uses `Res<T>` for the same reason.
- `Res<T>` can grow change-detection metadata (Phase 10) without breaking 8a callers.

### 6.3 Inline policy

`get_param` is **`#[inline]`** â€” single dependent load + cast, cross-crate visibility helps LTO inline into the system body. **`#[inline(always)]` is NOT used** (principle 7). `missing_resource_panic` and `intra_system_conflict_panic` are **`#[cold]` + `#[inline(never)]`**.

### 6.4 Lifetime contract

- `Res<'w, R>` borrows `R` for `'w`.
- `Item<'w, 's>` is `Res<'w, R>` â€” `'s` is the state lifetime but unused for `Res` (state contains only the cached ID).
- The borrow on `EcsMaster` is logically a `&` for `Res`, a `&mut` for `ResMut`. The actual `unsafe` mint goes through `UnsafeEcsCell`; discipline is enforced by `FilteredAccessSet` at init time and Phase 9 scheduler at run time.

---

## 7. Decision D5 â€” Tuple impls for `SystemParam` (M7 + C-NEW-2 resolution)

### 7.1 Decision

Single `macro_rules!` site in `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs` emitting `SystemParam` impl for tuples of arity 0..=12. **M7 + C-NEW-2 RESOLUTION:** also emit `const { panic!(...) }`-bearing stub impls for arities 13..=24 so users get a clear diagnostic at monomorphization time.

```rust
// File: crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs (new)

pub const MAX_SYSTEM_PARAM_ARITY: usize = 12;

macro_rules! impl_system_param_tuple {
    ($($p:ident),*) => {
        // SAFETY (SP3): see Â§10.
        unsafe impl<$($p: SystemParam),*> SystemParam for ($($p,)*) {
            type State = ($($p::State,)*);
            type Item<'w, 's> = ($($p::Item<'w, 's>,)*);

            fn init_state(world: &mut EcsMaster, meta: &mut SystemMeta) -> Self::State {
                ($(<$p as SystemParam>::init_state(world, meta),)*)
            }

            // C4 RESOLUTION: tuple impl calls each param's init_access
            // in declaration order, threading the FilteredAccessSet through.
            fn init_access(
                state: &Self::State,
                meta: &mut SystemMeta,
                access_set: &mut FilteredAccessSet,
                world: &mut EcsMaster,
            ) {
                let ($($p,)*) = state;
                $(<$p as SystemParam>::init_access($p, meta, access_set, world);)*
            }

            #[inline]
            unsafe fn get_param<'w, 's>(
                state: &'s mut Self::State,
                meta: &SystemMeta,
                world: UnsafeEcsCell<'w>,
            ) -> Self::Item<'w, 's> {
                // Destructure state into per-param mutable refs.
                let ($($p,)*) = state;
                // SAFETY (SP3): each per-param get_param contract upheld;
                //   intra-system conflicts validated at init_access via
                //   FilteredAccessSet; world is Copy (by-value receivers).
                ($(unsafe { <$p as SystemParam>::get_param($p, meta, world) },)*)
            }
        }
    };
}

// Empty-tuple base case.
unsafe impl SystemParam for () {
    type State = ();
    type Item<'w, 's> = ();
    fn init_state(_: &mut EcsMaster, _: &mut SystemMeta) -> Self::State {}
    fn init_access(_: &Self::State, _: &mut SystemMeta, _: &mut FilteredAccessSet, _: &mut EcsMaster) {}
    #[inline]
    unsafe fn get_param<'w, 's>(
        _: &'s mut Self::State, _: &SystemMeta, _: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {}
}

impl_system_param_tuple!(P0);
impl_system_param_tuple!(P0, P1);
impl_system_param_tuple!(P0, P1, P2);
impl_system_param_tuple!(P0, P1, P2, P3);
impl_system_param_tuple!(P0, P1, P2, P3, P4);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11);
```

### 7.5 M7 + C-NEW-2 RESOLUTION â€” diagnostic stubs for arity > 12

```rust
// In the same file, after the regular impls. Emits an arity-specific
// SystemParam impl for arities 13..=24 whose method bodies contain
// `const { panic!(...) }`. The const block evaluates ONLY at the point
// of monomorphization (when the user instantiates the impl); the wider
// crate compiles normally if no one uses a 13+ arity tuple.
//
// C-NEW-2 RESOLUTION: `compile_error!` was wrong (fires at macro-expand
// time, breaks the whole crate). `const { panic!(...) }` is a const block
// (stable since Rust 1.79; boyko targets Rust 2024 / 1.85+) that fires
// at type-check time PER monomorphization.
//
// rustc >= 1.79 required (const blocks with panic). Boyko targets the
// Rust 2024 edition which mandates rustc >= 1.85, so the requirement is
// unconditionally satisfied.

macro_rules! impl_system_param_tuple_too_large {
    ($($p:ident),*) => {
        // SAFETY: stub impl whose bodies all `const { panic!(...) }`.
        //   The impl is never *successfully* used at runtime; the const
        //   block fails at monomorphization with the message below.
        unsafe impl<$($p: SystemParam),*> SystemParam for ($($p,)*) {
            type State = ();
            type Item<'w, 's> = ();

            fn init_state(_: &mut EcsMaster, _: &mut SystemMeta) -> Self::State {
                const {
                    panic!(
                        "tuple has too many SystemParam elements. \
                         boyko-engine supports up to MAX_SYSTEM_PARAM_ARITY = 12. \
                         Split your system into smaller systems or wrap related \
                         params in a struct that implements SystemParam."
                    )
                }
            }

            fn init_access(
                _: &Self::State,
                _: &mut SystemMeta,
                _: &mut FilteredAccessSet,
                _: &mut EcsMaster,
            ) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            unsafe fn get_param<'w, 's>(
                _: &'s mut (),
                _: &SystemMeta,
                _: UnsafeEcsCell<'w>,
            ) -> () {
                const { panic!("tuple too large: see init_state diagnostic") }
            }
        }
    };
}

impl_system_param_tuple_too_large!(P0,  P1,  P2,  P3,  P4,  P5,  P6,  P7,  P8,  P9,  P10, P11, P12);
impl_system_param_tuple_too_large!(P0,  P1,  P2,  P3,  P4,  P5,  P6,  P7,  P8,  P9,  P10, P11, P12, P13);
impl_system_param_tuple_too_large!(P0,  P1,  P2,  P3,  P4,  P5,  P6,  P7,  P8,  P9,  P10, P11, P12, P13, P14);
impl_system_param_tuple_too_large!(P0,  P1,  P2,  P3,  P4,  P5,  P6,  P7,  P8,  P9,  P10, P11, P12, P13, P14, P15);
// ... up through arity 24.
```

**Why `const { panic!(...) }` over alternatives:**
1. `compile_error!` â€” REJECTED (C-NEW-2): fires at macro expansion, breaks the crate immediately.
2. `assert!(false, "...")` as an associated const â€” works but is more verbose than the inline `const { }` form.
3. Drop the diagnostic entirely â€” REJECTED: users get a wall of "trait not satisfied" errors from deeper resolution, with no hint about the arity cap.

A trybuild test (`tuple_arity_13_emits_const_panic.rs`) verifies the diagnostic. The test asserts that compilation fails with the "supports up to MAX_SYSTEM_PARAM_ARITY = 12" message in the output.

**Verifying the const block fires at the right time:** `const { panic!(...) }` is evaluated as part of MIR const-evaluation during monomorphization. Per the rust-lang RFC 2345 stabilization track, this means:
- Code that never instantiates a 13+ tuple as `SystemParam` compiles cleanly.
- Code that does instantiate one (e.g., `ecs.run_closure_once::<(P0..P12), _, _>(...)`) fails at the call site with the panic message in the error output.

### 7.2 Alternatives rejected

**(a) Move the macro to `boyko_macros` as a proc-macro.** Rejected â€” proc-macros recompile-on-touch and slow incremental builds; `macro_rules!` evaluates inline.

**(b) Arity 16.** Rejected â€” Phase 8 brief notes 12 covers 99% of real systems; arity 16 adds 4 more monomorphisations Ã— 4 traits in 8b/c = 16 extra impls.

### 7.3 Trade-off

Adding a 13th param requires editing the macro site. Acceptable; raising arity is a one-line patch. Compile time impact: only incurred per concrete tuple used.

### 7.4 Variadic recursion shape

Flat, not recursive â€” every arity has its own explicit invocation. Bevy's `all_tuples!` does the same internally.

---

## 8. Decision D6 â€” `System` trait + `EcsMaster::run_system_once` (M5 + W3 resolution)

### 8.1 Decision

Define a minimal **`System` trait** in 8a (so 8c's `FunctionSystem<F>` slots in cleanly later), and provide **`EcsMaster::run_system_once<S: System>`** plus **`EcsMaster::run_closure_once<P, F, O>`** (M5 RESOLUTION).

**W3 honesty note:** `run_closure_once::<P, _, _>(|p| ...)` requires turbofish on `P` because closure-argument inference cannot deduce the param tuple type. **This IS the API in Phase 8a.** Phase 8c's `IntoSystem` adapter removes the turbofish by inferring `P` from the closure's signature; until then, callers must spell out the param tuple.

```rust
// File: crates/boyko_ecs/src/ecs/core/system/system.rs (new)

pub trait System: 'static {
    type Out;
    fn name(&self) -> &'static str;
    fn access(&self) -> &Access;
    fn initialize(&mut self, world: &mut EcsMaster);

    /// SAFETY (S1): caller asserts no other `System::run_unsafe` is in
    /// flight on this `EcsMaster`. Phase 9 scheduler enforces via the
    /// `Access` conflict graph; Phase 8a's `run_system_once` enforces
    /// it trivially by taking `&mut EcsMaster`.
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out;
}
```

```rust
// EcsMaster facade methods (M5 RESOLUTION):
impl EcsMaster {
    /// Runs a single system once, end-to-end. Generic over `S: System`
    /// so the caller's system survives across calls.
    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out {
        system.initialize(self);
        // SAFETY (S1): `&mut self` exclusive â‡’ no other system in flight;
        //   `UnsafeEcsCell::new_mutable` consumes the borrow scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): same.
        unsafe { system.run_unsafe(cell) }
    }

    /// M5 RESOLUTION: convenience helper that wraps a closure in
    /// FnOnceSystem internally.
    ///
    /// W3 NOTE: callers still need turbofish on the param tuple:
    ///   ecs.run_closure_once::<(Res<A>, ResMut<B>), _, _>(|(a, b)| ...);
    /// Phase 8c's `IntoSystem` adapter removes this requirement by
    /// inferring `P` from the closure's signature.
    pub fn run_closure_once<P, F, O>(&mut self, body: F) -> O
    where
        P: SystemParam + 'static,
        F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + 'static,
        O: 'static,
    {
        let mut sys = FnOnceSystem::<P, F, O>::new(body);
        self.run_system_once(&mut sys)
    }
}
```

### 8.2 Alternatives rejected

**(a) Defer `System` trait to 8c entirely.** Rejected â€” without it, 8a cannot ship a working end-to-end test.

**(b) `Box<dyn System>` for `run_system_once`.** Rejected â€” virtual dispatch.

**(c) Make `System::run` a regular `&mut self` method instead of `unsafe fn run_unsafe`.** Rejected â€” Phase 9 scheduler needs the `unsafe` for derived-`UnsafeEcsCell` calls.

### 8.3 Trade-off

One more trait now. Cost: ~30 lines + the discipline of `unsafe fn`. Gain: Phase 8c plugs in without revisiting 8a's API.

### 8.4 `FnOnceSystem` (M5 RESOLUTION â€” full signature spelled out, W5 method name corrected)

```rust
// File: crates/boyko_ecs/src/ecs/core/system/fn_once_system.rs (new)
//
// 8a-only stub: wraps a closure `FnMut(P::Item<'w, 's>) -> O`. Phase 8c
// replaces this with the real FunctionSystem<F, M> via IntoSystem.

pub struct FnOnceSystem<P, F, O>
where
    P: SystemParam,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O,
{
    f: F,
    state: Option<P::State>,
    meta: SystemMeta,
    _marker: PhantomData<fn() -> (P, O)>,
}

impl<P, F, O> FnOnceSystem<P, F, O>
where
    P: SystemParam + 'static,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + 'static,
    O: 'static,
{
    /// M5 RESOLUTION: full signature spelled out. Direct construction
    /// requires turbofish: `FnOnceSystem::<MyParams, _, ()>::new(...)`.
    /// Use `EcsMaster::run_closure_once` for a slightly less ceremonious
    /// call site (still requires turbofish on the param tuple).
    pub fn new(body: F) -> Self {
        Self {
            f: body,
            state: None,
            meta: SystemMeta::new(std::any::type_name::<F>()),
            _marker: PhantomData,
        }
    }
}

impl<P, F, O> System for FnOnceSystem<P, F, O>
where
    P: SystemParam + 'static,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + 'static,
    O: 'static,
{
    type Out = O;
    fn name(&self) -> &'static str { self.meta.name }
    fn access(&self) -> &Access { &self.meta.access }

    fn initialize(&mut self, world: &mut EcsMaster) {
        if self.state.is_some() { return; }
        // M4 + W5 RESOLUTION: capture archetype generation before init for
        // mid-init mutation debug check. The accessor is `archetype_generation()`
        // (not `id_generation()`) â€” this is the existing API on ArchetypeMaster.
        #[cfg(debug_assertions)]
        let gen_before = world.archetype_master().archetype_generation();

        // Two-phase init: state, then access (C4 RESOLUTION).
        let state = <P as SystemParam>::init_state(world, &mut self.meta);
        let mut access_set = FilteredAccessSet::new();
        <P as SystemParam>::init_access(&state, &mut self.meta, &mut access_set, world);
        access_set.finalize(&mut self.meta);
        self.state = Some(state);

        // M4: debug-check no archetype/registry mutation happened mid-init.
        #[cfg(debug_assertions)]
        {
            let gen_after = world.archetype_master().archetype_generation();
            debug_assert_eq!(
                gen_before, gen_after,
                "invariant SP4: SystemParam::init_state/init_access must not register \
                 new archetypes or resources. Use a separate `EcsMaster::insert_resource` \
                 call before `run_system_once`."
            );
        }
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out {
        let state = self.state.as_mut()
            .expect("invariant: initialize ran before run_unsafe");
        // SAFETY (S1, SP1, SP2): caller upholds System::run_unsafe contract;
        //   per-param get_param contracts upheld; intra-system conflicts
        //   already checked at init_access (FilteredAccessSet).
        let item = unsafe { <P as SystemParam>::get_param(state, &self.meta, world) };
        (self.f)(item)
    }
}
```

---

## 9. Decision D7 â€” `SystemMeta` shape (deep dive)

Already specified inline in Â§4.2. Recap:

| Field | Purpose | Filled by |
|-------|---------|-----------|
| `access: Access` | Phase 9 conflict graph; Phase 10 change-detection scope | each param's `init_access` |
| `name: &'static str` | diagnostic | system constructor |
| `last_archetype_generation: ArchetypeGeneration` | Phase 8b `Query` cache invalidation | Phase 8b `Query::new_archetype` |
| `last_structural_generation: ArchetypeGeneration` | Phase 8b dual-generation cache | Phase 8b `Query::new_archetype` |

**No more fields in 8a.** Phase 10 will add `last_run_tick: Tick`.

---

## 10. SAFETY invariants

| ID | Invariant |
|----|-----------|
| **R1** | `Resources::registered_mask.get(i) == true` â‡’ `slots[i].slot` is properly initialised with a `ResourceSlot` whose `ptr` came from `Box::<R>::into_raw` for the registered type `R`. |
| **R2** | `Resources::get_ptr::<R>()` / `get_ptr_by_id` returns `Some(ptr)` â‡’ `ptr` points to a valid `R` for the lifetime of `&Resources`; caller respects single-`&mut` discipline at the `EcsMaster` level. |
| **R3** | `Resources::Drop` walks `registered_mask` and calls `drop_fn(slot.ptr)` exactly once per occupied slot, then `dealloc(slot.ptr, slot.layout)`. No double-free, no leak (modulo panic-in-drop leak per R4). |
| **R4** | `Resources::insert::<R>` replace path follows the **clear-bit-first** protocol (C3 + C-NEW-3 RESOLUTION): (1) extract old slot via `assume_init_read()` (sound because `ResourceSlot: Copy`); (2) clear registered_mask bit; (3) drop old; (4) dealloc old; (5) write new slot; (6) re-set bit. If `drop_fn` panics at step 3, the observable state is "slot empty"; one resource allocation leaks but no UB. |
| **R5** | `Resources::remove::<R>` clears `registered_mask` bit BEFORE reconstructing `Box::from_raw`, so a re-entrant `Drop` sees the slot as empty. Uses `assume_init_read()` (`ResourceSlot: Copy`). |
| **AB-R1** | **C-NEW-1 + C-R3-1 RESOLUTION** — `ArchetypeBundle::add_archetype` replace path follows clear-bit-first protocol: (1) cache `slot_idx`; (2a) clear the `occupied` bit (`self.occupied[slot_idx/64] &= !(1u64 << (slot_idx%64))`) BEFORE `drop_in_place` — THIS is the gate `Drop` walks; (2b) clear `id_to_slot[raw_id] = NO_SLOT` for observer safety; (3) `drop_in_place(slot_ptr)`; (4) `ptr::write(slot_ptr, archetype)`; (5a) re-set the `occupied` bit; (5b) re-set `id_to_slot[raw_id] = slot_idx`. If `drop_in_place` panics, `ArchetypeBundle::Drop` walks `self.occupied` and skips the cleared bit → no double-drop UB. The `count` field is intentionally not modified during the drop window (brief inconsistency with `occupied` is observable only via `len()`, which is not used by `Drop` or any read path). |
| **U_C1** | `UnsafeEcsCell::new_mutable(world: &'w mut EcsMaster)` requires the cell not outlive `'w`. Enforced by `PhantomData` and `!Send/!Sync`. |
| **U_C2** | `UnsafeEcsCell::world()` / `resources()` returns shared refs only when caller's `Access` declared a read; no `&mut` aliases. **Crucially, the cell is `Copy` and methods take `self` by value** so no `&self` retag occurs (C1 RESOLUTION). |
| **U_C3** | `UnsafeEcsCell::world_mut()` / `resources_mut()` / `archetype_ptr_mut()` returns exclusive refs only when caller's `Access` declared a write that does not conflict. In debug builds the cell carries `allows_mutable_access: bool`; mutable methods `debug_assert!(self.allows_mutable_access)`. |
| **SP1** | Every `SystemParam::init_access` impl declares its complete access surface via `access_set.add_*` calls. Any `get_param` access not declared by `init_access` is UB. (C4 RESOLUTION: this is now distinct from `init_state`.) |
| **SP2** | Every `SystemParam::get_param` impl honours the protocol: only reads/writes data within the access declared by `init_access`. Phase 9 scheduler relies on this. |
| **SP3** | Tuple `SystemParam::get_param` calls each component's `get_param` in declaration order. Intra-system conflicts were rejected at `init_access` time by `FilteredAccessSet`. |
| **SP4** | `SystemParam::init_state` MUST NOT mutate the world's structural shape (no register-new-archetype, no register-new-resource). Debug-asserted via `archetype_generation()` comparison (M4 RESOLUTION; W5 method-name fix). |
| **S1** | `System::run_unsafe(world: UnsafeEcsCell)` requires no other `System::run_unsafe` is in flight on the same `EcsMaster`. `run_system_once` enforces by taking `&mut self`. |

---

## 11. Data structures (summary, Q6 RESOLUTION applied)

```rust
// crates/boyko_ecs/src/ecs/core/system/mod.rs
pub mod access;
pub mod system_meta;
pub mod system;
pub mod unsafe_ecs_cell;
pub mod fn_once_system;
pub mod system_param;
pub mod filtered_access_set;
pub mod params {
    pub mod tuple_impl;
    pub mod res;
    // pub mod query;      // Phase 8b
    // pub mod commands;   // Phase 8d
    // pub mod local;      // Phase 8 backlog
}

// Public re-exports at crate root:
pub use system::System;
pub use system_meta::SystemMeta;
pub use access::Access;
pub use filtered_access_set::{FilteredAccessSet, AccessConflict, ConflictKind};
pub use unsafe_ecs_cell::UnsafeEcsCell;
pub use system_param::SystemParam;
pub use params::res::{Res, ResMut};

// crates/boyko_ecs/src/ecs/core/resources/mod.rs (Q6 RESOLUTION)
pub mod resource;
pub(crate) mod resource_registry;  // Q6: not pub.
pub mod resources;

pub use resource::{Resource, ResourceId};
pub use resources::Resources;
pub use resource_registry::RESOURCE_SLOT_COUNT;

// crates/boyko_ecs/src/ecs/identifiers/primitives.rs additions
define_id!(/// Public resource type identifier (assigned at registration).
    ResourceId);
```

### 11.1 Sizing verification (Q5 RESOLUTION â€” math corrected; W2 RESOLUTION applied)

| Type | Size | Cache lines |
|------|------|-------------|
| `Access` | **192 B** (M1: no align(64), no padding) | 3 lines |
| `SystemMeta` | 192 + 16 (name) + 16 (2 Ã— ArchetypeGeneration) = **224 B** | 4 lines |
| `FilteredAccessSet` | 192 + 8 (Box ptr) = 200 B on stack; **~24 KB heap during init only (W2: 1536 Ã— 16 B fat pointers)** | 3 lines stack |
| `AccessConflict` | 24 B (kind 4 + id 4 + 2 Ã— ptr 16) | <1 line |
| `ResourceSlot` | 24 B; **`Copy`** (C-NEW-3 RESOLUTION) | 1 line (4 per line) |
| `Resources` | 40 B (Box ptr 8 + BitSet256 32) | 1 line |
| `Resources::slots` | 256 Ã— 32 B = 8 KB | heap slab |
| `UnsafeEcsCell<'w>` | 8 B (Copy raw pointer + ZST PhantomData) + 1 B debug | trivially Copy |
| `Res<'w, R>` | 8 B | trivially Copy/Deref |
| `ResMut<'w, R>` | 8 B | move-only |
| `ResState<R>` | 8 B (ResourceId(usize)) | trivially Copy |

Full `Resources` subsystem fits in ~8.1 KB heap + ~40 B on `EcsMaster`. `SystemMeta` is 224 B (4 lines, Q5 corrected from Round 1's wrong 280 B claim). `FilteredAccessSet` transient heap is 24 KB (W2 corrected from Round 2's wrong 12 KB).

---

## 12. Public API surface (8a delta on `EcsMaster`)

```rust
impl EcsMaster {
    // === Resources ===
    pub fn insert_resource<R: Resource>(&mut self, value: R);
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R>;
    pub fn resource<R: Resource>(&self) -> &R;            // panics on missing
    pub fn resource_mut<R: Resource>(&mut self) -> &mut R; // panics on missing
    pub fn try_resource<R: Resource>(&self) -> Option<&R>;
    pub fn try_resource_mut<R: Resource>(&mut self) -> Option<&mut R>;
    pub fn contains_resource<R: Resource>(&self) -> bool;
    pub fn resource_count(&self) -> usize;
    pub fn resources(&self) -> &Resources;
    pub fn resources_mut(&mut self) -> &mut Resources;

    // === Systems ===
    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out;
    pub fn run_closure_once<P, F, O>(&mut self, body: F) -> O
    where
        P: SystemParam + 'static,
        F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + 'static,
        O: 'static;
}
```

No breaking changes to existing methods. Phase 7 fast read path API untouched.

---

## 13. The full `SystemParam` trait (C2 + C4 + M4 resolution)

```rust
// File: crates/boyko_ecs/src/ecs/core/system/system_param.rs (new)

/// SAFETY (SP1, SP2, SP4): see Â§10.
pub unsafe trait SystemParam: Sized {
    /// Long-lived state owned by the system. `Send + Sync + 'static`
    /// so the containing system can move across threads under Phase 9.
    type State: Send + Sync + 'static;

    /// The borrowed view delivered to the system body per run.
    /// GAT-form: `'w` is the world-access scope; `'s` is the state scope.
    /// `Item<'w, 's>` must itself implement `SystemParam` so tuples nest
    /// cleanly (Bevy convention).
    ///
    /// C2 RESOLUTION: this bound holds because impls are parameterised
    /// over the outer lifetime (e.g., `impl<'a, R: Resource> SystemParam
    /// for Res<'a, R>`), so the bound is satisfied for ALL lifetimes via
    /// the generic blanket.
    type Item<'w, 's>: SystemParam<State = Self::State>;

    /// Initialises the per-system state.
    ///
    /// SAFETY contract (SP4): MUST NOT mutate the world's structural
    /// shape â€” no new archetype/resource registrations. Debug-asserted
    /// via `archetype_generation()` comparison in `FnOnceSystem::initialize`.
    /// The `&mut EcsMaster` is provided for state types that need to
    /// pre-allocate (e.g., Phase 8b `QueryState` matches existing
    /// archetypes).
    fn init_state(world: &mut EcsMaster, meta: &mut SystemMeta) -> Self::State;

    /// C4 RESOLUTION: declare this param's access surface AND detect
    /// intra-system conflicts via the `FilteredAccessSet` accumulator.
    /// Called once per system after `init_state`. Implementations MUST
    /// declare every read/write they will perform via
    /// `access_set.add_resource_read/write/component_read/write`. Returning
    /// `Err` from those calls causes a panic with B0002 diagnostic.
    ///
    /// Default impl is empty (for params with no access â€” `Local<T>`,
    /// `Commands`).
    fn init_access(
        state: &Self::State,
        meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        world: &mut EcsMaster,
    );

    /// SAFETY (SP1, SP2): caller asserts the world-access protocol is
    /// upheld â€” `Access` declared in `init_access` is the complete and
    /// honest summary of this param's reads/writes, and Phase 9 scheduler
    /// (or `run_system_once` in 8a) has resolved aliasing.
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's>;

    /// Hook for deferred mutations after the system body returns. Phase 8d
    /// `Commands` overrides this. Default no-op.
    fn apply(_state: &mut Self::State, _meta: &SystemMeta, _world: &mut EcsMaster) {}

    /// Hook called by Phase 8b `Query` when a new archetype is added.
    /// Default no-op.
    fn new_archetype(_state: &mut Self::State, _meta: &mut SystemMeta, _archetype: &Archetype) {}
}
```

### 13.1 Why the GAT `Item<'w, 's>` and not just `Item<'w>`

Bevy uses `Item<'world, 'state>`. Phase 8b `Query<D, F>` stores `QueryState` inside `Self::State`; the returned `Query<'w, 's, D, F>` borrows BOTH the world (`'w`) AND the cached state (`'s`). Single-lifetime GAT collapses those and forces redundant coercions. Adopt the two-lifetime form now.

### 13.2 Why `unsafe trait`

Implementations must uphold SP1, SP2, SP4. The trait itself doesn't have a safety boundary the compiler can check, so the obligation is on the impl.

### 13.3 `Send + Sync + 'static` on `Self::State`

Required for Phase 9 (system moves across threads). All Phase 8a `State` types satisfy this trivially:
- `ResState<R>` is `Copy + Send + Sync` (ResourceId + PhantomData<fn()->R>).
- Tuple `State = (S0, S1, ..., Sn)` is `Send + Sync` if every component is.

### 13.4 C2 RESOLUTION â€” impl shape

The trait bound `type Item<'w, 's>: SystemParam<State = Self::State>` is satisfied because every concrete impl is parameterised over the outer lifetime:

```rust
unsafe impl<'a, R: Resource> SystemParam for Res<'a, R> {
    type Item<'w, 's> = Res<'w, R>;  // valid for ANY 'w because impl is for ALL 'a
    // ...
}
```

When the compiler checks the bound `Item<'w, 's>: SystemParam<State = Self::State>`, it looks for the impl `SystemParam for Res<'w, R>` â€” which exists by the `'a`-generic blanket. No `lifetimeless::Res` shim is needed; the canonical Bevy shape works directly.

### 13.5 M4 + W5 RESOLUTION â€” `init_state` structural-shape invariant

`init_state` takes `&mut EcsMaster` for legitimate state pre-allocation (e.g., Phase 8b `QueryState` queries existing archetypes during construction). The invariant SP4 forbids structural mutation:

- No `EcsMaster::insert_resource` during `init_state`.
- No `EcsMaster::register_component` (Phase 5).
- No `EcsMaster::create_entity` (creates archetypes implicitly).

Debug-asserted in `FnOnceSystem::initialize` by comparing **`archetype_master().archetype_generation()`** (W5: existing method name) before and after the init sweep. In release builds, violations are undetected â€” that's acceptable because user code rarely accidentally registers from `init_state`, and the debug-build check catches mistakes in CI.

If a `SystemParam` legitimately needs to register something at init time (none in 8a), it must do so during `init_access` instead (which runs after `init_state` and is also `&mut World`). This is the same convention Bevy uses.

---

## 14. Algorithms for critical paths

### 14.1 `Res<R>::get_param` (hot path, â‰¤ 3 ns target; W1 RESOLUTION applied)

```text
Step 1: Load state.id from &Self::State                            (1 register, 0 cache miss)
Step 2: Copy UnsafeEcsCell (8 B, Copy)                              (free; no &self retag)
Step 3: Load &EcsMaster::resources via cell.resources()             (1 register dependent load)
Step 4: Call resources.get_ptr_by_id(state.id)                      (W1: NO R::resource_id() OnceLock load)
   4a:   Check registered_mask.get(id.0)                            (1 bit-test, predictable branch)
   4b:   Load Resources::slots[id.0].slot.ptr                       (1 cache line, dependent)
Step 5: Cast *const u8 â†’ *const R â†’ &R                              (free)
```

**Big-O:** O(1). **Cache:** 1 dependent line.
**Branching:** 1 predictable branch on `registered_mask.get`.
**SIMD potential:** none â€” single load.
**W1 win:** the cached `state.id` is consumed directly; no `OnceLock::get()` acquire-load per access.

### 14.2 `Resources::insert::<R>` (cold path, â‰¤ 200 ns)

```text
First-insert path:
Step 1: Box::new(value)                                   (~ 100 ns malloc)
Step 2: Look up R::resource_id() (OnceLock cached load)   (~ 1 ns)
Step 3: Look up resource_registry::get_resource_info(id) (1 cache line)
Step 4: Write slot                                        (1 cache line)
Step 5: Set registered_mask bit                           (free)

Replace path (C3 + C-NEW-3 RESOLUTION clear-bit-first):
Step 1-3: same
Step 4: Extract old slot via assume_init_read (bitwise copy; Copy: ResourceSlot)  (1 cache line)
Step 5: Clear registered_mask bit                         (free)
Step 6: Call drop_fn(old.ptr)                             (variable)
Step 7: Dealloc(old.ptr, old.layout)                      (~ 50 ns free)
Step 8: Write new slot                                    (1 cache line)
Step 9: Re-set registered_mask bit                        (free)
```

**Big-O:** O(1). **Cache:** 2 lines on first-insert, 3 on replace.

### 14.3 Tuple `SystemParam::get_param` (hot path)

```text
For each component param in declaration order:
    call <Pi as SystemParam>::get_param(&mut state.i, meta, world)
    // `world` is Copy; no borrow contortion
Construct tuple (Item_0, Item_1, ..., Item_{n-1})
```

**Big-O:** O(N) where N = arity (â‰¤ 12).
**Cache:** sum of per-param accesses. Compiler should CSE `(*world.ptr).resources` across multiple `Res<>` params.

### 14.4 `FilteredAccessSet::add_resource_read` (init-only, â‰¤ 50 ns)

```text
Step 1: Check combined.resource_writes.get(idx)          (1 bit-test)
Step 2: If conflict, return Err with name lookup         (cold)
Step 3: combined.resource_reads.set(idx)                 (1 bit-set)
Step 4: bit_owners[1024 + idx] = param_name              (1 cache line)
```

**Big-O:** O(1). **Cache:** 2 lines. Cold path.

---

## 15. Multithreading model

### 15.1 Phase 8a stance

**Single-threaded.** `EcsMaster: !Send + !Sync`. `UnsafeEcsCell: !Send + !Sync`. `run_system_once` is exclusive (`&mut self`). The `Access` aggregation in `SystemMeta` is read-but-not-acted-on in 8a â€” machinery for Phase 9.

### 15.2 `Send + Sync` on data structures

| Type | `Send` | `Sync` |
|------|--------|--------|
| `Resources` | not in 8a | not in 8a |
| `UnsafeEcsCell<'w>` | explicitly `!Send + !Sync` | explicitly `!Send + !Sync` |
| `Access` | `Send + Sync` (POD bitset) | `Send + Sync` |
| `FilteredAccessSet` | `Send + Sync` (Box of `&'static str` = Send+Sync) | `Send + Sync` |
| `SystemMeta` | `Send + Sync` | `Send + Sync` |
| `ResState<R>` | `Send + Sync` (POD) | `Send + Sync` |
| `Res<'w, R>` | `Send + Sync` iff `R: Send + Sync` | same |
| `ResMut<'w, R>` | `Send + !Sync` (auto from `&'w mut R`) | `!Sync` |

Phase 9 will add `unsafe impl Send for UnsafeEcsCell` with discipline contract; **8a does not.**

### 15.3 Atomic operations

None on the hot path. `OnceLock` reads (per-type cached `ResourceId`) are acquire-loads only on cold/first call; warm path is direct copy. **W1 RESOLUTION** further removes the OnceLock load from `Res::get_param` by caching `id` in `ResState<R>` at `init_state` time and using `get_ptr_by_id`.

### 15.4 Proof of data-race freedom (8a)

- Only `run_system_once` / `run_closure_once` exist. Both take `&mut EcsMaster` â€” Rust's borrow checker proves no other access.
- The `UnsafeEcsCell` minted inside `run_system_once` is consumed only by the one `System::run_unsafe` call; the cell is `Copy` and the tuple impl makes copies, but every copy points to the same world and the `init_access`-validated `FilteredAccessSet` ensures no two params alias.
- Therefore no two pieces of code touch `EcsMaster` concurrently.

QED for 8a. Phase 9 will replace the trivial proof with a scheduler-discipline proof grounded in `Access::conflicts_with`.

---

## 16. Integration with existing modules

### 16.1 Modules created (new files)

| File | Lines (est.) | Purpose |
|------|--------------|---------|
| `crates/boyko_ecs/src/ecs/core/system/mod.rs` | ~30 | re-exports |
| `crates/boyko_ecs/src/ecs/core/system/access.rs` | ~150 | `Access` struct + `conflicts_with` + tests |
| `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | ~80 | `SystemMeta` struct |
| `crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs` | ~200 | C4 RESOLUTION â€” intra-system conflict detector |
| `crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs` | ~150 | C1 RESOLUTION â€” by-value receivers |
| `crates/boyko_ecs/src/ecs/core/system/system.rs` | ~80 | `System` trait |
| `crates/boyko_ecs/src/ecs/core/system/system_param.rs` | ~150 | `SystemParam` trait with `init_access` |
| `crates/boyko_ecs/src/ecs/core/system/fn_once_system.rs` | ~150 | 8a smoke-test adapter (M5 + W5: includes archetype-gen debug check) |
| `crates/boyko_ecs/src/ecs/core/system/params/mod.rs` | ~10 | submod registry |
| `crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs` | ~250 | macro + 12 invocations + M7/C-NEW-2 const-panic stubs |
| `crates/boyko_ecs/src/ecs/core/system/params/res.rs` | ~280 | `Res<T>`/`ResMut<T>` + tests + C4 init_access + W1 by-id fast path |
| `crates/boyko_ecs/src/ecs/core/resources/mod.rs` | ~15 | submod registry |
| `crates/boyko_ecs/src/ecs/core/resources/resource.rs` | ~120 | `Resource` trait + `ResourceId` + M2/C5 docs |
| `crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs` | ~280 | M6 RESOLUTION â€” Component exclusivity check (W4: best-effort doc) |
| `crates/boyko_ecs/src/ecs/core/resources/resources.rs` | ~400 | `Resources` + Drop + C3 clear-bit-first + C-NEW-3 `assume_init_read` |
| `crates/boyko_ecs/tests/system_param_smoke.rs` | ~250 | end-to-end smoke test + intra-system conflict test |

**Total new code:** ~2 600 lines (up from Round 1's 2 000 due to `FilteredAccessSet` + diagnostic stubs).

### 16.2 Modules edited (existing files)

| File | Change |
|------|--------|
| `crates/boyko_ecs/src/ecs/identifiers/primitives.rs` | Add `define_id!(ResourceId)`. |
| `crates/boyko_ecs/src/ecs/core/mod.rs` | Add `pub mod resources;` and `pub mod system;`. |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Add `resources: Resources` field **AT THE TOP** (C5: dropped first). Add 9+2 facade methods (Â§12). Update field-order doc-comment for new drop order. |
| `crates/boyko_macros/src/lib.rs` | Add `#[proc_macro_derive(Resource)]`. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry.rs` | M6 RESOLUTION: add `pub(crate) fn is_type_registered_as_component(type_id: TypeId) -> bool`. |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs` | **C-NEW-1 RESOLUTION** â€” apply clear-lookup-first protocol to `add_archetype` replace path (lines 412-436). See Â§16.5. |

### 16.3 Compatibility checks against Phase 7

- **`Arena` / `Box<Arena>` drop order**: `resources` does NOT hold arena pointers. New drop order (`resources â†’ events â†’ entity_master â†’ archetype_master â†’ arena`) is safe.
- **`ComponentPool` invariants**: untouched.
- **`EntityInland` / `Archetype.columns`**: untouched.
- **`OnceLock` collision detection**: `resource_registry` uses an independent table.

### 16.4 No change to Phase 6 events

`EventReader<E>` / `EventWriter<E>` `SystemParam` impls deferred to 8c.

### 16.5 C-NEW-1 RESOLUTION â€” patch `archetype_bundle.rs::add_archetype` replace path

**Round 2 erratum:** the previous audit incorrectly claimed `add_archetype` has no replace path. **The audit was wrong.** The actual code at `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs:412-436` has an explicit replace branch:

```rust
// CURRENT CODE (vulnerable to panic-in-drop double-drop UB):
if raw_id < self.id_to_slot.len() && self.id_to_slot[raw_id] != NO_SLOT {
    let slot_idx = self.id_to_slot[raw_id];
    let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
    let slot_ptr: *mut Archetype =
        unsafe { slab_base.add(slot_idx as usize) as *mut Archetype };
    unsafe {
        ptr::drop_in_place(slot_ptr);   // <-- if this panics:
        ptr::write(slot_ptr, archetype); //     - slot is half-dropped
    }                                    //     - id_to_slot[raw_id] still = slot_idx
    return InlandArchetypeId(slot_idx as usize);
}                                        //     - ArchetypeBundle::Drop revisits â†’ double-drop UB
```

**The bug:** if a user's component `Drop` impl panics during `ptr::drop_in_place(slot_ptr)`, the `Archetype` at `slot_idx` is partially destructed but the `occupied` bitset still has `slot_idx`'s bit set. When `ArchetypeBundle::Drop` later runs, it **walks `self.occupied`** (not `id_to_slot`) via TZCNT/BLSR (see `archetype_bundle.rs:664-689`), finds the bit still set, and calls `drop_in_place` again on the same partially-destructed `Archetype` → double-drop UB.

**Critical note (R3 critic finding C-R3-1):** the authoritative occupancy gate for `Drop`'s walk is `self.occupied`, NOT `self.id_to_slot`. Clearing only `id_to_slot` does NOT prevent the double-drop because `Drop` never reads `id_to_slot`. The fix must clear the `occupied` bit as well. This matches `remove_archetype` (lines 477-505) which clears both structures atomically.

**FIX (clear-bit-first protocol, mirrors R4 + matches `remove_archetype`):**

```rust
// PATCHED CODE (C-NEW-1 + C-R3-1 RESOLUTION):
if raw_id < self.id_to_slot.len() && self.id_to_slot[raw_id] != NO_SLOT {
    let slot_idx = self.id_to_slot[raw_id];
    let slab_base: *mut MaybeUninit<Archetype> = self.slots.as_mut_ptr();
    // SAFETY (U11): mint via raw arithmetic; slot_idx is in-bounds.
    let slot_ptr: *mut Archetype =
        unsafe { slab_base.add(slot_idx as usize) as *mut Archetype };

    // === AB-R1: clear-bit-first ===
    // Step 1a: clear the `occupied` bit BEFORE drop_in_place. THIS is what
    //   gates ArchetypeBundle::Drop's bitset walk — even if drop_in_place
    //   panics, Drop will skip this slot.
    let word_idx = (slot_idx as usize) / 64;
    let bit = (slot_idx as usize) % 64;
    self.occupied[word_idx] &= !(1u64 << bit);

    // Step 1b: clear the lookup too. Belt-and-suspenders: prevents any
    //   external observer calling get_archetype_ptr mid-replace from
    //   seeing a stale mapping. `count` is intentionally left unchanged
    //   during the drop window — it briefly disagrees with `occupied`
    //   (count says "1 archetype" while bitset says "0"); this is
    //   observable only via `len()`, never via Drop or the read path.
    self.id_to_slot[raw_id] = NO_SLOT;

    // Step 2: drop the old occupant. If this panics, the observable
    //   state is "bit cleared, lookup cleared, slot bytes partially valid,
    //   slab cell unreachable from both id_to_slot and Drop's bitset walk".
    //   One Archetype's allocations leak; no UB.
    // SAFETY (U12 + AB-R1): the previous occupancy was confirmed by the
    //   outer `if`; clear-bit-first ensures non-revisitation on panic.
    unsafe { ptr::drop_in_place(slot_ptr); }

    // Step 3: write the new value. Cannot panic (POD memcpy).
    // SAFETY (U13): slab cell is logically empty after step 2's drop.
    unsafe { ptr::write(slot_ptr, archetype); }

    // Step 4a: re-set the `occupied` bit. Single &mut, no atomicity needed.
    self.occupied[word_idx] |= 1u64 << bit;

    // Step 4b: re-set the lookup.
    self.id_to_slot[raw_id] = slot_idx;

    return InlandArchetypeId(slot_idx as usize);
}
```

**Why this is the right fix:**
- The cost is two extra `u64` bitset ops + two extra `usize` stores (Steps 1a/b + 4a/b) — ~4 ns on cold path.
- The benefit is panic-safety: a user `Drop` panic cannot corrupt the slab.
- The pattern matches `remove_archetype` (clears both `occupied` and `id_to_slot`) for protocol uniformity within `ArchetypeBundle`, AND mirrors `Resources::insert` (R4) for cross-subsystem uniformity.
- `count` is intentionally NOT temporarily decremented — the brief inconsistency between `count` and `occupied` is observable only via `ArchetypeBundle::len()`, which is not used during `Drop` or on any read path. Keeping `count` unchanged simplifies the protocol.

**Invariant AB-R1 added to Â§10.** **Step 12 in Â§17 changed** from "audit-only" to "apply the patch above + add unit test for panic-in-drop".

**Scope decision:** patch applied in Phase 8a (not deferred). Rationale: the bug is a Phase 7 carry-over discovered while reviewing Phase 8a's analogous resource path; fixing both in one phase produces a consistent project-wide clear-X-first protocol and eliminates the only known double-drop UB shape in the slab subsystem.

---

## 17. Implementation plan â€” 14-step checklist (revised)

Each step compiles cleanly; `cargo test --all-targets` green; one commit per step.

### Step 0 â€” `ResourceId` newtype + archetype-generation accessor verification

**Files:**
- `crates/boyko_ecs/src/ecs/identifiers/primitives.rs` â€” add `define_id!(ResourceId)`.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` â€” **W5 verification**: the existing `pub fn archetype_generation(&self) -> ArchetypeGeneration` is the accessor used by M4. No new code, no rename. If a visibility change is needed, surface it; otherwise no edit.

**Acceptance:** `cargo check -p boyko-ecs` clean.

### Step 1 â€” `Resource` trait + `resource_registry` (M6: Component exclusivity)

**Files:**
- `crates/boyko_ecs/src/ecs/core/resources/{mod.rs, resource.rs, resource_registry.rs}` (new)
- `crates/boyko_ecs/src/ecs/core/component/component_registry.rs` â€” add `is_type_registered_as_component`.
- `crates/boyko_ecs/src/ecs/core/mod.rs` â€” add `pub mod resources;`.

**Acceptance:** Unit tests:
- `register_resource_then_get_returns_info`
- `register_collision_panics`
- `register_idempotent_same_type`
- `next_id_distinctness`
- **`register_as_both_component_and_resource_panics`** (M6)

### Step 2 â€” `Resources` storage (C3 clear-bit-first protocol + C-NEW-3 assume_init_read)

**Files:** `crates/boyko_ecs/src/ecs/core/resources/resources.rs` (new).

**Acceptance:** Unit tests:
- `insert_then_get_round_trip`
- `insert_replace_runs_drop_on_old`
- `insert_replace_clears_bit_during_drop` â€” uses a drop type that checks `Resources::contains` from inside `Drop` and sees `false`.
- `remove_returns_value_and_clears_bit`
- `drop_runs_for_every_occupied_slot`
- **`insert_replace_panic_in_drop_leaks_but_does_not_corrupt`** (C3) â€” uses a panicking drop type; catches panic via `std::panic::catch_unwind`, then verifies `Resources::contains::<R>() == false` (state is empty, leak is one resource).
- **`get_ptr_by_id_matches_get_ptr`** (W1) â€” verifies the two paths return the same pointer for the same id.

### Step 3 â€” `#[derive(Resource)]` proc-macro

**Files:** `crates/boyko_macros/src/lib.rs`.
**Acceptance:** Trybuild test `derive_resource_emits_lazy_id`.

### Step 4 â€” `Access` + `SystemMeta` + `FilteredAccessSet` (C4 + M1 + M8)

**Files:**
- `crates/boyko_ecs/src/ecs/core/system/{mod.rs, access.rs, system_meta.rs, filtered_access_set.rs}` (new)
- `crates/boyko_ecs/src/ecs/core/mod.rs` â€” add `pub mod system;`.

**Acceptance:** Unit tests:
- `access_conflicts_self_when_writing`
- `access_no_conflict_when_both_read`
- `access_resource_conflicts_independent_of_components`
- **`filtered_access_set_detects_read_after_write`** (C4)
- **`filtered_access_set_detects_write_after_read`** (C4)
- **`filtered_access_set_detects_double_write`** (C4)
- **`filtered_access_set_allows_two_reads`** (C4)
- **`filtered_access_set_finalize_writes_into_meta`** (C4)

### Step 5 â€” `UnsafeEcsCell<'w>` (C1: by-value receivers)

**Files:** `crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs` (new).

**Acceptance:** Type-check + **two Miri tests**:
- `miri_unsafe_ecs_cell_world_mut_no_retag_ub` â€” mints cell from `&mut EcsMaster`, calls `world_mut()` by-value, writes through.
- `miri_unsafe_ecs_cell_archetype_ptr_mut_no_retag` â€” calls `archetype_ptr_mut()` by-value to verify the C1 fix.

### Step 6 â€” `SystemParam` trait + tuple impls + M7/C-NEW-2 const-panic stubs

**Files:**
- `crates/boyko_ecs/src/ecs/core/system/system_param.rs` (new)
- `crates/boyko_ecs/src/ecs/core/system/params/{mod.rs, tuple_impl.rs}` (new)

**Acceptance:**
- Compile-only `assert_impl_systemparam<T: SystemParam>()` shims for arities 0..=12.
- **Trybuild test `tuple_arity_13_emits_const_panic`** (M7 + C-NEW-2) â€” asserts compilation fails with the "supports up to MAX_SYSTEM_PARAM_ARITY = 12" message when a 13-element tuple is used. Verifies the const block fires at monomorphization (not at macro-expand).
- **Compile-clean test `unused_oversized_tuple_does_not_break_crate`** (C-NEW-2) â€” a file that defines arities 13..=24 but never instantiates them; must compile cleanly. This validates that `const { panic!() }` is monomorphization-gated, not expand-time.

### Step 7 â€” `Res<R>` + `ResMut<R>` (C2 impl shape + C4 init_access + W1 by-id fast path)

**Files:** `crates/boyko_ecs/src/ecs/core/system/params/res.rs` (new).

**Acceptance:** Unit tests:
- `res_init_state_caches_resource_id`
- `res_init_access_adds_resource_read_to_set`
- `resmut_init_access_adds_resource_write`
- `res_get_param_returns_correct_value`
- `res_get_param_panics_on_missing` (`#[should_panic]`).
- **`res_plus_resmut_same_type_panics_with_b0002`** (C4) â€” `#[should_panic(expected = "boyko-B0002")]`.
- **`res_get_param_uses_cached_id_not_oncelock_load`** (W1) â€” verifies `get_param` calls `get_ptr_by_id(state.id)` and not `get_ptr::<R>()`. Mechanism: a debug counter on `R::resource_id()` invocation; assert it is called exactly once (at `init_state`) and not again at `get_param`.

### Step 8 â€” `System` trait + `FnOnceSystem` + `EcsMaster::run_system_once` + `run_closure_once`

**Files:**
- `crates/boyko_ecs/src/ecs/core/system/{system.rs, fn_once_system.rs}` (new)
- `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` â€” add `run_system_once`, `run_closure_once`.

**Acceptance:** Smoke tests â€” `fn empty_system() {}` runs through both entry points without panic.

### Step 9 â€” `EcsMaster::insert_resource` + facade methods + drop-order change (C5)

**Files:** `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`.

**Acceptance:**
- `cargo test --all-targets` green (no Phase 7 regression).
- `ecs_master_insert_then_resource_round_trip`
- `ecs_master_drop_runs_resource_drop`
- **`ecs_master_drop_order_resource_drops_before_events`** (C5) â€” uses a marker resource that sets a flag in its Drop; an event handler that asserts the flag is NOT set proves resources dropped first.

### Step 10 â€” End-to-end smoke test through `Res<R>` + `ResMut<R>` (M5 + W3 path)

**Files:** `crates/boyko_ecs/tests/system_param_smoke.rs` (new).

```rust
#[derive(Resource)]
struct Tick(u32);

#[test]
fn res_mut_system_increments_resource() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Tick(0));
    // W3 NOTE: turbofish on the param tuple IS required in Phase 8a.
    // Closure-argument inference cannot deduce `P` from the body alone.
    // Phase 8c's `IntoSystem` adapter removes this requirement.
    ecs.run_closure_once::<ResMut<Tick>, _, _>(|mut tick| { tick.0 += 1; });
    assert_eq!(ecs.resource::<Tick>().0, 1);
}

#[test]
#[should_panic(expected = "boyko-B0002")]
fn res_and_resmut_same_type_in_same_system_panics() {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Tick(0));
    // Intra-system conflict â€” caught at init_access via FilteredAccessSet (C4).
    // W3: turbofish required.
    ecs.run_closure_once::<(Res<Tick>, ResMut<Tick>), _, _>(|(_r, _w)| {});
}
```

### Step 11 â€” Bench placeholders

**Files:** `crates/boyko_ecs/benches/system_param.rs` (new).

**Action:** Scaffold criterion benches:
- `bench_res_get_param_hot` â€” â‰¤ 3 ns.
- `bench_resmut_get_param_hot` â€” â‰¤ 3 ns.
- `bench_tuple4_get_param_hot` â€” â‰¤ 12 ns.
- `bench_empty_system_run_once` â€” â‰¤ 5 ns.
- `bench_resources_insert` â€” â‰¤ 200 ns.
- `bench_resources_drop_64_occupied` â€” â‰¤ 2 Âµs.
- `bench_filtered_access_set_add_conflict_check` â€” â‰¤ 50 ns.

### Step 12 — Patch `archetype_bundle.rs::add_archetype` replace path (C-NEW-1 + C-R3-1)

**Files:** `crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs` (edit lines 412-436).

**Action:** Apply the clear-bit-first protocol from §16.5:
1. Cache `slot_idx` before any mutation.
2a. Clear the `occupied` bit: `self.occupied[slot_idx as usize / 64] &= !(1u64 << (slot_idx as usize % 64))`. THIS is what gates `Drop`'s bitset walk.
2b. Set `self.id_to_slot[raw_id] = NO_SLOT` (belt-and-suspenders for external observers).
3. Call `drop_in_place(slot_ptr)`.
4. Call `ptr::write(slot_ptr, archetype)`.
5a. Re-set the `occupied` bit: `self.occupied[slot_idx as usize / 64] |= 1u64 << (slot_idx as usize % 64)`.
5b. Set `self.id_to_slot[raw_id] = slot_idx`.

`count` is intentionally left unchanged through the drop window (brief inconsistency with `occupied` is non-observable outside `len()`).

Update the function's SAFETY/invariant doc-comment to reference **AB-R1** in §10.

**Acceptance:** Unit tests:
- `archetype_bundle_add_archetype_replace_drops_old` (existing semantics preserved).
- **`archetype_bundle_add_archetype_replace_panic_in_drop_no_double_drop`** (C-NEW-1 + C-R3-1) — uses an `Archetype` with a `Component` whose `Drop` panics; catches panic via `catch_unwind`; verifies `id_to_slot[raw_id] == NO_SLOT` AND `self.occupied[slot_idx/64] & (1u64 << (slot_idx%64)) == 0` after the catch; verifies that subsequent `ArchetypeBundle::Drop` does NOT double-drop (use a per-component drop counter that must read exactly 1, not 2).
- **`miri_archetype_bundle_replace_panic_in_drop_no_ub`** (Miri tests in Step 13).

**Follow-up tracking item (out of Phase 8a scope):** `ArchetypeBundle::clear()` at lines 536-558 has the same panic-in-drop double-drop shape on the next-bit iteration of its `Drop`-time walk. Open as a Phase 7 carry-over ticket "ArchetypeBundle::clear() panic-in-drop double-drop" — not blocking Phase 8a.

### Step 13 â€” Miri test suite

**Files:** `crates/boyko_ecs/tests/miri_phase8a.rs` (new, gated `#[cfg(miri)]`).

**Action:**
- `miri_resources_drop_runs_drop_glue`
- `miri_resources_replace_no_double_free`
- `miri_resources_replace_panic_in_drop_no_ub`
- `miri_unsafe_ecs_cell_no_retag_via_by_value_methods` (C1)
- `miri_res_get_param_no_retag`
- `miri_run_system_once_full_e2e`
- **`miri_archetype_bundle_replace_panic_in_drop_no_double_drop`** (C-NEW-1)
- **`miri_resources_assume_init_read_does_not_move_slot_bytes`** (C-NEW-3) â€” verifies that `assume_init_read()` followed by a deliberate post-condition check does not leave the slot's underlying bytes in an aliased / moved-from state that Miri rejects.

---

## 18. Metrics and validation

### 18.1 Mandatory unit tests

Each module ships its own unit-test block. Mandatory minimum:

- `resources::insert_then_get_round_trip`
- `resources::insert_replace_runs_drop_on_old`
- `resources::insert_replace_panic_in_drop_leaks_but_does_not_corrupt` (C3)
- `resources::drop_runs_for_every_occupied_slot`
- `resources::get_ptr_by_id_matches_get_ptr` (W1)
- `resource_registry::register_collision_panics`
- `resource_registry::register_as_both_component_and_resource_panics` (M6)
- `access::conflicts_when_writing_what_other_reads`
- `access::no_conflict_when_both_read`
- `filtered_access_set::detects_read_after_write` (C4)
- `filtered_access_set::detects_double_write` (C4)
- `res::get_param_returns_correct_value`
- `res::get_param_panics_on_missing`
- `res::res_plus_resmut_same_type_panics_with_b0002` (C4)
- `res::get_param_uses_cached_id_not_oncelock_load` (W1)
- `system_param::tuple_impls_compile`
- `ecs_master::insert_then_resource_round_trip`
- `ecs_master::drop_runs_resource_drop`
- `ecs_master::drop_order_resource_drops_before_events` (C5)
- `archetype_bundle::add_archetype_replace_panic_in_drop_no_double_drop` (C-NEW-1)
- Integration: `system_param_smoke::res_mut_system_increments_resource`
- Integration: `system_param_smoke::res_and_resmut_same_type_in_same_system_panics` (C4)
- Trybuild: `tuple_arity_13_emits_const_panic` (M7 + C-NEW-2)
- Trybuild: `unused_oversized_tuple_does_not_break_crate` (C-NEW-2)

### 18.2 Property tests

- `proptest!` over `Resources::insert` and `Resources::remove` random sequences.

### 18.3 Miri tests (Step 13)

- `miri_phase8a_resources_drop_runs_drop_glue`
- `miri_phase8a_resources_replace_panic_in_drop_no_ub` (C3)
- `miri_phase8a_resources_assume_init_read_no_aliasing` (C-NEW-3)
- `miri_phase8a_unsafe_ecs_cell_no_retag` (C1)
- `miri_phase8a_run_system_once_full_e2e`
- `miri_phase8a_archetype_bundle_replace_panic_in_drop_no_double_drop` (C-NEW-1)

### 18.4 Mandatory `debug_assert!` invariants

In `Resources`:
- `debug_assert!(id.0 < RESOURCE_SLOT_COUNT)` at every `slots[]` index (including `get_ptr_by_id` and `get_mut_ptr_by_id`).

In `Res::get_param`:
- `debug_assert!(state.id.0 < RESOURCE_SLOT_COUNT)`.

In `Access::add_*`:
- `debug_assert!(id < bound)` per type.

In `UnsafeEcsCell`:
- `debug_assert!(self.allows_mutable_access)` in `world_mut`/`resources_mut`/`archetype_ptr_mut` (C1 sentinel).

In `FnOnceSystem::initialize`:
- `debug_assert_eq!(gen_before, gen_after)` for SP4 (M4 + W5: `archetype_generation()`).

### 18.5 Benches with targets (recap)

| Bench | Target |
|-------|--------|
| `bench_res_get_param_hot` | â‰¤ 3 ns |
| `bench_resmut_get_param_hot` | â‰¤ 3 ns |
| `bench_tuple4_get_param_hot` | â‰¤ 12 ns |
| `bench_empty_system_run_once` | â‰¤ 5 ns |
| `bench_resources_insert` | â‰¤ 200 ns |
| `bench_resources_drop_64` | â‰¤ 2 Âµs |
| `bench_filtered_access_set_add_conflict_check` | â‰¤ 50 ns |

---

## 19. Cross-phase dependencies

| Sub-phase | Depends on 8a delivering |
|-----------|--------------------------|
| **8b â€” `Query<D, F>` DSL** | `SystemParam` trait (esp. GAT `Item<'w, 's>`, `init_access`), `UnsafeEcsCell` (by-value receivers, C1), `SystemMeta.last_archetype_generation` / `last_structural_generation`, `Access::component_reads/writes`, `FilteredAccessSet::add_component_*`. |
| **8c â€” `IntoSystem` + function-as-system** | `System` trait, `SystemMeta`. Replaces 8a's `FnOnceSystem` with `FunctionSystem<F, M>` + `IntoSystem` + `SystemParamFunction`. `EventReader`/`EventWriter` land here. **Removes turbofish requirement for `run_closure_once` (W3 follow-up).** |
| **8d â€” `Commands`** | `SystemParam::apply` hook (already in trait), Phase 8a tuple impls. |
| **Phase 9 â€” Scheduler** | `Access::conflicts_with`, `SystemMeta`, `UnsafeEcsCell`. `unsafe impl Send for UnsafeEcsCell` lands here. Also: `NonSendResource` trait + `NonSendRes<R>` SystemParam (M2 deferral target â€” track as **Phase 9 Â§9.4**). Also: `Drop`-from-resource re-entrancy guard (C5 supporting â€” track as **Phase 9 Â§9.5**). |
| **Phase 10 â€” Change detection** | `SystemMeta.last_run_tick: Tick` (reserved). |

---

## 20. Risks and mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| GAT lifetime threading misbehaves with two-lifetime `Item<'w, 's>` on stable Rust | Medium | GATs stable since 1.65; spot-check macro expansion early in Step 6. |
| `UnsafeEcsCell::world_mut()` triggers Tree Borrows retag UB | High | **C1 RESOLUTION**: by-value receivers eliminate the retag at the language level. Miri test in Step 5 + Step 13 verify. |
| `Resources::insert` replace path leaks/UBs if `drop_fn` panics | Medium | **C3 RESOLUTION**: clear-bit-first protocol. Test in Step 2 + Miri in Step 13. |
| `#[derive(Resource)]` macro conflict with `#[derive(Component)]` | Low | **M6 RESOLUTION**: runtime check via `is_type_registered_as_component`. Test in Step 1. |
| Intra-system `Res<X> + ResMut<X>` aliases | High | **C4 RESOLUTION**: `FilteredAccessSet` detects at `init_access` time. Test in Step 7 + Step 10. |
| `SystemParam::init_state` accidentally registers archetypes mid-init | Low | **M4 RESOLUTION**: debug-assert via `archetype_generation()` comparison in `FnOnceSystem::initialize`. |
| User writes 13-param system | Low | **M7 + C-NEW-2 RESOLUTION**: `const { panic!(...) }` stub impls for arity 13..=24 fire at monomorphization with a clear message. Trybuild test in Step 6. |
| Phase 9 scheduler decides 8a `Access` shape is insufficient | Medium | `Access` is private to the system module; mark `#[non_exhaustive]` for forward compat. |
| `Res<T>` panic-on-missing surprises users | Low | Diagnostic tells user to call `EcsMaster::insert_resource::<X>(...)`. `try_resource<X>` provided. |
| Resource `Drop` impl touches the world (C5 contract violation) | Medium | Trait doc explicit; **Phase 9 Â§9.5** adds a re-entrancy guard. |
| Phase 7 `add_archetype` replace path double-drop UB | High | **C-NEW-1 RESOLUTION**: clear-lookup-first protocol applied in Step 12. Unit test + Miri test. |
| `MaybeUninit::assume_init()` on array element does not compile | High (was build-breaker) | **C-NEW-3 RESOLUTION**: derive `Copy` on `ResourceSlot` and use `assume_init_read()`. Soundness preserved by the clear-bit-first protocol. |
| `Res::get_param` ignores cached id and re-loads via OnceLock | Medium (perf) | **W1 RESOLUTION**: introduce `Resources::get_ptr_by_id`; `Res::get_param` uses cached `state.id`. Unit test asserts the OnceLock load count. |

---

## 21. Out of scope (deferred)

- **Phase 8b `Query<D, F>` DSL.** Trait shapes defined; impl deferred.
- **Phase 8c `IntoSystem` + `FunctionSystem<F>`.** `FnOnceSystem` is the 8a stub. **Removes turbofish in `run_closure_once`** (W3 follow-up).
- **Phase 8d `Commands` buffer.** `SystemParam::apply` hook exists; impl deferred.
- **`EventReader<E>` / `EventWriter<E>` `SystemParam`.** Deferred to 8c.
- **`Local<T>` `SystemParam`.** Deferred to Phase 8 backlog.
- **`NonSendResource` + `NonSendRes<T>`.** Deferred to **Phase 9 Â§9.4** (M2 tracking).
- **Resource `Drop` re-entrancy guard.** Deferred to **Phase 9 Â§9.5** (C5 tracking).
- **Parallel execution / `Schedule`.** Phase 9.
- **Change detection (`Tick`, `Mut<T>`, `Changed<T>`).** Phase 10.
- **PGO profile use.** Phase 10 backlog.

---

## 22. Resolved questions (was: Open questions for the critic)

1. **`Resource: Send + Sync`** â€” **resolved**: keep `Send + Sync`, defer `NonSendResource` to Phase 9 Â§9.4 (M2).
2. **`ResourceId` as `usize` vs `u16`** â€” **resolved**: `usize` for uniformity.
3. **`EcsMaster::resource<R>` panic policy** â€” **resolved**: panic on `resource()`, `Option` on `try_resource()`.
4. **`SystemMeta` size** â€” **resolved**: 224 B (Q5 corrected).
5. **`RESOURCE_SLOT_COUNT = 256`** â€” **resolved**: renamed from `MAX_RESOURCES`, locked to `BitSet256` width (M3).
6. **`Access` `#[non_exhaustive]`** â€” **resolved**: yes, mark `#[non_exhaustive]`.
7. **`init_state` takes `&mut EcsMaster`** â€” **resolved**: yes, with SP4 debug-asserted structural-shape invariant (M4 + W5).
8. **`add_archetype` replace path safety** â€” **resolved (C-NEW-1)**: clear-lookup-first patch applied in Step 12; new invariant AB-R1.
9. **Diagnostic for arity > 12** â€” **resolved (C-NEW-2)**: `const { panic!(...) }`, not `compile_error!` (which fires at expand-time).
10. **`MaybeUninit::assume_init()` on array element** â€” **resolved (C-NEW-3)**: `ResourceSlot: Copy` + `assume_init_read()`.
11. **`Res::get_param` cached id usage** â€” **resolved (W1)**: added `get_ptr_by_id(ResourceId)` untyped fast path.
12. **`run_closure_once` turbofish requirement** â€” **resolved (W3)**: honest documentation; turbofish required in 8a; Phase 8c eliminates it.
13. **M6 TOCTOU race** â€” **resolved (W4)**: documented as best-effort, registration is single-threaded by convention.
14. **`archetype_generation` method name** â€” **resolved (W5)**: use existing API name, no rename.

---

## 23. How to launch implementation

When the critic approves and the user gives explicit go-ahead:

1. Orchestrator re-reads this plan + `docs/plans/PHASE-08-system-api.md`.
2. Update `docs/plans/PHASE-08-system-api.md`: change 8a section status DRAFT â†’ PLANNED, link this file.
3. Dispatch `developer` for Step 0 + Step 1 (parallel â€” different files, no overlap).
4. Each subsequent step: developer â†’ code-reviewer cycle.
5. **Step 12 (C-NEW-1 patch) runs in parallel with Step 11 (benches)** â€” different files, no dependency.
6. After Step 13: `tester` runs bench harness + Miri; `results-analyst` provides final verdict.
7. Update `docs/FEATURE_MAP.md` with `Res<T>` / `ResMut<T>` / `Resource` / `Resources` / `EcsMaster::run_system_once` / `run_closure_once` / `FilteredAccessSet` entries.
8. Update `docs/SYSTEMS.md` with new `core/system/` and `core/resources/` modules. Note the Phase 7 `add_archetype` carry-over fix in Â§16.5.

Every commit: author `Celtokisa <bluesteelll@hotmail.com>`, no `Co-Authored-By`, compiles cleanly, passes `cargo test --all-targets`.

---

## 24. References

- Phase 7 detailed plan: `docs/PHASE-7-FAST-RANDOM-ACCESS-PLAN.md` â€” U1-U14 invariants reused.
- Phase 6 event dispatch plan: `docs/PHASE-6-EVENT-DISPATCH-PLAN.md` â€” slab + `OnceLock` registry pattern.
- Phase 8 framing: `docs/plans/PHASE-08-system-api.md`.
- Bevy `SystemParam` trait: <https://docs.rs/bevy_ecs/latest/bevy_ecs/system/trait.SystemParam.html>.
- Bevy `UnsafeWorldCell`: <https://docs.rs/bevy_ecs/latest/bevy_ecs/world/struct.UnsafeWorldCell.html> â€” by-value receiver shape (C1 RESOLUTION reference).
- Bevy `FilteredAccessSet`: <https://docs.rs/bevy_ecs/latest/bevy_ecs/query/struct.FilteredAccessSet.html> â€” intra-system conflict detector (C4 RESOLUTION reference).
- Rust RFC 2345 â€” Allow panicking in constants: <https://rust-lang.github.io/rfcs/2345-const-panic.html> â€” backing for C-NEW-2 resolution.
- `std::mem::MaybeUninit::assume_init_read` docs: <https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#method.assume_init_read> â€” backing for C-NEW-3 resolution.

---

## Plan readiness checklist (architect self-check, Round 3)

### Plan structure
- [x] Goal stated in terms of performance and functionality (Â§1)
- [x] Target metrics concrete (Â§1.2)
- [x] Every architectural decision has perf/cache/parallelism justification (D1-D7)
- [x] Each alternative has a reasoned rejection (Â§3.2, Â§5.2, Â§7.2, Â§8.2)
- [x] Trade-offs honestly listed (Â§3.3, Â§5.3, Â§7.3, Â§8.3)
- [x] **All 5 Round 1 critical findings have concrete code-shape resolutions** (C1, C2, C3, C4, C5)
- [x] **All 8 Round 1 major findings either fixed or explicitly deferred with tracking** (M1-M8)
- [x] **All 3 Round 2 critical findings resolved** (C-NEW-1: patch add_archetype; C-NEW-2: const panic; C-NEW-3: Copy + assume_init_read)
- [x] **All 5 Round 2 major findings resolved** (W1: get_ptr_by_id; W2: 24 KB math; W3: turbofish documented; W4: single-thread best-effort; W5: archetype_generation)

### Data structures
- [x] Each field has type + role comment
- [x] `#[repr(...)]` specified where it matters
- [x] Hot/cold split applied (`ResourceInfo` size+align+drop_fn hot, type_name+type_id cold)
- [x] Struct size known and justified (Â§11.1, W2-corrected)
- [x] Padding for false sharing N/A (M1: `Access` is write-once, no `align(64)`)
- [x] **`ResourceSlot: Copy` for `assume_init_read()` compatibility** (C-NEW-3)

### API
- [x] Public API minimal (Â§12)
- [x] No internal types leak
- [x] Lifetimes explicit (`Res<'w, R>`, `Item<'w, 's>`, `UnsafeEcsCell<'w>`)
- [x] No `dyn Trait` on hot path
- [x] Generics where specialization needed
- [x] **`get_ptr_by_id` / `get_mut_ptr_by_id` untyped fast path added** (W1)

### Multithreading
- [x] Model explicit (Â§15.1 â€” single-threaded in 8a)
- [x] Atomics with memory ordering specified
- [x] No synchronization point on hot path
- [x] `Send`/`Sync` for types specified (Â§15.2)

### Correctness
- [x] Edge cases enumerated (missing resource â†’ panic, registered_mask consistency, panic-in-drop)
- [x] Drop order discussed and **fixed for C5** (Â§2.2)
- [x] Invariants for `unsafe` blocks stated (Â§10 â€” added SP4, AB-R1)
- [x] **Tree Borrows retag analysed and fixed via by-value receivers** (C1)
- [x] **Intra-system aliasing detection added** (C4)
- [x] **Replace-path panic safety analysed (Resources + Archetypes)** (C3 + C-NEW-1)
- [x] **`MaybeUninit::assume_init` mechanism corrected** (C-NEW-3)
- [x] **`compile_error!` mechanism corrected to `const { panic!(...) }`** (C-NEW-2)

### Integration
- [x] Affected modules listed (Â§16)
- [x] Changes in existing APIs explicit
- [x] Compatibility with `Arena`/`ComponentPool`/`UnitId` verified (Â§16.3)
- [x] Implementation plan broken into 14 steps (Â§17)
- [x] **Phase 7 retroactive patch committed to 8a scope** (Â§16.5 rewritten â€” C-NEW-1)

### Validation
- [x] Unit tests specified (Â§18.1) â€” including dedicated tests for each critical resolution + Round 3 deltas
- [x] Property tests specified (Â§18.2)
- [x] Benchmarks specified (Â§18.5) â€” including `FilteredAccessSet` bench
- [x] `debug_assert!` invariants specified (Â§18.4)
- [x] **Miri test suite specified** (Â§18.3, Step 13) â€” extended for C-NEW-1 and C-NEW-3

### Q1-Q7 + Round 2/3 audit
- [x] Q1 â€” naming `UnsafeEcsCell` consistent throughout
- [x] Q2 â€” `init_state` contract via `unsafe trait`
- [x] Q3 â€” `#[inline]` discipline preserved
- [x] Q4 â€” `Box::new_uninit().assume_init()` pattern picked, justified
- [x] Q5 â€” `SystemMeta` size math corrected to 224 B
- [x] Q6 â€” `resource_registry` is `pub(crate)`
- [x] Q7 â€” `Box::new` in `insert` is cold path, acceptable
- [x] W1 â€” `get_ptr_by_id` introduced; `Res::get_param` uses cached id
- [x] W2 â€” `bit_owners` heap math corrected (12 KB â†’ 24 KB)
- [x] W3 â€” turbofish requirement honestly documented
- [x] W4 â€” single-threaded registration documented as best-effort
- [x] W5 â€” `archetype_generation()` name used throughout

End of Round 3 plan.

---

# Orchestrator briefing â€” Round 3 summary of resolutions

For the architecture-critic to focus on (Round 3):

1. **C-NEW-1 â€” `add_archetype` replace path double-drop**: Â§16.5 rewritten with the correct audit finding (the replace path DOES exist at archetype_bundle.rs:412-436). New **Step 12** applies the clear-lookup-first protocol: cache `slot_idx` â†’ clear `id_to_slot[raw_id] = NO_SLOT` â†’ `drop_in_place` â†’ `ptr::write` â†’ re-set `id_to_slot[raw_id] = slot_idx`. New invariant **AB-R1** added to Â§10. Scope: patch lands in Phase 8a (not deferred) for protocol uniformity with `Resources::insert`.

2. **C-NEW-2 â€” `compile_error!` in stub bodies**: replaced with **`const { panic!(...) }`**. Const block evaluates only at monomorphization (stable since Rust 1.79; boyko's Rust 2024 edition mandates 1.85+). New trybuild test `unused_oversized_tuple_does_not_break_crate` verifies the const block is gated by instantiation, not parsing.

3. **C-NEW-3 â€” `MaybeUninit::assume_init()` on array element**: `ResourceSlot` gains `#[derive(Clone, Copy)]` (all fields are already `Copy`: `*mut u8`, `Option<unsafe fn>`, `Layout`). All three call sites (insert replace, remove, Drop) use `assume_init_read()` instead of `assume_init()`. Soundness preserved by the clear-bit-first protocol â€” the registered_mask bit is cleared in the same code path, ensuring unique-ownership semantics for `slot.ptr` even though the slot bytes are bitwise-copied.

**Majors (W) resolved**:
- **W1 â€” `Res::get_param` re-resolves `R::resource_id()`**: added `Resources::get_ptr_by_id(id: ResourceId) -> Option<*const u8>` (and `get_mut_ptr_by_id`). `Res::get_param` now uses cached `state.id` directly. Test `res_get_param_uses_cached_id_not_oncelock_load` verifies the OnceLock load count.
- **W2 â€” `bit_owners` math**: corrected from 12 KB to **24 KB** (`&'static str` is a 16 B fat pointer, not 8 B). Updated Â§4.6 and Â§11.1.
- **W3 â€” turbofish in `run_closure_once`**: documented honestly in Â§8.1, Â§8.4, and the smoke test. Turbofish IS required in 8a; Phase 8c's `IntoSystem` removes it. Updated Â§10 smoke test comment.
- **W4 â€” M6 TOCTOU race**: documented as best-effort in `register_resource_new` doc-comment. Single-threaded registration is the de-facto pattern of `#[derive]`-generated `OnceLock` init; concurrent same-TypeId Component+Resource registration is a should-not-happen authoring error, not a race to defend against.
- **W5 â€” `id_generation()` does not exist**: all 4 references replaced with existing API name `archetype_generation()`. No rename, no breaking change.

Plan file path (to be saved by orchestrator): `D:\claude\BoykoEngine\docs\PHASE-8A-SYSTEMPARAM-PLAN.md`.

Sources:
- [2345-const-panic - The Rust RFC Book](https://rust-lang.github.io/rfcs/2345-const-panic.html)
- [Stabilize RFC 2345: Allow panicking in constants (rust-lang/rust#89006)](https://github.com/rust-lang/rust/issues/89006)
- [What are the guarantees around which constants (and callees) in a function get monomorphized? (rust-lang/rust#122301)](https://github.com/rust-lang/rust/issues/122301)
- [Bevy `UnsafeWorldCell` documentation](https://docs.rs/bevy_ecs/latest/bevy_ecs/world/struct.UnsafeWorldCell.html)
- [Bevy `FilteredAccessSet` documentation](https://docs.rs/bevy_ecs/latest/bevy_ecs/query/struct.FilteredAccessSet.html)
- [`std::mem::MaybeUninit::assume_init_read` documentation](https://doc.rust-lang.org/std/mem/union.MaybeUninit.html#method.assume_init_read)