# Phase 20.1 — Results: Interpolated GPU Mirror

Binding spec: [PHASE-20.1-PLAN.md](PHASE-20.1-PLAN.md) (critic Round 1 folded — ★R1-1…★R1-5, ★n6…★n11; see the plan's "Critic Round 1 — resolutions"). Branch `ecs`, on top of `45ae291`. Scope held: **`crates/boyko_demo` only + docs — zero engine diffs.**

## Mechanism (as planned, D1–D8)

`GpuInstance` grew a trailing `prev_pos: [f32; 2]` (16 → 24 B, offsets 2/3/4 preserved, new `@location(5)` at offset 16); the vertex shader renders `corner * scale + mix(prev_pos, pos, alpha)` with `alpha = FixedTime::overstep_fraction()` delivered through the camera uniform (64 → 80 B, explicit zeroed pad for deterministic uniform bytes, ★n8). `prev_pos` discipline is single-site (D3): the per-substep shuffle in `sync_gpu_instance` (old packed `pos` → `prev_pos`, ★n6 load-bearing note recorded in its docs) plus the `GpuInstance::new` spawn seed (`prev = pos`; ★R1-1 "spawn-seed only" doc guard on `new()` itself). `sync_ball_gpu` / `tint_collided` became field-granular writers (never touch `prev_pos`; `tint_collided` lost its pos/scale RMW — strictly cheaper). Uploads are gated in `app.rs` (D5): `upload_due(steps, count, last_uploaded_count)` — a 0-substep display frame with a stable entity count uploads **nothing** and reuses the cached draw count; no snap machinery anywhere (D8). Panel gained the upload events/s + MB/s probe (★R1-3, the wasm-visible witness).

**Headline**: at 144 Hz display / 64 Hz sim, upload events drop 144/s → ~64/s (**−55 %**, with the CPU column walk skipped on every gated frame); upload bandwidth 230.4 → 153.6 MB/s at 100 k (−33 %) despite the +50 % record size.

## Gate verdicts

| Gate | Verdict | Numbers |
|---|---|---|
| **G1** upload-event cut | **PASS** | T5 `gate_skip_rate_at_144_over_64` (deterministic, 1000 synthetic 144/64 frames, constant count): fires = **445** (444 substep-bearing + 1 forced first frame), skip rate = **55.5 % ≥ 55 %**. Burst/pause rows: paused → 1 fire (first frame only); burst-on-0-substep → exactly +1 fire. |
| **G2** pack perf (★R1-4 calibrated) | **PASS** (ratio form) | Baseline 16 B mirror measured FIRST: **7.52 ns/row** (752.47 µs / 100 k) — > 3 ns/row on this machine (divider-port-bound sqrt body), so the binding gate is the ratio. 24 B pack: **8.84 ns/row** (884.32 µs / 100 k). **Ratio = 1.18× ≤ 1.6×.** (Run 1 confirm: 7.62 vs 9.26 ns/row, 1.22×.) Absolute 24 B ≤ 5 ns/row reported informationally: not met, as anticipated by ★R1-4's calibration clause. |
| **G3** no regression | **PASS** | All 15 pre-existing demo tests green, unmodified. Full demo suite 30 tests (16 unit + 14 integration); workspace `--all-targets` fully green (100 suites, 0 failures). |
| **G4** wasm | **PASS** | `cargo check --target wasm32-unknown-unknown -p boyko_demo` clean (criterion/proptest target-gated out of the wasm graph). |
| **G5** layout fence | **PASS** | Const-asserts updated and compiling: `GpuInstance` 24 B / align 4, `CameraUniform` 80 B. |
| **G6** engine frozen | **PASS** | `git diff --name-only` ⊆ `crates/boyko_demo/**` + `docs/**`. Zero diffs under `crates/boyko_ecs` or any other engine crate. |
| **G7** prev correctness | **PASS** | T3 (Particles) + T4 (Physics) bitwise: `prev_pos == prior substep's packed pos` for every row; T4 additionally proves the field-granular writers never re-shuffle/clobber prev across 10 substeps and that tinted rows changed **color only** (★n10 via `pack_rgba8` re-derivation; ★n11 motion witnessed via `Position` diff). T6 spawn-seed `prev == pos`; T7 proptest (8 cases, random substep/idle sequences) holds the invariant after every pack and bit-identity across idle frames. |

★R1-5 W1 gate: `cargo check` clean + manual native launch — pipeline/bind-group validation passed at runtime (12 s alive, no wgpu validation errors), repeated on the final build with the live gate/alpha/probe path.

## Notes

- T4's flash color is asserted via `GpuInstance::pack_rgba8(COLLISION_FLASH_COLOR)`; the const was made `pub` (not `pub(crate)`) because integration tests are an external crate — the ★n10 substance (no duplicated magic bytes) is preserved.
- Bench host shows high baseline variance (laptop, background load); the ratio gate is exactly the ★R1-4 contingency for this.
- Behavior deltas accepted by the plan: paused frames freeze alpha mid-lerp (no snap, D7); pack-affecting sliders don't re-render while paused (baseline-identical); rare catch-up frames render only the final substep pair (D2, Bevy-equivalent).
