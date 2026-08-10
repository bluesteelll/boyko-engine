# Profiling — the game-facing half

<!-- CONTRACT
provides: profiling/game-facing-surface
assumes: substrate/clock-source
assumes: substrate/loss-vocabulary
assumes: seam/lifecycle-order
assumes: profiling/goal-and-audiences
assumes: profiling/emission-abi
assumes: profiling/store-and-fold
assumes: profiling/statistics-discipline
-->

**Carved from** `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §"Scope extension: the game-facing half"
(D20, D22, D23, D25, D26, D27, D28), the game-facing block of §Data structures, the game slice of
§Public API, and algorithms A8 and A10. Diff against that document until it is retired.

---

## Scope extension: the game-facing half

The owner's requirement is that games use this system, collecting as much data as possible, with
maximum flexibility. What follows are the decisions that requirement forces, including the two
places where **the requirement as literally stated is a bad idea for this engine and is refused
with a reason**.

**Where this file sits against D19.** The two-authoring-paths mechanism itself — `declare_zone!`
vs `register_zone`, the id space, the ring regions, and the partition keyed on the **declaring
crate** rather than on the macro (B3) — is `profiling/emission-abi`'s, in
`01-EMISSION-STORAGE.md`. What is carried here is the *consequence for a game*, which is the
half C-II decides and the half the extension's answers speak to:

- A Rust plugin crate writes `declare_zone!` verbatim, is re-exported through
  `boyko_ecs::prelude`, and pays the engine's ≤ 12 ns — **X1 needs no new mechanism.** What rev 4
  adds is one line at that crate's root, `profiling_partition!(User)`, without which it does not
  compile.
- A **data-defined** zone — config, script, mod — takes the dynamic path: ≤ 14 ns, ≤ 18 ns across
  an FFI/script boundary, **always** the `User` partition, and **not tier-foldable**, because a
  data zone has no compile-time tier.
- **C-II's cost to the game side, stated:** a dynamic zone costs ≤ 14 ns instead of ≤ 12, cannot
  be compile-time tier-folded, and is refused past `user_zone_budget`. **C-II's cost to the
  engine side:** every crate that declares a zone must state its partition once at its crate
  root, or it does not compile.

**Three environmental facts this half rests on are owned by `00-GOAL-TARGETS.md`** and are cited,
not restated: `EnableTag` toggles fire **no hook and no observer**
(`crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:77-88`); an enable-bit tag must be a
**fieldless** struct and the *read* path carries **no storage-kind assert at all**
(`crates/boyko_macros/src/component.rs:580-604`; `enable_tag_api.rs:201-215`); and a **deferred**
enable/disable exists and lands **inside the same frame**
(`crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs:220`, `:236`, `:249`, `:262`;
`schedule.rs:722-726` concurrent path, `:1130-1133` dispatcher-inline path). D20 below is what
those three facts force.

---

## D20 — Runtime toggling: `ProfilingScope` is an ECS entity with an `IsEnabled` bit — projected by the FOLD, because the kernel fires no observer

> **SHIPPED at profiling rung 11** (`crates/boyko_ecs/src/ecs/core/profiling/ecs_control.rs`), with
> four corrections to what is written below. They are folded into the text at each point and
> collected here so a reader meets them before the prose that predates them:
>
> 1. **The projection owns bits `8..64`, not the whole word.** `PROJECTED_SCOPE_BASE = 8`; `arm` /
>    `disarm` keep the channel half, `ROOT_SCOPE` included. The fold's entry gate is `any_armed()`
>    and the projection is a step of the fold, so a projection able to clear the last bit would stop
>    the fold and, with it, itself — a one-way switch with no diagnostic. `G12`'s re-enable clause
>    is the assertion that it is two-sided.
> 2. **There is no `scope_entity[b]` table and there should not be** — A8's loop is a query under
>    `Enabled<ProfilingScopeEnabled>`, so the bit → entity association *is* the component. See A8.
> 3. **`register_scope` returns the `ProfilingScope`**, not a bare `u8`. See the API section.
> 4. **`Profiler::latency()` publishes D25's CPU row only.** MEASURED: nothing outside `boyko_ecs`
>    pushes a sample, and `arm` has no non-test caller, so this store holds no GPU spans to have a
>    GPU lag about. See D25.

**`ARM_MASK: AtomicU64`** replaces rev 2's `CHANNEL_MASK: AtomicU32`. Identical instruction count
on the hot path (a `bt` against a 64-bit word). Bit layout:

```
0..7    channels        SchedulerCpu, GpuPass, Counter, Frame, User0..3
8..31   engine scopes   Render, Physics, Input, Assets, UI, Audio, Net, …
32..63  game scopes     assigned by register_scope()
```

**`ARM_MASK` is the profiler's RUNTIME axis, and it is zero by default because `.bss` is zero.**
It is the flag a shipped binary can be asked to turn on after it shipped; it is *not* the axis
that reaches literal zero per-site cost. The two axes, what each can and cannot do, and the
honest three-row cost table are S13's, in `SEAM.md` — this file does not restate them and does
not soften them. What belongs here is the one consequence for a game: **nothing in this section
runs, allocates, calibrates, opens a file or spawns anything until `Profiler::arm` runs**, and
`arm` *is* the enable path.

**The extension proposed an `IsEnabled` *observer* projecting into `ARM_MASK`. That mechanism
does not exist and cannot be built without a kernel change.** `enable_tag_api.rs:77-88` documents
the enable path as *"O(1) warm: no migration, no structural-generation bump, **no hook / observer
fire**, no deferred drain"* — the absence of a fire is precisely what buys the O(1) toggle. Any
design that "projects on the transition" is unimplementable here.

**Capability and data are SEPARATE components, because a bitset tag may not carry fields (B2).**
Rev 3 wrote `#[derive(Component)] pub struct ProfilingScope { pub bit: u8, pub name: &'static str }`
and used it as the enable tag. The macro refuses that outright — `reject_non_zst_bitset_tag`
(`crates/boyko_macros/src/component.rs:580-604`) accepts only a fieldless struct — and, worse, if
the id were somehow probed anyway the *read* path would not even complain:
`is_enabled → test_enable_bit` (`enable_tag_api.rs:201-215`) has no storage-kind assert, finds no
enable column and returns `false`. Rev 3's projection would therefore have produced an **all-zero
`ARM_MASK` in every build, silently** — a profiler permanently disarmed with no diagnostic. So:

```rust
/// CAPABILITY — the runtime on/off bit. Fieldless, by the macro's requirement.
#[derive(Component)]
#[component(storage = "bitset")]
pub struct ProfilingScopeEnabled;

/// DATA — an ordinary table-storage component on the same entity.
#[derive(Component)]
pub struct ProfilingScope { pub bit: u8, pub name: &'static str }
```

This is the project's capability/state rule applied exactly as written: capability = component
presence (`ProfilingScope`), runtime on/off = the kernel enable bit (`ProfilingScopeEnabled`).

**Replacement for the observer — the projection is a step of the fold, not a system and not an
observer.**

```
fold(world: &mut EcsMaster, ...):
    0. scope projection:  for b in registered_scopes:            // scope_count, typically < 16
           bit(b) = world.is_enabled::<ProfilingScopeEnabled>(scope_entity[b])   // &self, ≤ 5 ns
       if bits != ARM_MASK { ARM_MASK.store(bits, Release) }      // one store only on change
    1. .. the sample fold ..
```

**⚠️ SHIPPED DIFFERENTLY, in two ways that matter.** The pseudocode above is kept because its
*claims* are the shipped ones — one store only on change, `Release`, step 0 of the fold — but
neither of its two mechanisms survived:

```rust
// rung 11, ecs_control::project — the whole of A8
let mut bits = 0u64;
for scope in world.query::<&ProfilingScope, Enabled<ProfilingScopeEnabled>>().iter() {
    bits |= scope.arm_bit();
}
profiling_abi::project_scopes(bits)      // writes bits 8..64; fetch_update, no store if unchanged
```

* **No `scope_entity[]`.** The loop above is over the *search space* and needs a bit → entity table
  kept in step with the world. The query is over the *answer* — it yields exactly the enabled
  scopes — and the association it needs is the component itself. A `[Entity; 64]` beside it would be
  the mirror this decision forbids two paragraphs below.
* **`ARM_MASK.store(bits)` would clear the channel half**, including the bit `arm` holds. See the
  boxed note at the top of this decision: that store is a one-way switch. `project_scopes` writes
  bits `8..64` through a `fetch_update` and returns whether anything changed.

**The write path a game system actually uses, with its cost (B2).** `EcsMaster::enable`/`disable`
take **`&mut self`** (`enable_tag_api.rs:87`, `:95`), which no parallel system can hold, and rev 3
named no alternative — so its "only switch" had no caller. The tree already supplies one:

| Caller | Verb | When it lands | Cost |
|---|---|---|---|
| any **parallel** system (console command, dev menu, network handler, save-file loader) | `commands.entity(e).enable::<ProfilingScopeEnabled>()` / `.disable::<…>()` (`system/params/entity_commands.rs:220`, `:236`) | at that system's `apply`, **inside the same schedule run** (`schedule.rs:722-726` / `:1130-1133`) | one POD `EnableTagCommand` (`Entity` + `EnableTagId` + `bool`) in the system's own queue; no allocation, no new sync primitive, **no exclusive system and no schedule serialisation point** |
| the host, or an exclusive system that already holds the world | `world.enable::<ProfilingScopeEnabled>(e)` | immediately | one bit RMW |

Because the deferred command applies *within* the frame and the projection runs at the top of the
**next** frame's `update_with_delta`, **G12's "the next frame" assertion is exactly right for both
paths** — the latency did not change, only the caller did. What rev 3 lost by having no path at
all is what G12 now proves it has.

- **The ECS remains the single source of truth.** There is no parallel mechanism, **no public mask
  setter**, no mirror and no dirty flag.
- **The cost is measured, not assumed.** `is_enabled::<T>` is documented at
  `enable_tag_api.rs:100-105` as O(1), *"≤ 5 ns"*. At 16 registered scopes that is ≤ 80 ns per
  frame; at the 64-scope maximum, ≤ 320 ns. It runs **inside `__fold`**, i.e. inside
  `instrument_measured` and outside `__frame` (D16), so it is disclosed rather than hidden. A
  `scope_scan` bench leg reports it.
- **No new system, no new `SystemSet` for control, no reconciliation system in the schedule.** The
  extension's `ProfilerSet::Control` is rejected for the same reason `requires_dispatcher` on
  retire is (F14): adding scheduled systems to the subsystem that measures scheduling perturbs its
  own product.

**Rejected: a scope hierarchy.** Parent/child scopes make the emission gate more than one `bt`. A
game wanting hierarchy composes bits itself, in its own code, visibly.

---

## D22 — Retention: three RETENTION TIERS, because a fixed ring cannot hold an hour (C-I)

(`retention_tier`, never bare "tier" — `ZoneTier` is the other one, and S11 forbids the collision.)

| `retention_tier` | Structure | Horizon | Cost | Always on? |
|---|---|---|---|---|
| **A** | the frame-major ring | `WINDOW = 121` frames ≈ 2 s | **2.48 MiB** at `Z = 1024` (21 B/zone/frame — M9) | yes when armed |
| **B** | lifetime accumulators — `{ total: u64, count: u64, max: u32, min: u32 }` per zone | the whole session | `24 × Z` = 24 KiB at `Z = 1024` | yes when armed |
| **C** | log-linear histograms — 3 mantissa bits, 192 buckets of `u16`, 400 B per slot | the whole session | `cfg.hist_slots × 400 B`; 25 KiB at 64 slots | opt-in; **implied for every zone in the telemetry quantile subscription** (D23) |

**Every figure in the Cost column is an ARMED figure**, committed once at `arm` out of the store's
one `VmReservation` (D8/D15). `arm` is the enable path: with the profiler never armed, no column
is committed, no accumulator is touched and no histogram slot exists. The extents are reserved,
not resident. That is a statement about *when the commit happens*, not a softening of the number
— the number is exactly what a title pays the moment it turns the profiler on.

Retention tier B is folded in **one sequential pass over the current frame's row**, which the fold
just touched, so it is L1-warm. Retention tier C folds only for zones with a slot
(`hist_of[z] != 0`, a `Z`-byte L1-resident map).

**No per-frame history beyond `WINDOW`, ever.** That is C-I's cost to the game side, and it is
deliberate: per-frame retention over an hour is 216 000 rows × 21 B × `Z`, which is not a ring, it
is a database.

**Bucket geometry is chosen against the measured floor band (4.7-14.3 %), not against a general
requirement.** 3 mantissa bits ⇒ 6.25 % bucket width, the same order as the floor — which is
exactly why **`resolve` does not consume histograms**. If a title needs tighter session quantiles
the mantissa widens to 4 bits (384 buckets, 784 B/slot): a config, not a redesign.

Saturation is **counted** (`hist_saturations`), never silent: a `u16` bucket saturates at 65 535
samples, which for a per-frame zone is ~18 minutes in one bucket.

---

## D23 — Player telemetry: an append-only binary stream, window-granular, synchronous, no new thread

**Every record is inside a self-delimiting BLOCK, because an unframed stream cannot survive a real
disk (M8).**

```
file        := header, block*
header  (128 B, once per file): magic, schema_version, boyko_diag::SessionId, run_id, build_hash,
                                player_tag[16] (opaque; the engine never interprets it),
                                build_profile, zone_tier, zone_stride, window,
                                ticks_per_ns, calib_cv, clock_epoch
block   (16 B header + payload, ONE per window, ONE write_all):
                                magic: u32, len: u32, seq: u32, crc32: u32
payload := ZoneRow* WindowRec*
ZoneRow   (variable, once per zone per file): id, kind, unit, scope, name bytes
WindowRec (40 B, per subscribed zone per window): id, count, total, min, max,
                                median, p95, drops, clock_epoch, fixed_elapsed_ns
```

Rev 3 had no framing at all: `ZoneRow` is explicitly variable-length, nothing carried a length, a
magic or a checksum, and `write_all` on `ENOSPC` returns **after a partial write** — so the file
ends mid-record and a decoder cannot distinguish a torn tail from data. The round-trip property
test ("decode then re-encode is byte-identical") would fail on any real disk-full file, and G15
explicitly disclaimed the one failure a player's full disk actually produces.

**Decoder behaviour, specified rather than implied.** The decoder walks blocks; a block whose
`magic` is wrong, whose `len` exceeds the bytes remaining, or whose `crc32` mismatches
**terminates the walk**, and its records are not returned. The decoder reports `blocks_ok`,
`records_ok` and `truncated_tail_bytes`. The round-trip property becomes: re-encoding the decoded
blocks is byte-identical to the input **minus `truncated_tail_bytes`** — a property that holds on
a torn file instead of failing on it. Framing costs 16 B per 2 s window; per-record framing was
rejected at 8 B on a 40 B record (20 % overhead) when there is exactly one `write_all` per window
and therefore exactly one possible tear point.

One `write_all` per window (2 s), from a **`.bss` process-static double buffer in
`boyko_app::profiling::stream`** (not the `Profiler` `Resource` — the no-`World` rule for
`flush_on_panic`, `seam/lifecycle-order`), on the dispatcher, `#[cold]`, **inside
`instrument_measured`** (D16), so it is disclosed. Shipping volume ≈ 2.9 MB/h. Rotation at
`max_bytes`. A write error sets `telemetry = None`, counts `telemetry_write_errors` and emits
`W9215` once — **never panics, never retries in-frame**.

**The telemetry file is opened on the ENABLE path, never at process start.** `W9214` (telemetry
path unwritable) is raised where the path is first opened, which is inside `arm` with a
`TelemetryConfig` present — so a run that never arms the profiler opens no file, touches neither
double-buffer page and emits no `W9214`. The double buffer itself is `.bss`: declared, zero, and
untouched until the first window is written.

**The window REDUCTION is budgeted and benched separately, and it is the dominant term (M7).**
Rev 3 costed telemetry as "20-60 µs per 2 s" and benched `stream_encode` = *"400 `WindowRec`s +
the `write_all`"* — but `WindowRec` carries `median` and `p95`, which A4 obtains by a strided
gather over the frame-major columns **plus a sort of 121 values, per zone**. At a few hundred
subscribed zones that is hundreds of gathers over a 2.48 MiB working set plus hundreds of sorts,
plausibly 0.5-2 ms, synchronous, in-frame — an order of magnitude above the quoted number, and
X25's "refused on the number" rested on a number that omitted it. Two changes:

1. **`count` / `total` / `min` / `max` are O(1) folds and are carried for every subscribed zone.**
   `median` / `p95` require the sort and are carried **only for zones in
   `TelemetryConfig::quantiles`, capped at `MAX_TELEMETRY_QUANTILE_ZONES = 64`**; beyond the cap a
   subscription is refused, counted (`telemetry_zones_refused`) and reported once (`W9218`). A
   `WindowRec` outside the subscription writes `NO_QUANTILE` in both fields — an explicit format
   value, not a zero a reader could mistake for a measurement.
2. **The reduction is its own zone (`__telemetry_reduce`) and its own bench leg.** The budget is
   `__telemetry_reduce` p95 ≤ 150 µs at 64 quantile zones, and the **total**
   (`reduce + encode + write`) p95 ≤ 350 µs per window, in the budget table.

**No second thread (X25), still refused — but on the corrected total.** 350 µs once per 121 frames
is **2.1 % of one frame**, not 0.36 %; as a fraction of the 2.02 s window it is 0.017 %. It is a
*spike*, and it is stated as one rather than amortised into a per-frame average — but it is a
spike **below this box's own decidability floor** (4.7-14.3 %), i.e. one the project's instruments
cannot resolve. The engine's only threads stay the pool's. Named escalation trigger, restated
against the total: **`__telemetry_total` p95 > 500 µs** on a real title, measured ⇒ hand the byte
blob to `boyko_log`'s existing sink — **one thread for both subsystems, never two.**

**Loss bound: ≤ one window on a hard kill**, because there is no cross-window buffering. G15 proves
that bound, now also covers a short/failing write, and states what it cannot cover.

**`fixed_elapsed_ns` = `FixedTime::elapsed()`**
(`crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:162`, *"Exact sum of expended timesteps (the
determinism witness)"* — verified) is the kernel's own determinism witness, so a stream correlates
with a replay at 8 B per record (X18).

---

## D25 — The game reads its own numbers from ECS systems — windowed, lagged, and NOT a message bus

`Res<Profiler>` is readable from any system. Two things make that safe and cheap:

- **`ProfiledZone(ZoneId)`** — a component resolving a name to an id **once at setup**, so a reader
  never calls the `#[cold]` `by_name`.
- **A published latency table** (`Profiler::latency()`, and an artifact field — not a printed
  line, S1), because the lag is structural, not incidental:

| Datum | Freshest available | Why | Published by `latency()`? |
|---|---|---|---|
| CPU spans, counters, gauges | frame **N−1** | the fold folds closed frames only (A2's live-frame cut) | **yes**, rung 11 |
| GPU spans | frame **N−4 … N−2** | availability polling + `GPU_RING_DEPTH` + `RETIRE_GRACE_FRAMES` (D4) | **no — not this store's to publish** |
| lifetime / histogram | through N−1 | folded at the same fold | not yet — rung 12 builds the accumulators |

**Why the GPU row is absent rather than zero.** MEASURED at rung 11: `boyko_diag::sample::push` has
**zero callers outside `boyko_ecs`** (checked across `boyko_app`, `boyko_render` and
`boyko_rhi_vulkan`), and `Profiler::arm` has **no non-test caller**. The host's GPU channel folds
into the artifact reducer, not into this `Profiler`. A GPU row on this table would therefore describe
a lag this store's data cannot have — computed from `GPU_RING_DEPTH` and `RETIRE_GRACE_FRAMES`, two
constants living in a crate `boyko_ecs` does not depend on and must not. A field that is structurally
always the same value is indistinguishable from a measurement of that value, which is the rule this
module group has applied since rung 2.

**Ordering.** The retire step and the fold both run **outside the schedule** (D16, D4a), before any
system executes, so every `Res<Profiler>` reader in the frame sees the same consistent snapshot
with no intra-frame ordering edge and **no new `SystemSet`**. This is strictly better than the
extension's `ProfilerSet::{Retire, Read}` pair, which would have needed a scheduling edge and a
dispatcher-pinned system.

**Refused (X14, half):** *same-frame* counter readback as an inter-system message bus. It costs
either a shared-line RMW on the emission path or a mid-frame fold, and the ECS already has events
and resources for that. A game's own counters are ECS data the profiler **samples** (via `gauge!`
once per frame); they are not data the profiler **stores** on the game's behalf.

**Supported (X14, half):** reading *windowed statistics* to drive LOD, dynamic resolution or
quality scaling. That is a one-frame-stale median, which is what those controllers want anyway.

**Reference overlay** (`boyko_ui/src/profiling_overlay.rs`, rung 15) — allocation-free, gated by
G19 with a positive control.

---

## D26 — Session identity in; cross-process aggregation and a live viewer out

**In:** `boyko_diag::SessionId` (one 128-bit id minted once and shared with the logger's artifact
header — S11, so the two files join), `run_id`, `build_hash`, an opaque 16-byte `player_tag` the
engine never interprets, and replay correlation via `FixedTime::elapsed()`. 44 B of header, 8 B
per record.

**Out, argued:**

- *Cross-process / networked aggregation* — needs cross-machine clock correlation, which D14
  refuses to fake on **one** machine. Re-entry condition: the merge is a tool over files that
  already share `boyko_diag::SessionId` + `fixed_elapsed_ns`; build it when two files exist that
  anyone wants merged.
- *Live network viewer / remote streaming* — the Tracy protocol renamed (D10), plus a socket in
  the frame loop. A tailed file answers the same question at zero engine cost.
- *Remote arm/disarm* — **already served**: a network handler calls
  `commands.entity(e).enable::<ProfilingScopeEnabled>()` like any other parallel system (D20's
  write-path table). The engine supplies the switch; the game supplies the wire.

---

## D27 — A game's handles live in ECS storage; the profiler does not store them

`DynZoneHandle` is **16 B, `Copy`, `Send + Sync`, with no thread affinity**, so a game stores it in
a component or a `Resource`-owned column and emits from any lane. The profiler keeps the
*descriptor*; the game keeps the *handle*. This is Principle 0 applied to the game's side of the
seam: the durable per-entity association is ECS data, in ECS storage, owned by the game.

`Arena` / `ComponentPool` / `UnitId` remain **untouched by the profiler itself**, deliberately: it
stores no per-entity data, so two-level addressing is not involved, and routing transport through
`ComponentPool` would put a growth path on the emission side.

---

## D28 — What the extension asked for and this design refuses

| Asked | Refused | Reason |
|---|---|---|
| Toggle projected by an `IsEnabled` **observer** | yes | The kernel fires no observer on an enable-bit toggle (`enable_tag_api.rs:77-88`). Replaced by the fold-step projection (D20), which is cheaper than a system and honest about its ≤ 320 ns |
| A `ProfilerSet::{Retire, Read}` pair with an ordering edge | yes | The retire and fold run outside the schedule entirely (D16/D4a), so no edge is needed. Adding systems to the subsystem that measures scheduling perturbs its own product (F14) |
| 1-in-N sampling **at the call site** | yes | A per-site RMW on a shared line. Decimation happens at retention and via the scope bit. A game wanting 1-in-N writes it in its own code, visibly (X16) |
| Same-frame counter readback as a message bus | yes | X14 above |
| A second sink thread for telemetry | yes | 0.36 % of one frame in 120 does not justify a thread (X25) — and the number is **corrected to 2.1 %** by M7; the refusal survives on the corrected total, which is still below this box's decidability floor |
| A second `ARM_MASK` word (128 scopes) | deferred | 32 game bits are not yet full; a second word costs the hot path. Refuse until a title exhausts 32 |
| A live network viewer | yes | D26 |

---

## Data structures — the game-facing block

```rust
// ══════════════ boyko_diag::profiling_abi — the game's emission surface ══════════════

/// 16 B, Copy, Send + Sync, NO thread affinity (D27). The game stores it; the profiler
/// does not. `arm_bit` is carried so `zone_dyn!` needs no REGISTRY dereference to
/// recover the scope bit — the defect G17's RED injects.
#[repr(C)] pub struct DynZoneHandle { id: ZoneId, arm_bit: u64 }

pub struct ZoneSpec<'a> { pub name: &'a str, pub channel: Channel, pub kind: ZoneKind,
                          pub unit: Unit, pub scope: u8 }

// A7's refusal codomain, one variant per refusal the algorithm can produce:
//   EngineScopeRefused  (spec.scope < 32           -> W9212)
//   BudgetExhausted     (id >= armed_user_budget   -> W9210, counter restored)
//   NameArenaExhausted  (DYN_NAME_BYTES exhausted  -> W9210, counter restored)
pub enum RegisterError { EngineScopeRefused, BudgetExhausted, NameArenaExhausted }

// ══════════════ boyko_ecs::…::profiling::ecs_control — the runtime switch ═══════════

/// CAPABILITY — the runtime on/off bit. FIELDLESS, by the macro's requirement (B2).
#[derive(Component)] #[component(storage = "bitset")] pub struct ProfilingScopeEnabled;

/// DATA — an ordinary table-storage component on the same entity.
#[derive(Component)] pub struct ProfilingScope { pub bit: u8, pub name: &'static str }

/// A name resolved to an id ONCE at setup, so a reader never calls the #[cold] by_name.
#[derive(Component)] pub struct ProfiledZone(pub ZoneId);

// ══════════════ retention tiers B and C (D22) ═══════════════════════════════════════

/// Retention tier B — 24 B per zone, `24 × Z` = 24 KiB at Z = 1024. Always on when armed.
/// `min` on a zone that never received a sample stays u32::MAX and is REPORTED AS
/// "no samples", never as a value (rung-10..13 unit test).
#[repr(C)] pub struct LifetimeAcc { total: u64, count: u64, max: u32, min: u32 }

/// Retention tier C — log-linear, 3 mantissa bits => 6.25 % bucket width, 192 buckets
/// of u16 (384 B) + total + count = 400 B per slot. Opt-in; implied for every zone in
/// the telemetry quantile subscription. Saturation is COUNTED (hist_saturations),
/// never silent: a u16 bucket saturates at 65 535 samples ~ 18 minutes for a per-frame
/// zone. Widening to 4 mantissa bits (384 buckets, 784 B/slot) is a config, not a
/// redesign. Quantiles are returned as bucket EDGES, never as point estimates.
#[repr(C)] pub struct HistSlot { buckets: [u16; 192], total: u64, count: u64 }

// ══════════════ telemetry (D23) — every width pinned here ═══════════════════════════
//
// header    128 B, once per file
// block      16 B header { magic: u32, len: u32, seq: u32, crc32: u32 } + payload,
//            ONE per window, ONE write_all — hence exactly ONE possible tear point
// ZoneRow   VARIABLE, once per zone per file: id, kind, unit, scope, name bytes
// WindowRec  40 B, per subscribed zone per window
//
pub const MAX_TELEMETRY_QUANTILE_ZONES: usize = 64;   // the 65th is refused -> W9218 (M7)
pub const NO_QUANTILE: u64 = /* an explicit format value, NOT a zero a reader could
                                mistake for a measurement */;
```

**Types named in the API but specified by prose rather than by a literal in rev 4** — recorded so
a reader does not conclude they were dropped: `ScopeError` (`register_scope`'s refusal),
`HistView<'_>` (tier C's read view, whose quantiles are **edges**), `LatencyTable`
(`Profiler::latency()`'s published D25 table) and `TelemetryConfig` (whose `.quantiles: &[ZoneId]`
is capped at 64). Each is fully determined by the decision that owns it; none is invented here.

---

## Public API — the game slice

```rust
// ── dynamic emission (data-defined zones; USER partition, always) ──
pub fn register_zone(spec: ZoneSpec<'_>) -> Result<DynZoneHandle, RegisterError>;   // #[cold]
zone_dyn!(handle);  counter_dyn!(handle, v);  gauge_dyn!(handle, v);
pub fn zone_dyn_open(h: DynZoneHandle) -> u64;      // FFI/script seam: an opaque token
pub fn zone_dyn_close(h: DynZoneHandle, token: u64);

// ── scopes (the ONLY runtime switch; no public mask setter) ──
// SHIPPED at rung 11 returning the COMPONENT, not the bare bit: with a `u8` the caller must write
// `ProfilingScope { bit, name }` itself, naming the scope a second time, and nothing checks the two
// names agree. A component saying "ai" whose bit was minted for "audio" is a mislabelled
// measurement no gate downstream can detect. `scope.bit` is still there for anyone who wants it.
pub fn register_scope(name: &'static str) -> Result<ProfilingScope, ScopeError>;  // #[cold], 32..63
//     commands.entity(e).enable::<ProfilingScopeEnabled>()   // entity_commands.rs:220
//     commands.entity(e).disable::<ProfilingScopeEnabled>()  // entity_commands.rs:236
//   …or, where `&mut EcsMaster` is already held (host / exclusive system):
//     world.enable::<ProfilingScopeEnabled>(e) / world.disable::<ProfilingScopeEnabled>(e)
// Both take effect at the NEXT frame's fold-step projection (D20, G12).

// ── the game's read surface ──
impl Profiler {
    pub fn lifetime(&self, id: ZoneId)  -> Option<LifetimeAcc>;    // retention tier B (D22)
    pub fn histogram(&self, id: ZoneId) -> Option<HistView<'_>>;   // tier C; quantiles as EDGES
    pub fn latency(&self)               -> LatencyTable;           // the published table (D25)
}

// ── the panic seam ──
pub fn flush_on_panic();   // #[cold]; called BY the logging plan's single hook, never installed here
```

**Deliberately absent, from the whole API and therefore from this half** (the list is carried in
full by `profiling/emission-abi` because it constrains all three surfaces; the two clauses this
half is measured against are named here): **any accessor that panics on the wrong `ZoneKind`** —
`counter(id)` on a `Span` zone returns `None` — and **any public `ARM_MASK` setter**. There is
also no point-estimate quantile from a histogram: `HistView` yields edges.

---

## Algorithms

### A8 — Scope projection

Not an observer (there is none — `enable_tag_api.rs:77-88`) and not a system. It is **step 0 of
A2**. ~~≤ 5 ns × `scope_count`~~, one `Release` store only when the value changes, `#[cold]`-free
because it is already off the hot path. `ARM_MASK` toggling has no other public writer.

**SHIPPED at rung 11, and the cost claim changed shape with the mechanism.** The projection is one
cached-query lookup (`EcsMaster::query`'s own documented `~5 ns` warm) plus one archetype walk over
the scope entities — not `scope_count` separate `is_enabled` calls, because there is no
`scope_entity[]` table to index. The **first** call per world pays `query_cold_init`'s one-time
~1 µs, on the first armed frame; a process that never arms never folds and never reaches it. It runs
inside `__fold`, i.e. inside `instrument_measured` and outside `__frame` (D16), so it is disclosed.

"No other public writer" is now exact rather than aspirational: `project_scopes` can only write bits
`8..64`, so the channel half is unreachable through it, and the scope half it writes comes from the
ECS by construction — its one caller reads the enable bits and hands the result straight in.

### A10 — Telemetry window: `__telemetry_reduce` then `__telemetry_write` (dispatcher, `#[cold]`)

```
__telemetry_reduce:                        // THE DOMINANT TERM (M7) — its own zone, its own budget
 0. for each subscribed zone: count/total/min/max  <- O(1) folds already in the row
 1. for each zone in cfg.telemetry.quantiles (<= MAX_TELEMETRY_QUANTILE_ZONES = 64):
        gather 121 strided values; sort; median; p95      // A4; one sort per zone
    every other subscribed zone writes NO_QUANTILE in both fields

__telemetry_write:
 2. buf = encode[cur]                      // .bss double buffer in boyko_app::profiling::stream
                                           //   (NOT the Profiler Resource — S5/lifecycle-order)
 3. if first window in this file: write header (128 B)
 4. open a BLOCK: reserve {magic, len, seq, crc32}                          (M8 framing)
 5. for each zone whose id has not yet appeared in this file: append ZoneRow
 6. for each subscribed zone: append WindowRec (40 B)
    fixed_elapsed_ns = FixedTime::elapsed()  (time/fixed_time.rs:162) — the determinism witness
 7. close the block: len = bytes since the header; crc32 over the payload
 8. file.write_all(&buf[..n])              // ONE syscall; NO cross-window buffering
                                           //   a partial write on ENOSPC tears exactly ONE block,
                                           //   which the decoder detects and discards (M8)
 9. bytes_written += n; if bytes_written > max_bytes { rotate }
10. cur ^= 1
```

O(subscribed zones) for the encode, **O(quantile zones × 121 log 121) for the reduce**. One
`write_all` of ~1.6 KB (shipping) or ~16 KB (dev) per 2 s. Budgets: `__telemetry_reduce` p95
≤ 150 µs at 64 quantile zones, `__telemetry_write` p95 ≤ 200 µs, **total ≤ 350 µs**, all three
**inside `instrument_measured`** (D16) and all three benched (`telemetry_window`). Errors set
`telemetry = None`, count `telemetry_write_errors`, emit `W9215` once — never panic, never retry
in-frame.
