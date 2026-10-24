//! SSAOM F3 — the screen-space AO must not see the facets of a smooth-shaded mesh.
//!
//! The SSAO horizon step (`boyko_shaderdsl::ssao::ssao_horizon_step_body_params`, spliced into
//! `shaders/sdf_ssao*.comp.hlsl`) measures each neighbour tap's elevation `dot(delta, N) /
//! |delta|` against the CENTER pixel's normal `N`. On a tessellated, smooth-shaded mesh `N` is
//! the INTERPOLATED vertex normal, while the neighbours lie on the FLAT facet: a tap on the same
//! facet rises above `N`'s tangent plane by up to `sin(tau)`, `tau` being the angle between the
//! interpolated normal and the facet normal. Without an angle bias every such tap counts as a
//! horizon, so the AO darkens in a pattern locked to the facets (the owner's gold render,
//! 2026-10-24: regular vertical stripes on the 64-slice sphere — 0.889 at every slice edge vs
//! 0.939 mid-facet; SSAO off removes them). HBAO+ hides exactly this with its angle bias
//! ("AO Bias … hide low-tessellation artifacts", NVIDIA's documented default 0.1).
//!
//! The oracle is `goldens::golden_ssao_attributes_derived`: the SAME eDSL body the HLSL span is
//! traced from, instantiated over `f32`, behind the device's exact tap seam (the pixel-radius
//! clamp, the Hilbert+R2 dither, the slice rotation, the forward reconstruct). The G-buffer is
//! ray-cast here from the showcase's own sphere (`uv_sphere(1.0, 48, 64)`), with the producers'
//! 8-bit octahedral normal store.
//!
//! The tests:
//!   * [`the_fixture_tilt_is_inside_the_hbao_plus_margin`] — fixture validity: the largest
//!     facet tilt of the showcase sphere, plus the oct8 normal error, stays below the 0.1 bias.
//!   * [`a_tessellated_sphere_shows_no_facet_locked_ssao`] — the defect: on a convex sphere no
//!     pixel of the equator band may be occluded, and the AO binned by slice phase must be flat.
//!     Arm P (perspective, the 24-px radius clamp) at Low / Medium / High; arm O (ortho, NO
//!     clamp, the full 0.5-world kernel) at High — arm O separates the clamp from the cause.
//!   * [`contact_ao_under_a_resting_sphere_survives`] — non-vacuity: real contact occlusion on a
//!     floor under the sphere stays dark.
//!
//! Host-only: no device, no `#[ignore]`.

use std::cell::Cell;

use boyko_rhi_vulkan::compute::{
    CompositeCamera, SSAO_PARAMS, SSAO_QUALITY_HIGH, SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM,
    composite_pixel_ray,
};
use boyko_rhi_vulkan::goldens::{MarcherAttributes, golden_ssao_attributes_derived};
use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::oct::oct_encode_body;

/// The largest angular error of the producers' 8-bit octahedral normal store, as a sine (the
/// SSAOM research measured 0.942° at most, sin = 0.0164; rounded up).
const OCT8_MAX_SIN_ERROR: f32 = 0.0165;

/// HBAO+'s documented angle-bias default — the value the fixture tilt must clear.
const HBAO_PLUS_ANGLE_BIAS: f32 = 0.1;

/// The showcase sphere: `pbr_material_showcase.rs` builds `uv_sphere(1.0, 48, 64)`.
const SHOWCASE_STACKS: u32 = 48;
const SHOWCASE_SLICES: u32 = 64;

/// The slice-phase histogram resolution (code.md §2.3's method: 10 bins of `frac(theta /
/// (2 pi / slices))`).
const PHASE_BINS: usize = 10;

/// Anti-vacuity floors for the equator-band sample of one arm.
const MIN_BAND_PIXELS: usize = 2000;
const MIN_PIXELS_PER_BIN: usize = 100;

/// The floor plane of the contact fixture (the sphere rests on it).
const FLOOR_Y: f32 = -1.0;

// ---- vector helpers ------------------------------------------------------------------------

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

// ---- the mesh --------------------------------------------------------------------------------

/// A UV sphere: positions, outward vertex normals, triangles, and per-cell triangle ranges.
struct UvSphere {
    radius: f32,
    stacks: u32,
    slices: u32,
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
    /// `(first triangle, count)` of each `(stack, slice)` cell, row-major by stack.
    cells: Vec<(usize, usize)>,
}

/// The geometry of `crates/boyko_app/tests/common/mod.rs`'s `uv_sphere` (lines 28-63), copied
/// verbatim: the same `phi`/`theta` parametrisation, the same `f32` `sin_cos`, the same index
/// loop with the same skipped pole-fan triangles. The `Vertex` colour / UV / tangent fields do
/// not shape the surface and are left out; the per-cell triangle ranges are recorded for the
/// ray caster.
fn uv_sphere(radius: f32, stacks: u32, slices: u32) -> UvSphere {
    let pi = core::f32::consts::PI;
    let mut pos = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    let mut nrm = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            pos.push([n[0] * radius, n[1] * radius, n[2] * radius]);
            nrm.push(n);
        }
    }
    let stride = slices + 1;
    let mut tris = Vec::with_capacity((stacks * slices * 2) as usize);
    let mut cells = Vec::with_capacity((stacks * slices) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let first = tris.len();
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            if i != 0 {
                tris.push([a, a + 1, b]);
            }
            if i != stacks - 1 {
                tris.push([a + 1, b + 1, b]);
            }
            cells.push((first, tris.len() - first));
        }
    }
    UvSphere {
        radius,
        stacks,
        slices,
        pos,
        nrm,
        tris,
        cells,
    }
}

/// The `(stack, slice)` cell a point near the sphere surface falls in.
fn cell_of(s: &UvSphere, p: [f32; 3]) -> (i64, i64) {
    let pi = core::f32::consts::PI;
    let len = dot(p, p).sqrt().max(1.0e-12);
    let phi = (p[1] / len).clamp(-1.0, 1.0).acos();
    let mut theta = p[2].atan2(p[0]);
    if theta < 0.0 {
        theta += 2.0 * pi;
    }
    let i = ((phi / pi) * s.stacks as f32).floor() as i64;
    let j = ((theta / (2.0 * pi)) * s.slices as f32).floor() as i64;
    (
        i.clamp(0, i64::from(s.stacks) - 1),
        j.rem_euclid(i64::from(s.slices)),
    )
}

/// One surface hit: the ray parameter, the world position, and the barycentric-interpolated,
/// normalized normal (before the oct8 store).
#[derive(Clone, Copy)]
struct Hit {
    t: f32,
    p: [f32; 3],
    n: [f32; 3],
    on_floor: bool,
}

/// Möller–Trumbore against one triangle; `(t, u, v)` with the hit at `(1-u-v) v0 + u v1 + v v2`.
fn ray_triangle(
    ro: [f32; 3],
    rd: [f32; 3],
    v0: [f32; 3],
    v1: [f32; 3],
    v2: [f32; 3],
) -> Option<(f32, f32, f32)> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let pv = cross(rd, e2);
    let det = dot(e1, pv);
    if det.abs() < 1.0e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = sub(ro, v0);
    let u = dot(tv, pv) * inv;
    if !(-1.0e-6..=1.0 + 1.0e-6).contains(&u) {
        return None;
    }
    let qv = cross(tv, e1);
    let v = dot(rd, qv) * inv;
    if v < -1.0e-6 || u + v > 1.0 + 1.0e-6 {
        return None;
    }
    let t = dot(e2, qv) * inv;
    (t > 1.0e-6).then_some((t, u, v))
}

/// The nearest triangle hit among the 3x3 cells around `(ci, cj)`.
fn hit_window(s: &UvSphere, ro: [f32; 3], rd: [f32; 3], ci: i64, cj: i64, best: &mut Option<Hit>) {
    for di in -1..=1i64 {
        let i = ci + di;
        if i < 0 || i >= i64::from(s.stacks) {
            continue;
        }
        for dj in -1..=1i64 {
            let j = (cj + dj).rem_euclid(i64::from(s.slices));
            let (first, count) = s.cells[(i * i64::from(s.slices) + j) as usize];
            for tri in &s.tris[first..first + count] {
                let [a, b, c] = tri.map(|k| k as usize);
                if let Some((t, u, v)) = ray_triangle(ro, rd, s.pos[a], s.pos[b], s.pos[c]) {
                    if best.is_some_and(|h| h.t <= t) {
                        continue;
                    }
                    let w = 1.0 - u - v;
                    let n = normalize([
                        w * s.nrm[a][0] + u * s.nrm[b][0] + v * s.nrm[c][0],
                        w * s.nrm[a][1] + u * s.nrm[b][1] + v * s.nrm[c][1],
                        w * s.nrm[a][2] + u * s.nrm[b][2] + v * s.nrm[c][2],
                    ]);
                    let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                    *best = Some(Hit {
                        t,
                        p,
                        n,
                        on_floor: false,
                    });
                }
            }
        }
    }
}

/// The mesh hit of a ray: the vertices lie ON the radius, so every facet is inside the analytic
/// sphere and the mesh hit sits near the analytic entry point. The 3x3 cell window around the
/// entry is tried first; a ray that misses it (near the silhouette) tries windows further along
/// the chord, up to its closest approach to the centre.
fn cast_sphere(s: &UvSphere, ro: [f32; 3], rd: [f32; 3]) -> Option<Hit> {
    let r = s.radius * 1.0001;
    let b = dot(ro, rd);
    let c = dot(ro, ro) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t0 = -b - disc.sqrt();
    let t_mid = -b;
    if t_mid <= 0.0 {
        return None;
    }
    let mut best = None;
    for k in 0..4 {
        let t = t0 + (t_mid - t0) * (k as f32 / 3.0);
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let (ci, cj) = cell_of(s, p);
        hit_window(s, ro, rd, ci, cj, &mut best);
        if best.is_some() {
            break;
        }
    }
    best
}

/// The hit of a ray against the floor plane `y = FLOOR_Y`.
fn cast_floor(ro: [f32; 3], rd: [f32; 3]) -> Option<Hit> {
    if rd[1] >= -1.0e-6 {
        return None;
    }
    let t = (FLOOR_Y - ro[1]) / rd[1];
    (t > 1.0e-6).then(|| Hit {
        t,
        p: [ro[0] + rd[0] * t, FLOOR_Y, ro[2] + rd[2] * t],
        n: [0.0, 1.0, 0.0],
        on_floor: true,
    })
}

/// The producers' 8-bit octahedral normal store: the eDSL `oct_encode` (the body the HLSL
/// `oct_encode` is spliced from), then the UNORM8 round-half-up.
fn oct8(n: [f32; 3]) -> [u8; 2] {
    let ret = Cell::new([0.0f32; 2]);
    let _ = oct_encode_body::<EvalCf>(n, &ret);
    ret.get()
        .map(|e| (e.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8)
}

/// A ray-cast G-buffer: the SSAO inputs plus the per-pixel hit (for the pixel selection).
struct GBuffer {
    w: u32,
    h: u32,
    camera: CompositeCamera,
    attrs: Vec<MarcherAttributes>,
    hits: Vec<Option<Hit>>,
}

fn build_gbuffer(
    s: &UvSphere,
    with_floor: bool,
    w: u32,
    h: u32,
    camera: CompositeCamera,
) -> GBuffer {
    let background = MarcherAttributes {
        base_rgb: [0, 0, 0],
        oct_rg: [128, 128],
        mat_id: 0,
        shadow: 255,
        ao: 255,
        mask: 0,
        view_t: 1.0e30,
    };
    let mut attrs = Vec::with_capacity((w * h) as usize);
    let mut hits = Vec::with_capacity((w * h) as usize);
    for py in 0..h {
        for px in 0..w {
            let (ro, rd) = composite_pixel_ray(px, py, w, h, camera);
            let mut hit = cast_sphere(s, ro, rd);
            if with_floor
                && let Some(f) = cast_floor(ro, rd)
                && hit.is_none_or(|m| f.t < m.t)
            {
                hit = Some(f);
            }
            attrs.push(match hit {
                Some(hit) => MarcherAttributes {
                    base_rgb: [200, 200, 200],
                    oct_rg: oct8(hit.n),
                    mat_id: 0,
                    shadow: 255,
                    ao: 255,
                    mask: 1,
                    view_t: hit.t,
                },
                None => background,
            });
            hits.push(hit);
        }
    }
    GBuffer {
        w,
        h,
        camera,
        attrs,
        hits,
    }
}

fn perspective_camera() -> CompositeCamera {
    CompositeCamera::Perspective {
        eye: [0.0, 0.0, 4.2],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov: 22.5f32.to_radians().tan(),
        aspect: 1.0,
    }
}

// ---- the slice-phase measurement ---------------------------------------------------------

/// One arm's measurement: the SSAO factor over the equator band, binned by slice phase.
struct ArmReport {
    name: String,
    pixels: usize,
    below_one: usize,
    min_ao: f32,
    bin_sum: [f64; PHASE_BINS],
    bin_count: [usize; PHASE_BINS],
}

impl ArmReport {
    fn bin_means(&self) -> [f64; PHASE_BINS] {
        let mut m = [0.0; PHASE_BINS];
        for (k, v) in m.iter_mut().enumerate() {
            *v = self.bin_sum[k] / self.bin_count[k].max(1) as f64;
        }
        m
    }

    /// The mean of the two slice-edge bins minus the mean of the two centre bins.
    fn edge_minus_centre(&self) -> f64 {
        let m = self.bin_means();
        (m[0] + m[PHASE_BINS - 1]) * 0.5 - (m[PHASE_BINS / 2 - 1] + m[PHASE_BINS / 2]) * 0.5
    }

    fn summary(&self) -> String {
        let means = self.bin_means();
        let bins: Vec<String> = means
            .iter()
            .zip(self.bin_count)
            .map(|(m, c)| format!("{m:.4}(n={c})"))
            .collect();
        format!(
            "{}: {} band pixels, {} below 1.0, min {:.4}, edge - centre {:+.4}, phase bins [{}]",
            self.name,
            self.pixels,
            self.below_one,
            self.min_ao,
            self.edge_minus_centre(),
            bins.join(", ")
        )
    }
}

/// Measures the SSAO factor over the sphere's equator band (`|lat| < 30°`, `NoV > 0.35`),
/// every `stride`-th pixel in each axis, binned by `frac(theta / (2 pi / slices))`.
fn measure_band(name: &str, s: &UvSphere, g: &GBuffer, quality: usize, stride: u32) -> ArmReport {
    let tau = 2.0 * core::f32::consts::PI;
    let facet = tau / s.slices as f32;
    let mut r = ArmReport {
        name: name.to_string(),
        pixels: 0,
        below_one: 0,
        min_ao: 1.0,
        bin_sum: [0.0; PHASE_BINS],
        bin_count: [0; PHASE_BINS],
    };
    for py in (0..g.h).step_by(stride as usize) {
        for px in (0..g.w).step_by(stride as usize) {
            let Some(hit) = g.hits[(py * g.w + px) as usize] else {
                continue;
            };
            if hit.on_floor {
                continue;
            }
            let (_, rd) = composite_pixel_ray(px, py, g.w, g.h, g.camera);
            let nov = -dot(hit.n, rd);
            let lat = (hit.p[1] / dot(hit.p, hit.p).sqrt()).asin();
            if lat.abs() >= 30.0f32.to_radians() || nov <= 0.35 {
                continue;
            }
            let ao = golden_ssao_attributes_derived(
                &g.attrs,
                px,
                py,
                g.w,
                g.h,
                g.camera,
                &SSAO_PARAMS[quality],
            );
            let mut theta = hit.p[2].atan2(hit.p[0]);
            if theta < 0.0 {
                theta += tau;
            }
            let phase = (theta / facet).fract();
            let bin = ((phase * PHASE_BINS as f32) as usize).min(PHASE_BINS - 1);
            r.pixels += 1;
            r.bin_sum[bin] += f64::from(ao);
            r.bin_count[bin] += 1;
            if ao < 1.0 {
                r.below_one += 1;
            }
            r.min_ao = r.min_ao.min(ao);
        }
    }
    r
}

/// The largest angle between a triangle's flat normal and the interpolated vertex normal over
/// its corners (where the interpolated normal deviates most), in radians.
fn max_facet_tilt(s: &UvSphere) -> f32 {
    let mut worst = 0.0f32;
    for tri in &s.tris {
        let [a, b, c] = tri.map(|k| k as usize);
        let mut fnrm = normalize(cross(sub(s.pos[b], s.pos[a]), sub(s.pos[c], s.pos[a])));
        let centroid = [
            s.pos[a][0] + s.pos[b][0] + s.pos[c][0],
            s.pos[a][1] + s.pos[b][1] + s.pos[c][1],
            s.pos[a][2] + s.pos[b][2] + s.pos[c][2],
        ];
        if dot(fnrm, centroid) < 0.0 {
            fnrm = [-fnrm[0], -fnrm[1], -fnrm[2]];
        }
        for k in [a, b, c] {
            worst = worst.max(dot(fnrm, s.nrm[k]).clamp(-1.0, 1.0).acos());
        }
    }
    worst
}

// ---- the tests -----------------------------------------------------------------------------

/// Fixture validity: the showcase sphere's largest facet tilt, as a sine, plus the oct8 normal
/// error, is the largest false elevation any same-facet tap can show (a tap on another facet of
/// a convex mesh lies below the centre facet's plane, so it shows less). It must stay below the
/// HBAO+ bias, or the bias cannot remove the facet pattern on this mesh.
#[test]
fn the_fixture_tilt_is_inside_the_hbao_plus_margin() {
    let showcase = uv_sphere(1.0, SHOWCASE_STACKS, SHOWCASE_SLICES);
    let tilt = max_facet_tilt(&showcase);
    let bound = tilt.sin() + OCT8_MAX_SIN_ERROR;
    // `vb_mesh_ssao`'s sphere (`tests/vb_mesh_ssao.rs`), printed for the coverage record only.
    let vb = uv_sphere(0.62, 28, 40);
    let vb_tilt = max_facet_tilt(&vb);
    println!(
        "uv_sphere(1.0, 48, 64): max tilt {:.3} deg, sin + oct8 = {bound:.4}; \
         uv_sphere(0.62, 28, 40): max tilt {:.3} deg, sin + oct8 = {:.4}; bias {HBAO_PLUS_ANGLE_BIAS}",
        tilt.to_degrees(),
        vb_tilt.to_degrees(),
        vb_tilt.sin() + OCT8_MAX_SIN_ERROR
    );
    assert!(
        bound < HBAO_PLUS_ANGLE_BIAS,
        "the showcase sphere's false elevation bound {bound} (tilt {} deg + oct8) must stay \
         below the angle bias {HBAO_PLUS_ANGLE_BIAS}",
        tilt.to_degrees()
    );
}

/// The defect: a convex sphere occludes nothing on its own equator band, so the SSAO factor
/// there must be exactly 1.0 and flat across the slice phase. Without an angle bias, taps on
/// the centre pixel's own facet rise above the interpolated normal's tangent plane and the AO
/// darkens toward the facet edges.
#[test]
fn a_tessellated_sphere_shows_no_facet_locked_ssao() {
    let mut reports = Vec::new();

    // Arm P: the showcase sphere from 4.2 units, 45° vertical FOV, 1024². The world radius 0.5
    // spans ~193 px here, so the 24-px clamp (`SSAO_RADIUS_PIX_MAX`) binds: 0.062 world.
    let sphere = uv_sphere(1.0, SHOWCASE_STACKS, SHOWCASE_SLICES);
    let g = build_gbuffer(&sphere, false, 1024, 1024, perspective_camera());
    for (name, quality) in [
        ("P/Low", SSAO_QUALITY_LOW),
        ("P/Medium", SSAO_QUALITY_MEDIUM),
        ("P/High", SSAO_QUALITY_HIGH),
    ] {
        reports.push(measure_band(name, &sphere, &g, quality, 4));
    }

    // Arm O: ortho (half-extent 1), 512², a 0.8-radius sphere, High. Ortho has NO pixel clamp:
    // the full 0.5-world kernel (128 px, ~6.4 facets) — if the facet pattern survives here, the
    // clamp is not its cause.
    let sphere_o = uv_sphere(0.8, SHOWCASE_STACKS, SHOWCASE_SLICES);
    let g_o = build_gbuffer(&sphere_o, false, 512, 512, CompositeCamera::Ortho);
    reports.push(measure_band(
        "O/High",
        &sphere_o,
        &g_o,
        SSAO_QUALITY_HIGH,
        3,
    ));

    for r in &reports {
        println!("{}", r.summary());
    }

    // Anti-vacuity: each arm must sample the band, and every phase bin must be populated.
    for r in &reports {
        assert!(
            r.pixels >= MIN_BAND_PIXELS,
            "fixture: {} sampled only {} band pixels (< {MIN_BAND_PIXELS}) — a ray-caster defect",
            r.name,
            r.pixels
        );
        assert!(
            r.bin_count.iter().all(|&c| c >= MIN_PIXELS_PER_BIN),
            "fixture: {} has a phase bin under {MIN_PIXELS_PER_BIN} pixels: {:?}",
            r.name,
            r.bin_count
        );
    }

    let failing: Vec<String> = reports
        .iter()
        .filter(|r| {
            let means = r.bin_means();
            let spread = means.iter().copied().fold(f64::MIN, f64::max)
                - means.iter().copied().fold(f64::MAX, f64::min);
            r.below_one > 0 || spread != 0.0
        })
        .map(ArmReport::summary)
        .collect();
    assert!(
        failing.is_empty(),
        "a convex tessellated sphere must show NO screen-space occlusion on its equator band \
         (every AO == 1.0, phase bins flat); facet-locked false horizons in:\n  {}",
        failing.join("\n  ")
    );
}

/// Non-vacuity: the floor right under a resting sphere is genuinely occluded, and stays dark.
#[test]
fn contact_ao_under_a_resting_sphere_survives() {
    let sphere = uv_sphere(1.0, SHOWCASE_STACKS, SHOWCASE_SLICES);
    let g = build_gbuffer(&sphere, true, 1024, 1024, perspective_camera());
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for py in 0..g.h {
        for px in 0..g.w {
            let Some(hit) = g.hits[(py * g.w + px) as usize] else {
                continue;
            };
            if !hit.on_floor || (hit.p[0] * hit.p[0] + hit.p[2] * hit.p[2]).sqrt() >= 0.3 {
                continue;
            }
            let ao = golden_ssao_attributes_derived(
                &g.attrs,
                px,
                py,
                g.w,
                g.h,
                g.camera,
                &SSAO_PARAMS[SSAO_QUALITY_HIGH],
            );
            sum += f64::from(ao);
            count += 1;
        }
    }
    let mean = sum / count.max(1) as f64;
    println!("contact floor pixels within 0.3 of the contact: {count}, mean AO {mean:.4}");
    assert!(
        count >= 100,
        "fixture: only {count} visible floor pixels within 0.3 of the contact (< 100)"
    );
    assert!(
        mean < 0.9,
        "the floor under a resting sphere must stay occluded: mean AO {mean} >= 0.9 over {count} px"
    );
}
