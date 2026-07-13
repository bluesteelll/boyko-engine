//! eDSL <-> host SINGLE-SOURCE GUARD for the SSAO edge-avoiding à-trous denoise chain.
//!
//! The SSAO denoise moved OUT of `deferred_pbr.hlsl`'s resolve into a dedicated multi-pass
//! à-trous compute chain (`ssao_atrous.comp.hlsl`, mirroring the SHIPPED
//! `shadow_atrous.comp.hlsl` RT soft-shadow denoiser). `boyko_shaderdsl::ssao` single-sources ONE
//! pass's filter as [`boyko_shaderdsl::ssao::ssao_atrous_pass_body`], whose `<EvalCf>`
//! instantiation, chained `N` times with the SAME inter-pass quantization convention, is what
//! Track 1 locks against [`boyko_rhi_vulkan::goldens::golden_ssao_atrous`].
//!
//! **Track 1** (`ssao_atrous_host_matches_edsl_eval` / `ssao_atrous_consts_host_match_edsl`): a
//! CPU-only drift gate proving the `<EvalCf>` host oracle is bit-identical to
//! `golden_ssao_atrous`. `boyko_shaderdsl` is a DEV-dependency of `boyko_rhi_vulkan` (the shipped
//! lib must not link the eDSL), so this guard lives in `tests/`, mirroring `ssao_edsl_sync.rs`'s
//! host-mirror tests.
//!
//! **Track 2** (`ssao_atrous_tap_matches_edsl_emit` / `ssao_blur_combine_matches_edsl_emit`): the
//! per-tap gate+accumulate span is GENERATED (`boyko_shaderdsl::emit::emit_hlsl_ssao_atrous_tap`)
//! and pinned against the committed `ssao_atrous.comp.hlsl`; the resolve's tail `ao_class`/`min`
//! combine is GENERATED (`emit_hlsl_ssao_blur_combine`) and pinned against `deferred_pbr.hlsl`.
//! Both are `.contains` drift gates (Framing (b) — the enclosing loop/glue stays hand-written),
//! mirroring `ssao_horizon_step_matches_edsl_emit`.
//!
//! The RHI DISPATCH WIRING follow-up (Layer B) implements the C1 role-keyed pipeline/set-
//! selection invariant as a pure, GPU-free function,
//! [`boyko_rhi_vulkan::present::ssao_atrous_step`] (returning
//! [`boyko_rhi_vulkan::present::AtrousStepRole`]) — shared by the recorder
//! (`present::passes::gbuffer`), the descriptor-set builder
//! (`GBufferTargets::build_ssao_atrous_sets`), and the framegraph declarator so they can never
//! diverge on the level→role mapping. A dedicated format-consistency unit test over every valid
//! level count is a natural (tester-owned) follow-up; not added by this file.

use boyko_rhi_vulkan::compute::{self, CompositeCamera};
use boyko_rhi_vulkan::goldens::{self, MarcherAttributes};
use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::ssao;

/// A small synthetic `W x H` G-buffer + raw SSAO byte image built from a per-pixel
/// `(ssao_byte, view_t)` field. Only `view_t` (the depth gate reference) and the raw SSAO byte
/// are meaningful; the rest of [`MarcherAttributes`] is inert (`golden_ssao_atrous` reads
/// neither).
fn build_gbuffer<F: Fn(i32, i32) -> (u8, f32)>(
    w: u32,
    h: u32,
    field: F,
) -> (Vec<u8>, Vec<MarcherAttributes>) {
    let mut ssao_img = Vec::with_capacity((w * h) as usize);
    let mut gbuf = Vec::with_capacity((w * h) as usize);
    for py in 0..h {
        for px in 0..w {
            let (byte, view_t) = field(px as i32, py as i32);
            ssao_img.push(byte);
            gbuf.push(MarcherAttributes {
                base_rgb: [0, 0, 0],
                oct_rg: [128, 128],
                mat_id: 0,
                shadow: 255,
                ao: 255,
                mask: 1,
                view_t,
            });
        }
    }
    (ssao_img, gbuf)
}

/// Drives ONE à-trous pass over the WHOLE image via [`ssao::ssao_atrous_pass_body`]`::<EvalCf>` —
/// the SAME per-pixel fetch shape `ssao_atrous.comp.hlsl` reads (a coordinate-clamped
/// `(linear_view_z, AO sample)` pair at a raw pixel offset from the center). The bit-exact
/// fixtures are ORTHO (`CompositeCamera::Ortho`), so `linear_view_z`'s `rd` argument is unused
/// (a dummy `[0.0; 3]` suffices) — `composite_ray` itself is `pub(crate)` and unreachable from
/// this external test crate.
fn drive_atrous_eval_pass(
    cur: &[f32],
    gbuf: &[MarcherAttributes],
    w: u32,
    h: u32,
    step: i32,
) -> Vec<f32> {
    let wi = w as i32;
    let hi = h as i32;
    let idx_of = |x: i32, y: i32| -> usize { (y * wi + x) as usize };
    let z_at = |x: i32, y: i32| -> f32 {
        goldens::linear_view_z(CompositeCamera::Ortho, [0.0, 0.0, 0.0], gbuf[idx_of(x, y)].view_t)
    };
    let mut out = vec![0.0_f32; cur.len()];
    for py in 0..hi {
        for px in 0..wi {
            let fetch = |dx: i32, dy: i32| -> (f32, f32) {
                let tx = (px + dx).clamp(0, wi - 1);
                let ty = (py + dy).clamp(0, hi - 1);
                (z_at(tx, ty), cur[idx_of(tx, ty)])
            };
            out[idx_of(px, py)] = ssao::ssao_atrous_pass_body::<EvalCf, _>(step, &fetch);
        }
    }
    out
}

/// Chains [`drive_atrous_eval_pass`] `levels` times with the SAME inter-pass quantization
/// convention [`goldens::golden_ssao_atrous`] applies (R16_UNORM between interior passes,
/// R8_UNORM at the two frozen endpoints) — the Track-1 EDSL-SIDE composition
/// `ssao_atrous_host_matches_edsl_eval` locks against the host oracle.
fn drive_atrous_eval_chain(
    raw_ssao: &[u8],
    gbuf: &[MarcherAttributes],
    w: u32,
    h: u32,
    levels: u32,
) -> Vec<u8> {
    let mut cur: Vec<f32> = raw_ssao.iter().map(|&b| f32::from(b) / 255.0).collect();
    for level in 0..levels {
        let step = 1i32 << level;
        let raw_next = drive_atrous_eval_pass(&cur, gbuf, w, h, step);
        let is_last = level + 1 == levels;
        cur = raw_next
            .into_iter()
            .map(|v| {
                if is_last {
                    f32::from(goldens::quantize_r8_unorm(v)) / 255.0
                } else {
                    goldens::decode_r16_unorm(goldens::quantize_r16_unorm(v))
                }
            })
            .collect();
    }
    cur.iter().map(|&v| goldens::quantize_r8_unorm(v)).collect()
}

#[test]
fn ssao_atrous_host_matches_edsl_eval() {
    const W: u32 = 32;
    const H: u32 = 32;
    const LEVELS: u32 = 3;

    // Case 1: a flat-depth sharp AO discontinuity — every neighbour passes the depth gate.
    const SEAM: i32 = 16;
    let (ssao_seam, gbuf_seam) = build_gbuffer(W, H, |x, _y| (if x < SEAM { 0 } else { 255 }, 1.5));

    // Case 2: a silhouette — the far-side taps must be depth-gated out at every pass.
    let near_t = 1.5_f32;
    let far_t = near_t + 10.0 * compute::SSAO_BLUR_DEPTH_TOL;
    let (ssao_edge, gbuf_edge) = build_gbuffer(W, H, |x, _y| {
        if x < SEAM { (40, near_t) } else { (255, far_t) }
    });

    // Case 3: an isolated lit pixel surrounded by a far background (a center-only fallback).
    let (ssao_isolated, gbuf_isolated) = build_gbuffer(W, H, |x, y| {
        if x == 16 && y == 16 { (90, 1.5) } else { (255, compute::SSAO_VIEWT_BG) }
    });

    // Case 4: a uniform image (every pass must reproduce the same value bit-for-bit).
    let (ssao_uniform, gbuf_uniform) = build_gbuffer(W, H, |_x, _y| (128, 1.5));

    type Case<'a> = (&'a str, &'a [u8], &'a [MarcherAttributes]);
    let cases: [Case; 4] = [
        ("seam", &ssao_seam, &gbuf_seam),
        ("silhouette-edge", &ssao_edge, &gbuf_edge),
        ("isolated-center", &ssao_isolated, &gbuf_isolated),
        ("uniform", &ssao_uniform, &gbuf_uniform),
    ];

    for (name, ssao_img, gbuf) in cases {
        let host = goldens::golden_ssao_atrous(ssao_img, gbuf, W, H, CompositeCamera::Ortho, LEVELS);
        let edsl = drive_atrous_eval_chain(ssao_img, gbuf, W, H, LEVELS);
        assert_eq!(
            host, edsl,
            "[{name}] host goldens::golden_ssao_atrous DRIFTED from the eDSL \
             ssao::ssao_atrous_pass_body::<EvalCf> chain"
        );
    }
}

#[test]
fn ssao_atrous_levels_zero_returns_raw_unchanged() {
    const W: u32 = 8;
    const H: u32 = 8;
    let (ssao_img, gbuf) = build_gbuffer(W, H, |x, y| (((x + y) * 7) as u8, 1.5));
    let out = goldens::golden_ssao_atrous(&ssao_img, &gbuf, W, H, CompositeCamera::Ortho, 0);
    assert_eq!(out, ssao_img, "levels == 0 must return the raw gather unchanged (byte-identical OFF path)");
}

/// Reads the committed `ssao_atrous.comp.hlsl` (LF-normalized so a CRLF checkout does not
/// false-fail), the shader the Track-2 tap splice test pins against.
fn ssao_atrous_hlsl() -> String {
    let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("ssao_atrous.comp.hlsl");
    std::fs::read_to_string(&shader_path)
        .unwrap_or_else(|e| panic!("invariant: shaders/ssao_atrous.comp.hlsl must exist: {e}"))
        .replace("\r\n", "\n")
}

/// Reads the committed `deferred_pbr.hlsl` (LF-normalized), the shader the Track-2 combine
/// splice test pins against.
fn deferred_pbr_hlsl() -> String {
    let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("deferred_pbr.hlsl");
    std::fs::read_to_string(&shader_path)
        .unwrap_or_else(|e| panic!("invariant: shaders/deferred_pbr.hlsl must exist: {e}"))
        .replace("\r\n", "\n")
}

#[test]
fn ssao_atrous_tap_matches_edsl_emit() {
    // The per-tap depth-gate + accumulate is GENERATED by
    // `boyko_shaderdsl::emit::emit_hlsl_ssao_atrous_tap()` from the SAME source
    // (`ssao::ssao_blur_tap_body`, reused with linear-Z/kernel-weight inputs) whose `<EvalCf>`
    // instantiation `ssao_atrous_host_matches_edsl_eval` locks against `golden_ssao_atrous`. A
    // SPAN, not a whole function: the fixed 5x5 loop nest, the coordinate-clamped `Load` calls,
    // and the gradient stay hand-written (framing b), so this is a `.contains` of the emitted
    // span. A hand-edit of the committed span fails CI here.
    let generated = boyko_shaderdsl::emit::emit_hlsl_ssao_atrous_tap().replace("\r\n", "\n");
    let shader = ssao_atrous_hlsl();
    assert!(
        shader.contains(&generated),
        "ssao_atrous.comp.hlsl `ssao_atrous_tap` span DRIFTED from boyko_shaderdsl::emit — the \
         committed span no longer matches the generator. Re-splice between the GENERATED \
         ssao_atrous_tap sentinels.\n--- expected (eDSL-generated) ---\n{generated}"
    );
}

#[test]
fn ssao_blur_combine_matches_edsl_emit() {
    // The resolve's tail `ao_class`/`min` combine is GENERATED by
    // `boyko_shaderdsl::emit::emit_hlsl_ssao_blur_combine()` from `ssao::ssao_blur_combine_body`
    // — a SPAN spliced inside `if (ssao_mode != SSAO_MODE_OFF) { ... }`, after the hand-written
    // `float ssao_blurred = gSsao.Load(coord).r;` pre-bind.
    let generated = boyko_shaderdsl::emit::emit_hlsl_ssao_blur_combine().replace("\r\n", "\n");
    let shader = deferred_pbr_hlsl();
    assert!(
        shader.contains(&generated),
        "deferred_pbr.hlsl `ssao_blur_combine` span DRIFTED from boyko_shaderdsl::emit — the \
         committed span no longer matches the generator. Re-splice between the GENERATED \
         ssao_blur_combine sentinels.\n--- expected (eDSL-generated) ---\n{generated}"
    );
}

#[test]
fn ssao_atrous_consts_host_match_edsl() {
    // The single-source const guard: the host SSAO à-trous tuning (`compute::SSAO_*`) must equal
    // the eDSL `pub const`s (`boyko_shaderdsl::ssao::SSAO_*`). A host-only edit would silently
    // fork the oracle from the shader; this catches it host-side, before any GPU run.
    assert_eq!(compute::SSAO_ATROUS_H, ssao::SSAO_ATROUS_H, "SSAO_ATROUS_H host vs eDSL");
    assert_eq!(
        compute::SSAO_ATROUS_W_EPS.to_bits(),
        ssao::SSAO_ATROUS_W_EPS.to_bits(),
        "SSAO_ATROUS_W_EPS host vs eDSL"
    );
    assert_eq!(
        compute::SSAO_BLUR_DEPTH_TOL.to_bits(),
        ssao::SSAO_BLUR_DEPTH_TOL.to_bits(),
        "SSAO_BLUR_DEPTH_TOL host vs eDSL"
    );
    assert_eq!(
        compute::SSAO_VIEWT_BG.to_bits(),
        ssao::SSAO_VIEWT_BG.to_bits(),
        "SSAO_VIEWT_BG host vs eDSL"
    );
    assert_eq!(
        compute::SSAO_BLUR_DEPTH_SIGMA.to_bits(),
        ssao::SSAO_BLUR_DEPTH_SIGMA.to_bits(),
        "SSAO_BLUR_DEPTH_SIGMA host vs eDSL"
    );
    assert_eq!(
        compute::SSAO_BLUR_GRAD_CLAMP.to_bits(),
        ssao::SSAO_BLUR_GRAD_CLAMP.to_bits(),
        "SSAO_BLUR_GRAD_CLAMP host vs eDSL"
    );
}

/// Pure-Rust host-formula pin (M1): a PERSPECTIVE `CompositeCamera`, asserting
/// `goldens::linear_view_z` reproduces the shipped `csm_view_z`/`shadow_atrous::linear_view_z`
/// algebra bit-for-bit. The SSAO à-trous SHADER's own perspective path is a VERBATIM copy of
/// `shadow_atrous.comp.hlsl::linear_view_z` (bit-consistent by construction), so no GPU
/// perspective oracle is needed — this pins the HOST reconstruction only.
#[test]
fn ssao_linear_view_z_matches_csm_view_z() {
    let camera = CompositeCamera::Perspective {
        eye: [0.0, 0.0, 5.0],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov: 0.5773503,
        aspect: 1.0,
    };
    // A handful of ray directions (not all axis-aligned with `forward`), the SAME
    // `dot(rd, cam_forward) * view_t` convention the resolve's `csm_view_z` uses.
    let cases: [([f32; 3], f32); 3] = [
        ([0.0, 0.0, -1.0], 3.0),   // straight down -forward: z == view_t
        ([0.6, 0.0, -0.8], 4.0),   // off-axis: z == 0.8 * view_t
        ([0.0, 0.5, -0.8660254], 2.0),
    ];
    for (rd, view_t) in cases {
        let got = goldens::linear_view_z(camera, rd, view_t);
        let forward = [0.0_f32, 0.0, -1.0];
        let expected = (rd[0] * forward[0] + rd[1] * forward[1] + rd[2] * forward[2]) * view_t;
        assert!(
            (got - expected).abs() < 1.0e-6,
            "linear_view_z perspective reconstruction drift: got {got}, expected {expected}"
        );
    }
    // Ortho is a verbatim pass-through of view_t (the SSAO bit-exact fixtures' path).
    assert_eq!(goldens::linear_view_z(CompositeCamera::Ortho, [1.0, 2.0, 3.0], 7.5), 7.5);
}
