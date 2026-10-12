//! The SDF depth-merge shader's eDSL ↔ HLSL ↔ `.spv` guard and its host oracles (render wave
//! RD1, defect R0b).
//!
//! Four layers, each failing independently:
//!
//! 1. **Generator sync** — the committed `sdf_depth_merge.fs.hlsl` IS
//!    `boyko_shaderdsl::emit::emit_sdf_depth_merge_fs()`, whole; and each
//!    `// === GENERATED <leaf> BEGIN/END ===` span is the printer's output inside the right
//!    function (the narrower pin, kept for its diagnostic).
//! 2. **`.spv` byte identity** — the committed `.spv` is the re-DXC of its source under the frozen
//!    recipe pinned in its header. Prints a `SKIP` line when no `dxc` resolves: a skipped run is
//!    NOT a pass (the byte gate is then vacuous).
//! 3. **Host oracles over the `EvalCf` leaves** —
//!    * *oracle 1* compares each leaf with an **`f64` reference** of the path's own encode and a
//!      tolerance that scales with the conditioning of the evaluation (render-wave rule (xvi)),
//!      plus an absolute floor of `2⁻¹²⁶`. Three modes: Deferred perspective (norm 64), Deferred
//!      ortho (norm 10), reverse-Z perspective (`forward_view_proj_rows`' row algebra). There is
//!      no reverse-Z ortho mode: that projection is perspective-only (`boyko_render::view`).
//!    * *oracle 2* is the **discriminator**, deliberately in `f32`: it models the stored-depth
//!      round trip the device performs — the mesh depth as stored, the marcher's own `f32` decode
//!      of it, the leaf's re-encode, the path's compare op. A mesh-won pixel must reject the
//!      merge's write; an SDF-won pixel must end with a stored depth whose decode is the composite
//!      surface within `2⁻¹⁶` relative.
//! 4. **Interface census** — the source binds `gViewT` as a SAMPLED `Texture2D` (no `RWTexture`,
//!    which would need `fragmentStoresAndAtomics` in a fragment shader), writes exactly one
//!    `SV_Depth`, generates its ray through the `#include`d `ray_gen.hlsli` (never a pasted copy),
//!    floors `cos_fwd` like the marcher does, and the marchers' decode constants are the norms the
//!    host pushes.

use std::path::PathBuf;
use std::process::Command;

use boyko_render::gbuffer_depth::mesh_view_t_norm;
use boyko_render::view::forward_view_z_coeffs;
use boyko_rhi_vulkan::compute::{CAM_MODE_ORTHO, CAM_MODE_PERSPECTIVE};
use boyko_shaderdsl::depth::{
    SDF_DEPTH_MERGE_BIAS, sdf_t_to_depth_linear, sdf_t_to_depth_reverse_z,
};

// ---- Shared plumbing ---------------------------------------------------------------------------

/// The committed shader's stem.
const STEM: &str = "sdf_depth_merge.fs";

/// The shaders directory, where the committed `.hlsl`/`.spv` live and where DXC must run so the
/// `#include "ray_gen.hlsli"` resolves.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Reads a committed shader source, LF-normalized (a CRLF checkout must not mismatch the LF
/// generator output).
fn read_shader(name: &str) -> String {
    let path = shaders_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} must exist next to this crate: {e}"))
        .replace("\r\n", "\n")
}

/// Locates `dxc`: the pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`. `None` ⇒ the
/// byte-identity gate prints a SKIP line.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Extracts `<sig> { ... }` (signature through its MATCHING closing brace) from `src` — a brace
/// counter, so nested blocks do not end the scan early.
fn extract_fn(src: &str, sig: &str) -> String {
    let start = src.find(sig).unwrap_or_else(|| panic!("the committed shader is missing `{sig}`"));
    let after = &src[start..];
    let open = after.find('{').expect("a function must have an opening brace");
    let mut depth = 0i32;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return after[..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces extracting `{sig}` — the function never closed");
}

/// The text between `// === GENERATED <name> BEGIN ===` and its END sentinel, or a panic naming
/// the missing sentinel.
fn generated_span<'a>(src: &'a str, name: &str) -> &'a str {
    let begin = format!("// === GENERATED {name} BEGIN ===\n");
    let end = format!("// === GENERATED {name} END ===\n");
    let b = src.find(&begin).unwrap_or_else(|| panic!("missing sentinel `{}`", begin.trim_end()));
    let rest = &src[b + begin.len()..];
    let e = rest.find(&end).unwrap_or_else(|| panic!("missing sentinel `{}`", end.trim_end()));
    &rest[..e]
}

// ---- Layer 1: generator sync -------------------------------------------------------------------

#[test]
fn committed_source_is_the_generator_output() {
    let committed = read_shader(&format!("{STEM}.hlsl"));
    let generated = boyko_shaderdsl::emit::emit_sdf_depth_merge_fs();
    assert!(
        committed == generated,
        "{STEM}.hlsl DRIFTED from boyko_shaderdsl::emit::emit_sdf_depth_merge_fs — the committed \
         file is not the generator's output. Re-run `cargo run -p boyko_shaderdsl --features emit \
         --bin emit_depth_merge`, re-DXC the .spv with the recipe in the shader's header, and \
         commit both.\n--- generated ---\n{generated}\n--- committed ---\n{committed}"
    );
}

#[test]
fn sdf_t_to_depth_linear_span_is_the_edsl_emit() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let span = generated_span(&src, "sdf_t_to_depth_linear");
    let func = extract_fn(span, "float sdf_t_to_depth_linear(float t, float norm)");
    let body = boyko_shaderdsl::emit::emit_hlsl_sdf_t_to_depth_linear();
    assert!(
        func.contains(&body),
        "sdf_t_to_depth_linear DRIFTED from the eDSL emit.\n--- expected ---\n{body}\n--- committed ---\n{func}"
    );
}

#[test]
fn sdf_t_to_depth_reverse_z_span_is_the_edsl_emit() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let span = generated_span(&src, "sdf_t_to_depth_reverse_z");
    let func =
        extract_fn(span, "float sdf_t_to_depth_reverse_z(float t, float cos_fwd, float a, float b)");
    let body = boyko_shaderdsl::emit::emit_hlsl_sdf_t_to_depth_reverse_z();
    assert!(
        func.contains(&body),
        "sdf_t_to_depth_reverse_z DRIFTED from the eDSL emit.\n--- expected ---\n{body}\n--- committed ---\n{func}"
    );
}

// ---- Layer 2: `.spv` byte identity ---------------------------------------------------------------

/// Re-DXCs the source under the EXACT frozen recipe its header pins and returns the bytes.
fn redxc(dxc: &PathBuf) -> Vec<u8> {
    let out_spv = child_guard::scratch_path(&format!("{STEM}.spv.redxc.spv"));
    let status = Command::new(dxc)
        .current_dir(shaders_dir())
        .args(["-spirv", "-T", "ps_6_0", "-E", "main", "-fspv-target-env=vulkan1.3"])
        .arg(format!("{STEM}.hlsl"))
        .arg("-Fo")
        .arg(&*out_spv)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {STEM}.hlsl under the frozen recipe");
    std::fs::read(&*out_spv).expect("invariant: dxc wrote the re-DXC .spv")
}

#[test]
fn every_header_pins_the_frozen_recipe() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let recipe = "C:\\VulkanSDK\\1.4.350.0\\Bin\\dxc.exe -spirv -T ps_6_0 -E main \\\n\
                  //       -fspv-target-env=vulkan1.3 sdf_depth_merge.fs.hlsl -Fo sdf_depth_merge.fs.spv";
    assert!(src.contains(recipe), "{STEM}.hlsl's header no longer pins the recipe this gate runs");
}

#[test]
fn sdf_depth_merge_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP sdf_depth_merge_spv_byte_identical: no dxc on this host");
        return;
    };
    let committed_path = shaders_dir().join(format!("{STEM}.spv"));
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc);
    assert!(
        committed == fresh,
        "{STEM}.spv ({} bytes committed, {} fresh) is NOT the re-DXC of {STEM}.hlsl under the \
         frozen recipe — the committed .spv is stale, or the host dxc is not VulkanSDK 1.4.350.0.",
        committed.len(),
        fresh.len()
    );
}

// ---- Layer 3: host oracles ------------------------------------------------------------------------

/// `2⁻²⁴` — the f32 unit roundoff.
const U: f64 = 1.0 / 16_777_216.0;
/// The absolute floor every rule-(xvi) tolerance carries (`2⁻¹²⁶`, the smallest normal f32).
const FLOOR: f64 = 1.175_494_350_822_287_5e-38;
/// Samples per mode.
const SAMPLES: usize = 100_000;

/// SplitMix64 — a deterministic, dependency-free sample source (the domains are stated at each use).
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Log-uniform in `[lo, hi)` (both positive).
    fn log_range(&mut self, lo: f64, hi: f64) -> f64 {
        (lo.ln() + (hi.ln() - lo.ln()) * self.unit()).exp()
    }
}

/// One reverse-Z sample: a camera, a pixel ray and the f32 values the shader works with.
struct RzSample {
    /// `forward_view_z_coeffs(near, far)` — the engine's own f32 `(A, B)`.
    a: f32,
    b: f32,
    eye: [f32; 3],
    fwd: [f32; 3],
    rd: [f32; 3],
    /// `max(dot(fwd, rd), 1e-4)` in f32 — the shader's `cos_fwd` (the marcher's own factor).
    cos_fwd: f32,
    near: f32,
    far: f32,
}

fn dot32(x: [f32; 3], y: [f32; 3]) -> f32 {
    x[0] * y[0] + x[1] * y[1] + x[2] * y[2]
}

fn dot64(x: [f32; 3], y: [f32; 3]) -> f64 {
    f64::from(x[0]) * f64::from(y[0]) + f64::from(x[1]) * f64::from(y[1]) + f64::from(x[2]) * f64::from(y[2])
}

fn norm64(x: [f32; 3]) -> f64 {
    dot64(x, x).sqrt()
}

/// Draws a reverse-Z camera and one pixel ray. DOMAIN: `near ∈ [0.01, 1]`, `far ∈ [10·near, 5000]`,
/// `eye ∈ [-500, 500]³`, a yaw/pitch orthonormal basis (pitch within ±80°), `tan(fovY/2) ∈ [0.2,
/// 1.5]`, aspect 16/9, NDC uniform in `[-1, 1]²`; the ray is `generate_ray`'s perspective arm in f32.
fn rz_sample(rng: &mut Rng) -> RzSample {
    let near = rng.log_range(0.01, 1.0) as f32;
    let far = rng.log_range(f64::from(near) * 10.0, 5000.0) as f32;
    let (a, b) = forward_view_z_coeffs(near, far);
    let eye = [
        rng.range(-500.0, 500.0) as f32,
        rng.range(-500.0, 500.0) as f32,
        rng.range(-500.0, 500.0) as f32,
    ];
    let yaw = rng.range(-std::f64::consts::PI, std::f64::consts::PI);
    let pitch = rng.range(-1.39, 1.39);
    let fwd64 = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
    let right64 = [yaw.cos(), 0.0, -yaw.sin()];
    let up64 = [
        fwd64[1] * right64[2] - fwd64[2] * right64[1],
        fwd64[2] * right64[0] - fwd64[0] * right64[2],
        fwd64[0] * right64[1] - fwd64[1] * right64[0],
    ];
    let fwd = fwd64.map(|c| c as f32);
    let right = right64.map(|c| c as f32);
    let up = up64.map(|c| c as f32);
    let tan_half = rng.range(0.2, 1.5) as f32;
    let aspect = 16.0f32 / 9.0;
    let ndc_x = rng.range(-1.0, 1.0) as f32;
    let ndc_y = rng.range(-1.0, 1.0) as f32;
    let sx = ndc_x * aspect * tan_half;
    let sy = ndc_y * tan_half;
    let dir = [
        fwd[0] + right[0] * sx + up[0] * sy,
        fwd[1] + right[1] * sx + up[1] * sy,
        fwd[2] + right[2] * sx + up[2] * sy,
    ];
    let len = dot32(dir, dir).sqrt();
    let rd = dir.map(|c| c / len);
    let cos_fwd = dot32(fwd, rd).max(1.0e-4);
    RzSample { a, b, eye, fwd, rd, cos_fwd, near, far }
}

/// The `f64` reference of the reverse-Z encode: `P = eye + rd·s` with `s = t·(1+β)`, projected
/// through `forward_view_proj_rows`' row algebra — `view_z = row3·[P,1] = fwd·P − fwd·eye`,
/// `clip_z = row2·[P,1] = A·view_z + B` — and divided. Returns `(depth, view_z, conditioning)`,
/// the last being `κ_v = (2|eye| + |P|) / view_z`, the cancellation factor of `fwd·P − fwd·eye`.
fn rz_reference(s: &RzSample, t: f32) -> (f64, f64, f64) {
    let scale = f64::from(t) * (1.0 + f64::from(SDF_DEPTH_MERGE_BIAS));
    let p = [
        f64::from(s.eye[0]) + f64::from(s.rd[0]) * scale,
        f64::from(s.eye[1]) + f64::from(s.rd[1]) * scale,
        f64::from(s.eye[2]) + f64::from(s.rd[2]) * scale,
    ];
    let fwd = s.fwd.map(f64::from);
    let fwd_p = fwd[0] * p[0] + fwd[1] * p[1] + fwd[2] * p[2];
    let view_z = fwd_p - dot64(s.fwd, s.eye);
    let clip_z = f64::from(s.a) * view_z + f64::from(s.b);
    let p_norm = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    (clip_z / view_z, view_z, (2.0 * norm64(s.eye) + p_norm) / view_z)
}

/// Oracle 1, Deferred arms: `leaf(t, norm)` against `t·(1+β)/norm` in f64. Two f32 roundings
/// (the product, the quotient) and no cancellation, so the conditioning is 1:
/// `tol = (2u + u²)·|ref| + 2⁻¹²⁶`. DOMAIN: `t` log-uniform in `[1e-3, norm)`.
fn deferred_oracle_1(norm: f32, mode: &str) {
    let mut rng = Rng(0xD3F3_44ED ^ u64::from(norm.to_bits()));
    let mut worst = 0.0f64;
    for i in 0..SAMPLES {
        let t = rng.log_range(1.0e-3, f64::from(norm)) as f32;
        let leaf = f64::from(sdf_t_to_depth_linear(t, norm));
        let reference = f64::from(t) * (1.0 + f64::from(SDF_DEPTH_MERGE_BIAS)) / f64::from(norm);
        let tol = (2.0 * U + U * U) * reference.abs() + FLOOR;
        let err = (leaf - reference).abs();
        assert!(
            err <= tol,
            "{mode} oracle 1 sample {i}: leaf({t}, {norm}) = {leaf:e}, f64 reference {reference:e}, \
             |err| {err:e} > tol {tol:e}"
        );
        worst = worst.max(err / tol);
    }
    eprintln!("{mode} oracle 1: {SAMPLES} samples, worst |err|/tol = {worst:.3}");
}

#[test]
fn oracle_1_deferred_perspective_matches_f64_reference() {
    deferred_oracle_1(mesh_view_t_norm(CAM_MODE_PERSPECTIVE), "Deferred PERSP");
}

#[test]
fn oracle_1_deferred_ortho_matches_f64_reference() {
    deferred_oracle_1(mesh_view_t_norm(CAM_MODE_ORTHO), "Deferred ORTHO");
}

/// Oracle 1, reverse-Z: `leaf(t, cos_fwd, A, B)` against the f64 projection of
/// `P = eye + rd·t·(1+β)` (see [`rz_reference`]).
///
/// TOLERANCE (rule (xvi): scaled by the conditioning, plus the absolute floor). With
/// `q = B / view_z` the leaf's quotient, the leaf's error is bounded by
/// `|q|·((4 + 3κ_c)·u) + |ref|·u`, where `κ_c = |fwd||rd| / cos_fwd` is the conditioning of the
/// f32 `cos_fwd` dot product (≤ 3u·|fwd||rd| absolute) and the 4u covers the biased product, the
/// `·cos_fwd` product and the quotient; the final `A + q` adds `u·|ref|`. When `ref → 0` (a
/// surface near the far plane, `A + q` cancelling) the bound stays proportional to `|q|`: that is
/// the conditioning of the last add. The reference's own cancellation (`fwd·P − fwd·eye` in f64)
/// adds `|q|·κ_v·2⁻⁵⁰`. A missing or wrong bias moves the leaf by `β·|q| = 256u·|q|`, far outside.
///
/// DOMAIN: [`rz_sample`]'s cameras and rays, `t` such that `t·cos_fwd` is log-uniform in
/// `[near, far]`.
#[test]
fn oracle_1_reverse_z_perspective_matches_f64_projection() {
    let mut rng = Rng(0x5EF0_2E2E);
    let mut worst = 0.0f64;
    for i in 0..SAMPLES {
        let s = rz_sample(&mut rng);
        let view_z = rng.log_range(f64::from(s.near), f64::from(s.far));
        let t = (view_z / f64::from(s.cos_fwd)) as f32;
        let leaf = f64::from(sdf_t_to_depth_reverse_z(t, s.cos_fwd, s.a, s.b));
        let (reference, view_z_ref, kappa_v) = rz_reference(&s, t);
        let q = f64::from(s.b) / view_z_ref;
        let kappa_c = norm64(s.fwd) * norm64(s.rd) / f64::from(s.cos_fwd);
        let tol = q.abs() * ((4.0 + 3.0 * kappa_c) * U + kappa_v * f64::powi(2.0, -50))
            + reference.abs() * U
            + FLOOR;
        let err = (leaf - reference).abs();
        assert!(
            err <= tol,
            "reverse-Z oracle 1 sample {i}: leaf = {leaf:e}, f64 projection {reference:e}, |err| \
             {err:e} > tol {tol:e} (t {t}, cos_fwd {}, A {}, B {}, kappa_c {kappa_c:.3}, kappa_v \
             {kappa_v:.3e})",
            s.cos_fwd,
            s.a,
            s.b
        );
        worst = worst.max(err / tol);
    }
    eprintln!("reverse-Z oracle 1: {SAMPLES} samples, worst |err|/tol = {worst:.3}");
}

/// The marcher's f32 decode of a reverse-Z depth (`sdf_forward_march.comp.hlsl`, HAS_MESH):
/// `view_z = B / (d − A)`, `t = view_z / max(cos_fwd, 1e-4)` — op for op.
fn rz_decode(d: f32, s: &RzSample) -> f32 {
    let view_z = s.b / (d - s.a);
    view_z / s.cos_fwd.max(1.0e-4)
}

/// The raster's stored reverse-Z depth for a surface at ray parameter `t_surface` (exact f64
/// projection, then the one rounding the depth attachment's f32 store performs).
fn rz_stored(s: &RzSample, t_surface: f64) -> f32 {
    let view_z = t_surface * dot64(s.fwd, s.rd).max(1.0e-4);
    (f64::from(s.a) + f64::from(s.b) / view_z) as f32
}

/// The relative slack on oracle 2's "stored depth decodes to the composite surface" clause beyond
/// the `2⁻¹⁶` bias itself: the f32 decode's few roundings.
const DECODE_SLACK: f64 = 16.0 * U;

/// Oracle 2 bookkeeping: how many samples took each branch, so a vacuous run is visible.
#[derive(Default)]
struct Discriminator {
    mesh_front_rejected: usize,
    mesh_behind_bias_kept: usize,
    mesh_far_behind_written: usize,
}

/// Oracle 2, Deferred arms (LESS, clear 1.0): the stored mesh depth is `t_mesh / norm` in f32 (the
/// `gbuffer_mrt` encode), the composite decodes `t = md · norm` in f32.
/// DOMAIN: SDF `t_s` log-uniform in `[1e-3, norm/2.5)`.
fn deferred_oracle_2(norm: f32, mode: &str) {
    let mut rng = Rng(0xD15C_0002 ^ u64::from(norm.to_bits()));
    let mut seen = Discriminator::default();
    for i in 0..SAMPLES {
        let t_s = rng.log_range(1.0e-3, f64::from(norm) / 2.5);
        // Mesh IN FRONT by 2^-17: the mesh owns the pixel and the composite `t` is its decode.
        let d_mesh = ((t_s * (1.0 - f64::powi(2.0, -17))) as f32) / norm;
        let t_composite = d_mesh * norm;
        let d_merge = sdf_t_to_depth_linear(t_composite, norm);
        assert!(
            d_merge >= d_mesh,
            "{mode} oracle 2 sample {i}: merge writes at a mesh-won pixel (merge {d_merge:e} \
             LESS stored {d_mesh:e})"
        );
        seen.mesh_front_rejected += 1;
        // Mesh BEHIND within the bias (2^-17) and FAR behind (x2): the SDF owns the pixel.
        let t_s32 = t_s as f32;
        let d_merge = sdf_t_to_depth_linear(t_s32, norm);
        for (k, factor) in [1.0 + f64::powi(2.0, -17), 2.0].into_iter().enumerate() {
            let d_mesh = ((t_s * factor) as f32) / norm;
            let written = d_merge < d_mesh;
            let stored = if written { d_merge } else { d_mesh };
            let decoded = f64::from(stored * norm);
            let rel = (decoded / f64::from(t_s32) - 1.0).abs();
            assert!(
                rel <= f64::from(SDF_DEPTH_MERGE_BIAS) + DECODE_SLACK,
                "{mode} oracle 2 sample {i} (mesh x{factor}): the stored depth decodes to {decoded}, \
                 {rel:e} relative from the composite surface {t_s32}"
            );
            match (k, written) {
                (0, false) => seen.mesh_behind_bias_kept += 1,
                (1, true) => seen.mesh_far_behind_written += 1,
                _ => {}
            }
        }
    }
    report_and_check(mode, &seen);
}

fn report_and_check(mode: &str, seen: &Discriminator) {
    eprintln!(
        "{mode} oracle 2: {SAMPLES} samples; mesh-front rejected {}, mesh-behind-within-bias kept {}, \
         mesh-far-behind written {}",
        seen.mesh_front_rejected, seen.mesh_behind_bias_kept, seen.mesh_far_behind_written
    );
    // Anti-vacuity: the two SDF-won branches are each taken on EVERY sample — the bias is larger
    // than 2^-17 (kept) and far smaller than x2 (written). A branch count of 0 would mean the
    // oracle compared nothing.
    assert_eq!(seen.mesh_behind_bias_kept, SAMPLES, "{mode}: the within-bias mesh was overwritten");
    assert_eq!(seen.mesh_far_behind_written, SAMPLES, "{mode}: the merge failed to write over a far mesh");
}

#[test]
fn oracle_2_deferred_perspective_discriminates() {
    deferred_oracle_2(mesh_view_t_norm(CAM_MODE_PERSPECTIVE), "Deferred PERSP");
}

#[test]
fn oracle_2_deferred_ortho_discriminates() {
    deferred_oracle_2(mesh_view_t_norm(CAM_MODE_ORTHO), "Deferred ORTHO");
}

/// Oracle 2, reverse-Z (GREATER, clear 0.0). DOMAIN: [`rz_sample`]'s cameras and rays, the SDF
/// surface's view depth log-uniform in `[2·near, far/4]`.
#[test]
fn oracle_2_reverse_z_discriminates() {
    let mut rng = Rng(0x2E2E_0002);
    let mut seen = Discriminator::default();
    for i in 0..SAMPLES {
        let s = rz_sample(&mut rng);
        let view_z = rng.log_range(2.0 * f64::from(s.near), f64::from(s.far) / 4.0);
        let t_s = view_z / dot64(s.fwd, s.rd).max(1.0e-4);
        // Mesh IN FRONT by 2^-17: the composite `t` is the marcher's f32 decode of the stored depth.
        let d_mesh = rz_stored(&s, t_s * (1.0 - f64::powi(2.0, -17)));
        let t_composite = rz_decode(d_mesh, &s);
        let d_merge = sdf_t_to_depth_reverse_z(t_composite, s.cos_fwd, s.a, s.b);
        assert!(
            d_merge <= d_mesh,
            "reverse-Z oracle 2 sample {i}: merge writes at a mesh-won pixel (merge {d_merge:e} \
             GREATER stored {d_mesh:e}; t {t_composite}, cos_fwd {}, A {}, B {})",
            s.cos_fwd,
            s.a,
            s.b
        );
        seen.mesh_front_rejected += 1;
        // Mesh BEHIND within the bias and FAR behind: the SDF owns the pixel; `t` is the SDF's.
        let t_s32 = t_s as f32;
        let d_merge = sdf_t_to_depth_reverse_z(t_s32, s.cos_fwd, s.a, s.b);
        for (k, factor) in [1.0 + f64::powi(2.0, -17), 2.0].into_iter().enumerate() {
            let d_mesh = rz_stored(&s, t_s * factor);
            let written = d_merge > d_mesh;
            let stored = if written { d_merge } else { d_mesh };
            let decoded = f64::from(rz_decode(stored, &s));
            let rel = (decoded / f64::from(t_s32) - 1.0).abs();
            assert!(
                rel <= f64::from(SDF_DEPTH_MERGE_BIAS) + DECODE_SLACK,
                "reverse-Z oracle 2 sample {i} (mesh x{factor}): the stored depth decodes to \
                 {decoded}, {rel:e} relative from the composite surface {t_s32}"
            );
            match (k, written) {
                (0, false) => seen.mesh_behind_bias_kept += 1,
                (1, true) => seen.mesh_far_behind_written += 1,
                _ => {}
            }
        }
    }
    report_and_check("reverse-Z", &seen);
}

// ---- Layer 4: interface census -----------------------------------------------------------------------

/// `src` with every `//` comment removed, line by line — the census reads CODE, so the header's
/// prose about `SV_Depth` / storage images cannot satisfy or trip it.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|l| l.find("//").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join("
")
}

#[test]
fn gviewt_is_sampled_never_a_storage_image() {
    let src = code_only(&read_shader(&format!("{STEM}.hlsl")));
    assert!(
        !src.contains("RWTexture"),
        "{STEM}.hlsl declares a storage image: a fragment-stage storage image without NonWritable \
         needs fragmentStoresAndAtomics, which the device does not enable — bind gViewT as a \
         sampled Texture2D at GENERAL (SampledImageAtGeneral)"
    );
    assert!(src.contains("Texture2D<float> gViewT : register(t0);"));
    assert!(src.contains("cbuffer Camera : register(b1)"));
}

#[test]
fn exactly_one_depth_output_and_no_colour_target() {
    let src = code_only(&read_shader(&format!("{STEM}.hlsl")));
    assert_eq!(src.matches(": SV_Depth").count(), 1, "{STEM}.hlsl must write exactly one SV_Depth");
    assert!(!src.contains("SV_Target"), "{STEM}.hlsl is a depth-only pass");
}

#[test]
fn the_ray_is_the_marchers_include_not_a_copy() {
    let src = code_only(&read_shader(&format!("{STEM}.hlsl")));
    assert!(src.contains("#include \"ray_gen.hlsli\""));
    assert!(
        !src.contains("void generate_ray("),
        "{STEM}.hlsl defines its own generate_ray — a pasted copy desyncs the merge's rd from the \
         marcher's on the next ray-gen change"
    );
}

#[test]
fn the_reverse_z_factor_and_sentinel_are_the_marchers() {
    let merge = read_shader(&format!("{STEM}.hlsl"));
    let marcher = read_shader("sdf_forward_march.comp.hlsl");
    // The marcher's decode factor, floor included, and the merge's re-encode factor.
    assert!(marcher.contains("float cos_fwd = dot(cam_forward.xyz, rd);"));
    assert!(marcher.contains("float t_mesh = view_z_mesh / max(cos_fwd, 1.0e-4);"));
    assert!(marcher.contains("float view_z_mesh = pc.view_z_b / (depth_mesh - pc.view_z_a);"));
    assert!(merge.contains("float cos_fwd = max(dot(cam_forward.xyz, rd), 1.0e-4);"));
    // Both marchers' background sentinel sits above the merge's discard threshold.
    assert!(marcher.contains("static const float VIEWT_BG = 1.0e30;"));
    let composite = read_shader("sdf_gbuffer_composite.hlsl");
    assert!(composite.contains("gViewT[uint2(px, py)] = has_mesh ? t_mesh : 1.0e30;"));
    assert!(merge.contains("static const float VIEWT_MISS = 0.5e30;"));
}

#[test]
fn the_deferred_norms_are_the_composites_decode_constants() {
    // The Deferred merge pushes `mesh_view_t_norm(camera_mode)`; the composite decodes
    // `t_mesh = md * mesh_norm` with these two literals. One number, pinned at both homes.
    let composite = read_shader("sdf_gbuffer_composite.hlsl");
    assert!(composite.contains("static const float T_MAX  = 10.0;"));
    assert!(composite.contains("static const float MESH_DEPTH_T_MAX = 64.0;"));
    assert!(composite.contains(
        "float mesh_norm = (camera_mode == CAM_PERSPECTIVE) ? MESH_DEPTH_T_MAX : T_MAX;"
    ));
    assert_eq!(mesh_view_t_norm(CAM_MODE_ORTHO), 10.0);
    assert_eq!(mesh_view_t_norm(CAM_MODE_PERSPECTIVE), 64.0);
}

#[test]
fn the_host_push_and_mode_words_are_the_shaders() {
    use boyko_rhi_vulkan::compute::{
        SDF_DEPTH_MERGE_MODE_LINEAR, SDF_DEPTH_MERGE_MODE_REVERSE_Z, SDF_DEPTH_MERGE_PUSH_BYTES,
        SdfDepthEncode, SdfDepthMergePush,
    };
    // One number, two homes: the host push's mode words ARE the generator inputs.
    assert_eq!(SDF_DEPTH_MERGE_MODE_LINEAR, boyko_shaderdsl::depth::SDF_DEPTH_MERGE_MODE_LINEAR);
    assert_eq!(SDF_DEPTH_MERGE_MODE_REVERSE_Z, boyko_shaderdsl::depth::SDF_DEPTH_MERGE_MODE_REVERSE_Z);
    // The shader's push block, field for field, in the host struct's order (20 B, no padding).
    let src = code_only(&read_shader(&format!("{STEM}.hlsl")));
    let block = src
        .split("struct SdfDepthMergePush {")
        .nth(1)
        .and_then(|rest| rest.split("};").next())
        .expect("the shader declares `struct SdfDepthMergePush`");
    let fields: Vec<String> =
        block.split(';').map(|f| f.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|f| !f.is_empty()).collect();
    assert_eq!(fields, ["uint mode", "float k0", "float k1", "uint img_w", "uint img_h"], "the shader's push block");
    assert_eq!(SDF_DEPTH_MERGE_PUSH_BYTES, 20);
    // Each encode arm lands its words where the shader reads them.
    let word = |b: &[u8], i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    let lin = SdfDepthMergePush::new(SdfDepthEncode::Linear { norm: 64.0 }, 2560, 1440);
    let b = lin.as_bytes();
    assert_eq!(b.len(), 20);
    assert_eq!(
        [word(b, 0), word(b, 1), word(b, 2), word(b, 3), word(b, 4)],
        [SDF_DEPTH_MERGE_MODE_LINEAR, 64.0f32.to_bits(), 0, 2560, 1440]
    );
    let rz = SdfDepthMergePush::new(SdfDepthEncode::ReverseZ { a: -0.001, b: 0.1 }, 7, 9);
    let b = rz.as_bytes();
    assert_eq!(
        [word(b, 0), word(b, 1), word(b, 2), word(b, 3), word(b, 4)],
        [SDF_DEPTH_MERGE_MODE_REVERSE_Z, (-0.001f32).to_bits(), 0.1f32.to_bits(), 7, 9]
    );
}

#[test]
fn the_bias_is_the_leafs_constant() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    assert!(src.contains("static const float SDF_DEPTH_MERGE_BIAS = 1.0 / 65536.0;"));
    assert_eq!(SDF_DEPTH_MERGE_BIAS.to_bits(), (1.0f32 / 65536.0).to_bits());
}

// `status_within` / `output_within` (a deadline on each dxc this file spawns) and `scratch_path`
// live in `tests/child_guard/mod.rs`; declared last so no cited line above moves.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
