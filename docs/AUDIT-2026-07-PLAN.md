# Whole-Project Audit + Refactoring Plan (2026-07)

**STATUS: ACTIVE — plan approved-by-goal, fixes in flight.**

Owner directive: (1) root-cause the "shadows render slightly differently while the camera
is in motion" viewer artifact; (2) audit the whole project — structural cleanliness, bugs,
performance, Principle-0 (`Vec`/`HashMap` side stores), cross-crate coordination — produce a
detailed plan, then implement the fixes.

Method: one deterministic GPU diagnostic (the shadow-motion A/B harness, below) + four
parallel deep-audit agents covering all 18 workspace crates (ECS core / render stack /
sim+std-lib / UI+apps+workspace), findings verified against source with file:line.

---

## Part I — The shadow-motion bug: root cause PROVEN

### The diagnostic

`shadow_motion_ab_dump` (tests/window_present_gbuffer.rs, `BOYKO_SHADOW_AB=1`) drives the
EXACT interactive-viewer path with a scripted camera: capture pose P reached **statically**
vs reached **in motion** (24-frame yaw oscillation ±20°; 24-frame x-strafe ±0.8u), all
captures at bitwise-identical pose floats, byte-compared on the CPU. Results (RTX, 512²):

| Comparison | Differing px | Max Δ |
|---|---|---|
| static vs static (repeatability) | **0** | 0 |
| static vs rotation-arrival | **0** | 0 |
| static vs translation-arrival | **0** | 0 |
| static vs static+3 mrad yaw (0.17°) | **146 829 (56%)** | **226** |

### Verdict

1. **No cross-frame race.** The frame is a pure function of the camera pose — the G-buffer
   ring + camera ring + RDG barrier architecture is clean on the viewer path.
2. **The "dancing" is resampling scintillation of hard edges.** A 0.17° yaw flips 56% of
   pixels with near-full-swing deltas. The ×8 diff map localizes the perceived shadow
   component: (a) **shadow-map edges** (CSM sun + point-cube) are 1-tap
   `SampleCmpLevelZero` (hardware 2×2) — an edge spans ~1–2 screen px and flips binarily
   under sub-pixel motion; (b) bright rings around the SDF contact-shadow boundaries;
   (c) oct-normal quantization speckle on curved surfaces (gNormal is 8-bit oct in
   R8G8B8A8); (d) faint whole-screen marcher step-banding (×8-amplified only).

### Fix plan (staged, perceptual-impact order)

- **S1 (now): multi-tap PCF on the map shadows.** Castaño-style optimized PCF (the
  Witness/Bevy approach) in `deferred_pbr.hlsl` — `csm_sample_visibility` + the punctual
  atlas sample sites. Turns the binary edge into a ~3–4-texel ramp: sub-pixel camera motion
  produces proportional gray deltas instead of 0↔1 flips. Cost: 8 extra `SampleCmp` per
  shadowed pixel per term (hw-filtered), resolve-only, no new resources. NOT eDSL-owned
  territory (the eDSL pins only the `sdf_soft_shadow_ranged` + SSAO spans — untouched).
  Gate: the A/B harness micro-yaw metric (shadow-region max-delta must drop from ~226 to
  ramp-scale) + owner visual approval before commit.
- **S2 (follow-up, measured): oct-normal precision.** 8-bit oct normals quantize shading on
  curved surfaces (the sphere speckle). Candidate: RGB10A2 oct (10-bit) or R16G16 oct with
  the material id relocated. Same-bandwidth candidates exist; decide by measurement
  (HYBRID-perf rule), separate phase.
- **S3 (follow-up): marcher step-banding** — below perception at ×1; revisit only if the
  owner still sees floor/wall shimmer after S1.
- **NOT a fix here:** TAA — the engine deliberately has no temporal accumulation; the CSM
  cross-fade comment (deferred_pbr.hlsl:249) already encodes this constraint (analytic
  ramps, not dither).

### Related latent bug (fix in this campaign): cross-frame WAR on non-ringed resources

The render audit (B-002/B-003) found the mechanism that WOULD produce true motion-dependent
shadows on the **production** path: the framegraph resets every `ResSync` to `undefined()`
each frame, so a first-touch write to a NON-RINGED resource emits a src=`TOP_OF_PIPE`/0
barrier that does not order against the **sibling in-flight frame's** reads:
- `light_table` SSBO — torn lighting on light-dirty frames (swapchain.rs:2682, sync.rs:166);
- CSM cascade + shadow-atlas depth images — benign in the viewer (world-fixed content ⇒
  identical bytes) but live for any per-frame camera-fit CSM (csm_config.rs:500).
**Fix: persistent seed-state for non-ringed resources** — seed each such resource's
`ResSync` with its prior-frame consumer scopes (one small table in `graph.rs::compile`)
instead of unconditional `undefined()`. Keeps the graph the single sync authority.

---

## Part II — Bug backlog (from the 4 audits, priority order)

| # | Sev | Where | Defect | Fix |
|---|-----|-------|--------|-----|
| B1 | CRITICAL | boyko_render/light_system.rs:163-196 | >MAX_LIGHTS enabled lights ⇒ release-mode heap OOB write through safe API (debug_assert-only bounds; check fires after writes) | clamp/saturate inside the fold before writes |
| B2 | HIGH | boyko_physics/narrowphase/axis_cache.rs:131-208 | BoxAxisCache saturates (never cleared/evicted; grow preserves stale entries without rehash) ⇒ `set` probe loop can hang in release on the DEFAULT box-box path | occupancy counter + clear-on-grow (or epoch stamps) + release probe bound in `set` |
| B3 | HIGH | boyko_ui/text/measure.rs:68-77 | `&mut ContentSize` never bumps ticks ⇒ `Changed<ContentSize>` relayout gate never fires for text changes (Auto-sized labels stay stale) | `Mut<ContentSize>` + `set_if_neq` + one scheduled end-to-end test |
| B4 | HIGH | framegraph graph.rs/sync.rs + swapchain.rs:2682/2988 | cross-frame WAR on non-ringed resources (see Part I) | persistent seed-state table |
| B5 | MED | boyko_ui/components.rs:295-319 | `UiName::new` release clamp can split a UTF-8 char ⇒ `as_str()` `from_utf8_unchecked` UB from safe code | back off to char boundary (const-compatible loop) |
| B6 | MED | boyko_threadpool/worker.rs:125-132 | fire-and-forget task panic unwinds worker_main ⇒ silent permanent worker loss (comment claims rayon parity, wrongly) | catch_unwind + explicit policy + fix comment |
| B7 | MED | boyko_threadpool/worker.rs:280-288 | cross-pool `push_task` routes by bare TLS worker id into the WRONG pool's per-worker injector ⇒ unbounded delay | compare `tls::active_pool_ptr()` vs target pool; else global injector |
| B8 | MED | boyko_demo/sim/systems/physics.rs:130 vs 307 | snapshot/write-back query shapes differ (`&Radius` missing in write-back) ⇒ row misattribution for archetypes without Radius | make both queries component-identical |
| B9 | MED | boyko_serialize/load.rs:113-127 | owning dense components with ViaFn/Ignore types skipped on load (save→load data hole, observable counter) | implement v1.1 dense ViaFn decode |
| B10 | MED | framegraph record.rs:68-118 | MAX_PASS_BARRIERS=16 stack arrays guarded by debug_assert only; resolve already declares 11 | release-checked guard or chunked emission + cross-ref comment |
| B11 | LOW | boyko_physics/soft component.rs:498/solver.rs:616 | negative compliance not rejected at construction ⇒ NaN poisoning in release (finiteness keystone is debug-only) | validate `compliance >= 0` in the existing funnel |
| B12 | LOW | boyko_input/raw/queue.rs:83-86 | `debug_assert!(dropped == 0)` right after `dropped += 1` = disguised assert(false) | `debug_assert!(false, …)` or threshold |
| B13 | LOW | boyko_ui/interaction/focus.rs:480-523 | dead `press_gen`/`reset_next_frame` machinery + wrong doc claims | delete + fix comments |
| B14 | LOW | boyko_demo/app.rs:378-380 | `return`-as-continue in `for_each_chunk` capacity guard ⇒ order-scrambled subset on overflow | latch a `full` flag |
| B15 | LOW | rhi_impl.rs:2233… / ecs_master.rs:3429 / query/filter.rs:1410 | unsafe blocks without per-site SAFETY lines | add one-line SAFETY citations |
| B16 | LOW | boyko_scene/camera.rs:44-100 | `#[repr(C)]` "POD" doc on a struct containing `Option<Viewport>` | doc fix |

## Part III — Performance backlog

| # | Sev | Where | Problem | Fix |
|---|-----|-------|---------|-----|
| P1 | HIGH | boyko_render/gpu_column.rs:739-855 | `dispatch_compute` = fence+encoder+2 pools created, **blocking wait**, destroyed — per GPU system per frame | per-FIF persistent encoder+fence ring in RhiContext; batch a frame's GPU systems into one submit |
| P2 | HIGH(value) | boyko_utils/sparse_map.rs:90 | `swap_remove` clones instead of moving ⇒ hot clone+drop AND a `U: Clone` bound that forced dense_registry to hand-roll a replacement | `Vec::swap_remove`, delete the bound (unlocks C-001 dedup) |
| P3 | MED | boyko_ecs/schedule/schedule.rs:966-967 | two `Vec::new()` per dispatch round in the executor hot loop | move into the existing `executor_scratch`, clear per round |
| P4 | MED | boyko_render/mesh_draw.rs:154-199 | per-instance `dyn FnMut` ×2 passes (~200k indirect calls/frame @100k instances) | iterator-factory generic (`Fn() -> I`), fully monomorphic |
| P5 | MED | boyko_ui world/pick.rs:297/project.rs:261/visibility.rs:88 | per-frame `Vec` allocs (`query_entities`, `collect_bounds`) while sibling systems already use the retained-buffer API | `UiWorldScratch` resource, mirror `UiInteractionScratch` |
| P6 | MED | boyko_physics/solver/soft_step.rs:193 | default solver keeps AoS constraint Vecs (colored/SoA solver exists behind opt-in) | promote colored to default after its gates bake (measured) |
| P7 | LOW | axis_cache.rs:143/warm_start.rs:247 | `trailing_zeros` recomputed per probe | cache the shift |
| P8 | LOW | swapchain.rs:658-694 | full 16-slot barrier array built per group (inert tail) | write `[..n]` only |
| P9 | LOW | worker.rs:261 | `unpark_one_idle` lowest-bit bias | rotate start bit |

Workspace-level build perf: bench_bevy_vs_boyko compiles **the whole Bevy tree into every
workspace build** (normal deps of a stub lib → move to dev-deps); boyko_macros declares an
unused `boyko-ecs` dep that **serializes the entire build graph** (all 17 crates wait) —
delete it (the `quote!` token paths need no dependency).

## Part IV — Structural refactoring plan (staged)

**Verdict on the owner's `Vec`/`HashMap` concern:** the audits found **zero Principle-0
violations in kernel/render/sim non-test code** — every durable store is ECS-native or a
documented FFI/scratch/setup exception. The debt is elsewhere: per-frame alloc
inconsistencies in `boyko_ui` (P5), two demo snapshot mirrors that dogfood the DEPRECATED
pre-dense-components idiom (`boyko_demo/sim/resources.rs:131,189` — port to dense
components or annotate), and the items below.

Stage order (each stage independently green + committed):

1. **W1 — bug fixes** (Part II B1–B16; parallel developer agents; compiler+targeted tests
   as the gate; Miri for the unsafe-adjacent ones).
2. **W2 — workspace hygiene:** bench dev-deps; macros dep deletion; dead deps (root
   `rand`/`chrono`, boyko_utils `anyhow`); `[workspace.dependencies]` hoist; invert
   boyko_render `test-readback` default. **Docs refresh:** ARCHITECTURE/FEATURE_MAP/SYSTEMS
   still describe a **6-crate** workspace (reality: 18) — add the 12 missing crates, fix the
   stale serialization non-goal + test counts; move ~120 completed `PHASE-*`/`GUI-P*`/etc.
   plan docs into `docs/archive/` (one status line each). FEATURE_MAP is the declared first
   point of contact — this is the highest agent-productivity item.
3. **W3 — mechanical decompositions:** split `boyko_macros/lib.rs` (4 974 lines, 10 macros)
   into modules; extract the 37 `golden_*` CPU oracles out of `compute.rs` (10 113 lines)
   behind `#[cfg(any(test, feature = "goldens"))]`; merge the diff-verified
   `parameters_buffer`/`participants_buffer` rename-twins; evict in-file test megamodules
   from `colored.rs` (6 441 lines)/`resources.rs`.
4. **W4 — swapchain.rs decomposition (6 788 lines):** unify the 4 frame skeletons
   (`render_frame`/`render_scene_frame`/`present_sampled`/`render_gbuffer_frame` share a
   byte-identical ~120-line fence/acquire/submit/present prologue+epilogue ×4) into ONE
   generic frame driver, then split into `present/{surface,swapchain,frame_driver,
   passes/*,graph_bridge,scene_types,targets}.rs`. Gate: the 20-config byte-identity dump
   suite + framegraph equiv tests (already proven as a refactoring net).
5. **W5 — API retirements:** deprecate+migrate the legacy query stack (~2 kLOC:
   legacy_query.rs, query_state.rs, the allocating archetype_master wrapper); publish the
   kernel `TypeId→ResourceId` registry and delete `boyko_input/action/resource_id.rs`
   (documented Principle-0 duplication); flat re-exports for `boyko_utils` (kill the
   `sparse_map::sparse_map::SparseMap` stutter).
6. **W6 — perf items** P1/P4/P6 (each behind a measurement gate per the HYBRID-perf rule).
7. **Deferred/parked:** framegraph crate placement (plan doc says boyko_render, code ships
   in boyko_rhi_vulkan — bless the backend location in the plan doc for now; promotion is a
   Phase-2-RDG concern); viewer extraction from tests/ into examples/ (W3-adjacent,
   optional); demo snapshot→dense-components port (dogfooding improvement).

## Part V — Positives to protect (do not regress)

- SAFETY discipline: ~131 unsafe sites in swapchain.rs with ~120 SAFETY comments; 2 real
  gaps in 4 kernel crates. Error-path teardown drains partial rings; fence-reset-only-on-
  commit avoids the OOD deadlock; per-image render_finished semaphores.
- Zero `HashMap` in non-test render code; every registry is a documented cold-path interner.
- Physics: SP4 remediation held (both solvers on kernel `ScratchColumn`); steady-state
  zero-alloc contract verified; 2 TODOs in 40 kLOC.
- boyko_ui retained-scratch + `mem::take` + set-if-changed tick discipline (where it IS
  used) is exemplary — extend it (P5) rather than dilute it.
