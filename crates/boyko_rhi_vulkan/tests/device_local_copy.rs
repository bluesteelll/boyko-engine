//! Phase-5 Wave A oracle: a host-visible staging → device-local (VRAM) →
//! host-visible staging buffer-copy round-trip, driven through the `boyko_rhi`
//! trait surface.
//!
//! This proves the genuine Phase-5 RHI additions BEFORE anything depends on them
//! (oracle-first, plan Wave A):
//!
//! 1. [`MemoryLocation::DeviceLocal`] routes [`RhiDevice::create_buffer`] to the
//!    new `DeviceLocalBlock` (a never-mapped VRAM block), and
//!    [`RhiDevice::buffer_mapped_ptr`] returns `None` for it (device.rs:91
//!    contract).
//! 2. [`RhiCommandEncoder::copy_buffer`] records `vkCmdCopyBuffer` for both the
//!    upload (staging → device) and the readback (device → staging).
//!
//! The flow: write a known pattern to the first host-visible staging buffer →
//! `copy_buffer` staging → device (with a TRANSFER→TRANSFER barrier between the
//! two copies so the upload completes before the readback reads it) → `copy_buffer`
//! device → a second host-visible staging buffer → submit once → fence-wait →
//! map-read the second staging buffer → assert bit-exact. A device-local buffer
//! is NEVER CPU-mapped, so the bytes can only have arrived in the second staging
//! buffer through two real GPU copies, proving the device-local + copy path.
//!
//! # The validation oracle (plan §6)
//!
//! Boots with `InstanceConfig { enable_validation: true }` and asserts
//! `ctx.debug_state().total() == 0` after the run. A validation WARNING/ERROR
//! (a wrong usage flag, a missing transfer barrier, an OOB copy) FAILS the test —
//! the soundness oracle that substitutes for Miri on the raw-FFI path.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the SDK's validation layer,
//! makes `VulkanContext::boot` return `Err`; the test skips gracefully.

use boyko_rhi::{
    BarrierAccess, BarrierDesc, BarrierStage, BufferBarrier, BufferCopy, BufferDesc, BufferUsage,
    MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue,
};

use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Number of `u32`s in the round-trip buffers. Small but non-trivial; the exact
/// count is immaterial to a byte-for-byte copy.
const N: usize = 1024;

/// Total byte size of each buffer.
const SIZE: u64 = (N * 4) as u64;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when
/// no GPU / loader / validation layer is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages.
fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the copy round-trip — see the [vk-validation] log",
        state.total()
    );
}

/// A deterministic per-index byte pattern.
fn pattern_word(i: usize) -> u32 {
    // A spread-out, index-dependent value so a stale / partially-copied buffer
    // would mismatch loudly.
    (i as u32).wrapping_mul(0x9E37_79B1) ^ 0xA5A5_0000
}

#[test]
fn staging_to_device_to_staging_round_trip() {
    let Some(ctx) = boot_or_skip("staging_to_device_to_staging_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise -
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // The two host-visible staging buffers (upload source + readback destination)
    // and the device-local (VRAM) buffer in the middle.
    let staging_src = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible staging-src buffer");
    let device_buf = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            // The device-local path also adds TRANSFER_SRC|DST; STORAGE here keeps
            // the buffer usable as a future compute target.
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::DeviceLocal,
        })
        .expect("device-local buffer");
    let staging_dst = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible staging-dst buffer");

    // A device-local buffer is never host-mapped (device.rs:91 contract).
    assert!(
        device.buffer_mapped_ptr(&device_buf).is_none(),
        "a DeviceLocal buffer must NOT be host-mappable"
    );
    // The staging buffers are.
    let src_ptr = device
        .buffer_mapped_ptr(&staging_src)
        .expect("host-visible staging-src is mapped");
    assert!(
        device.buffer_mapped_ptr(&staging_dst).is_some(),
        "host-visible staging-dst is mapped"
    );

    // Write the known pattern into the source staging buffer (host-coherent — no
    // explicit flush needed).
    // SAFETY: `src_ptr` points to `SIZE` contiguous mapped bytes inside the
    // persistent host-coherent block; writing `N` u32s (== SIZE bytes) is
    // in-bounds; no other live alias touches this sub-region.
    unsafe {
        let p = src_ptr.as_ptr().cast::<u32>();
        for i in 0..N {
            p.add(i).write_unaligned(pattern_word(i));
        }
    }

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    let whole = [BufferCopy { src_offset: 0, dst_offset: 0, size: SIZE }];

    // Record: begin → copy(src → device) → TRANSFER→TRANSFER barrier on the device
    // buffer (the upload write must finish before the readback reads it) →
    // copy(device → dst) → end.
    encoder.begin().expect("begin");
    encoder.copy_buffer(&staging_src, &device_buf, &whole);
    // The barrier makes the first copy's write to `device_buf` available + visible
    // to the second copy's read. Omitting it is a transfer→transfer RAW hazard the
    // sync-validation layer flags (proving the barrier is load-bearing).
    let barrier_buffers = [BufferBarrier {
        buffer: &device_buf,
        src_access: BarrierAccess::TRANSFER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
    }];
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::TRANSFER,
        buffers: &barrier_buffers,
    });
    encoder.copy_buffer(&device_buf, &staging_dst, &whole);
    encoder.end().expect("end");

    // Submit once + fence-wait; the device-local bytes can only reach `staging_dst`
    // through the two GPU copies.
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the destination staging buffer and assert bit-exact.
    let dst_ptr = device
        .buffer_mapped_ptr(&staging_dst)
        .expect("host-visible staging-dst is mapped");
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU copies are complete + coherent; reading `N`
    // u32s is in-bounds.
    unsafe {
        let p = dst_ptr.as_ptr().cast::<u32>();
        for i in 0..N {
            let got = p.add(i).read_unaligned();
            let want = pattern_word(i);
            assert_eq!(
                got, want,
                "word {i} mismatched after staging→device→staging: got {got:#x}, want {want:#x}"
            );
        }
    }

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

    // Teardown. The encoder's last submission completed (fence-waited above), so
    // destroying everything is sound; reverse-ish order.
    // SAFETY: each resource was created on `device`, its GPU work has completed
    // (the fence was waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging_dst);
        device.destroy_buffer(device_buf);
        device.destroy_buffer(staging_src);
    }
    drop(ctx);
}
