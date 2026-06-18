//! O1 — width-only AVX2 SoA kernels for the two hottest per-substep owning-path
//! computations: the inertia refresh (`R · I⁻¹_local · Rᵀ`) and the
//! gravity/position/quaternion integrate loop.
//!
//! # Determinism is the load-bearing constraint
//!
//! Each AVX2 lane `k` performs the EXACT same sequence of IEEE round-to-nearest
//! `f32` operations the scalar kernel performs on body `k`, so the SIMD output is
//! **bit-identical** to the scalar output (`f32::to_bits()` equality — the
//! `simd_o1` differential proptest is the gate). This is achievable because AVX2
//! `vmulps` / `vaddps` / `vsubps` / `vdivps` / `vsqrtps` are the same IEEE
//! round-to-nearest operation as scalar `mulss` / `addss` / `subss` / `divss` /
//! `sqrtss` for the same operands — AS LONG AS:
//!
//! - **NO FMA** (`_mm256_fmadd_ps`): a fused multiply-add rounds ONCE; the scalar
//!   `a*b + c` rounds TWICE. The two differ by a single ULP on FMA-capable CPUs
//!   and would diverge per target. Every `a*b + c` here is a SEPARATE `_mm256_mul_ps`
//!   then `_mm256_add_ps`, mirroring the scalar two-rounding sequence exactly. (The
//!   crate also carries no `target-feature=+fma` and no `mul_add` call, so the
//!   compiler cannot contract the explicit `mul`+`add` either.)
//! - **NO `rsqrtps` / `rcpps`** (`_mm256_rsqrt_ps` / `_mm256_rcp_ps`): the
//!   approximate reciprocal/reciprocal-sqrt instructions return DIFFERENT bits on
//!   Intel vs AMD. The normalize uses exact `_mm256_sqrt_ps` then `_mm256_div_ps`,
//!   mirroring the scalar `len_sq.sqrt().recip()` → `x * inv_len`. Note the scalar
//!   `recip()` is `1.0 / len` (an exact `divss`), and `x * inv_len` is a `mulss`;
//!   to reproduce the SAME two roundings the SIMD computes `inv_len = 1.0 / sqrt`
//!   (exact `_mm256_div_ps`) then multiplies — NOT a single `x / len` divide.
//! - **The op ORDER matches** the scalar source line-for-line (the matrix product
//!   evaluates `(R · I) · Rᵀ` left-to-right; `from_quat` builds each element in the
//!   documented order; `integrate` builds `ω̂`, one Hamilton product, scale, add,
//!   normalize). There is no horizontal reduction in either kernel (every output
//!   element is a fixed `mul`/`add` tree, never a `reduce_*`), so lane-reduction
//!   order is a non-issue here.
//!
//! These kernels are PER-BODY INDEPENDENT — no contact-order change — so they
//! produce the same bits as scalar and do NOT change `solver_is_deterministic`.
//! This is NOT a value-changing step (that is O5's colored solve).
//!
//! # Safety
//!
//! The AVX2 kernels are gated `cfg(all(target_arch = "x86_64",
//! target_feature = "avx2"))` + `#[target_feature(enable = "avx2")]` (the house
//! `bitset_intersects_avx2` pattern): a non-AVX2 host cannot link the intrinsic
//! path, so the compile-time gate IS the runtime guarantee. Every load/store is
//! the unaligned variant (`_mm256_loadu_ps` / `_mm256_storeu_ps`), so the SoA
//! scratch needs no over-alignment. Each `unsafe` block documents its bounds
//! invariant inline.
//!
//! # Dispatch + the 0%-gate
//!
//! [`refresh_inertia`], [`apply_gravity`], and [`position_integrate`] are the
//! public entry points the solver calls. They run the AVX2 kernel ONLY when the
//! compile-time gate is satisfied AND the runtime `simd` flag is set; otherwise
//! (flag off, or a non-AVX2 build, or under Miri) they run the scalar kernel,
//! which is byte-identical to the shipped `refresh_inertia` / integrate loop —
//! the campaign 0%-gate. The scalar kernel is also the differential-test oracle.

use crate::components::BodyType;
use crate::math::{Mat3, Vec3};
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
use crate::math::Quat;
use crate::resources::BodyState;

use super::contact::BodyEffective;

/// AVX2 batch width (8 `f32` lanes per `__m256`).
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
const LANES: usize = 8;

/// Refreshes each dynamic body's world inverse inertia `R · I⁻¹_local · Rᵀ` from
/// its local tensor + current orientation, dispatching to the AVX2 batch kernel
/// when `simd` is set on an AVX2 build, else the scalar oracle.
///
/// Static / `inv_mass == 0` bodies keep their current `inv_inertia` (the scalar
/// kernel's `if eff.inv_mass != 0.0` guard); the SIMD kernel reproduces this by
/// blending the rotated tensor back only where `inv_mass != 0` (so a static lane
/// is byte-untouched). The result is bit-identical to [`refresh_inertia_scalar`]
/// regardless of which path runs.
#[inline]
pub fn refresh_inertia(
    bodies_eff: &mut [BodyEffective],
    snapshot: &[BodyState],
    simd: bool,
) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        if simd {
            // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees
            //   the executing CPU supports every AVX2 intrinsic the kernel uses;
            //   the kernel documents its per-load bounds invariants.
            unsafe {
                refresh_inertia_avx2(bodies_eff, snapshot);
            }
            return;
        }
    }
    // Flag off / non-AVX2 build / Miri: the byte-identical scalar oracle.
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    let _ = simd;
    refresh_inertia_scalar(bodies_eff, snapshot);
}

/// Scalar reference (the bit-oracle): `I⁻¹_world = R · I⁻¹_local · Rᵀ` for every
/// dynamic body, byte-identical to the shipped
/// `SoftStepSolver::refresh_inertia`.
///
/// `#[inline]` so it folds into the dispatcher when the AVX2 path is off.
#[inline]
pub fn refresh_inertia_scalar(bodies_eff: &mut [BodyEffective], snapshot: &[BodyState]) {
    for (eff, snap) in bodies_eff.iter_mut().zip(snapshot.iter()) {
        if eff.inv_mass != 0.0 {
            let r = Mat3::from_quat(snap.rotation);
            eff.inv_inertia = r * snap.inv_inertia_local * r.transpose();
        }
    }
}

/// Applies the per-substep gravity sub-pass (`v += g·h` for every DYNAMIC body,
/// the solver's pass (1)) — the velocity-only half of the integrate, dispatching
/// to the AVX2 batch kernel when `simd` is set on an AVX2 build, else the scalar
/// oracle.
///
/// Split from [`position_integrate`] because the solver runs `warm_start_apply` +
/// `solve_velocities` BETWEEN gravity and the position advance (they mutate
/// velocity), so fusing the two would reorder the control flow. Each half is
/// independently bit-identical to its scalar oracle. A static/kinematic lane is
/// byte-untouched (the `Dynamic && inv_mass != 0` gate).
#[inline]
pub fn apply_gravity(bodies_eff: &mut [BodyEffective], snapshot: &[BodyState], gravity: Vec3, h: f32, simd: bool) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        if simd {
            // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees
            //   AVX2 is present on the executing CPU; the kernel documents its
            //   per-load bounds invariants.
            unsafe {
                apply_gravity_avx2(bodies_eff, snapshot, gravity, h);
            }
            return;
        }
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    let _ = simd;
    apply_gravity_scalar(bodies_eff, snapshot, gravity, h);
}

/// Scalar reference (the bit-oracle) for the gravity sub-pass, byte-identical to
/// the solver's pass (1).
#[inline]
pub fn apply_gravity_scalar(
    bodies_eff: &mut [BodyEffective],
    snapshot: &[BodyState],
    gravity: Vec3,
    h: f32,
) {
    for (eff, snap) in bodies_eff.iter_mut().zip(snapshot.iter()) {
        if snap.body_type == BodyType::Dynamic && eff.inv_mass != 0.0 {
            eff.linear_velocity = eff.linear_velocity + gravity * h;
        }
    }
}

/// Advances the per-substep position + quaternion integrate (`pos += v·h`,
/// `rot = rot.integrate(ω, h)` for every DYNAMIC body — the solver's pass (5)),
/// dispatching to the AVX2 batch kernel when `simd` is set on an AVX2 build, else
/// the scalar oracle.
///
/// Reads the (post-solve) `linear_velocity` / `angular_velocity` from
/// `bodies_eff` and advances `position` / `rotation` in place in `snapshot`. A
/// static/kinematic lane is byte-untouched. Bit-identical to
/// [`position_integrate_scalar`].
#[inline]
pub fn position_integrate(
    bodies_eff: &[BodyEffective],
    snapshot: &mut [BodyState],
    h: f32,
    simd: bool,
) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        if simd {
            // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees
            //   AVX2 is present on the executing CPU; the kernel documents its
            //   per-load bounds invariants.
            unsafe {
                position_integrate_avx2(bodies_eff, snapshot, h);
            }
            return;
        }
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    let _ = simd;
    position_integrate_scalar(bodies_eff, snapshot, h);
}

/// Scalar reference (the bit-oracle) for the position + quaternion integrate
/// sub-pass, byte-identical to the solver's pass (5).
#[inline]
pub fn position_integrate_scalar(
    bodies_eff: &[BodyEffective],
    snapshot: &mut [BodyState],
    h: f32,
) {
    for (eff, snap) in bodies_eff.iter().zip(snapshot.iter_mut()) {
        if snap.body_type == BodyType::Dynamic && eff.inv_mass != 0.0 {
            snap.position = snap.position + eff.linear_velocity * h;
            snap.rotation = snap.rotation.integrate(eff.angular_velocity, h);
        }
    }
}

// ── AVX2 kernels ─────────────────────────────────────────────────────────────

/// AVX2 batched `refresh_inertia` — 8 bodies per iteration, bit-identical to
/// [`refresh_inertia_scalar`].
///
/// Gathers 8 bodies' `rotation` (Quat) + `inv_inertia_local` (Mat3) into SoA
/// lanes, builds the rotation matrix + the similarity transform 8-wide with the
/// EXACT scalar op sequence (no FMA), then scatters the world `inv_inertia` back —
/// blending only the `inv_mass != 0` lanes so a static lane is byte-untouched.
///
/// # Safety
///
/// The caller must guarantee AVX2 is available (the `cfg` + `target_feature` gate
/// enforces this — a non-AVX2 host cannot link). Every load/store is bounds-bound
/// to `[base, base + 8)` with `base + 8 <= n` (or a scalar tail), and uses the
/// unaligned variants so the SoA scratch needs no over-alignment.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn refresh_inertia_avx2(bodies_eff: &mut [BodyEffective], snapshot: &[BodyState]) {
    use core::arch::x86_64::{
        __m256, _mm256_blendv_ps, _mm256_cmp_ps, _mm256_loadu_ps, _mm256_set1_ps,
        _mm256_storeu_ps, _CMP_NEQ_OQ,
    };

    let n = bodies_eff.len().min(snapshot.len());
    let full = n / LANES * LANES;

    // SoA staging buffers (stack arrays, no heap): gather → compute → scatter.
    let mut qx = [0.0f32; LANES];
    let mut qy = [0.0f32; LANES];
    let mut qz = [0.0f32; LANES];
    let mut qw = [0.0f32; LANES];
    // Local inverse inertia rows (row-major Mat3): 9 SoA columns.
    let mut l00 = [0.0f32; LANES];
    let mut l01 = [0.0f32; LANES];
    let mut l02 = [0.0f32; LANES];
    let mut l10 = [0.0f32; LANES];
    let mut l11 = [0.0f32; LANES];
    let mut l12 = [0.0f32; LANES];
    let mut l20 = [0.0f32; LANES];
    let mut l21 = [0.0f32; LANES];
    let mut l22 = [0.0f32; LANES];
    let mut inv_mass = [0.0f32; LANES];

    let mut base = 0usize;
    while base < full {
        // ── Gather 8 bodies into SoA lanes ──────────────────────────────────
        for lane in 0..LANES {
            let snap = &snapshot[base + lane];
            qx[lane] = snap.rotation.x;
            qy[lane] = snap.rotation.y;
            qz[lane] = snap.rotation.z;
            qw[lane] = snap.rotation.w;
            let li = &snap.inv_inertia_local.rows;
            l00[lane] = li[0].x;
            l01[lane] = li[0].y;
            l02[lane] = li[0].z;
            l10[lane] = li[1].x;
            l11[lane] = li[1].y;
            l12[lane] = li[1].z;
            l20[lane] = li[2].x;
            l21[lane] = li[2].y;
            l22[lane] = li[2].z;
            inv_mass[lane] = bodies_eff[base + lane].inv_mass;
        }

        // SAFETY: each load reads exactly 8 contiguous `f32` from a `[f32; 8]`
        //   stack buffer — fully in bounds; unaligned loads accept any alignment.
        let world = unsafe {
            let x = _mm256_loadu_ps(qx.as_ptr());
            let y = _mm256_loadu_ps(qy.as_ptr());
            let z = _mm256_loadu_ps(qz.as_ptr());
            let w = _mm256_loadu_ps(qw.as_ptr());

            let il = [
                _mm256_loadu_ps(l00.as_ptr()),
                _mm256_loadu_ps(l01.as_ptr()),
                _mm256_loadu_ps(l02.as_ptr()),
                _mm256_loadu_ps(l10.as_ptr()),
                _mm256_loadu_ps(l11.as_ptr()),
                _mm256_loadu_ps(l12.as_ptr()),
                _mm256_loadu_ps(l20.as_ptr()),
                _mm256_loadu_ps(l21.as_ptr()),
                _mm256_loadu_ps(l22.as_ptr()),
            ];

            // R = from_quat(q): mirror Mat3::from_quat exactly (no FMA).
            let r = quat_to_mat3_x8(x, y, z, w);
            // (R · I_local) · Rᵀ, left-to-right, mirroring `r * I * r.transpose()`.
            mat3_mul_x8(mat3_mul_x8(r, il), mat3_transpose_x8(r))
        };

        // ── Scatter: blend the rotated tensor only into `inv_mass != 0` lanes ─
        // SAFETY: `world` columns are valid; the blend reads the freshly-loaded
        //   current tensor of each lane and the mask (inv_mass != 0), then stores
        //   8 `f32` into stack buffers we read back below — all in bounds.
        let mut out = [[0.0f32; LANES]; 9];
        unsafe {
            let imm = _mm256_loadu_ps(inv_mass.as_ptr());
            let zero = _mm256_set1_ps(0.0);
            // mask lane = all-ones iff inv_mass != 0 (ordered, NaN→false; inv_mass
            // is never NaN here, so this matches the scalar `!= 0.0` exactly).
            let mask = _mm256_cmp_ps::<_CMP_NEQ_OQ>(imm, zero);
            // Current (unchanged) world tensor of each lane, for the static blend.
            let cur = gather_current_inertia_x8(bodies_eff, base);
            for c in 0..9 {
                let blended: __m256 = _mm256_blendv_ps(cur[c], world[c], mask);
                _mm256_storeu_ps(out[c].as_mut_ptr(), blended);
            }
        }

        // ── Write back into the AoS BodyEffective slots ─────────────────────
        for lane in 0..LANES {
            let m = &mut bodies_eff[base + lane].inv_inertia.rows;
            m[0].x = out[0][lane];
            m[0].y = out[1][lane];
            m[0].z = out[2][lane];
            m[1].x = out[3][lane];
            m[1].y = out[4][lane];
            m[1].z = out[5][lane];
            m[2].x = out[6][lane];
            m[2].y = out[7][lane];
            m[2].z = out[8][lane];
        }

        base += LANES;
    }

    // Scalar tail: the last `n % 8` bodies fall through to the scalar oracle —
    // bit-identical, so the tail is covered by the differential test.
    refresh_inertia_scalar(&mut bodies_eff[full..n], &snapshot[full..n]);
}

/// Loads the current (pre-refresh) world inverse inertia of 8 bodies into 9 SoA
/// `__m256` columns — used to blend a static lane back unchanged.
///
/// # Safety
///
/// `base + 8 <= bodies_eff.len()`. Reads only `inv_inertia` of those 8 bodies.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
unsafe fn gather_current_inertia_x8(
    bodies_eff: &[BodyEffective],
    base: usize,
) -> [core::arch::x86_64::__m256; 9] {
    use core::arch::x86_64::_mm256_loadu_ps;
    let mut cols = [[0.0f32; LANES]; 9];
    for lane in 0..LANES {
        let m = &bodies_eff[base + lane].inv_inertia.rows;
        cols[0][lane] = m[0].x;
        cols[1][lane] = m[0].y;
        cols[2][lane] = m[0].z;
        cols[3][lane] = m[1].x;
        cols[4][lane] = m[1].y;
        cols[5][lane] = m[1].z;
        cols[6][lane] = m[2].x;
        cols[7][lane] = m[2].y;
        cols[8][lane] = m[2].z;
    }
    // SAFETY: each load reads 8 `f32` from an in-bounds `[f32; 8]` stack buffer.
    unsafe {
        [
            _mm256_loadu_ps(cols[0].as_ptr()),
            _mm256_loadu_ps(cols[1].as_ptr()),
            _mm256_loadu_ps(cols[2].as_ptr()),
            _mm256_loadu_ps(cols[3].as_ptr()),
            _mm256_loadu_ps(cols[4].as_ptr()),
            _mm256_loadu_ps(cols[5].as_ptr()),
            _mm256_loadu_ps(cols[6].as_ptr()),
            _mm256_loadu_ps(cols[7].as_ptr()),
            _mm256_loadu_ps(cols[8].as_ptr()),
        ]
    }
}

/// 8-wide `Mat3::from_quat` — builds the row-major rotation matrix of 8 unit
/// quaternions, element by element in the EXACT scalar order (no FMA).
///
/// Mirrors `Mat3::from_quat`: `xx = x*x; … ; xy = x*y; … ; wx = w*x; …` then each
/// element `1 - 2*(yy+zz)`, `2*(xy - wz)`, … built with separate `mul`/`add`/`sub`.
///
/// # Safety
///
/// AVX2-gated (the `cfg` + `target_feature`). No memory access — pure register ops.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn quat_to_mat3_x8(
    x: core::arch::x86_64::__m256,
    y: core::arch::x86_64::__m256,
    z: core::arch::x86_64::__m256,
    w: core::arch::x86_64::__m256,
) -> [core::arch::x86_64::__m256; 9] {
    use core::arch::x86_64::{_mm256_add_ps, _mm256_mul_ps, _mm256_set1_ps, _mm256_sub_ps};

    // Pure register arithmetic, no memory access — the AVX2 intrinsics are safe to
    // call inside this `#[target_feature(enable = "avx2")]` function.
    let one = _mm256_set1_ps(1.0);
    let two = _mm256_set1_ps(2.0);

    let xx = _mm256_mul_ps(x, x);
    let yy = _mm256_mul_ps(y, y);
    let zz = _mm256_mul_ps(z, z);
    let xy = _mm256_mul_ps(x, y);
    let xz = _mm256_mul_ps(x, z);
    let yz = _mm256_mul_ps(y, z);
    let wx = _mm256_mul_ps(w, x);
    let wy = _mm256_mul_ps(w, y);
    let wz = _mm256_mul_ps(w, z);

    // Row 0: [1 - 2*(yy+zz), 2*(xy - wz), 2*(xz + wy)]
    let r00 = _mm256_sub_ps(one, _mm256_mul_ps(two, _mm256_add_ps(yy, zz)));
    let r01 = _mm256_mul_ps(two, _mm256_sub_ps(xy, wz));
    let r02 = _mm256_mul_ps(two, _mm256_add_ps(xz, wy));
    // Row 1: [2*(xy + wz), 1 - 2*(xx+zz), 2*(yz - wx)]
    let r10 = _mm256_mul_ps(two, _mm256_add_ps(xy, wz));
    let r11 = _mm256_sub_ps(one, _mm256_mul_ps(two, _mm256_add_ps(xx, zz)));
    let r12 = _mm256_mul_ps(two, _mm256_sub_ps(yz, wx));
    // Row 2: [2*(xz - wy), 2*(yz + wx), 1 - 2*(xx+yy)]
    let r20 = _mm256_mul_ps(two, _mm256_sub_ps(xz, wy));
    let r21 = _mm256_mul_ps(two, _mm256_add_ps(yz, wx));
    let r22 = _mm256_sub_ps(one, _mm256_mul_ps(two, _mm256_add_ps(xx, yy)));

    [r00, r01, r02, r10, r11, r12, r20, r21, r22]
}

/// 8-wide row-major `Mat3` product `a · b`, bit-identical to the scalar `Mul`.
///
/// Mirrors the scalar `row` closure exactly: `out[i][j] = a[i][0]*b[0][j] +
/// a[i][1]*b[1][j] + a[i][2]*b[2][j]`, built as three separate `mul` then two
/// `add` in that left-to-right order (no FMA).
///
/// # Safety
///
/// AVX2-gated. No memory access — pure register ops on 9-element column arrays.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn mat3_mul_x8(
    a: [core::arch::x86_64::__m256; 9],
    b: [core::arch::x86_64::__m256; 9],
) -> [core::arch::x86_64::__m256; 9] {
    use core::arch::x86_64::{__m256, _mm256_add_ps, _mm256_mul_ps};

    // Pure register arithmetic — the intrinsics are safe inside this
    // `#[target_feature(enable = "avx2")]` function.
    // out[i*3 + j] = a[i*3+0]*b[0*3+j] + a[i*3+1]*b[1*3+j] + a[i*3+2]*b[2*3+j].
    let elem = |i: usize, j: usize| -> __m256 {
        let p0 = _mm256_mul_ps(a[i * 3], b[j]);
        let p1 = _mm256_mul_ps(a[i * 3 + 1], b[3 + j]);
        let p2 = _mm256_mul_ps(a[i * 3 + 2], b[6 + j]);
        // (p0 + p1) + p2 — left-to-right, matching `x*.. + y*.. + z*..`.
        _mm256_add_ps(_mm256_add_ps(p0, p1), p2)
    };
    [
        elem(0, 0),
        elem(0, 1),
        elem(0, 2),
        elem(1, 0),
        elem(1, 1),
        elem(1, 2),
        elem(2, 0),
        elem(2, 1),
        elem(2, 2),
    ]
}

/// 8-wide `Mat3::transpose` — swaps off-diagonal columns (a pure shuffle of the
/// 9-element column array, no arithmetic, so trivially bit-identical). No
/// intrinsics, no memory access — a plain register reorder.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn mat3_transpose_x8(m: [core::arch::x86_64::__m256; 9]) -> [core::arch::x86_64::__m256; 9] {
    // rows[i].col(j) → rows[j].col(i): [00,10,20, 01,11,21, 02,12,22].
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

/// AVX2 batched gravity sub-pass — 8 bodies per iteration, bit-identical to
/// [`apply_gravity_scalar`].
///
/// # Safety
///
/// The caller must guarantee AVX2 is available (the `cfg` + `target_feature`
/// gate). Every load/store is bounds-bound to `[base, base + 8)` with
/// `base + 8 <= n` (or a scalar tail). A static/kinematic lane is masked out so
/// it is byte-untouched (matching the scalar `Dynamic && inv_mass != 0` gate).
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn apply_gravity_avx2(
    bodies_eff: &mut [BodyEffective],
    snapshot: &[BodyState],
    gravity: Vec3,
    h: f32,
) {
    use core::arch::x86_64::{
        _mm256_add_ps, _mm256_blendv_ps, _mm256_cmp_ps, _mm256_loadu_ps, _mm256_mul_ps,
        _mm256_set1_ps, _mm256_storeu_ps, _CMP_NEQ_OQ,
    };

    let n = bodies_eff.len().min(snapshot.len());
    let full = n / LANES * LANES;

    let mut vx = [0.0f32; LANES];
    let mut vy = [0.0f32; LANES];
    let mut vz = [0.0f32; LANES];
    let mut active = [0.0f32; LANES];

    let mut base = 0usize;
    while base < full {
        for lane in 0..LANES {
            let eff = &bodies_eff[base + lane];
            let snap = &snapshot[base + lane];
            vx[lane] = eff.linear_velocity.x;
            vy[lane] = eff.linear_velocity.y;
            vz[lane] = eff.linear_velocity.z;
            active[lane] = if snap.body_type == BodyType::Dynamic && eff.inv_mass != 0.0 {
                1.0
            } else {
                0.0
            };
        }

        // SAFETY: every load reads 8 `f32` from an in-bounds `[f32; 8]` buffer.
        unsafe {
            let hv = _mm256_set1_ps(h);
            let gx = _mm256_set1_ps(gravity.x);
            let gy = _mm256_set1_ps(gravity.y);
            let gz = _mm256_set1_ps(gravity.z);
            let zero = _mm256_set1_ps(0.0);

            let v0 = _mm256_loadu_ps(vx.as_ptr());
            let v1 = _mm256_loadu_ps(vy.as_ptr());
            let v2 = _mm256_loadu_ps(vz.as_ptr());
            let act_mask = _mm256_cmp_ps::<_CMP_NEQ_OQ>(_mm256_loadu_ps(active.as_ptr()), zero);

            // v += g·h (separate mul then add — NO FMA).
            let nv0 = _mm256_add_ps(v0, _mm256_mul_ps(gx, hv));
            let nv1 = _mm256_add_ps(v1, _mm256_mul_ps(gy, hv));
            let nv2 = _mm256_add_ps(v2, _mm256_mul_ps(gz, hv));

            // A non-active lane keeps its original velocity (byte-untouched).
            _mm256_storeu_ps(vx.as_mut_ptr(), _mm256_blendv_ps(v0, nv0, act_mask));
            _mm256_storeu_ps(vy.as_mut_ptr(), _mm256_blendv_ps(v1, nv1, act_mask));
            _mm256_storeu_ps(vz.as_mut_ptr(), _mm256_blendv_ps(v2, nv2, act_mask));
        }

        for lane in 0..LANES {
            bodies_eff[base + lane].linear_velocity = Vec3::new(vx[lane], vy[lane], vz[lane]);
        }
        base += LANES;
    }

    // Scalar tail (bit-identical), covered by the differential test.
    apply_gravity_scalar(&mut bodies_eff[full..n], &snapshot[full..n], gravity, h);
}

/// AVX2 batched position + quaternion integrate sub-pass — 8 bodies per
/// iteration, bit-identical to [`position_integrate_scalar`].
///
/// # Safety
///
/// The caller must guarantee AVX2 is available (the `cfg` + `target_feature`
/// gate). Every load/store is bounds-bound to `[base, base + 8)` with
/// `base + 8 <= n` (or a scalar tail). A static/kinematic lane is masked out so
/// its `position` / `rotation` is byte-untouched.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn position_integrate_avx2(bodies_eff: &[BodyEffective], snapshot: &mut [BodyState], h: f32) {
    let n = bodies_eff.len().min(snapshot.len());
    let full = n / LANES * LANES;

    let mut base = 0usize;
    while base < full {
        // SAFETY: the lane block `[base, base + 8)` is fully in bounds
        //   (`base + 8 <= full <= n <= len` of both slices).
        unsafe {
            position_integrate_block_x8(bodies_eff, snapshot, base, h);
        }
        base += LANES;
    }

    // Scalar tail (bit-identical), covered by the differential test.
    position_integrate_scalar(&bodies_eff[full..n], &mut snapshot[full..n], h);
}

/// Integrates one 8-body block's position + orientation in SoA, bit-identical to
/// the scalar per-body path.
///
/// # Safety
///
/// `base + 8 <= bodies_eff.len()` and `base + 8 <= snapshot.len()`. Reads
/// `linear_velocity` / `angular_velocity` / `inv_mass` (eff) and writes
/// `position` / `rotation` of those 8 bodies (snapshot).
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
unsafe fn position_integrate_block_x8(
    bodies_eff: &[BodyEffective],
    snapshot: &mut [BodyState],
    base: usize,
    h: f32,
) {
    use core::arch::x86_64::{
        __m256, _mm256_add_ps, _mm256_blendv_ps, _mm256_cmp_ps, _mm256_div_ps, _mm256_loadu_ps,
        _mm256_mul_ps, _mm256_set1_ps, _mm256_sqrt_ps, _mm256_storeu_ps, _mm256_sub_ps, _CMP_LE_OQ,
        _CMP_NEQ_OQ,
    };

    // ── Gather 8 bodies into SoA stack buffers ──────────────────────────────
    let mut vx = [0.0f32; LANES];
    let mut vy = [0.0f32; LANES];
    let mut vz = [0.0f32; LANES];
    let mut wx = [0.0f32; LANES];
    let mut wy = [0.0f32; LANES];
    let mut wz = [0.0f32; LANES];
    let mut px = [0.0f32; LANES];
    let mut py = [0.0f32; LANES];
    let mut pz = [0.0f32; LANES];
    let mut qx = [0.0f32; LANES];
    let mut qy = [0.0f32; LANES];
    let mut qz = [0.0f32; LANES];
    let mut qw = [0.0f32; LANES];
    let mut active = [0.0f32; LANES];

    for lane in 0..LANES {
        let eff = &bodies_eff[base + lane];
        let snap = &snapshot[base + lane];
        vx[lane] = eff.linear_velocity.x;
        vy[lane] = eff.linear_velocity.y;
        vz[lane] = eff.linear_velocity.z;
        wx[lane] = eff.angular_velocity.x;
        wy[lane] = eff.angular_velocity.y;
        wz[lane] = eff.angular_velocity.z;
        px[lane] = snap.position.x;
        py[lane] = snap.position.y;
        pz[lane] = snap.position.z;
        qx[lane] = snap.rotation.x;
        qy[lane] = snap.rotation.y;
        qz[lane] = snap.rotation.z;
        qw[lane] = snap.rotation.w;
        active[lane] = if snap.body_type == BodyType::Dynamic && eff.inv_mass != 0.0 {
            1.0
        } else {
            0.0
        };
    }

    // SAFETY: every load reads 8 `f32` from an in-bounds `[f32; 8]` stack buffer.
    unsafe {
        let half_dt = _mm256_set1_ps(0.5 * h);
        let hv = _mm256_set1_ps(h);
        let zero = _mm256_set1_ps(0.0);
        let one = _mm256_set1_ps(1.0);
        let min_pos = _mm256_set1_ps(f32::MIN_POSITIVE);

        let v = [
            _mm256_loadu_ps(vx.as_ptr()),
            _mm256_loadu_ps(vy.as_ptr()),
            _mm256_loadu_ps(vz.as_ptr()),
        ];
        let av = [
            _mm256_loadu_ps(wx.as_ptr()),
            _mm256_loadu_ps(wy.as_ptr()),
            _mm256_loadu_ps(wz.as_ptr()),
        ];
        let p = [
            _mm256_loadu_ps(px.as_ptr()),
            _mm256_loadu_ps(py.as_ptr()),
            _mm256_loadu_ps(pz.as_ptr()),
        ];
        let q = [
            _mm256_loadu_ps(qx.as_ptr()),
            _mm256_loadu_ps(qy.as_ptr()),
            _mm256_loadu_ps(qz.as_ptr()),
            _mm256_loadu_ps(qw.as_ptr()),
        ];
        let act_mask = _mm256_cmp_ps::<_CMP_NEQ_OQ>(_mm256_loadu_ps(active.as_ptr()), zero);

        // (5a) position: pos += v·h (separate mul then add — NO FMA).
        let p_new = [
            _mm256_add_ps(p[0], _mm256_mul_ps(v[0], hv)),
            _mm256_add_ps(p[1], _mm256_mul_ps(v[1], hv)),
            _mm256_add_ps(p[2], _mm256_mul_ps(v[2], hv)),
        ];

        // (5b) quaternion integrate: q_next = normalize(q + ½·(ω̂ · q)·dt).
        // Build ω̂ = (ω.x, ω.y, ω.z, 0) and the Hamilton product ω̂ · q EXACTLY as
        // `Quat::mul` (delta = omega.mul(self), where omega is lhs, q is rhs):
        //   dx = ow*qx + ox*qw + oy*qz - oz*qy   (ow = 0)
        //   dy = ow*qy - ox*qz + oy*qw + oz*qx
        //   dz = ow*qz + ox*qy - oy*qx + oz*qw
        //   dw = ow*qw - ox*qx - oy*qy - oz*qz
        // with ow ≡ 0, so the `ow*..` terms are `0.0 * q.. = 0` — KEPT EXPLICIT so
        // the op count/order matches the scalar `Quat::mul` bit-for-bit.
        let (ox, oy, oz, ow) = (av[0], av[1], av[2], zero);
        let (rqx, rqy, rqz, rqw) = (q[0], q[1], q[2], q[3]);
        // dx = ow*qx + ox*qw + oy*qz - oz*qy
        let dx = _mm256_sub_ps(
            _mm256_add_ps(
                _mm256_add_ps(_mm256_mul_ps(ow, rqx), _mm256_mul_ps(ox, rqw)),
                _mm256_mul_ps(oy, rqz),
            ),
            _mm256_mul_ps(oz, rqy),
        );
        // dy = ow*qy - ox*qz + oy*qw + oz*qx
        let dy = _mm256_add_ps(
            _mm256_add_ps(
                _mm256_sub_ps(_mm256_mul_ps(ow, rqy), _mm256_mul_ps(ox, rqz)),
                _mm256_mul_ps(oy, rqw),
            ),
            _mm256_mul_ps(oz, rqx),
        );
        // dz = ow*qz + ox*qy - oy*qx + oz*qw
        let dz = _mm256_add_ps(
            _mm256_sub_ps(
                _mm256_add_ps(_mm256_mul_ps(ow, rqz), _mm256_mul_ps(ox, rqy)),
                _mm256_mul_ps(oy, rqx),
            ),
            _mm256_mul_ps(oz, rqw),
        );
        // dw = ow*qw - ox*qx - oy*qy - oz*qz
        let dw = _mm256_sub_ps(
            _mm256_sub_ps(
                _mm256_sub_ps(_mm256_mul_ps(ow, rqw), _mm256_mul_ps(ox, rqx)),
                _mm256_mul_ps(oy, rqy),
            ),
            _mm256_mul_ps(oz, rqz),
        );
        // q + delta * half_dt (separate mul then add — matches `self.x + delta.x*half_dt`).
        let nx = _mm256_add_ps(rqx, _mm256_mul_ps(dx, half_dt));
        let ny = _mm256_add_ps(rqy, _mm256_mul_ps(dy, half_dt));
        let nz = _mm256_add_ps(rqz, _mm256_mul_ps(dz, half_dt));
        let nw = _mm256_add_ps(rqw, _mm256_mul_ps(dw, half_dt));

        // normalize: len_sq = x*x + y*y + z*z + w*w (left-to-right), then
        // inv_len = 1.0 / sqrt(len_sq) (exact div), q *= inv_len. Mirrors
        // `Quat::normalize`: `len_sq.sqrt().recip()` is `1.0 / sqrt`, then `*`.
        // The zero-guard returns IDENTITY when len_sq <= MIN_POSITIVE.
        let len_sq = _mm256_add_ps(
            _mm256_add_ps(
                _mm256_add_ps(_mm256_mul_ps(nx, nx), _mm256_mul_ps(ny, ny)),
                _mm256_mul_ps(nz, nz),
            ),
            _mm256_mul_ps(nw, nw),
        );
        let inv_len = _mm256_div_ps(one, _mm256_sqrt_ps(len_sq));
        let norm = [
            _mm256_mul_ps(nx, inv_len),
            _mm256_mul_ps(ny, inv_len),
            _mm256_mul_ps(nz, inv_len),
            _mm256_mul_ps(nw, inv_len),
        ];
        // Degenerate guard (len_sq <= MIN_POSITIVE → IDENTITY (0,0,0,1)).
        let degen = _mm256_cmp_ps::<_CMP_LE_OQ>(len_sq, min_pos);
        let q_next = [
            _mm256_blendv_ps(norm[0], zero, degen),
            _mm256_blendv_ps(norm[1], zero, degen),
            _mm256_blendv_ps(norm[2], zero, degen),
            _mm256_blendv_ps(norm[3], one, degen),
        ];

        // Mask: a non-active lane keeps its ORIGINAL position/rotation.
        let store = |dst: &mut [f32; LANES], reg: __m256| _mm256_storeu_ps(dst.as_mut_ptr(), reg);
        store(&mut px, _mm256_blendv_ps(p[0], p_new[0], act_mask));
        store(&mut py, _mm256_blendv_ps(p[1], p_new[1], act_mask));
        store(&mut pz, _mm256_blendv_ps(p[2], p_new[2], act_mask));
        store(&mut qx, _mm256_blendv_ps(q[0], q_next[0], act_mask));
        store(&mut qy, _mm256_blendv_ps(q[1], q_next[1], act_mask));
        store(&mut qz, _mm256_blendv_ps(q[2], q_next[2], act_mask));
        store(&mut qw, _mm256_blendv_ps(q[3], q_next[3], act_mask));
    }

    for lane in 0..LANES {
        let snap = &mut snapshot[base + lane];
        snap.position = Vec3::new(px[lane], py[lane], pz[lane]);
        snap.rotation = Quat::new(qx[lane], qy[lane], qz[lane], qw[lane]);
    }
}

#[cfg(test)]
mod tests {
    //! The O1 differential bit-identity gate: each SIMD dispatcher
    //! (`refresh_inertia` / `apply_gravity` / `position_integrate` with `simd =
    //! true`) must produce output `f32`-BIT-IDENTICAL to its scalar oracle, over
    //! random bodies INCLUDING partial-lane tails (1..16 bodies). Mirrors the
    //! `bitset_intersects_avx2` differential pattern: the scalar path is the
    //! reference, the SIMD path is asserted equal bit-for-bit. Under a non-AVX2
    //! build the dispatcher IS the scalar oracle, so the test still holds (it then
    //! proves the scalar dispatch is the oracle); under AVX2 it proves the kernel
    //! is a pure speed path.

    use super::*;
    use crate::components::{BodyType, ColliderShape};
    use crate::math::Quat;
    use crate::resources::BodyState;

    /// A tiny deterministic splitmix64-based RNG — no external dep, reproducible.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// A finite `f32` in `[-range, range]`.
        fn f32_in(&mut self, range: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0,1)
            (u * 2.0 - 1.0) * range
        }
    }

    /// Builds a random, mostly-dynamic body (eff + snapshot) with a unit-ish
    /// orientation, a non-trivial local inertia tensor, and random velocities — a
    /// realistic input for both kernels.
    fn random_body(rng: &mut Rng) -> (BodyEffective, BodyState) {
        // Random orientation, normalized to a unit quaternion (the crate keeps
        // orientations unit; the kernels reproduce `Quat::normalize` exactly).
        let q = Quat::new(
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
        )
        .normalize();

        // Mostly dynamic; a minority static / kinematic to exercise the mask path.
        let r = rng.next_u64() % 10;
        let (body_type, inv_mass) = if r < 7 {
            (BodyType::Dynamic, 0.2 + (rng.f32_in(1.0).abs()))
        } else if r < 9 {
            (BodyType::Static, 0.0)
        } else {
            // Kinematic with a non-zero inv_mass would still be skipped by the
            // `Dynamic && inv_mass != 0` gate — exercise that the kernel masks it.
            (BodyType::Kinematic, 0.5)
        };

        let radius = 0.3 + rng.f32_in(1.0).abs();
        // Local inverse inertia of a solid sphere: `inv_mass · 5 / (2·r²)`
        // (isotropic). `Mat3::ZERO` for a static body (the refresh guard then
        // leaves a static lane untouched). A non-isotropic tensor would test more
        // of the matrix product; an isotropic diagonal already exercises every
        // `from_quat` element and the full `R · I · Rᵀ` product, so this is
        // sufficient and matches the production gather.
        let local = if inv_mass == 0.0 {
            Mat3::ZERO
        } else {
            let inv = inv_mass * 5.0 / (2.0 * radius * radius);
            // Slightly anisotropic so the off-diagonal terms of R·I·Rᵀ are
            // non-zero (a fuller exercise of the matrix product than a scalar
            // multiple of identity).
            Mat3::from_diagonal(Vec3::new(inv, inv * 1.3, inv * 0.7))
        };

        let snap = BodyState {
            inv_inertia: Mat3::ZERO, // overwritten by refresh; arbitrary start
            inv_inertia_local: local,
            position: Vec3::new(rng.f32_in(50.0), rng.f32_in(50.0), rng.f32_in(50.0)),
            linear_velocity: Vec3::new(rng.f32_in(20.0), rng.f32_in(20.0), rng.f32_in(20.0)),
            angular_velocity: Vec3::new(rng.f32_in(10.0), rng.f32_in(10.0), rng.f32_in(10.0)),
            rotation: q,
            inv_mass,
            restitution: 0.0,
            friction: 0.5,
            body_type,
            shape: ColliderShape::Sphere { radius },
        };
        let eff = BodyEffective {
            inv_mass,
            // A random (non-ZERO) starting world tensor, so a STATIC lane's
            // byte-untouched guarantee is non-vacuously checked (refresh must not
            // overwrite it; a wrong blend would change these bits).
            inv_inertia: Mat3::from_rows(
                Vec3::new(rng.f32_in(1.0), rng.f32_in(1.0), rng.f32_in(1.0)),
                Vec3::new(rng.f32_in(1.0), rng.f32_in(1.0), rng.f32_in(1.0)),
                Vec3::new(rng.f32_in(1.0), rng.f32_in(1.0), rng.f32_in(1.0)),
            ),
            linear_velocity: snap.linear_velocity,
            angular_velocity: snap.angular_velocity,
        };
        (eff, snap)
    }

    fn bits3(v: Vec3) -> [u32; 3] {
        [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
    }

    fn bits4(q: Quat) -> [u32; 4] {
        [q.x.to_bits(), q.y.to_bits(), q.z.to_bits(), q.w.to_bits()]
    }

    fn bits_mat3(m: Mat3) -> [[u32; 3]; 3] {
        [bits3(m.rows[0]), bits3(m.rows[1]), bits3(m.rows[2])]
    }

    /// `refresh_inertia` SIMD == scalar, bit-exact, over counts 1..16 (tails).
    #[test]
    fn refresh_inertia_simd_bits_match_scalar() {
        let mut rng = Rng::new(0x0010_0001_dead_beef);
        for count in 1..=16usize {
            for trial in 0..32 {
                let mut eff = Vec::with_capacity(count);
                let mut snap = Vec::with_capacity(count);
                for _ in 0..count {
                    let (e, s) = random_body(&mut rng);
                    eff.push(e);
                    snap.push(s);
                }
                let mut eff_scalar = eff.clone();
                let mut eff_simd = eff.clone();

                refresh_inertia_scalar(&mut eff_scalar, &snap);
                refresh_inertia(&mut eff_simd, &snap, true);

                for i in 0..count {
                    assert_eq!(
                        bits_mat3(eff_scalar[i].inv_inertia),
                        bits_mat3(eff_simd[i].inv_inertia),
                        "refresh_inertia bit mismatch at count={count} trial={trial} body={i}"
                    );
                }
            }
        }
    }

    /// `apply_gravity` SIMD == scalar, bit-exact, over counts 1..16 (tails).
    #[test]
    fn apply_gravity_simd_bits_match_scalar() {
        let mut rng = Rng::new(0x0020_0002_dead_beef);
        for count in 1..=16usize {
            for trial in 0..32 {
                let mut eff = Vec::with_capacity(count);
                let mut snap = Vec::with_capacity(count);
                for _ in 0..count {
                    let (e, s) = random_body(&mut rng);
                    eff.push(e);
                    snap.push(s);
                }
                let gravity = Vec3::new(rng.f32_in(20.0), rng.f32_in(20.0), rng.f32_in(20.0));
                let h = 0.001 + rng.f32_in(1.0).abs() * 0.01;

                let mut eff_scalar = eff.clone();
                let mut eff_simd = eff.clone();
                apply_gravity_scalar(&mut eff_scalar, &snap, gravity, h);
                apply_gravity(&mut eff_simd, &snap, gravity, h, true);

                for i in 0..count {
                    assert_eq!(
                        bits3(eff_scalar[i].linear_velocity),
                        bits3(eff_simd[i].linear_velocity),
                        "apply_gravity bit mismatch at count={count} trial={trial} body={i}"
                    );
                }
            }
        }
    }

    /// `position_integrate` SIMD == scalar, bit-exact, over counts 1..16 (tails).
    #[test]
    fn position_integrate_simd_bits_match_scalar() {
        let mut rng = Rng::new(0x0030_0003_dead_beef);
        for count in 1..=16usize {
            for trial in 0..32 {
                let mut eff = Vec::with_capacity(count);
                let mut snap = Vec::with_capacity(count);
                for _ in 0..count {
                    let (e, s) = random_body(&mut rng);
                    eff.push(e);
                    snap.push(s);
                }
                let h = 0.001 + rng.f32_in(1.0).abs() * 0.01;

                let mut snap_scalar = snap.clone();
                let mut snap_simd = snap.clone();
                position_integrate_scalar(&eff, &mut snap_scalar, h);
                position_integrate(&eff, &mut snap_simd, h, true);

                for i in 0..count {
                    assert_eq!(
                        bits3(snap_scalar[i].position),
                        bits3(snap_simd[i].position),
                        "position bit mismatch at count={count} trial={trial} body={i}"
                    );
                    assert_eq!(
                        bits4(snap_scalar[i].rotation),
                        bits4(snap_simd[i].rotation),
                        "rotation bit mismatch at count={count} trial={trial} body={i}"
                    );
                }
            }
        }
    }

    /// A degenerate (near-zero) quaternion lane normalizes to IDENTITY under BOTH
    /// paths bit-identically (the zero-guard mask), and a zero angular velocity is
    /// NaN-free (the divide is `1.0 / sqrt(len_sq)` with `len_sq >= 1` for a unit
    /// quat, but the guard covers a hand-built zero).
    #[test]
    fn degenerate_quat_lane_matches_scalar() {
        let mut eff = Vec::new();
        let mut snap = Vec::new();
        // One zero-quat body padded with normal bodies to force a partial lane.
        for k in 0..9usize {
            let q = if k == 0 {
                Quat::new(0.0, 0.0, 0.0, 0.0)
            } else {
                Quat::new(0.0, 0.0, 0.0, 1.0)
            };
            snap.push(BodyState {
                inv_inertia: Mat3::ZERO,
                inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)),
                position: Vec3::ZERO,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                rotation: q,
                inv_mass: 1.0,
                restitution: 0.0,
                friction: 0.5,
                body_type: BodyType::Dynamic,
                shape: ColliderShape::Sphere { radius: 1.0 },
            });
            eff.push(BodyEffective {
                inv_mass: 1.0,
                inv_inertia: Mat3::ZERO,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            });
        }
        let mut s_scalar = snap.clone();
        let mut s_simd = snap.clone();
        position_integrate_scalar(&eff, &mut s_scalar, 0.01);
        position_integrate(&eff, &mut s_simd, 0.01, true);
        for i in 0..snap.len() {
            assert_eq!(bits4(s_scalar[i].rotation), bits4(s_simd[i].rotation));
            assert!(s_simd[i].rotation.w.is_finite(), "no NaN in normalize");
        }
    }

    /// A dynamic Vec3 dynamic body whose only purpose is to fill a lane next to an
    /// adversarial one (a "filler" so a single hand-crafted body lands inside a
    /// real 8-wide block AND a partial tail, exercising both code paths).
    fn filler_body() -> (BodyEffective, BodyState) {
        let snap = BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: Mat3::from_diagonal(Vec3::new(1.0, 1.3, 0.7)),
            position: Vec3::new(1.0, 2.0, 3.0),
            linear_velocity: Vec3::new(0.5, -0.5, 0.25),
            angular_velocity: Vec3::new(0.1, -0.2, 0.3),
            rotation: Quat::new(0.0, 0.0, 0.0, 1.0),
            inv_mass: 1.0,
            restitution: 0.0,
            friction: 0.5,
            body_type: BodyType::Dynamic,
            shape: ColliderShape::Sphere { radius: 0.5 },
        };
        let eff = BodyEffective {
            inv_mass: 1.0,
            inv_inertia: Mat3::ZERO,
            linear_velocity: snap.linear_velocity,
            angular_velocity: snap.angular_velocity,
        };
        (eff, snap)
    }

    /// Builds a dynamic body with an EXACT quaternion + velocity set — for the
    /// hand-crafted adversarial inputs (denormal / -0.0 / near-MIN_POSITIVE len_sq).
    fn exact_body(
        rotation: Quat,
        linear_velocity: Vec3,
        angular_velocity: Vec3,
        inv_mass: f32,
        local: Mat3,
        start_world: Mat3,
    ) -> (BodyEffective, BodyState) {
        let snap = BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: local,
            position: Vec3::new(0.0, 0.0, 0.0),
            linear_velocity,
            angular_velocity,
            rotation,
            inv_mass,
            restitution: 0.0,
            friction: 0.5,
            body_type: BodyType::Dynamic,
            shape: ColliderShape::Sphere { radius: 0.5 },
        };
        let eff = BodyEffective {
            inv_mass,
            inv_inertia: start_world,
            linear_velocity,
            angular_velocity,
        };
        (eff, snap)
    }

    /// Reviewer O1 coverage: ADVERSARIAL differential inputs prove the
    /// `simd_bits == scalar_bits` claim non-vacuously on the corner cases the
    /// random corpus is unlikely to hit — a DENORMAL `f32` component, a NEGATIVE
    /// ZERO, and a body whose post-integrate `len_sq` lands JUST ABOVE
    /// `f32::MIN_POSITIVE` from a non-zero start (the normalize zero-guard boundary).
    ///
    /// Each adversarial body is embedded at several positions inside a 9-body
    /// scene (a full 8-lane block + a 1-body tail) AND tested in a count-1 scene,
    /// so the kernel's full-block path, mask blend, and scalar tail all see it.
    #[test]
    fn adversarial_inputs_simd_bits_match_scalar() {
        // The smallest positive denormal and the largest denormal (just below the
        // smallest normal). `-0.0` is the negative zero. These are fed as raw
        // quaternion / velocity / inertia components.
        let denorm_min = f32::from_bits(0x0000_0001); // ~1.4e-45
        let denorm_max = f32::from_bits(0x007F_FFFF); // ~1.18e-38 (largest subnormal)
        let neg_zero = -0.0f32;

        // A quaternion whose len_sq lands JUST ABOVE f32::MIN_POSITIVE so the
        // normalize takes the DIVIDE branch (not the IDENTITY guard) from a
        // non-zero start, and a paired one just BELOW so it takes the guard — both
        // must match scalar bit-for-bit either way. With zero angular velocity the
        // post-integrate quat == the input quat, so len_sq(q) is the boundary.
        let just_above = {
            // Want x*x just above MIN_POSITIVE (~1.1754944e-38). sqrt of that:
            let x = (f32::MIN_POSITIVE * 1.5).sqrt();
            Quat::new(x, 0.0, 0.0, 0.0)
        };
        let just_below = {
            let x = (f32::MIN_POSITIVE * 0.5).sqrt();
            Quat::new(x, 0.0, 0.0, 0.0)
        };

        let local = Mat3::from_diagonal(Vec3::new(2.0, 1.5, 0.5));
        let start_world = Mat3::from_rows(
            Vec3::new(0.11, 0.22, 0.33),
            Vec3::new(0.44, 0.55, 0.66),
            Vec3::new(0.77, 0.88, 0.99),
        );

        // The adversarial bodies. Each is a (rotation, lin_v, ang_v) triple.
        let adversaries: Vec<(BodyEffective, BodyState)> = vec![
            // Denormal in a velocity component (gravity add + position advance).
            exact_body(
                Quat::IDENTITY,
                Vec3::new(denorm_min, denorm_max, neg_zero),
                Vec3::new(denorm_max, neg_zero, denorm_min),
                1.0,
                local,
                start_world,
            ),
            // Negative-zero quaternion components (the normalize + Hamilton product
            // must treat -0.0 identically in both paths).
            exact_body(
                Quat::new(neg_zero, neg_zero, neg_zero, 1.0),
                Vec3::new(neg_zero, 1.0, neg_zero),
                Vec3::new(neg_zero, neg_zero, neg_zero),
                1.0,
                local,
                start_world,
            ),
            // Denormal quaternion + denormal local inertia (refresh + integrate).
            exact_body(
                Quat::new(denorm_max, neg_zero, denorm_min, 1.0).normalize(),
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(0.5, denorm_max, neg_zero),
                1.0,
                Mat3::from_diagonal(Vec3::new(denorm_max, denorm_min, 1.0)),
                start_world,
            ),
            // len_sq JUST ABOVE MIN_POSITIVE, zero spin → divide branch on a tiny
            // non-zero quat.
            exact_body(
                just_above,
                Vec3::ZERO,
                Vec3::ZERO,
                1.0,
                local,
                start_world,
            ),
            // len_sq JUST BELOW MIN_POSITIVE, zero spin → IDENTITY guard branch.
            exact_body(
                just_below,
                Vec3::ZERO,
                Vec3::ZERO,
                1.0,
                local,
                start_world,
            ),
        ];

        let h = 1.0 / 240.0;
        let gravity = Vec3::new(0.0, -9.81, 0.0);

        // For each adversary, build scenes that put it (a) alone (count 1), and
        // (b) at lane 0, lane 3, and lane 8 of a 9-body scene (full block + tail).
        for (adv_eff, adv_snap) in &adversaries {
            let mut scenes: Vec<(Vec<BodyEffective>, Vec<BodyState>)> = Vec::new();

            // Count-1 scene.
            scenes.push((vec![*adv_eff], vec![*adv_snap]));

            // 9-body scenes with the adversary at lanes 0, 3, 8.
            for slot in [0usize, 3, 8] {
                let mut eff = Vec::with_capacity(9);
                let mut snap = Vec::with_capacity(9);
                for lane in 0..9 {
                    if lane == slot {
                        eff.push(*adv_eff);
                        snap.push(*adv_snap);
                    } else {
                        let (e, s) = filler_body();
                        eff.push(e);
                        snap.push(s);
                    }
                }
                scenes.push((eff, snap));
            }

            for (eff, snap) in &scenes {
                // refresh_inertia
                let mut e_scalar = eff.clone();
                let mut e_simd = eff.clone();
                refresh_inertia_scalar(&mut e_scalar, snap);
                refresh_inertia(&mut e_simd, snap, true);
                for i in 0..eff.len() {
                    assert_eq!(
                        bits_mat3(e_scalar[i].inv_inertia),
                        bits_mat3(e_simd[i].inv_inertia),
                        "adversarial refresh_inertia mismatch at body {i}"
                    );
                }

                // apply_gravity
                let mut g_scalar = eff.clone();
                let mut g_simd = eff.clone();
                apply_gravity_scalar(&mut g_scalar, snap, gravity, h);
                apply_gravity(&mut g_simd, snap, gravity, h, true);
                for i in 0..eff.len() {
                    assert_eq!(
                        bits3(g_scalar[i].linear_velocity),
                        bits3(g_simd[i].linear_velocity),
                        "adversarial apply_gravity mismatch at body {i}"
                    );
                }

                // position_integrate
                let mut s_scalar = snap.clone();
                let mut s_simd = snap.clone();
                position_integrate_scalar(eff, &mut s_scalar, h);
                position_integrate(eff, &mut s_simd, h, true);
                for i in 0..snap.len() {
                    assert_eq!(
                        bits3(s_scalar[i].position),
                        bits3(s_simd[i].position),
                        "adversarial position mismatch at body {i}"
                    );
                    assert_eq!(
                        bits4(s_scalar[i].rotation),
                        bits4(s_simd[i].rotation),
                        "adversarial rotation mismatch at body {i}"
                    );
                }
            }
        }
    }
}
