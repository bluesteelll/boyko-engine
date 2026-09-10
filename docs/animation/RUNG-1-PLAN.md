# Animation rung 1 — the implementation plan

> Status: architect's plan, 2026-09-10, written against
> [`ANIMATION-DESIGN-SPACE.md`](ANIMATION-DESIGN-SPACE.md) (ratified, critic pass 1 answered) and
> [`ANIMATION-RESEARCH.md`](ANIMATION-RESEARCH.md). **The design's decisions are not re-opened
> here.** What this document adds is the rung-1 *subset*, the exact byte layouts, the plug-in
> points, and the forks the design left to the implementation.
>
> **Revision state: critic pass 1 answered, verifier pass 2 answered (2026-09-10).** Pass 2 found
> one pass-1 repair that did not hold at HEAD — the `precise` contraction pin of §5.3 named a
> printer capability the printer documents as not yet existing — and it is REWRITTEN in §5.3 (the
> bit-for-bit position claim is now conditional on the eDSL lane's Pass 2, with a ULP-bounded
> golden until then). Five smaller items (the `register` offset convention, the hook resource
> surface, the `frame_count = 65536` bound, the idle-lane pointer arithmetic, two anchors) are
> corrected in place. See the pass-2 critique log at the end.
>
> Every `file:line` below was **re-opened at HEAD `ed0bed45`** on `feat/multi-paradigm-render`.
> Several drifted from the design document's `128233be` citations and are corrected in place
> (notably `query.rs:434-436` → `:436-446`). This directory is **not** in
> `tests/internal_docs_anchors.rs`'s `GATED_DOCS` (`tests/internal_docs_anchors.rs:231`), so no
> anchor here is machine-checked — treat a line number as "true on 2026-09-10".
>
> Numbers are either measured elsewhere and cited, or **labelled estimates**. §12 separates the
> PERF/ARCHITECTURE forks decided here from the VALUES/SCOPE ballots that go to the owner.

---

## 0. What rung 1 is, and what it is not

Rung 1 is the smallest step that puts a **skinned, animated character on the screen through the
engine's own storage**, with every artifact it creates already in its final shape.

**In:** skeleton asset · clip asset (fixed-16-bit codec v1) · `.glb` skin + animation decode ·
the SoA-8 pose bank · single-clip playback (advance → sample → local→model) · the GPU palette
interp + LBS skin cache through the eDSL · one golden frame.

**Out, named so nothing is silently missing:** blending / masks / layers (L1) · inertialization
(L2) · Aether machines (L3) · root motion + kinematic drive (L4) · IK (L5) · **morph targets
(L6 — after skeletal, working assumption)** · ragdoll (L7) · motion matching (L9) · ML deformer
(L11) · LOD prefixes · the significance budget · the animated AABB and the skinned cull ·
`compact_lanes()` · the blend program · the inertialization bank · the scalar track stream ·
**growth of a skeleton's bank region past its config-fixed lane cap** (§3.1.1: the cap is set at
skeleton registration; exhaustion is a loud refusal, and moving a region is a later rung) · **the
per-vertex previous-position ring for motion vectors** (§5.5: it arrives with the shader that reads
it, not before) · **an AVX2 intrinsic path for the SoA-8 ops** (§4.2: rung 1 is `[f32; 8]` arrays,
one implementation).

**Working assumptions carried from `DESIGN-SPACE` §13.B (NOT ruled by the owner).** Each is
flagged at the step where it bears:

| # | Assumption | Steps it binds |
|---|---|---|
| WA-1 | The `.glb` default `decode` keeps **refusing** skins until a golden frame passes | S3, S13 |
| WA-2 | **No new Aether construct** | none — rung 1 registers plain `system`s (S10) |
| WA-3 | Codec v1 = **fixed 16-bit subset** | S5, S6 |
| WA-4 | Bone index = **u16** | S4, S7 |
| WA-5 | Ragdoll / motion matching / ML deformer **out** | none |
| WA-6 | Morph targets **after** skeletal | S3 (rung 1 *refuses* `targets`, it does not tolerate them) |

### 0.1 Gate G2 is red by construction — what rung 1 runs instead

`DESIGN-SPACE` §2.1 / AK-9: `pool.scope` called from inside a system takes the **worker-spawned**
route (`crates/boyko_threadpool/src/thread_pool.rs:424`), which on this branch **is** the serial
floor. Confirmed unchanged at HEAD: `thread_pool.rs:424` is the plain forward to `inner.scope`;
`try_with_active_pool` (`crates/boyko_threadpool/src/tls.rs:148-173`) is the ambient-discovery
surface a system would use, and it discovers exactly that route.

**Rung 1 therefore evaluates the pose bank SERIALLY, by an explicit `for` loop — not by a
`pool.scope` that would silently be the same thing.** A `scope` that happens to serialise reads
green while measuring nothing; a `for` loop is honest about the same wall clock. Concretely:

```rust
// crates/boyko_anim/src/evaluate.rs
for group in 0..bank.group_count() {
    evaluate_group(pose_view, &jobs[..], &skel, &clips, group);   // <- the fan-out unit
}
```

`evaluate_group` is a **free function over `Copy`/shared arguments only** — `ScratchSolveView`
(`crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:127-137`, `Copy + Send + Sync`), a
`&[LaneJob]`, two read-only asset views. No `&mut [T]` crosses its signature. When AK-9 lands, the
change is the dispatch line — **and that line must not be "one task per group".** The KE16 record
puts the worker route's healthy region at bodies of **≥ 10 µs**
(`D:/wt/threadpool/docs/threadpool/KE16-RESULTS.md:288-290`, a worktree file, not in this
checkout), and a lane group is only above that floor at ≥ ~64 bones on this plan's own
**estimate** (§4.2: 25–60 µs at 64 bones — a 16- or 32-bone group is a quarter to a half of that,
i.e. at or under the floor). The dispatch line therefore takes the design's D5 rule as written:
`groups_per_task = clamp(ceil(G / (6·W)), 1, floor(1 ms / t_group))`
(`ANIMATION-DESIGN-SPACE.md:355-359`), with `t_group` from B1, so a task is `k` consecutive groups
and the body clears the floor at every bone count.

G2 is replaced for rung 1 by **G2-SHAPE**, which tests the *content* of G2 without touching the
pool (§11, tests T14/T15): a trybuild compile-fail proving the whole-buffer reborrow is
un-typeable from the group view, plus a two-thread `std::thread::scope` run over disjoint group
ranges asserting byte-identity with the serial run. That test's header must state, in its first
paragraph, that **it does not measure the engine pool** — otherwise it becomes the next entry in
this repository's catalogue of gates that pass by measuring something else.

---

## 1. Crate layout and layering

**One new crate: `crates/boyko_anim`** (package `boyko-anim`, `[lib] name = "boyko_anim"` — the
workspace's hyphen/underscore split, `crates/boyko_render/Cargo.toml:1-16` is the pattern).

```
boyko_anim  →  boyko_ecs, boyko_macros, boyko_math, boyko_scene,
               boyko_serialize, boyko_threadpool, boyko_log, bytemuck
boyko_render → boyko_anim                    (NEW EDGE — acyclic)
```

**Why this direction.** The palette column, the skin-cache ring and the dispatch live in
`boyko_render` and must name `SkeletonAsset` / `PaletteRow`; animation names no render type.
`boyko_render` is already the crate licensed to name both the ECS and the RHI
(`crates/boyko_render/Cargo.toml:7-13`). The `.glb` **parse** stays where the JSON reader is
(`crates/boyko_render/src/loaders/glb.rs`) and the **bake** — which needs both the parse and
`boyko_anim`'s writers — therefore also lives in `boyko_render`. `boyko_anim` never sees a `.glb`.

**Why not fold it into `boyko_scene`.** `boyko_scene` is deliberately the bottom of the spatial
dependency DAG (`crates/boyko_scene/src/sets.rs:15-17`) so producers and consumers can name
`FixedSet` without a cycle. Putting a pose bank, a codec and a threadpool dependency there widens
the graph's floor. Owner ballot V4 (§12.B).

**Registration chores a new member requires** (each is a gate that reds otherwise):

1. `Cargo.toml:2` (`members`) **and** `Cargo.toml:13` (`default-members`) — both. The
   2026-07-23 audit found the whole CI vacuum-green because a member was absent from
   `default-members` (the comment at `Cargo.toml:4-12` records it).
2. `crates/boyko_diag/src/sample.rs:150` (`ENGINE_PACKAGES`) — `tests/engine_packages_census.rs`
   fails a member that is in neither `ENGINE_PACKAGES` nor its own `USER_PACKAGES` list
   (`tests/engine_packages_census.rs:135-146`).
3. `[lints] workspace = true` in the new manifest — `Cargo.toml:15-38`; without it the crate is
   outside the `disallowed_types = "deny"` hot-path ban.

---

## 2. Storage: ids, staggers, and the tick-page decision

### 2.1 The synthetic id band

`ScratchColumn::new` reads its element `Layout` from the global registry by `ComponentId`
(`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:62-83`), so every
non-`#[derive(Component)]` column needs a registered id. Physics owns a descending band from
`MAX_COMPONENTS - 1 = 511` down to `478`
(`crates/boyko_physics/src/scratch_ids.rs:55`, `:62-63`, `:100-111`).

**Animation takes `460..=477`** (18 ids reserved; **10 used in rung 1**), declared in
`crates/boyko_anim/src/scratch_ids.rs` mirroring the physics module verbatim
(`crates/boyko_physics/src/scratch_ids.rs:127-132` is the `register_*_layouts` shape — one
`register_layout` per column, so **every column §6.3 and §3.1.1 push into has a row here**; a
column without an id cannot be constructed, `scratch_column.rs:62-69`):

| id | column | element | stride | stagger `(id%64)*64` |
|---|---|---|---|---|
| 477 | skeleton bone table | `SkeletonBone` | 96 B | 1856 |
| 476 | skeleton bone names (cold) | `BoneNameHash(u32)` | 4 B | 1792 |
| 475 | clip header table | `ClipHeader` | 64 B | 1728 |
| 474 | clip byte blobs | `ClipBlobChunk` | **64 B** | 1664 |
| 473 | pose bank | `BoneSoa8` | 384 B | 1600 |
| 472 | lane jobs | `LaneJob` | 16 B | 1536 |
| 471 | palette staging | `BonePairPacked` | 96 B | 1472 |
| 470 | skeleton asset headers (on-disk record, §3.1) | `SkeletonAsset` | 16 B | 1408 |
| 469 | bank descriptors (runtime record, §3.1.1) | `BankDesc` | 32 B | 1344 |
| 468 | lane free stacks (§3.1.1) | `FreeLane(u32)` | 4 B | 1280 |
| 460..467 | **reserved** — inertialization bank (L2), MMDB columns (L9) | — | — | — |

`pool_base_stagger(id) = (id % 64) * CACHE_LINE_SIZE` with `POOL_STAGGER_LINES = 64` and
`CACHE_LINE_SIZE = 64` (`crates/boyko_ecs/src/ecs/constants.rs:166`, `:197`, `:11`), so
`477 % 64 = 29 → 1856` and each lower id steps down one line. The ten residues `20..=29` are
pairwise distinct and disjoint from physics' `478..=511 → 30..=63`, so all ten land in distinct
L1/L2 sets — the P2-CACHE-FIX property the ~40 % rigid-solver regression paid for
(`crates/boyko_ecs/src/ecs/constants.rs:178-195`,
`crates/boyko_ecs/src/ecs/memory/component_pool.rs:292-299`). *(Critique pass 1, NB1: the first
draft printed `3520 … 3136`, i.e. `55·64 … 49·64`, which the formula does not produce; the
conclusion — distinct residues — was true, the numbers were not.)*

⚠ **Two columns of the SAME element type must take DIFFERENT ids.** `register_layout` is a
same-type silent no-op, so re-using one id for two columns yields one `component_id`, hence one
stagger, hence exactly the conflict storm the stagger exists to remove. Rung 1 has no duplicate
element type; test T13 (§11) makes the property mechanical for the rungs that will.

`align_of::<BoneSoa8>() == 64 ≤ 4096`, so `ComponentPool::new`'s alignment assert
(`crates/boyko_ecs/src/ecs/memory/component_pool.rs:285-290`) passes. There is **no** requirement
that the stride divide `COMMIT_GRANULE` — that constraint belongs to `VmColumn`
(`crates/boyko_ecs/src/ecs/memory/vm_column.rs:145`), not to `ComponentPool`, whose data region is
`align_up(reserve_rows × stride, G)` (`crates/boyko_ecs/src/ecs/constants.rs:205-212`). Re-opened
specifically because 384 and 96 do not divide 65536.

### 2.2 DECIDED FORK — the clip blob is chunked to 64 B, not `ScratchColumn<u8>`

Every `ComponentPool` reserves `[pad | data | added_ticks | changed_ticks]` (the reservation
layout, `crates/boyko_ecs/src/ecs/constants.rs:200-212`) and **`grow_rows` commits all three
sub-regions in lockstep** (`crates/boyko_ecs/src/ecs/memory/component_pool.rs:494`, via
`commit_subregion` at `:453` — "data + both tick sub-regions in lockstep", `:470`), at 4 B per
tick region per row = **8 B of tick pages per row**, while `ScratchColumn`'s own header states "There is NO
change-detection tick use — this is raw scratch"
(`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:14-15`).

Amplification is `8/stride`:

| candidate backing | stride | tick overhead | tick floor for a 76 KB clip |
|---|---|---|---|
| `ScratchColumn<u8>` | 1 B | **9.0×** | **608 KB** |
| `ScratchColumn<ClipBlobChunk>` where `ClipBlobChunk = [u8; 64]` | 64 B | 1.125× | **9.5 KB** |

**Decision: `ClipBlobChunk = [u8; 64]`, `#[repr(C, align(64))]`.** A clip's byte stream occupies
`ceil(len/64)` consecutive chunks starting at a chunk boundary; `ClipHeader` records
`chunk_base: u32`. The tick floor drops **64×** for one type parameter, and every clip region
becomes cache-line aligned — which the whole-pose contiguous read (§4.3) wanted regardless.

Rejected alternative: the kernel "tick sub-regions reserved-uncommitted" feature
(`docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:27`) is the *correct* cure for this whole class and
would also help the pose bank and `LaneJob`. It is a kernel rung, not this campaign's, and rung 1
must not block on it. The chunking takes 87.5 % of the win today with zero kernel change, and it
composes with the kernel fix rather than competing with it.

Residual amplification rung 1 accepts, stated: pose bank 2.1 %, palette 8.3 %, `LaneJob` 50 %
(16-B stride). `LaneJob`'s absolute floor is small — at 10 000 instances it is 160 KB of data and
80 KB of tick — so it is not worth widening the row to hide.

---

## 3. Data structures — in full

All layouts are pinned by `const _: () = assert!(...)` on `size_of`, `align_of` and every
`offset_of`, the discipline `InstanceModelCol` (`crates/boyko_render/src/instance_model.rs:73-74`)
and `GpuTransform3D` (`crates/boyko_render/src/gpu_transform3d.rs:106-115`) already use. A silent
layout change must fail the build, not corrupt a pose.

### 3.1 Skeleton — `crates/boyko_anim/src/skeleton.rs`

```rust
/// One bone row. Bones are sorted PARENT-FIRST (a bone's parent index < its own) — a BAKE
/// CONTRACT, asserted at bake and debug_asserted at load, never re-checked in the hot loop.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SkeletonBone {          // 96 B, align 4 — two rows == three cache lines exactly
    pub parent:   u16,             // @0   u16::MAX == root
    pub lod:      u8,              // @2   rung 1 bakes 0 for every bone
    pub flags:    u8,              // @3   bit0 = skins at least one vertex
    pub radius:   f32,             // @4   bind-space bounding-sphere radius of the vertices this
                                   //      bone influences (0 if it skins nothing). BAKED AND GATED
                                   //      in rung 1, CONSUMED at the AABB rung — see §13.
    pub rest:     [f32; 10],       // @8   t3 | q4 (xyzw) | s3 — the LOCAL rest pose
    pub inv_bind: [f32; 12],       // @48  3x4 ROW-MAJOR, the InstanceModelCol convention
                                   //      (crates/boyko_render/src/instance_model.rs:56-62)
}
const _: () = assert!(size_of::<SkeletonBone>() == 96 && align_of::<SkeletonBone>() == 4);

/// COLD column, parallel by index. Bake/tooling only; the runtime binds by INDEX (RESEARCH P4).
/// Rung 1 DOES consume it — at bake, to verify a clip's channel targets the bone it claims (§6.4).
#[repr(transparent)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BoneNameHash(pub u32);  // FNV-1a 32 of the glTF node name

/// One skeleton asset header row — the ON-DISK, FINGERPRINTED record (`"skel.header"`, §6.2).
/// Append-only for the process lifetime (RESEARCH P27,
/// crates/boyko_ecs/src/ecs/core/asset/handle.rs:29-42). Carries ONLY what a bake can know:
/// nothing about how many instances a world will spawn (that is `BankDesc`, §3.1.1).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SkeletonAsset {         // 16 B
    pub name_hash:   u32,          // @0   FNV-1a 32 of the asset's stable name
    pub bone_base:   u32,          // @4   first row in the bone table; 0 in the file, REBASED at load
    pub bone_count:  u16,          // @8
    pub root_bone:   u16,          // @10
    pub _pad:        u32,          // @12
}
const _: () = assert!(size_of::<SkeletonAsset>() == 16);

/// A bare u32 carrier under the append-only / live-forever rule — the MeshHandle(u32) shape
/// (crates/boyko_scene/src/render_caps.rs:143), NOT a generational `Handle<T>`.
#[repr(transparent)] pub struct SkeletonId(pub u32);
#[repr(transparent)] pub struct ClipId(pub u32);
```

**Why a bare `u32` and not `Handle<T>`.** `Assets::remove` bumps a generation and the render
carrier stores only an index, so a freed-and-reused slot renders stale content silently — the
caveat is written into the kernel at `crates/boyko_ecs/src/ecs/core/asset/handle.rs:36-42`. Both
tables are append-only; there is no `remove` in rung 1.

### 3.1.1 The bank descriptor and the lane allocator — `crates/boyko_anim/src/bank.rs`

*(Critique pass 1, B2: the first draft put `group_count` / `bank_base` / `pair_base` / `lane_cap`
INSIDE the fingerprinted `SkeletonAsset`, so a bake was asked to know a world's instance count and
the first crowd rung would have bumped `ANIM_FP_SKELETON_ASSET`; and `AnimInstance.bank_lane` had
no writer. Both are repaired here by the plan's own rule — §3.3: freeze the fingerprinted layouts,
size the scratch layouts to the rung.)*

```rust
/// The RUNTIME bank coordinates of one registered skeleton. Row index == SkeletonId. NEVER on
/// disk; sized and written at `AnimBanks::register_skeleton`, read by every system.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BankDesc {              // 32 B
    pub skeleton:    u32,          // @0   debug cross-check (== its own row index)
    pub group_count: u32,          // @4   lane_cap / 8 — the bank's row pitch
    pub bank_base:   u32,          // @8   first BoneSoa8 row of this skeleton's bank region
    pub pair_base:   u32,          // @12  first BonePairPacked row of this skeleton's palette region
    pub lane_cap:    u32,          // @16  lanes reserved == 8 * group_count; FIXED for the region's life
    pub lane_top:    u32,          // @20  bump frontier: lanes [0, lane_top) have been handed out at least once
    pub free_base:   u32,          // @24  this skeleton's stack region in the FreeLane column (lane_cap rows)
    pub free_len:    u32,          // @28  live depth of that stack (LIFO)
}
const _: () = assert!(size_of::<BankDesc>() == 32);

/// One freed lane index on a skeleton's LIFO stack (id 468).
#[repr(transparent)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FreeLane(pub u32);
```

**Where `lane_cap` comes from.** A resource `AnimBankConfig { lanes_per_skeleton: u32 }` (rung-1
default **8** = one lane group; must be a multiple of 8, asserted at plugin build) read by
`AnimBanks::register_skeleton(skel: SkeletonId, cfg: &AnimBankConfig) -> BankId`, which appends
`group_count × bone_count` rows of `BoneSoa8`, `lane_cap` rows of `LaneJob` (all idle),
`lane_cap × bone_count` rows of `BonePairPacked` and `lane_cap` rows of `FreeLane` through their
`ScratchBuildView`s (in-place growth, address-stable — `views.rs:74-98`), and writes one
`BankDesc`. Regions are appended in registration order and **never move**: a skeleton registered
later starts after every earlier region, so `bank_base` is a stable coordinate.

**Growing a region is OUT of rung 1** (§0). The cap is a config value, not a hidden constant: a
crowd rung raises `lanes_per_skeleton` and pays for it in bank bytes (§1.3 of the design: 48 B ×
bones per lane). Exhaustion is **a loud coded diagnostic and a refused spawn** — the same policy
§5.5 applies to the skin ring (RESEARCH P16) — never a silent drop or a moved region.

**The writer of `AnimInstance.bank_lane` — the design's tombstone + LIFO free list
(`ANIMATION-DESIGN-SPACE.md:164-166`, `DENSE-COMPONENTS-PLAN.md:19`), adopted as written:**

```rust
impl AnimBanks {
    /// The ONLY producer of a lane. LIFO: pop the skeleton's free stack, else bump `lane_top`,
    /// else Err(AnimSpawnError::LanesExhausted { skeleton, lane_cap }) — loud, coded, refused.
    pub fn alloc_lane(&mut self, skel: SkeletonId) -> Result<u32, AnimSpawnError>;
    /// Push onto the free stack; the lane's `LaneJob.clip` becomes u32::MAX (idle) at the next
    /// `anim_advance` refill without any action here (§4.1 clears the column each substep).
    pub fn free_lane(&mut self, skel: SkeletonId, lane: u32);
}
```

The caller of `alloc_lane` is the spawn site (a `spawn_animated(skeleton, clip)` helper in
`boyko_anim::spawn` builds `AnimInstance { bank_lane, .. }` from it and seeds the palette pair
`prev == curr` — the no-teleport rule of §4.4). The caller of `free_lane` is an **`on_remove`
component hook on `AnimInstance`** (`crates/boyko_ecs/src/ecs/core/component/hooks/builder.rs:106`;
`HookFn = unsafe fn(DeferredEcsMaster<'_>, HookContext)`, `hooks/mod.rs:37`, and the hook "reads
the dying value", `builder.rs:104`), registered at plugin build.

**The hook CAN reach `AnimBanks` directly — answered at HEAD, not deferred to S2** *(verifier
pass 2)*. `DeferredEcsMaster::resource_mut<R: Resource>(&mut self) -> Option<&mut R>` exists at
`crates/boyko_ecs/src/ecs/core/component/hooks/deferred_master.rs:105-111`, its doc names it "the
canonical `on_remove` 'decrement a counter' pattern" (`:101-103`), and its SAFETY argument is that
resources live outside archetype storage so the `&mut R` cannot alias the apply's component writes
(`:106-108`). `AnimBanks` is a plain `Resource` (the same bound `ResMut<AnimBanks>` requires,
`crates/boyko_ecs/src/ecs/core/resources/resource.rs:42`), so the hook body is
`view.resource_mut::<AnimBanks>()` → `free_lane(dying.skeleton, dying.bank_lane)`. **Decision: the
DIRECT free.** The staged alternative the first draft kept in reserve — a `FreeLane`-typed staging
column drained by `anim_advance` — is dropped: it was insurance against a surface that turns out to
exist, and it would have been an eleventh id and a second free path to keep deterministic. A
`None` from `resource_mut` (the resource absent) is an invariant violation — `AnimationPlugin::
build` inserts `AnimBanks` before the hook is registered — and is reported through a coded
`boyko_log` diagnostic, not silently skipped. The free is deterministic in op order, which is what
makes lane identity reproducible run-to-run (the design's argument at `:164-166`).

**Determinism note (D15):** lane identity does not change the evaluated bits (T12), so a
different free-list history cannot change a pose — it can only change *which row* holds it.

### 3.2 Clip — `crates/boyko_anim/src/clip.rs`

```rust
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ClipHeader {            // 64 B == ONE cache line, align 4
    pub skeleton_name_hash: u32,   // @0   the skeleton this clip was baked against (pair guard)
    pub layout_fingerprint: u32,   // @4   codec-layout guard; a mismatch is a LOUD load refusal
    pub bone_count:         u16,   // @8   == the skeleton's bone_count (bake contract)
    pub sample_rate:        u16,   // @10  Hz (30 or 60)
    pub frame_count:        u32,   // @12  uniform samples; the clip spans [0, frame_count).
                                   //      BOUNDED: 1 <= frame_count <= 65536 (contract C8) — the
                                   //      Q16.16 clock addresses frames 0..=65535 and no more
    pub segment_count:      u32,   // @16  ceil(frame_count / 16)
    pub flags:              u32,   // @20  bit0 = looping, bit1 = has_root_motion (0 in v1)
    pub chunk_base:         u32,   // @24  first ClipBlobChunk row of this clip's byte stream
    pub blob_len:           u32,   // @28  exact byte length (chunks = ceil(blob_len/64))
    pub track_desc_off:     u32,   // @32  byte offsets INTO the blob
    pub track_desc_len:     u32,   // @36
    pub const_off:          u32,   // @40  constant-track values (raw f32)
    pub const_len:          u32,   // @44
    pub range_off:          u32,   // @48  per-segment range (T and S only)
    pub range_len:          u32,   // @52
    pub anim_off:           u32,   // @56  the animated sample stream
    pub anim_len:           u32,   // @60
}
const _: () = assert!(size_of::<ClipHeader>() == 64);

/// One track = one bone's one channel. 3 tracks per bone (T, R, S).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct TrackDesc {             // 8 B
    pub kind_bone:   u32,          // @0  (bone << 2) | kind ; kind: 0=T, 1=R, 2=S
    pub flags:       u16,          // @4  bit0 = constant, bit1 = default(dropped)
    pub const_index: u16,          // @6  slot in the const region when bit0 is set
}

/// The clip byte-stream backing row. 64 B so the tick sub-regions cost 1.125x, not 9x (§2.2).
#[repr(C, align(64))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ClipBlobChunk(pub [u8; 64]);
```

**Codec v1 sample encoding (WA-3, fixed 16-bit).** Per bone per animated frame:

| channel | bytes | encoding |
|---|---|---|
| translation | 6 | `[u16; 3]`, per-segment range-reduced: `v = min + (q / 65535) * extent` |
| rotation | **8** | `[u16; 4]`: lanes 0..2 = smallest-three components premultiplied by √2 (ozz's +41 % range), lane 3 bits 0..1 = index of the dropped largest component |
| scale | 6 | `[u16; 3]`, per-segment range-reduced |

**DECIDED FORK — the rotation sample is padded to 8 B, not bit-packed to 6.25 B.** Whole-pose at
64 bones: padded 20 B × 64 = **1280 B = 20 cache lines**; packed 18.25 B × 64 = 1168 B = 19 lines.
The saving is **one line in twenty (5 %)**, and the cost is a cross-byte bitfield extract in the
innermost decode loop — ~4 extra ALU ops × 64 bones ≈ 64 cycles per pose against ~14 cycles for
the one extra L2 miss it saves. It also destroys the uniform 8-byte stride that lets the SoA
scatter load rotations with a single `vpmovzxwd`. **Estimate**; re-measured at the codec bench
(§11, B2).

**Segment ranges.** Per segment (16 frames — ACL's `ideal_num_samples = 16`, [S]), per animated
T or S track: `min: [f32; 3]`, `extent: [f32; 3]` = 24 B. **Rotations carry no range** —
smallest-three is already normalised to `[-1/√2, +1/√2]`, which is what the √2 premultiply is
*for*. At 64 bones and a 1 s / 60 Hz clip (4 segments): range region = 4 × 64 × 48 = **12 KB**
against a 76.8 KB animated stream — 16 % overhead, and it is what makes 16-bit T/S accurate.

**Sample ORDER: frame-major, tracks contiguous within a frame.** ACL sorts "by time and by track"
([D], the algorithm page). Derived here rather than copied: a whole-pose decode of frame `f` is
then **one contiguous run** of `animated_bones × 20 B`, and the lerp's second frame `f+1` is the
immediately following run — 40 sequential lines at 64 bones, prefetcher-friendly. Per-bone
decode is refused by construction (RESEARCH P1: `decompress_bone` scans, it does not index).

**Root motion:** `flags` bit1 and the root-motion offsets stay in the header written as **0** in
v1, and a bake test asserts it. Keeping the 64-B one-cache-line header is worth 8 bytes of declared
debt; shrinking it now and growing it at L4 would bump `layout_fingerprint` on every baked clip.
Listed in the dead-datum ledger (§13).

### 3.3 Pose bank — `crates/boyko_anim/src/bank.rs`

```rust
/// One BONE ROW of ONE LANE GROUP: 12 fields x 8 instance lanes.
///  local pose  : fields 0..10  = tx ty tz | qx qy qz qw | sx sy sz   (fields 10,11 = 0)
///  after local->model, IN PLACE: 12 fields = the 3x4 ROW-MAJOR model affine of the bone
#[repr(C, align(64))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BoneSoa8 { pub f: [[f32; 8]; 12] }   // 384 B == 6 cache lines EXACTLY
const _: () = assert!(size_of::<BoneSoa8>() == 384 && align_of::<BoneSoa8>() == 64);

// Addressing: LANE-GROUP-MAJOR (DESIGN-SPACE D1 / B2).
//   row(desc, group, bone) = desc.bank_base + group * bone_count + bone
//   (desc: the skeleton's runtime BankDesc, §3.1.1; bone_count: its SkeletonAsset)
// The B bone rows of one lane group are ONE contiguous B x 384 B run.
```

**Rung 1 has exactly ONE bank** (the pose bank). The inertialization bank is L2.

**Working-set honesty, corrected against the design's L1 claim.** `DESIGN-SPACE` §1.3 bounds the
*bank* run at `B × 384 B` (24 576 B at 64 bones, L1d-resident for `B ≤ 85`). A rung-1 task also
streams the **clip**: two frames × 1280 B × 8 lanes = 20 KB. Total ≈ **45 KB per lane group → L2,
not L1**, at 64 bones. This is not a contradiction of the design — the design scopes its L1 claim
to "the pose bank's sample → blend → local→model passes" — but a plan that repeated "24 KB stays
in L1d" without counting the second stream would be wrong. Recorded as the rung-1 figure.

```rust
/// One lane's evaluation input, refilled EVERY substep by `anim_advance` — the
/// `SolverScratch::bodies_build` refill discipline (crates/boyko_physics/src/resources.rs:3160-3163,
/// :3223-3225) transplanted.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LaneJob {               // 16 B, align 4 — FOUR rows per cache line
    pub clip:       u32,           // @0  clip row index; u32::MAX == idle lane — NOT skipped: its load is
                                   //     pointer-SELECTED to a static zero record and its store is masked
                                   //     (§4.3), so the eight lanes keep one instruction stream
    pub frame:      u32,           // @4  integer FLOOR sample index, already wrapped into [0, frame_count).
                                   //     The PARTNER frame (frame+1, wrapped or clamped) is derived per
                                   //     lane at the sample from the clip's looping bit (§4.1) — it is
                                   //     never stored, and never equals frame_count
    pub frac_q16:   u16,           // @8  fraction between `frame` and `frame+1`, Q0.16
    pub bone_count: u16,           // @10 carried for the debug_assert against the bank's B
    pub skeleton:   u32,           // @12 debug cross-check that the lane belongs to this bank
}
const _: () = assert!(size_of::<LaneJob>() == 16);
```

**DECIDED FORK — the rung-1 `LaneJob` is 16 B, not the design's 192 B.** The design's row carries
`out_root`, `out_bounds`, `params[8]`, `slots[4]`, `inert_t`, `scalars[6]`, `lod_bones`,
`period_phase` — of which rung 1 uses **none**. Shipping 176 B of unread fields is precisely the
dead-datum class this repository catalogues. The row is **transient scratch with no on-disk
format** (unlike `ClipHeader`, which is fingerprinted), so widening it at L1/L2/L4 costs a
recompile and nothing else. That asymmetry — freeze the fingerprinted layouts, size the scratch
layouts to the rung — is the rule this plan applies throughout.

### 3.4 Per-instance components — `crates/boyko_anim/src/components.rs`

```rust
/// The lane handle. DENSE: every animated entity has exactly one, and the lane-indexed scratch
/// is keyed by `bank_lane`. One global column, no archetype fragmentation — the
/// GpuTransform3D rationale (crates/boyko_render/src/gpu_transform3d.rs:12-21).
#[repr(C)]
#[derive(Component, Clone, Copy, Pod, Zeroable)]
#[component(storage = "dense")]
pub struct AnimInstance {          // 12 B
    pub skeleton:  u32,            // @0
    pub bank_lane: u32,            // @4  lane index within that skeleton's bank; group = lane >> 3.
                                   //     Written ONLY by `AnimBanks::alloc_lane` (§3.1.1); freed by the
                                   //     `on_remove` hook — never assigned by hand
    pub lod:       u8,             // @8  rung 1: always 0
    pub period:    u8,             // @9  rung 1: always 1
    pub phase:     u8,             // @10 rung 1: always 0
    pub flags:     u8,             // @11 bit0 = evaluated this substep
}
```

The `lod`/`period`/`phase` bytes are the design's exact layout, written to their rung-1 constants
so the crowd-budget rung is a value change, never a fingerprint change.

```rust
/// Playback state. TABLE component (not dense): the L3 Aether machine will `joins (mut anim:
/// &mut AnimPlayer)` and fuse into its chunked pass, and MACHINES.md's R-DENSE refusal is exactly
/// why machine-facing state is not dense.
///
/// SERIALIZABLE WORLD STATE, so it is NOT under the §3.3 "size the scratch to the rung" rule:
/// it carries `format_version = 1`, and the L1 widening to `slots: [PlaySlot; 4]`
/// (ANIMATION-DESIGN-SPACE.md:185) is a COMPONENT-FORMAT change that bumps it to 2 — the
/// per-component version the format doc separates from the file version
/// (crates/boyko_serialize/src/format.rs:25-26; the derive key is
/// `#[component(stable_name = "..", format_version = N)]`, crates/boyko_macros/src/component.rs:452-454).
#[repr(C)]
#[derive(Component, Clone, Copy, Pod, Zeroable)]
#[component(stable_name = "boyko_anim::AnimPlayer", format_version = 1)]
pub struct AnimPlayer {            // 16 B
    pub clip:   u32,               // @0  clip row index; u32::MAX == stopped
    pub tick:   u32,               // @4  Q16.16 sample phase within [0, frame_count)
    pub speed:  i32,               // @8  Q16.16 playback rate (1<<16 == 1.0x; negative plays back)
    pub frac64: u8,                // @12 sub-substep REMAINDER carry (0..63) — see below
    pub flags:  u8,                // @13 bit0 = looping, bit1 = paused
    pub _pad:   [u8; 2],           // @14
}
```

**DECIDED FORK — the `frac64` remainder carry.** The design pins integer clip time
(`DESIGN-SPACE` D11): `advance = speed × rate / 64` per substep, in Q16.16 sample units. That
division is only exact when `speed × rate` is a multiple of 64; at `speed = 1.0` (65536) and
`rate = 30` it is (1 966 080 / 64 = 30 720 exactly), but at `rate = 30, speed = 0.7` it is not,
and a truncating divide **accumulates drift** — a clip slowly desynchronising is the worst kind of
determinism bug because it passes every single-frame gate. Carrying the remainder makes it exact
with one byte:

```
let num   = speed as i64 * rate as i64 + frac64 as i64;
let span  = (frame_count as i64) << 16;             // 2^32 EXACTLY at frame_count == 65536 — fits i64
let phase = tick as i64 + (num >> 6);               // arithmetic shift; num may be negative
tick   = if looping { phase.rem_euclid(span) } else { phase.clamp(0, span - 1) } as u32;
frac64 = (num & 63) as u8;
```

No float ever enters the clock. The only float is `frac_q16 / 65536.0` at the sample lerp, derived
from the integer phase (`DESIGN-SPACE` §8).

**The wrap is done in `i64`, and the bound at `frame_count = 65536` is stated** *(verifier pass 2,
NB5 edge)*. C8 admits `frame_count = 65536`, at which `frame_count << 16 = 2^32` **does not fit
`u32`** — a `u32` span is `0`, a `u32` `% span` panics, and a `u32` clamp bound of `span - 1`
wraps. In `i64` the span is `2^32` exactly, `rem_euclid` is the ordinary modulus, and the clamp
bound is `2^32 - 1 = u32::MAX`; the final `as u32` store is then the identity on `[0, 2^32)`. So at
the bound the loop case IS the natural `u32` modulus and the clamp bound IS `u32::MAX`, as
arithmetic, not as a special case — which is why the code carries no `if frame_count == 65536`
branch to forget. The `i64` form also removes a hazard the first draft's `wrapping_add` snippet had
at **negative `speed`** with `span < 2^32`: a `u32` wrap-around lands at `2^32 - |delta|`, which is
not `span - |delta|`, so a reversed looping clip would have jumped on every wrap. `rem_euclid` is
correct for a negative `phase`. Test **T24** (§11) samples the clock at exactly `frame_count =
65536` on both arms, and a reversed looping clip across its wrap.

⚠ **The `/ 64` is the 64 Hz substep, and it is PINNED, not assumed** *(critique pass 1, NB6)*.
`advance = speed × rate × Δt` collapses to `speed × rate / 64` only because the default fixed
timestep is `DEFAULT_FIXED_TIMESTEP = 15 625 000 ns`
(`crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:21`); `App::set_fixed_timestep` exists — the
method is defined at `crates/boyko_ecs/src/ecs/core/app/app.rs:457` (the `:68-70` cited by the
first draft is the `CoreSchedule::Fixed` doc comment that *links* it, not the definition) — and a
50 Hz or 120 Hz world would play every
clip at 78 % / 188 % of real time while every single-frame gate stays green — the R8 failure class
in a different coat. Rung 1 therefore:

1. **Asserts at `AnimationPlugin::build`** that `FixedTime::timestep()` (`fixed_time.rs:91`) equals
   `Duration::from_nanos(15_625_000)`, with a coded `boyko_log` diagnostic naming the two values —
   a loud refusal to build, never a silent wrong rate. Test T21 (§11) constructs an `App` at 50 Hz
   and asserts the refusal.
2. Records the general form for the rung that lifts the pin: `num = speed × rate × Δt_ns + carry`,
   `tick += num / 1e9`, `carry = num % 1e9` — the carry then needs a `u32`, which widens
   `AnimPlayer` and bumps its `format_version`. That is why rung 1 pins rather than generalises:
   the general form is a component-format change, and nothing in rung 1 needs it.

### 3.5 Palette — `crates/boyko_anim/src/palette.rs`

```rust
/// One bone's PREVIOUS -> CURRENT decomposed TRS pair, byte-identical to the B2 shader's
/// `TransformPair` (crates/boyko_rhi_vulkan/shaders/interp_instances.comp.hlsl:34-46) and to the
/// host mirror `GpuTransform3D` (crates/boyko_render/src/gpu_transform3d.rs:82-90).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BonePairPacked {        // 96 B: prev @0, curr @48; each Trs is pos@0 rot@16 scale@32
    pub prev: TrsPacked,           // (crates/boyko_render/src/gpu_transform3d.rs:53-62)
    pub curr: TrsPacked,
}
const _: () = assert!(size_of::<BonePairPacked>() == 96);

/// What the skin shader reads: the 3x4 ROW-MAJOR (bone_model * inv_bind) product.
/// Byte-identical to `InstanceModelCol` (crates/boyko_render/src/instance_model.rs:56-67).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PaletteRow { pub rows: [[f32; 4]; 3] }   // 48 B
const _: () = assert!(size_of::<PaletteRow>() == 48);
```

### 3.6 Skin side stream — `crates/boyko_render/src/skin_vertex.rs`

```rust
/// Per-vertex influences, a SIDE STREAM parallel to the 64-byte `Vertex`
/// (crates/boyko_render/src/mesh.rs:81-100). `VERTEX_STRIDE == 64` is const-asserted at
/// crates/boyko_render/src/mesh.rs:103-104 and every existing pipeline reads that record
/// unchanged — which is the deciding fact, not a bandwidth argument.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SkinVertex {            // 16 B (WA-4: u16 joint index)
    pub joints:  [u16; 4],         // @0  BONE indices (already remapped from glTF joint order)
    pub weights: [u16; 4],         // @8  unorm16, normalised at bake to sum exactly 65535
}
const _: () = assert!(size_of::<SkinVertex>() == 16);
```

**Weights are normalised at BAKE to sum to exactly 65535** (the residual folded into the largest
weight), so no per-vertex renormalising divide exists on either side. glTF only *recommends*
normalised weights; a bake that assumed it would be a silent wrong answer on non-conforming
content.

**The unorm16 → float conversion is a MULTIPLY by a literal reciprocal on BOTH sides, never a
divide** *(critique pass 1, NB7)*: `w_f = f32(w_u16) * (1.0 / 65535.0)`, the reciprocal written as
one literal so it is a single correctly-rounded constant. `w / 65535.0` in HLSL lowers to `OpFDiv`,
which Vulkan allows at 2.5 ULP while Rust's `/` is 0.5 ULP — a division can never be part of a
bit-exact contract (the repository's shader bit-exactness record, rule 7). The `skin_lbs_body`
leaf (§5.3) is the one place this form is written, so the `f32` oracle and the `Emit` printer
cannot disagree about it; `InterpBackend::div` (`crates/boyko_shaderdsl/src/interp.rs:65`) is
**not called** anywhere in the leaf, and T16 greps the emitted HLSL for `/` outside comments.

---

## 4. Algorithms on the critical path

### 4.1 `anim_advance` — the clock + the gather

Per instance: read `AnimInstance` (dense, via the per-row `e2s` gather), read/write `AnimPlayer`
(table), write one 16-B `LaneJob` at `bank_lane`.

**The column is reset to idle before the walk**: every one of the `lane_cap` rows of each bank's
`LaneJob` region gets the whole 16-B `LaneJob::IDLE` row first (`clip = u32::MAX`, every other field
0 — a TYPED `fill`, not a byte fill, over a contiguous 16 B × `lane_cap` run —
at 10 000 lanes, 160 KB), then the instance walk overwrites the live lanes. That is the
`SolverScratch::bodies_build` refill discipline (§3.3) and it is what makes a despawned entity's
lane idle **with no action by the freer** (§3.1.1): a lane nobody writes is idle by construction,
never by a flag someone remembered to clear.

- **Steps:** advance `tick` by the exact integer form (§3.4) → wrap into `[0, frame_count)` for a
  looping clip, clamp for a one-shot → split into `frame = tick >> 16` and `frac_q16 = tick as u16`
  → store `LaneJob`.
- **The partner frame** *(critique pass 1, NB4)*: the lerp reads `frame` **and** `frame1`. For a
  one-shot clip the clamp leaves `frame == frame_count − 1` at the end, and a naive `frame + 1`
  then addresses `anim_off + anim_len` — the next clip's chunks or past the column's live rows —
  while every listed `debug_assert` stays green. So `frame1` is derived **per lane, at the sample,
  by a `select` on the clip's looping bit**: `frame1 = looping ? (frame + 1) % frame_count :
  min(frame + 1, frame_count − 1)`, and a `debug_assert!(frame1 < frame_count)` sits beside the
  load. For a looping clip the partner wraps to `0` once per loop, which breaks the §3.2 "adjacent
  run" property for exactly one sample per loop — accepted, and stated so nobody "optimises" the
  wrap away. `frame_count == 1` gives `frame1 == 0` on both arms, so a single-frame clip is legal.
- **Complexity:** O(instances).
- **Cache:** two sequential column reads + one 16-B scattered store per instance (`bank_lane` is
  not row order). Four `LaneJob`s share a line, so a compacted bank writes ~0.25 lines/instance.
- **Branching:** two — `clip == u32::MAX` (idle) and the loop/clamp wrap. Both are
  `select`-shaped and should compile branchless; a `debug_assert` verifies the wrap invariant.
- **Not** a `dense_iter`: `Query::dense_iter` const-rejects a non-dense `D`
  (`crates/boyko_ecs/src/ecs/core/iters/query/query.rs:436-446` — corrected from the design's
  `:434-436`), and this query is mixed. It is `Query::iter` over
  `(&AnimInstance, Mut<AnimPlayer>)`, archetype-driven with the per-row dense gather.
- **Estimate:** ~40 ns/instance.

### 4.2 `anim_evaluate` — per lane group

```
for group in 0..group_count:                       # SERIAL in rung 1 (§0.1)
    for bone in 0..B:                              # parent-first order
        sample_bone_8(bone) -> bank[group][bone].f[0..10]      # 8 lanes at once
    for bone in 0..B:                              # SECOND pass, same order
        local_to_model_8(bone) IN PLACE            # f[0..12] := parent_affine * local
```

- **Complexity:** O(groups × B).
- **Cache:** the bank run is `B × 384 B` contiguous and is touched twice (write then
  read-modify-write); the clip stream is two contiguous per-frame runs per lane. Sequential
  throughout. §3.3 gives the 45 KB rung-1 working set.
- **Branching:** per bone, one `parent == u16::MAX` test (root) — hoisted out of the lane loop
  because `parent` is per bone, not per lane. Per track, one `flags & CONSTANT` test, likewise
  per bone. **Zero per-lane branches** — the eight lanes of a group run the same instruction
  stream by construction, which is what makes the SoA-8 shape pay.
- **SIMD — where the 8-wide ops live, decided** *(critique pass 1, B4: the first draft named
  `vpmovzxwd` / `blendv` and `sample_bone_8` without saying which crate holds the SoA-8 types or
  whether they are intrinsics; `boyko_math` has none — no file under `crates/boyko_math/src`
  matches `Soa8|F32x8|core::arch|target_feature`)*. Every op is 8-wide lane-wise `f32`, and:
  1. **The types land in `boyko_math::soa8`** — `F32x8`, `Vec3Soa8`, `QuatSoa8`, `Affine3Soa8`,
     exactly the AK-1 request (`ANIMATION-DESIGN-SPACE.md:568`) — as **`#[repr(C, align(32))]`
     `[f32; 8]` wrappers whose every method is a plain per-lane loop over the existing scalar
     `boyko_math` bodies**. `Quat::{dot, nlerp}` (the other half of AK-1) are not needed by rung 1
     (no blend) and are not added. This is a `boyko_math` change inside this rung's step S2, not a
     separate lane: it adds one module and touches nothing existing.
  2. **No `core::arch` intrinsics and no `cfg(target_feature)` fork in rung 1.** One implementation
     means the AVX2 build and any scalar build run the same IEEE `mul`/`add`/`sqrt` sequence, so the
     no-FMA property of the scalar bodies (`docs/ARCHITECTURE.md:107`) is inherited rather than
     re-proven, and cross-build bit-identity holds by construction. `rustc` does not contract
     `a*b+c` into an FMA without an explicit `mul_add`, and autovectorisation preserves the
     per-lane operation order, so a vectorised loop is the same bits as the scalar loop.
  3. **The gate is T20** (§11): each `soa8` op equals the scalar `boyko_math` op applied to every
     lane, `to_bits`-identical, over a seeded sweep including ±0, denormals, and inputs whose
     product would round differently under FMA. The "two implementations whose agreement nothing
     gates" hazard the critique named does not arise because there is one.
  4. `vpmovzxwd` / `cvtdq2ps` / `blendv` in this document are **what the compiler is expected to
     emit** for the u16→f32 widen, the convert and the per-lane `select`, not what is hand-written.
     Whether it does is what B1 (§11) and an assembly look decide; the intrinsic path is the
     measured fallback, listed in §14, and it would carry T20 as its agreement gate.

  The affine multiply is 12 FMA-free `mul`/`add` chains. The smallest-three reconstruction is the
  u16→f32 widen + a range `mul`/`add`, then one `sqrt` for the restored component.
- **D15 lane-wise purity holds in rung 1 and is gated (T12):** no horizontal op exists anywhere
  in `evaluate_group`. The largest-component index differs per lane, so the reconstruction
  selects branchlessly with three per-lane `select`s (expected to lower to `blendv`) on a per-lane
  mask — **not** a per-lane scalar branch,
  which would break the uniform instruction stream *and* make the result depend on lane order.
- **Estimate:** 25–60 µs per lane group (8 instances × 64 bones), anchored on ACL's cold-cache
  `decompress_pose` of 0.9–28.8 µs at 2.39 GHz [S/D]. **This box has not been measured.**

### 4.3 Whole-pose sample of one bone, 8 lanes

Eight lanes may sit at eight different `(clip, frame)`. Rung 1 therefore does **not** vectorise
the *load* across lanes — it loads each lane's 20-B bone record scalar and scatters into the SoA
row, then vectorises the *decode arithmetic* 8-wide. That is 8 scattered 20-B reads per bone,
filling 6 lines of the bank row completely before eviction (the design's §1.3 table row). Once
`compact_lanes()` exists (a later rung) a group is usually clip-uniform and a gathered load
becomes possible; rung 1 does not assume it and does not need it.

**Idle lanes — the load rule, stated** *(critique pass 1, NB9)*. An idle lane (`clip ==
u32::MAX`) has no clip and no frame, yet §4.2 promises zero per-lane branches. The resolution is a
**pointer `select`, not a branch**: per lane, the 20-B source pointer is
`select(idle, &IDLE_BONE_RECORD, live_ptr)` where `IDLE_BONE_RECORD: [u8; 20]` is a `static`
all-zero record (always valid memory, never a clip-table dependency — it works with an empty clip
table), and the store into the bank row is masked by `!idle`. The idle lane therefore runs the same
instruction stream, decodes zeros into a value nobody stores, and costs one masked store. The
branchy alternative — `if idle { continue }` inside the lane loop — would break the uniform
instruction stream the SoA-8 shape depends on, and is refused. Test T12's "alone in a group" case
covers exactly a group with seven idle lanes.

**The `select` happens on the INDEX, before any pointer arithmetic** *(verifier pass 2, NB9
caveat)*. As the paragraph above was first written, `live_ptr` for an idle lane would be computed
from `clip == u32::MAX` and a garbage `frame` — `base.add(u32::MAX × 64)` — *before* the select
discarded it. `ptr::add` past the end of the allocation is undefined behaviour under Miri **even
when the pointer is never dereferenced** (its contract is in-bounds arithmetic), so the stated form
invited exactly the UB T19 exists to catch. (And the idle `frame` is not garbage after §4.1's
reset: the typed fill writes `frame = 0`, so `live_byte_offset(hdr, 0)` is in range and no checked
`u32` multiply can overflow in debug even before the select — the word above describes the pre-fix
reasoning only.) The order is therefore:

```
hdr_idx  = select(idle, 0, job.clip)                     // 1. index select — no OOB index survives
off      = select(idle, 0, live_byte_offset(hdr, frame)) // 2. offset select, still integers
base     = select(idle, IDLE_BONE_RECORD.as_ptr(), clip_base)
src      = base.wrapping_add(off)                        // 3. the ONLY pointer arithmetic
                                                         //    idle: IDLE + 0, always in-bounds
```

`wrapping_add` (not `add`) is used at step 3 so that even a future edit that reorders steps 1-2
cannot re-introduce the UB: `ptr::wrapping_add` is documented as safe to leave the allocation with
as long as the result is not dereferenced, and the dereference sits after the final select. Step 1
matters on its own: `hdr_idx = 0` on an **empty** clip table would still be an out-of-range row, so
the header read is also routed through a `static IDLE_CLIP_HEADER: ClipHeader` selected by `idle`
rather than through the table — an all-idle group over an empty table (a bank registered, no clip
loaded yet) then touches no table row at all. T19 gains that exact fixture (eight idle lanes, zero
clips, under Miri) so the property is held by a test and not by the paragraph.

### 4.4 `anim_pack_palette`

**Group-major, not instance-major** *(critique pass 1, NB3: the first draft's "per instance, per
bone: read the bank's 12 model fields for its lane" fetches each 384-B bone row — 6 whole cache
lines — once per lane, i.e. 8× per group, to extract 48 useful bytes, because instances arrive in
dense-slot order and not lane order; the design's ⑥ traffic figure assumes the bank is read once)*.

Per **lane group**, per bone: read the bone's `BoneSoa8` row **once** (6 lines), transpose its
12 fields × 8 lanes into 8 model affines, multiply each by the bone's `inv_bind` (one 3×4, shared
across the 8 lanes — 3×4 × 3×4 affine compose), **decompose to TRS**, and write the 8 palette rows
`pair_base + lane × bone_count + bone` (`prev = old curr`, then `curr`). Idle lanes are written
too (their value is whatever the masked sample left — unread, and cheaper than a per-lane branch).
The walk is driven by `BankDesc` (`group_count`, `pair_base`) and needs **no per-instance query**
— `AnimInstance` is not read by this system at all, which also removes a scheduler edge (§8).

⚠ **Decompose?** The bank holds an affine; `BonePairPacked` holds TRS. The decompose (extract
scale as row lengths, orthonormalise, `Quat::from_mat3` — `crates/boyko_math/src/quat.rs:70`) is
~40 ops per bone per instance. **DECIDED FORK, §5.2** explains why the GPU pass needs TRS rather
than affines; the alternative — evaluating the bank in TRS all the way and composing on the GPU —
loses the in-place local→model of D2. The decompose is the price and it is paid once per bone per
substep on the CPU, not per vertex.

- **Single prev-shuffle site**, exactly as `pack_gpu_transforms`
  (`crates/boyko_render/src/gpu_transform_pack.rs:73`) is the sole writer of `GpuTransform3D.prev`.
  A spawn seeds `prev == curr` bitwise (the no-teleport rule).
- **Cache:** the bank is read **once** (`B × 384 B` per group, sequential); streaming write of
  96 B/bone/lane — 6 KB per 64-bone instance, 48 KB per full group.

---

## 5. The eDSL leaves and the GPU passes

### 5.1 Two dispatches, not one

```
⑦a  anim_palette.comp        one invocation per (instance, bone)
       reads  BonePairPacked (prev/curr TRS)  +  SkeletonBone.inv_bind
       calls  interp_trs(pair, alpha)          <- THE EXISTING GENERATED LEAF
       writes PaletteRow = interp_model * inv_bind
⑦b  skin_lbs.comp            one invocation per skinned vertex
       reads  Vertex, SkinVertex, 4x PaletteRow
       calls  skin_lbs_apply(...)              <- THE ONE NEW LEAF
       writes Vertex (skin-cache ring)          <- 64 B; NO prev-position ring in rung 1 (§5.5)
```

`alpha` is `FixedTime::overstep_fraction()`
(`crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:141`) — already the B2 pass's input.

### 5.2 DECIDED FORK — why two passes and a TRS palette pair

The obvious single-pass design lerps two **affine** palette rows componentwise. That is not a
rotation interpolation: two rotation matrices 15° apart (a fast limb at 64 Hz) blended at
`alpha = 0.5` give columns of length `cos(7.5°) = 0.9914` — a **0.86 % scale wobble** on every
skinned vertex, rising to 29 % at a 90° substep. `interp_trs` exists precisely to avoid this: it
lerps TRS and *then* composes (`crates/boyko_shaderdsl/src/interp.rs:24-32`).

Splitting the compose out costs `instances × bones` extra invocations — **64** for the rung-1
golden, 640 k at a 10 000-instance crowd, against ~1 M vertex invocations in ⑦b. The error goes to
zero, and — the decisive part — ⑦a's math is a leaf **that already exists, is already generated,
and is already gated**. The alternative burns a new leaf's worth of quaternion math and a new
oracle to prove the same thing.

**Consequence worth stating: `anim_palette.comp.hlsl` splices the SAME `emit_hlsl_transform_interp()`
output as `interp_instances.comp.hlsl`** (`crates/boyko_shaderdsl/src/emit/shaders.rs:31-69`). Two
shaders pinned to one generator is a stronger cross-check than either alone: a change to the
generator must move both `.spv` or both tests red.

### 5.3 The new leaf — `crates/boyko_shaderdsl/src/skin.rs`

```rust
/// Linear blend skinning of one vertex. Written ONCE, generic over the backend.
/// Instantiated over `f32` (the CPU ORACLE) and `Emit` (the HLSL printer).
pub fn skin_lbs_body<B: InterpBackend>(
    m0: [B; 12], m1: [B; 12], m2: [B; 12], m3: [B; 12],   // four 3x4 ROW-MAJOR palette rows
    w:  [B; 4],                                            // four weights (already sum-to-one)
    pos: [B; 3], nrm: [B; 3], tan: [B; 3],
) -> [B; 9];                                               // skinned pos(3) | nrm(3) | tan(3)
```

**Reuses `InterpBackend`** (`crates/boyko_shaderdsl/src/interp.rs:51-100`) — it already carries
`lit/add/sub/mul/div/neg/abs/sqrt/select/lt/gt/and/eq`, exactly the op set LBS plus the
normal/tangent renormalise needs. **No new backend trait, no new `Emit` node kind.**

Explicitly **not** `FieldScalar` (`crates/boyko_shaderdsl/src/scalar.rs:36`): that trait is the
`no_std` physics leaf's, and `interp.rs:12-22` records why it must not grow.

Ordering pinned in the body doc: `Σ wᵢ·Mᵢ` accumulated in **index order 0,1,2,3** with the same
operand order the CPU reference uses, so the `f32` instantiation is byte-identical (the
`eval_byte_identity` discipline).

**Three more pins, without which the order pin does not reach the GPU** *(critique pass 1, NB7 —
each is a divergence this repository has already measured)*:

1. **No contraction — REWRITTEN at verifier pass 2, because the pass-1 text named a printer
   capability that does not exist at HEAD.** `Σ wᵢ·Mᵢ·p` is a multiply-accumulate; Rust never
   contracts `a*b+c` without an explicit `mul_add`, and an operand-order pin constrains order, not
   contraction. The remedy is the SPIR-V `NoContraction` decoration, which HLSL `precise` emits —
   the same reason `cluster_cull.hlsl:127-143` writes its sum out `precise` instead of `dot()`,
   and `ddgi_resolve.hlsli:136-143` declares its accumulators `precise` and is then gated at
   0-4 ULP against its host oracle (`crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_resolve.rs:
   538-586`). **Where contraction is decided, stated correctly:** the pass-1 text said "DXC
   contracts by default"; the precedent it cited says the opposite — "DXC emits no Fma at all, so
   contraction is decided BELOW the .spv, where no byte- or disassembly-gate can see it"
   (`cluster_cull.hlsl:132-134`), i.e. by the driver's lowering, while `ddgi_resolve.hlsli:136-137`
   attributes it to DXC. The tree carries both accounts and this plan does not adjudicate between
   them, because the pin does not depend on which is right: an undecorated `OpFMul`/`OpFAdd` pair
   is *contractible* by whoever lowers it last, and `NoContraction` forbids it at every level
   below the source. What the pin cannot do is be checked from the `.spv` bytes alone — which is
   exactly why it needs a GPU readback gate and not only T17.

   **What the printer can and cannot do today.** The `Emit` printer emits **plain `float` temps
   with no `precise` qualifier** — `crates/boyko_shaderdsl/src/emit/mod.rs:8-14` titles it "Pass 1:
   NO `precise`" and defers `precise` to "Pass 2", `emit/shaders.rs:76-77` still describes Pass 2
   as future, and the shipped generated span in `interp_instances.comp.hlsl:82-202` contains zero
   occurrences of `precise` (grep). So "the `Emit` printer declares every accumulator `precise`"
   was false as a statement about HEAD, and neither §5.3's "no new `Emit` node kind" nor S11's file
   list named the printer change it silently required. **Pass 2 is a prerequisite owned by the eDSL
   lane** (§14): a *per-leaf* `precise` option on the printer — per leaf, because a global switch
   would move every existing generated span and every pinned `.spv` at once, and each of those is
   a separate `*_edsl_sync` red the eDSL lane must re-bless with its own readback evidence.

   **The claim, at its true width until Pass 2 lands.** Two spans feed G-FRAME (b)'s positions:
   ⑦a's `interp_trs` (the SAME `emit_hlsl_transform_interp()` output as `interp_instances`, §5.2 —
   non-`precise` today, and giving it `precise` moves `interp_instances`' pinned span and `.spv`,
   the plan's own "both red" rule) and ⑦b's new `skin_lbs_apply`. Neither is `precise` at HEAD.
   Therefore:

   - **The G-ORACLE half (T16) is unconditional:** `skin_lbs_body::<f32>` against the hand-written
     scalar reference is byte-for-byte on every machine, because both sides are Rust `f32` and
     neither contracts. This is where the operand-order pin lives, and it is not weakened.
   - **G-FRAME (b)'s GPU position clause is a ULP-bounded golden, not bit-for-bit, until Pass 2:**
     the skin-cache `Vertex` readback is compared to the CPU oracle at a tolerance of **≤ 2 ULP per
     position lane** (labelled **estimate**: one contractible multiply-add per weight term, four
     terms, each contraction removing one rounding — the 4-ULP figure `ddgi_probe_gi_resolve.rs:550`
     uses covers an extra sampler residual this leaf does not have; the number is pinned by the
     first recorded run and tightened, never loosened, after it). The comparison uses the signed
     integer bit distance with a sign-aware step across zero, since a position lane is not
     non-negative the way the DDGI irradiance lanes are (`:567-569`). A breach beyond the bound is
     a real divergence (wrong bone, wrong weight, wrong row order), not a contraction artifact —
     that is the discrimination the DDGI gate makes at `:548-549` and it holds here for the same
     reason.
   - **The claim is scoped at the palette boundary:** G-FRAME (b) also reads back the `PaletteRow`
     buffer ⑦a wrote and feeds *those* rows to the CPU LBS oracle, so ⑦b is judged **given the
     palette rows as read back** — independently of whether ⑦a's `interp_trs` contracted. ⑦a's
     rows are judged separately against `interp_trs::<f32>` ∘ `inv_bind` at the same ULP bound.
     One golden, two clauses, each naming the span it constrains.
   - **When Pass 2 lands** (for `skin_lbs_apply` first; for `interp_trs` only when the eDSL lane
     re-blesses `interp_instances`), the corresponding clause tightens to bit-for-bit, T17's
     byte-identity re-runs after every re-DXC so a recipe change that drops the decoration is red,
     and a `NoContraction` census over `spirv-dis` of the committed `.spv` (grep the decoration
     count against the span's op count) makes the presence of the decoration a checked fact rather
     than a comment.

   **Why "design the pin without `precise`" is not viable, stated so nobody retries it:** the host
   oracle would have to mirror the contraction the lowering *actually* chose, and that choice is
   made below the `.spv` by a driver whose decision is not observable from the bytes
   (`cluster_cull.hlsl:132-134`) and may differ per driver version; there is no source-level form
   that reproduces "whatever the driver did". A non-`precise` GPU result is therefore only ever
   *bounded* by an oracle, never *equal* to one — which is what the ULP clause above says.

   **One candidate that could avoid Pass 2 entirely — unverified, and not relied on:** HLSL
   `precise` propagates backward through the dependency tree feeding the decorated variable
   (`ddgi_resolve.hlsli:140-141`), and every HLSL function is inlined, so a `precise float3 p =
   skin_lbs_apply(...).pos;` in the hand-written wrapper *outside* the sentinels might decorate the
   whole generated body without touching the span or the printer. Whether propagation reaches
   through the inlined call far enough to cover every op is not established anywhere in the tree;
   the `spirv-dis` census above is how it would be, and if it holds it is the cheaper route for
   `interp_trs`, whose span is shared. Listed in §14.
2. **No division.** The unorm16 weight conversion is `w * (1.0/65535.0)` on both sides (§3.6);
   `InterpBackend::div` is never called in the leaf, and T16 asserts the emitted span contains no
   `/` outside a comment.
3. **The same `alpha`.** ⑦a's `alpha` is `FixedTime::overstep_fraction()` (§5.1), so the
   G-FRAME oracle must run `interp_trs::<f32>` at the **same** phase: the golden pins the substep
   count and the render-time offset (`BOYKO_WINDOW_FRAMES`-style, one fixed frame) so
   `overstep_fraction()` is a known constant on both sides, and the test prints it. An unstated
   `alpha` is an oracle that agrees only at `alpha == 0`.

Normals and tangents are **excluded** from any bit-for-bit clause regardless of Pass 2 (they need
`sqrt` + a renormalising divide, and division is never part of a bit-exact contract); G-FRAME (b)
compares them within 1 ULP of the oracle's renormalised value, stated as such. Positions follow
pin 1: ULP-bounded until Pass 2, bit-for-bit after.

### 5.4 Gates the leaves must join

| artifact | gate file | the two gates |
|---|---|---|
| `shaders/anim_palette.comp.hlsl` + `.spv` | `crates/boyko_rhi_vulkan/tests/anim_palette_edsl_sync.rs` | (1) the `// === GENERATED interp_trs BEGIN/END ===` span equals `emit_hlsl_transform_interp()` EXACTLY; (2) fresh DXC byte-equals the committed `.spv` |
| `shaders/skin_lbs.comp.hlsl` + `.spv` | `crates/boyko_rhi_vulkan/tests/skin_lbs_edsl_sync.rs` | (1) the `// === GENERATED skin_lbs_apply BEGIN/END ===` span equals `emit_hlsl_skin_lbs()`; (2) fresh DXC byte-equals `.spv` |

Both copy `crates/boyko_rhi_vulkan/tests/interp_edsl_sync.rs` structurally: the brace-counting
`extract_fn` (`:38-60`), the DXC locator that SKIPS when `dxc` is absent (`:65-80`), and the frozen
recipe `-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, NO `-O`
(`crates/boyko_rhi_vulkan/shaders/interp_instances.comp.hlsl:26-28`). Neither gate can see
contraction (§5.3 pin 1): they pin the text and the bytes, and the bytes of an undecorated span
are the same whether or not the driver later fuses them. That is G-FRAME (b)'s job.

**No `docs/SHADER-VARIANT-MANIFEST.md` row — and this CORRECTS `DESIGN-SPACE` §4.** That manifest
is the registry of `-D`-preprocessor variants that cannot collapse to one `.spv`
(`docs/SHADER-VARIANT-MANIFEST.md:3-16`: "a `-D` variant belongs HERE only if it changes the
*interface*"). Both new shaders are single-variant and `-D`-free. The nearest precedent settles
it: `interp_instances` — generated, gated, shipped — has **no** row. Adding rows for
single-variant shaders would dilute the one table whose value is that every row is a real fork.

**Registration:** two `embed_spirv!` blocks in `crates/boyko_rhi_vulkan/src/compute.rs` (the macro
is at `:86-90`, the first use at `:92-101`). **Generator bin:**
`crates/boyko_shaderdsl/src/bin/emit_skin.rs` — a new bin, matching the one-bin-per-family
precedent (`emit_vb`, `emit_particles`, `emit_ssao_variants`, `emit_probe_gi`) rather than growing
`emit_field` further.

### 5.5 Skin-cache ring

⑦b writes a full 64-B `Vertex` (`crates/boyko_render/src/mesh.rs:81-100`) — positions, normals and
tangents skinned, `color`/`uv` copied. **64 B per visible skinned vertex.**

**No previous-position ring in rung 1** *(critique pass 1, NB2)*. The first draft added a 12-B
per-vertex previous-position ring "for motion vectors, with no shader change". No shader under
`crates/boyko_rhi_vulkan/shaders` reads a per-vertex previous position (grep
`gPrevPos|prev_pos|PrevPosition|prev_vertex`: zero files); the only motion-vector consumer in the
tree is the `gbuffer_mrt_mv` variant, which reads a **per-instance** prev ring
(`docs/SHADER-VARIANT-MANIFEST.md:69`). The ring would have been 12 B × every skinned vertex of
write bandwidth per frame with no reader — a dead datum by this repository's own definition. It
arrives at the motion-vector rung **together with** the shader change that consumes it.

**"No shader change" is true of the shader CODE and false of the host binding tables** *(critique
pass 1, B3)*. Every consumer reads a vertex through a buffer the host names per mesh, and the skin
cache is per **instance**, so three host-side changes are part of S12 — not a shader edit, but not
nothing either:

| # | consumer | today | the S12 change |
|---|---|---|---|
| (a) | **VB** `vb_geom_fetch.hlsli` fetches `gMeshVerts[NonUniformResourceIndex(mesh_id)]` (`:120`), a bindless slot **allocated one per registered mesh** (`mesh_geometry_table.rs:10-14`, `MeshGeometryTable::register` at `:627`) | one slot per `MeshGpu`; **`register` hardcodes the descriptor offset to `0` and the range to `vertex_buffer.size`** (`mesh_geometry_table.rs:667-676`) — it takes no offset at all | a **per-skinned-instance geometry slot** whose `gMeshVerts` entry is the skin-cache ring buffer **at that instance's byte offset/range**. The slot write itself already takes `offset`/`range` (`crates/boyko_rhi_vulkan/src/geometry_bindless.rs:295-312`), but `register` does not expose them, so this is a **NEW entry point, `register_range`** (S12 below — with the convention it deliberately breaks and the gate that makes the break visible), whose `gMeshIndices`/meta reuse the source mesh's index buffer and counts |
| (b) | the VB instance ring's `mesh_id` lane is **copied from the mesh asset**: `mesh_assets.get_by_index(mesh_id).map_or(RESERVED, \|m\| m.geometry_slot)` (`mesh_draw.rs:535-537`, `sync_vb_instance_ring` at `:516`) | per-mesh | a **per-instance override**: a dense `SkinnedDraw { skin_geometry_slot: u32, ring_vertex_base: u32 }` component written at spawn, read by `sync_vb_instance_ring` alongside `mesh_ids`, so a skinned row packs its own slot |
| (c) | the raster paths bind a batch's vertex buffer from `DrawBatch.mesh_id → MeshGpu.vertex_buffer` (`mesh_draw.rs:81-86`, `mesh.rs:137-139`; the encoder's `bind_vertex_buffer(buffer, binding, offset)`, `crates/boyko_rhi/src/encoder.rs:256`) | per-mesh buffer, offset 0 | a skinned instance is its **own one-instance batch** whose vertex source is `(ring buffer, ring_vertex_base × 64)` — `DrawBatch` grows a `vertex_source` discriminant (`Mesh` \| `SkinRing { byte_offset }`), and the recorder binds accordingly. Batching skinned instances of one mesh into one draw is a later rung (they would need contiguous ring slices) |

Rung 1's golden runs one skinned instance, so (a)–(c) are each exercised exactly once; S13 cannot
be reached without them, and S12 names all three so nobody improvises a fourth.

Rung 1 dispatches for **every** skinned instance (there is one). Culling- and LOD-awareness is the
next rung (RESEARCH P7 measured the cost of not having it: Rukhanka, 17.6 ms with an *empty* view).
Ring overflow in rung 1 is a **loud coded diagnostic and a refusal to dispatch**, never a silent
path switch (RESEARCH P16); the LOD-drop response arrives with LOD.

---

## 6. Bake and load — reusing the `boyko_serialize` byte format

Gaia's rule is binding: "The bake tool PRINTS the existing `boyko_serialize` format — a second
byte format is forbidden" (`docs/gaia/DECISIONS.md:11-15`). **There is no `gaia` crate in the
tree** (verified: no workspace member, zero `gaia` hits under `crates/`) — Gaia is a ratified docs
campaign. Rung 1 therefore bakes **directly into the `BOYKOSAV` container**, and a Gaia *document*
front-end for the two asset kinds is a later rung that changes the authoring surface, not the bytes.

### 6.1 DECIDED FORK — format v3 adds a blob region; assets are worldless files

`save_world` requires a live `EcsMaster` (`crates/boyko_serialize/src/save.rs:159`), and an asset
bake has no world. Expressing a skeleton as entities-with-components would put one entity per bone
into an archetype — exactly the fragmentation §3.1 rejects.

**`FORMAT_VERSION: u32 = 3`** (`crates/boyko_serialize/src/format.rs:34`), with the header growing
a purely-appended 16-byte descriptor — the same move v2 made for the dense region (`:29-33`,
`:117-127`):

```rust
pub struct SaveHeader {           // 96 B (was 80); every v1 AND v2 field keeps its offset
    ...                           // magic@0 .. dense_store_count@72 _pad@76   (unchanged)
    pub blob_table_off: u64,      // @80  byte offset of the BlobBlock array
    pub blob_count:     u32,      // @88  0 for a world save (the 0%-gate: no blob bytes)
    pub _pad2:          u32,      // @92
}
pub const SIZE: usize = 96;

/// One asset blob. Mirrors DenseStoreBlock's shape (crates/boyko_serialize/src/format.rs:366-390).
#[repr(C)]
pub struct BlobBlock {            // 48 B
    pub name_hash:          u64,  // @0   FNV-1a 64 of the blob's stable name
    pub layout_fingerprint: u64,  // @8   the BLIT GUARD — a mismatch is a loud refusal, never a blit
    pub data_off:           u64,  // @16  from file start, >= COLUMN_REGION_ALIGN-aligned (:63)
    pub data_byte_len:      u64,  // @24
    pub elem_size:          u32,  // @32  stride (0 == opaque byte run)
    pub elem_align:         u32,  // @36
    pub format_version:     u16,  // @40  per-blob version (mirrors TypeTableEntry::format_version)
    pub _pad:               [u8; 6], // @42
}
```

**New public functions in `boyko_serialize` (the names the developer implements):**

```rust
// crates/boyko_serialize/src/save.rs   (beside save_world :159 / save_world_to_file :710)
pub fn save_blobs(out: &mut Vec<u8>, blobs: &[BlobInput<'_>]) -> Result<(), SaveError>;
pub fn save_blobs_to_file(path: &Path, blobs: &[BlobInput<'_>]) -> Result<(), SaveError>;

// crates/boyko_serialize/src/load.rs   (beside load_world :213 / load_world_from_file :282)
pub fn load_blobs(bytes: &[u8]) -> Result<BlobDir<'_>, LoadError>;   // ZERO-COPY, NO world
impl<'a> BlobDir<'a> {
    pub fn get(&self, name_hash: u64) -> Option<Blob<'a>>;
}
impl<'a> Blob<'a> {
    pub fn bytes(&self) -> &'a [u8];
    /// Checked reinterpret: fingerprint + elem_size + elem_align + length divisibility, all
    /// LOUD. Never a silent blit of stale bytes.
    pub fn as_slice<T: Pod>(&self, fingerprint: u64) -> Result<&'a [T], LoadError>;
}
```

A blob file is a `SaveHeader` with `archetype_count = 0`, `entity_count = 0`,
`dense_store_count = 0`, `blob_count = N`. `load_world` on such a file succeeds and loads nothing;
`load_blobs` on a world save returns an empty directory. Both are the 0 %-gate.

**Cost of the version bump, checked rather than assumed:** `read_header` rejects
`format_version != FORMAT_VERSION` outright (`crates/boyko_serialize/src/load.rs:301-304`), so v2
files die. There are **zero committed `.boykosav` files in the tree** (glob: no matches) and the
format tests compare against the constant (`crates/boyko_serialize/tests/save_format.rs:152`,
`:281`), so the in-tree cost is zero. Out-of-tree saves are owner ballot **V1** (§12.B).

### 6.2 The two asset files

| file | blobs (`name_hash` of) | element | elem_size |
|---|---|---|---|
| `*.bskel` | `"skel.header"` | `SkeletonAsset` | **16** (§3.1 — bake-knowable fields only; bank coordinates are runtime `BankDesc`) |
|  | `"skel.bones"` | `SkeletonBone` | 96 |
|  | `"skel.names"` | `BoneNameHash` | 4 |
| `*.bclip` | `"clip.header"` | `ClipHeader` | 64 |
|  | `"clip.stream"` | opaque bytes | 0 |

**Fingerprints are hand-pinned constants**, `ANIM_FP_SKELETON_BONE: u64` etc., each sitting beside
a const-assert block over `size_of` / `align_of` / every `offset_of`. This is deliberately *not*
routed through a derive: the pin discipline is stronger (it names each field offset) and it
introduces no dependency on the component-derive machinery for types that are not components.

### 6.3 The load path is reflection-free — the exact call sequence

```rust
// crates/boyko_anim/src/assets.rs
pub fn load_skeleton(tab: &mut SkeletonTable, bytes: &[u8]) -> Result<SkeletonId, AnimLoadError> {
    let dir   = boyko_serialize::load_blobs(bytes)?;              // header walk only
    let hdr   = dir.get(FNV_SKEL_HEADER)?.as_slice::<SkeletonAsset>(ANIM_FP_SKELETON_ASSET)?;
    let bones = dir.get(FNV_SKEL_BONES )?.as_slice::<SkeletonBone>(ANIM_FP_SKELETON_BONE)?;
    let names = dir.get(FNV_SKEL_NAMES )?.as_slice::<BoneNameHash>(ANIM_FP_BONE_NAME)?;
    let base  = tab.bones.len() as u32;                           // rebase target for bone_base
    tab.bones.build_view().extend_from_slice(bones);              // ONE memcpy   (id 477)
    tab.names.build_view().extend_from_slice(names);              // ONE memcpy   (id 476)
    let id    = tab.headers.build_view().push(rebased(hdr[0], base)); // ONE push (id 470, §2.1)
    // debug_assert: parent[i] < i for every i (the bake contract, not re-verified in release)
    Ok(SkeletonId(id))
}
```

`tab.headers` is `ScratchColumn<SkeletonAsset>` under **id 470** — *(critique pass 1, B1: the
first draft pushed into a `headers` column that had no row in the §2.1 table, which
`ScratchColumn::new` cannot construct — `ComponentPool::new(component_id.get(), …)` reads the
registered layout, `scratch_column.rs:62-69` — leaving a developer to pick an id outside T13's
census or back it with a `Vec`.)* Loading a skeleton registers **no bank**: the bank region is a
separate, explicit `AnimBanks::register_skeleton(id, &cfg)` (§3.1.1), because a skeleton loaded for
tooling or retargeting must not reserve lanes it never uses.

`extend_from_slice` is `ScratchBuildView`'s
(`crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:93-98`) — the single-threaded refill
surface, `!Send` by construction. No `resolve_stable_name`, no `TypeTableEntry`, no
`deserialize_fn`, no allocation beyond the pool's own in-place growth.

`load_clip` is the same shape with the byte stream copied into `ceil(blob_len/64)`
`ClipBlobChunk` rows.

### 6.4 Bake contracts — eager, closed, one red fixture PER AXIS

Gaia's evaluator rules (`docs/gaia/DECISIONS.md:56-79`) and its fixture rule (`:81-88`, "One
committed red fixture PER AXIS, not one for the set") apply to the bake:

| # | contract | refusal |
|---|---|---|
| C1 | `parent[i] < i` for every bone after the parent-first sort | `BakeError::ChildBeforeParent` |
| C2 | `lod` monotone along parents (rung 1: all zero) | `BakeError::LodNotMonotone` |
| C3 | clip `bone_count == skeleton.bone_count` **and** `skeleton_name_hash` matches | `BakeError::SkeletonMismatch` |
| C4 | every channel's target node resolves to a joint of the skin, and its `BoneNameHash` matches | `BakeError::ChannelTargetsNonJoint` |
| C5 | sampler `input` strictly increasing; `input.len() * components == output.len()` (×3 for CUBICSPLINE) | `BakeError::MalformedSampler` |
| C6 | per-track quantisation error within tolerance; the bake PRINTS **max AND exceedance fraction** (RESEARCH P3) | `BakeError::QuantizationExceeded` |
| C7 | `has_root_motion == 0` in codec v1 | `BakeError::UnsupportedCodecFlag` |
| C8 | `1 <= frame_count <= 65536` after resampling — the Q16.16 `tick` holds the frame in its high 16 bits (`frame = tick >> 16`), so a longer clip (18.2 min at 60 Hz, 36.4 min at 30 Hz — a cinematic reel) would overflow the phase silently; `load_clip` re-checks it *(critique pass 1, NB5)* | `BakeError::ClipTooLong { frame_count, max: 65536 }` |
| C9 | **the non-joint-ancestor fold**: glTF's joint matrix is `globalTransform(joint) · inverseBind` and `globalTransform` composes EVERY ancestor node, joint or not (a Blender export puts an `Armature` node — often with a −90° X rotation — above the root joint). The root bone's `rest` MUST be `fold(non-joint ancestors of the root joint, root-to-leaf) · trs(root joint)`, and a non-joint node BETWEEN two joints is folded into the CHILD joint's `rest`. The fold is static by construction: C4 already refuses a channel targeting a non-joint node, so no folded node can animate *(critique pass 1, NB10)* | `BakeError::AnimatedNonJointAncestor` (defensive; C4 fires first) — and the S3 probe REPORTS the ancestor shape of both real assets (§7.3) |
| C10 | skin weights: `Σ weights == 65535` after the residual fold, `joints[i] < bone_count`, and every referenced bone has `flags & 1` set | `BakeError::MalformedInfluence` |

Each gets its own committed red fixture (§11) — a single fixture would prove at most one axis
while the rest ship unguarded. **The V2 synthetic 2-bone fixture cannot fail C9** (it has no
non-joint ancestor), so the C9 red fixture is a THIRD synthetic: 2 bones under a rotated
non-joint node, whose golden rest pose is hand-derived and would be wrong by exactly that rotation
if the fold were skipped.

**Resampling at bake:** glTF samplers carry irregular times; the bake resamples to the uniform
rate. LINEAR → lerp, with rotations using **nlerp with the shortest-path sign fix** — the same rule
the runtime uses (`DESIGN-SPACE` §3) so bake and runtime cannot disagree. STEP → hold.
CUBICSPLINE → the glTF Hermite form. All three offline.

---

## 7. The `.glb` decode delta

### 7.1 What the loader does today (re-opened at HEAD)

| fact | site |
|---|---|
| default `decode` **refuses** `animations` and `skins` outright | `crates/boyko_render/src/loaders/glb.rs:1090-1105` (the refusal loop), `:17-21` (the subset doc) |
| `decode_static_pose` tolerates the deformation rows and **drops joints/weights/deltas** | `:1027-1029` → `decode_document(bytes, true)` → `decode_scene_impl(bytes, true)` at `:1097` |
| `decode_primitive` reads POSITION, NORMAL, TEXCOORD_0, TANGENT, COLOR_0 — **`JOINTS_0` / `WEIGHTS_0` are never read at all** | `:849-877` |
| morph `targets` refused unless `static_pose` | `:839-841` |
| a skinned node is placed at **IDENTITY** per glTF §3.7.4 | `:776-783` |
| the accessor reader already handles `f32` / `u8` / `u16` / `u32` with the `normalized` convention | `:485-514` |

### 7.2 What rung 1 accepts, and how

**A third by-name entry point, not a relaxation of `decode`** (WA-1):

```rust
// crates/boyko_render/src/loaders/glb.rs — beside decode_static_pose (:1027) and
// decode_scene_static_pose (:1047)
impl GlbMeshLoader {
    pub fn decode_skinned(bytes: &[u8]) -> Result<GlbSkinned, AssetError>;
}

pub struct GlbSkinned {
    pub scene:      GlbScene,               // the existing per-primitive parts, BIND-POSE geometry
    pub skin:       GlbSkin,
    pub influences: Vec<GlbSkinVertexRaw>,  // parallel to the CONCATENATED vertex order of scene.parts
    pub animations: Vec<GlbAnimation>,
}
pub struct GlbSkin {
    pub joint_nodes:    Vec<u32>,      // skin.joints — glTF node index per joint
    pub inverse_bind:   Vec<[f32; 16]>,// skin.inverseBindMatrices, COLUMN-major (glTF convention)
    pub skeleton_root:  Option<u32>,   // skin.skeleton
    pub node_parent:    Vec<i32>,      // parent NODE index per node, -1 == root
    pub node_trs:       Vec<[f32; 10]>,// per-node LOCAL rest T3|R4|S3
    pub node_name_hash: Vec<u32>,
}
pub struct GlbSkinVertexRaw { pub joints: [u16; 4], pub weights: [f32; 4] }
pub struct GlbAnimation { pub name_hash: u32, pub channels: Vec<GlbAnimChannel>, pub samplers: Vec<GlbAnimSampler> }
pub struct GlbAnimChannel { pub target_node: u32, pub path: GlbAnimPath /* T|R|S|Weights */, pub sampler: u32 }
pub struct GlbAnimSampler { pub input: Vec<f32>, pub output: Vec<f32>, pub interp: GlbInterp, pub components: u8 }
```

**Finding worth stating: the byte-reading layer needs ZERO changes.** `JOINTS_0` is
`UNSIGNED_BYTE`/`UNSIGNED_SHORT` and glTF does **not** mark it normalized, so
`Accessor::read` (`:485-514`) already returns the integer value exactly as an `f32` (both widths
< 2²⁴). `WEIGHTS_0` is `f32` or normalized `u8`/`u16`, which the same function already divides by
255/65535 (`:497-504`). The whole delta is glTF *structure* walking — **estimate: ~250 lines**,
no new numeric code.

**One new sibling helper**,
`fn resolve_node_hierarchy(root: &Json) -> Result<(Vec<i32>, Vec<[f32; 10]>), AssetError>`,
placed next to `resolve_instance_transform` (`:729`) and **not modifying it** — the shipped
placement path takes zero behavioural risk. The identity rule at `:776-783` stays correct and
untouched: rung 1 places the skinned instance by the entity's `Transform`, and the bone model rows
carry the rest.

### 7.3 What `decode_skinned` REFUSES (each loud, each its own red fixture)

- morph `targets` — **stricter than `decode_static_pose`, deliberately.** Once a path claims to
  render skins, silently dropping a blend shape is the half-decode the module doc forbids
  (`:46-56`). WA-6 puts morphs after skeletal, so rung 1 refuses rather than tolerates.
- `JOINTS_1` / `WEIGHTS_1` present (>4 influences) — the side stream is 4-wide by design D9.
- a **joint node carrying `matrix`** instead of TRS (see the measurement below).
- everything `decode` already refuses: sparse accessors, `extensionsRequired`, non-TRIANGLES,
  non-indexed, a node reachable twice.

⚠ **The `matrix`-on-joint refusal must be MEASURED before it ships, not assumed.** This is the
Rev-33 lesson written into this very file (`:58-78`): refusing a legal-but-inconvenient construct
looked principled and refused essentially every real `.glb`. **Step S3 therefore begins by running
`decode_skinned`'s node walk against the two rigged assets already in the tree —
`assets/models/anime_girl/model.glb` and `assets/models/alien/model.glb` — and reporting (1) which
form their joint nodes take, AND (2) the ancestor shape above the root joint: how many non-joint
nodes, their TRS, and whether any non-joint node sits between two joints (contract C9).** If
either carries `matrix`, TRS decomposition lands inside rung 1 rather than as a refusal. (Both
files are untracked owner content per `git status`, so they are a *probe*, never a committed
fixture.) The second question is asked here because the synthetic gate fixture cannot exhibit it
and the silent wrong pose would otherwise first appear on the real assets.

---

## 8. Systems and schedule

```
CoreSchedule::Fixed                              (crates/boyko_ecs/src/ecs/core/app/app.rs:64-71)
                                                 64 Hz = 15 625 000 ns (fixed_time.rs:19-21)
  FixedSet::Gameplay                             (crates/boyko_scene/src/sets.rs:34-41)
    anim_advance      Query<(&AnimInstance, Mut<AnimPlayer>)>, ResMut<AnimBanks>, Res<ClipTable>
    anim_evaluate     ResMut<AnimBanks>, Res<SkeletonTable>, Res<ClipTable>
                      .after(anim_advance)
  FixedSet::Snapshot                             (wired .after(Gameplay) at
                                                  crates/boyko_app/src/plugins.rs:705-708)
    anim_pack_palette Res<AnimBanks>, Res<SkeletonTable>, ResMut<PaletteStaging>
                      (group-major over BankDesc, §4.4 — NO Query<&AnimInstance>)

CoreSchedule::Main
    anim_skin_dispatch  .gpu()                   (SystemConfig::gpu, system_config.rs:196-219 —
                                                  a GpuCompute system runs dispatcher-solo at the
                                                  apply-window barrier)
```

**Scheduler shape.** One `&mut` writer per column per system — write conflicts are filter-agnostic,
so `anim_evaluate` owning `ResMut<AnimBanks>` is one conflict node. `anim_pack_palette` joins
`FixedSet::Snapshot` beside `pack_gpu_transforms`
(`crates/boyko_render/src/gpu_transform_pack.rs:73`, wired at
`crates/boyko_app/src/plugins.rs:707`); they write different columns and are unordered relative to
each other, which is correct.

**Why `anim_pack_palette` must be in `Snapshot` and not `Gameplay`:** `FixedSet` exists solely to
stop "a permanent one-substep lag", and a Fixed system in **neither** set is unordered relative to
the seam (`crates/boyko_scene/src/sets.rs:31-33`). Test T11 (§11) makes that failure visible rather
than trusting the registration.

**Plugin:** `crates/boyko_anim/src/plugin.rs` → `AnimationPlugin`, added from
`boyko_app::EnginePlugins::build` (`crates/boyko_app/src/plugins.rs:335-340`) in the same
`add_systems_cfg_in(CoreSchedule::Fixed, …)` block that already wires the seam (`:705-708`).

**Zero-cost when unused:** a world with no `AnimInstance` column yields zero matching archetypes
and an empty bank, so every system does zero work — the 0 %-gate discipline
`sync_instance_model_cols` documents (`crates/boyko_render/src/instance_model.rs:105-108`).

---

## 9. Steps, in order

Each step names its crate, its files, and the gate that must be **red before** the code exists.

### S1 — `boyko_serialize` v3: the blob region
**Files:** `crates/boyko_serialize/src/format.rs` (`SaveHeader` @80..96, `BlobBlock`, the const
asserts at `:135-151` extended), `src/save.rs` (`save_blobs`, `save_blobs_to_file`), `src/load.rs`
(`load_blobs`, `BlobDir`, `Blob`; `read_header` at `:294-309` accepts the 96-B image),
`src/error.rs` (`LoadError::BlobFingerprintMismatch`, `::BlobMisaligned`).
**Red first:** T1, T2, T3.
**Note:** `SaveHeader::SIZE` moves 80 → 96; every existing offset assert stays true.

### S2 — `boyko_anim` crate skeleton + storage
**Files:** `crates/boyko_anim/Cargo.toml`, `src/lib.rs`, `src/scratch_ids.rs` (the `460..=477`
band — **ten** ids, `register_anim_scratch_layouts()`), `src/skeleton.rs`, `src/clip.rs`,
`src/bank.rs` (`BoneSoa8`, `BankDesc`, `FreeLane`, `AnimBanks::{register_skeleton, alloc_lane,
free_lane}`, `AnimBankConfig`), `src/components.rs`, `src/palette.rs`, `src/assets.rs`,
`src/spawn.rs` (`spawn_animated`), **and `crates/boyko_math/src/soa8.rs`** (`F32x8`, `Vec3Soa8`,
`QuatSoa8`, `Affine3Soa8` as `[f32; 8]` per-lane wrappers — §4.2).
**Also:** `Cargo.toml:2` + `Cargo.toml:13`, `crates/boyko_diag/src/sample.rs:150`; the
`on_remove` hook registration for `AnimInstance` (§3.1.1) — the **direct** free through
`DeferredEcsMaster::resource_mut::<AnimBanks>` (`hooks/deferred_master.rs:105-111`; decided in
§3.1.1, nothing left for S2 to verify).
**Red first:** T13 (id distinctness, now over ten ids), T10 (blob-chunk stride), T20 (`soa8` ==
scalar per lane), T22 (lane alloc/free is LIFO and exhaustion is a coded refusal), the layout
const-asserts.

### S3 — `.glb` skinned decode
**Files:** `crates/boyko_render/src/loaders/glb.rs` (`decode_skinned` beside `:1027`;
`resolve_node_hierarchy` beside `:729`; the four new refusals).
**Begins with the `matrix`-vs-TRS probe of §7.3.**
**Red first:** T4 (refusal unweakened), T5 (rest-pose bit-identity), T6/T7/T8 (the refusal axes).

### S4 — the bake
**Files:** `crates/boyko_render/src/loaders/anim_bake.rs` — `bake_skeleton`, `bake_clip`,
`bake_skin_stream`; the ten contracts C1–C10 (§6.4).
**Red first:** one fixture per contract axis.

### S5 — codec v1 encoder
**Files:** `crates/boyko_anim/src/codec/encode.rs` (segmenting at 16 frames, constant-track
detection, per-segment range extraction, smallest-three quantisation).
**Red first:** T9 (round trip prints max AND exceedance; a fixture with one bone past tolerance is
RED).

### S6 — codec v1 decoder + the SoA-8 sample
**Files:** `crates/boyko_anim/src/codec/decode.rs`, `src/evaluate.rs::sample_bone_8`.
**Red first:** decode∘encode round trip within tolerance; a `debug_assert` that the reconstructed
quaternion is unit within 1e-6; T23 (the one-shot partner frame at `frame_count − 1` reads inside
the clip — a clip placed LAST in the column with `get_unchecked` under Miri, T19's harness).

### S7 — `local_to_model_8` and `evaluate_group`
**Files:** `crates/boyko_anim/src/evaluate.rs`.
**Red first:** T12 (lane independence — the D15 gate), T14/T15 (G2-SHAPE).

### S8 — `anim_advance`
**Files:** `crates/boyko_anim/src/systems/advance.rs`, `src/plugin.rs` (the 64 Hz pin, §3.4).
**Red first:** T18 — 10 000 substeps at `speed = 0.7`, `rate = 30` land on the exact rational
phase with zero accumulated drift (the `frac64` carry, §3.4); T21 (a 50 Hz `App` is refused at
plugin build).

### S9 — `anim_pack_palette`
**Files:** `crates/boyko_anim/src/systems/pack.rs` — **group-major over `BankDesc`** (§4.4), no
instance query.
**Red first:** the decompose∘compose round trip is within a stated tolerance for a rotation-only
bone; the prev-shuffle happens at exactly one site (a census test greps for writers of `.prev`);
and the bank-read count of **benchmark B3** (`pack_palette`, §11 — not critique item B3) is
`group_count × B` rows, not `instances × B` (asserted through a counting `ScratchSolveView` shim in
the test, not by timing).

### S10 — plugin + schedule wiring
**Files:** `crates/boyko_anim/src/plugin.rs`, `crates/boyko_app/src/plugins.rs:705-708`.
**Red first:** T11 (the out-of-set one-substep lag, made visible).

### S11 — the eDSL leaf + the two shaders
**Files:** `crates/boyko_shaderdsl/src/skin.rs`, `src/emit/shaders.rs` (`emit_hlsl_skin_lbs`),
`src/bin/emit_skin.rs`, `crates/boyko_rhi_vulkan/shaders/anim_palette.comp.{hlsl,spv}`,
`shaders/skin_lbs.comp.{hlsl,spv}`, `src/compute.rs` (two `embed_spirv!`).
**NOT in this step's files:** `crates/boyko_shaderdsl/src/emit/mod.rs` — the printer's Pass 2
(per-leaf `precise`, `emit/mod.rs:8-14`) is the eDSL lane's, and S11 emits a plain-`float` span
like every other leaf at HEAD (§5.3 pin 1). If Pass 2 has landed by S11, the skin leaf opts in;
if not, S11 does not wait for it.
**Red first:** T17 (the two `*_edsl_sync` gates) and **T16 / G-ORACLE**, which runs everywhere.

### S12 — skin-cache ring + dispatch
**Files:** `crates/boyko_render/src/skin_vertex.rs`, `src/skin_cache.rs`,
`crates/boyko_render/src/mesh_assets.rs` (a skinned sibling of `build_mesh_gpu` at `:386` that also
uploads the side stream), the dispatch in `boyko_app`'s frame graph, **and the three host-binding
changes of §5.5** *(critique pass 1, B3; the shape of (a) corrected at verifier pass 2)*:

(a) **A per-skinned-instance geometry slot into the ring, through a NEW `MeshGeometryTable::
register_range`** — not through `register`. What `register` really is at HEAD
(`crates/boyko_render/src/mesh_geometry_table.rs:627-676`): it takes `&BoundBuffer` and a vertex
count, and writes the `gMeshVerts` slot with **`offset = 0` hardcoded and `range = vertex_buffer.
size`** (`:667-676`). That `0` is not an omission; the comment block at `:646-666` records it as
the fix for "rung R8 GPU debug, round 6", where a nonzero offset in this exact call produced a
**silent zero read** (no `robustBufferAccess`, validation off — no VUID, no crash), and names the
engine-wide convention it matches: every `BoundBuffer` is its own freshly created `VkBuffer`, so
`0` is always "the whole buffer", and `create_bind_group`'s `StorageBuffer` arm in
`rhi_impl/device.rs` hardcodes `offset: 0` for the same reason. The skin cache breaks the premise
of that convention on purpose — **one ring `VkBuffer`, many instances inside it** — so its slot is
the first nonzero buffer-relative descriptor offset this table will ever write, and the plan says
so rather than letting a developer "just pass the offset" into a function that discards it.

Why this offset is correct where the R8 one was wrong, stated so the incident cannot repeat by
analogy: the R8 value was a **memory-bind** offset (where the buffer sat inside a shared
`VkDeviceMemory` block) passed as a **buffer-relative** one; `ring_vertex_base × 64` is
buffer-relative by construction — bytes from the start of the ring's own `VkBuffer`, which is the
buffer named in the same descriptor write. Two further rules `register_range` carries: (1) the
Vulkan alignment requirement on a storage-buffer descriptor offset — `VkDescriptorBufferInfo.
offset` must be a multiple of `minStorageBufferOffsetAlignment` (Vulkan spec,
`VUID-VkWriteDescriptorSet-descriptorType-00328`), so the ring allocator rounds each instance's
slice start up to `max(64, minStorageBufferOffsetAlignment)` as read from the device limits, and
`register_range` **debug-asserts** that alignment plus `offset + range <= buffer.size`; (2) the
existing `register` keeps its `0` and its comment gains one sentence naming `register_range` as
the sanctioned sub-range path, so the convention is *stated as amended*, not silently violated.
`register_range`'s `gMeshIndices`/meta reuse the source mesh's index buffer and counts.

*Alternative considered and refused:* one dedicated `BoundBuffer` (one `VkBuffer`) per skinned
instance, registered through the unchanged `register`, would keep `offset = 0` true and take zero
risk on the recorded failure at rung 1's single instance — but it is not a ring, every instance
becomes a buffer-object and a descriptor, and the crowd rung would replace it wholesale. The ring
with `register_range` is the final shape; its risk is retired by the gate below, not by avoiding
the offset.

(b) the `SkinnedDraw` dense component and its read in `sync_vb_instance_ring`
(`crates/boyko_render/src/mesh_draw.rs:516-537`); (c) `DrawBatch.vertex_source` and the recorder's
`bind_vertex_buffer` offset (`mesh_draw.rs:81`, `crates/boyko_rhi/src/encoder.rs:256`) — a
vertex-input binding offset, a different mechanism from (a)'s descriptor offset, and one the R8
incident did not touch.

**Red first:** ring-overflow is a coded diagnostic (a fixture that overflows asserts the code, not
a truncated draw); **`register_range`'s descriptor write is unit-tested against a recording
`write_geometry_buffer_slot` shim: the written `offset` equals `ring_vertex_base × 64`, the `range`
equals `vertex_count × 64`, the offset is `minStorageBufferOffsetAlignment`-aligned, and a
deliberately misaligned or overrunning request is refused with the code** — this is the gate that
is red before the entry point exists and that the R8 incident lacked; a VB-path unit test asserts
a skinned row's `mesh_id` lane equals its `SkinnedDraw.skin_geometry_slot`, not the mesh's slot; a
raster-path unit test asserts the skinned batch's bind offset equals `ring_vertex_base × 64`.
End-to-end, the silent-zero-read failure mode is caught by G-FRAME (a): the fixture's skinned quad
has no vertex at the origin, so a slot reading zeros paints none of the three named texels.

### S13 — the golden frame
**Files:** `crates/boyko_render/tests/skinned_frame_gpu_golden.rs`.
**WA-1: the `.glb` default `decode` DOES NOT FLIP in rung 1.** The flip (M3) is the next rung and
its precondition is this test having **run and been recorded** on the owner's box, not having
"not failed" on a GPU-less one.

---

## 10. Multithreading model

| datum | ownership | sharing |
|---|---|---|
| `AnimPlayer`, `AnimInstance` | one `&mut` writer (`anim_advance`) | scheduler-enforced |
| pose bank (`ScratchColumn<BoneSoa8>`) | `anim_evaluate` exclusively | rung 1: single thread |
| `LaneJob` column | written by `anim_advance`, read by `anim_evaluate` | `.after` edge |
| skeleton / clip tables | append-only, read-only after load | shared `&` |
| palette staging | one `&mut` writer (`anim_pack_palette`) | scheduler-enforced |

**Rung 1 is single-threaded inside `anim_evaluate`** (§0.1). What G2-SHAPE proves, stated at its
true width *(critique pass 1, NB8: the first draft said the data-race question is "answered
structurally rather than by discipline", which overstates T14)*:

- **Structural (T14):** `evaluate_group` receives a `ScratchSolveView` whose *only* mutable
  surface is `row_ptr(i) -> *mut T`
  (`crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:198-225`); there is no
  `as_mut_slice`, no `Deref<[T]>`, no whole-buffer reborrow — the SP4 structural fix
  (`views.rs:1-19`). The trybuild test proves a whole-buffer slice is **un-typeable** from the
  group view. That is the SP4 class, closed.
- **Discipline (T15 + the `SAFETY` contract):** `row_ptr(index)` accepts ANY `index < len`
  (`views.rs:212-217`), and disjointness across callers is an explicit **caller** obligation in
  the view's own text — "the CALLER guarantees distinct-index access across workers"
  (`views.rs:159-162`, and `row_ptr`'s `# Safety` at `:205-208`). Nothing in the type stops
  `evaluate_group(g)` from computing a row in another group's range; that `row = bank_base +
  group*B + bone` stays inside `[bank_base + g*B, bank_base + (g+1)*B)` is a property of the
  arithmetic, checked by a `debug_assert` on every write and by T15's byte-identity over two
  threads, and it remains discipline.
- **Direction, not design:** a `ScratchRangeView { base, lo, hi }` constructed by
  `ScratchSolveView::range(lo, hi)` and asserting `lo <= index < hi` in `row_ptr` would move the
  group bound into the type. It is a kernel addition, listed in §14, not a rung-1 requirement.

Distinct groups own distinct row ranges, so two concurrent callers over disjoint group ranges
write disjoint rows. The view is `Send + Sync` because the backing base is address-stable across
in-place growth (`views.rs:148-172`).

**No `Mutex`, `RwLock`, `Rc`, `RefCell`, `Box<dyn Trait>` or `HashMap` anywhere on this path.**
No `std::Vec` side store exists to race (RESEARCH P25): every per-substep buffer is a kernel
column.

**Atomics:** none in rung 1. The one atomic the design anticipates (a worker touch counter for G2)
arrives with AK-9.

**Miri:** `crates/boyko_anim/tests/miri_pose_bank.rs` exercises the `row_ptr` writes and the
`ScratchBuildView` refill under Tree Borrows. Invoke as
`cargo +nightly-x86_64-pc-windows-gnu miri test` with `RUSTUP_TOOLCHAIN` unset — a bare `+nightly`
resolves to MSVC on this machine and dies in the linker.

---

## 11. Tests, gates and benchmarks — red first

| # | test | file | what it forbids |
|---|---|---|---|
| T1 | `v3_header_is_96_bytes_and_v2_offsets_are_unchanged` | `crates/boyko_serialize/tests/save_format.rs` | a silent header reshuffle |
| T2 | `blob_load_rejects_fingerprint_mismatch` | `crates/boyko_serialize/tests/blob_region.rs` | a blit of stale bytes |
| T3 | `blob_load_rejects_misaligned_or_truncated_region` | same | an out-of-bounds reinterpret |
| T4 | `default_glb_decode_still_refuses_skins` | `crates/boyko_render/tests/glb_skinned_decode.rs` | WA-1 weakened by accident |
| T5 | `decode_skinned_rest_pose_equals_decode_static_pose_bit_for_bit` | same | a skinned decode that moves the bind pose |
| T6 | `decode_skinned_refuses_morph_targets` | same | a silently dropped blend shape |
| T7 | `decode_skinned_refuses_five_influences` | same | a silently truncated 5th weight |
| T8 | `decode_skinned_refuses_matrix_joint_node` (**conditional on the S3 probe**) | same | a lossy implicit decomposition |
| T9 | `codec_roundtrip_reports_max_and_exceedance` + a RED over-tolerance fixture | `crates/boyko_anim/tests/codec.rs` | RESEARCH P3 — "max error" alone is a misleading metric |
| T10 | `clip_blob_column_stride_is_64` | `crates/boyko_anim/tests/storage.rs` | a regression to `ScratchColumn<u8>` and its 9× tick floor |
| T11 | `palette_pack_outside_snapshot_lags_one_substep` | `crates/boyko_anim/tests/schedule.rs` | the failure `FixedSet` exists to prevent, made visible |
| T12 | `lane_independence` — one instance in lane 0 of a full group, in lane 7 of a group whose other lanes run a DIFFERENT clip, and alone → three **bit-identical** model-row sets | `crates/boyko_anim/tests/determinism.rs` | D15 becoming incidental instead of structural |
| T13 | `anim_scratch_ids_are_pairwise_distinct_and_disjoint_from_physics` | `crates/boyko_anim/tests/storage.rs` | two columns sharing an id → one stagger → the conflict storm |
| T14 | `group_view_cannot_produce_a_whole_buffer_slice` (trybuild compile-fail) | `crates/boyko_anim/tests/compile_fail/` | the SP4 reborrow re-entering |
| T15 | `two_threads_over_disjoint_groups_equal_the_serial_run` — **header states it does NOT measure the engine pool** | `crates/boyko_anim/tests/determinism.rs` | G2's content, without G2's dependency |
| T16 | **G-ORACLE**: `skin_lbs_body::<f32>` == a hand-written scalar LBS reference **byte-for-byte** (`f32::to_bits`); `interp_trs::<f32>` ∘ `inv_bind` == `palette_row_reference` | `crates/boyko_shaderdsl/tests/skin_lbs_oracle.rs` | a GPU/host divergence, on **every** machine (this one cannot skip) |
| T17 | `anim_palette_edsl_sync` / `skin_lbs_edsl_sync` | `crates/boyko_rhi_vulkan/tests/` | a hand-edited generated span; a stale `.spv` |
| T18 | `exact_clock_no_drift_over_10k_substeps` | `crates/boyko_anim/tests/clock.rs` | Q16.16 truncation drift |
| T19 | Miri over the bank writes | `crates/boyko_anim/tests/miri_pose_bank.rs` | UB the `// SAFETY:` comments claim is absent |
| T20 | `soa8_ops_equal_scalar_per_lane` — every `boyko_math::soa8` op `to_bits`-equals the scalar op on each lane over a seeded sweep (±0, denormals, FMA-sensitive products) | `crates/boyko_math/tests/soa8.rs` | a second, divergent implementation of the 8-wide math (§4.2) |
| T21 | `plugin_build_refuses_non_64hz_fixed_timestep` — an `App` at 50 Hz fails `AnimationPlugin::build` with the coded diagnostic | `crates/boyko_anim/tests/clock.rs` | the `/ 64` becoming a silent 78 %/188 % playback rate (§3.4) |
| T22 | `lane_alloc_is_lifo_and_exhaustion_is_refused` — alloc/free/alloc returns the freed lane; the `lane_cap + 1`-th alloc is `Err(LanesExhausted)` with the code, not a panic or a moved region | `crates/boyko_anim/tests/bank.rs` | an ad-hoc `bank_lane` writer; a silent region move (§3.1.1) |
| T23 | `one_shot_partner_frame_stays_inside_the_clip` — a one-shot clip placed LAST in the blob column, sampled at `frame_count − 1`, under Miri (T19's harness) | `crates/boyko_anim/tests/miri_pose_bank.rs` | the `frame + 1` read past `anim_off + anim_len` (§4.1) |
| T24 | `clock_is_exact_at_the_frame_count_bound` — a clip with `frame_count = 65536` (C8's maximum, `span = 2^32`): a looping clip wraps through `tick = u32::MAX → 0` with `frame1` correct on both sides, a one-shot clamps to `u32::MAX` (`frame = 65535`, `frac_q16 = 65535`), and a **reversed** (`speed < 0`) looping clip with a small `frame_count` crosses its wrap to `span − |delta|`, not `2^32 − |delta|` | `crates/boyko_anim/tests/clock.rs` | the `u32` span overflow and the negative-speed wrap the first draft's `wrapping_add` had (§3.4) |
| T19 (fixture added) | `all_idle_group_over_empty_clip_table` — eight idle lanes, zero clips loaded, one bank; under Miri | `crates/boyko_anim/tests/miri_pose_bank.rs` | out-of-bounds pointer arithmetic on an idle lane before the select (§4.3) |
| **G-FRAME** | the golden: 64×64 offscreen, one skinned quad, 2 bones, clip rotating bone 1 by 90° at frame 8 of 16, **at a pinned `overstep_fraction()` the test prints** (§5.3 pin 3). Asserts (a) three named texels (the quad has no vertex at the origin, so a zero-reading geometry slot paints none of them — S12's silent-failure mode); (b) two readbacks — the `PaletteRow` buffer ⑦a wrote against `interp_trs::<f32>` ∘ `inv_bind`, and the skin-cache `Vertex` ring against the CPU LBS oracle **fed those read-back palette rows** — skinned positions **within the §5.3 ULP bound (estimate ≤ 2 ULP, pinned by the first recorded run) until the printer's Pass 2 lands, bit-for-bit after** (division-free, index-ordered — §5.3), normals/tangents within 1 ULP of the oracle's renormalised value; (c) validation messenger == 0 | `crates/boyko_render/tests/skinned_frame_gpu_golden.rs` | a frame that "looks right" |

⚠ **G-FRAME must carry `#[ignore = "gpu: needs a Vulkan device; offscreen skinned-frame readback"]`
and run with `--test-threads=1`.** The `boot_or_skip` idiom the existing goldens use
(`crates/boyko_render/tests/ui_rect_gpu_golden.rs:34-38`) returns early and **passes** on a
GPU-less box — CLAUDE.md names exactly that as a leg reporting green while measuring nothing. The
ignore reason takes the closed-vocabulary prefix so `tests/ignore_reasons_census.rs` classifies it
mechanically. **Assertion (b), not (a), is the load-bearing half**: a texel test alone cannot
distinguish "skinning ran" from "the mesh happened to land there".

**Benchmarks** (criterion, `crates/boyko_anim/benches/`):

- **B1 `evaluate_group`** — 8 lanes × {32, 64, 128} bones, reporting ns/instance and
  ns/bone/instance. Establishes the §4.2 estimate on this box.
- **B2 `decode_pose_cold`** — **must flush or stream ≥ L2 of clip data between samples**
  (RESEARCH P2: clips are touched once per frame, so a benchmark keeping the clip hot measures the
  wrong thing). Also the arm that settles the §3.2 padded-vs-packed rotation estimate; it must
  carry a bit-packed variant or R3 stays an argument.
- **B3 `pack_palette`** — ns/bone/instance including the decompose.
- **0 %-gate** — an empty world through the four systems must be indistinguishable from no plugin.

**`debug_assert!` invariants** (vanish in release):
`parent[i] < i` at load · `frame < frame_count` in `LaneJob` · **`frame1 < frame_count` beside
the partner load** (§4.1) · `LaneJob.bone_count == bank B` ·
`LaneJob.skeleton == the bank's skeleton` · the reconstructed quaternion is unit within 1e-6 ·
`Σ weights == 65535` per skin vertex · `bank_lane < lane_cap` · **every bank write in
`evaluate_group(g)` lands in `[bank_base + g*B, bank_base + (g+1)*B)`** (§10) ·
`chunk_base + ceil(blob_len/64) <= column.len()` · `frame_count <= 65536` at `load_clip` (C8,
re-checked because a file can lie).

---

## 12. Decisions

### 12.A PERF / ARCHITECTURE — taken here, with the numbers

| # | decision | the number that decides it |
|---|---|---|
| R1 | Clip blob backed by `ClipBlobChunk = [u8; 64]`, not `ScratchColumn<u8>` | tick amplification 9.0× → 1.125×; a 76 KB clip's tick floor 608 KB → 9.5 KB, a **64×** reduction, with zero kernel change |
| R2 | Rung-1 `LaneJob` is **16 B**, not the design's 192 B | 176 B of the design's row is unread at rung 1; the row is transient scratch with **no on-disk format**, so widening later costs a recompile. `ClipHeader` (fingerprinted) is frozen at its final 64 B for the opposite reason |
| R3 | Rotation samples padded to **8 B** (`[u16; 4]`), not bit-packed to 6.25 B | packing saves **1 cache line in 20** per whole pose (5 %) and costs a cross-byte extract (~64 cycles/pose) plus the uniform stride that lets `vpmovzxwd` load 8-wide. **Estimate**, re-measured at B2 |
| R4 | Samples **frame-major**, tracks contiguous within a frame | a whole-pose read is ONE contiguous 1280 B run at 64 bones (20 lines); the lerp's second frame is the adjacent run. Per-bone decode is refused (RESEARCH P1) |
| R5 | **Two** GPU passes: palette-interp (TRS) then skin | componentwise affine lerp shears **0.86 %** at a 15°/substep limb and **29 %** at 90°; the split reuses `interp_trs`, an ALREADY generated and ALREADY gated leaf, for 64 extra invocations at the golden and 640 k at a 10 000-instance crowd |
| R6 | The new leaf reuses `InterpBackend`, not `FieldScalar`, and adds **no** trait | LBS + renormalise needs only ops `InterpBackend` already has (`interp.rs:51-100`); `FieldScalar` is the `no_std` physics leaf's and `interp.rs:12-22` records why it must not grow |
| R7 | **No `SHADER-VARIANT-MANIFEST.md` row** — corrects `DESIGN-SPACE` §4 | the manifest is the `-D`-variant registry (`SHADER-VARIANT-MANIFEST.md:3-16`); both shaders are single-variant, and the closest precedent — generated, gated, shipped `interp_instances` — has no row |
| R8 | `AnimPlayer` carries a **`frac64` remainder carry** | `speed × rate / 64` truncates for most `speed`; the carry makes the advance exact with 1 byte and no float, so a clip cannot slowly desynchronise past every single-frame gate |
| R9 | Rung 1 evaluates **serially by an explicit loop**, not by `pool.scope` | the worker-spawned route IS the serial floor on this branch (`thread_pool.rs:424`, AK-9); a `scope` that serialises reads green while measuring nothing. G2-SHAPE (T14/T15) tests G2's content without the pool |
| R10 | Assets are **worldless BOYKOSAV files** via a new blob region (v3) | `save_world` needs a live world (`save.rs:159`); entities-per-bone would fragment archetypes. Zero committed `.boykosav` exist, so the version bump costs nothing in-tree |
| R11 | New crate `boyko_anim`, with `boyko_render → boyko_anim` | keeps the `.glb` parse where the JSON reader is and the palette where the RHI is, acyclically; `boyko_scene` stays the thin floor of the spatial DAG |
| R12 | Skin weights normalised to sum **exactly 65535** at bake, converted by `w * (1.0/65535.0)` on both sides | no renormalising divide, and no `OpFDiv` (2.5 ULP) in the conversion; glTF only *recommends* normalisation, so assuming it would be a silent wrong answer |
| R13 | `SkeletonAsset` is **16 B of bake-knowable fields**; bank coordinates live in a runtime `BankDesc` (id 469) with a **config-fixed `lane_cap`** and a **LIFO free stack** (id 468); exhaustion is a coded refusal | a bake cannot know a world's instance count; the first draft's 32-B record would have bumped `ANIM_FP_SKELETON_ASSET` at the first crowd rung (§3.1.1, critique B2) |
| R14 | The SoA-8 types are **`[f32; 8]` per-lane wrappers in `boyko_math::soa8`**, one implementation, no intrinsics in rung 1 | cross-build bit-identity by construction and the scalar bodies' no-FMA rule inherited; T20 gates lane-wise equality; an intrinsic path is a measured fallback (§4.2, critique B4) |
| R15 | **No per-vertex previous-position ring** in rung 1 | zero readers in the tree (grep), 12 B × vertices of write bandwidth per frame for nothing; it lands with the motion-vector shader (§5.5, critique NB2) |
| R16 | `anim_pack_palette` walks **group-major over `BankDesc`**, no instance query | the instance-major form reads each 6-line bone row 8× per group; group-major reads it once and matches the design's ⑥ traffic figure (§4.4, critique NB3) |
| R17 | The 64 Hz substep is **pinned at plugin build**, not generalised | the general carry needs a `u32` and is a component-format change; rung 1 needs no rate but 64 Hz (§3.4, critique NB6) |

### 12.B VALUES / SCOPE — to the owner

**Still-unruled `DESIGN-SPACE` §13.B items carried as working assumptions:** WA-1 … WA-6 (§0).

New ballots rung 1 raises:

1. **V1 — `FORMAT_VERSION` 2 → 3.** `read_header` rejects any other version outright
   (`load.rs:301-304`). **In-tree cost is zero** (no committed `.boykosav`; tests use the
   constant). If the owner holds out-of-tree saved worlds they die. Alternative: accept `2..=3`.
   **Recommend the strict bump** — it matches the v1→v2 precedent and a lenient reader is a second
   code path forever.
2. **V2 — the gate fixture.** A synthetic 2-bone / 8-vertex `.glb`, committed and deterministic,
   or the real rigged assets in `assets/models/`? **Recommend synthetic for every gate**, plus a
   smoke run over the real assets asserting only "decodes without error" — a real asset's byte
   content cannot be a stable golden, and both files are untracked.
3. **V3 — skinned culling.** Rung 1 culls a skinned instance by its **rest-pose** mesh box
   (`MeshLocalBounds`, `crates/boyko_render/src/mesh_geometry_table.rs:178`), so a pose leaving
   that box **pops**. **Recommend accepting for rung 1** (one instance, in view) and scheduling
   the animated-AABB rung immediately after; `SkeletonBone.radius` is already baked for it.
4. **V4 — crate placement.** `boyko_anim` as a new member, or folded into `boyko_scene`?
   **Recommend the new crate** (§1).
5. **V5 — the extra dispatch.** R5 buys exactness with one more dispatch per frame. An owner who
   prefers one pass accepts ≤1 % shear at typical rates. **Recommend two passes.**

---

## 13. Dead-datum ledger — declared, not discovered

Rung 1 knowingly bakes three things nothing reads at runtime. Each is listed here so a later audit
finds a declaration rather than a surprise, and each carries a gate so it cannot be silently wrong:

| datum | why it is baked now | its rung-1 gate | its consumer |
|---|---|---|---|
| `SkeletonBone.radius` | freezes the 96-B row so the AABB rung is not a fingerprint change | a bake test: a bone that skins nothing has radius 0; an influencing bone has radius > 0 | the animated-AABB rung |
| `BoneNameHash` column | needed by retargeting and socket-by-name | **consumed at BAKE** in rung 1 (contract C4 verifies a channel targets the bone it names) | L4 sockets, retargeting |
| `ClipHeader.flags` bit1 + the root-motion offsets | keeps the header at exactly one cache line | a bake test asserts they are 0 in codec v1 (contract C7) | L4 root motion |

---

## 14. Unverified / open

- Every per-instance and per-group cost in §4 is an **estimate** anchored on ACL's iPad figures
  [S/D]. **This box has not been measured**; B1/B2/B3 are what settle them.
- R3 (padded rotation samples) rests on a cycle-count comparison, not a measurement. B2's arms
  must include a bit-packed variant or R3 stays an argument.
- The `matrix`-vs-TRS question on joint nodes (§7.3) is **unmeasured until S3's first action**,
  and the refusal must not ship before that probe reports.
- Whether the decompose in `anim_pack_palette` (§4.4) is cheap enough at crowd scale is unmeasured;
  at rung 1's single instance it is 64 decomposes per substep and cannot matter.
- Whether an exclusive-system placement of `anim_evaluate` would put a future `scope` on the
  dispatcher route (not serial in the KE16 record) is **unmeasured** and is not claimed — the
  design already lists it (`DESIGN-SPACE` §14) and rung 1 does not depend on it either way.
- The 45 KB rung-1 working set of §3.3 is size arithmetic over the bank and the clip stream, not a
  cache-miss measurement.
- Whether the compiler autovectorises the `[f32; 8]` per-lane loops of `boyko_math::soa8` into
  8-wide AVX2 (`vpmovzxwd` / `blendv` as §4.2 expects) is **unmeasured**; B1 plus an assembly look
  decide, and the intrinsic path (behind T20) is the fallback, not the default.
- ~~Whether `DeferredEcsMaster` gives an `on_remove` hook access to the `AnimBanks` resource~~ —
  **answered at HEAD** (verifier pass 2): `DeferredEcsMaster::resource_mut` at
  `hooks/deferred_master.rs:105-111`; the direct free is decided in §3.1.1 and the staged
  alternative is dropped.
- **The eDSL printer's Pass 2 (`precise` on generated temps) is a PREREQUISITE owned by the eDSL
  lane, not by this campaign, and it has not landed** (`crates/boyko_shaderdsl/src/emit/mod.rs:8-14`,
  `emit/shaders.rs:76-77`). Until it does, G-FRAME (b)'s position clause is the ULP-bounded golden
  of §5.3, not bit-for-bit. Whether a hand-written `precise` at the call site outside the sentinels
  propagates backward through the inlined generated body far enough to decorate every op in the
  span is **unverified** (§5.3 names how it would be verified — a `NoContraction` census over
  `spirv-dis` output — and does not rely on it).
- A range-bounded `ScratchRangeView` that would move the group bound of §10 into the type is a
  kernel addition, **not designed here**; rung 1 keeps the bound as a `debug_assert` + T15.
- The `k`-groups-per-task rule for AK-9's dispatch line (§0.1) uses `t_group` from B1, which does
  not exist yet; until it does the rule's second clamp is an **estimate** from §4.2.
- This document's `file:line` anchors are **not machine-checked**
  (`tests/internal_docs_anchors.rs:231`). Adding `docs/animation/` to `GATED_DOCS` is worth doing
  **after** S13, when the cited code has stopped moving; doing it now would gate a document against
  a tree the campaign is about to change.

---

## Critique log — pass 1 (2026-09-10)

Every finding of the architecture-critic's first pass, and what this revision did with it. "Fixed"
means the document now says the corrected thing at the section named; nothing was refuted.

| # | finding (short) | action | where |
|---|---|---|---|
| B1 | `tab.headers` is an eighth column with no scratch id | **Fixed.** Id 470 `SkeletonAsset`, plus 469 `BankDesc` and 468 `FreeLane` that B2's fix needs; band note now says ten ids used, reserved `460..467`; T13 covers ten | §2.1, §6.3, S2 |
| B2 | `SkeletonAsset` carries runtime bank coordinates; `bank_lane` has no writer; no lane allocator | **Fixed.** `SkeletonAsset` shrunk to 16 B of bake-knowable fields; runtime `BankDesc` (32 B) with a config-fixed `lane_cap`, `AnimBanks::{register_skeleton, alloc_lane, free_lane}`, the design's tombstone + LIFO free stack, an `on_remove` hook as the freer, exhaustion a coded refusal; growth past the cap listed under Out | §3.1, **§3.1.1 (new)**, §0, §6.2, §6.3, R13, T22 |
| B3 | "no shader change" names no mechanism; VB slots are per mesh, the ring is per instance | **Fixed.** Claim narrowed to shader *code*; the three host-binding changes — per-instance geometry slot over the ring with offset/range, `SkinnedDraw` override of the instance ring's `mesh_id` lane, `DrawBatch.vertex_source` + bind offset for raster — are now S12's named content with anchors | §5.5, S12 |
| B4 | The 8-wide SoA layer does not exist and the plan names no crate or intrinsic decision | **Fixed.** Decided: `boyko_math::soa8` as `[f32; 8]` per-lane wrappers, one implementation, no intrinsics in rung 1 (cross-build bit-identity by construction), T20 as the agreement gate; the intrinsic path listed as measured fallback in §14 | §4.2, S2, R14, T20, §14 |
| NB1 | Stagger column not computed from the cited formula; commit fact cited at the wrong site | **Fixed.** `477 → 1856` and down by 64 per id; the three-sub-region commit now cited to `grow_rows` (`component_pool.rs:494`) with `constants.rs:200-212` as the reservation layout | §2.1, §2.2 |
| NB2 | 12-B previous-position ring is a dead datum with no reader | **Fixed.** Dropped from rung 1 (grep evidence recorded); lands with the motion-vector shader; Out list updated | §5.1, §5.5, §0, R15 |
| NB3 | `anim_pack_palette` is an 8× read-amplifying gather | **Fixed.** Group-major walk over `BankDesc`, bank read once per group, no `Query<&AnimInstance>`; S9's gate counts bank reads | §4.4, §8, S9, R16 |
| NB4 | One-shot clamp reads `frame + 1 == frame_count` | **Fixed.** Partner frame derived per lane by a `select` on the looping bit; `debug_assert!(frame1 < frame_count)`; the once-per-loop wrap stated; T23 under Miri | §4.1, §3.3, §11 |
| NB5 | `frame_count: u32` cannot be addressed by the Q16.16 tick | **Fixed.** Contract C8 `1 <= frame_count <= 65536`, `BakeError::ClipTooLong`, re-checked at `load_clip`; header comment says so | §3.2, §6.4, §11 |
| NB6 | `/ 64` hard-codes the 64 Hz substep | **Fixed.** Pinned at `AnimationPlugin::build` against `FixedTime::timestep()` with a coded refusal; T21; the general carry recorded as a component-format change and therefore deferred | §3.4, S8, R17, T21 |
| NB7 | G-FRAME (b) does not pin division, contraction, or `alpha` | **Fixed.** Reciprocal-multiply form on both sides, `precise` accumulators (NoContraction) with T17 re-run per re-DXC, `alpha` pinned and printed; normals/tangents excluded from bit-for-bit and given a 1-ULP clause | §3.6, §5.3, §11, R12 |
| NB8 | §10 overstates what G2-SHAPE proves | **Fixed.** Split into what T14 proves structurally and what stays caller discipline (the view's own SAFETY text quoted); a `debug_assert` on every bank write; the range-bounded view named as a direction in §14 | §10, §11, §14 |
| NB9 | Idle lanes have no stated load rule | **Fixed.** Pointer-`select` to a static zero 20-B record + masked store; branchy form refused; T12's "alone" case named as the cover | §4.3, §3.3 |
| NB10 | Bake contracts omit the non-joint-ancestor fold | **Fixed.** Contract C9 (fold rule for the root and for a non-joint node between joints; static because C4 refuses animated non-joints), a third synthetic fixture that fails without the fold, and the S3 probe now reports the ancestor shape of both real assets | §6.4, §7.3 |
| NB11 | `AnimPlayer` sized by the scratch rule but is a component | **Fixed.** `format_version = 1` declared via the derive key; L1's `slots: [PlaySlot; 4]` named as the bump to 2 | §3.4 |
| NB12 | "the dispatch line and nothing else" glosses the KE16 floor | **Fixed.** The ≥ 10 µs floor cited (worktree path stated), the D5 `groups_per_task` rule adopted for the dispatch line, `t_group` from B1 flagged as an estimate until measured | §0.1, §14 |

**Open after this pass:** nothing from the critic's list. New unverified items this revision
introduced are in §14 (autovectorisation, `DeferredEcsMaster`'s hook surface, the range view,
`t_group`).

---

## Critique log — pass 2 (2026-09-10)

The verifier re-opened every anchor of the pass-1 revision at HEAD `ed0bed45`. Fifteen of sixteen
pass-1 repairs held; one did not, and five held with a wording or edge defect. Each row names the
finding, what this revision did, and where. Nothing was refuted; one pass-1 repair (NB7's
contraction pin) is **withdrawn and rewritten**, which is recorded as such rather than as a fix.

| # | finding (short) | action | where |
|---|---|---|---|
| NB7 (contraction) | **DID NOT HOLD.** Pin 1 said the `Emit` printer declares accumulators `precise`; the printer is documented "Pass 1: NO `precise`" (`emit/mod.rs:8-14`, `emit/shaders.rs:76-77`) and the shipped `interp_trs` span has zero `precise`. ⑦a's shared non-`precise` span sat outside the pin while the position claim passed through it. The mechanism sentence ("DXC contracts by default") contradicted the precedent it cited (`cluster_cull.hlsl:132-134`: decided below the `.spv`) | **Withdrawn and rewritten** (option (a)). Pass 2 named as a prerequisite owned by the eDSL lane, per-leaf, not S11's file. Until it lands: T16 stays byte-for-byte (both sides Rust `f32`); G-FRAME (b)'s GPU position clause is a **ULP-bounded golden** on the `ddgi_probe_gi_resolve.rs:538-586` pattern (estimate ≤ 2 ULP, sign-aware bit distance, pinned by the first run); the claim is **scoped at the palette boundary** — ⑦b judged given the read-back palette rows, ⑦a judged separately. Mechanism corrected: the tree carries two accounts (`cluster_cull` vs `ddgi_resolve.hlsli:136-137`), the pin depends on neither, `NoContraction` forbids it at every level. Option (b) shown non-viable (the driver's choice is unobservable from the bytes). A call-site-`precise` candidate listed as unverified with its `spirv-dis` census | §5.3 pin 1, §5.3 closing paragraph, §5.4, S11, G-FRAME row, §14, status header |
| B3 / S12 | S12 said `register` is called "with the instance's `offset`/`range`"; `register` takes no offset — it hardcodes `0` and `vertex_buffer.size` (`mesh_geometry_table.rs:667-676`), and `:646-666` records a silent-zero-read incident on a nonzero offset in this exact call plus the engine-wide `offset: 0` convention | **Fixed.** (a) is a NEW entry point `register_range`, stated as the first nonzero buffer-relative offset this table writes; why it is correct where R8's was wrong (buffer-relative vs memory-bind); the `minStorageBufferOffsetAlignment` rule (VUID-…-00328) on the ring allocator; `register`'s comment amended to name the sub-range path; the per-instance-`VkBuffer` alternative refused with reasons; red-first gate: a recording-shim unit test on the descriptor write (offset, range, alignment, overrun refusal) plus G-FRAME (a)'s fixture geometry catching a zero read | §5.5 row (a), S12 |
| S14 `DeferredEcsMaster` | The question was answerable at HEAD: `resource_mut` at `deferred_master.rs:105-111` | **Answered; direct free decided.** Staged `FreeLane` staging column dropped (it would have been an eleventh id and a second free path); `None` from `resource_mut` is a coded diagnostic; item struck from §14; S2 no longer "verifies" it | §3.1.1, S2, §14 |
| NB5 edge | At `frame_count = 65536`, `frame_count << 16 = 2^32` overflows `u32` | **Fixed.** The wrap is computed in `i64` (`span = frame_count << 16` fits; `rem_euclid` for looping, `clamp(0, span − 1)` for one-shot; the `as u32` store is the identity), so at the bound the loop case is the natural `u32` modulus and the clamp bound is `u32::MAX` by arithmetic, no special case. The same form removes a negative-`speed` wrap hazard the first draft's `u32` `wrapping_add` had. T24 at the bound and across a reversed wrap | §3.4, T24 |
| NB9 caveat | `live_ptr` computed from `clip == u32::MAX` before the select — `ptr::add` out of bounds is UB under Miri even if never dereferenced | **Fixed.** Select on the header index and the byte offset first, `wrapping_add` for the one pointer arithmetic, pointer select last, dereference after; the header read routed through a `static IDLE_CLIP_HEADER` so an all-idle group over an empty clip table touches no row; T19 gains that fixture | §4.3, T19 row |
| NB6 anchor | `app.rs:68-70` is a doc comment linking `App::set_fixed_timestep`; the method is at `:457` | **Fixed.** Anchor now `:457`, with `:68-70` named as the mention | §3.4 |
| NB3 nit | S9's "B3's bank-read count" meant benchmark B3, colliding with critique item B3 | **Fixed.** "benchmark B3 (`pack_palette`, §11 — not critique item B3)" | S9 |

**Verifier items that held and needed no edit** (recorded so the log is complete): B1, B2, B4,
NB1, NB2, NB4, NB8, NB10, NB11, NB12; NB7's division and `alpha` pins.

**Open after this pass:** the eDSL printer's Pass 2 (a prerequisite this campaign does not own —
§14); whether call-site `precise` propagates through the inlined generated body (§5.3, §14,
unverified); the ULP bound for G-FRAME (b) is an estimate until the first recorded run pins it
(§5.3). Nothing in this pass was compiled or run; every file:line above was re-opened by reading.

**Verifier pass-2 nits, closed by the orchestrator the same day (2026-09-10):** the preamble's count
("four" → "five" held with a defect); the bare `geometry_bindless.rs` path in §5.5 row (a) now carries its
crate (`crates/boyko_rhi_vulkan/src/`); `InterpBackend::div` cited at `interp.rs:65` (the `fn`, not its doc
line); and the idle row's `frame` is DECIDED — §4.1's reset is a typed fill of `LaneJob::IDLE`, so an idle
lane's `frame` is 0 and §4.3's pre-select arithmetic cannot overflow in debug (§4.1, §4.3 amended).
Verifier verdict after these: every substantive item HOLDS at `ed0bed45`; no bit-for-bit GPU-position claim
survives without a landed mechanism.
