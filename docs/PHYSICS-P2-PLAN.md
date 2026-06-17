# Physics P2 — In-house Soft-Step 3D rigid contact RESOLVE + CPU SDF-collision queries

Status: **design APPROVED** (architect → architecture-critic → revision delta; all 3 CRITICAL + 5 MAJOR
blockers closed). Implementation is wave-by-wave (W1..W5), each `dev → code-review → tester(Miri+proptest)
→ commit`. CPU only, NO GPU. Builds on the P1 3D foundation (commit `6e8318d`).

This is the authoritative spec for the implementation waves; it is the architect's plan as amended by the
critic-driven revision delta. Bind to the LIVE code (line refs are indicative).

---

## Goal

A real, deterministic, fully in-house 3D rigid-body contact solver (`SoftStepSolver`) behind the existing
`RigidSolver` seam (unchanged signature), plus the contact generation it needs (sphere-box, box-box) and CPU
SDF-collision queries against the shared analytic edit-list field. Replaces the integrate-only `NoopSolver`
behavior with stable stacking, correct restitution, real 3D Coulomb friction (2-DOF cone), and SDF-surface
resting — single-threaded over the deterministic manifold order, ZERO new `unsafe`, bit-reproducible.

## Algorithm: TGS-Soft (Temporal Gauss-Seidel Soft), NOT XPBD

Velocity-level sequential-impulse with soft-constraint bias, warm-starting, relaxation, and a post-loop
restitution pass. Chosen over XPBD because it composes with P1's first-order `Quat::integrate` (XPBD would
replace the integrate stage and has weak restitution + worse stacking under equal substep budget). The solver
owns the substep loop (the schedule runs the pipeline once/frame; the solver loops internally over the same
contact set, re-projecting anchors each substep — it does NOT re-run narrowphase per substep).

`h = cfg.dt / cfg.substeps`.

### Per-substep loop (×`cfg.substeps`)
1. **Gravity integrate** (DYNAMIC bodies only): `v += g·h`.
2. **Warm-start apply**: seed each contact's accumulated `normal_impulse·n + tangent_impulse·(t1,t2)` to both bodies (`v ±= invMass·P`, `ω ±= I⁻¹·(r×P)`).
3. **Soft normal solve** per point: soft coefficients from `contact_hertz`,`contact_damping`,`h`:
   `omega=2π·hertz; a1=2·zeta+omega·h; a2=h·omega·a1; a3=1/(1+a2); biasRate=omega/a1; massCoeff=a2·a3; impulseCoeff=a3`.
   `vn = (vB+ωB×rB − vA−ωA×rA)·n`; `bias = max(biasRate·separation, -maxBiasVel)`;
   `dλ = -massCoeff·mEff·(vn+bias) − impulseCoeff·λ; λ_new = max(λ+dλ,0); apply (λ_new−λ)·n`.
4. **Friction (2-DOF coupled cone)**: see W2.
5. **Position integrate** (DYNAMIC only): `pos += v·h`; `rot = rot.integrate(ω, h)`; re-rotate `I⁻¹_world = R·I⁻¹_local·Rᵀ` for next substep's effective mass.
6. **Relax** (`cfg.relax_iterations` passes): re-solve normal+friction with `bias=0`, no restitution (removes soft-bias energy).

### Post-loop restitution (W5): ONCE, velocity-only, bias-free, vs the relative normal approach velocity captured BEFORE substep 0. `v_target = -e·min(vn_initial,0)`; apply `Δλ=(v_target−vn_current)/k_n`, total `λn≥0`. NO position write. Skip zero-normal contacts. Only restitutes APPROACHING contacts — and "approaching" means above a restitution velocity threshold `RESTITUTION_THRESHOLD` (Box2D-v3 `b2_velocityThreshold`, 1.0 m/s for meter-scale): `vn_initial < -RESTITUTION_THRESHOLD`. A body in SUSTAINED contact under gravity carries a small residual closing velocity in the gather snapshot every frame; without the threshold `v_target>0` would inject energy and a resting `restitution>0` stack would creep/jitter upward (C1). Resting/slow contacts therefore get `e=0` effectively. **W3/W4 forward risk:** the single bias-free sweep is EXACT for single-point manifolds (W2 sphere-sphere), but a 4-point box manifold (W4) couples points through the shared body and needs an iteration loop — revisit at W4.

### Effective mass (the genuinely-new 3D angular term)
`rnA=rA×n; rnB=rB×n; k = invMassA+invMassB + n·((I⁻¹_world_A·rnA)×rA) + n·((I⁻¹_world_B·rnB)×rB); mEff=(k>0)?1/k:0`.

## Determinism (load-bearing — `solver_is_deterministic` / IM-2)
Fixed manifold order (D4 `(min,max)` pair order), fixed point order `0..count`, normal-before-friction, fixed
`substeps`/`relax_iterations`, fixed float op order (no reduction reorder), single-thread (no atomics, no
rayon), warm-start probe = pure function of key. No `fast-math`/`float_algebraic` on this crate.

---

## CRITICAL resolutions (committed)

### C1 — SDF contact uses a sentinel `body_b`, never an anchor row
`const SDF_SENTINEL: BodyIndex = BodyIndex(u32::MAX)` (in `manifold.rs`). An SDF contact is a `Manifold` with
`body_b = SDF_SENTINEL`; the solve treats it as `IMMOVABLE_AT_REST` (inv_mass=0, inv_inertia=ZERO, vel=0) via
the existing `inv_mass==0` one-sided path — NO row appended, `bodies.len()` unchanged, the `physics_apply`
desync `debug_assert!` (`systems.rs:268-273`) stays verbatim. The sentinel is `u32::MAX` (not a row index) so
`touched` is never set out of range. SDF narrowphase emits `normal` pointing from the SDF surface toward body A.

### C2 — integration-ownership contract (pinned; reproduce as a `systems.rs` module doc block + a comment at the gate)
When `solver.owns_integration()`: (1) `physics_integrate` is gated OFF, so broad/narrowphase consume the
**pre-integration (end-of-previous-frame) snapshot** — correct & intentional for TGS (supersedes the
foundation docstrings for the owning-solver mode). (2) The solver integrates **DYNAMIC bodies only**
(`body_type==Dynamic`/`inv_mass!=0`) inside its substep loop — mandatory: it applies a per-substep gravity
bias, so without the gate a static floor drifts. (3) A comment at the gate site warns against un-gating
(double-integration). Gate via an `IntegrationMode` resource (`Owned`/`Foundation`) the plugin inserts from
`S::default().owns_integration()` (keeps `physics_integrate` monomorphic). `seam_swap_noop_vs_softstep` asserts
end-state (not snapshot) so it holds despite the two paths seeing different pre-gather state; add a
"static body bit-identical across a TGS step" assertion (guards gate (2)).

### C3 — warm-start table rebuilt each frame (no stamp tombstoning)
Flat open-addressed array keyed by `pack(body_a, body_b, feature_id)` (fallback `pack(a,b,0)` = dense-row,
bit-deterministic under fixed spawn order). Each frame: read from last frame's table; build a FRESH zeroed
write table (cap `next_pow2(2·points)`, load≤0.5); insert each live contact once in manifold order; swap
read↔write at frame end. Probe = `h=key·GOLDEN_64>>shift; h=(h+1)&mask` (pure function of key). ONE sentinel
`EMPTY=u64::MAX` (a real key can't produce it). No tombstones → no insertion-history dependence. Feature-id
stability (W3) is a declared precondition (a flicker = a warm-start MISS = 1-frame convergence cost, never a
determinism break).

---

## MAJOR resolutions (committed)

### W1 — real shape-derived world inverse inertia, derived at GATHER
`physics_gather` query widens to `(&RigidBody, &RigidBodyMass, &Collider)`; `BodyState::from_columns(body,
mass, collider)` derives the orientation-free **local** inverse inertia into `BodyState.inv_inertia_local`:
- Sphere r: `I=(2/5)·m·r²` → `inv_inertia_local = from_diagonal((inv_mass·5/(2r²)) × 3)`.
- Box half-extents (hx,hy,hz), full (w,h,d)=2·(hx,hy,hz): `Ixx=(1/12)m(h²+d²)`, `Iyy=(1/12)m(w²+d²)`, `Izz=(1/12)m(w²+h²)` → `from_diagonal(1/Ixx,1/Iyy,1/Izz)`.
- Static / inv_mass==0: `Mat3::ZERO`.
`inv_inertia` (world) = `R₀·I⁻¹_local·R₀ᵀ` at gather, refreshed `R·I⁻¹_local·Rᵀ` per substep. `RigidBodyMass.inv_inertia` is retained for custom authoring but auto-overridden at gather (kills the `Mat3::IDENTITY` unit-tensor placeholder bug).
NET-NEW `math.rs` ops (all pure `core` f32): `Mat3::from_quat(Quat)`, `impl Mul<Mat3> for Mat3`, `Mat3::transpose`, `Mat3::from_diagonal(Vec3)`.

### W2 — coupled friction cone + degeneracy-safe tangent basis
Coupled clamp on the ACCUMULATED (warm-started) tangent impulse vs current accumulated `λn`:
`λt_new=λt_old+dλt; if |λt_new|>μ·λn { λt_new *= μ·λn/|λt_new| } apply (λt_new−λt_old)`. For 2 tangents clamp the
2D magnitude `|(λt1,λt2)|` (a cone, not a box). Tangent basis (committed branch — `cross(n,ẑ)` is ZERO at
n≈±ẑ, the vertical floor normal): `t1 = if n.z.abs()<0.999 { cross(n,(0,0,1)) } else { cross(n,(1,0,0)) }.normalize(); t2 = cross(n,t1)` (t2 already unit).

### W3 — box-box feature-id stability (the stacking failure mode)
Feature-id from clipped-feature IDENTITY (incident-vertex idx + reference-feature id), not the SAT axis index;
+ reference-face HYSTERESIS (keep last frame's axis if current best penetration is within 1.05×). Ids for all
3 classes, disjoint via high-bit tags: face-face `pack(faceIdx(0..5), incidentVtx(0..7))`; edge-edge
`pack(0x8000|edgeA<<4|edgeB, 0)`; vertex-face `pack(faceIdx, 0x8000|vtx)`. Deterministic ≤4-point reduction:
deepest + 3 max-area, ties broken by lowest incident-vertex index.

### W4 — `boyko_sdf_math` `no_std` leaf crate (VERBATIM cut from `compute.rs`)
MOVES to leaf: `SdfEdit`+ctors + ALL std430/repr(C) const-asserts (`compute.rs:432-533`), `sdf_kind`/`sdf_op`,
`MAX_SDF_EDITS`/`SDF_EDIT_WORDS`/`HEADER_BASE_WORDS`, `SDF_GRAD_H`, `SDF_IMG_W/H`, and the field math
(`v_*`, `sd_sphere`, `sd_box`, `edit_distance`, `smin`, `smax`, `combine`, `sdf_edit_list`,
`sdf_edit_list_normal`). STAYS in `compute.rs` (re-imports leaf): `encode_edit_list`, `PIXEL_BASE_WORDS`/
composite offsets, `golden_*_pixel`, spirv getters. VERBATIM cut (no float-op reorder — a 1-ULP shift could
push a golden past ±2/255). `boyko_physics` depends on `boyko_sdf_math` (NOT `boyko_rhi_vulkan`; acyclic,
single source of truth, zero readback). `no_std` confirmed (only `core` f32 + `[f32;N]`, no `Vec`/`std`).
Extraction acceptance gate: rung 8/9/10/11 `boyko_rhi_vulkan` SDF goldens re-run bit-exact/±2-255 on RTX 3060
`--test-threads=1`. SDF narrowphase skips zero-normal contacts (`|∇|<SDF_NORMAL_EPS`, the CSG-seam case, O3).
Rename `ColliderShape::Aabb` → `Box` (OBB; the only readers are `systems.rs:~297` + tests).

### W5 — restitution placement (see Algorithm above) + integrate the C1/C2/C3 machinery.

---

## Open-question verdicts (folded)
- **OQ-1 (dt):** no new system. `physics_gather` stamps `PhysicsConfig.dt` from `FixedTime::delta_secs()` (gains `ResMut<PhysicsConfig>`+`Res<FixedTime>`). `solve()` signature UNCHANGED; solver reads `h=cfg.dt/cfg.substeps`.
- **OQ-2 (accept):** 1-frame warm-start cold-start after structural change is deterministic.
- **OQ-3 (accept):** rename `Aabb`→`Box` in W4.
- **OQ-5 (accept):** `PhysicsConfig::default().substeps` 1→4 (Noop ignores it; breaks no P1 test).

---

## Consolidated data-structure deltas
- `BodyState` (+`inv_inertia_local: Mat3`, filled at gather; `from_columns(body,mass,collider)`).
- `PhysicsConfig` (+`dt: f32` stamped by gather; `substeps` default 1→4; +`relax_iterations` (default 2), +`contact_hertz` (30.0), +`contact_damping`/zeta (10.0)).
- `SolverScratch` (+`warm_read`/`warm_write: WarmStartTable`, +`vn_initial: Vec<f32>` — all capacity-reused, no per-step alloc).
- `SoftStepSolver { cache_capacity… }` ZST-ish Resource (warm tables live in `SolverScratch`, reached via `&mut SolverScratch` in `solve`); `owns_integration()=true`, `is_noop()=false`. `NoopSolver` retained (default for the seam test).
- `IntegrationMode` resource (`Owned`/`Foundation`).
- `SdfField` resource (CPU-authoritative `SdfEdit` list; §3.4 Option A).
- `manifold.rs`: `SDF_SENTINEL`, feature-id pack helpers.
- `math.rs`: `Mat3::{from_quat, transpose, from_diagonal}` + `Mul<Mat3>`.

## Wave-by-wave plan (each independently testable; ZERO new unsafe; single-threaded solve)
| Wave | Files | Gates |
|---|---|---|
| **W1** | `math.rs` (+4 Mat3 ops), `resources.rs` (`BodyState.inv_inertia_local`, `PhysicsConfig.dt`/substeps, `from_columns` sig), `systems.rs` (gather widens to `&Collider`, stamps dt) | unit: sphere/box local-tensor values; `R·I⁻¹·Rᵀ` symmetry; `from_quat(IDENTITY)==IDENTITY`; transpose involution; `from_diagonal`. |
| **W2** | `solver/soft_step.rs` (normal+coupled-friction substep core), `solver/contact.rs` (effective mass, tangent basis), `solver.rs` (`owns_integration` default method) | `softstep_resolves_penetration`, `seam_swap_noop_vs_softstep`, `solver_is_deterministic` (sphere), `sphere_friction_on_static`. (`box_box_friction_3d` is W4+.) |
| **W3** | `solver/warm_start.rs` (rebuild-each-frame flat table), gather `stable_ids`, key build/apply/store | `stacking_is_stable` (sphere stack), warm-start convergence (fewer substeps to rest), determinism re-check. |
| **W4** | NEW `boyko_sdf_math` (verbatim cut), `compute.rs` re-import, `narrowphase/{box_box,sphere_box}.rs`, `components.rs` (`Aabb`→`Box`), shape dispatch | rung 8/9/10/11 SDF goldens bit-exact on RTX 3060; `stacking_is_stable` (box stack); `box_box_friction_3d` (box on incline). |
| **W5** | `solver/soft_step.rs` (C1 sentinel path, C2 IntegrationMode gate+contract, C3 table rebuild, post-loop restitution), `sdf_query.rs`, `SdfField`, `physics_narrowphase_sdf`, plugin wiring | `box_sdf_resting`, `sphere_vs_sdf_box_manifold`, `cpu_gpu_sdf_agreement` (3-way conformance), static-body bit-identical under TGS step, final `solver_is_deterministic`, Miri-green. |

## Reserved (NOT built in P2, per plan §3.4)
Collision-mesh-from-SDF hand-off (GPU-authoritative brick-atlas scale); island-parallel solve; sleeping;
continuous collision; SIMD across contact points; capsule/convex-hull colliders; GJK/EPA.
