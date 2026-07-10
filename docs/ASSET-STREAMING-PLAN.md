# Asset system rework — assets-as-components + VM-native store + full streaming

Status: **DESIGN LOCKED (2026-07-10)** — v2 architecture + delta-fixes below. Owner-approved
direction (assets-as-components, S1 VM-native shared store, full-streaming scope). Supersedes the
committed A0–A3b `Assets<T>`-on-`std::Vec` + `Box<dyn Any>` foundation (Principle-0/1 violations).

This plan was produced by an architect↔critic loop (2 design rounds, 5 adversarial critics). The
raw v2 design + all critiques live in the session scratchpad; this file is the durable, corrected,
implementation-ready spec. Every rung is atomic + gated.

## Locked decisions

- **Instance side (D1):** a renderable game object = `Transform + StaticMesh + MeshMaterial`, where
  the carriers are THIN SoA components (a bare dense id per column, `#[repr(transparent)]`), NOT fat
  structs. "Extensible metadata" = ADD sibling columns, never fields inside the carrier. This is
  already the engine shape: `MeshHandle | MaterialHandle | Visibility | InstanceModelCol`
  (`crates/boyko_scene/src/render_caps.rs`).
- **Shared store (S1):** VM-native, resource-owned, keyed by a dense slot id. The
  `Handle`/`Assets`/`AssetServer` *wrapper* is dissolved in spirit (the id inside the component IS
  the handle); `Assets<T>` and `Handle<T>` names are RETAINED as VM-native facades to minimise churn
  and the byte-identity blast radius. The four `std::Vec` columns in `assets.rs` are deleted.
- **Store backing = the shipped DenseStore recipe.** Both stores are a store-owned `ComponentPool`
  (VM-native, occupancy-tracked, address-stable) + `LiveBitmap` + free-list + VM-native gen/state/
  refcount lanes. `ComponentPool::new(id, reserve_rows)` is standalone (no archetype), exactly as
  `DenseStore.column` / `ScratchColumn` already are (`dense_store.rs:69`, `scratch_column.rs:69`).
  This **refutes and retracts** the `slot.rs` "a VM-native SlotColumn is unsound (drop `!Copy` UB)"
  rationale — `DenseStore` already does occupancy-tracked exactly-once drop over such a pool.
- **Scope = FULL STREAMING** (owner-selected): generation-in-carrier + gather validation; refcount
  lifetime owner via hooks; fence-gated deferred GPU-free; path→handle dedup without HashMap; plus
  GPU-mirror growth (stream-in) and the material-into-raster wiring (a confirmed bug: the raster path
  ignores material — `gbuffer_mrt.fs.hlsl:51` hardcodes `DEFAULT_MESH_MATERIAL_ID = 0u`).
- **Carry-forwards:** `HasLoaders` const-table static dispatch (delete `Box<dyn Any>`/`TypeId`/loaders
  HashMap/`decode_thunk`); OBJ vertex sort-dedup (delete its HashMap); ZERO HashMap in the asset system.

## The unified store

```rust
pub struct Assets<T: AssetBacking> {
    col:        ComponentPool,   // store-owned data column (stride = size_of::<T>()), address-stable
    slot_word:  VmColumn<u32>,   // {gen: high 29b, state: low 3b} per slot — single packed probe
    refcount:   VmColumn<u32>,   // per-slot live-ref count — streaming lifetime owner
    live:       LiveBitmap,      // occupancy (DenseStore-sanctioned)
    free:       Vec<u32>,        // LIFO free-list (DenseStore-sanctioned); slot returns ONLY at retire
    pinned:     BitSet,          // NEVER_RETIRE bits (slot 0 = default asset)
    dirty:      BitSet,          // slots whose gen advanced/retired THIS frame — drives validation
    live_count: usize,
    high_water: usize,           // == col.count(); dense id space [0,high_water) for gather sizing
    dirty_gen:  u64,             // GPU-mirror re-upload trigger (unchanged semantics)
    free_epoch: u64,             // monotonic; bumped on ANY free/gen-advance; validate early-out oracle
    id:         AssetKind,       // routing tag for the deferred-free queue
    _t:         PhantomData<T>,
}
```

- **`slot_word`**: states `Vacant=0 Loading=1 Loaded=2 Failed=3 Retiring=4` (3 bits); gen = high 29
  bits (wrap by difference-compare, same as ticks). No `states: Vec` (the last unsanctioned Vec is gone).
- **Send/Sync:** `Assets<T>` must reproduce the `Vec<T>` auto-trait profile so `Assets<MaterialGpu>`
  stays a `Send+Sync` `Resource` and `Assets<MeshGpu>` stays `!Send` (`NonSendResource`). Since
  `ComponentPool`/`VmColumn` hold a `NonNull` (auto-`!Send`), add
  `unsafe impl<T: AssetBacking + Send> Send for Assets<T>` + `Sync` mirror, with a SAFETY comment
  (single-owner, `&mut` mutation, `&` shared reads, no interior mutability — the `Vec<T>` profile).
- **`take_at` (the ONLY new store unsafe):** `ComponentPool::take_at<T>(idx) -> T` = `ptr::read` the
  row WITHOUT running `drop_at`; caller clears the `live` bit first so terminal `Drop`/`drop_at` never
  re-touches it (exactly-once move-out). Miri-TB gated.
- **`fill` signature:** `fill(h, v) -> Result<(), (AssetError, T)>` — on a Retiring/Vacant/gen-mismatch
  target the value is RETURNED (never dropped: `MeshGpu` has no device-freeing `Drop`; a bare drop
  leaks BoundBuffer+BLAS), and the caller routes it to the fence-gated deferred-free queue.
- **remove/retire = take-at-RETIRE, single owner = the store.** `remove(h)`/refcount→0 marks the slot
  `Retiring` (occupied-but-UNREUSABLE), enqueues `FreeEntry{kind,slot,retire_frame}` (NO value),
  bumps `free_epoch`, sets `dirty`. `retire_deferred_frees` does the single `take_at` + device
  destroy + gen-bump + free-list push once the fence retires.
- **Terminal `Drop`** mirrors `DenseStore::drop` (drop live slots via `drop_at`, `pop_entity_no_drop`
  to 0). Explicit `destroy(ctx)` remains the device teardown (drain deferred queue with `wait_idle`).

### AssetBacking — via a macro, NOT a blanket impl (FIX-A)

A blanket `impl<T: bytemuck::Pod> AssetBacking for T` + `impl AssetBacking for MeshGpu` is **E0119**
on stable (coherence does no negative reasoning over the foreign `Pod` trait — it cannot prove
`MeshGpu: !Pod`). Use a declarative macro instead:

```rust
pub trait AssetBacking: Sized + Send + Sync + 'static {
    const NEEDS_TEARDOWN: bool;
    fn register_layout() -> ComponentId;   // cached OnceLock<ComponentId> per T
}
macro_rules! impl_asset_pod_backing { ($($t:ty),*) => { $(
    impl AssetBacking for $t {
        const NEEDS_TEARDOWN: bool = false;               // POD: drop_fn = None
        fn register_layout() -> ComponentId { register_asset_layout::<$t>(None) }
    })* } }
impl_asset_pod_backing!(MaterialGpu);                      // + POD test types in #[cfg(test)]
impl AssetBacking for MeshGpu {                            // Resident: manual drop_fn (frees no device mem)
    const NEEDS_TEARDOWN: bool = true;
    fn register_layout() -> ComponentId { register_asset_layout::<MeshGpu>(Some(MeshGpu::drop_glue)) }
}
```

The ~30 `assets.rs` unit tests + the 256-case `assets_matches_hashmap_oracle` proptest instantiate
`Assets::<u64>` — the test module does `impl_asset_pod_backing!(u64, u32);` once, then ports verbatim.

## State machine (states V/L/D=Loaded/F/R=Retiring; gen advances only on R→V)

| Current | Event | Next | refcount | deferred-free | value |
|---|---|---|---|---|---|
| V | `add(v)` | D | =0 | — | write v; live=1; stamp gen |
| V | `reserve()` | L | =0 | — | no value; live=1; stamp gen |
| L | `fill` gen-match | D | — | — | write v |
| L | `fail` | F | — | — | — |
| L | `dec_ref→0` (cancel-load-on-retire) | R | 0 | enqueue `{slot,cur+FIF}`; epoch++; dirty | pending fill will be rejected |
| D | `fill` | D | — | — | **Err((AlreadyLoaded,v))** → deferred-free |
| R\|V | `fill` | unchanged | — | — | **Err((Retiring/Stale,v))** → deferred-free |
| D,L,F | `inc_ref` **gen-match** | same | +1 | — | — |
| any | `inc_ref` **gen-MISMATCH** | unchanged | **no-op** | — | stale weak handle; validate disables entity (FIX-B) |
| D,F | `dec_ref→>0` | same | -1 | — | — |
| D | `dec_ref→0` (unpinned) | R | 0 | enqueue; epoch++; dirty | — |
| F | `dec_ref→0` | R | 0 | enqueue (no device value); epoch++; dirty | — |
| F | `fill` gen-match (late) | unchanged | — | — | **Err((Failed,v))** → deferred-free (FIX-F defensive) |
| any pinned | `dec_ref→0` | unchanged | 0 | — (never enqueued) | slot 0 immortal |
| R | `retire` (fence elapsed) | V | — | dequeued | `take_at`(D)→destroy; None(F/cancelled-L); **gen++**; free-list |

- Enqueue-to-Retiring is **IDEMPOTENT** (guard: only a non-Retiring source state may enqueue) so a slot
  already queued can never be double-enqueued → no double-`take_at`/double-destroy (FIX-B).
- `inc_ref`/`dec_ref` carry the handle **generation**; a gen-mismatch is a no-op (a raw `Handle` copy
  held outside a carrier is WEAK/non-owning — only carrier components are strong refs). The v2
  `debug_assert!(false)` "a fresh handle can never name a Retiring slot" is DELETED — it is reachable
  from safe user code via a stale weak handle (FIX-B).
- Re-add can't alias a queued slot: `add` draws ONLY from the free-list, and a Retiring slot enters the
  free-list solely via `retire` (after destroy).
- **Never-attached reclamation (FIX-F):** retire triggers only on a `1→0` decrement; an `add` that is
  never attached stays `Loaded` at refcount 0 (speculative loads need explicit `unload`). Documented
  accumulation; high-water compaction is a follow-up (with O2).

## Four streaming pieces

1. **Refcount via hooks → Send resource → apply system.** On both `MeshHandle`/`MaterialHandle`:
   `on_insert` (fresh add AND replace-insert) → `RefDelta{kind, slot, gen, +1}` + `SyncRefGen`;
   `on_replace` (before overwrite, reads old) → `RefDelta{.. -1}`; `on_remove` → `RefDelta{.. -1}` +
   `ClearRefGen`. In-place `*mh = MeshHandle(new)` fires `on_replace(-1)+on_insert(+1)` → balanced +
   ref-gen re-derived. `RefcountDeltas` is a **plain Send `Resource`** (POD deltas), reached from the
   hook via `DeferredEcsMaster::resource_mut` (the sanctioned on_remove-counter pattern,
   `deferred_master.rs:105`). `RefDelta` carries `gen` (FIX-B). `apply_refcount_deltas` folds the buffer
   into each store, gen-checking each delta (mismatch → no-op).
2. **Generation-in-carrier + validate.** `MeshRefGen(u32)` / `MaterialRefGen(u32)` are TWO independent
   auto-maintained components (separate columns — binding one never clobbers the other). Written by a
   `SyncRefGen{e, lane, slot}` deferred command the hook enqueues (apply-time has NonSend-store access);
   drained BEFORE `validate_asset_refs`. `validate_asset_refs`: **O(1) early-out** when no store's
   `free_epoch` advanced (all-static/golden frames — provably zero-cost); on a churn frame, a branchless
   L1-resident `dirty.test(slot)` per visible row, and the full `slot_word` probe ONLY on churned rows —
   gen-mismatch/not-Loaded → `disable::<RenderEnabled>` (mesh) or substitute id 0 (material). The
   gather's count-pass key stays a thin 4 B `MeshHandle`; ref-gen never enters the gather. **Cost bound
   (FIX-C):** O(1) static; on churn O(visible) cheap bit-tests + O(referenced∩retired) full probes — the
   perf-critic's sanctioned "branchless L1-resident dirty-bitset" relaxation of "not a full scan". The
   bit-test may be FUSED into the gather's existing per-instance count-pass to avoid a second iteration.
3. **Deferred GPU-free (fence-gated, monotonic counter).** `FreeEntry{kind, slot, retire_frame}` (no
   value) + `FreeEntry::Device{buf, retire_frame}` for grown buffers. `retire_deferred_frees` runs LAST,
   after `wait_frame_in_flight`: pop `retire_frame <= current_monotonic_frame`; `store.retire(slot)`
   (single `take_at`); `ctx.destroy_buffer`/`destroy_blas`; gen-bump + free-list push; also drains the
   fill-reject queue. `current_frame` = the MONOTONIC renderer counter (`runner.rs` `frame_index`,
   `wrapping_add(1)`), NOT the `%FIF` slot. `retire_frame = F_free + FIF` (proof §Fence-gate).
4. **PathIndex dedup (append + merge, ZERO new unsafe).** `entries: VmColumn<PathEntry>` with a sorted
   prefix `[0,sorted_len)` + unsorted tail; lookup = binary-search prefix ∥ linear-scan small tail;
   insert = `push` to tail, sort-merge into the prefix when the tail exceeds a threshold. Uses only safe
   `push`/`set`/index-read (no shift/insert-at primitive needed). Host primitives get synthetic stable
   keys (`hash("@prim/cube/{size.to_bits()}")`) so `cube(size)×1000` dedups to one resident handle.

## GPU-mirror growth (grow-and-defer-old) — corrected inventory (FIX-D)

Only the **two CPU-mirrored SSBOs grow**: (a) the `MaterialTable` device SSBO + FIF staging ring, and
(b) the mesh **instance** SSBO. Both re-seed from CPU-resident stores (materials from the store
ComponentPool; instances assembled per-frame from the gather) — **no device→device copy**. Per-mesh
`vertex_buffer`/`index_buffer` are **self-allocated per mesh** (`build_mesh_gpu`) and freed individually
at retire — they are NOT a strided shared mirror and do NOT participate in mirror-growth (v2's listing of
them as growable was wrong).

On a frame where the store grew past device capacity, `grow_to(new_cap.next_power_of_two())`: alloc a
larger buffer, `seed_live_rows` (holes zeroed), route the OLD buffer through the SAME fence-gated queue
(`retire_frame = current + FIF`), set `rebind_pending = [true; FIF]`, rebind THIS frame's descriptor set.
`rebind_current(fif_slot)` runs **every frame right after `wait_frame_in_flight`**, gated ONLY on
`rebind_pending[fif_slot]`, **decoupled from `flush_if_dirty`** (FIX-E) — otherwise a non-dirty frame
leaves a FIF slot bound to the freed old buffer → UAF at M+FIF. `grow_to`/`rebind_current` run strictly
AFTER `wait_frame_in_flight` (updating an in-flight descriptor set is UB) — asserted with a
`debug_assert` on the ordering (FIX-E). Every FIF slot is rebound within `[M, M+FIF-1]` before the old
buffer's `retire_frame = M+FIF` elapses.

## Fence-gate correctness

After `wait_frame_in_flight` at monotonic frame M, all GPU work from frame ≤ M−FIF is complete (the
swapchain slot `M%FIF`'s previous submit was frame M−FIF). A slot marked Retiring at frame `F_free` may
be referenced by `F_free`'s submit; safe to destroy once `M ≥ F_free + FIF`. BLAS-address depth: the
TLAS is fully rebuilt per-frame from live instances, so a BLAS device address is consumed only within
the frame that gathered it — FIF=2 covers it; a `debug_assert` pins the per-frame-TLAS-rebuild contract
(if the TLAS ever becomes persistent/compacted or an async-compute queue with a separate fence is added,
deepen the queue). Deterministic fence-ordering test: retire is observed strictly after the fence-wait.

## Material-into-raster wiring + golden pipeline binding

Bind the `PER_INSTANCE_MATERIAL` gbuffer variant ONLY when the scene contains a non-default material
(any `MaterialHandle.0 != 0`). Default-material scenes (the goldens: `shadow_denoise_eval.rs:96` spawns
`MeshBundle::new(cube, …)` → `MaterialHandle(0)`) stay on the **frozen base `.spv`** → `dac6dbbb` /
`58f6c6c3` rest on a true frozen-shader guarantee. `gbuffer_mrt.fs.hlsl` is hand-authored HLSL (only the
`oct_encode`/`pack_material_id_ba` bodies eDSL-spliced between sentinels; `MOTION_VECTORS` `#ifdef`
precedent), so a `#ifdef PER_INSTANCE_MATERIAL` wrapper (reading `instance_materials[base+SV_InstanceID]`
via a `nointerpolation` varying, replacing the `=0` constant) is a legitimate hand edit;
`gbuffer_mrt_edsl_sync.rs`'s `contains()` guard stays green. Data path: `mesh_draw`/`csm_caster` size
the gather on `high_water()` (fixes the len() hole); a `material_ids` lane parallel to the instance ring
scatters each instance's `MaterialHandle.0` into a FIF-ringed instance-material SSBO (grows via §growth).

## Rung sequence (each atomic + gated)

| Rung | What | Gate | Byte-identity |
|---|---|---|---|
| **F1** | Unified store on standalone `ComponentPool` (`AssetBacking` via macro, packed `slot_word`, `refcount`/`free_epoch`/`dirty`/`pinned`, `take_at`, `fill→Result`, terminal Drop, conditional `unsafe impl Send/Sync`); delete `slot.rs` + 4 Vecs; retract "SlotColumn unsound". **Includes the two concrete backings so the workspace stays green:** `impl_asset_pod_backing!(MaterialGpu)` in `material.rs`, `impl AssetBacking for MeshGpu` + a no-op `drop_glue` (= `drop_in_place`, frees no device memory — matches MeshGpu's existing no-Drop contract) in `mesh.rs`. `Assets<T>` is constructed for both types in `boyko_app::runner.rs`, so these must land with F1 (orphan rules place them in `boyko_render`). | port ~30 unit tests + 256-proptest via `impl_asset_pod_backing!(u64)`; Miri-TB take_at exactly-once; **F1 slot-id parity test** (golden scene mints identical slot ids old-vs-new, FIX-F); build+clippy | goldens hold (CPU-container swap; append order preserved) |
| **F2** | State machine + refcount hooks (`on_insert/on_replace/on_remove`) + `RefcountDeltas` Send Resource + `apply_refcount_deltas` (gen-checked) + `Retiring` + slot-0 pin | refcount unit tests; in-place-rebind fires on_replace+on_insert; despawn-cascade reentrancy; slot-0-never-retires; joint `(slot,gen,state,refcount)` proptest oracle | N/A |
| **F3** | Static loader dispatch `HasLoaders` const-table; delete `Box<dyn Any>`/`TypeId`/`loaders` HashMap/`decode_thunk` | loader-dispatch tests rewritten dyn→HasLoaders | N/A |
| **F4** | `PathIndex` (append+merge) replacing `interned` HashMap; synthetic primitive keys | intern/dedup tests; cube×N→1 slot; binary-search proptest vs BTreeMap oracle | N/A |
| **F5** | `MeshRefGen`/`MaterialRefGen` two lanes + `SyncRefGen`/`ClearRefGen` commands + `validate_asset_refs` (free_epoch-gated) + gather sizing `high_water()` | staleness test (free+reuse→row skipped); two-lane independence; free_epoch early-out zero-cost; all-live golden byte-identity | goldens hold |
| **F6** | Deferred GPU-free + fence gate (monotonic) + `retire_deferred_frees` after `wait_frame_in_flight` + fill-reject routing | fence-ordering unit test; Miri-TB take/destroy exactly-once; churn stress (10k over 1k frames, destroy==create, no UAF) | N/A |
| **F7** | GPU-mirror growth (MaterialTable SSBO + instance SSBO only; FIF descriptor rebind; old-buffer defer) | grow-past-boot (mint slot 4 over boot=4 → no panic); rebind-under-FIF; multi-grow-per-window (two grows M,M+1 → both retire on own M+FIF, all slots rebind, FIX-F); old-buffer fence-gated destroy | N/A |
| **F8** | Material-into-raster: `#ifdef PER_INSTANCE_MATERIAL` VS/FS variant + `material_ids` lane + instance-material SSBO; base pipeline for default scenes | `gbuffer_mrt_edsl_sync.rs` green; MeshBundle-material-id==0 precondition; NEW 2-material golden; `dac6dbbb`/`58f6c6c3` hold | default scenes on frozen base → hold by construction |
| **F-obj** | OBJ sort-dedup (delete `HashMap<(i32,i32,i32),u32>`) | obj decode tests (same tri count, dedup == HashMap oracle) | N/A |

Ordering: F1 store under a green byte-identity gate → F2 lifecycle/refcount → F3/F4 remove last HashMaps
(independent, may parallelise) → F5 validation → F6 teardown → F7 growth → F8 raster wiring → F-obj.

**Invariants discovered during F2 review (load-bearing — do not regress):**
- **HARD GATE (W2):** F5's generation-check MUST land before F6 (retire/reuse). Until F5, `inc_ref`/`dec_ref`
  are generation-oblivious (RefDelta carries no gen); this is sound ONLY because no slot is retired-and-reused
  until F6. A stale weak `Handle(slot)` decrementing a reused slot would corrupt the new tenant's refcount —
  F5's gen-check closes it, so F6 (the first reuse) must not precede F5.
- **Store read invariant (C1):** `get_by_index` (and `get`) resolve a row iff its `live` bit is set. `dec_ref`
  may transition a `Loading`/`Failed` row (which has `live=0` and holds inert zeroed scratch, never a valid `T`)
  to `Retiring`; such a row must resolve to `None` (forming `&T` over the scratch is UB — a zeroed `MeshGpu`
  niche is immediate UB). Only a `Loaded→Retiring` row (`live=1`) resolves. `dec_ref` does not bump generation,
  so `state()`/`remove()` must have an explicit `Retiring` arm (both return `None`; `remove` must NOT `take_at`
  a Retiring row — the deferred-retire path in F6 is its sole owner) instead of `unreachable!()`.
- **Carrier rebind contract (W1):** a `MeshHandle`/`MaterialHandle` may be rebound ONLY via spawn or a
  SINGLE-component `insert` — never inside a migrating multi-component bundle that re-supplies the handle (the
  insert-migration overlap path fires `on_insert` but NOT the table `on_replace` → the old slot's ref leaks).
  Also: hooks fire on `insert`, NOT on a raw `Query<&mut MeshHandle>` deref write (none exist in-tree today).

**F5 locked mechanism (architect→soundness-critic, design-locked):**
- Generation lanes `MeshRefGen(u32)`/`MaterialRefGen(u32)` are `#[require]`d by the carriers (co-presence
  guaranteed; `Default = GEN_UNSYNCED = u32::MAX`, real gens are 29-bit so never collide). Reconciled with F2's
  ACTUAL 2 hooks: lane is set/re-synced ONLY on the `+1` path (no `ClearRefGen`); on removal `on_replace(-1)` does
  not rewrite it (dropped with the entity on despawn; orphaned-but-never-read on a bare remove; self-heals on
  re-insert). `RefDelta` grows `{entity, gen}`; `on_replace(-1)` reads the sibling lane via `get_component`
  (valid: despawn fires hooks PRE-structural-removal, `entity_api.rs:1035` before `:1071`) so the decrement
  carries the BIND generation. `dec_ref(slot, gen)` gen-checks (mismatch → no-op — closes the FIX-B stale-decrement
  corruption); `inc_ref → bool` state-guarded (refuses on Retiring/Vacant).
- **BLOCKER FIX (critic C):** `apply_refcount_deltas` stamps the lane on `+1` **UNCONDITIONALLY** with the
  attach-time `generation(slot)` — even when `inc_ref` REFUSES (resurrection: carrier binds an already-Retiring
  slot). Otherwise the refused-bind carrier stays `GEN_UNSYNCED`, which both makes validate skip it AND bypasses
  the dec gen-check → its later `-1` corrupts a reused slot's refcount (premature free / UAF at F6). With the
  unconditional stamp: validate sees `Retiring != Loaded` → disables it; the `-1` carries the attach-gen → mismatches
  the reused slot's gen → suppressed. Slot refcount still never rises (inc refused) → F5/F6 boundary airtight.
  `GEN_UNSYNCED` is thus only the transient pre-`apply` `Default` (apply runs `.before(validate)`, per-system flush).
- `validate_asset_refs`: `free_epoch` early-out (O(1), zero on churn-free/golden frames); on a churn frame a full
  dense O(visible) `u32`-compare (the plan's per-row `dirty.test` gating is DROPPED — it had a visible-later hole);
  gen-mismatch or `state != Loaded` → `disable::<RenderEnabled>` (mesh) / `insert(MaterialHandle(0))` (material).
  Ordered `apply → validate → gather`.
- `mesh_count = high_water()` (not `len()`) at `mesh_draw.rs:521/:559`, `csm_caster.rs:174` (fixes the hole where a
  live index exceeds `len()` once a hole exists; byte-identical on hole-free goldens).
- **Single point of failure (research: Bevy gets staleness free via a gen-keyed HashMap; we don't):** `validate`
  is the SOLE staleness backstop for the bare-slot carriers → every raw `MeshHandle.0`/`MaterialHandle.0` read site
  MUST be downstream of `validate` this frame; document each read site + audit completeness.

**HARD PREREQ before async streaming (F6/F7) — F5's validate is deliberately disable-only + latent today:**
- `fill` (Loading→Loaded) does NOT bump `free_epoch`, and `validate` never re-enables → a carrier bound while its
  asset is Loading would be disabled and stranded invisible once it finishes loading. Latent in F5 (in-tree loads
  are synchronous `add()`→Loaded; validate never fires on goldens). Before async `reserve`/`fill` streaming is
  exercised, ADD: (a) `fill` bumps a validation epoch + `validate` gains an enable path; and (b) DECOUPLE staleness
  from user visibility — `validate` disabling `RenderEnabled` fights `visibility_sync` (both drive that bit); use a
  separate `RenderStale` EnableTag the gather also filters on, instead of reusing `RenderEnabled`. (Bevy PR #18734
  is the same-frame-handle-swap race this defends against.)
- (c) **Hard `validate → gather` scheduler edge (F5 review O1):** F5's `validate_asset_refs .before(gather_*)` is
  currently pinned by ADD-ORDER only (deterministic today — NonSend systems are dispatcher-solo, lowest-index-first,
  per-system flush, and `plugins.rs` adds AssetRefcountPlugin before the gather closure — but EMERGENT, not an
  explicit contract). Before F6/F7 (when `validate` actually fires), fold the sketched
  `add_asset_validate_systems(&mut ScheduleBuilder) -> SystemKey` helper into the host gather closure and add the
  explicit `.before` edge (mirror `add_gpu_transform_pack`).
- (d) **Material substitution (F5 review W1, DEFERRED to F8):** F5's `validate` does NOT substitute stale materials
  (it disables stale MESHES only). Stale-material refcount corruption is already prevented by the `dec_ref` gen-check
  at despawn; the VISIBLE substitution (point a stale material at the default slot 0) is inert until F8 (the raster
  hardcodes material 0) and needs `Entity`-in-query / `RenderStale` infrastructure F8 will add — so it lands in F8,
  not F5. (The F5 dev's `&mut MaterialHandle`-in-`validate` workaround was removed: it bypassed the hook contract and
  dropped a retire ticket on a matching-gen Loading/Failed slot.)
- (e) **Serialize/`#[require]` version-skew (F5 review, S0-S3 concern):** `load_archetype` builds archetypes from
  the file's saved component-id list and does NOT run `#[require]` expansion → a `MeshHandle` row from a schema-older
  save would lack `MeshRefGen` and be AND-filtered out of `validate`'s query (silent, no panic). Latent (no such
  save in-tree; same-build round-trips serialize the lane). Also confirm the deserialize path does not fire the
  carrier hooks without the lane present (a `-1` with `GEN_UNSYNCED` against a never-incremented refcount).

## Test plan

- **PORT VERBATIM:** `assets.rs` ~30 unit + 256-proptest (`Assets::<u64>` via `impl_asset_pod_backing!`).
- **DELETE:** `slot.rs` intrusive-`next_free` tests; any `server.rs` HashMap-iteration/collision-internal test.
- **REWRITE:** `server.rs` loader-dispatch (F3), intern/dedup (F4, + BTreeMap oracle); `obj.rs` dedup (F-obj).
- **ADAPT + re-gate:** `asset_pipeline_integration.rs` (4) at F1/F3; `boyko_app` smoke `interp_smoke`/
  `sdf_room_smoke`/`room_smoke` at F5/F8 (assert default scenes bind the base pipeline).
- **NEW:** Miri-TB take/destroy exactly-once (F1/F6); joint state-machine oracle (F2); deterministic
  fence-ordering (F6); churn stress (F6); growth + multi-grow-per-window (F7); two-lane independence
  (F5); F1 slot-id parity; NEW 2-material golden (F8).
- **loom DROPPED** for the deferred-free/refcount path — it is dispatcher-serial (hooks fire only in the
  single-threaded apply window), so loom is vacuous; the real hazard is a GPU-fence happens-before loom
  cannot model. Gated instead by the fence-ordering test + Miri-TB + the model oracle.
- **Goldens** `dac6dbbb` + `58f6c6c3` re-run after F1/F5/F8 (SHA-256 of the framebuffer BMP; opaque draws
  order-independent + append-order-preserving mint ⇒ CPU-container swap invisible).

## Open questions (non-blocking)

1. **ComponentId budget:** each asset type consumes one id from the 512 cap (MeshGpu, MaterialGpu, + POD
   test types) — a handful today; if asset types proliferate, lift the resident store onto a dedicated
   `VmReservation`-owned column (localized `col`-type swap).
2. **Material staleness UX:** substitute id 0 (draw continues, neutral) vs skip-like-mesh — design
   substitutes id 0.
