# Audio engine — the design space

**Status:** rev 2, architect's draft (2026-09-25), revised after the first critique pass. That pass
raised 2 critical, 9 important and 6 optional remarks. All 17 are accepted and fixed here, and the
[Review log](#review-log) at the end maps each one to its fix. Nothing here is ratified. Every
technical fork is decided below, with its reason. §12 holds only the owner's **value** questions.

**Evidence.** Sources, tags and the repository inventory are in [AUDIO-RESEARCH.md](AUDIO-RESEARCH.md).
Its tags are reused here: [V] primary page read, [S] snippet only, [2] secondary source, [E] this
document's derivation, [R] repository fact at `6394bc5e`. A number without [V], [S] or [R] is an
estimate [E], and §11 queues its measurement.

**Trunk read:** `6394bc5e`. Paths are relative to the repository root. Code is cited as path plus
symbol.

---

## TL;DR — the recommendation

1. **Build it in-house.** Middleware and the Rust audio crates all fail at least one owner rule, and
   the parts that must be written anyway dominate the effort: the ECS surface, voice selection and
   the cross-thread transport (§4 F1).
2. **One real-time (RT) audio thread, and nothing else in the render path.**
   - The thread is registered with MMCSS as "Pro Audio" and sets flush-to-zero for itself only. Every
     device call that can make the OS or a plugin create a thread runs under the default MXCSR (F19).
   - It runs the WASAPI shared-mode event loop with a **bounded** wait. It renders fixed 256-frame
     blocks at a **fixed 48 kHz internal rate**, and resamples the master once when the device runs
     at another rate (F2, F5).
   - Everything Unreal's "audio thread" does (sound selection, parameters, virtualization) runs as
     ordinary ECS systems on the pool at frame rate.
   - The RT thread knows only the real voices (default 64, plus 32 rows of fade headroom) and never
     touches the World (F4, F7, F11).
3. **Forward: one command ring. Return: a lossless board, not a queue.**
   - The forward ring carries fixed-size records, one batch per frame. Traffic is bounded by the RT
     row count, because each row receives at most one record per frame.
   - A batch that does not fit is deferred whole, and no command is ever lost: publishing is a diff
     between the slot table's desired state and what was last published.
   - The return direction carries only state that must not be lost: one acknowledgement word per RT
     row, monotonic counters, a block-time histogram and the clock. The RT thread overwrites them in
     place, so a stalled consumer loses nothing and nothing can overflow (F6).
4. **Principle 0.**
   - What plays, where, how loud and why lives in components and resources.
   - The RT working set is kernel storage owned by a plugin resource. It is **sized, committed and
     touched at boot**. The RT thread gets fixed-length views and has no path that grows memory. A
     thread-scoped commit deny proves it (F7, K-A7).
   - That set is disposable and is rebuilt after a device reset, the way a GPU mirror is.
5. **Samples.**
   - PCM16 and IMA ADPCM are resident: one write-once span per clip in a KC-15 segmented column,
     which never moves a span that does not grow. A freed span is reused only after the RT thread has
     acknowledged its last read (F8).
   - Streams (music, ambience, dialogue) use QOA, decoded ahead on a dedicated low-priority IO
     thread. The RT thread never waits (F9).
   - Vorbis and Opus sit behind owner question Q4.
6. **The mixer.**
   - A bus graph declared as data and flattened at boot, with effects as a closed enum. No virtual
     dispatch.
   - Voices are processed eight to an AVX2 register for filter state.
   - DSP kernels are generic over a `Lane` type: a scalar oracle and an x8 instantiation that is
     bit-identical to it. The kernels are also **deterministic across hosts**: they use IEEE basic
     operations and `sqrt` only, with in-house transcendental approximations, because Rust documents
     `sin`, `exp` and `ln` as non-deterministic [V S63] (F10).
7. **Spatial.**
   - Stereo constant-power panning, attenuation curves, Doppler and distance low-pass, computed on
     the ECS side (M2).
   - Headphone 3D through an ambisonic bus with one binaural decode. It is cheaper than per-source
     HRTF above roughly 3–40 real voices, with a central estimate of about 10 (F12, M6).
8. **Clocks.** `Timing::Exact` maps each frame's span of game time onto real time anew every frame.
   Pause, slow motion and clamped frames therefore never produce stale targets. The late events of a
   hitch follow a per-sound late policy (F16).
9. **Acoustics, where the hybrid engine matters.** Occlusion takes the minimum of two CPU arms on the
   pool:
   - a sphere trace through the analytic SDF field, using the existing bit-exact 8-wide kernel. It
     costs about 0.4–0.6 µs per ray at 16 edits and grows linearly with the edit count. Endpoints are
     lifted off nearby surfaces and the penumbra term is two-sided, so a source resting on a floor or
     against a wall does not read as occluded;
   - a collider ray arm that needs a ray arm in SI1.

   The SDF arm covers the analytic edit list only. F13 is re-decided when the edit cap rises or the
   brick clipmap lands. A GPU arm is rejected today because there is no gameplay readback path and
   no async compute. Latency is not the reason: the occlusion smoothing hides it (F13).
10. **Cost [E] on the reference CPU** (Ryzen 9 5900HS):
    - about 290 µs of RT time per 10 ms of audio (2.9% of one core) for 64 ADPCM voices with full
      per-voice DSP, 2 reverbs and 4 streams. The worst transient, with all 96 rows active, is about
      400 µs (4%);
    - about 350 µs of pool CPU per frame on the ECS side;
    - 0 GPU ms at every resolution and 0 MB of VRAM;
    - about 4.5–6.5 MiB of engine RAM, of which the RT working set (about 3.9 MiB) is committed at
      boot. Content comes on top, and the sample bank commits about 1.4× its content bytes (§3).
11. **Sequencing.**
    - The leaf parts land now, in new files only: FFI, DSP kernels and their deterministic math,
      decoders, the offline harness.
    - Everything that needs kernel storage lands after D-S2 and C1, so audio is not built against
      the pre-Phase-D contract and ported twice (§8).

---

## 0. Inputs

### 0.1 What the tree already decided about audio

This is the first audio design in the tree, so it is not a delta. These earlier decisions bind it:

| Decision | Where | Consequence here |
|---|---|---|
| Capability = component presence (Axis 1); runtime on/off = an `EnableTag` bit (Axis 2); audio is named as a covered subsystem | [CAPABILITY-STATE-MODEL.md](../CAPABILITY-STATE-MODEL.md) | `AudioEmitter` presence and an `AudioEmitterEnabled` bit (§6.2). Never an `enabled` field that a system iterates over. |
| "Audio" is a reserved engine scope in the profiler's `ARM_MASK` | [PROFILING-SYSTEM-PLAN.md](../PROFILING-SYSTEM-PLAN.md), D20 | The RT thread's zones go under that scope (F17). |
| The replay promise covers exactly its digest: "not rendering, not audio" | [editor/EDITOR-REPLAY.md](../editor/EDITOR-REPLAY.md), point 3 | Audio output is outside replay. Whether audio *decisions* are inside it is Q3. |
| A runtime that sets FTZ/DAZ on its init thread breaks the physics inline-versus-worker bit-identity; nothing in the tree writes MXCSR; gate G-FP pins workers to the default | [physics/FMA-DETERMINISM.md](../physics/FMA-DETERMINISM.md); `crates/boyko_threadpool/tests/mxcsr_uniform.rs` | Flush-to-zero is confined to the RT thread, set inside its own entry. The thread creates no thread itself, and every backend call that can make the OS or a plugin create one runs under the default MXCSR (F19). |
| GPU particle counts read back for gameplay or audio is an open question | [PARTICLES-RESEARCH.md](../PARTICLES-RESEARCH.md), Q8 | Audio drives rain and fire textures from CPU-side emitter parameters. No readback is needed (§6.8). |
| "sound" is a Gaia asset reference kind (path-name hash, `PathIndex::lookup`) | [gaia/DECISIONS.md](../gaia/DECISIONS.md) | `SoundDef`, bus graphs and snapshots are Gaia-authorable assets (§6.6). |
| An Aether machine's push event lane feeds sound one frame later | [aether-v2/MACHINES.md](../aether-v2/MACHINES.md) | `PlaySound` is an ordinary event that Aether can send (§6.4). |
| Only the host changes process-wide state | `crates/boyko_app/src/timer_resolution.rs` (module doc) | MMCSS and FTZ are per-thread, so the audio crate may do them on its own thread. The low-latency period is different: it switches **every application on the endpoint** to the small period while the stream runs [V S1]. That reaches wider than the process, so only the host may request it. The library default never changes the engine period (F2). |
| `windows-sys` is approved for the OS layer, with features added per crate | root `Cargo.toml`; `crates/boyko_rhi_vulkan/Cargo.toml` | The audio crate adds its own `windows-sys` features. COM vtables are hand-declared. |
| A new engine crate is registered in `ENGINE_PACKAGES` | `crates/boyko_diag/src/sample.rs`; `tests/engine_packages_census.rs` | Part of M0. |
| Ignored tests carry a class from a closed vocabulary that lives in CLAUDE.md | `IGNORE_CLASSES` in `tests/ignore_reasons_census.rs` | Real-endpoint tests need a new class (`audio-device`), which is a CLAUDE.md edit made with the owner. Every other gate runs device-free (§9). |
| The allocator is replaced per type by lifetime class, and `#[global_allocator]` is kept only as a deny gate (`DenyAfterSteady`) | [memory/ALLOCATOR-DESIGN-SPACE.md](../memory/ALLOCATOR-DESIGN-SPACE.md) | The RT thread allocates nothing after boot. A heap counter is blind to kernel columns, which grow by VM commits (`crates/boyko_physics/tests/alloc_frame_census.rs`, section "Coverage boundary"). So the RT gate denies commits too (§9 M0, K-A7). |
| The kernel contract (Phase D) plans `ByteColumn` (KC-03), `ScratchColumn::for_type` (KC-10), `SegmentedColumn` (KC-15), `OwnedColumn::retain_ready` (KC-16), event lanes and a dispatcher-only `OsEventSink` (KC-26), and a thread context (KC-04) | [unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md](../unification/UNIFIED-SYSTEM-PLAN-01-KERNEL-CONTRACT.md) | Audio's storage is KC-03, KC-10 and KC-15. `OsEventSink` cannot serve an RT thread, being "Copy but not Send" ([ENGINE-RUNTIME-ECS-DESIGN.md](../unification/ENGINE-RUNTIME-ECS-DESIGN.md) §11), so K-A2 is needed (§8). |

### 0.2 Corrections to the research input

The research report this design was handed contained five statements that the repository refutes
or sharpens. Each is corrected here, because an estimate that feeds a decision must be right.

1. **The SDF occlusion ray costs about 0.4–0.6 µs, not about 20–40 µs.**
   - The report assumed 100 edits per field evaluation. The CPU analytic field is capped at
     `MAX_SDF_EDITS = 16` (`crates/boyko_sdf_math/src/lib.rs`).
   - An 8-point AVX2 evaluator already exists (`sdf_edit_list_x8`, `crates/boyko_physics/src/sdf_simd.rs`).
   - The cost is linear in the edit count, so the figure holds only at today's cap. Derivation and
     scaling are in §3.2.
2. **"~32 → ~440 M ops/s" is not a boyko measurement.** `crates/boyko_log/src/lane.rs` cites it as
   "the one published measurement". The shape of the ring is ours; the number is not.
3. **Decode-ahead does not go on pool workers.** The report proposed Unreal-style async decode tasks.
   The boyko pool has no priority lanes and no async-IO facility, and physics saturates it. F9 moves
   decode-ahead to a dedicated IO thread and gives the reason.
4. **Kernel event lanes cannot carry RT traffic.** `OsEventSink` is dispatcher-only (KC-26 / EK7), so
   the forward path needs its own ring (K-A2). The return path needs no ring at all (F6).
5. **Two more RT hazards exist, and neither is in the report.**
   - A lane-less thread that logs at `Warn` or `Error` takes a synchronous channel with a 50 ms
     deadline (`crates/boyko_log/src/lane.rs`, `emit_to`).
   - Linux threads inherit MXCSR from their creator (`crates/boyko_threadpool/tests/mxcsr_uniform.rs`).

   F17 and F19 answer them.

---

## 1. The owner rules as scoring columns

| Rule | How audio reads it |
|---|---|
| **R1 — web research** | Done in [AUDIO-RESEARCH.md](AUDIO-RESEARCH.md). Where the search budget stopped short, the claim is tagged [S]. None of those claims decides an option on its own. |
| **R2 — HYBRID perf with numbers** | Audio is geometry-agnostic except in acoustics. There the mesh + SDF split decides the occlusion design, argued with numbers and with scaling in the edit count in F13. The recommended architecture records **no GPU work: 0 GPU ms at 1080p, 1440p and 4K, and 0 MB of VRAM**. Its costs are CPU µs per 10 ms of audio (RT thread) and CPU µs per frame (ECS side). They scale with voices, listeners, zones, rays and SDF edits, never with resolution. |
| **R3 — principle 0, structural capability, zero cost when unused** | Emitters, listeners and zones are components. Voice tables, graphs, configuration and the RT working set are resources. Logic is systems. With the plugin absent or `AudioMode::Off`, nothing is registered, spawned or opened, **in any crate**, including the composing layer's bridges (§6.1). |
| **R4 — hot path** | After boot, the RT thread makes no heap allocation and no VM commit, takes no lock, and uses no virtual dispatch. Its only blocking call is its own bounded pacing wait. The same rules hold for the per-frame ECS systems. The gates are in §9. |
| **R5 — in-house** | In-house everywhere, including the bake-time encoders (F9) and the transcendental math (F10). `windows-sys` covers the OS layer, as it already does for input. The one bake-time exception is HRTF conversion from SOFA/HDF5 (F12). Vorbis and Opus are Q4. |
| **R6 — shaders, eDSL, RHI** | No shader, so no manifest rows. The eDSL's principle (one generic body instantiated as a host oracle and as the production arm) is applied to the DSP kernels through a `Lane` type (F10, §7). The rejected GPU path's RHI gaps are listed in §7. |

---

## 2. Reference machine and assumptions

- **CPU.** The owner's workstation, a Ryzen 9 5900HS (Zen 3, 8C/16T) [R], with the `x86-64-v3` ISA
  baseline (AVX2, FMA) [R]. Its L2 is 512 KiB per core, shared by the two SMT siblings
  ([p0b diag report](../measurements/2026-09-19-physics-p0/p0b/diag_report.md)) [R].
- **Clock.** A single busy thread is assumed to run at about 4 GHz [E]. SMT contention with a busy
  sibling can cost up to 2× on the RT thread; the budget in §3.3 absorbs that.
- **Audio format.** 48 kHz stereo internally, whatever the device rate (F5). One 10 ms reference unit
  = 480 frames = 960 samples. Every CPU figure below is **µs per 10 ms of audio**, whatever the
  internal block size, so the figures compare directly with the research report's.
- **GPU.** The reference GPU elsewhere in the tree is an RTX 3060 Laptop [R]. Audio uses it only in
  the rejected arm of F13.

---

## 3. Cost model

### 3.1 Why these numbers are estimates

No audio code exists, so nothing can be timed. Every per-stage figure is derived from an operation
count and a latency chain, and marked [E]. The M0-leaf benches (§11, MQ-A1) replace them before any
rung depends on them. A decision below that would flip if a figure were off by more than 2× says
so.

### 3.2 Per-stage cost, per voice, per 10 ms

| Stage | µs | Derivation [E] |
|---|---|---|
| PCM16 → f32 | 0.03–0.05 | 120 vector groups of 8 samples, about 4 ops each |
| Gain-ramped stereo accumulate into a bus | ~0.1 | 120 AVX2 iterations × (load, mul, add, store, ramp add) |
| Resampler bypass (same rate, pitch 1) | ~0.05 | A copy. This is the common case for 48 kHz assets without pitch randomisation |
| Linear resample | ~0.6 | ~5 cycles per frame × 480 |
| Cubic (4-point Hermite) resample | ~1.5 | ~12 cycles per frame × 480 |
| **Master SRC to the device rate** (per mix, not per voice): 32-tap polyphase windowed sinc, stereo | 1–2 at 44.1 kHz; 2–4 at 96 kHz; 4–8 at 192 kHz | Device frames per 10 ms × 2 channels × 32 MACs over 8 lanes, plus phase bookkeeping. Zero at 48 kHz (F5) |
| One-pole LP/HP, scalar | ~0.5–0.8 | The serial chain y ← y + a(x − y), 6–7 cycles per sample, two channels interleaved |
| One-pole, batched 8 voices per register | ~0.06–0.1 | The same chain shared by 8 voices |
| Biquad (DF2T), scalar | ~1.0–1.5 | ~10–12 cycles per sample of serial dependency |
| Biquad, batched ×8 | ~0.12–0.2 | Shared chain |
| Coefficient updates with in-house transcendentals (F10) | < 0.05 | Per block or per parameter change, never per sample: a few polynomial evaluations of ~20 ops |
| IMA ADPCM stereo decode | ~1.5–3 | ~12 cycles per sample of serial dependency per channel; two channels overlap or not |
| QOA stereo decode | ~2.5–5 | 4-tap LMS predict, dequantize, clamp, sign-sign update: ~15–20 cycles per sample per channel |
| Vorbis stereo decode | ~5–30 | Low confidence. The only anchor is "1.5–3× ADPCM" [S31]. MQ-A7 measures it if Q4 funds it |
| Opus 128 kb/s stereo decode | ~20–28 | 11 MHz [2 S38] × 10 ms = 110 k cycles ÷ 4 GHz = 27.5 µs; Zen 3's higher IPC lowers it |
| HRTF per source, direct 256-tap FIR per ear | ~4–8 | 480 × 256 × 2 = 246 k MACs at 16 MAC per cycle (FMA) or 8 (no FMA) |
| HRTF per source, partitioned FFT with crossfade | ~3–6 | Two 512-point frames per ear per 10 ms |
| Ambisonic encode, 3rd order (16 ch) | ~0.2–1 | 16 MACs per sample plus gain ramps |
| Ambisonic binaural decode, 3rd order (per mix, not per voice) | ~20–80 | 16 × 2 FIRs of 256 taps, or FFT convolution |
| Ambisonic encode, 1st order (4 ch) | ~0.05 | 4 MACs per sample |
| Ambisonic binaural decode, 1st order (per mix) | ~5–20 | 4 × 2 FIRs |
| FDN reverb, 16 lines (per instance) | ~2–5 | Per sample: 16 delay taps, a 16-point fast Hadamard (64 add/sub), 16 one-pole dampers, over 2 AVX2 registers: ~40 cycles × 480. Assumes the taps hit L1 or L2; §3.5 explains why four instances do not fit L2, and MQ-A1 measures that case |
| Per-block fixed overhead (command drain, board stores, setup) | ~2–5 per block | ~1.9 blocks per 10 ms at a 256-frame block (F5) |
| **SDF occlusion ray, x8 kernel (per ray), 16 edits** | **~0.4–0.6** | The compiled body of `sdf_edit_list_x8` is 210 instructions [R, FMA-DETERMINISM-MEASUREMENTS.md]. That is a static count; the per-edit dynamic count is not measured. Assuming ~40–60 instructions per edit per 8 points, 16 edits at ~1.5–2 IPC give ~100 ns per 8-point step, × 32–48 steps ÷ 8 rays |
| **SDF occlusion ray, x8, N edits** | **~0.03–0.04 × N** | Linear in N, because every step evaluates every edit: 16 → 0.4–0.6; 64 → 1.6–2.4; 256 → 6–10. A per-ray edit cull (the ray's capsule against each edit's bound) would make it follow the edits near the ray instead; it is not built (F13) |
| SDF endpoint lift (F13), per ray | ~0.05 | Per voice update: 4 tetrahedral gradient evaluations and a short trace at each end, 8 voices in lockstep, shared by the voice's 4 rays |
| SDF occlusion ray, scalar (per ray, 16 edits) | ~2–5 | 32–48 steps × 16 edits × ~3–6 ns |
| Collider ray through an 8-wide BVH (per ray) | ~0.2–1 | 3–5 levels of `Node8` slab tests plus leaf sphere/box tests; the ray arm does not exist yet (F13) |

### 3.3 RT-thread totals

**Full chain per voice**: pitch-varied cubic resample, batched LPF, pan, 2 sends.
- Resident PCM: about 1.5 + 0.12 + 0.1 + 0.2 ≈ **1.9 µs**.
- Resident ADPCM: add about 2.25, giving **≈ 4.2 µs**.

| Real voices | PCM µs / 10 ms | % of one core | ADPCM µs / 10 ms | % of one core |
|---|---|---|---|---|
| 16 | 31 | 0.3 | 67 | 0.7 |
| 64 (default `MAX_REAL`) | 124 | 1.2 | 267 | 2.7 |
| 96 (default `RT_ROWS`: a flip transient, F11) | 186 | 1.9 | 403 | 4.0 |
| 256 | 496 | 5.0 | 1,070 | 10.7 |
| 1024 | 1,980 | 19.8 | 4,300 | 43 |

**A typical action scene** [E]:
- 64 ADPCM voices: 267 µs;
- 4 QOA streams, mix only (decoding happens on the IO thread): ~1 µs;
- 2 FDN reverbs: ~10 µs;
- master soft clip and output conversion: ~2 µs;
- fixed per-block overhead: ~8 µs.

The total is **≈ 290 µs per 10 ms, 2.9% of one core**. Adding a 3rd-order ambisonic bus with a
binaural decode costs 64 × 0.6 + 50 ≈ 88 µs more, **≈ 3.8%**. Doubling for SMT contention still
leaves 6–8% of one core. That margin is what makes a single RT thread sufficient (F4).

**Transients.** A burst of real/virtual flips keeps up to `FADE_HEADROOM` (default 32) extra rows
fading for at most two frames (F7, F11). The worst block then processes all 96 rows: ≈ 403 µs of
ADPCM voices plus the rest of the scene, **≈ 4.2%**. The steady figure above is unchanged.

**Device rate.** The mix runs at 48 kHz whatever the device rate (F5). A 96 or 192 kHz endpoint adds
only the master SRC, 2–8 µs. Mixing at the device rate would have multiplied the whole scene by 2 to
4, to about 580–1,160 µs.

**Which decisions depend on these numbers.**
- The single-thread decision holds unless real voices exceed about 1000 at full DSP.
- The `MAX_REAL = 64` default holds unless the M0 benches come in more than 2× above the table.

### 3.4 ECS-side (pool) totals, per frame

| System | Work [E] | CPU µs | Wall µs, 16 workers |
|---|---|---|---|
| `audio_drain` | Reads the return board: 96 row words, ~40 counters, a 32-bucket histogram, the clock; retires clips | ~2 | ~2 (serial) |
| `audio_intake` | Events plus emitter transitions, typically < 100; clock mapping (F16) | ~5 | ~5 |
| `audio_spatialize` | 1024 emitters × ~50 ns: listener transform, curve evaluation, pan, Doppler | ~50 | ~5 |
| `audio_select` | 4096 slots × ~40 ns: score plus top-K with hysteresis and the flip budget | ~160 | ~15 |
| `audio_publish` | ≤ 160 records × 64 B | ~2 | ~2 (serial) |
| Occlusion arms | 64 voices × 4 rays at 30 Hz round-robin ≈ 128 rays per frame at 60 fps × 0.55 µs (16 edits, with the lift) | ~70 | ~5 |
| **Total** | | **~290–360** | **~35–40** |

At 256 edits the occlusion row alone becomes ~0.8–1.3 ms of pool CPU per frame (F13).

### 3.5 Memory

| Item | Size [E] | Notes |
|---|---|---|
| RT working set (`AudioRtStorage`, itemised in F7) | ≈ 3.9 MiB | Committed and touched at boot; never grows (F7) |
| – of which touched every block | voice rows 24 KiB, board ~1.3 KiB, ring 40 KiB, FIFO 10 KiB, buses 64 KiB | L1/L2-resident |
| – of which FDN delay lines | 16 lines × ≤ 100 ms × 48 kHz × 4 B ≈ 300 KiB per instance; 4 instances ≈ 1.2 MiB | **Larger than the 512 KiB L2** [R] and sharing L3 with physics. Each line is two sequential streams (read and write), so the prefetchers see 32 streams per instance. §3.2's per-sample cost assumes cache-resident taps; MQ-A1 measures four interleaved instances with a cold L2 |
| – of which stream page rings and IO read buffers | 1.5 MiB + 1 MiB | 8 streams, 2 pages of 0.5 s each, decoded to i16 |
| ECS slot table | 4096 × ~96 B ≈ 384 KiB | KC-10 columns, SoA |
| HRTF dataset | 0.23 MiB (vendor figure for a compressed set [V S48]) to about 2 MiB | M6 |
| **Engine total, excluding content** | **≈ 4.5–6.5 MiB** | |
| Content: 500 SFX × 1.5 s stereo | PCM16 144 MB · ADPCM 36 MB · QOA 29 MB of content bytes | Mono halves each figure |
| `SampleBank` commit for that content | ≈ 1.39× content expected, ≤ 2× worst: ADPCM ≈ 50 MB expected, ≤ 72 MB | Power-of-two span classes (F8); MQ-A9 measures real content |
| Content: 60 min of music on disk | PCM16 691 MB · ADPCM 173 MB · QOA 138 MB · Vorbis/Opus at 128 kb/s 58 MB | The derivation behind Q4 |

**GPU:** 0 ms and 0 MB for the recommended architecture.

---

## 4. Forks and decisions

### F1 — Build or buy

| Option | R3 (P0) | R4 (hot path) | R5 (deps) | Linux | Numbers | Verdict |
|---|---|---|---|---|---|---|
| A. Wwise / FMOD | Voice and emitter state live in the middleware's own structures: the parallel-data-system failure P0 names | Vendor threads and allocator hooks | Closed SDK and licence | Yes | Vendor-tuned, unpublished | Rejected |
| B. Kira or Firewheel + cpal + Symphonia | Firewheel/seedling diff components into its own graph, which is still a second store | Custom nodes as trait objects (not verified); Firewheel's readiness notes mention "nontrivial UB" [V S45] | Several crates and their transitive dependencies | Yes (cpal) | Unpublished | Rejected |
| C. miniaudio | Its own node graph and resource manager | Spinlocked job queue [V S12] | One C file, needs a C build | Yes | Unpublished | Rejected |
| D. **In-house** over WASAPI and ALSA | Designed to fit (F7) | Designed to fit (§9 gates) | None beyond `windows-sys` | Via F3 | §3 | **Chosen** |
| E. XAudio2 as the mixer | Its own voices | Critical sections; `DestroyVoice` can block [V S5] | OS component | No | OS-tuned | Rejected |

**Why D.**
- Every buy option keeps authoritative audio state outside ECS storage.
- The parts that decide quality are integration parts, and they must be written in any case:
  intake, selection, spatial parameters, the transport and the gates.
- The remaining DSP core is small [E]:
  - device FFI: about 1.5 k lines;
  - RT loop and mixer: about 2 k lines;
  - decoders for WAV, IMA ADPCM and QOA: about 1 k lines (QOA's reference is about 400 lines of C
    [V S35]).
- The precedent is the tree's own: raw-FFI Vulkan and in-house PNG. Firefox's cubeb is a shipped
  example of the same hand-declared `IAudioClient3` surface over raw WASAPI [V S13].

The architecture patterns are borrowed with attribution in §5.

### F2 — Windows output backend

| Option | Latency | Other apps | Power | Rule fit | Verdict |
|---|---|---|---|---|---|
| WASAPI shared, event-driven, default period | 10 ms period + 1.3 ms engine [V S1] | Unaffected | Baseline | ✓ | **Default** |
| WASAPI shared, `IAudioClient3` low period | Down to 128 frames (2.67 ms) on the inbox driver [V S1] | **Switched to the same small period** while our stream runs [V S1]; can fail with `ENGINE_PERIODICITY_LOCKED` if another client holds a period [V S3] | Higher, for the whole endpoint (Microsoft FAQ) [V S1] | Host-owned: it changes state beyond the process | **Host option**, `LatencyClass::Low`, per Q5 |
| WASAPI exclusive | One device period, ping-pong buffers [V S2] | **Silenced** | — | ✓ | Not in v1; Microsoft advises low-period shared first [V S1] |
| XAudio2 | OS-tuned | — | — | Blocking API [V S5] | Rejected |
| `ISpatialAudioClient` objects | Its own | — | — | ✓ | Optional output mode, M6, per Q1 |

**How it runs.**
- The RT thread is the WASAPI loop. `CoInitializeEx` (MTA), device enumeration, and `Initialize` or
  `InitializeSharedAudioStream` all run under the default MXCSR (F19).
- **Each period.** The thread waits on the event with a **bounded timeout**: four device periods,
  and at least 20 ms [E]. Microsoft's own event-driven sample bounds its wait at 2 s and treats a
  timeout as an error [V S2]. It then reads `GetCurrentPadding`, fills from the block FIFO up to the
  target padding, and calls `ReleaseBuffer`.
- **Target padding: one device period plus one block.** Filling all free buffer space would queue
  about two periods. MQ-A6 records `GetBufferSize`, `GetStreamLatency` and the padding at each wake,
  and the FIFO-underrun counter decides whether the margin must grow.
- **Default-device change** arrives through `IMMNotificationClient` on an OS thread. That callback
  only sets an atomic flag; the RT thread re-initialises on its next wake-up.
- **Device loss, with or without a replacement.** `AUDCLNT_E_DEVICE_INVALIDATED` from any call, or a
  wait timeout followed by a failing `GetCurrentPadding`, switches the loop to the **wall-clock
  pump**. That is the same backend a boot failure uses: it renders and discards blocks paced by
  QPC.
  - Voices keep advancing, one-shots end, their rows are acknowledged, and `SoundFinished` fires on
    time. Gameplay that waits on it cannot hang on a missing endpoint.
  - Microsoft's recovery recipe is to release the client and activate one on the current default
    endpoint [V S65]. The RT thread tries that at most every 500 ms [E], and at once when the
    notification flag is set.
  - An attempt that blocks delays the pump. The pump then catches up by rendering the due blocks
    back to back: a 100 ms attempt costs about 19 blocks, about 3 ms of CPU [E].
  - When a device returns, the loop re-initialises and the RT working set is rebuilt (F7). The ECS
    side learns every transition from the board's device word.
- **Boot failure** degrades to `ResolvedAudio::Silent` with the wall-clock pump, so gameplay behaves
  identically. It never panics, following the host's rule for capability probes such as SSAA arming
  in `EnginePlugins`.

**Latency, event to ear, at 60 fps** [E]:
- up to one frame, until the frame's batch is published (≤ 16.7 ms);
- plus up to one device period, until the RT thread wakes and reads it (≤ 10 ms, or 2.67 ms in the
  low class). Rev 1 counted one block here, but the block is the render granularity; the thread
  wakes once per device period;
- plus the FIFO residue: frames already rendered but not yet copied (≤ one block, 5.33 ms);
- plus the padding still queued at the wake: about one block at the target padding;
- plus the stream latency: 1.3 ms of engine latency [V S1], until MQ-A6 reads `GetStreamLatency`.

That is **≈ 39 ms worst case by default and ≈ 31 ms in the low class**.
- `Timing::Immediate` sounds average about 23 ms by default: half the frame, wake and residue terms.
- `Timing::Exact` sounds pay a constant ≈ 44 ms by default and ≈ 36 ms in the low class, in exchange
  for sample-exact spacing (F16).

### F3 — Linux backend

| Option | Reaches | API surface | Verdict |
|---|---|---|---|
| **ALSA `libasound.so.2`, loaded at runtime** (`dlopen`/`dlsym`) | Pure ALSA, PipeWire (pipewire-alsa), PulseAudio (its plugin) | ~15 `snd_pcm_*` functions | **Chosen**. It mirrors `crates/boyko_rhi_vulkan/src/device.rs`, which loads `vulkan-1.dll` through `LoadLibraryA` + `GetProcAddress` |
| Native PipeWire (`libpipewire`) | PipeWire only | Large: loops, streams, SPA pods | Deferred until a measurement shows the ALSA plugin adds more than one server quantum |
| PulseAudio simple API | Pulse and PipeWire-pulse | Small | Adds a server hop for no gain over ALSA |
| JACK | Pro setups | Medium | Out of scope |

Latency on plugin systems follows the server quantum: 21.3 ms by default on PipeWire [V S8]. ALSA
periods do not set it [V S10]. The pipewire-alsa plugin starts its own loop thread inside
`snd_pcm_open` [V S66], which is why the open runs under the default MXCSR (F19). The backend does
not need a window, so it can ship before the Linux host exists. Its timing is Q6.

### F4 — Thread topology

| Option | Numbers | Problem | Verdict |
|---|---|---|---|
| **A. One RT thread (MMCSS "Pro Audio"), which is also the device loop; all DSP on it; resident decode inline; streams decoded ahead on a dedicated IO thread** | RT duty 1.2–4.2% of one core (§3.3) | Preempts one pool worker for ≈ 0.1–0.4 ms per 5.33 ms block | **Chosen** |
| B. The RT thread fans DSP out to pool workers each block | Only pays above ~50% of a core, i.e. more than ~1000 full-DSP voices | The RT thread waits on normal-priority workers that a physics step saturates, which is priority inversion (research pitfall 2). The pool's park backstops are 50–100 µs and its timed waits are quantised (`crates/boyko_app/src/timer_resolution.rs`) | Rejected |
| C. Mix on the pool as an ECS system every frame | Must render at least one frame ahead: +16.7 ms at 60 fps, +33 ms at 30 fps | Any frame hitch (load, pipeline compile) becomes an audible gap, because audio is continuous and frames are not | Rejected |
| D. Unreal's three threads | — | Its "audio thread" role is exactly ECS systems on the scheduler here; its render thread is option A | Merged into A |

**MMCSS task and thread census.**
- The task is `"Pro Audio"` (High, priority 23–26 [V S4]). The thread uses at most about 5% of a
  core, so boosting it takes little from the pool. What the boost buys is no underruns while 16
  normal-priority workers run a physics step.
- On the reference machine: 16 workers + main + RT + IO = 19 threads on 16 logical CPUs. The IO
  thread is mostly blocked. On Linux the ALSA plugin adds its own loop thread, which is not ours
  (F19).
- The pool may lose up to one audio block of CPU on one worker. The perf gate in §9 (M0) measures
  the physics step with audio on and off.

### F5 — Block size, sample rate, format

| Internal block | Fixed overhead per second [E] | Added FIFO latency | SIMD trip count per channel |
|---|---|---|---|
| 128 frames | 0.75–1.9 ms/s (0.08–0.19%) | ≤ 2.67 ms | 16 |
| **256 frames** | 0.37–0.94 ms/s (0.04–0.09%) | ≤ 5.33 ms | 32 |
| 480 (= the default device period) | — | 0 at the default period only | Varies with the device |
| 1024 | ~0.1–0.2 ms/s | ≤ 21.3 ms | 128 |

**Decision.** A fixed internal block of **256 frames** at a fixed internal rate of **48 kHz**,
rendered into a small FIFO that adapts to any device period from 128 to 1024 internal frames
(2.67–21.3 ms at any device rate).
- **Why fixed.** Rendered output depends only on the command stream and on the block at which each
  batch is applied, never on the device period. The offline pump fixes that batch → block
  assignment, so an offline golden is reproducible, and SIMD loops have constant trip counts. A live
  render equals the offline one only when the pump replays the live assignment. The RT loop keeps
  that log (batch sequence → block index) in debug captures, for M9's capture tool.
- **Rate policy.** The mixer always runs at 48 kHz.
  - A 48 kHz device (`GetMixFormat`) needs no conversion.
  - Any other rate (44.1, 96 or 192 kHz) gets one in-house polyphase windowed-sinc SRC on the master
    output [V S58], 1–8 µs per 10 ms (§3.2). A 192 kHz endpoint's default 10 ms period is 1,920
    device frames = 480 internal frames, inside the FIFO range.
  - Rejected: mixing at the device rate. It multiplies every per-sample cost by 2–4 (§3.3), and the
    goldens would multiply per rate.
  - Rejected: the engine's own converter (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` [V S64]). It costs
    nothing on our thread, but it is Windows-only, invisible to the offline goldens, and of unstated
    quality. It stays the fallback if MQ-A1 prices the in-house SRC above 1% of a core.
  - 44.1 kHz assets are absorbed by the per-voice resampler, so a 44.1 kHz asset on a 44.1 kHz
    device is resampled twice. That is the price of one rate, and it is audible only in the
    resampler's own quality, which M1 gates.
- **DSP coefficients** are computed once at boot for 48 kHz, with the deterministic math of F10.
- **Offline goldens** hash the 48 kHz master before the SRC. The SRC has its own golden (M0).
- **Format.** f32 internally. Stereo in v1, up to 8 channels at M6.

### F6 — ECS ↔ RT transport

| Option | Traffic per frame [E] | Problem | Verdict |
|---|---|---|---|
| a. Command ring (Kira, Unreal, Unity [V S41][V S20][V S25]) | ≤ one record per RT row + control records: (96 + 64) × 64 B = 10 KiB | None, if records are grouped per frame | **Chosen** for ECS → RT, with (b) as its source |
| b. Component diff → events (Firewheel/seedling [V S43][V S44]) | Same records | A diff library is unnecessary: the kernel already has change ticks (`Changed<T>`, `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs`) | Its idea is kept, its mechanism replaced |
| c. Per-frame snapshot of the whole voice state (double or triple buffer) | Copies unchanged rows too | Needs a third buffer or a handshake | Rejected for ECS → RT; **the RT → ECS direction is a board of exactly this kind, made lossless by being level-triggered** (below) |
| d. Kernel event lanes / `OsEventSink` (KC-26) | — | `OsEventSink` is dispatcher-only and not `Send` [R] | Impossible for the RT side |
| e. One atomic per parameter | 1 atomic per field | Tears multi-field updates (position + gain); an RMW per field | Rejected |

**Decision, ECS → RT.**
- **Records.** Fixed 64-byte, one-cache-line, `#[repr(C)]` records: kind, row, start sequence, target
  sample time, and a payload of gains, pan, pitch, filter and sends.
- **Producer.** Exactly one system per frame writes them (`audio_publish`, §6.5) and publishes the
  batch with **one Release store**. That is Firewheel's grouped flush [V S43], so the records of one
  frame cannot straddle blocks.
- **Record bound.** Each RT row receives at most one record per frame: a start (which carries its
  parameters), a stop, or a parameter update. A stop supersedes a parameter update in the diff. A
  row is reused only after the ECS side has read its release, which is never in the frame that
  stopped it. A real/virtual flip therefore costs two records on two rows. The bound is
  `RT_ROWS + MAX_CTL` records per frame (`MAX_CTL`, default 64, covers bus and snapshot records),
  and the ring holds 4 frames of it: 4 × 160 × 64 B = 40 KiB.
- **No command is ever lost.**
  - Virtual voices never reach the RT thread (F11), so the record count per frame is bounded as
    above.
  - The RT thread can still stop draining, for example during a device reset. So publishing is a diff
    between each slot's desired state and its last-published state.
  - When a whole batch does not fit, the producer publishes nothing that frame and leaves the
    last-published state unchanged. The next frame's diff then re-derives every pending record from
    the authoritative slot table. The producer counts these deferrals.
  - An `Exact` start whose target sample has passed follows its sound's late policy (F16).
  - The M0 gate proves that the deferral counter stays zero in steady state, and that a forced stall
    loses nothing (§9, M0).
- **Ring shape.** `LogLane`'s: the opposite cursor cached, three padded partitions
  (`crates/boyko_log/src/lane.rs`). It becomes a kernel primitive (K-A2) rather than a second copy,
  per principle 0's "a capability a subsystem needs is made a first-class kernel feature".

**Decision, RT → ECS: a return board, not a ring.** Rev 1 sent voice-ended acknowledgements, block
statistics and conditions through a 32 KiB ring with no overflow policy. At one statistics record per
block, 512 records fill in about 2.7 s of stalled consumer, and a synchronous `AssetServer::load`,
a loading screen or a window drag stalls the main thread that long. A dropped voice-ended record then
leaks a real-voice row and a clip, and `SoundFinished` never fires. The fix is to report only state,
which the RT thread overwrites in place:

| Word | Writer → reader | Semantics |
|---|---|---|
| **Row acknowledgement**, one `u64` per RT row: (start sequence, state ∈ {Playing, Ended, Released}) | RT (Release store) → `audio_drain` (Acquire load) | Level-triggered. The ECS side allocates rows, and reuses a row only after reading `Released` for its latest start. So no acknowledgement is overwritten before it is read. "Voice ended", "virtualization acknowledged" and "the clip's last read" are all this one word |
| **Monotonic counters** (`u64`): per-stream underflows, FIFO underruns, device resets, late starts, skipped starts, pump blocks | RT → drain | Read as deltas since the last read. A wrap needs 2^64 events |
| **Block-time histogram**: 32 log-spaced buckets (monotonic `u32` counts) plus a maximum | RT → drain | A stall loses no sample; p50 and p99 are computed on the ECS side from bucket deltas |
| **Clock**: (sample position, QPC) under a sequence word | RT → drain | A seqlock with one writer per wake. The reader retries on a torn read; the writer never waits |
| **Device word**: (generation, state, granted rate, period) | RT → drain | `AudioDeviceChanged` fires on a generation change |

- **Storage.** One KC-03 column of `u64` words read through `AtomicU64::from_ptr` views (KF-42),
  fixed at boot: 96 × 8 + ~40 × 8 + 32 × 4 + ~64 ≈ 1.3 KiB [E], part of the RT working set (F7).
- **There is no overflow case.** Nothing lifecycle-relevant is edge-triggered, so there is no drop
  policy to state. No diagnostics text is carried: `audio_drain` logs counter deltas (F17).
- **Gate (M1).** The consumer stalls for 2,000 blocks (about 10.7 s of audio, four times what a
  512-record queue holds at one record per block) while every one-shot ends. On resume, every slot
  and every clip is retired, `SoundFinished` has fired exactly once per sound, and all rows are
  free. Red control: a planted bounded return queue, the rev-1 design, leaks a row and a clip.

### F7 — Principle 0: where voice state lives

The research left this as an owner question. It is technical, so it is decided here.

| Option | Cost | Problem |
|---|---|---|
| i. World columns read by the RT thread concurrently | — | The scheduler's access model does not cover a thread outside it; reading would need a per-block handshake, and so a wait or a lock |
| ii. RT-private `Vec` or statics | Same bytes | A side store; principle 0 forbids it by name |
| iii. **Kernel columns built by the plugin at boot, filled to capacity, owned by a plugin resource (`AudioRtStorage`) for the session; the RT and IO threads receive fixed-length views** | Same bytes | None. See below |
| iv. All voice state as dense components that the RT thread reads through `row_ptr` views | — | The same concurrency problem as (i) |

**Decision: (iii).**
- **Durable state stays in the World.** What should sound, where, how loud and under which priority
  lives in components and resources.
- **The RT working set is World-resident too.** Its owning columns live in the `AudioRtStorage`
  resource. The RT thread holds only views: a base pointer from `ScratchColumn::solve_base`, a
  length, and indexed row access. No view type has a push, so the RT thread has no path that grows
  memory. The rows hold only derived state: resampler phase and history, filter memories, ramp
  states, decoder cursors. No query can see them, because they are not components.
- **Sized, committed and touched at boot.** Every column the RT thread touches has a fixed capacity
  derived from `AudioConfig`. The boot path fills it to that capacity with zeroed rows (`Reserve::Now`
  plus KC-03's `ensure_len_zeroed`), so every page is committed and resident before the first
  block. After boot, no column in `AudioRtStorage` changes length. The default sizes:

  | Column (in `AudioRtStorage`) | Rows × stride | Default size |
  |---|---|---|
  | Voice DSP rows | `RT_ROWS` (96) × 256 B | 24 KiB |
  | Return board (F6) | ~170 words | ~1.3 KiB |
  | Command ring (the RT thread consumes) | 4 × 160 × 64 B | 40 KiB |
  | Block FIFO | 5 blocks × 256 frames × 2 ch × 4 B | 10 KiB (8 channels at M6: 40 KiB) |
  | Bus buffers | 32 × 256 × 2 ch × 4 B | 64 KiB (+16 KiB for the ambisonic bus at M6) |
  | Master SRC state | history plus phase table | < 8 KiB |
  | FDN delay lines (M4) | 4 instances × 16 lines × ≤ 4,800 × 4 B | 1.2 MiB, sized by the configured instance count |
  | Stream page rings (the IO thread writes, the RT thread reads) | 8 × 2 pages × 0.5 s × 2 ch × 2 B | 1.5 MiB |
  | IO read buffers (IO thread only) | 8 × 2 × 64 KiB | 1 MiB |
  | **Total** | | **≈ 3.9 MiB, committed and touched at boot** |

- **Why a heap counter is not enough.** The tree's G2 census states its own boundary. A
  `#[global_allocator]` sees the Rust heap only, and "a column that committed one more granule every
  frame would read 0" (`crates/boyko_physics/tests/alloc_frame_census.rs`, section "Coverage
  boundary") [R]. `ComponentPool` grows in place by committing at the frontier, on the pushing
  thread (`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs`, the `ScratchColumn`
  doc) [R]. A KC-10 `Default` column reserves only at its first push [R]. So M0's gate has three
  legs: the heap deny, a **thread-scoped commit deny** (K-A7), and a "committed bytes do not move"
  check (§9).
- **Page faults after boot are not closed by construction.** Windows can trim a resident page from
  the working set. Locking pages with `VirtualLock` is capped by the process's minimum working set,
  which only `SetProcessWorkingSetSize` raises [V S68]. That is a process-wide setting, and so the
  host's under the timer-resolution rule; [PERF-DIRECTIONS.md](../PERF-DIRECTIONS.md) already lists
  page-locking as an opt-in lever to take only on measurement. v1 does not lock. MQ-A8 counts
  RT-thread page faults: per thread on Linux through `getrusage(RUSAGE_THREAD)` [V S67], and
  process-wide on Windows during the soak. A nonzero count makes locking a host option.
- **`RT_ROWS`.** `RT_ROWS = MAX_REAL + FADE_HEADROOM`, with `FADE_HEADROOM = 2 × FLIP_BUDGET`
  (default 16, so 96 rows). A demoted voice keeps its row for its fade (≤ 10 ms), plus up to one
  frame until `audio_drain` reads its release: at most two frames at 60 fps [E]. `audio_select` flips
  at most `FLIP_BUDGET` voices per frame (F11). A promotion that finds no free row waits one frame,
  and is counted as a start slip.
- **It is disposable, like a GPU mirror.** After a device reset the RT state is rebuilt from ECS
  state. That rebuild is a gate (§9, M1). The same shape exists for GPU data: device-resident
  columns are a kernel concept (`DeviceColumnHandle`, `crates/boyko_ecs/src/ecs/memory/device_column.rs`),
  and GPU instance data are dense components mirrored through staging.

#### F7.1 — Cross-thread access: the SAFETY contract

KC-03 and KC-15 state a single writer under `&mut`. Audio adds readers on other threads, through
raw views. These are the invariants that each `unsafe` block's `// SAFETY:` comment must state:

1. **Provenance.** Every view's base pointer comes from the column's raw base (the VM reservation),
   never from a reference to element memory. Retagging `&mut AudioRtStorage` or `&mut SampleBank`
   covers those structs' own fields, not the memory they point to.
2. **No whole-column references.** No code path forms a `&[T]` or `&mut [T]` over a column that
   another thread reads or writes. Each thread materialises references only to the rows or spans it
   owns by protocol.
3. **Ownership by protocol.**
   - Voice DSP rows, bus buffers and the FIFO belong to the RT thread only.
   - The forward ring follows `LogLane`'s cursor protocol.
   - Board words are atomics (F6).
   - Page rings carry one state word per page. The IO thread fills a page and stores its state with
     Release; the RT thread consumes it and stores its state with Release; each side reads the
     other's state with Acquire.
   - A `SampleBank` span is written by the loader before its clip header is published. The RT
     thread learns of a span only from a start record, and the batch's Release store orders the bytes
     before the RT thread's Acquire. A span is freed only after the board shows `Released` for every
     row that played it; the RT thread's Release store after its last read orders that read before
     any reuse.
4. **No growth under readers.** `AudioRtStorage` columns never grow after boot. `SampleBank` grows
   only at its frontier, on the loader thread, into pages no reader holds.
5. **Teardown.** Columns are dropped only after the RT and IO threads are joined.

**Coverage.** `crates/boyko_ecs/src/ecs/memory/vm.rs` has a Miri arm: under `cfg(miri)` the
reservation falls back to the standard allocator [R]. So a Miri test under Tree Borrows can run the
`SampleBank` cycle on two threads: load and publish, read, acknowledge, free, reuse. Its receipt
records the full `MIRIFLAGS`. The board and page-ring protocols get loom models, which is the tree's
existing practice for atomics. The G3 deny allocator is `cfg(not(miri))`
([memory/ALLOCATOR-DESIGN-SPACE.md](../memory/ALLOCATOR-DESIGN-SPACE.md)), so Miri covers the
aliasing contract and not the deny gate.

### F8 — Sample data: storage and lifetime

| Option | Problem | Verdict |
|---|---|---|
| i. PCM inside the `Assets<T>` row | Fixed stride, so impossible | — |
| ii. `Assets<AudioClip>` header rows plus one KC-03 `ByteColumn` holding every clip's bytes | KC-03's API is a push/pop stack. A clip freed in the middle cannot be reused, so the column commits the sum of every clip ever loaded. With level streaming it grows by tens of MB per level set and never shrinks | Rejected. It was rev 1's choice |
| iii. One heap `Vec<u8>` per clip | A side store on the general allocator | Rejected |
| **iv. `Assets<AudioClip>` header rows plus one KC-15 `SegmentedColumn<u8>`, one write-once span per clip** | Power-of-two slack (below) | **Chosen** |
| v. A new non-relocating range allocator (best fit over a byte column) | A new kernel primitive for one consumer; best-fit fragmentation depends on the workload | Held. It is filed only if MQ-A9 prices KC-15's slack above budget |

**Why KC-15 after all.** Rev 1 rejected it because it "relocates on growth". ED15 relocates a span
only when that span grows past its class, and the base never moves, so "a `SpanRef` stays valid until
its own span relocates or is freed" ([ENGINE-RUNTIME-ECS-DESIGN.md](../unification/ENGINE-RUNTIME-ECS-DESIGN.md),
ED15). A clip span is written once at load and never grows, so it never moves.

**Its cost.**
- A span of n bytes occupies a block of 2^⌈log2 n⌉ bytes, so the slack is at most 2× per clip.
- For clip sizes spread evenly in log-size, the expected fill is 1 ÷ (2 ln 2) ≈ 72%. That is about
  1.39× committed per content byte [E]. The §3.5 example (500 SFX, 36 MB of ADPCM) commits about
  50 MB expected and 72 MB at worst.
- Freed blocks return to their class's LIFO free list. Committed memory is therefore bounded by the
  peak per class, not by the sum ever loaded.
- Finer classes, four per octave, would bound the slack at 1.19× (≈ 1.09× expected [E]). ED15 adds a
  finer policy only when "a measurement asks for it", so this is K-A6's optional part, requested
  with MQ-A9's number.

**Lifetime.**
- `Assets<T>` already refcounts, retires and generation-checks rows (`inc_ref`, `dec_ref`,
  `RetireTicket` in `crates/boyko_ecs/src/ecs/core/asset/assets.rs`).
- A clip's references are the carrier hooks on `AudioEmitter` plus the live slots that play it.
- A clip retires when its refcount is zero **and** the board shows `Released` for the latest start of
  every row that played it. The RT thread writes `Released` after the fade-out, as a Release store
  after its last read, so no later block can touch the bytes (F6, F7.1).
- The span then returns through `SegmentedColumn::free` on the ECS side. Pending retires are a KC-10
  column of (clip, span, outstanding row count) that `audio_drain` scans. No epoch is needed,
  because the acknowledgement is exact. KC-16 is not used.
- The RT thread never frees anything; it only stops reading. So no basedrop-style collector is
  needed [V S16].

### F9 — Codecs and streaming

| Format | Role | Decode µs per voice per 10 ms [E] | Size | R5 |
|---|---|---|---|---|
| PCM16 WAV | Short resident SFX, UI | 0 (conversion only) | 11.5 MB/min stereo | In-house RIFF parser, ~100 lines |
| **IMA ADPCM** | **Default resident compressed format** | 1.5–3 | 2.9 MB/min | In-house, ~150 lines |
| **QOA** | **Streams: music, ambience, dialogue** | 2.5–5 (on the IO thread) | 2.3 MB/min | In-house, ~400 lines of reference C [V S35] |
| Vorbis | Streams, if Q4 funds it | 5–30 | ~1 MB/min at 128 kb/s | In-house is a large decoder; otherwise a rule-5 exception |
| Opus | Voice chat (Q9) | 20–28 | ~1 MB/min | Not practical in-house (SILK + CELT + hybrid, RFC 6716 [S40]); rule-5 exception only |
| FLAC | Lossless masters | Cheap [V S36] | Large | Not needed at runtime |

**Encoders.** The WAV → IMA ADPCM and WAV → QOA encoders are bake-time tools, and they are in-house:
the IMA encoder runs the decoder's step table forwards, and the QOA encoder is ported from the
format's reference implementation [V S35]. The reference C implementations are used once, offline, to
generate the decoders' test vectors (M0-leaf). That is the same bake-time-only use as the DXC recipe
and the SOFA converter, not a build dependency.

**Streaming.**
- A dedicated IO thread, `boyko-audio-io`, at normal priority and not MMCSS, reads file pages and
  decodes them into per-stream page rings: 2 pages of 0.5 s each, i16, in columns of
  `AudioRtStorage` (F7).
- The RT thread consumes pages and never waits. An underflow outputs silence and increments a
  counter on the board.

**Why a dedicated thread and not pool jobs.**
- Pool workers must not block on file IO, and the pool has no async IO.
- The pool has no priority lanes, so decode-ahead would starve behind a physics step.
- Decode is cheap: one second of QOA stereo is ≈ 0.25–0.5 ms. One thread serves dozens of streams.
- When the asset system grows a kernel IO thread (K-A5), this thread folds into it.

**Virtual voices and streams.** A stream is never virtualised: it plays or it stops. A seek on
resume would restart IO, and Opus-class codecs need 80 ms of pre-roll [V S38].

### F10 — Mixer and DSP graph shape

| Option | Dispatch | Verdict |
|---|---|---|
| a. Dynamic node graph of trait objects (Kira/Firewheel style) | `dyn` per node per block | Rejected by R4 |
| b. **A bus tree declared as data, validated and flattened at boot into a static order; effects as a closed enum** | One `match` per effect per block (ns), never per sample | **Chosen**: the MetaSounds lesson [V S21] without code generation |
| c. A graph monomorphised per game | Zero | Needs a code generator per game; revisit if (b)'s dispatch ever measures above 1% |

**How it runs.**
- **Per block.** Buses run in flattened order: sum inputs, run the effect chain, write sends.
- **Topology changes are cold.** A new pre-flattened schedule is built off the RT thread into a
  second schedule column sized at boot, and swapped in by index at a block boundary. That lands at
  M4; v1 topology is fixed at boot.
- **Snapshots change parameters only.**
- **Buses are `world` or `ui`** (F16): world buses pause with the virtual clock.

**v1 effect enum:**
- gain and pan;
- one-pole LP/HP;
- biquad (LP, HP, BP, shelf, peak);
- envelope follower, used as a sidechain source;
- compressor and ducker;
- delay;
- 16-line FDN reverb;
- soft-clip limiter.

Convolution is a later arm (M8).

**SIMD layout.**
- Real voices are processed in groups of 8. Filter and ramp state is SoA across the 8 voices: one
  AVX2 register per state variable, so serial filter chains are shared (§3.2: about 8× cheaper).
- Sample fetch and resampling are vectorised along time within a voice.
- Bus buffers are planar f32, 64-byte aligned, one block long.

**`Lane` discipline** (the eDSL principle applied to CPU DSP):
- Each kernel is one generic body over `L: Lane`, instantiated as scalar `f32` (the oracle) and as
  x8 (AVX2, production). The x8 lanes must be `to_bits`-identical to the scalar oracle.
- **Only IEEE basic operations and `sqrt`.** Rust guarantees correct rounding for addition,
  subtraction, multiplication, division and `sqrt`, and documents `sin`, `cos`, `exp` and `ln` as
  non-deterministic: their precision "varies by platform, Rust version, and can even differ within
  the same execution" [V S63]. So the kernels use no FMA, no `rcp`/`rsqrt`, and **no
  `f32::sin/cos/exp/ln/powf`**. A per-file source census enforces the whole list. This is the
  discipline of `crates/boyko_physics/src/sdf_simd.rs`, whose field needs only `sqrt`.
- **In-house transcendentals.** Audio needs sin and cos (biquad coefficients, cone and Doppler
  geometry), exp (one-pole coefficients, time constants), and log2 and exp2 (dB ↔ gain, the
  logarithmic and natural attenuation curves). They are polynomial approximations built from the
  same basic operations: one generic body per function in `boyko_audio::dsp::math`, instantiated
  scalar and x8 like every kernel. The accuracy target is about 1e-6 relative [E], far below
  audibility. They run per block or per parameter change, never per sample (§3.2).
- **The eDSL is not the source for them.** Its f32 oracle forwards `sin` and `cos` to `f32::sin` and
  `f32::cos`, or to the nightly intrinsics (`crates/boyko_shaderdsl/src/cf.rs`, the `f32` backend's
  `sin` and `cos`) [R], so it inherits the same platform variance. A GPU arm that needs this math
  later would add eDSL leaves with the same polynomial bodies (§7).
- **The pan law is the square-root constant-power law**, L = √((1 − x) ÷ 2) and R = √((1 + x) ÷ 2).
  It needs only `sqrt`, and L² + R² = 1 holds within a bound derived from the operation count (M2).
- **Consequence.** Goldens blessed on the MSVC host reproduce bit for bit on windows-gnu and on
  Linux x86-64, and survive toolchain bumps (F18).
- A fused kernel is revisited only for HRTF and convolution (M6/M8), where FMA could save about
  10–20%. That would be an A/B measurement with a tolerance oracle and per-host goldens.

### F11 — Voice management

| Option | Verdict |
|---|---|
| Virtualization on the RT side, as FMOD and Wwise do in their engine update | Rejected: the RT thread would carry the whole voice table and all its bookkeeping |
| **Selection on the ECS side at frame rate, in a parallel system. The RT thread sees only real voices** | **Chosen** |

**Mechanics.**
- **Slots.** `AudioSlots` holds up to `MAX_SLOTS` (default 4096) slots.
- **Priority score.** An authored priority byte plus importance weights, the Overwatch model with
  weights such as "shot at" 0.6 and "damaged" 0.5 [V S51], multiplied by audibility.
  - Audibility is the gain the listener would receive: attenuation × occlusion × bus gain.
  - HDR windowing is not in v1; Overwatch rejected it for readability [V S51].
- **Real set.** The top `MAX_REAL` (default 64, configurable 16–256) are real. Hysteresis stops
  thrashing: a challenger must beat the weakest real voice by a margin for N frames.
- **Flips.** Every real/virtual flip fades over 5–10 ms. The outgoing voice keeps its RT row during
  its fade, and the incoming voice takes a free row from the fade headroom (F7).
- **Flip budget.** `audio_select` changes at most `FLIP_BUDGET` (default 16) real/virtual memberships
  per frame. A new one-shot that enters the real set counts as one. Beyond the budget, a new voice
  starts virtual and is promoted the next frame at its elapsed time: a start slip of one frame,
  counted in `AudioStats`. Fades are never cut to make room.
- **Resume.** A virtual voice advances a time cursor, so it resumes at its elapsed time (the Wwise
  mode [S32]). Resident PCM and ADPCM seek exactly (ADPCM by block plus skip).
- **Concurrency groups** use Unreal's vocabulary (prevent new, stop oldest, stop farthest then
  oldest, stop lowest priority, stop quietest, retrigger time [V S22]) and are enforced at intake.
- **Why `MAX_REAL = 64`.** FMOD suggests 32–128 real software channels [S29], Apex plays 80–100
  [S34], and 64 costs 1.2–2.7% of a core (§3.3).
- **Determinism.** Selection is a function of ECS state. Random picks use a counter-based generator:
  each draw is a hash of (seed, frame tick, a key derived from the event's own fields: sound, follow
  entity, stamped time, position bits). It is not a step of shared RNG state, so the result does not
  depend on the order in which intake visits events, and it stays the same if intake is ever
  parallelised. Two identical events get identical picks, which is harmless. `AudioRng` holds only
  the seed. Replay coverage is therefore possible if Q3 asks for it.

### F12 — Spatialization

| Option | µs per 10 ms at 64 voices [E] | Verdict |
|---|---|---|
| **a. Constant-power stereo pan + attenuation curve + distance/occlusion LPF + Doppler, computed on the ECS side and ramped per block by the RT thread** | Pan and LPF are already in the full chain (§3.3) | **M2 default** |
| b. Per-source HRTF convolution | 190–510 | Rejected as the default |
| **c. Ambisonic bus (1st → 3rd order) + one binaural decode** | 33–144 at 3rd order; ~8–25 at 1st | **M6 headphone mode**. It wins above N = F ÷ (h − a) ≈ 3–40 voices, central 50 ÷ 4.9 ≈ 10 [E], where F is the fixed decode (20–80 µs), h the per-source HRTF cost (3–8 µs) and a the per-voice encode (0.2–1 µs) |
| d. Windows Spatial Sound objects | OS-side; capped at 20 objects on Atmos over HDMI [V S6] | Optional output mode, Q1 |
| e. VBAP over the device channel mask (5.1 / 7.1) [V S60] | ≈ the stereo pan cost × 2–3 | **M6 speaker mode** |

**Details.**
- **The crossover does not decide M6 by itself.** At the default 64 real voices, the ambisonic bus is
  cheaper than per-source HRTF across the whole estimate range, because 64 > 40.
- **Attenuation curves:** inverse-clamped, linear, logarithmic, natural, or a custom curve asset
  [V S23][V S12]. Logarithmic and natural curves use the in-house log2 and exp2 (F10).
- **Doppler.** Pitch = (c − v_listener·d̂) / (c − v_source·d̂), clamped, where d̂ is the unit vector
  from the source to the listener and c = 343 m/s. Velocities come from `GlobalTransform` deltas over
  the fixed step, or from the rigid body when one is present.
- **Propagation delay** (distance ÷ 343 m/s) is an opt-in start offset per sound, oddio's model
  [V S42]. An explosion 340 m away is heard about 1 s later.
- **Multiple listeners.** v1 has one listener. With N listeners, each voice is panned and attenuated
  by its nearest listener, so the cost stays ×1 [E].
- **HRTF data.** SOFA is netCDF-4/HDF5 [V S57]. An in-house HDF5 reader is out of proportion, so an
  offline tool converts SOFA into an engine binary asset. The tool may use a third-party reader at
  bake time only, like the offline, hermetic DXC recipe. The runtime stays dependency-free.

### F13 — Occlusion and obstruction (the hybrid fork)

| Arm | Cost per ray [E] | Latency | Coverage in a boyko scene | Rule fit |
|---|---|---|---|---|
| **a. SDF sphere trace on the CPU through the analytic edit field, 8 rays in lockstep with `sdf_edit_list_x8`** | **0.4–0.6 µs at 16 edits, linear in the edit count** (§3.2); scalar 2–5 | Same frame, on the pool | **The analytic edit list only**: the same first `MAX_SDF_EDITS` edits the renderer marches (both clamp). Mesh only where an MDF CPU copy is kept | ✓. Reuses the bit-exact kernel. The marcher's soft-shadow term gives a **continuous** occlusion value, which removes the single-ray on/off problem (research pitfall 10) without extra rays, under the endpoint rule below |
| **b. Collider ray through SI1's 8-wide BVH** | 0.2–1 µs | Same frame | What carries colliders: sphere and box (`ColliderShape`, `crates/boyko_physics/src/components.rs`). This is Unreal's simple-collision default [V S23] | ✓ once SI1 gains a ray arm (K-A3) |
| c. GPU batch: SDF march or `rayQuery` in a compute pass | ~0.02–0.05 ms of GPU per frame for 4096 rays on an RTX 3060 Laptop [E] | ≥ 2 frames (`FRAMES_IN_FLIGHT = 2`) plus a readback. That is below the 100 ms occlusion smoothing, so it is **not** the discriminator | Full scene, including content the CPU field does not hold (bricks; meshes through `rayQuery`) | ✗ today: no gameplay readback path, no async compute, no timeline semaphores [R]; PARTICLES Q8 still open |
| d. Baked wave acoustics (Triton class) | 15–40 µs per query [V S48] | Lookup | Baked scenes | ✗ R5 (proprietary, cloud bake), and a large in-house effort |
| e. Rooms and portals graph (authored) | Small, cached | Same frame | Authored spaces | ✓, M8 |

**The endpoint rule.** Rev 1's estimator, the minimum of k·d/t along the march, reads a source
resting on geometry as occluded: d goes to 0 near the source while t stays the full distance. A
footstep on a floor 10 m away gives d ≈ 0.01 and t ≈ 10, so k·d/t ≈ 0.008 at k = 8: "behind a
wall". Three changes fix it.
1. **Lift both endpoints.** An endpoint within r_lift of a surface (default 0.5 m for sources and
   0.3 m for the listener [E]) is moved along the field gradient by a short sphere trace of at most
   r_lift. The gradient points into the endpoint's own free space, so a source against a wall stays
   on its side of it. The lift is computed once per voice update and shared by the voice's rays
   (≈ 0.05 µs per ray, §3.2).
2. **A two-sided penumbra.** The term becomes k·d ÷ min(t, D − t), where D is the endpoint distance.
   A surface near either end weighs the same, and the estimate is symmetric in listener and source.
3. **k from a fixture, not by taste.** A corridor 2 m wide and 20 m long has d = 1 m along its axis
   and min(t, D − t) ≤ 10 m, so it reads clear only if k ≥ 10. The default is **k = 16**, a penumbra
   half-angle of about 3.6°.

The residual is stated, not hidden. Over a flat floor, with the source lifted to 0.5 m and the
listener at 1.7 m, the term's minimum is about 2.2k ÷ D at mid-path. A floor source therefore reads
clear up to about 2.2k ≈ 35 m and fades gradually beyond that, to about 0.7 at 50 m [E]. Authored
distance curves already attenuate at those ranges. M5a pins the numbers with fixtures.

**Scaling with the edit count.** The cost is linear: 16 edits give 0.4–0.6 µs per ray, 64 give
1.6–2.4 and 256 give 6–10 [E]. At the budget below (7,680 rays per second) that is 3–5, 12–18 and
46–77 ms of pool CPU per second: 0.3–0.5%, 1.2–1.8% and 4.6–7.7% of one core. Above about 64 edits
the arm needs a per-ray edit cull before it is affordable.

**The coverage contract.**
- The SDF arm sees exactly the analytic edit list that physics holds (`SdfField`,
  `crates/boyko_physics/src/sdf_query.rs`). That is the list the renderer gathers (`SdfEditStaging`),
  both clamped to `MAX_SDF_EDITS`. The render gather drops the excess silently (`collect_sdf_edits`
  in `crates/boyko_render/src/sdf_edit.rs`) [R].
- It does not see brick-only content, or MDF meshes without a CPU copy.
- A census gate (M5a) compares the geometry sources the renderer marches with those the occlusion
  arms read. It goes red when the renderer gains a source that no arm covers, unless that source is
  listed with a reason.

**Revisit trigger.** F13 (c) is re-decided when any of these lands: a raise of `MAX_SDF_EDITS`,
dynamic per-frame edits, or the streaming brick clipmap. All three are named as deferred in the
module doc of `crates/boyko_render/src/sdf_edit.rs` [R]. The census gate forces the question, because
it goes red in the rung that adds the source.

**What actually decides (c).** Not latency. Two frames at 60 fps (33 ms) is below this design's
100 ms occlusion smoothing time [E]; Unreal exposes the same knob, "Occlusion Interpolation Time",
without a documented default [V S23]. The discriminators are coverage, where the GPU arm wins once
bricks exist, and the missing readback path and async compute, where it loses today.

**Budget.** 64 real voices × 4 rays × 30 Hz round-robin = 7,680 rays per second:
- arm (a): 3–5 ms of pool CPU per second at 16 edits (0.3–0.5% of one core);
- arm (b): similar;
- arm (c): 1–3 ms of GPU per second plus two frames of latency.

**Decision.** Arms **(a) + (b)**, combined by the minimum transmission.
- M5a ships the SDF arm, with the endpoint rule; it depends only on the edit list.
- M5b ships the collider arm once SI1 has a ray arm.
- The result is written as `AudioOcclusion { transmission, lowpass_hz }` and smoothed with an
  interpolation time, 100 ms by default [E].

**Why this wins for the hybrid engine.**
- A boyko level is SDF or mesh.
- The SDF field answers "is the path blocked, and by how much" directly, with a free penumbra.
- Mesh geometry answers through collider proxies, which physics needs anyway.
- Both arms run on the pool in parallel, off the RT thread, with zero GPU cost.
- The GPU arm becomes the right choice when the scene's geometry outgrows the CPU field (the revisit
  trigger) or for thousands of rays (reflections, M8), once a gameplay readback path and async
  compute exist.

**Crate edges.**
- `boyko_audio` defines the component and must not depend on `boyko_render` or `boyko_physics`.
- The arms are systems registered by the composing layer (`boyko_app`), next to the edit-list
  owners: `SdfEditStaging` in `crates/boyko_render/src/sdf_edit.rs` and `SdfField` in
  `crates/boyko_physics/src/sdf_query.rs`. They are registered only when audio is On (§6.1).
- The x8 kernel lives in `boyko_physics`. Moving it to a shared leaf is request K-A4. Until then the
  SDF arm calls physics' kernel from the composing layer.

### F14 — Reverb and zones

| Option | µs per 10 ms [E] | Assets | Verdict |
|---|---|---|---|
| **FDN, 16 lines, per-band decay (Jot [V S61])** | 2–5 per instance | Presets | **Chosen**, at most 4 zone buses |
| Convolution | 25–100 per 2 s stereo IR | 768 KB per 2 s stereo f32 IR | Later arm (M8), for hero spaces |
| Steam Audio parametric or hybrid | — | Bakes | ✗ R5 |

- **Zone weights** are computed on the ECS side from the listener position against `ReverbZone`
  shapes (AABB or sphere; SI1 proximity once it exists). They become ramped send levels.
- **Cache footprint.** Four instances hold 1.2 MiB of delay lines, more than the 512 KiB L2 (§3.5).
  MQ-A1 measures the per-instance cost in that case. If it exceeds 2× the table, the default drops
  to 2 instances, or the maximum line length drops below 100 ms.
- **Experimental (M8): an SDF-derived reverb estimate.** About 32 rays from the listener through the
  field give a mean free path and an enclosure ratio, which map to RT60 through Sabine or Eyring.
  That costs about 20 µs per update [E].

### F15 — Music and clocks

- **The clock.** The RT thread publishes its sample position and a QPC stamp on the board at every
  wake (F6). The ECS side fits a real-time → sample-time map from them (F16).
- **Music structure.**
  - Music is streamed voices on a music bus, which is a `ui` bus by default, so music keeps playing
    and keeps its clock through a pause (F16).
  - `MusicClock` (tempo map: BPM, metre, start sample) quantises a transition to the next beat or bar
    as a **target sample**.
  - Stingers are scheduled one-shots.
  - Vertical layering is synchronised streams started on the same sample.
  - Horizontal re-sequencing uses segments with exit cues.
- **Precedent:** Kira clocks, Firewheel's musical clock, MetaSounds' sample-exact triggers [V S41]
  [V S43][V S21], and iMUSE's decision points [V S56].
- **Depth** is Q8.

### F16 — Gameplay API shape and the clock domains

| Option | Cost | Verdict |
|---|---|---|
| An entity per sound (Bevy/seedling [V S44]) | Archetype traffic for every footstep | For persistent emitters only |
| **An event per one-shot (`PlaySound`), with no entity** | One event-lane write | **Chosen for one-shots**. A sound that follows a moving entity names it (`follow: Option<Entity>`) |

Timing is decided here too, because it shapes the API. It is a **per-sound** choice, since it trades
latency against exact spacing, and no single lead can give both:

| Mode | Target sample | Latency at 60 fps, default period [E] | Spacing between sounds | Default for |
|---|---|---|---|---|
| `Timing::Immediate` | The first block rendered after the RT thread reads the frame's batch | Averages ≈ 23 ms; worst case ≈ 39 ms (F2) | Jitters by up to one frame plus one device period | Input-reactive one-shots (UI, the player's own shots) |
| `Timing::Exact` | The per-frame map below | Constant ≈ 44 ms (≈ 36 ms in the low class) | Sample-exact within and across normal frames | Fixed-step gameplay (footsteps, impacts, rapid repeats) and music |

**Three clock domains.**
1. **Sample time.** The RT thread's count of rendered frames at the internal rate.
2. **Real time (QPC).** The RT thread publishes (sample position, QPC) pairs on the board. The ECS
   side fits `real_to_sample` from them and **slews** it, never steps it, to follow drift between
   the device clock and QPC. This is the only slewed map, and it never sees game time.
3. **Virtual time.** `Time::elapsed`. It stops on `Time::pause()`, scales with `relative_speed`, and
   is clamped by `max_delta` (`crates/boyko_ecs/src/ecs/core/time/time.rs`, `Time`) [R]. Senders
   stamp events with it; fixed-step senders stamp substep time. A UI sender may stamp real time
   instead, and the event records which domain it used.

**The per-frame map.** At frame N the ECS side knows R_N (the real instant at which the frame's
`Time` was sampled), V_{N−1} and V_N (virtual elapsed time at the previous and current frame) and s
(`relative_speed`). An `Exact` event stamped with virtual time t in [V_{N−1}, V_N] targets

  sample(t) = real_to_sample(R_N) + lead − (V_N − t) ÷ s × 48,000

so the frame's virtual span is laid onto the real interval that ends at R_N + lead.
- **Pause.** Virtual time stops and no fixed substep runs. An event stamped while paused maps to
  `real_to_sample(R_N) + lead`, so it is never stale. The first frame after resume anchors afresh.
  Nothing is slewed through the pause.
- **Slow motion.** Real-time spacing is Δt ÷ s, so at 0.5× speed the footsteps are twice as far
  apart, which matches what the player sees.
- **Continuity.** When a frame is not clamped, its real delta equals its virtual delta ÷ s, and
  frame N's span ends exactly where frame N + 1's begins. Substeps on either side of a frame boundary
  keep exact spacing.
- **Clamped frames** (real delta above `max_delta`) have a virtual span shorter than their real
  interval. The map places it at the end of that interval: the result is a gap, never an overlap.
- **The lead** is measured, not assumed: the 95th percentile of (the frame's real delta + W) plus one
  device period. W is the time from the frame's `Time` sample to `audio_publish` (2–8 ms [E]). The
  lead is slewed like a jitter buffer, so a steady 30 fps game gets a 30 fps lead instead of every
  event arriving late. At 60 fps: 16.7 + ~5 + 10 ≈ 32 ms of render lead. Adding the residue, the
  padding and the stream latency (F2) gives the ≈ 44 ms in the table.
- **Hitches.** A frame whose virtual span is longer than the lead produces targets in the past. What
  happens to them is a per-sound `LatePolicy`:
  - `Skip`, the default for rapid repeats such as footsteps, drops a start that is later than the
    late window (default 100 ms), because a stale footstep is wrong;
  - `Clamp`, the default for impacts and explosions, starts it at the earliest sample the RT thread
    can still honour. A 16-substep hitch (`crates/boyko_ecs/src/ecs/core/time/fixed_loop.rs`, the
    `fixed_advance` bound) then loses spacing, not sounds, and the impacts concurrency group bounds
    the clump.

  Starts later than the target but within the late window start late, under either policy. All
  three cases are counted on the board.

**World and UI buses.** Each bus is declared `world` or `ui` in the bus graph.
- `Time::pause()` pauses the world buses. Their voices fade out over 10 ms, hold their cursors, and
  fade back in on resume. Virtual voices on them stop advancing. UI buses keep playing.
- `relative_speed` changes event spacing on world buses (above). It changes their pitch only on
  buses that opt in, for bullet-time effects. That is an authored choice, off by default.

**Offline goldens** drive both modes deterministically, because the offline pump fixes when each
batch is seen and supplies R_N.

### F17 — Diagnostics on the RT thread

- **Lane.** The RT thread calls `claim_lane()` at start and `release_lane()` at exit
  (`crates/boyko_diag/src/lane.rs`; the doc names OS and driver callbacks as claimants). Once KC-04
  lands, the lane comes from the engine thread context's single cold claim.
- **Profiling.** Zones go under the "Audio" engine scope.
- **No logging on the RT thread.** It never calls a log macro, so the lane-less `Warn`/`Error`
  synchronous path and its 50 ms deadline can never be reached. It reports conditions as board
  counters (F6), and `audio_drain` logs their deltas on the ECS side.
- **Statistics** (`AudioStats`) are computed by `audio_drain` from board deltas: block time p50/p99
  from the histogram, underflows, late and skipped starts, start slips, real and virtual counts,
  ring high-water marks, device resets.

### F18 — Determinism, replay, tests

- **The offline backend** runs the same RT loop, with the same entry function and FTZ state, driven
  by a test pump instead of a device. It is how every gate in §9 runs without an audio endpoint.
- **Goldens** pin hashes of rendered buffers. They are bit-reproducible because the kernels use only
  IEEE basic operations and `sqrt`, with no FMA and no library transcendentals (F10), and because the
  block and the internal rate are fixed.
  - That holds across hosts: MSVC, windows-gnu and Linux x86-64 execute these operations
    identically. The L1 leg re-runs every golden on Linux.
  - Offline and live renders agree only for the same batch → block assignment (F5).
- **Audio output is outside replay**, per the tree's existing decision. Whether voice selection and
  random picks join the determinism set is Q3; F11's counter-based draws make that possible.

### F19 — Floating-point environment

**Decision.**
- **One audited function.** FTZ | DAZ is set **inside the RT thread's entry** through a single
  function in `boyko_utils::fp_env`, which is the home the FMA document already gives the planned
  MXCSR probe. It writes `ldmxcsr` in `asm!`, with a `// SAFETY:` comment.
- **Nowhere else.** Not on the main thread, the pool or the IO thread.
- **Threads created by the OS or a plugin.** The RT thread creates no thread itself, but a device
  call can create one on its behalf.
  - On Linux a new thread inherits its creator's MXCSR [R]. The pipewire-alsa plugin creates and
    starts its "alsa-pipewire" loop thread inside `snd_pcm_pipewire_open` [V S66], that is, on the
    thread that calls `snd_pcm_open`.
  - On Windows a new thread does not inherit its creator's MXCSR: the G-FP module doc notes that a
    spawner-side mutation "reds only on Windows" (`crates/boyko_threadpool/tests/mxcsr_uniform.rs`)
    [R]. COM and WASAPI initialisation are opaque all the same.
  - So every backend call that can create a thread (COM initialisation, enumeration, device open,
    re-initialisation) runs inside `fp_env::with_default(…)`, which restores the IEEE default MXCSR
    for the call and FTZ | DAZ after it. A plugin's loop thread is neither a pool worker nor physics,
    so it could not have broken G-FP. The rule is simpler than that: no thread anywhere inherits a
    floating-point state that its author did not choose.
- **Gates:**
  - G-FP stays green with audio on. A new arm checks that after `AudioPlugin` has booted, the main
    thread and every worker report the default MXCSR.
  - The L1 leg checks that the RT thread's MXCSR is the default inside the guarded device open.
  - A denormal gate (M4) shows that FTZ matters: with FTZ off, a decaying-tail fixture's block time
    spikes. Its threshold is derived from MQ-A1's measurement on the reference CPU (§9).

### F20 — Coupling with effects, physics and the languages

- **Impacts.** A composing-layer system reads the physics contact report for bodies that carry
  `ImpactSound` and emits `PlaySound` events with `Timing::Exact` and `LatePolicy::Clamp`. Gain
  follows impulse, within a per-frame budget and an "impacts" concurrency group. Bodies without the
  component are never scanned, so the capability is structural. The system is registered only when
  audio is On (§6.1).
- **Rain and fire textures.** A looping, layered ambience emitter whose `AudioParams` follow the
  particle emitter's CPU-side `rate`. There is no per-drop voice and no GPU readback.
- **Explosions.** A one-shot with a propagation delay, a low layer on its own bus, and a ducking
  snapshot on the other buses.
- **Aether** machines send `PlaySound` like any system. **Gaia** authors `SoundDef`, bus graphs and
  snapshots (§6.6).

---

## 5. Recommended architecture

```
 ECS world — main thread + pool workers (frame rate)          RT audio thread — MMCSS "Pro Audio", FTZ|DAZ
 ────────────────────────────────────────────────           ─────────────────────────────────────────────
 components: AudioListener, AudioEmitter (+Enabled bit),      views into AudioRtStorage (fixed at boot):
   AudioParams, AudioOcclusion(Probe), ReverbZone               real-voice DSP rows (≤ RT_ROWS)
 resources:  AudioConfig/ResolvedAudio, AudioSlots,             flattened bus schedule (enum effects)
   AudioLink, AudioRtStorage, AudioClockMap,                    block FIFO (256 frames @ 48 kHz → any period)
   AudioBusGraph, Assets<AudioClip>, SampleBank (KC-15),      WASAPI loop, bounded wait → master SRC → device
   Assets<SoundDef>, AudioStats                                 (ALSA / wall-clock pump / offline backends)
 Main, after CameraSet::Resolve:                              per wake:
   AudioSet::Drain      ◄──── return board (atomics) ──────   store row acks, counters, histogram, clock
   AudioSet::Intake     (clock map, world pause)              read the frame batch (Acquire)
   AudioSet::Occlusion  (SDF arm, collider arm; pool)         apply starts at sample offsets, stops (fade),
   AudioSet::Spatialize (parallel)                              parameter targets (ramped)
   AudioSet::Select     (top-K real, hysteresis, budget)      voices ×8: fetch/decode → resample → filters
   AudioSet::Publish    ───── command ring (SPSC) ───────►      → gain/pan ramps → bus + sends
                              one Release per frame           buses in order → master soft clip → SRC → device

 IO thread "boyko-audio-io" (normal priority): file pages → QOA decode → per-stream page rings ──► RT reads
```

**Patterns borrowed, with their source.**
- RT rules and deferred freeing [S14][S15][V S16]. Freeing is not needed at all here (F8).
- A thread split in which the "audio thread" role becomes ECS systems [V S20].
- The grouped flush [V S43].
- Compile-to-static graphs [V S21].
- The concurrency vocabulary [V S22].
- The virtual-voice resume mode [S32].
- The attenuation vocabulary [V S23][V S12].
- The ambisonic bus [S49].
- Clock-quantized music [V S41][V S43].
- A bounded device wait and invalid-device recovery [V S2][V S65].

---

## 6. ECS landing

### 6.1 Crate, plugin, zero cost when off

**The crate.** A new crate, `boyko_audio` (package `boyko-audio`; proposed, not yet in the tree),
registered in `ENGINE_PACKAGES`.
- It depends on `boyko-ecs`, `boyko-utils`, `boyko-math`, `boyko-scene` (for `GlobalTransform` and
  `CameraSet`) and `boyko-diag`.
- On Windows it also takes `windows-sys` with `Win32_Foundation`, `Win32_Media_Audio`,
  `Win32_System_Com` and `Win32_System_Threading`. `AvSetMmThreadCharacteristicsW` and
  `WAVEFORMATEXTENSIBLE` are confirmed in those modules [V S62]. COM vtables are hand-declared.
- It does not depend on `boyko_render` or `boyko_physics`.

**The plugin.** `AudioPlugin` reads `AudioConfig` inserted before it.
- `AudioMode::On` boots the device on the RT thread in the startup device phase: today a startup
  system, and `StartupSet::Device` once EK9 lands. Teardown is ranked, today in `Drop` and later
  through `NonSendTeardown` (EK11). Teardown joins the RT and IO threads before `AudioRtStorage`
  drops (F7.1).
- Entities that carry audio components in a world with audio off cost nothing, because no system
  iterates them.

**The composing layer.** The acoustics arms and the impacts bridge live in `boyko_app`, because they
need physics and render types. They are registered by one function, `add_audio_bridges`, which
`EnginePlugins` calls only when `AudioConfig.mode` is On.
- The condition is the *requested* mode, not the device result. A `Silent` device still runs the
  wall-clock pump, so gameplay that waits on `SoundFinished` behaves the same (F2).

**`AudioMode::Off` registers nothing, in any crate**: no system, no resource beyond the
configuration, no thread, no device, and no bridge. The gate (§9, M0) builds the **whole engine
composition** twice, once with audio Off and once with no audio configured at all, and compares the
system, resource and thread censuses. Comparing only the audio plugin against its absence would miss
the bridges, which live outside it. Red control: register the impacts bridge unconditionally.

### 6.2 Components

| Component | Axis | Contents | Notes |
|---|---|---|---|
| `AudioListener` | 1 | Gain, listener index | Reads `GlobalTransform` |
| `AudioEmitter` | 1 | `sound: Handle<SoundDef>`, `bus: BusId`, `priority: u8`, flags (loop, spatial, doppler, propagation delay) | Authoring data; carrier hooks do the refcounting (F8) |
| `AudioEmitterEnabled` | 2 (bitset) | — | Playing or paused, O(1), no migration |
| `AudioVoiceLink` | 1 (written by intake) | `slot: u32`, `generation: u32` | Links an emitter to its slot. Removing it releases the slot through a hook that pushes a delta, which an apply system folds in (the `Assets` carrier-hook precedent). EK3's `Removed<T>` replaces the hook once it lands |
| `AudioParams` | 1 (optional) | `[f32; 4]` per-emitter parameter values (RTPCs) | Only emitters that have parameters |
| `AudioOcclusionProbe` | 1 (optional) | Rays, update rate, lift radius | The capability to be traced |
| `AudioOcclusion` | written by the occlusion arms | `transmission`, `lowpass_hz` | Smoothed |
| `ReverbZone` | 1 | Shape, preset, priority, fade distance | |
| `ImpactSound` | 1 (physics bridge) | `SoundDef`, impulse → gain curve | Composing layer |

### 6.3 Resources

| Resource | Role |
|---|---|
| `AudioConfig` | The owner's knob: mode, `LatencyClass` (set by the host only, F2), `max_real`, `flip_budget`, `max_slots`, backend. `RT_ROWS` and every `AudioRtStorage` capacity derive from it |
| `ResolvedAudio` | What the device granted: backend, device rate, period, channels, latency, or `Silent` with a reason |
| `AudioSlots` | The voice table: sound, clip, state, priority, audibility, cursor, generation, emitter, RT row, start sequence. KC-10 columns, SoA |
| `AudioLink` | The producer end of the command ring and the reader side of the return board. `Send` |
| `AudioRtStorage` | The owning columns of the RT working set, sized, committed and touched at boot (F7). Never grows; dropped after the threads join |
| `AudioClockMap` | `real_to_sample`, the measured lead, and the last frame's (R, V) anchor (F16) |
| `AudioBusGraph` | Declared buses (`world` or `ui`) and effect chains. Frozen at boot in v1 |
| `AudioSnapshots` | The active snapshot stack with fade times |
| `AudioGlobalParams` | Global parameters, a fixed array indexed by `ParamId` |
| `Assets<AudioClip>` + `SampleBank` | Clip headers, and the KC-15 segmented column holding one span per clip (F8) |
| `Assets<SoundDef>` | Random containers (weights, no-repeat), volume and pitch randomisation, attenuation, bus, priority, concurrency group, late policy, loop |
| `AudioRng` | The seed of the counter-based draws (F11, Q3) |
| `AudioStats` | The census |

### 6.4 Events

- **In:** `PlaySound`, `StopSound`, `SetAudioParam`, `ActivateSnapshot`. A sender stamps each with
  its virtual time; fixed-step senders stamp substep time. A UI sender may stamp real time, and the
  event records which (F16).
- **Out:** `SoundFinished`, `AudioDeviceChanged`.
- Events use the kernel's `EventWriter`/`EventReader` today and D-E20's writer lanes later.

### 6.5 Systems and ordering

All run in `CoreSchedule::Main`, in the `AudioSet` chain, ordered after `CameraSet::Resolve` so that
`GlobalTransform` is current.

| Set | System | Access | Parallel |
|---|---|---|---|
| `Drain` | `audio_drain` | `Res<AudioLink>` (return board), `ResMut<AudioSlots>`, `ResMut<AudioClockMap>`, `ResMut<SampleBank>` (retires), `EventWriter<SoundFinished>`, `ResMut<AudioStats>` | Serial |
| `Intake` | `audio_intake` | `PlaySound`/`StopSound`; `Added<AudioEmitter>`; emitters filtered by the enable bit; `Res<Time>`; `Res<AudioClockMap>`; `ResMut<AudioSlots>`; `Res<AudioRng>` | Serial today; concurrency rules live here. Order-independent by construction (F11) |
| `Occlusion` | SDF arm, collider arm (composing layer, only when audio is On) | `&AudioOcclusionProbe`, `&mut AudioOcclusion`, edit list or colliders | Parallel, round-robin budget |
| `Spatialize` | `audio_spatialize` | Listener, emitters, `AudioParams`, `AudioOcclusion`, zones → per-slot targets and audibility | Parallel |
| `Select` | `audio_select` | `ResMut<AudioSlots>` | Parallel score, serial top-K with the flip budget |
| `Publish` | `audio_publish` | `ResMut<AudioLink>` (command ring), `Res<AudioSlots>` | Serial: the single writer, one Release per frame |

### 6.6 Assets and authoring

- `AudioClip` loaders plug into `HasLoaders::LOADERS` (WAV, IMA ADPCM, QOA).
- Decoding at load writes a new span into the `SampleBank` (F8). KC-30c's loader decode context is
  the natural seam.
- `SoundDef`, bus graphs and snapshots are data assets that Gaia authors; "sound" is already a Gaia
  asset kind.
- `AssetServer::load` is synchronous today [R]. Resident clips load on it; streams are opened by the
  IO thread (F9). A long synchronous load stalls `audio_drain`, which the return board tolerates
  (F6).

### 6.7 Gameplay API sketch

```rust
// A persistent, positional, looping emitter. Pause it by flipping the enable bit.
commands.spawn((
    Transform::from_translation(pos),
    AudioEmitter::new(sounds.campfire).looping().bus(Bus::AMBIENCE),
    AudioOcclusionProbe::default(), // opt in to occlusion (Axis 1)
));

// Fire-and-forget: no entity. Follows the shooter; heard after the propagation delay.
play.send(PlaySound::at(sounds.gunshot, muzzle).follow(shooter).propagation_delay());
```

### 6.8 What audio does not do

- It reads no GPU data: no particle readback, and no GPU acoustics in v1.
- It does not touch MXCSR outside the RT thread.
- It changes no process-wide or endpoint-wide state on its own; the timer resolution and the
  low-latency period are the host's (F2).

---

## 7. Render graph, RHI, shader eDSL

- **Render graph:** no pass. With audio on, a frame declares and records the same command stream it
  records with audio off. That is a gate: the profiling command census, or byte-identical goldens
  with audio on (§9, M0).
- **RHI gaps for the recommendation:** none.
- **RHI gaps for the rejected GPU acoustics arm** (F13c), should it ever be revived:
  - an async compute queue (today one GRAPHICS|COMPUTE family, `find_queue_family` in
    `crates/boyko_rhi_vulkan/src/device.rs`);
  - timeline semaphores (absent);
  - a per-frame gameplay readback ring. `crates/boyko_app/src/particle_readback.rs` is a gate probe,
    not a gameplay path.

  `vkCmdDispatchIndirect` exists (`cmd_dispatch_indirect`) and would be enough to size the dispatch.
  `DrawIndirectCount` is absent and irrelevant here.
- **Shader eDSL:** no shader, so no rows in
  [SHADER-VARIANT-MANIFEST.md](../SHADER-VARIANT-MANIFEST.md). The DSP kernels adopt the eDSL's
  "one body, host oracle plus production arm" principle through the `Lane` type (F10).
- **If a GPU arm is revived later,** its attenuation and occlusion math moves into `boyko_shaderdsl`
  leaves, so that the CPU and GPU arms share one body. Any transcendental among them takes the
  polynomial body of F10, because the eDSL's f32 oracle forwards `sin` and `cos` to the host library.

---

## 8. Kernel requests and sequencing against the unified plan

| ID | Request | Covered by | Audio rung that waits |
|---|---|---|---|
| **K-A1** | Registry-free, address-stable Copy columns, sized and committed at boot, for the RT working set and the slot table | KC-10 `ScratchColumn::for_type` with `Reserve::Now` (D-S2), filled with KC-03's `ensure_len_zeroed` | M0 |
| **K-A2** | A public SPSC transport from a scheduler system to a non-scheduler thread: the `LogLane` shape, fixed-size records. Forward direction only; the return direction is a KC-03 board read through KF-42 atomic views (F6) | **New.** A file in `boyko_utils` (or `boyko_memory` after C1). Logging may migrate onto it later | M0 |
| **K-A3** | A ray (segment) arm in SI1: 8-wide slab traversal plus a leaf test per shape | **New.** SI1 lists sphere, AABB and user-predicate arms only | M5b |
| **K-A4** | `sdf_edit_list_x8` in a shared leaf rather than in `boyko_physics` | **New.** It must stay bit-identical to `boyko_sdf_math::sdf_edit_list` | M5a (a workaround exists) |
| **K-A5** | A kernel IO thread for streaming assets | **New.** The asset-streaming plan has none | M3 works without it; folds in later |
| **K-A6** | Write-once byte spans with free-and-reuse, read by another thread through raw span pointers | **KC-15 `SegmentedColumn<u8>`**: write-once spans never relocate (F8). The request asks KC-15 to state two things it does not state today: readers on other threads through raw span pointers, under F7.1's invariants; and, optionally, finer size classes (four per octave) if MQ-A9 prices power-of-two slack above budget. Rev 1 named KC-03 + KC-16 here, which cannot reuse a freed middle range | M0 (one boot-loaded clip); M1 (the loader) |
| **K-A7** | A thread-scoped commit deny: a bit in the thread's record (KC-04) that the cold commit path (`VmReservation::commit` in `crates/boyko_ecs/src/ecs/memory/vm.rs`) checks, aborting in gate builds. It is the VM-side sibling of `DenyAfterSteady` | **New.** One load and one branch on a cold syscall path; 0 on hot paths | M0 |

**Sequencing.**
- **New files only.** Audio is new files, outside every Phase D/E lock set in
  [UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](../unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md)
  §4.3. K-A7 is the exception: it touches the kernel's commit path, so it is filed to the kernel
  plan's owner and lands in that plan's cut, not audio's.
- **Shared edits**, each declared at its cut: root `Cargo.toml` (the member), `Cargo.lock`,
  `ENGINE_PACKAGES` in `crates/boyko_diag/src/sample.rs`, and the `IGNORE_CLASSES` row, which is a
  CLAUDE.md change.
- **D-S2 changes `ScratchColumn::new` and the scratch-id band.** Every user of either joins its touch
  set. So audio's storage rungs are **cut after D-S2 and C1**, and M0 is built on KC-10, KC-03 and
  KC-15 rather than joining the migration list. Building on the pre-D contract would mean porting
  twice.
- **The kernel-free parts land first as M0-leaf:** FFI declarations and a device probe, the DSP
  kernels with their `Lane` oracles and the deterministic math, the decoders and encoders as pure
  functions over slices, and the offline harness (test storage under `#[cfg(test)]`).

---

## 9. Rung plan

Every rung has a red-first gate (a control that must fail before the fix and pass after), a golden
where output exists, and a perf gate. Every gate runs on the offline backend, device-free, unless
marked `audio-device`.

| Rung | Content | Depends on | Red-first | Golden | Perf |
|---|---|---|---|---|---|
| **M0-leaf** | WASAPI/COM FFI declarations and a device probe; `Lane` DSP kernels (resamplers, master SRC, ramps, one-pole, biquad, soft clip); the in-house transcendentals and the square-root pan law; WAV, IMA ADPCM and QOA as pure decoders, and the two encoders; offline harness | Nothing (new files) | x8 ≠ scalar on a planted FMA call site: the per-file census must go red. A planted `f32::sin` in a kernel file must red the same census | Decoders bit-exact against committed reference vectors, generated once offline by each format's reference implementation (the QOA reference C decoder; an independent IMA ADPCM decoder) and committed with their provenance. Transcendentals within their stated error against an f64 oracle, with identical bits across the scalar and x8 arms | MQ-A1: µs per stage per 10 ms on the reference CPU, including the denormal FDN multiply penalty, four FDN instances with a cold L2, and the master SRC. Replaces §3.2 |
| **M0** | RT thread (MMCSS, FTZ through `fp_env`, the default-MXCSR guard around device calls, claimed lane); K-A2 ring; `AudioRtStorage` sized at boot; the return board's clock and device words; WASAPI shared backend (default and low class) with the bounded wait; wall-clock pump; master SRC; offline backend; `AudioPlugin` and `add_audio_bridges`, `AudioConfig`/`ResolvedAudio`; `SampleBank` with one boot-loaded clip; one-shot PCM `PlaySound`; soft clip | D-S2, C1 (K-A1, K-A6, K-A7) | (1) **RT memory deny, three legs.** Heap: a gate binary whose counting allocator aborts on an RT-thread allocation after the first block; red control: a test-only hook pushes to a `Vec` in the loop. Commit: K-A7's deny bit is set on the RT thread after boot; red control: a test-only hook grows an `AudioRtStorage` column from the RT thread, and the commit must abort. Frontier: after a 10,000-block soak, every `AudioRtStorage` column's committed bytes equal their boot value, the pattern of clause (d) of T-RED-1 in `crates/boyko_ecs/tests/em_deferred_recycle.rs`. (2) **No lost command:** a fixture stalls the RT consumer for 10 frames. Whole batches are deferred, and after the stall every slot's rendered state matches its desired state, with no leaked and no missing voice. Red control: a planted partial-batch publish loses a stop and leaks a voice. (3) **Zero cost off, whole engine:** the whole composition with audio Off equals the composition with no audio configured, in system, resource and thread counts. Red control: the impacts bridge registered unconditionally | A three-one-shot script, 44.1 kHz sources into the 48 kHz mix: its hash is identical across FIFO adapters for periods of 128, 480 and 1024 frames. The master SRC 48 → 96 kHz against an f64 windowed-sinc oracle within tolerance, x8 = scalar. **Click test:** maximum \|Δx\| at start and stop stays under a bound; red control: ramps off | RT block time p50/p99 with the pool saturated by physics (MQ-A2); physics step with audio on and off, quiet window (MQ-A3); the device's granted period, buffer size, stream latency and padding at wake (MQ-A6, `audio-device`); RT-thread page faults across the soak (MQ-A8). **G-FP arm:** main thread and workers at the default MXCSR after boot |
| **M1** | `AudioSlots`, intake, concurrency groups, `audio_select` with hysteresis and the flip budget, virtual cursors; row acknowledgements, counters and histogram on the board, and `SoundFinished`; clip retire; looping emitters with the enable bit; IMA ADPCM playback; cubic resampler; mirror rebuild after device reset | M0 | (a) Selection oracle (proptest against a brute-force sort). Red control: hysteresis off makes the thrash counter positive on the oscillation fixture. (b) **No lost acknowledgement:** the consumer stalls for 2,000 blocks while every one-shot ends; on resume every slot and clip is retired, `SoundFinished` fired exactly once per sound, all rows are free. Red control: a planted 512-record return queue leaks a row and a clip. (c) **Device gone, none returns:** fault injection invalidates the device and offers no replacement. The pump keeps the timeline, one-shots end on schedule, `SoundFinished` fires and slots free; re-attach runs when the injector offers a device. Red control: an unbounded wait, where the fake event never signals, produces no `SoundFinished` within the bound. (d) **Flip transient:** a burst that displaces 32 of 64 real voices cuts no fade (the click bound holds) and slips no start by more than one frame. Red control: `FADE_HEADROOM = 0` cuts fades | Resume at elapsed time is sample-exact. Mirror rebuild renders the same as an uninterrupted run from the reset block on | 4096-slot selection wall time; ADPCM µs per voice (MQ-A4) |
| **M2** | Listener; attenuation curves; square-root constant-power pan; Doppler; cones; distance LPF; propagation delay; buses (`world` and `ui`), sends, snapshots, ducking (envelope follower); `AudioClockMap` and `LatePolicy` | M1 | Pan law: L² + R² = 1 within 3 × `f32::EPSILON`, a bound derived from the operation count (≤ 2.5 ulp at 1.0). Red control: a linear law violates it | **`Timing::Exact` footsteps from consecutive fixed substeps start 750 samples apart.** At 0.5× speed they start 1,500 samples apart. Across a 5 s pause, the first footstep after resume starts within one frame of the resume and none is skipped; world buses fall silent and UI buses play on. A 16-substep hitch loses no `Clamp` impact and starts no `Skip` footstep older than the late window. Fly-by render hash | `audio_spatialize` at 1024 emitters |
| **M3** | IO thread; QOA streams; music bus; page rings | M1 | **Underflow counter:** a throttled IO fixture must count more than zero (red); an unthrottled run counts zero | QOA stream render equals a resident decode of the same file | RT block time unchanged with 8 streams; IO µs per stream-second |
| **M4** | Batched biquads; FDN reverb; `ReverbZone` weights; compressor, ducker, delay; runtime graph swap at block boundaries | M2 | **Denormal gate:** with FTZ off, a decaying-tail fixture's block time rises by at least half the ratio MQ-A1 measured for denormal FDN multiplies on the reference CPU (the red control); with FTZ on it stays flat. If MQ-A1 measures less than 2×, FTZ stays as hygiene and this gate becomes an MXCSR readback on the RT thread | FDN impulse-response decay within tolerance of the design RT60; x8 biquad = scalar bit-exact; scalar against an f64 reference within tolerance | FDN µs per instance, four instances interleaved |
| **M5a** | Occlusion SDF arm (composing layer) with lifted endpoints and the two-sided penumbra; smoothing | M2, K-A4 or the workaround | Wall-between fixture: transmission < t. **Clear fixtures, each ≥ 0.95:** open space; a source on a floor at 10 m and at 20 m; a source against a wall; a listener against a wall; the 2 × 20 m corridor. Red control: rev 1's one-sided, unlifted estimator reads the 10 m floor fixture below 0.5. **Coverage census:** a render geometry source that no occlusion arm reads, and that is not listed with a reason, reds the gate | Continuity: transmission changes continuously as a source slides behind an edge (no step larger than ε) | µs per ray, x8 against scalar, at 16, 64 and 256 edits (MQ-A5) |
| **M5b** | Collider arm | SI1 + K-A3 | The same fixtures with box colliders | The minimum-combine of the two arms | Rays per ms |
| **M6** | Ambisonic bus (1st → 3rd order) + binaural decode; SOFA conversion tool; VBAP 5.1/7.1; optional Spatial Sound objects (Q1) | M2 | Energy preservation of encode + decode. Red control: wrong normalisation | Binaural render hash; VBAP gains against the closed form | Decode µs per mix |
| **M7** | `MusicClock`, quantized transitions, stingers, layering | M3 | A transition requested mid-bar lands on the next bar sample; red control: unquantized | Transition render hash. Across a pause, music on the `ui` music bus keeps its bar grid; a world-bus layer pauses and resumes on the grid | — |
| **M8** | Rooms and portals; convolution arm; experimental SDF reverb estimate | M4, M5 | Portal path oracle | Convolution against a direct FIR oracle | Convolution µs per instance |
| **M9** | Tooling: editor mixer over reflection, capture to WAV with the batch → block log, census view | M1+ | Audio output is unchanged with tools on (a golden, not a counter) | A captured live session replays offline to the same hash | Tool overhead |
| **L1** | ALSA backend (`dlopen`), with device calls under the default MXCSR | M0, per Q6 | Loader-absent fixture degrades to `Silent` | Offline goldens identical bit for bit to the Windows host's | Achieved period on a PipeWire system; RT-thread page faults per thread (MQ-A8) |

---

## 10. Risks

1. **MMCSS preemption of pool workers.** Up to about 0.4 ms of one core per 5.33 ms block can shift a
   physics step's critical path. MQ-A3 measures it. If the regression exceeds noise, the fallback is
   to size the pool at one fewer worker while audio is on. That is a plugin-owned choice, and it is
   measured, not assumed.
2. **Stream starvation.** Mitigated by the dedicated IO thread and a second of look-ahead. The
   underflow counter is visible in `AudioStats`.
3. **Device churn** (default device changes, sleep and resume, Bluetooth, another application taking
   exclusive mode, a device that never comes back). Handled by re-initialising and by the wall-clock
   pump, and tested offline by injecting faults into the backend (M1). Real endpoints are covered
   only by the `audio-device` class.
4. **FTZ leaking to another thread.** Closed by construction (F19), including threads that a device
   plugin creates, and gated by G-FP with an audio arm.
5. **The estimates are wrong.** The §3 figures are [E]. MQ-A1 re-derives them before M0 relies on
   them. Decisions that would flip at 2× error: `MAX_REAL` (F11), the ambisonics crossover (F12), the
   single RT thread (F4, only above about 1000 voices), the FDN instance count (F14).
6. **Phase D delays audible output.** Storage waits for D-S2 and C1. M0-leaf proceeds meanwhile.
7. **The SI1 ray arm is not in the plan.** Until it is, mesh-heavy scenes get occlusion only where
   SDF geometry exists. The SDF arm alone serves SDF-heavy scenes.
8. **Vendor claims tagged [S]** (FMOD, Wwise, Frostbite). None decides an option on its own. The
   decisions rest on [V] sources and on the MQ measurements.
9. **HRTF bake-time dependency.** SOFA/HDF5 conversion uses a third-party reader offline only. If
   that is unacceptable, the fallback is to ship one pre-converted dataset.
10. **A glitch in a soak test that does not reproduce is not a result.** The owner's workstation is
    also the measurement machine, so an unreproduced single anomaly must be reproduced before it is
    attributed.
11. **Working-set trimming.** Pages committed at boot can still fault if Windows trims them (F7).
    MQ-A8 measures it; locking is a host option, not a library default.
12. **`SampleBank` slack.** Power-of-two classes commit about 1.4× the resident content (F8). MQ-A9
    measures real content, and K-A6's finer classes are the remedy if the number is too high.
13. **Latency is higher than rev 1 stated.** The corrected chain gives ≈ 39 ms worst case by default
    and ≈ 44 ms for `Exact` (F2, F16), not 33 ms. The low class recovers about 8 ms at a power cost
    and with an effect on other applications. That is Q5.
14. **The SDF arm's coverage can fall behind the renderer.** The census gate (M5a) reds when it
    does, and F13's revisit trigger names the three rungs that would cause it.

---

## 11. Measurement queue

| ID | What | Rung | Replaces |
|---|---|---|---|
| MQ-A1 | µs per stage per 10 ms, scalar and x8, reference CPU. Includes the denormal FDN multiply penalty (M4's threshold), four FDN instances interleaved with a cold L2, the master SRC at 44.1/96/192 kHz, and the transcendental approximations | M0-leaf | §3.2 |
| MQ-A2 | RT block time p50/p99 with the pool saturated by the physics J-A scene | M0 | F4's margin |
| MQ-A3 | Physics step time with audio on and off (quiet window, the physics measurement protocol) | M0 | Risk 1 |
| MQ-A4 | ADPCM and QOA decode µs | M1 / M3 | F9 |
| MQ-A5 | SDF occlusion µs per ray, x8 and scalar, at 16, 64 and 256 edits, with the endpoint lift | M5a | F13 |
| MQ-A6 | On the owner's endpoint: granted shared-mode periods (`GetSharedModeEnginePeriod`), `GetBufferSize`, `GetStreamLatency`, and the padding at each wake | M0 | F2 latency |
| MQ-A7 | Vorbis decode µs (only if Q4 funds an in-house decoder) | — | F9 |
| MQ-A8 | RT-thread page faults across a soak: per thread on Linux (`getrusage(RUSAGE_THREAD)`), process-wide on Windows | M0 / L1 | F7's residual, risk 11 |
| MQ-A9 | `SampleBank` committed bytes ÷ content bytes on a real content set | M1 | F8's 1.39× |

---

## 12. Owner value questions

Only values. Every technical fork is decided above.

1. **Spatial target.**
   - Stereo only?
   - Headphone binaural (M6)?
   - Atmos / Windows Sonic objects, which Windows caps at 20 objects over HDMI and 128 on
     headphones?
2. **How ambitious should acoustics be?**
   - None.
   - Occlusion (M5).
   - Rooms and portals (M8).
   - Baked wave acoustics, which this design rejects on rule 5 and on effort.
3. **Replay.** Should the determinism promise cover audio **decisions** (which voice plays, random
   picks, virtualization), or stay as it is ("not audio")? Audio output bits are never promised.
4. **Music and dialogue storage.** The choice is between disk size and dependencies:
   - accept QOA at about 2.4× Vorbis's size (60 minutes of music: 138 MB against 58 MB) and stay
     fully in-house;
   - fund an in-house Vorbis decoder, a large decoder;
   - allow a rule-5 exception (libvorbis or libopus).
5. **Latency against power and against other applications.** The 10 ms shared period by default
   (≈ 39 ms worst case), or the low `IAudioClient3` period (≈ 31 ms)? The low one costs power, and
   while the game runs it switches every other application on the same endpoint to the small period
   [V S1]. The owner accepted a similar trade for `timeBeginPeriod(1)`. Either way the host sets it,
   not the library (F2).
6. **Linux audio timing.** Now, headless, before Linux windowing exists (L1 needs no window), or
   together with the Linux host?
7. **Tooling ambition.** Profiling, census and capture only (M9), or authoring and live-mixing tools
   comparable to Wwise or FMOD?
8. **Interactive music depth.** Stingers and quantized transitions (M7), or a full
   horizontal/vertical adaptive system with an authoring tool?
9. **Voice chat and microphone capture.** In scope? A yes brings Opus, and with it the rule-5
   question.

---

## Revision log

- **rev 1 (2026-09-25):** first design. It decides F1–F20 and corrects five statements of the
  research input (§0.2). It queues MQ-A1…A7 and asks Q1–Q9.
  - The author's own read-back fixed three errors in the first draft before publication:
    - One fixed one-block lead cannot give exact spacing for events stamped earlier in the frame.
      Timing is now a per-sound `Immediate`/`Exact` choice with stated latencies (F16).
    - "Overflow impossible by construction" did not survive a device reset that stops the consumer.
      The claim is now "no command lost", through diff-based publishing and whole-batch deferral (F6).
    - The ambisonics crossover range was mis-derived. It is now N = F ÷ (h − a) ≈ 3–40 voices (F12).
- **rev 2 (2026-09-25):** the first critique pass. All 17 remarks accepted; see the Review log. The
  largest changes:
  - the RT working set is sized, committed and touched at boot and owned by a resource, with a
    commit-deny gate (F7, K-A7);
  - the return ring became a lossless board (F6);
  - three clock domains, with a per-frame map and a late policy (F16);
  - deterministic in-house transcendentals (F10);
  - `SampleBank` on KC-15 (F8);
  - endpoint lift and a two-sided penumbra (F13), plus edit-count scaling, a coverage contract and a
    revisit trigger;
  - a fixed 48 kHz internal rate (F5);
  - a bounded wait and the wall-clock pump on device loss (F2);
  - `RT_ROWS` with fade headroom (F7, F11);
  - a whole-engine zero-cost gate (§6.1).
  - The author's own re-derivation while revising found one more error: the rev-1 latency chain
    counted one block for the RT thread's pickup, but the thread wakes once per device period. The
    chain is now ≈ 39 ms worst case by default, not 33 ms (F2, risk 13). MQ-A8 and MQ-A9 are new.

---

## Review log

The first critique pass (2026-09-25) returned CHANGES_REQUESTED with 2 critical, 9 important and 6
optional remarks. Every remark was checked against the tree at `6394bc5e` or against a primary
source before it was accepted. None is refuted.

| # | Remark (short) | Verdict | Fix |
|---|---|---|---|
| C1 | The RT allocation-deny gate cannot see VM commits, and RT columns were never pre-sized | Accepted. The G2 census's own "Coverage boundary" section states the blindness [R] | F7: `AudioRtStorage` sized, committed and touched at boot, with a size table; the RT thread holds views with no push. §9 M0 (1): heap deny, commit deny (K-A7), committed-bytes frontier after a soak (T-RED-1 clause (d) pattern). Residual page faults: MQ-A8, risk 11 |
| C2 | The return ring has no overflow policy and carries the acknowledgements clip lifetime needs | Accepted. The 2.7 s fill time re-derives from rev 1's own sizes | F6: the return ring is replaced by a level-triggered board (row words, monotonic counters, histogram, seqlock clock, device word), so there is no overflow case. F8's retire reads the row words. §9 M1 (b): a 2,000-block stall gate whose red control is the rev-1 queue |
| W1 | `Exact` through a never-stepped QPC map breaks on pause and time scaling | Accepted. `Time` has `pause()` and `relative_speed` [R] | F16: three clock domains, a per-frame virtual → real map (pause, slow motion, clamped frames, continuity), a measured lead, `LatePolicy::{Skip, Clamp}`, `world`/`ui` buses. §9 M2 fixtures (pause, 0.5×, hitch) and an M7 pause case |
| W2 | Bit-reproducibility ignores `f32::sin/cos/exp/ln` | Accepted. Rust documents them as non-deterministic [V S63]. The eDSL's f32 oracle also forwards `sin`/`cos` to the host library [R], so it cannot be the fix | F10: basic operations and `sqrt` only; in-house polynomial transcendentals; the census bans the std names; the square-root pan law. M2's pan tolerance is derived (3 × `f32::EPSILON`). F18: goldens hold across hosts, re-run by L1 |
| W3 | K-A6 is not covered by KC-03 + KC-16, and KC-15 was rejected for a reason ED15 does not support | Accepted. ED15 relocates only a span that grows [R] | F8: `SampleBank` on KC-15 with write-once spans; slack ≤ 2×, ≈ 1.39× expected; the retire reads the board. K-A6 corrected, with an optional finer-class request; MQ-A9; risk 12 |
| W4 | The free penumbra reads a source on a surface as occluded, and M5a cannot see it | Accepted. The arithmetic re-derives | F13: the endpoint lift, a two-sided term, k = 16 from the corridor fixture, and the flat-floor residual stated. M5a: floor, wall, listener-at-wall and corridor fixtures; red control is the rev-1 estimator |
| W5 | Device rates above 48 kHz fall outside the FIFO range and the cost model | Accepted | F5: a fixed 48 kHz internal rate and an in-house master SRC (1–8 µs); `AUTOCONVERTPCM` [V S64] kept as the fallback. §3.2 and §3.3 rows; M0 SRC golden |
| W6 | A device lost mid-session with no replacement stops the voice timeline | Accepted. Microsoft's sample bounds its own wait [V S2] | F2: a bounded wait; on loss, the wall-clock pump; re-attach every 500 ms or on notification, per [V S65]. §9 M1 (c): a "device gone, none returns" fault gate |
| W7 | RT rows equal `MAX_REAL`, but every flip fades | Accepted | F7/F11: `RT_ROWS = MAX_REAL + 2 × FLIP_BUDGET` (96), a flip budget, start slips counted, fades never cut. F6: the record bound is one per row per frame. §3.3 transient row; §9 M1 (d) |
| W8 | The zero-cost gate cannot see the composing-layer systems | Accepted | §6.1: `add_audio_bridges`, called only when audio is On; the gate compares the whole engine composition. Red control: the impacts bridge registered unconditionally |
| W9 | F13 is decided at one fixed point, 16 edits, with no scaling and no revisit trigger | Accepted. `collect_sdf_edits` drops the excess silently, and the module doc defers the edit-cap raise and the brick clipmap [R] | F13: cost at 16/64/256 edits; the coverage contract; a census gate; the revisit trigger; the GPU arm's rejection re-argued on coverage and readback, not latency. MQ-A5 at three edit counts |
| O1 | `LatencyClass::Low` changes the endpoint period for other clients | Adopted. Microsoft: every application on the endpoint switches to the small buffer [V S1] | §0.1 row, F2 table, §6.3, §6.8, Q5: the low class is host-owned |
| O2 | "The RT thread spawns no thread" does not hold on Linux | Adopted. pipewire-alsa starts its loop thread in `snd_pcm_pipewire_open` [V S66] | F19: `fp_env::with_default` around every device call that can create a thread; F3; L1 check |
| O3 | Encoders are never mentioned | Adopted | F9: in-house bake-time encoders; the reference implementations are used once, offline, for test vectors |
| O4 | "Offline equals live" needs the same batch → block assignment | Adopted | F5 and F18 narrowed; M9 logs the assignment and replays a live capture |
| O5 | Four FDN instances exceed the 512 KiB L2 | Adopted. The tree records the L2 size [R] | §3.2, §3.5, F14; MQ-A1 measures the cold case, with a stated fallback |
| O6 | Research [S13] is cited nowhere | Adopted | Now cited: F1 here and §1.1 of the research, as a shipped raw-WASAPI `IAudioClient3` client [V S13] |

**Arithmetic note.** The critique found that the ambisonic crossover's central value is 50 ÷ 4.9 ≈
10, not 11. Corrected in the TL;DR and F12. It decides nothing, because 64 real voices exceed the
whole 3–40 range.

**The critique's open questions.**
1. *Does the latency chain count the device queue?* It did not count it correctly. The loop now fills
   to a target padding of one period plus one block, and MQ-A6 records `GetBufferSize`,
   `GetStreamLatency` and the padding at each wake. The chain was re-derived (F2): rev 1 had also
   counted one block where the pickup granularity is one device period.
2. *Is M4's 10× threshold measured on Zen 3?* No. It came from the 2015 arXiv figure. The threshold
   is now half of the ratio MQ-A1 measures on the reference CPU, with a stated fallback if the
   penalty turns out small (§9 M4).
3. *Where are the SAFETY argument and the Miri coverage for cross-thread column access?* In F7.1: five
   invariants, a Miri test under Tree Borrows over `vm.rs`'s Miri arm, and loom models for the
   atomic protocols. K-A6 asks KC-15 to state the reader contract.
4. *Does the seeded RNG stay order-independent if intake is parallelised?* It does now: draws are
   counter-based hashes of the event's own key, not steps of shared state (F11).

**The scope flag.** The critique notes that the user's request in the triggering session named render
topics (post-FX and anti-aliasing, VFX, sun rays, volumetrics), and that audio overlaps them only in
the sound of explosions and rain (F20). [VFX-RESEARCH.md](../render/VFX-RESEARCH.md) cross-references
this survey for that overlap. That is a question for the owner, not a defect in these documents.
They stay uncommitted on `u/research-0925`, and whether they land with the render batch is the
owner's call.
