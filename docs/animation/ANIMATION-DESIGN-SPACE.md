# Animation — the design for THIS engine

> Status: architect's design, 2026-09-10, **revised after critic pass 1** (4 blocking + 11
> non-blocking findings; each is answered in §15 — fixed in place, or refuted with evidence).
> The survey it rests on is [`ANIMATION-RESEARCH.md`](ANIMATION-RESEARCH.md) (§0 there lists what
> the tree holds today; every `file:line` below was re-opened at commit `128233be` on
> `feat/multi-paradigm-render`). Numbers are either measured elsewhere and cited, or **labelled
> estimates**. §13 separates the PERF/ARCHITECTURE decisions taken here from the VALUES/SCOPE
> decisions that go to the owner. This directory is not machine-anchored
> (`tests/internal_docs_anchors.rs:231`).

## 0. The shape in one paragraph

Animation is three ECS-native things and one GPU pass. **Assets** (skeleton, compressed clip, blend
program, motion database) are Gaia-baked byte columns loaded by blit into append-only tables.
**Per-instance state** is one dense component (`AnimInstance`, the lane handle) plus ordinary table
components for playback / graph / machine state. **Poses** live in a resource-owned, lane-group-major,
8-lane SoA **pose bank** per skeleton — the physics `SolverScratch` shape
(`crates/boyko_physics/src/resources.rs:3159`), never a `Vec` — and are recomputed every fixed
substep by one system that fans out over lane groups with ≥10 µs bodies. **That fan-out is
serial on this branch** (§2.1, AK-9): the worker-spawned `pool.scope` route is the measured serial
floor until the KE16 pool lands, and L0's parallel gate (G2) is red by construction until it does. The pose reaches the
screen as a **palette** in the `GpuTransform3D` discipline (prev/curr, single shuffle site, GPU lerp
at `alpha`; `crates/boyko_render/src/gpu_transform3d.rs:85`) and is skinned once per frame by an
eDSL compute pass into a skin-cache ring the existing G-buffer / VB passes read unchanged. Logic is
the per-entity Aether `machine` (rung R5) over these components; nothing new is added to the
language for v1.

## 1. Data model

### 1.1 Skeleton asset — `SkeletonAsset`

One row per bone, bones sorted **parent-first and LOD-first** (a bone's parent has index < its own
and LOD ≤ its own — a bake contract, not a runtime check), so `bones[0..B_l]` is the complete
skeleton at LOD `l`. Both ozz orderings satisfy this (RESEARCH §2.1's recorded conflict does not
matter here).

```
SkeletonBone {              // 96 B, #[repr(C)], POB
    parent:    u16,         // u16::MAX = root
    lod:       u8,          // 0 = always present
    flags:     u8,          // socket-eligible, root-motion bone, …
    radius:    f32,         // bind-space bounding-sphere radius of the vertices this bone influences
                            //   (baked from the skin; 0 for a bone that skins nothing) — feeds the
                            //   per-instance animated AABB of §2 ③ (critic NB3)
    rest:      [f32; 10],   // t3 q4 s3 — local rest pose
    inv_bind:  [f32; 12],   // 3×4 row-major, the InstanceModelCol convention (instance_model.rs:58)
}
SkeletonBoneName { name_hash: u32 }   // COLD column parallel by index — bake / tooling only; the
                                      //   runtime binds by index (RESEARCH P4). Moved out of the
                                      //   hot row to make room for `radius` at the same 96 B.
SkeletonAsset { bones: Range<u32> into the bone table, lod_counts: [u16; 4], root_bone: u16,
                mask_words: u16 (= ceil(bone_count / 64)), masks: Range<u32> into the mask table, … }
```

**Bone masks** are a per-skeleton table of `u64` words: mask `m` occupies words
`masks.start + m·mask_words ..+ mask_words`; mask 0 is baked as all-ones ("every bone"). A `mask:
u16` field anywhere in this document is an **index into that table**, never a bitmask itself — a
16-bit bitmask covers 16 bones and the examples here are 64–128 (critic NB7). The width matches
Bevy's `u64` per node [S] (RESEARCH §2.5) extended to any bone count by the word run.

Storage: an append-only, resource-owned column of `SkeletonBone` (`ScratchColumn`-class backing —
`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:39` — with the synthetic-id
registration precedent of `crates/boyko_physics/src/scratch_ids.rs:1-45`), indexed by a bare `u32`
carrier like `MeshHandle(u32)` (`crates/boyko_scene/src/render_caps.rs:143`) under the same
append-only / live-forever rule (`crates/boyko_ecs/src/ecs/core/asset/handle.rs:41-44`).

### 1.2 Clip asset — `ClipAsset` (the codec)

**Uniform sampling** at a baked rate (30 / 60 Hz per clip), no cursor — every sample is a random
access by integer frame index, so a task is pure and direction/rate-independent (ACL's measured
property, RESEARCH §2.3). Layout follows ACL's design, in-house — sourced per item:

- per-track **constant** detection (a static bone costs 0 B in the animated stream) — [D]
  `github.com/nfrechette/acl/blob/develop/docs/algorithm_uniformly_sampled.md`: "Constant tracks
  are compacted to a single sample and default tracks are dropped";
- **segments of 16 frames** (ACL's `compression_segmenting_settings { ideal_num_samples = 16;
  max_num_samples = 31; }` — [S] `includes/acl/compression/impl/segment_context.h`) with
  **per-segment range reduction** (`normalize_segment_streams` / `extract_segment_bone_ranges`,
  "Segment ranges are always normalized and live between [0.0 ... 1.0]" — [S]
  `includes/acl/compression/impl/normalize.transform.h`); samples "sorted by time and by track"
  ([D] the algorithm page). The **four-stream** split and the "2–5 cache lines per frame" figure
  are [B] (nfrechette.github.io, RESEARCH §2.2) — recorded, and re-derived here from the sort
  order rather than relied on;
- a **scalar track stream** (critic NB2): N per-clip scalar tracks, uniformly sampled at the clip
  rate, 16-bit range-reduced, constant-detected like the bone tracks. It carries glTF `weights`
  channels (morph-target weights — [D] glTF 2.0 Specification.adoc, opened:
  `animation.channel.target.path` ∈ {`translation`, `rotation`, `scale`, `weights`}) and any
  authored curve (IK alpha, material parameter). Decoded per lane into the `LaneJob.scalars` slots
  of §1.4, whole-vector like the pose;
- rotations as 3×16-bit smallest-three (+√2 premultiply, ozz's +41 % range), translations / scales
  as 3×16-bit range-reduced (v1; ACL's per-track variable bit rate is a v2 rung with a measured
  size/quality trade);
- a per-clip **event table** (`sample_index, event_id`) and **sync-marker table**;
- a root-motion track split out (translation + yaw) so root delta extraction never touches the
  pose stream.

Compression is a bake-time tool (ACL: 1 h 53 m for 6 558 clips on 11 threads [B] — offline by
construction). The runtime holds one `ClipHeader` row per clip in an append-only column and the
byte streams in an asset-blob region (§6). **Decode is whole-pose only** (RESEARCH P1); masks are
applied at blend.

### 1.3 The pose bank — lane-group-major, 8-lane SoA (the numbers that decide it)

Per skeleton asset, one **bank**: rows are bone groups, lanes are instances.

```
BoneSoa8 { f: [[f32; 8]; 12] }      // 384 B = 6 cache lines, #[repr(C, align(64))]
// local pose uses fields 0..10  (tx ty tz | qx qy qz qw | sx sy sz)
// local→model converts IN PLACE to 12 fields = the 3×4 row-major affine of the bone in model space
bank[lane_group][bone]              // LANE-GROUP-major: the B bone rows of one lane group are
                                    //   ONE contiguous B × 384 B run
```

**Why lane-group-major and not bone-major** (critic B2 — the first draft had it backwards).
Every pass in §2 — sample, blend, inertialize, local→model, palette pack — walks the bones of a
FIXED lane group; nothing ever walks the lane groups of a fixed bone. Under `bank[bone][group]` a
task touches bone `b` at `base + (b·G + g)·384`, a constant stride of `G × 384 B`; since
`384 = 3·128`, that stride is a multiple of 4096 whenever `G` is a multiple of 32 (every 256
instances), so at 1 024 instances (`G = 128`, stride 49 152 B = 12 × 4096) every bone row of a task
lands in the **same six L1 sets** and 100 bones × 6 lines fight for 8–12 ways — the exact
conflict-miss storm this tree already paid for once (`crates/boyko_ecs/src/ecs/memory/component_pool.rs:292-298`
"P2-CACHE-FIX … puts element `i` of every column in the SAME cache set … a conflict-miss storm that
cost the ~40% rigid-solver regression"; the set arithmetic at
`crates/boyko_ecs/src/ecs/constants.rs:178-196`). Lane-group-major makes a task's working set one
contiguous run that spreads over every set evenly, which is what the L1 argument below needs.

**L1d fit, stated as a bound**: the run is `B × 384 B`, so it fits a 32 KB L1d only for
`B ≤ 85`; the 64-bone example is 24 576 B (fits), a 100-bone skeleton is 38 400 B (L2-resident by
capacity, before any set argument), 128 bones is 49 152 B. The LOD prefix (§9) is what brings a
task under 85 bones, not the layout. The inertialization bank (§1.5) is a second `B × 384 B` run
touched only by the decay pass; when a group is in transition the pair is `2 × 24 576 B` — L2,
not L1. The "24 KB stays in L1d for the whole body" claim of §2.1 is therefore made for the pose
bank's sample → blend → local→model passes at ≤ 85 bones, and for nothing else.

The two banks and the `LaneJob` column are three `ComponentPool`-backed columns
(`ScratchColumn` wraps one — `scratch_column.rs:39-41`), so each gets the `pool_base_stagger`
leading offset (`component_pool.rs:299`) and row `b` of the pose bank and row `b` of the
inertialization bank do not share a set either.

Why lanes = instances rather than ozz's 4 joints per group (the fork of RESEARCH §2.1):

| Axis | joints-per-group (ozz SoA4) | instances-per-group (this design, SoA8) |
|---|---|---|
| SIMD width filled | 4 (SSE-shaped) | **8** — the AVX2 baseline (`CLAUDE.md` §Target platform) |
| Intra-group dependency in local→model | parent chains cross groups; ozz converts to AoS per joint | **none** — every lane walks the same parent index; one `parent[b]` load serves 8 instances |
| Blend / inertialize / palette | per instance | the same op on 8 instances, no shuffles |
| Sampling writes | contiguous 40 B per bone | scattered 4-B stores into 6 lines per bone — but a task samples all 8 lanes of its group, so the 6 lines are filled completely before eviction |
| Working set, 64 bones, one lane group | 2.5 KB per instance × k instances | **24 576 B** (64 × 384), one contiguous run — under the 32 KB L1d the principles target; the bound is **B ≤ 85** (above) |
| Crowd of distinct skeletons | no waste | ≤ 7/8 lanes idle per bank — at 50 singleton banks the padding is 50 × 24 KB = 1.2 MB, and no extra *traffic* (idle lanes are never read by anyone) |

Per-instance cost of the bank: 48 B × bones (384 B / 8 lanes × B). 64 bones → **3 072 B**; 10 000
instances → 30.7 MB of bank. Traffic per substep at that size (**estimate** from the sizes; critic
NB6 — the first draft's "~2 GB/s write + read" counted the write-back alone): the sample → blend →
local→model passes run inside a task's L1 run and reach DRAM once as the bank's write-back
(30.7 MB); ⑥ then re-reads the model rows (30.7 MB — a separate system after the physics block,
so not from L1) and writes the palette (48 B × bones × instances = 30.7 MB), which the upload reads
again (30.7 MB). That is ~123 MB per substep × 64 Hz ≈ **7.9 GB/s**, or ~4 GB/s if the bank and
palette stay L3-resident between passes (30.7 MB is at the edge of a 32 MB L3) — a crowd that size
is DRAM-bound and needs §9's LOD prefix and update-period budget, not a different layout.
128-bone skeletons put a lane group at 48 KB (L2-resident, not L1); the LOD prefix, not the
layout, is the answer there too.

Lane assignment is **tombstone + LIFO free list** (the `DenseStore` Decision 3,
`docs/DENSE-COMPONENTS-PLAN.md:19`), so a fixed op sequence yields identical lanes run-to-run — the
same argument as C3(c) there gives bit-identical evaluation order. A cold `compact_lanes()` at one
fixed schedule point (the `compact()` boundary discipline — item (d) at
`DENSE-COMPONENTS-PLAN.md:59`; critic NB10 caught the off-by-one) re-sorts lanes by
**(LOD, blend program)** so a lane group holds one LOD class and one program where possible
(§1.6 says what happens when it does not). Lane identity is, however, **not** what makes
evaluation deterministic — §8 D15: an instance's bits do not depend on which lane or group it
occupies.

The bank is recomputed every substep, so it IS scratch in the `SolverScratch` sense; it is not a
`DenseStore` because a `DenseStore` maps one entity to one fixed-stride slot
(`crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:65-79`) and a bone run per entity is
a shape the kernel does not have. The per-entity durable handle is the dense component below.

### 1.4 Per-instance components

```
#[component(storage = "dense")]           // one global column, no archetype fragmentation
AnimInstance { skeleton: u32, bank_lane: u32, lod: u8, period: u8, phase: u8, flags: u8 }   // 12 B

AnimPlayer   { slots: [PlaySlot; 4] }     // table component; PlaySlot { clip: u32, tick: u32 (Q16.16 sample
                                          //   phase, §8), speed: i32 (Q16.16), weight: f32, mask: u16 (mask-TABLE
                                          //   index, §1.1), flags }
AnimGraphState { program: u32, params: [f32; 8] }      // table; the baked blend program (§1.6) + its inputs
RootMotion   { delta_t: [f32; 3], delta_yaw: f32 }     // table; written by evaluate, read by gameplay/physics
AnimSocket   { instance: Entity, bone: u16 }           // table; on an attachment entity (§7)
AnimBounds   { min: [f32; 3], max: [f32; 3] }          // table; pose-dependent LOCAL AABB written by ③ (§2, §4)
```

`AnimInstance` is dense because every animated entity has exactly one and the lane-indexed
scratch below is keyed by its `bank_lane`; the others are table components so the Aether machine's
`joins (mut anim: &mut AnimPlayer)` fuses into its single chunked pass
(`docs/aether-v2/MACHINES.md:14-17`) — the R-DENSE refusal there (`:103`) is exactly why the
machine-facing state is NOT dense.

**The lane-indexed job column** (critic B3 — the first draft had no gather step, so ③'s tasks had
no way to reach a lane's clip / tick / weight / program). Per bank, a resource-owned
`ScratchColumn<LaneJob>` indexed by `bank_lane`, **refilled every substep by ②** — the
`bodies_build` refill of `SolverScratch` (`crates/boyko_physics/src/resources.rs:3159-3163`,
"refilled each gather via `bodies_build`") transplanted:

```
LaneJob {                                 // 192 B, #[repr(C)], POB — one per lane; the input half written by ②
                                          //   and read by ③'s tasks, the `out_*` half written by the tasks
    entity:     EntityId,                 // owner (or unset); ③'s TASKS never touch it — the sequential
                                          //   tail of ③, after the scope joins, scatters `out_root` /
                                          //   `out_bounds` (below) into RootMotion / AnimBounds by entity
    out_root:   [f32; 4],                 // root delta (t3 + yaw) written by the task
    out_bounds: [f32; 6],                 // local AABB min/max written by the task
    lod_bones:  u16, period_phase: u16,   // this substep's LOD prefix and (period, phase)
    program:    u32,                      // blend program id (§1.6)
    slots:      [(clip: u32, frame: u32, frac_q16: u16, weight_q16: u16, mask: u16); 4],   // 64 B
    params:     [f32; 8],                 // AnimGraphState.params snapshot
    inert_t:    f32,                      // inertialization elapsed time — PER INSTANCE, so it lives
                                          //   here and not in the per-bone bank (critic NB8)
    scalars:    [u16; 6],                 // decoded scalar-track outputs (§1.2) — morph weights, curves
}
```

② is therefore **not** a `dense_iter` (`Query::dense_iter` const-rejects a table term —
`crates/boyko_ecs/src/ecs/core/iters/query/query.rs:434-436`): it is the mixed `Query::iter` over
`(&AnimInstance, Mut<AnimPlayer>, &AnimGraphState)`, archetype-driven with the per-row `e2s`
gather the dense plan specifies (`docs/DENSE-COMPONENTS-PLAN.md:11`), and its output is the
`LaneJob` row at `bank_lane`. ③'s tasks read **only** `LaneJob` rows, clip bytes and the skeleton —
never a table component — which is what makes them pure over the lane-indexed scratch.

### 1.5 Inertialization bank

A second bank per skeleton with the same lane geometry and the same row type: `InertSoa8 = BoneSoa8`
(`[[f32; 8]; 12]`, 384 B = exactly 6 lines) — offset (t3 + rotation-vector 3 + s3) + offset
velocity (same 9) is 18 floats; v1 drops scale offsets to 12. The transition clock `t_elapsed` is
per **instance**, not per bone, so it is `LaneJob.inert_t` (§1.4), and the row is not the 13-field,
416-B, line-straddling shape of the first draft (critic NB8). It is written once at a transition and
decayed each substep by the critically-damped-spring form (RESEARCH §2.6, [B] spring-roll-call), with
the per-skeleton `t, t², t³` precomputed once (Kinemation's `InertialBlendingTimingData` [S]). This
column is **durable** across substeps — the same storage class used without `clear` (§11 AK-2
names the naming debt).

### 1.6 Blend program (graph state as data)

A blend tree is baked into a flat **postorder program** — the Bevy `ThreadedAnimationGraph`
shape [S] and the UAF task list [B] — of ≤32 ops `{ op: u8, a: u8, b: u8, mask: u16, weight_src:
u8 }` over a small pose-register stack; no node objects, no recursion (RESEARCH P18). Evaluation of
one program over a lane group is a loop over ops applying the same SIMD op to 8 instances.

**Divergent programs in one lane group** (critic B3): `program` is per instance and
`compact_lanes()` only sorts by (LOD, program) at its cold fixed point, so between compactions a
group may hold lanes with different programs. Then ③ runs the **masked union**: the group's op
stream is the concatenation of the distinct programs present, each op tagged with the 8-bit lane
mask of the lanes that own it, and every blend op is an AVX2 `blendv` under that mask. Registers
are lane-wise, so a lane only ever observes its own program's ops — the result is bit-identical to
running the lane alone (D15). Cost is Σ(distinct program lengths) ≤ 8 × 32 ops per group, and the
steady state after a compaction is one program per group. No other mechanism (program-keyed
sub-banks, per-lane scalar fallback) is used.

State machines are NOT in the program: they are the Aether `machine` (§5) that writes
`AnimPlayer` slots and `AnimGraphState.params`.

### 1.7 Palette (GPU side)

`PaletteRow = [f32; 12]` (3×4 row-major = `InstanceModelCol`, 48 B, pinned at
`crates/boyko_render/src/instance_model.rs:73`) per bone per instance, **double-buffered**
(prev/curr) in one SSBO ring with per-instance `(period, phase)` so the compute pass lerps at
`alpha' = (phase + overstep) / period` — the `GpuTransform3D` pair generalised to a bone array
(`gpu_transform3d.rs:66-84`). Sizes: 96 B per bone per instance; 64 bones × 10 000 = 61 MB (an
**estimate** of the ceiling the ring reserves; commit is lazy). Dual-quaternion rows (32 B) are a
measured later variant — LBS first, DQS as an A/B with the golden harness.

### 1.8 Skinned vertex stream

The static `Vertex` stays 64 B (`mesh.rs:83`, stride pinned `:103-104`). Skinning adds a **side
stream** per skinned mesh: `SkinVertex { joints: [u16; 4], weights: [u16; 4] }` = 16 B (u16 joint
index → 65 535 bones per skeleton, no per-section cap; unorm16 weights). Rukhanka's measured
36 → 20 B win [B] is the reason the side stream is not folded into the 64-B record. The skin-cache
output is a full 64-B `Vertex` ring (positions, normals, tangents skinned; color / uv copied) plus a
12-B previous-position ring for motion vectors, so the G-buffer, shadow and VB passes read it with
**no shader change**.

### 1.9 Motion-matching database as columns

Feature-major SoA: `features[f][frame]` (one f32 column per feature, frames contiguous) so the
brute-force distance loop streams 8 frames per op — the orangeduck reference is frame-major AoS
with no SIMD [S]. Frames reference `(clip, sample_index)`; the pose itself is **not** stored —
it is decoded from the clip on selection, which is what keeps the database at features only
(30 features × 100 k frames × 4 B = **12 MB**, versus the 100–590 MB pose-storing databases of
RESEARCH §2.7 [B]). Coarse AABB tables at 64 / 16 frames [S] are two more small columns.

## 2. Evaluation pipeline — systems on the schedule

All in `CoreSchedule::Fixed` (`crates/boyko_ecs/src/ecs/core/app/app.rs:64`), 64 Hz default
(`fixed_time.rs:21`), joined to `FixedSet` by name (`crates/boyko_scene/src/sets.rs:35`):

```
FixedSet::Gameplay
  ① machine passes (Aether, R5)         write AnimPlayer / AnimGraphState          [chunked, table]
  ② anim_advance                        significance snapshot (§8) → budget rank, period, LOD; clocks,
                                        clip events → event lanes, slot fades; GATHER into LaneJob rows
                                        (.after every machine that joins AnimPlayer)
                                        [mixed Query::iter over (&AnimInstance, Mut<AnimPlayer>, &AnimGraphState) — §1.4]
  ③ anim_evaluate                       per lane group, from LaneJob only: sample → blend program →
                                        inertialize → local→model (in place) → per-instance AABB
                                        (bone spheres ∘ model rows, §1.1 radius) → RootMotion, AnimBounds
                                        [pool.scope, ≥10 µs bodies — SERIAL on this branch, AK-9]
  ④ anim_drive_bodies                   hit-box / ragdoll targets: Transform + RigidBody velocity
                                        .before(sync_transform_to_body)  (crates/boyko_physics/src/plugin.rs:143-148)
  [physics block  sync_transform_to_body → integrate → … → apply → sync_body_to_transform]
  ⑤ anim_readback                       ragdoll bodies → bone rows blend; IK against ground; sockets
                                        .after(sync_body_to_transform)
FixedSet::Snapshot
  ⑥ anim_pack_palette                   model rows × inv_bind → palette staging; prev-shuffle (single site)
Main (GpuCompute, dispatcher-solo — system_config.rs:218)
  ⑦ skin_cache                          eDSL compute: palette lerp at alpha' + LBS → vertex ring (§4)
```

Why the pose is evaluated at the fixed rate: gameplay consumers (④, hit boxes, root motion,
events) need it inside the fixed step, and the render side already interpolates fixed-rate poses
(`gpu_transform_pack.rs:1-9`). One evaluation per substep, 0–N per frame, never per render frame;
at 144 Hz that is 64 evaluations/s instead of 144. The price is up to one substep (15.6 ms) of
visual latency, the same the engine already accepts for physics bodies (owner value, §13.B).

### 2.1 Wave shape for ③ (the pool's measured floor)

The unit of work is a **lane group** (8 instances of one bank): sample 8 whole poses, run the blend
program 8-wide, decay inertialization, convert in place — the 24 KB working set stays in L1d for
the whole body. Body time per lane group, **estimate**: 8 × (one whole-pose decode — ACL measured
0.9–28.8 µs cold-cache at 2.39 GHz [S/D] — plus ~1 affine multiply per bone) ≈ **40–250 µs** at 64
bones. That is inside the 10 µs–1 ms band the KE16 record measures as healthy **on the KE16
arm** (`D:/wt/threadpool/docs/threadpool/KE16-RESULTS.md:285-290`, `:655-671`: ≥10 µs bodies
unaffected by the 1.98.1 regression) and far above the 1 µs cells that read the serial floor.

**⚠ The route this fan-out uses is serial on this branch** (critic B1). `pool.scope` called from
inside a system runs on a worker, and the worker-spawned route is, at the pool this branch ships,
exactly the serial floor: "Pool worker route, 1024 tasks × 1 ms body: 1024.26 ms against a
1024.0 ms serial floor … The worker route is not 'slow', it **is** the serial floor"; "ECS:
`par_in_system` equals `seq` to four significant figures … `max_in_flight = 1`"
(`KE16-RESULTS.md:292-300`, [D], worktree `feat/threadpool-ke16`). That fix is **not here**:
`git merge-base --is-ancestor feat/threadpool-ke16 HEAD` exits 1, `grep -rn KE16
crates/boyko_threadpool/src` is empty, and `crates/boyko_threadpool/src/thread_pool.rs:424-430`
is the unchanged forward to `inner.scope`. Consequences, stated rather than worked around:

- **AK-9** (§11) — landing the KE16 pool (`a1f+b1+wc+c0`, its Step App take at
  `KE16-RESULTS.md:1668`) is a **prerequisite of L0's G2**, not of L0's correctness: ③ evaluates
  the same bits on one lane, so G1, G3–G7 can go green before it.
- **G2 is red by construction until AK-9 lands** and the ladder says so; a G2 that reads green on
  this branch is measuring something other than ③ (the per-worker touch counts are what make that
  visible — a `max_in_flight = 1` run is red, exactly the shape the record reports).
- Whether registering ③ as an exclusive system would put its `scope` on the dispatcher route
  (`par_from_dispatcher` is **not** serial in the record, `:1668-1697`) is **unmeasured** and is
  not claimed as an interim; it is listed in §14.

Task count: `groups_per_task = clamp(ceil(G / (6·W)), 1, floor(1 ms / t_group))`, `G` = live lane
groups, `W` = `ThreadPool::worker_count()` (`crates/boyko_threadpool/src/thread_pool.rs:356`). At
W = 16: G ≤ 96 → one group per task (the W..6W shape of `KE16-DESIGN-W.md:63`); G = 1 250 (10 000
instances) → 4 groups per task at t_group ≈ 250 µs → ~312 tasks of ~1 ms. The 6W target yields to
the 1 ms body cap because the record shows 64W-wide waves of ≥10 µs bodies healthy while it shows
nothing about 3 ms bodies. Dispatch is `pool.scope` from inside the system (the worker-spawned
route — the one KE16 fixes and this branch does not carry; `thread_pool.rs:424`, `scope.rs:288`),
copying one `ScratchSolveView` per bank (pose, inertialization, `LaneJob`) into the closure — each
task writes only its own lane groups' rows (the distinct-slot discipline of
`crates/boyko_ecs/src/ecs/core/component/dense/views.rs:130-157`). No `Vec` mirror exists to race
(RESEARCH P25).

Both kernel parallel drivers reject dense terms (`par_iter.rs:300-309`, `par_chunk.rs:117-124`), so
③ is hand-fanned like the physics colored solve; ② is a sequential mixed-query gather (~1 µs per
instance, **estimate**), parallelised only if measured necessary (KE9's
`par_for_each_chunk_entities` is the candidate, `docs/aether-v2/KERNEL-BACKLOG.md` KE9).

### 2.2 Scheduler shape

One `&mut` writer per column per system (write conflicts are filter-agnostic,
`gpu_transform_pack.rs:44-56`): ③ owns the pose bank, ⑥ owns the palette staging, ④ owns the hit-box
`Transform`s. The bank is a `Resource`, so ③ is one conflict node on it — the same "one column =
one node" that made the dense plan's scheduler story exact (`DENSE-COMPONENTS-PLAN.md:13`).

## 3. Blending semantics (pinned)

Local-space nlerp with shortest-path sign fix for rotations (slerp is a cold path for large
angles only), lerp for translation / scale; layer order = program order; weights normalised at
bake where the graph is a pure mix and left un-normalised for additive ops. Pinned by a
bit-identity fixture (RESEARCH P19). Inertialization envelope asserted in debug: half-life ≤ 0.4 s
and a pose-difference bound (RESEARCH P12).

## 4. GPU skinning — the eDSL compute pass

One leaf, `skin_lbs`, written once as a generic body over the scalar trait
(`crates/boyko_shaderdsl/src/scalar.rs:36`), instantiated over `f32` (the CPU oracle) and `Emit`
(the HLSL printer), spliced between `// === GENERATED skin_lbs BEGIN/END ===` sentinels, gated by a
`skin_edsl_sync` test with the same two gates as `interp_edsl_sync` (text match; fresh DXC under
`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3` byte-equal to the committed `.spv`;
`crates/boyko_rhi_vulkan/tests/interp_edsl_sync.rs:12-22`), and a row in
`docs/SHADER-VARIANT-MANIFEST.md`.

Dispatch: one invocation per skinned vertex of a **visible, LOD-selected** instance (RESEARCH P7),
batched per skeleton (Kinemation's one-dispatch-per-skeleton [D]); reads the palette pair, lerps at
`alpha'`, applies `Σ wᵢ · Mᵢ` to position / normal / tangent, writes the 64-B `Vertex` ring and the
12-B prev-position ring. The oracle test: the `f32` instantiation over a fixture mesh equals a
scalar CPU skinning reference byte-for-byte (the `eval_byte_identity` discipline).

**Who decides "visible", and against which bounds** (critic NB3). Today's cull tests the
instance's mesh-LOCAL box `gMeshBounds[mesh_id]` transformed by the instance model
(`crates/boyko_render/src/frustum.rs:152`, `instance_visible_after_cull` at `:170`) — one
`MeshLocalBounds` row per **mesh** (`mesh_geometry_table.rs:178`), which is the rest-pose box and
pops the moment a pose leaves it. The design adds the animated box: ③ folds each LOD-prefix bone's
bind-space sphere (`SkeletonBone.radius`, §1.1) through its model row — 8-wide min/max, one
fused pass over rows the task already holds — and writes `AnimBounds` (§1.4); the cull reads
`AnimBounds` in place of `gMeshBounds[mesh_id]` for a skinned instance (AK-7). That is
Kinemation's "Accurately computed at runtime" column ([D] Latios-Framework-Documentation,
"Kinemation vs Entities Graphics.md") rather than Entities Graphics' "Estimated from animator".
The **visibility verdict is CPU-side** in v1 — `instance_visible_after_cull` over `AnimBounds`,
run in Main before ⑦ builds its dispatch list — because the GPU cull cannot drive a compute
dispatch yet: `dispatch_indirect` is a Phase-6+ seam whose default body is a no-op
(`crates/boyko_rhi/src/encoder.rs:362-372`, "no foundation code calls this"). A GPU-cull-driven
indirect skin dispatch is a later rung, gated on that seam landing.

Why a skin cache and not VS skinning: the engine renders through shadow + G-buffer / VB passes,
and the VB material pass reconstructs attributes by vertex id — it needs skinned positions
addressable after rasterisation. Skinning once and caching costs 76 B per visible skinned vertex
(64 + 12); 1 M visible skinned vertices = **76 MB**. The ring has a byte budget; overflow is a coded
diagnostic and an LOD drop for the lowest-significance instances, **never** a silent path switch
(RESEARCH P16). The budget value is an owner value (§13.B).

## 5. Aether constructs — what exists, what is new

| Need | Construct | Status |
|---|---|---|
| Animation state logic per character | `machine … on entity` with `joins (mut anim: &mut AnimPlayer)`, `on E => T`, `after D => T` where `D` is any expression (`clip_secs(RUN)`), `enter { anim.play(RUN, fade: 0.15) }` | designed (`docs/aether-v2/MACHINES.md`), **unbuilt** — rung R5, itself behind R2 `state_chart!` and ballot AB-5 (`CAMPAIGN.md:56`) |
| Clip events into machines | `event AnimHit { victim: entity(AnimPlayer), … }` + `inbox (AnimHit)` — ② emits from the clip event table | existing `event` + the R5 router |
| Per-instance gameplay passes (significance, params) | `each` (v2) over `AnimInstance`-adjacent table components; `system` for anything with a `pool.scope` | `each` is v2/R3; `system` exists |
| The evaluator itself | `system` (hand-written Rust; `gpu` group for ⑦ → `.gpu()`, `CONSTRUCTS.md:142-144`) | exists |
| Blend trees | **not** a construct — Gaia data (§6), following Bevy's `.animgraph.ron` [S] and Kinemation blobs [D]; a live node graph is the shape that failed to ship (RESEARCH P18) | — |

**New constructs for v1: none.** One candidate is recorded, not adopted: a `clip` reference literal
resolved at compile time to the baked clip id (today `RUN` is a generated const from the Gaia
table's row names, `docs/gaia/LANGUAGE.md:131-132`, which already gives the compile-error-on-rename
property). Owner ballot §13.B.

## 6. Gaia asset profiles — bake and reflection-free load

Four asset kinds under `profile=data`:

| Kind | Authored as | Baked to | Load |
|---|---|---|---|
| `skeleton` | a Gaia document naming the foreign `.glb` (sidecar id, hard fail on absence — `docs/gaia/DECISIONS.md:125-127`) + import settings: root bone, per-bone LOD, socket names | `SkeletonBone` rows (§1.1) + one header | column blit; name hash → `u32` carrier once, cold (GN1, `DECISIONS.md:96-103`) |
| `clip` | a Gaia document: source `.glb` animation name, sample rate, compression preset, event / sync-marker tables (the Kinemation `SkeletonClipCompressionSettings` shape [D]; UE Animation Modifiers' build-time generation [D]) | one `ClipHeader` row + a byte blob | header blit + blob blit; **no per-field decode** |
| `animgraph` | Gaia nodes (`mix`, `add`, `mask`, `param`) with `@`-free lexical references to clips by name | the flat postorder program (§1.6) | blit |
| `mmdb` | a Gaia document listing clips + schema (feature channels, weights, sample rate) | feature columns + frame index + AABB tables | blit |

Bake happens in the tool with reflection behind the default-off feature; the shipped load path has
zero reflection (`DECISIONS.md:11-14`). Two things the format does not have today and the design
requests (§11 AK-6): an **asset-blob region** in `BOYKOSAV` (the precedent is v2's dense region,
`crates/boyko_serialize/src/format.rs:22-34`) gated by `layout_fingerprint` per blob kind, and the
blit into a resource-owned append-only column rather than an archetype (Gaia's `table` bakes rows
into ONE archetype column, `LANGUAGE.md:131-135` — that is the right shape for `SkeletonBone` rows
but not for a clip's byte stream). Bake contracts (eager, closed, `DECISIONS.md:56-79`): parent
index < child index; LOD monotone along parents; every sync marker present in every clip of a sync
group (RESEARCH P15 becomes a bake refusal); clip bone count == skeleton bone count; events sorted
by sample index.

## 7. Physics coupling

Order per Jolt's `PoweredRigTest` [S]: drive before the step, read back after. Concretely:

- **Hit boxes / attached colliders (L4)**: entities with `Transform + RigidBody + RigidBodyMass
  { inv_mass: 0 } + Kinematic + AnimSocket`, with the `Simulated` bit **OFF** and **no `ChildOf`**
  (critic NB1 — the first draft named the `Kinematic` tag as the gate; it is not). What
  `sync_transform_to_body` actually gates on is `!simulated && !is_dynamic_row(inv_mass)`
  (`crates/boyko_physics/src/scene_sync.rs:64-66`, the skip at `:93`; query
  `(&Transform, Mut<RigidBody>, &RigidBodyMass, IsEnabled<Simulated>)` at `:83`) — the `Kinematic`
  tag only marks the body one-sided in the contact response (`components.rs:66-74`). And it copies
  `Transform` — the **local** pose — into the body, so a hit box parented under the character
  would be double-composed by propagation; hit boxes are hierarchy roots, the same rule
  `debug_assert_dynamic_bodies_are_roots` enforces for dynamics (`scene_sync.rs:192-212`). ④
  writes `Transform` = bone model row ∘ character root `Transform`, and
  `RigidBody.{linear,angular}_velocity = Δpose / dt` (`components.rs:29-37` holds both) — Jolt
  `MoveKinematic` semantics done animation-side because the solver's kinematic motion is a
  deferral (`components.rs:66-74`). `sync_transform_to_body`
  (`crates/boyko_physics/src/plugin.rs:142-145`) then carries the pose into the gather. This
  **works on today's physics**. Kinematic bodies are read-only in the colored scatter (RESEARCH
  P22).
- **Ragdoll (L7)**: bodies are hierarchy roots (`scene_sync.rs:192-212`), one per ragdoll bone of a
  low-detail skeleton mapped to the animation skeleton (Jolt `SkeletonMapper` [D~]); joints enter
  the existing colorer with Box2D's same-pair rule (RESEARCH P23); a pose write resets warm start
  for the touched bodies (P21); ⑤ blends body poses into the bone rows by a per-bone weight (UE's
  physics blend weight [D]). Prerequisite: joints — absent today.
- **Sockets**: ⑤ writes the socket entity's `Transform`. A character that is itself parented
  reads a `GlobalTransform` propagated in Main (`crates/boyko_scene/src/propagation.rs:222`,
  exclusive, once per frame) — one frame old inside Fixed; recorded as a known lag, cured by
  keeping animated characters as roots, which the physics tripwire already requires of dynamics.
- **Cloth on the soft path**: needs a skinned constraint (joint indices, weights, max distance per
  particle) on `SoftBody` (`crates/boyko_physics/src/soft/component.rs:69`, which pins by
  `inv_mass == 0` only) plus a per-step "skin to the current pose" pass — Jolt's shape [D~]. Not in
  the v1 ladder — and **not buildable on `SoftBody` as it stands** (critic NB11): its particle
  state is `Vec<f32>` fields inside the component (`soft/component.rs:69-71`, `pub pos_x:
  Vec<f32>, …`), i.e. the `std::Vec` side store that `DENSE-COMPONENTS-PLAN.md:6` names as the
  SP4 root cause; a parallel skin-to-pose pass over it would be a parallel writer into a
  `Vec`-backed column. The rung's first prerequisite is therefore the soft body's own migration to
  a kernel column (dense or `ScratchColumn`), which belongs to the physics remediation, not to
  this campaign.

## 8. Determinism and the fixed step

- Clip time is an integer: `tick: u32` in Q16.16 clip-sample units at the clip's baked rate;
  `speed: i32` Q16.16; advance per substep = `speed × rate / 64` computed in integer arithmetic.
  No float time accumulates.
- Sampling by frame index; the fractional part is the only float that enters, derived from the
  integer phase.
- `boyko_math` is bit-deterministic (exact sqrt, no FMA — `docs/ARCHITECTURE.md:107`); the SoA8
  ops stay in that discipline.
- **D15 — lane-wise purity.** Every SoA8 op in ③ (decode scatter, nlerp, spring decay, affine
  multiply, the AABB fold) is lane-wise: there is **no horizontal (cross-lane) operation anywhere
  in the evaluator**, and a task writes only rows of its own groups. So an instance's palette bits
  are a function of its `LaneJob`, its clip bytes and its skeleton **only** — not of its lane
  index, its group, which other instances share the group, or how groups are cut into tasks. Lane
  assignment being deterministic (§1.3) is a nice property; it is **not** what G4 rests on, and the
  first draft's chain "deterministic lanes → evaluation order → bits" was the wrong argument
  (critic B4 — the layout does not reach the bits at all). Gate: G4b below moves one instance
  between lanes and demands identical bits.
- **The budget is timing-dependent in its INPUT, and the record must carry that input** (critic
  B4). §9's period / LOD / `compact_lanes()` order derive from a significance rank whose inputs
  (camera pose, visibility) are produced once per frame by Main
  (`crates/boyko_scene/src/propagation.rs:1-6`, `:220-222`), and Main interleaves 0–N substeps per
  frame — so *which* camera state substep `k` sees is a function of wall-clock frame timing.
  Nothing in the fixed step can make that a pure function of gameplay input. The design therefore
  makes the significance inputs an explicit **per-substep snapshot** — `AnimSignificanceInput
  { camera_pose, frustum, screen_scale }`, read by ② at exactly one point (its first
  instruction) and **part of the recorded input** of a replay, alongside the gameplay inputs. On
  replay ② reads the recorded snapshot instead of Main's. With that, the evaluated set, the
  periods, the LOD prefixes and the `compact_lanes()` order are pure functions of the record, and
  D15 makes the bits independent of everything else. This is the same move physics makes for
  `GlobalTransform` (§7, "one frame old inside Fixed" — an input, recorded as such).
- The budget (§9) ranks by that deterministic significance, never by measured time (UE's ms budget
  [D] would make the evaluated set timing-dependent **in the rank itself**, which no snapshot can
  record).

## 9. LOD and budgeting

- **Bone LOD** = the prefix `bones[0..B_l]` (§1.1). A lane group evaluates the max LOD prefix of
  its lanes; `compact_lanes()` groups lanes by LOD at the fixed point.
- **Update period** per instance ∈ {1, 2, 4} substeps (64 / 32 / 16 Hz), chosen each substep by
  significance rank (screen size × visibility × owner-set class, all read from the §8
  `AnimSignificanceInput` snapshot) under a **count** budget `max_full_evaluations` per substep;
  skipped instances keep their palette pair, and the compute pass lerps over the longer interval
  via `(period, phase)` (§1.7). **Latency follows the period**: an instance at period `p` shows a
  pose up to `p` substeps behind the fixed-step truth — 15.6 / 31.25 / **62.5 ms** for p = 1 / 2 /
  4 (critic NB9; the owner ballot §13.B.1 states the full range). UE's floors — 10 Hz tick,
  16 interpolated components [D] — are the reference points; the values are the owner's (§13.B).
- The gate is a count: evaluations executed == budget on a fixture (the counts-not-clocks rule
  `docs/gaia/DECISIONS.md:282-289`).
- Deformation is dispatched only for visible, LOD-selected instances (§4).

## 10. Migration path from the loader's refusal

| Rung | Change | Gate |
|---|---|---|
| **M0** | `GlbMeshLoader::decode_skinned` (by name, beside `decode_static_pose`, `glb.rs:1027`): reads `JOINTS_0` / `WEIGHTS_0` into the side stream (§1.8), `inverseBindMatrices`, the `joints` node list and `skeleton` root into `SkeletonBone` rows; skinned nodes stay at identity (`glb.rs:777-782`) | rest pose at frame 0 through the full path == `decode_static_pose` positions bit-for-bit (the skinned mesh at its bind pose is the static mesh) |
| **M1** | `animations` decode: samplers (LINEAR / STEP / CUBICSPLINE) resampled to the uniform rate at bake into the §1.2 codec — `translation` / `rotation` / `scale` channels into bone tracks, `weights` channels into the **scalar track stream** (§1.2; [D] glTF 2.0 spec, the four `path` values); root-motion track split | codec round trip: per-clip max error AND exceedance fraction printed (RESEARCH P3), for bone AND scalar tracks; a red fixture with one bone past tolerance |
| **M2** | morph `targets` (`glb.rs:834-840`) into a delta array + per-instance weights (L6), the weights **animated from the scalar stream** through `LaneJob.scalars` (§1.4) | delta-at-zero-weight == neutral shape; a `weights`-animated fixture reaches the GPU weight buffer through ③ |
| **M3** | flip the default `AssetLoader::decode` for `.glb` from refusal to skinned decode **only once L0 renders skins** — until then the refusal stands (RESEARCH P28: "it loaded fine" must not go green early) | the golden harness renders a skinned frame |

## 11. Kernel and crate requests born from the design (each gets its own pass)

| # | Item | Why | Zero when unused |
|---|---|---|---|
| AK-1 | `boyko_math`: `Quat::{dot, nlerp}` and the SoA8 types (`F32x8`, `Vec3Soa8`, `QuatSoa8`, `Affine3Soa8`), no-FMA | no quaternion blend primitive exists (`quat.rs:140` is `normalize`) | yes |
| AK-2 | kernel: ratify `ScratchColumn` as the resource-owned column for *durable* data too, or add a differently-named alias — the pose bank is scratch, the inertialization bank is not | naming honesty; `VmColumn` is `pub(crate)` (`crates/boyko_ecs/src/ecs/memory/vm_column.rs`) | yes |
| AK-3 | kernel: KE9 `par_for_each_chunk_entities` for ② if measured necessary | already in `KERNEL-BACKLOG.md` | yes |
| AK-4 | physics: joints in the colored graph with the same-pair rule; `reset_warm_start(body)` | L7 | opt-in stage |
| AK-5 | physics: a ground query (ray / shape cast); interim = `sample_sdf` gradient | L5 | yes |
| AK-6 | `boyko_serialize` / Gaia: asset-blob region (format v3) + blit into append-only asset columns | §6 | v2 files unchanged |
| AK-7 | render: skinned side stream, palette SSBO ring, skin-cache ring, `skin_lbs` leaf, manifest row; the cull reads `AnimBounds` for a skinned instance in place of `gMeshBounds[mesh_id]` (`frustum.rs:152`, `:170`) | §1.7, §1.8, §4 | no skinned mesh → no dispatch; static instances keep the per-mesh row |
| AK-8 | Aether: R2 `state_chart!` and R5 `machine … on entity` land before L3 | §5 | — |
| **AK-9** | threadpool: land the KE16 pool (`a1f+b1+wc+c0`, Step App taken at `KE16-RESULTS.md:1668`; branch `feat/threadpool-ke16`, **not an ancestor of this branch**) — the worker-spawned `scope` route is the serial floor without it (`:292-300`) | §2.1: ③'s fan-out; **G2 is red until this lands** | — (the pool is shared by physics and `par_iter`) |

## 12. Gates (all red-first)

- **G1 oracle**: `skin_lbs::<f32>` == scalar CPU skinning byte-for-byte on a fixture mesh; the
  `.spv` byte-equal under the frozen recipe.
- **G2 serial vs parallel**: the pose bank after ③ with `W ∈ {1, 2, N}` equals the serial reference
  byte-for-byte, with **per-worker touch counts reported** so a silently-serial run is red (the R6
  gate shape, `docs/aether-v2/CAMPAIGN.md:57`). **Known red on this branch until AK-9 lands**
  (§2.1): the touch counts will read one worker, which is the correct verdict for the shipped pool,
  not a defect in ③.
- **G3 codec**: per-clip error distribution (max + exceedance fraction), cold-cache decode bench
  that streams ≥ L2 of clip data between samples (RESEARCH P2).
- **G4 determinism**, in two parts (critic B4): **G4a** — two runs of one recorded input, where the
  record includes the per-substep `AnimSignificanceInput` snapshots (§8), → bit-identical palette
  AND identical evaluated set / period / LOD per instance per substep; a counter-test with a
  different op order that differs. **G4b — lane independence (D15)**: one instance evaluated in
  lane 0 of a full group, in lane 7 of a group whose other lanes run a different program (the
  masked union of §1.6), and alone in a group → three bit-identical palettes. G4b is the gate that
  makes G4a's claim structural rather than incidental.
- **G5 budget**: evaluations executed == `max_full_evaluations` on a crowd fixture (count, not clock).
- **G6 render**: a skinned frame in the golden byte-identity harness; `decode` for `.glb` flips only
  with it (M3).
- **G7 ordering**: a fixture that registers ③ outside `FixedSet::Gameplay` and observes the
  one-substep lag (`sets.rs:31-33`) — the failure the seam exists for, made visible.

## 13. Decisions

### 13.A PERF / ARCHITECTURE — taken here, with the numbers

| # | Decision | The number that decides it |
|---|---|---|
| D1 | Poses are **lane-group-major**, **8 instances per SIMD group**, not 4 joints per group | AVX2 = 8 lanes; zero intra-group dependency in local→model; a 64-bone lane group is ONE contiguous 24 576 B run < 32 KB L1d, L1-resident for B ≤ 85; bone-major's `G × 384 B` stride is a 4096-multiple at every 256 instances and maps every row of a task to six L1 sets (§1.3) |
| D2 | In-place local→model, 12-field rows from the start | avoids a second 24 KB buffer that would push the working set past L1d |
| D3 | Uniform sampling, whole-pose decode, no sampling context | ACL: decode independent of direction/rate; ozz's context is per-instance mutable state (P9) |
| D4 | In-house codec on ACL's design — 16-frame segments and per-segment range reduction **[S]** (`segment_context.h`: `ideal_num_samples = 16`, `max_num_samples = 31`; `normalize.transform.h`: `normalize_segment_streams`), constant-track compaction and time-then-track sort **[D]** (algorithm page); the four-stream split, "2–5 lines per frame" and the Paragon 20.79:1 are **[B]** and are recorded, not relied on — plus a scalar track stream; fixed 16-bit tracks in v1, variable bit rate in v2 | the v1 subset keeps the layout so v2 is a codec change, not a format change; the in-house size ratio is a G3 measurement, not the [B] number |
| D5 | Task = 1..k lane groups, `groups_per_task = clamp(ceil(G/6W), 1, floor(1 ms / t_group))` | KE16: ≥10 µs bodies healthy at W..64W; 1 µs bodies serial under 1.98.1 — **on the KE16 arm; the shipped route is the serial floor at every body size until AK-9** |
| D6 | Evaluate at the fixed rate, lerp on the GPU via a double-buffered palette with `(period, phase)` | 64 evaluations/s independent of frame rate; the existing `GpuTransform3D` pipeline; up to one substep of latency (owner-visible, §13.B) |
| D7 | Skin cache (compute pre-pass) rather than VS skinning | the VB path needs post-raster attribute access; 76 B per visible skinned vertex; overflow = diagnostic + LOD, never a path switch |
| D8 | Palette = 48-B 3×4 rows (`InstanceModelCol` layout); DQS as a measured A/B later | one layout for instances and bones; 32-B DQ rows are a 1.5× upload saving to be measured, not assumed |
| D9 | Skin data as a 16-B side stream; the 64-B `Vertex` untouched | the deciding fact is in-tree: `VERTEX_STRIDE == 64` pinned (`mesh.rs:103-104`) and every existing pass reads that record unchanged. Rukhanka's "36 → 20 B" is **[B]** (RESEARCH §2.8) — an estimate of the bandwidth side, recorded, not the reason (critic NB5) |
| D10 | Blend trees baked to a flat postorder program; state machines in Aether | the live node-graph runtime is the one that did not ship (P18); Bevy's flattening [S] |
| D11 | Integer clip time (Q16.16), count-based budget over a **recorded per-substep significance snapshot** | bit-identical replay; a ms budget is timing-dependent in its rank, which no snapshot can record |
| D12 | Motion database stores features + `(clip, frame)` only, feature-major | 12 MB vs 100–590 MB [B]; 8 frames per op in the search loop |
| D13 | Bones are never entities at runtime (no "exposed skeleton"); sockets are a component reading a bone row | the propagation walker is exclusive and per-row (`propagation.rs:1-50`); `Children` order is unspecified (P30) |
| D14 | Inertialization state is a second bank with the `BoneSoa8` row (12 fields: translation + rotation offsets and velocities, no scale); the transition clock is per instance in `LaneJob` | 384 B = 6 whole lines per bone group (the 13-field / 416-B draft straddled lines, critic NB8); scale offsets are the first thing UE's node also drops in practice — an **estimate** of need, revisit with a fixture |
| D15 | **Lane-wise purity**: no cross-lane op in ③; an instance's bits depend on its `LaneJob`, clip and skeleton only | G4b; makes lane layout, group membership and task cut irrelevant to determinism (§8) |
| D16 | Per-instance animated AABB from baked bone spheres, folded in ③; CPU-side visibility verdict for the skin dispatch in v1 | `dispatch_indirect` is a no-op seam (`encoder.rs:370`); a rest-pose box pops (Kinemation's "Accurately computed at runtime" [D]) |

### 13.B VALUES / SCOPE — to the owner

1. **Latency**: accept up to one fixed substep (15.6 ms at 64 Hz) of animation latency for
   full-rate instances, as physics bodies already have — **and up to `period` substeps for a
   budgeted one: 31.25 ms at period 2, 62.5 ms at period 4** (§9; critic NB9 — the first ballot
   understated the crowd tier) — in exchange for one evaluation pipeline (D6). The period floor
   (item 6) is therefore also a latency ballot. The alternative — a second, render-rate evaluation
   for hero characters — is a scope addition, not a redesign.
2. **Codec ambition**: ship the v1 fixed-16-bit subset first (D4) or go straight to variable bit
   rate. The v1 subset is smaller work and a format-compatible stepping stone; the owner sets the
   size target that decides whether v2 is needed at all.
3. **Ragdoll in this campaign or its own**: L7 needs physics joints (AK-4), which is a physics
   campaign in itself.
4. **Motion matching / LMM / ML deformer**: L9–L11 each need data (a mocap set, a trained model)
   the engine does not have; plan them now or leave the ladder at L8.
5. **Bone-index width**: u16 (65 535 bones, 16-B side stream) as designed, or u8 (256 bones, 8-B
   stream — Bevy's `MAX_JOINTS = 256` [S]).
6. **Budget values**: `max_full_evaluations`, the update-period floor (16 Hz here vs UE's 10 Hz),
   the skin-cache byte budget.
7. **Morph targets (faces) priority** relative to the skeletal ladder.
8. **The `clip` literal**: whether Aether gains one construct for clip references or stays at the
   generated row consts (§5).
9. **Keep the `.glb` default `decode` refusing until skins render** (M3) — recommended yes.

## 14. Unverified / open

- The brief's "<10 µs is counter-productive" is consistent with the KE16 record but not a
  sentence in it; the Step App numbers for the shipped unconditional code are owed
  (`KE16-RESULTS.md:15`). D5's constants are to be re-measured on the animation body itself.
- Every per-instance cost in §2.1 is an **estimate** anchored on ACL's iPad numbers; this box has
  not been measured.
- Whether `compact_lanes()` needs to run at all (LOD / program churn rate) is unmeasured; so is
  the masked-union overhead (§1.6) between compactions.
- The scale-offset omission in D14 is a judgement, not a measurement.
- UE header-level names quoted via the survey are [B]-sourced (RESEARCH P32).
- Whether an exclusive-system placement of ③ would put its `scope` on the dispatcher route (not
  serial in the KE16 record, `KE16-RESULTS.md:1668-1697`) is unmeasured and not claimed (§2.1).
- The ~7.9 GB/s / ~4 GB/s traffic figures of §1.3 are size arithmetic, not a measurement; which
  of the two a 10 000-instance crowd actually sees depends on L3 residency between ③ and ⑥.
- The `SkeletonBone.radius` fold gives a conservative box (sphere union); how much tighter a
  per-bone OBB would be, and whether it matters for the cull, is unmeasured.

## 15. Critique log — pass 1 (2026-09-10)

Each finding of the `architecture-critic`'s first pass, with what this revision did about it.
"Fixed" means the text above now says something different; "refuted" means the finding's evidence
was re-opened and does not support it; "partly" means part of the chain was wrong and the rest was
fixed. Every `file:line` here was re-opened at commit `128233be`.

| # | Finding (short) | Action |
|---|---|---|
| B1 | ③'s fan-out uses the worker-spawned `scope`, which is the serial floor on this branch; KE16 is not an ancestor and not a prerequisite | **Fixed.** Verified: `git merge-base --is-ancestor feat/threadpool-ke16 HEAD` → exit 1; `grep -rn KE16 crates/boyko_threadpool/src` → 0; `thread_pool.rs:424-430` unchanged; `KE16-RESULTS.md:292-300`. §0 and §2.1 now state the route is serial here; **AK-9** added (§11); G2 marked red-until-AK-9 (§12); D5 annotated. No interim is claimed (the exclusive-system/dispatcher-route idea is listed as unmeasured in §14) |
| B2 | Bone-major is the wrong major order: `G × 384 B` stride is a 4096-multiple at every 256 instances → set conflicts; 100 bones exceed L1 by capacity | **Fixed.** Layout is now `bank[lane_group][bone]` (§1.3, D1) with the set arithmetic written out and the P2-CACHE-FIX precedent cited (`component_pool.rs:292-298`, `constants.rs:178-196`); the L1 claim is restated as a bound (B ≤ 85) and scoped to the pose bank's three passes; the inertialization bank is counted (pair = L2) |
| B3 | ② cannot be a `dense_iter` over table components; ③ has no gather into lane-indexed scratch; divergent programs in one group are unspecified | **Fixed.** `query.rs:434-436` confirms the const reject. §1.4 adds the `LaneJob` column refilled by ② (the `bodies_build` precedent, `resources.rs:3159-3163`); ② is relabelled a mixed `Query::iter` with the per-row gather (`DENSE-COMPONENTS-PLAN.md:11`); ③ reads `LaneJob` only; §1.6 specifies the masked-union execution for divergent programs and `compact_lanes()` sorts by (LOD, program) |
| B4 | G4 contradicts §9: significance inputs come from Main, which frames interleave timing-dependently; lane layout → order → bits | **Partly refuted, rest fixed.** Refuted: the layout does not reach the bits — every SoA8 op is lane-wise, there is no horizontal op, and a task writes only its own rows, so an instance's bits are independent of lane, group and task cut (now **D15**, gated by **G4b**). The first draft's own argument ("deterministic lanes → order → bits") was wrong and is withdrawn. Fixed: the evaluated set / period / LOD ARE timing-dependent in their input; §8 makes `AnimSignificanceInput` a per-substep snapshot read at one point in ② and **part of the recorded input**, so G4a is passable as a replay of the record (`propagation.rs:1-6`, `:220-222` re-opened) |
| NB1 | Hit-box spec names the `Kinematic` tag as the sync gate; the gate is `!simulated && !is_dynamic_row(inv_mass)`; `Transform` is read as world → must be a root | **Fixed** (§7): spec now `RigidBodyMass { inv_mass: 0 }` + `Simulated` OFF + no `ChildOf`; `scene_sync.rs:64-66`, `:83`, `:93`, `:192-212` cited; velocity fields confirmed at `components.rs:29-37` |
| NB2 | Codec has no scalar / morph-weight track; glTF `weights` is a first-class channel path | **Fixed** (§1.2 scalar track stream; `LaneJob.scalars`; M1/M2 updated). Source opened: glTF 2.0 `Specification.adoc`, `path` ∈ {translation, rotation, scale, weights} [D] |
| NB3 | No animated bounds; visibility CPU/GPU undecided; rest-pose culling pops | **Fixed**: `SkeletonBone.radius` (§1.1, at the same 96 B — `name_hash` moved to a cold column), the AABB fold in ③, `AnimBounds` (§1.4), the cull read in AK-7, CPU-side verdict in v1 because `dispatch_indirect` is a no-op seam (`encoder.rs:362-372`); D16. `frustum.rs:152`, `:170`, `mesh_geometry_table.rs:178` re-opened |
| NB4 | `plugin.rs:143-148` cited without a crate path beside a `boyko_app/src/plugins.rs` citation | **Fixed** (§2 diagram, §7): `crates/boyko_physics/src/plugin.rs:142-145`; RESEARCH L4 row likewise |
| NB5 | D4 and D9 rest on [B] sources the document says to record, not rely on | **Fixed by upgrading and by relabelling.** D4's segment size and per-segment range reduction are now [S] (`acl/.../impl/segment_context.h`: `ideal_num_samples = 16`, `max_num_samples = 31`; `impl/normalize.transform.h`: `normalize_segment_streams`, `extract_segment_bone_ranges` — both opened); constant compaction and sort order [D] (algorithm page, opened — it indeed says nothing about segments, as the critic found); four streams, "2–5 lines" and 20.79:1 stay [B] and are marked recorded-not-relied-on. D9 now names the in-tree `VERTEX_STRIDE` pin as the reason and Rukhanka as a [B] estimate |
| NB6 | Bandwidth low by ≥ 2× | **Fixed** (§1.3): itemised per-substep traffic (bank write-back, ⑥ re-read, palette write, upload read) → ~7.9 GB/s, ~4 GB/s if L3-resident; labelled an estimate; §14 notes the L3 dependence |
| NB7 | `mask: u16` cannot be a per-bone mask past 16 bones | **Fixed** (§1.1): a per-skeleton `u64`-word mask table; every `mask: u16` is an index into it |
| NB8 | `InertSoa8` at 416 B straddles lines | **Fixed** (§1.5, D14): 12 fields = `BoneSoa8`, 384 B; `t_elapsed` is per instance and moved to `LaneJob.inert_t` |
| NB9 | Ballot 13.B.1 understates latency: period 4 = 62.5 ms | **Fixed** (§9, §13.B.1) |
| NB10 | `DENSE-COMPONENTS-PLAN.md:58` should be `:59` | **Fixed** (§1.3); confirmed `:58` is (c), `:59` is (d) |
| NB11 | Cloth rung inherits `SoftBody`'s `Vec<f32>` side store | **Fixed** (§7): named as a Principle-0 violation with `soft/component.rs:69-71` and `DENSE-COMPONENTS-PLAN.md:6`; the rung's first prerequisite is the soft body's own migration |

Nothing was left open: every finding is either fixed above or refuted with the evidence stated in
this table.
