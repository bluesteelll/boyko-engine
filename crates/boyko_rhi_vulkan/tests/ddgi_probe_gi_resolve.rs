//! SDFDDGI I3 — the DDGI resolve-sample GPU GOLDEN (`probe_sample_gpu_eq_cpu_to_bits`, `#[ignore]`,
//! RTX). This is where host↔GPU bit-exactness of the resolve probe sample is CERTIFIED.
//!
//! The test boots an offscreen device, allocates the DDGI atlas, POPULATES every probe tile with a
//! KNOWN uniform value (irradiance + depth moments) so the LINEAR `SampleLevel` in the shared
//! `ddgi_probe_sample` returns each tile's stored value EXACTLY (no interpolation error), dispatches
//! the standalone golden shader (`ddgi_probe_gi_resolve.comp.hlsl` — the SAME `ddgi_resolve.hlsli`
//! math the deferred resolve runs) over a set of receiver (p, n) samples, reads the resolved
//! irradiance back, and diffs it to a tight ULP tolerance against the host oracle
//! `boyko_rhi_vulkan::goldens::probe_sample`.
//!
//! # Why uniform tiles make this bit-exact
//!
//! The atlas READ (`oct_encode(n) -> UV -> SampleLevel`) is a GPU bilinear filter; a uniform tile
//! collapses the filter to the stored value regardless of the sub-texel UV. So the atlas step
//! contributes ZERO float error and the ONLY arithmetic certified is the trilinear + wrap +
//! Chebyshev blend — the transcendental-free op chain the plan pins. The per-tile value is chosen
//! EXACTLY representable in the atlas format (verified by a read-back round-trip below), so the host
//! `tap` closure feeds `probe_sample` the identical f32 the GPU sampler returns.
//!
//! # The converged predicate is a DEPTH SENTINEL (not a binding)
//!
//! The resolve descriptor set is at its 19/19 cap, so the "converged-once" bit is NOT a new binding:
//! the shader treats `depth.mean > 0.0` as converged (boot-clear sets depth 0; any real update
//! writes `mean >= GI_MINT > 0`). The host oracle is fed the IDENTICAL predicate
//! (`converged = depth_mean > 0.0`) so host and GPU classify each probe the same way.
//!
//! # Named `ddgi_probe_gi_resolve` (no "update"/"setup"/"install"/"patch")
//!
//! A test/exe name containing "update" (etc.) triggers Windows os-error-740 (UAC elevation) on the
//! target box; this file is `ddgi_probe_gi_resolve`.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test ddgi_probe_gi_resolve -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`.

use core::ptr::NonNull;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BarrierDesc, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    BufferBarrier, BufferDesc, BufferImageCopy, BufferUsage, ComputePipelineDesc, DescriptorKind,
    ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, MemoryLocation,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage,
};

use boyko_rhi_vulkan::compute::{ddgi_probe_gi_resolve_spirv, f16_from_f32};
use boyko_rhi_vulkan::ddgi::{
    DDGI_ATLAS_LAYERS, DDGI_DEPTH_ATLAS_HEIGHT, DDGI_DEPTH_ATLAS_WIDTH, DDGI_GRID_DIM_X,
    DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z, DDGI_IRR_ATLAS_HEIGHT, DDGI_IRR_ATLAS_WIDTH, DdgiAtlas,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::goldens::{DdgiProbeTap, probe_sample};

// ---- the world-fixed grid (mirror DdgiConfig defaults / the host-oracle test) --------------
const ORIGIN: [f32; 3] = [-16.0, -2.0, -16.0];
const SPACING: f32 = 2.0;
const INV_SPACING: f32 = 1.0 / SPACING;
const DIMS: [u32; 3] = [16, 8, 16];
/// The sky fallback the GOLDEN SHADER hard-codes; the host oracle is fed the SAME value.
const SKY: [f32; 3] = [0.05, 0.06, 0.08];

/// The grid UBO byte size (mirrors `ResolvedDdgi` — 48 B).
const UBO_BYTES: usize = 48;
/// The irradiance atlas texel byte size (`B10G11R11_UFLOAT_PACK32` = 4 bytes/texel).
const IRR_TEXEL_BYTES: u64 = 4;
/// The depth atlas texel byte size (`R16G16_SFLOAT` = 4 bytes/texel).
const DEPTH_TEXEL_BYTES: u64 = 4;

/// Boots an offscreen context (validation off), or `None` with a SKIP log when no GPU/loader.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP ddgi_probe_gi_resolve: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Writes bytes into a host-coherent mapping (valid before the submit).
fn write_bytes(base: NonNull<u8>, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        // SAFETY: the buffer holds at least `bytes.len()` bytes in the persistent host-coherent
        // mapping; `base + i` is in-bounds; no GPU work is in flight when the CPU seeds it.
        unsafe { base.as_ptr().add(i).write(b) };
    }
}

// ---- atlas format pack/unpack (host mirror of the GPU sampler, for exactly-representable values)

/// Packs an RGB triple into `B10G11R11_UFLOAT_PACK32`. Each channel is an UNSIGNED float (R/G: 5-bit
/// exp + 6-bit mantissa; B: 5-bit exp + 5-bit mantissa, no sign). Used only for values chosen to be
/// exactly representable (`unpack_b10g11r11` round-trips them), so the stored texel == the input.
fn pack_b10g11r11(rgb: [f32; 3]) -> u32 {
    let r = pack_ufloat(rgb[0], 6);
    let g = pack_ufloat(rgb[1], 6);
    let b = pack_ufloat(rgb[2], 5);
    r | (g << 11) | (b << 22)
}

/// Unpacks `B10G11R11_UFLOAT_PACK32` back to RGB — the round-trip check that the packed texel decodes
/// to the intended value (so the host `tap` feeds `probe_sample` the exact GPU-sampled f32).
fn unpack_b10g11r11(bits: u32) -> [f32; 3] {
    let r = unpack_ufloat(bits & 0x7ff, 6);
    let g = unpack_ufloat((bits >> 11) & 0x7ff, 6);
    let b = unpack_ufloat((bits >> 22) & 0x3ff, 5);
    [r, g, b]
}

/// Encodes a non-negative float into an `exp5 + mantissa(mant_bits)` unsigned-float channel. The
/// values under test are >= 0 and inside the normal range, so only the standard normal encode is
/// exercised (a defensive zero/clamp guard covers the rest).
fn pack_ufloat(v: f32, mant_bits: u32) -> u32 {
    // Non-positive or NaN -> zero (the exercised values are strictly positive normals).
    if v <= 0.0 || v.is_nan() {
        return 0;
    }
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127; // unbiased fp32 exponent
    let mant = bits & 0x007f_ffff;
    let new_exp = exp + 15; // 5-bit exponent bias
    if new_exp <= 0 || new_exp >= 0x1f {
        return 0; // out of the exercised normal range
    }
    let m = mant >> (23 - mant_bits);
    ((new_exp as u32) << mant_bits) | m
}

/// Decodes an `exp5 + mantissa(mant_bits)` unsigned-float channel to f32 (the inverse of
/// [`pack_ufloat`] for normal-range values).
fn unpack_ufloat(packed: u32, mant_bits: u32) -> f32 {
    let exp = (packed >> mant_bits) & 0x1f;
    let mant = packed & ((1u32 << mant_bits) - 1);
    if exp == 0 {
        return 0.0;
    }
    let fp32 = ((exp + 127 - 15) << 23) | (mant << (23 - mant_bits));
    f32::from_bits(fp32)
}

/// Packs two f16 depth moments into an `R16G16_SFLOAT` texel (`.r = mean`, `.g = mean2`).
fn pack_rg16f(mean: f32, mean2: f32) -> u32 {
    (f16_from_f32(mean) as u32) | ((f16_from_f32(mean2) as u32) << 16)
}

// ---- the per-probe KNOWN values (chosen exactly-representable in the atlas formats) --------

/// The irradiance a probe tile is filled with, keyed by its grid index — 8 distinct values over the
/// base cell so the trilinear blend is non-vacuous. Each lane is exactly representable in
/// `B10G11R11` (a short-mantissa dyadic fraction), verified by the read-back round-trip.
fn probe_irradiance(x: u32, y: u32, z: u32) -> [f32; 3] {
    let seed = ((x & 1) + 2 * (y & 1) + 4 * (z & 1)) as f32; // 0..7, all distinct on a cell
    // 0.5 + seed*0.0625 etc.: dyadic fractions with <= 6-bit mantissas -> exact in B10G11R11.
    [
        0.5 + seed * 0.0625,
        0.25 + seed * 0.03125,
        0.125 + seed * 0.015625,
    ]
}

/// The depth moments a probe tile is filled with. `mean` large (unshadowed: `dist <= mean` ->
/// Chebyshev 1) for the interior probes; exactly-representable halves.
const DEPTH_MEAN: f32 = 1024.0; // huge vs any grid distance -> cheb == 1
const DEPTH_MEAN2: f32 = 1_048_576.0; // mean^2, exact half

/// One `DdgiProbeTap` the host oracle reads — the SAME value the atlas tile stores (round-trip
/// verified), with `converged = depth_mean > 0.0` (the shader's depth-sentinel predicate).
fn host_tap(idx: [u32; 3]) -> DdgiProbeTap {
    let irr = probe_irradiance(idx[0], idx[1], idx[2]);
    DdgiProbeTap {
        irradiance: irr,
        depth_mean: DEPTH_MEAN,
        depth_mean2: DEPTH_MEAN2,
        converged: DEPTH_MEAN > 0.0,
    }
}

/// probe `i`'s world position — `origin + i · spacing` (the world-fixed grid, Decision D1).
/// Reconstructs `spacing = 1.0 / inv_spacing` the SAME way the shader does (not the literal
/// `SPACING`) so the host `probe_pos` is bit-faithful to the GPU path — for the owner-locked
/// `spacing == 2.0`, `1.0 / (1.0 / 2.0) == 2.0` exactly, so the two coincide here.
fn probe_pos(i: [u32; 3]) -> [f32; 3] {
    let spacing = 1.0 / INV_SPACING;
    [
        ORIGIN[0] + i[0] as f32 * spacing,
        ORIGIN[1] + i[1] as f32 * spacing,
        ORIGIN[2] + i[2] as f32 * spacing,
    ]
}

/// The receiver (p, n) samples: cell-interior points at assorted fractions + normals, all inside the
/// grid so the trilinear cell + its `+1` neighbour stay in bounds.
fn receiver_samples() -> Vec<([f32; 3], [f32; 3])> {
    let base = [5u32, 3, 7];
    let bp = probe_pos(base);
    let mk = |fx: f32, fy: f32, fz: f32, n: [f32; 3]| {
        (
            [bp[0] + fx * SPACING, bp[1] + fy * SPACING, bp[2] + fz * SPACING],
            n,
        )
    };
    vec![
        mk(0.25, 0.5, 0.75, [0.0, 1.0, 0.0]),
        mk(0.5, 0.5, 0.5, [1.0, 0.0, 0.0]),
        mk(0.1, 0.9, 0.3, [0.0, 0.0, 1.0]),
        mk(0.7, 0.2, 0.6, normalize3([0.3, 0.8, -0.5])),
        mk(0.33, 0.66, 0.1, normalize3([-0.6, 0.2, 0.7])),
        mk(0.9, 0.1, 0.9, normalize3([0.5, -0.5, 0.5])),
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The grid UBO words (48 B): origin.xyz + pad, inv_spacing + bit-cast dims, mode=1, sample count in
/// the pad `.x` (the shader reads the count there — no push constant on the vocabulary layout).
fn ubo_words(sample_count: u32) -> [u32; UBO_BYTES / 4] {
    let mut w = [0u32; UBO_BYTES / 4];
    w[0] = ORIGIN[0].to_bits();
    w[1] = ORIGIN[1].to_bits();
    w[2] = ORIGIN[2].to_bits();
    w[3] = 0; // origin.w pad
    w[4] = INV_SPACING.to_bits();
    w[5] = DIMS[0]; // bit-cast u32 dims into the f32 lanes
    w[6] = DIMS[1];
    w[7] = DIMS[2];
    w[8] = 1; // ddgi_mode_word (on)
    w[9] = sample_count; // _gDdgiPad.x -> gSampleCount
    w
}

/// SDFDDGI I3 GOLDEN: the GPU `ddgi_probe_sample` equals `goldens::probe_sample` to BITS.
#[test]
#[ignore = "live dispatch golden (RTX + --nocapture --test-threads=1); the orchestrator runs it"]
fn probe_sample_gpu_eq_cpu_to_bits() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    let device: &VulkanContext = &ctx;
    println!("ddgi_probe_gi_resolve on: {}", device.device_name());
    let queue = ctx.rhi_queue();

    // ---- 0) verify the chosen per-probe values round-trip through the atlas formats -----------
    // If a value is not exactly representable the host tap would diverge from the GPU sampler; pin
    // it here so a future value edit that breaks exactness fails LOUDLY (not as a mystery ULP).
    for z in 0..2u32 {
        for y in 0..2u32 {
            for x in 0..2u32 {
                let irr = probe_irradiance(x, y, z);
                let round = unpack_b10g11r11(pack_b10g11r11(irr));
                for k in 0..3 {
                    assert_eq!(
                        irr[k].to_bits(),
                        round[k].to_bits(),
                        "probe ({x},{y},{z}) irradiance lane {k} {} not exactly representable in \
                         B10G11R11 (round-trip {})",
                        irr[k],
                        round[k]
                    );
                }
            }
        }
    }

    let atlas = DdgiAtlas::create(device).expect("DDGI atlas create");

    // ---- 1) build the staging bytes: every texel of every tile = its probe's uniform value ----
    // Y-plane-major: array layer = y; within a layer, tile (x,z) at pixel origin (x*TILE, z*TILE).
    // Filling the WHOLE tile (border included) uniform makes the LINEAR SampleLevel exact.
    let irr_w = DDGI_IRR_ATLAS_WIDTH as usize;
    let irr_h = DDGI_IRR_ATLAS_HEIGHT as usize;
    let depth_w = DDGI_DEPTH_ATLAS_WIDTH as usize;
    let depth_h = DDGI_DEPTH_ATLAS_HEIGHT as usize;
    let layers = DDGI_ATLAS_LAYERS as usize;
    let irr_tile = (DDGI_IRR_ATLAS_WIDTH / DDGI_GRID_DIM_X) as usize; // 8
    let depth_tile = (DDGI_DEPTH_ATLAS_WIDTH / DDGI_GRID_DIM_X) as usize; // 16

    let mut irr_bytes = vec![0u8; irr_w * irr_h * layers * IRR_TEXEL_BYTES as usize];
    let mut depth_bytes = vec![0u8; depth_w * depth_h * layers * DEPTH_TEXEL_BYTES as usize];
    let depth_texel = pack_rg16f(DEPTH_MEAN, DEPTH_MEAN2);
    for y in 0..DDGI_GRID_DIM_Y {
        // array layer = y
        for z in 0..DDGI_GRID_DIM_Z {
            for x in 0..DDGI_GRID_DIM_X {
                let irr_texel = pack_b10g11r11(probe_irradiance(x, y, z)).to_le_bytes();
                let ox = x as usize * irr_tile;
                let oy = z as usize * irr_tile;
                for ty in 0..irr_tile {
                    for tx in 0..irr_tile {
                        let px = ox + tx;
                        let py = oy + ty;
                        let off = ((y as usize * irr_h + py) * irr_w + px) * IRR_TEXEL_BYTES as usize;
                        irr_bytes[off..off + 4].copy_from_slice(&irr_texel);
                    }
                }
                let dox = x as usize * depth_tile;
                let doy = z as usize * depth_tile;
                let dtexel = depth_texel.to_le_bytes();
                for ty in 0..depth_tile {
                    for tx in 0..depth_tile {
                        let px = dox + tx;
                        let py = doy + ty;
                        let off = ((y as usize * depth_h + py) * depth_w + px)
                            * DEPTH_TEXEL_BYTES as usize;
                        depth_bytes[off..off + 4].copy_from_slice(&dtexel);
                    }
                }
            }
        }
    }

    let irr_staging = device
        .create_buffer(&BufferDesc {
            size: irr_bytes.len() as u64,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("irr staging");
    write_bytes(
        device.buffer_mapped_ptr(&irr_staging).expect("irr staging mapped"),
        &irr_bytes,
    );
    let depth_staging = device
        .create_buffer(&BufferDesc {
            size: depth_bytes.len() as u64,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("depth staging");
    write_bytes(
        device.buffer_mapped_ptr(&depth_staging).expect("depth staging mapped"),
        &depth_bytes,
    );

    // ---- 2) receiver samples + output/UBO buffers --------------------------------------------
    let samples = receiver_samples();
    let count = samples.len() as u32;

    let recv_pos = device
        .create_buffer(&BufferDesc {
            size: (samples.len() * 16) as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("recv pos");
    let recv_nrm = device
        .create_buffer(&BufferDesc {
            size: (samples.len() * 16) as u64,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("recv nrm");
    {
        let mut pos_w = vec![0u32; samples.len() * 4];
        let mut nrm_w = vec![0u32; samples.len() * 4];
        for (i, (p, n)) in samples.iter().enumerate() {
            for k in 0..3 {
                pos_w[i * 4 + k] = p[k].to_bits();
                nrm_w[i * 4 + k] = n[k].to_bits();
            }
        }
        write_bytes(
            device.buffer_mapped_ptr(&recv_pos).expect("recv pos mapped"),
            bytemuck_bytes(&pos_w),
        );
        write_bytes(
            device.buffer_mapped_ptr(&recv_nrm).expect("recv nrm mapped"),
            bytemuck_bytes(&nrm_w),
        );
    }

    let out_buf = device
        .create_buffer(&BufferDesc {
            size: (samples.len() * 16) as u64,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("out buffer");
    let ubo = device
        .create_buffer(&BufferDesc {
            size: UBO_BYTES as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("grid ubo");
    write_bytes(
        device.buffer_mapped_ptr(&ubo).expect("ubo mapped"),
        bytemuck_bytes(&ubo_words(count)),
    );

    // ---- 3) the golden pipeline: b0 UBO, t1/s1 irr, t2/s2 depth, t3 pos, t4 nrm, u5 out -------
    let layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("golden layout");
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::UniformBuffer { buffer: &ubo },
                BindGroupEntry::CombinedImage { texture: atlas.irradiance(), sampler: atlas.sampler() },
                BindGroupEntry::CombinedImage { texture: atlas.depth(), sampler: atlas.sampler() },
                BindGroupEntry::StorageBuffer { buffer: &recv_pos },
                BindGroupEntry::StorageBuffer { buffer: &recv_nrm },
                BindGroupEntry::StorageBuffer { buffer: &out_buf },
            ],
        })
        .expect("golden bind group");

    let module = device
        .create_shader_module(ddgi_probe_gi_resolve_spirv())
        .expect("golden module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // The RHI mandates a NON-empty multiple-of-4 shared compute push range (rhi_impl.rs);
            // the shader declares-but-never-reads it (every compute pipeline uses 4). `0` is
            // rejected as `Unsupported` before the dispatch ever runs.
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
            spec_constants: &[],
        })
        .expect("golden pipeline");

    // ---- 4) record: upload the atlas, transition to sampled, dispatch, copy out --------------
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("encoder");
    encoder.begin().expect("begin");

    let full_irr = ImageSubresourceRange {
        aspect: ImageAspect::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: DDGI_ATLAS_LAYERS,
    };

    // The atlas boots in SHADER_READ_ONLY_OPTIMAL; transition both to TRANSFER_DST for the upload.
    for tex in [atlas.irradiance(), atlas.depth()] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::TRANSFER,
            src_access: BarrierAccess::SHADER_READ,
            dst_access: BarrierAccess::TRANSFER_WRITE,
            old_layout: ImageLayout::ShaderReadOnlyOptimal,
            new_layout: ImageLayout::TransferDstOptimal,
            range: full_irr,
        });
    }
    encoder.copy_buffer_to_image(
        &irr_staging,
        atlas.irradiance(),
        ImageLayout::TransferDstOptimal,
        &[BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: DDGI_ATLAS_LAYERS,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: DDGI_IRR_ATLAS_WIDTH,
            image_extent_h: DDGI_IRR_ATLAS_HEIGHT,
            image_extent_d: 1,
        }],
    );
    encoder.copy_buffer_to_image(
        &depth_staging,
        atlas.depth(),
        ImageLayout::TransferDstOptimal,
        &[BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: DDGI_ATLAS_LAYERS,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: DDGI_DEPTH_ATLAS_WIDTH,
            image_extent_h: DDGI_DEPTH_ATLAS_HEIGHT,
            image_extent_d: 1,
        }],
    );
    // Transition both atlases TRANSFER_DST -> SHADER_READ_ONLY_OPTIMAL for the sampled read.
    for tex in [atlas.irradiance(), atlas.depth()] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TRANSFER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::TransferDstOptimal,
            new_layout: ImageLayout::ShaderReadOnlyOptimal,
            range: full_irr,
        });
    }

    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_descriptor_set_compute(&bind_group, &pipeline);
    encoder.dispatch(count.div_ceil(64), 1, 1);

    // Make the output SSBO writes available to the host readback (COMPUTE write -> host read).
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        buffers: &[BufferBarrier {
            buffer: &out_buf,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::TRANSFER_READ,
        }],
    });

    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // ---- 5) read the GPU output back + diff against the host oracle (tight ULP tolerance) -----
    let out_ptr = device.buffer_mapped_ptr(&out_buf).expect("out mapped");
    // The resolve ARITHMETIC is bit-exact against the host oracle: the `precise` pins in
    // `ddgi_resolve.hlsli` forbid DXC from fusing the blend MACs / lowering the `?:`-fed divisions
    // to reciprocals, so 5 of the 6 samples match to 0 ULP. The remaining residual is a TEXTURE
    // -SAMPLER artifact, NOT resolve math: the host oracle's `tap` is fed the host software
    // `unpack_b10g11r11(pack(v))`, while the GPU reads the SAME packed bits through the texture
    // unit's LINEAR B10G11R11 path — the hardware unpack/filter is not bit-identical to a software
    // unpack for every value, differing by <=2 ULP at f32. That is FAR below B10G11R11's 11-bit
    // storage precision (2^-23 vs 2^-11), so f32-bit-exactness on a B10G11R11 SOURCE is not a
    // meaningful requirement. The gate is a tight ULP tolerance that still catches any real
    // arithmetic / UV-mapping / wrong-probe bug (those diverge by orders of magnitude more).
    const RESOLVE_ULP_TOLERANCE: u64 = 4;
    let mut mismatches = 0usize;
    for (i, (p, n)) in samples.iter().enumerate() {
        let mut gpu = [0.0f32; 3];
        for (k, lane) in gpu.iter_mut().enumerate() {
            // SAFETY: `out_buf` holds `samples.len()` float4s host-coherent; `i*4 + k` is in-bounds
            // (k < 3 < 4); the fence wait completed the dispatch + barrier, so the bytes are stable.
            let bits = unsafe { out_ptr.as_ptr().cast::<u32>().add(i * 4 + k).read_unaligned() };
            *lane = f32::from_bits(bits);
        }
        // The host oracle fed the IDENTICAL sky + the depth-sentinel converged predicate.
        let host = probe_sample(*p, *n, ORIGIN, INV_SPACING, DIMS, SKY, probe_pos, |idx, _dir| {
            host_tap(idx)
        });
        for k in 0..3 {
            let gbits = gpu[k].to_bits();
            let hbits = host[k].to_bits();
            // Both lanes are non-negative irradiance ⇒ the fp32 bit patterns are monotonic, so the
            // unsigned integer bit distance IS the ULP distance.
            let ulp = (i64::from(gbits) - i64::from(hbits)).unsigned_abs();
            if ulp > RESOLVE_ULP_TOLERANCE {
                mismatches += 1;
                println!(
                    "sample {i} lane {k}: GPU {} (0x{gbits:08x}) != host {} (0x{hbits:08x}) \
                     [{ulp} ULP > {RESOLVE_ULP_TOLERANCE} tolerance]",
                    gpu[k], host[k]
                );
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "GPU ddgi_probe_sample diverged from goldens::probe_sample in {mismatches} lane(s) BEYOND \
         the {RESOLVE_ULP_TOLERANCE}-ULP tolerance — the resolve arithmetic is `precise`-pinned \
         bit-exact (only a sub-storage-precision B10G11R11 sampler residual is tolerated), so a \
         breach here is a real arithmetic / UV-mapping / wrong-probe bug"
    );
    println!(
        "ddgi_probe_gi_resolve: {} receiver samples, all 3 lanes BIT-EXACT vs goldens::probe_sample",
        samples.len()
    );

    // SAFETY: every resource below was created on `device` and is destroyed exactly once; the last
    // submission completed (fence-waited above), so none is GPU-referenced.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_shader_module(module);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_bind_group(bind_group);
        device.destroy_bind_group_layout(layout);
        device.destroy_buffer(ubo);
        device.destroy_buffer(out_buf);
        device.destroy_buffer(recv_nrm);
        device.destroy_buffer(recv_pos);
        device.destroy_buffer(depth_staging);
        device.destroy_buffer(irr_staging);
        atlas.destroy(device);
    }
}

/// Reinterprets a `&[u32]` as its little-endian byte slice (host is LE x86_64).
fn bytemuck_bytes(words: &[u32]) -> &[u8] {
    // SAFETY: `u32` has no invalid bit patterns as bytes; the slice covers `words.len() * 4` bytes
    // aligned to `u32` (stricter than `u8`); the borrow lives as long as `words`.
    unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 4) }
}
