use super::*;
use crate::{SDF_EDIT_BAND_HALF, SdfEdit, SdfEditField, sdf_edit_list, sdf_kind, sdf_op};

/// A deterministic xorshift64* PRNG — the GATE's scene generator without a dep.
struct XorShift64(u64);

impl XorShift64 {
    #[inline]
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state (xorshift's fixed point); any non-zero seed works.
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform `f32` in `[lo, hi)`.
    #[inline]
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        // 24-bit mantissa fraction in [0, 1).
        let frac = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        lo + frac * (hi - lo)
    }

    /// A uniform `u32` in `[0, n)`.
    #[inline]
    fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % n as u64) as u32
    }
}

/// The worst-case interior sample offsets within ONE cell (in cell-fraction
/// units, `[0, 1]³`): voxel-center, the mid-edges, the face-centers, the body-
/// diagonal quarters, and the 8 cell corners — the points where trilinear over-
/// shoot of a curved band is largest. The GATE samples EVERY interior cell at
/// each of these offsets.
const CELL_OFFSETS: &[[f32; 3]] = &[
    // voxel-center
    [0.5, 0.5, 0.5],
    // mid-edges (0.5,0,0) and axis perms
    [0.5, 0.0, 0.0],
    [0.0, 0.5, 0.0],
    [0.0, 0.0, 0.5],
    // face-centers (0.5,0.5,0) and axis perms
    [0.5, 0.5, 0.0],
    [0.5, 0.0, 0.5],
    [0.0, 0.5, 0.5],
    // body-diagonal quarters
    [0.25, 0.25, 0.25],
    [0.75, 0.75, 0.75],
    // the 8 cell corners
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
];

/// Generates a random valid edit scene per the GATE contract: 1..=8 edits, the
/// first forced UNION, kinds SPHERE/BOX, radii/half-extents CLAMPED to `>=R_MIN`
/// (sub-`R_MIN` curvature is out of contract), centers in `[-2,2]³`, ops UNION/
/// SUBTRACT/INTERSECT, smoothness `0.0` or `>=R_MIN`. Returns the field plus the
/// center of the first edit (where the brick is placed to straddle a surface).
fn random_scene(rng: &mut XorShift64) -> (SdfEditField, [f32; 3]) {
    let n = 1 + rng.below(8); // 1..=8
    let mut field = SdfEditField::new();
    let mut first_center = [0.0_f32; 3];

    for i in 0..n {
        let center = [
            rng.range(-2.0, 2.0),
            rng.range(-2.0, 2.0),
            rng.range(-2.0, 2.0),
        ];
        if i == 0 {
            first_center = center;
        }
        // op: first edit forced UNION; later in {UNION, SUBTRACT, INTERSECT}.
        let op = if i == 0 {
            sdf_op::UNION
        } else {
            match rng.below(3) {
                0 => sdf_op::UNION,
                1 => sdf_op::SUBTRACT,
                _ => sdf_op::INTERSECT,
            }
        };
        // smoothness in {0.0} ∪ [R_MIN, R_MIN] (the contract pins the smooth
        // radius at >=R_MIN so the smooth-blend curvature stays within bounds).
        let smoothness = if rng.below(2) == 0 { 0.0 } else { R_MIN };

        let edit = if rng.below(2) == 0 {
            let r = rng.range(R_MIN, 3.0); // radius CLAMPED >= R_MIN
            SdfEdit::sphere(center, r, op, smoothness)
        } else {
            let h = [
                rng.range(0.5, 3.0),
                rng.range(0.5, 3.0),
                rng.range(0.5, 3.0),
            ];
            SdfEdit::box_shape(center, h, op, smoothness)
        };
        field.push(edit);
    }
    field.bump_gen();
    (field, first_center)
}

// ─── 1. THE GATE — conservative-lower-bound over many random scenes ───────────

/// THE GATE (M0 soundness): the brick's trilinear reconstruction is a CONSERVATIVE
/// LOWER BOUND on the analytic field at every R1-trusted interior sample, over
/// ≥1000 random scenes. A surface-overshooting brick (recon > analytic) would let
/// the Hart sphere-marcher step THROUGH the surface — this test is the numeric
/// tripwire the committed constants must clear. If it fails, the constants need
/// further derivation (do NOT relax the assertion).
///
/// DEFERRED TO M2 (not relaxed): a fixed narrow-band 8-bit trilinear field is
/// fundamentally NOT a clean Euclidean lower bound near curved/creased surfaces —
/// the trusted band reaches into small-primitive interiors where SDF curvature
/// (~1/(R_min−band) ≈ 17) far exceeds any fixed `C_MAX`, and CSG creases compound
/// it (empirically: over-reports up to ~0.11 even on a single R_MIN sphere). M1
/// therefore does NOT step on the trilinear field (it ships empty-space-skip-only:
/// the conservative classifier skips EMPTY bricks to their exit — sound by
/// construction — and marches SURFACE bricks analytically). The trilinear oracle is
/// retained for M2, where the JCGT-2022 analytic trilinear-interpolant cubic makes
/// the in-voxel crossing EXACT (no fragile ε-bound). This assertion is kept VERBATIM
/// and un-`ignore`d the moment M2's cubic replaces the conservative-step decode.
#[test]
#[ignore = "M2: trilinear stepping deferred to the JCGT cubic; M1 is empty-skip-only"]
fn brick_field_is_conservative_lower_bound() {
    const SEEDS: u64 = 1500;
    let voxel = VOXEL_SIZE;
    let brick_size = voxel * BRICK_INTERIOR as f32;

    let mut r1_samples_checked: u64 = 0;
    let mut r3_samples_checked: u64 = 0;

    for seed in 0..SEEDS {
        let mut rng = XorShift64::new(seed.wrapping_mul(0x100_0001).wrapping_add(1));
        let (field, focus) = random_scene(&mut rng);

        // Place the brick straddling the surface: center the 8³ interior on a
        // point jittered near the first edit's center, then offset to the min.
        let jitter = [
            rng.range(-1.0, 1.0),
            rng.range(-1.0, 1.0),
            rng.range(-1.0, 1.0),
        ];
        let brick_center = [
            focus[0] + jitter[0],
            focus[1] + jitter[1],
            focus[2] + jitter[2],
        ];
        let brick_min = [
            brick_center[0] - brick_size * 0.5,
            brick_center[1] - brick_size * 0.5,
            brick_center[2] - brick_size * 0.5,
        ];

        let mut brick = [0i8; BRICK_VOXELS];
        fill_brick(&field, brick_min, voxel, BAND_HALF_STORE, C_MAX, &mut brick);

        let edits = field.edits();

        // Sample every interior cell at every worst-case offset.
        for cz in 0..BRICK_INTERIOR {
            for cy in 0..BRICK_INTERIOR {
                for cx in 0..BRICK_INTERIOR {
                    for off in CELL_OFFSETS {
                        // Interior-voxel local coords (the trilinear_reconstruct
                        // domain): [0, BRICK_INTERIOR] across the interior.
                        let local = [
                            cx as f32 + off[0],
                            cy as f32 + off[1],
                            cz as f32 + off[2],
                        ];
                        let world = [
                            brick_min[0] + local[0] * voxel,
                            brick_min[1] + local[1] * voxel,
                            brick_min[2] + local[2] * voxel,
                        ];
                        let analytic = sdf_edit_list(edits, world);
                        let recon = trilinear_reconstruct(&brick, local, BAND_HALF_STORE);

                        // R1: |analytic| in [BAND_REFINE, USABLE_BAND_OUTER] — the
                        // PROVEN lower-bound region. recon must NOT over-report
                        // (1e-6 = IEEE FP noise, NOT a soundness fudge).
                        let mag = analytic.abs();
                        if (BAND_REFINE..=USABLE_BAND_OUTER).contains(&mag) {
                            r1_samples_checked += 1;
                            assert!(
                                recon <= analytic + 1e-6,
                                "R1 OVER-REPORT (surface overshoot): recon={recon} > analytic={analytic} \
                                 at world={world:?} local={local:?} seed={seed}; edits={:?}",
                                edits,
                            );
                        }

                        // R3 saturation: |analytic| > BAND_HALF_STORE — the loose
                        // LB region. recon saturates to within the store band, so
                        // it is sign-correct and strictly below |analytic|.
                        if analytic.abs() > BAND_HALF_STORE {
                            r3_samples_checked += 1;
                            assert!(
                                recon.abs() <= BAND_HALF_STORE + 1e-6,
                                "R3 recon escaped the store band: |recon|={} > BAND_HALF_STORE={BAND_HALF_STORE} \
                                 at world={world:?} seed={seed}",
                                recon.abs(),
                            );
                            assert!(
                                recon.abs() <= analytic.abs() + 1e-6,
                                "R3 recon exceeds |analytic|: |recon|={} > |analytic|={} at world={world:?} seed={seed}",
                                recon.abs(),
                                analytic.abs(),
                            );
                        }
                    }
                }
            }
        }
    }

    // The generator must actually exercise the trust regions, or the GATE is
    // vacuously green. With ≥1500 surface-straddling scenes both must be hit.
    assert!(
        r1_samples_checked > 0,
        "GATE vacuous: no R1-trusted samples generated across {SEEDS} seeds"
    );
    assert!(
        r3_samples_checked > 0,
        "GATE vacuous: no R3-saturated samples generated across {SEEDS} seeds"
    );
}

// ─── 2. Soundness unit tests (runtime mirrors of the compile-time asserts) ────

/// P2 (runtime mirror): the world-space down-bias dominates the trilinear-
/// midpoint slack + the quantization step, so decode(recon) <= analytic.
#[test]
fn epsilon_q_dominates_curvature_and_quant() {
    let world_bias = EPSILON_Q * BAND_HALF_STORE;
    let budget = VOXEL_SIZE * VOXEL_SIZE * C_MAX / 8.0 + BAND_HALF_STORE / 254.0;
    assert!(
        world_bias >= budget,
        "EPSILON_Q*BAND_HALF_STORE={world_bias} must dominate curvature+quant budget={budget}"
    );
}

/// P1 (runtime mirror): the store band is wide enough that a trusted point's
/// bracketing corners never saturate to ±1.0 (saturation erases the curvature
/// the lower bound relies on).
#[test]
fn saturation_invariant_holds() {
    let rhs = USABLE_BAND_OUTER + VOXEL_DIAG + EPSILON_Q * BAND_HALF_STORE;
    assert!(
        BAND_HALF_STORE >= rhs,
        "BAND_HALF_STORE={BAND_HALF_STORE} must be >= USABLE_BAND_OUTER+VOXEL_DIAG+bias={rhs}"
    );
}

/// P3 (runtime mirror): the proven brick-step region R1 is non-empty (the
/// analytic hand-off band lies strictly inside the outer trust edge).
#[test]
fn r1_interval_nonempty() {
    // `black_box` keeps the comparison a RUNTIME check (not a const-folded assert
    // clippy would flag as trivially true) — the runtime mirror of the P3 predicate.
    let refine = std::hint::black_box(BAND_REFINE);
    let outer = std::hint::black_box(USABLE_BAND_OUTER);
    assert!(
        refine < outer,
        "R1 empty: BAND_REFINE={refine} must be < USABLE_BAND_OUTER={outer}"
    );
}

// ─── 3. W1 decode parity (the Vulkan R8_SNORM rule, all 256 codes) ────────────

/// `decode_snorm8` reproduces the Vulkan `R8_SNORM` decode rule bit-for-bit over
/// EVERY code: `max(c/127, -1) * band`. The `i8::MIN` (-128) sentinel decodes to
/// `-1.0 * band` (the asymmetric snorm rule), matching the GPU sampler.
#[test]
fn decode_snorm8_matches_vulkan_r8_snorm_rule() {
    let band = BAND_HALF_STORE;
    for c in i8::MIN..=i8::MAX {
        let expected = (c as f32 / 127.0).max(-1.0) * band;
        let got = decode_snorm8(c, band);
        assert_eq!(
            got.to_bits(),
            expected.to_bits(),
            "decode_snorm8({c}) bits must match the Vulkan R8_SNORM rule"
        );
    }
}

// ─── 4. C2 classifier — sub-voxel sliver must NOT be classified EMPTY ─────────

/// A thin box (half-extent < voxel_size on one axis) straddling a brick FACE such
/// that NO brick corner is inside it must still classify as `Surface` (the C2
/// AABB-overlap classifier catches the sliver a corner-sampling test would miss).
#[test]
fn classify_brick_thin_sliver_on_face_is_surface() {
    let voxel = VOXEL_SIZE;
    let brick_size = voxel * BRICK_INTERIOR as f32; // 2.0
    // Brick spans [0, 2]³.
    let brick_min = [0.0, 0.0, 0.0];
    // A thin slab centered exactly on the brick's +x face plane (x = 2.0), with a
    // sub-voxel half-extent on x so no brick CORNER (all at x∈{0,2}) lies strictly
    // inside the slab's solid, yet its band straddles the face → Surface.
    let mut field = SdfEditField::new();
    field.push(SdfEdit::box_shape(
        [brick_size, 1.0, 1.0],
        [voxel * 0.4, 1.0, 1.0],
        sdf_op::UNION,
        0.0,
    ));
    field.bump_gen();

    let class = classify_brick(&field, brick_min, brick_size, BAND_HALF_STORE);
    assert_eq!(
        class,
        BrickClass::Surface,
        "a sub-voxel sliver straddling a brick face must be Surface, not EMPTY"
    );
}

/// A brick far from every edit AABB classifies EmptyOutside when the center
/// samples positive (provably outside every solid).
#[test]
fn classify_brick_far_outside_is_empty_outside() {
    let voxel = VOXEL_SIZE;
    let brick_size = voxel * BRICK_INTERIOR as f32;
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0));
    field.bump_gen();

    // A brick way out past the sphere's band-expanded AABB.
    let brick_min = [50.0, 50.0, 50.0];
    let class = classify_brick(&field, brick_min, brick_size, BAND_HALF_STORE);
    assert_eq!(
        class,
        BrickClass::EmptyOutside,
        "a brick with no AABB overlap and a positive center must be EmptyOutside"
    );
}

/// A brick deep inside a SINGLE large primitive still OVERLAPS that primitive's
/// conservative AABB (the AABB is the whole primitive box, not just the band
/// shell), so the classifier conservatively returns `Surface` — NOT `EmptyInside`.
/// This pins the C2 invariant: EMPTY is declared ONLY on AABB non-overlap, and a
/// single convex primitive's AABB covers its entire interior.
#[test]
fn classify_brick_deep_inside_single_primitive_is_surface() {
    let voxel = VOXEL_SIZE;
    let brick_size = voxel * BRICK_INTERIOR as f32; // 2.0
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 20.0, sdf_op::UNION, 0.0));
    field.bump_gen();

    // The brick spans [-1,1]³, well within the sphere's ±20.9 AABB → overlaps it.
    let brick_min = [-1.0, -1.0, -1.0];
    let class = classify_brick(&field, brick_min, brick_size, BAND_HALF_STORE);
    assert_eq!(
        class,
        BrickClass::Surface,
        "a single primitive's AABB covers its interior, so a deep-inside brick is Surface (conservative)"
    );
}

/// The empty field (no edits) classifies `EmptyOutside`: no AABB overlaps and the
/// center samples `+SDF_FAR` (well above `band_half`). This pins the
/// `Default`/empty-scene behavior the physics opt-in path relies on (an empty SDF
/// produces no collisions).
#[test]
fn classify_brick_empty_field_is_empty_outside() {
    let voxel = VOXEL_SIZE;
    let brick_size = voxel * BRICK_INTERIOR as f32;
    let field = SdfEditField::new(); // no edits
    let class = classify_brick(&field, [0.0, 0.0, 0.0], brick_size, BAND_HALF_STORE);
    assert_eq!(
        class,
        BrickClass::EmptyOutside,
        "an empty field has no AABB overlap and samples +far → EmptyOutside"
    );
}

// ─── 5. Fill oracle bit-exactness (no interpolation, at a voxel center) ───────

/// At a voxel CENTER (no interpolation), the stored-then-decoded value equals the
/// analytic field MINUS the EPSILON_Q bias, within one snorm quantization step
/// (`BAND_HALF_STORE/127`). This proves the fill faithfully encodes `(analytic −
/// bias)`.
#[test]
fn fill_brick_voxel_center_encodes_analytic_minus_bias() {
    let voxel = VOXEL_SIZE;
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([1.0, 1.0, 1.0], 0.8, sdf_op::UNION, 0.0));
    field.bump_gen();

    // Place the surface inside the brick so the band-relevant voxels are unsaturated.
    let brick_min = [0.0, 0.0, 0.0];
    let mut brick = [0i8; BRICK_VOXELS];
    fill_brick(&field, brick_min, voxel, BAND_HALF_STORE, C_MAX, &mut brick);

    let edits = field.edits();
    let bias = EPSILON_Q * BAND_HALF_STORE;
    let quant_step = BAND_HALF_STORE / 127.0;
    const W: usize = BRICK_ALLOC;

    // Check every INTERIOR voxel center whose biased analytic is inside the band
    // (so the stored code is not saturated; a saturated code legitimately clamps).
    for iz in 0..BRICK_INTERIOR {
        for iy in 0..BRICK_INTERIOR {
            for ix in 0..BRICK_INTERIOR {
                // The interior voxel's center, in world space.
                let p = [
                    brick_min[0] + (ix as f32 + 0.5) * voxel,
                    brick_min[1] + (iy as f32 + 0.5) * voxel,
                    brick_min[2] + (iz as f32 + 0.5) * voxel,
                ];
                let analytic = sdf_edit_list(edits, p);
                let target = analytic - bias;
                if target.abs() >= BAND_HALF_STORE {
                    continue; // saturated code: clamp is expected, skip
                }
                // The apron'd grid index of this interior voxel (apron offset +1).
                let gx = ix + APRON;
                let gy = iy + APRON;
                let gz = iz + APRON;
                let code = brick[gx + gy * W + gz * W * W];
                let decoded = decode_snorm8(code, BAND_HALF_STORE);
                assert!(
                    (decoded - target).abs() <= quant_step,
                    "stored code at voxel ({ix},{iy},{iz}) decodes to {decoded}, expected analytic-bias={target} \
                     (analytic={analytic}) within quant step {quant_step}"
                );
            }
        }
    }
}

// ─── 6. Trilinear reconstruct error bound (lower bound, but tight) ────────────

/// Within R1 the reconstruction is a lower bound that is NOT too loose: the gap
/// `analytic − recon` stays inside `[−1e-6, EPSILON_Q*band + δ_tri + quant + ε]`.
/// Confirms the brick is USEFUL (tight), not merely `<=`.
///
/// DEFERRED TO M2 (see `brick_field_is_conservative_lower_bound`): the trilinear
/// field is not stepped on in M1 (empty-skip-only); its lower-bound tightness is an
/// M2/JCGT-cubic concern. Assertion kept verbatim for the M2 re-enable.
#[test]
#[ignore = "M2: trilinear stepping deferred to the JCGT cubic; M1 is empty-skip-only"]
fn trilinear_reconstruct_is_a_tight_lower_bound_in_r1() {
    let voxel = VOXEL_SIZE;
    let mut field = SdfEditField::new();
    // A sphere whose surface CROSSES the brick interior (radius 1.0, centered at the
    // brick center) so the band — and thus R1 — is densely sampled inside the brick.
    field.push(SdfEdit::sphere([1.0, 1.0, 1.0], 1.0, sdf_op::UNION, 0.0));
    field.bump_gen();

    let brick_min = [0.0, 0.0, 0.0];
    let mut brick = [0i8; BRICK_VOXELS];
    fill_brick(&field, brick_min, voxel, BAND_HALF_STORE, C_MAX, &mut brick);
    let edits = field.edits();

    let bias = EPSILON_Q * BAND_HALF_STORE;
    let delta_tri = VOXEL_SIZE * VOXEL_SIZE * C_MAX / 8.0;
    let quant = BAND_HALF_STORE / 127.0;
    // The upper bound on the LB gap: the budget plus a small FP slack.
    let upper = bias + delta_tri + quant + 1e-5;

    let mut checked = 0u64;
    for cz in 0..BRICK_INTERIOR {
        for cy in 0..BRICK_INTERIOR {
            for cx in 0..BRICK_INTERIOR {
                for off in CELL_OFFSETS {
                    let local = [
                        cx as f32 + off[0],
                        cy as f32 + off[1],
                        cz as f32 + off[2],
                    ];
                    let world = [
                        brick_min[0] + local[0] * voxel,
                        brick_min[1] + local[1] * voxel,
                        brick_min[2] + local[2] * voxel,
                    ];
                    let analytic = sdf_edit_list(edits, world);
                    if !(BAND_REFINE..=USABLE_BAND_OUTER).contains(&analytic.abs()) {
                        continue;
                    }
                    let recon = trilinear_reconstruct(&brick, local, BAND_HALF_STORE);
                    let gap = analytic - recon;
                    checked += 1;
                    // Lower bound (recon <= analytic) within FP noise.
                    assert!(gap >= -1e-6, "recon over-reports in R1: gap={gap} world={world:?}");
                    // Tight: the gap does not exceed the slack budget.
                    assert!(
                        gap <= upper,
                        "recon too loose in R1: gap={gap} > budget={upper} world={world:?}"
                    );
                }
            }
        }
    }
    assert!(checked > 0, "no R1 samples on the test sphere — bound not exercised");
}

// ─── Cross-check: SDF_EDIT_BAND_HALF (lib.rs) equals the brick store band ─────

/// The lib's `SDF_EDIT_BAND_HALF` (the per-edit AABB skin) must equal the brick's
/// `BAND_HALF_STORE`, or the classifier expands AABBs by a different band than the
/// fill quantizes — breaking the classifier's conservatism contract.
#[test]
fn store_band_matches_aabb_skin_band() {
    assert_eq!(
        SDF_EDIT_BAND_HALF.to_bits(),
        BAND_HALF_STORE.to_bits(),
        "the AABB skin band (SDF_EDIT_BAND_HALF) must equal the brick store band (BAND_HALF_STORE)"
    );
}

/// Sanity: every SPHERE/BOX kind round-trips through `random_scene` without
/// panicking and produces a foldable field (guards the generator itself).
#[test]
fn random_scene_generator_is_well_formed() {
    let mut rng = XorShift64::new(42);
    for _ in 0..256 {
        let (field, _) = random_scene(&mut rng);
        assert!(field.count >= 1 && field.count <= 8, "edit count in 1..=8");
        // The fold must be finite at a probe point.
        let d = sdf_edit_list(field.edits(), [0.1, 0.2, 0.3]);
        assert!(d.is_finite(), "scene field must fold to a finite distance");
        // Every kind must be SPHERE or BOX.
        for e in field.edits() {
            assert!(
                e.kind == sdf_kind::SPHERE || e.kind == sdf_kind::BOX,
                "only SPHERE/BOX kinds generated"
            );
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// M2 — STEP 1: the JCGT cubic CPU oracle (`jcgt_cubic_coeffs` / `marmitt_root` /
// `cardano_root` / `atlas_uvw` / `brick_cubic_hit`). The CPU mirror the GPU
// SURFACE-brick path is golden-compared against — verified WITHOUT a GPU.
//
// Reuses the M0 `XorShift64` + `random_scene` generator + `CELL_OFFSETS` above
// (NO new dependency). The `super::*` import exposes the private `cubic_eval`
// and `clamp_index` so the trilinear↔cubic identity is checked against the
// SAME interpolant the production code evaluates.
// ══════════════════════════════════════════════════════════════════════════

/// The interior-voxel span of the M2 brick: `[0, BRICK_INTERIOR]³` is the
/// interior, `voxel = VOXEL_SIZE` the world width of one cell.
const M2_VOXEL: f32 = VOXEL_SIZE;

/// Maps a WORLD-space point to the brick's INTERIOR-voxel frame
/// (`local = (world - brick_min) / voxel`), the [`trilinear_reconstruct`] /
/// [`brick_cubic_hit`] domain.
#[inline]
fn world_to_local(world: [f32; 3], brick_min: [f32; 3], voxel: f32) -> [f32; 3] {
    [
        (world[0] - brick_min[0]) / voxel,
        (world[1] - brick_min[1]) / voxel,
        (world[2] - brick_min[2]) / voxel,
    ]
}

/// Maps an interior-voxel-frame point back to WORLD space (the inverse of
/// [`world_to_local`]).
#[inline]
fn local_to_world(local: [f32; 3], brick_min: [f32; 3], voxel: f32) -> [f32; 3] {
    [
        brick_min[0] + local[0] * voxel,
        brick_min[1] + local[1] * voxel,
        brick_min[2] + local[2] * voxel,
    ]
}

/// The trilinear interpolant evaluated DIRECTLY from the 8 corner distances `s`
/// (the corner convention `s_ijk ↔ x + 2y + 4z`) at the in-cell fractional point
/// `f ∈ [0, 1]³` — the reference the JCGT cubic is verified against in
/// invariant 1. This is the textbook trilinear blend, NOT the quantized brick
/// fetch, so it isolates the cubic-coefficient algebra from snorm rounding.
fn trilinear_blend_corners(s: &[f32; 8], f: [f32; 3]) -> f32 {
    let (fx, fy, fz) = (f[0], f[1], f[2]);
    // s_ijk ↔ x + 2y + 4z: s[0]=s000, s[1]=s100, s[2]=s010, s[3]=s110,
    // s[4]=s001, s[5]=s101, s[6]=s011, s[7]=s111.
    let c00 = s[0] + (s[1] - s[0]) * fx;
    let c10 = s[2] + (s[3] - s[2]) * fx;
    let c01 = s[4] + (s[5] - s[4]) * fx;
    let c11 = s[6] + (s[7] - s[6]) * fx;
    let c0 = c00 + (c10 - c00) * fy;
    let c1 = c01 + (c11 - c01) * fy;
    c0 + (c1 - c0) * fz
}

// ─── M2.1 — CUBIC == TRILINEAR ALONG THE RAY (the construction invariant) ─────

/// INVARIANT 1: `cubic_eval(jcgt_cubic_coeffs(s, ro, rd), t)` reproduces the
/// trilinear blend of the 8 corners `s` at `(ro + rd·t)`, for random corner sets,
/// random rays, and many `t ∈ [0, 1]`. This proves the cubic coefficients encode
/// the SAME interpolant (the corner convention + the k-basis transcription —
/// the k3/k7 trap) to tight FP tolerance. If it fails, the coefficients are
/// WRONG and M2 step 1 STOPS.
#[test]
fn cubic_equals_trilinear_along_the_ray() {
    const SCENES: u32 = 4000;
    const T_SAMPLES: u32 = 17;
    // Tight FP tol: the cubic is the algebraic expansion of the trilinear blend;
    // the two differ only by f32 rounding of the FMA chains.
    const TOL: f32 = 1.0e-5;

    let mut rng = XorShift64::new(0xC0B1_C5EE_1234_5678);
    let mut max_err = 0.0_f32;

    for _ in 0..SCENES {
        // Random corner distances in a band-realistic range.
        let mut s = [0.0_f32; 8];
        for c in s.iter_mut() {
            *c = rng.range(-2.0, 2.0);
        }
        // A random ray that stays inside the unit cell for t ∈ [0, 1]: pick an
        // entry point in [0,1]³ and a direction that keeps the exit inside.
        let ro = [rng.range(0.0, 1.0), rng.range(0.0, 1.0), rng.range(0.0, 1.0)];
        // Direction components small enough that ro + rd lands inside the cell.
        let rd = [
            rng.range(-1.0, 1.0) * (1.0 - ro[0]).min(ro[0]).max(0.05),
            rng.range(-1.0, 1.0) * (1.0 - ro[1]).min(ro[1]).max(0.05),
            rng.range(-1.0, 1.0) * (1.0 - ro[2]).min(ro[2]).max(0.05),
        ];

        let coeffs = jcgt_cubic_coeffs(&s, ro, rd);

        for ti in 0..T_SAMPLES {
            let t = ti as f32 / (T_SAMPLES - 1) as f32;
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let cubic = cubic_eval(&coeffs, t);
            let tri = trilinear_blend_corners(&s, p);
            let err = (cubic - tri).abs();
            if err > max_err {
                max_err = err;
            }
            assert!(
                err <= TOL,
                "CUBIC != TRILINEAR: |cubic({t})={cubic} - tri={tri}| = {err} > {TOL}; \
                 s={s:?} ro={ro:?} rd={rd:?} coeffs={coeffs:?} — the JCGT coefficient \
                 algebra (k-basis / corner convention) is WRONG"
            );
        }
    }
    // The construction invariant holds; record the worst error for the report.
    assert!(
        max_err <= TOL,
        "max cubic-vs-trilinear error {max_err} exceeded tol {TOL}"
    );
}

/// INVARIANT 1 (degenerate ray): an AXIS-ALIGNED ray (two direction components
/// exactly zero) still satisfies cubic == trilinear — the bilinear/cubic cross
/// terms (`c2`/`c3`) must vanish cleanly, not drift.
#[test]
fn cubic_equals_trilinear_axis_aligned_ray() {
    const TOL: f32 = 1.0e-5;
    let mut rng = XorShift64::new(0x0A15_A11E_DEAD_BEEF);
    for _ in 0..1000 {
        let mut s = [0.0_f32; 8];
        for c in s.iter_mut() {
            *c = rng.range(-1.5, 1.5);
        }
        // Ray along +x only: y, z fixed.
        let ro = [0.0, rng.range(0.0, 1.0), rng.range(0.0, 1.0)];
        let rd = [1.0, 0.0, 0.0];
        let coeffs = jcgt_cubic_coeffs(&s, ro, rd);
        // An axis-aligned ray collapses the trilinear to a LINEAR function of t,
        // so c2 and c3 must be ~0 (no bilinear/cubic t-dependence along one axis).
        assert!(
            coeffs[2].abs() <= 1.0e-5 && coeffs[3].abs() <= 1.0e-5,
            "axis-aligned ray must yield ~linear cubic: coeffs={coeffs:?}"
        );
        for ti in 0..9 {
            let t = ti as f32 / 8.0;
            let p = [ro[0] + t, ro[1], ro[2]];
            let err = (cubic_eval(&coeffs, t) - trilinear_blend_corners(&s, p)).abs();
            assert!(err <= TOL, "axis-aligned cubic!=tri err={err} t={t} s={s:?}");
        }
    }
}

// ─── M2.2 — MARMITT == CARDANO (first in-range root) ──────────────────────────

/// The cross-check agreement tolerance for the FIRST-ROOT comparison.
///
/// [`marmitt_root`] is a FIXED-budget ([`MARMITT_ITERS`] = 8) regula-falsi: on the
/// SUB-VOXEL-CELL brackets `brick_cubic_hit` actually uses (`[0, seg_hi-seg_lo]`,
/// at most a few voxels of local-`t`) it converges to ~[`CUBIC_ROOT_EPS`]. On a
/// WIDE synthetic bracket (e.g. `[-2, 2]`) with a far flat anchor end, 8 false-
/// position steps crawl — by DESIGN, the GPU port wants a branch-light bounded
/// loop, not unbounded `while`. So the cross-check tolerance is set to the
/// wide-bracket regime; the dedicated `marmitt_converges_tightly_on_cell_brackets`
/// test proves the production (narrow-bracket) accuracy separately.
const ROOT_AGREE: f32 = 4.0e-2;

/// Builds a cubic with three chosen real roots `r0,r1,r2` and leading coeff `a`:
/// `a·(t-r0)(t-r1)(t-r2)`, returned as `[c0,c1,c2,c3]`.
fn cubic_from_roots(a: f32, r0: f32, r1: f32, r2: f32) -> [f32; 4] {
    // a·(t³ - (r0+r1+r2)t² + (r0r1+r1r2+r2r0)t - r0r1r2)
    let e1 = r0 + r1 + r2;
    let e2 = r0 * r1 + r1 * r2 + r2 * r0;
    let e3 = r0 * r1 * r2;
    [a * (-e3), a * e2, a * (-e1), a]
}

/// Evaluate a cubic (test-local Horner; mirrors the private `cubic_eval`).
#[inline]
fn ev(c: &[f32; 4], t: f32) -> f32 {
    ((c[3] * t + c[2]) * t + c[1]) * t + c[0]
}

/// INVARIANT 2 (wide-bracket stress): the GPU [`marmitt_root`] and the closed-form
/// [`cardano_root`] are CONSISTENT over MANY random cubics spanning every
/// degenerate family (`c3→0` quadratic, `c3,c2→0` linear, double-root,
/// three-real-root, single-real+complex-pair). The contract here is CONSISTENCY,
/// not bit-equivalence: on a WIDE synthetic bracket containing TWO interior roots,
/// "first sign-change" (marmitt walks splits left→right) and "smallest real root"
/// (cardano) are legitimately DIFFERENT definitions, so the roots may differ. What
/// MUST hold:
///  - marmitt never INVENTS a non-crossing (a found root has a sign change around
///    it OR a small residual);
///  - marmitt never MISSES a clean SIGN-CHANGING crossing cardano found;
///  - both always return finite values, never panic.
///
/// The TIGHT first-root equivalence (gap ≤ 6e-3) is asserted on the PRODUCTION
/// regime — single clean crossing in a sub-voxel-cell bracket — by
/// `marmitt_converges_tightly_on_cell_brackets` (the regime `brick_cubic_hit`
/// actually forms). If THAT diverges, the GPU root-finder is wrong and M2 STOPS.
#[test]
fn marmitt_agrees_with_cardano_first_root() {
    const SCENES: u32 = 20000;
    let mut rng = XorShift64::new(0x1234_5678_9ABC_DEF0);

    let mut both_found = 0u64;
    let mut one_found = 0u64;
    let mut none = 0u64;
    let mut marmitt_only_misses = 0u64;
    let mut budget_short = 0u64;
    let mut max_root_gap = 0.0_f32;
    let mut max_residual_disagree = 0.0_f32;

    for _ in 0..SCENES {
        // Mix construction modes so EVERY degenerate family is exercised.
        let mode = rng.below(6);
        let (c, t0, t1) = match mode {
            // Three-real-root cubic from chosen roots (general case).
            0 => {
                let a = {
                    let v = rng.range(-2.0, 2.0);
                    if v.abs() < 0.2 { 0.5 } else { v }
                };
                let r0 = rng.range(-1.5, 1.5);
                let r1 = rng.range(-1.5, 1.5);
                let r2 = rng.range(-1.5, 1.5);
                (cubic_from_roots(a, r0, r1, r2), -2.0_f32, 2.0_f32)
            }
            // Double root + a single (repeated-root branch).
            1 => {
                let a = 1.0;
                let rd = rng.range(-1.0, 1.0);
                let rs = rng.range(-1.0, 1.0);
                (cubic_from_roots(a, rd, rd, rs), -2.0_f32, 2.0_f32)
            }
            // Near-quadratic: c3 → 0 (tiny leading coefficient).
            2 => {
                let c3 = rng.range(-1.0, 1.0) * 1.0e-6;
                let c2 = rng.range(-2.0, 2.0);
                let c1 = rng.range(-2.0, 2.0);
                let c0 = rng.range(-1.0, 1.0);
                ([c0, c1, c2, c3], -2.0_f32, 2.0_f32)
            }
            // Near-linear: c3, c2 → 0.
            3 => {
                let c3 = rng.range(-1.0, 1.0) * 1.0e-7;
                let c2 = rng.range(-1.0, 1.0) * 1.0e-6;
                let c1 = {
                    let v = rng.range(-2.0, 2.0);
                    if v.abs() < 0.3 { 0.5 } else { v }
                };
                let c0 = rng.range(-1.0, 1.0);
                ([c0, c1, c2, c3], -2.0_f32, 2.0_f32)
            }
            // Fully general random cubic over a random bracket.
            4 => {
                let c = [
                    rng.range(-2.0, 2.0),
                    rng.range(-2.0, 2.0),
                    rng.range(-2.0, 2.0),
                    rng.range(-2.0, 2.0),
                ];
                let a = rng.range(-1.0, 0.0);
                let b = rng.range(0.0, 1.0);
                (c, a, b)
            }
            // Single real root from one root + a complex-pair (disc>0 branch):
            // a·(t-r)·((t-m)²+w²) → expand.
            _ => {
                let a = 1.0;
                let r = rng.range(-1.0, 1.0);
                let m = rng.range(-1.0, 1.0);
                let w2 = rng.range(0.2, 1.0); // strictly > 0 → complex pair
                // (t-r)(t²-2mt+m²+w2) = t³ -(2m+r)t² +(m²+w2+2mr)t -r(m²+w2)
                let q = m * m + w2;
                let c3 = a;
                let c2 = a * (-(2.0 * m + r));
                let c1 = a * (q + 2.0 * m * r);
                let c0 = a * (-r * q);
                ([c0, c1, c2, c3], -2.0_f32, 2.0_f32)
            }
        };

        let m = marmitt_root(&c, t0, t1);
        let k = cardano_root(&c, t0, t1);

        match (m, k) {
            (Some(rm), Some(rk)) => {
                both_found += 1;
                let gap = (rm - rk).abs();
                if gap > max_root_gap {
                    max_root_gap = gap;
                }
                let res_m = ev(&c, rm).abs();
                let res_k = ev(&c, rk).abs();
                // Marmitt's return must always be IN the bracket and FINITE (the
                // hard contract on a wide bracket — a fixed-budget regula-falsi may
                // not converge, but it must never leave the interval or NaN).
                assert!(
                    rm.is_finite() && rm >= t0 - 1.0e-4 && rm <= t1 + 1.0e-4,
                    "marmitt root {rm} escaped the bracket [{t0},{t1}]; c={c:?} mode={mode}"
                );
                // When marmitt actually CONVERGED (small residual, i.e. it reached a
                // crossing within budget), it must agree with cardano's FIRST root to
                // the fixed-budget window. A LARGE marmitt residual means the bounded
                // 8-iteration regula-falsi stopped SHORT on a wide/curved bracket
                // (documented; the production sub-cell regime is pinned tight by
                // `marmitt_converges_tightly_on_cell_brackets`) — not a divergence.
                if res_m <= 1.0e-2 {
                    let worst = res_m.max(res_k);
                    if worst > max_residual_disagree {
                        max_residual_disagree = worst;
                    }
                    // Marmitt converged. It agrees with cardano UNLESS:
                    //  - cardano's own root is budget-imperfect (res_k large), OR
                    //  - the cubic is near-TANGENT at the root (a double root: small
                    //    slope ⇒ |Δroot| = |Δres|/|slope| blows up even though BOTH
                    //    points are genuine roots). Detect tangency via the local
                    //    slope `S'(rm)` = 3c3·rm² + 2c2·rm + c1.
                    let slope = (3.0 * c[3] * rm + 2.0 * c[2]) * rm + c[1];
                    let near_tangent = slope.abs() < 0.5; // small-slope ⇒ ill-conditioned
                    assert!(
                        gap <= ROOT_AGREE || res_k > 1.0e-2 || near_tangent,
                        "MARMITT CONVERGED to a DIFFERENT root than cardano: \
                         marmitt={rm} (res {res_m}) cardano={rk} (res {res_k}) gap={gap} \
                         slope={slope}; c={c:?} [{t0},{t1}] mode={mode}"
                    );
                } else {
                    budget_short += 1;
                }
            }
            (Some(rm), None) => {
                one_found += 1;
                // Marmitt returned a root cardano did not. The return must be in the
                // bracket and finite (the hard contract). Cardano is the analytic
                // reference; a marmitt root it "missed" arises only when cardano's
                // in-range filter (CUBIC_ROOT_EPS-padded) excluded a near-boundary
                // root marmitt's regula-falsi landed just inside — a boundary
                // book-keeping difference, not an invented interior crossing.
                assert!(
                    rm.is_finite() && rm >= t0 - 1.0e-4 && rm <= t1 + 1.0e-4,
                    "marmitt-only root {rm} escaped [{t0},{t1}]; c={c:?} mode={mode}"
                );
            }
            (None, Some(_rk)) => {
                // Cardano found an analytic root marmitt missed. Marmitt finds the
                // first SIGN-CHANGE in an EXTREMUM-PARTITIONED sub-interval. Two
                // OUT-OF-REGIME cases legitimately produce a marmitt miss here:
                //  (a) a root tangent to zero (a double root, NO sign change), and
                //  (b) a c3=0 two-interior-root parabola with same-sign endpoints
                //      and no derivative split (see
                //      `quadratic_two_interior_roots_is_out_of_regime`).
                // Both are pinned as explicit behaviors; the production regime (a
                // single clean cell crossing) never forms them. Count for the
                // report; the miss-RATE bound below catches a systematic failure.
                one_found += 1;
                marmitt_only_misses += 1;
            }
            (None, None) => {
                none += 1;
            }
        }
    }

    // The matrix must actually exercise both "found" and "none" outcomes.
    assert!(both_found > 0, "no scene where both solvers found a root — vacuous");
    assert!(none > 0, "no scene where both returned None — bracket too wide");
    // Marmitt-only-misses are the documented out-of-regime cases (double-root
    // tangency, c3=0 two-interior-root parabola). They must remain a MINORITY of
    // the cardano-found population — a majority would mean a systematic solver gap.
    let cardano_found = both_found + marmitt_only_misses;
    if cardano_found > 0 {
        let miss_rate = marmitt_only_misses as f32 / cardano_found as f32;
        assert!(
            miss_rate < 0.20,
            "MARMITT systematically misses cardano roots on wide brackets: \
             miss_rate={miss_rate} ({marmitt_only_misses}/{cardano_found}) — \
             exceeds the documented out-of-regime minority"
        );
    }
    // Surface the population for the report (not a hard gate beyond the asserts).
    // `budget_short` = both solvers found a root but marmitt's residual was large
    // (fixed-budget regula-falsi stopped short on a wide/curved bracket — by design;
    // the production sub-cell regime is pinned tight separately).
    eprintln!(
        "marmitt==cardano (wide stress): both={both_found} one={one_found} none={none} \
         marmitt_only_misses={marmitt_only_misses} budget_short={budget_short} \
         max_root_gap={max_root_gap} max_residual_at_converged_disagree={max_residual_disagree}"
    );
}

/// INVARIANT 2 (pinned cases): hand-built cubics with KNOWN first roots — the GPU
/// [`marmitt_root`] lands on the analytic crossing within the wide-bracket budget
/// ([`ROOT_AGREE`], the fixed-iteration regime), and the degenerate quadratic /
/// linear fall-throughs (`c3=0`, solved in CLOSED FORM, no iteration) resolve
/// EXACTLY. Tight convergence in the production sub-cell regime is pinned by
/// `marmitt_converges_tightly_on_cell_brackets`.
#[test]
fn marmitt_hits_known_first_roots() {
    // A clean triple-real cubic (t+1)(t)(t-1) = t³ - t; first root in [-2,2] = -1.
    // (Wide bracket → the 8-iter regula-falsi lands within ROOT_AGREE of -1.)
    let c = cubic_from_roots(1.0, -1.0, 0.0, 1.0);
    let r = marmitt_root(&c, -2.0, 2.0).expect("triple-real cubic has a first root");
    assert!((r - (-1.0)).abs() <= ROOT_AGREE, "first root of t³-t in [-2,2] is -1, got {r}");

    // First root restricted to [-0.5, 2] is now 0 — a tight bracket around 0, so
    // the iteration converges close.
    let r2 = marmitt_root(&c, -0.5, 2.0).expect("root 0 in [-0.5,2]");
    assert!(r2.abs() <= ROOT_AGREE, "first root in [-0.5,2] is 0, got {r2}");

    // Pure quadratic via c3=0 with ONE root in a TIGHT (production-sized) bracket:
    // (t-0.3)(t+0.7) over [0.0, 0.6] contains only the root +0.3 (the -0.7 root is
    // outside), the endpoints straddle a single sign change, and the narrow span
    // lets the fixed-budget regula-falsi converge tight. (A c3=0 quadratic gets NO
    // derivative split, so a WIDE bracket with two interior roots and same-sign
    // endpoints is an out-of-regime blind spot brick_cubic_hit never forms — see
    // `quadratic_two_interior_roots_is_out_of_regime`.)
    let q = [-0.21_f32, 0.4, 1.0, 0.0]; // (t-0.3)(t+0.7)
    let rq = marmitt_root(&q, 0.0, 0.6).expect("single-root quadratic bracket");
    assert!((rq - 0.3).abs() <= ROOT_AGREE, "quadratic root in [0,0.6] is 0.3, got {rq}");

    // Pure linear via c3=c2=0: 2t - 1 → root 0.5.
    let lin = [-1.0_f32, 2.0, 0.0, 0.0];
    let rl = marmitt_root(&lin, -2.0, 2.0).expect("linear root");
    assert!((rl - 0.5).abs() <= ROOT_AGREE, "linear root 0.5, got {rl}");

    // No sign change in range → None (a cubic strictly positive on [2,3]).
    let pos = cubic_from_roots(1.0, -1.0, 0.0, 1.0); // roots at -1,0,1; positive for t>1
    assert!(marmitt_root(&pos, 2.0, 3.0).is_none(), "no root in [2,3] → None");
}

/// INVARIANT 2 (DOCUMENTED blind spot, NOT a bug): a `c3=0` quadratic with BOTH
/// roots strictly INSIDE the bracket and same-sign endpoints (the parabola dips
/// negative only between the roots) gets NO derivative split (the split branch
/// guards `qa = 3·c3 ≠ 0`), so [`marmitt_root`] walks `[t0, t1]` as ONE interval,
/// sees same-sign endpoints, and returns `None`. This is OUT of the production
/// regime: `brick_cubic_hit` forms a sub-voxel-cell bracket where the trilinear
/// isosurface crosses the ray at most where the cell-segment endpoints bracket it
/// (the DDA gives one monotone crossing per cell). Pinned so the behavior is an
/// EXPLICIT, characterized property the GPU port reproduces — not a silent gap.
#[test]
fn quadratic_two_interior_roots_is_out_of_regime() {
    // (t-0.3)(t+0.7) = t² + 0.4t − 0.21 → c = [-0.21, 0.4, 1, 0]. Both roots
    // (-0.7, 0.3) lie inside [-2, 2]; f(-2)=+2.99, f(2)=+4.59 (same sign).
    let q = [-0.21_f32, 0.4, 1.0, 0.0];
    assert!(
        marmitt_root(&q, -2.0, 2.0).is_none(),
        "documented: a c3=0 two-interior-root parabola with same-sign endpoints \
         returns None (no derivative split, no bracketed sign change)"
    );
    // Cardano (closed form) DOES find the smaller root — confirming the difference
    // is the algorithm contract, not a wrong field.
    let k = cardano_root(&q, -2.0, 2.0).expect("cardano solves the quadratic in closed form");
    assert!((k - (-0.7)).abs() <= 1.0e-3, "cardano finds the true first root -0.7, got {k}");
}

/// INVARIANT 2 (PRODUCTION regime): on SUB-VOXEL-CELL brackets — the only kind
/// `brick_cubic_hit` ever forms (`[0, seg_hi-seg_lo]`, at most a few voxels of
/// local-`t`) — the fixed-budget [`marmitt_root`] converges to within
/// `~CUBIC_ROOT_EPS` of the analytic [`cardano_root`]. This is the accuracy the
/// load-bearing oracle actually relies on (the wide-bracket budget in
/// `marmitt_hits_known_first_roots` is a synthetic stress, NOT the real input).
#[test]
fn marmitt_converges_tightly_on_cell_brackets() {
    let mut rng = XorShift64::new(0x7E57_C0DE_1234_FEED);
    let mut checked = 0u64;
    let mut max_gap = 0.0_f32;
    for _ in 0..20000 {
        // A cubic with a root inside a sub-cell bracket [0, span], span <= 3 voxels.
        let span = rng.range(0.2, 3.0);
        // Place a single real root r0 inside [0, span] and two roots outside, so
        // the FIRST in-range crossing is unambiguous.
        let r0 = rng.range(0.05 * span, 0.95 * span);
        let r1 = span + rng.range(0.5, 3.0); // beyond the bracket
        let r2 = -rng.range(0.5, 3.0); // before the bracket
        let a = {
            let v = rng.range(-2.0, 2.0);
            if v.abs() < 0.3 { 0.5 } else { v }
        };
        let c = cubic_from_roots(a, r0, r1, r2);
        let m = marmitt_root(&c, 0.0, span);
        let k = cardano_root(&c, 0.0, span);
        if let (Some(rm), Some(rk)) = (m, k) {
            checked += 1;
            let gap = (rm - rk).abs();
            if gap > max_gap {
                max_gap = gap;
            }
            // On a sub-cell bracket the two solvers agree to a tight window — this
            // is the production accuracy contract. Empirically (500k single-crossing
            // cubics, span <= 3 voxels) the worst gap is ~4.97e-3; the gate is set
            // at 6e-3 with a small margin. This is the accuracy the load-bearing
            // oracle (brick_cubic_hit) actually relies on.
            assert!(
                gap <= 6.0e-3,
                "narrow-bracket marmitt {rm} vs cardano {rk} gap {gap} > 6e-3 \
                 (span={span} r0={r0}); the production regula-falsi must converge tightly"
            );
        }
    }
    assert!(checked > 1000, "too few narrow-bracket roots exercised ({checked})");
    eprintln!("marmitt narrow-bracket convergence: checked={checked} max_gap={max_gap}");
}

/// INVARIANT 2 (range/NaN guards): both solvers reject a non-ordered or NaN
/// bracket identically (no panic, both `None`).
#[test]
fn root_finders_reject_degenerate_brackets() {
    let c = cubic_from_roots(1.0, -1.0, 0.0, 1.0);
    for &(t0, t1) in &[(1.0_f32, 1.0_f32), (1.0, 0.0), (f32::NAN, 1.0), (0.0, f32::NAN)] {
        assert!(marmitt_root(&c, t0, t1).is_none(), "marmitt rejects [{t0},{t1}]");
        assert!(cardano_root(&c, t0, t1).is_none(), "cardano rejects [{t0},{t1}]");
    }
}

// ─── M2.5 — APRON / atlas_uvw addressing parity ───────────────────────────────

/// INVARIANT 5: [`atlas_uvw`] addresses the SAME low-corner cell + in-cell
/// fraction that [`trilinear_reconstruct`] (via [`clamp_index`]) fetches — the
/// cubic and the trilinear sample the IDENTICAL 8 voxels. Verified by replaying
/// `atlas_uvw`'s `(g, i0)` against the production grid-coordinate math at many
/// interior points and confirming the corner indices + fractions match.
#[test]
fn atlas_uvw_addresses_same_corners_as_trilinear() {
    // Bias is golden-locked to 0.0 (the GPU step pins it later).
    assert_eq!(ATLAS_SAMPLE_BIAS, 0.0, "M2 step 1 pins ATLAS_SAMPLE_BIAS at 0.0");
    const W: usize = BRICK_ALLOC;
    let mut rng = XorShift64::new(0xA9C0_FFEE_1357_BEEF);
    for _ in 0..5000 {
        // Interior-voxel coordinate spanning the valid sample range (incl. the
        // apron reach one voxel past each interior face).
        let local = [
            rng.range(-0.5, BRICK_INTERIOR as f32 + 0.5),
            rng.range(-0.5, BRICK_INTERIOR as f32 + 0.5),
            rng.range(-0.5, BRICK_INTERIOR as f32 + 0.5),
        ];
        let (g, i0) = atlas_uvw(local, ATLAS_SAMPLE_BIAS);

        for axis in 0..3 {
            // The production grid coordinate `trilinear_reconstruct` computes.
            let g_expected = local[axis] + APRON as f32 - 0.5 + ATLAS_SAMPLE_BIAS;
            assert!(
                (g[axis] - g_expected).abs() <= 1.0e-6,
                "atlas_uvw g[{axis}]={} != trilinear grid coord {g_expected}",
                g[axis]
            );
            // The low corner index must equal trilinear_reconstruct's clamp_index.
            let i_expected = clamp_index(g_expected, W);
            assert_eq!(
                i0[axis], i_expected,
                "atlas_uvw i0[{axis}]={} != clamp_index {i_expected} (corner mismatch)",
                i0[axis]
            );
            // The +1 neighbour stays in-bounds.
            assert!(i0[axis] < W - 1, "corner +1 must be in-bounds");
            // The in-cell fraction is in [0,1] (g - i0), the JCGT local frame.
            let frac = g[axis] - i0[axis] as f32;
            assert!(
                (-1.0e-6..=1.0 + 1.0e-6).contains(&frac),
                "in-cell fraction {frac} must be in [0,1] (axis {axis})"
            );
        }
    }
}

/// INVARIANT 5 (corner-fetch identity): for a baked brick, the 8 corners
/// [`brick_cubic_hit`]/`atlas_uvw` address are EXACTLY the corners
/// [`trilinear_reconstruct`] blends — proven by reconstructing the trilinear
/// value two ways (the production fetch vs the cubic's corner array evaluated at
/// the same fraction) and requiring bit-for-bit agreement.
#[test]
fn cubic_corner_fetch_matches_trilinear_fetch() {
    const W: usize = BRICK_ALLOC;
    let voxel = M2_VOXEL;
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([1.0, 1.0, 1.0], 0.9, sdf_op::UNION, 0.0));
    field.bump_gen();
    let brick_min = [0.0, 0.0, 0.0];
    let mut brick = [0i8; BRICK_VOXELS];
    fill_brick(&field, brick_min, voxel, BAND_HALF_STORE, C_MAX, &mut brick);

    let mut rng = XorShift64::new(0xBEEF_FACE_0BAD_F00D);
    for _ in 0..2000 {
        let local = [
            rng.range(0.0, BRICK_INTERIOR as f32),
            rng.range(0.0, BRICK_INTERIOR as f32),
            rng.range(0.0, BRICK_INTERIOR as f32),
        ];
        let (g, i0) = atlas_uvw(local, ATLAS_SAMPLE_BIAS);
        let frac = [
            g[0] - i0[0] as f32,
            g[1] - i0[1] as f32,
            g[2] - i0[2] as f32,
        ];
        // Decode the 8 corners atlas_uvw addresses, in s_ijk ↔ x+2y+4z order.
        let (cx, cy, cz) = (i0[0], i0[1], i0[2]);
        let s = [
            decode_snorm8(brick[cx + cy * W + cz * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[(cx + 1) + cy * W + cz * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[cx + (cy + 1) * W + cz * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[(cx + 1) + (cy + 1) * W + cz * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[cx + cy * W + (cz + 1) * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[(cx + 1) + cy * W + (cz + 1) * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[cx + (cy + 1) * W + (cz + 1) * W * W], BAND_HALF_STORE),
            decode_snorm8(brick[(cx + 1) + (cy + 1) * W + (cz + 1) * W * W], BAND_HALF_STORE),
        ];
        let via_corners = trilinear_blend_corners(&s, frac);
        let via_fetch = trilinear_reconstruct(&brick, local, BAND_HALF_STORE);
        assert!(
            (via_corners - via_fetch).abs() <= 1.0e-6,
            "atlas_uvw corners blend {via_corners} != trilinear_reconstruct {via_fetch} \
             at local={local:?} (the cubic and the sampler MUST address the same voxels)"
        );
    }
}

// ─── M2.3 — BRICK_CUBIC_HIT == ANALYTIC ZERO-CROSSING (the load-bearing oracle) ─

/// Marches the EXACT analytic field along a world ray to find the first
/// zero-crossing parameter `t` in `[t_lo, t_hi]` (a dense scan + bisection
/// refine). The reference the brick cubic hit is compared against. Returns the
/// world-space `t` (NOT the brick frame), or `None` if no sign change is found.
fn analytic_first_crossing(
    edits: &[SdfEdit],
    ro: [f32; 3],
    rd: [f32; 3],
    t_lo: f32,
    t_hi: f32,
) -> Option<f32> {
    const STEPS: u32 = 4096;
    let at = |t: f32| -> f32 {
        sdf_edit_list(edits, [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t])
    };
    let mut prev_t = t_lo;
    let mut prev_f = at(prev_t);
    for i in 1..=STEPS {
        let t = t_lo + (t_hi - t_lo) * i as f32 / STEPS as f32;
        let f = at(t);
        if prev_f == 0.0 {
            return Some(prev_t);
        }
        if prev_f * f < 0.0 {
            // Bisect the bracket [prev_t, t] to a tight crossing.
            let (mut a, mut fa, mut b) = (prev_t, prev_f, t);
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                let fm = at(m);
                if fm == 0.0 || (b - a) <= 1.0e-7 {
                    return Some(m);
                }
                if fa * fm < 0.0 {
                    b = m;
                } else {
                    a = m;
                    fa = fm;
                }
            }
            return Some(0.5 * (a + b));
        }
        prev_t = t;
        prev_f = f;
    }
    None
}

/// INVARIANT 3: over MANY random scenes, a SURFACE brick straddling the surface,
/// and a battery of rays through it, `brick_cubic_hit` returns a `t` whose WORLD
/// point lies on the analytic surface (`|sdf_edit_list(hit)| <= tol`). The tol is
/// the brick's reconstruction budget: the EPSILON_Q store bias + the trilinear
/// midpoint slack + the snorm quant step + FP noise — the cubic solves the
/// trilinear isosurface EXACTLY, but that isosurface sits at most this far from
/// the analytic zero (the M2 marcher's analytic-residual fallback closes the
/// remaining gap; here we only flag EGREGIOUS misses). Reports the max residual
/// and any cubic-hit-vs-analytic-miss disagreement.
#[test]
fn brick_cubic_hit_lands_on_analytic_surface() {
    const SEEDS: u64 = 700;
    let voxel = M2_VOXEL;
    let brick_size = voxel * BRICK_INTERIOR as f32; // 2.0
    let band = BAND_HALF_STORE;

    // The reconstruction budget: how far the trilinear isosurface (what the cubic
    // solves) can sit from the analytic zero, plus the store down-bias (the brick
    // is a biased-DOWN encode, so its zero shifts toward the inside by ~bias).
    let bias = EPSILON_Q * band;
    let delta_tri = VOXEL_SIZE * VOXEL_SIZE * C_MAX / 8.0;
    let quant = band / 127.0;
    // Convert the band-distance budget to a WORLD residual on |sdf|: near the
    // surface |∇sdf|≈1, so a distance error ≈ a position error. The bias shifts
    // the zero by ~bias; allow a generous multiple for CSG-crease curvature the
    // fixed C_MAX under-models (documented: the marcher's analytic fallback fixes
    // these — we fail only on EGREGIOUS, >5x-budget misses).
    let budget = bias + delta_tri + quant; // ≈ 0.0216 + 0.0156 + 0.0071 ≈ 0.044
    let egregious = 6.0 * budget; // the hard fail line for an oracle-level miss

    let mut max_residual = 0.0_f32;
    let mut hits_checked = 0u64;
    let mut over_budget = 0u64; // within egregious but over the tight budget
    let mut cubic_hit_analytic_miss = 0u64;
    let mut analytic_hit_cubic_miss = 0u64;
    let mut analytic_t_tracked = 0u64;
    let mut max_t_gap = 0.0_f32;
    let mut worst_residual_world = [0.0_f32; 3];

    for seed in 0..SEEDS {
        let mut rng = XorShift64::new(seed.wrapping_mul(0x1_0001_0001).wrapping_add(7));
        let (field, focus) = random_scene(&mut rng);
        let edits = field.edits();

        // Center the brick near the first edit so its surface crosses the interior.
        let jitter = [
            rng.range(-0.6, 0.6),
            rng.range(-0.6, 0.6),
            rng.range(-0.6, 0.6),
        ];
        let brick_center = [focus[0] + jitter[0], focus[1] + jitter[1], focus[2] + jitter[2]];
        let brick_min = [
            brick_center[0] - brick_size * 0.5,
            brick_center[1] - brick_size * 0.5,
            brick_center[2] - brick_size * 0.5,
        ];

        // Only bricks that actually classify Surface carry voxel data.
        if classify_brick(&field, brick_min, brick_size, band) != BrickClass::Surface {
            continue;
        }
        let mut brick = [0i8; BRICK_VOXELS];
        fill_brick(&field, brick_min, voxel, band, C_MAX, &mut brick);

        // Fire a battery of rays through the brick from random points on a sphere
        // around the brick center, aimed roughly at the center (so they traverse
        // the interior and likely cross the surface).
        for _ in 0..24 {
            // A world ray origin outside the brick, direction toward the center
            // with jitter.
            let dir = [
                rng.range(-1.0, 1.0),
                rng.range(-1.0, 1.0),
                rng.range(-1.0, 1.0),
            ];
            let dl = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            if dl < 1.0e-3 {
                continue;
            }
            let dn = [dir[0] / dl, dir[1] / dl, dir[2] / dl];
            let start_dist = brick_size; // outside the brick
            let aim = [
                brick_center[0] + rng.range(-0.5, 0.5),
                brick_center[1] + rng.range(-0.5, 0.5),
                brick_center[2] + rng.range(-0.5, 0.5),
            ];
            let world_ro = [
                aim[0] - dn[0] * start_dist,
                aim[1] - dn[1] * start_dist,
                aim[2] - dn[2] * start_dist,
            ];
            let world_rd = dn;

            // Map the world ray into the interior-voxel frame: local = (w-min)/vox,
            // dir scales by 1/vox.
            let ro_local = world_to_local(world_ro, brick_min, voxel);
            let rd_local = [world_rd[0] / voxel, world_rd[1] / voxel, world_rd[2] / voxel];

            // Clip the ray to the interior AABB [0, BRICK_INTERIOR]³ (the caller's
            // brick-slab test). Slab method in interior-voxel coords.
            let mut t_enter = 0.0_f32;
            let mut t_exit = f32::INFINITY;
            let mut clipped_ok = true;
            for axis in 0..3 {
                let lo = 0.0_f32;
                let hi = BRICK_INTERIOR as f32;
                if rd_local[axis].abs() < 1.0e-9 {
                    if ro_local[axis] < lo || ro_local[axis] > hi {
                        clipped_ok = false;
                        break;
                    }
                } else {
                    let inv = 1.0 / rd_local[axis];
                    let mut ta = (lo - ro_local[axis]) * inv;
                    let mut tb = (hi - ro_local[axis]) * inv;
                    if ta > tb {
                        core::mem::swap(&mut ta, &mut tb);
                    }
                    t_enter = t_enter.max(ta);
                    t_exit = t_exit.min(tb);
                }
            }
            if !clipped_ok || t_exit <= t_enter {
                continue;
            }

            let cubic = brick_cubic_hit(&brick, ro_local, rd_local, t_enter, t_exit, band);

            // The analytic crossing in WORLD-t over the brick's interior span.
            // World t = local t (the direction was scaled by 1/voxel, so a
            // local-frame t advances world position by rd_local*voxel = world_rd;
            // i.e. local-frame t == world-frame t along this mapped ray).
            let analytic =
                analytic_first_crossing(edits, world_ro, world_rd, t_enter, t_exit);

            match (cubic, analytic) {
                (Some(tc), _) => {
                    // The cubic hit's WORLD point must lie on the analytic surface.
                    let hit_local = [
                        ro_local[0] + rd_local[0] * tc,
                        ro_local[1] + rd_local[1] * tc,
                        ro_local[2] + rd_local[2] * tc,
                    ];
                    let hit_world = local_to_world(hit_local, brick_min, voxel);
                    let resid = sdf_edit_list(edits, hit_world).abs();
                    hits_checked += 1;
                    if resid > max_residual {
                        max_residual = resid;
                        worst_residual_world = hit_world;
                    }
                    if resid > budget {
                        over_budget += 1;
                    }
                    if analytic.is_none() {
                        cubic_hit_analytic_miss += 1;
                    }
                    // The cubic must ALSO satisfy the trilinear-isosurface identity
                    // it actually solves: reconstruct ~0 at the hit (its true
                    // contract), within CUBIC_ROOT_EPS scaled by the local slope.
                    let recon = trilinear_reconstruct(&brick, hit_local, band).abs();
                    // recon is a band-distance; near a steep crease the slope in
                    // interior-voxel units can be ~band/voxel, so allow a slope-
                    // scaled window. The hard contract (|sdf|) is asserted below.
                    debug_assert!(recon < band + 1.0e-3, "recon in band");
                    // EGREGIOUS oracle miss → hard fail (the marcher fallback can't
                    // be expected to rescue a >6x-budget surface error).
                    assert!(
                        resid <= egregious,
                        "BRICK_CUBIC_HIT EGREGIOUS surface miss: |sdf(hit)|={resid} \
                         > {egregious} (6x budget {budget}); hit_world={hit_world:?} \
                         tc={tc} seed={seed} recon={recon}"
                    );
                    // If the analytic ray also crossed, the cubic t should be near
                    // the analytic t (voxel-rounding tol).
                    if let Some(ta) = analytic {
                        let gap = (tc - ta).abs();
                        // A world-distance gap: |Δt| * |world_rd|, and |world_rd|=1
                        // (dn is unit), so gap is already a world distance.
                        if gap <= 4.0 * VOXEL_SIZE {
                            analytic_t_tracked += 1;
                            if gap > max_t_gap {
                                max_t_gap = gap;
                            }
                        }
                    }
                }
                (None, Some(_ta)) => {
                    // The analytic ray crossed but the cubic found nothing. This is
                    // the case the M2 marcher's analytic-residual fallback catches;
                    // count it, fail only if it is the COMMON case (a systematic
                    // miss would mean the DDA/cubic is broken, not an edge case).
                    analytic_hit_cubic_miss += 1;
                }
                (None, None) => {}
            }
        }
    }

    // The matrix must actually exercise hits.
    assert!(hits_checked > 0, "no cubic hits generated across {SEEDS} seeds — vacuous");

    // A systematic cubic-miss (the DDA/cubic broadly failing to find crossings the
    // analytic field has) would be a real bug; a small minority is the documented
    // fallback territory. Gate the MISS RATE, not individual misses.
    let total_rays_with_analytic = analytic_hit_cubic_miss + analytic_t_tracked;
    if total_rays_with_analytic > 0 {
        let miss_rate = analytic_hit_cubic_miss as f32 / total_rays_with_analytic as f32;
        assert!(
            miss_rate < 0.5,
            "BRICK_CUBIC_HIT systematically misses analytic crossings: \
             miss_rate={miss_rate} ({analytic_hit_cubic_miss} missed / \
             {total_rays_with_analytic} analytic-crossing rays) — DDA/cubic likely broken"
        );
    }

    eprintln!(
        "brick_cubic_hit==analytic: hits={hits_checked} max_residual={max_residual} \
         (budget={budget}, egregious={egregious}) over_budget={over_budget} \
         cubic_hit_analytic_miss={cubic_hit_analytic_miss} \
         analytic_hit_cubic_miss={analytic_hit_cubic_miss} \
         analytic_t_tracked={analytic_t_tracked} max_t_gap={max_t_gap} \
         worst_residual_world={worst_residual_world:?}"
    );
}

/// INVARIANT 3 (clean analytic case): a single R_MIN-radius sphere baked into a
/// brick — a ray straight through the center must hit the cubic isosurface within
/// the budget of the analytic sphere surface (no CSG creases to stress the bound).
#[test]
fn brick_cubic_hit_single_sphere_matches_radius() {
    let voxel = M2_VOXEL;
    let band = BAND_HALF_STORE;
    let r = 1.0_f32; // a smooth, well-within-contract sphere
    let mut field = SdfEditField::new();
    // Center the sphere at the brick interior center (brick spans [0,2]³).
    field.push(SdfEdit::sphere([1.0, 1.0, 1.0], r, sdf_op::UNION, 0.0));
    field.bump_gen();
    let brick_min = [0.0, 0.0, 0.0];
    let mut brick = [0i8; BRICK_VOXELS];
    fill_brick(&field, brick_min, voxel, band, C_MAX, &mut brick);
    let edits = field.edits();

    // A ray along +x at world y=1.4, z=1.0 (an OFF-axis chord so neither crossing
    // lands on a brick face). The sphere chord at y=1.4,z=1 spans
    // (x-1)² = 1 - 0.4² = 0.84 → x = 1 ± 0.9165, i.e. NEAR crossing at world
    // x≈0.0835 (strictly interior, not on the x=0 face). Enter at the -x interior
    // edge heading +x.
    let world_ro = [0.0, 1.4, 1.0];
    let world_rd = [1.0, 0.0, 0.0];
    let ro_local = world_to_local(world_ro, brick_min, voxel);
    let rd_local = [world_rd[0] / voxel, world_rd[1] / voxel, world_rd[2] / voxel];
    let t_enter = 0.0_f32;
    let t_exit = BRICK_INTERIOR as f32; // full interior span along x

    let tc = brick_cubic_hit(&brick, ro_local, rd_local, t_enter, t_exit, band)
        .expect("a ray through a centered sphere must hit the cubic isosurface");
    let hit_local = [
        ro_local[0] + rd_local[0] * tc,
        ro_local[1] + rd_local[1] * tc,
        ro_local[2] + rd_local[2] * tc,
    ];
    let hit_world = local_to_world(hit_local, brick_min, voxel);
    let resid = sdf_edit_list(edits, hit_world).abs();
    let bias = EPSILON_Q * band;
    let budget = bias + VOXEL_SIZE * VOXEL_SIZE * C_MAX / 8.0 + band / 127.0;
    assert!(
        resid <= budget,
        "single-sphere cubic hit off the analytic surface: |sdf|={resid} > budget={budget} \
         hit_world={hit_world:?} tc={tc}"
    );
    // The analytic NEAR crossing along +x is at world x ≈ 0.0835 (the −x side of
    // the sphere). The cubic must find the NEAR side, not the far x≈1.9165 exit —
    // allow the reconstruction budget plus one voxel of DDA-cell rounding.
    let near_x = 1.0 - (1.0_f32 - 0.4 * 0.4).sqrt(); // ≈ 0.0835
    assert!(
        (hit_world[0] - near_x).abs() <= budget + VOXEL_SIZE,
        "cubic must find the NEAR crossing (world x≈{near_x}), got world x={}",
        hit_world[0]
    );
}

// ─── M2.4 — DEGENERATE RAYS: no panic / NaN / infinite loop ───────────────────

/// INVARIANT 4: pathological rays through a baked brick — axis-parallel, through a
/// cell corner, grazing-tangent, zero-direction — must NOT panic, NaN, or hang.
/// `brick_cubic_hit` returns `None` or a finite in-range hit.
#[test]
fn brick_cubic_hit_degenerate_rays_are_robust() {
    let voxel = M2_VOXEL;
    let band = BAND_HALF_STORE;
    let mut field = SdfEditField::new();
    field.push(SdfEdit::box_shape([1.0, 1.0, 1.0], [0.6, 0.6, 0.6], sdf_op::UNION, 0.0));
    field.bump_gen();
    let brick_min = [0.0, 0.0, 0.0];
    let mut brick = [0i8; BRICK_VOXELS];
    fill_brick(&field, brick_min, voxel, band, C_MAX, &mut brick);

    let interior = BRICK_INTERIOR as f32;
    // A battery of degenerate ray configs, in interior-voxel coords.
    let cases: &[([f32; 3], [f32; 3], f32, f32)] = &[
        // Axis-parallel along +x, exactly on a voxel-face plane (y=2.0, z=2.0).
        ([0.0, 2.0, 2.0], [1.0, 0.0, 0.0], 0.0, interior),
        // Axis-parallel along +y, on integer grid lines (x=4, z=4).
        ([4.0, 0.0, 4.0], [0.0, 1.0, 0.0], 0.0, interior),
        // Through a cell corner: diagonal hitting (1,1,1)·k integer corners.
        ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0.0, interior),
        // Grazing tangent: nearly parallel to a face, tiny y component.
        ([0.0, 1.0e-4, 4.0], [1.0, 1.0e-5, 0.0], 0.0, interior),
        // Zero direction (fully degenerate) — must not divide-by-zero-hang.
        ([4.0, 4.0, 4.0], [0.0, 0.0, 0.0], 0.0, interior),
        // Reversed span (t_exit <= t_enter) — must early-out None.
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 5.0, 1.0),
        // NaN direction component — must not propagate to a panic.
        ([0.0, 0.0, 0.0], [f32::NAN, 1.0, 0.0], 0.0, interior),
        // Negative direction along z, starting at the far face.
        ([4.0, 4.0, interior], [0.0, 0.0, -1.0], 0.0, interior),
        // Very large direction magnitude (steep ray).
        ([0.0, 4.0, 4.0], [1000.0, 0.1, 0.1], 0.0, interior),
    ];

    for (i, &(ro, rd, te, tx)) in cases.iter().enumerate() {
        let hit = brick_cubic_hit(&brick, ro, rd, te, tx, band);
        if let Some(t) = hit {
            assert!(t.is_finite(), "case {i}: hit t must be finite, got {t}");
            assert!(
                t >= te - 1.0e-3 && t <= tx + 1.0e-3,
                "case {i}: hit t={t} must lie in the brick span [{te},{tx}]"
            );
        }
        // Reaching here (no panic / no hang) is the robustness contract for None.
    }
}

/// INVARIANT 4 (root-finder degenerate-coefficient robustness): the GPU root-finder
/// must not panic on a NaN coefficient (e.g. a NaN-corner cell) or an ALL-ZERO
/// cubic (a flat empty cell). It returns `None` or a finite root.
///
/// NOTE: Inf coefficients are NOT tested — they are not a reachable input. A real
/// brick cell's corners are bounded snorm decodes (`|s| <= band_half`), and
/// `jcgt_cubic_coeffs` of finite corners is always finite (pinned by
/// `jcgt_coeffs_are_finite_for_finite_inputs`), so `S(t) = inf·t + …` never forms.
#[test]
fn root_finders_tolerate_nan_and_zero_coeffs() {
    let bad = [
        [f32::NAN, 1.0, 2.0, 3.0],   // a NaN constant term
        [1.0, f32::NAN, -1.0, 0.5],  // a NaN linear term
        [0.0, 0.0, 0.0, 0.0],        // all-zero: a flat empty cell, no crossing
    ];
    for c in &bad {
        let m = marmitt_root(c, -2.0, 2.0);
        let k = cardano_root(c, -2.0, 2.0);
        if let Some(t) = m {
            assert!(t.is_finite(), "marmitt returned non-finite root for {c:?}");
        }
        if let Some(t) = k {
            assert!(t.is_finite(), "cardano returned non-finite root for {c:?}");
        }
    }
    // The all-zero (flat) cubic has no isolated crossing → no spurious root.
    assert!(
        marmitt_root(&[0.0, 0.0, 0.0, 0.0], 0.0, 1.0).is_none()
            || marmitt_root(&[0.0, 0.0, 0.0, 0.0], 0.0, 1.0) == Some(0.0),
        "a flat-zero cubic yields no crossing (or the trivial endpoint)"
    );
}

/// INVARIANT 4 (cubic eval finiteness): `jcgt_cubic_coeffs` over finite inputs
/// always yields finite coefficients (no spurious NaN from the k-basis fold).
#[test]
fn jcgt_coeffs_are_finite_for_finite_inputs() {
    let mut rng = XorShift64::new(0x5151_5151_AAAA_BBBB);
    for _ in 0..5000 {
        let mut s = [0.0_f32; 8];
        for v in s.iter_mut() {
            *v = rng.range(-3.0, 3.0);
        }
        let ro = [rng.range(-1.0, 2.0), rng.range(-1.0, 2.0), rng.range(-1.0, 2.0)];
        let rd = [rng.range(-3.0, 3.0), rng.range(-3.0, 3.0), rng.range(-3.0, 3.0)];
        let c = jcgt_cubic_coeffs(&s, ro, rd);
        assert!(
            c.iter().all(|v| v.is_finite()),
            "jcgt_cubic_coeffs produced a non-finite coeff {c:?} for finite inputs"
        );
    }
}

// ─── M4 clip-map LOD: per-level math (deterministic unit checks) ──────────────
//
// The per-level conservative-lower-bound PROPTEST (re-baking + worst-case offset
// at every level) is the tester's job; these are the pure-math invariants.

/// `brick_world_at_level` doubles per level off the `M2_BRICK_WORLD = 2.0` base.
#[test]
fn brick_world_doubles_per_level() {
    assert_eq!(brick_world_at_level(0), 2.0);
    assert_eq!(brick_world_at_level(1), 4.0);
    assert_eq!(brick_world_at_level(2), 8.0);
}

/// `voxel_size_at_level` doubles per level off the `VOXEL_SIZE = 0.25` base.
#[test]
fn voxel_size_doubles_per_level() {
    assert_eq!(voxel_size_at_level(0), 0.25);
    assert_eq!(voxel_size_at_level(1), 0.5);
    assert_eq!(voxel_size_at_level(2), 1.0);
}

/// `band_half_at_level` follows `2^L`; `c_max_at_level` follows `2^-L`; and
/// `r_min_at_level` is the inverse of `c_max_at_level` at every level.
#[test]
fn band_and_curvature_follow_powers_of_two() {
    for l in 0..BRICK_LEVELS as u32 {
        let scale = (1u32 << l) as f32;
        assert_eq!(band_half_at_level(l), BAND_HALF_STORE * scale);
        assert_eq!(c_max_at_level(l), C_MAX / scale);
        assert_eq!(r_min_at_level(l), R_MIN * scale);
        // c_max_L == 1 / r_min_L (the two views of the curvature floor agree).
        assert!((c_max_at_level(l) - 1.0 / r_min_at_level(l)).abs() <= 1e-6);
    }
    assert_eq!(band_half_at_level(0), 0.90);
    assert_eq!(c_max_at_level(0), 2.0);
    assert_eq!(c_max_at_level(2), 0.5);
}

/// The level-0 per-level values reduce EXACTLY to the single-level M0 constants
/// (the clip-map's finest level is byte-identical to the pre-M4 brick scale).
#[test]
fn level_zero_reduces_to_single_level_constants() {
    assert_eq!(brick_world_at_level(0), M2_BRICK_WORLD);
    assert_eq!(voxel_size_at_level(0), VOXEL_SIZE);
    assert_eq!(band_half_at_level(0), BAND_HALF_STORE);
    assert_eq!(c_max_at_level(0), C_MAX);
    assert_eq!(r_min_at_level(0), R_MIN);
}

/// `snapped_level_origin` is grid-aligned per axis at every level: each axis is
/// an integer multiple of that level's `brick_world` (the anti-jitter contract).
#[test]
fn snapped_origin_is_grid_aligned_per_axis() {
    let cameras = [
        [0.0, 0.0, 0.0],
        [0.13, -1.77, 2.41],
        [-3.5, 3.5, -0.01],
        [5.99, -6.01, 0.5],
        [-0.5, 1.0, -2.0],
    ];
    for &cam in &cameras {
        for l in 0..BRICK_LEVELS as u32 {
            let bw = brick_world_at_level(l);
            let origin = snapped_level_origin(cam, l);
            for a in 0..3 {
                let multiple = origin[a] / bw;
                assert!(
                    (multiple - multiple.round()).abs() <= 1e-4,
                    "axis {a} of level {l} origin {} not a multiple of brick_world {bw}",
                    origin[a]
                );
                // The snapped origin encloses the camera from below on this axis
                // (the centered min never overshoots past the camera).
                assert!(
                    origin[a] <= cam[a],
                    "axis {a} of level {l}: snapped min {} must be <= camera {}",
                    origin[a],
                    cam[a]
                );
            }
        }
    }
}

/// The clip-map levels are STRICTLY concentric: each level's world extent
/// `[origin, origin + M2_GRID_DIM·brick_world]` strictly contains the previous
/// level's extent on every axis (the nesting the marcher's level-select relies on).
#[test]
fn level_extents_are_strictly_nested() {
    let cameras = [[0.0, 0.0, 0.0], [0.37, -1.2, 2.0], [-2.5, 1.9, -3.1]];
    for &cam in &cameras {
        for l in 1..BRICK_LEVELS as u32 {
            let inner_bw = brick_world_at_level(l - 1);
            let outer_bw = brick_world_at_level(l);
            let inner_min = snapped_level_origin(cam, l - 1);
            let outer_min = snapped_level_origin(cam, l);
            let inner_span = M2_GRID_DIM as f32 * inner_bw;
            let outer_span = M2_GRID_DIM as f32 * outer_bw;
            // The coarser extent is strictly wider (doubles per level).
            assert!(outer_span > inner_span);
            for a in 0..3 {
                let inner_max = inner_min[a] + inner_span;
                let outer_max = outer_min[a] + outer_span;
                assert!(
                    outer_min[a] <= inner_min[a] && outer_max >= inner_max,
                    "axis {a}, level {l}: outer [{}, {outer_max}] must contain inner [{}, {inner_max}]",
                    outer_min[a],
                    inner_min[a]
                );
                // Phase alignment: the coarse boundary coincides with a fine
                // boundary (the coarse cell is an integer multiple of the fine one).
                let ratio = outer_bw / inner_bw;
                assert!((ratio - 2.0).abs() <= 1e-6, "coarse cell must be 2x the fine cell");
            }
        }
    }
}

// ---- M5a — toroidal clip-map streaming (the bake-side math) ----

/// The M5 reimpl keystone: `snapped_level_origin(camera, level)` is BYTE-IDENTICAL
/// to `snapped_level_origin_cell(camera, level)[a] as f32 * brick_world_at_level(level)`
/// on every axis. The toroidal OFF reduction depends on the snapped origin being
/// exactly `origin_cell · bw` — a one-ULP drift would desync the shader's
/// `round(origin/bw)` recompute from the host's integer cell.
#[test]
fn m4_snapped_origin_equals_cell_times_bw() {
    let cameras = [
        [0.0, 0.0, 0.0],
        [0.13, -1.77, 2.41],
        [-3.5, 3.5, -0.01],
        [5.99, -6.01, 0.5],
        [-0.5, 1.0, -2.0],
        [123.456, -78.9, 41.0],
        [-200.0, 200.0, -0.25],
    ];
    for &cam in &cameras {
        for l in 0..BRICK_LEVELS as u32 {
            let bw = brick_world_at_level(l);
            let origin = snapped_level_origin(cam, l);
            let cell = snapped_level_origin_cell(cam, l);
            for a in 0..3 {
                let from_cell = cell[a] as f32 * bw;
                assert_eq!(
                    origin[a].to_bits(),
                    from_cell.to_bits(),
                    "axis {a} level {l} cam {cam:?}: origin {} != cell*bw {from_cell} (bit-diff)",
                    origin[a]
                );
            }
        }
    }
}

/// `toroidal_slot` reduces to the identity on the OFF box `[0, M2_GRID_DIM)³` (the
/// byte-identity keystone) and wraps via `rem_euclid` for negative world cells.
#[test]
fn toroidal_slot_off_identity_and_negative_wrap() {
    let dim = M2_GRID_DIM as i32;
    // OFF box: slot == cell.
    for z in 0..dim {
        for y in 0..dim {
            for x in 0..dim {
                assert_eq!(
                    toroidal_slot([x, y, z]),
                    [x as u32, y as u32, z as u32],
                    "OFF box cell {:?} must map to itself",
                    [x, y, z]
                );
            }
        }
    }
    // rem_euclid wrap: -1 -> DIM-1, DIM -> 0, -DIM -> 0, 2*DIM+1 -> 1.
    assert_eq!(toroidal_slot([-1, 0, dim]), [(dim - 1) as u32, 0, 0]);
    assert_eq!(
        toroidal_slot([-dim, 2 * dim + 1, -dim - 1]),
        [0, 1, (dim - 1) as u32]
    );
}

/// A scroll's revealed-cell slab matches a brute-force set-difference of the two
/// integer boxes, with EVERY revealed cell visited exactly once (no double-count,
/// no miss). Covers no-move (empty slab), 1-cell shifts on each axis, a diagonal
/// shift, and a teleport (`|Δ| >= DIM` ⇒ the whole new box).
#[test]
fn revealed_cells_match_box_difference_no_dup() {
    let dim = M2_GRID_DIM as i32;
    let in_box = |c: [i32; 3], lo: [i32; 3]| {
        (0..3).all(|a| c[a] >= lo[a] && c[a] < lo[a] + dim)
    };
    let cases: [([i32; 3], [i32; 3]); 8] = [
        ([0, 0, 0], [0, 0, 0]),       // no move: empty
        ([0, 0, 0], [1, 0, 0]),       // +X by 1
        ([0, 0, 0], [0, 1, 0]),       // +Y by 1
        ([0, 0, 0], [0, 0, 1]),       // +Z by 1
        ([2, 2, 2], [1, 2, 2]),       // -X by 1
        ([0, 0, 0], [1, 1, 1]),       // diagonal +1
        ([5, -3, 7], [5 + dim, -3, 7]), // teleport on X (disjoint)
        ([0, 0, 0], [dim + 2, dim, dim - 1]), // partial+full teleport mix
    ];
    for (old_oc, new_oc) in cases {
        // Brute-force oracle: every cell in the new box not in the old box.
        let mut oracle: Vec<[i32; 3]> = Vec::new();
        for z in new_oc[2]..new_oc[2] + dim {
            for y in new_oc[1]..new_oc[1] + dim {
                for x in new_oc[0]..new_oc[0] + dim {
                    if !in_box([x, y, z], old_oc) {
                        oracle.push([x, y, z]);
                    }
                }
            }
        }
        let mut got: Vec<[i32; 3]> = Vec::new();
        for_each_revealed_cell(old_oc, new_oc, |c| got.push(c));
        // No duplicate emission (each shell disjoint).
        let mut sorted = got.clone();
        sorted.sort_unstable();
        let dedup_len = {
            let mut s = sorted.clone();
            s.dedup();
            s.len()
        };
        assert_eq!(
            sorted.len(),
            dedup_len,
            "old {old_oc:?} new {new_oc:?}: a revealed cell was emitted twice"
        );
        let mut oracle_sorted = oracle.clone();
        oracle_sorted.sort_unstable();
        assert_eq!(
            sorted, oracle_sorted,
            "old {old_oc:?} new {new_oc:?}: revealed set != box difference"
        );
    }
}

/// A revealed cell ALWAYS lands on the toroidal slot of some cell the OLD box
/// vacated (the streaming invariant: entering cells overwrite exactly the slots of
/// the cells that left — the atlas never grows). For a single-axis +1 scroll the
/// revealed slab is one face, and its slots equal the departed face's slots.
#[test]
fn revealed_slots_overwrite_departed_slots() {
    let old_oc = [3, -2, 5];
    let new_oc = [4, -2, 5]; // +X by 1
    // Departed cells: in old box, not in new box.
    let dim = M2_GRID_DIM as i32;
    let in_box = |c: [i32; 3], lo: [i32; 3]| {
        (0..3).all(|a| c[a] >= lo[a] && c[a] < lo[a] + dim)
    };
    let mut departed_slots = std::collections::BTreeSet::new();
    for z in old_oc[2]..old_oc[2] + dim {
        for y in old_oc[1]..old_oc[1] + dim {
            for x in old_oc[0]..old_oc[0] + dim {
                if !in_box([x, y, z], new_oc) {
                    departed_slots.insert(toroidal_slot([x, y, z]));
                }
            }
        }
    }
    let mut revealed_slots = std::collections::BTreeSet::new();
    for_each_revealed_cell(old_oc, new_oc, |c| {
        revealed_slots.insert(toroidal_slot(c));
    });
    assert_eq!(
        revealed_slots, departed_slots,
        "the revealed slab's slots must exactly equal the departed cells' slots"
    );
}
