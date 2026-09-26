# Visual effects (explosions, rain, weather): the design space for this engine

> Status: architect's design, 2026-09-25, **pass 2**: revised after critique pass 1
> (CHANGES_REQUESTED, 0 critical, 7 important, 9 optional). §12 is the review log, recording how
> each remark was closed. Trunk `6394bc5e`, read
> through the `u/research-0925` worktree. It is a **delta over the particle plan**
> ([`../PARTICLES-PLAN.md`](../PARTICLES-PLAN.md), Rev 4, approved). §0 states what that plan and
> its neighbours decided, what this document leaves untouched, and what it changes and why.
>
> The survey it rests on is [`VFX-RESEARCH.md`](VFX-RESEARCH.md). Every number is tagged:
> - **[M]** measured in this tree (rig named);
> - **[P]** primary published;
> - **[S]** secondary;
> - **[E]** estimate, with inputs shown.
>
> The reference GPU is the owner's **RTX 3060 Laptop** (48 ROPs, 3 MB L2, 288–336 GB/s, research §2).
> Nothing was timed for this document and no `cargo` command was run. Repository citations are a
> path plus a symbol, each re-opened at `6394bc5e`. This directory is not in `GATED_DOCS`.
>
> §10 separates the PERF/ARCHITECTURE decisions taken here (with the number that decides each)
> from the VALUES/SCOPE questions that go to the owner.
>
> **Boundaries with the sibling threads of the same date.** Post-processing, anti-aliasing, sun
> rays and volumetric fog are separate research threads. This document consumes them only through
> named seams:
> - the HDR scene-colour move;
> - the froxel fog volume (`E-FOG`, [`../OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md));
> - the opaque colour chain (transparency R4);
> - the TAA coverage mask (transparency R6).
>
> The audio hook's consumer is surveyed in `docs/audio/AUDIO-RESEARCH.md`. Three same-day sibling
> designs bear directly on this one:
> - `docs/render/POSTFX-AA-DESIGN-SPACE.md` adopts the HDR move as its first rung **PX1**;
> - `docs/render/VOLUMETRICS-DESIGN-SPACE.md` owns the fog grid, particle density injection, hero
>   dense volumes and the `-D PARTICLE_FOG` draw axis. Its D18 has already decided that axis: one
>   fetch per vertex at the particle centre;
> - `docs/render/DYNAMIC-MATERIALS-DESIGN-SPACE.md` (with its survey
>   `docs/render/DYNAMIC-MATERIALS-RESEARCH.md`) places rain wetness in its rung **DM2**: a `Wetness`
>   driver curve and a `SurfaceWetness { level }` resource. It asks its owner question Q5 about
>   "the rain split". §2.4 below states which half of wetness each design owns. That split is
>   **one owner question shared by both documents**, and this document does not answer it
>   unilaterally.
>
> All of them are named by path and not linked, because they are not yet on the trunk. Their rungs
> are numbered `PX*`, `V*` and `DM*`; this document's rungs are `FX*`, so the ladders cannot be
> confused.

---

## 0. What the earlier documents decided, what is unchanged, what changes

### 0.1 Decided earlier

- **[`../PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) Rev 4:**
  - **D1** particles are not entities; emitters are.
  - **D2** records: 48 B sim + 32 B render.
  - **D3** dead list, dual alive lists and a one-thread kickoff; no concurrent push/pop on the dead
    stack; emit+sim fusion rejected outright.
  - **D4** split indirect blocks.
  - **D5** wave-aggregated atomics.
  - **D6** own 64 Hz clock.
  - **D7** composite into `lit` with a per-path compare op and depth contract.
  - **D8** the emit prefix orders lanes only.
  - **D9** SDF collision with a Lipschitz skip.
  - **D10** additive is unsorted; alpha class plus radix sort; R10 (sort ⇒ no motion vectors).
  - **D11** lit particles are lit per particle, in the sim, from the froxel lists.
  - **D12** one generator owns each whole `.hlsl`.
  - **D13** default-off on three axes.
  - **D14** capacity frozen at boot.
  - **D15** release-present clamps on fixed tables.
  - **D16** emitter hot/cold mix accepted.
  - **D17** subsystem containment.
  - Landed: E, P0, P1, P1b and P2 items 1–3. Designed, not landed: P2 item 4 (soft particles,
    with its own D13–D17: pushed depth descriptor, two FS variants, view-space Z, an 84 B push
    range, a **global** `fade_distance`, an orthographic sentinel).
  - Planned: P2b (interpolation, a compile-time variant), P3 (lit particles + motion vectors),
    P4 (trails, ribbons, mesh particles), P5 (measured micro-optimisations).
  - Owner questions open: HDR (OQ1), `CAP` (OQ2), `MAX_EMITTERS` (OQ3), serialization,
    P4 scope, the determinism waiver, step rate, substep ceiling.
- **[`../PARTICLES-RESEARCH.md`](../PARTICLES-RESEARCH.md):** the industry skeleton; AoS records;
  radix, not bitonic; SDF collision beats depth collision; the effect lives on the CPU and the
  particle on the GPU.
- **[`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) (pass 1):**
  - The seam: after the path's last `lit` producer, before `taa_resolve`, particles drawing
    after the translucent meshes.
  - **R3**: sampled depth, shared with `-D SOFT`, on the same push-descriptor enable.
  - **R4**: the opaque colour mip chain.
  - **R5**: `-D REFRACT` thin transmission.
  - **R6**: `taa_cov`, written by `particle_draw` too.
  - **R7**: MBOIT merges alpha particles with premultiplied meshes; additive draws last.
  - **R9**: `csm_trans` translucent shadows.
  - **R11**: HDR as a coupled owner decision.
  - **D18**: `particle_draw.fs` bounded at 12 variants (`DEPTH_LINEAR × SOFT × OIT_STAGE`).
  - **TK-1**: growth of the `BlendFactor` enum.
- **[`../OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md):**
  - `E-PART` (particles + SDF collision + fog density injection).
  - `E-FOG` (a 160×90×64 froxel fog "whose XY×Z IS the cluster grid").
  - `E-GOD` (god rays; route a is free from `E-SDFVOL`), `E-SDFVOL`, and `D-FWD`.
- **Same-day siblings (not yet on the trunk):**
  - The post/AA design takes HDR `lit` as rung **PX1** and answers the particle plan's OQ1 through
    it, pending its own owner question.
  - The volumetrics design replaces `E-FOG`'s "fog grid == cluster grid" with a fog-owned
    160×90×64 grid. It schedules particle density injection (its **V10**), dense local volumes
    for hero explosions (its **V12**), and a per-vertex fog fetch in the particle VS behind
    `-D PARTICLE_FOG` (its **V8**). That fetch is decided there as VS-only, at the particle
    centre (its D18).
  - The dynamic-materials design owns per-material parameters. Its DM2 carries a `Wetness` curve
    and the `SurfaceWetness { level }` resource. Its F10 records that transparency's
    `MaterialXGpu` lanes "are full, and the opaque producers never bind it". It puts per-pixel
    material parameters in a cold `MaterialDynGpu` table (48 B per row), whose `rim` word 3 is
    `_reserved`.

### 0.2 Unchanged

- D1–D10, D12–D15, D17.
- Every landed rung and its pins.
- P2 item 4's design, except where C1 below moves one field.
- P2b, P3's D11 for its class, P4 and P5.
- The transparency seam, R3/R4/R5/R6/R7/R9, and the additive-last invariant.
- `E-FOG` as the owner of any fog density injection.

This document adds rungs; it does not reopen the skeleton.

### 0.3 Changed, and why

| # | Change | Why (the number) |
|---|---|---|
| **C1** | The draw binds a small **per-effect draw table** (`EffectDrawGpu`, 48 B × `MAX_EFFECTS` = **12 KB**). `fade_distance` moves into it, so it becomes **per-effect**. This supersedes P2-item-4 D16's "global" ruling. The table also carries the flipbook grid and guard band (§2.14) and the coverage bound (§2.13) | D16 refused a per-effect fade because the draw could not reach the effect table, so the only route was **+4 B per particle (+3.1 % of sim traffic)**. Flipbooks need per-effect grid dimensions in the draw anyway (§2.1). Once the table exists, a per-effect fade, clamp or band costs **0 B per particle** |
| **C2** | `ParticleRender.flags` is repacked: `u8 effect_index \| u8 frame \| u8 frame_frac \| u8 render_bits` | The sim writes `p.effect_flags >> 16u` there, and **no draw stage reads it** [T]. `MAX_EFFECTS = 256` needs exactly 8 bits. **0 B per particle** |
| **C3** | **Fire-and-forget bursts** (`ParticleBursts`, a `Resource` over a `ScratchColumn`) join emitters as a second input to A1, for **per-contact** one-shots | Per-contact sparks arrive at tens per frame, each at its own contact point and normal. A burst row costs ≈ 10 ns. An emitter entity per contact costs a structural spawn and despawn (30–100 ns each [M-derived, plan D1]). For an explosion the two are equivalent: it spawns ~19 entities anyway, and `ParticleEmitter::burst` already exists [T]. Pass 1's "persistent slot" argument is withdrawn (§2.12). D1 is unchanged |
| **C12** | **A1 skips zero-count requests.** An enabled emitter that spawns nothing this frame pushes no row | Today every enabled emitter pushes a row [T], so 256 idle emitters starve a 257th that wants to spawn. After the change, `MAX_EMITTERS` bounds *spawning requests per frame* (emitters and bursts alike). **0 B**, one compare per emitter |
| **C13** | `ParticleSim.color_rgba8` (offset 32) becomes **`life_total: f32`**, written by emit. The colour is evaluated from the effect's ramp every step | Once the ramp is evaluated from the keys, the stored colour is dead state (it was key 0 held for life [T]). The flipbook's `fps` mode needs age in seconds, and the record holds only f16 `inv_life`. So `age = life_total − life_remaining` replaces a divide. **0 B per particle** |
| **C4** | **GPU events** (spawn-on-death / on-collide) arrive **next frame** through a double-buffered event ring. The kickoff gains an `EVENTS` variant | D3 forbids same-pass spawning. P2 item 3 kept one kickoff module for every arming; that ends **only** for the `EVENTS` arming (§2.8) |
| **C5** | A **second lighting family**: six-way smoke cards are lit **per vertex in the VS**. D11 (per particle, in the sim) stays for the many small particles | Six-way needs 6 × RGB of incoming light, and the 32 B record has no room. Cards number in the tens to low hundreds, so 4 evaluations per card is noise (§2.5) |
| **C6** | **Rain and snow are not pool clients.** They are a stateless analytic draw | −0.16 ms and −12.1 MB at 131k drops versus pool rain; no pool contention; deterministic by construction (§2.2) |
| **C7** | The collision axis gains a **top-down height map** arm (`Height`, `SdfHeight`) beside `Sdf` | The mesh leg has no GPU collider. The map already exists for rain occlusion (§2.3). Depth-buffer collision stays rejected (research R6) |
| **C8** | **In-sim frustum culling**: a culled survivor takes no render index | The plan has none (zero `cull`/`frustum` in `emit_particles.rs` [T]). It saves a 32 B render write and 4 VS invocations plus clipping per culled particle: **≈ 0.013–0.09 ms** at 100k with half culled [E] (§7.5; the upper end assumes a culled quad costs as much as an on-screen one, which overstates it) |
| **C9** | One `PARTICLE_FX` axis on the draw with **three values** {off, `FLIP`, `SIXWAY`}. Flipbook, MV flipbook and six-way are per-effect runtime modes inside it; `SIXWAY` is legal on the alpha class only | Transparency D18's bound of 12 goes to **≤ 36**, not **96** (§6.3). The third value exists so a flipbook-only arming never pays six-way's 18 extra interpolants per vertex |
| **C14** | A **per-effect coverage bound** in the draw VS: a screen-size clamp (a fraction of viewport height, Unity's "Max Particle Size") and a near-camera fade that culls inside `near` | A count budget cannot see coverage. Near the camera, 64 E1 cards would cost 3.6–13.2 ms at 1080p unbounded; at the provisional default clamp (0.35) they cost 0.25–0.91 ms (§7.1). 3–5 ALU per vertex, no divide |
| C10 | The plan's F13c is stale: `Cf` now carries `sin`, `cos`, `rsqrt` and `vec3_dot` [T] | Informational. D6's host precompute and the no-divide rule still hold for bit-exact leaves |
| C11 | The research report's "`TextureDimension::D2Array`" does not exist (`{D2, D3}` only [T]) | Flipbooks are 2D atlases in the bindless `Texture2D gTextures[]`. The UI flipbook (`UiSheet::frame_uv`) has the same shape but is unmipped, so a mipped sheet needs its own containment (§2.14) |

---

## 1. The shape in one paragraph

Effects are **three layers on the shipped particle skeleton, and none of them is a new data
system**.

**The CPU layer** is a new optional crate, `boyko_vfx`, holding two plugins:
- `VfxPlugin` turns one-shot recipe entities (`ExplosionFx`) into work on existing subsystems:
  burst requests into the particle pool, a `PointLight` with a fade, `RigidBodyBundle` debris, a
  box decal, trauma on the camera's `CameraShake`, and an optional audio cue.
- `WeatherPlugin` owns weather state and arms the weather passes.

Neither plugin keeps a side store: durable state is components and resources, and staging is
`ScratchColumn`.

**The particle layer** grows within the pool, not beside it:
- a colour ramp and flipbooks (with motion-vector interpolation) at 0 B per particle, through a
  per-effect draw table. Sheets carry a guard band and a truncated mip chain, so no frame bleeds
  into its neighbour at any level;
- six-way lit smoke cards lit per vertex;
- a per-effect coverage bound (screen-size clamp and near fade), so near-camera overdraw has a
  priced worst case;
- next-frame GPU events;
- in-sim frustum culling;
- a height-map collision arm next to the SDF one.

**The weather layer** is the one place the pool is deliberately not used:
- Rain and snow are a **stateless analytic draw**. Drop `i`'s position is a pure function of
  `(i, host-wrapped phase, camera cell)`, so there are no records, no atomics, no emit churn and
  no pool capacity.
- Drops are culled against a **top-down occlusion map** with three producers:
  - a depth-only ortho raster of the *static* mesh casters, re-run on re-centre;
  - a toroidal SDF height march;
  - a small per-frame raster of the *dynamic* casters (bodies, and anything marked `RainDynamic`).

  Consumers take the maximum. A coarse far level covers surface exposure out to ±128 m.
- The same map drives splash placement, spark collision with mesh floors, and the exposure term
  of a `-D WEATHER` leaf spliced into every lit producer. That leaf applies wetness, puddles and
  snow cover **once, per pixel**. The global wetness level has one home, `SurfaceWetness`, shared
  with the dynamic-materials design, and per-material porosity is that design's datum (§2.4).
- One analytic wind leaf, authored on `FieldScalar`, gives the particle sim and the CPU soft-body
  solver the same function. Global wind moves the rain; vorticle entities add local swirl.

Everything is **structurally absent when off**:
- no plugin, no system;
- no config armed, no pass, no image, no variant loaded;
- an effect without a lane pays one uniform branch in an armed variant, and a gate measures that
  branch.

---

## 2. The forks, scored against the owner rules

Rule key:
- **R1** web-backed practice;
- **R2** the faster candidate for *our hybrid scene* wins, with numbers;
- **R3** Principle 0, structural capability, zero cost when off;
- **R4** no heap, locks or dynamic dispatch on the hot path;
- **R5** in-house;
- **R6** eDSL leaves, manifest rows, RHI gaps.

All millisecond figures use research §2's fill model unless marked. That model has two regimes:
- an overdrawn region that stays **on chip** runs at up to the ROP peak, 66.6–81.7 Gpx/s;
- a region that does **not** fit streams through DRAM at an effective 15.1–42 Gpx/s.

A compact explosion at 1080p plausibly sits in the first. Full-screen layers and 4K sit in the
second. Ranges below span both unless the regime is named. FX0 measures which one applies.

### 2.1 Fireball and smoke core

| Candidate | GPU, the card draw of one reference explosion (§7.1), 1080p / 1440p / 4K | VRAM | Rules |
|---|---|---|---|
| **A. Flipbook + motion-vector interpolation; six-way lighting for smoke** (Killzone 2 → Lozar, Unity, Star Citizen) | texture-bound (on chip) to fill-bound (off chip): **0.29–1.05 / 0.51–1.87 / 1.14–4.2 ms** [E] | 72.7 MB RGBA8 mipped, 22.4 MB with BC7 (§7.6) | R2 ✔ (pays per covered pixel, never per voxel). R3 ✔ (per-effect mode; one draw axis). R5 ✔ (sheets are baked offline by tools, not a runtime dependency). R6: leaves in §6; no RHI gap |
| A′. Baked 3D density playback (UE Heterogeneous Volumes: OpenVDB → Sparse Volume Texture, ray-marched) | per pixel it is C's march (0.15–0.3 ms per 15 % of 1080p [E]), plus a self-shadow march; no sim | a 64³ × R8 density frame is 0.26 MB, so 64 frames are **16.8 MB per effect** [E]. Sparse storage lowers that, and needs a sparse-texture path the RHI does not have | R2 ✗ against A: the same per-pixel march as C, plus flipbook-scale VRAM. Epic ships it as "Experimental" and says streaming animated frames "is not recommended for real-time playback" [P]. R5: an OpenVDB importer is a third-party format. Recorded as the volumetrics sibling's hero option (its V12), not as a gameplay core |
| B. Real-time 3D gas, ray-marched (Niagara Fluids class) | not published by Epic ("higher memory and GPU cost" [P]); a 64³ × 5-channel f16 grid is 2.6 MB per instance [E] | 2.6 MB per instance plus sim buffers | R2 ✗: Epic scopes it to "hero effects or cinematics" [P]. It needs multi-pass sim stages and a ray-march pass nobody else uses |
| C. Procedural ray-marched noise density (an eDSL leaf) | 15 % of screen × 48 steps × ~110 flops ≈ 1.6 GFLOP → **0.15–0.3 ms** at 1080p, ×1.78 / ×4 at 1440p / 4K [E], with no lighting; a self-shadow march multiplies it | ≈ 0 | R2: 3–6× A's cost per covered pixel for parallax that cards fake; unlit without a second march. It belongs with the volumetrics campaign's march leaves |
| D. Particle density injected into froxel fog (UE Volume domain, Frostbite) | shares `E-FOG`'s cost (UE: 1 ms PS4, 3 ms GTX 970 for the whole fog [P]) | `E-FOG`'s | Low resolution; soft only. Blocked on `E-FOG`, which is not built |

**Decided: A.**
- Its cost scales with covered pixels, and every alternative with the same look costs more per
  pixel (C) or needs an unbuilt subsystem (B, D).
- A′, C and D are *additions* for hero shots and soft volumes, not replacements. All three belong
  to the volumetrics sibling design: its V12 (dense local volumes for hero explosions) covers A′'s
  and C's role, and its V10 (particle density injection) is D.
- The fire core is emissive and **additive-class** (order-independent, unsorted). Smoke is
  **alpha-class** (sorted today, MBOIT under transparency R7).

### 2.2 Rain and snow streaks

| Candidate | Heavy (131 072 drops), 1080p / 1440p / 4K | Moderate (32 768 drops), 1080p | VRAM | Rules |
|---|---|---|---|---|
| R-A. Drops as pool particles (emitter at the camera box's top face; SDF + height collision; Wicked's in-place splash) | draw **0.26–0.38 / 0.28–0.44 / 0.32–0.56 ms** + sim **0.16 ms** (formula [M-derived]) | ≈ 0.07–0.1 ms + 0.05 ms sim | +12.1 MB pool share (131 072 × 92 B) | R3 ✗ in spirit: weather competes with explosions for `CAP` and `MAX_EMITTERS` (research pitfall 17) |
| **R-B. Stateless analytic draw.** Drop `i` = hash-seeded world-lattice position advected by gravity and the *global* wind integral, wrapped into a camera-cell box; occlusion-map fetch in the VS; splashes as a second stateless sprite set | **0.26–0.38 / 0.28–0.44 / 0.32–0.56 ms**, no sim | **0.07–0.1 ms** | **0 B** for drops (§7.2) | R2 ✔ (same draw, no sim, no memory). R3 ✔ (a plugin; nothing in the pool). R4 ✔. R6: one generated VS family (§6) |
| R-C. Layered cylinder textures (Remember Me) | 2 full-resolution + 2 quarter-resolution layers ≈ 5.2 Mfrag → **0.15–0.42 / 0.27–0.75 / 0.6–1.7 ms**, **flat in intensity** [E; "Timings are the same whatever the strength of the rain" P] | same as heavy | a few textures | R2: ties R-B at heavy 1080p and loses everywhere below. "Lack depth" and cannot respond to local wind or lights (Tariq [P]) |

**Decided: R-B.**
- R-A pays the same draw plus 0.16 ms and 12.1 MB of sim for nothing rain needs: rain needs no
  per-drop history.
- R-C is constant-cost, which is its virtue at storm density and its defect at every lighter
  setting. R-B scales with intensity and gives 3D parallax.
- **Rain sees only the global wind, in position and in orientation.** Stateless drops cannot
  follow *local* vorticles, whose integral has no closed form. They follow the host-accumulated
  global wind integral, and each streak is oriented along `fall velocity + global wind velocity`.
  Pass 1 oriented streaks by the local sample, which contradicted §2.7 and would have slanted a
  streak one way while it moved another. Local swirl reaches particles and soft bodies, not rain.
  Stated as a limitation.
- Snow is the same draw with a flake VS arm (sway from `sin`, available on `Cf`).
- **Rain and splashes are additive-class**: order-independent, so they join the additive proof and
  draw last.
- **Snow is premultiplied-alpha ("over"), unsorted.** Additive white flakes would vanish against a
  bright overcast sky in the 8-bit post-tonemap `lit`, where the sum clips.
  - Over-compositing of layers that all share one colour `c` is order-independent:
    `bg·Π(1−αᵢ) + c·(1 − Π(1−αᵢ))`.
  - Flakes share one per-frame colour (the flake arm lights them uniformly), so no sort is needed.
  - Snow draws after the sorted alpha particles and before the additive tail. That keeps
    transparency §2.3's "additive draws last" invariant.
  - The cost of not sorting is stated: a flake behind a smoke card but drawn after it appears in
    front of it. Flakes are small, and this is accepted.
- **Lightning is not designed.** No primary source was retrieved (research §7.6). The in-tree
  pieces compose it: a flash through the light table (as the explosion flash), a bolt ribbon
  (plan P4), and a sky pulse owned by the post/sky threads. Scope question B12.

**Two details decided with the rung (FX9):**
- **The world lattice, not the camera.** Positions wrap modulo the box in *world* space, anchored
  to the camera's cell, so drops do not swim when the camera moves (NVIDIA re-spawns drops "that
  have fallen out of bounds" [P]; a lattice wrap is the stateless form of the same idea).
- **The phase is host-wrapped.** `Time::elapsed()` is reduced modulo the fall period in f64 before
  the push. An f32 phase at 24 h would jitter drops by 7 cm (research pitfall 15).

### 2.3 Mesh-leg collision and occlusion

| Candidate | Mesh leg | SDF leg | Cost | Rules |
|---|---|---|---|---|
| SDF field (P1, shipped) | ✗: no mesh→SDF bake exists [T] | ✔ | shipped | — |
| Previous-frame depth buffer (Wicked) | on-screen only | ✔ (depth holds SDF hits) | 3 fetches per particle-substep; **two decode arms** (Deferred euclidean, reverse-Z elsewhere) and a cross-frame depth seed per path | R2 ✗ for rain (off-screen drops vanish); R3 ✗ (per-path state inside a path-independent sim) |
| **Top-down occlusion map, three producers:** a depth-only ortho raster of the *static* mesh casters (on re-centre) + a toroidal SDF height march (on re-centre) + a per-frame raster of the *dynamic* casters; consumers take the max | ✔ floors and roofs, including vehicles and bodies that move; blind to overhangs | ✔ | 1024² D32 (4.19 MB) + 1024² R32F (4.19 MB) + 512² D32 dynamic (1.05 MB). Static raster ≈ one CSM cascade's worth of the same 64 m of casters, once per re-centre (comparable: **0.067 ms** for one 160 k-vertex model over all cascades, [M] [`../diagnostics/W2208.md`](../diagnostics/W2208.md), `ZONE_GBUF_CSM_DEPTH` — a one-model scene, not a bound). SDF strip march ≈ **0.06–0.11 ms** per re-centre (1024 × 256 texels × 12 steps × 240 flops = 755 MFLOP at 6.9–13.1 TFLOPS [E]). Dynamic layer **≈ 0.01–0.06 ms per frame** [E, below] | R2 ✔: one map serves rain culling, splashes, spark floor collision, wetness and snow exposure. R3 ✔ (armed at boot by `WeatherConfig` or by `ParticleConfig::collision` including `Height`, §2.13) |
| Mesh-SDF bake | ✔ | ✔ | offline bake + atlas VRAM; no code exists | the long-term answer; scope question B7 |
| Physics colliders as an analytic table (Godot's ≤ 32) | dynamic bodies only | n/a | ≤ 32 × ~30 flops per particle-substep ≈ 0.01 ms at 100k [E] | ✔ as an opt-in marker on bodies (`ParticleCollider`) |

**Decided: the three-producer map** (FX7), plus the analytic table as an opt-in (FX8), plus a far
level for surface exposure (FX10).

**Update policy of the static layers: re-centre in quarter-box steps.**
- The box is 64 m, so the map resolution is 6.25 cm per texel, better than Remember Me's 7.8 cm.
- The static mesh raster re-runs whole on a re-centre.
- The SDF producer re-marches only the newly exposed quarter strip into a toroidally-addressed
  image.
- At 10 m/s a re-centre happens about every 1.6 s. The static layers cost **≈ 0 per frame
  amortised**, plus one update-frame spike priced by FX7 on a many-caster scene [E].
- Godot's follow-camera mode updates on every move and is rejected for exactly that cost [P].
- **The static layers hold only what cannot move.** The SDF edit list is boot-static today
  (`sdf_edit.rs` module doc: "the gather runs once"), so the SDF producer's world data never
  changes. The mesh producer draws `With<ShadowCaster>, Without<RainDynamic>`: the CSM's own
  structural caster set, minus the dynamic marker.

**Dynamic casters: a per-frame layer, selected structurally.** Pass 1 put every CSM caster into
the re-centre-only raster. A vehicle parked at re-centre time then stayed in the map after it drove
away, leaving a dry, rain-free ghost that FX8 sparks bounced off. A vehicle that arrived was
rained through.

The policy follows shipped practice:
- Remember Me's map cost is "mainly dominated by character" [P], so dynamic casters are in its map.
- Far Cry 6 handles dynamic objects separately, with ray casts [P].

This design keeps one representation and splits it by update rate:
- **`RainDynamic`** (a marker in `boyko_render`, beside `ShadowCaster`) puts a caster in the
  dynamic layer.
- `WeatherPlugin` inserts it once, on `Added<RigidBody>`, for entities that are also
  `ShadowCaster`s. That covers vehicles, bodies, doors (kinematic bodies carry `RigidBody` [T]) and
  explosion debris. It is a one-time structural insert, not a per-frame check.
- A mover **without** a body (an animated transform) must carry the marker by authoring. A
  developer-build validation system reports a static-layer caster whose `GlobalTransform` changed
  after the last static raster, and costs 0 in release. That is the one remaining route to a ghost,
  and it is named, not silent.
- **The dynamic layer is a 512² D32 image over the same 64 m box** (12.5 cm texels). It is cleared
  and re-rasterised every frame from `With<ShadowCaster>, With<RainDynamic>`.

**Price of the dynamic layer [E]:**
- The clear is 1.05 MB of writes, about 0.003–0.009 ms.
- Raster: the W2208 comparable is 0.067 ms for 160 k vertices × 4 cascades, about 0.1 ns per
  vertex-pass. Eight vehicle-class casters at 50 k vertices each cost ≈ 0.04 ms.
- Total **≈ 0.01–0.06 ms per frame** for 1–10 such casters.
- Consumers read one more texel: +1 fetch.

The dynamic layer is declared only when the weather is armed, and empty frames still clear
(structurally present while armed, absent when off).

**Outside the maps: a stated fallback, not whatever the fetch returns.**
- **Drops and splashes stay inside the near map by sizing.**
  - The near map is ±32 m around a centre that lags the camera by at most one quarter-step
    (16 m), so it always covers ±16 m around the camera.
  - The rain box is anchored to the camera's lattice cell, so it can sit up to one cell off the
    camera. FX9 sizes it so that box width + one lattice cell ≤ 32 m.
  - The VS kills any drop whose map coordinate still falls outside [0, 1], Remember Me's rule ("If
    we are out of shadowmap, just kill the pixel" [P]). FX9 tests that kill.
- **Surface exposure uses a far level.** A 512² D32 static raster plus a 512² R32F SDF march over
  a 256 m box (50 cm texels) cover ±128 m. Both re-centre in 64 m quarter-steps, about every 6.4 s
  at 10 m/s. Far updates never share a frame with a near update: when both fall due, the far one
  waits one frame, which its 64 m of slack absorbs. The far level has no dynamic layer, so
  sheltering by moving objects beyond the near map is not modelled.
  - Cost: +2.1 MB. Per pixel, a near-or-far select, which is spatially coherent, so almost
    wave-uniform. The far SDF march is 512 × 128 × 12 × 240 = 189 MFLOP ≈ 0.015–0.03 ms per far
    update [E]. The far static raster is priced by FX10 as a ratio of `ZONE_GBUF_CSM_DEPTH` on the
    same scene. It draws casters in a 256 m box, i.e. at most one far cascade's worth.
  - The alternative, "exposure = 1 beyond ±32 m", wets the floors of distant tunnels and the roads
    under distant bridges. That is the artifact the critique named, and any view down a road meets
    it within 32 m. The far level costs 2.1 MB and a rare update to remove it.
- **Beyond ±128 m, exposure = 1** (fully exposed). This is a stated artifact: a road under a bridge
  more than 128 m away reads wet. The far box is the lever if a scene needs more, at 4× the far
  VRAM per doubling.
- **Sparks beyond the near map** get no height collision. The SDF and analytic arms still apply.

**Consumers read one texel per layer and take the max**, instead of a merge pass: +2 fetches per
near consumer, against a 4.19 MB merged image and a pass.

### 2.4 Where surface effects apply (wetness, puddles, snow cover, scorch)

| Candidate | Paths × legs | Cost when off | Cost when armed | Rules |
|---|---|---|---|---|
| **W1. An eDSL leaf in every lit producer under `-D WEATHER`** (`deferred_pbr`, `forward_opaque.fs`, `sdf_forward_march`, `vb_resolve`, `vb_shade`, `vb_shade_split`) | all 4 paths × both legs (the Deferred resolve covers both legs, because the marcher writes the G-buffer) | **0**: no variant loaded | ≈ 4.2 fetches (3 map layers or 2 far layers, puddle noise, 0.2 ripple) + ~19 ALU per pixel → **0.04–0.06 / 0.08–0.10 / 0.17–0.22 ms** [E], plus a register cost gated at ≤ 3 % of the producer zone | R3 ✔. R6: +1 axis on up to **22 lit-producer `.spv`** (§6.3) |
| W2. A runtime branch in the same producers | same | the F24 dark tax (VB-SV0: +75 % while off [T]) | same | R3 ✗ |
| W3. A G-buffer modification pass | Deferred only (VB and Forward have no albedo G-buffer) | 0 | 1 fullscreen pass | fails 3 of 4 paths |

**Decided: W1 for puddles, snow cover and the exposure term.** Where the level-driven wetness
darkening is applied is a split with the dynamic-materials design. That design's DM2 also claims
"rain wetness (the level): a global level × per-material porosity", through a `Wetness` driver
curve that writes the material rows. If both shipped, a wet surface would be darkened twice:
- DM2 writes a darker albedo and a lower roughness into the row;
- `wet_surface` then applies Lagarde's factor again on top.

"How wet is the world" would also live in two resources. Pass 1 did not see this.

**Who owns which datum:**

| Datum | Varies with | Home | Owner |
|---|---|---|---|
| The global wetness level | per frame | **`SurfaceWetness { level }`**, the one home: a `Resource` declared by DM2 ("written by weather or gameplay"). Its crate cell there is empty; `boyko_render`, where DM's neighbouring rows live, is assumed. `WeatherState` has **no** `wet` field. `weather_update` is one of its writers | the dynamic-materials design declares it; this design writes it when `WeatherPlugin` is present |
| Porosity | per material, authored | the dynamic-materials parameter surface (the `Material.dynamic` sidecar → `MaterialDynGpu`). Unauthored materials use the zero-lane heuristic `(1 − metalness) · roughness` | the dynamic-materials design |
| Exposure (sky visibility) | per pixel | the occlusion map (§2.3) | this design |
| Puddles, ripples, snow cover | per pixel | `wet_surface` | this design |
| Lagarde's darkening itself | per pixel × per material | **applied exactly once** | the owner's ruling, below |

**The invariant decided here, whatever the ruling:** exactly one consumer applies the level-driven
darkening in any app. The consumer that does not own it reads no level:
- if the per-pixel leaf owns it, DM2 registers no `Wetness` curve kind;
- if DM2 owns it, `wet_surface` is instantiated with `wet ≡ 0` (its identity term) and keeps only
  puddles, ripples and snow.

FX10 carries a red-first test: one material under both routes darkens exactly once.

**Recommendation for the ruling (merged question B11 ≡ DM Q5): the per-pixel leaf owns the
darkening, and DM owns porosity.** The numbers:

| Route | GPU | CPU | Indoors |
|---|---|---|---|
| Per-pixel (`wet_surface`) | +≈ 4 ALU inside a leaf that already runs for puddles, ripples and snow (0.04–0.06 ms at 1080p for the whole leaf) | 0 | **masked by exposure**: a covered floor stays dry |
| Material row (DM2 `Wetness`) | 0 per pixel | 12–25 ns per driver per frame, plus a 48–96 B row upload while the level changes (DM's own [E]) | **cannot be masked**: an interior floor that shares a material with the street darkens during rain. The only fix is duplicate materials, against the 65,536-row cap |

- Under this recommendation DM2 drops the `Wetness` curve kind. Its `porosity` parameter becomes a
  per-material lane. Its `dry_base`/`dry_roughness` parameters are unnecessary, because the row
  keeps its authored dry values.
- A scripted wet look without weather remains possible with DM's generic curves (`Ease`,
  `Gradient`) on base colour and roughness.
- **The lane** is a request to the dynamic-materials design, not a decision taken here. The
  suggested carrier is the one free f32 in its `MaterialDynGpu` (`rim` word 3, `_reserved`),
  flagged by a new `flags` bit (`POROSITY`) so that "authored 0" and "unauthored" differ. Under
  `-D WEATHER` the leaf reads that word when DM's table is bound, and the heuristic otherwise.
  DM's 48 B zero buffer, which it binds when off, has the bit clear, so it selects the heuristic.
- **Pass 1's B11 is withdrawn.** It proposed transparency's `MaterialXGpu` for the lane, whose lanes
  are full and which the opaque producers never bind (DM F10).

**The rest of the leaf:**
- The leaf is written as `x + w·y` so that **wet = puddle = snow = 0 is an exact identity**. The
  gate checks that bit-for-bit.
- **`n_blend` blends toward world up (+Y), weighted by `puddle · saturate(N·up)`.** The thin-film
  wetness term does not touch the normal. Lagarde blends toward the *vertex* normal as the water
  layer fills the normal-map detail [P], and for the case that matters (standing water on an
  up-facing surface) that normal is up. This needs no geometric normal, and Deferred's G-buffer
  carries none. So the leaf is the same on every path, and no G-buffer changes.
- **Bindings.** Every weather binding is declared **entirely under `#if WEATHER`**, appended to each
  producer's own set 0 above its highest existing binding. That is `deferred_pbr`'s HWRT precedent
  (binding 19's two arms; "the 20th resolve descriptor the HWRT layout adds atop the 19 the
  software resolve uses" [T]).
  - The bindings are the weather UBO, five map images (near static, near SDF, near dynamic, far
    static, far SDF), ripples, puddle noise, one clamp sampler, and DM's table when porosity is
    read.
  - No set index is shared: the VB producers already use sets 0–3 (`vb_geom_fetch.hlsli` owns set
    2) [T].
  - Disarmed `.spv` and their host layouts are **unchanged**. The WEATHER layout is a per-variant
    superset, built at boot by the resolver that arms the variant. FX10's byte-identity gate covers
    both.

**Scorch marks are not W1.**

| Candidate | Paths × legs | Cost | Decision |
|---|---|---|---|
| **S1. Box decals at the seam**, multiply-blended into `lit` after the last opaque producer, reconstructing P from the path's depth | all 4 × both legs | per decal footprint: 1 depth + 1 texture fetch + an 8 B blend per pixel; ≈ 0.01 ms for a 5 %-screen decal [E]; **0 lit-producer variants** | **taken** (FX12) |
| S2. Clustered shading-time decals (DOOM, "256 decals" per cluster [P]) | all, but the froxel lists exist under VB only; a flat loop elsewhere | a second axis on the same 22 producers | deferred: the quality upgrade (normal and roughness decals) |

S1's limit, stated: a multiply over *lit* colour also darkens specular. For soot that is visually
correct; for a wet or normal-mapped crater it is not, and that is S2's job.

### 2.5 Lighting smoke cards

| Candidate | Evaluations per card | Where | Rules |
|---|---|---|---|
| D11 per particle, in the sim | 1 | sim | ✔ for sparks and small smoke. Cannot carry six directions in a 32 B record |
| **Per vertex, six-way, in the VS** (Unity [P]) | 4 × lights in the vertex's froxel (flat table off VB) + 6 DDGI probe-blend samples per vertex | VS | ✔: 48 cards × 4 × 64 lights × ~40 flops ≈ 0.5 MFLOP ≈ 0.0001 ms [E] |
| DOOM lighting atlas | tile² (16²–64²) | an atlas pass | ✗: a GPU atlas allocator and a pass, for a 64–1024× evaluation count |
| UE lighting volume | 64³ per cascade | a volume pass | ✗: a new volume whose cost scales ×8 per doubling [P] |

**Decided: per-vertex six-way (C5)**, riding the `PARTICLE_FX` axis as a per-effect mode (§6.3).

**Received shadows**: one CSM fetch per vertex. The card goes flat under shadow, Unity's stated
limit [P].

**Cast shadows: none.** Transparency's participation matrix gives particles "receives (P3)" in the
CSM/atlas row and no caster role. R9's `csm_trans` is for translucent *meshes*. Pass 1 said
particles would become `csm_trans` casters when R9 lands, which contradicted that matrix; the
sentence is withdrawn. Particle shadow casting stays unscheduled. If it is ever designed, C8's
camera-frustum cull has to be exempted for casters, since an off-screen card can shadow an
on-screen floor.

**DDGI ambient**: the probes see only the SDF leg (research §1.2). A mesh-occluded card is lit as
if unoccluded, the same limit every DDGI consumer carries.

### 2.6 Shockwave and heat distortion

**Decided: distortion is a *translucent card entity* using transparency R5's `-D REFRACT` over
R4's opaque colour chain.** It is not a particle variant.
- A shockwave is 1–4 camera-facing cards animated by a CPU system (scale and fade over about
  0.3 s): ECS entities, not GPU particles.
- **0 new particle variants**; 0 new images beyond R4's chain (11.1 MB at 1080p, priced there).
- The rejected alternative was a screen offset buffer (RG16F, 8.3 MB at 1080p) plus a warp pass
  (Froyok [P]).
- The limit is R4's: distortion sees opaque + sky only.

### 2.7 Wind

| Candidate | CPU consumers (soft bodies) | GPU consumers | Rules |
|---|---|---|---|
| **Analytic: global direction × host-evaluated gust + Σ vorticles** (Ghost of Tsushima [P]) | the same leaf through `boyko_sdf_math → boyko_shaderdsl` (a live edge [T]) | the same leaf in the sim; the global term only in the rain VS | R2 ✔: ≤ 32 vorticles × 32 B = 1 KB; ≈ 25 flops per vorticle in range, so ≈ 50–300 flops per sample after sphere culling [E]. R3 ✔: vorticles are entities; receivers opt in. R6 ✔: one `FieldScalar` op tree is both the CPU function and the HLSL |
| 3D GPU fluid grid (God of War [P, abstract]) | needs a readback or a CPU mirror | ✔ | R3 ✗: a CPU mirror is the parallel data system Principle 0 forbids; a readback is forbidden by the particle plan |

**Decided: analytic.**
- The per-vorticle term is a **`FieldScalar` leaf using `mul`, `add`, `sub` and `clamp01` only**.
  It has no `sqrt`, no divide, no trig and no `floor`:
  - `d = p − c`;
  - `w = clamp01(1 − (d·d) · inv_r²)²`, with `inv_r²` host-precomputed per vorticle;
  - `v = s · w · cross(axis, d)`.

  This is a Rankine-like swirl: rigid rotation near the core, zero at radius `r`. Pass 1
  normalised by `|d|` with `sqrt`.
- **The parity claim is restated.** Pass 1 said "the same body is bit-exact on the CPU and the
  GPU". That is not what the tree guarantees:
  - the eDSL's float leaves match their oracle "modulo FMA contraction", the crate's standing
    carve-out (`crates/boyko_shaderdsl/src/particle.rs` module doc) [T];
  - a GPU `sqrt` or divide is not correctly rounded (M7's rule for `OpFDiv`).

  The claim is now:
  - **(a) one op tree.** Eval and Emit instantiate the same body, pinned by the `*_edsl_sync`
    regeneration. A mutation of the leaf changes both.
  - **(b) a bounded difference.** With no `sqrt` and no divide, the only divergence left is
    contraction of `a·b + c`, one rounding per contractible pair. The leaf has ≈ 6 such pairs:
    two in `d·d`, one in `w`, three in the cross product. With `|axis| = 1` and `|d| ≤ r` where
    `w > 0`, each component differs by at most ≈ 2⁻²⁰ · s · r [E, derived: each pair shifts its
    result by ≤ 1 ULP of a term bounded by `r` or by 1]. The GPU gate uses **|Δv| ≤ 2⁻¹⁸ · s · r
    per component**, a 4× margin, as an absolute bound. A relative bound would be wrong, because
    the cross product cancels near the axis.
  - The CPU soft body and the GPU particles never need to agree bitwise. They are different
    consumers of one function, and the claim that matters is "one function".
- The loop over the table is the caller's.
- Gust is a host scalar per frame (Rust `sin` on the CPU); the GPU never evaluates time-trig for
  wind.
- **Turbulence (curl noise) is GPU-only**, a `Cf` leaf using `float_to_uint` lattice hashing.
  CPU consumers do not see it. That is the firewall `cf.rs` states for codegen-only nodes.

### 2.8 Secondary spawns (GPU events)

| Candidate | Latency | Cost | Rules |
|---|---|---|---|
| **Next-frame ring.** The sim appends `ParticleEventGpu` (32 B) records into `events[parity]` with one wave-aggregated atomic; the next frame's kickoff folds `min(count, EVENT_CAP)` into `real_emit_count`; emit maps lanes beyond the host total to event records one-to-one | one particle step (≤ 15.6 ms at 64 Hz) | +1 atomic per event-producing wave; 2 × 16 384 × 32 B = **1.05 MB** | ✔ D3 preserved (emit reads, sim pushes) |
| Same-frame (Unity: triggers run "at the end of Update") | 0 | a fourth pass, or emit+sim fusion | ✗: D3 rejects fusion outright |
| CPU readback | 1–2 frames | a readback | ✗: "Readback: none, ever" |

**Decided: the next-frame ring**, with **one child per record**. An effect that wants `k` children
writes `k` records, which removes a GPU prefix scan at k × 32 B.

**Where the append sits: flagged in the substep loop, appended once in the retirement block.**
- Inside the substep loop, a contact sets a register flag (`collided |= hit`) and latches the first
  contact's position and reflected velocity. There is no atomic inside the divergent contact
  branch.
- After the loop, the block that already decides retirement computes each lane's record count:
  `children · (died & DEATH) + children · (collided & COLLIDE)`. The wave then takes **one**
  wave-aggregated atomic for all of its lanes' records, with a wave prefix for each lane's offset.
- So the claims hold: **+1 atomic site** in the module (the census gate) and **+1 atomic per
  event-producing wave**.
- Stated limitation: at most one collide event per particle per 64 Hz step, at the first contact
  of that step.

The kickoff's new variant is the stated cost (C4).

### 2.9 Authoring model

| Candidate | Dispatches | Shader cache | Rules |
|---|---|---|---|
| **POD effect rows + feature lanes in the spare words of `EffectParamsGpu`; one sim** (Wicked's option bits [P]) | 1 | none | ✔: the undefined `flags` word [T] takes the mode and mask bits. The five spare words (`_r0[2]`, `_r1`, `_r2`, `_r3` [T]) take flipbook, turbulence, wind response, event and the colour ramp's host-baked reciprocals (§4.4). The row stays **128 B** |
| eDSL-compiled per-effect modules (Niagara stacks as `-D` variants) | up to 256 | a permutation cache | ✗: "1 particle can cost as 64" (Epic, carried); the manifest grows per effect |
| A runtime expression interpreter (Destiny) | 1 | none | ✗: dynamic dispatch on the hot path (R4); already rejected by `PARTICLES-RESEARCH.md` |

**Decided: POD rows plus lanes.** Explosion and weather presets are assets
(`Assets<ExplosionPreset>`) composed from `Handle<ParticleEffect>`s. A Gaia on-disk format is
owner question B3.

### 2.10 Overdraw: half-resolution alpha particles

Priced for the reference explosion's smoke (3.84 screens of layers, §7.1) at 1080p:
- An off-screen `R16G16B16A16_SFLOAT` target is 16 B per fragment, so the smoke's fill term falls
  from 3.84 to 1.92 screen-equivalents (−37.5 % of all layers), and its texture samples by 75 %.
- The composite reads the half-resolution target and read-modify-writes `lit`: about 21 MB, i.e.
  **0.06–0.17 ms**.
- The max-depth downsample adds about 10 MB.
- **Net at 1080p: ≈ 0 to +0.1 ms saved. At 4K: ≈ 0.15–0.5 ms saved** [E].
- GPU Gems 3 measured 1.14× for a light shader and 2.04× only for a 73-instruction one [P].

**Decided: not scheduled.** The images (4.1 / 7.4 / 16.6 MB colour + 2.1 / 3.7 / 8.3 MB depth at
1080p / 1440p / 4K) and the edge re-render are not built speculatively.

**The trigger names the rung that produces its number.** Pass 1 pointed at FX0, which has no
six-way smoke: six-way cards exist only at FX6. The trigger is now FX6's measurement, taken on the
two arms that exercise this lever:
- **the E1 smoke arm**: 48 textured six-way MV cards over an E1-shaped region;
- **the near-camera arm**: the camera at E1's centre, under the default coverage bound (§2.13).

Half resolution is scheduled if the E1 smoke arm exceeds **1 ms at 1440p**, or the near-camera arm
exceeds **2 ms at 4K**. The near-camera case is where half resolution pays: large layers in the
off-chip regime, where the saving is bandwidth.

**Variable-rate shading is recorded, not taken.**
- It cuts fragment-shader work, texture samples and ALU, by shading once per 2×2. It does not cut
  blend traffic (research §4.6b).
- So it helps the on-chip, texture-bound regime (a compact E1 at 1080p, where texture binds at
  0.29–0.35 ms), and not the off-chip regime that dominates the near-camera worst case.
- It needs `VK_KHR_fragment_shading_rate`, which has zero hits in the tree [T]: an RHI gap (§5).
- It is re-evaluated if FX6 shows the E1 arm texture-bound above 0.5 ms at 1440p.

### 2.11 Texture compression

The RHI has no BC formats and does not enable `textureCompressionBC` [T]. One reference explosion
set costs **72.7 MB** uncompressed (mipped) against **22.4 MB** with BC7 colour + uncompressed RG8
motion vectors, a ratio of **3.2×** (§7.6).

**Decided (technical): BC is not a prerequisite of any rung here.** VFX will be the first consumer
large enough to see it. All PBR textures are uncompressed too, so this is an engine-wide texture
campaign, and its scope and timing go to the owner (B2) with this number attached.

### 2.12 Bursts versus emitter entities

**Decided: bursts (C3), on the per-contact ground, plus C12.** A continuous effect (rain drips, a
burning barrel) is an emitter *entity*, as D1 says. A one-shot effect is a burst *request*:
`ParticleBursts` rows consumed once by A1, sharing the per-frame `MAX_EMITTERS = 256` request
budget.

**The comparison pass 1 skipped.** The tree already has a one-shot field,
`ParticleEmitter::burst`, "consumed (zeroed) by the tick that reads it" [T]. So an explosion could
spawn three emitter entities with `burst` set and despawn them through `Lifetime`. Against that:
- **For an explosion the two are equivalent.** `vfx_explode` spawns about 19 entities anyway (16
  debris, a light, a decal, the recipe). Three more structural spawns (≈ 0.1–0.3 µs) are noise.
  Pass 1's "structural spawn" saving is withdrawn.
- **Pass 1's "persistent slot" argument is withdrawn too.** The slot was held only because A1
  pushes a row for every enabled emitter, including idle ones [T]. **C12** removes that: A1 skips
  zero-count requests. Then an emitter whose burst has fired costs nothing per frame, and
  `MAX_EMITTERS` counts only emitters that spawn this frame. That fixes a real defect for every
  client: today 256 idle emitters starve a 257th that wants to spawn.
- **Where bursts still win: per-contact sparks** (`vfx_impact_sparks`).
  - Contacts arrive at tens per frame, each with its own point and normal basis, for one frame.
  - A burst row is one `ScratchColumn` push, ≈ 10 ns [E]. An emitter entity per contact is a spawn
    plus a later despawn, 60–200 ns [M-derived], and deferred-command churn.
  - At 50 contacts per frame: 0.5 µs against 3–10 µs, with no entities created.
- `vfx_explode` uses `ParticleBursts` too, so there is one code path for one-shots. That is a
  uniformity choice, not a performance claim.

### 2.13 Budgets and LOD, with no readback

**Decided:**
- **In-sim frustum culling** (C8), with a conservative sphere of radius `size·√2`, so a culled
  particle is invisible by construction and the pins cannot move.
- **Host-side expected-alive budgeting.** `ParticleBudget { target_alive }`; A1 computes
  `Σ rate·E[life] + Σ burst·E[life]` from host-known effect rows (Unity's field rule [P]) and
  scales continuous emitters by distance-to-camera significance when the sum exceeds the target.
  0 readback; ≤ 256 rows × ~10 ns ≈ 2.6 µs [E].
- **Surface the existing `clamped_spawns`** in the diagnostics counters. Today it is counted and
  never shown (research pitfall 17).

**Coverage: a per-effect bound in the draw VS (C14).** `ParticleBudget` counts particles and is
blind to coverage. A coverage-aware budget would need screen-space feedback, i.e. a readback, which
the plan forbids. So coverage is bounded per card, structurally, and the worst case is the product
of count and bound. Two controls follow Unity's shipped pair [P]:
- **Screen-size clamp.** A card's full size is at most `max_screen_e` of the viewport height:
  `h′ = min(h, max_screen_e · (a·z_view + b))`.
  - `(a, b) = (tan(fov_y/2), 0)` for perspective and `(0, ortho_half_height)` for orthographic.
    That is one per-view constant pair, written by the host.
  - `z_view = dot(P − cam_eye, cam_forward)` comes from the camera block the VS already binds [T].
  - Cost: ≈ 3 ALU per vertex, **no divide**.
  - `max_screen_e` is per effect (`EffectDrawGpu`). **Default 0.35**, provisional, fixed by FX6's
    measurement below.
- **Near-camera fade and cull.** `α ×= clamp01((z_view − near_e) · inv_fade_e)`, where
  `inv_fade_e = 1/(far_e − near_e)` is host-baked. When `z_view < near_e`, all four corners collapse
  to one clip point: zero area, **zero fragments**. Cost ≈ 2 ALU and a select per vertex. Default
  `near_e = 0.3 m` and `far_e = 1.5 m`, per effect.
- **The priced worst case [E]** (the table is in §7.1):
  - A square card clamped at `f` covers `f² · H/W = f² · 0.5625` of a 16:9 screen.
  - Camera at the centre of E1, all 64 cards beyond `near_e`. A full-screen region is always off
    chip, and the cards also sample 5.5 textures each on average.
  - **At the default `f = 0.35`:** 4.41 screen-layers, a draw of **0.25–0.91 ms at 1080p,
    0.44–1.61 at 1440p, 0.98–3.63 at 4K**.
  - **Unbounded:** up to 64 screen-layers, 3.6–13.2 ms at 1080p. Fill alone is 3.1–8.8 ms. Pass
    1's own model, as the critique computed it, gave 2.4–6.6 ms for the 48 smoke cards' fill alone.
- **FX6 fixes the default from a measurement.** The default `f` is the largest of {0.5, 0.35,
  0.25} whose near-camera arm is ≤ 1 ms at 1440p. The model predicts 0.25–0.35; `f = 0.25`
  gives 0.22–0.82 ms at 1440p. If even 0.25 fails, the half-resolution trigger of §2.10 fires.

**Arming is boot-frozen and explicit** (open question 5 of the critique):
- Per-class pipelines, passes and images are declared at boot (plan D13/D14). So "armed by an
  effect with `Height`" is replaced by a config field.
- `ParticleConfig::collision` grows from `{Off, Sdf, SdfStats}` [T] to include `Height` and
  `SdfHeight`, and `ParticleConfig::fx` ∈ {`Off`, `Flip`, `SixWay`} arms `PARTICLE_FX`.
- The occlusion map is declared iff `WeatherConfig::mode != Off` or `collision` includes `Height`.
- An effect registered later that requests an unarmed feature has that lane cleared at upload and
  counted (`clamped_features`). This is D15's release clamp, with a `debug_assert!` in developer
  builds. The effect then renders plain and collides as armed, and the counter says why.

### 2.14 Flipbook sheets: mip and warp containment

Pass 1 carried the UI's half-texel inset into a *mipped* atlas and priced the sheets as mipped.
The inset protects mip 0 only. At level `k`, half a texel is `2ᵏ⁻¹` mip-0 texels, so bilinear and
trilinear taps reach into the neighbouring frame. The tree's chain goes all the way to 1×1: it is
built by LINEAR blit, and the one shared sampler has max LOD unclamped and anisotropy 16 [T]. Below
the level where a cell is one texel, the chain mixes whole frames. NVIDIA's atlas whitepaper
describes both failures [P].

| Candidate | Cost | Verdict |
|---|---|---|
| **G. Sheet contract: power-of-two cells, a truncated chain, an empty guard band** | 0 shader ALU for the base sample; importer check O(band texels) at load; VRAM ≈ unchanged (the dropped levels are < 0.1 % of the chain) | **taken** |
| L. LOD-aware UV clamp in the FS (NVIDIA's first cure) | a `CalculateLevelOfDetail` query + ≈ 8 ALU per sample, × 2–6 samples | rejected: G gives the same guarantee for 0 ALU on the unwarped samples |
| P. Per-level border padding (NVIDIA's second cure) | "quickly wastes texture-memory" [P] | rejected |
| A. Texture arrays | 0 | unavailable: the bindless table is `Texture2D gTextures[]` (C11) |

**The contract (G)**, enforced by the sheet importer and stated in the asset's doc:
1. **Power-of-two sheet, power-of-two cells** (`cols_log2`, `rows_log2`). A 2×2 box filter (which is
   what a LINEAR blit halving a POT image computes) then never mixes cells until a cell reaches one
   texel [P]. So the existing blit path builds a clean chain; no per-cell generator is needed.
2. **The chain stops at `L_max = log2(cell_px) − 3`**, the level where a cell is 8 texels. It is set
   through `TextureDesc::mip_levels` [T], so neither the shared sampler nor the descriptor changes.
   A card smaller than 8 px samples `L_max` and minifies with mild aliasing, which is stated.
3. **Every cell's outer guard band of `g = 2^(L_max − 1) = cell_px / 16` mip-0 texels is empty**:
   alpha 0 for the alpha class, colour 0 for additive. At any level `L ≤ L_max`, a tap centred in
   the cell reaches at most `2^(L−1) ≤ g` mip-0 texels across its edge, into the neighbour's band,
   which is empty.
   - So **no inset is needed**, and `EffectDrawGpu.inset_uv` becomes `band_uv` (g in UV units),
     used only by the warp clamp below.
   - Effect flipbooks already leave empty margins: a fireball cut at its cell edge would show a
     hard seam at mip 0. The rule makes that margin a checked property.
   - The importer rejects a sheet with lit band texels and names the cell.
4. **Anisotropy.** Every particle quad is camera-facing and square: the billboard basis is
   `cam_right`/`cam_up`, and `size` is a scalar [T]. So the anisotropic footprint stays ≈ 1 texel.
   A velocity-stretched flipbook card would break this and is outside the contract, stated.

**The warp clamp (FX2).** Lozar's warp moves UVs by up to `mv_strength` of a cell. `flipbook_mv_warp`
clamps each warped UV to the cell rectangle shrunk by `band_uv`: 4 `min`/`max` per warped sample,
8 ALU per fragment for the two warped samples. A warped tap then lands at worst on the frame's own
band, never in the neighbour.

**Why this is decided now:** the leaf signatures and the FX1/FX2 pins are frozen together. Adding
a band or a clamp later would re-bless both pins.

---

## 3. How it lands in the ECS

### 3.1 Crates and plugins

| Where | What | Why there |
|---|---|---|
| `boyko_render` (existing) | the particle changes C1–C8; `ParticleBursts`; `ParticleBudget`; the weather GPU bundle and passes | the subsystem already lives there; D17 containment applies unchanged |
| **`boyko_vfx` (new, optional)** | `VfxPlugin`, `WeatherPlugin`, recipe components and assets, CPU systems | an explosion spawns rigid bodies (`boyko_physics`), and `boyko_render` must not depend on physics; `boyko_vfx` depends on `boyko_render`, `boyko_physics` and `boyko_scene` |
| `boyko_scene` | `WindField` (Resource), `WindVorticle` (Component), `CameraShake` (Component) | its charter is "the spatial components every world-space subsystem … builds on" [T]. Wind is read by render *and* physics; the shake modifies the view |
| `boyko_shaderdsl` | the new leaves (§6) | the eDSL rule; physics reaches the wind leaf through the existing `boyko_sdf_math` edge |
| kernel / std-lib (request) | `Lifetime(f32)` + a despawn system | a general capability (debris, flash lights, decals, recipe entities), so Principle 0 makes it a first-class feature, not a VFX adapter (§11 request K1) |

**Zero cost when off:**
- No `VfxPlugin` ⇒ no system and no resource. `ParticleBursts` is inserted **only** by
  `VfxPlugin`. The particle plugin never inserts it, and A1 takes it as an optional resource. So an
  app without `VfxPlugin` has no `ParticleBursts` at all, and FX3's absence gate can fail: a
  mutation that inserts it from the particle plugin reds it.
- No `WeatherPlugin`, or `WeatherConfig::mode == Off` (the `#[default]`) ⇒ no pass, no image, no
  pipeline, no variant loaded. That is plan D13's template verbatim.
- The audio cue system is registered by the *audio* plugin, so with no audio plugin nothing
  iterates `CueOnFire`.

### 3.2 Components, resources, systems

```rust
// boyko_render — pool inputs (Principle 0: ScratchColumn staging, release clamp + debug_assert)
#[derive(Resource)] pub struct ParticleBursts { rows: ScratchColumn<BurstRequest>, count: u32 }
#[repr(C)] pub struct BurstRequest { effect: Handle<ParticleEffect>, origin: [f32; 3], basis: [[f32; 3]; 3], count: u32, speed_scale: f32 }
#[derive(Resource)] pub struct ParticleBudget { pub target_alive: u32 }
// ParticleConfig (existing) gains boot-frozen arming (§2.13):
//   collision: ParticleCollision { Off, Sdf, SdfStats, Height, SdfHeight }   // Height/SdfHeight new at FX8
//   fx: ParticleFx { #[default] Off, Flip, SixWay }                         // arms PARTICLE_FX at FX1/FX6
#[derive(Component)] pub struct RainDynamic;                                   // caster enters the per-frame dynamic layer (§2.3)
// SurfaceWetness { level: f32 } — declared by the dynamic-materials design (its DM2); the ONE home of the level (§2.4)

// boyko_vfx — recipes are entities; capability = component presence
#[derive(Component)] pub struct ExplosionFx { pub preset: Handle<ExplosionPreset> }      // one-shot; despawned via Lifetime
#[derive(Asset)] pub struct ExplosionPreset {
    fireball: Handle<ParticleEffect>, smoke: Handle<ParticleEffect>, sparks: Handle<ParticleEffect>,
    counts: [u32; 3], flash: FlashDesc, debris: DebrisDesc, decal: Option<DecalDesc>,
    trauma: f32, trauma_radius: f32, cue: u32, shockwave: Option<ShockwaveDesc>,
}
#[derive(Component)] pub struct LightFade { pub start: f32, pub duration: f32, pub power0: f32 }  // on a PointLight entity
#[derive(Component)] pub struct Decal { pub tex: u32, pub half_extent: [f32; 3], pub opacity: f32 }  // box decal entity
#[derive(Component)] pub struct ImpactSparks { pub effect: Handle<ParticleEffect>, pub per_contact: u32 } // opt-in on a body with Contact
#[derive(Component)] pub struct ParticleCollider;                                             // opt-in: body enters the analytic table (FX8)
#[derive(Component)] pub struct CueOnFire(pub u32);                                           // read only by the audio plugin's system
#[derive(Resource)] pub struct WeatherConfig { pub mode: WeatherMode /* #[default] Off */, pub max_drops: u32, pub occ_texels: u32, pub occ_box_m: f32 }
#[derive(Resource)] pub struct WeatherState { pub rain: f32, pub snow: f32, pub puddle: f32, pub snow_cover: f32 } // no `wet`: the level is SurfaceWetness.level

// boyko_scene
#[derive(Resource)] pub struct WindField { pub dir: [f32; 3], pub speed: f32, pub gust_amp: f32, pub gust_hz: f32, pub turbulence: f32 }
#[derive(Component)] pub struct WindVorticle { pub radius: f32, pub strength: f32, pub axis: [f32; 3] } // + GlobalTransform
#[derive(Component)] pub struct WindReceiver { pub response: f32 }                          // opt-in on a SoftBody entity
#[derive(Component)] pub struct CameraShake { pub trauma: f32, pub decay_per_s: f32, pub max_rot: [f32; 3], pub hz: f32, pub seed: u32 }
```

**Systems (all in `CoreSchedule::Main`, ordered by real edges):**

| System | Reads → writes | Cost [E] |
|---|---|---|
| `vfx_explode` | `Added<ExplosionFx>` → `ParticleBursts`, spawns `PointLight + LightFade + Lifetime`, debris bundles, `Decal`, `CameraShake.trauma` | < 10 µs per explosion (16 debris spawns at 30–100 ns [M-derived]) |
| `vfx_impact_sparks` | `(&Contact, &ImpactSparks)` → `ParticleBursts` | per contact |
| `particle_tick_emitters` (A1, extended) | emitters **and** `ParticleBursts` → emit requests; `ParticleBudget` scaling | unchanged ≤ 15 µs budget (plan §Goal) |
| `vfx_light_fade` | `(&mut PointLight, &LightFade)` | per flash |
| `lifetime_despawn` (K1) | `&mut Lifetime` → a deferred despawn (the kernel's deferred command path, `component/hooks/deferred_master.rs`) | per entity |
| `weather_update` | `WeatherState`, `Time`, camera → `SurfaceWetness.level` and the weather UBO; near/far re-centre decisions (never both on one frame); host-wrapped phase | < 2 µs |
| `weather_mark_dynamic` | `Added<RigidBody>` with `ShadowCaster` → inserts `RainDynamic` (once per entity) | per added body |
| `weather_validate_static` (developer builds only) | `Changed<GlobalTransform>` on `With<ShadowCaster>, Without<RainDynamic>` since the last static raster → a named diagnostic | 0 in release |
| `wind_gather` | `WindField`, `(&WindVorticle, &GlobalTransform)` → the ≤ 32-row table (release clamp, D15) | < 1 µs |
| `physics_soft_wind` (in `boyko_physics`) | `(&mut SoftBody, &WindReceiver)` + the table → per-vertex acceleration | ≈ 6–15 µs per 1k vertices after sphere culling [E] |
| `camera_shake_apply` | `(&mut CameraShake)` → a rotation offset composed in the view build; trauma² × smoothed noise; rotation only (Eiserloh [P]) | < 1 µs |

### 3.3 The Principle 0 audit

| Datum | Home |
|---|---|
| per-explosion state | component columns (`ExplosionFx`, `LightFade`, `Decal`, `Lifetime`) |
| burst requests | a `Resource` over a `ScratchColumn`, refilled per frame |
| weather and wind state | resources + components |
| the global wetness level | `SurfaceWetness` only, shared with the dynamic-materials design; the weather UBO carries a per-frame staging copy, not a second home |
| per-material porosity | the dynamic-materials material sidecar (a requested lane); never a VFX-side table |
| per-particle state | the pool: the sanctioned GPU-contiguity exception, no CPU mirror (unchanged) |
| rain drops | **no state at all** |
| occlusion map, event rings, draw table | GPU buffers and images owned by the render bundle, like every `gpu_scene` ring |

No `Vec`, `HashMap` or side store exists anywhere in the layer. The only CPU/GPU-shared function
(wind) is **one eDSL body**, not two implementations.

---

## 4. How it lands in the frame graph

### 4.1 Frame order (every path; brackets are rung-gated; `U` means near-update frames only, `UF` far-update frames only)

```
[FX7,U]  weather_occ_mesh    depth-only ortho raster of the static casters, Without<RainDynamic> (the csm depth pipeline, a new view)
[FX7,U]  weather_occ_sdf     toroidal strip march into R32F (sdf_field.hlsli consumer)
[FX7]    weather_occ_dyn     every frame while armed: clear + raster of With<RainDynamic> casters, 512² D32
[FX10,UF] weather_far_mesh / weather_far_sdf   far level, 512² each over 256 m; never on a near-update frame
[FX10]   weather_ripples     256² compute, only while SurfaceWetness.level > 0
        particle_kickoff → particle_emit → particle_sim → [P2 sort]       (declared early; overlaps opaque, plan F8)
          sim additions: [FX1] ramp + flipbook frame + frustum cull · [FX5] events · [FX8] height/analytic collide · [FX11] wind
        … opaque producers — [FX10] -D WEATHER variants read weather_occ_* and ripples …
        [sdf_forward_march]
[FX12]   decal_draw          box decals, multiply into lit, depth-reconstructed P (both legs)
[R4]    scene_color_copy → scene_color_mips
        translucent_draw / [R7] OIT — incl. [FX13] shockwave cards (-D REFRACT)
        particle_draw (alpha)    — [FX6] six-way cards, flipbooks
[FX9]    weather_draw (snow)  flakes, premultiplied "over", unsorted (one colour per frame ⇒ order-independent)
        particle_draw (additive) — sparks, fire flipbooks
[FX9]    weather_draw (rain)  rain streaks + splash sprites (additive; after particle additive)
[R6]    every blended draw above writes taa_cov
[Deferred, VB]  taa_resolve → present chain        [Forward, ForwardPlus]  present chain
```

- The decals sit **before** R4 so that refraction sees scorch marks.
- Rain sits in the additive tail, and snow sits just before it, so transparency §2.3's "additive
  draws last, pure sum" invariant holds.
- The weather draw on Deferred needs its own `-D DEPTH_LINEAR` arm (plan D7's contract) and
  inherits the 64-unit Deferred horizon. The rain box is ≤ 32 m, so the horizon never binds.

### 4.2 New resources (appended last, conditional tail; the `declare_particle_*` template in `graph_bridge.rs`)

| ResId | Kind | Size (1080p) | Seeded (cross-frame) | Rung |
|---|---|---|---|---|
| `effect_draw` | buffer, 48 B × 256 | 12 KB | boot-static per generation (the `effects` upload gate) | FX1 |
| `particle_events[2]` | buffers, 16 384 × 32 B each | 1.05 MB | yes (the alive-list parity idiom) | FX5 |
| `weather_occ_mesh` | image, D32, 1024² | 4.19 MB | yes (`add_image_seeded`) | FX7 |
| `weather_occ_sdf` | image, R32F, 1024², toroidal | 4.19 MB | yes | FX7 |
| `weather_occ_dyn` | image, D32, 512² | 1.05 MB | no (cleared every frame) | FX7 |
| `weather_far_mesh`, `weather_far_sdf` | images, D32 + R32F, 512² each, the SDF one toroidal | 2.1 MB | yes | FX10 |
| `weather_ripples` | image, RG16F, 256² | 0.26 MB | no | FX10 |
| `weather_ubo`, `wind_table` | buffers | 112 B, 1 KB + 32 B | per-FIF staging (F19 token) | FX9, FX11 |

- **No new image scales with resolution.**
- The sampled-depth-in-compute precedent for the sim's map fetch is `hzb_build.comp.hlsl`'s
  `Texture2D<float> gSrcDepth` [T].
- A resolution-scaled image appears only if §2.10's half-resolution rung is ever scheduled.

### 4.3 Participation matrix

| Effect | Deferred | Forward | ForwardPlus | VB | Mesh leg | SDF leg |
|---|---|---|---|---|---|---|
| Particle draw (all classes) | ✔ `DEPTH_LINEAR`, early-Z lost, 64-unit horizon | ✔ | ✔ | ✔ | depth-occluded | depth-occluded |
| Rain / snow draw | ✔ own `DEPTH_LINEAR` arm | ✔ | ✔ | ✔ | occlusion map (raster) | occlusion map (march) |
| Height collision | path-independent | | | | ✔ | ✔ |
| SDF collision (P1) | path-independent | | | | ✗ | ✔ |
| Wetness / snow cover (`-D WEATHER`) | `deferred_pbr` (both legs) | `forward_opaque.fs` + `sdf_forward_march` | same | `vb_shade`/`vb_resolve`/`vb_shade_split` + `sdf_forward_march` | ✔ | ✔ |
| Box decals | ✔ euclidean decode | ✔ reverse-Z decode | ✔ | ✔ | ✔ | ✔ |
| Six-way lights | flat table | flat table | flat table | froxel lists | — | — |
| DDGI ambient on cards | ✔ | ✔ | ✔ | ✔ | probes blind to mesh | ✔ |
| TAA coverage (R6) | ✔ | — (no TAA) | — | ✔ | | |

The two depth decodes for decals are the **soft-particle decoders** of P2 item 4 (D14 there: stored
× `T_MAX` on Deferred, `B / (depth − A)` on reverse-Z), reused. D15 there makes the metric
view-space Z on both.

### 4.4 GPU data structures (changed and new)

```rust
// CHANGED at FX1 — same 32 B, same offsets; only the meaning of the last word moves (C2).
pub struct ParticleRender {
    position: [f32; 3], size: f32, color_rgba8: u32, rot_cs: u32,
    tex_index: u32,          // unchanged: the colour sheet (per particle, so a variation index stays possible)
    flags: u32,              // [0,8) effect_index · [8,16) frame · [16,24) frame_frac unorm8 · [24,32) render bits
}

// CHANGED at FX1 (C13) — same 48 B, same offsets; the word at offset 32 changes meaning.
pub struct ParticleSim {
    position: [f32; 3], life_remaining: f32, velocity: [f32; 3], cached_field_d: f32,
    life_total: f32,         // was color_rgba8 (key 0 held for life); written by emit, which already divides once
    size0_invlife: u32, effect_flags: u32, rot_cs: u32,
}
// The colour is now evaluated each step from the effect's ramp and written straight into
// ParticleRender::color_rgba8. Per-particle tint variation, if ever wanted, derives from a hash of
// the slot, not from a stored colour.

// NEW at FX1 — the draw-side per-effect row (C1); set 0, binding 2, VERTEX | FRAGMENT.
#[repr(C, align(16))]
pub struct EffectDrawGpu {   // 48 B × MAX_EFFECTS = 12 KB
    aux_tex: [u32; 3],       // MV sheet (FX2) · six-way B (FX6) · six-way MV (FX6); 0 = none
    grid: u32,               // u8 cols_log2 · u8 rows_log2 · u8 mode {0 plain, 1 blend, 2 mv, 3 sixway_mv} · u8 bits
    band_uv: [f32; 2],       // the sheet's guard band in UV (§2.14); replaces pass 1's mip-0 inset
    mv_strength: f32,
    fade_distance: f32,      // per-effect soft fade (C1); P2-item-4 D16's global push word is retired
    max_screen: f32,         // coverage clamp: max full size as a fraction of viewport height (C14)
    near_fade: f32,          // z_view below which the card is culled (C14)
    inv_fade: f32,           // 1 / (far_fade − near_fade), host-baked (C14)
    _r: u32,
}

// CHANGED at FX1 — ParticleDrawPush grows 72 → 80 B: one per-view pair (a, b) for the clamp,
// (tan(fov_y/2), 0) perspective or (0, ortho_half_height) orthographic. With P2 item 4's fields
// the range stays ≤ 88 B, under Vulkan's guaranteed 128 B.

// CHANGED at FX1/FX5/FX8/FX11 — EffectParamsGpu stays 128 B. `flags` (undefined today [T]) takes
// every mode and mask bit, which frees the spare words for values:
//   flags  → [0,2) flip mode {none, over-life, fps, fps + random start} · [2,5) collide {SDF, HEIGHT, ANALYTIC}
//            · [5,7) event kinds {DEATH, COLLIDE} · [7] WIND · [8] TURBULENCE · [9,16) reserved
//            (the low 16 bits are still copied into the particle's effect_flags half, as today)
//   _r0[0] → flipbook: u8 frame_count−1 · u8 — · f16 fps                     (FX1)
//   _r0[1] → turbulence: f16 strength · f16 frequency                       (FX11)
//   _r1    → wind_response: f32                                             (FX11)
//   _r2    → event: u8 child_effect · u8 children · f16 ramp_inv_dt[2]      (FX5; FX1 for the f16)
//   _r3    → ramp: f16 ramp_inv_dt[0] · f16 ramp_inv_dt[1]                  (FX1)
// ramp_inv_dt[i] = 1 / (t[i+1] − t[i]) over color_times, host-baked; a degenerate segment bakes 0
// and the ordered compares never select it.

// NEW at FX5
#[repr(C, align(16))]
pub struct ParticleEventGpu { position: [f32; 3], child_effect: u32, velocity: [f32; 3], seed: u32 } // 32 B

// NEW at FX9 (FX10 adds the far level and the surface terms) — one UBO, per-FIF staged.
#[repr(C, align(16))]
pub struct WeatherGpu {      // 112 B (7 × 16 B)
    cell_origin: [f32; 3], box_m: f32,
    phase: f32, fall_speed: f32, drop_count: u32, splash_count: u32,
    wind_offset: [f32; 3], streak_len: f32,          // ∫ global wind dt, host-accumulated and wrapped
    wind_vel: [f32; 3], snow_rgb_packed: u32,        // global wind velocity (streak orientation, O3); the one flake colour (O2)
    occ_origin_xz: [f32; 2], occ_inv_extent: f32, occ_top_y: f32,       // near level (static, SDF and dynamic share it)
    far_origin_xz: [f32; 2], far_inv_extent: f32, wet_level: f32,       // far level (FX10); a staging copy of SurfaceWetness.level
    puddle: f32, snow_cover: f32, _r: [u32; 2],
}
```

**Why `frame` and `frame_frac` are resolved in the sim and not the VS:** the VS cannot see age,
and it runs 4× per particle. The frame therefore advances at the 64 Hz particle step, the
limitation P2b already owns (plan M6).

---

## 5. RHI gaps, verified at `6394bc5e`, and who needs them

| Gap | Evidence [T] | Needed by | Decision |
|---|---|---|---|
| `VK_KHR_push_descriptor` (core in Vulkan 1.4) | zero hits under `crates/` | FX4 (soft, as designed), FX12 (decals read depth) | take P2 item 4's D13 as written, including the `DeviceCaps` probe in the `rg8_unorm_storage_ok` idiom and "absent ⇒ unarmable" |
| Multiply blend factors | `BlendFactor::{Zero, One, SrcAlpha, OneMinusSrcAlpha}` only | FX12 | transparency **TK-1** (its R9), including the four `ffi.rs` constants and `const` guards that pass names |
| `textureCompressionBC` + BC `Format`s | not in `REQUIRED_CORE`; 19 formats, none BC | nothing mandatory; VRAM lever | owner question B2 |
| `vkCmdDrawIndexedIndirectCount` | not loaded; `drawIndirectCount` never chained | **nothing** | each class is one indirect command whose `instanceCount` the sim writes (plan D4) |
| Async compute, timeline semaphores | one `GRAPHICS \| COMPUTE` queue | **nothing** | the sim already overlaps opaque work through declaration order (plan F8); the occlusion map runs on update frames only |
| `independentBlend`, extended dynamic state 3, 64-bit atomics, mesh shaders | absent | **nothing** | MBOIT and R6 need none (transparency R6/R7); per-class pipelines are boot-frozen |
| A second ortho depth view | CSM depth pipeline exists (`CsmCasterScratch`) | FX7 | a render-graph pass with a new view-projection; **not an RHI gap** |
| `vertexPipelineStoresAndAtomics` | FFI field only; `REQUIRED_CORE` holds `samplerAnisotropy` and `geometryShader` (`crates/boyko_rhi_vulkan/src/device.rs`, `REQUIRED_CORE`) | **nothing** | FX9 observes the rain VS through the rasteriser instead (the probe arm, §8 FX9). A VS store would also need a SPIR-V capability census entry |
| Colour-renderable `R32G32Uint` + `R32Sfloat` | both in `boyko_rhi::enums::Format`; `vkGetPhysicalDeviceFormatProperties` is loaded [T] | FX9's probe (test-only) | from knowledge, both are in Vulkan's mandatory colour-attachment set. FX9 checks the format properties at test start and SKIPs by name if either is missing. **Not an RHI gap** |
| A per-variant pipeline layout (extra set-0 bindings under `-D WEATHER`) | `deferred_pbr`'s HWRT layout already extends set 0 per variant [T] | FX10 | the same mechanism, applied to six producer families. **Not an RHI gap** |
| `VK_KHR_fragment_shading_rate` (VRS) | zero hits | **nothing** (recorded candidate, §2.10) | a gap if the VRS trigger ever fires |

---

## 6. Shader surface

### 6.1 New leaves

All leaves are generic bodies with an `f32` oracle. Texture sampling, atomics and stores stay in
generator-owned skeletons (F13, D12).

| Leaf | Family | Divide? | Trig? | Rung | Oracle notes |
|---|---|---|---|---|---|
| `particle_color_ramp(keys, times_f16, inv_dt_f16, t)` | `Cf` | **no**: the segment weight is `(t − tᵢ) · inv_dtᵢ`, with `inv_dtᵢ` host-baked into `_r3`/`_r2` (§4.4); the segment is chosen by two ordered compares | no | FX1 | written `a + s·(b − a)`: **equal keys give an exact identity**, which is what keeps both particle pins byte-identical (§8, FX1). f16 reciprocals carry 2⁻¹¹ relative error in the weight: ≤ 0.13/255 per channel over a full swing, below 8-bit resolution except at rounding boundaries |
| `flipbook_frame(age01, age_s, count, fps, mode, rnd) → (frame, frac)` | `Cf` | **no** | no | FX1 | over-life: `float_to_uint(age01 · count)`; fps: `(float_to_uint(age_s · fps) + start) % count`, an integer modulo (OpUMod, exact, not `OpFDiv`); `age_s = life_total − life_remaining` (C13); `frac` needs scalar `floor` (E-FX1) |
| `flipbook_cell_uv(uv, frame, cols_log2, rows_log2)` | `Cf` | **no**: `1/cols` is an exact power of two built from `ushl` + `asfloat` | no | FX1 | no inset: the guard band makes the full cell safe at every stored level (§2.14). It differs from `UiSheet::frame_uv`, which divides and insets; the two are **separate contracts** |
| `flipbook_mv_warp(uv_a, uv_b, mv_a, mv_b, frac, strength, cell_a, cell_b, band_uv)` | `Cf` | no | no | FX2 | Lozar's forward and backward warps [P], each clamped to its cell rectangle shrunk by `band_uv` (4 `min`/`max`). Red mutations: flip one sign; drop the clamp |
| `particle_coverage(h, z_view, a, b, max_screen, near, inv_fade) → (h′, fade, cull)` | `Cf` | no | no | FX1 | `min(h, max_screen·(a·z + b))`; `clamp01((z − near)·inv_fade)`; `cull = z < near` (C14) |
| `sixway_weights(l_card) → [6]` | `Cf` | no | no | FX6 | `max(0, ±l)` per axis |
| `rain_drop(i, phase, cell, box, wind_offset, wind_vel) → (pos, stretch_dir)` | `Cf` | no | no | FX9 | PCG32 (`particle_rng_body` in `crates/boyko_shaderdsl/src/particle.rs`, shipped) + lattice wrap; **needs scalar `frac`** (E-FX1). The hash and lattice cell are integer, so bit-exact; the float position carries the FMA carve-out. `stretch_dir` = fall + **global** wind velocity (O3) |
| `splash_sprite(k, phase) → (xz, age01)` | `Cf` | no | no | FX9 | same wrap |
| `snow_sway(i, phase) → offset` | `Cf` | no | `sin` (on `Cf` [T]) | FX9 | render-only; a 1-ULP transcendental tolerance, stated in the oracle |
| `wet_surface(albedo, rough, metal, porosity_or_neg, wet, puddle, snow, exposure, n) → (albedo', rough', n′)` | `Cf` | no | no | FX10 | Lagarde 3b [P]; **wet = puddle = snow = 0 ⇒ bit-exact identity**; `porosity_or_neg < 0` selects the heuristic `(1 − metal)·rough`; `n′` blends toward +Y by `puddle · clamp01(n.y)` (§2.4) |
| `ripple_normal(uv, t4)` | `Cf` | no | `sin` | FX10 | Lagarde 2b's four layers and constants [P] |
| `vorticle_term(p, c, axis, inv_r2, s) → (vx, vy, vz)` | **`FieldScalar`** (`mul`, `add`, `sub`, `clamp01` only) | no | no | FX11 | reached from `boyko_physics` through `boyko_sdf_math` (the brick-decode edge [T]); **one op tree**, Eval = Emit modulo FMA contraction; GPU gate `\|Δv\| ≤ 2⁻¹⁸·s·r` (§2.7) |
| `curl_turbulence(p, freq, t)` | `Cf` | no | no | FX11 | GPU-only; hash lattice via `float_to_uint` on an offset domain |
| `decal_project(P, inv_box) → (uv, mask)` | `Cf` | no | no | FX12 | `mask` from `all3_lt`/`all3_ge` (shipped nodes) |
| Height response | — | — | — | FX8 | **reuses** `particle_sdf_response_body` (P1, `crates/boyko_shaderdsl/src/particle.rs`) with the map's normal; no new leaf |

### 6.2 E-rung items (new `Cf` nodes)

| Item | Nodes | For | Size |
|---|---|---|---|
| **E-FX1** | scalar `frac`, `floor` on `Cf::Scalar` (`vec2_frac` exists; the scalar form does not [T]) | the flipbook frame fraction (**FX1**, so E-FX1 lands before FX1); the rain and splash wrap (FX9) | XS |
| E-FX2 | `vec3_lerp`, `vec3_mul` (component-wise) | six-way, turbulence | XS |

`FieldScalar` gains **nothing**. The wind leaf is written within its existing algebra, which is
the point of choosing the analytic model.

### 6.3 Variant accounting (each row goes into [`../SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md) at its rung)

| Family | Axes after this design | `.spv` count |
|---|---|---|
| `particle_draw.fs` | `DEPTH_LINEAR (2) × SOFT (2) × OIT_STAGE (3) × PARTICLE_FX (3: off, FLIP, SIXWAY)` | **≤ 36** at the top of both ladders (transparency D18: 12). `SIXWAY` is legal on the alpha class only; FX6's resolver writes the legal table |
| `particle_draw.vs` | `DEPTH_LINEAR (2) × PARTICLE_FX (3) × PARTICLE_FOG (2)` | **12** (today 2). The fog axis is the volumetrics sibling's, decided VS-only (its D18) |
| `weather_probe.{vs,fs}` (test-only) | the rain VS with its clip-position line replaced; an FS writing position bits to `R32G32Uint` + `R32Sfloat` | 1 + 1 (FX9) |
| `particle_sim.comp` | today: base / `SDF_COLLIDE` / `SDF_COLLIDE_STATS`; adds `HEIGHT_COLLIDE`, `EVENTS` as **boot-resolved arms**, not a full product | ≤ +4 rows; the product is bounded by the resolver's legal table, written at FX5/FX8 |
| `particle_kickoff`, `particle_emit` | `+EVENTS` | +2 |
| `weather_draw.{vs,fs}` | `DEPTH_LINEAR (2) × {RAIN, SNOW}` | 4 VS + 2 FS |
| `weather_occ_sdf.comp`, `weather_ripples.comp` | — | 2 (the far SDF level runs the same module over a different box) |
| lit producers | `+WEATHER` on the shipping variants of `deferred_pbr` (6), `forward_opaque.fs` (2), `sdf_forward_march` (4), `vb_resolve` (2), `vb_shade` (4), `vb_shade_split` (4) | **up to +22** [T census of committed `.spv`]. FX10's resolver pins which subset is armable (the `hwrt_vis_mv` debug builds need not be) |
| `decal_draw.{vs,fs}` | `DEPTH_LINEAR (2)` | 2 + 2 |

The `PARTICLE_FX` consolidation is **C9**. Three separate feature axes (flipbook MV, six-way,
blend-flipbook) would have made the FS bound `12 × 8 = 96`. The cost of folding them is one
per-effect runtime branch inside the armed variant, divergent only in waves that mix effects.
FX2's perf gate measures that branch against the plain arm, for the fragment stage.

**Why three values, not two (open question 4 of the critique).** `PARTICLE_FX` is armed **per
class pipeline**, because the additive and alpha classes are separate pipelines (plan D10).
- The six-way VS writes 6 × RGB = 18 extra interpolant floats for *every* vertex of an armed draw,
  whether or not that particle's effect is six-way.
- With two values, a flipbook-only additive class would pay those 18 floats × 4 vertices on every
  spark.
- With three values, only the alpha pipeline is armed `SIXWAY`, and only when
  `ParticleConfig::fx == SixWay`. The additive class stays at `FLIP`.
- The VS output interface must match the FS input interface, so the FS needs the third value too.
  That is why the FS bound is 36 and not 24.
- Inside an armed `SIXWAY` alpha draw, plain and flipbook alpha effects still pay the 18 floats.
  That is inherent: smoke cards and other alpha particles share one sorted draw, and splitting them
  into two draws would break the sort. **FX6 measures this VS dark tax** on a scene of 100 k small
  alpha particles with no six-way effect present, armed vs plain. The ceiling is ≤ 5 % of
  `ZONE_PARTICLE_DRAW`. Above it, `SixWay` arming becomes an owner-visible cost line in B9.

**`-D PARTICLE_FOG` is decided by the volumetrics sibling** (its D18): one fetch per vertex at the
particle centre, VS-only. Additive particles take `c·T`, and alpha particles `c·T + S·α`, both
folded into the interpolated colour. So the axis multiplies the VS bound only (4 → 12 with the
three `PARTICLE_FX` values) and adds no FS arm. Pass 1 treated this as pending.

---

## 7. Numbers per effect (RTX 3060 Laptop)

### 7.1 The reference explosion, E1

**Definition, stated so it can be re-priced.**
- **Sparks**: 2 000 additive.
- **Fireball**: 16 motion-vector flipbook cards (4 samples each), average 8 % of screen, additive,
  emissive.
- **Smoke**: 48 six-way motion-vector cards (6 samples each), average 8 %, alpha.
- **Shockwave**: 1 refraction card, 30 %.
- **Flash**: 1 `PointLight` for 0.3 s.
- **Decal**: 1 at 5 %.
- **Debris**: 16 rigid bodies.
- **Trauma**: 0.5.

| Term | 1080p | 1440p | 4K | Basis |
|---|---|---|---|---|
| Sparks sim + draw | ≤ 17.4 + 37.9 µs | same | same | [M, SUSPECT] 10 240 row, an upper bound |
| Card fill (5.12 screens of layers) | 0.13–0.70 ms (on chip 0.13–0.16) | 0.23–1.25 (borderline: 2.2 MB region) | 1.01–2.81 (off chip: 5.0 MB region) | [E] research §2, both regimes |
| Card texture (28.16 screen-samples) | 0.29–0.35 ms | 0.51–0.62 | 1.14–1.40 | [E] at 166–204 Gtex/s [S] |
| **Card draw (overlapped … serialised)** | **0.29–1.05 ms** (texture-bound if on chip) | **0.51–1.87** | **1.14–4.2** | [E] |
| Shockwave card (R5 chain sample) | ≈ 0.015–0.045 ms | ≈ 0.03–0.08 | ≈ 0.06–0.18 | [E] 0.3 screens of layers + one chain sample |
| Flash light (per pixel in range) | ≈ 0.01 ms | ≈ 0.02 | ≈ 0.04 | [E] 2.07 Mpx × ~40 flops |
| Decal | < 0.01 ms | < 0.01 | < 0.02 | [E] |
| Six-way VS lighting (48 cards) | < 0.001 ms | same | same | [E] §2.5 |
| **GPU total, one E1** | **≈ 0.37–1.2 ms** | **≈ 0.62–2.05** | **≈ 1.3–4.5** | [E] |
| CPU fan-out | < 10 µs | | | [E] 16 debris at 30–100 ns [M-derived] + bursts + light |
| Pool share | 2 064 particles ≈ 190 KB | | | D2's 92 B |

**The spread is the fill model's.** FX0 replaces it with a measurement before any explosion rung
is gated on a millisecond, and FX6 re-measures it with the real six-way cards.

**The near-camera worst case (W7), E1 viewed from its centre.** All 64 cards (16 fire, 48 smoke)
lie beyond `near_fade`, and each is clamped to `max_screen = f`. A full-screen region is off chip.
Cards average 5.5 samples each (16 × 4 + 48 × 6, over 64).

| Clamp `f` | Screen-layers | 1080p draw | 1440p draw | 4K draw |
|---|---|---|---|---|
| none (a card can fill the screen) | 64 | 3.6–13.2 ms | 6.3–23.4 | 14.3–52.7 |
| 0.5 | 9.0 | 0.50–1.85 | 0.89–3.29 | 2.0–7.4 |
| **0.35 (provisional default)** | 4.41 | 0.25–0.91 | 0.44–1.61 | 0.98–3.63 |
| 0.25 | 2.25 | 0.13–0.46 | 0.22–0.82 | 0.50–1.85 |

[E: screen-layers = 64 · f² · 0.5625. Fill at 15.1–42 Gpx/s. Texture at 166–204 Gtex/s. The range
runs from "the larger term alone" to "the sum".] FX6 sets the default to the largest `f` whose
measured 1440p arm is ≤ 1 ms. The model predicts 0.25–0.35.

### 7.2 Rain (stateless, R-B)

**Inputs** [E]:
- 131 072 drops (heavy) or 32 768 (moderate). No primary count exists; NVIDIA ran 200k–5M on an
  8800 GTX [P].
- 50 % survive the VS frustum cull.
- 20 px average streak area at 1080p, and ×2 for 2×2-quad shading of 1–2 px-wide triangles.
- Streak area scales about linearly with resolution (width clamps near 1 px): ×1.33 at 1440p,
  ×2 at 4K.
- The VS costs 1.5 ns per drop [E ← 1.55 ns per tiny particle, M].

| Term | 1080p | 1440p | 4K |
|---|---|---|---|
| VS, heavy | 0.20 ms | 0.20 | 0.20 |
| Fill, heavy (2.62 Mfrag at 1080p) | 0.06–0.17 ms | 0.08–0.23 | 0.12–0.35 |
| Splashes (2 048 sprites × 40 px) | < 0.01 ms | < 0.01 | < 0.02 |
| **Heavy total** | **0.26–0.38 ms** | **0.28–0.44** | **0.32–0.56** |
| **Moderate total** | **0.07–0.1 ms** | 0.08–0.11 | 0.09–0.14 |
| Occlusion map, static layers | ≈ 0 amortised; on a re-centre frame, one static raster (comparable: 0.067 ms [M], one model) + 0.06–0.11 ms SDF strip [E] | same | same |
| Occlusion map, dynamic layer | ≈ 0.01–0.06 ms every frame for 1–10 vehicle-class casters [E, §2.3] | same | same |
| *Pool rain (R-A) extra* | *+0.16 ms, +12.1 MB* | | |
| *Layers (R-C), flat* | *0.15–0.42 ms* | *0.27–0.75* | *0.6–1.7* |

For comparison, Remember Me's whole rain system was 2.8 ms on PS3 at 720p [P].

### 7.3 Snow

- **Flakes**: the rain draw with the sway arm, +~10 ALU per vertex. Flakes are larger (≈ 30 px ×
  1.3 quad overhead) and fall at about 1 m/s. At 131 072 flakes: **≈ 0.25–0.4 ms at 1080p** [E].
  Premultiplied "over" (§2.2) blends the same 8 B per fragment as additive, so the price is
  unchanged.
- **Snow cover**: a term of the same `wet_surface` leaf, +2 ALU.
- **Deformable snow** (Batman: < 1 ms PS3, 2–4 MB, 2 surfaces/frame [P]) is **not designed**.
  Owner question B6.

### 7.4 Weather surface shading (`-D WEATHER`)

Per pixel:
- 3 map fetches inside the near map (static, SDF, dynamic), or 2 in the far map;
- 1 puddle-noise fetch and ≈ 0.2 ripple fetches;
- when porosity is authored, one 4 B load of DM's `MaterialDynGpu` word, from a row that is
  coherent across a material;
- ≈ 19 ALU.

| Resolution | Cost |
|---|---|
| 1080p | **0.04–0.06 ms** |
| 1440p | 0.08–0.10 ms |
| 4K | 0.17–0.22 ms |

[E, ≈ 4.2 fetches at 166–204 Gtex/s. Pass 1 counted 3.2 fetches, before the dynamic layer.] The register (dark-tax) cost is unknown until measured and is gated at
≤ 3 % of the producer zone (FX10). Ripples: 256² × 4 layers ≈ 5 MFLOP, < 0.01 ms (Lagarde: 0.14 ms
on PS3 [P]).

### 7.5 Wind, events, culling

| Item | Cost [E] |
|---|---|
| Wind in the sim | ≈ 25 flops per vorticle in range, ≈ 50–300 flops per opted-in particle after culling ⇒ 100k particles ≤ 30 MFLOP ≈ **≤ 0.003–0.004 ms** |
| Wind on the CPU | ≈ 6–15 µs per 1k soft-body vertices, with 0–2 vorticles in range after per-body sphere culling. SIMD instantiation of the `FieldScalar` body is a measured follow-up |
| Events | +1 atomic per event-producing wave; 32 B read per child in emit; 1.05 MB of rings |
| Frustum cull, 100k alive, 50 % culled | saves 1.6 MB of render writes (≈ 13 µs at 121 GB/s) plus the VS and clip work of 50 000 off-screen quads. That work is **at most** 50 000 × 1.55 ns ≈ 78 µs, because the 1.55 ns measured an *on-screen* tiny billboard including raster; an off-screen quad is rejected at the clipper and costs less. ⇒ **≈ −0.013 to −0.09 ms**, with FX1 measuring where in that range it lands. The test costs 6 plane dots ≈ 24 flops per particle ≈ 2.4 MFLOP |

### 7.6 VRAM

| Item | Size |
|---|---|
| Weather: near occlusion map (D32 + R32F, 1024²) | 8.39 MB |
| Weather: dynamic layer (D32, 512²) | 1.05 MB |
| Weather: far level (D32 + R32F, 512²) | 2.1 MB |
| Weather: ripples RG16F 256² | 0.26 MB |
| Weather: puddle noise R8 512² | 0.26 MB |
| Weather: splash sheet RGBA8 1024², mipped | 5.6 MB |
| **Weather total** | **≈ 17.7 MB** |
| Rain drops | **0** (vs 12.1 MB pool rain) |
| Draw table / event rings / wind table | 12 KB / 1.05 MB / 1 KB |
| **One explosion texture set, RGBA8, mipped** | **72.7 MB** (2048² colour 16.78 MB + six-way 2 × 16.78 MB + two 1024² RG8 motion-vector sheets 2 × 2.10 MB = 54.5 MB, × 4/3). The truncated chain of §2.14 drops < 0.1 % of that |
| The same set with BC7 colour + RG8 motion vectors | **22.4 MB** (16.8 × 4/3); motion vectors stay uncompressed (Lozar [P]) |
| Distortion | 0 beyond transparency R4's 11.1 MB chain at 1080p |

### 7.7 CPU

Steady state with both plugins: **< 5 µs per frame** [E]. Add < 10 µs per explosion frame and the
A1 budget the plan already states (≤ 15 µs). **Zero allocations** in any of it: staging is
`ScratchColumn`, and spawns go through `Commands`' existing path.

---

## 8. Rung ladder

**Unconditional on every rung** (the plan's rule, carried unchanged):
- the `goldens/PINS.toml` image hashes unchanged unless the rung names the pin it adds;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --no-fail-fast`;
- Miri where `unsafe` is new;
- every `*_spv_sync` / `*_edsl_sync` run **with dxc present** and the result reported (plan F15);
- manifest rows for every new `.spv`;
- author-only commits.

Every gate below is **red-first**: the test is written, shown red on the pre-rung tree (or on a
named mutation), then made green.

| Rung | Lands | Red-first | Golden | Perf |
|---|---|---|---|---|
| **FX0: fill instrument** (S) | `particle_lab` arms, per class, at 1080p / 1440p / 4K. **(A) coverage**: one untextured layer at 5 / 10 / 25 / 100 % of the screen. **(B) stacking depth**: k ∈ {1, 4, 16, 64} layers over one fixed E1-shaped region (15 % of the screen), untextured and with one 2048² RGBA8 sample. It separates on-chip from off-chip and fill from texture. **(C) near camera**: 64 full-screen layers, unbounded. `ZONE_PARTICLE_DRAW` same-scene (plan P2-item-4 warning) or after the `BottomOfPipe` restamp; re-run gate #17 excluding warm-up | non-vacuity: the readback's `alive` equals the requested count, and the covered-pixel count from a CPU projection is > 0 per arm; arm B's region is identical at every k (the same CPU projection) | none | **measured per-layer time per arm** replaces research §2's two-regime range in §7. E1's card draw is re-priced from arm B (E1-shaped), **not** by scaling arm A linearly |
| **FX1: ramp, flipbook, draw table, coverage, cull** (M; after E-FX1) | `particle_color_ramp` (host-baked f16 reciprocals in `_r3`/`_r2`), `flipbook_frame` (over-life and fps), `flipbook_cell_uv`, `particle_coverage`; C13 (`life_total` at offset 32); the `flags` bit budget (§4.4); `EffectDrawGpu` (48 B) at set 0 binding 2; the render `flags` repack; in-sim frustum cull; `PARTICLE_FX` introduced with `FLIP`; the sheet importer's contract (§2.14) | (a) a 16-frame **mutually distinct** sheet at two step counts (the UI S5 lesson, `ui_flipbook_gpu_golden.rs`) reds on a constant frame; (b) oracle equality per leaf; (c) a cull test with a particle whose centre is off-screen but whose quad overlaps it must still draw (conservative-radius mutation reds); (d) **non-uniform ramp times**: keys at t = [0, 0.1, 0.9, 1] with distinct colours match the oracle at 8 ages, red on a uniform-time mutation; gate #14's zero-`OpFDiv` test stays green unchanged; (e) **fps mode**: two effects with different lifetimes at the same fps show the same frame at the same age in seconds, red on an `age01`-based mutation; (f) **sheet contract**: the importer rejects one lit band texel and names the cell (CPU); a card drawn at ≈ cell/16 px, forcing `L_max`, shows no neighbour-cell colour, red with the full chain and no band; (g) **coverage**: a card nearer than `near_fade` covers 0 pixels; a card whose projection exceeds `max_screen` covers ≤ `max_screen` of the viewport height, red when the `min` is dropped | **`particle_additive` and `particle_sdf_collide` byte-identical**: `lab_effect`'s four colour keys are all white (`particle_scene/mod.rs`), so the ramp is an exact identity; a 1×1 grid is the identity UV; the lab cards are far below `max_screen` and beyond `near_fade` (asserted). New pin `particle_flipbook` | `ZONE_PARTICLE_SIM` within the plan's formula; draw delta reported same-scene; the cull saving measured against §7.5's 0.013–0.09 ms |
| **FX2: motion-vector flipbook** (S) | `flipbook_mv_warp` with the cell clamp; mode 2 in the `PARTICLE_FX` FS | oracle; mutation: flip the backward warp's sign; **warp containment**: a sheet whose motion vectors point outward at full strength shows no neighbour-cell colour, red when the clamp is dropped | new pin at `frac = 0.5` whose two source frames are distinct | FX-armed vs plain FS, same scene: the per-effect branch's cost (C9's price) |
| **FX3: the CPU layer** (M) | `boyko_vfx` with `VfxPlugin`; `ParticleBursts`; `ExplosionFx`/`ExplosionPreset`; `LightFade`; `CameraShake`; `ParticleBudget`; K1 `Lifetime`; `CueOnFire` read by nothing yet | burst consumed exactly once; lifetime despawns at the exact tick; trauma decays linearly and shake = trauma²; **containment**: two apps differing only by `VfxPlugin` have identical `EventUpdatePolicy` and schedule labels (plan gate #11's shape); **absence**: without the plugin the `ParticleBursts` resource does not exist, red on a mutation that inserts it from the particle plugin (O6); **C12**: 256 enabled idle emitters plus one bursting emitter, and the burst spawns, red on the pre-rung tree, where the D15 clamp drops it | new pin `vfx_explosion_f30` (needs FX1/FX2) | criterion: explosion fan-out < 10 µs; A1 ≤ 15 µs at 256 emitters + 256 bursts; burst push ≈ 10 ns at 50 contacts per frame |
| **FX4: soft particles** (M) | P2 item 4 as designed; `fade_distance` read from `EffectDrawGpu` (C1), push range 88 B (item 4's 80 B + FX1's clamp pair) | the item's own red list (cross-decoder equality, sentinel, 72 B push, bind-vs-push) | item 4's degrade gate | item 4's |
| **FX5: GPU events** (M) | event rings; `EVENTS` kickoff/emit/sim arms; the `_r2` lane; the collide flag latched in the substep loop and the append in the retirement block (§2.8) | four-boundary partition property **with events**: `alive + dead == CAP` at B0–B3; ring overflow is clamped and counted; atomic census **+1 site exactly**, red on a mutation that appends inside the contact branch; a particle colliding in two substeps of one step emits one collide event | new pin: a parent dying at step k spawns children visible at step k+1 | sim delta per event-producing wave |
| **FX6: six-way cards** (M) | `sixway_weights`; VS light loop (froxel lists / flat table) + 6 DDGI samples per vertex; `PARTICLE_FX = SIXWAY` on the alpha class only; `ParticleConfig::fx` | two light directions give **different** pins (non-vacuity); an oracle for the six weights; the resolver refuses `SIXWAY` on the additive class | new pins ×2 | VS cost vs light count (1 / 16 / 64). **FX0's arms B and C re-run with textured six-way MV cards**: the E1 smoke arm and the near-camera arm at 1080p / 1440p / 4K. They fix the default `max_screen` (§2.13) and feed the half-res and VRS triggers (§2.10). **VS dark tax**: 100 k small alpha particles, no six-way effect, `SIXWAY`-armed vs plain, ≤ 5 % of `ZONE_PARTICLE_DRAW` |
| **FX7: occlusion map** (M) | `weather_occ_mesh` (static casters, `Without<RainDynamic>`), `weather_occ_sdf` (toroidal strip march), `weather_occ_dyn` (per frame), `RainDynamic` + its auto-insert on `Added<RigidBody>`, the developer-build static validator, the re-centre policy | a roof fixture per leg: the map height under a mesh roof and under an `SdfPrimitive` slab equals the roof height; the overhang limitation is pinned as a stated test; mutation: drop the max ⇒ the SDF roof vanishes. **Moving caster, stationary camera**: a body that drives off a floor fixture returns the map height there to the floor within one frame, and one that arrives raises it within one frame. Red on the pass-1 policy (re-centre only). **Validator**: an unmarked static caster that moves is reported by name in a developer build | none (not visible alone) | static update zone reported as a ratio of `ZONE_GBUF_CSM_DEPTH` on the W2208 one-model scene **and** on a ≥ 1 000-instance scene (the comparable is not a bound); **the static zones are absent on non-update frames**; dynamic zone ≤ 0.06 ms at 8 × 50 k-vertex casters |
| **FX8: height and analytic collision** (M) | `HEIGHT_COLLIDE` arm, armed at boot by `ParticleConfig::collision ∈ {Height, SdfHeight}`; `ParticleCollider` table (≤ 32, D15 clamp) | OOB test at 33 colliders; a spark lands on a **mesh** floor (red before: it falls through); a spark lands on a **moving** body's roof through the dynamic layer; an effect requesting `HEIGHT` in an unarmed build has the bit cleared and `clamped_features` incremented | new pin `particle_height_collide` | sim delta per armed variant |
| **FX9: rain and snow** (M) | `WeatherPlugin`, `WeatherConfig` (default Off), `WeatherGpu`; `weather_draw` (rain additive; snow premultiplied "over", before the additive tail; splashes); the test-only `weather_probe.{vs,fs}` | **determinism through the rasteriser, with no VS store**: the probe VS is the generated rain VS with only its clip-position line replaced, by a pixel-grid address `(i mod W, i / W)`. The drop's world position travels as a `nointerpolation` interpolant, and the probe FS writes its bits to `R32G32Uint` (x, y) + `R32Sfloat` (z). An `edsl_sync` test asserts that the probe and shipped VS sources differ in that line only. The readback equals the oracle **exactly for the hash and lattice cell and within the FMA carve-out for the float position** (≤ 1 ULP per contractible pair, 4 pairs). The formats are checked at test start (SKIP by name). **Occlusion**: zero rain pixels inside the roof fixture's indoor volume. **Out-of-map kill**: a drop forced outside [0, 1] map coordinates covers 0 pixels. **Snow order-independence**: two draw orders of the same flakes give byte-identical images, red on a per-flake colour jitter. The phase wrap test at t = 24 h | new pins: `weather_rain`, `weather_snow` at a fixed phase | draw zone at 32 768 and 131 072 drops, against §7.2 |
| **FX10: weather shading** (L) | `wet_surface`, `ripple_normal`; `-D WEATHER` on the armable producer set, with every binding declared under `#if WEATHER` in set 0 (the HWRT precedent); puddle noise; the far level (`weather_far_mesh`, `weather_far_sdf`); `SurfaceWetness` as the level's one home | **WEATHER off ⇒ every producer `.spv` byte-identical, every host layout unchanged, and every pin unchanged**; armed at `wet = puddle = snow = 0` ⇒ images **byte-identical to disarmed** (the identity leaf). **Darkened once**: with both the dynamic-materials plugin and `WeatherPlugin` present, a wet material's albedo equals the single-application oracle, red on a double-application mutation. **Far level**: a bridge fixture 60 m from the camera keeps the road beneath it dry, red with the far level disabled (exposure = 1). Near and far updates never share a frame | new pins: a wet and a snow scene, both legs | **dark tax** of the armed-at-zero variant ≤ 3 % of each producer zone, else split the axis; far-update zone as a ratio of `ZONE_GBUF_CSM_DEPTH` |
| **FX11: wind** (M) | `WindField`, `WindVorticle`, `WindReceiver`; `vorticle_term` (`FieldScalar`: `mul`/`add`/`sub`/`clamp01`), `curl_turbulence`; sim, rain VS (global term only) and soft-body consumers | **one op tree**: the `*_edsl_sync` pin regenerates the Emit instantiation from the same body the CPU calls (runs everywhere); a mutation that swaps the cross product's operand order reds both instantiations. **GPU readback** of one sim step against the oracle within `\|Δv\| ≤ 2⁻¹⁸·s·r` per component (§2.7), gate #13's single-step shape. A soft body without `WindReceiver` has a byte-identical physics trace; table clamp at 33 | new pin: smoke bent by a vorticle | CPU µs per 1k soft vertices |
| **FX12: scorch decals** (M) | `Decal` entities, `decal_draw` (multiply), the two soft decoders reused | needs **TK-1**. Mutation: a wrong factor binding reds the `const` guards TK-1 adds beside `crates/boyko_rhi_vulkan/src/abi_guard.rs`'s existing four; one decal on an SDF floor and one on a mesh floor darken identically (cross-leg) | new pin, both legs | per-decal footprint cost |
| **FX13: shockwave cards** (S) | a CPU-animated card entity using transparency R5 | R5's own gates | new pin | R5's |

**Order and prerequisites:**

```
E-FX1 → FX0 → FX1 → FX2 → FX3 → FX4 → FX5 → FX6
FX7 → FX8 → FX9 → FX10 → FX11
FX10 after the owner's ruling on the wetness split (B11 ≡ DM Q5)
FX12 after transparency R3 + TK-1
FX13 after R4 + R5
```

**HDR (owner question B1) is recommended before FX6**: lit smoke in a post-tonemap 8-bit `lit` has
to be tonemapped per fragment and blended display-referred.

**Not scheduled**, each with its trigger:
- half-resolution alpha (§2.10: FX6's E1 smoke arm > 1 ms at 1440p, or its near-camera arm
  > 2 ms at 4K);
- variable-rate shading for large cards (§2.10: FX6 shows the E1 arm texture-bound above 0.5 ms at
  1440p; needs `VK_KHR_fragment_shading_rate`);
- fog density injection (owned by the volumetrics sibling's V10);
- ray-marched fireball, baked density playback or hero dense volume (the volumetrics sibling's
  V12, an owner value question there);
- particle shadow casting (not in transparency's matrix; would need a caster exemption from C8);
- lightning (B12);
- deformable snow (B6);
- SDF craters (B5 plus the dynamic SDF edit campaign);
- mesh-SDF bake (B7);
- BC textures (B2);
- lens droplets (B8).

---

## 9. Risks

1. **The fill model spans two regimes and up to 5.4×** (15.1 to 81.7 Gpx/s). Which regime applies
   depends on the overdrawn region, not the target size. Every explosion row rests on it, which is
   why FX0 comes first, measures stacking depth and texturing and not coverage alone, and why no
   explosion gate may quote a millisecond before it.
2. **LDR `lit`.** Fire clips at white, and lit smoke blends in display space. Authoring keeps
   contributions ≥ 2/255 (plan D7). HDR is B1.
3. **Dark tax** of the `WEATHER` variants (F24's +75 % class) and of `PARTICLE_FX`'s per-effect
   branch. Both carry an armed-at-zero A/B gate. The fallback is to split the axis.
4. **Variant growth**: `particle_draw.fs` goes 12 → ≤ 36, `particle_draw.vs` 2 → 12, and lit
   producers gain up to +22. Every one is a manifest row and a byte pin, which is the cost of R3's
   zero-when-off.
5. **Overhang blindness.** Drops are culled and surfaces kept dry under overhangs above an open
   floor, which is correct. Horizontal wind-driven rain under a balcony is not modelled. The sides
   of a vehicle in the dynamic layer read as sheltered for the same reason.
6. **An unmarked mover without a body** freezes into the static layer until the next re-centre.
   The developer-build validator names it; release builds pay nothing and do not detect it.
7. **Beyond ±128 m exposure is 1**, so a road under a distant bridge reads wet.
8. **Near-camera overdraw** is bounded per card, not per frame. Many overlapping effects around
   the camera still sum: two E1s at the camera cost twice the §7.1 row.
9. **Event latency.** One particle step (≤ 15.6 ms). A fast parent's children spawn a step behind.
10. **Pool contention** for explosions only (rain left the pool): 256 requests per frame. Clamps
    are now surfaced (§2.13).
11. **Time precision** of stateless effects. Mitigated by the host-wrapped phase; FX9 carries a
    24 h test.
12. **Boot-static SDF edits.** The SDF producer never sees a runtime SDF change, and SDF craters are
    impossible until that campaign lands.
13. **TAA ghosting** of thin rain and alpha smoke until transparency R6 lands (Deferred and VB
    only; Forward paths have no TAA).
14. **Wind parity firewall.** CPU consumers do not see turbulence. Stated, and not "fixed" by
    putting trig or `floor` into `FieldScalar`. CPU and GPU wind agree to one op tree and a bounded
    difference, not bitwise (§2.7).
15. **Cross-document dependency.** FX10's wetness depends on the dynamic-materials design
    accepting the split of §2.4: `SurfaceWetness` as the one home, porosity as its lane, no
    `Wetness` row curve. Until the owner rules, FX10 cannot land. FX7–FX9 do not wait.
16. **The `E-FOG` grid statement is internally inconsistent.** It is 160×90×64 while requiring
    "fog grid == froxel grid", and the tree's froxel grid is 16×9×24 [T]. The volumetrics sibling
    resolves it with a fog-owned grid (its C1). Density injection and the particle fog fetch
    inherit that grid; this design reads it and never writes it.

---

## 10. Decisions

### 10.A PERF / ARCHITECTURE: taken here, with the number that decides each

| # | Decision | The number |
|---|---|---|
| X1 | Fireball and smoke are motion-vector flipbooks; smoke is six-way; gas simulation is rejected for gameplay; ray-marched noise is deferred to volumetrics | per-pixel cost 3–6× lower than C; Epic scopes gas to hero shots [P] (§2.1) |
| X2 | Rain and snow are a stateless analytic draw, not pool clients | −0.16 ms and −12.1 MB at 131k; ties layers at storm density, 2–6× cheaper at moderate density (§2.2, §7.2) |
| X3 | Mesh-leg occlusion and collision use a top-down map with static (re-centre), SDF (re-centre) and dynamic (per-frame) producers, plus a far level for surface exposure; depth-buffer collision is rejected; mesh-SDF is not scheduled | static ≈ 0 ms amortised; dynamic ≈ 0.01–0.06 ms per frame; 11.5 MB of maps; one map, five consumers (§2.3) |
| X4 | Puddles, snow cover and the exposure term are `-D WEATHER` leaves in the lit producers, with bindings under `#if WEATHER` in set 0; the level lives only in `SurfaceWetness`; the darkening is applied exactly once (the per-pixel route is recommended, pending the merged owner question B11 ≡ DM Q5); porosity is the heuristic until the dynamic-materials lane lands | 0 when off; 0.04–0.06 ms at 1080p armed (§2.4) |
| X5 | Scorch is a box decal at the seam (multiply); clustered decals are deferred | 0 producer variants; ≈ 0.01 ms per decal (§2.4) |
| X6 | Smoke cards are lit per vertex, six-way; D11 is kept for small particles; atlas and volume are rejected | 4 evaluations per card vs 256–4096 (§2.5) |
| X7 | Distortion is a translucent card through transparency R5 | 0 particle variants; 0 new images (§2.6) |
| X8 | Wind is analytic: a `FieldScalar` vorticle leaf (`mul`/`add`/`sub`/`clamp01`, no `sqrt`) shared by CPU and GPU; turbulence is GPU-only; rain sees the global term only | 1 KB; one op tree, `\|Δv\| ≤ 2⁻¹⁸·s·r`; no CPU mirror (§2.7) |
| X9 | Events arrive next frame through a double-buffered ring, one child per record; the collide flag is latched in the substep loop and appended in the retirement block | D3 preserved; 1.05 MB; +1 atomic site, +1 atomic per wave (§2.8) |
| X10 | Authoring is POD rows; mode and mask bits in `EffectParamsGpu::flags`, values in its five spare words (including the ramp's reciprocals); `ParticleSim.color_rgba8` becomes `life_total` | 1 dispatch; row stays 128 B; zero `OpFDiv` in the sim (§2.9, §4.4) |
| X11 | One `PARTICLE_FX` draw axis with three values, armed per class pipeline | FS bound ≤ 36, not 96; VS 12 with `PARTICLE_FOG`; six-way interpolants never on the additive class (§6.3) |
| X12 | Per-effect draw table; `fade_distance`, band, coverage bound become per-effect | 12 KB total, 0 B per particle (C1) |
| X13 | Per-contact one-shots are burst requests; A1 skips zero-count requests | 0.5 µs vs 3–10 µs at 50 contacts per frame; idle emitters no longer consume `MAX_EMITTERS` (§2.12) |
| X14 | In-sim conservative frustum culling | ≈ −0.013 to −0.09 ms at 100k with half culled (§7.5) |
| X15 | Half-resolution particles and VRS are not scheduled; both are triggered by FX6's measured arms | net ≈ 0 at 1080p (§2.10) |
| X16 | BC is not a prerequisite | 3.2× VRAM stated for B2 (§2.11) |
| X17 | New optional crate `boyko_vfx`; wind and shake in `boyko_scene`; `Lifetime` as a kernel or std-lib request | the physics dependency; `boyko_scene`'s charter (§3.1) |
| X18 | Rain and splashes are additive-class, drawn last; snow is premultiplied "over", unsorted, before the additive tail; all inside the TAA loop with R6's mask | order-independence by construction for both (one flake colour per frame); "additive last" unchanged (§2.2, §4.1) |
| X19 | A per-effect coverage bound: screen-size clamp + near fade/cull in the draw VS; default `max_screen` fixed by FX6 | near-camera E1 at 1080p: 3.6–13.2 ms unbounded → 0.25–0.91 ms at 0.35 (§2.13, §7.1) |
| X20 | Flipbook sheets: POT cells, a chain truncated at 8-texel cells, an empty `cell/16` guard band; the MV warp clamps to the cell | 0 ALU for the base sample, 8 ALU for the warp clamp; < 0.1 % VRAM (§2.14) |
| X21 | Particle arming is boot-frozen config (`ParticleConfig::collision`, `::fx`); a late effect that asks for an unarmed feature is clamped and counted | no pass declared on demand, no silent drop (§2.13) |

### 10.B VALUES / SCOPE: to the owner

1. **HDR scene colour** (the plan's OQ1, transparency ballot 1, reflections R4a, and the post/AA
   sibling's rung PX1 with its own owner question). This document is one more consumer asking.
   Recommended: land PX1 before FX6 (lit smoke). FX0–FX5 do not need it; they run under the LDR
   authoring rule (contributions ≥ 2/255, plan D7).
2. **Texture compression.** Options: an in-house BC7/BC5 encoder + container now; a third-party
   *offline* encoder as a tool (a rule-5 exception with its reason stated); or accept **3.2×**
   flipbook VRAM (72.7 vs 22.4 MB per explosion set).
3. **Effect authoring surface**: a Gaia on-disk format for `ParticleEffect` / `ExplosionPreset`,
   and hot reload.
4. **Hero volumetric explosions** (ray-marched noise or gas). This is asked once, in the
   volumetrics sibling (its V12 value question), and is listed here only so the VFX side sees the
   dependency. The flipbook route of §2.1 does not wait on the answer.
5. **Destructible SDF craters** (explosions writing subtractive `SdfEdit`s). This needs the
   dynamic SDF edit campaign.
6. **Deformable snow** (a Batman-style height texture).
7. **Mesh-leg collision beyond floors** (a mesh-SDF bake: walls, overhangs).
8. **Camera-space rain** (lens droplets, screen streaks).
9. **Budgets**: concurrent explosions, `CAP` (the plan's OQ2) and `MAX_EMITTERS` (OQ3) as shared
   by bursts.
10. **Weather as gameplay state** (Aether-driven transitions) or a per-level setting.
11. **The wetness split: one question, shared with the dynamic-materials design's Q5.** Which
    applies Lagarde's darkening: the per-pixel `wet_surface` (exposure-masked, dry under roofs) or
    DM2's row-writing `Wetness` curve (no per-pixel cost, cannot mask indoor floors)? Recommended:
    **per pixel**. DM keeps porosity as an authored lane and `SurfaceWetness` as the level's one
    home, and drops the `Wetness` curve kind (§2.4, with the numbers). Pass 1's version of this
    question (a `MaterialXGpu` porosity lane) is **withdrawn**: those lanes are full, and the opaque
    producers never bind that table.
12. **Lightning** as a weather element: a flash through the light table, a bolt ribbon (P4), a sky
    pulse (the post/sky threads). No primary source was retrieved (research §7.6).

---

## 11. Unverified, open, and requests

- **Every millisecond here is [E]** except the in-tree anchors (gate #17, SUSPECT; the CSM
  0.067 ms) and the published console figures. FX0 is the first measurement; FX9 and FX10 carry
  their own.
- **The per-drop VS cost of 1.5 ns** is inferred from a VS that reads a 32 B record. The rain VS
  replaces that read with a hash and one map fetch, and FX9 measures it.
- **`E-FOG`'s grid inconsistency** (risk 16) is resolved by the volumetrics sibling, not here. The
  `PARTICLE_FOG` variant bound (§6.3) follows that design's D18: VS-only, so the VS bound is 12
  and the FS has no fog arm.
- **The research could not retrieve** God of War's wind numbers, Epic's ribbon and light renderer
  pages, or any Frostbite or Decima particle-authoring source (research header).
- **Kernel and crate requests** born from this design, each its own pass:
  - **K1**: `Lifetime` + despawn as a kernel or std-lib capability.
  - **K2**: a SIMD instantiation of `FieldScalar` for SoA CPU consumers (the soft-body wind loop).
  - **K3**: surface `clamped_spawns`, `clamped_features` and the event-ring overflow count in the
    diagnostics counters.
- **Requests to the dynamic-materials design.** This role writes only the two VFX documents, so
  the matching sentences there are requests for that design's next pass, not edits made here:
  - **DM-1**: state the §2.4 split in its §2 table and its Q5, or record its disagreement.
  - **DM-2**: carry porosity as a `MaterialDynGpu` lane. The suggestion is `rim` word 3 plus a
    `POROSITY` flag bit.
  - **DM-3**: drop the `Wetness` curve kind if the owner rules per pixel. Otherwise, instantiate
    `wet_surface` with `wet ≡ 0`.
- **From knowledge, not fetched:** `R32G32Uint` and `R32Sfloat` being in Vulkan's mandatory
  colour-attachment set (FX9 checks at run time), and the magnitude of GPU `sqrt` error. The design
  no longer depends on the latter: the wind leaf has no `sqrt`.
- **Not re-derived**: the physics step cost of debris bodies (the physics campaign's number), and
  the light system's per-frame repack cost while a flash fades (FX3 measures it).

---

## 12. Review log

**Critique pass 1:** CHANGES_REQUESTED, 0 critical, 7 important, 9 optional, plus 5 open
questions. **All 16 remarks are accepted; none is refuted.** Every tree citation in the critique
was re-opened at `6394bc5e` and held. One of this revision's own first drafts was wrong and is
recorded under W1.

| # | Remark | Outcome | Where |
|---|---|---|---|
| W1 | Wetness decided twice (here and in DM2); `SurfaceWetness` vs `WeatherState.wet`; B11 answers DM Q5 silently | **Accepted.** The level has one home (`SurfaceWetness`; `WeatherState.wet` deleted). Each datum has a named owner. The invariant "darkening applied exactly once" is decided here and gated at FX10. The split itself is one owner question shared with DM Q5 (B11 rewritten), with a per-pixel recommendation and its numbers. Pass 1's B11 is withdrawn as infeasible: `MaterialXGpu` is full and unbound by opaque producers (DM F10). This role cannot write the dynamic-materials document, so the "stated in both documents" half is recorded as requests DM-1 to DM-3 (§11). A draft of this revision put the weather bindings in "free" set 2. Checking found that `vb_geom_fetch.hlsli` owns set 2, so they extend set 0 under `#if WEATHER` instead | §0.1, §1, §2.4, §3.2, §3.3, §10.B-11, §11, FX10 |
| W2 | Occlusion map freezes dynamic casters; no out-of-map fallback; 0.067 ms is a comparable | **Accepted.** Static layer = `ShadowCaster ∧ ¬RainDynamic` on re-centre. A per-frame 512² dynamic layer, with `RainDynamic` auto-inserted on `Added<RigidBody>`, priced at 0.01–0.06 ms per frame. A developer-build validator for body-less movers. Drops are kept inside the map by sizing, and the VS kills any outside it (Remember Me's rule). A far level (512² over 256 m, +2.1 MB) serves surface exposure, with exposure = 1 beyond ±128 m as a stated artifact. 0.067 ms is restated as a one-model comparable, and FX7 measures a ≥ 1 000-instance scene. FX7 gains the moving-caster red test; FX10 the far-level bridge test | §1, §2.3, §4.1, §4.2, §7.2, §7.6, FX7, FX8, FX10 |
| W3 | The fill model ignores the working set; FX0 cannot measure E1's shape; the half-res trigger can never fire | **Accepted.** Research §0/§2 now carries two regimes, on chip (up to the ROP peak) and off chip, decided by the overdrawn region, with Kanter's tiled caching cited [S, fetched]. E1's draw range does not move, because its lower end was already texture-bound (0.29 ms). The stated regime changes to "texture-bound when on chip". FX0 gains a stacking-depth arm over a fixed E1-shaped region, textured and untextured, plus a near-camera arm. FX6 re-measures both with six-way cards. The half-res trigger now points at FX6's arms | research §0, §2; §2 intro, §2.1, §2.10, §7.1, FX0, FX6 |
| W4 | Ramp and fps mode need a divide or an unbudgeted lane; `flags` is not in the budget | **Accepted.** `EffectParamsGpu::flags` (no bit defined [T]) takes every mode and mask bit. That frees `_r3` and half of `_r2` for three host-baked f16 segment reciprocals. `ParticleSim.color_rgba8` becomes `life_total` (C13), the critique's own observation, so fps mode needs a subtract, not a divide. The frame modulo is integer. Gate #14 stays green unchanged, and FX1 gains a non-uniform-time red test and an fps test | §0.3 C13, §4.4, §6.1, §6.2, FX1 |
| W5 | Flipbook atlas has no mip or warp containment | **Accepted.** §2.14: POT cells (a box blit is then clean down to 1-texel cells, NVIDIA 2004 [P, fetched]); a chain truncated at 8-texel cells through `TextureDesc::mip_levels`, so the shared sampler is unchanged; an empty `cell/16` guard band checked by the importer, which makes the inset unnecessary; the MV warp clamped to the band-shrunk cell. A coarse-LOD red fixture at FX1 and a warp-containment red at FX2. Leaf signatures carry the band from the start, so no pin is re-blessed later | §0.3 C11, §2.14, §4.4, §6.1, FX1, FX2 |
| W6 | FX11's bit-exact premise; FX9's VS readback needs an unenabled feature | **Accepted.** The vorticle leaf is rewritten `sqrt`- and divide-free. The parity claim is restated as "one op tree" plus a derived absolute bound of 2⁻¹⁸·s·r under the crate's own FMA carve-out [T]. FX9 observes the shipped VS through the rasteriser with a probe arm differing in one line, needing no VS store and no RHI enable. The formats are present and checked at run time. §5 records `vertexPipelineStoresAndAtomics` as not needed | §2.7, §5, §6.1, §6.3, FX9, FX11 |
| W7 | No near-camera overdraw worst case; the budget is blind to coverage | **Accepted.** C14: a per-effect screen-size clamp (Unity's Max Particle Size [P, fetched]) and a near fade/cull (Unity's Camera Fading [P, fetched]). 3–5 ALU per vertex, no divide. Priced with texture included: 3.6–13.2 ms unbounded vs 0.25–0.91 ms at the provisional default 0.35, at 1080p. FX6 fixes the default from a measured 1440p arm | §0.3 C14, §2.13, §4.4, §6.1, §7.1, FX1, FX6 |
| O1 | C3 never compared against `ParticleEmitter::burst`; the persistent slot is an artefact of A1 | **Accepted.** Both of pass 1's arguments are withdrawn. C12 (A1 skips zero-count requests) fixes the real defect, with a red test at FX3. Bursts are re-argued on per-contact sparks | §0.3 C3/C12, §2.12, FX3 |
| O2 | Additive snow vanishes in LDR against bright sky | **Accepted.** Snow is premultiplied "over", unsorted: order-independent because flakes share one colour. It draws before the additive tail | §2.2, §4.1, §7.3, X18, FX9 |
| O3 | Local-vs-global wind contradiction for rain | **Accepted.** Rain uses the global term in both position and orientation | §2.2, §4.4, §6.1 |
| O4 | `PARTICLE_FOG` already decided (VS, per vertex) | **Accepted.** VS bound 12, no FS arm | header, §0.1, §6.3, §11 |
| O5 | "Particles become `csm_trans` casters" contradicts transparency's matrix | **Accepted.** Withdrawn; C8's caster exemption noted | §2.5, §8 |
| O6 | FX3's absence test cannot fail | **Accepted.** `ParticleBursts` is inserted only by `VfxPlugin`; the gate tests resource presence, with a named red mutation | §3.1, FX3 |
| O7 | FP32 range inconsistent with the clock pair | **Accepted.** 10.65–13.08 TFLOPS at 1.387–1.703 GHz; the strip march carries 6.9–13.1 | research §2; §2.3 |
| O8 | Culling saving priced at the on-screen 1.55 ns | **Accepted.** Restated as 0.013–0.09 ms, with 0.09 as the upper bound; FX1 measures | §0.3 C8, §7.5, X14 |
| O9 | Missing: VRS, baked 3D density playback, lightning | **Accepted, recorded.** VRS: an RHI gap, triggered only for the texture-bound regime (DirectX VRS spec [P, fetched]). Baked playback: row A′, deferred to V12 (UE Heterogeneous Volumes [P, fetched]: "Experimental"). Lightning: B12, no source retrieved | research §4.6b, §6, §7.6; §2.1, §2.2, §2.10, §5, §10.B |

**Open questions for the architect, answered:**
1. **On-collide append:** flagged in the substep loop, appended once in the retirement block with
   one wave-aggregated atomic. So +1 site and +1 atomic per wave hold, gated at FX5 (§2.8).
2. **`-D WEATHER` bindings:** declared entirely under `#if WEATHER`, appended to each producer's
   own set 0. This is `deferred_pbr`'s HWRT precedent. Disarmed `.spv` and host layouts are
   unchanged, and FX10 gates both. No set index below 4 is free in every producer (§2.4).
3. **`n_blend` on Deferred:** toward world up, weighted by `puddle · clamp01(n.y)`. It needs no
   geometric normal on any path (§2.4).
4. **`PARTICLE_FX` per class:** yes, with three values so the additive class never carries six-way
   interpolants. FX6 gates the VS dark tax, not only the FS branch (§6.3).
5. **Boot-known arming:** yes. `ParticleConfig::collision`/`::fx` are boot-frozen. A late effect
   asking for an unarmed feature is clamped and counted (§2.13).

**Kept, as the critique asked:** C2's repack onto the unread `flags`; the corrections of
`TextureDimension`, `Cf` and `BlendFactor`; the 22-`.spv` producer census; the crate layering;
stateless rain with a host-wrapped phase; D13-style zero-when-off; FX7's absent-zone gate (now for
the static producers); the wet = 0 identity; C1's 0 B per particle; red-first throughout.
