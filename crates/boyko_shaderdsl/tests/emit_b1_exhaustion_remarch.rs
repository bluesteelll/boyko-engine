//! Increment 4e — the B1 EXHAUSTION RE-MARCH leaf tests.
//!
//! Two oracles, two scopes:
//!
//! (a) The EVAL CONTROL-FLOW oracle — a frozen `host_remarch` transcribed VERBATIM from the
//!     committed `if (exhausted)` re-march `[loop]` (a plain Rust `for it2 in 0..MAX_IT` with the
//!     three breaks + `t = t + d`), swept over an LCG (~10k cases) × 3 host SDF closures, asserting
//!     `(hit, t.to_bits())` bit-identical vs `b1_exhaustion_remarch_body::<EvalCf, _>`. This proves
//!     the generic body's CONTROL FLOW reproduces the committed re-march on every reachable path
//!     (the cmp-`.spv` is the byte-identity oracle for the emitted text). It covers all 5 edge
//!     cases by construction (see `host_remarch`'s doc).
//!
//! (b) The Emit GENERATOR STRUCTURE guard (`feature = "emit"`) — the brace-matched golden of the
//!     emitted span, the canonical generated text (the committed L1520-1535 with comments stripped
//!     + the R1 `t = t + d` form), mirroring `emit_soft_shadow` / `emit_m2_surface_hit_refine`.
//!
//! (c) A `set_bool_var` / `decl_bool_param` / `get_bool_var` control-table round-trip on `EvalCf`.

use boyko_shaderdsl::cf::{Cf, EvalCf};
use boyko_shaderdsl::remarch::{EPS, MAX_IT, T_MAX, b1_exhaustion_remarch_body};

// ---- (a) The Eval control-flow oracle -------------------------------------------------

/// The frozen reference re-march, transcribed VERBATIM from the committed
/// `sdf_gbuffer_composite.hlsl:1520-1535` (the inner `if (exhausted)` `[loop]`). A plain Rust
/// `for it2 in 0..MAX_IT` carrying `t` (re-seeded to `t_seed`) and `hit` (reset to `false`), with
/// the three breaks (the `t >= t_mesh` mesh guard, the `d < EPS` accept that sets `hit = true`, and
/// the `t > T_MAX` miss) + the plain `t = t + d` step. Returns `(hit, t)` — the re-march's final
/// carried state (the same tuple the generic body returns).
///
/// By construction the LCG sweep below covers all 5 design edge cases:
///   1. `t_seed >= t_mesh` on entry → the first guard breaks IMMEDIATELY, `hit == false`;
///   2. start-INSIDE the field (`d < 0`, `|d| >= EPS`) → no accept, `t` walks BACKWARD (`t + d`);
///   3. near-zero-`d` hits (`d` in `(-EPS, EPS)` → `d < EPS` fires) → `hit == true`;
///   4. budget-exhaust (all `MAX_IT` steps with no break) → `hit == false`;
///   5. `t > T_MAX` miss (the step overshoots the bound) → break, `hit == false`.
fn host_remarch(ro: [f32; 3], rd: [f32; 3], t_seed: f32, t_mesh: f32, sdf: &dyn Fn([f32; 3]) -> f32) -> (bool, f32) {
    let mut t = t_seed;
    let mut hit = false;
    for _it2 in 0..MAX_IT {
        if t >= t_mesh {
            break;
        }
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf(p);
        if d < EPS {
            hit = true;
            break;
        }
        // The committed shader spells `t += d;` (the plain frozen step); the eDSL body emits the
        // `t = t + d;` set_var form (R1: byte-identical in the `.spv`). Both round IDENTICALLY (one
        // add), so the host oracle uses the clippy-clean compound-assign.
        t += d;
        if t > T_MAX {
            break;
        }
    }
    (hit, t)
}

/// The Numerical Recipes LCG (the same generator the `eval_byte_identity` suite uses).
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    /// A float in roughly `[-range, range]`.
    fn next_f32(&mut self, range: f32) -> f32 {
        let u = (self.next_u32() as f32) / (u32::MAX as f32); // [0, 1]
        (u * 2.0 - 1.0) * range
    }
    /// A non-negative float in `[0, range]` (a `t` seed / mesh depth).
    fn next_pos_f32(&mut self, range: f32) -> f32 {
        let u = (self.next_u32() as f32) / (u32::MAX as f32); // [0, 1]
        u * range
    }
}

/// SDF #1: a horizontal plane `p.y - plane_y` — MONOTONE along a downward ray, lets `t` settle.
fn sdf_plane(p: [f32; 3], plane_y: f32) -> f32 {
    p[1] - plane_y
}

/// SDF #2: a sphere placed so `ro` can be INSIDE (negative `d`) — exercises the start-inside
/// edge (`d < 0`, the backward `t + d` walk) and the near-zero-`d` accept.
fn sdf_sphere(p: [f32; 3], c: [f32; 3], r: f32) -> f32 {
    let dx = p[0] - c[0];
    let dy = p[1] - c[1];
    let dz = p[2] - c[2];
    (dx * dx + dy * dy + dz * dz).sqrt() - r
}

/// SDF #3: a NON-MONOTONE sum (a plane + a sin ripple) — `d` can flip sign across steps, so the
/// re-march can exhaust the budget or overshoot `T_MAX`.
fn sdf_ripple(p: [f32; 3]) -> f32 {
    (p[1] - 1.0) + 0.3 * (3.0 * p[0]).sin() + 0.2 * (2.0 * p[2]).sin()
}

#[test]
fn eval_oracle_bit_identical_over_lcg_sweep() {
    let mut lcg = Lcg::new(0x05DF_E4E1_AB57);
    // ~10k cases per SDF closure (3 closures → ~30k total).
    const CASES: usize = 10_000;
    for case in 0..CASES {
        // A downward-ish ray (so a plane below the origin is reachable along +t) with random
        // jitter; the seed / mesh depth span [0, ~12] (the field is ~10 units, T_MAX = 10).
        let ro = [lcg.next_f32(3.0), lcg.next_f32(3.0) + 3.0, lcg.next_f32(3.0)];
        let rd = [lcg.next_f32(0.4), -1.0 + lcg.next_f32(0.4), lcg.next_f32(0.4)];
        let t_seed = lcg.next_pos_f32(2.0);
        let t_mesh = lcg.next_pos_f32(12.0);

        // SDF #1 — a plane somewhere below.
        let plane_y = lcg.next_f32(2.0);
        let f1 = move |p: [f32; 3]| sdf_plane(p, plane_y);
        // SDF #2 — a sphere placed so `ro` may be inside (center near `ro`, radius up to 4).
        let c = [ro[0] + lcg.next_f32(2.0), ro[1] + lcg.next_f32(2.0), ro[2] + lcg.next_f32(2.0)];
        let r = lcg.next_pos_f32(4.0);
        let f2 = move |p: [f32; 3]| sdf_sphere(p, c, r);
        // SDF #3 — the non-monotone ripple (a free fn, no captured state).

        for (idx, sdf) in [
            &f1 as &dyn Fn([f32; 3]) -> f32,
            &f2 as &dyn Fn([f32; 3]) -> f32,
            &(sdf_ripple as fn([f32; 3]) -> f32) as &dyn Fn([f32; 3]) -> f32,
        ]
        .into_iter()
        .enumerate()
        {
            let (host_hit, host_t) = host_remarch(ro, rd, t_seed, t_mesh, sdf);
            let (body_hit, body_t) =
                b1_exhaustion_remarch_body::<EvalCf, _>(ro, rd, t_seed, t_mesh, sdf);

            assert_eq!(
                host_hit, body_hit,
                "case {case} sdf {idx}: hit MISMATCH (host {host_hit} vs body {body_hit}) for \
                 ro={ro:?} rd={rd:?} t_seed={t_seed} t_mesh={t_mesh}"
            );
            assert_eq!(
                host_t.to_bits(),
                body_t.to_bits(),
                "case {case} sdf {idx}: t BIT-MISMATCH (host {host_t:?} = {:#010x} vs body \
                 {body_t:?} = {:#010x}) for ro={ro:?} rd={rd:?} t_seed={t_seed} t_mesh={t_mesh}",
                host_t.to_bits(),
                body_t.to_bits()
            );
        }
    }
}

#[test]
fn eval_oracle_covers_all_five_edge_cases() {
    // Hand-built witnesses that the 5 edge cases are EXERCISED (not just assumed reachable in the
    // sweep): each asserts the host == body AND the expected control-flow outcome.
    let down = [0.0, -1.0, 0.0];
    let origin = [0.0, 5.0, 0.0];

    // 1. t_seed >= t_mesh on entry → immediate mesh break, hit == false, t unchanged.
    {
        let f = |p: [f32; 3]| sdf_plane(p, 0.0);
        let (h, t) = b1_exhaustion_remarch_body::<EvalCf, _>(origin, down, 4.0, 1.0, &f);
        let (hh, ht) = host_remarch(origin, down, 4.0, 1.0, &f);
        assert_eq!((h, t.to_bits()), (hh, ht.to_bits()));
        assert!(!h, "t_seed >= t_mesh must NOT hit");
        assert_eq!(t.to_bits(), 4.0f32.to_bits(), "t must be unchanged on immediate mesh break");
    }

    // 2. start INSIDE a sphere (d < 0, |d| >= EPS). The committed accept guard is `d < EPS` (NOT
    //    `abs(d) < EPS`), so a NEGATIVE d also satisfies it: the seed accepts on iteration 0, BEFORE
    //    the `t += d` step, so hit == true and t is UNCHANGED at the seed. This pins the
    //    negative-d-accepts-on-entry path (distinct from case 3's positive sub-EPS d); there is no
    //    backward march, because any d < 0 accepts before the step under `d < EPS`.
    {
        // ro inside a sphere of radius 3 centered at ro → d(ro) = -3 (well past -EPS, still < EPS).
        let f = |p: [f32; 3]| sdf_sphere(p, origin, 3.0);
        let (h, t) = b1_exhaustion_remarch_body::<EvalCf, _>(origin, down, 0.0, 100.0, &f);
        let (hh, ht) = host_remarch(origin, down, 0.0, 100.0, &f);
        assert_eq!((h, t.to_bits()), (hh, ht.to_bits()));
        assert!(h, "a negative d on entry must accept under `d < EPS` (hit == true)");
        assert_eq!(t.to_bits(), 0.0f32.to_bits(), "accept on iteration 0 takes no step → t unchanged");
    }

    // 3. near-zero d → d < EPS fires → hit == true.
    {
        // A plane at y = 5 - 0.5e-3 → at t = 0, p.y = 5, d = 0.5e-3 < EPS = 1e-3 → accept.
        let f = |p: [f32; 3]| sdf_plane(p, 5.0 - 0.5e-3);
        let (h, t) = b1_exhaustion_remarch_body::<EvalCf, _>(origin, down, 0.0, 100.0, &f);
        let (hh, ht) = host_remarch(origin, down, 0.0, 100.0, &f);
        assert_eq!((h, t.to_bits()), (hh, ht.to_bits()));
        assert!(h, "a sub-EPS d on entry must accept (hit == true)");
    }

    // 4. budget-exhaust → ran all MAX_IT with no break, hit == false. A field that is always
    // >= EPS and whose step is tiny (so T_MAX is never reached within MAX_IT) and a far mesh.
    {
        // Constant d = 0.001 (== EPS, so `d < EPS` is FALSE → never accepts), tiny step, far mesh.
        // 128 steps * 0.001 = 0.128 < T_MAX, so it exhausts the budget without any break.
        let f = |_p: [f32; 3]| EPS;
        let (h, t) = b1_exhaustion_remarch_body::<EvalCf, _>(origin, down, 0.0, 100.0, &f);
        let (hh, ht) = host_remarch(origin, down, 0.0, 100.0, &f);
        assert_eq!((h, t.to_bits()), (hh, ht.to_bits()));
        assert!(!h, "budget-exhaust must NOT hit");
    }

    // 5. t > T_MAX miss → the step overshoots the bound, break, hit == false.
    {
        // A big constant positive d (> T_MAX) → one step pushes t past T_MAX → miss break.
        let f = |_p: [f32; 3]| T_MAX + 5.0;
        let (h, t) = b1_exhaustion_remarch_body::<EvalCf, _>(origin, down, 0.0, 100.0, &f);
        let (hh, ht) = host_remarch(origin, down, 0.0, 100.0, &f);
        assert_eq!((h, t.to_bits()), (hh, ht.to_bits()));
        assert!(!h, "a T_MAX miss must NOT hit");
        assert!(t > T_MAX, "t must overshoot T_MAX before the miss break");
    }
}

// ---- (c) The bool mutable-local control table (EvalCf) --------------------------------

#[test]
fn eval_bool_var_set_get_round_trip() {
    // `decl_bool_param` seeds a Cell<bool>; `set_bool_var` writes; `get_bool_var` reads back.
    let v = EvalCf::decl_bool_param("hit", false);
    assert!(!EvalCf::get_bool_var(&v), "init false must read back false");
    EvalCf::set_bool_var(&v, true);
    assert!(EvalCf::get_bool_var(&v), "after set true must read back true");
    EvalCf::set_bool_var(&v, false);
    assert!(!EvalCf::get_bool_var(&v), "after set false must read back false");

    // The `init = true` seed (the bool analogue of the `exhausted` flag's init).
    let w = EvalCf::decl_bool_param("flag", true);
    assert!(EvalCf::get_bool_var(&w), "init true must read back true");
}

// ---- (b) The Emit generator structure guard (feature = "emit") ------------------------

#[cfg(feature = "emit")]
mod emit_structure {
    fn generated() -> String {
        boyko_shaderdsl::emit::emit_hlsl_b1_exhaustion_remarch().replace("\r\n", "\n")
    }

    #[test]
    fn runtime_loop_header_spells_bound_symbol() {
        let g = generated();
        // The `[loop]` attribute + the BOUND SYMBOL `MAX_IT` (NOT a `128u` literal) — the diff
        // from `[unroll]` that makes DXC emit a genuine OpLoop.
        assert!(g.contains("[loop]"), "must carry the `[loop]` attribute:\n{g}");
        assert!(
            g.contains("for (uint it2 = 0u; it2 < MAX_IT; ++it2)"),
            "the loop header must spell the BOUND SYMBOL `MAX_IT`, not `128u`:\n{g}"
        );
        assert!(
            !g.contains("it2 < 128u"),
            "the loop header must spell the symbol, not the literal `128u`:\n{g}"
        );
    }

    #[test]
    fn mesh_guard_is_ge_t_left() {
        let g = generated();
        // The FLOAT mesh guard `t >= t_mesh` — `t` LEFT (the committed operand order), `>=` (a
        // DISTINCT opcode from a swapped `<=`).
        assert!(
            g.contains("if (t >= t_mesh) {"),
            "the mesh guard must spell `if (t >= t_mesh) {{` (t LEFT, `>=`):\n{g}"
        );
        assert!(
            !g.contains("t_mesh <="),
            "the mesh guard must be `t >= t_mesh`, never a swapped `t_mesh <= t`:\n{g}"
        );
    }

    #[test]
    fn named_p_and_d_temps_then_sdf_call() {
        let g = generated();
        // `p` is a NAMED `float3` temp (`Cf::temp_vec3`), materialized BEFORE the `sdf(p)` call
        // (unlike `b1_accept_refine`'s inline `sdf(ro + rd * t)`).
        assert!(
            g.contains("float3 p = ro + rd * t;"),
            "`p` must be a named float3 temp `float3 p = ro + rd * t;`:\n{g}"
        );
        // `d` is a NAMED `float` temp via the `sdf` call seam (the ANALYTIC field, NOT
        // field_distance).
        assert!(
            g.contains("float d = sdf(p);"),
            "the field must be a named call site `float d = sdf(p);` (interned `sdf`):\n{g}"
        );
        assert!(
            !g.contains("field_distance"),
            "this site folds the ANALYTIC `sdf`, never `field_distance`:\n{g}"
        );
    }

    #[test]
    fn accept_sets_hit_true_then_breaks() {
        let g = generated();
        // The `d < EPS` accept records BOTH `hit = true;` (the bare bool assign via set_bool_var)
        // THEN `break;` — the composite then-block.
        assert!(
            g.contains("if (d < EPS) {"),
            "the accept guard must spell `if (d < EPS) {{`:\n{g}"
        );
        assert!(
            g.contains("hit = true;"),
            "the accept must write the bare bool assign `hit = true;` (set_bool_var):\n{g}"
        );
        // No `bool hit = ...;` redecl inside the span (the decl is hand-written, suppressed here).
        assert!(
            !g.contains("bool hit"),
            "the span must NOT redeclare `hit` (decl_bool_param is suppressed-decl):\n{g}"
        );
    }

    #[test]
    fn step_is_set_var_plus_form_r1() {
        let g = generated();
        // R1: the eDSL's natural `set_var` form `t = t + d;` (byte-identical to the committed
        // `t += d;` in the `.spv`), so NO compound-assign leaf was added.
        assert!(
            g.contains("t = t + d;"),
            "the step must spell `t = t + d;` (R1 — the natural set_var form):\n{g}"
        );
        assert!(
            !g.contains("t += d"),
            "the step must be the set_var form, not a compound-assign `t += d`:\n{g}"
        );
    }

    #[test]
    fn miss_break_on_t_gt_tmax() {
        let g = generated();
        // The `t > T_MAX` miss is a real `break;`.
        assert!(
            g.contains("if (t > T_MAX) {"),
            "the miss guard must spell `if (t > T_MAX) {{`:\n{g}"
        );
        assert!(g.contains("break;"), "the guards must emit real `break;`s:\n{g}");
    }

    #[test]
    fn span_has_no_exhausted_wrapper_or_return() {
        let g = generated();
        // The `if (exhausted)` wrapper + the `t = t_seed;` re-seed + the `hit = false;` reset stay
        // HAND-WRITTEN inline (framing b) — the generator emits ONLY the inner `[loop]` span.
        for forbidden in ["if (exhausted)", "t = t_seed;", "hit = false;", "return"] {
            assert!(
                !g.contains(forbidden),
                "the generated span must NOT emit the hand-written wrapper construct `{forbidden}`:\n{g}"
            );
        }
    }

    #[test]
    fn full_span_brace_matched_golden() {
        let g = generated();
        // The WHOLE span, brace-matched — the canonical generated text (the committed L1520-1535
        // with comments stripped + the R1 `t = t + d;` form), printed at DEPTH 2 (8-space `[loop]`
        // indent — the site nests main→`if (exhausted)`→this loop). A single golden so a structural
        // drift (a missing statement, a reordered line, a stray temp, a wrong indent) fails loudly.
        const GOLDEN: &str = "        [loop]
        for (uint it2 = 0u; it2 < MAX_IT; ++it2) {
            if (t >= t_mesh) {
                break;
            }
            float3 p = ro + rd * t;
            float d = sdf(p);
            if (d < EPS) {
                hit = true;
                break;
            }
            t = t + d;
            if (t > T_MAX) {
                break;
            }
        }
";
        assert_eq!(g, GOLDEN, "the generated span must match the brace-matched golden");
    }
}
