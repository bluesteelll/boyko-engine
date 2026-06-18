# Render P4b — Tile-Cull / Coarse Pre-Trace (FINAL design, critic-resolved)

> Status: implementation-ready. Architect→critic→revise complete; every critic finding (C1–C4 conservativeness holes, W1 false-bit-identity, W2–W4, O1–O3) resolved. P4a (`sdf_field.hlsli` `field_distance()` gateway) is committed (971ed0c). P4b is a strict FIELD-CONSUMER — it never edits the frozen field math.

## Goal
A 1/8-res CONSERVATIVE coarse cone-trace emits, per 8×8 tile, a `TileBound{near_t, far_t, flags}`. The fine marcher then seeds `t = near_t` (skips the proven-empty prefix) and early-outs `EMPTY` tiles into the existing mesh/background composite. Win: on a sparse SDF scene most tiles are EMPTY or large-`near_t`, collapsing the per-pixel `MAX_IT=128` loop.

Gate (W1 — three contracts, NOT hit-bit-identity):
- (i) field-eval SPIR-V byte-identical (P4a property re-verified — P4b adds only a comment + a const to sdf_field.hlsli).
- (ii) image ±2/255 vs the un-culled marcher (fp sphere-tracing from a different start `t0≤t_hit` converges to the SAME surface within EPS, not a bit-identical `t`; ±2/255 absorbs the ≤1-LSB color shift).
- (iii) cull-OFF (`coarse_enabled=0`) BYTE-IDENTICAL to today's marcher (the 0%-gate anchor).

## Decisions (all critic-resolved)

### D1 (C1) — pin the coarse ray to the tile's TRUE geometric center via the fine ray-gen's EXACT arithmetic
Tile (tx,ty) covers fine pixels [tx·8 .. tx·8+7]². Center pixel `px_c = tx*8 + 3.5`. Coarse `u_c = ((px_c+0.5)/w)*2-1 = ((tx*8 + 4.0)/w)*2 - 1` (3.5+0.5=4.0 exact in fp) — derived line-for-line from the fine `composite_ray` so host + shader emit identical ops. NOT half-res-grid sampling (`(tx+0.5)/(w/8)` is not fp-identical → drifts the center → eats the margin).

### D2 (C1) — ORTHO cone radius = sqrt(2)·(9/w)·HE (footprint-enclosing + 1 full pixel positive margin)
Parallel rays → constant-radius cylinder. World pixel pitch Δ=(2/w)·HE. Tight footprint-enclosing radius = sqrt(2)·4·Δ = sqrt(2)·(8/w)·HE (center-to-corner-center sqrt(2)·3.5·Δ + half-pixel-footprint sqrt(2)·0.5·Δ). Add a FULL extra pixel of fp-ULP-safe slack: `r_ortho = sqrt(2)*(9/w)*HE`. (The old (8/w) was zero-margin → C1 hole.)

### D3 (C4) — PER-TILE perspective half-angle from the exact ray-gen (aspect + tan-convexity + footprint)
For each tile, `alpha_tile = max over the 4 corner pixels' OUTER-EDGE directions of acos(dot(d_center, d_corner_edge))`, computed from the exact ray-gen `dir = forward + right·(ndc_x·aspect·tan_half_fov) + up·(ndc_y·tan_half_fov)` (captures aspect anisotropy + tan-convexity of edge tiles + the footprint via the ±4.0 outer edge, not the ±3.5 corner center). `alpha_tile_safe = alpha_tile + ALPHA_MARGIN` (1e-4 rad). Per-tile (not a global scalar/max) — same shader cost, tighter near_t. The rejected scalar 4√2·(center per-pixel angle) under-encloses (ignores aspect + convexity + footprint) → holes.

### D4 (C3 + W4) — cone-aware step with radius-GROWTH and Lipschitz-L division
At t, d=field_distance(p) (true lower bound), cone radius r(t) (ortho r_ortho const; perspective t·tan(alpha_tile_safe)). Safe advance: far cross-section at t+Δt (radius r(t)+Δt·tan) must lie within the empty sphere of Euclidean radius d/L → `Δt = (d/L − r(t)) / (1 + tan(alpha_tile_safe))` (ortho: /(1+0)). The old Δt=d−r over-stepped perspective by (1+tan) → holes; /L corrects smin's super-Lipschitz under-report. Cone-entry: when `d/L − r(t) ≤ EPS_COARSE`, RECORD near_t=t and STOP (the old EPS_COARSE floor over-stepped at grazing → wrong-EMPTY → hole).

### D5 (W2) — far_t/near_t clamping + exhaustion fallback
`far_t = min(MAX over the 8×8 depth texels of depth→t, T_MAX)` (MAX = conservative: deepest mesh; a cleared/out-of-range texel → T_MAX; coarse march bounds at T_MAX not 1e30). near_t clamped [0, far_t]. Partial-edge tiles: out-of-range depth fetch → T_MAX (NOT clamp-to-edge). MAX_IT_COARSE exhaustion → NON-empty, near_t=0 (safe full-march fallback; NEVER near_t=last_t → hole).

### D6 (W3) — EMPTY fast-path runs the marcher's mesh/background composite
An EMPTY tile (no SDF surface in cone in front of deepest mesh) can still be MESH-covered. The fine EMPTY path reads gDepth + writes `has_mesh ? MESH_COLOR : BACKGROUND` + neutral normal/material (the marcher's existing else-if(has_mesh)/else arms with hit=false) — NOT blind BACKGROUND (which would erase mesh → golden regression).

### D7 (W4) — field lower-bound invariant + smin Lipschitz L
Add to sdf_field.hlsli a documented INVARIANT: every op MUST return ≤ true Euclidean distance-to-surface (Hart precondition); an over-estimating op voids P4/B1/B7. Current ops: sd_sphere/sd_box/min/max EXACT; smin/smax UNDER-report (lower bound holds). `FIELD_LIPSCHITZ_L = sqrt(2)` — the IQ poly smin's k-INDEPENDENT worst-case spatial gradient (two unit-gradient fields at 90° in the blend; k controls band WIDTH not peak slope). The cone step divides by L. A host property test numerically asserts max|∇ field_distance| ≤ L over the scene (the tripwire). OWNER CALL: √2 is the safe default; hard-CSG-only (k=0) scenes could set L=1 for tighter steps, but any future smooth edit re-introduces super-Lipschitz (the test fails loudly).

### D8 (C2) — near_t ≤ every corner-hit is a COROLLARY of D2/D3 (enclosure-with-margin) ∧ D4 (non-skipping step), NOT independent.

## Conservativeness proof
- Claim 1 (enclosure w/ margin): ortho margin sqrt(2)·(1/w)·HE > fp-ULP; perspective lateral offset t·sin(angle_i) < t·tan(alpha_tile_safe)=r(t). ∎
- Claim 2 (non-skipping step): far cross-section's farthest point from p = Δt(1+tan)+r(t) = d/L (exactly the Euclidean clearance) → swept segment surface-free; induction ⇒ never skips first cone-surface contact. ∎
- Claim 3 (near_t ≤ corner-hit, corollary): at any pixel's hit t_i, its hit point ∈ S is within the cone (Claim 1) ⇒ d_center(t_i)/L − r(t_i) ≤ 0 ≤ EPS_COARSE ⇒ cone-entry ≤ t_i; Claim 2 ⇒ near_t = first cone-entry ≤ t_i. ∎
- Claim 4 (EMPTY correct): reaching far_t without cone-entry ⇒ cone (containing every in-tile ray to far_t ≥ every pixel's mesh t) is surface-free ⇒ no pixel has SDF in front of mesh. ∎
- Claim 5 (exhaustion safe): near_t=0 non-EMPTY ⇒ full march ⇒ identical to un-culled. ∎

## Data structures
`#[repr(C)] TileBound { near_t: f32@0, far_t: f32@4, flags: u32@8, _pad: u32@12 }` (16B std430, const-asserted). `TILE_FLAG_EMPTY=1`, `TILE_SIZE=8`. Grid `tiles_w=(w+7)/8, tiles_h=(h+7)/8`, buffer `tiles_w*tiles_h`. Shader mirror identical. `coarse_enabled` = a 4-byte push constant on the fine pipeline (keeps the 80-byte Camera UBO const-asserts frozen).

## Algorithms
Coarse (`sdf_tile_cull.hlsl`, [numthreads(64,1,1)], i<tiles_w*tiles_h): coarse_ray (D1) → far_t = min(max 8×8 depth→t, T_MAX) → march `for it in 0..MAX_IT_COARSE { if t>=far_t break; d=field_distance(p); r=ortho?r_const:t*tan_a; budget=d/L−r; if budget<=EPS_COARSE {near_t=t;break}; t+=budget/(1+tan_a); if t>T_MAX break }` else (exhausted) near_t=0 non-empty → emit TileBound (near_t≥0 → flags=0; exhausted → near_t=0 flags=0; else → near_t=0 flags=EMPTY).
Fine (`sdf_gbuffer_composite.hlsl`, gated): `if coarse_enabled { tb=Tiles[ty*tiles_w+tx]; if tb.flags&EMPTY { mesh/bg composite; return } t=tb.near_t } else t=0.0` then the unchanged MAX_IT loop.

## Sync
2 dispatches: coarse (writes Tiles RW) → buffer barrier (Tiles COMPUTE_SHADER/SHADER_WRITE → COMPUTE_SHADER/SHADER_READ) → fine (reads Tiles). Depth already SHADER_READ_ONLY (P1b), read by both. Per-tile disjoint writes → race-free. Manual barrier (F-GRAPH deferred).

## Implementation plan
1. compute.rs: TileBound POD + consts (MAX_IT_COARSE=64, EPS_COARSE=0.001, ALPHA_MARGIN=1e-4, FIELD_LIPSCHITZ_L=sqrt(2)) + const-asserts + tile_grid_extent.
2. compute.rs: coarse_ray (from composite_ray, px_c=tx*8+4.0 form), ortho_cone_radius, perspective_alpha_tile (4 outer-corner dirs).
3. compute.rs: golden_tile_bound (host cone-trace mirror) + golden_composite_pixel_culled (wraps golden_composite_pixel_ex with near_t seed + EMPTY composite; coarse_enabled=false bit-identical).
4. sdf_field.hlsli: + the W4 invariant comment + FIELD_LIPSCHITZ_L (no field-math edit).
5. sdf_tile_cull.hlsl (NEW): bindings (Buf t0, gDepth t1, Tiles u6, Camera b5) + include + the coarse algorithm. DXC + spirv-val.
6. sdf_gbuffer_composite.hlsl: + Tiles binding u6 + coarse_enabled push + the gated cull prefix. Recompile + spirv-val.
7. host wiring: binding 6 on set-0 (both pipelines), the coarse pipeline, the inter-dispatch barrier, the fine push-constant range.

## Gate / tests
- field-eval SPIR-V byte-identical (i); image ±2/255 vs un-culled (ii); cull-OFF byte-identical (iii).
- Conservative golden (GPU): P0 scene cull-ON within ±2/255 of cull-OFF (a hole = a >tol pixel).
- Negative Test-B (GPU): a too-aggressive cull TRIPS the golden; + a MESH-COVERED EMPTY tile asserts MESH_COLOR shows (W3).
- EXHAUSTIVE host conservative-invariant (ortho): every tile, all 64 footprint corners within ortho_cone_radius (exact u/v arithmetic) (C1).
- Host perspective alpha (C4): every tile's 4 outer-corner dirs within perspective_alpha_tile.
- Randomized perspective sweep with analytic sphere/box first-hit oracle: golden_tile_bound.near_t ≤ min in-tile pixel hit; EMPTY ⇒ no in-tile hit before mesh (C2/C4 oracle).
- Host/GPU agreement (golden_tile_bound vs the GPU Tiles buffer). W4 Lipschitz property test (max|∇field| ≤ L).
- debug_asserts: near_t∈[0,far_t]; far_t≤T_MAX; EMPTY⇒near_t==0; 0<alpha_tile_safe<PI/2.

## Owner VALUE/SCOPE call (defaulted, overridable)
FIELD_LIPSCHITZ_L = √2 (sound default + the property-test tripwire). Owner may set L=1 if the scene is hard-CSG-only (k=0); a future smooth edit then trips the W4 test.
