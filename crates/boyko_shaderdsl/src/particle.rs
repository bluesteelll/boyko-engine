//! The **P0 particle leaves** (`docs/PARTICLES-PLAN.md` rung E's leaf table) — the six generic
//! `C: Cf` bodies the GPU particle system's five shaders splice their math out of.
//!
//! Rung E landed the eDSL nodes these leaves need ([`crate::particle_facets`] is the per-node
//! facet probe set); this module is the first REAL consumer. Every leaf here is authored ONCE
//! and instantiated twice:
//!
//! - `<EvalCf>` — the CPU oracle (real `u32`/`f32` ops), which is what the P0 gate's
//!   determinism harness (plan gate #13) compares a device readback against;
//! - `<EmitCf>` — the HLSL recorder, printed by `crate::emit::emit_hlsl_particle_*` and spliced
//!   between the `// === GENERATED <name> BEGIN/END ===` sentinels of the five files
//!   `boyko_shaderdsl/src/bin/emit_particles.rs` owns.
//!
//! # The family, and what it costs
//!
//! Every leaf is on the [`Cf`] family (plan rung E's table). **None** contains trig and **none**
//! contains a divide, and both are load-bearing rather than incidental:
//!
//! - **No trig.** `sin`/`cos` exist on this axis ([`Cf::sin`] / [`Cf::cos`], rung E4) and are
//!   deliberately unused. The rotation state is a stored `(cos, sin)` PAIR advanced by a complex
//!   multiply against a host-precomputed `(cos ω·timestep, sin ω·timestep)` (plan D6/M7), and the
//!   cone direction comes out of a square→disc→cap composition that needs only `sqrt`.
//! - **No divide.** `OpFDiv` carries 2.5 ULP, and the house rule is that a division is never part
//!   of a bit-exact contract (plan M7). Every reciprocal here is a literal constant multiply, so
//!   each leaf's Eval instantiation is a bit-exact oracle for its Emit one modulo FMA contraction.
//!   Plan gate #14 asserts ZERO `OpFDiv` in [`particle_rot_advance_body`]'s generated span.
//!
//! # `f32`-exact vs. close
//!
//! [`particle_rng_body`] is INTEGER — bit-exact by construction on both backends. The float
//! leaves are exact up to the compiler's freedom to contract `a * b + c` into an FMA; that is the
//! standing carve-out every float leaf in this crate carries, and it is why the P0 determinism
//! harness compares a SINGLE step exactly and multi-step runs as a multiset (plan gate #13).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

// ---- Packing constants (mirrored by the generated HLSL as bare literals) ------------------

/// The low-16 mask (`0xFFFF`), spelled in decimal like every other `uint` literal the printer
/// emits.
const U16_MASK: u32 = 65535;

/// The high-half shift of a packed 16-bit pair.
const HI16_SHIFT: u32 = 16;

/// The snorm16 DECODE bias, as a `uint`. Adding it to the stored two's-complement 16-bit value
/// and masking to 16 bits yields `n + 32768` in `[1, 65535]` — an unsigned lane no `int` cast is
/// needed to read. See [`snorm16_lane`] for the algebra.
const SNORM16_BIAS_U: u32 = 32768;

/// The snorm16 DECODE bias, as a `float` — subtracted after the unsigned widen to recover the
/// signed `n`.
const SNORM16_BIAS_F: f32 = 32768.0;

/// The snorm16 ENCODE bias: `v * 32767 + 65536.5`. The `+0.5` is the round-to-nearest term and
/// the `+65536` keeps the pre-cast value POSITIVE for every `v` in `[-1, 1]`, so the HLSL
/// `(uint)` truncation is a `floor` (an `OpConvertFToU` of a negative value is undefined) and the
/// subsequent `& 65535u` recovers `n`'s two's-complement 16 bits exactly. Choosing `+65536`
/// rather than `+32768` also removes the only `uint` SUBTRACT the encode would otherwise need.
const SNORM16_ENCODE_BIAS: f32 = 65536.5;

/// The snorm16 scale — `32767`, so `+1.0` maps to `32767` and `-1.0` to `-32767`.
const SNORM16_SCALE: f32 = 32767.0;

/// The snorm16 inverse scale, `1 / 32767`, as a literal MULTIPLY (never a divide — see the module
/// doc).
const SNORM16_INV: f32 = 1.0 / 32767.0;

// ---- PCG32 constants (Jarzynski & Olano, *Hash Functions for GPU Rendering*) ---------------

/// The LCG multiplier of the 32-bit PCG state advance.
const PCG_MUL: u32 = 747_796_405;

/// The LCG increment of the 32-bit PCG state advance.
const PCG_INC: u32 = 2_891_336_453;

/// The output permutation's multiplier.
const PCG_XSH_MUL: u32 = 277_803_737;

/// The xorshift's ROTATION-SELECT shift: `state >> 28u` yields a 4-bit selector in `[0, 15]`.
const PCG_ROT_SELECT_SHIFT: u32 = 28;

/// The xorshift's rotation-select BIAS: the dynamic shift amount is `(state >> 28u) + 4u`, hence
/// bounded by **19** — the property that keeps [`Cf::shr_u`]'s Eval arm (a plain host `>>`, which
/// panics in debug at an amount `>= 32`) agreeing with the GPU over the WHOLE reachable domain.
/// Rung E's shift-amount note states the rule; this constant is what makes it hold here.
const PCG_ROT_SELECT_BIAS: u32 = 4;

/// The final output shift of the permutation.
const PCG_OUT_SHIFT: u32 = 22;

/// The `[0, 1)` conversion scale — `2^-24`, applied to the TOP 24 bits of a hash word
/// (`h >> 8u`). A multiply by an exact power of two, so the conversion is exact and needs no
/// divide.
const RNG_TO_UNIT: f32 = 1.0 / 16_777_216.0;

/// The shift that keeps a hash word's TOP 24 bits — the 24 a binary32 significand holds exactly.
const RNG_UNIT_SHIFT: u32 = 8;

// ---- Shared snorm16 helpers (single-sourced across the two rotation leaves) ----------------

/// Decodes ONE snorm16 lane out of an already-selected 32-bit word `lane` (the low lane is the
/// packed word itself; the high lane is `packed >> 16u`), emitting the biasing `uint` temp under
/// `bias_name`.
///
/// ```text
/// uint <bias_name> = <lane> + 32768u;
/// // ... ((float)(<bias_name> & 65535u) - 32768.0) * 3.0518509e-5
/// ```
///
/// The bias-then-mask is what avoids a signed 16-bit sign extension: with `n` the stored value in
/// `[-32767, 32767]`, `(n + 32768) & 65535 == n + 32768` without wrapping, so the unsigned widen
/// followed by `- 32768.0` recovers `n` exactly. Adding the bias to the WHOLE word is safe
/// because the carry can only propagate into bit 16, and the mask discards it — the high lane is
/// taken from the ORIGINAL word, never from the biased one.
#[inline]
fn snorm16_lane<C: Cf>(bias_name: &'static str, lane: C::Uint) -> C::Scalar {
    let biased = C::temp_uint(bias_name, C::uadd(lane, C::uint_lit(SNORM16_BIAS_U)));
    C::float_from_uint(C::and_u(biased, C::uint_lit(U16_MASK)))
        .sub(C::Scalar::lit(SNORM16_BIAS_F))
        .mul(C::Scalar::lit(SNORM16_INV))
}

/// Re-quantizes `v` to the low 16 bits of a `uint` as a snorm16, emitting the pre-cast `float`
/// temp under `q_name`.
///
/// ```text
/// float <q_name> = min(max(<v>, -1.0), 1.0) * 32767.0 + 65536.5;
/// // ... ((uint)<q_name> & 65535u)
/// ```
///
/// The clamp is NOT cosmetic: plan M7/K1 drops the renormalization, so the advanced `(cos, sin)`
/// pair random-walks slightly off the unit circle and a lane may exceed `1.0` by ~1e-3 over a
/// long life. Un-clamped, that would push the pre-cast value out of the `floor`-safe window and
/// the mask would silently wrap the sign.
#[inline]
fn snorm16_encode<C: Cf>(q_name: &'static str, v: C::Scalar) -> C::Uint {
    let q = C::temp_float(
        q_name,
        v.max(C::Scalar::lit(-1.0))
            .min(C::Scalar::lit(1.0))
            .mul(C::Scalar::lit(SNORM16_SCALE))
            .add(C::Scalar::lit(SNORM16_ENCODE_BIAS)),
    );
    C::and_u(C::float_to_uint(q), C::uint_lit(U16_MASK))
}

/// Converts a 32-bit hash word to a `float` in `[0, 1)` by taking its TOP 24 bits and scaling by
/// `2^-24` — exact, and divide-free (see the module doc).
///
/// The TOP bits rather than the bottom: an LCG's low bits are its weakest, and `h >> 8u` keeps
/// exactly the 24 a binary32 significand can hold without a rounding step.
#[inline]
fn rng_unit<C: Cf>(h: C::Uint) -> C::Scalar {
    C::float_from_uint(C::shr_u(h, C::uint_lit(RNG_UNIT_SHIFT))).mul(C::Scalar::lit(RNG_TO_UNIT))
}

// ---- The six P0 leaves ---------------------------------------------------------------------

/// **`particle_integrate`** (plan rung E leaf table, row 1) — ONE substep of the explicit-Euler
/// integrator, over the caller's `inout` state.
///
/// ```text
/// vel = (vel + gravity * dt) * damping;
/// pos = pos + vel * dt;
/// life = life - dt;
/// ```
///
/// `damping` is a HOST constant (`exp2(-drag * timestep)`, plan D6) — which is what deletes
/// `exp2` from the device entirely, and is only expressible because `ParticleClock` guarantees a
/// CONSTANT `timestep`. The velocity is damped BEFORE the position advance (semi-implicit in the
/// damping term, explicit in gravity), matching the plan's D3 pseudocode statement for statement.
///
/// `pos`/`vel`/`life` are SUPPRESSED-DECL locals ([`Cf::decl_param_vec3`] / [`Cf::decl_param`]):
/// the generated span assigns them by name and declares nothing, because the HLSL wrapper spells
/// them as `inout` parameters.
#[inline]
pub fn particle_integrate_body<C: Cf>(
    pos: &C::Vec3Var,
    vel: &C::Vec3Var,
    life: &C::Var,
    gravity: C::Vec3f,
    damping: C::Scalar,
    dt: C::Scalar,
) -> Flow {
    // vel = (vel + gravity * dt) * damping;
    C::set_var_vec3(
        vel,
        C::vec3_mul_scalar(
            C::vec3_add(C::get_var_vec3(vel), C::vec3_mul_scalar(gravity, dt)),
            damping,
        ),
    );
    // pos = pos + vel * dt;   — reads the JUST-DAMPED velocity.
    C::set_var_vec3(
        pos,
        C::vec3_add(
            C::get_var_vec3(pos),
            C::vec3_mul_scalar(C::get_var_vec3(vel), dt),
        ),
    );
    // life = life - dt;
    C::set_var(life, C::get_var(life).sub(dt));
    Flow::Continue(())
}

/// **`particle_rng`** (plan rung E leaf table, row 2) — the 32-bit PCG hash: one LCG state
/// advance followed by the xorshift-multiply-xorshift output permutation.
///
/// ```text
/// uint s = state * 747796405u + 2891336453u;
/// uint shift = (s >> 28u) + 4u;
/// uint word = ((s >> shift) ^ s) * 277803737u;
/// return (word >> 22u) ^ word;
/// ```
///
/// INTEGER throughout, so it is **bit-exact by construction** on both backends — the one leaf in
/// this module whose oracle carries no FMA carve-out.
///
/// # The dynamic shift is bounded by 19
///
/// `s >> 28u` is a 4-bit selector in `[0, 15]`, so `shift` lies in `[4, 19]`. That matters
/// because [`Cf::shr_u`]'s Eval arm is the plain host `>>`, which PANICS in debug for an amount
/// `>= 32` where the GPU masks to the low 5 bits (rung E's shift-amount note). The bound is a
/// property of the constants, not an assumption about the input: EVERY `u32` state satisfies it.
///
/// # Chaining
///
/// The caller uses the returned word as the next state (`r1 = particle_rng(r0)`), the standard
/// hash-chain form: each call is one LCG step plus a permutation, so a chain of `n` calls has the
/// full period of the underlying LCG.
#[inline]
pub fn particle_rng_body<C: Cf>(state: C::Uint, ret_out: &C::RetCell) -> Flow {
    // uint s = state * 747796405u + 2891336453u;
    let s = C::temp_uint(
        "s",
        C::uadd(
            C::umul(state, C::uint_lit(PCG_MUL)),
            C::uint_lit(PCG_INC),
        ),
    );
    // uint shift = (s >> 28u) + 4u;  — the dynamic amount, bounded by 19 (see the doc above).
    let shift = C::temp_uint(
        "shift",
        C::uadd(
            C::shr_u(s, C::uint_lit(PCG_ROT_SELECT_SHIFT)),
            C::uint_lit(PCG_ROT_SELECT_BIAS),
        ),
    );
    // uint word = ((s >> shift) ^ s) * 277803737u;
    let word = C::temp_uint(
        "word",
        C::umul(
            C::uxor(C::shr_u(s, shift), s),
            C::uint_lit(PCG_XSH_MUL),
        ),
    );
    // return (word >> 22u) ^ word;
    C::ret(
        ret_out,
        C::uxor(C::shr_u(word, C::uint_lit(PCG_OUT_SHIFT)), word),
    )
}

/// **`particle_spawn_state`** (plan rung E leaf table, row 3) — the spawn VELOCITY (a cone
/// direction times a sampled speed) and the spawn LIFETIME, from four hash words.
///
/// ```text
/// float ux = (float)(r_dir_x >> 8u) * 5.9604645e-8;   // and uy / us / ul
/// float a = ux * 2.0 - 1.0;
/// float b = uy * 2.0 - 1.0;
/// float px = a * sqrt(1.0 - 0.5 * b * b);             // square -> unit disc
/// float py = b * sqrt(1.0 - 0.5 * a * a);
/// float cap = 2.0 - 2.0 * cone_cos;                   // the cap disc's R^2
/// float q = (px * px + py * py) * cap;
/// float sr = sqrt(max(1.0 - 0.25 * q, 0.0)) * sqrt(cap);   // disc -> spherical cap
/// float dz = 1.0 - 0.5 * q;
/// velocity = (basis_x * sx + basis_y * sy + basis_z * dz) * speed;
/// life = lerp(life_min, life_max, ul);
/// ```
///
/// # Why THIS trig-free composition
///
/// A cone sample is conventionally `(r·cosφ, r·sinφ, z)`, which needs trig. The plan's leaf table
/// pins this row **no trig, no divide** (rung E's E5 row is deleted, and a divide would drag
/// `OpFDiv`'s 2.5 ULP into the oracle), so the sample is composed of two `sqrt`-only maps:
///
/// 1. **Square → unit disc**, the elliptical grid mapping
///    `(a, b) ↦ (a·√(1 − b²/2), b·√(1 − a²/2))`. Bijective from `[-1,1]²` onto the closed unit
///    disc, continuous, and free of the rejection loop a polar method would need.
/// 2. **Disc → spherical cap**, the Lambert azimuthal equal-area lift. With `q = |c|²` over the
///    disc of radius `R = √(2(1 − cone_cos))`, the point `(s·c, 1 − q/2)` with `s = √(1 − q/4)`
///    has EXACTLY unit length — `s²q + (1 − q/2)² = q − q²/4 + 1 − q + q²/4 = 1`, an algebraic
///    identity, not a normalization — and its `z` sweeps `[cone_cos, 1]`. So the leaf emits a
///    unit direction with no `rsqrt` and no renormalization.
///
/// `cone_cos == 1` degenerates correctly (`cap == 0` ⇒ the direction is exactly `basis_z`), and
/// `cone_cos == -1` gives the full sphere. `cone_cos > 1` would put a negative under the `sqrt`;
/// the effect table is host-validated, and this leaf does not re-check it.
///
/// # Speed and lifetime
///
/// Both are `lerp`s over their own hash word, so an emitter's particles spread over
/// `[speed_min, speed_max]` / `[life_min, life_max]` without a second dispatch.
#[allow(clippy::too_many_arguments)]
// The spawn state IS this many independent inputs: three basis vectors, four scalar ranges, a
// cone aperture and four decorrelated hash words. Grouping them into a struct would either
// invent an eDSL aggregate type that no `Cf` backend has, or bind this leaf to one HLSL `struct`
// layout — both worse than the flat list the generated signature mirrors one-for-one.
#[inline]
pub fn particle_spawn_state_body<C: Cf>(
    basis_x: C::Vec3f,
    basis_y: C::Vec3f,
    basis_z: C::Vec3f,
    cone_cos: C::Scalar,
    speed_min: C::Scalar,
    speed_max: C::Scalar,
    life_min: C::Scalar,
    life_max: C::Scalar,
    r_dir_x: C::Uint,
    r_dir_y: C::Uint,
    r_speed: C::Uint,
    r_life: C::Uint,
    velocity_out: &C::OutVec3,
    life_out: &C::OutFloat,
) -> Flow {
    let ux = C::temp_float("ux", rng_unit::<C>(r_dir_x));
    let uy = C::temp_float("uy", rng_unit::<C>(r_dir_y));
    let us = C::temp_float("us", rng_unit::<C>(r_speed));
    let ul = C::temp_float("ul", rng_unit::<C>(r_life));

    // The unit square, centred: [0,1) -> [-1,1).
    let a = C::temp_float("a", ux.mul(C::Scalar::lit(2.0)).sub(C::Scalar::lit(1.0)));
    let b = C::temp_float("b", uy.mul(C::Scalar::lit(2.0)).sub(C::Scalar::lit(1.0)));

    // Step 1 — the elliptical grid mapping onto the unit disc.
    let px = C::temp_float(
        "px",
        a.mul(
            C::Scalar::lit(1.0)
                .sub(C::Scalar::lit(0.5).mul(b).mul(b))
                .sqrt(),
        ),
    );
    let py = C::temp_float(
        "py",
        b.mul(
            C::Scalar::lit(1.0)
                .sub(C::Scalar::lit(0.5).mul(a).mul(a))
                .sqrt(),
        ),
    );

    // Step 2 — the Lambert azimuthal equal-area lift onto the cap `z >= cone_cos`.
    let cap = C::temp_float(
        "cap",
        C::Scalar::lit(2.0).sub(C::Scalar::lit(2.0).mul(cone_cos)),
    );
    let q = C::temp_float("q", px.mul(px).add(py.mul(py)).mul(cap));
    // `max(·, 0.0)` is a ROUNDING guard, not a range guard. In exact arithmetic `q <= cap <= 4`
    // and `1 - q/4 >= 0`; in binary32 the disc mapping's `px^2 + py^2` can round to just above 1
    // at the square's corners, so at the FULL-SPHERE aperture (`cone_cos == -1`, `cap == 4`) `q`
    // reaches 4 + 5e-7 and the radicand goes negative — `sqrt` of which is NaN, and a NaN
    // direction is a particle at an undefined position for the rest of its life. MEASURED: the
    // unit-length sweep in `tests/particle_leaves.rs` reproduces it at
    // `(cone_cos, r) = (-1, (0xFFFFFFFF, 0xFFFFFFFF))`.
    let sr = C::temp_float(
        "sr",
        C::Scalar::lit(1.0)
            .sub(C::Scalar::lit(0.25).mul(q))
            .max(C::Scalar::lit(0.0))
            .sqrt()
            .mul(cap.sqrt()),
    );
    // The two in-plane components are materialized so the emitted `basis_* * s?` groups as
    // `basis * (sr * p)` rather than the left-associated `basis * sr * p` — a DIFFERENT rounding.
    let sx = C::temp_float("sx", sr.mul(px));
    let sy = C::temp_float("sy", sr.mul(py));
    let dz = C::temp_float(
        "dz",
        C::Scalar::lit(1.0).sub(C::Scalar::lit(0.5).mul(q)),
    );

    let speed = C::temp_float("speed", speed_min.lerp(speed_max, us));
    C::out_vec3_assign(
        velocity_out,
        C::vec3_mul_scalar(
            C::vec3_add(
                C::vec3_add(
                    C::vec3_mul_scalar(basis_x, sx),
                    C::vec3_mul_scalar(basis_y, sy),
                ),
                C::vec3_mul_scalar(basis_z, dz),
            ),
            speed,
        ),
    );
    C::out_float_assign(life_out, life_min.lerp(life_max, ul));
    Flow::Continue(())
}

/// **`particle_curve_eval`** (plan rung E leaf table, row 4) — a 4-key piecewise-linear ramp over
/// keys stored as two packed IEEE binary16 pairs, evaluated at `t` in `[0, 1]`.
///
/// ```text
/// float k0 = f16tof32(keys_lo & 65535u);   // and k1 / k2 / k3
/// float u = clamp(t, 0.0, 1.0) * 3.0;
/// float w0 = clamp(u, 0.0, 1.0);
/// float w1 = clamp(u - 1.0, 0.0, 1.0);
/// float w2 = clamp(u - 2.0, 0.0, 1.0);
/// float v0 = lerp(k0, k1, w0);
/// float v1 = lerp(v0, k2, w1);
/// return lerp(v1, k3, w2);
/// ```
///
/// # Why three cascaded `lerp`s and no `floor`
///
/// The obvious form — pick a segment with `floor(u)` and interpolate inside it — is unavailable
/// twice over: `FieldScalar` carries no `floor`, and a per-lane segment SELECT is a branch in the
/// hottest loop the sim has. The cascade is exactly equivalent and branch-free: at `u = 0/1/2/3`
/// the weight triple is `(0,0,0) / (1,0,0) / (1,1,0) / (1,1,1)`, which collapses the cascade onto
/// `k0 / k1 / k2 / k3`; and for `u` inside segment `i` the later weights are 0 (so those `lerp`s
/// are identities) and the earlier ones are 1 (so those `lerp`s have already delivered `k_i`).
///
/// # The keys are UNIFORMLY spaced
///
/// Keys sit at `t = 0, 1/3, 2/3, 1`. Non-uniform key times would need `(t − t_i) / (t_{i+1} − t_i)`
/// — a DIVIDE, which this row's plan entry forbids — so a non-uniform ramp requires the host to
/// precompute the inverse spans, exactly as it precomputes `damping` (plan D6). That is a host
/// change, not an eDSL one, and is not part of P0.
#[inline]
pub fn particle_curve_eval_body<C: Cf>(
    keys_lo: C::Uint,
    keys_hi: C::Uint,
    t: C::Scalar,
    ret_out: &C::RetCellF,
) -> Flow {
    let k0 = C::temp_float("k0", C::f16tof32(C::and_u(keys_lo, C::uint_lit(U16_MASK))));
    let k1 = C::temp_float("k1", C::f16tof32(C::shr_u(keys_lo, C::uint_lit(HI16_SHIFT))));
    let k2 = C::temp_float("k2", C::f16tof32(C::and_u(keys_hi, C::uint_lit(U16_MASK))));
    let k3 = C::temp_float("k3", C::f16tof32(C::shr_u(keys_hi, C::uint_lit(HI16_SHIFT))));

    let u = C::temp_float("u", t.clamp01().mul(C::Scalar::lit(3.0)));
    let w0 = C::temp_float("w0", u.clamp01());
    let w1 = C::temp_float("w1", u.sub(C::Scalar::lit(1.0)).clamp01());
    let w2 = C::temp_float("w2", u.sub(C::Scalar::lit(2.0)).clamp01());

    let v0 = C::temp_float("v0", k0.lerp(k1, w0));
    let v1 = C::temp_float("v1", v0.lerp(k2, w1));
    C::ret_f(ret_out, v1.lerp(k3, w2))
}

/// **`particle_billboard_corner`** (plan rung E leaf table, row 5) — the world position of one
/// billboard corner: the particle centre plus the camera basis scaled by the corner offset, the
/// size, and the particle's stored `(cos, sin)` rotation.
///
/// ```text
/// uint cb = rot_cs + 32768u;
/// uint sb = (rot_cs >> 16u) + 32768u;
/// float rc = ((float)(cb & 65535u) - 32768.0) * 3.0518509e-5;
/// float rs = ((float)(sb & 65535u) - 32768.0) * 3.0518509e-5;
/// float rx = (cx * rc - cy * rs) * size;
/// float ry = (cx * rs + cy * rc) * size;
/// world_pos = center + cam_right * rx + cam_up * ry;
/// ```
///
/// Pure multiply/add: the rotation arrives ALREADY as a `(cos, sin)` pair (plan M7), so the VS
/// spends no trig and performs no renormalization. The snorm16 decode is the same
/// [`snorm16_lane`] the sim's [`particle_rot_advance_body`] uses, so the two cannot drift.
///
/// `cx`/`cy` are the caller's corner offsets (the shipped quad uses `±0.5`), which keeps the leaf
/// independent of the vertex-id → corner convention the VS chooses.
#[allow(clippy::too_many_arguments)]
// A billboard corner is a centre, two camera basis vectors, a 2-D corner offset, a size and a
// packed rotation — eight independent inputs with no natural grouping that any `Cf` backend can
// express as one value.
#[inline]
pub fn particle_billboard_corner_body<C: Cf>(
    center: C::Vec3f,
    cam_right: C::Vec3f,
    cam_up: C::Vec3f,
    cx: C::Scalar,
    cy: C::Scalar,
    size: C::Scalar,
    rot_cs: C::Uint,
    world_out: &C::OutVec3,
) -> Flow {
    let rc = C::temp_float("rc", snorm16_lane::<C>("cb", rot_cs));
    let rs = C::temp_float(
        "rs",
        snorm16_lane::<C>("sb", C::shr_u(rot_cs, C::uint_lit(HI16_SHIFT))),
    );
    // The 2-D rotation of the corner offset, folded with the size scale.
    let rx = C::temp_float("rx", cx.mul(rc).sub(cy.mul(rs)).mul(size));
    let ry = C::temp_float("ry", cx.mul(rs).add(cy.mul(rc)).mul(size));
    C::out_vec3_assign(
        world_out,
        C::vec3_add(
            C::vec3_add(center, C::vec3_mul_scalar(cam_right, rx)),
            C::vec3_mul_scalar(cam_up, ry),
        ),
    );
    Flow::Continue(())
}

/// **`particle_rot_advance`** (plan rung E leaf table, row 6) — advances the packed `(cos, sin)`
/// rotation by ONE substep: a complex multiply against the host-precomputed
/// `(cos ω·timestep, sin ω·timestep)` pair, re-quantized to snorm16.
///
/// ```text
/// uint cb = rot_cs + 32768u;
/// uint sb = (rot_cs >> 16u) + 32768u;
/// float rc = ((float)(cb & 65535u) - 32768.0) * 3.0518509e-5;
/// float rs = ((float)(sb & 65535u) - 32768.0) * 3.0518509e-5;
/// float nc = rc * mul_cos - rs * mul_sin;
/// float ns = rc * mul_sin + rs * mul_cos;
/// float qc = min(max(nc, -1.0), 1.0) * 32767.0 + 65536.5;
/// float qs = min(max(ns, -1.0), 1.0) * 32767.0 + 65536.5;
/// return ((uint)qc & 65535u) | (((uint)qs & 65535u) << 16u);
/// ```
///
/// # NO renormalization, and no divide (plan M7 / K1)
///
/// The obvious `1/√(c² + s²)` rescale is deliberately absent. It would put either an `OpFDiv`
/// (2.5 ULP) or an `rsqrt` (2 ULP, approximate) inside this leaf's oracle, and the house rule is
/// that neither belongs in a bit-exact contract. Plan gate #14 asserts **zero `OpFDiv`** in this
/// leaf's generated span, so the absence is checked rather than asserted in prose.
///
/// What replaces it is a BOUND. The multiplier is stored as an **f32 pair** in `EffectParamsGpu`
/// (K1 — a snorm16-quantized multiplier's magnitude error is a per-effect CONSTANT and compounds
/// as `(1+δ)ⁿ`, reaching ~±1 % over 640 steps; at f32, `|δ| ≤ 1 ULP ≈ 6e-8` gives `(1+δ)⁶⁴⁰ − 1 ≈
/// 4e-5`). The remaining error is the per-step snorm16 re-quantization of the STATE, which is
/// unbiased round-to-nearest and RANDOM-WALKS: `≈ 3e-5·√640 ≈ 7.6e-4`, i.e. a 0.08 % billboard
/// size error over a 10 s life. The clamp in [`snorm16_encode`] is what keeps that drift from
/// turning into a sign wrap.
#[inline]
pub fn particle_rot_advance_body<C: Cf>(
    rot_cs: C::Uint,
    mul_cos: C::Scalar,
    mul_sin: C::Scalar,
    ret_out: &C::RetCell,
) -> Flow {
    let rc = C::temp_float("rc", snorm16_lane::<C>("cb", rot_cs));
    let rs = C::temp_float(
        "rs",
        snorm16_lane::<C>("sb", C::shr_u(rot_cs, C::uint_lit(HI16_SHIFT))),
    );
    // The complex multiply (rc + i·rs) · (mul_cos + i·mul_sin).
    let nc = C::temp_float("nc", rc.mul(mul_cos).sub(rs.mul(mul_sin)));
    let ns = C::temp_float("ns", rc.mul(mul_sin).add(rs.mul(mul_cos)));
    C::ret(
        ret_out,
        C::uor(
            snorm16_encode::<C>("qc", nc),
            C::ushl(
                snorm16_encode::<C>("qs", ns),
                C::uint_lit(HI16_SHIFT),
            ),
        ),
    )
}
