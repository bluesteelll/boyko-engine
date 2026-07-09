//! eDSL <-> host SINGLE-SOURCE GUARD for the Render P7 POLISH resolve SSAO box blur.
//!
//! `deferred_pbr.hlsl`'s inline 7x7 depth-gated box blur of `gSsao` (inside the resolve's
//! `ssao_mode != SSAO_MODE_OFF` combine) is duplicated as plain Rust in the host oracle
//! ([`goldens::golden_ssao_blur`] for the `(2*SSAO_BLUR_R+1)^2` gather). `boyko_shaderdsl::ssao`
//! now single-sources that gather (plus the `ao_class`/`min` fold that follows it in the SAME
//! HLSL block) as ONE generic body, [`boyko_shaderdsl::ssao::ssao_blur_body`], whose `<EvalCf>`
//! instantiation is what this file locks against `golden_ssao_blur`.
//!
//! **Track 1** (below): a CPU-only drift gate (`ssao_blur_host_matches_edsl_eval` /
//! `ssao_blur_consts_host_match_edsl`) proving the `<EvalCf>` host oracle is bit-identical to
//! `golden_ssao_blur`. `boyko_shaderdsl` is a DEV-dependency of `boyko_rhi_vulkan` (the shipped
//! lib must not link the eDSL), so this guard lives in `tests/`, mirroring `ssao_edsl_sync.rs`'s
//! host-mirror tests.
//!
//! **Track 2** (`ssao_blur_tap_matches_edsl_emit` / `ssao_blur_combine_matches_edsl_emit`): the
//! shader itself is now spliced. The committed loop (`for (int dy = -R; dy <= R; ++dy) { for
//! (int dx = ...) { ... } }`) is a SIGNED, symmetric-range, `<=`-terminated header — no existing
//! `Cf` loop-emit facet can print it (both `unroll_for`/`runtime_for` are hardcoded to `for (uint
//! <iv> = 0u; <iv> < <bound>; ++<iv>)`) — so, mirroring `ssao_edsl_sync.rs`'s established
//! "Framing (b)", only the per-tap gate+accumulate ([`boyko_shaderdsl::ssao::ssao_blur_tap_body`])
//! and the tail combine ([`boyko_shaderdsl::ssao::ssao_blur_combine_body`]) are GENERATED as two
//! SEPARATE spans (`boyko_shaderdsl::emit::emit_hlsl_ssao_blur_tap` /
//! `emit_hlsl_ssao_blur_combine`), spliced inside `deferred_pbr.hlsl`'s hand-written loop nest
//! (which stays untouched — the loop headers, the bounds `continue`, and the `gViewT`/`gSsao`
//! `Load` calls). `.contains` drift gates, like `ssao_horizon_step_matches_edsl_emit`.
//!
//! `golden_ssao_blur` returns ONLY the box-filter average (`ssao_blurred`); the `ao_class`/`min`
//! fold that follows it in the shader is a SEPARATE `pub(crate) ssao_combine` the host factors
//! out (not reachable from an external `tests/` file). `ssao_blur_body`, by contrast, folds
//! BOTH halves into one `ao_final` (an op-for-op mirror of the shader's ONE combine block). So
//! every case below pins `ao == 1.0`: since a box-filter average of `[0,1]` samples is always
//! `<= 1.0`, `ao_class` (which is `1.0` on EITHER branch when `ao == 1.0`) is always
//! `>= ssao_blurred`, making `min(ao_class, ssao_blurred) == ssao_blurred` bit-exact by
//! construction — isolating the gather (`golden_ssao_blur`'s exact scope) for a direct
//! bit-comparison while still exercising the fold's `select`/`ge`/`min` op path. The fold's own
//! correctness (`ao < ssao_blurred` winning the `min`, the background-sentinel branch) is
//! covered by `boyko_shaderdsl::ssao`'s own unit tests (it owns `ssao_combine`'s math, which
//! this crate cannot reach from `tests/`).

use boyko_rhi_vulkan::compute;
use boyko_rhi_vulkan::goldens::{self, MarcherAttributes};
use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::ssao;

/// A small synthetic `W x H` G-buffer + raw SSAO byte image built from a per-pixel
/// `(ssao_byte, view_t)` field, mirroring `compute::ssao_blur_tests::build` (the host's own
/// `golden_ssao_blur` fixture builder). Only `view_t` (the depth gate) and the raw SSAO byte are
/// meaningful; the rest of [`MarcherAttributes`] is inert (`golden_ssao_blur` reads neither).
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

/// Builds the `fetch: Fn(i32, i32) -> Option<(vt, s)>` neighbour seam `ssao_blur_body` needs
/// from a `(raw_ssao, gbuf)` image at pixel `(px, py)` — the SAME bounds test + `gViewT`/`gSsao`
/// reads `golden_ssao_blur`'s inner loop performs, just threaded as a closure instead of an
/// inline loop body (the established `ssao_estimate_body` `tap`-seam discipline).
fn fetch_from_gbuffer<'a>(
    ssao_img: &'a [u8],
    gbuf: &'a [MarcherAttributes],
    px: u32,
    py: u32,
    w: u32,
    h: u32,
) -> impl Fn(i32, i32) -> Option<(f32, f32)> + 'a {
    move |dx: i32, dy: i32| {
        let cx = px as i32 + dx;
        let cy = py as i32 + dy;
        if cx < 0 || cy < 0 || cx >= w as i32 || cy >= h as i32 {
            return None; // bounds
        }
        let idx = (cy * w as i32 + cx) as usize;
        let vt = gbuf[idx].view_t;
        let s = ssao_img[idx] as f32 / 255.0;
        Some((vt, s))
    }
}

#[test]
fn ssao_blur_host_matches_edsl_eval() {
    const W: u32 = 32;
    const H: u32 = 32;

    // Case 1: a flat-depth center pixel with a sharp AO discontinuity in-kernel (the seam case
    // `compute::ssao_blur_tests::sharp_ring_is_smoothed` locks host-side) — every neighbour
    // passes the depth gate, so the gather exercises the plain average.
    const SEAM: i32 = 16;
    let (ssao_seam, gbuf_seam) = build_gbuffer(W, H, |x, _y| (if x < SEAM { 0 } else { 255 }, 1.5));
    let center_px = (SEAM + 1) as u32;
    let center_py = 16;

    // Case 2: a silhouette straddled by the kernel (the near-surface pixel in
    // `compute::ssao_blur_tests::depth_gate_prevents_silhouette_bleed`) — the far-side taps must
    // be depth-gated out.
    let near_t = 1.5_f32;
    let far_t = near_t + 10.0 * compute::SSAO_BLUR_DEPTH_TOL;
    let (ssao_edge, gbuf_edge) = build_gbuffer(W, H, |x, _y| {
        if x < SEAM {
            (40, near_t)
        } else {
            (255, far_t)
        }
    });
    let edge_px = (SEAM - 1) as u32;
    let edge_py = 16;

    // Case 3: an isolated lit pixel surrounded by a far background (every OTHER neighbour is
    // gated out, forcing the corner/all-out-of-bounds-style center-only fallback — the SAME
    // fixture `compute::ssao_blur_tests::center_always_counts_no_divide_by_zero` locks).
    let (ssao_isolated, gbuf_isolated) = build_gbuffer(W, H, |x, y| {
        if x == 16 && y == 16 {
            (90, 1.5)
        } else {
            (255, compute::SSAO_VIEWT_BG)
        }
    });

    // Case 4: a TRUE image corner (0, 0) — a quarter of the `(2R+1)^2` kernel is genuinely
    // out-of-bounds (negative indices), so `fetch` returns `None` for those taps.
    let (ssao_corner, gbuf_corner) = build_gbuffer(W, H, |x, y| (((x + y) * 7) as u8, 1.5));

    // `(case name, raw SSAO image, gbuffer, pixel x, pixel y)`.
    type Case<'a> = (&'a str, &'a [u8], &'a [MarcherAttributes], u32, u32);
    let cases: [Case; 4] = [
        ("seam", &ssao_seam, &gbuf_seam, center_px, center_py),
        ("silhouette-edge", &ssao_edge, &gbuf_edge, edge_px, edge_py),
        ("isolated-center", &ssao_isolated, &gbuf_isolated, 16, 16),
        ("true-corner", &ssao_corner, &gbuf_corner, 0, 0),
    ];

    for (name, ssao_img, gbuf, px, py) in cases {
        let host = goldens::golden_ssao_blur(ssao_img, gbuf, px, py, W, H);
        let view_t = gbuf[(py * W + px) as usize].view_t;
        let fetch = fetch_from_gbuffer(ssao_img, gbuf, px, py, W, H);
        // `ao == 1.0`: forces `ao_class == 1.0` on EITHER branch (finite or sentinel `view_t`),
        // and a box average of `[0,1]` samples is always `<= 1.0`, so `min(1.0, ssao_blurred) ==
        // ssao_blurred` bit-exact — isolating the gather for a direct comparison to
        // `golden_ssao_blur` (see the module doc).
        let edsl = ssao::ssao_blur_body::<EvalCf, _>(view_t, 1.0, &fetch);
        assert_eq!(
            host.to_bits(),
            edsl.to_bits(),
            "[{name}] host goldens::golden_ssao_blur DRIFTED from the eDSL \
             ssao::ssao_blur_body::<EvalCf> (host = {host}, eDSL = {edsl}) at ({px},{py})"
        );
    }
}

/// Reads the committed `deferred_pbr.hlsl` (LF-normalized so a CRLF checkout does not
/// false-fail), the shader both Track-2 splice tests below pin against.
fn deferred_pbr_hlsl() -> String {
    let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("deferred_pbr.hlsl");
    std::fs::read_to_string(&shader_path)
        .unwrap_or_else(|e| panic!("invariant: shaders/deferred_pbr.hlsl must exist: {e}"))
        .replace("\r\n", "\n")
}

#[test]
fn ssao_blur_tap_matches_edsl_emit() {
    // The per-tap depth-gate + accumulate (Render P7 POLISH Track 2) is GENERATED by
    // `boyko_shaderdsl::emit::emit_hlsl_ssao_blur_tap()` from the SAME source
    // (`ssao::ssao_blur_tap_body`) whose `<EvalCf>` instantiation
    // `ssao_blur_host_matches_edsl_eval` above locks against `golden_ssao_blur`. It is a SPAN,
    // not a whole function: the `(2*SSAO_BLUR_R+1)^2` loop nest, the bounds `continue`, and the
    // `gViewT`/`gSsao` `Load` calls stay hand-written (framing b), so this is a `.contains` of
    // the emitted span (spliced between the `// === GENERATED ssao_blur_tap BEGIN/END ===`
    // sentinels), not an extract_fn. A hand-edit of the committed span fails CI here.
    let generated = boyko_shaderdsl::emit::emit_hlsl_ssao_blur_tap().replace("\r\n", "\n");
    let shader = deferred_pbr_hlsl();
    assert!(
        shader.contains(&generated),
        "deferred_pbr.hlsl `ssao_blur_tap` span DRIFTED from boyko_shaderdsl::emit — the \
         committed span no longer matches the generator. Re-splice between the GENERATED \
         ssao_blur_tap sentinels.\n--- expected (eDSL-generated) ---\n{generated}"
    );
}

#[test]
fn ssao_blur_combine_matches_edsl_emit() {
    // The tail `ao_class`/`min` combine (Render P7 POLISH Track 2) is GENERATED by
    // `boyko_shaderdsl::emit::emit_hlsl_ssao_blur_combine()` from `ssao::ssao_blur_combine_body`
    // — a SPAN spliced AFTER the hand-written loop nest (framing b), see
    // `ssao_blur_tap_matches_edsl_emit`'s doc.
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
fn ssao_blur_consts_host_match_edsl() {
    // The single-source const guard: the host SSAO blur tuning (`compute::SSAO_BLUR_*`) must
    // equal the eDSL `pub const`s (`boyko_shaderdsl::ssao::SSAO_BLUR_*`). A host-only edit of
    // the radius/tolerance would silently fork the oracle from the shader; this catches it
    // host-side, before any GPU run.
    assert_eq!(compute::SSAO_BLUR_R, ssao::SSAO_BLUR_R, "SSAO_BLUR_R host vs eDSL");
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
}
