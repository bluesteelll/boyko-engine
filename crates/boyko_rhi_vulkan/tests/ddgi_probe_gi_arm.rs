//! SDFDDGI I2 (the ARM rung) — the live probe-update DISPATCH smoke test (`#[ignore]`, RTX).
//!
//! I2b proved the STANDALONE dispatch cost (the bench); this rung ARMS the pass in the render path.
//! This smoke proves the update dispatch actually WRITES probe tiles into the irradiance atlas: it
//! boots an offscreen device, enables GI, records the update pass (barrier SRO→GENERAL → bind →
//! dispatch), reads the irradiance atlas back through a host-visible staging buffer, and asserts it
//! is NON-ZERO. A zero atlas means the dispatch never wrote (a dead bind-group / a layer-clamped
//! storage view / a subset-map bug); a non-zero atlas confirms the live compute wrote real probe
//! irradiance.
//!
//! # Byte-identity is NOT the concern here
//!
//! The render stays byte-identical even with GI ON at this rung (I3 has not wired the resolve
//! sample — the atlas is written-but-unread). This test is the SEPARATE, direct proof that the write
//! path runs; the golden byte-identity gate (GI OFF) is a distinct harness.
//!
//! # Named `ddgi_probe_gi_arm` (no "update"/"setup"/"install"/"patch")
//!
//! A test/exe name containing "update" (etc.) triggers Windows os-error-740 (UAC elevation) on the
//! target box; this file is `ddgi_probe_gi_arm`.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test ddgi_probe_gi_arm -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`.

use core::ptr::NonNull;

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, ComputePipelineDesc, DescriptorKind, ImageAspect,
    ImageBarrierDesc, ImageLayout, MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue,
    ShaderStage,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};

use boyko_rhi_vulkan::compute::{
    EDITLIST_BUFFER_WORDS, GI_MAX_IT_DEFAULT, encode_edit_list, sdf_op, sdf_probe_update_spirv,
    SdfEdit,
};
use boyko_rhi_vulkan::ddgi::{
    DDGI_ATLAS_LAYERS, DDGI_IRR_ATLAS_HEIGHT, DDGI_IRR_ATLAS_WIDTH, DDGI_PROBE_COUNT, DdgiAtlas,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The Fibonacci ray count (== `GI_MAX_RAYS`) — the ray table + the UBO `rays_per_probe`.
const RAY_TABLE_RAYS: usize = 128;
/// The light-table word budget (16-word header + `GpuLight[]`), seeded with one directional light.
const LIGHT_TABLE_WORDS: usize = 16 + 12 * 64;
/// The round-robin subset divisor for the smoke — 1 = update EVERY probe this single frame (the
/// widest coverage, so the whole atlas is written in one dispatch). Divides `DDGI_PROBE_COUNT`.
const SMOKE_SUBSET_N: u32 = 1;
/// The b6 UBO byte size (mirrors `DdgiUpdateUbo` — 48 B).
const UBO_BYTES: usize = 48;
/// The irradiance atlas texel byte size (`B10G11R11_UFLOAT_PACK32` = 4 bytes/texel).
const IRR_TEXEL_BYTES: u64 = 4;

/// Boots an offscreen context (validation off), or `None` with a SKIP log when no GPU/loader.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP ddgi_probe_gi_arm: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Writes `words` `u32`s into a host-coherent mapping (valid before the submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer holds at least `words.len()` `u32`s in the persistent host-coherent
        // mapping; `dst + i` is in-bounds; no GPU work is in flight when the CPU seeds it.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// A grid-filling CSG edit-list (the same shape the cost bench uses): geometry filling the default
/// probe-grid volume so a large fraction of probe rays HIT and write real irradiance.
fn grid_filling_edits() -> Vec<SdfEdit> {
    const CENTER: [f32; 3] = [-1.0, 5.0, -1.0];
    let mut edits = Vec::with_capacity(16);
    edits.push(SdfEdit::sphere(CENTER, 9.0, sdf_op::UNION, 0.0));
    edits.push(SdfEdit::sphere([CENTER[0] + 5.0, CENTER[1], CENTER[2]], 4.0, sdf_op::SUBTRACT, 0.6));
    edits.push(SdfEdit::box_shape([CENTER[0], CENTER[1] - 5.0, CENTER[2]], [8.0, 1.5, 8.0], sdf_op::UNION, 1.0));
    let mut i = edits.len();
    while i < 16 {
        let a = i as f32 * 0.7;
        let (cx, cz) = (CENTER[0] + a.cos() * 7.0, CENTER[2] + a.sin() * 7.0);
        let cy = CENTER[1] + (a * 1.3).sin() * 4.0;
        if i % 2 == 0 {
            edits.push(SdfEdit::sphere([cx, cy, cz], 2.6, sdf_op::UNION, 1.2));
        } else {
            edits.push(SdfEdit::box_shape([cx, cy, cz], [2.0, 2.0, 2.0], sdf_op::UNION, 1.0));
        }
        i += 1;
    }
    edits
}

/// The light-table words with ONE valid directional light at entry 0 (so `shade_hit` marches a
/// real shadow and accumulates non-zero radiance). Layout: 16-word header (`light_count` @0), entry
/// 0 at word 16 (`dir@[+0..2]` unit, `kind@[+3]` = DIRECTIONAL 0, `color@[+8..10]` premultiplied).
fn directional_light_table() -> Vec<u32> {
    const HEADER_WORDS: usize = 16;
    let mut words = vec![0u32; LIGHT_TABLE_WORDS];
    words[0] = 1; // light_count
    let dir = {
        let v = [0.3_f32, -1.0, 0.2];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    };
    words[HEADER_WORDS] = dir[0].to_bits();
    words[HEADER_WORDS + 1] = dir[1].to_bits();
    words[HEADER_WORDS + 2] = dir[2].to_bits();
    words[HEADER_WORDS + 3] = 0; // LIGHT_KIND_DIRECTIONAL
    words[HEADER_WORDS + 8] = 3.0_f32.to_bits();
    words[HEADER_WORDS + 9] = 3.0_f32.to_bits();
    words[HEADER_WORDS + 10] = 3.0_f32.to_bits();
    words
}

/// The b6 update UBO words (GI enabled): `float4 origin` (xyz = default grid origin, w = spacing),
/// `uint4 grid_dims` (the default `[16,8,16]`), then `frame_index / subset_n / rays / light_count`.
fn update_ubo_words() -> [u32; UBO_BYTES / 4] {
    let mut w = [0u32; UBO_BYTES / 4];
    // origin.xyz = [-16,-2,-16], origin.w = spacing 2.0.
    w[0] = (-16.0_f32).to_bits();
    w[1] = (-2.0_f32).to_bits();
    w[2] = (-16.0_f32).to_bits();
    w[3] = 2.0_f32.to_bits();
    // grid_dims.xyz = [16,8,16].
    w[4] = 16;
    w[5] = 8;
    w[6] = 16;
    w[7] = 0;
    w[8] = 0; // frame_index
    w[9] = SMOKE_SUBSET_N; // subset_n
    w[10] = RAY_TABLE_RAYS as u32; // rays_per_probe
    w[11] = 1; // light_count
    w
}

/// SDFDDGI I2 ARM: dispatch the probe-update pass once (GI enabled), read the irradiance atlas back,
/// and assert it is NON-ZERO (the live compute wrote real probe tiles).
#[test]
#[ignore = "live dispatch smoke (RTX + --nocapture --test-threads=1); the orchestrator runs it"]
fn probe_update_dispatch_writes_nonzero_irradiance() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    let device: &VulkanContext = &ctx;
    println!("ddgi_probe_gi_arm on: {}", device.device_name());

    if !device.device_caps().ddgi_storage_ok() {
        eprintln!("SKIP ddgi_probe_gi_arm: device lacks B10G11R11/RG16F STORAGE");
        return;
    }
    let queue = ctx.rhi_queue();

    // The persistent atlas (STORAGE irradiance/depth + classification), created WITH the storage
    // views (the caps gate above guarantees the STORAGE usage bit was added).
    let atlas = DdgiAtlas::create(device).expect("DDGI atlas create");

    // The Fibonacci ray table (128 float4s).
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
            for (k, c) in [r * phi.cos(), r * phi.sin(), z, 0.0].into_iter().enumerate() {
                words[i * 4 + k] = c.to_bits();
            }
        }
        write_words(mapped, &words);
    }

    // The edit-list SSBO (grid-filling CSG fold).
    let edit_list = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit list");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &grid_filling_edits());
        let mapped = device.buffer_mapped_ptr(&edit_list).expect("edit list mapped");
        write_words(mapped, &header);
    }

    // The light table (one directional light) + the update UBO (GI enabled).
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
    let update_ubo = device
        .create_buffer(&BufferDesc {
            size: UBO_BYTES as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("update UBO");
    {
        let mapped = device.buffer_mapped_ptr(&update_ubo).expect("update UBO mapped");
        write_words(mapped, &update_ubo_words());
    }

    // The 7-binding update set + bind group (the SAME layout the host builds). The atlas irradiance/
    // depth bind as StorageImage — the fixed multi-layer `array_view` path reaches all 8 layers.
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
        .expect("update layout");
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
        .create_shader_module(sdf_probe_update_spirv(GI_MAX_IT_DEFAULT))
        .expect("probe-update module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
            spec_constants: &[],
        })
        .expect("probe-update pipeline");

    // The readback staging buffer for the irradiance atlas (all 8 layers, 128x128 texels each).
    let irr_texels = (DDGI_IRR_ATLAS_WIDTH as u64) * (DDGI_IRR_ATLAS_HEIGHT as u64) * (DDGI_ATLAS_LAYERS as u64);
    let readback_bytes = irr_texels * IRR_TEXEL_BYTES;
    let readback = device
        .create_buffer(&BufferDesc {
            size: readback_bytes,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("readback buffer");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("encoder");
    encoder.begin().expect("begin");

    // The atlas boots in SHADER_READ_ONLY_OPTIMAL; the update stores at GENERAL. Transition both
    // atlas images SRO → GENERAL for the storage write (all layers).
    let full_range = boyko_rhi::ImageSubresourceRange {
        aspect: ImageAspect::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: DDGI_ATLAS_LAYERS,
    };
    for tex in [atlas.irradiance(), atlas.depth()] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::SHADER_READ,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::ShaderReadOnlyOptimal,
            new_layout: ImageLayout::General,
            range: full_range,
        });
    }

    // Bind + dispatch: one block per active probe (subset_n = 1 → every probe this frame).
    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_descriptor_set_compute(&bind_group, &pipeline);
    let groups = DDGI_PROBE_COUNT / SMOKE_SUBSET_N;
    encoder.dispatch(groups, 1, 1);

    // Make the irradiance stores available + transition GENERAL → TRANSFER_SRC for the readback.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: atlas.irradiance(),
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: full_range,
    });

    // Copy the whole irradiance array (all 8 layers) into the readback buffer.
    let region = BufferImageCopy {
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
    };
    encoder.copy_image_to_buffer(
        atlas.irradiance(),
        ImageLayout::TransferSrcOptimal,
        &readback,
        &[region],
    );

    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the irradiance bytes and assert NON-ZERO (the dispatch wrote real probe tiles).
    let mapped = device.buffer_mapped_ptr(&readback).expect("readback mapped");
    let n = readback_bytes as usize;
    let mut nonzero_bytes = 0usize;
    for i in 0..n {
        // SAFETY: the readback buffer is `readback_bytes` host-coherent bytes; `i < n` is in-bounds;
        // the fence wait above completed the copy, so the bytes are coherent.
        let b = unsafe { mapped.as_ptr().add(i).read() };
        if b != 0 {
            nonzero_bytes += 1;
        }
    }
    println!(
        "ddgi_probe_gi_arm: {nonzero_bytes} / {n} irradiance atlas bytes are non-zero after the \
         live update dispatch"
    );
    assert!(
        nonzero_bytes > 0,
        "the probe-update dispatch wrote ZERO irradiance — the live RDG dispatch did not run (dead \
         bind group / layer-clamped storage view / subset-map bug)"
    );

    // SAFETY: every resource below was created on `device` and is destroyed exactly once; the last
    // submission completed (fence-waited above), so none is GPU-referenced.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_bind_group(bind_group);
        device.destroy_bind_group_layout(layout);
        device.destroy_buffer(readback);
        device.destroy_buffer(update_ubo);
        device.destroy_buffer(light_table);
        device.destroy_buffer(edit_list);
        device.destroy_buffer(ray_table);
        atlas.destroy(device);
    }
}
