//! The **particle leaf** pins (`feature = "emit"`) — `docs/PARTICLES-PLAN.md` rung E's leaf table,
//! one test per leaf: the six P0 rows plus rung P1's `particle_sdf_response`.
//!
//! Each test pins BOTH halves of the dual instantiation of the SAME generic body
//! ([`boyko_shaderdsl::particle`]):
//!
//! - the **Eval** value — `<EvalCf>` over host `u32`/`f32` ops, checked against constants derived
//!   OUTSIDE this crate (an independent transcription for the integer hash, closed-form algebra
//!   for the float leaves) rather than against a re-run of the implementation;
//! - the **Emit** text — `<EmitCf>` through the HLSL printer, checked as the FULL span, so a
//!   wrong spelling, a wrong type token, a lost temp or a lost/added paren all fail.
//!
//! Pinning both is what makes the pair meaningful, and the particle leaves sharpen the point the
//! rung-E probes made: the Eval half cannot see a printer that spells `&` for `|`, and the Emit
//! half cannot see an encode whose bias is off by one. The `particle_edsl_sync` test in
//! `boyko_rhi_vulkan` then pins the SHIPPED `.hlsl` to these same spans, so the chain runs
//! body → span → committed shader → committed `.spv`.
//!
//! Gated on `feature = "emit"` (the printer surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

use core::cell::Cell;

use boyko_shaderdsl::cf::{Cf, EvalCf};
use boyko_shaderdsl::particle as leaves;

// ---- Eval drivers ------------------------------------------------------------------------

/// Runs one `particle_integrate` substep over `EvalCf`, returning `(pos, vel, life)`.
fn eval_integrate(
    pos: [f32; 3],
    vel: [f32; 3],
    life: f32,
    gravity: [f32; 3],
    damping: f32,
    dt: f32,
) -> ([f32; 3], [f32; 3], f32) {
    let pos_var = EvalCf::decl_param_vec3("pos", pos);
    let vel_var = EvalCf::decl_param_vec3("vel", vel);
    let life_var = EvalCf::decl_param("life", life);
    let _ = leaves::particle_integrate_body::<EvalCf>(
        &pos_var, &vel_var, &life_var, gravity, damping, dt,
    );
    (
        EvalCf::get_var_vec3(&pos_var),
        EvalCf::get_var_vec3(&vel_var),
        EvalCf::get_var(&life_var),
    )
}

/// Runs `particle_rng` over `EvalCf`.
fn eval_rng(state: u32) -> u32 {
    let cell: Cell<u32> = Cell::new(0);
    let _ = leaves::particle_rng_body::<EvalCf>(state, &cell);
    cell.get()
}

/// Runs `particle_spawn_state` over `EvalCf`, returning `(velocity, life)`.
#[allow(clippy::too_many_arguments)]
// Mirrors the leaf's own parameter list one-for-one; see the leaf's `#[allow]` rationale.
fn eval_spawn_state(
    basis_x: [f32; 3],
    basis_y: [f32; 3],
    basis_z: [f32; 3],
    cone_cos: f32,
    speed_min: f32,
    speed_max: f32,
    life_min: f32,
    life_max: f32,
    r_dir_x: u32,
    r_dir_y: u32,
    r_speed: u32,
    r_life: u32,
) -> ([f32; 3], f32) {
    let velocity: Cell<[f32; 3]> = Cell::new([0.0; 3]);
    let life: Cell<f32> = Cell::new(0.0);
    let _ = leaves::particle_spawn_state_body::<EvalCf>(
        basis_x, basis_y, basis_z, cone_cos, speed_min, speed_max, life_min, life_max, r_dir_x,
        r_dir_y, r_speed, r_life, &velocity, &life,
    );
    (velocity.get(), life.get())
}

/// Runs `particle_curve_eval` over `EvalCf`.
fn eval_curve(keys_lo: u32, keys_hi: u32, t: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = leaves::particle_curve_eval_body::<EvalCf>(keys_lo, keys_hi, t, &cell);
    cell.get()
}

/// Runs `particle_billboard_corner` over `EvalCf`.
#[allow(clippy::too_many_arguments)]
// Mirrors the leaf's own parameter list one-for-one; see the leaf's `#[allow]` rationale.
fn eval_corner(
    center: [f32; 3],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cx: f32,
    cy: f32,
    size: f32,
    rot_cs: u32,
) -> [f32; 3] {
    let world: Cell<[f32; 3]> = Cell::new([0.0; 3]);
    let _ = leaves::particle_billboard_corner_body::<EvalCf>(
        center, cam_right, cam_up, cx, cy, size, rot_cs, &world,
    );
    world.get()
}

/// Runs `particle_rot_advance` over `EvalCf`.
fn eval_rot_advance(rot_cs: u32, mul_cos: f32, mul_sin: f32) -> u32 {
    let cell: Cell<u32> = Cell::new(0);
    let _ = leaves::particle_rot_advance_body::<EvalCf>(rot_cs, mul_cos, mul_sin, &cell);
    cell.get()
}

/// Runs `particle_sdf_response` over `EvalCf`, returning the resolved `(pos, vel)`.
#[allow(clippy::too_many_arguments)]
// Mirrors the leaf's own parameter list one-for-one; see the leaf's own rationale.
fn eval_sdf_response(
    pos: [f32; 3],
    vel: [f32; 3],
    normal: [f32; 3],
    d: f32,
    radius: f32,
    restitution: f32,
    friction: f32,
) -> ([f32; 3], [f32; 3]) {
    let pos_var = EvalCf::decl_param_vec3("pos", pos);
    let vel_var = EvalCf::decl_param_vec3("vel", vel);
    let _ = leaves::particle_sdf_response_body::<EvalCf>(
        &pos_var,
        &vel_var,
        normal,
        d,
        radius,
        restitution,
        friction,
    );
    (
        EvalCf::get_var_vec3(&pos_var),
        EvalCf::get_var_vec3(&vel_var),
    )
}

/// Packs a `(cos, sin)` pair the way [`leaves::particle_rot_advance_body`]'s encode does, for the
/// round-trip pins. Independent of the leaf: plain host two's-complement, no eDSL node.
fn pack_rot(n_cos: i32, n_sin: i32) -> u32 {
    ((n_cos as u32) & 0xFFFF) | (((n_sin as u32) & 0xFFFF) << 16)
}

// ---- particle_integrate ---------------------------------------------------------------------

#[test]
fn particle_integrate_eval_and_emit() {
    // Every quantity is a dyadic rational, so the whole substep is EXACT in binary32 and the
    // expectation carries no tolerance:
    //   vel  = ((1,0,0) + (0,-4,0)*0.5) * 0.5 = ((1,-2,0)) * 0.5 = (0.5,-1,0)
    //   pos  = (0,0,0) + (0.5,-1,0)*0.5       = (0.25,-0.5,0)
    //   life = 1.0 - 0.5                       = 0.5
    let (pos, vel, life) = eval_integrate(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        1.0,
        [0.0, -4.0, 0.0],
        0.5,
        0.5,
    );
    assert_eq!(vel, [0.5, -1.0, 0.0]);
    assert_eq!(pos, [0.25, -0.5, 0.0]);
    assert_eq!(life, 0.5);

    // ORDER is the load-bearing property the values above also pin: the position advance reads
    // the ALREADY-DAMPED velocity. Had it read the entry velocity, `pos.x` would be 0.5, not
    // 0.25 — so a reordered body fails on the value, not only on the text.
    let (pos_zero_g, _, _) = eval_integrate(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        1.0,
        [0.0, 0.0, 0.0],
        0.5,
        1.0,
    );
    assert_eq!(
        pos_zero_g[0], 0.5,
        "pos must advance by the DAMPED velocity (0.5), not the entry velocity (1.0)"
    );

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_integrate().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    vel = (vel + gravity * dt) * damping;\n\
         \x20   pos = pos + vel * dt;\n\
         \x20   life = life - dt;\n",
        "the particle_integrate span must damp BEFORE advancing, with no decls (the wrapper's \
         params are `inout`):\n{g}"
    );
}

// ---- particle_rng ---------------------------------------------------------------------------

#[test]
fn particle_rng_eval_and_emit() {
    // The four vectors below were produced by an INDEPENDENT transcription of the PCG hash
    // (a stand-alone `rustc` program using explicit `wrapping_*` ops, outside this crate), not by
    // re-running the leaf. The hash is pure integer arithmetic, so these are EXACT on both
    // backends — the one leaf in the module with no floating-point carve-out.
    assert_eq!(eval_rng(0x0000_0000), 0x07BB_2FE2);
    assert_eq!(eval_rng(0x0000_0001), 0xA8BE_EA3C);
    assert_eq!(eval_rng(0x1234_5678), 0x9953_12E1);
    assert_eq!(eval_rng(0xA5A5_A5A5), 0xA7A9_2DB0);
    // The CHAIN form the emit shader uses (`r1 = particle_rng(r0)`).
    assert_eq!(eval_rng(eval_rng(0)), 0x30BE_035E);

    // Adjacent seeds must decorrelate — an emit lane's seed is `rng_seed ^ gid ^ frame`, so
    // neighbouring lanes differ in the LOW bits and a hash that leaked them would spawn visibly
    // banded particles.
    assert_ne!(eval_rng(0), eval_rng(1));
    assert_ne!(eval_rng(1), eval_rng(2));

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_rng().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint s = state * 747796405u + 2891336453u;\n\
         \x20   uint shift = (s >> 28u) + 4u;\n\
         \x20   uint word = ((s >> shift) ^ s) * 277803737u;\n\
         \x20   return (word >> 22u) ^ word;\n",
        "the particle_rng span must spell the PCG advance then the xorshift-multiply-xorshift \
         permutation, with the shift parenthesized under `+`:\n{g}"
    );
}

#[test]
fn particle_rng_dynamic_shift_never_reaches_thirty_two() {
    // `Cf::shr_u`'s Eval arm is the plain host `>>`, which PANICS in debug for an amount >= 32
    // where the GPU masks to the low 5 bits (rung E's shift-amount note). The leaf is safe only
    // because `(s >> 28u) + 4u` is bounded by 19 BY CONSTRUCTION — a property of the constants,
    // not of the input. This sweep is what makes that claim checkable rather than asserted: it
    // recomputes the amount from the same constants over a wide state sample.
    for i in 0..4096u32 {
        let state = i.wrapping_mul(0x9E37_79B9); // a golden-ratio stride over the whole u32 range
        let s = state
            .wrapping_mul(747_796_405)
            .wrapping_add(2_891_336_453);
        let shift = (s >> 28) + 4;
        assert!(
            (4..=19).contains(&shift),
            "the PCG rotation-select amount must stay in [4, 19]; state {state:#010x} gave {shift}"
        );
    }
    // The full domain of the selector, exhaustively: every 4-bit value maps into the band.
    for sel in 0..16u32 {
        assert!((4..=19).contains(&(sel + 4)));
    }
}

// ---- particle_spawn_state ---------------------------------------------------------------------

#[test]
fn particle_spawn_state_eval_and_emit() {
    // r_dir_x = r_dir_y = 0 puts the square sample at the corner (-1, -1), which the elliptical
    // grid mapping sends to (-1/sqrt2, -1/sqrt2) — a point ON the unit circle. With cone_cos = 0
    // (a hemisphere) the cap disc has R^2 = 2, so q = 2, z = 1 - q/2 = 0 (the cone RIM, exactly)
    // and sr = sqrt(1 - 0.5) * sqrt(2) = 1. Under the identity basis and speed 2 the velocity is
    // therefore 2 * (-1/sqrt2, -1/sqrt2, 0) = (-sqrt2, -sqrt2, 0).
    let (v, life) = eval_spawn_state(
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        0.0,
        2.0,
        2.0,
        3.0,
        3.0,
        0,
        0,
        0,
        0,
    );
    let root2 = 2.0f32.sqrt();
    assert!(
        (v[0] + root2).abs() <= 1.0e-6 && (v[1] + root2).abs() <= 1.0e-6 && v[2].abs() <= 1.0e-6,
        "the corner sample at cone_cos = 0 must land on the rim at (-sqrt2, -sqrt2, 0), got {v:?}"
    );
    assert_eq!(life, 3.0, "a degenerate [3,3] lifetime range must give exactly 3");

    // The UNIT-LENGTH property is the whole reason for the Lambert lift: it is an ALGEBRAIC
    // identity (s^2*q + (1 - q/2)^2 == 1), not a normalization, so it must hold for every sample
    // and every aperture — including the degenerate ones. Swept with speed 1 so |v| IS the
    // direction's length.
    for &cone_cos in &[1.0f32, 0.999, 0.5, 0.0, -0.5, -1.0] {
        for &(rx, ry) in &[
            (0u32, 0u32),
            (0xFFFF_FFFF, 0xFFFF_FFFF),
            (0x4000_0000, 0xC000_0000),
            (0x1234_5678, 0x9ABC_DEF0),
            (0x8000_0000, 0x8000_0000),
        ] {
            let (d, _) = eval_spawn_state(
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                cone_cos,
                1.0,
                1.0,
                1.0,
                1.0,
                rx,
                ry,
                0,
                0,
            );
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!(
                (len - 1.0).abs() <= 1.0e-5,
                "cone direction must be unit length by algebra; cone_cos {cone_cos}, \
                 r ({rx:#010x}, {ry:#010x}) gave |d| = {len}"
            );
            // …and it must stay INSIDE the cone. A z below `cone_cos` is the failure a
            // renormalizing implementation would hide.
            assert!(
                d[2] >= cone_cos - 1.0e-5,
                "cone direction must satisfy z >= cone_cos; cone_cos {cone_cos} gave z = {}",
                d[2]
            );
        }
    }

    // cone_cos == 1 is the ZERO-aperture degenerate: cap == 0, so every sample collapses onto
    // basis_z exactly. A leaf that divided by the cap radius would produce NaN here.
    let (d, _) = eval_spawn_state(
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        0x1234_5678,
        0x9ABC_DEF0,
        0,
        0,
    );
    assert_eq!(d, [0.0, 0.0, 1.0], "a zero-aperture cone must emit basis_z exactly");

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_spawn_state().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float ux = (float)(r_dir_x >> 8u) * 5.9604645e-8;\n\
         \x20   float uy = (float)(r_dir_y >> 8u) * 5.9604645e-8;\n\
         \x20   float us = (float)(r_speed >> 8u) * 5.9604645e-8;\n\
         \x20   float ul = (float)(r_life >> 8u) * 5.9604645e-8;\n\
         \x20   float a = ux * 2.0 - 1.0;\n\
         \x20   float b = uy * 2.0 - 1.0;\n\
         \x20   float px = a * sqrt(1.0 - 0.5 * b * b);\n\
         \x20   float py = b * sqrt(1.0 - 0.5 * a * a);\n\
         \x20   float cap = 2.0 - 2.0 * cone_cos;\n\
         \x20   float q = (px * px + py * py) * cap;\n\
         \x20   float sr = sqrt(max(1.0 - 0.25 * q, 0.0)) * sqrt(cap);\n\
         \x20   float sx = sr * px;\n\
         \x20   float sy = sr * py;\n\
         \x20   float dz = 1.0 - 0.5 * q;\n\
         \x20   float speed = lerp(speed_min, speed_max, us);\n\
         \x20   velocity = (basis_x * sx + basis_y * sy + basis_z * dz) * speed;\n\
         \x20   life = lerp(life_min, life_max, ul);\n",
        "the particle_spawn_state span must be trig-free and divide-free — the square->disc \
         mapping's two sqrt, the cap lift's two, and no others:\n{g}"
    );
    assert!(
        !g.contains('/'),
        "particle_spawn_state must contain NO divide (plan rung E leaf table):\n{g}"
    );
    assert!(
        !g.contains("sin(") && !g.contains("cos(") && !g.contains("rsqrt("),
        "particle_spawn_state must contain NO trig and NO rsqrt (plan rung E leaf table):\n{g}"
    );
}

// ---- particle_curve_eval ----------------------------------------------------------------------

/// The 4-key ramp `1, 2, 4, 8` packed as two binary16 pairs — `1.0` is `0x3C00`, `2.0` `0x4000`,
/// `4.0` `0x4400`, `8.0` `0x4800`, each pair low-then-high.
const RAMP_LO: u32 = 0x4000_3C00;
/// The high half of the `1, 2, 4, 8` ramp (see [`RAMP_LO`]).
const RAMP_HI: u32 = 0x4800_4400;

#[test]
fn particle_curve_eval_eval_and_emit() {
    // The four KEY positions. Only 0 and 1 are exactly representable as thirds are not, so the
    // interior keys are checked at their nearest binary32 and given the tolerance that carries.
    assert_eq!(eval_curve(RAMP_LO, RAMP_HI, 0.0), 1.0);
    assert_eq!(eval_curve(RAMP_LO, RAMP_HI, 1.0), 8.0);
    assert!((eval_curve(RAMP_LO, RAMP_HI, 1.0 / 3.0) - 2.0).abs() <= 1.0e-5);
    assert!((eval_curve(RAMP_LO, RAMP_HI, 2.0 / 3.0) - 4.0).abs() <= 1.0e-5);

    // A midpoint INSIDE segment 1: u = 1.5, so w = (1, 0.5, 0) and the cascade gives
    // lerp(k1, k2, 0.5) = lerp(2, 4, 0.5) = 3 — exactly, every term being dyadic.
    assert_eq!(eval_curve(RAMP_LO, RAMP_HI, 0.5), 3.0);

    // The clamp at BOTH ends: `t` outside [0,1] holds the end key rather than extrapolating.
    assert_eq!(eval_curve(RAMP_LO, RAMP_HI, -5.0), 1.0);
    assert_eq!(eval_curve(RAMP_LO, RAMP_HI, 5.0), 8.0);

    // A DESCENDING ramp — the common size-fade authoring — to prove the cascade carries sign.
    // `8, 4, 2, 1` is the reversal of the ramp above.
    let desc_lo = 0x4400_4800u32;
    let desc_hi = 0x3C00_4000u32;
    assert_eq!(eval_curve(desc_lo, desc_hi, 0.0), 8.0);
    assert_eq!(eval_curve(desc_lo, desc_hi, 1.0), 1.0);
    assert_eq!(eval_curve(desc_lo, desc_hi, 0.5), 3.0);

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_curve_eval().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float k0 = f16tof32(keys_lo & 65535u);\n\
         \x20   float k1 = f16tof32(keys_lo >> 16u);\n\
         \x20   float k2 = f16tof32(keys_hi & 65535u);\n\
         \x20   float k3 = f16tof32(keys_hi >> 16u);\n\
         \x20   float u = clamp(t, 0.0, 1.0) * 3.0;\n\
         \x20   float w0 = clamp(u, 0.0, 1.0);\n\
         \x20   float w1 = clamp(u - 1.0, 0.0, 1.0);\n\
         \x20   float w2 = clamp(u - 2.0, 0.0, 1.0);\n\
         \x20   float v0 = lerp(k0, k1, w0);\n\
         \x20   float v1 = lerp(v0, k2, w1);\n\
         \x20   return lerp(v1, k3, w2);\n",
        "the particle_curve_eval span must be the branch-free three-weight cascade — no floor, \
         no segment select:\n{g}"
    );
    assert!(
        !g.contains('/'),
        "particle_curve_eval must contain NO divide (plan rung E leaf table):\n{g}"
    );
}

// ---- particle_billboard_corner ------------------------------------------------------------------

#[test]
fn particle_billboard_corner_eval_and_emit() {
    // The IDENTITY rotation: cos = +1 stores as 32767, sin = 0 as 0. With the camera basis
    // (right = +X, up = +Y), corner (+0.5, -0.5) and size 2, the offset is exactly (+1, -1, 0).
    let identity = pack_rot(32767, 0);
    let w = eval_corner(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        0.5,
        -0.5,
        2.0,
        identity,
    );
    assert_eq!(w, [1.0, -1.0, 0.0]);

    // A QUARTER TURN: cos = 0, sin = +1. The corner (+0.5, -0.5) rotates to (+0.5, +0.5), so at
    // size 2 the offset is (+1, +1, 0). A swapped `rx`/`ry` (or a sign error in the 2x2) shows up
    // here and nowhere in the identity case above.
    let quarter = pack_rot(0, 32767);
    let w = eval_corner(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        0.5,
        -0.5,
        2.0,
        quarter,
    );
    assert!(
        (w[0] - 1.0).abs() <= 1.0e-4 && (w[1] - 1.0).abs() <= 1.0e-4 && w[2].abs() <= 1.0e-6,
        "a quarter turn must send (+0.5,-0.5) to (+1,+1) at size 2, got {w:?}"
    );

    // The centre is an ADDITIVE offset, and the camera basis is what the corner rides — a leaf
    // that used a world basis would be invisible in the two cases above (both use the identity
    // basis). Here the basis is swapped, so `rx` must ride +Y and `ry` must ride +X.
    let w = eval_corner(
        [10.0, 20.0, 30.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        0.5,
        -0.5,
        2.0,
        identity,
    );
    assert_eq!(w, [10.0 - 1.0, 20.0 + 1.0, 30.0]);

    // A NEGATIVE cosine exercises the snorm16 two's-complement decode: -1 stores as -32767,
    // whose 16-bit pattern is 0x8001 — the case a naive unsigned decode reads as +32769/32767.
    let flipped = pack_rot(-32767, 0);
    let w = eval_corner(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        0.5,
        -0.5,
        2.0,
        flipped,
    );
    assert_eq!(
        w,
        [-1.0, 1.0, 0.0],
        "a cos of -1 must decode NEGATIVE — the two's-complement lane, not an unsigned one"
    );

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_billboard_corner().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint cb = rot_cs + 32768u;\n\
         \x20   float rc = ((float)(cb & 65535u) - 32768.0) * 3.051851e-5;\n\
         \x20   uint sb = (rot_cs >> 16u) + 32768u;\n\
         \x20   float rs = ((float)(sb & 65535u) - 32768.0) * 3.051851e-5;\n\
         \x20   float rx = (cx * rc - cy * rs) * size;\n\
         \x20   float ry = (cx * rs + cy * rc) * size;\n\
         \x20   world_pos = center + cam_right * rx + cam_up * ry;\n",
        "the particle_billboard_corner span must decode the stored (cos, sin) pair and place the \
         corner with pure multiply/add — no trig:\n{g}"
    );
    assert!(
        !g.contains('/'),
        "particle_billboard_corner must contain NO divide (plan rung E leaf table):\n{g}"
    );
}

// ---- particle_rot_advance ------------------------------------------------------------------------

#[test]
fn particle_rot_advance_eval_and_emit() {
    // A QUARTER TURN applied to the identity: (1, 0) * (0, 1) = (0, 1). The result must be the
    // exact stored pattern for cos = 0 (n = 0) and sin = +1 (n = 32767).
    let identity = pack_rot(32767, 0);
    assert_eq!(
        eval_rot_advance(identity, 0.0, 1.0),
        pack_rot(0, 32767),
        "(1,0) advanced by (0,1) must store as (0, +1)"
    );
    // …and again, giving (-1, 0). This is the step that catches a decode which cannot represent
    // a negative cosine.
    assert_eq!(
        eval_rot_advance(pack_rot(0, 32767), 0.0, 1.0),
        pack_rot(-32767, 0),
        "(0,1) advanced by (0,1) must store as (-1, 0)"
    );

    // The IDENTITY multiplier is a fixed point of the whole encode/decode round trip: for every
    // representable snorm16 pair, advancing by (1, 0) must return the SAME bits. This is the
    // property that bounds the drift the plan's M7/K1 analysis budgets — a re-quantization that
    // were biased by even one LSB would fail here immediately, and would then compound over the
    // 640 steps of a 10 s life.
    for n_cos in [-32767i32, -32768, -1, 0, 1, 12345, 32767] {
        for n_sin in [-32767i32, -1, 0, 1, 32767] {
            // -32768 is representable in the STORED pattern but decodes to -1.00003, which the
            // encode clamps back to -32767 — so it is deliberately excluded from the fixed-point
            // sweep on the sin lane and asserted separately below.
            if n_cos == -32768 {
                continue;
            }
            let packed = pack_rot(n_cos, n_sin);
            assert_eq!(
                eval_rot_advance(packed, 1.0, 0.0),
                packed,
                "the identity multiplier must be a fixed point; ({n_cos}, {n_sin}) moved"
            );
        }
    }

    // The out-of-band pattern -32768 decodes to -1.00003 and the encode's clamp pulls it to
    // -32767. Stated rather than left to the sweep: it is the ONE input where the round trip is
    // deliberately not the identity, and the clamp is why the drift cannot wrap the sign.
    assert_eq!(
        eval_rot_advance(pack_rot(-32768, 0), 1.0, 0.0),
        pack_rot(-32767, 0),
        "the out-of-band -32768 lane must clamp to -32767, never wrap to +32767"
    );

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_rot_advance().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint cb = rot_cs + 32768u;\n\
         \x20   float rc = ((float)(cb & 65535u) - 32768.0) * 3.051851e-5;\n\
         \x20   uint sb = (rot_cs >> 16u) + 32768u;\n\
         \x20   float rs = ((float)(sb & 65535u) - 32768.0) * 3.051851e-5;\n\
         \x20   float nc = rc * mul_cos - rs * mul_sin;\n\
         \x20   float ns = rc * mul_sin + rs * mul_cos;\n\
         \x20   float qc = min(max(nc, -1.0), 1.0) * 32767.0 + 65536.5;\n\
         \x20   float qs = min(max(ns, -1.0), 1.0) * 32767.0 + 65536.5;\n\
         \x20   return ((uint)qc & 65535u) | (((uint)qs & 65535u) << 16u);\n",
        "the particle_rot_advance span must be the clamped complex multiply with NO \
         renormalization (plan M7/K1):\n{g}"
    );
    // Plan gate #14, at the SOURCE level. The artifact-level counterpart (zero `OpFDiv` in the
    // spliced span's module) lives in `boyko_rhi_vulkan/tests/particle_edsl_sync.rs`; this is the
    // half that runs on every host, with or without dxc.
    assert!(
        !g.contains('/'),
        "particle_rot_advance must contain NO divide — plan gate #14 (M7):\n{g}"
    );
    assert!(
        !g.contains("rsqrt(") && !g.contains("sqrt("),
        "particle_rot_advance must NOT renormalize (plan M7/K1 drops it deliberately):\n{g}"
    );
}

// ---- particle_sdf_response (rung P1) --------------------------------------------------------

#[test]
fn particle_sdf_response_eval_and_emit() {
    // EVERY quantity below is a dyadic rational, so the whole response is EXACT in binary32 and
    // the expectations carry no tolerance. That is deliberate: the leaf's only non-exact
    // ingredient is `dot`'s freedom to contract into an FMA, which cannot change a sum of exactly
    // representable products.

    // 1. HEAD-ON, on the plane normal (0,1,0): a particle 0.25 above the surface with a contact
    //    radius of 0.5 is 0.25 INSIDE the shell, so it lifts by exactly that, and its -2 approach
    //    speed leaves at +1 under restitution 0.5.
    let (pos, vel) = eval_sdf_response(
        [0.0, 0.25, 0.0],
        [0.0, -2.0, 0.0],
        [0.0, 1.0, 0.0],
        0.25,
        0.5,
        0.5,
        0.25,
    );
    assert_eq!(pos, [0.0, 0.5, 0.0], "the lift is n * (radius - d) = (0, 0.25, 0)");
    assert_eq!(
        vel,
        [0.0, 1.0, 0.0],
        "a -2 normal speed under restitution 0.5 must leave at +1, i.e. REVERSED and halved"
    );

    // 2. OBLIQUE: the same contact with a tangential component. Friction 0.25 damps the tangent to
    //    0.75 of itself and leaves the normal reflection untouched — the two coefficients are
    //    independent, which is the whole reason the effect row carries both.
    let (_, vel) = eval_sdf_response(
        [0.0, 0.25, 0.0],
        [4.0, -2.0, 0.0],
        [0.0, 1.0, 0.0],
        0.25,
        0.5,
        0.5,
        0.25,
    );
    assert_eq!(
        vel,
        [3.0, 1.0, 0.0],
        "tangential 4 -> 4*(1-0.25) = 3, normal -2 -> +1; friction must not touch the normal term \
         and restitution must not touch the tangent"
    );

    // 3. A NON-AXIS normal, so the `dot` is exercised as a real sum of three products rather than
    //    as a component read. `n = (0.5, 0.5, 0)` is exactly representable (its non-unit length is
    //    fine here — the leaf's contract is that the CALLER passes `sdf_normal`'s already-
    //    normalized gradient, and this case is about the algebra):
    //      vn  = 2*0.5 + (-4)*0.5 = -1        v_n = (-0.5, -0.5, 0)
    //      v_t = (2.5, -3.5, 0)               v_t*(1-0.5) = (1.25, -1.75, 0)
    //      vel = (1.25, -1.75, 0) - (-0.25, -0.25, 0) = (1.5, -1.5, 0)
    //      pos = (1,1,1) + (0.5,0.5,0)*(0.5-0.25) = (1.125, 1.125, 1)
    let (pos, vel) = eval_sdf_response(
        [1.0, 1.0, 1.0],
        [2.0, -4.0, 0.0],
        [0.5, 0.5, 0.0],
        0.25,
        0.5,
        0.5,
        0.5,
    );
    assert_eq!(pos, [1.125, 1.125, 1.0]);
    assert_eq!(
        vel,
        [1.5, -1.5, 0.0],
        "the normal component must come from a real dot product over all three lanes"
    );

    // 4. The two COEFFICIENT EXTREMES, as properties rather than as numbers pulled from a run.
    //    restitution 0 ⇒ the normal component is annihilated (the particle slides along the
    //    surface instead of bouncing); friction 1 ⇒ the tangential component is annihilated (it
    //    stops dead where it hit and only the bounce remains).
    let (_, slide) = eval_sdf_response(
        [0.0, 0.0, 0.0],
        [3.0, -2.0, 0.0],
        [0.0, 1.0, 0.0],
        0.0,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(slide, [3.0, 0.0, 0.0], "restitution 0 must remove the normal component entirely");
    let (_, stopped) = eval_sdf_response(
        [0.0, 0.0, 0.0],
        [3.0, -2.0, 0.0],
        [0.0, 1.0, 0.0],
        0.0,
        0.0,
        1.0,
        1.0,
    );
    assert_eq!(
        stopped,
        [0.0, 2.0, 0.0],
        "friction 1 must remove the tangential component entirely, leaving the full elastic bounce"
    );

    // 5. NON-VACUITY: a contact at exactly the shell with no coefficients at all still has to be a
    //    no-op, or the leaf would be silently moving particles that never touched anything.
    let (pos, vel) = eval_sdf_response(
        [1.0, 2.0, 3.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        0.5,
        0.5,
        0.0,
        0.0,
    );
    assert_eq!(pos, [1.0, 2.0, 3.0], "d == radius means zero penetration, hence zero lift");
    assert_eq!(vel, [1.0, 0.0, 0.0], "a purely tangential velocity is untouched at friction 0");

    let g = boyko_shaderdsl::emit::emit_hlsl_particle_sdf_response().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float vn = dot(vel, normal);\n\
         \x20   float3 v_n = normal * vn;\n\
         \x20   pos = pos + normal * (radius - d);\n\
         \x20   vel = (vel - v_n) * (1.0 - friction) - v_n * restitution;\n",
        "the particle_sdf_response span must be plan D9's response verbatim, with no decls (the \
         wrapper's pos/vel are `inout`):\n{g}"
    );
    // The particle side of the collide variant adds NO divide. The artifact-level half of this
    // claim cannot be zero — the variant `#include`s the frozen field header, whose smin/smax and
    // capsule carry divides — so `particle_edsl_sync` pins that module's count to the FIELD's own
    // contribution and this is the half that says the particle side contributed none of it.
    assert!(
        !g.contains('/'),
        "particle_sdf_response must contain NO divide:\n{g}"
    );
    assert!(
        !g.contains("sqrt(") && !g.contains("normalize("),
        "particle_sdf_response must not renormalize: `sdf_normal` already ends in `normalize`, and \
         doing it twice would put an approximate op inside this leaf's oracle for nothing:\n{g}"
    );
}
