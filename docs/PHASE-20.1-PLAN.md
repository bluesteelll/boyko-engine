# Architecture: Phase 20.1 — Interpolated GPU Mirror (fixed-step sim, display-rate smoothness)

Companion to `docs/PHASE-20.1-RESEARCH.md`. Branch `ecs`, HEAD `45ae291`. Format reference: `docs/PHASE-20-PLAN.md`.

> Architect's verification note: all load-bearing research facts re-verified against HEAD. Notable: the 16 B `GpuInstance` const-asserts at instance.rs:52-53; all three spawn sites funnel through `GpuInstance::new` (app.rs:92, modes.rs:126, modes.rs:250); `SimRunner::step` already returns the substep count on both targets (runner.rs:344, :467) and app.rs:403 currently discards it; the three `GpuInstance` writers are full-struct rewrites (common.rs:43, physics.rs:347, :370); `overstep_fraction` is pinned to `[0,1)` (fixed_time.rs:141-150). One researcher correction: the upload is per DISPLAY frame (app.rs, after the whole `fixed_advance` loop), so the GPU-lerp variant uploads at `min(display, sim)` rate — this also removes any need for `last_fixed_substep()`.

## Goal

Decouple visual smoothness from the 64 Hz fixed step: render position = `mix(prev_pos, pos, alpha)` evaluated **on the GPU** per vertex, with `alpha = FixedTime::overstep_fraction()`. Simultaneously **cut upload traffic**: at 144 Hz display / 64 Hz sim, ~56 % of display frames expend 0 substeps and currently re-upload an unchanged 16 B × N column (230.4 MB/s at 100 k); after this phase those frames upload **nothing** (net 153.6 MB/s at 24 B × N × 64 Hz, −33 %, and −55 % upload *events* + CPU column walks). Engine core (`boyko_ecs`, `boyko_threadpool`, `boyko_macros`, `boyko_utils`): **zero diffs**.

Functional bar: all three modes (Particles 100 k / Boids 30 k / Physics 4 k) render motion smoothly at any display Hz ≥ sim Hz, with no cross-entity smear on despawn/mode switch, no spawn-frame flash, and correct pause/click-burst behavior.

## Context and constraints

- **Affected**: `crates/boyko_demo` only — `render/instance.rs`, `render/camera.rs`, `render/mod.rs`, `render/shader.wgsl`, `sim/systems/common.rs`, `sim/systems/physics.rs`, `app.rs`, tests, one new bench. `boyko_ecs` untouched (D6).
- **Invariants preserved**:
  - The zero-copy headline: `for_each_chunk(&[GpuInstance])` → `cast_slice` → `write_buffer`, one contiguous column, no AoS repack (app.rs:286-314).
  - The const-assert fence on `GpuInstance` layout (instance.rs:52-53) — updated, not removed; a silent layout drift must still fail the build.
  - Pack runs **every substep** (prev must be exactly one substep behind; catch-up frames must not widen the lerp window — D2).
  - `Changed<Velocity>` tint precision (physics.rs module docs) — untouched.
  - wasm sequential runner reuses the same system bodies unchanged (runner.rs:503-540 already calls `sync_gpu_instance` per substep).
  - WebGL2 budget: stays at 2 vertex buffers / 5 attributes / 80 B uniform — far under the 8/16 limits.
- **Targets** (binding gates in §Metrics): upload events −55 % at 144/64 (deterministic gate test); pack ≤ 5 ns/row at 100 k rows (criterion); zero new `unsafe`; 15 existing demo tests green; wasm32 check clean.

## Key decisions

### D1: Mechanism = GPU-side lerp with `prev_pos` embedded in `GpuInstance` (16 B → 24 B) — research V2

**What**: `GpuInstance` grows a trailing `prev_pos: [f32; 2]`. The vertex shader computes `mix(prev_pos, pos, alpha)`. One mechanism for all three modes.

**Why** (perf + correctness):
- *Bandwidth*: 24 B × N × 64 Hz = 153.6 MB/s at 100 k vs baseline 230.4 MB/s — because correct interpolation data on the GPU lets every 0-substep display frame skip the upload entirely (D5). CPU-lerp (V1) is structurally barred from this win: it must rewrite and re-upload positions at **display** rate, making it the *most* expensive variant (research: 40-72 ms/s) — and the demo has no Main-schedule home for a display-rate pass anyway.
- *Correctness is structural, not policy*: `prev_pos` lives in the same struct as `pos`, so `swap_remove` despawn moves them **together**; row-misalignment between prev and current data is impossible by construction. Spawn seeding is a single funnel: all three spawn sites call `GpuInstance::new`, which seeds `prev_pos = pos` — exact and free.
- *One buffer, one upload loop, one `cast_slice`*: the upload path changes only in stride. No second buffer, no second `write_buffer` per chunk, no third vertex-buffer bind.

**Alternatives**:
- **V3 — double-buffered 16 B instance buffers (swap = snapshot), 102.4 MB/s** — REJECTED, dual verdict:
  - *Demo verdict*: the `count_changed ⇒ snap` policy IS sound for this demo as built (structural changes only at mode transitions with distinct population constants and append-only click bursts — every row movement is accompanied by a count change today).
  - *Engine/general verdict*: **disqualified**. The soundness rests on a non-local, non-enforced population invariant. General failure: one despawn + one spawn inside the same upload window preserves count while `swap_remove` moves a row ⇒ prev-buffer row j is a DIFFERENT entity ⇒ cross-entity smear with no signal — the compiles-but-lies class the house bars. No surveyed engine ships positional double-buffering (Unity keys prev per-object = V2-shape). The payback (~51 MB/s of PCIe, <0.4 % of PCIe 3.0 x16; ~5-10 ms/s of one core) is not worth a latent smear class in the engine's showcase.
- **V6.1 — separate 8 B `PrevPos` component column, third vertex buffer** — rejected on cost and churn: same 153.6 MB/s as V2 but DOUBLE the `write_buffer` calls per upload, two write streams in the pack, a 4-column query, a new component threaded through 3 bundles + 4 `create_entity` lists, a second 8 MB GPU buffer, an extra vertex-buffer bind. Research costs it strictly above V2. Its one advantage — prev structurally isolated from color/scale writers — is recovered in V2 at zero cost by D3 + T3/T4.
- **V4 — GPU extrapolation (`pos + vel·alpha·dt`)** — rejected for all modes: zero latency but mis-predicts every collision/bounce (overshoot up to ~3 dot diameters), and Physics mode bounces constantly — the artifact would be the most visible thing on screen. Interpolation's one-substep (15.625 ms) latency is the standard, invisible trade (Bevy, Godot, Unity DOTS interp path).

**Trade-off**: +50 % bytes per upload event (recouped ×2 by event-rate cut); 24 MiB instance buffer (was 16 MiB); one-substep render latency; per-writer prev discipline (made structural in D3).

### D2: Prev maintenance = pack-shuffle inside `sync_gpu_instance`, every substep

**What**: the shared pack (common.rs:33-45) becomes: read `gpu.pos` (the previous substep's packed position) into `prev`, then write the full 24 B record. No integrator changes; no new system.

**Why**:
- `sync_gpu_instance` already runs every substep in ALL three modes on both targets (native: runner.rs:283-291; wasm: runner.rs:536), ordered after every integrator/write-back and after the spawn systems — exactly where "old `gpu.pos` = position after substep N−1" holds. In Physics, the last pos writer of substep N−1 was `sync_ball_gpu`, writing from the same post-solve `Position` — the shuffle source is identical there too.
- Catch-up frames: pack-per-substep keeps `prev` exactly one step behind; the lerp window never widens. Rendering skips intermediate substeps on catch-up frames — same as Bevy; accepted.
- Cost: +8 B load +8 B store per row per substep = 1.6 MB/s of L1-resident traffic at 100 k × 64 Hz — noise against the 24 B streaming write. Single sequential SoA stream; `par_iter_mut` over disjoint rows unchanged.

**Alternatives**: integrator-fused prev — rejected: touches 3 integrators + `apply_ball_motion` (4 sites vs 1), and in Physics the integrator is NOT the last pos writer (the snapshot write-back is), so "fused" would be wrong there without a 5th site. Pack-maintained is one site and provably correct in all modes.

**Trade-off**: the shuffle source is the *packed* pos, so anything writing `gpu.pos` mid-substep after the shared sync joins the prev chain — handled by D3.

### D3: Writer discipline made structural — single shuffle site + field-granular downstream writers

**What**: exactly ONE site writes `prev_pos` per substep (the shuffle in `sync_gpu_instance`) and exactly ONE site seeds it (`GpuInstance::new`). The two downstream writers stop doing full-struct rewrites:
- `sync_ball_gpu` (physics.rs:337-350): field writes `inst.pos / inst.scale / inst.color` — never `prev_pos`. (Keeps writing `pos` — same value, zero cost, removes a hidden ordering dependency.)
- `tint_collided` (physics.rs:362-372): single field write `inst.color = GpuInstance::pack_rgba8(COLLISION_FLASH_COLOR)` — its current read-modify-write of pos/scale is deleted (strictly fewer instructions).

**Why (★R1-1 corrected wording)**: recovers V6.1's isolation advantage **for the current writer set**, enforced BEHAVIORALLY (T3/T4 exact bitwise assertions) plus by doc invariant — NOT structurally: a future system writing `*inst = GpuInstance::new(pos, scale, color)` (the most natural-looking write, copied from today's `sync_ball_gpu`) would silently reset `prev_pos = pos` every substep (snap-to-pos for those rows), and `new()` must stay `pub` for the three spawn sites. The mitigation is the doc guard ON `new()` ITSELF (where a future author reads it): "**spawn-seed only — never call inside a per-substep system; per-substep writers use field writes or `with_prev`**". A new writer is a new fire site with zero test coverage (the 14b lesson) — the invariant doc + T3/T4 are the fence we have.

**Alternatives**: keep full-struct rewrites + `with_prev` everywhere — rejected: every future writer must remember to thread prev (the silent-bug class); field writes make forgetting impossible.

**Trade-off**: none measurable; `tint_collided` gets cheaper.

### D4: Layout = append `prev_pos` at the END; stride 16 → 24; `@location(5)`

**What**: `{ pos: [f32;2] @0, scale: f32 @8, color: u32 @12, prev_pos: [f32;2] @16 }`, size 24, align 4, no padding (Pod-compatible).

**Why**: appending preserves the byte offsets of every existing attribute — `VertexBufferLayout` locations 2/3/4 (render/mod.rs:132-151) and WGSL `@location(2..4)` are byte-identical; the diff is one new attribute at offset 16 + the stride constant. Const-asserts flip to 24 and keep working. 24 B = 2⅔ rows per cache line — irrelevant for a pure streaming pass.

**Alternatives**: prepend prev — rejected: shifts every existing offset for zero benefit. Pad to 32 B — rejected: +33 % bandwidth buys nothing (no random GPU-buffer indexing; wgpu needs no power-of-2 stride).

**Trade-off**: instance buffer 16 MiB → 24 MiB at the 1 M cap (device-local; trivial).

### D5: Upload gate = `steps > 0 || entity_count != last_uploaded_count`, once per display frame in `app.rs`

**What**: bind the already-returned substep count at app.rs:403 (currently discarded), then: gate fires → run the existing `upload_instances`, record `last_uploaded_count` + cache the count; gate holds → reuse `cached_instance_count` for the paint callback; NO column walk, NO `write_buffer`.

**Why**:
- `steps > 0` covers every in-schedule mutation: integration, mode transitions (the state pass runs only inside substeps), spawns/despawns-on-enter/exit, tint.
- `count != last` covers the only out-of-substep structural path: the click burst (app.rs:392-396, direct `create_entity` BEFORE `runner.step`). A burst on a 0-substep frame uploads seeded instances (prev = pos) rendering pinned at the spawn point — correct under any alpha. At-capacity clicks are no-ops (count unchanged ⇒ correctly no upload). Audit: no other out-of-substep column writers exist (no component insert/remove anywhere in the demo; the panel writes only resources).
- Skipping the upload also skips the display-rate `for_each_chunk` walk — 230 MB/s of CPU-side read traffic drops to the 64 Hz event rate.
- Correctness of NOT uploading: the GPU buffer still holds the last substep's exact `{prev, pos}`; only alpha (D7) changes between substeps — precisely the interpolation contract.

**Why impossible in the baseline**: without prev/alpha on the GPU, a skipped upload froze motion visibly between substeps — the current code uploads identical data every frame to hide this (the 56 % redundancy this phase deletes).

**Trade-off**: a `u64` + `u32` of app state and a two-condition branch per frame. Pause: while paused, sliders affecting the pack don't re-render — IDENTICAL to baseline (the pack doesn't run while paused either; baseline merely re-uploaded the same stale bytes). Documented.

### D6: NO engine changes — `last_fixed_substep()` common-condition NOT shipped

**What**: zero diffs under `crates/boyko_ecs` (gate G6).

**Why**: the condition's only use would be skipping the pack on non-final catch-up substeps — but D2 REQUIRES the pack every substep (it is the prev maintainer), and the upload already runs at most once per display frame (it lives in `app.rs` after the whole `fixed_advance` loop). The condition would optimize nothing here while adding engine surface plus the ★M3 footnote (predicate reads the live timestep, the loop runs on its entry snapshot). If a future phase splits prev maintenance from packing, revisit; the predicate IS expressible (`overstep < timestep` during a substep ⟺ last substep, fixed_loop.rs:77 — verified) and would follow the Phase-17 F1 shape (`impl System<Out = bool>`).

**Trade-off**: a rare catch-up frame (≥2 substeps; impossible at steady 144/64) packs color/scale redundantly on non-final substeps — at 30 Hz display that is 1-2 redundant 100 k-row streaming passes per frame, ~0.2 ms each. Accepted to keep the engine frozen.

### D7: Alpha delivery via the camera uniform, 64 B → 80 B; sampled post-loop in `app.rs`; pause = frozen alpha

**What**: `CameraUniform` gains `alpha: f32` + 12 B explicit padding (WGSL uniform struct size rounds to align(16): 64 + 4 → 80). `app.rs` samples `overstep_fraction()` AFTER `runner.step` (post-loop ⇒ `overstep < timestep` ⇒ alpha ∈ [0,1), pinned at fixed_time.rs:141-150), passes it as a new `Copy` field on `RenderCallback`; `prepare` writes it in the SAME 80 B `write_buffer` it already issues every frame. Zero additional GPU writes.

Pause: `overstep` freezes mid-value ⇒ alpha freezes ⇒ a static frame mid-lerp. **No snap.** Snapping alpha to 1 on pause would produce a visible forward jump on pause and a backward jump on unpause; freezing produces neither.

**Alternatives**: wgpu 29 immediates — feature-gated and unnecessary; a second tiny uniform — a second write_buffer + bind-group churn for nothing.

**Trade-off**: `min_binding_size` and the host const-assert move to 80; `ortho_fit` gains an alpha parameter (2 call sites). On no-upload frames the GPU lerps an unchanged `{prev,pos}` with fresh alpha — exactly the design.

### D8: No snap machinery at all (mode switch, despawn, spawn)

**What**: no "snap frame", no `copy_buffer_to_buffer`, no alpha override path.

**Why**: every hazard V3 needed snapping for vanishes structurally under D1+D2+D3: despawn moves prev with the row; spawns seed prev = pos; mode transitions happen inside a substep (⇒ steps > 0 ⇒ fresh upload of the fully-reseeded population). The teleport-reset class (Bevy's Changed-based easing reset, Godot's `reset_physics_interpolation`) collapses to "write prev = pos", which the seed funnel already does; the demo has no other teleports.

**Trade-off**: none. Deletes an entire policy axis from review.

## Data structures

```rust
// render/instance.rs — 24 B, no padding (6 × 4 B), align 4, Pod holds.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    pub pos: [f32; 2],      // @location(2), offset 0  — position after the LAST substep
    pub scale: f32,         // @location(3), offset 8
    pub color: u32,         // @location(4), offset 12 — packed RGBA8
    pub prev_pos: [f32; 2], // @location(5), offset 16 — position after the SECOND-TO-LAST substep
                            // INVARIANT: written ONLY by sync_gpu_instance's shuffle
                            // and the GpuInstance::new spawn seed (D3).
}
pub const GPU_INSTANCE_SIZE: usize = 24;
const _: () = assert!(size_of::<GpuInstance>() == GPU_INSTANCE_SIZE);
const _: () = assert!(align_of::<GpuInstance>() == 4);

// render/camera.rs — 80 B host mirror of the WGSL uniform (mat4 64 + alpha 4 + pad 12).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub alpha: f32,        // interpolation alpha ∈ [0,1); 1.0 in identity() (snap semantics)
    pub _pad: [f32; 3],    // uniform-layout padding; must stay zeroed (Pod)
}
pub const CAMERA_UNIFORM_SIZE: usize = 80;

// render/mod.rs
pub struct RenderCallback {
    pub viewport_px: [f32; 2],
    pub instance_count: u32,
    pub alpha: f32,        // sampled post-fixed-loop in App::ui (Copy — 'static-safe)
}

// app.rs — DemoApp gains (cold, per-frame-once):
//   last_uploaded_count: u64,   // u64::MAX at init ⇒ first frame always evaluates the upload
//   cached_instance_count: u32, // draw count reused on skipped frames
//   (probe, see Q7) upload events/s + MB/s counters for the panel
```

```wgsl
struct Camera {
    view_proj: mat4x4<f32>,
    alpha: f32,            // struct size rounds to 80
};
struct VertexInput {
    @location(0) corner: vec2<f32>,
    @location(2) inst_pos: vec2<f32>,
    @location(3) inst_scale: f32,
    @location(4) inst_color: u32,
    @location(5) inst_prev_pos: vec2<f32>,
};
// vs_main: let world = in.corner * in.inst_scale + mix(in.inst_prev_pos, in.inst_pos, camera.alpha);
```

## API deltas

```rust
// render/instance.rs
impl GpuInstance {
    pub const fn new(pos: [f32; 2], scale: f32, rgba: [u8; 4]) -> Self;        // UNCHANGED signature; seeds prev_pos = pos
    pub const fn with_prev(prev_pos: [f32; 2], pos: [f32; 2], scale: f32, rgba: [u8; 4]) -> Self; // NEW (the pack's constructor)
    pub const fn pack_rgba8(rgba: [u8; 4]) -> u32;                              // NEW (extracted from new())
}

// render/camera.rs
impl CameraUniform {
    pub const fn identity() -> Self;                                            // alpha = 1.0
    pub fn ortho_fit(viewport_w: f32, viewport_h: f32,
                     half_world_w: f32, half_world_h: f32, alpha: f32) -> Self; // gains alpha
}

// app.rs (private)
#[inline]
fn upload_due(steps: u32, entity_count: u64, last_uploaded_count: u64) -> bool; // pure, unit-tested

// sim systems: all public signatures UNCHANGED (bodies only).
// Spawn sites, bundles, runner wiring, engine: UNCHANGED.
```

## Algorithms for critical paths

**1. Pack (`sync_gpu_instance`, per substep, all modes)** — per row: load Position/Velocity (16 B seq), load old `gpu.pos` (8 B seq, same line being written), sqrt+clamp color ramp (unchanged), store 24 B. O(N); pure streaming SoA→SoA; `par_iter_mut` unchanged; branchless except the existing ramp. Budget ≤ 5 ns/row (G2). Model: ~40 B/row, L2/L3-streaming ≥ 30 GB/s ⇒ ~1.3 ns/row memory floor; sqrt+convert ≈ 2-3 ns.

**2. Upload (`App::ui` step 4, per display frame)** — gate: 2 compares, O(1), predictable (64/144 beat). Fired: existing for_each_chunk + cast_slice + write_buffer, stride 24. Skipped: zero work. Events/s = min(display, sim) + click/transition extras: 64/s at 144 Hz (was 144/s); 30/s at 30 Hz display.

**3. Alpha + camera (per display frame)** — one 80 B write_buffer (was 64 B), the same single call as today. O(1).

**4. Vertex shader** — +1 `mix` (2 FMA lanes) per vertex, 6 verts/instance; rasterization-bound; unmeasurable.

## Soundness

- **Zero new `unsafe`.** The Pod surface re-derived: `#[repr(C)]`, 24 B no padding (8+4+4+8), all fields Pod ⇒ derive holds. CameraUniform 80 B likewise (pad explicit zeroed field).
- **Aliasing**: pack reads and writes the SAME row's GpuInstance through one `&mut` — no new aliasing. Field-granular writers go through the same `&mut GpuInstance` query items as today.
- **No change-detection consumers of GpuInstance** (only `Changed<Velocity>` is used as a filter) — field writes vs struct rewrites carry no tick semantics (plain `&mut T` never bumps ticks in boyko).
- **Alpha invariant**: sampled strictly after `fixed_advance` returns ⇒ post-loop `overstep < timestep` (debug_assert fixed_loop.rs:84-87) ⇒ `overstep_fraction ∈ [0,1)`, upper edge pinned (fixed_time.rs:149).
- **Row identity across the lerp pair**: prev and pos share one struct in one buffer written by one write_buffer sequence ⇒ a drawn instance can NEVER mix two entities' endpoints (the formal disposal of V3's smear class).
- **Edge cases**: empty world (count 0 ⇒ paint early-return, mod.rs:299); count == MAX_INSTANCES (existing overrun guard + draw clamp unaffected; at-capacity click ⇒ count unchanged ⇒ no upload — correct); first frame 0 substeps (`last_uploaded_count = u64::MAX` forces one evaluation); paused frame (0 steps, count stable ⇒ skip; D7 freeze); mode switch while paused (transition queued, applies on first substep after unpause ⇒ steps > 0 ⇒ upload). Drop order: N/A.

## Integration

| Module | Change |
|---|---|
| `render/instance.rs` | 24 B struct, asserts, `with_prev`/`pack_rgba8`, invariant docs |
| `render/camera.rs` | 80 B uniform, `alpha`, assert, `ortho_fit` param |
| `render/mod.rs` | stride 24, attribute @5 offset 16, `min_binding_size` 80, `RenderCallback.alpha`, prepare passes alpha; buffer-size comment 24 MiB |
| `render/shader.wgsl` | `Camera.alpha`, `@location(5)`, `mix` in `vs_main` |
| `sim/systems/common.rs` | prev shuffle in the pack |
| `sim/systems/physics.rs` | `sync_ball_gpu` / `tint_collided` → field-granular writes |
| `app.rs` | bind `steps`, gate + cached count fields, alpha sampling, probe counters |
| `ui/panel.rs` | ★R1-3: probe line (upload events/s + MB/s — the only witness visible in the wasm deploy; Q7 KEEP ruling) |
| `sim/runner.rs`, `modes.rs`, `bundles.rs`, engine crates | **no changes** |
| `docs/SYSTEMS.md` / `FEATURE_MAP.md` | post-landing sync |

Existing-API breaks: `ortho_fit` arity (demo-internal, 1 prod call site) and `RenderCallback` field addition (demo-internal). Nothing crosses a crate boundary.

## Implementation plan (waves)

1. **W1 — layout + render path (atomic)**: instance.rs (struct, asserts, ctors + the ★R1-1 `new()` doc guard), camera.rs (80 B, alpha; ★R1-2: the degenerate-viewport `ortho_fit` guard returns `identity()` whose alpha is 1.0 — DOCUMENTED snap semantics for a zero viewport, nothing renders there), mod.rs (layout entries, stride, binding size, callback field, prepare), shader.wgsl (input, uniform, mix). One unit. **★R1-5 honest gate wording**: Rust-side mismatches fail at `cargo check`; the WGSL leg (`include_str!`) fails at RUNTIME pipeline/bind-group validation (loud, app start) — the W1 done-criterion is `cargo check -p boyko_demo --all-targets` PLUS one manual launch (or a naga-validation dev test if trivially available).
2. **W2 — pack + writers**: common.rs shuffle; physics.rs field-granular writers; struct invariant docs. Depends on W1.
3. **W3 — upload gate + alpha plumbing**: app.rs (`steps` binding, `upload_due`, cached fields, alpha sample, panel probe). Depends on W1; independent of W2.
4. **W4 — tests + bench + gates**: T1-T7, `benches/gpu_pack.rs` (criterion dev-dep, native-only), wasm32 check, full suite, manual visual smoke (3 modes × {motion smoothness, pause mid-motion, click burst, rapid mode switching, resize}).

## Metrics and validation

### Binding gates
- **G1 (upload-event cut)**: deterministic unit test T5 — synthetic 144/64 frame sequence (1000 frames, constant count): `upload_due` fires on exactly the substep-bearing frames; skip rate ≥ 55 %; plus burst/transition/pause rows.
- **G2 (pack perf, ★R1-4 calibrated form)**: criterion `benches/gpu_pack.rs`, 100 k rows, pack body mirrored on plain slices (pin-comment to common.rs; the mirror is chosen for ISOLATION of the 16→24 B delta, not noise — ★n7). **Measure the 16 B BASELINE mirror FIRST**: if baseline ≤ 3 ns/row, the binding gate is the absolute 24 B ≤ 5 ns/row; if baseline > 3 ns/row (divider-port-bound sqrt+divss body is machine-dependent), the binding gate becomes the RATIO ≤ 1.6× with the absolute reported informationally.
- **G3 (no regression)**: all 15 existing demo tests green, unmodified except where an assertion names `GPU_INSTANCE_SIZE`.
- **G4 (wasm)**: `cargo check -p boyko_demo --target wasm32-unknown-unknown` clean.
- **G5 (layout fence)**: const-asserts at 24/80 compile — the build IS the gate.
- **G6 (engine frozen)**: `git diff --name-only` for the phase ⊆ `crates/boyko_demo/**` + `docs/**`.
- **G7 (prev correctness)**: T3/T4 exact bitwise assertions pass natively (wasm shares the system bodies — native suffices).
- **Upload bandwidth number**: frame-time probe (panel: upload events/s + MB/s), checked in the manual smoke at ~64 events/s @144 Hz — informational; the BINDING form is G1.

### Test matrix
| # | Test | Asserts |
|---|---|---|
| T1 | unit, instance.rs | `new()` seeds `prev_pos == pos`; `pack_rgba8` == `new()`'s packing; `with_prev` field placement |
| T2 | unit, camera.rs | size 80; `identity().alpha == 1.0`; `ortho_fit` stores alpha verbatim on the NON-degenerate path (★R1-2: the zero-viewport guard returns identity ⇒ alpha 1.0, documented) |
| T3 | integration `tests/interpolation.rs` | Particles: step once (spawn), snapshot `gpu.pos` AND `Position` per row; step exactly one substep; ∀ rows `gpu.prev_pos` == prior snapshot BITWISE; ★n11: "moved" witnessed independently via `Position != pre-step Position` — for those rows `pos != prev` |
| T4 | integration, same file | Physics: queue `NextState(Physics)` (mirror mode_switch.rs harness), two substeps; ∀ ball rows prev == prior substep's pos bitwise ⇒ `sync_ball_gpu`/`tint_collided` did not re-shuffle/clobber prev; tint rows changed `color` only (★n10: assert via `GpuInstance::pack_rgba8(COLLISION_FLASH_COLOR-value)` re-derivation or make the const `pub(crate)`) |
| T5 | unit, app.rs | `upload_due` table: (0, eq)→false; (≥1, _)→true; (0, neq)→true; synthetic 144/64 sequence skip-rate ≥ 55 %; paused → all false; burst-on-0-substep → true exactly once |
| T6 | integration | spawn seed: a click-path-shaped spawn (direct create_entity mirror) has prev == pos before any substep |
| T7 | proptest (light) | random substep/idle sequences: "prev equals pos-of-previous-pack" holds after every pack |
| — | debug_asserts | none new on the hot path (the per-row invariant is O(N) — enforced by tests, not asserts) |

## Critic Round 1 — resolutions (REVISE → folded; mechanism verified correct)

The critic re-verified every anchor and re-derived the D2 shuffle in all 3 modes × both targets (incl. the Physics trick case — `sync_ball_gpu` is the last pos writer of substep N−1 and writes exactly the rendered endpoint; and the transition substep — fresh rows seed prev=pos then integrate the SAME substep ⇒ first render lerps spawn→first-position, no flash). D5 gate completeness audited exhaustively (no out-of-substep column/order mutation without a count change exists today; `target_count` is consumed by nothing). D7 layout verified (80 B; a host-side miss fails LOUDLY at pipeline creation). The paused-slider baseline-identical claim VERIFIED correct. Folded: ★R1-1 (D3 enforcement is behavioral not structural; `new()` doc guard "spawn-seed only"; Q6 record carries the corrected claim), ★R1-2 (degenerate-viewport alpha = identity 1.0, documented; T2 scoped), ★R1-3 (ui/panel.rs Integration row; Q7 probe KEPT — the only wasm-visible witness), ★R1-4 (G2 baseline-first calibration; ratio-primary if baseline >3 ns/row), ★R1-5 (W1 gate = check + manual launch; WGSL fails at runtime validation, not build). Notes: n6 (the shared sync is LOAD-BEARING in Physics as the prev maintainer — record in system docs; a future "optimization" gating it out of Physics would kill prev), n7 (G2 mirror chosen for isolation, not noise), n8 (_pad comment rationale = deterministic uniform bytes, not Pod), n9 (ortho_fit has ONE prod call site), n10/n11 (T3/T4 spec pins). Q-rulings: Q1 field writes AGREE (+ the R1-1 fence), Q2 keep the pos write (the cheap half of the n6 coupling), Q3 none found, Q4 mirror accepted (reason corrected), Q5 frozen alpha accepted (the alternative is incoherent — scale lives in the column), Q6 accepted with corrected wording, Q7 keep probe, Q8 keep the 1M cap (mobile-wasm note filed).

## Open questions for the critic (original, superseded by the resolutions above)

1. **D3 field-granular writers**: agree that `inst.color = ...` field writes through `&mut GpuInstance` are the right structural guard, given GpuInstance has no change-detection consumers? Any objection to `tint_collided` dropping its pos/scale read-modify-write entirely?
2. **`sync_ball_gpu` keeps writing `pos`** (defensively, same value the shared sync wrote) vs skipping it to save 8 B/row over 4 k balls — plan keeps it for ordering-independence; confirm or strike.
3. **Upload-gate count proxy**: the audit found click-burst as the only out-of-substep structural mutation. Does the critic see another path that mutates the GpuInstance column or row order without changing entity_count and outside a substep?
4. **G2 bench shape**: pack body mirrored on plain slices (clean isolation, frozen copy of the loop) vs driving the real system through a schedule (dispatch noise swamps the signal). Plan picks the mirror + a comment pinning it to common.rs; acceptable, or require both?
5. **Pause = frozen alpha** (static mid-lerp frame; pack-affecting sliders don't re-render while paused — baseline-identical): accepted, or force an upload on `params changed && paused` frames (would re-render stale-pos with new scale — arguably MORE confusing)?
6. **V3 dual verdict** (demo-sound by population invariant, engine-disqualified as compiles-but-lies): accept as the recorded rationale so the variant is not re-litigated when the engine grows a first-party render mirror?
7. **Panel probe** (upload events/s + MB/s in FrameStats): worth the churn, or demote to a debug-only log line and keep G1 as the sole witness?
8. **24 MiB instance buffer at the 1 M cap on WebGL2**: under desktop-browser limits, but should the wasm build drop MAX_INSTANCES to 256 k (6 MiB) as a mobile courtesy? Plan says no (out of scope, cap untouched) — confirm.
