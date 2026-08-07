# Profiling — the GPU half and the RHI zone seam

<!-- CONTRACT
provides: profiling/gpu-zone-seam
assumes:  profiling/budgets-and-invariants
assumes:  profiling/emission-abi
assumes:  seam/lifecycle-order
assumes:  seam/vocabulary
-->

**Carved from** `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §D4, §D4a, §D5, §D12, §D14, §D17, §D18,
the GPU block of §Data structures, the GPU slice of §Public API, and §Algorithms A3. Diff against
that file until it is retired.

**What this file owns.** Everything between a `vkCmdWriteTimestamp` and a number that a window
reducer may read: how results are collected without blocking, how a slot is proved to have retired,
how a pass is labelled `MEASURED` / `NOT_BRACKETED` / `TORN` / `LOST`, how record order is
witnessed host-side, how the disarmed byte-identity claim is actually proved, and the three RHI
verbs the seam gains.

**What this file does NOT own, and must not restate.** The hard environmental constraints —
`VK_PRESENT_MODE_FIFO_KHR` unconditional, `VK_QUERY_RESULT_WITH_AVAILABILITY_BIT` undeclared,
`hostQueryReset` never enabled and `vkResetQueryPool` never loaded, `debug_assert!` inheriting the
driver's release profile, and "a hang is not a showable red in this repo" — belong to
`profiling/budgets-and-invariants`. They are *cited here as the reason a decision has the shape it
has*; the statements themselves live once, there.

---

## D4 — GPU readback is availability-polled and N-frames deferred; `WAIT_BIT` is made UNREPRESENTABLE

```rust
/// The verb takes NO flags parameter. The Vulkan implementation's flag word is a
/// private `const` const-asserted to exclude WAIT_BIT, so a blocking read is a
/// COMPILE error, not a grep result (G2a).
fn read_query_pool_pairs_available(
    &self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64],                 // len >= 4 * pair_count (value + availability per query)
    out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8],            // one byte per pair: 1 iff BOTH queries available
) -> Result<(), Self::Error>;
```

```rust
const GPU_ZONE_QUERY_FLAGS: u32 =
    VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WITH_AVAILABILITY_BIT;   // 0x1 | 0x4
const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0);
```

`VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004` (rev 1 said `0x20`, which is undefined;
`0x10` is `WITH_STATUS_BIT_KHR`). `VK_NOT_READY` maps to `Ok(())` with the corresponding
availability bits **clear** — a normal outcome, not an error. The availability output is a **byte
slice, not a `u128`** — no fixed-width wall.

**Why the const-assert and not a source gate (F3).** Rev 2's G2a grepped `gpu_zone.rs` and
`profiling/**` for `WAIT_BIT`. The verb's *body* must live in
`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs`, beside its siblings `fetch_query_raw_ticks`,
`fetch_query_pair_ticks` (`:1249`) and `fetch_query_pair_stamps` (`:1288`) — **a file the gate's
scope structurally excludes**. That is the `-ValidationOn` failure shape the plan itself cites: a
mechanical check whose scope excludes the defect. And the behavioural red is unavailable, because a
blocking read **hangs**, and a hang is not a showable red in this repo
(`crates/boyko_app/tests/vb_bench_totality_gate.rs:44-53` — *"this repository has no
kill-after-timeout pattern to borrow"*). Making the flag word a checked `const` converts the red
into a build failure. The source gate is *kept as well*, but re-scoped: it asserts that **the set of
files naming `vkGetQueryPoolResults` equals a pinned list**, so a new blocking reader in a new file
fails the gate by existing.

`GPU_RING_DEPTH = 4 > FRAMES_IN_FLIGHT = 2`. A frame slot retires when every bracketed pair is
available, or on the deadline in D4a.

**Why.** Tracy polls with the availability bit and breaks at the first unavailable query; Bevy
resolves into a readback buffer and picks it up via `map_async` + `AtomicBool(Release)`. Neither
blocks. The two hang classes documented at `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:186-203`
and `:575-584` exist *only* because the reader blocks — both blocks say the same thing in the same
words, that a `VK_QUERY_RESULT_WAIT_BIT` reader *"BLOCKS FOREVER on any pair its recorder never
wrote this frame"*, and that this is why a second and then a third collector had to exist at all.
Removing the block closes both structurally, and with them the reason the three collectors are
separate.

**Rejected.** Keeping `WAIT_BIT` + widening the totality epilogue — a device-side patch for a
host-side mistake: it records two extra timestamps per unbracketed pass **into the stream being
measured**, and makes termination depend on recorder discipline forever.
`VK_QUERY_RESULT_PARTIAL_BIT` is also rejected: the spec makes an unavailable result *undefined*,
not zero.

**Trade-off.** Results arrive 2-4 frames late; a frame is `Pending` until its slot retires. Live
display of the *current* frame's GPU cost is impossible — this is the mechanical source of C-IV's
latency table (`profiling/game-facing-surface`).

### D4a — `fence_seen` is derived from `RenderEpoch`; the deadline is TWO-horned, and the second horn counts FRAMES, not submits (F13, F28, M13)

Rev 2's `FrameSlot.fence_seen: AtomicBool` had **no source**: it named
`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:265` as "the fence gate … which is signalled",
but that line is `pub fn submission_epoch(&self) -> u64`, a *submit* counter, and the tree has no
non-blocking fence-status verb at all. Every fence operation in the tree is
`wait_for_fences(..., VK_TIMEOUT_INFINITE)` (`frame_driver.rs:319`, `:400`).

The derivation the counter's own doc supplies (`frame_driver.rs:256-263`) is used instead:

```
slot.submit_epoch = RenderEpoch at record time
fence_seen(slot)  = RenderEpoch >= slot.submit_epoch + FRAMES_IN_FLIGHT
```

The doc's words, verbatim: *"a resource enqueued at epoch `N` is safe to free once the host has
observed epoch `>= N + FRAMES_IN_FLIGHT` (its last possible submit, `N`, is then guaranteed
GPU-complete by the ring's own fence discipline)."* It also pins the one property the derivation
needs — the counter *"advances exactly once per successful `vkQueueSubmit` … and only there: a
pre-acquire out-of-date recreate returns before either counter moves."*

`RenderEpoch` is already an ECS `Resource` (`crates/boyko_render/src/asset_refcount.rs:55` —
`pub struct RenderEpoch(pub u64);`) written by the host every frame at
`crates/boyko_app/src/runner.rs:1320`, one line above the ECS frame call at `:1321`. The retire step
reads it. **No new RHI verb, no fence poll, no block.** The retire seam is the same one the asset
system already trusts for freeing GPU-referenced memory.

**Termination when frames STOP** (shutdown, device-lost — F28): `Profiler::flush_gpu(&mut world,
&recorder)` on the runner's teardown path force-retires every in-flight slot as `Partial`, labelling
unavailable pairs `LOST` and counting `gpu_slots_abandoned`. The count is **release-live** and
reported; rev 2 lost up to `GPU_RING_DEPTH` slots silently. `flush_gpu` runs **ahead of the log
flush** in the teardown order, which is `seam/lifecycle-order`'s to state and is the whole of that
hole's fix.

**Termination when SUBMITS stop while frames CONTINUE — the common case on this host loop (M13).**
F28 addressed "frames stop"; the loop's actual behaviour is the other way round.
`runner.rs:1328-1332` `continue`s on a 0×0 client **after** `update_with_delta` (`:1321`) and
**before** `wait_frame_in_flight` (`:1337`), record and submit, so a minimised window keeps folding,
keeps serving `Res<Profiler>` readers and keeps writing telemetry while `submission_epoch()` — hence
`RenderEpoch` — is frozen. An epoch-only deadline can never fire there, and teardown is never
reached because the process is alive. Two changes:

1. **`retire_gpu` is called at `runner.rs:1320`, immediately after the `RenderEpoch` publication and
   BEFORE the 0×0 `continue`** — so it runs on every iteration of the host loop, minimised or not.
   (Rev 3's A3 said both "between `wait_frame_in_flight()` and the record" and "on the line that
   publishes `RenderEpoch`"; those are on opposite sides of the `continue` and only the second one
   runs while minimised. The contradiction is resolved in favour of the second.)
2. **A second, frame-counted horn.** `FrameSlot.record_frame: u64` records the ECS frame counter at
   record time; a slot retires `Partial` once `frame_now - slot.record_frame > GPU_FRAME_DEADLINE`
   (`= GPU_RING_DEPTH + RETIRE_GRACE_FRAMES + 2 = 8`) **regardless of the epoch**, counting
   `gpu_frame_deadline`. The two horns are independent: the epoch horn is the tight one in normal
   running; the frame horn is the one that fires when submits freeze.

**And the grace decrement is corrected.** Rev 3's A3 read
`… else if epoch_ok && slot.grace == 0 { retire } else { slot.grace -= 1 }`, so a slot whose epoch
condition was **false** with `grace` already 0 executed `0u8 - 1`: a debug panic, or in release a
wrap to 255 that silently restarts the deadline for another 255 frames. The decrement now lives
**inside** the epoch arm and is guarded (`if slot.grace > 0 { slot.grace -= 1 } else { retire }`),
which is A3's corrected form below.

---

## D5 — The witness survives as a per-pair mark array with a single seal; `__gpu_null` is DELETED

`AtomicU128` does not exist (not stable, not nightly; zero occurrences in the tree), and a
hand-rolled 128-bit atomic is `cmpxchg16b`, a full RMW — **not** the cheap `Release` store the
ordering argument assumes. Representation instead:

```rust
struct FrameSlot {
    marks: UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 = begun, bit1 = ended; single producer
    seal:  AtomicU32,                        // the ONE release edge: stores `frame` after marks
    ...
}
```

The recorder writes marks (plain stores; exactly one thread per slot), then
`seal.store(frame, Release)`. Retire does `seal.load(Acquire)`; if it equals the expected frame, the
marks are visible. This scales to any pair count with no bitmask width wall and costs a plain byte
store instead of an atomic OR.

Label is the 2×2 over (witness, availability):

| begun | ended | available at deadline | label |
|---|---|---|---|
| 1 | 1 | yes | `MEASURED` |
| 0 | 0 | – | `NOT_BRACKETED` (this leg does not run that pass) |
| 1 | 0 | – | `TORN` (recorder bug) |
| 1 | 1 | no | `LOST` → **NOT RESOLVED, no number printed** |

**Why the witness is still needed:** availability answers *"the GPU wrote this query"*, not *"the
recorder bracketed this pass"*. A pass that never ran and a pass whose queries were never reported
are both `available == 0` and mean opposite things. The tree's own argument at
`gpu_timing.rs:432-445` is unchanged and is quoted because it is the load-bearing half: *"a
`write_zero_pair` fallback reads ~0 like a genuinely free pass, and its begin offset is only
*usually* the frame's largest — a `TOP_OF_PIPE` stamp recorded last may legally report an EARLY time
…, so an offset-position rule is a heuristic, not a proof."* A duration cannot distinguish a free
pass from a filled one, and a begin-offset rule is a heuristic under mixed TOP/BOTTOM stages.

**`__gpu_null` is deleted (F6).** Rev 2 promoted `write_zero_pair`'s mechanism — two back-to-back
`BOTTOM` stamps — to "the quantum probe". **Measured on this box, two adjacent `BOTTOM_OF_PIPE`
stamps with no command between them report the same value: the probe reads 0, every time.** It is
the same defect VG R3 P4-6 found in its own first design (*"a strict order where two adjacent BOTTOM
stamps cannot provide one"*). A probe that is measured-inert is not a probe; keeping it would make
D11's `BELOW QUANTUM` guard protect nothing on the GPU channel and reduce `resolve`'s
`max(floor, quantum)` to `floor`. The GPU quantum is obtained the way the tree obtains it — D11a,
in `profiling/statistics-discipline`.

`LOST` remains a state the old design could not express — it hung instead. **`LOST` is counted at
the site and reported once per window with its count; it does not print per pair** (F20: rev 2's
`emit_diag(W9205)` per `LOST` put up to 128 `eprintln!`s, each locking stderr and formatting, inside
a per-frame path — the exact rule the plan applies to lane overflow and then abandoned here).

---

## D12 — Present mode becomes configurable; wall clock is demoted to a labelled, probed observation

`PresentModeConfig { Fifo, Immediate }` (`Mailbox` is declared and returns `Unsupported` until a
harness needs it — one code path, not three). Default `Fifo`, so **no golden pin moves**. Support is
**probed** with the existing `present_mode_supported`
(`crates/boyko_rhi_vulkan/src/present/surface.rs:218`, imported at `present/swapchain.rs:10` and
already used at `present/swapchain.rs:164`) and the *resolved* mode is recorded in the artifact; an
unsupported request falls back to `Fifo` with a loud notice (the `BootError::ValidationUnavailable`
precedent: refuse or announce, never silently degrade).

The `Frame` channel's wall clock always carries its bound: `frame_wall_ns=… bound=FIFO(refresh≈16.67ms)`
or `bound=none`. **Even under `Immediate`, wall clock stays secondary**: the primary CPU number is
the `__frame` span (D16, in `profiling/store-and-fold`), and the primary GPU number is the
device-tick delta.

**Why at all.** While FIFO is unconditional (`present/swapchain.rs:199` hard-codes
`present_mode: VK_PRESENT_MODE_FIFO_KHR` in the create-info) no wall-clock gate can fail for
GPU-side work, and this project treats a gate that cannot fail as a defect — the measured precedent
being `-ValidationOn` reporting *"clean, 0 messages"* for all 22 pins while an illegal
`mip_levels: 12` drew zero.

**Honest scope note, promoted out of the open questions (F-rung-8).** `Immediate` support on this
box is **unproven**. If it is unsupported, rung 8's present-mode work reduces to *labelling* — the
frame channel stays FIFO-bounded and non-decidable, and no new wall-clock gate becomes showable. The
rung table (`profiling/ladder`) says so, not only an open question.

---

## D14 — Clock correlation is two-tier; tier 1 says `UNCORRELATED` rather than guessing

**Tier 1 (v1, mandatory, zero cost).** GPU spans on a device-tick axis anchored at the frame's first
`BOTTOM` stamp; CPU spans on a TSC axis anchored at `update_with_delta` entry. Two lanes, declared
unmeasured offset, artifact field `cpu_gpu_offset = UNCORRELATED`.

**Tier 2 (v1.1, gated on `VK_EXT_calibrated_timestamps`).** 32 probes at arm, acceptance threshold
`min_deviation × 3/2`, recalibration each fold, `max_deviation_ns` published with every correlated
number.

**Why defer.** Every question the audit found being asked is within-domain. The Khronos problem
statement is why it cannot be faked: core Vulkan timestamps *"cannot be compared even across
separate submits within the same run of an application, as power management events can reset the
timer."* An uncalibrated cross-domain offset is not an approximation; it is a fabrication. **Rev 3
adds a second trigger to revisit:** an in-game overlay showing two axes (D25) will make users ask for
one, and that request must be answered with v1.1 or with a refusal — never with an uncalibrated
offset.

**Third clause, new in rev 4 (S4): the CPU↔log-record correlation IS exact, and it is the only
cross-domain correlation v1 offers.** Because `boyko_diag::clock` is one counter, one scale and one
`clock_epoch` for both subsystems, a log record and a CPU zone are on the *same* axis — a reader can
place a log line inside the zone it happened in, exactly, with no offset and no estimate. That is a
genuine v1 capability and it costs nothing beyond the shared crate. It does **not** extend to the
GPU axis, which stays `UNCORRELATED` until v1.1.

---

## D17 — Disarmed byte-identity is proved by a COMMAND CENSUS, not by image hashes, and the armed clause is an EQUALITY

`goldens/PINS.toml:3` pins the SHA-256 **of a dumped BMP** (*"Each pin records the SHA-256 of a
dumped BMP plus the exact pipeline it was blessed under"*). A `vkCmdResetQueryPool` plus two
`vkCmdWriteTimestamp`s change zero pixels, so rev 1's G5 ("record one command on the disarmed path →
pins move") was **false as written**.

**Mechanism:** `CommandWitness` (rev 3's `RecordCensus`, renamed — `seam/vocabulary`: "census" is
`LogCensus`'s word), a `&mut` host parameter threaded into the recorders and incremented **at the
`vkCmd*` call sites** (D13 rule 1), exactly like `VbRecordProbe`
(`crates/boyko_rhi_vulkan/src/present/passes/vb.rs`: the struct and its per-field contract at
`:107-156`, the originate-here argument at `:86-100`, and the increments themselves at the recorder's
`vkCmd*` sites — `:1710`, `:1948`, `:2313`, `:2583`, `:2798`):

```rust
pub struct CommandWitness {
    profiling_cmds: u32, query_resets: u32, timestamps: u32,
    recorded_pairs: u16,
    first_pair_of: [ZoneId; MAX_GPU_PAIRS],   // pair -> zone, in the order pairs were OPENED
    stream_pos: u32,                          // every recorded vkCmd* in the witnessed region
    stamp_positions: [u32; 2 * MAX_GPU_PAIRS],// stream_pos at each timestamp, in record order
}
```

Two-sided gate:

- disarmed frame ⇒ `profiling_cmds == 0` (and every sub-counter 0);
- armed frame ⇒ **`timestamps == 2 × recorded_pairs` and `recorded_pairs == declared_bracket_count`**
  — an *equality against a host-side declared count*.

Rev 2's armed clause was `timestamps >= 2`, which **the instrument's own `__gpu_null` probe
satisfied by itself** — a recorder that dropped every real bracket passed. `__gpu_null` is now
deleted (D5) and the clause is an equality, so the only way to pass is to record exactly the declared
brackets.

**`first_pair_of` is the RECORD-ORDER WITNESS *within* the new vocabulary** (P4-6 fact 2). Timestamps
cannot license a conclusion about record order — two stamps that resolve on the same tick say nothing
about which `vkCmd*` came first. The witness records, host-side at the call site, the order in which
pairs were opened. **Every claim in this system about *record order* reads `first_pair_of`; no claim
about record order reads a timestamp.**

**`stamp_positions` is the CROSS-LEG witness, and it exists because `first_pair_of` cannot be one
(M12).** G10 licenses deleting the old collector by comparing the two legs — but `first_pair_of` is
`[ZoneId; …]` and the old collector has no `ZoneId`, only `VbTimedPass` slots
(`gpu_timing.rs:229` declares the enum; `VB_PASS_COUNT = 10` at `:391`; the hand-maintained
slot→member table is `from_slot` at `:311-329`). Comparing them therefore needs exactly the
`VbTimedPass → ZoneId` table D6 rejects, and a table written by hand *alongside* the ported brackets
makes the equality a tautology — *"it agrees with itself"*, D13 rule 1's named failure.
`stamp_positions` has **no vocabulary**: it is the value of a monotone "commands recorded so far in
this witnessed region" counter at the moment each timestamp is recorded. Both collectors produce it
from the *same* instrumentation, and the licensing clause becomes **`stamp_positions` (and its
length) identical between the two legs** — same number of timestamps, each at the same position in
the recorded stream. Shifting one bracket by a single command changes one entry. **No mapping table
exists, so none can be wrong.**

**The witness's own perturbation, bounded.** `stream_pos` must be incremented at *every* recorded
`vkCmd*` in the witnessed region, not only the profiling ones, so the whole `CommandWitness` sits
behind `feature = "profiling-census"`, **default off**, enabled in the G5/G10 gate binaries only. The
increments are host-side `u32` adds that record no command and change no device state — which is why
a census build records the *same* command stream as a non-census build, and therefore why G5's
disarmed byte-identity claim still speaks about the shipped configuration.

Golden pins remain a *secondary* check on pixels, with the explicit note that they are structurally
incapable of the command claim.

---

## D18 — `hostQueryReset` is an optimisation with a fully specified fallback

Enabling `VkPhysicalDeviceHostQueryResetFeatures` at device creation records **no commands** and
changes no frame; it is a `pNext` bit, and the goldens are unaffected. It is enabled when the
physical device advertises it.

Nothing establishes that this box's driver does — `crates/boyko_rhi_vulkan/src/ffi.rs:2716` shows
`pub host_query_reset: VkBool32` exists as a feature field and it is never enabled, and
`vkResetQueryPool` is not loaded (the constraint is stated once in
`profiling/budgets-and-invariants`). **The design does not depend on it.** Fallback, specified rather
than named: a slot that retires without host reset sets `needs_cmd_reset`; slot recycling refuses
that slot until an armed frame issues `vkCmdResetQueryPool` for it at the frame top — the exact site
the current code already uses, outside any render scope, satisfying
`VUID-vkCmdResetQueryPool-renderpass`. With `GPU_RING_DEPTH = 4` and `FRAMES_IN_FLIGHT = 2` there is
always a clean slot, so **the fallback never stalls**. Host reset merely removes the one-frame
recycle latency.

---

## Data structures — the GPU block

```rust
// ══════════════ boyko_rhi_vulkan::present::gpu_zone ══════════════

pub struct GpuZoneRecorder {
    pools: [VulkanQueryPool; GPU_RING_DEPTH],
    slots: [FrameSlot; GPU_RING_DEPTH],
    next:  u32,
}
#[repr(C)]
struct FrameSlot {
    marks:   UnsafeCell<[u8; MAX_GPU_PAIRS]>,  // bit0 begun, bit1 ended — single producer (D5)
    zone_of: [ZoneId; MAX_GPU_PAIRS],          // pair -> ZoneId (boyko_diag::profiling_abi)
    seal:    AtomicU32,                        // THE release edge; == frame when marks are valid
    frame:   u32,
    submit_epoch: u64,                         // RenderEpoch at record time (D4a) — replaces
                                               // rev 2's sourceless `fence_seen: AtomicBool`
    record_frame: u64,                         // ECS frame counter at record time — HORN 2 (M13b)
    used_pairs: u16,                           // bump allocator
    grace: u8,
    needs_cmd_reset: bool,                     // set when host reset is unavailable (D18)
}
pub const MAX_GPU_PAIRS: usize = 128;   // 256 queries — Bevy's QuerySet size
const _: () = assert!(MAX_GPU_PAIRS * 2 <= QUERY_POOL_WIDTH);
pub const GPU_RING_DEPTH: usize = 4;
pub const RETIRE_GRACE_FRAMES: u8 = 2;
pub const GPU_FRAME_DEADLINE: u64 = GPU_RING_DEPTH as u64 + RETIRE_GRACE_FRAMES as u64 + 2;  // 8

const GPU_ZONE_QUERY_FLAGS: u32 = VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WITH_AVAILABILITY_BIT;
const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0);   // G2a's real red
```

`CommandWitness` is declared in D17 above and lives behind `feature = "profiling-census"`.

**The `FrameSlot` `marks` / `seal` rows of the multithreading table are NOT restated here.** They
live in `profiling/emission-abi`'s one per-datum sharing table with the rest of the orderings,
because that table is one object with one ordering rationale and splitting it is how two files come
to disagree about a memory ordering.

**GPU host-side residency is 8 KiB** in every configuration of the sizing table — the pools, slots
and mark arrays are compile-time-extent host state. The table itself is
`profiling/budgets-and-invariants`'s.

---

## Public API — the GPU slice

```rust
// ── RHI seam (three verbs; NONE of them can block — D4) ──
fn read_query_pool_pairs_available(&self, pool: &A::QueryPool, pair_count: u32,
    scratch: &mut [u64], out_begin_ticks: &mut [u64], out_dur_ticks: &mut [u64],
    out_available: &mut [u8]) -> Result<(), Self::Error>;
fn reset_query_pool_host(&self, pool: &A::QueryPool, first: u32, count: u32)
    -> Result<(), Self::Error>;
fn host_query_reset_supported(&self) -> bool;

// ── host-called retire / drain (NOT scheduled systems — A3) ──
pub fn retire_gpu(world: &mut EcsMaster, rec: &mut GpuZoneRecorder,
                  render_epoch: u64, frame_now: u64);
pub fn flush_gpu(world: &mut EcsMaster, rec: &mut GpuZoneRecorder);   // teardown; D4a / F28
```

From the corpus-wide **Deliberately absent** list (`profiling/emission-abi` carries it in full,
because it constrains all three halves), the clause this file is responsible for making true:
**any GPU reader that can block**. It is absent by construction, not by convention —
`GPU_ZONE_QUERY_FLAGS`'s `const _` assert is the mechanism.

---

## A3 — GPU slot retire (host-called, NOT a scheduled system)

Called by the runner on the line that already publishes `RenderEpoch`
(`crates/boyko_app/src/runner.rs:1320`):

```
retire_gpu(world, recorder, render_epoch, frame_now):
  for slot in ring where slot.in_flight:
    read_query_pool_pairs_available(...)          // never blocks; VK_NOT_READY -> Ok, bits clear
    if avail covers every bracketed pair { publish MEASURED; retire; continue }

    // HORN 1 — the submit-epoch deadline (D4a). The decrement lives INSIDE this arm and is
    // guarded, so it can never run on a zero `grace` (rev 3 wrapped u8 0-1 to 255 — M13a).
    if render_epoch >= slot.submit_epoch + FRAMES_IN_FLIGHT {
        if slot.grace > 0 { slot.grace -= 1; continue }
        label_and_retire(slot); drops.gpu_lost += lost_count; continue
    }

    // HORN 2 — the FRAME deadline (M13b). Fires when submits freeze but frames do not:
    // the 0x0-client `continue` at runner.rs:1330 skips record+submit while
    // update_with_delta keeps running, so RenderEpoch stops and horn 1 never fires.
    if frame_now - slot.record_frame > GPU_FRAME_DEADLINE {      // = 4 + 2 + 2 = 8
        label_and_retire(slot); drops.gpu_frame_deadline += 1; continue
    }

label_and_retire(slot):
    marks = (slot.seal.load(Acquire) == slot.frame) ? read marks : all-zero
    per pair: (1,1,1)=>MEASURED (0,0,_)=>NOT_BRACKETED (1,0,_)=>TORN (1,1,0)=>LOST
    retire PARTIAL                                // COUNTED, never printed per pair (F20)
    reset_query_pool_host(..) if supported else set needs_cmd_reset (D18)
```

**Where it is called from, and why that line (M13b).** At `crates/boyko_app/src/runner.rs:1320`,
immediately after the `RenderEpoch` publication and **before** the 0×0-client `continue` at
`:1328-1332` — so it runs on every iteration of the host loop, minimised or not. Rev 3 said both
"between `wait_frame_in_flight()` and the record" and "on the line that publishes `RenderEpoch`";
those sit on opposite sides of that `continue`, and only the second runs while minimised.

O(`GPU_RING_DEPTH` × pairs) = 512/frame. **Termination proof, now covering both stalls:** a slot
retires on availability, on the submit-epoch horn, or on the frame horn. `RenderEpoch` advances once
per successful `vkQueueSubmit` (`frame_driver.rs:256-263`); if it stops while frames continue, horn 2
fires within `GPU_FRAME_DEADLINE` frames; if frames themselves stop, `flush_gpu` at teardown covers
it (D4a/F28). **No path waits on a query and no path waits on a fence, and no path can underflow
`grace`.**

**Why NOT a `requires_dispatcher` system (F14).**
`crates/boyko_ecs/src/ecs/core/system/system_meta.rs:130-141` states that `requires_dispatcher` is
set by `NonSendRes`/`NonSendResMut::init_access`, and *"Those params ALSO declare universal access
(CR-B), so the existing `is_universal()` derivation resolves the system to
`SystemKind::CpuExclusive`."* Pinning retire that way inserts **a full schedule serialisation point
every frame** — in the subsystem whose headline product is a concurrency statistic, and against D16.
Rev 2's M5 fold bought thread pinning at the price of the property D9 exists to measure. Running it
at the host seam costs nothing, needs no `SystemSet`, and reads host state (the recorder, the epoch)
where that state already lives. **Cost, stated:** the retire is a host-called function rather than an
ECS system, which is less ECS-native in *shape*; the precedent is the adjacent line in the same loop,
where the host publishes `RenderEpoch` into the world by hand.

### What `retire_gpu` costs with the runtime flag OFF

`retire_gpu` is on the host loop unconditionally, so it is a *site* in the S13 sense
(`SEAM.md` §S13 owns that argument; it is not re-derived here). With `ARM_MASK == 0` no slot is ever
marked in-flight, so the loop body executes zero times: **one `.bss` load and one statically
predicted branch per host iteration — once per frame, not per zone.** That cost is not driven to zero
by the flag and only the compile-time ceiling removes it, which is exactly the per-site row of S13's
cost table; it is recorded here so the number is not discovered later as a surprise. No query pool is
created, no slot is touched and no `vkCmd*` is recorded while the flag is off — which is the same
claim G5's disarmed side already makes, measured by the census rather than asserted.

---

## Citations re-verified at the carve (2026-08-08, against HEAD)

Carried claims were re-read in the tree rather than inherited. Confirmed unchanged:
`ffi.rs:846` (`VK_QUERY_RESULT_64_BIT = 0x0000_0001`), `ffi.rs:849`
(`VK_QUERY_RESULT_WAIT_BIT = 0x0000_0002`), **zero occurrences of `WITH_AVAILABILITY` anywhere in
`crates/` or `src/`**, `ffi.rs:2716` (`pub host_query_reset: VkBool32`), `swapchain.rs:199`
(`present_mode: VK_PRESENT_MODE_FIFO_KHR`), `surface.rs:218` (`present_mode_supported`),
`swapchain.rs:10` (its import), `swapchain.rs:164` (its existing use), `frame_driver.rs:319` and
`:400` (`wait_for_fences(..., VK_TIMEOUT_INFINITE)`), `asset_refcount.rs:55`
(`pub struct RenderEpoch(pub u64)`), `runner.rs:1320` (the `RenderEpoch` publication), `:1321`
(`app.update_with_delta(dt)`), `:1328-1332` (the 0×0 `continue`), `gpu_timing.rs:186-203` and
`:575-584` (the two `WAIT_BIT`-blocks-forever comment blocks), `:229` (`pub enum VbTimedPass`),
`:311-329` (`from_slot`'s hand-maintained table), `:333-365` (`begin_stage` + its prefix-completion
argument), `:391` (`VB_PASS_COUNT: u32 = 10`), `:432-445` (the "why the masks cross the seam"
argument, including *"Recorder and readback are the same thread today"* at `:443-444`),
`device.rs:1249` (`fetch_query_pair_ticks`) with masking at `:1257-1265`,
`vb_bench_totality_gate.rs:44-53` (no kill-after-timeout pattern), `PINS.toml:3` (SHA-256 of a
dumped BMP).

**Corrected while carrying** (rev 4's citation was stale or imprecise; the argument is unaffected):

| Rev 4 said | Tree says | Effect |
|---|---|---|
| `frame_driver.rs:255-262` — the asset-retire doc | `:256-263` | none; text identical |
| `wait_frame_in_flight` at `runner.rs:1336` | `:1337` (`:1335-1336` are its comment) | none |
| "its three siblings `fetch_query_pair_ticks` / `fetch_query_pair_stamps` (`:1249`)" | three exist, two were named: `fetch_query_raw_ticks`, `fetch_query_pair_ticks` (`:1249`), `fetch_query_pair_stamps` (`:1288`) | none |
| `VbRecordProbe` "increment sites at `vb.rs:107-156`" | `:107-156` is the **struct declaration and its field docs**; the increments are at `:1710`, `:1948`, `:2313`, `:2583`, `:2798` | none to the rule; the citation would have sent a reader to the wrong lines |
| `runner.rs:261` "already force-drains host-owned per-frame resources there" | `:258-263` is a **comment block describing** the teardown force-drain, not the teardown path itself | the teardown site `flush_gpu` hooks into must be named by file:line at the rung, not inherited from this citation |
