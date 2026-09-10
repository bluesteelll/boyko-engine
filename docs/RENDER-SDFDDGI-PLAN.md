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
visibly wrong, `#[ignore = "gpu-windowed: …"]`): its sha256 must differ from the GI-OFF
`sdf_room_smoke` dump, and the pinned GI-OFF goldens binding the DDGI descriptors
(`[grand_showcase_2mat]`, `[vb_both_sdf]`, `[sdf_forward_only]`, `[vb_both]`) must stay
byte-identical after D1's unconditional composition.

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
