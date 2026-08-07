# Architecture: VG R3 piece 4 — the owner-facing occlusion config, and the instrument that can finally see the split

> **Design history.** Four architect passes (revisions 1–4) against four `architecture-critic` passes. Round 1: structural rework of the config surface. Round 2: the measurement story rebuilt around a zero-twin band discipline. Round 3: 3 blockers (the readback seam had no API path; the vacuity control reached no executing gate; the headline quantity contradicted its own derivation) + 7 conditions. Round 4 (this document's predecessor) closed all three blockers and folded all seven conditions; the round-4 verdict is **APPROVED-WITH-CONDITIONS** with seven new plan-text conditions and a **Preserve** list. This revision folds all seven, corrects the two drifted anchors the verdict names (`vb.rs:2467`, `vb_barrier_stream_baseline.rs:4350` — both re-verified in this session), and preserves the verdict's Preserve list verbatim in substance (marked ⟨P⟩).
>
> **Status: IMPLEMENTED. P4-1 … P4-7 all landed** — `49e5630`, `28c3772`, `85b3313`, `c7465bf`, `58687d3`, `cf2d367`, and this rung. Every anchor below was re-verified against branch `feat/multi-paradigm-render`, HEAD `c502c9f`, **at PLANNING time**. Line numbers drift — P4-7 measured this plan's own anchors moving by +8 and +20 lines, and three of the six it was handed were stale — so the implementer re-verifies at authoring time and treats a mismatch as a finding, not a transcription error.
>
> ⚠️ **Two sections were REPAIRED AFTER APPROVAL, by the rungs that executed them, and are marked in place:** §C2's band definition (a **plan-level defect**: the zero twin alone is a drift estimator, and P4-6 measured drift at structurally zero) and §C4's zero-control clause, which the shipped harness has never asserted in the form written here. A third divergence is recorded but not repaired: §P4-5's re-sourcing prediction is **refuted by the tree**, and the refutation lives at `vb_barrier_stream_baseline.rs`'s own P4-5 section — the plan text is left standing so the prediction and its refutation stay readable together.

## Changelog against revision 4

| # | condition | what changed |
|---|---|---|
| 1 | P4-4 control (ii) cannot fire | **The executable red is now a non-pinned GPU leg**: a fourth worker in `vb_occ_split_gate.rs` that arms occlusion **without** inserting `HzbConfig` and asserts `scopes == 2`. A3's byte-identity argument is restated as the *reason no pin can red* rather than as coverage. The claim is not withdrawn |
| 2 | C4 mixes TOP and BOTTOM stages | `off(2b) > e9` and `off(6b) > off(2b)` are **demoted to reported observations**; §B3 gains a paragraph deriving why a later-recorded TOP stamp may legally report an earlier time. The disarmed-leg slot-6 rule keeps its BOTTOM-vs-BOTTOM half, which carries the whole claim |
| 3 | slot 6's witness crosses a function boundary | Introduced **`TsWitness`** — a `record_vb`-local carrier owning the collector, both masks and the dev-profile counter, through which **every** stamp is recorded. `record_hzb_poison_build` (`vb.rs:4500`) takes `&mut TsWitness`; `finish` consumes it |
| 4 | the sibling hang class the epilogue cannot reach | **Structurally disarmed**: `disarm_vb_bench_unless_vb` runs once at `runner.rs:561`, the earliest instant the path is knowable (`GpuSceneBundles::boot` at `host.rs:246` precedes `resolve_render_path` at `runner.rs:554`). Disarm-then-panic; the disarm is the load-bearing half |
| 5 | the dual-read gate has no site | It is a **dev-profile invariant inside `read_vb_bench_ns`**, not an env knob: no new knob, nothing to compose with the P4-2 exclusivity panic. Its profile limitation is named |
| 6 | the deleted boot-decode's second rationale | **The regime is recorded in the artifact** (recorder-stamped `VbRecordProbe::occ_flags` + host-derived `VbProbeContext::occ_mode/occ_force`, schema bumped to 3) and in the bench summary as the **set of distinct regime words observed**. Constancy is recorded, never asserted |
| 7 | two drifted anchors | `vb.rs:2467` is the late scope's `cmd_end_rendering` (`:2472` is the block close; the post-late readback block is `:2490-2550`). `assert_row_is_pinned` is `vb_barrier_stream_baseline.rs:4350` (doc `:4334-4349`; the quoted phrase is the module-doc bullet at `:120-122`). Both re-verified |

---

## Goal

1. **An instrument.** Piece 3's timing triple returned `NOT RESOLVED` on every contrast for a structural reason: nothing brackets the cull. Piece 4 puts `vkCmdWriteTimestamp` brackets in the shipping recorder around ten frame units and re-runs the piece-3 protocol on the timestamp channel. First per-pass cost of the two-phase split in this repository.
2. **A config field.** `OcclusionConfig { mode: OcclusionMode }` — a `#[derive(Resource)]` singleton on `HzbConfig`'s surface, two variants — and `BOYKO_VG_OCC_FORCE` leaves shipping code, its regimes becoming a `boyko_app`-side diagnostic Resource.

Not a goal: making the split faster, changing its granularity, changing a pixel, re-blessing a pin.

## Context and constraints

### Facts, verified this session

| # | fact | anchor |
|---|---|---|
| F1 | `VbTimedPass` has exactly three members; `VB_PASS_COUNT == 3` | `gpu_timing.rs:203-230`, `:242` |
| F2 | `read_query_pool_ns` reads a prefix of pairs with `VK_QUERY_RESULT_WAIT_BIT` and **blocks forever** on a pair the recorder did not write | `gpu_timing.rs:186-194`; FFI contract restated at `rhi_impl/device.rs:1215-1221` |
| F3 | The hang class is **latent, not live**: `read_vb_bench_ns` has exactly one caller (`runner.rs:2610`), downstream of a **release-live** `assert!(host.resolved_render_path.mesh_leg, …)` at `runner.rs:1038-1042`. A `VisibilityBuffer × Sdf` bench boot **panics before frame 1** | those |
| F3b | `resolved_render_path` is resolved once, before the frame loop, and never changes mid-run | `runner.rs:488-498`, `:554`, `:561`, `:2094` |
| F4 | `record_vb` spans `vb.rs:601-4458`; exactly two `return`s — `:630` (`vkBeginCommandBuffer` failed, **before** `reset_frame` at `:640`) and `:4455` (after all recording). `end_command_buffer` at `:4452` | those |
| F5 | The `mesh_leg` block spans `vb.rs:1152-3055`; `if !occlusion_split { record_hzb_poison_build(…) }` at `:3068` is outside it | those |
| F6 | `CullReset`/`CullDispatch` are written unconditionally per armed frame (`vb.rs:716/755`, `:762/848`), **before** the `mesh_leg` block. `VbShade` has three mutually exclusive arms: `:2774/2938`, `:2945/3050` (inside the block) and **`:3573/3732`** (outside it, under `path_vb_split()`) | those |
| F7 | The collector is `Option<&VbTimestampCollector>`; `None` records **zero** commands (`vb.rs:640-645`), armed only under `BOYKO_VB_BENCH` + `timestamps_usable()` | `gpu_scene/mod.rs:1578-1585`, `:4453-4470`, `vb.rs:637-645` |
| F8 | **No pin sets `BOYKO_VB_BENCH`.** The 30 `[*.env]` blocks carry only `BOYKO_DISABLE_VALIDATION`, `BOYKO_SHADOW_DENOISE`, `BOYKO_HOST_DUMP`, the `BOYKO_VG_*` selectors and `RUSTUP_TOOLCHAIN` | `PINS.toml` |
| F9 | `record_hzb_poison_build` (`vb.rs:4500`) has exactly two call sites — **`:2034`** (`if occlusion_split`, inside the mesh-leg block) and **`:3069`** (`if !occlusion_split`, outside it) — and no early return | those |
| F10 | `path_vb_occlusion_split()` is `scene_types.rs:3943-3949`, five conjuncts; `VB_CULL_OCC_ARMED` is folded by **calling** it at `gpu_scene/mod.rs:6973-6988` | those |
| F11 | The FORCE field is `gpu_scene/mod.rs:1495-1502`, decoded at `:4297-4306`, stored at `:4623` | those |
| F12 | The barrier-stream gate is a **replica of the declarator**; `vb_cull_readback` is OFF across the matrix; the PROBE-ON row set is unauthored | `vb_barrier_stream_baseline.rs:110-118`, `:620`, `:912-916` |
| F13 | Present mode is `VK_PRESENT_MODE_FIFO_KHR`, unconditional | `present/swapchain.rs:199` |
| F14 | D12's fixed point: on a converged static scene the late scope correctly draws zero | `PINS.toml:449-451` |
| F15 | `[vb_occ_split].sha256_software` is the same literal as `[vb_mesh]`, `[vb_mesh_hzb]`, `[vb_both]`; the four `vb_occ_mixed*` pins share one literal | `PINS.toml:300`, `:332`, `:398`, `:584` |
| F16 | **All five occlusion pins run one fixture**: `test_binary="vb_mesh"`, `test_name="vb_mesh_screenshot_dump"` | `PINS.toml:393-394`, `:461-462`, `:486-487`, `:515-516`, `:543-544` |
| F17 | Cross-pin equality is machine-checked by `the_pins_declared_byte_identical_actually_agree` | `PINS.toml:361-364`, `:429-432`; `vg_density_census.rs` |
| F18 | `hzb_arm` is captured onto the targets (`targets.rs:8569`), joins the resync predicate (`:8687`) | those |
| F19 | `HzbConfig` is read **live per frame** via `hzb_plan_for(world.try_resource::<HzbConfig>().copied(), …)` | `runner.rs:2323-2327` |
| F20 | The in-tree statement "every golden run is [a dev-profile run]" is at **`graph_bridge.rs:3773`** (also `:4028`, `vb.rs:4473`) | those |
| F21 | The bench reduces by mean (`vb_bench_mean_ns`, `runner.rs:2861`); keys `cull_reset_ns= cull_dispatch_ns= froxel_cull_ns= froxel_shade_ns= froxel_total_ns=` (`:2939-2943`), `flat_shade_ns=` (`:2946`); `vg_occ_split_timing.rs:332` parses only `flat_shade_ns=`/`froxel_shade_ns=` | those |
| F22 | The bench serializes: `ctx.wait_idle()` on each bench frame before readback | `runner.rs:2605-2617` |
| F23 | `vb_occlusion_instances` comes from `MeshRenderScratch::occlusion_instances()` | `runner.rs:1487-1500` |
| F24 | The timing worker re-execs `current_exe()` and inherits the driver's profile; a release bench run has `debug_assertions` OFF | `vg_occ_split_timing.rs:277-278`, `:311-312` |
| F25 | `vg_occ_split_timing.rs` disarms A0/A1 by **not inserting the marker** (`Leg::marked()`, `:134-136`, consumed `:195`) | those |
| F26 | `vb_indirect_late`, `vb_late_visible`, `vb_late_count` are minted at **boot** by `GpuSceneBundles` (`gpu_scene/mod.rs:4098`, `:4190`, `:4203`) — **not** by `GBufferTargets` | those |
| F27 | `hzb_plan.rs` carries the "only `Build` produces a plan" claim twice: `:17` and `:26-29` | those |
| F28 | `read_query_pool_ns` and `read_query_pool_ticks` both funnel through `VulkanContext::fetch_query_pair_ticks` (`rhi_impl/device.rs:1181-1236`), which **compacts each pair into a masked delta in place** (`:1230-1234`). No absolute stamp crosses the seam. Trait defaults at `boyko_rhi/src/device.rs:874-884`, `:912-922` | those |
| F29 | Five App-booting fixtures each insert their own config: `vb_mesh.rs:315`, `vb_occ_mixed.rs:173`, `vb_occ_split_gate.rs:354`, `hzb_engine_pyramid_gate.rs:390/411/452`, `vg_occ_split_timing.rs:229`. **`mod vb_occ_mixed_scene;` is declared by only five files** — `vb_mesh.rs:55`, `vb_occ_mixed.rs:100`, `hzb_engine_pyramid_gate.rs:127`, `vg_occ_split_timing.rs:88`, `vg_occ_verdict_census.rs:42` — i.e. **`vb_occ_split_gate.rs` does not include it**, and `vg_occ_verdict_census.rs` boots no App | those |
| F30 | `BOYKO_VB_PROBE=<path.toml>` arms `boyko_app::vb_probe_dump` (`lib.rs:69`, `vb_probe_dump.rs:79-83`, requested `runner.rs:2545`, written `:2744`): **host-side counters** at the `vkCmd*` call sites (`vb.rs:1492`, `:2019`, `:2256`, `:2464`, `:2469-2471`), emitting `probe.scopes`, `late_draws`, `late_seed_instances`, `late_cull_dispatches`. It records **no** commands and does not enter `path_vb_occlusion_split()` | those |
| F31 | `targets.rs:8632` is a **`debug_assert!`** on `hzb_arm_matches_allocation`, not a release-live assert | that |
| F32 | **ANCHOR CORRECTED.** The late scope's `cmd_end_rendering` is **`vb.rs:2467`**; the probe `scopes` increment is `:2469-2471`; the `if occlusion_split` block closes at `:2472`. A **second** PROBE-ON readback block sits at **`vb.rs:2490-2550`** (`plan.vb_cull_readback_late`, `debug_assert!(occlusion_split && vb_cull_readback.is_some())`) — after slot 8's and slot 9's ends | those |
| F33 | The `cmd_fill_buffer`/`cmd_update_buffer` pair is `vb.rs:1604-1613`; `record_vb_pass(cull_pass)` is `:1621`; both inside the `batch_cull_armed` block | those |
| F34 | `VbTimestampCollector::write_begin` hardcodes `TimestampStage::TopOfPipe` (`gpu_timing.rs:305-308`); `write_end` hardcodes `BottomOfPipe`. The stage is a call-site-invisible property today | those |
| F35 | There is no Mock *backend crate*; the RHI's test doubles are `#[cfg(test)] struct MockApi/MockDevice` in `boyko_rhi/src/handle.rs:449-525` (`impl RhiDevice` `:476-514`, overriding no reader). No `boyko_rhi/tests/**` asserts any `Unsupported` default | those |
| **F36** | **NEW (condition 4).** `GpuSceneBundles::boot` (`gpu_scene/mod.rs:1650`) takes `(ctx, composite, swap_format)` and is called from **`host.rs:246`**, i.e. **before** `resolve_render_path` runs at **`runner.rs:554`** and writes `host.resolved_render_path` at **`:561`**. The resolved path is therefore *not knowable inside `boot`* | those |
| **F37** | **NEW (condition 4).** `record_vb` — hence `reset_frame` and every timestamp write — runs only under **`if scene.path_is_vb()`** (`frame_driver.rs:900`). A Deferred/Forward boot with `BOYKO_VB_BENCH` set arms the collector, resets nothing, writes nothing, and passes **both** surviving asserts (`mesh_leg` is true on `Deferred × Both`) | those |
| **F38** | **NEW.** The precedent for a bench-knob exclusivity assert is `runner.rs:1173-1177` (`BOYKO_VB_BENCH` ⊥ `BOYKO_SV0_BENCH`), whose own comment states the failure mode is *"a HANG, not a wrong number"* and that the `mesh_leg` / `!mesh_geo_shade_split` preconditions do **not** catch it | that |
| **F39** | **NEW (condition 6).** `VbRecordProbe` already carries recorder-observed **provenance** beside its counters, and `VbProbeContext` (`vb_probe_dump.rs:51-64`) carries the host's independent derivation — by design: *"the gate can compare two independent derivations … instead of comparing the recorder with itself"* (`:46-50`). The artifact is versioned (`schema_version = 2`, `:160`) with a stated bump discipline (`:156-159`): a schema change must make an old reader **fail loudly** rather than read a field whose meaning moved. `finish` **consumes** the driver so a second write is not expressible (`:119-121`) | those |
| **F40** | **NEW (condition 1).** `vb_occ_split_gate.rs` inserts `const HZB_BUILD: HzbConfig` on **all three** workers (`:354`, used `:367`, `:383`, and the multi worker) and the binary is **non-pinned** — no golden hashes anything it renders. Its doc (`:342-353`) states why the pyramid is armed even on the control worker | those |

### Target metrics

- **Instrument resolution.** `VbShade` measured CV 0.24–0.68 % in one sitting. A per-pass difference is reportable at ≳ 3× that (~2 %) provided the same-sitting zero twin agrees. ~60× better than channel W's floor (4.7 / 6.3 / 13.5 / 14.3 %, **not constant**).
- **Instrument frame cost.** 20 `vkCmdWriteTimestamp` + 1 `vkCmdResetQueryPool` per armed frame, in the existing command buffer. No new submission, no new fence, no new allocation. **Zero commands on every non-bench frame** (F7).
- **Pool footprint.** `2 × VB_PASS_COUNT = 20` queries × 8 B × `FRAMES_IN_FLIGHT` = 320 B, boot-owned.
- **Epilogue cost.** One `TsWitness` (two `u16`s + an `Option<&…>`); on an unarmed frame one `is_some()` test; on an armed frame 10 bit-pair tests and normally zero extra stamps.
- **Readback cost.** One extra `vkGetQueryPoolResults` argument set — the *same single* FFI call, now filling two `[f64; 10]` stack arrays instead of one. No allocation, no second device round trip. (Dev profile adds one idempotent re-read per bench frame; see §B1b.)
- **Config cost per frame.** Two `Copy` Resource reads at the existing `try_resource` site, one `Option<VbOcclusionArm>` field on `GBufferScene`, one extra conjunct in a predicate already evaluated twice per frame.

---

## Part A — the config

### A1. Two types, two crates, two audiences ⟨P⟩

**Decision.** `boyko_render`'s owner-facing enum has **exactly two** variants. The FORCE regimes are a diagnostic instrument and live in `boyko_app`.

`HzbMode`'s own doc sets the standard (`hzb_config.rs:73-76`): *"Exactly two variants, and that is permanent: this enum is the PRODUCER knob."* The consumer knob answers one question — does the owner want the occlusion decision — and a verdict override is not an answer to it. Two axes, two types; composition is `On × Force{None, KeepAll, DeferAll}`, and `Force` without `On` is inert **by the existing fold**: the FORCE bits are OR-ed only on a frame that takes the split (`gpu_scene/mod.rs:6988`) and the shader tests them only inside its guard (`gpu_scene/mod.rs:1499-1502`).

```rust
// crates/boyko_render/src/occlusion_config.rs   (new — the hzb_config.rs shape, verbatim)

/// Whether the engine performs the two-phase HZB occlusion decision on instances carrying
/// `OcclusionCulling`. `#[repr(u32)]` so the discriminant is a stable arm word.
///
/// Exactly two variants, for `HzbMode`'s reason: this is the CONSUMER knob. There is no quality
/// dimension (the decision is one conservative min-over-footprint predicate), and the diagnostic
/// verdict overrides are a different axis living in `boyko_app::OcclusionForce`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OcclusionMode {
    /// The 0%-gate and the DEFAULT: `path_vb_occlusion_split()` is false, no late passes are
    /// declared or recorded, no marked instance is ever tested. Byte-identical to a world that
    /// never inserts this Resource.
    #[default]
    Off,
    /// The shipping decision: the early phase tests every marked instance against the previous
    /// frame's pyramid and defers the rejected ones; the late phase re-tests them.
    TwoPhase,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct OcclusionConfig { pub mode: OcclusionMode }

impl OcclusionConfig {
    /// Structural, never stored: `mode != Off`.
    pub const fn enabled(&self) -> bool;
}
```

```rust
// crates/boyko_app/src/occlusion_force.rs   (new — the DIAGNOSTIC axis, host-side)

/// A verdict override for measurement and gating. NOT an owner knob: it exists so a fixture can
/// hold every mechanism constant and vary one push-constant bit (the `[vb_occ_mixed*]` ladder,
/// `PINS.toml:434-439`). Default `None`. Inert unless `OcclusionConfig` armed the split.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OcclusionForce {
    #[default] None,
    /// `VB_CULL_OCC_FORCE_KEEP` — the early phase defers NOTHING. `[vb_occ_mixed_keep]`.
    KeepAll,
    /// `VB_CULL_OCC_FORCE_LATE` — the early phase defers EVERY marked instance; the only regime
    /// in which a static scene reaches a nonzero late-survivor count (D12). `[vb_occ_mixed_late]`.
    DeferAll,
}

impl OcclusionForce {
    /// `0` | `VB_CULL_OCC_FORCE_KEEP` | `VB_CULL_OCC_FORCE_LATE` — never both, by construction.
    pub const fn flags(self) -> u32;
    /// The artifact spelling (`"none"` | `"keep"` | `"late"`), shared by the probe dump's decode
    /// and the fixtures' env decode so one table serves both directions.
    pub const fn as_str(self) -> &'static str;
}
```

**Location and composition.** `occlusion_config.rs` + `occlusion_plugin.rs` beside `hzb_config.rs`/`hzb_plugin.rs`; `OcclusionPlugin` registered in `EnginePlugins::build` immediately after `app.add_plugin(boyko_render::HzbPlugin)` (`plugins.rs:267`). One plugin per config family is the shipped mapping. `OcclusionForce` is **not** plugin-composed: `try_resource` returning `None` is its default, exactly as `hzb_plan.rs:26-29` treats an absent `HzbConfig`.

**Default `Off`,** in order of weight:

1. **Error asymmetry.** The split's failure mode is deleted geometry; its upside is bounded by the early raster's share of a frame. `OcclusionCulling` itself is opt-in for that reason.
2. **On this corpus the benefit is provably zero and the cost is not** (F14). A default that costs on every static scene and pays on none is not a default.
3. It is the 0%-gate every sibling config anchors on, so composing `OcclusionPlugin` unconditionally leaves all 30 pins byte-identical **by construction**.

### A2. Liveness — LIVE per frame; the fence-safety claim derived from ownership

**Decision: `OcclusionConfig` and `OcclusionForce` are read live, per frame, at `runner.rs:2323`,** beside `HzbConfig`. Not frozen, not `RenderPathFrozenConsumers`.

- **The drift class `RenderPathFrozenConsumers` guards does not exist here.** That carrier exists because a live SSAO/DDGI flip can make the light header ask the shade to combine a term whose pass was never armed (`hzb_config.rs:44-46`). The occlusion split has no header term: its arming is one predicate read by declarator, recorder and shader from **one folded word** (`gpu_scene/mod.rs:6988` folding `scene_types.rs:3943-3949`), computed after the flip, in the same frame, from the same scene.
- **A flip that changes `scene.hzb.is_some()` forces a full `GBufferTargets` recreate.** `hzb_arm` is captured onto the targets (`targets.rs:8569`) and joins the resync predicate (`:8687`). After A3's disjunct, `Off → TwoPhase` on a world whose `HzbConfig` is `Off` flips `hzb_arm` and takes that route.

**Derivation of fence safety, from where the state lives** (the argument does **not** rest on any assert; `targets.rs:8632` is a `debug_assert!`, F31, and is cited only as the in-tree *check* of the lockstep, never as its guarantee):

1. **The late buffers are not owned by `GBufferTargets`** (F26): `vb_indirect_late`, `vb_late_visible`, `vb_late_count` are minted at boot by `GpuSceneBundles`. A targets recreate does not destroy, recreate or reallocate any of them, so the dangling-handle class the resize fence exists for does not reach them at all.
2. **Seeding and consumption are same-frame and co-gated.** The `vb_indirect_late` fill (`vb.rs:1398`), `vb_cull_late` (`:2186`) and the late raster (`:2278`) read the *same local*, derived from one `path_vb_occlusion_split()` call on one assembled scene. A frame records all three or none.
3. **The recreated objects** (the HZB image, its per-mip views, `vb_cull_set`/`vb_set0_late`) are consumed only under `hzb.is_some()` — exactly what `hzb_arm` tracks, and what `hzb_arm_matches_allocation` (`targets.rs:1836`) checks at `:8632` **in the dev profile, which every golden run is** (F20).

The `Off → TwoPhase` recreate is therefore safe for the same reason a resize is, plus the stronger fact that the split's own state is not in the recreated set. Cost: a per-extent image + descriptor-ring rebuild plus `boot_clear_hzb_pyramid` / `boot_seed_hzb_null` — a stall, once, on the frame the owner flips a render feature.

- A flip on a world already carrying `HzbMode::Build` triggers no recreate (`hzb_arm` does not move).
- **D12's fixed point restarts after a recreate**: the first frame evaluates against a cleared pyramid, defers conservatively, the late phase re-admits; convergence takes one frame. Written down so a reader profiling the flip frame does not report the transient as steady state.
- **Markers with the config `Off`:** `Off` means **do not test**, never *do not gather*. `vb_occlusion_instances` stays `MeshRenderScratch::occlusion_instances()` (F23). Gating the gather on the config would make the counter mean two things depending on a knob. Cost of a marked-but-disarmed world: one `u32` read.
- **Liveness costs one guarantee, and piece 4 pays for it in the artifact, not in prose** — see A5's provenance rule (condition 6).

### A3. The pyramid dependency — the disjunct stays; **three** doc repairs land in the same commit

```rust
// boyko_app/src/hzb_plan.rs
pub(crate) fn hzb_plan_for(
    hzb: Option<HzbConfig>,
    occ: Option<OcclusionConfig>,   // NEW
    width: u32, height: u32,
) -> Option<HzbPlan>;                // Some iff hzb.enabled() || occ.enabled()
```

`TwoPhase` with `HzbConfig::Off` would arm nothing (the `hzb.is_some()` conjunct, `scene_types.rs:3947`) and say nothing — a silently-dead knob. The tree refuses that explicitly elsewhere: `SsaoPlugin` was *"deliberately NOT composed: … composing it would ship a silently-dead knob"* (`plugins.rs:233-235`). The fixtures already solved it locally (`vb_mesh.rs:314`); piece 4 promotes the disjunction into the host **and deletes the fixture's `|| occ_marked()` half in the same commit**.

| site | today | repair |
|---|---|---|
| `hzb_config.rs:80-88` (`HzbMode::Off`) | *"No pyramid — the 0%-gate: … not one recorded command."* | `Off` means **this config does not ask for a pyramid**. Since piece 4 a pyramid is also built when a CONSUMER needs one (`OcclusionMode::TwoPhase`). Cites `occlusion_alone_plans_a_pyramid` **and** the GPU leg `vb_occ_probe_dump_marked_no_hzb` (below). |
| `hzb_plan.rs:26-29` | *"Only `HzbMode::Build` produces a plan."* | A plan is produced iff a producer asks **or** a consumer needs. Cites that test plus `off_and_absent_both_yield_no_plan` (now `(None, None)`). |
| `hzb_plan.rs:17` | the module header repeats the claim in the degrade paragraph | reworded to "the same disarmed state a plan-less config pair produces". Same citation. |
| `hzb_config.rs:42-51` (*"does NOT join `RenderPathFrozenConsumers`"*) | rests on *"the pyramid … is read by nothing"* — **stale since piece 3** | the dead half is deleted; A2's ownership derivation replaces it. The lockstep check is cited as a **`debug_assert!`** (`targets.rs:1836`, checked at `:8632`) **and the sentence says so** — a repair that upgraded it to "release-live" would be this campaign's fourth doc-rot repair to introduce a new lie. |

**Byte-identity: zero pins move — and that is exactly why no pin can serve as the disjunct's control** (condition 1). The disjunct can only change a run that arms occlusion **without** `BOYKO_VG_HZB`. All five occlusion pins set `BOYKO_VG_HZB = "1"` (`PINS.toml:411`, `:474`, `:499`, `:528`, `:556`) and the other 25 arm no occlusion, so deleting the disjunct leaves every pinned configuration — and `vb_mesh_occ_pins_actually_split` — **green**. The executable red therefore has to come from a configuration no pin renders:

> **`vb_occ_probe_dump_marked_no_hzb`** — a **fourth worker** in the non-pinned `vb_occ_split_gate.rs` (F40): the same marked single-batch scene, `VB_MESH_PATH`, occlusion armed to `TwoPhase` through `occ_fixture`, and **`HZB_BUILD` deliberately NOT inserted**. Its driver asserts `scopes == 2`. It is not the marked/unmarked control pair's twin and must not be read as one: its partner is `vb_occ_probe_dump_marked` (identical in every respect except the `HzbConfig` insert), so a green pair means "the pyramid arrives by either route" and a red on this leg alone means "the consumer route is gone". Costs no blessing — the binary is pinned by nothing.

### A4. Composition with the per-instance marker

```
split(frame) = owner asked        (OcclusionConfig::enabled)
             ∧ capability present (vb_occlusion_instances > 0)
             ∧ path can do it     (path_is_vb ∧ mesh_leg ∧ hzb.is_some() ∧ vb_mesh_bounds.is_some())
```

```rust
// scene_types.rs — GBufferScene
/// VG R3 piece 4: the OWNER's arming, threaded from `OcclusionConfig` by the runner. `None` on the
/// default `OcclusionMode::Off` — no split, no late passes (the `hzb: Option<HzbPlan>` shape).
pub vb_occlusion: Option<VbOcclusionArm>,

// scene_types.rs:3943 — the conjunct is PREPENDED to the five that exist
pub fn path_vb_occlusion_split(&self) -> bool {
    self.vb_occlusion.is_some()          // NEW — the owner knob
        && self.path_is_vb()
        && self.resolved_render_path.mesh_leg
        && self.vb_occlusion_instances > 0
        && self.hzb.is_some()
        && self.vb_mesh_bounds.is_some()
}

/// Presence IS the arming; the payload is which verdict is forced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VbOcclusionArm { pub force_flags: u32 }   // 0 | FORCE_KEEP | FORCE_LATE, never both
```

The fold at `gpu_scene/mod.rs:6988` becomes `VB_CULL_OCC_ARMED | scene.vb_occlusion.map_or(0, |a| a.force_flags)`, still gated by **calling** `path_vb_occlusion_split()` on the assembled scene — so **ARMED ⇒ split stays true by construction**, the property the mandate forbids regressing. The `debug_assert!` at `vb.rs:1656-1662` stays.

### A5. What happens to `BOYKO_VG_OCC_FORCE`

**Deleted from shipping code** (`gpu_scene/mod.rs:4297-4306` and the field `:1495-1502`, F11), re-implemented in the fixtures.

- One source of truth: today the arming is an ECS-derived per-frame predicate while the regime is a boot-time env read in another crate; they can disagree and nothing checks them.
- It removes an `env::var` **and a boot panic** from shipping code.
- **No pin file changes and no re-blessing.** The `[*.env]` blocks keep `BOYKO_VG_OCC_FORCE = "keep" | "late"` verbatim (`PINS.toml:501`, `:558`); the *fixture* translates env → Resource, exactly as it already translates `BOYKO_VG_HZB` → `HzbConfig`.
- **The panic moves with the decode.** A typo'd regime must never silently render the default.

Decode **and insert** live in exactly one place — the new `occ_fixture` module (P4-4).

**The deleted decode's second rationale, answered in the artifact (condition 6).** `gpu_scene/mod.rs:4290-4292` justifies the boot read partly by: *"a knob that can change mid-run would make 'which regime produced this capture?' unanswerable from the artifact."* A live Resource reopens exactly that. Piece 4 does **not** answer it by asserting constancy — an assertion would have to hold on hosts this repository does not own. It answers it the way this campaign already answers provenance questions (the recorder-stamped depth-dump frame index, commit `4c0d5dc`): **by recording the regime in the artifact, from the recorder, on the frame it describes.**

| carrier | what it records | source |
|---|---|---|
| `VbRecordProbe::occ_flags: u32` | the **exact word pushed to the GPU** on the probed frame | stamped in `record_vb` beside the existing counters (F30's sites) |
| `[probe] occ_flags` / `occ_regime` in the dump | that word, plus its `"none"/"keep"/"late"` decode | `write_probe`, `vb_probe_dump.rs:162-163` region |
| `VbProbeContext::occ_mode` / `occ_force` | the **host's** independent view — what the Resources said at that frame's `try_resource` read | `runner.rs:2323` region, the `[host]` table |
| `VB-P4 regime observed=[…] n_distinct=<k>` | the **set** of distinct regime words seen across the bench's timed frames | the `#[cold]` summary fn |

The two probe carriers are derived at different sites, so the gate compares two derivations rather than one site with itself — `vb_probe_dump.rs:46-50`'s stated design principle, applied to provenance instead of counts. A mid-run flip is then **visible** (`n_distinct > 1`, or a `[probe]`/`[host]` disagreement) instead of silently attributed. `schema_version` bumps **2 → 3** so a schema-2 artifact read by a schema-3 reader fails loudly rather than defaulting the regime to `"none"` and certifying the wrong one (F39's own bump discipline).

Within the pinned corpus constancy additionally holds **by construction, not by assertion**: `occ_fixture` inserts once at setup and no fixture mutates the Resource afterwards.

⚠️ **After P4-4, `ForceKeep` is no longer "the disarm route"** — `OcclusionMode::Off` is, and unlike `ForceKeep` it suppresses the split predicate, the late passes and the extra descriptor-set bindings as well as the verdict. `PINS.toml:481` said otherwise; **P4-7 repaired that comment** (comments are not hashed) and, unbooked, the two neighbouring claims in the same file that P4-4 had also falsified: the `[vb_occ_split.env]` rationale *"the fixture makes `BOYKO_VG_OCC` imply `HzbMode::Build`"* (the fixture's `|| occ_marked()` half was deleted at P4-4 — the redundancy now comes from the HOST disjunct) and the `[vb_occ_mixed*]` family's *"all four `sha256_*` are seeded PENDING"* (the four SOFTWARE legs were blessed at `e160434`; only the hwrt legs are still `PENDING`).

⚠️ **`Off` is still not an allocation-backed disarm.** It suppresses the decision, the late passes and the second/third descriptor-set *binding*; the buffers, `hzb_null`, the widened layout and the sets are still minted on every VB boot (F26). Piece 3's Boundary costed that; OQ 8 owns it.

---

## Part B — the timestamp bracket

### B0. What the hang actually is, and what P4-1 changes ⟨P⟩

The **latent** hang class (F3/F3b) is not the reachable defect. The reachable defect is that **a legal render-path configuration cannot be measured at all**, and that the totality the readback needs is purchased by a boot-time panic standing in for a per-frame invariant. Three properties make that the wrong trade:

- The guarantee is **premise-shaped**, not structural: it holds because `mesh_leg` happens to be boot-constant and happens to gate every writer. P4-2 puts seven more writers under that gate; every future writer inherits the obligation silently and nothing in `record_vb` states it.
- The guarantee is **caller-local**: `gpu_timing.rs:186-194` records that a second caller resurrects the hazard, and `:222-228` records that this already happened once.
- **A `debug_assert!` cannot substitute** (F24): the timing worker inherits the driver's profile, and a release bench run has `debug_assertions` OFF. Only a **release-live** mechanism prevents the hang in the configuration that matters.

**And the epilogue alone does not close the class** (condition 4). The collector arms on `BOYKO_VB_BENCH` + `timestamps_usable()` alone (F7), while every writer — and `reset_frame` — lives inside `record_vb`, which runs only under `scene.path_is_vb()` (F37). A `Deferred × Both` boot with the knob set passes **both** surviving asserts and reaches the readback on a pool that was never even reset. No epilogue inside `record_vb` can reach a frame that never calls `record_vb`. This is the sibling of the hazard `runner.rs:1173-1177` already documents for the SV0 bench (F38).

**Decision.** P4-1 lands three things in one commit:

1. A **release-live totality epilogue** in `record_vb` (§B1) — covers every VB frame.
2. A **structural disarm** of the collector on a non-VB resolved path (§B0b) — covers every non-VB boot.
3. The **replacement** of `runner.rs:1038-1042`'s `assert!` by a `#[cold]` printed scope note, and the **repair** of the stale comment block at `:1043-1053`.

Together those give a complete chain: *collector armed ⇒ path is VB ⇒ every presented frame ran `record_vb` to completion ⇒ every pair written* — and the readback runs only after `presented_ok` (F22), so a recreate-skipped frame reads nothing.

**Condition 7 of the previous round, disposed in one sentence used in both places:** piece 4 **keeps** the `assert!` at `runner.rs:1054-1059` (a VB-P1d SCOPE statement, already re-doc'd as such at `gpu_timing.rs:222-228`) and **repairs the stale comment block at `runner.rs:1043-1053`**, whose claim that a split-armed frame "leaves the VbShade pair reset-but-never-written" is refuted by the third producer arm at `vb.rs:3573`/`:3732` (F6). The repaired text states the true reason the assert survives: the break-even number is defined against the fused/classified tail, not that the split arm hangs.

### B0b. The structural disarm (condition 4)

```rust
// GpuSceneBundles — a &mut method, called ONCE, cold.
/// VG R3 P4-1: the VB-P1d collector's writers all live in `record_vb`, which the frame driver
/// calls only under `scene.path_is_vb()` (`frame_driver.rs:900`). `boot` cannot key the arming on
/// the path because `boot` runs FIRST (`host.rs:246`) and the path is resolved later
/// (`runner.rs:554`), so the gate lands at the earliest instant the answer exists.
#[cold]
pub(crate) fn disarm_vb_bench_unless_vb(&mut self, resolved: &boyko_render::ResolvedRenderPath);

/// The ONE predicate `scene()` and the runner both read, so the two can never disagree about
/// whether this frame carries a collector.
pub(crate) fn vb_timing_for_frame(&self) -> Option<&VbTimestampCollector>;
```

- **Site:** `runner.rs`, immediately after `host.resolved_render_path = resolved_render_path;` (`:561`).
- **Mechanism:** sets a `vb_bench_disarmed: bool`; `vb_timing_for_frame()` folds it with `self.vb_bench.as_ref()`; `scene()` (`:6641` region) and `vb_bench_armed()` both route through it. The pools are **not destroyed** — they are boot-time allocations on a diagnostic knob, off every shipping path, and inventing a mid-run destroy sequence to reclaim 320 B would add a lifetime question for nothing.
- **Then a `#[cold]` panic** naming both the knob and the resolved path — because a silently declined bench is the O2 failure the tree already refuses (`gpu_scene/mod.rs:4455-4463`): the windowed run would simply never terminate.
- **Which half is load-bearing, stated so a later editor cannot get it wrong:** the **disarm** is. Delete the panic and the hang class stays closed (the collector is `None`, `record_vb` records nothing, the readback is never reached); delete the disarm and the panic is again the only thing between the operator and an infinite wait — which is precisely the shape P4-1 is removing at `runner.rs:1038`. They are ordered disarm-then-panic for that reason.

### B1. The totality epilogue — one witness, two masks, three states

**Every stamp is recorded through one carrier** (condition 3), so no call site can record a timestamp without witnessing it, and no function boundary can lose the witness:

```rust
// record_vb-local. Constructed AFTER `reset_frame` (vb.rs:640), so "the witness exists" structurally
// implies "the pool was reset this frame". `finish` CONSUMES it — a stamp after the epilogue is not
// expressible (the `VbProbeDump::finish` precedent, vb_probe_dump.rs:119-121).
struct TsWitness<'a> {
    tc: Option<&'a VbTimestampCollector>,   // None => every method is a no-op
    begun: u16,                             // bit p set when pass p's BEGIN was recorded
    ended: u16,                             // bit p set when pass p's END   was recorded
    #[cfg(debug_assertions)] writes: [u8; 2 * VB_PASS_COUNT as usize],
}

impl TsWitness<'_> {
    unsafe fn begin(&mut self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, p: VbTimedPass);
    unsafe fn end  (&mut self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, p: VbTimedPass);
    unsafe fn finish(self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize);
}
```

`record_hzb_poison_build` (`vb.rs:4500`) gains a `ts: &mut TsWitness<'_>` parameter, threaded from both call sites (`:2034`, `:3069`). Setting the bit at the *call site* instead would restore exactly the premise-shaped guarantee P4-1 exists to delete: the bit would be a claim about a function's body made from outside it.

```rust
// ... immediately before `end_command_buffer` (vb.rs:4451) ...
ts.finish(self.fns, cmd, fi);

// TsWitness::finish — release-live
// VG R3 P4-1: TOTALITY. `read_vb_bench_ns` reads every pair with VK_QUERY_RESULT_WAIT_BIT and
// blocks forever on one this frame did not write (`gpu_timing.rs:186-194`). Which brackets executed
// is a property of the LEG (mesh/sdf, split/unsplit, classified/fused), so no per-site gate can be
// the guarantee. This is one. It keys on BOTH halves: re-stamping an already-written begin query
// after one reset is itself invalid (VUID-vkCmdWriteTimestamp: the query must be unavailable), so a
// torn pair must be CLOSED, not re-opened. Torn is unreachable under F4's single-exit shape today —
// and that is exactly the premise-shaped guarantee this epilogue exists to stop relying on.
for p in 0..VB_PASS_COUNT {
    match (self.begun & (1 << p) != 0, self.ended & (1 << p) != 0) {
        (true, true)   => {}                                    // written, nothing to do
        (false, false) => unsafe { tc.write_zero_pair(fns, cmd, fi, VbTimedPass::from_slot(p)) },
        (true, false)  => unsafe { tc.write_end     (fns, cmd, fi, VbTimedPass::from_slot(p)) },
        (false, true)  => { /* recorder bug: end without begin — reported by the dev counter */ }
    }
}
debug_assert_eq!(ended_after_fill, (1u16 << VB_PASS_COUNT) - 1, "…");
debug_assert_eq!(self.begun & !self.ended, 0, "invariant: no VB timestamp pair is left torn");
// SAFETY (both writes): recording is open; the pool was reset this frame (`reset_frame`, :640) —
// implied by the witness's construction site; `fi` is this present's in-flight slot; `p <
// VB_PASS_COUNT` so both query indices are in bounds; each query is written at most once after the
// reset (the masks are the witness).
```

**Why `write_zero_pair` and not `write_begin` + `write_end`.** `write_begin` stamps `TOP_OF_PIPE` (F34). At the *frame top* that is harmless — nothing precedes it, which is why `CullReset`'s always-written pair reads ~0 on a boot with no froxel arm (`gpu_timing.rs:205-210`). At the *frame end* it is not: a `TOP_OF_PIPE` stamp fires as the command reaches the front of the pipe, a `BOTTOM_OF_PIPE` stamp only after the entire preceding frame has completed, so a TOP/BOTTOM fallback pair would report **the whole frame's drain time** as that pass's cost — a large, plausible-looking, fabricated number. `write_zero_pair` records **both** queries at `BOTTOM_OF_PIPE`, back to back, nothing between them: the two stamps wait on prefixes differing by nothing, so the delta is the lattice quantisation and is a genuine zero.

**Three outcomes, three labels.** A delta alone cannot distinguish a fallback from a genuinely zero-cost pass — both read ~0. §B1b's readback returns each pair's **begin offset**, which does:

| label | condition | reported as |
|---|---|---|
| MEASURED | begin offset sits at the slot's recording position for that leg | a number |
| FALLBACK | `(false,false)` filled at the epilogue ⇒ begin offset is the frame-end offset, the largest in the frame and out of record order | `pass=<label> FALLBACK`, excluded from every aggregate |
| TORN | `(true,false)` closed at the epilogue ⇒ begin offset is in position but the duration runs to the frame end | `pass=<label> TORN`, excluded, and the run is **rejected** |

**Totality claim, with its one exception (F4).** *On every frame that reaches `vb.rs:4451` with a live witness, every `VbTimedPass` pair is written exactly once.* The one path that does not reach it is `vb.rs:630` (`vkBeginCommandBuffer` failed) — which precedes `reset_frame` at `:640`, so that frame resets nothing, writes nothing, returns `Err`, and the caller never reaches the readback. `vb.rs:4455` is after the epilogue. There is no third exit.

**Double-write detection stays a dev-profile counter.** A slot written twice after one reset is a VUID violation and a silently wrong delta, not a hang, so nothing else catches it and the epilogue cannot. `TsWitness::writes` is incremented at each stamp and asserted `<= 1` at `finish`, slot named via `label()`.

### B1b. The readback seam — the new RHI verb

F28 is decisive: **the API compacts pairs into deltas in place and no absolute stamp survives the seam.** Every control that distinguishes FALLBACK/TORN from a measurement rests on offsets, so the verb is designed here, not assumed.

```rust
// crates/boyko_rhi/src/device.rs — a THIRD reader beside read_query_pool_ns / _ticks (:874-922)

/// [`Self::read_query_pool_ns`]'s UNCOMPACTED sibling: the same host-wait + read + mask, but
/// returning BOTH halves of each pair — `out_dur_ns[i]` is bit-for-bit the value
/// `read_query_pool_ns` produces, and `out_begin_ns[i]` is pair `i`'s BEGIN stamp expressed as an
/// offset from pair 0's BEGIN stamp.
///
/// # Why a caller would want offsets
///
/// A pair's DURATION cannot distinguish "this pass cost nothing" from "this pass was never
/// bracketed and a totality epilogue filled it at the frame end" — both read ~0. The begin OFFSET
/// can: a filled pair's offset is the frame's largest and out of record order. A harness that
/// cannot tell those apart reports fabricated zeros as measurements.
///
/// # The base, and the wrap rule
///
/// `base = scratch[0] & mask` — pair 0's begin stamp. Offsets use the SAME arithmetic as the
/// durations: `off_i = (begin_i & mask).wrapping_sub(base) & mask`, scaled by `timestampPeriod`.
/// CALLER CONTRACT: pair 0's begin must be the EARLIEST-recorded stamp of the frame, else that
/// pair's offset wraps to ~`2^timestampValidBits` instead of going negative. Vulkan guarantees
/// `timestampValidBits >= 36` on any queue that supports timestamps (≈68 s at 1 ns/tick), so a
/// genuine counter wrap inside one submitted frame is not reachable; a huge offset means the
/// contract was broken, and the caller is expected to reject the sample rather than scale it.
///
/// Same contract as the siblings otherwise: `WAIT_BIT` semantics (only WRITTEN pairs may be
/// requested), `scratch` is caller-owned staging of length `>= 2 * pair_count` and is CLOBBERED,
/// both out slices receive `pair_count` values.
///
/// The default body is `#[cold] #[inline(never)]` and errors `Unsupported`; the Vulkan backend
/// overrides it (`vkGetQueryPoolResults`).
#[cold]
#[inline(never)]
fn read_query_pool_pairs_ns(
    &self,
    _pool: &A::QueryPool,
    _pair_count: u32,
    _scratch: &mut [u64],
    _out_begin_ns: &mut [f64],
    _out_dur_ns: &mut [f64],
) -> Result<(), Self::Error> {
    Err(RhiError::unsupported("read_query_pool_pairs_ns").into())
}
```

**Two parallel `&mut [f64]` slices, not `&mut [(f64, f64)]`** — SoA at the seam, matching the shipped `out_ns`/`out_ticks` shape, and no tuple layout crosses the RHI boundary.

**Vulkan side — one FFI call, three consumers.** `fetch_query_pair_ticks` (`rhi_impl/device.rs:1181-1236`) splits into:

- `fetch_query_raw_ticks(pool, query_count, scratch)` — the `vkGetQueryPoolResults` call and its `is_success()` handling, moved **verbatim** (`:1197-1224`), leaving `scratch[0..query_count]` as raw stamps;
- `fetch_query_pair_ticks` — calls it, then runs the existing ascending in-place compaction (`:1226-1234`) **unchanged**, so its "no input is destroyed before it is read" proof at `:1171-1176` still holds word for word;
- `fetch_query_pair_stamps` — calls it, then writes `out_begin`/`out_dur` into **separate** slices (so there is no in-place aliasing question at all) with the arithmetic above.

**Bit-identity of the shipped number is a gate, not a hope — and it has a site (condition 5).** `vkGetQueryPoolResults` is idempotent until the pool is reset, so both readers may read one pool in one frame. The dual read is **not** an env knob and **not** a one-off harness: it is a `#[cfg(debug_assertions)]` block **inside `GpuSceneBundles::read_vb_bench_ns`**, which after this rung reads via `read_query_pool_pairs_ns` and, in the dev profile only, re-reads the same pool via `read_query_pool_ns` and asserts `out_ns[i] == out_dur_ns[i]` **exactly** (both are `f64` from identical integer math) on every slot, every bench frame.

- **Why there:** it is the only site that holds both a live pool and the shipped consumer, and it makes the equality a standing invariant rather than a rung-local ceremony — a future edit to the compaction cannot silently diverge from the stamps path.
- **What it composes with: nothing.** It touches only the timestamp pool; `BOYKO_VB_CULL_READBACK` is excluded from every bench boot by P4-2's exclusivity panic, and no new knob exists to interact with.
- **Its limit, named:** the check runs in the dev profile. P4-1's gate run is `cargo test` (dev) and executes it; a release bench run (F24) does not. It is an implementation-equality check, not a safety mechanism, and nothing else in the plan depends on it holding at run time.

**Default-body coverage.** There is no Mock backend crate; the RHI's doubles are `#[cfg(test)] MockApi`/`MockDevice` in `boyko_rhi/src/handle.rs:449-525`, which override no reader (F35). P4-1 adds the first `Unsupported`-default unit test in that module for this verb, and says in its doc comment that the siblings have none — a new precedent, stated as new.

### B2. The brackets

`gpu_timing.rs:184-194` argues for separate collectors, and the argument is F2: widening the **shared** `PASS_COUNT` makes an existing harness demand pairs its recorder never writes. It does not apply to the VB collector — the epilogue plus the path disarm make that state unreachable, and `read_vb_bench_ns`'s single caller reads the same collector the same recorder fills. So `VbTimedPass` grows 3 → 10, one collector, one pool per FIF, one readback. **Slots 0/1/2 keep their meaning** so VB-P1d's published numbers stay comparable.

| slot | member | opens | closes | gate it sits OUTSIDE |
|---|---|---|---|---|
| 0 | `CullReset` *(existing)* | `vb.rs:716` | `:755` | froxel cull wiring |
| 1 | `CullDispatch` *(existing)* | `:762` | `:848` | froxel cull wiring |
| 2 | `VbShade` *(existing)* | 3 producer arms | same | — |
| 3 | `VbLateUpload` | before the late-record fill block (`if occlusion_split`, `:1398`) | after it | `occlusion_split` |
| 4 | `VbEarlyCull` | before `if batch_cull_armed {` (`:1528`) | after that block (`:1824`) | `batch_cull_armed` |
| 5 | `VbEarlyRaster` | before `record_vb_pass(plan.vb_raster …)` (`:1831`) | after the scope's `cmd_end_rendering` (`:2014`) | — |
| 6 | `VbHzbBuild` | first statement of `record_hzb_poison_build` (`:4500`) | last statement | both call sites, ONE bracket site |
| 7 | `VbLateCull` | before `if occlusion_split {` (`:2186`) | after that block (`:2258`) | `occlusion_split` |
| 8 | `VbLateRaster` | before `if occlusion_split {` (`:2278`) | after that block (`:2472`) — i.e. after the scope's `cmd_end_rendering` at **`:2467`** and after the probe increment at `:2469-2471` | `occlusion_split` |
| 9 | `VbRun` | immediately before slot 3's begin | immediately after slot 8's end | — |

**Record order is LEG-DEPENDENT, and the harness knows which slots move.** Two slots move; the run does not:

| leg | order of BEGIN stamps |
|---|---|
| armed (B, C) | `0 1` ‖ **`9b 3 4 5 6 7 8 9e`** ‖ `2` |
| disarmed (A0, A1) | `0 1` ‖ **`9b 3 4 5 7 8 9e`** ‖ `2` ‖ `6` |

- Slot 6 moves because `record_hzb_poison_build` has two call sites on opposite sides of the shade producer (F9: `:2034` armed, `:3069` disarmed).
- Slot 2 moves *within* the mesh-leg block between the classified (`:2774`) and fused (`:2945`) arms; the split arm (`:3573`) is excluded from every timing leg by `runner.rs:1054-1059`.
- The bold span is **identical on all four legs**, which is what C4's monotonicity clause is scoped to.

**Slot 6's two call sites** are mutually exclusive on the same local and the function has no early return (F9), so first-statement/last-statement brackets are written exactly once per call — and the witness travels into the function, so the masks record the fact rather than the caller predicting it. The bracket site names the mutual exclusion as load-bearing; the dev-profile double-write counter reds if a future edit makes both reachable.

**Slots 3/4/5/7/8/9 sit inside the `mesh_leg` block (F5) and that is now irrelevant** — the epilogue supplies their pairs on a non-mesh-leg frame and marks them FALLBACK, so no number pretends they measured anything.

**The readback probe's cost, both halves.** Under `BOYKO_VB_CULL_READBACK` the probe records copies in **two** places: the pre-snapshot inside slot 4's extent (`vb.rs:1722-1824`) and a **post-late** block at `vb.rs:2490-2550` (F32), which sits after `e8` **and after `e9`**. Rather than stretch slot 9 over a diagnostic (which would make the shipped run's headline interval depend on a probe), P4-2 makes the two instruments **mutually exclusive at boot**: if `BOYKO_VB_BENCH` and `BOYKO_VB_CULL_READBACK` are both set, the runner panics from a `#[cold]` boot path naming both knobs — the `runner.rs:1173-1177` precedent (F38), for the same reason. Both are diagnostic env knobs, neither can reach a shipping run, and today no configuration sets both. Consequence: **no published timestamp number contains either half of the probe's cost**, and the plan states that instead of qualifying slot 4.

### B3. Stage semantics, and the headline

**Stages.** Slots 0–2 keep `TOP_OF_PIPE` begins — VB-P1d's published break-even is defined against them and redefining it would silently invalidate a published number, at a documented ~3 % bias (`runner.rs:2891-2895`). **Slots 3–9 stamp `BOTTOM_OF_PIPE` at both ends.** The stage becomes a property of the pass, consulted inside `write_begin` (F34: it is hardcoded today), so no call site can drift:

```rust
impl VbTimedPass {
    /// TOP for slots 0..2 (kept: VB-P1d's published numbers are defined against it);
    /// BOTTOM for 3..9 (partitioning brackets — see the module doc's derivation).
    pub const fn begin_stage(self) -> TimestampStage;
    /// The printed key. Table-driven so no summary site can drift from the enum.
    pub const fn label(self) -> &'static str;
    pub const fn from_slot(slot: u32) -> Self;
}
```

**The derivation ⟨P⟩.** `vkCmdWriteTimestamp` at stage `S` writes when all previously-submitted commands have completed execution up to `S`. For `BOTTOM_OF_PIPE` that is: all prior commands have fully completed — a **prefix-completion time** `t_k`. Two mechanical consequences:

1. **Monotonicity and partition.** Prefixes are nested, so `t_k` is non-decreasing. For consecutive `BOTTOM` stamps `t_0 ≤ … ≤ t_m`, the intervals `[t_k, t_{k+1}]` **exactly partition** `[t_0, t_m]`: `Σ (t_{k+1} − t_k) = t_m − t_0`. No time is double-counted, none is lost.
2. **Direction of migration.** If commands after stamp `k` execute concurrently with commands before it, `t_k ≥ completion(A)`, so the overlap is already inside `[t_{k-1}, t_k]`: **overlap is charged to the EARLIER interval.**

**What the derivation does NOT license: comparing a TOP stamp with a BOTTOM stamp** (condition 2). A `TOP_OF_PIPE` stamp waits only for prior commands to *reach* the top of the pipe, so a TOP stamp recorded **after** a BOTTOM stamp may legally report an **earlier** time — the two are not measurements of the same quantity and are not ordered by record position. Every clause in §C4 that would compare slot 2's begin (TOP, kept for compatibility) with a BOTTOM stamp is therefore a **reported observation**, not an assertion. Only BOTTOM-vs-BOTTOM comparisons carry the partition property.

Applied to the run: stamps `b9 … e9` are a consecutive `BOTTOM` run, therefore within it

```
m3 + m4 + m5 + m6 + m7 + m8 + gaps = m9        (exact; on the ARMED legs, where slot 6 is inside)
```

The gaps are enumerable: `e5(:2014) → b6(:2034)` holds a host-side counter increment only; `e6 → b7(:2186)` holds the EARLY-DEPTH dump copy, gated `occlusion_split && scene.hzb_dump`, which no timing leg arms; `e7(:2258) → b8(:2278)` holds a host-side counter; `e3 → b4` and `e4 → b5` hold nothing.

**The headline.**

```
NetRun_r := Δ9  =  m9(C) − m9(B)      ← THE HEADLINE. + ⇒ the decision costs more than it saves.
```

`Δ9` is a paired difference of two intervals delimited by stamps at identical positions and identical stages on command streams that differ **only** in one push-constant bit and the indirect counts it produces. Within-run migration between slots 3–8 is zero-sum by the partition property and cancels exactly. B and C are both split legs, so slot 6 sits at `vb.rs:2034` on both; subtracting `Δ6` here would cure nothing and inject variance into the one number the decision table rides on. The per-slot terms remain as attribution:

```
Saving_r   := −Δ5                              (+ ⇒ the decision SHRINKS the early raster)
Overhead_r := Δ3 + Δ4 + Δ7 + Δ8                (attribution only, no bound in either direction)
Net_r      := Overhead_r + Δ6 − Saving_r       (all six in-run slots)
GapResidual_r := NetRun_r − Net_r  (= Δgaps)   (CHECKED: must be ≈ 0)
HzbResidual_r := Δ6                            (CHECKED: must be ≈ 0 — same dispatch chain,
                                                same site, same pyramid extent on B and C)
```

**Where the `−Δ6` subtraction genuinely belongs** is the armed-vs-disarmed contrast, because there slot 6 leaves the run entirely (`:3069` is after `e9`):

```
PlumbRun_r := [ m9(B) − m6(B) ] − m9(A0)       (attribution-grade, not headline-grade)
```

with `m6(B)` and `m6(A0)` also reported as bare magnitudes. `PlumbRun` is labelled attribution-grade because removing an interval from inside a partition cannot undo migration into or out of it.

**What remains exposed, and its direction ⟨P⟩.** Only the run's two outer boundaries:

| boundary | what can migrate | effect on `m9` |
|---|---|---|
| `b9` | work recorded inside the run that overlaps the tail of pre-run work (froxel dispatch, CSM/atlas/sky scopes) — charged to the unbracketed span before `b9` | deflates |
| `e9` | work recorded inside the run that would otherwise overlap post-run commands — `e9` waits for it | inflates |

Both are present with the same structure on B and C, so they are **paired** and cancel to first order in `Δ9`. What does not cancel is a count-dependent change in how much run work is available to overlap a boundary — second-order, unknown sign. The report carries this verbatim: *`NetRun` is a paired difference of two structurally identical intervals; its residual bias is second-order and unsigned; no directional confidence is claimed for either sign of the result.*

### B4. Query pool, per-FIF, readback, printed keys

- **Pool.** `create_query_pool(&QueryPoolDesc { count: 2 * VB_PASS_COUNT })` already derives from the const (`gpu_scene/mod.rs:4466`), so 3 → 10 propagates to creation, reset (`gpu_timing.rs:294`) and readback from one number. 20 × 8 B × 2 FIF = 320 B.
- **The assert the file lacks.** `debug_assert_eq!(pool.count, 2 * VB_PASS_COUNT)` in `reset_frame`. Today only `write` checks `index < pool.count` (`gpu_timing.rs:172`); a pool created at the old width would reset out of range before any write asserted.
- **Ring.** `pools[fi]`, `fi = self.frame_index` (`vb.rs:633`). Readback after `presented_ok` + `ctx.wait_idle()` on the same slot (`runner.rs:2605-2617`). Unchanged.
- **Readback shape.** `read_vb_bench_ns(&self, ctx, fi) -> Option<([f64; N], [f64; N])>` — `(begin_offset_ns, duration_ns)`, `N = VB_PASS_COUNT as usize`, stack arrays, no allocation. The runner's three `Vec<f64>` (`runner.rs:1028-1030`) become two `[Vec<f64>; N]` tables preallocated at `vb_bench_frames` (Principle 5).
- **Printed keys (F21) ⟨P⟩.** The `VB-P1d …` line stays **byte-identical** — same keys, same **mean** reduction, same NOTE — so `vg_occ_split_timing.rs:332`'s parse keeps working. The same `#[cold]` summary fn additionally emits:
  `VB-P4 pass=<label> median_ns=<..> mean_ns=<..> p95_ns=<..> begin_off_ns=<..> n=<..> [FALLBACK|TORN]`
  and one provenance line (condition 6):
  `VB-P4 regime observed=[none|keep|late,…] n_distinct=<k> mode=[off|two_phase,…]`
  computed host-side from the preallocated tables and the per-frame regime words. No per-frame I/O, no reallocation.

---

## Part C — the measurement protocol

### C1. Protocol

`crates/boyko_app/tests/vg_occ_split_timing.rs`, extended — same file, same four-leg interleave `A0 → B → C → A1` per round (`:45-54`).

| leg | configuration |
|---|---|
| `A0` | markers PRESENT, `OcclusionMode::Off`, `HzbConfig::Build` — the disarmed baseline |
| `B` | markers present, `TwoPhase` + `OcclusionForce::KeepAll` — every mechanism, no decision |
| `C` | markers present, `TwoPhase` + `OcclusionForce::None` — the real decision |
| `A1` | `Off` again, markers present — **the zero control** |

**A0/A1 are `Off` WITH markers ⟨P⟩,** not marker-absent. Today `Leg::marked()` (F25) disarms by withholding the component, moving two variables at once (the arming predicate *and* the spawn flush + `occlusion_instances` accounting). `Off`-with-markers is the one-variable contrast, and it is the only leg that exercises §A2's "do not test, never do not gather". `Leg::marked()` becomes `true` on all four legs and `Leg::Disarmed` gains `OcclusionConfig { mode: Off }`. **All four legs also insert `HzbConfig { mode: Build }`** — mirroring `[vb_occ_mixed_off]`, which carries `BOYKO_VG_HZB=1` — so the pyramid exists on every leg and slot 6 differs only in POSITION, never in existence. If slot 6 comes back FALLBACK on any leg, the run is rejected. ⚠️ **What this does not measure:** the marker's own gather cost; the difference between a marker-absent boot and `Off`-with-markers is on no leg of this protocol and is not claimed.

- **Channel G becomes real.** Per leg, per round, one worker with `BOYKO_VB_BENCH=1`; the per-pass **median over that worker's timed frames** (warm-up discarded by `VB_BENCH_WARMUP`), parsed from the `VB-P4 pass=…` lines. Any pass flagged `FALLBACK`/`TORN` is excluded from every aggregate and reported by name. A worker whose `VB-P4 regime` line reports `n_distinct > 1` is **rejected**, not averaged (condition 6).
- **Channel W is relabelled `KNOWN-BLIND`** and decides nothing. One claim: *did arming the instrument wreck the frame?* — threshold **>20 % frame-period inflation** of the armed leg over A0, ~1.4× its own worst measured floor (14.3 %). Below that, W reports "inside my own noise".
- **A second fixture, `vb_occ_dense`** — `vb_occ_mixed`'s geometry with the hidden set replicated `K` times behind the same slab (`K` from env, default 64), 512², **no golden pin** (`vb_occ_multi`'s precedent, `vb_occ_split_gate.rs:26-31`). The mixed fixture's 8 instances cannot produce a saving larger than the band.
  **Its correctness oracle.** Not the A/B hash identity (on a static converged scene `ARMED == FORCE_KEEP` pixels is D12 restated, and on a fixture with no pixel pin that is worth nothing). Instead the **same host-oracle verdict cross-check `vb_occ_mixed` already runs**: `boyko_render::hzb::occlusion_verdict` (imported at `vb_occ_mixed.rs:90`) computes the expected per-instance verdict on the host; the GPU readback's `Σ n_defer` and per-instance verdicts must match, scaling with `K`.
  **Trap folded:** `vb_occ_dense`'s setup asserts `resolved_render_path.mesh_geo_shade_split == false` before the bench loop, naming `runner.rs:1054-1059` — otherwise an SSAO-armed variant panics the channel-G worker with an assertion about VB-P1d's break-even, which reads like an instrument failure and is not one.
  **What it still cannot claim:** nothing pins its PIXELS. A defect producing the oracle's verdicts and the wrong image is invisible here. Pixel correctness stays `[vb_occ_mixed]`'s job, on 8 instances.

### C2. The quantities

**Convention: every quantity is a COST in nanoseconds. Positive = more time. Exactly one sign flip in the document, named `Saving`.**

Per round `r`, per pass `p`, per leg `L`: `m_p(L)` = the median of that worker's timed frames.

| symbol | definition | sign meaning |
|---|---|---|
| **`NetRun_r`** | **`m_9(C) − m_9(B)`** | **the headline. + ⇒ the decision costs more than it saves.** Migration-immune within slots 3–8 |
| `Saving_r` | `−Δ_5` | + ⇒ the decision **saves** early-raster time. Attribution-grade |
| `Overhead_r` | `Δ_3 + Δ_4 + Δ_7 + Δ_8` | attribution only — not a bound in either direction |
| `Net_r` | `Overhead_r + Δ_6 − Saving_r` | per-slot attribution sum over all six in-run slots |
| `GapResidual_r` | `NetRun_r − Net_r` (`= Δgaps`) | must be ≈ 0 — the gap commands are leg-independent |
| `HzbResidual_r` | `Δ_6` | must be ≈ 0 — same dispatch chain, same site on B and C |
| `Residual_r` | `Δ_0 + Δ_1 + Δ_2` | must be ≈ 0 — three passes carrying no split-dependent work |
| `PlumbRun_r` | `[m_9(B) − m_6(B)] − m_9(A0)` | + ⇒ arming the mechanism costs. Attribution-grade (slot 6 leaves the run when disarmed) |
| `Bracketed_r` | `Σ_{p∈0..8, p≠6} [ m_p(C) − m_p(A0) ]` + `m_9` reported separately | + ⇒ the bracketed ranges cost more armed. Slot 6 excluded: its site moves between these legs |
| `LateShare_r` | `Overhead_r / m_5(A0)` | the decision machinery's cost as a fraction of the un-split early raster it exists to shrink |

**`Bracketed` is NOT end-to-end, and after P4-6 this repository still has no end-to-end number.** Outside the brackets: everything before `CullReset`, the CSM cascade loop, the punctual atlas loop, the sky scope, the classify chain (`vb.rs:2571-2706`), `vb_viewt`/`vb_geo`/SSAO/à-trous under `path_vb_split()`, `sdf_forward_march`, the present blit, the whole non-`record_vb` frame. Channel W, the only end-to-end channel, is `KNOWN-BLIND`. The report's first paragraph says this.

**Bands — ⚠️ REPAIRED AT RUNG P4-7. The paragraph this replaces was a PLAN-LEVEL DEFECT, and the rule below is what shipped.**

This section used to define the band as the zero twin ALONE: *"the band comes from running the identical reduction on the zero control — substitute `A1` for the armed leg and `A0` for the baseline wherever `Q`'s definition names them — `band(Q) = max( |median_r Q⁰_r| , p90_r |Q⁰_r| )`."* **That is a DRIFT estimator and only a drift estimator**, and on this instrument drift is structurally zero: `A0` and `A1` are the SAME configuration — same scene, byte-identical environment, same command stream — measured on a fully serialized frame (`wait_idle` per bench frame on top of unconditional FIFO), so on a deterministic GPU the twin's expected value is **exactly zero**. P4-6's third sitting measured exactly that: the twin came back **exactly `0`** — on every one of the ten passes, in every worker process of that sitting — while `m_9` was a perfectly healthy 47 104 ns (`vb_occ_mixed`) / 691 200 ns (`vb_occ_dense`). With drift at zero the verdict rule collapses from *"|Q| clears the instrument's noise"* to **"Q ≠ 0"** — a false-win machine — and it duly reported `Saving` **RESOLVED** on eight low-poly instances at 512², which the harness's own contradiction notice caught.

**The zero control is not at fault and is not removed.** It did its job and reported honestly — there is no process-to-process drift on the GPU channel, while channel W's own legs wandered 6–9 % in the same sitting. What it structurally cannot supply is **RESOLUTION**, the other half of what a verdict needs. This section conflated the two.

**The shipped rule:** `band(Q) = max( FLOOR(Q), TWIN(Q) )`, both terms printed beside every quantity so a degenerate one is visible rather than silent.

- **`FLOOR`** — the reading's own resolution: the propagated standard error of **every median the reading is built from**, `SE ≈ 1.2533·σ̂/√n` with `σ̂ ≈ (p95 − median)/1.645`, taken from the `p95_ns`/`n` the runner already publishes, on the same legs the reading uses, in the same sitting. Summed with unit coefficients rather than in quadrature: the per-leg errors are not independent (one GPU, one sitting), and a band that is too wide refuses a real result while one that is too narrow manufactures one. **Sub-floored per median at the MEASURED lattice quantum**, because a pass reporting `p95 == median` yields `σ̂ = 0`, and a zero floor is the degenerate band the mechanism exists to prevent.
- **`TWIN`** — the original zero-twin term, `max( |median_r Q⁰_r| , p90_r |Q⁰_r| )`, unchanged. It stays: it is the only term that can see drift.

⚠️ **The lattice is MEASURED, never queried.** `VkPhysicalDeviceLimits::timestampPeriod` is the tick→ns SCALE (`1.0` on this vendor) and says nothing about how often the counter increments. The quantum is recovered per sitting as the GCD of every timestamp-derived value published (`vg_occ_split_timing.rs`'s `measured_quantum_ns`) and came back **32 ns**; flooring at `period × 1 tick` would have satisfied every assertion in the harness while under-stating the resolution by the whole lattice factor — the alarm silenced, the false win intact. ⚠️ The 32 needs an **ODD** timed-frame budget: `vb_bench_stats_ns` reduces an even `n` to the mean of the two middle samples, which can land a half-tick off the lattice, and a GCD taken under an even budget read **16 ns**. Hence `DEFAULT_BENCH_FRAMES = 221`. **Neither number belongs in code as a literal** — the harness re-measures it every sitting.

A sum's band is that sum's own band; no per-pass band is applied to an aggregate.

**Where a band would be vacuous, no band is claimed ⟨P⟩.** `A0` and `A1` are both disarmed, so slots 3, 7, 8 bracket empty blocks on both, and their zero-twin band is the lattice quantisation: testing "the late passes cost more than nothing" against it is unfalsifiable *and* trivially true. Those are reported as **magnitudes with a scale** (`LateShare`), never as significance verdicts. Significance bands are claimed for exactly six quantities whose **sign is the claim**: `NetRun`, `Saving`, `GapResidual`, `HzbResidual`, `Residual`, `Bracketed`.

### C3. Decision table

| reading | verdict | strength | consequence |
|---|---|---|---|
| `NetRun < −band(NetRun)` on both fixtures | the decision pays | as strong as its opposite — §B3 supports no asymmetry | publish; the field's doc recommends `TwoPhase` for occluder-dense scenes. The default still does not move |
| `NetRun < −band` but `Bracketed > +band` | the decision pays, the plumbing eats it | same | the target is `PlumbRun` (late upload / second scope) — an R4+ rung, not a default change |
| `\|NetRun\| ≤ band` and `\|Bracketed\| ≤ band` | **NOT RESOLVED** | — | a result: the instrument resolves per-pass costs, the fixtures do not separate the arms |
| `NetRun > +band` and `\|Saving\| ≤ band` | "the split costs more than it saves" | same | the campaign's recorded finding stands ("the bottleneck is GRANULARITY, not the test"). Default `Off` becomes a recommendation; next investment is the meshlet rung |
| `LateShare > 1.0` at `K = 64` | the machinery costs more than the raster it shrinks | descriptive, no band | same, with a scale attached |
| `Δ_4 > band` | the occlusion **leaf** is expensive (one lane per batch, serial inner loop) | — | names the first thing to change if the split is pursued |
| `\|GapResidual\| > band` | **the partition identity does not hold as derived** | — | either an unaccounted gap command is leg-dependent or a stamp is not where the plan thinks. Per-slot numbers become attribution-only; `NetRun` survives (measured directly) |
| `\|HzbResidual\| > band` | the pyramid build is leg-dependent, or migration across slot 6 is large | — | `Net`'s decomposition is suspect; `NetRun` survives |
| `\|Residual\| > band` | **the instrument is contaminated** | — | every number is suspect. First suspect: leg-dependent neighbourhood of `VbShade`. Reported, not worked around |

**What would justify default-ON ⟨P⟩:** `NetRun < −band` across ≥3 sittings on ≥2 fixtures of different occlusion density, **and** a second consumer for the pyramid so `HzbBuild` is not charged to this feature alone. P4-6 cannot meet that (one campaign, two fixtures, one machine, no second consumer), so the default stays `Off` and the document says why, rather than implying the measurement was inconclusive.

**Prediction, written before the run ⟨P⟩.** On `vb_occ_mixed`: `Saving` will not clear its band (8 low-poly instances at 512²; the early raster is dominated by scope fixed cost) while `Overhead` will be a measurable magnitude. On `vb_occ_dense` at `K = 64`: `Saving` should clear its band if the instrument resolves at all. A run contradicting either half is a finding about the instrument and is reported as one.

### C4. What the harness asserts

Instrument-level only (P3-8's rule), plus non-vacuity clauses for the instrument itself:

**Asserted (BOTTOM-vs-BOTTOM only — §B3's stage rule):**

- every leg produced `rounds` samples on every pass, and no pass was flagged `FALLBACK` or `TORN` on any leg;
- every worker's `VB-P4 regime` line reports `n_distinct == 1` and the expected regime for its leg;
- **`begin_offset_ns` is monotone across the leg-independent run** `b9, b3, b4, b5, b7, b8` and `off(9b) < off(3b)`, `off(8b)+dur8 ≤ off(9b)+dur9` — checked on all four legs (§B2's bold span);
- **slot 6's placement per leg**: armed ⇒ `off(5b)+dur5 ≤ off(6b) ≤ off(7b)`; disarmed ⇒ `off(6b) > off(9b)+dur9`. All BOTTOM stamps; this half carries the whole "slot 6 left the run" claim;
- every `begin_offset_ns < 1e9` — the base-stamp contract of §B1b (a broken contract shows as ~2^36 ticks, not as a plausible number);
- every published quantity's **band** is nonzero (⚠️ REPAIRED AT P4-7: this clause used to read *"the zero control is not exactly `0.00 %` on any quantity"*, which the shipped harness has never asserted and which would red a healthy run — a zero TWIN is the EXPECTED reading here, per the band repair in §C2. A zero BAND means the resolution floor also came out zero, i.e. the sitting published no scale at all);
- `m_8(A0) < m_8(B)` and `m_7(A0) < m_7(B)` — a zero-width bracket must read smaller than one containing real work. If this fails, the timestamp channel does not resolve at this magnitude and **every number in the report is noise**; the message says exactly that;
- `|Residual| ≤ band(Residual)`, `|GapResidual| ≤ band(GapResidual)`, `|HzbResidual| ≤ band(HzbResidual)`.

**Reported, never asserted (TOP-vs-BOTTOM — condition 2):**

- `off(2b)` versus `off(9b)+dur9`, and on the disarmed legs `off(6b)` versus `off(2b)`. Slot 2's begin is `TOP_OF_PIPE` by compatibility decision, so a later-recorded stamp may legally report an earlier time; these two are printed with the label `OBSERVATION (TOP vs BOTTOM — not ordered by record position)` and decide nothing.

No threshold on any performance property is asserted. The numbers are published as prose in the P4-6 commit message.

### C5. Scope limits of every published number

- Measured on a **fully serialized** frame: `ctx.wait_idle()` on every bench frame (F22) on top of unconditional FIFO. Correct for timestamp deltas; **not** the frame the shipped renderer executes. A small `Bracketed` does not imply a short critical path.
- One machine, one driver, one sitting, two fixtures, static scenes.
- `Saving` under **motion** is not measured and cannot be: the pyramid is a fixed point on a static scene (D12). The early phase's hit rate under motion is piece 3's OQ 3 and stays open.
- The marker's own gather cost is on no leg (§C1).
- The `vb_cull_readback` probe's cost is on no leg (§B2, the boot-time exclusivity).
- The dual-read equality invariant does not execute in a release bench run (§B1b).

---

## Rung ladder

All 30 pins stay byte-identical at every rung. **The central vacuity hazard (F15/F16):** `[vb_occ_split]` renders the same literal as three pins that never armed anything; the four `[vb_occ_mixed*]` render one literal including `[vb_occ_mixed_off]`, which never splits; and all five run **one fixture** (`vb_mesh` / `vb_mesh_screenshot_dump`). One edit can silently disarm all five while every hash — *and* `the_pins_declared_byte_identical_actually_agree`, which compares pins to each other — stays green. Named per rung, demonstrated **and covered** in P4-4.

### P4-1 — the readback seam, the disarm, and the totality epilogue, before the payload

**Moves:**
- `boyko_rhi`: `RhiDevice::read_query_pool_pairs_ns` (default `#[cold] #[inline(never)]` → `Unsupported`, §B1b), plus the first `Unsupported`-default unit test in `handle.rs`'s `#[cfg(test)] MockDevice` module (F35);
- `boyko_rhi_vulkan`: `fetch_query_raw_ticks` extracted verbatim from `fetch_query_pair_ticks` (`rhi_impl/device.rs:1197-1224`), `fetch_query_pair_stamps` added, `read_query_pool_pairs_ns` override;
- `gpu_timing.rs`: `VbTimedPass::begin_stage()/label()/from_slot()` (three current members → `TopOfPipe`), `write_begin` consults `begin_stage()` (F34), `write_zero_pair` (BOTTOM/BOTTOM), `debug_assert_eq!(pool.count, 2 * VB_PASS_COUNT)` in `reset_frame`;
- `vb.rs`: `TsWitness` (release-live masks + the dev-profile counter), constructed after `reset_frame` (`:640`), consumed by `finish` before `:4451`; the three existing bracket sites routed through it;
- `gpu_scene/mod.rs`: `disarm_vb_bench_unless_vb` + `vb_timing_for_frame` (§B0b); `read_vb_bench_ns` returns `([f64; N], [f64; N])` and carries the dev-profile dual-read invariant;
- `runner.rs`: the disarm call at `:561` + its `#[cold]` panic; the two preallocated tables; the summary's `begin_off_ns` + `FALLBACK`/`TORN` labels; **the `assert!` at `:1038-1042` replaced by a `#[cold]` printed scope note**; **the stale comment block `:1043-1053` repaired**.
- `VB_PASS_COUNT` unchanged at 3.

**Must not move:** all 30 pins — the witness is `None`-armed on every pin run (F7/F8) and the disarm is a boot-time bool; the masks are host-local. Barrier replica untouched (no declared access changes). The `VB-P1d …` print stays byte-identical, and the dual-read invariant proves the underlying numbers are bit-identical, not merely similarly formatted.

**Gates:**
- `cargo test -p boyko-rhi --lib`, `-p boyko-rhi-vulkan --lib`, `-p boyko-app --lib`, clippy;
- `[vb_mesh]` + `[vb_occ_mixed]` re-verified;
- **the dual-read invariant** exercised by any dev-profile `BOYKO_VB_BENCH=1` run (§B1b);
- one `BOYKO_VB_BENCH=1` run on the VB-P1d scene printing its **byte-identical** `VB-P1d …` line;
- **headline gate A, against a reachable state:** `BOYKO_VB_BENCH=1` on a `VisibilityBuffer × Sdf` boot. **Before this rung it PANICS** at `runner.rs:1039`; **after it, it completes and prints three finite numbers**, `VbShade` flagged `FALLBACK` with `begin_off_ns` at the frame end;
- **headline gate B (condition 4):** `BOYKO_VB_BENCH=1` on a `Deferred × Both` boot. **Before this rung it HANGS** (both asserts pass, the pool is never reset, the WAIT_BIT readback never returns — F37/F38); **after it, it exits immediately** with the `#[cold]` message naming the knob and the resolved path.

**Controls (RED), all executable:**
- (i) delete the epilogue, keep the double-write counter and add a totality `debug_assert!` → the VB×Sdf run reds in the dev profile naming slot 2.
- (ii) narrow the epilogue's loop to `0..2` → the VB×Sdf run hangs in **release** and reds in **dev** — the demonstration of why the epilogue is release-live and a `debug_assert!` cannot replace it (F24).
- (iii) implement the fallback with `write_begin`+`write_end` instead of `write_zero_pair` → the run reports a large `VbShade` number instead of ~0; the `FALLBACK` flag still fires and `fallback ⇒ duration_ns < 1_000` reds.
- (iv) delete `write_end(VbShade)` at `vb.rs:2938` → on the mesh leg the pair becomes **TORN** (begin present, end filled at the epilogue), the run is rejected by name — the control that proves the two masks are not one mask.
- (v) make `fetch_query_pair_stamps` use `base = 0` instead of pair 0's begin → the dual-read invariant still passes (durations unaffected) but the `< 1e9` offset clause reds, showing the base rule is tested independently of the deltas.
- (vi) **delete only the panic** from §B0b, keeping the disarm → headline gate B still exits (the run completes without a bench, printing the decline notice) instead of hanging: the executable demonstration of which half is load-bearing.

**Cannot claim:** that any bracket measures the right extent; that offsets are meaningful for pairs the recorder writes out of order (that is C4's job); **anything on a golden frame** — no pin arms the bench (F8), so the epilogue executes on **zero** golden frames; that the dual-read invariant holds in a release run (§B1b).

**What P4-1 buys over P4-2's gates ⟨P⟩.** (1) Two **release-live** mechanisms shipped alone, each shown to change a reachable configuration's outcome; control (ii) is the only place in the ladder where "release hangs, dev reds" is demonstrated on the *same* configuration. (2) It makes P4-2's seven `mesh_leg`-scoped writers safe *before* they exist: a misplaced bracket then produces a named `FALLBACK` line, not a hang and not a mystery. (3) It removes a release-live `assert!` from the runner and adds a new RHI verb — behaviour-bearing changes that must bisect separately from seven brackets, a stage split and a new statistics path.

### P4-2 — the seven brackets

**Moves:** `VbTimedPass` 3 → 10, `VB_PASS_COUNT` 3 → 10, the seven bracket pairs at B2's sites (six units + `VbRun`), `record_hzb_poison_build`'s `ts: &mut TsWitness` parameter and its two call sites (`:2034`, `:3069`), `begin_stage()` → `BottomOfPipe` for the new seven, the two `[Vec<f64>; 10]` tables, the `VB-P4 pass=…` lines beside the byte-identical `VB-P1d …` line, the boot-time bench↔readback exclusivity panic; **`crates/boyko_app/tests/vb_bench_query_validation.rs`** (new).

**Must not move:** all 30 pins. The argument is **structural** (F7/F8: witness `None` ⇒ zero recorded commands; no pin sets `BOYKO_VB_BENCH`), not empirical. ⚠️ The barrier replica is blind to timestamps **by construction** (F12) — it is therefore **not** evidence of inertness here, and the rung says so instead of banking it. The risk lives in (a) the query-pool reset staying outside every rendering scope, (b) the pool width, (c) per-FIF plumbing — each with a control below.

**Gates:**
- **`vb_bench_query_validation.rs`, a THREE-outcome gate ⟨P⟩.** Two workers on `vb_occ_mixed` with validation **ON** (no `BOYKO_DISABLE_VALIDATION`) — one with `BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_FRAMES=8`, one without:
  - **GREEN** — both completed, the bench-armed one printed ≥1 `VB-P4 pass=` line (**the non-vacuity clause**: without it, two workers that both recorded nothing agree trivially), and the normalized message sets are equal.
  - **RED** — both completed and the message sets differ. The only failure this gate claims.
  - **INSTRUMENT-DEAD** — *neither* completed. The standing environment note is "validation layer crash-prone"; a layer that takes both workers down is not a finding about piece 4. Printed loudly, not asserted.
  - **INCONCLUSIVE** — exactly one completed. Printed, **not green** — a real bench-only defect would take this shape too, so it escalates to the operator rather than being classified.
  The bench-armed run is the only configuration in the tree that executes `vkCmdResetQueryPool`/`vkCmdWriteTimestamp` on the VB path.
- One bench run on `vb_occ_mixed` in each of the three regimes printing 10 finite `(begin_off, duration)` pairs with the per-leg order of §B2 holding.
- P4-1's epilogue + double-write counter, now load-bearing on seven new slots and across one function boundary.

**Controls (RED):** (i) move `reset_frame` inside the rendering scope → `VUID-vkCmdResetQueryPool-renderpass` in the armed worker and the message-set comparison reds; (ii) place `VbLateRaster`'s bracket *inside* `if occlusion_split` → the disarmed leg's pair becomes FALLBACK and the "no leg is FALLBACK" clause reds with the slot named — where before P4-1 the same edit hung; (iii) size the pool at `2*3` while `VB_PASS_COUNT == 10` → the `reset_frame` assert reds, and with asserts off the validation worker reports an out-of-range reset; (iv) swap `VbRun`'s begin to `TOP_OF_PIPE` → `GapResidual` blows past its band in P4-6 (recorded here as the reason the stage is table-driven and unit-pinned); (v) set both `BOYKO_VB_BENCH` and `BOYKO_VB_CULL_READBACK` → the boot panic fires by name; (vi) set slot 6's bits at the *call sites* instead of threading the witness, then delete the begin stamp inside `record_hzb_poison_build` → the masks read complete while one query is unwritten, and the armed bench run hangs — the executable reason the witness crosses the boundary.

**Cannot claim:** that per-slot numbers are exclusive costs (§B3: within-run migration is zero-sum but redistributes freely); that `m_6` is comparable across an armed/disarmed pair (its call site moves); anything about **barriers** — sync-validation is measured dead on this machine (`PINS.toml:384-388`), so the validation leg sees static legality only; anything about the probe's cost (it cannot co-exist with the bench any more).

### P4-3 — the record-order witness for `VbCullUniform` (OQ 9, the closable half)

**Moves:** `let mut cull_uniform_filled = false;` in `record_vb`, set to `true` immediately after the `cmd_fill_buffer`/`cmd_update_buffer` pair at `vb.rs:1613`, and read by `debug_assert!(cull_uniform_filled, …)` immediately before `record_vb_pass(cull_pass)` at `:1621`. Both sites are inside the same `batch_cull_armed` block (F33), so there is no leg on which the read runs without the write's block. The read is syntactically present in **both** profiles (`debug_assert!` expands to `if cfg!(debug_assertions) { … }`, which liveness analysis sees), so `unused_assignments` cannot fire under `-D warnings`.
**Must not move:** all 30 pins (host-local bool).
**Gates:** all VB pins in the dev profile — **this** witness genuinely rides every golden run (F20: `graph_bridge.rs:3773`).
**Control (RED):** move the `cmd_fill_buffer`/`cmd_update_buffer` pair below `record_vb_pass(cull_pass)` — piece 3's F-M4b defect, which had **no executable red anywhere on this machine** (sync-validation dead, the derived stream identical under the defect, no image gate). `[vb_mesh]` now reds in the dev profile.
**Cannot claim:** anything about the *declaration* half — that the graph derives the right `TRANSFER → COMPUTE` edge, or that any other pass's transfer work is ordered. One pass's local order witness in one function. The declaration half stays open (OQ 9, narrowed), and the site comment says so rather than a plan.

### P4-4 — the config, and the site that owns the arming

**The open question, answered: no.** `vb_occ_mixed_scene/mod.rs` keeps the geometry and does **not** take the insert, because two of the consumers do not line up with it (F29): `vb_occ_split_gate.rs` arms occlusion on the FIVE-SPHERE scene and does not declare `mod vb_occ_mixed_scene;` at all, while `vg_occ_verdict_census.rs` declares the scene and boots no `App`. Making one module own both axes would force every consumer of one to compile the other, and would leave `vb_occ_split_gate.rs` — the binary that owns G2 and, now, the disjunct's only executable red — outside the single-edit reach, which is precisely the property the control needs.

**Instead: a new shared module `crates/boyko_app/tests/occ_fixture/mod.rs`,** declared by the five App-booting fixtures (`vb_mesh.rs`, `vb_occ_mixed.rs`, `vb_occ_split_gate.rs`, `hzb_engine_pyramid_gate.rs`, `vg_occ_split_timing.rs`), owning **decode and insert**:

```rust
// crates/boyko_app/tests/occ_fixture/mod.rs   (new)
/// Decodes `BOYKO_VG_OCC` / `BOYKO_VG_OCC_FORCE` into the two Resources. PANICS on an unknown
/// regime — the panic the shipping decode (`gpu_scene/mod.rs:4297`) used to own, moved with it.
pub fn occlusion_from_env() -> (Option<OcclusionConfig>, OcclusionForce);

/// THE single insert site for the occlusion axis across every fixture that boots an `App`.
/// One edit here reaches the pinned binary AND every gate binary — which is what makes the
/// vacuity control's "one edit, five green pins, gates red" a true sentence rather than a hope.
pub fn arm_occlusion_with(app: &mut App, mode: OcclusionMode, force: OcclusionForce);

/// The env-driven entry point: `occlusion_from_env` then `arm_occlusion_with`. Gates that need a
/// fixed configuration call `arm_occlusion_with` directly — both routes pass through ONE insert.
pub fn arm_occlusion(app: &mut App);
```

`vb_mesh.rs:314`'s `|| occ_marked()` HzbConfig workaround is deleted in this commit (A3's host disjunct replaces it).

**Moves:** `occlusion_config.rs` + `occlusion_plugin.rs` (new, `boyko_render`), `lib.rs` exports, `plugins.rs:267` registration; `occlusion_force.rs` (new, `boyko_app`); `VbOcclusionArm` + `GBufferScene::vb_occlusion` + the conjunct at `scene_types.rs:3943`; `occlusion_arm.rs` (new, `boyko_app`, the `hzb_plan.rs` shape with its own unit tests); `hzb_plan_for`'s second parameter and disjunct; the runner's two reads + threading; **deletion** of `gpu_scene/mod.rs:4297-4306` and `:1495-1502`; the fold at `:6988`; the regime provenance of A5 (`VbRecordProbe::occ_flags`, `VbProbeContext::occ_mode/occ_force`, `schema_version = 3`, the `VB-P4 regime` line); `occ_fixture/` and its five call sites; four `GBufferScene` literals in `window_present_gbuffer.rs` gain `vb_occlusion: None`; **the four doc repairs of A3**.

**Must not move:** all 30 pins **and** the five occlusion pins must still SPLIT — which no hash can show, and which this rung now covers directly:

**Gates:**
- **NEW — `vb_mesh_occ_pins_actually_split` (in the `vb_mesh` binary, the binary the pins render through).** Five `#[ignore]`d workers, one per occlusion pin, each re-execing `current_exe()` on `vb_mesh_screenshot_dump` with **that pin's `[*.env]` block verbatim**, plus `BOYKO_VB_PROBE=<tmp>` and a redirected `BOYKO_HOST_DUMP`. Asserted from the probe file (F30): `scopes == 2` for `[vb_occ_split]`, `[vb_occ_mixed_keep]`, `[vb_occ_mixed]`, `[vb_occ_mixed_late]`; **`scopes == 1` for `[vb_occ_mixed_off]`** — a required negative inside the same gate, so the gate is non-vacuous in both directions — `late_draws == draw_batches` on the four split legs and `== 0` on `off`; and `[probe] occ_regime` equal to `[host] occ_force` and to the pin's env value on every leg (the provenance cross-check of A5).
  **What it cannot claim:** it runs PROBE-ON while the pins are PROBE-OFF. That gap is small and named: `vb_probe_dump` is a host-side counter sink that records no commands and does not enter `path_vb_occlusion_split()` (F30). It also cannot distinguish the two FORCE regimes by *effect* — that stays G-P3-B/C's job, which reads GPU counters.
- **NEW — `vb_occ_probe_dump_marked_no_hzb`** in `vb_occ_split_gate.rs` (A3, condition 1): the marked single-batch scene with occlusion armed to `TwoPhase` and **no `HzbConfig` inserted**, asserted `scopes == 2`. Non-pinned; the disjunct's only executable red.
- `vb_occ_split_gate.rs` G2: `scopes == 2`, `late_draws == batch_count`, `late_cull_dispatches == 1`;
- `vb_occ_mixed.rs` G-P3-A/B/C: `Σ n_defer == 4` armed, `== 0` under `KeepAll`, `== 6` with `0 < Σ n_keep < Σ n_defer` under `DeferAll` — the only legs that prove the env → Resource translation of the two regimes happened;
- `hzb_engine_pyramid_gate.rs` G-P3-E (FORCE-LATE); `hzb_verdict_oracle_gate.rs` unchanged and green;
- `vg_density_census.rs`'s `the_pins_declared_byte_identical_actually_agree` (F17) — no pin literal moves, so it must stay green;
- new unit tests: `occlusion_alone_plans_a_pyramid` (`hzb_plan_for(Some(Off), Some(TwoPhase), …).is_some()`); `hzb_plan_for(Some(Build), Some(Off), …).is_some()`; `hzb_plan_for(None, None, …).is_none()`; `occlusion_arm_for` over `{Off, TwoPhase} × {None, KeepAll, DeferAll}`; `OcclusionForce::flags()` disjoint-single-bit const-asserts and `as_str()` round-trip against the env decode; `OcclusionConfig::default()` is `Off` from both routes (`hzb_config.rs:137-154`'s shape).

**Controls (RED), all executed and published:**
- **(i) the vacuity demonstration, with a gate that executes.** Delete the `OcclusionConfig` insert from `occ_fixture::arm_occlusion_with` — **one edit**. **All five occlusion pins stay GREEN** (F15/F16), **and so does the cross-pin equality guard**, while `vb_mesh_occ_pins_actually_split` (four legs), G2, `vb_occ_probe_dump_marked_no_hzb` and G-P3-B all red. Verbatim in the commit message, with the pin hashes shown unchanged beside the red gate output.
- **(ii) the disjunct (condition 1).** Delete `hzb_plan_for`'s disjunct → `occlusion_alone_plans_a_pyramid` reds **and `vb_occ_probe_dump_marked_no_hzb` reds on the GPU**, while **every pin and `vb_mesh_occ_pins_actually_split` stay GREEN** — because all five occlusion pins set `BOYKO_VG_HZB="1"` (`PINS.toml:411/474/499/528/556`) and receive the pyramid by the producer route regardless. The green half is published beside the red half: it is the measured statement of what the pinned corpus cannot see.
- **(iii)** map `DeferAll` to `VB_CULL_OCC_FORCE_KEEP` in `flags()` → G-P3-C reds on `Σ n_defer == 6` (the regimes are distinguishable, not merely present).
- **(iv)** stamp `occ_flags` from the Resource instead of from the pushed word → the `[probe]`/`[host]` provenance cross-check still passes, but flipping `OcclusionForce` after the fold (a deliberate one-line test edit) leaves the artifact reporting the Resource's regime while the GPU ran the other one; the recorder-sourced version reds. The control that proves the stamp is recorder-sourced.

**Cannot claim:** that `Off` un-allocates anything (A5); that the marker's semantics changed (they did not); that any device-side code moved (`vb_batch_cull.comp.spv` byte-unchanged, re-verified by the `*_spv_sync` tests).

### P4-5 — the PROBE-ON barrier delta (disposition (c3), IN)

**Moves:** `VbRow` gains a `probe: bool` field (`vb_barrier_stream_baseline.rs:310`); one test fn declaring the S1 and S3 rows with `scene.vb_cull_readback = Some(...)`, compared against their PROBE-OFF twins via the **`RowStream` owned-stream pattern the file already uses** (`:5871-5875`), asserting the **set difference**: the added `TRANSFER_READ` accesses, and the re-sourcing of `vb_raster_late`'s indirect fetch from `COMPUTE(SHADER_WRITE) → INDIRECT_COMMAND_READ` into `COMPUTE → TRANSFER` + `TRANSFER → INDIRECT_COMMAND_READ`. **Derived, never regenerated** — `dump_vb_split_barrier_streams` (`:1647`) prints streams as Rust source, which makes the replica agree with production by construction.
**Cost.** `assert_row_is_pinned` (**`:4350`**, whose RE-PINNED doc is `:4334-4349`; the "no longer stops at the count" phrasing quoted here is the module-doc bullet at `:120-122`) is **not** called on the probe rows; the delta test asserts a set difference against a small named expectation. One `VbRow` field + one test fn + one expectation, not four whole-stream arrays. The `probe` field also forces the eight existing rows to name their value, which is the file's own single-value discipline.
**Control (RED):** drop the `vb_late_visible` `TRANSFER_READ` from the probe branch → the delta set reds.
**Cannot claim:** anything about production. This is a replica of the declarator (`:110-118`), and the campaign has measured that a replica cannot see a missing barrier in the real recorder. It covers one class: a future edit that changes the probe branch's declared accesses without noticing.

### P4-6 — the measurement

**Moves:** `vg_occ_split_timing.rs` (channel G over 10 passes, §C2's statistics with zero-twin bands, A0/A1 as `Off`-with-markers + explicit `HzbConfig::Build`, `KNOWN-BLIND` channel W with its 20 % wreck threshold, the non-vacuity clauses, the per-leg order rules, the TOP-vs-BOTTOM observations, the `n_distinct == 1` regime clause, the `Residual`/`GapResidual`/`HzbResidual` checks); `vb_occ_dense` (new fixture module, no pin, host-oracle verdict cross-check + the `!mesh_geo_shade_split` setup assertion).
**Must not move:** nothing shipping. Test-only.
**Gates:** the run itself; the report; the host-oracle verdict agreement on `vb_occ_dense` at `K ∈ {1, 8, 64}`.
**Controls (RED):** (i) run with `VbLateRaster`'s bracket collapsed to zero width → the non-vacuity clause (`m_8(A0) < m_8(B)`) fires; (ii) corrupt one instance's bound on `vb_occ_dense` → the host-oracle cross-check reds (proving the dense fixture has a real oracle, not a restated theorem); (iii) shift `VbRun`'s end past the classify chain → `GapResidual` clears its band and the identity check reds, demonstrating the partition claim is tested and not assumed; (iv) swap slot 6's armed call site to the disarmed one → the per-leg placement rule reds by name (BOTTOM-vs-BOTTOM half), showing C4 detects a moved stamp rather than silently averaging it; (v) flip `OcclusionForce` mid-run in a scratch build → `n_distinct == 2` and the worker is rejected instead of reported as one regime.
**Cannot claim:** that any number generalizes beyond these two fixtures, this machine, this driver, this sitting; that `Saving` under motion is anything (C5); that `HzbBuild` is attributable to occlusion once a second consumer exists; that `vb_occ_dense`'s **pixels** are correct (no pin); that the marker's gather cost is measured; that the dual-read invariant ran (release profile, §B1b).
**VERIFY here, not earlier:** whether `vg_density_census_gate` wants a row for `vb_occ_dense`. It has no pin; `vg_density_census.rs:75-77` answers the analogous question for `vb_occ_split` — read it before authoring; do not assume the answer transfers.

### P4-7 — the doc repairs and the dispositions — **LANDED**

**Anchors, re-verified against the tree at authoring time rather than transcribed.** Of the SEVEN this rung was handed, **two were exact**; three had line-drifted, one pointed at the right line and made a wrong claim there, and one named a literal the tree derives. The count this section itself flagged was wrong in BOTH of its numbers:

| what this plan said | what the tree said at P4-7 |
|---|---|
| `vb_occ_split_gate.rs:52-53` | **`:60-61`** — drifted |
| `PINS.toml:377-378` | exact |
| `PINS.toml:481` | exact |
| `PINS.toml:410`, *"TWO of the 26 pins"* | line exact; the CLAIM is wrong twice — **SIX of 30** pins are HZB-armed (`[vb_mesh_hzb]`, `[vb_occ_split]` and the four `[vb_occ_mixed*]`), counted from the file and cross-checked by a strict-TOML parse that decodes 30 tables |
| §(c5)'s `vb_occ_split_gate.rs:632-641` for `late_draws == draw_batches` | that range is the `multi.draw_batches == MULTI_BATCHES` assert — a DIFFERENT clause. The per-batch clause is `:730-755` after this rung's own edit, so the repaired text names it by test function instead of by line |
| §(c5)'s `vb_occ_mixed.rs:485` for `draw_batches == BATCH_COUNT` | **`:505`** — drifted by 20 |
| §(c5)'s *"`Σ n_defer == 6` under FORCE-LATE"* | the tree asserts `total_defer == MARKED_TOTAL` (`vb_occ_mixed.rs:753`); `MARKED_TOTAL = BATCH_COUNT * MARKED_PER_MESH` and is 6 **derived**, never a literal. The repaired text cites the symbol, not the number |

**Moves, as landed:** the (c5) bullet in `vb_occ_split_gate.rs` and its two further copies — `PINS.toml`'s `[vb_occ_split]` block and, unbooked, `vb_occ_mixed.rs`'s G-P3-C doc + its `draw_batches` assert message; `PINS.toml`'s "named disarm route until piece 4", the `BOYKO_VG_OCC ⇒ HzbMode::Build` redundancy rationale (the fixture no longer supplies it — the host disjunct does) and the HZB-armed count; `occlusion_marker.rs`'s ladder paragraph and marker doc gain the config axis; §C2's band defect (below) and §C4's zero-control clause; `vg_occ_split_timing.rs`'s two `16 ns` lattice statements and its two "the quantum measured 0" conflations; `vb_occ_mixed.rs`'s PROBE-ON understatement; `docs/OPEN-QUESTIONS.md` (+ its Russian mirror) closes the 2026-08-07 piece-4 scope item and records the (c) dispositions; `docs/FEATURE_MAP.md` + `docs/SYSTEMS.md` gain the occlusion surface. The mdBook page is `doc-writer`'s.

**Gates:** `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings` (BOTH profiles), `cargo doc`, a strict-TOML parse of `PINS.toml`, the full 30-pin sweep once. The `PINS.toml` diff is **comments only** — machine-checked: every `+`/`-` line in it begins with `#`.

**Control:** N/A (documentation). The discipline instead: **every repaired sentence cites something that runs, or is deleted rather than rewritten** — 3 of this campaign's 5 doc-rot repairs introduced a new lie, and round-4's condition 2 was a fourth caught in review.

---

## Dispositions for every item in (c) — FINAL, closed at P4-7

Every item below carries its ending state: **DONE** (with the rung that did it), **OUT** (with the reason and where it is now tracked), or **BOOKED** (with the named owner). Nothing is left implied.

**(c1) The unconditional FIFO present mode — OUT. FINAL.** `present/swapchain.rs`'s `VK_PRESENT_MODE_FIFO_KHR`. **Channel W is superseded, not repaired**: the timestamp channel measures the split directly at CV 0.24–0.68 % where W's floor is 4.7–14.3 % and not constant, so removing the present limit would improve a channel that no longer decides anything — and P4-6 duly labelled W `KNOWN-BLIND` and gave it exactly one claim (*did arming the instrument wreck the frame?*). Separately, present mode is a **product-surface** decision (vsync, tearing, power) belonging to an owner-facing `PresentConfig`, not to an occlusion piece. **Now tracked in `docs/OPEN-QUESTIONS.md` as a VALUES item with that framing** (2026-08-07, "VG R3 piece 4 is complete"). No forward reference to any number P4-6 produced.

**(c2) D8 — `vb_indirect_late`'s provenance covered by nothing — OUT, unchanged. BOOKED to framegraph core.** Piece 3 retired the P2-8 first-touch guard's reach over that buffer by adding `vb_batch_cull`'s write and gained `vb_late_count` in exchange (net coverage unchanged, OQ 7; `graph_bridge.rs:3769-3775` states the mechanism — re-verified at P4-7 — in as many words: *"`vb_indirect_late`'s first touch is the upload's TRANSFER write, and a write is never tested"*). The fix is P2-7's framegraph-core change (`is_write || res_written || res_seeded` + a 14-site audit), whose only gate is a replica the campaign has **measured** blind to the class it would catch. **Piece 4 declared no new access and therefore neither improved nor worsened it** — stated explicitly so a reader does not infer coverage from the fact that piece 4 touched the recorder. P4-5 does not touch it either: it adds `probe: true` rows and one delta assertion, and asserts that the shipping `vb_indirect_late_upload → vb_cull_late → vb_raster_late` chain is field-identical with and without the probe. **Owner: framegraph core; carried as OQ 3 here and in `docs/OPEN-QUESTIONS.md`.**

**(c3) PROBE-ON barrier-stream rows — IN. DONE at P4-5**, as a derived **delta** rather than a second field-by-field matrix: `VbRow::probe`, two rows, one set-difference assertion, `assert_row_is_pinned` deliberately not called on them. Two findings the plan did not have, both recorded at the site rather than only in a report: (i) **the plan's prediction is refuted by the tree** — the probe does NOT re-source `vb_raster_late`'s indirect fetch, because `vb_cull_readback_late` is declared *after* the late raster, exactly as the declarator's own comment says; (ii) the perturbation is **larger** than either the plan or `vb_occ_mixed.rs`'s own disclaimer said — nine declared accesses over two passes on seven buffers, yielding eight barriers, with five pinned barriers moving (four re-sourcings plus a one-bit WIDENING of `vb_cull_late`'s self-WAR). The RED control was executed and was discriminating: exactly one test reds. **Its limit is unchanged and restated: this is a replica, and P2-0 measured that a replica cannot see a missing barrier in the real recorder.**

**(c4) The intra-pass `TRANSFER → COMPUTE` control on `VbCullUniform` — IN. DONE at P4-3, and only the closable half.** The record-order half is an executable **dev-profile** red on every VB golden — previously no red existed anywhere on this machine (sync-validation measured dead; a static fixture reading the previous frame's uniform is bit-identical). The shipped shape improves on the plan's: the witness IS the transfer block's value (`let cull_uniform_filled = unsafe { fill; update; true };`), so the plan's own control edit — moving the pair below `record_vb_pass` — cannot be performed at all and reds at COMPILE time in both profiles, where the plan's shape would have stayed green on exactly the defect it existed to catch. **The declaration half remains uncovered and stays OQ 9**: closing it needs a per-pass access accessor that does not exist (`FrameGraph::pass_access_count` is private), and P4-3's doc says so at the site rather than promising a plan.

**(c5) The stale future-tense header in `vb_occ_split_gate.rs` — IN. DONE at P4-7.** The stale text was *"`vb_occ_multi` has no golden, so **no golden covers a multi-BATCH late scope.** That is piece 3's first gate."* It sat in **four** places, not the two this plan named: `vb_occ_split_gate.rs`'s module doc, `PINS.toml`'s `[vb_occ_split]` block, `vb_occ_mixed.rs`'s G-P3-C doc (which additionally ended *"and which no golden covers"* and pointed at a drifted `vb_occ_split_gate.rs:43-44`), and `vb_occ_mixed.rs`'s `draw_batches` assert message. All four repaired to:

> The `vb_occ_mixed` family (two registered meshes, `BATCH_COUNT == 2`) is pinned in four regimes, so a multi-BATCH split now renders under a golden — and all four hashes are one literal (`85b7d378…4d2913d9`), including `[vb_occ_mixed_off]`, which never splits. That is the pins' claim and its limit: they prove a two-batch split reproduces the unsplit pixels; **no hash can show that the late scope drew.** The executed evidence is the `multi.late_draws == multi.draw_batches` clause in `vb_occ_split_records_two_scopes`, `draw_batches == BATCH_COUNT` (`vb_occ_mixed.rs:505`), `Σ n_defer == MARKED_TOTAL` under FORCE-LATE (G-P3-C, `vb_occ_mixed.rs:753`), and — since piece 4 — `vb_mesh_occ_pins_actually_split`, which adjudicates the pinned configurations inside the pinned binary.

**(c6) `goldens/PINS.toml`'s UTF-8 BOM — OUT, knowingly left. FINAL, with the reason MEASURED.** Commit `11a3e8c` fixed 24 invalid TOML escapes in that file and deliberately left the leading BOM, which strict TOML rejects; P4-7 was asked to decide, and the decision is to leave it and record why. **Stripping it does not stay stripped.** `scripts/golden.ps1`'s `-Bless` path writes the file back with `Set-Content -Encoding UTF8`, and the only PowerShell on this box is Windows PowerShell **5.1**, whose `-Encoding UTF8` is BOM-**ful**. Measured directly: a BOM-less file round-tripped through that exact call came back with `EF BB BF` prepended (and LF→CRLF, which the file already is). So a BOM removal would be silently undone by the next bless, and the honest fix is at the WRITER — `golden.ps1` must emit BOM-less UTF-8 (`[IO.File]::WriteAllText` with a `UTF8Encoding($false)`, or `-Encoding utf8NoBOM` once a PowerShell 6+ is present) **and** the file stripped in the same commit. That is a tooling change gated by a bless run, not a documentation rung. **Impact today: none** — `golden.ps1` parses with line regexes and tolerates the BOM, and every strict-TOML check in this campaign strips it explicitly. **BOOKED: the golden harness.**

---

## Integration

| file | change |
|---|---|
| `boyko_rhi/src/device.rs` | **new verb** `read_query_pool_pairs_ns` (default `#[cold] #[inline(never)]` → `Unsupported`), beside `:874-922` |
| `boyko_rhi/src/handle.rs` | `#[cfg(test)]` default-body test on `MockDevice` (~`:453`, `impl RhiDevice` `:476-514`) — the crate's first |
| `boyko_rhi_vulkan/src/rhi_impl/device.rs` | `fetch_query_raw_ticks` extracted verbatim from `:1197-1224`; `fetch_query_pair_ticks` keeps its compaction unchanged; `fetch_query_pair_stamps` added; `read_query_pool_pairs_ns` override |
| `boyko_render/src/occlusion_config.rs`, `occlusion_plugin.rs` | **new** |
| `boyko_render/src/lib.rs` | export `OcclusionConfig`, `OcclusionMode`, `OcclusionPlugin` |
| `boyko_render/src/hzb_config.rs` | two doc repairs (`:80-88`, `:42-51`) — P4-4; the lockstep citation names `debug_assert!` |
| `boyko_app/src/plugins.rs` | register `OcclusionPlugin` after `HzbPlugin` (`:267`) |
| `boyko_app/src/occlusion_force.rs`, `occlusion_arm.rs` | **new** — the diagnostic Resource; `occlusion_arm_for`, unit-tested without a GPU |
| `boyko_app/src/hzb_plan.rs` | second parameter + disjunct + **two** doc repairs (`:17`, `:26-29`) + tests |
| `boyko_app/src/runner.rs` | `disarm_vb_bench_unless_vb` + its `#[cold]` panic at `:561`; two `try_resource` reads at `:2323`; thread the arm into `scene()`; the two 10-pass tables at `:1028` / `:2605-2628`; `print_vb_bench_summary` gains the per-pass and `regime` lines (`VB-P1d` line unchanged); **`:1038-1042`'s `assert!` → `#[cold]` scope note**; **`:1043-1053` comment repaired** (refuted by `vb.rs:3573`/`:3732`); `:1054-1059`'s `assert!` **kept as written**; the bench↔readback exclusivity panic beside the `:1173-1177` precedent |
| `boyko_app/src/gpu_scene/mod.rs` | **delete** `:4297-4306` and `:1495-1502`; `disarm_vb_bench_unless_vb` + `vb_timing_for_frame`; `scene()` gains the arm param and routes the collector through the new accessor (`:6641` region); the fold at `:6988`; `read_vb_bench_ns` (`:7372`) returns two `[f64; 10]` + the dev-profile dual-read invariant; pool count from the const at `:4466` |
| `boyko_app/src/vb_probe_dump.rs` | `VbProbeContext::occ_mode`/`occ_force`; `[probe] occ_flags`/`occ_regime`; `schema_version = 3` (`:160`) with the F39 bump note |
| `boyko_rhi_vulkan/src/present/scene_types.rs` | `VbOcclusionArm`, `GBufferScene::vb_occlusion`, the conjunct in `path_vb_occlusion_split` (`:3943`) |
| `boyko_rhi_vulkan/src/present/gpu_timing.rs` | 7 new `VbTimedPass` members, `VB_PASS_COUNT` 10, `begin_stage()`/`label()`/`from_slot()`, `write_begin` consults the stage, `write_zero_pair`, `reset_frame` assert |
| `boyko_rhi_vulkan/src/present/passes/vb.rs` | `TsWitness` + 14 bracket sites routed through it; `finish` before `:4451`; `record_hzb_poison_build` (`:4500`) gains `ts: &mut TsWitness` (call sites `:2034`, `:3069`); the order witness (`:1613`/`:1621`); `VbRecordProbe::occ_flags` stamped at the push site; the slot-6 mutual-exclusion comment |
| `boyko_rhi_vulkan/src/present/mod.rs` | `VbRecordProbe` gains `occ_flags: u32` |
| `boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | 4 literals gain `vb_occlusion: None` |
| `boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | `VbRow::probe` + P4-5's delta test (`assert_row_is_pinned` at `:4350` is **not** invoked on the probe rows) |
| `boyko_app/tests/occ_fixture/mod.rs` | **new** — decode + THE single insert site (P4-4) |
| `boyko_app/tests/vb_bench_query_validation.rs` | **new**, P4-2 |
| `boyko_app/tests/vb_mesh.rs` | `mod occ_fixture;`, the `|| occ_marked()` HzbConfig workaround deleted, **`vb_mesh_occ_pins_actually_split` (new gate)** |
| `boyko_app/tests/vb_occ_split_gate.rs` | `mod occ_fixture;`; **`vb_occ_probe_dump_marked_no_hzb` (new non-pinned GPU leg)** + its driver assertion |
| `boyko_app/tests/{vb_occ_mixed, hzb_engine_pyramid_gate, vg_occ_split_timing}.rs` | `mod occ_fixture;` + `arm_occlusion*` replacing local inserts; `Leg::marked()` → all-true |
| `boyko_app/tests/vb_occ_dense/` | **new**, P4-6, no pin, host-oracle cross-check + `!mesh_geo_shade_split` setup assertion |
| `goldens/PINS.toml` | **comments only** — no key, no hash, no env value changes |

## Validation

**Unit (no GPU):** `OcclusionConfig::default() == Off` from both routes; `enabled()` pinned **against the `#[repr(u32)]` discriminant** (`hzb_config.rs:157-169`'s shape, not a restatement of `matches!`) plus a wildcard-free `match` in the test so a new variant fails to compile; `OcclusionForce::flags()` is 0 / single-bit / single-bit with the two bits disjoint (const-assert); `as_str()` ↔ env decode round-trip; `occlusion_arm_for` over the 2 × 3 product; `hzb_plan_for` — `(Some(Off), Some(TwoPhase))`, `(Some(Build), Some(Off))`, `(None, None)`, `None` at zero extent; `VbTimedPass::begin_stage()/slot()/label()/from_slot()` pinned for all ten, `label()` unique, `from_slot(p.slot()) == p`; `read_query_pool_pairs_ns`'s default returns `Unsupported` on `MockDevice`.

**Property:** `slot()` is a bijection onto `0..VB_PASS_COUNT`; `begin_stage()` is `TopOfPipe` exactly for slots < 3 (the compatibility contract as a test, not a comment).

**Integration (GPU):** the two headline gates (VB×Sdf completes with `FALLBACK`; Deferred×Both exits instead of hanging) + the dev-profile dual-read invariant at P4-1; `vb_bench_query_validation` + the three-regime 10-pass run at P4-2; the 30-pin sweep at P4-2, P4-4 and P4-7; `vb_mesh_occ_pins_actually_split` / `vb_occ_probe_dump_marked_no_hzb` / G2 / G-P3-A/B/C / G-P3-E / the verdict oracle / the cross-pin equality guard at P4-4.

**`debug_assert!` invariants added:** per-slot write count ≤ 1 on an armed frame; mask-complete **and no torn pair** at `TsWitness::finish` (P4-1); `pool.count == 2 * VB_PASS_COUNT` (P4-1); `pair_count > 0` and `scratch.len() >= 2 * pair_count` in the new seam (P4-1, mirroring `rhi_impl/device.rs:1188-1195`); the dual-read equality (P4-1); `cull_uniform_filled` order witness (P4-3); `VbOcclusionArm::force_flags` is 0 or exactly one bit (P4-4, at the fold); `occ_flags` stamped == the word pushed (P4-4, at the push site). **Not** added: `vb_occlusion.is_none() || hzb.is_some()` — false on a legitimate non-VB world.

**No benchmark is pinned and no threshold is asserted** (P3-8's rule). The numbers are prose in the P4-6 commit message.

## Open questions

1. **VALUES, owner.** The default is `Off` (A1). Flipping it to `TwoPhase` is one attribute, but a real behaviour change: with A3's disjunct, any world carrying an `OcclusionCulling` marker would then build a pyramid by default.
2. **Open, unchanged.** The moving-camera hit-rate question (current view-proj against a stale pyramid vs `prev_view_proj`). P4-6 measures static scenes; the pyramid is a fixed point there.
3. **Open, unchanged.** `vb_indirect_late` provenance ((c2)) and the declaration half of the `VbCullUniform` edge ((c4)).
4. **Open, narrowed.** Slot 6's call site sits on opposite sides of the shade producer (`vb.rs:2034` / `:3069`), so no armed-vs-disarmed aggregate can contain `m_6` honestly. Fixing it means moving a production call for a measurement's convenience; deferred. It does **not** affect `NetRun` (both its legs are armed).
5. **Open, new.** `runner.rs:1054-1059` (`!mesh_geo_shade_split`) remains a boot-time scope assert with the premise-shaped character P4-1 removed from `:1038`. P4-6 works around it with a fixture assertion; whether the split producer should be bench-measurable at all is a VB-P1d scope question, not piece 4's.
6. **Open, new.** The new pin-binary split gate runs PROBE-ON while the pins are PROBE-OFF. F30 makes the gap small (host counters, zero commands, no arming input), but it is a gap and is recorded as one.
7. **Open, new.** The dev-profile-only dual-read invariant does not execute in a release bench run (F24, §B1b). If a release-only divergence between the two readers is ever suspected, the check must be re-run in the dev profile on the same scene; nothing in the ladder can detect it in release.

---

## Final disposition table

| item | verdict | where answered |
|---|---|---|
| **Round-3 blocker 1** — no API path for `(begin_offset, duration)` | **CLOSED (verified by the round-4 critic)** | §B1b + F28; new `read_query_pool_pairs_ns`, `fetch_query_raw_ticks` extraction, dual-read equality |
| **Round-3 blocker 2** — the vacuity control reached no executing gate | **CLOSED (verified)** | §P4-4 + F29: `occ_fixture/` single insert + `vb_mesh_occ_pins_actually_split` with its required negative |
| **Round-3 blocker 3** — the headline contradicted its derivation | **CLOSED (verified)** | §B3: `NetRun := Δ9`; `Δ6 → HzbResidual`; `PlumbRun` attribution-grade |
| **Round-3 conditions 1–7** | **CLOSED (verified)** | per-leg record order; `debug_assert!` at `targets.rs:8632`; `graph_bridge.rs:3773`; inverted P4-3 polarity; two masks; the post-late probe block; the three `runner.rs` objects |
| **Condition 1** — P4-4 control (ii) cannot fire; contradicts A3's own byte-identity argument | **FOLDED — executable red added** | §A3 + §P4-4 control (ii): new non-pinned GPU leg `vb_occ_probe_dump_marked_no_hzb` in `vb_occ_split_gate.rs` (F40: that binary is pinned by nothing and already inserts `HZB_BUILD` on all three existing workers, so removing it on one new worker is a one-variable change). The claim is **not** withdrawn; the green half (all five pins + `vb_mesh_occ_pins_actually_split` stay green) is **published beside the red half** as the measured statement of what the pinned corpus cannot see |
| **Condition 2** — `off(2b) > e9` and `off(6b) > off(2b)` mix TOP and BOTTOM stages | **FOLDED — demoted** | §B3 gains the TOP-vs-BOTTOM paragraph (a TOP stamp waits only for prior commands to *reach* the pipe top, so a later-recorded TOP may legally report an earlier time); §C4 splits into **Asserted** (BOTTOM-vs-BOTTOM only) and **Reported observations**. The disarmed-leg slot-6 rule keeps `off(6b) > off(9b)+dur9`, which carries the entire "slot 6 left the run" claim |
| **Condition 3** — slot 6's witness crosses a function boundary | **FOLDED — `TsWitness`** | §B1: one carrier owns the collector, both masks and the dev counter; **every** stamp goes through it; `record_hzb_poison_build` (`vb.rs:4500`) takes `&mut TsWitness` at both call sites (`:2034`, `:3069`); `finish` consumes it (the `vb_probe_dump.rs:119-121` precedent). P4-2 control (vi) reds the alternative (bits set at the call site) with a hang |
| **Condition 4** — the sibling hang class the epilogue cannot reach | **FOLDED — boot-gated** | §B0b + F36/F37/F38: `disarm_vb_bench_unless_vb` at `runner.rs:561` — the earliest instant the path exists, since `boot` (`host.rs:246`) precedes `resolve_render_path` (`:554`). Disarm-then-panic, with the disarm named as the load-bearing half and P4-1 control (vi) proving it. **P4-1 headline gate B**: `Deferred × Both` + `BOYKO_VB_BENCH` **hangs before**, exits with a named message **after** |
| **Condition 5** — the dual-read gate has no site | **FOLDED — sited** | §B1b: a `#[cfg(debug_assertions)]` block **inside `read_vb_bench_ns`**, not an env knob. Composes with nothing (`BOYKO_VB_CULL_READBACK` is excluded from every bench boot by P4-2's panic). Its release-profile blindness is stated in §C5 and OQ 7 |
| **Condition 6** — the deleted boot-decode's provenance rationale | **FOLDED — recorded in the artifact** | §A5 + F39: recorder-stamped `VbRecordProbe::occ_flags` (from the **pushed word**, not the Resource), host-derived `VbProbeContext::occ_mode/occ_force`, `schema_version` 2 → 3, `VB-P4 regime observed=[…] n_distinct=<k>`. Constancy is **recorded, never asserted**; `n_distinct > 1` rejects a worker (P4-6 control (v)); P4-4 control (iv) proves the stamp is recorder-sourced. Within the pinned corpus constancy additionally holds by construction (one insert, no mutation) |
| **Condition 7** — two drifted anchors | **CORRECTED and re-verified** | **`vb.rs:2467`** is the late scope's `cmd_end_rendering`; `:2469-2471` is the probe increment; `:2472` closes the `if occlusion_split` block (slot 8's close); the post-late readback block is **`:2490-2550`** (F32). **`assert_row_is_pinned` is `vb_barrier_stream_baseline.rs:4350`**, its RE-PINNED doc `:4334-4349`, the quoted phrase the module-doc bullet `:120-122` (F12/P4-5) |
| **(c1)** unconditional FIFO | **OUT — FINAL** | superseded channel (W is `KNOWN-BLIND` after P4-6) + product-surface decision; now a VALUES item in `docs/OPEN-QUESTIONS.md` |
| **(c2)** D8 `vb_indirect_late` provenance | **OUT — BOOKED to framegraph core** | piece 4 declared no new access — neither improved nor worsened it; P4-5 asserts the shipping late chain is field-identical with and without the probe. OQ 3 |
| **(c3)** PROBE-ON barrier rows | **IN — DONE at P4-5** | derived delta, not a second matrix; the plan's re-sourcing prediction REFUTED by the tree and the refutation written into the file; the perturbation is nine accesses / two passes / seven buffers / five moved barriers |
| **(c4)** `VbCullUniform` `TRANSFER → COMPUTE` | **IN — DONE at P4-3, half** | record-order half is a COMPILE-time red in both profiles (the shipped shape beats the plan's, whose control edit would have stayed green); declaration half stays OQ 9 |
| **(c5)** stale `vb_occ_split_gate.rs` header | **IN — DONE at P4-7** | found in FOUR places, not two; all repaired, each citing something that runs |
| **(c6)** `PINS.toml`'s UTF-8 BOM | **OUT — knowingly left, reason MEASURED; BOOKED to the golden harness** | `golden.ps1 -Bless` writes via `Set-Content -Encoding UTF8` on Windows PowerShell 5.1, which re-adds the BOM — measured on this box. The fix belongs at the writer and is gated by a bless run |
| **Ladder** | **P4-1 → P4-7 — COMPLETE**, each landing green and committing alone | P4-1 seam+disarm+epilogue · P4-2 brackets · P4-3 order witness · P4-4 config · P4-5 probe delta · P4-6 measurement · P4-7 docs |
| **Blessing** | **No rung required re-blessing any pin.** All 30 hashes, all `[*.env]` values and all `[*.sha256*]` literals are unchanged at every rung; `PINS.toml` edits are comments only — machine-checked at P4-7 (every `+`/`-` line in its diff begins with `#`) | §Integration, last row |