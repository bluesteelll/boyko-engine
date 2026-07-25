//! VB-P1e rung H3 — the GPU set-level equality and memory-safety oracle
//! (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §8.10, `[P0-1]`/`[P0-2]`/`[P0-3]`).
//!
//! **Scope, stated up front: `[P0-1]`, `[P0-2]` and `[P0-3]` are NOT closed by this rung.** This
//! file lands only the GREEN half of the plan's own mutation protocol (§8.3) — the shipped,
//! unmutated shaders driven across the grid, plus every RED demonstration this sandbox can
//! actually run (assertion 1 on real hardware; assertions 5, 6, 7, 8 on hand-built synthetic
//! buffers). The plan's twelve named mutations, (i) through (xii), are UNEXECUTED here: each one
//! needs a `cluster_cull.hlsl` edit re-compiled through `dxc` and driven on a GPU, and that is a
//! separate session from this one. Closing `[P0-1]`/`[P0-2]`/`[P0-3]` requires running that
//! protocol on device; until then, this file is evidence the detectors do not false-positive on
//! correct code, not evidence that they catch the faults they were built to catch.
//!
//! A CULL-ONLY driver (camera UBO + light table + the three cull buffers + one dispatch per
//! arm + readbacks) that arms BOTH the shipped base cluster-cull SPIR-V (`cluster_cull_spirv()`,
//! `[numthreads(64,1,1)]`, the 16-byte [`ClusterCullPush`]) and the shipped `-D HIER=1` variant
//! (`cluster_cull_hier_spirv()`, `[numthreads(256,1,1)]`, the 24-byte [`ClusterCullHierPush`] —
//! §8.9's H2 "dark infra", never selected by any production path). It does NOT go through
//! `run_gbuffer_hybrid_lit_clustered` (`sdf_gbuffer_hybrid.rs:5279`) — no SDF, no resolve, and it
//! can drive a PERSPECTIVE camera trivially (§8.10).
//!
//! **This rung creates a pipeline INSIDE this test only** — no host wiring, no render-path
//! change, no golden moves. [`ClusterCullHierPush`] has no production Rust mirror yet (H2 built
//! the shader; nothing arms it), so the 24-byte push type lives here, matching
//! `cluster_cull.hlsl`'s `#ifdef HIER` push tail field-for-field.
//!
//! # The ten assertions (§8.2, §8.3, §8.10)
//!
//! Every assertion below states the concrete mutation that turns it RED, per the plan's own
//! rule ("an assertion is only listed if a concrete mutation was constructed AND simulated to
//! confirm it turns the assertion red" — §8). **Assertions 5, 6, 7 and 8 are shader-level
//! detectors** whose red mutations ((i), (v)/(vi), a duplicate write, (iv)) require a re-DXC'd
//! HLSL variant this file does not build (this rung cannot run a GPU from this sandbox at all,
//! let alone spawn `dxc.exe` against a modified source) — their failability is instead proven
//! on **synthetic readback buffers** in `mod tests` below (hand-built buffers that encode each
//! fault), per the plan's own precedent for CPU-provable properties. The remaining assertions
//! (1, 2, 3, 9, 10) are exercised synthetically AND checked GREEN on every device run across the
//! whole matrix — but of those, only assertion 1 has actually been DRIVEN RED on hardware in this
//! rung (`cull_hier_equiv_saturation_precondition_fires_red_on_device`; no shader edit is needed
//! there, since `index_list_cap` is a runtime push constant). Assertions 9 and 10 pass on every
//! device run recorded here; their RED path is demonstrated only synthetically, in `mod tests`.
//!
//! Evaluation discipline (§8.2(A) limit 4, Rev 6): assertions 1, 5, 6, 7, 8, 10 are evaluated
//! PER ARM on that arm's own readback, taken after that arm's OWN dispatch and before the next
//! arm's pre-fill; assertions 2, 3, 4 compare two SEPARATELY captured single-arm readbacks;
//! assertion 9 is structural. [`run_cull_arm`] allocates fresh `ClusterGrid` / `LightIndexList` /
//! `LightIndexAlloc` buffers per arm (never shared across the two arms of one config) precisely
//! so no assertion can mix one arm's `ClusterGrid` offsets with the other arm's `LightIndexList`.
//!
//! # What this file does NOT do
//!
//! It never arms the hierarchical variant on any production path, never runs a GPU executable
//! from this development sandbox (every `#[test]` below degrades to a printed SKIP via
//! [`boot_or_skip`] when no device is available — see `cargo test`'s output), and never re-DXCs
//! a mutated shader (the mutation protocol's shader-level rows — (i), (ii), (iii), (v), (vi),
//! (vii), (ix)-(xii) — are out of scope for this rung; see the doc comment on each assertion
//! function for exactly which mutation it discharges and how).
//!
//! It also does not add the plan's `packed_dims == 0` degenerate-header probe (§8.10's "GPU
//! matrix" bullet) — an EXPLICIT scope decision, not a silent omission. That config pushes a
//! `cluster_dims_packed` of zero while dispatching real threads over it, so the shader's own
//! `x = s % dim_x` thread map divides by zero on the GPU; integer divide-by-zero is undefined on
//! most drivers (hang or TDR, not a clean fault), and [`run_cull_arm`]'s
//! `ctx.wait_fence(&fence, u64::MAX)` has no timeout, so a hang there blocks the whole test
//! process rather than failing loudly. It is deferred to the session that runs the mutation
//! protocol on device (a `wait_fence` timeout, or a `dxc`-side guard, needs to land first). It
//! DOES add the plan's other matrix addition, the `N = 0` row (see
//! [`MatrixCase::expect_vacuous`]): zero point/spot lights is a single safe dispatch (`ps_n == 0`,
//! no division by a swept dimension), and it is the only path in the matrix that exercises the
//! hierarchical arm's three barriers with an empty coarse loop and an empty fine walk.

mod common;
use common::*;

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, ComputePipelineDesc, DescriptorKind, MemoryLocation, RhiCommandEncoder,
    RhiDevice, RhiQueue, ShaderStage,
};
use boyko_rhi_vulkan::compute::{
    CLUSTER_CULL_PUSH_BYTES, COMPOSITE_PUSH_CONSTANT_BYTES, ClusterCullPush, CompositeCamera,
    CompositePushConstants, GOLDEN_LIGHT_HEADER_BASE_WORDS, SDF_IMG_H, SDF_IMG_W,
    cluster_cull_hier_spirv, cluster_cull_spirv,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::goldens::{
    GoldenClusterConfig, GoldenLight, GoldenLightHeader, HIER_GROUP_THREADS, HIER_MASK_BITS,
    golden_cluster_cull, golden_hier_groups_per_slice,
};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{ComputePipeline, VulkanBindGroupLayout, VulkanShaderModule};

// ============================================================================
// The 24-byte hierarchical-arm push constant (D11) — no production Rust mirror exists yet.
// ============================================================================

/// The 24-byte hierarchical-arm push constant (VB-P1e D11, `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`):
/// the base [`ClusterCullPush`] (16 B: `z_near`, `z_far`, `max_lights_per_cluster`,
/// `index_list_cap`) widened by two BOOT-snapshot words the `-D HIER=1` arm reads instead of
/// re-deriving its dims/capacity from the live light-table header
/// (`shaders/cluster_cull.hlsl`'s `#ifdef HIER` push-constant tail, lines 76-83). No production
/// Rust mirror of this struct exists (H2 built the shader; nothing arms it) — this rung's driver
/// is the first Rust caller, so the type lives here rather than in `boyko_rhi_vulkan::compute`
/// (this rung creates the hierarchical pipeline INSIDE the test only; no host wiring).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClusterCullHierPush {
    z_near: f32,
    z_far: f32,
    max_lights_per_cluster: u32,
    index_list_cap: u32,
    /// `dim_x | dim_y<<8 | dim_z<<16` — the boot-snapshot dims D11 requires (never re-derived
    /// from the live header behind this dispatch).
    cluster_dims_packed: u32,
    /// `cluster_count()` in full precision at BOOT — the hard device-write bound.
    cluster_capacity: u32,
}

/// Byte size of [`ClusterCullHierPush`] — the hierarchical cull pipeline's declared COMPUTE push
/// range (24 bytes).
const CLUSTER_CULL_HIER_PUSH_BYTES: u32 = core::mem::size_of::<ClusterCullHierPush>() as u32;

const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, z_near) == 0);
const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, z_far) == 4);
const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, max_lights_per_cluster) == 8);
const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, index_list_cap) == 12);
const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, cluster_dims_packed) == 16);
const _: () = assert!(core::mem::offset_of!(ClusterCullHierPush, cluster_capacity) == 20);
const _: () =
    assert!(CLUSTER_CULL_HIER_PUSH_BYTES == 24, "ClusterCullHierPush must be 24 bytes");

impl ClusterCullHierPush {
    /// Builds the hierarchical-arm push from the base cull parameters + D11's boot snapshot.
    #[inline]
    const fn new(
        z_near: f32,
        z_far: f32,
        max_lights_per_cluster: u32,
        index_list_cap: u32,
        cluster_dims_packed: u32,
        cluster_capacity: u32,
    ) -> Self {
        Self {
            z_near,
            z_far,
            max_lights_per_cluster,
            index_list_cap,
            cluster_dims_packed,
            cluster_capacity,
        }
    }

    /// Re-views the push constants as their raw 24-byte slice for `push_constants`.
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `f32`/`u32` fields (all `Copy`), every offset
        // and the 24-byte total pinned by the const-asserts above (no uninit padding), so its
        // `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern. The `&self`
        // borrow keeps the struct alive for the slice's lifetime; the slice is read-only.
        unsafe {
            core::slice::from_raw_parts(
                (self as *const Self).cast::<u8>(),
                core::mem::size_of::<Self>(),
            )
        }
    }
}

// ============================================================================
// Device boot (the crate's established skip-if-no-device idiom, copied here since integration
// test binaries cannot share private helpers across files).
// ============================================================================

/// Boots a validation-enabled headless context, or returns `None` — with a SKIP log printed to
/// stderr, so a skipped run is never mistaken for a passing one — when no GPU / loader /
/// validation layer / dynamic-rendering is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig { enable_validation: true, ..InstanceConfig::default() }) {
        Ok(ctx) => {
            println!("[{test}] Vulkan device: {}", ctx.device_name());
            if !ctx.validation_enabled() {
                eprintln!(
                    "[{test}] NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — the H3 \
                     device assertions still run"
                );
            }
            Some(ctx)
        }
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages, a no-op (with a note) when
/// validation is disabled (`BOYKO_DISABLE_VALIDATION`).
fn assert_validation_clean(ctx: &VulkanContext, test: &str) {
    if !ctx.validation_enabled() {
        eprintln!("[{test}] NOTE: validation disabled — skipping the clean-oracle assert");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "[{test}] validation layer reported {} message(s) during the H3 cull-only run",
        state.total()
    );
}

// ============================================================================
// Fixtures: the grid matrix, the camera rigs, the light rigs, and light-table packing.
// ============================================================================

/// The VB-P1d bench camera's square render target — mirrors
/// `tests/lighting_l1_host_oracle.rs`'s own `IMG` fixture.
const BENCH_IMG: u32 = 512;

/// `boyko_render::light::MAX_LIGHTS` mirrored via [`HIER_MASK_BITS`] (D6's pinned equality
/// `MAX_LIGHTS == HIER_MASK_WORDS * 32`, `crates/boyko_render/src/light.rs:65-69`) — the vulkan
/// crate cannot depend on `boyko_render` (`goldens.rs`'s own `GoldenClusterConfig` doc explains
/// why), so this reuses the existing mirror rather than adding a third literal.
const MAX_LIGHTS: u32 = HIER_MASK_BITS;

/// Driver requirement 2 (§8.10): the light table is allocated at `MAX_LIGHTS + 1024` rows.
const POISON_TAIL_ROWS: u32 = MAX_LIGHTS + 1024;

/// The `ClusterGrid`/`LightIndexList` pre-fill sentinel word (§8.2(A)) — every 32-bit word of a
/// cell that has not yet been written.
const SENTINEL_WORD: u32 = 0xFFFF_FFFF;
const SENTINEL_CELL: (u32, u32) = (SENTINEL_WORD, SENTINEL_WORD);

fn v_norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn v_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The VB-P1d bench camera fixture — mirrors `tests/lighting_l1_host_oracle.rs`'s own `camera()`
/// (a self-contained copy: integration test binaries cannot share private helpers across files):
/// eye `(0, 1.1, 7.8)` looking at `(0, 0.55, 0)`, 52-degree vertical FOV, aspect 1.0 (a
/// `BENCH_IMG x BENCH_IMG` square target). Returns the matching `(CompositeCamera,
/// CompositePushConstants)` pair built from the SAME basis + FOV expression, so the host oracle
/// and the GPU push are bit-identical BY CONSTRUCTION, not merely equivalent.
fn bench_camera() -> (CompositeCamera, CompositePushConstants) {
    let eye = [0.0, 1.1, 7.8];
    let look_at = [0.0, 0.55, 0.0];
    let forward = v_norm([look_at[0] - eye[0], look_at[1] - eye[1], look_at[2] - eye[2]]);
    let right = v_norm(v_cross(forward, [0.0, 1.0, 0.0]));
    let up = v_cross(right, forward);
    let fov_y = 52.0_f32.to_radians();
    let push = CompositePushConstants::perspective(eye, forward, right, up, fov_y, BENCH_IMG, BENCH_IMG);
    let camera = CompositeCamera::Perspective {
        eye,
        forward,
        right,
        up,
        tan_half_fov: (fov_y * 0.5).tan(),
        aspect: 1.0,
    };
    (camera, push)
}

/// The ray origin the shared `composite_ray`/`ray_gen.hlsli` ray-gen uses for a given camera —
/// `[0,0,2]` for [`CompositeCamera::Ortho`] (the fixed M1 fixture's `RO`, mirroring
/// `tests/lighting_l1_host_oracle.rs`'s own `RO` constant) or the PERSPECTIVE `eye` field
/// otherwise. Used to place the exactly-once permutation probe's light and the light-table
/// poison tail (driver requirement 2, §8.10) at a point every froxel's AABB is guaranteed close
/// to (well under the probe/poison `range = 1e6`).
fn camera_eye(camera: CompositeCamera) -> [f32; 3] {
    match camera {
        CompositeCamera::Ortho => [0.0, 0.0, 2.0],
        CompositeCamera::Perspective { eye, .. } => eye,
    }
}

/// A small, deterministic in-world light rig for the M1 ORTHO fixture (world extent
/// `SDF_HALF_EXTENT == 1`, matching `tests/lighting_l1_host_oracle.rs`'s
/// `cull_keeps_an_in_range_light_and_drops_an_out_of_range_one` scale): 2 global (`l0a`) lights +
/// 4 point/spot lights placed inside `[-1, 1]` so several froxels are non-empty (assertion 9).
fn ortho_lights() -> (Vec<GoldenLight>, Vec<GoldenLight>) {
    let l0a = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    let ps = vec![
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0),
        GoldenLight::point([0.5, 0.3, 0.2], [1.0, 1.0, 1.0], 100.0, 2.0),
        GoldenLight::spot([-0.4, 0.2, 0.5], [0.0, 0.0, -1.0], [1.0, 1.0, 1.0], 3000.0, 3.0, 15.0, 30.0),
        GoldenLight::point([0.2, -0.5, -0.3], [1.0, 1.0, 1.0], 80.0, 2.5),
    ];
    (l0a, ps)
}

/// Mirrors `tests/lighting_l1_host_oracle.rs`'s `light_position`/`light_range` (a self-contained
/// copy) pinned at that file's fixed `BENCH_DEFAULT_N_PS` placement volume — the "dense
/// in-frustum" rig (that file's `lights_for_in_frustum`), so occupancy stays non-vacuous at a
/// modest light count rather than collapsing as the placement volume grows with the light count
/// (§1.4's defect — irrelevant here, since this rung is about correctness, not the selectivity
/// measurement H1 already pins).
fn bench_light_position(i: u32) -> [f32; 3] {
    let half_x = 4.5_f32;
    let y_min = 0.3_f32;
    let y_span = 3.3_f32;
    let z_min = -2.0_f32;
    let z_span = 6.0_f32;
    let t = f64::from(i);
    let fx = (t * 0.618_033_988_75).fract() as f32;
    let fy = (t * 0.381_966_011_25).fract() as f32;
    let fz = (t * 0.236_067_977_5).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

fn bench_light_range(i: u32) -> f32 {
    1.2 + ((f64::from(i) * 0.142_857).fract() as f32) * 0.8
}

/// `n_ps` point/spot lights via [`bench_light_position`]/[`bench_light_range`] (every 4th a
/// downward spot, the rest points) + the same 2 global (`l0a`) lights [`ortho_lights`] uses.
fn bench_lights(n_ps: u32) -> (Vec<GoldenLight>, Vec<GoldenLight>) {
    let l0a = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    let mut ps = Vec::with_capacity(n_ps as usize);
    for i in 0..n_ps {
        let p = bench_light_position(i);
        let r = bench_light_range(i);
        if i % 4 == 3 {
            ps.push(GoldenLight::spot(p, [0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 65.0, r, 15.0, 30.0));
        } else {
            ps.push(GoldenLight::point(p, [1.0, 1.0, 1.0], 65.0, r));
        }
    }
    (l0a, ps)
}

/// Serializes `(header, lights)` into the std430 word-packed `[LightHeaderGpu (16w) ||
/// GpuLight[] (12w each)]` layout `LightBuf`/`light_table.hlsli` expects — an integration-test
/// self-contained copy of `sdf_gbuffer_hybrid.rs`'s private `pack_light_table` (integration test
/// binaries cannot share private helpers across files).
fn pack_light_table(header: &GoldenLightHeader, lights: &[GoldenLight]) -> Vec<u32> {
    let mut words = vec![0u32; GOLDEN_LIGHT_HEADER_BASE_WORDS + lights.len() * 12];
    let lanes = [header.counts_exposure, header.sky_diffuse, header.sky_spec, header.cluster_params];
    for (li, lane) in lanes.iter().enumerate() {
        for (c, &v) in lane.iter().enumerate() {
            words[li * 4 + c] = v.to_bits();
        }
    }
    for (i, l) in lights.iter().enumerate() {
        let base = GOLDEN_LIGHT_HEADER_BASE_WORDS + i * 12;
        for (c, &v) in l.dir_kind.iter().enumerate() {
            words[base + c] = v.to_bits();
        }
        for (c, &v) in l.pos_range.iter().enumerate() {
            words[base + 4 + c] = v.to_bits();
        }
        for (c, &v) in l.color_cone.iter().enumerate() {
            words[base + 8 + c] = v.to_bits();
        }
    }
    words
}

/// Driver requirement 2 (§8.10): builds the light table at [`POISON_TAIL_ROWS`] (`MAX_LIGHTS +
/// 1024`) rows, every row `>= light_count` filled with the POISON light (`POINT`, at `eye`,
/// `range = 1e6` — a light every froxel WOULD accept, so a producer mutation that reads past
/// `light_count` shows up as an ACCEPTED index rather than silence, assertion 8). Returns
/// `(header, full_lights, packed_words)`. `full_lights` (real + poison) is fed to
/// [`golden_cluster_cull`] too: rows past `header.light_count()` are never indexed by the
/// oracle's own `[l0a_count..light_count)` loop, so passing the SAME oversized table to both the
/// host oracle and the GPU is sound, not a shortcut. **This construction's own armed-ness — that
/// the poison row IS accepted by the shipped cull geometry, not merely that the assertion
/// function can detect an out-of-range index — is proven separately by
/// `mod tests::poison_row_is_accepted_by_the_unmutated_cull_geometry` (`[P1-2]`).**
fn build_light_table(
    l0a: &[GoldenLight],
    ps: &[GoldenLight],
    eye: [f32; 3],
    cfg: &GoldenClusterConfig,
) -> (GoldenLightHeader, Vec<GoldenLight>, Vec<u32>) {
    let header = GoldenLightHeader::new_clustered(l0a.len() as u32, ps.len() as u32, 1.0, cfg);
    let mut lights: Vec<GoldenLight> = l0a.iter().chain(ps.iter()).copied().collect();
    let total_rows = POISON_TAIL_ROWS as usize;
    debug_assert!(
        lights.len() <= total_rows,
        "invariant: real light rows must fit under the poison-tail row budget"
    );
    let poison = GoldenLight::point(eye, [1.0, 1.0, 1.0], 0.0, 1.0e6);
    lights.resize(total_rows, poison);
    let words = pack_light_table(&header, &lights);
    (header, lights, words)
}

/// Packs `(dim_x, dim_y, dim_z)` into the boot-snapshot word D11 pushes (`dim_x | dim_y<<8 |
/// dim_z<<16`) — the SAME packing [`GoldenLightHeader::new_clustered`] writes into the live
/// header's `cluster_params.z` lane, so [`unpack_dims`] can recover either source for
/// assertion 10.
fn dims_packed(cfg: &GoldenClusterConfig) -> u32 {
    cfg.dim_x | (cfg.dim_y << 8) | (cfg.dim_z << 16)
}

/// The inverse of [`dims_packed`] — decodes a packed dims word back to `(dim_x, dim_y, dim_z)`.
fn unpack_dims(packed: u32) -> (u32, u32, u32) {
    (packed & 0xFF, (packed >> 8) & 0xFF, (packed >> 16) & 0xFF)
}

/// The HIER arm's guard-tail size `G` (§8.2(A)): `(HIER_GROUP_THREADS * gps - dim_x*dim_y) *
/// dim_z` — the bijection's image size onto `[capacity, capacity+G)` under the shipped thread
/// map. `ClusterGrid` must be allocated at `capacity + G` cells (driver requirement 1, §8.10).
fn hier_guard_tail(dim_x: u32, dim_y: u32, dim_z: u32) -> u32 {
    let gps = golden_hier_groups_per_slice(dim_x, dim_y);
    (HIER_GROUP_THREADS * gps - dim_x * dim_y) * dim_z
}

/// One config in H3's device matrix (§8.10 "GPU matrix"): M1 (ORTHO, the shipped default dims),
/// M2 (PERSPECTIVE bench, same dims), E1 (`gps=1`-from-above), E2 (`gps=2` exact, `G=0`), E3
/// (`gps=2` ragged, the only `gps>=2` config with `G>0`), E4 (`gps=3` exact, `G=0`) — the SAME
/// six entries `tests/lighting_l1_host_oracle.rs`'s `hier_matrix_cases` uses (a self-contained
/// copy: integration test binaries cannot share private helpers across files) — plus N0, this
/// rung's own addition (`[P1-3]`, §8.10 "GPU matrix"): zero point/spot lights, so `ps_n == 0`,
/// which is the only config where the hierarchical arm's three barriers run with an empty coarse
/// loop and an empty fine walk (the `packed_dims == 0` hang/divide-by-zero probe the plan also
/// lists is deliberately NOT added here — see the module doc's "What this file does NOT do").
struct MatrixCase {
    name: &'static str,
    cfg: GoldenClusterConfig,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    camera_push: CompositePushConstants,
    l0a: Vec<GoldenLight>,
    ps: Vec<GoldenLight>,
    /// `true` only for the N0 row: with zero point/spot lights every froxel is empty BY DESIGN,
    /// so [`assert_non_vacuous`] (assertion 9) does not apply and the driver checks the stronger
    /// exact `(0, 0)` shape instead (§8.10 GPU matrix).
    expect_vacuous: bool,
}

fn matrix_cases() -> Vec<MatrixCase> {
    let (m1_l0a, m1_ps) = ortho_lights();
    let (bench_l0a, bench_ps) = bench_lights(24);
    let (bench_cam, bench_push) = bench_camera();
    let (n0_l0a, n0_ps) = bench_lights(0);

    let m1 = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.25, z_far: 4.0 };
    let m2 = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.1, z_far: 50.0 };
    let e1 = GoldenClusterConfig { dim_x: 16, dim_y: 16, dim_z: 24, ..m2 };
    let e2 = GoldenClusterConfig { dim_x: 32, dim_y: 16, dim_z: 24, ..m2 };
    let e3 = GoldenClusterConfig { dim_x: 16, dim_y: 17, dim_z: 24, ..m2 };
    let e4 = GoldenClusterConfig { dim_x: 32, dim_y: 24, dim_z: 24, ..m2 };

    vec![
        MatrixCase {
            name: "M1 16x9x24 ORTHO",
            cfg: m1,
            img_w: SDF_IMG_W,
            img_h: SDF_IMG_H,
            camera: CompositeCamera::Ortho,
            camera_push: CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H),
            l0a: m1_l0a,
            ps: m1_ps,
            expect_vacuous: false,
        },
        MatrixCase {
            name: "M2 16x9x24 PERSPECTIVE (bench)",
            cfg: m2,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: bench_l0a.clone(),
            ps: bench_ps.clone(),
            expect_vacuous: false,
        },
        MatrixCase {
            name: "E1 16x16x24 gps=1-from-above",
            cfg: e1,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: bench_l0a.clone(),
            ps: bench_ps.clone(),
            expect_vacuous: false,
        },
        MatrixCase {
            name: "E2 32x16x24 gps=2-exact",
            cfg: e2,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: bench_l0a.clone(),
            ps: bench_ps.clone(),
            expect_vacuous: false,
        },
        MatrixCase {
            name: "E3 16x17x24 gps=2-ragged",
            cfg: e3,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: bench_l0a.clone(),
            ps: bench_ps.clone(),
            expect_vacuous: false,
        },
        MatrixCase {
            name: "E4 32x24x24 gps=3-exact",
            cfg: e4,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: bench_l0a,
            ps: bench_ps,
            expect_vacuous: false,
        },
        MatrixCase {
            name: "N0 16x9x24 zero-lights (ps_n=0)",
            cfg: m2,
            img_w: BENCH_IMG,
            img_h: BENCH_IMG,
            camera: bench_cam,
            camera_push: bench_push,
            l0a: n0_l0a,
            ps: n0_ps,
            expect_vacuous: true,
        },
    ]
}

// ============================================================================
// The pure assertion functions — callable on either a real device readback or a hand-built
// synthetic buffer (see `mod tests` below). Every function states the concrete mutation that
// turns it red, per the plan's own discipline (§8).
// ============================================================================

/// Assertion 1 (§8.10 row 1, `[P1-3]`/§6): `alloc_total < index_list_cap`, the O2 saturation
/// PRECONDITION — a saturated claim means the flat list's tail was dropped in claim order, so a
/// downstream comparison would silently diff two independently-truncated results rather than
/// fail loudly. **Red mutation:** pin `index_list_cap = 1` on any non-empty config — demonstrated
/// on REAL hardware by `cull_hier_equiv_saturation_precondition_fires_red_on_device` (no shader
/// edit needed: the cap is a runtime push constant) and on a synthetic value in `mod tests`.
fn assert_saturation_precondition(alloc_total: u32, index_list_cap: u32) -> Result<(), String> {
    if alloc_total < index_list_cap {
        Ok(())
    } else {
        Err(format!(
            "alloc_total {alloc_total} >= index_list_cap {index_list_cap} — the O2 clamp \
             saturated; a downstream comparison would silently diff two truncated results"
        ))
    }
}

/// Assertion 2 (§8.10 row 2): per-froxel `count` equal between the two arms. **Red mutation:**
/// (ii) replace the radix-16 fold with lane 0's value, on the adversarial rig — a shader edit;
/// this function's failability is shown on a synthetic pair in `mod tests`.
fn assert_counts_equal(a: &[(u32, u32)], b: &[(u32, u32)], capacity: u32) -> Result<(), String> {
    for fi in 0..capacity as usize {
        if a[fi].1 != b[fi].1 {
            return Err(format!("froxel {fi} count differs: base={} hier={}", a[fi].1, b[fi].1));
        }
    }
    Ok(())
}

/// Assertion 3 (§8.10 row 3, `[P0-3]`): per-froxel `LightIndexList[offset..offset+count)` equal
/// AS A SEQUENCE (order included) between the two arms. **Red mutation:** (iii) walk mask words
/// descending, on a froxel holding accepted lights in two different mask words — a shader edit;
/// this function's failability is shown on a synthetic pair in `mod tests`.
fn assert_sequences_equal(
    a_grid: &[(u32, u32)],
    a_list: &[u32],
    b_grid: &[(u32, u32)],
    b_list: &[u32],
    capacity: u32,
) -> Result<(), String> {
    for fi in 0..capacity as usize {
        let (ao, ac) = a_grid[fi];
        let (bo, bc) = b_grid[fi];
        if ac != bc {
            continue; // already reported by assertion 2
        }
        let a_slice = a_list
            .get(ao as usize..(ao + ac) as usize)
            .ok_or_else(|| format!("froxel {fi}: base slice [{ao}..{}) is out of range", ao + ac))?;
        let b_slice = b_list
            .get(bo as usize..(bo + bc) as usize)
            .ok_or_else(|| format!("froxel {fi}: hier slice [{bo}..{}) is out of range", bo + bc))?;
        if a_slice != b_slice {
            return Err(format!("froxel {fi} index sequence differs: base={a_slice:?} hier={b_slice:?}"));
        }
    }
    Ok(())
}

/// Assertion 4 (§8.10 row 4): both arms equal to the host `golden_cluster_cull` set (per-froxel,
/// AS A SET — order is assertion 3's concern, not this one's). **Red mutation:** (ii) the
/// lane-0-fold adversarial rig — a shader edit; this function's failability is shown on a
/// synthetic pair in `mod tests`.
fn assert_matches_host_set(
    grid: &[(u32, u32)],
    list: &[u32],
    golden: &[Vec<u32>],
    capacity: u32,
) -> Result<(), String> {
    for fi in 0..capacity as usize {
        let (o, c) = grid[fi];
        let mut got: Vec<u32> = list
            .get(o as usize..(o + c) as usize)
            .ok_or_else(|| format!("froxel {fi}: slice [{o}..{}) is out of range", o + c))?
            .to_vec();
        got.sort_unstable();
        let mut want = golden[fi].clone();
        want.sort_unstable();
        if got != want {
            return Err(format!("froxel {fi} set differs: got={got:?} want={want:?}"));
        }
    }
    Ok(())
}

/// Assertion 5 (§8.10 row 5, detector A1, TOTALITY / at-least-once): no cell in `[0, capacity)`
/// still holds the `0xFFFFFFFF` sentinel. **Explicitly at-least-once** — this does NOT prove
/// exactly-once (that is assertion 7 / §8.2(B)); Rev 3's overclaim to the contrary is exactly
/// what let mutation (i) slip through. **Red mutation:** (i) drop the `valid` guard on phase 6 —
/// at M1/M2 this writes 2 688 cells PAST the buffer, every one landing in the guard tail
/// (assertion 6's concern, not a gap here); the (v) re-specified skew mutation (138 gaps, §8.3)
/// IS this assertion's own red — a shader edit either way; demonstrated on a synthetic buffer in
/// `mod tests`.
fn assert_totality(grid: &[(u32, u32)], capacity: u32) -> Result<(), String> {
    let gaps: Vec<usize> = (0..capacity as usize).filter(|&fi| grid[fi] == SENTINEL_CELL).collect();
    if gaps.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} cell(s) in [0,{capacity}) still hold the 0xFFFFFFFF sentinel (first gap at fi={})",
            gaps.len(),
            gaps[0]
        ))
    }
}

/// Assertion 6 (§8.10 row 6, detector A2, GUARD-TAIL INTEGRITY): every cell in
/// `[capacity, capacity + guard)` still holds the sentinel. **Red mutation:** (i) drop the
/// `valid` guard on phase 6 — writes land in the tail by the §8.2(A) bijection (2 688 cells at
/// M1/M2, `G = (256*gps - dim_x*dim_y) * dim_z`) — a shader edit; demonstrated on a synthetic
/// buffer in `mod tests` (this file cannot re-DXC a mutated shader to exercise it on device).
fn assert_guard_tail_integrity(grid: &[(u32, u32)], capacity: u32, guard: u32) -> Result<(), String> {
    let cleared: Vec<usize> =
        (capacity as usize..(capacity + guard) as usize).filter(|&fi| grid[fi] != SENTINEL_CELL).collect();
    if cleared.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} tail cell(s) in [{capacity},{}) were cleared — an out-of-range WRITE with no \
             validation layer to catch it (max fi observed = {})",
            cleared.len(),
            capacity + guard,
            cleared.last().expect("invariant: cleared is non-empty in this branch")
        ))
    }
}

/// Assertion 7 (§8.10 row 7, detector B, EXACTLY-ONCE): on the permutation-probe config,
/// `alloc_total == capacity`, every `count == 1`, and the `offset` multiset is EXACTLY
/// `{0, ..., capacity-1}` (§8.2(B2)) — strictly stronger than assertion 5 (it also catches the
/// case where a THIRD lane happens to fill a duplicate-write's gap, which A1 alone would miss).
/// **Red mutation:** any duplicate in-range write loses one offset and repeats another — a
/// shader edit; demonstrated on a synthetic buffer in `mod tests`.
fn assert_exactly_once_permutation(grid: &[(u32, u32)], alloc_total: u32, capacity: u32) -> Result<(), String> {
    if alloc_total != capacity {
        return Err(format!("alloc_total {alloc_total} != capacity {capacity}"));
    }
    let mut seen = vec![false; capacity as usize];
    for (fi, &(offset, count)) in grid.iter().enumerate().take(capacity as usize) {
        if count != 1 {
            return Err(format!("froxel {fi} count {count} != 1"));
        }
        if offset >= capacity {
            return Err(format!("froxel {fi} offset {offset} >= capacity {capacity}"));
        }
        if seen[offset as usize] {
            return Err(format!(
                "duplicate offset {offset} — a claim was overwritten and never appears \
                 elsewhere in the grid"
            ));
        }
        seen[offset as usize] = true;
    }
    if seen.iter().all(|&s| s) {
        Ok(())
    } else {
        Err("the offset multiset has a gap".to_string())
    }
}

/// Assertion 8 (§8.10 row 8, detector C, NO OUT-OF-RANGE LIGHT INDEX): no emitted
/// `LightIndexList` value is `>= light_count` — turns an out-of-range light READ into a
/// detectable index (the poison-tail light, D7). **Red mutation:** (iv) phase 4 loops
/// `j < HIER_MASK_BITS` with the fine clamp deleted, on a config with
/// `light_count < ps_begin + HIER_MASK_BITS` — a shader edit; demonstrated on a synthetic buffer
/// in `mod tests`.
fn assert_no_out_of_range_light_index(
    list: &[u32],
    grid: &[(u32, u32)],
    capacity: u32,
    light_count: u32,
) -> Result<(), String> {
    for (fi, &(o, c)) in grid.iter().enumerate().take(capacity as usize) {
        let slice = list
            .get(o as usize..(o + c) as usize)
            .ok_or_else(|| format!("froxel {fi}: slice [{o}..{}) is out of range", o + c))?;
        if let Some(&idx) = slice.iter().find(|&&idx| idx >= light_count) {
            return Err(format!(
                "froxel {fi} emitted index {idx} >= light_count {light_count} — a poison-tail \
                 row was read and accepted"
            ));
        }
    }
    Ok(())
}

/// Assertion 9 (§8.10 row 9): non-vacuity (at least one froxel non-empty) — a vacuous run proves
/// nothing (the "invalid run, not a pass" idiom §8.3's mutation (vii) rig requirement uses).
/// **Red mutation:** an empty light rig. The pipeline-DISTINCTNESS half of this assertion is
/// discharged structurally at [`CullPipelines`]'s own doc comment (this crate keeps
/// `ComputePipeline`'s `VkPipeline` field `pub(crate)`, so an integration test cannot read the
/// raw handle) — a SWAPPED pipeline is instead caught COLLATERALLY: at `gps >= 2` (E2/E3/E4) a
/// swapped pipeline's dispatch shape does not match its compiled `numthreads`, so totality
/// (assertion 5) or guard-tail (assertion 6) fails first, in the SAME green-matrix run.
///
/// **The N=0 row is the one deliberate exception** (§8.10 GPU matrix,
/// [`MatrixCase::expect_vacuous`]): with zero point/spot lights every froxel is empty BY DESIGN,
/// so this assertion does not apply there — the driver checks the stronger exact `(0, 0)` shape
/// directly instead of calling this function.
fn assert_non_vacuous(grid: &[(u32, u32)], capacity: u32) -> Result<(), String> {
    if (0..capacity as usize).any(|fi| grid[fi].1 > 0) {
        Ok(())
    } else {
        Err("every froxel is empty — a vacuous run proves nothing".to_string())
    }
}

/// Assertion 10 (§8.10 row 10, D11/D4 scope clause (b)): the boot snapshot dims equal the LIVE
/// header dims — evaluated only when `allow_skew` is `false` (every run in this file, since the
/// shader-level skew mutation (v) also needs a re-DXC'd edit this file does not build). **Red
/// mutation:** run a config that edits `ClusterConfig` after boot with `allow_skew == false`;
/// demonstrated on a synthetic boot/live mismatch in `mod tests`.
///
/// **Caller obligation (`[P1-1]`):** `boot_dims` MUST be derived from the value actually PUSHED to
/// the shader (e.g. [`ClusterCullHierPush::cluster_dims_packed`]), never re-derived from the same
/// `cfg` the live header was itself built from. Re-deriving both sides from one `cfg` makes the
/// comparison `d == unpack(pack(d))` — true for every input, by construction, and unable to go red
/// under any mutation; only comparing two INDEPENDENTLY computed values can fail.
fn assert_no_skew(boot_dims: (u32, u32, u32), live_dims: (u32, u32, u32), allow_skew: bool) -> Result<(), String> {
    if allow_skew || boot_dims == live_dims {
        Ok(())
    } else {
        Err(format!("boot snapshot dims {boot_dims:?} != live header dims {live_dims:?}"))
    }
}

/// Panics with `context` prefixed onto `result`'s error, for a device test's per-assertion call
/// sites (kept separate from the pure assertion functions so they stay callable — and
/// synthetically red-testable — with no formatting concerns).
fn expect_green(result: Result<(), String>, context: &str) {
    if let Err(msg) = result {
        panic!("{context}: {msg}");
    }
}

// ============================================================================
// The device driver.
// ============================================================================

/// The two cull-only compute pipelines this rung's driver needs, built ONCE and reused across
/// every config in the matrix: the SHIPPED base arm (`cluster_cull_spirv()`, the 16-byte
/// [`ClusterCullPush`]) and the SHIPPED `-D HIER=1` arm (`cluster_cull_hier_spirv()`, the
/// 24-byte [`ClusterCullHierPush`]) — both bound to the SAME 5-entry cull bind-group layout
/// (`cluster_cull.hlsl`'s register map: camera UBO @0, light table @1, `ClusterGrid` @2,
/// `LightIndexList` @3, `LightIndexAlloc` @4). Created INSIDE this test only (§8.10: "no host
/// wiring, no render path change").
///
/// **Assertion 9's handle-distinctness half.** `base_pipeline` and `hier_pipeline` come from two
/// separate [`RhiDevice::create_compute_pipeline`] calls with different shader modules and
/// different `push_constant_bytes` (16 vs 24) — Vulkan cannot coalesce those into one
/// `VkPipeline` (no pipeline derivatives are requested), so they are two distinct objects by
/// construction. This crate's `ComputePipeline` (`src/rhi_impl/mod.rs`) keeps its `VkPipeline`
/// field `pub(crate)`, so an integration test cannot read the raw handle to assert numeric
/// inequality directly — handle inequality alone would only prove two objects exist, not that
/// the right one was bound, which is why `assert_non_vacuous`'s own doc comment states the
/// STRONGER bind-check this driver relies on instead: each arm's dispatch shape is matched to its
/// own pipeline's compiled `numthreads`, so a swap manifests as an assertion 5/6 failure.
struct CullPipelines {
    layout: VulkanBindGroupLayout,
    base_module: VulkanShaderModule,
    hier_module: VulkanShaderModule,
    base_pipeline: ComputePipeline,
    hier_pipeline: ComputePipeline,
}

impl CullPipelines {
    fn create(ctx: &VulkanContext) -> Self {
        let cull_layout_entries = [
            BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        ];
        let layout = ctx
            .create_bind_group_layout(&BindGroupLayoutDesc { entries: &cull_layout_entries })
            .expect("H3 cull bind-group layout");

        let base_module = ctx.create_shader_module(cluster_cull_spirv()).expect("H3 base cluster-cull shader module");
        let hier_module = ctx.create_shader_module(cluster_cull_hier_spirv()).expect("H3 hier cluster-cull shader module");

        let base_pipeline = ctx
            .create_compute_pipeline(&ComputePipelineDesc {
                module: &base_module,
                entry: c"main",
                push_constant_bytes: CLUSTER_CULL_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            })
            .expect("H3 base cluster-cull compute pipeline");
        let hier_pipeline = ctx
            .create_compute_pipeline(&ComputePipelineDesc {
                module: &hier_module,
                entry: c"main",
                push_constant_bytes: CLUSTER_CULL_HIER_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            })
            .expect("H3 hier cluster-cull compute pipeline");

        Self { layout, base_module, hier_module, base_pipeline, hier_pipeline }
    }

    fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: every field was created on `ctx`; every dispatch that used these objects was
        // fence-waited inside `run_cull_arm` before that call returned, so none is in use; the
        // by-value `self` move + field extraction below make each destroyed exactly once.
        unsafe {
            ctx.destroy_compute_pipeline(self.hier_pipeline);
            ctx.destroy_compute_pipeline(self.base_pipeline);
            ctx.destroy_shader_module(self.hier_module);
            ctx.destroy_shader_module(self.base_module);
            ctx.destroy_bind_group_layout(self.layout);
        }
    }
}

/// One arm's readback: the `ClusterGrid` cells (as `(offset, count)` pairs, `capacity + guard`
/// long), the flat `LightIndexList` (`index_list_cap` `u32`s), and the `LightIndexAlloc` scalar.
struct ArmReadback {
    grid: Vec<(u32, u32)>,
    list: Vec<u32>,
    alloc_total: u32,
}

fn read_bytes(ctx: &VulkanContext, buffer: &BoundBuffer, byte_len: usize) -> Vec<u8> {
    let ptr = ctx.buffer_mapped_ptr(buffer).expect("host-visible buffer is mapped");
    let mut out = vec![0u8; byte_len];
    // SAFETY: `ptr` points to at least `byte_len` mapped host-coherent bytes (the buffer was
    // sized for exactly `byte_len` bytes by the caller); the caller's fence wait completed before
    // this call, so the GPU's writes are complete and coherent; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(ptr.as_ptr(), out.as_mut_ptr(), byte_len);
    }
    out
}

fn words_from_bytes(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Runs ONE arm's cull-only dispatch to completion and reads its three buffers back — the
/// §8.2(A) limit-4 / §8.10 protocol: `cluster_grid`/`light_index`/`light_index_alloc` are FRESH
/// allocations for this call (never shared across the two arms of a config), pre-filled with the
/// `0xFFFFFFFF` sentinel / re-zeroed immediately before THIS dispatch, and read back after THIS
/// dispatch's fence signals and before any other arm's dispatch is recorded. No
/// `vkCmdCopyBuffer`, no staging buffer: every buffer is `HostVisibleCoherent`, written before
/// the submit and read after the fence wait (the idiom at `tests/sdf_gbuffer_hybrid.rs:6202-6230`).
#[allow(clippy::too_many_arguments)]
// Each argument names one independently-varying dispatch parameter (which pipeline, the two
// read-only shared inputs, the push bytes, the dispatch shape, and the two buffer-sizing
// numbers); grouping them into a struct would relocate the same field count with no reduction in
// essential complexity, and the struct would still need constructing at every call site.
fn run_cull_arm(
    ctx: &VulkanContext,
    pipeline: &ComputePipeline,
    layout: &VulkanBindGroupLayout,
    camera_uniform: &BoundBuffer,
    light_table: &BoundBuffer,
    push_bytes: &[u8],
    dispatch_groups_x: u32,
    capacity: u32,
    guard: u32,
    index_list_cap: u32,
) -> ArmReadback {
    let cells = u64::from(capacity + guard);

    let cluster_grid = ctx
        .create_buffer(&BufferDesc { size: cells * 8, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent })
        .expect("H3 ClusterGrid storage buffer");
    {
        // Detector (A)'s pre-fill: EVERY cell in `[0, capacity+guard)`, immediately before this
        // arm's own dispatch (§8.2(A) limit 4).
        let mapped = ctx.buffer_mapped_ptr(&cluster_grid).expect("host-visible ClusterGrid is mapped");
        let sentinel = vec![SENTINEL_WORD; (cells * 2) as usize];
        write_words(mapped, &sentinel);
    }

    let light_index = ctx
        .create_buffer(&BufferDesc {
            size: u64::from(index_list_cap) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("H3 LightIndexList storage buffer");

    let light_index_alloc = ctx
        .create_buffer(&BufferDesc { size: 4, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent })
        .expect("H3 LightIndexAlloc counter buffer");
    {
        let mapped = ctx.buffer_mapped_ptr(&light_index_alloc).expect("host-visible LightIndexAlloc is mapped");
        write_words(mapped, &[0u32]);
    }

    let bind_group = ctx
        .create_bind_group(&BindGroupDesc {
            layout,
            entries: &[
                BindGroupEntry::UniformBuffer { buffer: camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: light_table },
                BindGroupEntry::StorageBuffer { buffer: &cluster_grid },
                BindGroupEntry::StorageBuffer { buffer: &light_index },
                BindGroupEntry::StorageBuffer { buffer: &light_index_alloc },
            ],
        })
        .expect("H3 cull bind group");

    let fence = ctx.create_fence(false).expect("H3 fence");
    let mut encoder = ctx.create_command_encoder().expect("H3 command encoder");
    encoder.begin().expect("H3 begin");
    encoder.bind_compute_pipeline(pipeline);
    encoder.bind_descriptor_set_compute(&bind_group, pipeline);
    encoder.push_compute_constants(pipeline, ShaderStage::COMPUTE, 0, push_bytes);
    encoder.dispatch(dispatch_groups_x, 1, 1);
    encoder.end().expect("H3 end");

    let queue = ctx.rhi_queue();
    queue.submit(&encoder, &fence).expect("H3 submit");
    ctx.wait_fence(&fence, u64::MAX).expect("H3 wait_fence");

    // Post-fence mapped reads (the idiom at `tests/sdf_gbuffer_hybrid.rs:6202-6230`): no copy, no
    // staging — the writes are complete and host-coherent-visible once the fence signals.
    let grid_words = words_from_bytes(&read_bytes(ctx, &cluster_grid, (cells * 8) as usize));
    let grid: Vec<(u32, u32)> = grid_words.chunks_exact(2).map(|w| (w[0], w[1])).collect();
    let list = words_from_bytes(&read_bytes(ctx, &light_index, (index_list_cap as usize) * 4));
    let alloc_total = words_from_bytes(&read_bytes(ctx, &light_index_alloc, 4))[0];

    // SAFETY: every resource created above was used only by the submission fence-waited just
    // above, so none is in use; each is destroyed exactly once.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
        ctx.destroy_bind_group(bind_group);
        ctx.destroy_buffer(light_index_alloc);
        ctx.destroy_buffer(light_index);
        ctx.destroy_buffer(cluster_grid);
    }

    ArmReadback { grid, list, alloc_total }
}

/// Creates and host-fills the shared (read-only, safe to share across both arms of one config)
/// camera UBO + light-table SSBO for one [`MatrixCase`].
fn create_shared_inputs(
    ctx: &VulkanContext,
    camera_push: &CompositePushConstants,
    light_words: &[u32],
) -> (BoundBuffer, BoundBuffer) {
    let camera_uniform = ctx
        .create_buffer(&BufferDesc {
            size: u64::from(COMPOSITE_PUSH_CONSTANT_BYTES),
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("H3 camera uniform buffer");
    {
        let mapped = ctx.buffer_mapped_ptr(&camera_uniform).expect("host-visible camera UBO is mapped");
        let bytes = camera_push.as_bytes();
        // SAFETY: `mapped` points to `COMPOSITE_PUSH_CONSTANT_BYTES` (80) mapped host-coherent
        // bytes; `bytes` is exactly that length (the const-asserted `CompositePushConstants`
        // size); no GPU work is in flight yet (the per-arm submits follow).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    let light_table = ctx
        .create_buffer(&BufferDesc {
            size: (light_words.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("H3 light table storage buffer");
    {
        let mapped = ctx.buffer_mapped_ptr(&light_table).expect("host-visible light table is mapped");
        write_words(mapped, light_words);
    }

    (camera_uniform, light_table)
}

// ============================================================================
// Device-gated tests (skip, with a printed SKIP, when no GPU is available).
// ============================================================================

/// The green matrix (§8.10's "GPU matrix"): both arms, across M1/M2/E1/E2/E3/E4, hold
/// assertions 1, 2, 3, 4, 5, 6, 8, 9 and 10 on the SHIPPED (unmutated) shaders — proving the
/// detectors do not false-positive on correct code, which is the precondition for trusting them
/// when a mutation IS present (§8.2, §8.3). N0 (zero point/spot lights, `[P1-3]`) holds the same
/// set MINUS assertion 9, which does not apply to a config that is empty by design (see
/// [`MatrixCase::expect_vacuous`]). Assertion 7 (exactly-once) is a dedicated configuration, run
/// separately by [`cull_hier_equiv_permutation_probe_exactly_once_at_m1_m2_and_e3`].
#[test]
fn cull_hier_equiv_green_matrix_both_arms_hold_across_the_grid() {
    let test = "cull_hier_equiv_green_matrix_both_arms_hold_across_the_grid";
    let Some(ctx) = boot_or_skip(test) else { return };
    let pipelines = CullPipelines::create(&ctx);

    for case in matrix_cases() {
        let capacity = case.cfg.cluster_count();
        let guard = hier_guard_tail(case.cfg.dim_x, case.cfg.dim_y, case.cfg.dim_z);
        let index_list_cap = capacity * 8;
        let eye = camera_eye(case.camera);

        let (header, full_lights, light_words) = build_light_table(&case.l0a, &case.ps, eye, &case.cfg);
        let light_count = header.light_count();
        let golden = golden_cluster_cull(case.img_w, case.img_h, case.camera, &case.cfg, &header, &full_lights, None);

        // Built here (ahead of its own dispatch below) because assertion 10 must compare THIS
        // value, not re-derive one from `case.cfg` — see `assert_no_skew`'s own doc comment
        // (`[P1-1]`).
        let hier_push = ClusterCullHierPush::new(
            case.cfg.z_near, case.cfg.z_far, case.cfg.max_lights_per_cluster, index_list_cap,
            dims_packed(&case.cfg), capacity,
        );

        // Assertion 10: compare the value actually PUSHED to the shader
        // (`hier_push.cluster_dims_packed`, computed via `dims_packed`) against the LIVE header's
        // own packed dims (computed independently inside `GoldenLightHeader::new_clustered`,
        // `goldens.rs`) — two separately-computed quantities, not one checked against its own
        // derivation (`[P1-1]`). Green on every run in this file except the shader-level skew
        // mutation this file does not execute (§8.10 protocol item 3).
        let pushed_dims = unpack_dims(hier_push.cluster_dims_packed);
        let live_dims = unpack_dims(header.cluster_params[2].to_bits());
        expect_green(
            assert_no_skew(pushed_dims, live_dims, false),
            &format!("[{}] assertion 10", case.name),
        );
        assert_eq!(
            hier_push.cluster_capacity,
            case.cfg.cluster_count(),
            "[{}] assertion 10: the pushed cluster_capacity must equal the live cfg's cluster_count()",
            case.name
        );

        let (camera_uniform, light_table) = create_shared_inputs(&ctx, &case.camera_push, &light_words);

        let groups_base = capacity.div_ceil(64);
        let base_push = ClusterCullPush::new(case.cfg.z_near, case.cfg.z_far, case.cfg.max_lights_per_cluster, index_list_cap);
        let base = run_cull_arm(
            &ctx, &pipelines.base_pipeline, &pipelines.layout, &camera_uniform, &light_table,
            base_push.as_bytes(), groups_base, capacity, guard, index_list_cap,
        );

        let groups_hier = golden_hier_groups_per_slice(case.cfg.dim_x, case.cfg.dim_y) * case.cfg.dim_z;
        let hier = run_cull_arm(
            &ctx, &pipelines.hier_pipeline, &pipelines.layout, &camera_uniform, &light_table,
            hier_push.as_bytes(), groups_hier, capacity, guard, index_list_cap,
        );

        for (arm_name, readback) in [("base", &base), ("hier", &hier)] {
            expect_green(assert_saturation_precondition(readback.alloc_total, index_list_cap), &format!("[{}] assertion 1 ({arm_name})", case.name));
            expect_green(assert_totality(&readback.grid, capacity), &format!("[{}] assertion 5 ({arm_name})", case.name));
            expect_green(assert_guard_tail_integrity(&readback.grid, capacity, guard), &format!("[{}] assertion 6 ({arm_name})", case.name));
            expect_green(assert_no_out_of_range_light_index(&readback.list, &readback.grid, capacity, light_count), &format!("[{}] assertion 8 ({arm_name})", case.name));
            if case.expect_vacuous {
                // N0 row (§8.10 GPU matrix, `[P1-3]`): `ps_n == 0` BY DESIGN — this config exists
                // to prove the hierarchical arm's barriers execute correctly with an empty coarse
                // loop, not to catch an accidentally-empty rig, so assertion 9 does not apply
                // (see `assert_non_vacuous`'s own doc comment). Assert the STRONGER exact shape
                // instead: every froxel is precisely `(0, 0)`.
                let non_zero: Vec<usize> =
                    (0..capacity as usize).filter(|&fi| readback.grid[fi] != (0, 0)).collect();
                assert!(
                    non_zero.is_empty(),
                    "[{}] N0 row ({arm_name}): {} froxel(s) are not exactly (0,0) (first at fi={})",
                    case.name,
                    non_zero.len(),
                    non_zero[0]
                );
                assert_eq!(
                    readback.alloc_total, 0,
                    "[{}] N0 row ({arm_name}): alloc_total must stay 0 with zero point/spot lights",
                    case.name
                );
            } else {
                expect_green(assert_non_vacuous(&readback.grid, capacity), &format!("[{}] assertion 9 ({arm_name})", case.name));
            }
            expect_green(assert_matches_host_set(&readback.grid, &readback.list, &golden, capacity), &format!("[{}] assertion 4 ({arm_name})", case.name));
        }

        expect_green(assert_counts_equal(&base.grid, &hier.grid, capacity), &format!("[{}] assertion 2", case.name));
        expect_green(assert_sequences_equal(&base.grid, &base.list, &hier.grid, &hier.list, capacity), &format!("[{}] assertion 3", case.name));

        if case.expect_vacuous {
            println!(
                "[{}] H3 N0 row OK: base+hier hold assertions 1,2,3,4,5,6,8,10 and the exact-zero \
                 shape (assertion 9 does not apply — see module doc) (capacity={capacity}, \
                 guard={guard}, alloc_total base={} hier={})",
                case.name, base.alloc_total, hier.alloc_total
            );
        } else {
            println!(
                "[{}] H3 green matrix OK: base+hier hold assertions 1,2,3,4,5,6,8,9,10 \
                 (capacity={capacity}, guard={guard}, alloc_total base={} hier={})",
                case.name, base.alloc_total, hier.alloc_total
            );
        }

        // SAFETY: every resource here was created on `ctx`; `run_cull_arm` already fence-waited
        // its own dispatch before returning, so neither buffer is in use; each is destroyed once.
        unsafe {
            ctx.destroy_buffer(light_table);
            ctx.destroy_buffer(camera_uniform);
        }
    }

    assert_validation_clean(&ctx, test);
    pipelines.destroy(&ctx);
}

/// The permutation probe (§8.2(B2), §8.10 driver requirement 3): the HIER arm at M1, M2 and E3,
/// each with ONE point light at the camera eye (`range = 1e6`, so every froxel accepts exactly
/// one light) and `index_list_cap = 2 * capacity`. E3 is the only `gps >= 2` config with `G > 0`,
/// hence the only device measurement of exactly-once with a NON-DEGENERATE thread map (at
/// `gps = 1`, M1/M2's map collapses to `slice = gid; s = lane`, which mutation (vi) is
/// bit-identical to by construction — §8.3).
#[test]
fn cull_hier_equiv_permutation_probe_exactly_once_at_m1_m2_and_e3() {
    let test = "cull_hier_equiv_permutation_probe_exactly_once_at_m1_m2_and_e3";
    let Some(ctx) = boot_or_skip(test) else { return };
    let pipelines = CullPipelines::create(&ctx);

    let m1 = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.25, z_far: 4.0 };
    let m2 = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.1, z_far: 50.0 };
    let e3 = GoldenClusterConfig { dim_x: 16, dim_y: 17, dim_z: 24, ..m2 };
    let (bench_cam, bench_push) = bench_camera();
    let ortho_push = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);

    let probes: [(&str, GoldenClusterConfig, CompositeCamera, CompositePushConstants); 3] = [
        ("M1 16x9x24 ORTHO", m1, CompositeCamera::Ortho, ortho_push),
        ("M2 16x9x24 PERSPECTIVE (bench)", m2, bench_cam, bench_push),
        ("E3 16x17x24 gps=2-ragged", e3, bench_cam, bench_push),
    ];

    for (name, cfg, camera, camera_push) in probes {
        let capacity = cfg.cluster_count();
        let guard = hier_guard_tail(cfg.dim_x, cfg.dim_y, cfg.dim_z);
        let index_list_cap = capacity * 2; // §8.2(B2): `index_list_cap = 2 * capacity`
        let eye = camera_eye(camera);

        let l0a: Vec<GoldenLight> = Vec::new();
        let ps = vec![GoldenLight::point(eye, [1.0, 1.0, 1.0], 0.0, 1.0e6)];
        let (_header, _full_lights, light_words) = build_light_table(&l0a, &ps, eye, &cfg);

        let (camera_uniform, light_table) = create_shared_inputs(&ctx, &camera_push, &light_words);

        let groups_hier = golden_hier_groups_per_slice(cfg.dim_x, cfg.dim_y) * cfg.dim_z;
        let hier_push = ClusterCullHierPush::new(cfg.z_near, cfg.z_far, cfg.max_lights_per_cluster, index_list_cap, dims_packed(&cfg), capacity);
        let hier = run_cull_arm(
            &ctx, &pipelines.hier_pipeline, &pipelines.layout, &camera_uniform, &light_table,
            hier_push.as_bytes(), groups_hier, capacity, guard, index_list_cap,
        );

        expect_green(assert_exactly_once_permutation(&hier.grid, hier.alloc_total, capacity), &format!("[{name}] assertion 7"));
        expect_green(assert_totality(&hier.grid, capacity), &format!("[{name}] assertion 5 (permutation probe)"));
        expect_green(assert_guard_tail_integrity(&hier.grid, capacity, guard), &format!("[{name}] assertion 6 (permutation probe)"));

        println!("[{name}] H3 permutation probe OK: alloc_total == capacity == {capacity}, every count == 1, offsets == {{0..{capacity}}}");

        // SAFETY: every resource here was created on `ctx`; `run_cull_arm` already fence-waited
        // its own dispatch before returning, so neither buffer is in use; each is destroyed once.
        unsafe {
            ctx.destroy_buffer(light_table);
            ctx.destroy_buffer(camera_uniform);
        }
    }

    assert_validation_clean(&ctx, test);
    pipelines.destroy(&ctx);
}

/// A REAL device demonstration of assertion 1 going RED (§8.10 row 1) — no shader edit needed,
/// since `index_list_cap` is a runtime push constant: pinning it at `1` on a non-empty light rig
/// must saturate the O2 clamp and fire the precondition, proving the detector's failability on
/// hardware rather than only on a synthetic value.
#[test]
fn cull_hier_equiv_saturation_precondition_fires_red_on_device() {
    let test = "cull_hier_equiv_saturation_precondition_fires_red_on_device";
    let Some(ctx) = boot_or_skip(test) else { return };
    let pipelines = CullPipelines::create(&ctx);

    let cfg = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.1, z_far: 50.0 };
    let capacity = cfg.cluster_count();
    let guard = hier_guard_tail(cfg.dim_x, cfg.dim_y, cfg.dim_z);
    let (bench_cam, bench_push) = bench_camera();
    let (l0a, ps) = bench_lights(24);
    let eye = camera_eye(bench_cam);
    let (_header, _full_lights, light_words) = build_light_table(&l0a, &ps, eye, &cfg);

    let (camera_uniform, light_table) = create_shared_inputs(&ctx, &bench_push, &light_words);

    let index_list_cap = 1; // the red mutation: pin the cap far below what a 24-light rig claims
    let groups_hier = golden_hier_groups_per_slice(cfg.dim_x, cfg.dim_y) * cfg.dim_z;
    let hier_push = ClusterCullHierPush::new(cfg.z_near, cfg.z_far, cfg.max_lights_per_cluster, index_list_cap, dims_packed(&cfg), capacity);
    let hier = run_cull_arm(
        &ctx, &pipelines.hier_pipeline, &pipelines.layout, &camera_uniform, &light_table,
        hier_push.as_bytes(), groups_hier, capacity, guard, index_list_cap,
    );

    let result = assert_saturation_precondition(hier.alloc_total, index_list_cap);
    assert!(
        result.is_err(),
        "assertion 1's RED mechanism did not fire: alloc_total={} stayed under index_list_cap=1 \
         even though the 24-light rig claims far more than one index",
        hier.alloc_total
    );
    println!("[saturation] assertion 1 fired RED as designed: {}", result.unwrap_err());

    // SAFETY: every resource here was created on `ctx`; `run_cull_arm` already fence-waited its
    // own dispatch before returning, so neither buffer is in use; each is destroyed once.
    unsafe {
        ctx.destroy_buffer(light_table);
        ctx.destroy_buffer(camera_uniform);
    }

    assert_validation_clean(&ctx, test);
    pipelines.destroy(&ctx);
}

// ============================================================================
// Synthetic-buffer demonstrations (NO GPU): each assertion function fed a hand-built buffer that
// encodes the fault its own doc comment names, showing the RED path fires — the discharge the
// plan requires when a device re-DXC of the mutated shader is out of scope (assertions 5, 6, 7,
// 8), plus the remaining six for completeness.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// A fully green `capacity`-cell grid (every cell written, offsets `0..capacity`, count 1
    /// each) plus a `guard`-cell sentinel tail — the well-formed permutation shape assertions
    /// 5/6/7 all accept.
    fn green_permutation_grid(capacity: u32, guard: u32) -> Vec<(u32, u32)> {
        let mut grid: Vec<(u32, u32)> = (0..capacity).map(|fi| (fi, 1)).collect();
        grid.extend(std::iter::repeat_n(SENTINEL_CELL, guard as usize));
        grid
    }

    #[test]
    fn assertion_5_totality_green_then_red_on_a_gap() {
        let capacity = 16;
        let grid = green_permutation_grid(capacity, 0);
        assert!(assert_totality(&grid, capacity).is_ok(), "a fully-written grid must be green");

        // Red mutation: clear cell 7 back to the sentinel (the (v)-style skew fault's shape,
        // §8.3 — one gap is enough to demonstrate the mechanism).
        let mut gapped = grid;
        gapped[7] = SENTINEL_CELL;
        let err = assert_totality(&gapped, capacity).expect_err("a cleared in-range cell must fire assertion 5");
        assert!(err.contains("fi=7"), "the error must name the gap's froxel index: {err}");
    }

    #[test]
    fn assertion_6_guard_tail_green_then_red_on_a_tail_write() {
        let capacity = 16;
        let guard = 8;
        let grid = green_permutation_grid(capacity, guard);
        assert!(assert_guard_tail_integrity(&grid, capacity, guard).is_ok(), "an untouched tail must be green");

        // Red mutation: mutation (i)'s effect — a phase-6 write landed in the guard tail (here,
        // cell `capacity + 3`) because the `valid` guard was dropped.
        let mut cleared = grid;
        cleared[(capacity + 3) as usize] = (42, 1);
        let err = assert_guard_tail_integrity(&cleared, capacity, guard).expect_err("a cleared tail cell must fire assertion 6");
        assert!(err.contains("1 tail cell"), "the error must report the cleared-cell count: {err}");
    }

    #[test]
    fn assertion_7_exactly_once_green_then_red_on_a_duplicate_offset() {
        let capacity = 8;
        let grid = green_permutation_grid(capacity, 0);
        assert!(assert_exactly_once_permutation(&grid, capacity, capacity).is_ok(), "a true permutation must be green");

        // Red mutation: froxel 3 duplicates froxel 5's offset instead of claiming its own
        // distinct slot — offset 3 never appears anywhere in the grid (a gap), offset 5 appears
        // twice (a duplicate).
        let mut dup = grid;
        dup[3] = (5, 1);
        let err = assert_exactly_once_permutation(&dup, capacity, capacity).expect_err("a duplicate offset must fire assertion 7");
        assert!(err.contains("duplicate offset 5"), "the error must name the duplicate: {err}");
    }

    #[test]
    fn assertion_8_no_out_of_range_light_index_green_then_red_on_a_poison_read() {
        let capacity = 4;
        let light_count = 10;
        let grid = vec![(0, 3), (3, 2), (5, 0), (5, 1)];
        let list = vec![0, 1, 2, 4, 5, 9];
        assert!(assert_no_out_of_range_light_index(&list, &grid, capacity, light_count).is_ok(), "every emitted index is < light_count; must be green");

        // Red mutation: (iv)'s effect — froxel 3 emits index 10, a poison-tail row read past
        // `light_count` and accepted (the poison light is one every froxel would accept).
        let mut poisoned_list = list;
        poisoned_list[5] = 10;
        let err = assert_no_out_of_range_light_index(&poisoned_list, &grid, capacity, light_count).expect_err("an index >= light_count must fire assertion 8");
        assert!(err.contains("index 10"), "the error must name the offending index: {err}");
    }

    /// `[P1-2]`: the test above proves the ASSERTION FUNCTION returns `Err` on a hand-built
    /// out-of-range index; it says nothing about whether the actual POISON CONSTRUCTION
    /// (`build_light_table`'s `GoldenLight::point(eye, .., power=0.0, range=1e6)`) is a light the
    /// (unmutated) cull geometry would ever accept. `golden_cluster_cull` tests only `kind` and
    /// `range` — POWER is never read — so this proves the poison row IS a light every froxel
    /// accepts: it runs the SAME host cull geometry the shader uses with `light_count` extended
    /// by ONE past the real rows, exactly what a producer mutation like (iv) would read on
    /// device, and asserts the first poison row's index appears in at least one froxel's accepted
    /// set. If the poison were made unacceptable (a kind change, a zero-power skip, a range
    /// clamp), this test goes RED — see the developer report's RED transcript for a temporary
    /// kind-change demonstration.
    #[test]
    fn poison_row_is_accepted_by_the_unmutated_cull_geometry() {
        let cfg = GoldenClusterConfig {
            dim_x: 16,
            dim_y: 9,
            dim_z: 24,
            max_lights_per_cluster: 256,
            z_near: 0.1,
            z_far: 50.0,
        };
        let (bench_cam, _push) = bench_camera();
        let eye = camera_eye(bench_cam);
        let (l0a, ps) = bench_lights(8);
        let (header, full_lights, _words) = build_light_table(&l0a, &ps, eye, &cfg);
        let real_light_count = header.light_count();

        // Extend `light_count` by one row past the real lights — the SAME oversized `full_lights`
        // table `build_light_table` already produced now has its first poison row indexed by the
        // host oracle's own `[l0a_count..light_count)` loop.
        let mut extended = header;
        extended.counts_exposure[0] = f32::from_bits(real_light_count + 1);
        let poison_index = real_light_count;

        let golden = golden_cluster_cull(BENCH_IMG, BENCH_IMG, bench_cam, &cfg, &extended, &full_lights, None);
        let accepted = golden.iter().any(|cell| cell.contains(&poison_index));
        assert!(
            accepted,
            "poison row {poison_index} (POINT, power=0.0, range=1e6 at the camera eye) was not \
             accepted by any froxel — detector C (`assert_no_out_of_range_light_index`) would \
             never fire on an out-of-range read, because the poison construction itself is \
             unarmed against the shipped (kind, range)-only cull geometry"
        );
    }

    #[test]
    fn assertion_1_saturation_precondition_green_then_red() {
        assert!(assert_saturation_precondition(5, 10).is_ok());
        let err = assert_saturation_precondition(10, 10).expect_err("alloc_total == cap must fire (>= is red)");
        assert!(err.contains("10"));
    }

    #[test]
    fn assertion_2_counts_equal_green_then_red() {
        let a = vec![(0, 2), (2, 3)];
        let b = vec![(0, 2), (5, 3)];
        assert!(assert_counts_equal(&a, &b, 2).is_ok(), "equal counts (offsets may differ) must be green");
        let c = vec![(0, 2), (5, 1)];
        let err = assert_counts_equal(&a, &c, 2).expect_err("a differing count must fire assertion 2");
        assert!(err.contains("froxel 1"));
    }

    #[test]
    fn assertion_3_sequences_equal_green_then_red_on_order() {
        let a_grid = vec![(0, 3)];
        let a_list = vec![7, 2, 9];
        let b_grid = vec![(0, 3)];
        let b_list = vec![7, 2, 9];
        assert!(assert_sequences_equal(&a_grid, &a_list, &b_grid, &b_list, 1).is_ok());

        // Red mutation: (iii)'s effect — the same SET, different ORDER.
        let c_list = vec![9, 2, 7];
        let err = assert_sequences_equal(&a_grid, &a_list, &b_grid, &c_list, 1)
            .expect_err("a reordered sequence must fire assertion 3 even though the set matches");
        assert!(err.contains("froxel 0"));
    }

    #[test]
    fn assertion_4_matches_host_set_green_then_red() {
        let grid = vec![(0, 2)];
        let list = vec![3, 1];
        let golden = vec![vec![1, 3]];
        assert!(assert_matches_host_set(&grid, &list, &golden, 1).is_ok(), "the same set in different order must still be green (set equality)");
        let bad_golden = vec![vec![1, 4]];
        let err = assert_matches_host_set(&grid, &list, &bad_golden, 1).expect_err("a differing set must fire assertion 4");
        assert!(err.contains("froxel 0"));
    }

    #[test]
    fn assertion_9_non_vacuous_green_then_red() {
        let non_empty = vec![(0, 0), (0, 1)];
        assert!(assert_non_vacuous(&non_empty, 2).is_ok());
        let all_empty = vec![(0, 0), (0, 0)];
        assert!(assert_non_vacuous(&all_empty, 2).is_err(), "an all-empty grid must fire assertion 9");
    }

    #[test]
    fn assertion_10_no_skew_green_then_red() {
        assert!(assert_no_skew((16, 9, 24), (16, 9, 24), false).is_ok());
        let err = assert_no_skew((16, 9, 23), (16, 9, 24), false).expect_err("a boot/live dims mismatch must fire assertion 10 when allow_skew is false");
        assert!(err.contains("16, 9, 23"));
        // `allow_skew == true` scopes the check off (§8.10 protocol item 3) — the same mismatch
        // is permitted for exactly the one run that needs it.
        assert!(assert_no_skew((16, 9, 23), (16, 9, 24), true).is_ok());
    }

    #[test]
    fn hier_guard_tail_matches_the_plan_worked_examples() {
        // §8.2(A)'s own worked examples: default 16x9x24 (`G = 2688`), E2 32x16x24 (`G = 0`), E3
        // 16x17x24 (`G = 5760`), E4 32x24x24 (`G = 0`).
        assert_eq!(hier_guard_tail(16, 9, 24), 2688);
        assert_eq!(hier_guard_tail(32, 16, 24), 0);
        assert_eq!(hier_guard_tail(16, 17, 24), 5760);
        assert_eq!(hier_guard_tail(32, 24, 24), 0);
    }

    #[test]
    fn dims_pack_unpack_round_trips() {
        let cfg = GoldenClusterConfig { dim_x: 16, dim_y: 9, dim_z: 24, max_lights_per_cluster: 256, z_near: 0.1, z_far: 50.0 };
        assert_eq!(unpack_dims(dims_packed(&cfg)), (16, 9, 24));
    }
}
