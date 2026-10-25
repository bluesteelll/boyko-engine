//! IBL1 RK-11 + RK-1 (`ibl1/cut.md` C6) — **a pre-built mip chain uploads level by level and an
//! explicit `SampleLevel(k)` reads level `k`**.
//!
//! The prefiltered environment chain is seven separate GGX convolutions, so it is uploaded with
//! `boyko_render::upload_texture_2d_raw_mipped` (one staging buffer, one `mip_level = k` copy per
//! level, never a blit) and sampled through a `MipMode::Linear` sampler (`maxLod` unclamped). Two
//! silent-wrong classes hide here: a copy that lands every level at mip 0, and a sampler whose
//! `maxLod = 0` (`MipMode::None`) reads level 0 for every `k`. Either makes every roughness read
//! the mirror level, and both are invisible to a host test.
//!
//! - `ibl_level_probe_spv_is_the_redxc_of_its_source` (host): the probe kernel's committed `.spv`
//!   is the re-DXC of `ibl_level_probe.comp.hlsl` under its header recipe. SKIPS (with an
//!   `eprintln`) when no `dxc` resolves — a skipped run is not a pass.
//! - `ibl_upload_mipped_roundtrip` (`gpu:`): a 4² × 3 B10G11R11 chain whose levels hold three
//!   distinct constants is uploaded, sampled at `k = 0..8` through a `MipMode::Linear` sampler and
//!   read back. Level `k` reads its own constant (`k >= 2` clamps to the last level). The mutation
//!   the gate exists for — the sampler built with `MipMode::None` — reads level 0 at every `k`.
//!
//! Run the device leg: `cargo test -p boyko_rhi_vulkan --test ibl_upload_mipped -- --ignored
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`.

use std::path::PathBuf;
use std::process::Command;

use boyko_rhi::enums::{BarrierAccess, BarrierStage, DescriptorKind};
use boyko_rhi::{
    AddressMode, BarrierDesc, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    BufferBarrier, BufferCopy, BufferDesc, BufferUsage, ComputePipelineDesc, Filter, Format,
    MemoryLocation, MipMode, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_shaderdsl::pixfmt::{Rounding, pack_b10g11r11, unpack_b10g11r11};

mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};

static PROBE_SPV: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ibl_level_probe.comp.spv"));

/// The probe's source and its committed module.
const PROBE_SRC: &str = "ibl_level_probe.comp.hlsl";
/// The committed module of [`PROBE_SRC`].
const PROBE_SPV_NAME: &str = "ibl_level_probe.comp.spv";

/// The three levels' constants — exact in UFLOAT11 / UFLOAT10 (each a short binary fraction), so
/// the upload and the decode are lossless and a level is told apart from its neighbours by orders
/// of magnitude.
const LEVEL_RGB: [[f32; 3]; 3] = [[0.25, 0.5, 0.75], [1.0, 2.0, 4.0], [8.0, 0.125, 16.0]];
/// The chain's base extent and level count (4² → 2² → 1²).
const BASE: u32 = 4;
/// Levels in the test chain.
const LEVELS: u32 = 3;
/// Invocations of the probe (`[numthreads(8, 1, 1)]`, one dispatch group).
const READS: usize = 8;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// The pinned SDK's `dxc`, then `$VULKAN_SDK/Bin`, then `PATH` (the `px0_probe_spv_sync` idiom).
fn find_dxc() -> Option<PathBuf> {
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let c = PathBuf::from(sdk).join("Bin").join(bare);
        if c.exists() {
            return Some(c);
        }
    }
    Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok().then(|| PathBuf::from(bare))
}

#[test]
fn ibl_level_probe_spv_is_the_redxc_of_its_source() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP ibl_level_probe_spv_is_the_redxc_of_its_source: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let dir = shaders_dir();
    let out = child_guard::scratch_path(&format!("{PROBE_SPV_NAME}.redxc"));
    let status = Command::new(&dxc)
        .current_dir(&dir)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", PROBE_SRC, "-Fo"])
        .arg(&out)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed on {PROBE_SRC} under the frozen recipe");
    let fresh = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out);
    let committed = std::fs::read(dir.join(PROBE_SPV_NAME)).unwrap_or_else(|e| panic!("{PROBE_SPV_NAME}: {e}"));
    assert!(
        fresh == committed,
        "{PROBE_SPV_NAME} ({} bytes committed, {} re-DXC'd) is stale against {PROBE_SRC}: re-run its header recipe",
        committed.len(),
        fresh.len()
    );
}

/// The level-major B10G11R11 bytes of the test chain (level `k` filled with `LEVEL_RGB[k]`).
fn chain_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (k, rgb) in LEVEL_RGB.iter().enumerate() {
        let side = (BASE >> k).max(1);
        let word = pack_b10g11r11(*rgb, Rounding::NearestEven);
        assert_eq!(unpack_b10g11r11(word), *rgb, "invariant: level {k}'s constant is exact in B10G11R11");
        for _ in 0..side * side {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}

/// One UFLOAT step at `v`'s exponent for a channel with `mantissa_bits` (6 for R/G, 5 for B).
fn uf_step(v: f32, mantissa_bits: i32) -> f32 {
    let e = v.max(f32::MIN_POSITIVE).log2().floor() as i32;
    2f32.powi(e - mantissa_bits)
}

#[test]
#[ignore = "gpu: needs a Vulkan device — IBL1 RK-11/RK-1: a 3-level B10G11R11 chain uploaded level by level reads back level k at SampleLevel(k) through a MipMode::Linear sampler; run --ignored --test-threads=1"]
fn ibl_upload_mipped_roundtrip() {
    let ctx = VulkanContext::boot(InstanceConfig { enable_validation: false, ..InstanceConfig::default() })
        .unwrap_or_else(|e| panic!("ibl_upload_mipped_roundtrip needs a Vulkan device: {e:?}"));
    println!("Vulkan device: {}", ctx.device_name());
    let device: &VulkanContext = &ctx;
    let texture = boyko_render::upload_texture_2d_raw_mipped(
        device,
        BASE,
        BASE,
        LEVELS,
        &chain_bytes(),
        Format::B10G11R11UfloatPack32,
    )
    .expect("the mipped chain uploads");
    let words: Vec<u32> = PROBE_SPV.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let module = device.create_shader_module(&words).expect("probe module");
    let bgl = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("probe layout");
    let pipe = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 0,
            bind_group_layout: Some(&bgl),
            spec_constants: &[],
        })
        .expect("probe pipeline");
    let sampler = device
        .create_sampler(&SamplerDesc {
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::Linear,
            ..SamplerDesc::default()
        })
        .expect("mip sampler");
    let out_bytes = (READS * 16) as u64;
    let out = device
        .create_buffer(&BufferDesc {
            size: out_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::DeviceLocal,
        })
        .expect("probe output buffer");
    // The OBS host-read route (`rk16_dispatch_indirect.rs`): the probe output is copied into a
    // MAPPED readback buffer, whose `copy_buffer` records the TRANSFER -> HOST barrier itself.
    let readback = device
        .create_buffer(&BufferDesc { size: out_bytes, usage: BufferUsage::TRANSFER_DST, location: MemoryLocation::HostVisibleCoherent })
        .expect("readback buffer");
    let group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bgl,
            entries: &[
                BindGroupEntry::CombinedImage { texture: &texture, sampler: &sampler },
                BindGroupEntry::StorageBuffer { buffer: &out },
            ],
        })
        .expect("probe group");
    let fence = device.create_fence(false).expect("fence");
    let mut enc = device.create_command_encoder().expect("encoder");
    enc.begin().expect("begin");
    enc.bind_compute_pipeline(&pipe);
    enc.bind_descriptor_set_compute(&group, &pipe);
    enc.dispatch(1, 1, 1);
    enc.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        buffers: &[BufferBarrier { buffer: &out, src_access: BarrierAccess::SHADER_WRITE, dst_access: BarrierAccess::TRANSFER_READ }],
    });
    enc.copy_buffer(&out, &readback, &[BufferCopy { src_offset: 0, dst_offset: 0, size: out_bytes }]);
    enc.end().expect("end");
    ctx.rhi_queue().submit(&enc, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait");
    let mapped = device.buffer_mapped_ptr(&readback).expect("the readback buffer is host-mapped");
    let mut reads = [[0f32; 4]; READS];
    // SAFETY: `mapped` is the persistently-mapped first byte of the host-coherent `readback` buffer of
    // `READS * 16` bytes; the copy that wrote it was fence-waited above, so no device write is
    // in flight; `reads` is a distinct stack array of exactly `READS * 16` bytes, so the ranges are
    // in bounds and cannot overlap.
    unsafe {
        core::ptr::copy_nonoverlapping(mapped.as_ptr(), reads.as_mut_ptr().cast::<u8>(), READS * 16);
    }
    let mut failures = Vec::new();
    for (k, read) in reads.iter().enumerate() {
        let want = LEVEL_RGB[k.min(LEVEL_RGB.len() - 1)];
        println!("SampleLevel({k}) = {:?} (want level {} = {want:?})", &read[..3], k.min(LEVEL_RGB.len() - 1));
        for (c, mbits) in [6, 6, 5].into_iter().enumerate() {
            if (read[c] - want[c]).abs() > uf_step(want[c], mbits) {
                failures.push(format!("SampleLevel({k}) channel {c}: {} vs level {} = {}", read[c], k.min(2), want[c]));
            }
        }
    }
    // SAFETY: the one submission that used these objects was fence-waited above; each object was
    // created on `device` and is destroyed exactly once, the view-owning texture last.
    unsafe {
        device.destroy_command_encoder(enc);
        device.destroy_fence(fence);
        device.destroy_bind_group(group);
        device.destroy_buffer(readback);
        device.destroy_buffer(out);
        device.destroy_sampler(sampler);
        device.destroy_compute_pipeline(pipe);
        device.destroy_bind_group_layout(bgl);
        device.destroy_shader_module(module);
        device.destroy_texture(texture);
    }
    assert!(failures.is_empty(), "a level read the wrong mip (MipMode::None-class or a mis-planned copy):\n{}", failures.join("\n"));
}
