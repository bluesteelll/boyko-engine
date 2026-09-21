# SDFDDGI — SDF-Native Dynamic Diffuse Global Illumination (build plan)

Converged design plan (branch `ecs`). Produced by an architect → 3 adversarial critics
(perf-cache · principle0-byteidentity · scope-foundation) → converge loop. This is the
build bible: every rung references it. Companion verdict doc:
[docs/RENDER-SHADOW-GI-PLAN.md](RENDER-SHADOW-GI-PLAN.md) (the GI technique matrix).

## What it is

An octahedral **irradiance-probe grid** (Hu et al. 2021, arXiv:2007.14394; structurally =
Majercik/RTXGI DDGI) whose probes are updated by **sphere-tracing the existing CSG edit-list
marcher** (no RT hardware). Direct light + SDF-shadow visibility + previous-frame probe
feedback (multi-bounce), Chebyshev two-moment leak suppression, temporal hysteresis. The
deferred resolve samples probe irradiance into the existing `ambient` accumulator. Fully
dynamic in light **and** geometry (the marcher reads the field per frame — no re-voxelize).

## Owner-locked VALUES (2026-07-04)

- **Grid:** `16×8×16 = 2048` probes, spacing `2.0` → a `32×16×32` unit box. ~2.6 MB VRAM.
  Grows by config, not code. Confirm the showcase AABB covers the playable area.
- **Update budget ceiling:** ~3 ms (2080Ti class). Cadence/rays/MAX_IT are **derived from
  the `ddgi_probe_update_cost` bench**, not asserted.
- **Diffuse-only** now. Keep R11G11B10F-no-gamma (bit-exact resolve). Specular = a later
  cone-trace, accepting an atlas-format revisit then.
- **DDGI, not Radiance Cascades** (RC is a later re-research if convergence/coverage limits).
- **Bit-exact resolve** via the gamma-drop (accept mild imperceptible banding) — the update
  pass stays GPU-only-golden + tolerance.

## The 3 P0s the critics caught (all verified against code, all folded)

1. **`set=1` does not exist.** The RHI is single-set (`rhi_impl.rs` `set_layout_count: 1`;
   zero `space1` shaders). → Raise `MAX_BIND_GROUP_BINDINGS` 16→19 (RHI pre-rung) +
   combined-image-collapse for the 2 probe textures (the proven `gCsm`/`gShadowAtlas`
   pattern). NOT multi-set plumbing.
2. **No spare LightBuf words.** `LightHeaderGpu` is a sealed const-asserted 64 B / 16-word
   struct; `GpuLight[]` begins at word 16. Appending grid words shifts every element offset →
   0%-gate broken at rest. → Grid params ride a **dedicated `ResolvedDdgi` UBO**, not folded.
3. **~3 ms budget asserted, not derived.** The code-grounded op count (probes × rays × up-to-128
   march steps × edit_count + per-hit `sdf_normal`×6 + `sdf_soft_shadow` full second march per
   light) is 1–3 orders higher on our CSG fold. → **Gate on the `ddgi_probe_update_cost` bench
   before locking grid/rays/cadence**; escape hatches (fewer rays, lower GI-MAX_IT, coarser
   grid, brick-cache-ON for GI rays) promoted to first-class bench-derived knobs.

## Key decisions

- **D1 World-fixed bounded volume** (not camera-centered cascades): one AABB, `origin +
  spacing + dims`. Camera-independent → the grid UBO needs **no per-FIF ring** and temporal
  feedback needs **no reprojection** (probe *i* is the same world point every frame → kills
  the "wrong-only-in-motion" race class). Outside the box → graceful fallback to the existing
  sky/hemisphere ambient.
- **D2 Persistent single atlas per moment** (NOT ping-pong — the critic-forced amendment):
  under round-robin, ping-pong would discard the 3/4 un-updated converged tiles every flip. A
  probe writes ONLY its own tile and never reads its own tile mid-update, so there is no
  intra-dispatch WAR on the written tile; a neighbor-read-during-write is a benign temporal
  lag (multi-bounce is lagged by design). A barrier separates the update dispatch from the
  resolve read. Irradiance R11G11B10F (8×8 tile: 6×6 valid + 1-texel border); depth RG16F
  (16×16 tile: 14×14 + border). ~2.6 MB total.
- **D3 Round-robin `1/N` (default 4) + transcendental-free classification** (inside-geometry
  probes → INACTIVE, skipped). Rays/probe, N, GI-MAX_IT **bench-derived** to the ~3 ms ceiling.
- **D4 Binding budget** (the central amendment): cap 16→19; the 2 probe textures land as
  combined-image collapses `gDdgiIrr`+`gDdgiIrrSamp` @ register 16 and `gDdgiDepth`+samp @ 17
  (the `gCsm t12+s12`/`gShadowAtlas t14+s14` precedent); the grid UBO at slot 18. GI-OFF: all
  3 are bound-but-unread dummies. The update compute pass has its OWN pipeline layout (no
  resolve-set pressure).
- **D5 ECS-native quartet** mirroring `ResolvedCsm` exactly: `DdgiConfig` (cold Resource,
  structural `enabled()`), `ResolvedDdgi` (`#[repr(C)]` const-asserted carrier,
  DISABLED==Default==all-zero, `ddgi_mode_word` from `enabled()`), `resolve_ddgi_grid`
  (single-writer, camera-independent → no refit), `sync_ddgi_light_gate` (sole writer of the
  word-7 bit-4 gate). `DdgiResolveSet` orders resolve-before-consumer.
- **D6 Drop the irradiance gamma** (the best part — all critics endorse): store R11G11B10F
  WITHOUT the pow encode/decode → the per-pixel resolve path is dot/max/sqrt/div/lerp-only →
  **host-oracle bit-exact** (the SSAO/HBAO-lite lesson). Ray-gen Fibonacci table is
  CPU-precomputed + boot-uploaded (per-frame decorrelation = a quaternion rotate, mul/add);
  the only remaining transcendental (`pow` depth-sharpen) is confined to the update pass
  (GPU-only golden + bounded tolerance).

## Principle-0 storage (three durable data classes, no std Vec/HashMap side store)

1. **Grid metadata** → `ResolvedDdgi` Resource-owned `#[repr(C)]` carrier (mirror `ResolvedCsm`).
2. **Probe irradiance + depth atlas** → RHI-owned device textures (the legitimate FFI/GPU
   contiguity exception, identical lifecycle to `gCsm`/`gShadowAtlas`).
3. **Per-probe classification/liveness** (active/inactive + converged-once bit) → a dedicated
   GPU classification buffer (1 byte/probe), a declared FFI-GPU exception like the atlas —
   NOT a host `std::Vec<bool>` (the SP4-race lesson). The converged-once bit gates the resolve
   and the feedback to treat unconverged probes as sky-ambient fallback until first write.

**Serialize seam (non-goal, documented):** the atlas + classification buffer are TRANSIENT GPU
cache (re-converge on load, like TAA history); `DdgiConfig` IS serialized; `frame_index` is
host-frame-derived, not world state.

## 0%-gate

GI-OFF is byte-identical (grand_showcase sha256 + offscreen goldens). Gate = LightBuf word-7
**bit 4** (`DDGI_MODE_BIT=4`; bits 2=CSM, 3=punctual taken, 4 free; default
`LightingConfig.ddgi_indirect=false` → word 7 unchanged). The 3 new resolve bindings are
bound-but-unread dummies when off (the punctual/CSM/SSAO dummy precedent). Grand golden hashes
the rendered IMAGE, not UBO bytes, so unread words/bindings are image-invisible.

## Host-oracle bit-exactness

Accepted primitive set (mirrored in `goldens.rs`): `{+ - * / abs min max clamp/saturate
floor sqrt/normalize select}`. (`floor` is a deterministic non-transcendental intrinsic;
GPU/CPU agree bit-for-bit ONLY when its input is `clamp`ed to `≥ 0` first — where
`floor == trunc` and HLSL `floor` matches — so the world→probe base-cell `floor` MUST stay
after the `[0, dims-1]` clamp.) **Resolve path (per-pixel) op set MUST stay bit-exact-capable**
— world→probe index, trilinear, wrap/backface weight `((dot+1)*0.5)²+0.2`, Chebyshev
`var/(var+max(0,d-μ)²)` are all in the set, and octahedral decode ends in `normalize` (`sqrt`,
in the set). The host `oct_decode` is a HAND-WRITTEN mirror (there is NO `oct_decode` eDSL
body — `oct.rs` authors only ENCODE); its bit-parity with the I3 HLSL decode is NOT yet
proven — it is certified at I3 by the GPU golden, exactly like the marcher/SSAO decode
oracles. **I0b deliverable (SHIPPED):** a host-Rust `probe_sample` reference (goldens.rs
mirror) proving the texel-index→UV→direction→weight chain is transcendental-free +
math-correct on the host BEFORE any GI logic ships (the encode reuse diverges from the eDSL
body by ≤2 ULP — `x*(1/s)` vs `x/s` — documented, not bit-asserted). **I3 will add**
`probe_sample_gpu_eq_cpu_to_bits` (dispatch the HLSL `probe_sample`, read the atlas back, diff
to the host reference to bits) — that GPU golden is where host↔GPU bit-exactness is actually
certified. If any transcendental had surfaced, the resolve golden would re-classify to
GPU-only+tolerance NOW; none did. **Update pass** (marches) is GPU-only-golden + tolerance
regardless.

## Increment ladder

- **I(-1)** — RHI cap-raise 16→19 (`rhi_impl.rs` const + boyko_rhi mirror + `targets.rs`
  assert). Own 0%-gate: byte-identical. Boot device-limit check. *(building)*
- **I0** — Gate bit + `Ddgi*` quartet + 3 dummy bound-but-unread resolve bindings (restore
  exact-fill 19/19) + gated (empty) resolve injection + **`probe_sample` host-mirror bit-exact
  proof**. THE 0%-gate certificate rung.
- **I1** — Atlas + classification buffer allocation, **boot-clear** (fixes uninitialized read),
  boot transition. Persistent single atlas. No shader reads yet.
- **I2** — Probe-update pass (eDSL `sdf_probe_update`): single-bounce, direct + `sdf_soft_shadow`,
  CPU Fibonacci ray table, subset-limited from the start. **Run `ddgi_probe_update_cost` bench →
  DERIVE cadence.** INTERNAL checkpoint (strobes without hysteresis — not owner-committable).
- **I3** — Resolve sample: trilinear + wrap weight, unconverged→sky fallback. Bit-exact.
  INTERNAL checkpoint (leaks without Chebyshev).
- **I4** — Hysteresis (0.97) + per-frame quaternion ray rotation + **Chebyshev + depth tile
  folded in** = the FIRST owner-committable visual rung (bounce+resolve+hysteresis+leak-fix
  together). OWNER-EVAL on RTX; commit render only after visual OK.
- **I5** — Multi-bounce (prev-frame probe feedback), converged-bit gated. OWNER-EVAL.
- **I7** — Round-robin cadence (bench-derived) + classification + border wrap-copy /
  interior-clamp sampler addressing (anti cross-probe-tile bleed). OWNER-EVAL + perf capture.

Every I0..I3 keeps GI-OFF byte-identical; only flipping `ddgi_indirect=true` changes pixels.

## Defects found after SHIPPED — the host hook (lane `ddgi`, 2026-09-10)

**The feature drew zero pixels with `ddgi_indirect = true`, and every gate on the ladder was
green over it.** Verified at `ed0bed45` in the composing app, not in the render crate — the
whole defect is between the two.

| # | Mechanism (as shipped) | Consequence | Fix |
|---|---|---|---|
| D1 | `DdgiPlugin` was composed by **no host**: `boyko_app::plugins` added Lighting/Ssao/Csm/ShadowAtlas/Ray/... and never `DdgiPlugin`. | The production world had no `ResolvedDdgi`, no `DdgiCaps` reader (the runner's boot `insert_resource(DdgiCaps)` was a dead datum), and `resolve_ddgi_grid_gated` never ran. | `EnginePlugins::build` composes `DdgiPlugin` unconditionally after `ShadowAtlasPlugin` (the default carrier is the all-zero DISABLED image, so GI-OFF stays byte-identical). Gate (a1): `tests/ddgi_plugin_composed.rs`. |
| D2 | `sync_ddgi_light_gate` — the SOLE writer of the LightBuf word-7 bit-4 header gate — was registered **nowhere** (its doc said "the composing app registers it"; the app's closure mentioned it only in a comment). | The header bit was never set, so `deferred_pbr.hlsl`'s `if (ddgi_mode != 0u)` never ran: the probe atlas was updated every enabled frame and never sampled ("shipped to the GPU, not to the screen"). | Registered in `register_main_frame_systems` (the verbatim code motion of the Main closure, made a named fn so the registration is testable headless) as `.after_set(DdgiResolveSet).before_set(LightCollectSet)` — after the resolve so it reads THIS frame's carrier, before `collect_lights` so the bit lands the same frame. Gate (a2): `plugins.rs` tests. |
| D3 | The resolve's b18 grid UBO (`gpu_scene/csm.rs::ddgi_ubo`) was zero-filled at boot and **never written again**; the update pass's b6 UBO was packed from a LOCAL `DdgiConfig { ddgi_indirect: true, ..Default }` at the arm site — the owner-locked default grid, whatever the owner's config said. | Had D2 been fixed alone: the gate opens over a zero grid ⇒ `spacing = 1 / inv_spacing = +inf` ⇒ NaN in `ambient` on every `is_sdf_lit` pixel (NaN inverts under `NMin`/`NMax` into black). And the update pass marched a grid the resolve could never sample. | `upload_ddgi_grid` (mirror of `upload_atlas_ring` minus the ring) writes `ResolvedDdgi::as_bytes()` into the SINGLE b18 buffer from the runner (step 5d''), **monotone + value-gated** (enabled AND changed; the zero image never written after boot). b6 now packs from the same carrier via `scene(ddgi: Option<&ResolvedDdgi>)`. Gate (b): `ddgi_config.rs` / `upload.rs` tests. |
| D4 | Three predicates for "GI on": the gate folded config + R9c freeze (not caps); `resolve_ddgi_grid_gated` folded config + caps (not the freeze); the runner's arming re-derived config + freeze + `device_caps().ddgi_storage_ok()`. | On a no-storage device: header bit 1, grid DISABLED ⇒ the NaN case above. On a frozen-ON non-Deferred boot with the config flipped OFF: bit 1, grid zero ⇒ NaN. | ONE fold — `resolve_ddgi_grid_frozen(cfg, caps, frozen)` — inside the single writer; the gate reads `ResolvedDdgi.ddgi_mode_word`, the runner arms from it, b6/b18 pack from it. `bit == 1 ⇒ mode_word == 1` by that fold; `mode_word == 1 ⇒ inv_spacing > 0` by the D5 clamp below (the two halves are enforced at different sites — see D5). Gate (c): `ddgi_update.rs` tests + the frozen-OFF production test in `plugins.rs`. |
| D5 | (Found by the fix pass's review, W1 — a defect this lane's own repair would have CREATED.) `resolve_ddgi` packed `ddgi_mode_word: 1` with `inv_spacing = 0` for an owner `spacing` of `0` / negative / NaN / `+inf` (`if cfg.spacing > 0.0 { 1.0 / cfg.spacing } else { 0.0 }`, commented "benign degenerate"), and a positive SUBNORMAL spacing packed `inv_spacing = +inf`. `DdgiConfig::enabled()` was exactly `ddgi_indirect`, so the second half of D4's invariant — `mode_word == 1 ⇒ inv_spacing > 0` — was prose, not construction. | The shader is not benign about it: `ddgi_resolve.hlsli` computes `spacing = 1.0 / inv_spacing` = `+inf` and `origin + float3(c) * spacing` = NaN at `c == 0` — the same NaN-through-`NMin`/`NMax` black pixel as D3/D4, now on a config the owner can write. Unreachable BEFORE this lane (nothing set the bit); reachable the moment D1+D2 land. The `debug_assert!` in runner step 5d'' codifies the invariant but does not enforce it — in release it is absent, and in debug it panics the frame loop instead. | The clamp moves into the ONE structural predicate: `DdgiConfig::enabled()` = `ddgi_indirect && grid_is_sampleable()`, where `grid_is_sampleable()` is `spacing.is_normal() && spacing > 0.0 && dims != 0` (`is_normal` is exact, not stylistic: it also rejects the subnormal whose reciprocal overflows). Every variant funnels through it — `resolve_ddgi`, `resolve_ddgi_grid_clamped`, `resolve_ddgi_grid_frozen`, and the runner's boot freeze snapshot (`c.enabled()`) — so a degenerate grid is DISABLED everywhere, the reciprocal is a plain `1.0 / spacing`, and D4's second implication holds by construction. Gate: `a_degenerate_grid_resolves_disabled` + `a_degenerate_grid_stays_disabled_through_the_frozen_fold` (`ddgi_config.rs`), RED before the clamp. |

**Why the ladder's gates were blind.** The DDGI dump gate named for this rung,
`engine_grand_showcase_512_ddgi_screenshot_dump` (`boyko_rhi_vulkan/tests/window_present_gbuffer.rs`),
hand-writes the b18 grid UBO itself and drives the header bit at the RHI level — it never
touches `boyko_app`, so it was green with the host hook broken (a gate that could not fail).
The `boyko_render` unit tests pinned every piece in isolation (the resolve, the gate, the byte
layout) and no test ever asked whether a host *composes* them — the same shape as
`log_host_reachable.rs`. The substitute device gate is `boyko_app/tests/sdf_room_ddgi_dump.rs`
(the production runner, a NON-default grid so a resurrected local-default b6 pack renders
visibly wrong, `#[ignore = "gpu-windowed: …"]`).

### The substitute gate's OWN first run refuted it (measured on the device, 2026-09-10)

The gate as first shipped stated: dump this binary, dump `sdf_room_smoke` with the same env as
the GI-OFF control, require the two sha256 to differ. **Neither half of that could fail for the
right reason** — this repository's own catalogued class, found by running it rather than by
reading it.

| # | Finding | Consequence | Correction |
|---|---|---|---|
| F1 | **The control produces no artifact, on ANY machine.** `host_dump` needs `SETTLE_FRAMES (30) + 1 + DRAIN_FRAMES (3)` ≈ 34 PRESENTED frames; the runner's step-3 `AppExit` check returns from the frame loop BEFORE the present, unconditionally (`runner.rs`, "after the frame completes, before the present"). `sdf_room_smoke` sets `BUDGET = 10` ⇒ ~9 presents, exits green, writes no BMP. The GI-ON dump exists only because this binary sets `BUDGET = 40`. | The A/B was **UNANSWERED** — not passed and not failed. A borrowed control is a control that differs from the treatment in more than the treatment. | The control moves INTO this binary as a second arm selected by `BOYKO_DDGI_GATE=off`, sharing `BUDGET`, scene, camera, `CsmConfig`, window size and title. Not a second `#[test]` (`LightingPlugin`'s eviction hooks are process-global), not a second binary (that is what F1 is). The arms differ in EXACTLY ONE expression: the `DdgiConfig` inserted after `add_plugins`. The OFF arm asserts `ResolvedDdgi::ddgi_mode_word == 0` and `LightingConfig::ddgi_indirect == false`, so an arm that silently armed cannot masquerade as the control. |
| F2 | **The dump is not bit-reproducible cold-vs-warm.** Run 1 (cold, 28.22 s) vs run 2 (warm, 3.91 s) of the SAME test: 2220 of 76800 px differ (2.891 %), max per-channel delta **2**, mean max-channel delta 1.03, spread over a 288x72 band across the room rather than localised. Runs 2 and 3 (both warm) were BYTE-IDENTICAL. | A sha256 INEQUALITY between a GI-ON and a GI-OFF dump is satisfied by frame pacing alone, with no GI term whatsoever. **"The hashes differ" is not evidence.** | Each arm is run TWICE and the WARM capture is the datum; the cold one is discarded. Warm-vs-warm within one arm must be byte-identical — that is the noise-floor control, and it must hold before the A/B is read. The A/B threshold is max per-channel delta **strictly greater than 2** (the first value the measured jitter cannot produce) AND concentration on the SDF receiver, since F2's noise was NOT localised. |
| F3 | The GI-ON run **armed**: no SKIP line, no `DdgiCaps` clamp note, the in-World assertions on the armed branch executed (origin, `inv_spacing`, dims, `LightingConfig::ddgi_indirect == true`), and the staged light-table header read word7 = `0x00000014` — `DDGI_MODE_BIT` (bit 4) set in the bytes uploaded to the GPU. | This is a GPU-side claim of exactly the shape the defect is named after ("shipped to the GPU, not to the screen"). | It is therefore **NOT the gate**. The gate is the pixels. |
| F4 | The 0%-gate half DID pass: `[grand_showcase_2mat]`, `[vb_both_sdf]`, `[sdf_forward_only]`, `[vb_both]` byte-identical after `DdgiPlugin` became unconditional (`scripts/golden.ps1` CHECK, `PINS.toml` untouched). | D1's unconditional composition costs GI-OFF nothing. | Unchanged; kept as the separate half of the gate. |

| F5 | **The corrected gate's own first run refuted its second clause, and the run PASSED anyway.** Measured on the device 2026-09-10 with the two-arm binary: session noise floor **0** (three byte-identical captures per arm), ON-vs-OFF **12654 of 76800 px differ, max per-channel delta 37**, **all 12654 brighter on ON and 0 darker**, the 22720 sky pixels untouched, and the sphere's own disc 1038/2071 px all brighter (max 6). But **88.9 % of the differing mass, and every pixel above delta 10, lie OUTSIDE the sphere disc** — the strongest blob (mean +9.0, max 37) is the CUBE face at `(-2, 0.5, -1)`. | The clause "concentrated on the SDF receiver" would have returned a **RED against a working fix**. Its premise — "GI applies to `is_sdf_lit` pixels, so the SDF sphere is the receiver" — is false in its second half: `deferred_pbr.hlsl:786-788` defines `is_sdf_lit = material_texel.b > 0.5`, the SDF-LIGHTING MASK, and `sdf_gbuffer_composite.hlsl:1884` writes it as `1.0` for RASTERISED geometry too, leaving `0` only on the background. The cubes and the floor are receivers. The gate had moved from "cannot fail for the right reason" to "can fail for a WRONG reason". | Clause replaced by the three that the measurement shows are the discriminating ones: **magnitude** (> 2), **sign** (every differing pixel brighter — pacing jitter is two-sided, an additive radiance term is not), and the **mask boundary** (delta 0 where the mask is 0; a non-zero all-brighter term on the sphere's disc). The receiver box is now used to CHECK that the disc carries a term, never to reject a difference for being elsewhere. |

**The corrected gate.** (a) Four captures — ON cold/warm, OFF cold/warm — under
`BOYKO_DISABLE_VALIDATION=1 --test-threads=1`, each to its own path, `BOYKO_DDGI_GATE=off`
selecting the control. (b) Warm-vs-warm within an arm byte-identical (noise floor). (c) ON-warm
vs OFF-warm, THREE clauses (see F5): max per-channel delta > 2, EVERY differing pixel brighter
on ON, and delta 0 wherever the mask is 0 (the sky), with the SDF sphere's disc carrying a
non-zero all-brighter term — analytically a
~53x53 disc at `x ∈ [101, 153]`, `y ∈ [73, 125]` with `y` DOWNWARD, i.e. file rows `[114, 166]`
in the bottom-up BMP; recompute or locate it in the OFF dump before rejecting a difference.
(d) The pinned GI-OFF goldens binding the DDGI descriptors stay byte-identical after D1's
unconditional composition. The run protocol lives in the test's module doc, which also records
F1 as the reason the previous protocol was replaced.

**Layout pin recorded (mechanical).** Every committed resolve `.spv` (`deferred_pbr*.comp.spv`,
`vb_shade_split*.comp.spv`) carries `OpMemberDecorate %type_ResolvedDdgi 3 Offset 36` — a
48-byte block, field-for-field `ResolvedDdgi`; `DDGI_UBO_BYTES == RESOLVED_DDGI_BYTES == 48`,
so there are NO bytes past the struct — bytes 36..48 are the three `_pad` words (zero, read by
no shader).

**Single buffer, stated.** b18 stays ONE buffer by DESCRIPTOR contract, not because the grid is
static: `GBufferTargets::create` builds the resolve sets once and captures the boot buffer, so a
host `[slot]` ring would not be observed by the GPU (the same is true of the atlas "ring" at
binding 15 — a pre-existing class, out of this lane). The token proves one slot's fence; the
monotone value gate is what bounds a concurrent sibling read to finite grids: steady state
writes nothing; the first ENABLE lands on the frame whose own light staging carries the bit
(the sibling's per-slot staging still has bit 0, so it never reads b18); a runtime edit can tear
one sibling read for one frame, but both halves are finite grids; a DISABLE leaves the last grid
bound-but-unread.

### "Why is the GI ~18x stronger on the cube than on the SDF sphere?" — ANSWERED (C1), not a defect

The device gate above proved the term reaches the screen; the same capture showed it far stronger
on a mesh cube face than on the SDF sphere, and the obvious reading — "the sphere is not receiving
GI" — is wrong. **The next reader will ask this again, so the answer is recorded here rather than
re-derived.**

**The probe-update pass's world is the SDF edit list, and nothing else.** `sdf_probe_update.comp.hlsl`
binds `Buf` (the edit list) + the two atlases + the ray table + the light table; `probe_march`
sphere-traces `field_distance`, and a miss returns radiance `0` (a black sky). In the eval scene the
edit list is ONE sphere. So the sun-lit sphere is the **sole emitter in the bounce**, and:

- **A convex emitter cannot light itself.** For a receiver point `X` on the sphere with outward
  normal `n`, every other sphere point `S` satisfies `(S - X)·n <= 0`, so no sphere point lies in
  the `+n` hemisphere and `probe_blend`'s `max(dot(texelDir, rayDir), 0)` weight is `0` for every
  ray that carries sphere radiance. The sphere's own GI is therefore near-zero **by geometry**, not
  by a bug.
- **The cube face at `(-2, 0.5, -1)` is the surface that looks straight at the sphere's SUN-LIT
  side**, which is why it carries the strongest term.

The decisive evidence is the per-surface decode, because it removes solid angle as the explanation:

| surface | relation to the emitter | measured GI term |
|---|---|---|
| cube face at `(-2, 0.5, -1)` (the strongest blob) | faces the sphere's **sun-lit** side | mean **+9.0**, max **37** |
| cube0 `+z` vs cube3 `-x` | **near-identical solid angle** onto the sphere, opposite SIDES of it | **20x** apart |
| SDF sphere's own disc (1038 of 2071 px) | the emitter itself — convex, cannot light itself | all brighter, max **+6** |
| sky (22720 px) | not a receiver (`is_sdf_lit == 0`) | delta **0** |
| whole frame | — | 12654 of 76800 px differ, **all brighter**, none darker |

Two faces of near-identical solid angle differing 20x **purely by which side of the sphere they
see** is an emitter signature, not an attenuation signature. *(Provenance: these figures are the
device gate + analysis pass of 2026-09-10, quoted; this pass ran no device and did not re-measure
them. The convexity argument above is geometry and needs no measurement.)*

The premise the question rests on — "GI should reach the sphere" — assumes a bouncer the update
pass never sees. **That assumption is a real scope question, and it is the design limit stated at
the bottom of this section, not a defect.**

### Two REAL defects, found on the way to that answer — both FIXED (lane `ddgi`, 2026-09-10)

Confirmed independently before anything was changed: every site re-opened, every figure re-derived,
and D-B's host gate written and run RED first. One of the two was **refuted in its stated location**
and is recorded that way.

#### D-A — the bounce carried no `rho/PI`. CONFIRMED as stated. Factor exactly `PI/rho`.

`shade_hit` accumulated `lit += e.color * (NoL * vis)` and returned `lit * GI_BOUNCE_SCALE` with
`GI_BOUNCE_SCALE = 1.0`. `e.color` is `linear_color x illuminance` already baked, so `lit` is the
**irradiance E arriving at the hit point** — no reflectance, no `1/PI`. A Lambertian bounce must
carry `L_o = (rho/PI) * E`.

**The read side was checked, not assumed, because a matching omission there would have cancelled
it — and it does not.** `probe_blend` divides by `sum_w`, i.e. stores a cosine-weighted **mean
radiance** (`E/PI` for a constant field), and `deferred_pbr.hlsl`'s
`ambient += diffuse_color * gi * ao_final` applies the RECEIVER's albedo with no further `1/PI` —
correct **precisely because** `gi` is already `E/PI`. Exactly **one** factor of `rho/PI` was missing
from the chain, on the write side.

- **`PI / 0.8 = 3.926991`** at the engine-default material (`MaterialGpu::default` base `0.8`,
  non-metal ⇒ `diffuse_color = base * (1 - metallic) = 0.8`).
- Even at a perfectly white bouncer (`rho = 1`) it would be `PI = 3.1416x` too strong: the omission
  was never a tint, it was the whole BRDF normalisation.
- The constant's own comment conceded the omission and said it had been *"tuned from the owner-eval
  picture"* — a picture whose only bounce receiver was a mesh face.

**Fixed as TRANSPORT, with the look left as an explicit knob.** `GI_BOUNCE_SCALE` is replaced by
three named constants:

| constant | value | what it is |
|---|---|---|
| `GI_BOUNCE_ALBEDO` | `0.8` | **Physics.** A stand-in for the per-hit reflectance the update bind-set cannot look up (it binds no material table), set to the engine's OWN default material base colour. A measurable quantity, not a preference. |
| `GI_INV_PI` | `0.318309886` | The Lambert normalisation. |
| `GI_BOUNCE_INTENSITY` | `1.0` | **The artistic knob** — the only dial here that expresses a look preference. `1.0` = no artistic scaling. |

**What this does to the picture, stated so the owner can choose the look without the maths being
wrong:** at `GI_BOUNCE_INTENSITY = 1.0` the GI term becomes **3.926991x darker** than the shipped
picture (a factor of `0.8/PI = 0.2546479`). **`GI_BOUNCE_INTENSITY = PI / GI_BOUNCE_ALBEDO =
3.926991` reproduces today's picture** — to within float rounding of the product, not bit-exactly.
**No new default brightness was chosen here**: `1.0` is physically-correct transport, and moving it
is an owner call.

**The firefly clamp moved with it, on purpose.** `DDGI_MAX_RADIANCE = 16.0` was applied in `main`
to `shade_hit`'s return — a quantity that was `PI/rho` too large. Left at the same numeric `16` on
the now-correct radiance it would have silently **loosened** by `PI/rho`, letting through fireflies
the shipped code clips. It is therefore renamed `DDGI_MAX_HIT_IRRADIANCE` and applied INSIDE
`shade_hit`, ahead of the BRDF factor, on the same `lit` the old code clamped — so **the set of rays
clipped is bit-identical to before**, and the fix is transport only.

#### D-B — REFUTED as stated (`oct_encode`/`oct_decode`), CONFIRMED one layer down (`border_copy_index`)

**`oct_encode` and `oct_decode` do NOT disagree, and were not touched.** They are the standard
Cigolle pair and exact mutual inverses, edges and corners included
(`boyko_shaderdsl/src/oct.rs:98` / `:181`). The decode's `nx += nx >= 0 ? -t : t` with
`t = saturate(-nz)` is algebraically the classic `(1 - |n.yx|) * signNotZero(n.xy)`. Round-trip
verified over every interior texel of both tiles plus the four edge extremes; the tree's own
`oct_decode_edsl_matches_host` was already green. **Changing them would have forked the G-buffer
normal goldens that depend on `oct_encode`** (`goldens.rs:4958-4967` documents that dependency) — so
the defect as originally stated would have been "fixed" in the one place that must not move.

**The real defect was the border-copy index map**, `sdf_probe_update.comp.hlsl`'s
`border_copy_index` — hand-written HLSL **outside** every `// === GENERATED ... ===` span.

The octahedral square folds **each edge onto itself, reversed**: `(1, s)` decodes to
`normalize(1-|s|, 0, -|s|)`, independent of `sign(s)`, so a path leaving through the right edge
re-enters through the **right** edge at the mirrored row. The continuation of a border texel is the
reflection of its own position about the crossed edge, tangential coordinate negated. The committed
map took every edge run from the **opposite** side of the tile instead:

| border run | committed `src` (pre-D-B) | correct `src` |
|---|---|---|
| top row | `(v-1-cx, v-1)` — bottom interior row | `(v-1-cx, 0)` — the **top** row it touches, column reversed |
| bottom row | `(v-1-cx, 0)` | `(v-1-cx, v-1)` |
| left col | `(v-1, v-1-bt)` — right interior col | `(0, v-1-bt)` |
| right col | `(0, v-1-bt)` | `(v-1, v-1-bt)` |

**The four CORNER arms were correct and are unchanged**, because at a corner the two reflections
compose (in either order — they commute) to the diagonally-opposite interior corner. That is almost
certainly how the bug survived review: the correct corner rule was generalised to the edges, where
it does not hold. The code comment stated the wrong rule in as many words ("copies the OPPOSITE
interior row"), so comment and code agreed with each other and both were wrong.

**Measured damage (host oracle):** **24 of 28** irradiance border texels and **56 of 60** depth
border texels held a direction from the far side of the sphere — exactly the edge texels, with the
4 corners of each tile correct. A receiver normal that encodes onto a tile edge (`±x`, `±y`) lands
at `px = ox + BORDER + e.x * VALID_EXTENT` (`ddgi_resolve.hlsli:73`), i.e. exactly on the
last-interior/border boundary, so a LINEAR tap draws **50 %** of its weight from that ring: for `+x`
the border texel held `[-0.98, 0.196, 0]`, `cos = -0.98` against the receiver normal. `+y` is the
normal of **every floor and every upward-facing face**, so this was the most common receiver normal
there is, not an exotic corner. The **depth** tile is affected identically and feeds Chebyshev, so
the repair fixes a **leak** term as well as a colour term.

*(This also re-attributes the analysis pass's "+6 on the `+x` rim" and "+1.4 at `n.x = -0.9`": the
symptom was real and reproduced, the cause named for it was not.)*

**Other consumers of `oct_encode`/`oct_decode` were enumerated before touching anything, and none
depended on the border behaviour:** `deferred_pbr.hlsl`, `gbuffer_mrt.fs.hlsl`,
`sdf_gbuffer_composite.hlsl`, the four `sdf_ssao*`, `shadow_atrous`, `vb_*` all encode/decode into
**unbordered** RGBA8 normal targets. The bordered-atlas chain is only `sdf_probe_update.comp.hlsl`
(write) + `ddgi_resolve.hlsli` (read). Nothing else reads `border_copy_index`.

#### Shader ownership, and the gates

`border_copy_index` and the bounce constants are **hand-written glue OUTSIDE the generated
sentinels** (the generated spans are `oct_decode`, `probe_march`, `probe_blend`,
`probe_depth_blend`). They are nonetheless carried verbatim in the generator's `format!` literal
(`boyko_shaderdsl/src/bin/emit_probe_gi.rs`), so **both files were patched from one substitution
table**, the generator was **re-run**, and its output was diffed against the hand-patched file:
**identical** (LF-normalised). The `.spv` was then re-compiled with the frozen recipe from the
shader's own header, cwd = the shaders dir:

```text
dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 sdf_probe_update.comp.hlsl -Fo sdf_probe_update.comp.spv
```

| gate | result |
|---|---|
| `ddgi_oct_border_host_oracle` (4 arms — **new**, CPU-only, no device) | `4 passed` — RED before the fix (`24 of 28` / arm 3 `cos = -0.98`) |
| `ddgi_probe_update_spv_sync::sdf_probe_update_spv_byte_identical` (**new**) | `1 passed` |
| `emit_probe_gi` (eDSL drift + CPU oracles, `--features emit`) | `10 passed` |
| `ddgi_probe_gi_sync` (`oct_decode_edsl_matches_host`, `sdf_soft_shadow_ranged_copy_matches_resolve`) | `2 passed` |
| `sdf_field_edsl_sync` | `24 passed` |
| `marcher_spv_sync` | `2 passed` |
| `ddgi_probe_sample_host_oracle` | `13 passed` |
| `boyko-render --lib ddgi` | `22 passed` |
| `cargo check --workspace --all-targets` | clean |

**`sdf_probe_update.comp.spv` had NO byte gate before this pass**, and `emit_probe_gi.rs`'s own
header claimed one existed ("the emit drift/sync tests pin the committed `.spv` to a fresh re-DXC").
A tree-wide grep found the embed site, two doc comments and the recipe line — and no test. The new
`ddgi_probe_update_spv_sync.rs` is that gate. Line endings do not leak into it: the generator writes
LF and this checkout is CRLF (`core.autocrlf=true`), and compiling both forms was **measured** to
produce identical SPIR-V (`f8fe2da5…` either way) — so it gates the shader, not the checkout.

The host oracle's fourth arm exists because arms 1-3 judge a **Rust transcription** of HLSL: it
`include_str!`s the committed shader and pins the eight `src = uint2(...)` expressions, in source
order, against the constants written beside the transcribed returns. Demonstrated to fire — with
the transcription corrected and the shader not yet patched it failed, naming all four swapped edge
expressions and showing the four corner expressions unchanged. **Its residual is stated in the test:
it catches an edit to ONE side, not a matching wrong edit to both.**

#### GI-OFF is untouched — by construction, and the construction is checked

1. **Exactly 1 of the 117 committed `.spv` in the tree changed** (`sha256` of every one, before and
   after): `sdf_probe_update.comp.spv`. Every resolve blob — `deferred_pbr*.comp.spv`,
   `vb_shade_split*.comp.spv` — is byte-identical, so the pixel-producing shader on the GI-OFF path
   is literally the same artifact.
2. That one `.spv` IS compiled into a pipeline at boot unconditionally
   (`boyko_app/src/gpu_scene/csm.rs:291`) — the honest statement is "loaded, never dispatched", not
   "never loaded". The **pass** is added only under `scene.ddgi_update.is_some()`
   (`graph_bridge.rs:2194` deferred; `:5902` under VB via `path_vb_ddgi()`, which ANDs
   `ddgi_update.is_some()`), so on the OFF path no `ddgi_update` pass exists, nothing dispatches,
   and the atlases keep their boot-clear contents.
3. The resolve's GI block is behind `if (ddgi_mode != 0u)` (`deferred_pbr.hlsl:1189`), and the OFF
   path carries `ddgi_mode_word == 0`.
4. Every pinned golden routes through `run_showcase_body`
   (`boyko_rhi_vulkan/tests/window_present_gbuffer.rs:9005`), whose `ddgi_update: None` is at
   `:9978`. The two other `None` sites are `body_windowed_gbuffer_composite` and
   `body_p0_coarse_cull`; the single `Some(...)` site is `run_showcase_body_ddgi`, the ignored
   device gate.
5. **No golden was re-blessed, and `goldens/PINS.toml` is untouched** (`git status` shows five
   paths: the shader, its `.spv`, the generator, and two new test files).

**What that does NOT prove:** the goldens were not re-rendered in this pass (no device run here).
The claim above is a reachability construction plus the measured 1-of-117 artifact delta, which is
what "byte-identical by construction" means — it is not a substitute for the device CHECK, which the
orchestrator or the owner should still run (`scripts/golden.ps1 -Pin <name>` over the four DDGI-
descriptor-binding pins named in F4).

### Known asymmetry, no code (C3): probe burial is tested against the SDF only

`sdf_probe_update.comp.hlsl`'s classification does `bool inside = field_distance(pw) < GI_INSIDE_EPS`
— the **SDF field alone**. A probe buried inside a **mesh** is therefore never deactivated, and
because `probe_march` also marches `field_distance`, the depth moments never see the mesh either, so
Chebyshev cannot reject it. **No magnitude effect in the eval scene** (its cube is transparent to the
field), which is why this is recorded rather than fixed: fixing it needs mesh occupancy in the
update pass, which is the same scope question as the design limit below.

### What remains UNPROVEN after this pass

- **Single-bounce GI over an SDF-ONLY world is a DESIGN limit, not a bug.** The I2 row of the ladder
  above says it in as many words: *"single-bounce, direct + `sdf_soft_shadow`"*. The update pass
  binds the edit list and nothing else, so **meshes never bounce and the sky never bounces** —
  a mesh wall beside a lit floor contributes zero indirect light, whatever the knob is set to. That
  is the honest frame for C1: the sphere-vs-cube asymmetry is the *correct* behaviour of a bounce
  whose only emitter is the SDF. **Whether the update pass should see mesh geometry at all is a
  SCOPE question this pass does not answer** — it is an owner/architecture call (a mesh occupancy
  or BVH source in the update bind-set), not a defect to repair.
- **Neither fix has been seen on a screen.** Both are proved on the host and in the compiled
  artifact; the D-B repair's on-screen effect (the `±x`/`±y` rims and every floor/upward face) and
  the D-A darkening are **unmeasured** here. The `sdf_room_ddgi_dump` two-arm gate and its three
  clauses (magnitude / sign / mask boundary) remain valid over the fix and should be re-run.
- **The generator↔committed link is still only a 4-span `.contains`.** `emit_probe_gi.rs`'s drift
  gate pins the four eDSL spans, not the whole file, so the hand-written glue the two files share
  has no automatic gate. This pass verified equality by re-running the generator and diffing —
  once, by hand. Making it mechanical needs `build_shader` moved out of the `[[bin]]` into the
  library so a test can call it; not done here.
- **`GI_BOUNCE_ALBEDO` is one constant standing in for every bouncer.** Coloured bleeding and any
  per-hit reflectance need the material table in the update bind-set — unchanged scope.

## Open risks (carried)

- Update-pass µs/probe is the one genuinely-new number — UNMEASURED until the I2 bench; if far
  above the paper's simple-primitive rate, coarsen grid / cut rays (bench-derived knobs absorb it).
- Cap-raise assumes 3 extra descriptors stay under device `maxPerStageDescriptor*` — assert at boot.
- Cross-probe tile bleed via trilinear near the 1-texel border — border wrap-copy + interior-clamp
  pinned at I7.
- Multi-bounce gather (I5) is texture-bound (8 scattered tiles/hit); plane-major layout puts
  vertical neighbours a row-stride apart — if it dominates the bench, feedback at lower cadence.
- First-frames convergence: round-robin 1/4 → ≥4 frames to first coverage; converged-once bit
  gates unconverged probes to sky-ambient in both resolve and feedback.
- Persistent-single-atlas (D2) relies on a probe never reading its own tile mid-write + a
  mandatory update→resolve barrier — prove under validation/Miri-TB for the dispatch ordering.
