# Phase 4 — The Four Abstract Core ECS Seams (GPU-Capability Without Graphics)

> **Status: REVISED (architect → 3-lens critic panel → architect revise; all CRITICAL/IMPORTANT findings
> resolved).** Branch `ecs`, 2026-06-17. **The "Post-Critic Binding Revisions" section at the BOTTOM is
> AUTHORITATIVE where it conflicts with the original decisions D1–D7 below** (the originals are kept for
> rationale/context; the revisions correct stale line refs, the Send model, the SCH15 form, the `PoolBacking`
> shape, the residency scan sites, the NonSend SystemParam surface, and turn every layout claim into a
> compile-time pin). Implement from the revisions where they overlap.
> Implements [docs/RENDER-PHYSICS-GPU-PLAN.md](RENDER-PHYSICS-GPU-PLAN.md) §5 (the four seams), §2 (CPU/GPU
> population partition), §1 (perf model), §6 (validation), §10 (metrics). The SINGLE sanctioned touch of
> `boyko_ecs` core. Conforms to Phase 1 ([docs/RHI-TRAIT-PLAN.md](RHI-TRAIT-PLAN.md)): the opaque
> `DeviceColumnHandle(u64)` is defined HERE in `boyko_ecs` (graphics-pure bare-`u64` newtype); `boyko_ecs`
> does NOT depend on `boyko_rhi`.

## Goal

Make `boyko_ecs` GPU-capable by cutting four abstract seams into core, such that a future
`boyko_render`/`GpuColumn` (Phase 5) can mark archetypes GPU-resident, store columns in device memory behind
an opaque `u64`, run GPU-compute systems ordered by the existing conflict graph, and home the RHI
`Device`/`Queue` in a `!Send`-safe resource — **without `boyko_ecs` naming a single graphics type or
depending on any graphics crate**, and with **0% measurable CPU cost when no GPU archetype exists**.

## Target metrics (acceptance gate)

- **0%-gate**: `bench_spawn_10k`, `bench_iter_*`, `bench_schedule_50_systems` within ±2% (criterion noise) of
  the pre-Phase-4 baseline; inner-loop assembly byte-identical (`cargo asm` spot-check on
  `ComponentPool::row_ptr`, `Schedule::find_ready`, `Query::for_each_chunk`).
- **Residency stamp**: one OR of a `u16` into `Archetype.flags` at mint (cold, once per signature for life);
  read cost = one `test`/`jz` on an already-loaded `u16`.
- **Miri**: all existing Miri-TB suites stay green; the device pool variant is `cfg(miri)`-compiled out to
  host-only.
- **Purity**: `grep -r "boyko_rhi|vulkan|Vk|Device" crates/boyko_ecs/src` finds only the abstract names
  defined here (`DeviceColumnHandle`, `GpuAccessIntent`, `ResidencyKind`, `SystemKind`, `NonSend*`) — no
  graphics types; no `boyko_rhi` in `Cargo.toml`.

## Invariants preserved (verified against live code)

- Archetype dedup key = `filtered_signature_mask` ONLY (`:151`, `:498`, `:674`) — residency is NEVER a second
  identity axis.
- `offset_of!(Archetype, columns) == 0` const-assert; `flags` stays `#[repr(transparent)] u16`.
- `ComponentPool::row_ptr` byte-identical for Host backing; all `// SAFETY` invariants + write-once base
  pointers preserved.
- Resources slab 32 B/slot, `Send + Sync` — untouched (so `Res`/`ResMut` are 0% impacted).
- Arena/ALLOC1 `!Send + !Sync` apply-window discipline — the NonSend seam reuses it verbatim, no new model.

---

## Decisions

### D1 — Residency = cold per-`ComponentId` table + a sticky `ArchetypeFlags` bit, deterministic from signature

Add `RESIDENCY_CLASS: [AtomicU8; MAX_COMPONENTS]` to `component_registry`, mirroring `STORAGE_KIND` (`:421`).
An archetype is `Gpu`-resident iff its signature contains ≥1 `ResidencyKind::Gpu` component — computed once at
mint, ORed into `Archetype.flags` as `GPU_RESIDENT = 1<<11` (bits 0..=10 used; bit 11 free — verified).
**0%-gate**: identical mechanism to the Phase-14a hook bits (the proven 0%-when-unused precedent). A CPU-only
world never sets the bit; consumers read the already-in-L1 `flags` and do one `test`/`jz`.
**Deterministic-from-signature** resolves the dedup contradiction (umbrella C2): residency is a pure function
of the dedup key's inputs → two spawns of the same signature ALWAYS agree → it can never fork identity.
`RESIDENCY_CLASS` is cold (written at registration, read once per component at mint); `AtomicU8`+`Relaxed`
matches `STORAGE_KIND`'s justification.
Rejected: residency as a separate per-archetype field (a second cold load + bytes; the bit is free); residency
in the dedup key (forks identity, doubles archetype count); per-spawn residency arg (the C2 contradiction).

### D2 — Conflict-on-mint rejection: loud `#[cold]` panic at both mint sites

At `create_archetype` (`:142`) and `add_existing_archetype` (`:481`), the residency scan folds into the
EXISTING `component_ids` loop (one extra `RESIDENCY_CLASS` load per component, cold path, zero new iteration).
If a signature mixes a `ResidencyKind::Gpu` component with a `ResidencyKind::CpuPinned` component →
`#[cold] #[inline(never)] residency_conflict_panic(...)`. A default/unclassified `Cpu` component is always
compatible with a `Gpu` signature (it becomes a device column in Phase 5). Release-present panic (not
`debug_assert`) — a silent wrong-residency archetype would corrupt the partition (the readback trap). Mirrors
`set_storage_kind`'s reclassify-panic. Rejected: `Result` return (would reshape the infallible mint API for a
programmer-error case); `debug_assert` only (release-silent corruption).

### D3 — `ComponentPool` gains a `PoolBacking` enum (NOT a trait); Host variant byte-identical

Replace `ComponentPool.vm: VmReservation` (`:112`) with `backing: PoolBacking`:
```rust
pub(crate) enum PoolBacking {
    Host(VmReservation),
    #[cfg(not(miri))] Device(DeviceColumn),   // handle:DeviceColumnHandle(u64) + device_len/cap
}
```
The three write-once base pointers (`buffer`/`added_base`/`changed_base`) **stay top-level `ComponentPool`
fields**, derived from the `Host` arm in `new` exactly as today. `row_ptr` reads `self.buffer`, never
`self.backing` → **byte-identical codegen**. `PoolBacking` holds only the reservation's *ownership* (its Drop
releases it), declared last (same Drop-order slot as `vm` today).
**Device-variant reconciliation** (Phase 5 consumes): no per-row ticks on device (change detection is
CPU-archetype only; `added_base`/`changed_base` are never-read sentinels); grow = realloc+copy+fence+re-fetch
(the cold `#[inline(never)] grow_rows` path), the `u64` handle MAY change on grow (sound — no CPU code caches
a device row pointer).
**Miri**: `Device` is `#[cfg(not(miri))]` → under Miri `PoolBacking` is a single-variant enum, niche-optimized
to the exact pre-Phase-4 layout; Miri sees today's `ComponentPool`. `DeviceColumnHandle(u64)` itself is
Miri-safe (bare int) and stays compiled in.
Rejected: moving base pointers into `Host` (forces `match self.backing` per `row_ptr` call — breaks the gate);
`Box<dyn ColumnStorage>` (vtable + heap, violates principles 1+5); a second `DeviceComponentPool` type (forks
every column path).

### D4 — The device-pool 0%-gate is the residency bit, NOT a per-access backing check

A CPU `Access` is built from CPU-archetype component ids; a GPU-resident archetype carries `GPU_RESIDENT`.
Phase 5 makes the CPU `Query` match loop **skip GPU-resident archetypes** via one
`archetype.is_gpu_resident()` test at **archetype-collection time** (once per query rebuild — already a cold
path that walks archetypes), NOT in the per-row hot loop. Therefore `row_ptr` on a `Device` pool is **never
reached** from CPU code — the hot inner loop only ever sees Host pools by construction, so no tag check on the
hot path. The skip is itself 0% when no GPU archetype exists (bit never set → identical candidate set).
Phase 4 ships the seam: `pub(crate) fn Archetype::is_gpu_resident(&self) -> bool` (one `flags.contains`); the
consume-side skip is the documented Phase-5 fill. Phase 4 mints no device pool (the classification table is
empty until a GPU component registers) → Phase 4's own benches are trivially 0%.

### D5 — `SystemKind` enum replaces `SystemBox.is_exclusive`, same 1-byte hot load

```rust
#[repr(u8)] pub(crate) enum SystemKind { CpuConcurrent=0, CpuExclusive=1, GpuCompute=2 }
impl SystemKind { #[inline] fn runs_on_dispatcher(self) -> bool { matches!(self, CpuExclusive|GpuCompute) } }
```
Replace `SystemBox.is_exclusive: bool` (`system_box.rs:80`) with `kind: SystemKind` (`#[repr(u8)]` = 1 byte,
same offset/size/padding; `SystemBox` stays 40 B). The dispatch gate at `schedule.rs:953`
(`if ...is_exclusive`) → `if ...kind.runs_on_dispatcher()` (a `matches!(.., 1|2)` → one range check, same cost
class). `GpuCompute` runs dispatcher-only (it records/submits via the NonSend RHI, touchable only when
`running == 0`); the conflict graph still orders it. **SCH15 migration**: the `is_exclusive ==
access().is_universal()` debug-assert (`:232`) → the implication `kind==CpuExclusive ⟹ access().is_universal()`
(`GpuCompute` is set by an explicit adapter marker, not derived from access). `kind` resolved at
`schedule_builder.rs:369` from `(is_gpu, requires_dispatcher, access.is_universal())`; `is_gpu` rides the
`SystemDescriptor` (default false → zero change for every existing system).
**Migration scope correction**: `is_exclusive` is `pub(crate)` with ~10 real sites (NOT 78), mechanical.
Rejected: `is_exclusive` + separate `is_gpu` bool (two loads/branches); three bools (invalid states).

### D6 — `NonSendResource` via a parallel `!Send` slab + apply-window gate (no new soundness model)

`pub trait NonSendResource: 'static {}` (NO `Send+Sync` bound — the point). A `NonSendResources` slab on
`EcsMaster`, structurally identical to `Resources` but **`!Send + !Sync`** (no `unsafe impl Send`), via a
parallel `nonsend_resource_registry`. `NonSendRes<'w,R>`/`NonSendResMut<'w,R>` SystemParams mirror
`Res`/`ResMut`; their `init_access` calls `mark_requires_dispatcher()`. A system with `requires_dispatcher`
resolves to `SystemKind::CpuExclusive` → runs only on the dispatcher in the apply-window (`running == 0`,
`schedule.rs:555`).
**Soundness (no new model)**: at `running == 0` no worker is live → the dispatcher has exclusive world access
→ the `!Send` slab is touched single-threaded on its owning thread. Identical to the retired-Arena `!Send`
discipline. A separate slab keeps `Resources` `Send+Sync` (0% `Res`/`ResMut` impact) and makes "NonSend" a
type-level fact (different slab/registry). Mirrors Bevy's separate non-send storage.
**0%-gate**: the slab is lazy (`Option<Box<...>>` / the Phase-12.6 lazy-field pattern) → zero alloc when
unused; no system sets `requires_dispatcher` → `SystemKind` resolution + schedule codegen unchanged;
`Res`/`ResMut` untouched.
Rejected: `unsafe impl Send` on RHI types + `Resources` (a lie → UB); `Mutex<Device>` (lock on a path that is
single-threaded-at-the-boundary anyway).

### D7 — `GpuAccessIntent` abstract per-system descriptor; conflict graph stays ordering-only; lowering in `boyko_render`

`Access::conflicts_with` (`access.rs:163`) is symmetric/undirected/coarse — perfect for ORDERING (is there an
edge), carries nothing a Vulkan barrier needs. So: the conflict graph (unchanged) gives edges; a new
graphics-pure `GpuAccessIntent` (abstract `GpuStage`/`GpuAccess` enums + `DeviceColumnHandle`, NO `Vk*`)
declared by a GPU system gives the stages/accesses; `boyko_render` lowers `(edge, intent_src, intent_dst)` →
`vkCmdPipelineBarrier`. The intent is graphics-pure so it CAN live in core (`SystemMeta`, cold,
`Option<Box<...>>`, `None` for every CPU system → 0%); the lowering needs `Vk*` → stays in `boyko_render`.
Superset-correct obligation (Phase 5): `boyko_render` over-synchronizes + a sync-validation golden test. Core
only exposes honest abstract edges + intents.
Rejected: Vulkan stage masks in `Access` (pollutes core, bloats the 192 B `Access`); auto-tracking resource
state in core (per-resource state machine on the hot path).

---

## Data structures (key types)

```rust
// Seam 1 — component_registry.rs (mirror STORAGE_KIND :421)
#[repr(u8)] pub enum ResidencyKind { Cpu=0, Gpu=1, CpuPinned=2 }
static RESIDENCY_CLASS: [AtomicU8; MAX_COMPONENTS] = [const { AtomicU8::new(0) }; MAX_COMPONENTS];
pub fn residency_class(id: usize) -> ResidencyKind;            // Relaxed, range-checked → Cpu
pub(crate) fn set_residency_class(id: usize, k: ResidencyKind); // write-once, reclassify-panics
pub fn install_residency_class<C: Component>(id: usize);        // from C::RESIDENCY (derive)

// archetype_flags.rs
impl ArchetypeFlags { pub const GPU_RESIDENT: u16 = 1<<11; #[inline] pub const fn is_gpu_resident(self)->bool; }

// Seam 3 — graphics-pure opaque handle (boyko_rhi's slot<->u64 bridge packs into this)
#[repr(transparent)] pub struct DeviceColumnHandle(pub u64);
// component_pool.rs: vm: VmReservation  ->  backing: PoolBacking (Host | #[cfg(not(miri))] Device)
#[cfg(not(miri))] pub(crate) struct DeviceColumn { handle: DeviceColumnHandle, device_len: usize, device_cap: usize }

// Seam 4 — system/system_kind.rs + system/gpu_intent.rs (graphics-pure)
#[repr(u8)] pub(crate) enum SystemKind { CpuConcurrent=0, CpuExclusive=1, GpuCompute=2 }
#[repr(u8)] pub enum GpuStage { Compute, Transfer, Indirect }
#[repr(u8)] pub enum GpuAccess { Read, Write }
pub struct GpuAccessIntent { pub stage: GpuStage, pub touches: /* fixed inline array | Box<[(DeviceColumnHandle, GpuAccess)]> */ }
// SystemMeta gains (into existing tail padding → no size_of bump; re-pin 256 B assert):
//   gpu_intent: Option<Box<GpuAccessIntent>>   requires_dispatcher: bool
```

`Component` trait gains `const RESIDENCY: ResidencyKind = Cpu` (default → 0% for every existing component);
the derive emits `install_residency_class` like `install_storage_kind`.

---

## Integration sites (file:line)

| Seam | File | Site | Change |
|---|---|---|---|
| 1 | `component_registry.rs` | `:421` (STORAGE_KIND) | add RESIDENCY_CLASS + 3 fns |
| 1 | `archetype_flags.rs` | const block | `GPU_RESIDENT = 1<<11` + `is_gpu_resident` |
| 1 | `archetype.rs` | `create_by_ids:283`, `register_component_inplace:380` | seed bit into `flags` |
| 1/2 | `archetype_master.rs` | `create_archetype:142`, `add_existing_archetype:481` | residency scan + `#[cold]` reject, folded into existing loops |
| 3 | `component_pool.rs` | `vm:112`, `row_ptr:531` | `vm`→`backing: PoolBacking`; `row_ptr` UNCHANGED |
| 3 | new `memory/device_column.rs` | — | `#[cfg(not(miri))] DeviceColumn` + `DeviceColumnHandle` |
| 4 | `system_box.rs` | `is_exclusive:80` | → `kind: SystemKind` |
| 4 | `schedule.rs` | gate `:953`, SCH15 `:232` | `runs_on_dispatcher()`; implication |
| 4 | `schedule_builder.rs` | `:369` | resolve `SystemKind` |
| 4 | `system_meta.rs` | `:322` (256 B) | `gpu_intent` + `requires_dispatcher` into tail padding |
| 2 | `resource.rs` | spec comment `:18` | real `NonSendResource` trait |
| 2 | new `resources/nonsend_*` + `params/nonsend_*` | — | mirror `Resources`/`Res`/`ResMut` (`!Send`) |
| 2 | `ecs_master.rs` | — | lazy `NonSendResources` field + 3 accessors |
| 1 | `Component` trait + `boyko_macros` | — | `const RESIDENCY` (default Cpu) + emit install |

---

## Implementation waves (each independently compilable + Miri-green)

1. **Residency classification (leaf)**: `ResidencyKind` + `RESIDENCY_CLASS` + 3 fns (copy STORAGE_KIND block);
   `GPU_RESIDENT` const + `is_gpu_resident`; `Component::RESIDENCY` default + derive emit. *No behavior change.*
2. **Stamp + reject at mint**: seed bit in `create_by_ids`/`register_component_inplace`; scan + `#[cold]` reject
   in both mint sites (folded into existing loops). *Synthetic-Gpu test + Cpu-only property test + mixed panic;
   spawn bench flat.*
3. **PoolBacking**: `DeviceColumnHandle`; `vm`→`backing` with `#[cfg(not(miri))] Device`; `new` wraps Host; Drop/
   grow via Host; `row_ptr` unchanged (`cargo asm` spot-check). `device_column.rs` stub. *17 pool tests + proptests;
   Miri single-variant; pool bench flat.*
4. **SystemKind**: `system_kind.rs`; `is_exclusive`→`kind`; migrate gate + SCH15 implication + resolution;
   `is_gpu` default false. *50-system bench flat; scheduler tests; Miri-green.*
5. **NonSend resources**: `NonSendResource` trait; `!Send` slab + registry; lazy `EcsMaster` field + accessors;
   `NonSendRes`/`NonSendResMut` params → `mark_requires_dispatcher` → `CpuExclusive`. *Insert/get round-trip; `!Send`
   compile-fail test; NonSend system dispatcher-only; Miri NonSend test; 0% when unused.*
6. **GpuAccessIntent**: `gpu_intent.rs`; `SystemMeta` fields (re-pin 256 B assert) + accessors. *Size assert;
   purity grep; Miri-green.*

Waves 1, 3, 6 are cross-independent (parallelizable); 2 depends on 1; 4 before 5.

---

## Validation

- **Benches (acceptance)**: `bench_spawn_10k`/`bench_iter_*`/`bench_schedule_50_systems` A/B vs baseline ±2%;
  `cargo asm` byte-identical on `row_ptr`/`find_ready`/`for_each_chunk`.
- **Unit**: residency default Cpu + round-trip/reclassify-panic; Gpu component → `is_gpu_resident`; Cpu-only →
  not; mixed `Gpu`+`CpuPinned` `#[should_panic]`; `runs_on_dispatcher` truth table; NonSend insert/get +
  `requires_dispatcher` + `!Send` compile-fail; `PoolBacking::Host` round-trip + (non-miri) `Device` constructs.
- **Property**: no Cpu-only archetype carries `GPU_RESIDENT`; residency is a pure function of the signature.
- **Miri**: all existing suites green (`Device` cfg-out, single-variant enum); new `miri_phase4_nonsend.rs`.
- **`debug_assert!`/release-`assert`**: mint `!(saw_gpu && saw_cpu_pinned)` (release panic); SCH15 implication;
  `SystemMeta` size 256 B.

---

## Open questions (for the architecture-critic)

1. **`requires_dispatcher` → `CpuExclusive` over-serializes NonSend systems** (forces `running==0`). Fine for
   the foundation (1-2 submit systems/frame); a `DispatcherConcurrent` kind is YAGNI. Confirm.
2. **`DeviceColumnHandle` home**: `boyko_ecs::memory` (column-scoped) vs `identifiers/primitives.rs` (where other
   ids live). Lean `memory`. Both graphics-pure.
3. **`GpuAccessIntent.touches` storage**: fixed inline array (caps touch count) vs `Box<[...]>` (setup-time cold
   alloc). Lean fixed array for the foundation (≤ a few columns/system).
4. **`Component::RESIDENCY` const vs runtime builder**: keep `set_residency_class` `pub(crate)` (derive-only) vs
   expose a public runtime path for `boyko_render` to classify non-derive foreign types. Flag.
5. **Conflict-reject coverage**: Phase 4 can't mint a `Gpu` archetype end-to-end (no `boyko_render`); the reject
   path is testable only with a synthetic `Gpu`-classified component. Judge: sufficient (the reject logic is pure
   on the classification table, independent of any device pool existing).

---

# Post-Critic Binding Revisions (AUTHORITATIVE)

> Resolutions to the 3-lens critic panel (perf/0%-gate APPROVED_WITH_NITS; soundness REVISE C1–C3; scope REVISE
> C1–C5), grounded against the LIVE tree. Where these conflict with D1–D7 above, THESE WIN. Stale `:NNN` refs in
> the originals are corrected here.

## CR-A — `EcsMaster: Send/Sync` + the NonSend slab. RESOLVED: type-erased pointer slab + extended SEND1 protocol justification (NOT an `!Send` slab, NOT an `unsafe impl` lie).

The `NonSendResources` slab stores **type-erased entries** (`NonNull<u8>` data ptr + `unsafe fn(NonNull<u8>)`
drop fn + `TypeId`), exactly like `Resources`. The slab is therefore **unconditionally structurally
`Send + Sync`** (raw pointers + POD, no `R` value inline) → adding it as an `EcsMaster` field touches NEITHER
the SEND1 `unsafe impl` (`ecs_master.rs:3395-3396`) NOR its compile gate. The `!Send` payload `R` is reachable
ONLY through `NonSendRes`/`NonSendResMut::get_param` (`unsafe`), whose SAFETY contract is the apply-window
single-thread-touch invariant: CR-B routes every NonSend system to `runs_on_dispatcher()`, so the payload is
constructed/projected/dropped only on the dispatcher thread (the `Schedule::run`-caller, `schedule.rs:563`),
never concurrently with a worker. This is the existing `Resources` erased-pointer + protocol model,
strengthened from "never concurrently *aliased*" to "never concurrently *touched*". Vulkan's
external-synchronization rule needs "never concurrent", not "same thread across frames", so this satisfies it.
- **SEND1 doc extension (`ecs_master.rs:3370-3394`)** — add SEND10: *`nonsend_resources` is structurally
  `Send + Sync` (erased `NonNull<u8>` + fn-ptr + `TypeId`); its `!Send` payload is touched exclusively on the
  dispatcher thread in the apply-window, routed by `runs_on_dispatcher()`; no worker cell can reach a value
  (the slab is `Send` but the value needs an `unsafe` accessor whose contract holds only on-dispatcher).*
- Rejected: side-table not owned by `EcsMaster` (needs its own drop-order story outside the C5 contract);
  downgrade `EcsMaster` to `!Send` (un-parallelizes the whole Phase-9 scheduler the moment one NonSend
  resource exists — unacceptable for 1–2 submit systems/frame).

## CR-B — SCH15 vs non-universal `CpuExclusive`. RESOLVED: NonSend params declare UNIVERSAL access; SCH15 stays an EQUALITY for `CpuExclusive`; `GpuCompute` is the marker carve-out.

A `requires_dispatcher`/NonSend system declares **universal access** in `init_access` → `is_universal()` is
true → the EXISTING derivation at `schedule_builder.rs:369` sets it `CpuExclusive` → SCH15 stays a literal
**equality** (`kind == CpuExclusive  <==>  access().is_universal()`). The conflict graph serializes it (universal
access conflicts with all, `access.rs:163-171`); EXC2 (`schedule.rs:953-967`) runs it solo. The original D5
"implication" rewrite is **withdrawn**. Final form:
```
SCH15 (Phase 4):  (sb.kind == CpuExclusive)  <==>  sb.system.access().is_universal()
                   AND  sb.kind == GpuCompute is set ONLY by the explicit GpuCompute marker
                        (no access constraint; Phase-5-scheduled; runs_on_dispatcher() still forces solo via EXC2)
```
`kind` resolution at `:369`: GpuCompute-marker-first → else `is_universal()` → `CpuExclusive` → else
`CpuConcurrent` (byte-identical to today's `is_exclusive` derivation, widened to 3-valued). There is NO
`requires_dispatcher`-derived `CpuExclusive` that bypasses universal access.

## CR-C — `ComponentPool::drop` over a Device pool. RESOLVED: Device pool keeps Host `self.len == 0` for life.

Device row count lives ONLY in `PoolBacking::Device(Box<DeviceColumn>).device_len`; Host `self.len == 0`, so
`Drop`'s `for row in 0..self.len { drop_fn(row_ptr(row)) }` (`component_pool.rs:1690-1712`) is a no-op — never
`drop_in_place` over uninitialized Host bytes. Device teardown = `DeviceColumn::drop` (Phase 5: RHI release of
the handle), never the CPU `drop_fn`. Add `debug_assert!(!self.backing.is_device() || self.len == 0, ...)` at
the top of `Drop::drop`. Phase-5 obligation: a `ResidencyKind::Gpu` type whose CPU `T::drop` has host-side
meaning is rejected at residency-install (device bytes never see the CPU drop).

## IM-1 — `PoolBacking` host size. RESOLVED: `Device(Box<DeviceColumn>)` (8 B ≤ `VmReservation`'s 16 B → `PoolBacking` stays 16 B; host build pays zero). Add `const _: () = assert!(size_of::<ComponentPool>() == <pre-Phase-4 literal>)` on BOTH host and `#[cfg(miri)]` builds (the swap must add zero bytes).

## IM-2 — residency scan/seed sites (corrects D2's stale claim that `create_archetype:142` has a per-component loop — it does NOT).
- **Per-component `GPU_RESIDENT` OR** rides the existing hook-bit loops: `archetype.rs create_by_ids:318-326`
  (the `flags` accumulator, after `flags.insert_from_hooks`) and `register_component_inplace:387-398` (single
  component; pure bit-stamp, **never rejects**).
- **Set-level `saw_gpu && saw_cpu_pinned` conflict scan + `#[cold] #[inline(never)] residency_conflict_panic`**
  lives at the single full-slice funnel `create_by_ids:283` (reached by both `ArchetypeMaster` paths: the slab
  builder `archetype_bundle.rs:425` calls `create_by_ids`/`register_component_inplace`; `add_existing_archetype`
  consumes a `create_by_ids`-built archetype).
- `add_existing_archetype:481` re-derives the GPU bit + re-runs the set scan from its OWN `component_ids`
  (OBS-SEED2 pattern `:522-524`) — NEVER trusts incoming `archetype.flags`. (O1: it is currently DEAD CODE —
  keep the defensive stamp, but focus tests on the live `create_by_ids` funnel.)
- "Zero new iteration" corrected to: one extra `residency_class` load per component in the existing
  `create_by_ids` walk + a 2-bool fold; no new loop.

## IM-3 — full `self.vm.*` rewrite enumeration. `vm`→`backing: PoolBacking` at `:112` (last field, same Drop slot); `:199` (`new`: `vm.base()` → host accessor / `match`); grow commits `:372,:392,:396,:496,:500` → `if let PoolBacking::Host(vm) = &mut self.backing { ... } else { #[cold] unreachable!() }`. `grow_rows`/`grow_rows_zst` Device arm = `#[cold] unreachable!` for Phase 4 (mints no Device pool); Phase 5 fills realloc+copy+fence. `row_ptr:531-548` UNCHANGED (reads `self.buffer`).

## IM-4 — full NonSend SystemParam surface (the Phase-8cd/14b missed-forwarder lesson). `NonSendRes`/`NonSendResMut` implement: `init_state` (nonsend registry id); `init_access` (declares **universal access** per CR-B + sets `requires_dispatcher` on `SystemMeta`); `get_param` (`unsafe`; via new `UnsafeEcsCell::nonsend_resources[_mut]()` by-value accessors mirroring `resources[_mut]()`); `apply` (default no-op — but the variadic-tuple `apply` forwarder must forward them; guarded by a behavioral test); `new_archetype` (no-op). `mark_requires_dispatcher` lives on `SystemMeta`. **Behavioral test required** (not compile-only): `nonsend_system_runs_on_dispatcher_and_observes_resource` — assert the param was actually fetched (counter changed → guards the silent-no-op class) AND `thread::current().id() == run-caller` (dispatcher-only).

## IM-5 — extend `ComponentPool`'s `unsafe impl Send/Sync` SAFETY (`:1665-1673`) with a Device-arm bullet (handle is `Copy` POD `u64`; device backing never touched concurrently by CPU per D4; no Host aliasing since Host `len==0`). Add `fn _assert() where DeviceColumn: Send + Sync {}` under `#[cfg(not(miri))]`.

## IM-6 — every layout claim → COMPILE-TIME pin:
1. `size_of::<ComponentPool>()` host + miri pins (IM-1).
2. `SystemMeta == 256`: append `gpu_intent: Option<Box<GpuAccessIntent>>` (8 B) + `requires_dispatcher: bool`
   (1 B) AFTER `this_run` (`:113`) — `#[repr(C)]` append-only (cannot reorder existing fields); fits the 24 B
   tail padding (232 + 9 = 241 ≤ 256). Keep the `:324` 256-pin as the tripwire.
3. `OnceLock<SystemMeta> <= 320` BSS tripwire (`:49-53`) re-verified (unchanged since size stays 256).
4. `GPU_RESIDENT == 1<<11` + distinctness pin in the `archetype_flags.rs:251-335` ledger (bits 0..=10 used —
   `HAS_ENTITY_OBSERVER = 1<<10`; bit 11 is the first free bit). `size_of::<ArchetypeFlags> == 2` unchanged.
5. **Wave-3 acceptance gate**: run the existing pool/drop Miri-TB suites on the host-only `#[cfg(miri)]`
   single-variant `PoolBacking` build and prove green BEFORE wave 3 is accepted.

## MINORs folded
- **M1**: `RESIDENCY_CLASS` follows `STORAGE_KIND`'s write-once-before-first-mint discipline (Relaxed mint-read
  sound; `set_residency_class` is a guarded `store`, not a CAS; same-id race benign, cross-kind = debug panic).
- **P5**: `nonsend_resources: Option<Box<NonSendResources>>` is a LAZY field (None until first insert; 0 alloc
  when unused), UNLIKE the eager `resources`. Declared immediately AFTER `resources` (`ecs_master.rs:144`) — the
  C5 drop-order contract (`resources` first) is preserved.
- **P7**: Miri single-variant = "a single-variant enum carries no discriminant, so `PoolBacking` has
  `VmReservation`'s exact layout under Miri" (proven by the `#[cfg(miri)]` size pin).
- **P4/P6**: `GpuCompute` reuses `runs_on_dispatcher()` → serialized identically to `CpuExclusive` (EXC2 solo +
  `break`). Phase-5 forward note: NO `match self.backing` may leak into the host `add()`/`add_typed()` warm
  capacity compare (`:574,:620`) — those stay byte-identical (the 0%-gate); the Host/Device split is resolved
  at the COLD grow/new paths + query-collection-time skip, never the hot add/iter loop.
- **Q-verdicts**: Q1 confirmed (subject to CR-B); Q2 `DeviceColumnHandle` in `memory` OK; Q3 keep
  `GpuAccessIntent` boxed (`Option<Box>`) with a fixed inline `touches` array; Q4 expose a PUBLIC runtime
  classify path (`pub fn` + write-once/reclassify-panic) in addition to the derive; Q5 synthetic-component
  reject test sufficient.

## Live-tree path anchors (the original `:NNN` were stale)
`ecs_master.rs` (SEND1 `:3370-3409`, fields `:102-144`); `unsafe_ecs_cell.rs` (`:341-342`, accessors
`:236-320`); `schedule.rs` (apply-window `:553-565`, SCH15 `:229-234`, EXC2 `:953-967`); `schedule_builder.rs`
`:369`; `conflict_graph.rs` `:108-118`; `access.rs` `:125-171`; `component_pool.rs` (field `:112`, `new`/base
`:199`, grow `:372/392/396/496/500`, `row_ptr` `:531-548`, Drop `:1675-1718`, Send/Sync `:1665-1673`, warm add
`:574/620`); `archetype.rs` (`create_by_ids:283-330`, `register_component_inplace:380-399`);
`archetype_master.rs` (`create_archetype:142-229`, `add_existing_archetype:481-550`, `get_or_create:667`);
`archetype_bundle.rs` (`:425`, `:513`); `system_meta.rs` (`#[repr(C)]:72`, tail `:105/113`, size pin
`:324-325`, BSS `:49-53`); `archetype_flags.rs` (ledger `:29-69`, size pin `:323`, distinctness `:251-335`);
`system_param.rs` `:80-172`; `params/res.rs` `:85-139`.
