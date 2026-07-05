//! SDFDDGI I2 — the probe-update pass COST bench (`#[ignore]`, plan §5).
//!
//! This rung exists to produce ONE number: the per-config p95 GPU cost of the probe-update
//! dispatch, from which the orchestrator DERIVES the shipped cadence (`rays_per_probe`,
//! `subset_n`, `GI_MAX_IT`, grid) under the ~3 ms ceiling. The harness only EMITS clean per-config
//! `median / p95 / stddev`; it does NOT hardcode a derived cadence (the orchestrator runs it and
//! reads the numbers).
//!
//! # Named `ddgi_probe_gi_cost`, NOT `..._update_cost`
//!
//! A test/exe name containing "update" triggers Windows os-error-740 (UAC elevation) on the target
//! box, so this file is `ddgi_probe_gi_cost` (plan directive).
//!
//! # Measurement method (plan §5, P0-1 fix)
//!
//! No `vkCmdWriteTimestamp` subsystem exists (grep-confirmed), so cost is a CPU WALL-CLOCK around a
//! FENCED, dispatch-ONLY, swapchain-absent isolated submit:
//! 1. Boot an OFFSCREEN device (no swapchain acquire/present).
//! 2. Allocate the atlas (irradiance + depth STORAGE images) + classification (u32/probe) + the
//!    Fibonacci ray table + the `grand_showcase`-shaped CSG edit-list SSBO (the real 16-edit fold
//!    cost) + a zeroed light table + the b6 update UBO.
//! 3. Create the update pipeline for the `GI_MAX_IT` variant under test (measured==shipped — the
//!    re-DXC'd variant, plan §1.2/§5).
//! 4. Record a command buffer of ONLY reset→bind→dispatch; `submit` + `wait_fence`; wall-clock the
//!    submit+wait.
//! 5. Measure the empty-submit overhead ONCE (a no-op-dispatch fenced submit) and SUBTRACT it from
//!    every measurement (at the ~3 ms target the ~20-50 µs overhead is <2%).
//! 6. `>= 200` iterations, discard the first 20; report median + p95 + stddev per config.
//!
//! # The sweep (all knobs first-class, plan §5)
//!
//! `rays_per_probe ∈ {16,32,64,128}` × `subset_n ∈ {1,2,4,8}` × `GI_MAX_IT ∈ {32,64,96,128}` ×
//! grid {default, coarser} × shadow {off / 1 directional}. Every knob is a UBO field or the
//! re-DXC'd pipeline variant, so no cadence is baked into the shader.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test ddgi_probe_gi_cost -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1` (validation is crash-prone on the box).

use core::ptr::NonNull;
use std::time::Instant;

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, ComputePipelineDesc, DescriptorKind, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};

use boyko_rhi_vulkan::compute::{
    EDITLIST_BUFFER_WORDS, GI_MAX_IT_VARIANTS, encode_edit_list, sdf_op, sdf_probe_update_spirv,
    SdfEdit,
};
use boyko_rhi_vulkan::ddgi::{DDGI_PROBE_COUNT, DdgiAtlas};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

// ---- the b6 update UBO (a local byte-mirror — the bench does not depend on boyko_render) --------

/// The number of `float4`s in the Fibonacci ray table (`GI_MAX_RAYS` — the shader's groupshared
/// cache bound; the sweep's `rays_per_probe` maxes at 128).
const RAY_TABLE_RAYS: usize = 128;

/// The light-table word budget: a 16-word header + a generous `GpuLight[]` span (12 words each). The
/// bench seeds ONE valid directional light at entry 0 ([`directional_light_table`]); the UBO's
/// `light_count` drives the shade loop (0 = the "shadow OFF" row, 1 = the "shadow ON" row).
const LIGHT_TABLE_WORDS: usize = 16 + 12 * 64;

/// The b6 `DdgiUpdate` cbuffer byte-mirror (48 B, the committed shader's field order): `float4
/// origin` (xyz = origin, w = spacing), `uint4 grid_dims`, then `frame_index / subset_n /
/// rays_per_probe / light_count`. Written host-coherent before each dispatch.
#[repr(C)]
#[derive(Clone, Copy)]
struct DdgiUpdateUbo {
    origin: [f32; 4],
    grid_dims: [u32; 4],
    frame_index: u32,
    subset_n: u32,
    rays_per_probe: u32,
    light_count: u32,
}

const _: () = assert!(size_of::<DdgiUpdateUbo>() == 48);

impl DdgiUpdateUbo {
    fn as_bytes(&self) -> [u8; 48] {
        // SAFETY: `#[repr(C)]`, 48-byte const-asserted layout, all-POD fields — every bit pattern is
        // valid, so the transmute reads only initialized bytes.
        unsafe { core::mem::transmute::<Self, [u8; 48]>(*self) }
    }
}

/// One sweep configuration (the axes plan §5 enumerates).
#[derive(Clone, Copy, Debug)]
struct Config {
    rays_per_probe: u32,
    subset_n: u32,
    gi_max_it: u32,
    /// The grid dims (default `[16,8,16]` or a coarser variant).
    grid_dims: [u32; 3],
    /// Whether one shadowed directional light is present (`light_count = 1` vs `0`).
    shadow_light: bool,
}

impl Config {
    /// `DDGI_PROBE_COUNT / subset_n` — the dispatch block count (one block per active probe).
    fn dispatch_groups(&self, probe_count: u32) -> u32 {
        probe_count / self.subset_n.max(1)
    }
}

/// The reported per-config timing summary (all in microseconds, overhead-subtracted).
#[derive(Clone, Copy, Debug)]
struct Summary {
    median_us: f64,
    p95_us: f64,
    stddev_us: f64,
}

/// Boots an offscreen context (validation OFF — the bench measures cost, not correctness), or
/// `None` with a SKIP log when no GPU / loader is present.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        // Validation adds large per-submit overhead + is crash-prone on the box; the bench measures
        // steady-state GPU cost, so it runs validation-OFF (mirrors the windowed-dump convention).
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP ddgi_probe_gi_cost: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Writes `words` `u32`s into a host-coherent mapping (valid before the submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer holds at least `words.len()` `u32`s inside the persistent host-coherent
        // mapping; `dst + i` is in-bounds; no GPU work is in flight when the CPU seeds it.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// A representative `grand_showcase`-shaped 16-edit CSG fold, SCALED TO FILL THE DEFAULT PROBE-GRID
/// VOLUME (the real inner-loop cost the field march pays per step). Fills the fixed `MAX_SDF_EDITS`
/// cap with a mix of unions / a subtract / smooth-blends so the shader's `min(Buf[0], 16)`-deep edit
/// loop runs at full depth AND so a LARGE FRACTION of probe rays actually HIT geometry (the shadow
/// march runs only inside `shade_hit`, i.e. only on a HIT — geometry near the tiny world origin would
/// leave most probes' rays escaping to sky within `GI_T_MAX`, under-measuring the dominant shadow
/// cost, P1-1). The default grid origin `[-16,-2,-16]` + spacing `2.0` × dims `[16,8,16]` spans world
/// `[-16,-2,-16] .. [+14,+12,+14]` (a `30×14×30` box, center ≈ `[-1,5,-1]`); this fold fills it.
fn grand_showcase_edits() -> Vec<SdfEdit> {
    // The grid center + a half-extent that keeps the field within a ray's `GI_T_MAX = 10` reach of
    // most probes (probe spacing 2.0, so a march reaches ~5 probes; geometry spread over the volume
    // means the nearest surface is well within reach from nearly every probe).
    const CENTER: [f32; 3] = [-1.0, 5.0, -1.0];
    let mut edits = Vec::with_capacity(16);
    // A large base body filling the volume core + a carved bite (the CSG discriminator).
    edits.push(SdfEdit::sphere(CENTER, 9.0, sdf_op::UNION, 0.0));
    edits.push(SdfEdit::sphere([CENTER[0] + 5.0, CENTER[1], CENTER[2]], 4.0, sdf_op::SUBTRACT, 0.6));
    edits.push(SdfEdit::box_shape([CENTER[0], CENTER[1] - 5.0, CENTER[2]], [8.0, 1.5, 8.0], sdf_op::UNION, 1.0));
    // Fill to the full 16-edit cap with an alternating ring of smooth spheres/boxes spread across a
    // radius-7 orbit of the center (a dense grid-filling fold — the shader loops all 16).
    let mut i = edits.len();
    while i < 16 {
        let a = i as f32 * 0.7;
        let (cx, cz) = (CENTER[0] + a.cos() * 7.0, CENTER[2] + a.sin() * 7.0);
        let cy = CENTER[1] + ((a * 1.3).sin()) * 4.0;
        if i % 2 == 0 {
            edits.push(SdfEdit::sphere([cx, cy, cz], 2.6, sdf_op::UNION, 1.2));
        } else {
            edits.push(SdfEdit::box_shape([cx, cy, cz], [2.0, 2.0, 2.0], sdf_op::UNION, 1.0));
        }
        i += 1;
    }
    edits
}

/// Builds the light-table SSBO words with ONE valid DIRECTIONAL light at entry 0 (the P1-1 fix; see
/// the seed call-site). The `light_table.hlsli` layout: a 16-word `LightHeaderGpu` header then a flat
/// `GpuLight[]` of 12 words each starting at `LIGHT_HEADER_BASE = 16`. Entry `i`'s words (relative to
/// its base): `dir@[+0..2] / kind@[+3] (bitcast u32) / pos@[+4..6] / range@[+7] / color@[+8..10] /
/// cone@[+11]`. The shader's `shade_hit` needs a NON-ZERO UNIT `e.dir` (else `normalize` is
/// degenerate and `NoL=0` skips the march), `kind == LIGHT_KIND_DIRECTIONAL (0)`, and a non-zero
/// `e.color` (already `linear_color × illuminance` — `from_directional` bakes it, so the shade
/// multiplies by `e.color` only). Header word 0 (`light_count`) is set to 1 for layout consistency,
/// though this shader loops the UBO's `light_count`, not the header word.
fn directional_light_table() -> Vec<u32> {
    // The light kinds mirror `light_table.hlsli` / `boyko_render::light::LIGHT_KIND_*`.
    const LIGHT_KIND_DIRECTIONAL: u32 = 0;
    const HEADER_WORDS: usize = 16; // LIGHT_HEADER_WORDS
    const ENTRY0_BASE: usize = HEADER_WORDS; // LIGHT_HEADER_BASE
    let mut words = vec![0u32; LIGHT_TABLE_WORDS];
    // Header word 0 = light_count (asuint). 1 directional light.
    words[0] = 1;
    // Entry 0: dir@[+0..2] = a unit TO-LIGHT direction; kind@[+3] = DIRECTIONAL (bitcast u32);
    // color@[+8..10] = a non-zero premultiplied radiance so the diffuse term is real.
    let dir = {
        let v = [0.3_f32, -1.0, 0.2];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    };
    words[ENTRY0_BASE] = dir[0].to_bits();
    words[ENTRY0_BASE + 1] = dir[1].to_bits();
    words[ENTRY0_BASE + 2] = dir[2].to_bits();
    words[ENTRY0_BASE + 3] = LIGHT_KIND_DIRECTIONAL;
    // pos@[+4..6] + range@[+7] are unused for a directional; leave 0.
    words[ENTRY0_BASE + 8] = 3.0_f32.to_bits();
    words[ENTRY0_BASE + 9] = 3.0_f32.to_bits();
    words[ENTRY0_BASE + 10] = 3.0_f32.to_bits();
    // cone@[+11] unused for a directional; leave 0.
    words
}

/// Reduces raw per-iteration microsecond samples (already overhead-subtracted, first 20 discarded)
/// to a `Summary` (median + p95 + stddev). Sorts a copy for the percentiles.
fn summarize(samples_us: &[f64]) -> Summary {
    let mut s = samples_us.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    let median_us = s[n / 2];
    // The p95 index (nearest-rank); `n >= 1` guaranteed by the caller.
    let p95_idx = ((n as f64) * 0.95).ceil() as usize;
    let p95_us = s[p95_idx.min(n - 1)];
    let mean = s.iter().sum::<f64>() / n as f64;
    let var = s.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64;
    Summary { median_us, p95_us, stddev_us: var.sqrt() }
}

/// The number of timed iterations per config (`>= 200`, plan §5).
const ITERS: usize = 220;
/// The warm-up iterations discarded from the front (plan §5).
const WARMUP: usize = 20;

/// Runs the full sweep on `ctx`, printing one `median / p95 / stddev` line per config plus the
/// measured empty-submit overhead. The GPU-facing resources (atlas / classification / ray table /
/// edit list / light table / UBO) are allocated once and reused across configs; only the pipeline
/// is rebuilt per `GI_MAX_IT` variant (the re-DXC'd measured==shipped variant).
fn run_sweep(ctx: &VulkanContext) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // The persistent atlas (irradiance + depth STORAGE images + classification u32/probe). Skips
    // the whole bench if the device lacks B10G11R11/RG16F storage (the update pass cannot run).
    if !device.device_caps().ddgi_storage_ok() {
        eprintln!(
            "SKIP ddgi_probe_gi_cost: device lacks B10G11R11/RG16F STORAGE (irr_ok={}, depth_ok={})",
            device.device_caps().ddgi_irr_storage_ok,
            device.device_caps().ddgi_depth_storage_ok
        );
        return;
    }
    let atlas = DdgiAtlas::create(device).expect("DDGI atlas create");

    // The Fibonacci ray table (128 `float4`s = 2 KB, STORAGE). A simple golden-angle spiral fills
    // it; the exact directions do not change the cost (they only vary hit distances), so a local
    // spiral is fine for the timing harness.
    let ray_table = device
        .create_buffer(&BufferDesc {
            size: (RAY_TABLE_RAYS * 16) as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("ray table");
    {
        let mapped = device.buffer_mapped_ptr(&ray_table).expect("ray table mapped");
        let golden = core::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        let mut words = vec![0u32; RAY_TABLE_RAYS * 4];
        for i in 0..RAY_TABLE_RAYS {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / RAY_TABLE_RAYS as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = i as f32 * golden;
            let dir = [r * phi.cos(), r * phi.sin(), z, 0.0];
            for (k, &c) in dir.iter().enumerate() {
                words[i * 4 + k] = c.to_bits();
            }
        }
        write_words(mapped, &words);
    }

    // The edit-list SSBO (the real 16-edit CSG fold — `Buf` @0).
    let edit_list = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit list");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &grand_showcase_edits());
        let mapped = device.buffer_mapped_ptr(&edit_list).expect("edit list mapped");
        write_words(mapped, &header);
    }

    // The light-table SSBO (`LightBuf` @5). Seeded ONCE with ONE VALID DIRECTIONAL light at entry 0
    // (P1-1 fix — a zeroed light has `dir=[0,0,0]`, so the shader's `normalize(e.dir)` is degenerate,
    // `NoL=0`, and the `NoL <= 0.0 { continue }` guard SKIPS the shadow march — the shadow-ON row
    // would then measure the identical field-march-only cost as shadow-OFF, under-measuring the
    // DOMINANT cost multiplier per plan §5). The shadow-OFF row still measures no shadow because it
    // sets the UBO `light_count = 0` (the shade loop never runs); the shadow-ON row sets it to 1, so
    // every non-escaping probe-ray HIT pays exactly one real `sdf_soft_shadow_ranged` march — the
    // representative dominant cost (a directional reaches everywhere). The valid entry stays inert on
    // the OFF row (loop count 0), so a single boot-time seed drives BOTH rows.
    let light_table = device
        .create_buffer(&BufferDesc {
            size: (LIGHT_TABLE_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("light table");
    {
        let mapped = device.buffer_mapped_ptr(&light_table).expect("light table mapped");
        write_words(mapped, &directional_light_table());
    }

    // The b6 update UBO (48 B, host-coherent — rewritten per config).
    let update_ubo = device
        .create_buffer(&BufferDesc {
            size: 48,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("update UBO");

    // The 7-binding update set layout (matching `sdf_probe_update.comp` set 0). One layout drives
    // every variant.
    let layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("update set layout");
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &edit_list },
                BindGroupEntry::StorageImage { texture: atlas.irradiance() },
                BindGroupEntry::StorageImage { texture: atlas.depth() },
                BindGroupEntry::StorageBuffer { buffer: atlas.classification() },
                BindGroupEntry::StorageBuffer { buffer: &ray_table },
                BindGroupEntry::StorageBuffer { buffer: &light_table },
                BindGroupEntry::UniformBuffer { buffer: &update_ubo },
            ],
        })
        .expect("update bind group");

    // Measure the empty-submit overhead ONCE: a fenced submit of an EMPTY command buffer (no
    // dispatch), the fixed cost subtracted from every measured dispatch (plan §5 step 4).
    let overhead_us = measure_empty_submit(device, &queue);
    println!(
        "ddgi_probe_gi_cost: empty-submit overhead = {overhead_us:.1} µs (subtracted from all \
         measurements); probe_count = {DDGI_PROBE_COUNT}"
    );
    // P1-1 caveat: the shadow march runs only on a probe-ray HIT (inside `shade_hit`). The edit-list
    // fills the default grid volume so most probes hit, but any probe whose rays escape to sky pays
    // NO shade/shadow cost — so the shadow-ON `p95` is a LOWER BOUND if a significant fraction of
    // probes sit outside the field. The one directional light reaches everywhere, so every HIT pays
    // exactly one shadow march (the representative dominant cost); a fuller-occupancy scene can only
    // raise the shadow-ON cost, never lower it. The orchestrator should apply a safety margin when
    // deriving the cadence against the ~3 ms ceiling.
    println!(
        "ddgi_probe_gi_cost: NOTE shadow-ON cost is a LOWER BOUND — only probe rays that HIT the \
         grid-filling edit-list pay the shadow march (sky-escaping probes pay none)."
    );

    // The sweep axes (plan §5).
    let default_grid = [16u32, 8, 16];
    let coarse_grid = [12u32, 6, 12];
    let rays_sweep = [16u32, 32, 64, 128];
    let subset_sweep = [1u32, 2, 4, 8];
    let shadow_sweep = [false, true];

    println!(
        "{:>8} {:>8} {:>8} {:>12} {:>8} {:>12} {:>12} {:>12}",
        "rays", "subset", "gi_it", "grid", "shadow", "median_us", "p95_us", "stddev_us"
    );

    for &gi_max_it in &GI_MAX_IT_VARIANTS {
        // Rebuild the pipeline for this GI_MAX_IT variant (measured==shipped).
        let module = device
            .create_shader_module(sdf_probe_update_spirv(gi_max_it))
            .expect("probe-update shader module");
        let pipeline = device
            .create_compute_pipeline(&ComputePipelineDesc {
                module: &module,
                entry: c"main",
                // The update shader uses NO push constant (every param rides the b6 UBO), but this
                // RHI mandates a NON-EMPTY multiple-of-4 shared compute push range (rhi_impl.rs
                // 1208-1217 rejects 0), so declare the standard 4-byte range every other compute
                // pipeline in the tree carries. Vulkan allows a layout to declare a push range the
                // shader never reads; the recorder pushes nothing.
                push_constant_bytes: 4,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            })
            .expect("probe-update compute pipeline");

        for &grid_dims in &[default_grid, coarse_grid] {
            let probe_count = grid_dims[0] * grid_dims[1] * grid_dims[2];
            for &shadow_light in &shadow_sweep {
                for &subset_n in &subset_sweep {
                    // Skip a subset that does not divide this grid's probe count (a ragged residue
                    // class — plan §4 P1-5; the shader debug-asserts it).
                    if probe_count % subset_n != 0 {
                        continue;
                    }
                    for &rays_per_probe in &rays_sweep {
                        let cfg = Config { rays_per_probe, subset_n, gi_max_it, grid_dims, shadow_light };
                        let summary = measure_config(
                            device,
                            &queue,
                            &pipeline,
                            &bind_group,
                            &update_ubo,
                            &cfg,
                            probe_count,
                            overhead_us,
                        );
                        println!(
                            "{:>8} {:>8} {:>8} {:>4}x{:>1}x{:>2} {:>8} {:>12.1} {:>12.1} {:>12.1}",
                            cfg.rays_per_probe,
                            cfg.subset_n,
                            cfg.gi_max_it,
                            cfg.grid_dims[0],
                            cfg.grid_dims[1],
                            cfg.grid_dims[2],
                            if cfg.shadow_light { "on" } else { "off" },
                            summary.median_us,
                            summary.p95_us,
                            summary.stddev_us,
                        );
                    }
                }
            }
        }

        // SAFETY: the pipeline + module were created on `device`; every submission referencing them
        // completed (each `measure_config` fence-waits before returning), so neither is in use.
        unsafe {
            device.destroy_compute_pipeline(pipeline);
            device.destroy_shader_module(module);
        }
    }

    // SAFETY: every resource below was created on `device` and is destroyed exactly once; the last
    // submission completed (the final `measure_config` fence-waited), so none is GPU-referenced.
    unsafe {
        device.destroy_bind_group(bind_group);
        device.destroy_bind_group_layout(layout);
        device.destroy_buffer(update_ubo);
        device.destroy_buffer(light_table);
        device.destroy_buffer(edit_list);
        device.destroy_buffer(ray_table);
        atlas.destroy(device);
    }
}

/// Measures the fixed empty-submit overhead: `ITERS` fenced submits of an EMPTY command buffer,
/// wall-clocked, median reported (the stable fixed cost — plan §5 step 6 also cross-checks its
/// stability).
fn measure_empty_submit(device: &VulkanContext, queue: &impl RhiQueue<boyko_rhi_vulkan::rhi_impl::Vulkan>) -> f64 {
    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        encoder.end().expect("end");
        let t = Instant::now();
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        // SAFETY: created on `device`, the submission fence-waited above ⇒ not GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }
    let usable = &samples[WARMUP..];
    summarize(usable).median_us
}

/// Times one `Config`: writes its UBO, then `ITERS` fenced reset→bind→dispatch submits, discards
/// the first `WARMUP`, subtracts the empty-submit `overhead_us`, and summarizes.
#[allow(clippy::too_many_arguments)]
fn measure_config(
    device: &VulkanContext,
    queue: &impl RhiQueue<boyko_rhi_vulkan::rhi_impl::Vulkan>,
    pipeline: &boyko_rhi_vulkan::rhi_impl::ComputePipeline,
    bind_group: &boyko_rhi_vulkan::rhi_impl::VulkanBindGroup,
    update_ubo: &boyko_rhi_vulkan::memory::BoundBuffer,
    cfg: &Config,
    probe_count: u32,
    overhead_us: f64,
) -> Summary {
    // Write the per-config UBO (origin/spacing arbitrary — they only shift hit distances; the cost
    // model is dominated by rays × GI_MAX_IT × edits regardless of the exact geometry).
    {
        let ubo = DdgiUpdateUbo {
            origin: [-16.0, -2.0, -16.0, 2.0],
            grid_dims: [cfg.grid_dims[0], cfg.grid_dims[1], cfg.grid_dims[2], 0],
            frame_index: 0,
            subset_n: cfg.subset_n,
            rays_per_probe: cfg.rays_per_probe,
            light_count: u32::from(cfg.shadow_light),
        };
        let mapped = device.buffer_mapped_ptr(update_ubo).expect("update UBO mapped");
        write_words(mapped, &bytemuck_u32s(&ubo.as_bytes()));
    }

    let groups = cfg.dispatch_groups(probe_count);
    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        encoder.bind_compute_pipeline(pipeline);
        encoder.bind_descriptor_set_compute(bind_group, pipeline);
        encoder.dispatch(groups, 1, 1);
        encoder.end().expect("end");
        let t = Instant::now();
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        // SAFETY: created on `device`, the submission fence-waited above ⇒ not GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    // Subtract the fixed overhead from each sample (never below 0), discard the warm-up.
    let corrected: Vec<f64> =
        samples[WARMUP..].iter().map(|&s| (s - overhead_us).max(0.0)).collect();
    summarize(&corrected)
}

/// Re-views a 48-byte UBO image as its 12 `u32` words for the host-coherent write (the write helper
/// is `u32`-granular; `48 == 12 * 4`).
fn bytemuck_u32s(bytes: &[u8; 48]) -> Vec<u32> {
    let mut out = Vec::with_capacity(12);
    for chunk in bytes.chunks_exact(4) {
        out.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    out
}

/// The bench entry (`#[ignore]` — a measurement, not a pass/fail gate). Boots offscreen, runs the
/// full sweep, prints per-config `median / p95 / stddev`. The orchestrator reads these numbers to
/// derive the shipped cadence; nothing here asserts a derived value.
#[test]
#[ignore = "cost measurement (RTX + --nocapture --test-threads=1); the orchestrator runs it"]
fn ddgi_probe_gi_cost() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    println!("ddgi_probe_gi_cost on: {}", ctx.device_name());
    run_sweep(&ctx);
}
