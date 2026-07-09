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
