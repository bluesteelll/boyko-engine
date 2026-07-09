//! HW-RT rung R0 — the software-ray baseline COST harness, measured with GPU timestamps
//! (`#[ignore]`, plan `docs/RENDER-R0-INSTRUMENT-PLAN.md`).
//!
//! This rung times the dominant software-ray pass — the SDFDDGI probe-update dispatch (the
//! probe field-march + soft-shadow march + blend) — with the R0 GPU timestamp-query
//! primitive: a per-frame `vkCmdResetQueryPool` + a `TOP_OF_PIPE`/`BOTTOM_OF_PIPE` bracket
//! (`vkCmdWriteTimestamp`) around the dispatch, read back with `vkGetQueryPoolResults`
//! (`64_BIT | WAIT_BIT`) after the fence. Unlike the CPU-wall-clock precedent
//! (`ddgi_probe_gi_cost`), the timestamps bracket the pass ON-DEVICE — a strict improvement
//! (no empty-submit subtraction, no host scheduling noise in the measured window).
//!
//! # The four-pass bracket on the real combined frame
//!
//! The R0 primitive + the gated `GBufferScene::gpu_timing` collector bracket ALL FOUR
//! software-ray passes (DDGI update, deferred resolve incl. the inline SDF shadow march, CSM
//! cascade depth, punctual atlas depth) inside the real showcase frame — see
//! `present::passes::gbuffer` (the `Some`-gated brackets) and `present::gpu_timing`. That
//! combined-frame path is byte-identical when `gpu_timing == None` (the framegraph
//! byte-identity golden + the grand_showcase pixel dump both run with `None`). This
//! standalone harness exercises the primitive END-TO-END (create pool → reset → bracket →
//! dispatch → fence → read ns) on the DDGI-update pass in ISOLATION, which is the one pass a
//! self-contained boot can drive without the full showcase scene wiring; it reports the
//! per-pass GPU wall-clock (ns/pass) + derived ns/ray the orchestrator reads to size the
//! HW-RT cadence.
//!
//! # Named `software_ray_baseline_cost`, NOT `..._update_time_setup`
//!
//! A test/exe name containing "update"/"time"/"setup" triggers Windows os-error-740 (UAC
//! elevation) on the target box (the `ddgi_probe_gi_cost` precedent), so this file avoids all
//! three substrings.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test software_ray_baseline_cost -- --ignored
//! --nocapture --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1` (validation is crash-prone
//! on the box; the orchestrator runs it on the RTX).

use core::ptr::NonNull;

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, ComputePipelineDesc, DescriptorKind, MemoryLocation, QueryPoolDesc,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage, TimestampStage,
};

use boyko_rhi_vulkan::compute::{
    EDITLIST_BUFFER_WORDS, encode_edit_list, sdf_op, sdf_probe_update_spirv, SdfEdit,
};
use boyko_rhi_vulkan::ddgi::{DDGI_GRID_DIM_X, DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z, DDGI_PROBE_COUNT, DdgiAtlas};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::present::{FRAMES_IN_FLIGHT, PASS_COUNT, TimedPass, TimestampCollector};
use boyko_rhi_vulkan::rhi_impl::VulkanQueryPool;

// ---- the b6 update UBO byte-mirror (the harness does not depend on boyko_render) ----------

/// The number of `float4`s in the Fibonacci ray table (the shader's groupshared cache bound).
const RAY_TABLE_RAYS: usize = 128;

/// The showcase's I4 probe-update ray count (`DDGI_UPDATE_RAYS` — the ns/ray denominator). The
/// grand_showcase GI-ON path dispatches `subset_n = 1` → one block per probe, 64 rays/probe.
const RAYS_PER_PROBE: u32 = 64;

/// The GI_MAX_IT sphere-trace iteration cap the showcase ships (the measured==shipped variant).
const GI_MAX_IT: u32 = 64;

/// The light-table word budget: a 16-word header + a generous `GpuLight[]` span (12 words each).
const LIGHT_TABLE_WORDS: usize = 16 + 12 * 64;

/// The b6 `DdgiUpdate` cbuffer byte-mirror (48 B, the committed shader's field order).
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
        // SAFETY: `#[repr(C)]`, 48-byte const-asserted layout, all-POD fields — every bit
        // pattern is valid, so the transmute reads only initialized bytes.
        unsafe { core::mem::transmute::<Self, [u8; 48]>(*self) }
    }
}

/// The reported per-pass timing summary (all in nanoseconds, GPU wall-clock).
#[derive(Clone, Copy, Debug)]
struct Summary {
    median_ns: f64,
    p95_ns: f64,
    stddev_ns: f64,
}

/// The number of timed frames (`>= 200`, plan Part C step 4).
const FRAMES: usize = 220;
/// The warm-up frames discarded from the front (shader compile + GPU clock ramp).
const WARMUP: usize = 20;

/// Boots an offscreen context (validation OFF — the harness measures cost, not correctness),
/// or `None` with a SKIP log when no GPU / loader is present.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP software_ray_baseline_cost: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Writes `words` `u32`s into a host-coherent mapping (valid before the submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    // SAFETY: `base` points to a host-coherent mapping of at least `words.len() * 4` bytes
    // (the caller sizes the buffer to fit); the write is host-unique before any GPU work.
    unsafe {
        core::ptr::copy_nonoverlapping(words.as_ptr(), base.as_ptr().cast::<u32>(), words.len());
    }
}

/// A `grand_showcase`-shaped CSG edit-list (the representative 16-edit fold — the real per-ray
/// field-evaluation cost). Verbatim shape from the `ddgi_probe_gi_cost` precedent so the
/// measured march cost is comparable to the CPU-wall-clock baseline.
fn grand_showcase_edits() -> Vec<SdfEdit> {
    const CENTER: [f32; 3] = [-1.0, 5.0, -1.0];
    let mut edits = Vec::with_capacity(16);
    edits.push(SdfEdit::sphere(CENTER, 9.0, sdf_op::UNION, 0.0));
    edits.push(SdfEdit::sphere([CENTER[0] + 5.0, CENTER[1], CENTER[2]], 4.0, sdf_op::SUBTRACT, 0.6));
    edits.push(SdfEdit::box_shape([CENTER[0], CENTER[1] - 5.0, CENTER[2]], [8.0, 1.5, 8.0], sdf_op::UNION, 1.0));
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

/// One valid DIRECTIONAL light at entry 0, so a probe-ray HIT pays exactly one real
/// `sdf_soft_shadow_ranged` march (the representative dominant cost — a directional reaches
/// everywhere). Verbatim from the `ddgi_probe_gi_cost` seed (the `light_table.hlsli` layout: a
/// 16-word header then a flat `GpuLight[]` of 12 words each from `LIGHT_HEADER_BASE = 16`).
fn directional_light_table() -> Vec<u32> {
    const LIGHT_KIND_DIRECTIONAL: u32 = 0;
    const HEADER_WORDS: usize = 16;
    const ENTRY0_BASE: usize = HEADER_WORDS;
    let mut words = vec![0u32; LIGHT_TABLE_WORDS];
    words[0] = 1;
    let dir = {
        let v = [0.3_f32, -1.0, 0.2];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    };
    words[ENTRY0_BASE] = dir[0].to_bits();
    words[ENTRY0_BASE + 1] = dir[1].to_bits();
    words[ENTRY0_BASE + 2] = dir[2].to_bits();
    words[ENTRY0_BASE + 3] = LIGHT_KIND_DIRECTIONAL;
    words[ENTRY0_BASE + 8] = 3.0_f32.to_bits();
    words[ENTRY0_BASE + 9] = 3.0_f32.to_bits();
    words[ENTRY0_BASE + 10] = 3.0_f32.to_bits();
    words
}

/// Summarizes a slice of ns samples to a `Summary` (median + p95 + stddev). Sorts a copy for
/// the percentiles.
fn summarize(samples_ns: &[f64]) -> Summary {
    let mut s = samples_ns.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    let median_ns = s[n / 2];
    let p95_idx = ((n as f64) * 0.95).ceil() as usize;
    let p95_ns = s[p95_idx.min(n - 1)];
    let mean = s.iter().sum::<f64>() / n as f64;
    let var = s.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64;
    Summary { median_ns, p95_ns, stddev_ns: var.sqrt() }
}

/// Re-views a 48-byte UBO image as its 12 `u32` words for the host-coherent write.
fn ubo_u32s(bytes: &[u8; 48]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// The measurement entry (`#[ignore]` — a measurement, not a pass/fail gate). Boots offscreen,
/// times the DDGI-update pass with GPU timestamps, prints `median / p95 / stddev` (ns) +
/// ns/ray. The orchestrator reads these to size the HW-RT cadence.
#[test]
#[ignore = "GPU-timestamp cost measurement (RTX + --nocapture --test-threads=1); the orchestrator runs it"]
fn software_ray_baseline_cost() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    println!("software_ray_baseline_cost on: {}", ctx.device_name());

    let caps = ctx.device_caps();
    // Graceful-skip (plan Part C step 1): a device with no valid timestamp bits or an
    // implausible period (or a wrong `timestampPeriod` offset the plausibility guard catches)
    // cannot be measured — print a skip line + return, NEVER panic.
    if !caps.timestamps_usable() {
        println!(
            "SKIP software_ray_baseline_cost: GPU timestamps unusable (valid_bits={}, period={} ns/tick)",
            caps.timestamp_valid_bits, caps.timestamp_period
        );
        return;
    }
    // DDGI storage is the precondition for the update dispatch (the atlas needs B10G11R11 +
    // RG16F storage images). Degrade gracefully (DDGI is opt-in) — skip, do not crash.
    if !caps.ddgi_storage_ok() {
        println!("SKIP software_ray_baseline_cost: device lacks B10G11R11/RG16F STORAGE for the DDGI atlas");
        return;
    }
    println!(
        "software_ray_baseline_cost: timestamps OK (valid_bits={}, period={} ns/tick, mask=0x{:x})",
        caps.timestamp_valid_bits,
        caps.timestamp_period,
        caps.timestamp_mask()
    );

    run(&ctx);
}

/// Boots the DDGI-update resources, times the pass over `FRAMES` fenced frames with the R0
/// timestamp bracket, and reports per-pass ns + ns/ray. Every GPU-facing resource is created
/// once + reused; teardown waits idle then destroys in reverse dependency order.
fn run(ctx: &VulkanContext) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    let atlas = DdgiAtlas::create(device).expect("DDGI atlas create");

    // The Fibonacci ray table (128 `float4`s, STORAGE). A golden-angle spiral fills it (the
    // exact directions do not change the cost — they only vary hit distances).
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

    // The edit-list SSBO (the representative CSG fold — `Buf` @0).
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

    // The light-table SSBO (`LightBuf` @5) — ONE directional so every HIT pays a shadow march.
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

    // The b6 update UBO (48 B, host-coherent — written once; the harness does not rotate rays).
    let update_ubo = device
        .create_buffer(&BufferDesc {
            size: 48,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("update UBO");
    {
        let ubo = DdgiUpdateUbo {
            origin: [-5.25, 0.20, -6.00, 0.70],
            grid_dims: [DDGI_GRID_DIM_X, DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z, 0],
            frame_index: 0,
            subset_n: 1,
            rays_per_probe: RAYS_PER_PROBE,
            light_count: 1,
        };
        let mapped = device.buffer_mapped_ptr(&update_ubo).expect("update UBO mapped");
        write_words(mapped, &ubo_u32s(&ubo.as_bytes()));
    }

    // The 7-binding update set layout (matching `sdf_probe_update.comp` set 0).
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

    let module = device
        // A-1: `GI_MAX_IT` is now a spec-const (id 0, default 64); this baseline ships the default
        // (`GI_MAX_IT == 64`), so `spec_constants: &[]` below resolves it byte-identically.
        .create_shader_module(sdf_probe_update_spirv())
        .expect("probe-update shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // The update shader reads no push constant (every param rides b6), but the RHI
            // mandates a non-empty multiple-of-4 shared push range; declare the standard 4 bytes.
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
            spec_constants: &[],
        })
        .expect("probe-update compute pipeline");

    // The R0 collector: one `2 * PASS_COUNT`-query TIMESTAMP pool per in-flight frame.
    let pools: [VulkanQueryPool; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
        device
            .create_query_pool(&QueryPoolDesc { count: 2 * PASS_COUNT })
            .expect("timestamp query pool")
    });
    let collector = TimestampCollector::new(pools);

    // The DDGI-update dispatch is `DDGI_PROBE_COUNT / subset_n` blocks (subset_n = 1 → one
    // `[numthreads(64,1,1)]` block per probe).
    let groups = DDGI_PROBE_COUNT;

    let mut scratch = [0u64; (2 * PASS_COUNT) as usize];
    let mut out_ns = [0.0f64; PASS_COUNT as usize];
    let mut samples: Vec<f64> = Vec::with_capacity(FRAMES);

    for frame in 0..FRAMES {
        // Ring the query pool by the frame's in-flight slot (matches the renderer's `fi`).
        let fi = frame % FRAMES_IN_FLIGHT;
        let pool = collector.pool(fi);

        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        // R0: reset ALL queries at the frame top (a compute-only prologue — trivially outside
        // any render pass), then bracket the DDGI-update dispatch TOP..BOTTOM.
        encoder.reset_query_pool(pool, 0, 2 * PASS_COUNT);
        encoder.write_timestamp(pool, TimestampStage::TopOfPipe, 2 * TimedPass::DdgiUpdate.slot());
        encoder.bind_compute_pipeline(&pipeline);
        encoder.bind_descriptor_set_compute(&bind_group, &pipeline);
        encoder.dispatch(groups, 1, 1);
        encoder.write_timestamp(pool, TimestampStage::BottomOfPipe, 2 * TimedPass::DdgiUpdate.slot() + 1);
        encoder.end().expect("end");

        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");

        // R0: read back the masked/period-scaled ns for the ONE written pair (DdgiUpdate, queries
        // 0,1). This harness brackets ONLY the DDGI-update dispatch, so only pair 0 is written; the
        // other 6 queries were reset-but-never-written. Reading them with `VK_QUERY_RESULT_WAIT_BIT`
        // would BLOCK FOREVER (an unwritten query never becomes available) — so read exactly
        // `pair_count = 1`, not `PASS_COUNT`. `out_ns[0]` is the DdgiUpdate duration.
        device
            .read_query_pool_ns(pool, 1, &mut scratch, &mut out_ns)
            .expect("read_query_pool_ns");
        samples.push(out_ns[0]);

        // SAFETY: created on `device`, the submission fence-waited above ⇒ not GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    let kept = &samples[WARMUP..];
    let summary = summarize(kept);
    // ns/ray: DDGI update casts `DDGI_PROBE_COUNT * RAYS_PER_PROBE` rays (subset_n = 1).
    let rays = (DDGI_PROBE_COUNT * RAYS_PER_PROBE) as f64;

    println!(
        "software_ray_baseline_cost: DDGI probe-update pass (GI_MAX_IT={GI_MAX_IT}, rays/probe={RAYS_PER_PROBE}, \
         probes={DDGI_PROBE_COUNT}, kept {}/{FRAMES} frames):",
        kept.len()
    );
    println!(
        "  median = {:.1} ns ({:.3} ms), p95 = {:.1} ns ({:.3} ms), stddev = {:.1} ns",
        summary.median_ns,
        summary.median_ns / 1e6,
        summary.p95_ns,
        summary.p95_ns / 1e6,
        summary.stddev_ns
    );
    println!(
        "  ns/ray: median = {:.3} ns, p95 = {:.3} ns (over {} rays)",
        summary.median_ns / rays,
        summary.p95_ns / rays,
        rays as u64
    );
    println!(
        "  NOTE: TOP/BOTTOM brackets the whole pass wall-clock (inclusive of pipeline overlap), \
         not isolated kernel time; the four-pass combined-frame bracket runs through \
         GBufferScene::gpu_timing (byte-identical when None)."
    );

    // Teardown: wait idle, then destroy the query pools + all resources in reverse dependency
    // order (the last submission fence-waited, so nothing is GPU-referenced).
    device.wait_idle().expect("wait_idle");
    // SAFETY: `wait_idle` above completed every submission; each resource was created on
    // `device` and is destroyed exactly once. The collector's pools are moved out for
    // destruction (the collector is dropped first, releasing its borrow of the pools' data —
    // it owns no GPU objects itself, only the `VulkanQueryPool` values it now yields back).
    unsafe {
        for pool in collector.into_pools() {
            device.destroy_query_pool(pool);
        }
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_bind_group(bind_group);
        device.destroy_bind_group_layout(layout);
        device.destroy_buffer(update_ubo);
        device.destroy_buffer(light_table);
        device.destroy_buffer(edit_list);
        device.destroy_buffer(ray_table);
        atlas.destroy(device);
    }
}
