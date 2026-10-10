# Animation — the comparative survey (research record)

> Status: research record for the animation campaign, 2026-09-10. Companion: the design for THIS
> engine is [`ANIMATION-DESIGN-SPACE.md`](ANIMATION-DESIGN-SPACE.md). Nothing here is implemented;
> the tree holds **no animation code** (§0).
>
> **Provenance discipline.** Every claim about another engine carries a tag: **[S]** source code
> read, **[D]** official docs / talk / paper, **[B]** blog / marketing (recorded, not relied on),
> **[T]** a file in this tree re-opened by the author of this document. External URLs were opened by
> the five research lenses of this session and are carried through verbatim; the in-tree
> `file:line` citations were **re-opened by the architect** on branch `feat/multi-paradigm-render`
> at commit `128233be` and corrected where a lens was off by a line (e.g. the fixed-timestep
> constant is at `fixed_time.rs:21`, not `:19`). Anything neither source-read nor doc-read is
> marked **UNVERIFIED**. This directory is NOT in `tests/internal_docs_anchors.rs`'s `GATED_DOCS`
> (`tests/internal_docs_anchors.rs:231`), so no anchor below is machine-checked — treat a line
> number here as "true on 2026-09-10", not as a live fact.

## 0. The tree today — what is true before any design (all [T])

| Fact | Where |
|---|---|
| The glTF loader's default `decode` **refuses** `animations`, `skins` and morph `targets` as a hard `AssetError`; the module doc names reading a skinned file while ignoring `skins` "exactly the silent half-decode this subset exists to forbid" | `crates/boyko_render/src/loaders/glb.rs:17-21` (refusal list), `:49-56` (the half-decode sentence), `:1092-1100` (the row check) |
| The by-name entry `GlbMeshLoader::decode_static_pose` tolerates the three deformation rows and **drops joints, weights and blend-shape deltas**, returning the rest shape | `crates/boyko_render/src/loaders/glb.rs:1027` (fn), `:1016-1026` (doc) |
| A skinned node is placed at **identity** per glTF §3.7.4 — correct only once joint matrices exist | `crates/boyko_render/src/loaders/glb.rs:777-782` |
| Morph `POSITION` data are **deltas** against the neutral shape | `crates/boyko_render/src/loaders/glb.rs:834-840` |
| `Vertex` is a 64-byte `#[repr(C)]` record (position/normal/color/uv/tangent) with **no joint or weight lanes**; `VERTEX_STRIDE == 64` is pinned by a const assert | `crates/boyko_render/src/mesh.rs:83`, `:103-104` |
| `MeshData { vertices: Vec<Vertex>, indices: Vec<u32> }` | `crates/boyko_render/src/mesh_data.rs:23` |
| `boyko_math` has `Quat` (`quat.rs:23`), `Mat3` (`mat.rs:28`), `Mat4` (`mat.rs:256`), `Affine3A` (`affine.rs:24`, `#[repr(C, align(16))]`); **`Quat` has `normalize` (`quat.rs:140`) but no `slerp` / `nlerp` / `dot`** — there is no quaternion blend primitive | `crates/boyko_math/src/{quat,mat,affine}.rs` |
| `state_chart!` exists only in design documents (`docs/aether-v2/MACHINES.md`, `CONSTRUCTS.md`, `OPEN-QUESTIONS.md`, …) — `grep -rn state_chart crates/` is empty. The Aether v1 `machine` is **global** (`State<S>` / `NextState`, one system per route) | `crates/aether_lang/src/expand.rs:135-148`, `:1204-1240`; `docs/OPEN-QUESTIONS.md:950-965` |
| The physics crate has **no joints** (`Hinge|BallSocket|Revolute|Joint` over `crates/boyko_physics/src`: zero hits) and the workspace has **no ray-cast / shape-cast API** (only a render-test comment mentions `raycast`); the only spatial query is `sample_sdf` | `crates/boyko_physics/src/sdf_query.rs` |
| `Kinematic` is an EnableTag whose pose is NOT advanced by the solver — "kinematic MOTION is an intentional deferral, not built yet" | `crates/boyko_physics/src/components.rs:66-74`; `crates/boyko_physics/src/solver/soft_step.rs:884-890` |
| A simulated dynamic body must be a hierarchy **root** (debug tripwire) — parented dynamics are unsupported in v1 | `crates/boyko_physics/src/scene_sync.rs:192-212` |
| `FixedSet::{Gameplay, Snapshot}` with `Snapshot.after(Gameplay)` exists solely to prevent "a permanent one-substep lag"; a Fixed system in neither set is unordered relative to the seam | `crates/boyko_scene/src/sets.rs:1-41` (enum at `:35`); wired in `crates/boyko_app/src/plugins.rs:73-74` |
| The fixed timestep defaults to exactly 64 Hz = 15 625 000 ns | `crates/boyko_ecs/src/ecs/core/time/fixed_time.rs:21` |
| `GpuTransform3D` is the ONE production dense component: a 96-byte prev/curr TRS pair, single prev-shuffle site per substep, lerped on the GPU at `alpha` by the B2 compute pre-pass | `crates/boyko_render/src/gpu_transform3d.rs:85`; `crates/boyko_render/src/gpu_transform_pack.rs:73`; `crates/boyko_rhi_vulkan/shaders/interp_instances.comp.hlsl` (header) |
| The B2 pre-pass body is eDSL-generated and gated by `interp_edsl_sync` (text match + fresh DXC byte-compare under the frozen recipe) | `crates/boyko_rhi_vulkan/tests/interp_edsl_sync.rs:1-22`; `crates/boyko_shaderdsl/src/scalar.rs:36` |

The brief's premise "state machines already exist" is therefore **false of the code** and true only
of the design corpus; the design document takes the per-entity `machine` (campaign rung R5,
`docs/aether-v2/CAMPAIGN.md:56`) as a *prerequisite*, not a given.

## 1. Systems surveyed

ozz-animation (C++ SoA reference runtime) · Latios Kinemation (Unity DOTS, open source) · Unity's
own `com.unity.animation` (never shipped) and Entities Graphics deformation · Rukhanka (commercial
DOTS replacement) · Bevy `bevy_animation` (Rust ECS) · Godot 4 `AnimationMixer` · Unreal Engine
(classic AnimGraph, UAF/AnimNext, Pose Search, ACL, Skin Cache, Budget Allocator) · ACL (the
compression layer under UE and Kinemation) · Jolt / PhysX / Box2D / Avian for the physics coupling
· the papers (Bollo 2018, Holden et al. 2017/2020, Kavan 2007/2008, Aristidou & Lasenby 2011,
Witkin & Popović 1995). flecs and EnTT ship no animation module — searches of the flecs docs and
EnTT wiki surfaced none (negative claim, **UNVERIFIED** as absolute).

## 2. Data model — per system

### 2.1 Pose representation (the biggest fork)

| System | Layout | Source |
|---|---|---|
| **ozz** | SoA, **4 joints per `SoaTransform { SoaFloat3 translation; SoaQuaternion rotation; SoaFloat3 scale }`**; `num_soa_joints() = (num_joints()+3)/4`; `kMaxJoints = 1024`; parents `int16_t`. `LocalToModelJob` takes SoA in, emits AoS matrices — one transpose at the skinning boundary | [S] `ozz/base/maths/soa_transform.h`, `ozz/animation/runtime/skeleton.h` (raw.githubusercontent.com/guillaumeblanc/ozz-animation/master/include/…); [D] guillaumeblanc.github.io/ozz-animation/documentation/animation_runtime/ |
| **Kinemation (optimized skeleton)** | AoS `TransformQvvs` per bone (quat + float3 position + float3 stretch + uniform scale + `context32`), **6 transforms per bone** in ONE `DynamicBuffer` = {root-space, local-space} × {current, previous, two-ago}, addressed by rotating base offsets; asserts `requiredBones * 6 == m_boneTransforms.Length` | [S] Latios-Framework/master/Kinemation/Components/AnimationComponents.cs, OptimizedSkeletonAspect.cs |
| **Kinemation (exposed skeleton)** | one **entity per bone** (`BoneIndex` short + `BoneOwningSkeletonReference`); root holds a `BoneReference` buffer | [S] Kinemation/Components/SkeletonComponents.cs |
| **Bevy** | **no pose buffer**: `AnimationCurves = HashMap<AnimationTargetId, Vec<VariableCurve>, NoOpHash>`, curves are `Box<dyn AnimationCurve>`, sampled values are written into each bone entity's `Transform`; `AnimationTargetId` = first 16 bytes of a blake3 hash of the bone name path | [S] bevy/main/crates/bevy_animation/src/lib.rs |
| **Godot 4** | per-track resolved `TrackCache*` (`TrackCacheTransform{skeleton_id, bone_idx}`, `TrackCacheBlendShape`, `TrackCacheValue`, …) in `AHashMap<TrackCacheID, TrackCache*>` + `AHashMap<Ref<Animation>, LocalVector<TrackCache*>>`; blend accumulates into the cache, one apply pass | [S] godot/master/scene/animation/animation_mixer.h |
| **UE classic** | `FCompactPose` over an `FBoneContainer` of LOD-filtered "required bones"; bone arrays double-buffered (`BoneSpaceTransforms`/`Cached…`, `ComponentSpaceTransforms`/`Cached…`) swapped by `SwapEvaluationContextBuffers()` | [B] cloud.tencent.com/developer/article/2347202 (third-party source walkthrough) — **UNVERIFIED against Epic headers**, whose API pages rendered empty to every fetch |
| **UE UAF/AnimNext** | traits split into `FSharedData` (read-only, cooked) / `FInstanceData` (mutable); three-tier `FNodeTemplate` / `FNodeDescription` / `FNodeInstance`; evaluation = `FAnimNextEvaluationTask` list on an `FEvaluationVMStack`, not recursive virtual calls; RigVM bytecode underneath | [B] remremremre.github.io/posts/My-understanding-of-Unreal-Animation-Framework-in-5.6/; [D] dev.epicgames.com/…/API/PluginIndex/UAF ("a framework for defining functional data flow for animation systems", Experimental in 5.8); [D] dev.epicgames.com/…/API/Plugins/RigVM |

The two mature ECS answers (Kinemation, UE classic) both keep **history** in the pose store
(previous / two-ago; current / cached) — motion vectors and inertialization need it. Bevy is the
anti-pattern for this codebase on every axis Principle 0/1 names (`HashMap`, `Box<dyn>`, hashed-name
binding, a `commit(entity_mut)` random write per target).

Open source conflict, recorded: ozz's `animation.h` says tracks follow the skeleton's
**breadth-first** joint order [S] while the runtime documentation page says joints are stored
**depth-first** [D]. Both orders satisfy "parent index < child index", which is the only property
the design below relies on. Do not copy either claim without reading `skeleton.h`.

### 2.2 Clip compression

| System | Format | Source |
|---|---|---|
| **ACL** | uniform sampling; **16-key-frame segments** — `compression_segmenting_settings { ideal_num_samples = 16; max_num_samples = 31; }` **[S]** `includes/acl/compression/impl/segment_context.h`, split in `segment.transform.h` (`split_samples_per_segment`) **[S]**; range reduction per clip and **per segment** — `normalize_clip_streams` / `normalize_segment_streams`, `extract_segment_bone_ranges` ("Segment ranges are always normalized and live between [0.0 ... 1.0]") **[S]** `impl/normalize.transform.h`; constant tracks "compacted to a single sample", samples "sorted by time and by track" **[D]** `docs/algorithm_uniformly_sampled.md` (which says nothing about segment size — the critic's finding, confirmed on re-opening); variable bit rate per track type; **four linear streams** (constant track values, clip range, segment range, animated segment) so "a full key frame typically fits within a handful of cache lines (2-5)" — **[B]** only | [S] raw.githubusercontent.com/nfrechette/acl/develop/includes/acl/compression/impl/{segment_context.h, segment.transform.h, normalize.transform.h}; [D] github.com/nfrechette/acl/blob/develop/docs/algorithm_uniformly_sampled.md; [B] nfrechette.github.io/2016/11/10/anim_compression_uniform_segmenting/, …/2017/09/10/acl_v0.4.0/, …/2018/05/11/acl_decompression_baseline/ |
| ACL, measured | Paragon: 6 558 clips, 4 276.11 MB raw → **205.69 MB (20.79:1)** vs UE 4.15's 496.24 MB (8.62:1); compression 1 h 53 m on 11 threads. `decompress_pose` **0.910 / 2.304 / 28.837 µs** for `104_30` / `Trooper_1` / `Trooper_Main` on an iPad Pro 10.5 @2.39 GHz, cold cache, independent of playback direction and rate | [B] nfrechette.github.io/2017/12/05/acl_paragon/; [S/D] github.com/nfrechette/acl/blob/develop/docs/decompression_performance.md |
| ACL adoption | default codec in **UE 5.3+**, maintained by Epic in their fork; >30 % smaller than UE 5.2 defaults; Kinemation wraps it through a native plugin (`SkeletonClip { BlobArray<byte> compressedClipDataAligned16; }`, `SkeletonClipSetBlob { short boneCount; … }`) | [B] nfrechette.github.io/2023/09/17/acl_in_ue/, acl-ue4-plugin README; [S] Kinemation AnimationComponents.cs |
| **ozz** | translations / scales as 3 half floats; rotations 3×16-bit with the largest component dropped and premultiplied by √2 (+41 % quantization range); key frames in one array per channel **sorted by time then track**; since 0.15 a 16-bit previous-key offset + group-varint "iframe" snapshots (default every ~10 s) for O(1) seeking; backward playback explicitly not optimized | [D] ozz animation_runtime doc; [S] ozz `runtime/animation.h`, `runtime/sampling_job.h` |
| **Bevy** | none — boxed curve trait objects sampled per target | [S] bevy_animation/src/lib.rs, animation_curves.rs |

### 2.3 Per-instance sampling state

ozz's `SamplingJob::Context` is a per-instance, non-copyable cache of decompressed key frames and
per-track cursors; it must be `Resize()`d to the skeleton and `Invalidate()`d on clip change, and a
shared context across entities or clips silently serialises or corrupts sampling [S] ozz
`sampling_job.h`; [D] runtime doc. ACL's uniform-sampling layout has **no cursor** — decompression
is independent of direction and rate [S/D] `decompression_performance.md`. This is the single most
consequential format fact for a parallel evaluator: uniform sampling makes every task **pure**.

### 2.4 Skeleton / skin binding

Bevy: `SkinnedMesh { inverse_bindposes: Handle<SkinnedMeshInverseBindposes>, joints: Vec<Entity> }`,
`SkinnedMeshInverseBindposes(Box<[Mat4]>)`, `JointIndex = u16` [S] bevy_mesh/src/skinning.rs — a
scattered per-entity gather to build the palette. Kinemation: bind poses + hierarchy in blob assets,
bone count from `hierarchyBlob.parentIndices.Length` [S] OptimizedSkeletonAspect.cs. Jolt's pose
currency across the animation/physics boundary is a root offset plus a raw
`const Mat44 *inJointMatrices` — a flat array, not a graph [D]
jrouwe.github.io/JoltPhysicsDocs/5.0.0/class_ragdoll.html.

### 2.5 Graph / blend state

- **Bevy**: `DiGraph<AnimationGraphNode, (), u32>` (petgraph) with `Clip | Blend | Add` nodes, per-node
  `mask: AnimationMask` (u64) and `weight`; precomputed `ThreadedAnimationGraph { threaded_graph
  (postorder), sorted_edge_list_ranges, sorted_edges, computed_masks }` plus per-target subgraphs so a
  target evaluates only the nodes that touch it; evaluation is a blend-register stack machine;
  graphs serialise as `.animgraph` / `.animgraph.ron` with `Handle<AnimationClip>` replaced by
  `AssetPath` [S] bevy_animation/src/graph.rs, animation_curves.rs. **No state machine in core**
  [S]; the third-party `bevy_animation_graph` adds FSM/IK nodes, Bevy-master only [B]
  github.com/mbrea-c/bevy_animation_graph.
- **Godot**: `AnimationTree` node graph evaluated per process frame, each node receiving a time and a
  **per-track weight vector** [D] deepwiki.com/godotengine/godot/4.9-animationtree-and-animationnodes;
  `AnimationMixer` (4.2) owns libraries and applies results [D]
  godotengine.org/article/migrating-animations-from-godot-4-0-to-4-3/.
- **Unity `com.unity.animation`**: a DataFlowGraph (`MixerNode`, `NormalizedMixerNode`,
  `NormalizedRootMotionNode`) that **never shipped** — Q4 2022: "at least another year", gated on a
  stable Entities 1.0 [D] github.com/Unity-Technologies/Unity.Animation.Samples; [B]
  discussions.unity.com/t/animation-status-update-q4-2022/903925.
- **Kinemation**: a baked Mecanim Animator runtime (`MecanimAspect`, v0.8.0, 2023-09-23); masked
  sampling with `ReadOnlySpan<ulong>` bone masks (v0.10.0); bone-mask blobs from Avatar Masks
  (v0.12.0) [D] Latios-Framework-Documentation …/CHANGELOG.md.
- **UE classic**: Update (logic/weights) then Evaluate (transforms); per-node `*_AnyThread` entry
  points; game-thread data **copied in** via Property Access with an explicit Call Site enum [D]
  dev.epicgames.com/…/property-access-in-unreal-engine; [B] the tencent walkthrough
  (**UNVERIFIED** names). Blend catalogue: Layered Blend per Bone (branch filter / Blend Mask),
  Blend Bone by Channel, additive vs mesh-space additive, all under the LOD Threshold system [D]
  dev.epicgames.com/…/animation-blueprint-blend-nodes-in-unreal-engine. Sync Groups: leader /
  follower, marker sync uses only markers **common to all** members, else length sync [D]
  dev.epicgames.com/…/animation-sync-groups-in-unreal-engine.

### 2.6 Transitions — inertialization

Bollo (GDC 2018, Gears of War 4): a cross-fade evaluates **both** states during the transition,
"effectively doubling the animation evaluation cost"; inertialization evaluates ONE pose and decays
the recorded source−target offset as a post-process [D] gdcvault.com/play/1025165 (also
/1025331); [B] gamedeveloper.com summary. Per-joint state = offset + offset velocity decayed by a
critically damped spring parameterised by half-life, one `exp` per update shared across joints when
half-life and dt are equal [B] theorangeduck.com/page/spring-roll-call. UE: "less than 0.4 seconds
is best", "when poses are extremely different, do not use inertialization" [D] blend-nodes page.
**Dead Blending** (UE 5.3+, experimental): extrapolate the outgoing pose with **per-axis half-lives**
(three extra floats per rotation axis), decay `x * fast_negexp((0.693147*dt)/(halflife+eps))`,
half-lives chosen from `src_dst_diff / max_mag(src_vel, eps)` clamped — a fixed decay constant is not
sufficient [B] theorangeduck.com/page/dead-blending-node-unreal-engine, /page/dead-blending.
Kinemation ships it as `OptimizedBoneInertialBlendState` (AoS buffer element) with
`InertialBlendingTimingData` precomputing t, t², t³ once per skeleton [S]
Kinemation/Components/InertialBlendingComponents.cs.

### 2.7 Motion matching

- Reference implementation (orangeduck): frame-major `array2d<vec3> bone_positions` /
  `array2d<quat> bone_rotations` (nframes × nbones) + velocity arrays + `bone_parents`; a normalised
  feature matrix (feet position/velocity, hip velocity, 2-D trajectory positions and directions at
  three future times ≈ frames 20/40/60) with per-column `features_offset` (mean) and
  `features_scale` (stddev / weight); search = hierarchical AABB prune (64-frame coarse, 16-frame
  fine), squared-distance accumulation with early exit, **no explicit SIMD** [S]
  raw.githubusercontent.com/orangeduck/Motion-Matching/main/database.h. (The exact feature
  dimensionality — 30 columns — is **UNVERIFIED**.)
- UE Pose Search (5.4+): Schema = ordered channels (Trajectory, Pose, Velocity, Position, Heading,
  Phase, Group, Padding) → one flat weighted feature vector; Database indexed at a Sample Rate with
  optional Permutations (`PermutationTime = SamplingTime + PermutationTimeOffset +
  PermutationIndex / PermutationSampleRate`); search modes Brute Force / **PCAKDTree** (then full
  cost on `KNNQuerryNumNeighbors` candidates because the tree cost is approximate) / experimental
  VPTree; Continuing Pose Cost Bias, Looping Cost Bias, Pose Pruning Similarity Threshold; output
  through a blend stack with Use Inertial Blend; `Notify Recency Time Out` (default 0.2 s)
  de-duplicates re-fired notifies; **Chooser** tables gate which database is searched [D]
  dev.epicgames.com/…/motion-matching-in-unreal-engine, …/game-animation-sample-project-in-unreal-engine,
  …/dynamic-asset-selection-in-unreal-engine.
- Origins: Clavet, Ubisoft, GDC 2016 [D] gdcvault.com/play/1023280; TLOU2 [D]
  gdcvault.com/play/1027118 (framed as "initial joy, later frustration and ultimate fear").
- **Learned Motion Matching** (Holden, Kanoun, Perepichka, Popa, SIGGRAPH 2020 / TOG 39(4)):
  decompressor / stepper / projector networks replace the database; >100 MB → ~10 MB, and
  **590 MB → ~17 MB** (8.5 MB at 16-bit, ≈70×), constant CPU cost independent of database size [D]
  dl.acm.org/doi/10.1145/3386569.3392440; [B] ubisoft.com/…/introducing-learned-motion-matching.
  PFNN (2017) already claimed "milliseconds of execution and a few megabytes of memory" [D]
  pure.ed.ac.uk/…/phasefunction.pdf. **LMM presupposes a working motion matcher** (it is trained
  against one) and inertialization.

### 2.8 Skinning / deformation

- LBS default; **dual-quaternion skinning** fixes candy-wrapper collapse "at minimal additional
  cost", GPU-friendly [D] users.cs.utah.edu/~ladislav/kavan08geometric/kavan08geometric.pdf,
  kavan07skinning.pdf. Kinemation supports LBS **and** DQS; Entities Graphics matrix LBS only [D]
  Kinemation vs Entities Graphics page.
- **UE**: three GPU paths — vertex-factory skinning (4/8 influences or Unlimited Bone Influences),
  **Skin Cache** (compute, results cached in vertex buffers; mandatory for HW ray tracing and hair;
  per-world MB budget with fallback), Deformer Graph; mobile caps 75 bones per section vs 65 536
  default [D] dev.epicgames.com/…/skeletal-mesh-rendering-paths-in-unreal-engine. **ML Deformer**:
  inference via NNE + Deformer Graph "to keep everything on the GPU"; the Vertex Delta model "should
  not be used in production"; **<100 µs / character CPU, <50 µs GPU, ~1 MB GPU memory** on current
  consoles [D] …/ml-deformer-framework-in-unreal-engine, …/how-to-use-the-machine-learning-deformer-in-unreal-engine.
- **Kinemation**: skin matrices computed on GPU, **one dispatch batched by skeleton** ("a batched
  cache-coherent linked list with bones cached in groupshared"); deformation only for "visible meshes
  of active LODs" [D] Kinemation vs Entities Graphics.
- **Unity Entities Graphics**: user-written `SkinMatrix` / `BlendShapeWeight` buffers → compute
  deformation; still **experimental**; vertex-shader deform path deprecated [D]
  docs.unity3d.com/Packages/com.unity.entities.graphics@1.4/manual/mesh_deformations.html.
  **Rukhanka measured the naive path**: 50k skinned entities, RTX 3070Ti — **26.1 ms full view /
  17.6 ms EMPTY view**; removing the pure-copy `InstantiateDeformationSystem` → 17.3 / 8.3 ms;
  compressing skin data 36 B → 20 B per vertex → **14.6 / 5.5 ms**; "skin deformation calculation
  is performed for every mesh instance in every frame" with no culling [B] blog.rukhanka.com/optimizing-smr.
- **Bevy**: joint matrices (`affine * inverse_bindpose`) in a storage buffer (uniform fallback on
  WebGL2), `MAX_JOINTS = 256` per mesh, an `offset_allocator` in 4-matrix units for 256-byte
  alignment, **double-buffered** for motion vectors; morph weights double-buffered likewise with
  `GpuMorphDescriptor { targets_offset, .. }` into one centralized delta array [S]
  bevy_pbr/src/render/skin.rs, morph.rs. skin.rs itself records a one-frame motion-blur glitch on
  buffer resize (the previous-frame buffer is invalidated) [S].

### 2.9 IK, warping, root motion, events, rigging

- IK sits after local-to-model. FABRIK (Aristidou & Lasenby, Graphical Models 73(5), 2011):
  position-based, few iterations, compared against CCD and Jacobian methods [D]
  dl.acm.org/doi/abs/10.1016/j.gmod.2011.05.003. Kinemation: two-bone IK (v0.14.0, 2025-10-18) and an
  EWBIK multi-target solver (v0.12.0) [D] CHANGELOG; ozz: two-bone and aim IK jobs [D].
- Motion warping: Witkin & Popović SIGGRAPH 1995 [D] dl.acm.org/doi/10.1145/218380.218422; UE
  ships root-motion Motion Warping (Scale / Skew Warp inside Anim Notify State windows) [D]
  dev.epicgames.com/…/motion-warping-in-unreal-engine.
- Root motion: Kinemation exposes user-side delta accumulation (`RootMotionDeltaAccumulator`) [D]
  Getting Started Part 3; Godot accumulates `root_motion_position/rotation/scale` inside
  `AnimationMixer` [S] animation_mixer.h; Bevy core: none found [S/**UNVERIFIED**].
- Events: Kinemation stores `ClipEvents` inside the clip blob [S]; Bevy keys events by
  `AnimationEventTarget::{Root, Node(AnimationTargetId)}` [S] lib.rs (issue #21473 asks to make them
  EntityEvents); Godot has method/audio track kinds [S].
- UE Control Rig / IK Rig / IK Retargeter (FK chains "run before the IK pass"), Animation Modifiers
  (editor-time generators of sync markers, curves, notifies — build-time authorship), Mutable
  (`CustomizableObject` + `Instance` + States) [D] the respective dev.epicgames.com pages.
- Determinism: `ozz-animation-rs` is a full Rust rewrite made "since the C++ runtime was not
  cross-platform deterministic", relying on IEEE-754 f32 (rejected fixed-point), SSE2 + NEON,
  nightly required [B/D] github.com/SlimeYummy/ozz-animation-rs.

## 3. Evaluation and threading — per system

| System | Unit of parallel work | Notes | Source |
|---|---|---|---|
| **ozz** | a **range of characters**, subdivided "small enough (less than a predefined number of characters)"; jobs thread-safe by data isolation; the only mutable per-instance state is `SamplingJob::Context` | no joint-level tasks; no measured guidance | [D] guillaumeblanc.github.io/ozz-animation/samples/multithread/; runtime doc |
| **Kinemation** | user-written `IJobEntity` + `ScheduleParallel()`; the tutorial's exposed-skeleton job runs one `Execute` per **bone entity**; optimized skeletons sample per skeleton via `SamplePose(clip, time, weight)` ("weight blending built-in for performance reasons") then `EndSamplingAndSync()` | the framework's own warning: "the overhead of scheduling can become too much… lots of tiny idle gaps between jobs"; culling is chunk-granular | [D] Getting Started Part 3 / Part 4; Culling Performance.md |
| **Bevy** | `advance_animations` par_iter over players; `animate_targets` = `targets.par_iter_mut()` — one task per **animation target (bone entity)** — with `ThreadLocal<RefCell<AnimationEvaluationState>>`; `extract_skins` is **sequential**, relying on `Changed<GlobalTransform>` | perf issues #15412 (closed via #15413), #17535 | [S] bevy_animation/src/lib.rs, bevy_pbr/src/render/skin.rs |
| **Bevy RFC 51** (design, not shipped as written) | three independently parallel stages (graph eval per graph, binding per graph, sampling across graphs); forbids **overlapping hierarchies** so each bone is touched by exactly one graph | | [D] github.com/bevyengine/rfcs/…/51-animation-composition.md |
| **Godot** | none — `_process_animation` / `_blend_init` / `_blend_process` are serial | | [S] animation_mixer.h |
| **UE classic** | one **skeletal mesh component** per `FParallelAnimationEvaluationTask`; `FParallelAnimationCompletionTask` dependent; game-thread sync deferred to end of tick; opt-in per instance (`bUseMultiThreadedAnimationUpdate` + project setting); Event Graphs "always run on the Game Thread" | **root motion forces the AnimGraph back onto the game thread** ("character movement is not multithreaded") | [B] tencent walkthrough (**UNVERIFIED** names); [D] …/animation-optimization-in-unreal-engine, …/root-motion-in-unreal-engine |
| **UE UAF** | Module and AnimationGraph inside RigVM "supporting multithreaded execution"; cross-thread exchange via `PublicVariablesProxy` dirty-copy, double-buffering planned | no isolated UAF-vs-AnimBP benchmark found; the Witcher 4 60 FPS claim is marketing | [B] remremremre; [B] eyad.tv/blog/unreal-animation-framework-uaf |
| **UE crowds** | a **millisecond budget**, not a thread count: `a.Budget.BudgetMs` 1.0, `MaxTickRate` 10, `MaxInterpolatedComponents` 16, `InterpolationMaxRate` 6, `BudgetFactorBeforeReduceWork` 1.5, first-tick estimate 0.08 ms per component; Epic recommends it over URO (≤15 Hz at distance) | | [D] …/animation-budget-allocator-in-unreal-engine, …/animation-optimization-in-unreal-engine |
| **Entities Graphics / Rukhanka / Kinemation** | GPU compute for deformation, not CPU threads | | [D]/[B] as in §2.8 |

**Against this tree's measured pool** [D, `D:/wt/threadpool/docs/threadpool/KE16-RESULTS.md`
(worktree `feat/threadpool-ke16`, not on this branch)]: the worker-spawned route at the baseline
**is** the serial floor (1 024 × 1 ms = 1 024.26 ms, `max_in_flight = 1`; `:292-300`); under
rustc 1.98.1 a 1 µs × 64W (= 1 024 tasks) wave from a worker reads **1.15–1.2 ms** against a 64 µs
floor — a shipped codegen regression confined to the 1 µs body, "≥ 10 µs bodies unaffected"
(`:655-671`); the consumer shapes the winning arm was tuned on are ONE spawner + W−1 thieves, a
`par_iter` wave of W chunks and a physics color of **4W–6W** chunks
(`KE16-DESIGN-W.md:63`). The task brief's phrasing "bodies under ~10 µs are counter-productive" is
consistent with that record but is not a sentence in it — **UNVERIFIED as phrased**; the Step App
numbers for the unconditional shipped code are **owed** (`KE16-RESULTS.md:15`, `§Owed`).
Consequences: Bevy's per-bone-entity granularity and Kinemation's per-bone `Execute` are the wrong
unit here; ozz's whole-character range (and UE's per-component task, when a component's whole
evaluation exceeds ~10 µs) is the compatible one; the kernel primitive that expresses it is
`Query::par_for_each_chunk(f, BatchingStrategy)` (`crates/boyko_ecs/src/ecs/core/iters/query/query.rs:658`)
with the `MIN_ARCHETYPE_FOR_PARALLEL = 1024` floor (`par_iter.rs:73`) — but **both parallel drivers
compile-reject dense terms** (`par_iter.rs:300-309`, `par_chunk.rs:117-124`), so a dense pose column
gets no kernel parallelism for free; physics fans out by hand over `DenseSolveView` copies inside
`pool.scope` [T] `crates/boyko_ecs/src/ecs/core/component/dense/views.rs:110-157`.

## 4. Authoring and loading — the Gaia analogue

- **Kinemation**: bake-time "Smart Blobbers" produce `BlobAssetReference<SkeletonClipSetBlob>` from
  `SkeletonClipCompressionSettings`; bone-mask blobs from Avatar Masks; runtime reads blobs, no
  reflection [D] Kinemation README / CHANGELOG.
- **ozz**: an offline toolchain converts fbx/glTF into `.ozz` runtime archives; runtime structures
  are the serialized form [D].
- **Bevy**: clips built from glTF at load; graphs are RON assets [S].
- **UE**: Animation Modifiers generate sync markers / curves / notifies at editor time, marked "Out
  of Date" until re-applied [D]; Pose Search Schema + Database assets [D]; ACL codec settings per
  sequence [D] …/animation-compression-library-in-unreal-engine.
- **This tree**: Gaia bakes text → the existing `BOYKOSAV` v2 binary (`crates/boyko_serialize/src/format.rs:34`),
  load = whole-column blit + `LoadEntityMap` remap + `load_dense_store` (`load.rs:213`, `:580-720`);
  reflection only at bake behind a default-off feature; references travel as name hashes (GN1) and
  raw handles are unrepresentable (`docs/gaia/DECISIONS.md:96-103`); foreign binaries (`.glb`) get
  a sidecar id with a hard fail on absence, "recorded, not fully designed" (`DECISIONS.md:125-127`);
  curves are a first-class asset kind baked into flat keyframe arrays (`DECISIONS.md:74-76`) [T].

## 5. Physics coupling — per system

- **Jolt** (the clearest primary source for ordering): `PoweredRigTest::PrePhysicsUpdate` samples
  the clip, zeroes the root translation and takes the root from the ragdoll, calls
  `CalculateJointMatrices()`, samples the **previous** pose, then
  `DriveToPoseUsingMotors(mPrevPose, mPose, dt)` — all before `PhysicsSystem::Update`; `GetPose`
  reads body transforms back after [S] JoltPhysics/master/Samples/Tests/Rig/PoweredRigTest.cpp.
  Three couplings on `Ragdoll`: `SetPose` (+ `ResetWarmStart` "to reset previous frames impulses"),
  `DriveToPoseUsingKinematics`, `DriveToPoseUsingMotors` [D] class_ragdoll.html; `SkeletonMapper`
  maps a low-detail ragdoll skeleton to the high-detail animation skeleton [D~ snippet].
  `BodyInterface::MoveKinematic` sets a velocity so the body reaches the target pose in dt [D~];
  `CharacterVirtual` is collision-query-only and "the update can happen at the appropriate moment
  in the game code" [D] class_character_virtual.html, Docs/Architecture.md. Soft bodies: **skinned
  constraints** (`SkinVertices(com, jointMatrices, n, hardSkin)` per step, max travel distance from
  the skinned vertex) [D~]; `LargeIslandSplitter` splits islands ≥128 constraints into ≤32 groups
  in batches of 16 [S] Jolt/Physics/LargeIslandSplitter.h.
- **PhysX** articulations: PD drives (stiffness / damping, `eACCELERATION` mode mass-independent),
  bulk state via `PxArticulationCache`; "It is not possible to set a link's global pose or velocity
  directly" and "**links cannot be kinematic**" [D] nvidia-omniverse.github.io/PhysX/physx/5.6.0/docs/Articulations.html.
- **Box2D v3**: joints and contacts in ONE graph coloring with the rule that "a joint cannot share
  the same color as a contact between the same two bodies"; static bodies exempt; overflow color
  serial; explicit warning that "the SIMD solver body scatter would write to the same kinematic body
  from multiple threads" [S] box2d/main/src/constraint_graph.c; [B] box2d.org/posts/2024/08/simd-matters/.
- **Avian** (Rust): manifold-level greedy coloring, contacts only — "joint graph coloring left to
  follow-ups"; >3× solver speedup on Large Pyramid 2D [D] github.com/Jondolf/avian/pull/771.
- **UE**: per-bone `Physics Blend Weight` (1.0 physics / 0.0 keyframe, bone and below) driven from
  the Event Graph [D] …/physics-driven-animation-in-unreal-engine; `AnimDynamics` owns its own cheap
  bodies; the `Rigid Body` node simulates in Base Bone Space [D~]. GDC 2018 (Sachania, EA): the
  recurring production problems are animation follow, complexity reduction for performance, and
  "reducing bad output poses" [D] gdcvault.com/play/1025210.
- **Unity**: `Animate Physics` evaluates animation on the fixed step; a moving platform must be a
  kinematic Rigidbody; "do not mix and match timesteps unless you are willing to accept some
  stutter" [D~ snippets].
- **bevy_transform_interpolation**: start in `FixedFirst`, end in `FixedLast`, ease in
  `RunFixedMainLoop`, `alpha = overstep_fraction()`, lerp + slerp, extrapolation separate [D]
  raw.githubusercontent.com/Jondolf/bevy_transform_interpolation/main/README.md — the design this
  tree's `FixedSet` seam and `GpuTransform3D` pair already embody [T].

## 6. Pitfalls register

Each row: the pitfall, who measured it, what it means for this engine.

| # | Pitfall | Evidence | Consequence here |
|---|---|---|---|
| P1 | **Per-bone decompression is a trap**: `decompress_bone` is "much slower" than `decompress_pose` because unneeded bones are scanned, not indexed | [B] acl_decompression_baseline | sample whole poses; bone masks apply at blend, never at decode |
| P2 | Clips are touched once per frame ⇒ decompression is **always cold-cache**; a benchmark keeping the clip hot measures the wrong thing | [B] same | the codec bench must flush or stream ≥ L2 of clip data between samples |
| P3 | **"Max error" is a misleading codec metric**: ACL 0.5 showed 9.79 cm max vs UE's 0.86 cm, yet 0.77 % of 112 M samples exceeded a sub-mm threshold and the outliers came from three clips | [B] acl_paragon | the codec gate reports the exceedance fraction AND the max, per clip |
| P4 | Bone-name-hash binding breaks retargeting and costs a hash per target | [S] bevy lib.rs; [D] issues #15612, #14230 | bind by baked bone index; names only at bake |
| P5 | System-ordering **one-frame lag** (`animate_targets` after `inherit_weights`) | [D] bevy #16554 | already a catalogued class here (`sets.rs:1-41`); every animation system joins a named `FixedSet` |
| P6 | GPU buffer resize invalidates the previous-frame buffer ⇒ one-frame motion-blur glitch | [S] bevy skin.rs | palette ring is preallocated at a ceiling; growth is a loud event, never a silent realloc |
| P7 | Deforming every instance every frame: 17.6 ms with an EMPTY view; a pure copy pass ate half the GPU frame | [B] rukhanka optimizing-smr | deformation is culling- and LOD-aware from day one; no copy-only passes |
| P8 | Scheduling overhead beats work: "lots of tiny idle gaps between jobs"; 1 µs × 1024 from a worker runs serial | [D] Kinemation Culling Performance; [D] KE16-RESULTS:655-671 | task = many instances; body ≥ 10 µs |
| P9 | A shared **sampling context** silently serialises or corrupts parallel sampling | [S] ozz sampling_job.h | uniform sampling ⇒ no context at all |
| P10 | **Root motion de-parallelises** the whole AnimGraph in UE | [D] root-motion page | root motion is an output column consumed by a later system, never a call back into a serial mover |
| P11 | Thread-safety opt-in per function with a silent fallback; the Fast Path breaks on any VM call | [D] animation-optimization | there is no scripting in the evaluator; logic is Aether (Rust) systems |
| P12 | Inertialization has a validity envelope (<0.4 s, not for extreme pose differences); Dead Blending's unbounded extrapolation needs per-axis clamped half-lives | [D] blend-nodes; [B] theorangeduck | ship inertialization with the envelope asserted in debug; dead blending is a later rung |
| P13 | More motion-matching features is NOT better ("use as few samples as possible"); an approximate index without a full-cost re-rank returns wrong poses | [D] motion-matching page | the search rung ships brute force first, PCA+KD-tree with mandatory re-rank second |
| P14 | Re-selecting the same clip region re-fires the same notifies (0.2 s recency timeout) | [D] same | clip events carry a per-instance "last fired sample index" |
| P15 | Sync groups **silently degrade** to length sync when one member lacks a marker | [D] sync-groups | marker coverage is a bake-time contract, not a runtime fallback |
| P16 | A fixed-size GPU skin cache fails non-monotonically (an LOD switch can free a high LOD then fail to fit the lower one) and the fallback changes visual features | [D] skeletal-mesh-rendering-paths | budget overflow is a coded diagnostic + LOD drop, never a path switch |
| P17 | Per-component knobs (URO) do not compose across a crowd; the budget allocator measures actual per-component work | [D] budget-allocator | budgeting is a first-class system with a **count** gate |
| P18 | A DFG node-graph animation runtime failed to ship (Unity, 2019→2022) | [D]/[B] Unity samples + status thread | blend trees are baked flat programs (Bevy `ThreadedAnimationGraph`, UAF task list), never a live node graph |
| P19 | Blend semantics are a compatibility hazard (Godot rewrote 3.x → 4.x for consistency) | [D] Godot migration article | blend order and normalisation are pinned by a bit-identity fixture |
| P20 | Precomputed track→slot bindings desync ("No animation in cache") | [S] godot #98647 | bindings are baked per (skeleton, clip) pair with a layout fingerprint; a mismatch is a bake refusal |
| P21 | Warm-start impulses survive a pose teleport (`ResetWarmStart` after `SetPose`) — this solver warm-starts across frames (`crates/boyko_physics/src/lib.rs:18-22` [T]) | [D] Jolt class_ragdoll | a pose write into physics resets warm start for the touched bodies |
| P22 | Kinematic bodies are a multithreaded write hazard in a colored SIMD scatter | [S] box2d constraint_graph.c | animation-driven kinematic bodies are read-only in the scatter (treated like statics) |
| P23 | A joint and a contact between the SAME two bodies must not share a color | [S] same | the future joint kind enters the existing colorer with that rule |
| P24 | Reduced-coordinate articulations forbid per-link kinematic / direct pose writes | [D] PhysX Articulations | ragdolls here are maximal-coordinate bodies + joints (the Jolt/Box2D shape) |
| P25 | A `std::Vec` side store mutated in parallel = the measured SP4 race | [T] `docs/DENSE-COMPONENTS-PLAN.md:6`, `:24`; `views.rs:100-135` | pose storage is kernel-owned columns with the Build/Solve view split |
| P26 | `for_each_chunk` is tick-blind; both parallel drivers reject dense terms; scheduler write conflicts are filter-agnostic | [T] `docs/FEATURE_MAP.md:1016`; `par_iter.rs:300-309`; `par_chunk.rs:117-124`; `gpu_transform_pack.rs:44-56` | one `&mut` writer system per pose column; parallelism hand-rolled over Solve views |
| P27 | Asset `Handle` generational reuse is unsafe for render-referenced assets; render carriers are bare indices (`MeshHandle(u32)`) under an append-only rule | [T] `crates/boyko_ecs/src/ecs/core/asset/handle.rs:41-44`; `crates/boyko_scene/src/render_caps.rs:143` | skeleton / clip tables are append-only for the process lifetime |
| P28 | The loader's rest-shape path returns a plausible, wrong pose for a skinned file | [T] `glb.rs:1016-1027` | "it loaded fine" says nothing about animation readiness; the default `decode` keeps refusing until skins render |
| P29 | 3D render interpolation has no `prev_pose` lane on the static instance row ("3D interpolation is deferred, not dropped") | [T] `crates/boyko_render/src/gpu3d_instance.rs:45-48` | skinned instances ride the dense `GpuTransform3D` pair + palette double-buffer, not the static row |
| P30 | Parented dynamic bodies are unsupported; `Children` sibling order is unspecified (`swap_remove`) | [T] `scene_sync.rs:192-212`; `docs/FEATURE_MAP.md:682` | ragdoll bodies are roots; bone order never comes from `Children` |
| P31 | UE's own retrofit (UAF) "completely abandons the ABP framework" and is still Experimental after years | [B] remremremre, eyad.tv; [D] UAF plugin index | decide the pose / graph data model BEFORE writing nodes |
| P32 | Epic API pages render client-side (empty fetches); the source needs an account link | — | every UE header-level name in this record is [B]-sourced and **UNVERIFIED** |

## 7. The feature ladder — prerequisites and cost model

Every surveyed system builds the same base and only then adds selection, correction and physics.
Rungs are ordered by dependency; "cost" is what the rung adds per instance per evaluation, with the
measured anchor named. Estimates are labelled.

| Rung | Feature | Prerequisites | Data added | Cost model (anchor) |
|---|---|---|---|---|
| **L0** | Skinned clip playback: skeleton asset, uniformly-sampled compressed clip, local pose, local→model, LBS on GPU | glb `skins` + `animations` decode (§0); a quaternion blend primitive in `boyko_math` (absent); a skinning compute pass in the eDSL | skeleton (parents, rest, inverse bind), clip streams, pose column, palette | per instance: one whole-pose decompress — ACL: 0.9–28.8 µs cold-cache on a 2.39 GHz iPad [S/D]; one affine multiply per bone; GPU: one dispatch batched per skeleton [D] Kinemation. **Estimate** for a 64-bone skeleton on this box: 5–30 µs CPU per evaluation |
| **L1** | Layered blending, additive, per-bone masks, sync groups | L0 | per-node weight + `u64` masks (Bevy) or bone bitmasks (Kinemation); marker tables | +1 nlerp per bone per extra layer; masks are a scan at blend, never at decode (P1) |
| **L2** | Inertialization | L0, a durable per-bone offset + velocity state | ~12 floats per bone (offset + velocity, translation + rotation) [B] spring-roll-call; Kinemation `OptimizedBoneInertialBlendState` [S] | removes the second evaluation during transitions — "effectively doubling" avoided [D] Bollo; one `exp` per update shared across bones |
| **L3** | Per-entity animation state machines | Aether `machine … on entity` (rung R5, **unbuilt**), `state_chart!` (R2, unbuilt) | one `#[repr(C)]` table component per machine (`docs/aether-v2/MACHINES.md:7-17`) | one O(N) pass; measured 3.4–4.0 ns/row for a shuffled 5-arm match (`MACHINES.md:115-123`) |
| **L4** | Root motion + kinematic drive of attached colliders | L0, physics `Kinematic` pose flow (`Transform → RigidBody` before the head, `crates/boyko_physics/src/plugin.rs:142-145` [T]; the gate is `!simulated && !is_dynamic_row(inv_mass)`, `crates/boyko_physics/src/scene_sync.rs:64-66`, `:93` [T] — not the `Kinematic` tag) | a root-delta column; hit-box entities with `Transform` + `RigidBody` + `RigidBodyMass { inv_mass: 0 }` + `Kinematic`, `Simulated` OFF, hierarchy roots | a pose-delta / dt velocity per body (Jolt `MoveKinematic` semantics) — done animation-side because the solver defers kinematic motion |
| **L5** | IK (two-bone, FABRIK, look-at) and foot placement | L0 model-space pose; a ground query — **absent** (no ray cast; SDF sampling only) | goal columns per instance | FABRIK: few iterations per chain [D]; ground probe = `sample_sdf` gradient today |
| **L6** | Morph targets | glb `targets` decode; a delta array + weights (Bevy `GpuMorphDescriptor` shape [S]) | per-mesh delta array, per-instance weights (double-buffered) | GPU: one gather per active target per vertex |
| **L7** | Ragdoll / physics-driven animation | **physics joints in the colored graph** (absent; Box2D rule P23), `ResetWarmStart` (P21), a low→high skeleton map (Jolt `SkeletonMapper`) | joint constraints per bone pair; per-bone blend weight | a ~15–20-body island per character, below the P2 large-island gate (`resources.rs:2108-2126` [T]) — parallelism is across characters, never within one |
| **L8** | Dead blending | L2 | +3 floats per rotation axis [B] | closed-form extrapolation per bone per step |
| **L9** | Motion matching | L0, L2, L4, a feature database (a mocap set — an **acquisition cost**, not an engineering one) | frame-major pose + velocity + feature matrix; AABB/KD index | brute force = O(frames × features) per query; database memory 100–590 MB at production scale [B] |
| **L10** | Learned motion matching | L9 working and trained against | three weight blobs (~10–17 MB) | constant CPU per query; the 590 MB → 17 MB trade [B/D] |
| **L11** | ML deformer | L0 GPU skinning, L6, a trained model per character | ~1 MB GPU per character [D] | <50 µs GPU per character [D] |
| **X** | Budgeting / LOD (cross-cutting, needed from ~10² instances) | L0 | bone-LOD prefix per skeleton; per-instance significance + update period | UE: 1.0 ms budget, 10 Hz tick floor, 16 interpolated components [D] |

## 8. Sources (by system)

**ozz**: [S] raw.githubusercontent.com/guillaumeblanc/ozz-animation/master/include/ozz/{base/maths/soa_transform.h, animation/runtime/skeleton.h, animation/runtime/animation.h, animation/runtime/sampling_job.h}; [D] guillaumeblanc.github.io/ozz-animation/documentation/animation_runtime/; [D] guillaumeblanc.github.io/ozz-animation/samples/multithread/; [B/D] github.com/SlimeYummy/ozz-animation-rs.
**Kinemation**: [S] raw.githubusercontent.com/Dreaming381/Latios-Framework/master/Kinemation/Components/{AnimationComponents.cs, SkeletonComponents.cs, OptimizedSkeletonAspect.cs, InertialBlendingComponents.cs}; [D] raw.githubusercontent.com/Dreaming381/Latios-Framework-Documentation/main/Kinemation%20Animation%20and%20Rendering/{Getting Started - Part 3.md, Getting Started - Part 4.md, Culling Performance.md, Kinemation vs Entities Graphics.md, CHANGELOG.md}; [B] the README.
**Unity**: [D] docs.unity3d.com/Packages/com.unity.entities.graphics@1.4/manual/mesh_deformations.html; [D] github.com/Unity-Technologies/Unity.Animation.Samples; [B] discussions.unity.com/t/animation-status-update-q4-2022/903925; [B] blog.rukhanka.com/optimizing-smr; [B] docs.rukhanka.com.
**Bevy**: [S] raw.githubusercontent.com/bevyengine/bevy/main/crates/{bevy_animation/src/lib.rs, bevy_animation/src/graph.rs, bevy_animation/src/animation_curves.rs, bevy_mesh/src/skinning.rs, bevy_pbr/src/render/skin.rs, bevy_pbr/src/render/morph.rs}; [D] raw.githubusercontent.com/bevyengine/rfcs/main/rfcs/51-animation-composition.md; [D] issues #15412, #17535, #16554, #15612, #14230, #21473; [B] github.com/mbrea-c/bevy_animation_graph; [B] glocq.com/en/blog/20260827/.
**Godot**: [S] raw.githubusercontent.com/godotengine/godot/master/scene/animation/animation_mixer.h; [S] github.com/godotengine/godot/issues/98647; [D] godotengine.org/article/migrating-animations-from-godot-4-0-to-4-3/; [D] deepwiki.com/godotengine/godot/4.9-animationtree-and-animationnodes.
**Unreal**: [D] dev.epicgames.com/documentation/…/{animation-optimization-in-unreal-engine, root-motion-in-unreal-engine, property-access-in-unreal-engine, animation-blueprint-node-functions-in-unreal-engine, animation-blueprint-blend-nodes-in-unreal-engine, animation-sync-groups-in-unreal-engine, motion-matching-in-unreal-engine, motion-matching-debugging-in-unreal-engine, game-animation-sample-project-in-unreal-engine, dynamic-asset-selection-in-unreal-engine, animation-budget-allocator-in-unreal-engine, skeletal-mesh-rendering-paths-in-unreal-engine, ml-deformer-framework-in-unreal-engine, how-to-use-the-machine-learning-deformer-in-unreal-engine, mutable-overview-in-unreal-engine, ik-rig-in-unreal-engine, ik-rig-animation-retargeting-in-unreal-engine, motion-warping-in-unreal-engine, animation-modifiers-in-unreal-engine, physics-driven-animation-in-unreal-engine, control-rig-in-unreal-engine, animation-compression-library-in-unreal-engine, API/PluginIndex/UAF, API/Plugins/RigVM}; [D~] animation-blueprint-animdynamics, animation-blueprint-rigid-body, physics-sub-stepping, panel-cloth-editor-overview, python-api/class/ChaosClothComponent (search snippets, pages not opened); [B] cloud.tencent.com/developer/article/2347202; [B] remremremre.github.io/posts/My-understanding-of-Unreal-Animation-Framework-in-5.6/; [B] eyad.tv/blog/unreal-animation-framework-uaf; [B] theorangeduck.com/page/{dead-blending-node-unreal-engine, dead-blending, spring-roll-call}.
**ACL**: [S/D] github.com/nfrechette/acl/blob/develop/docs/decompression_performance.md; [D] github.com/nfrechette/acl; [B] nfrechette.github.io/{2016/11/10/anim_compression_uniform_segmenting, 2017/09/10/acl_v0.4.0, 2017/12/05/acl_paragon, 2018/05/11/acl_decompression_baseline, 2019/04/15/acl_v1.2.0, 2023/09/17/acl_in_ue}; [B] github.com/nfrechette/acl-ue4-plugin README.
**Papers / talks**: [D] gdcvault.com/play/1025165 and /1025331 (Bollo, Inertialization, GDC 2018; the PDF at media.gdcvault.com was fetched but unreadable — slide numbers **UNVERIFIED**); [B] gamedeveloper.com summary; [D] dl.acm.org/doi/10.1145/3386569.3392440 (Learned Motion Matching); [B] ubisoft.com/…/introducing-learned-motion-matching; [D] pure.ed.ac.uk/…/phasefunction.pdf (PFNN); [D] gdcvault.com/play/1023280 (Clavet 2016); [D] gdcvault.com/play/1027118 (TLOU2); [D] users.cs.utah.edu/~ladislav/{kavan08geometric, kavan07skinning}; [D] dl.acm.org/doi/abs/10.1016/j.gmod.2011.05.003 (FABRIK); [D] dl.acm.org/doi/10.1145/218380.218422 (Motion Warping); [D] gdcvault.com/play/1024087 (Uncharted 4 physics animation, GDC **2017**); [D] gdcvault.com/play/1025210 (EA ragdolls); [D~] gdcvault.com/play/1026712 (Ragdoll Motion Matching); [B] gdcvault.com/play/1020583 (Overgrowth — talk not opened).
**Physics**: [S] raw.githubusercontent.com/jrouwe/JoltPhysics/master/{Samples/Tests/Rig/PoweredRigTest.cpp, Jolt/Physics/Ragdoll/Ragdoll.h, Jolt/Physics/LargeIslandSplitter.h, Docs/Architecture.md}; [D] jrouwe.github.io/JoltPhysicsDocs/5.0.0/class_ragdoll.html; [D] jrouwe.github.io/JoltPhysics/class_character_virtual.html; [D~] class_skeleton_mapper, class_soft_body_shared_settings_1_1_skinned, md__docs_2_release_notes, BodyInterface.h; [B] github.com/jrouwe/JoltPhysics/discussions/1895; [S] raw.githubusercontent.com/erincatto/box2d/main/src/constraint_graph.c; [B] box2d.org/posts/2024/08/simd-matters/; [D] nvidia-omniverse.github.io/PhysX/physx/5.6.0/docs/Articulations.html; [D] github.com/Jondolf/avian/pull/771; [D~] rapier.rs/docs/user_guides/templates/rigid_body_type/; [D] raw.githubusercontent.com/Jondolf/bevy_transform_interpolation/main/README.md; [D~] docs.unity3d.com/ScriptReference/Animation-animatePhysics.html, Manual/rigidbody-interpolation.html, kinematicsoup.com timestep article.
**This tree** [T]: `CLAUDE.md`; `docs/{FEATURE_MAP.md, DENSE-COMPONENTS-PLAN.md, OPEN-QUESTIONS.md, AETHER-LANG-PLAN.md}`; `docs/aether-v2/{CONSTRUCTS.md, MACHINES.md, CAMPAIGN.md, KERNEL-BACKLOG.md}`; `docs/gaia/{DECISIONS.md, LANGUAGE.md, CAMPAIGN.md}`; the source files cited inline; `D:/wt/threadpool/docs/threadpool/{KE16-RESULTS.md, KE16-DESIGN-W.md}` (worktree).
**glTF**: [D] raw.githubusercontent.com/KhronosGroup/glTF/main/specification/2.0/Specification.adoc (`animation.channel.target.path` ∈ {translation, rotation, scale, weights}).

## 9. Critique log — pass 1 (2026-09-10)

The `architecture-critic`'s first pass was filed against the design document; three of its
findings touch lines in this record. Each was re-opened and answered here; the design document's
§15 carries the full table.

| # | Finding (short) | Action here |
|---|---|---|
| NB4 | L4 row (§7) cites `plugin.rs:143-148` with no crate path; it resolves to `crates/boyko_physics/src/plugin.rs`, while `crates/boyko_app/src/plugins.rs:143-148` is `with_ssaa_scale` | **Fixed**: the row now carries the crate path (`:142-145`, re-opened) and, per NB1, the actual sync gate (`scene_sync.rs:64-66`, `:93`) instead of the `Kinematic` tag |
| NB5 | The ACL layout claims (16-frame segments, per-segment range reduction) were [B]-tagged, and the [D] algorithm page does not state them | **Fixed by upgrading the source**: `segment_context.h` (`ideal_num_samples = 16`, `max_num_samples = 31`), `segment.transform.h`, `normalize.transform.h` opened and quoted — [S]. The four-stream description and "2–5 cache lines" remain [B], marked as such; the algorithm page's silence on segments is recorded |
| NB2 | L6 needs per-instance morph weights that something must animate; the codec had no scalar stream | **Fixed in the design** (§1.2 scalar track stream, M1/M2); the glTF `weights` channel path is sourced here as [D] (Specification.adoc opened). L6's row is unchanged — its data column ("per-instance weights") was already right; what was missing was the producer |
| §3 ¶ | The record's own paragraph on the pool (worker route = serial floor, `KE16-RESULTS.md:292-300`) was correct; the design built on the fixed route anyway (B1) | **No change here** — the record was right and the design was wrong; the design now names AK-9 and marks G2 red until it lands |
