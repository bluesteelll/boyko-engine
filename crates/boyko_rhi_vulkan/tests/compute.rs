//! Slice-0 steps 0c + 0d integration tests, now driven through the `boyko_rhi`
//! trait surface (Phase 1, Wave D) instead of the dissolved `ComputeHarness`.
//!
//! The intent is byte-for-byte unchanged: 0c writes a known pattern
//! (`buffer[i] = i*2 + 1`), fence-waits, reads back the persistent mapping,
//! asserts the pattern; 0d records BOTH `write_pattern` and `transform_add`
//! (`buffer[i] += 100`) into ONE command buffer with a buffer memory barrier
//! between them, submits once, and diffs the result bit-exact against the CPU
//! golden `(i*2 + 1) + 100`. Only the driver changed: the flow now goes
//! `Vulkan` → [`RhiDevice::create_*`] → [`RhiCommandEncoder`] → [`RhiQueue::submit`]
//! → [`RhiDevice::wait_fence`] → read back via [`RhiDevice::buffer_mapped_ptr`].
//!
//! # The validation oracle (plan §6)
//!
//! Both tests boot with `InstanceConfig { enable_validation: true }` and assert
//! `ctx.debug_state().total() == 0` after the run. A validation WARNING/ERROR
//! FAILS the test — this counter is the soundness oracle that substitutes for
//! Miri on the raw-FFI path. Synchronization validation is enabled on the
//! instance (plan G2, `VkValidationFeaturesEXT`), so the layer gets a chance to
//! flag a wrong/missing barrier.
//!
//! # The barrier oracle (plan G2)
//!
//! For the chained-barrier test, the **bit-exact golden** is the primary proof of
//! the barrier: a missing barrier makes the second pass read stale data and the
//! diff fails (empirically verified). Sync-validation is enabled as a second line
//! of defense, but on this RTX 3060 / NVIDIA stack it does NOT additionally flag a
//! compute→compute RAW hazard on a host-coherent buffer (a known sync-validation
//! gap for host-visible memory); the `negative_chained_barrier_hazard` companion
//! test (`#[ignore]`d) records the chain WITHOUT the barrier and documents that
//! the golden — not the layer — is what catches it here.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the SDK's validation layer,
//! makes `VulkanContext::boot` return `Err`; both tests skip gracefully.

use core::ptr::NonNull;
use std::sync::Mutex;

use boyko_rhi::{
    BarrierAccess, BarrierDesc, BufferBarrier, BufferDesc, BufferUsage, ComputePipelineDesc,
    MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage,
};
use boyko_rhi::enums::BarrierStage;

use boyko_rhi_vulkan::compute::{LOCAL_SIZE_X, transform_add_spirv, write_pattern_spirv};
use boyko_rhi_vulkan::goldens::{golden_chained, golden_write_pattern};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Element count for both tests. Deliberately NOT a multiple of 64 so the
/// `ceil(N/64)` dispatch + the shaders' `i < count` bounds check are exercised
/// (a non-multiple `N` must never write out of range — a validation fault would
/// flag an OOB descriptor access).
const N: u32 = 4096 + 17;

/// Serializes Vulkan device boot across the test threads in this binary.
///
/// Concurrent `VkInstance` / `VkDevice` creation races on the loader / NVIDIA
/// driver, which made some GPU tests spuriously fail to boot and then silently
/// SKIP — unreliable coverage (chip task_10cc8e0b). Holding this lock only for
/// the boot call serializes creation; the tests still run concurrently
/// afterwards. Poison-tolerant so a single panicking boot does not cascade-skip
/// the remaining tests.
static BOOT_LOCK: Mutex<()> = Mutex::new(());

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when
/// no GPU / loader / validation layer is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    // Serialize the boot (see `BOOT_LOCK`); the guard releases at function return.
    let _boot_guard = BOOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

/// Asserts the validation messenger recorded ZERO messages, with the count in
/// the failure message (the `[vk-validation]` log lines identify the fault).
fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the compute run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(N / LOCAL_SIZE_X)` — the dispatch group count the caller now computes
/// (the trait's `dispatch` takes explicit group counts).
fn group_count_x() -> u32 {
    N.div_ceil(LOCAL_SIZE_X)
}

/// Reads `N` `u32`s from a buffer's persistent host-coherent mapping (valid only
/// after a fence-waited submit). Mirrors the dissolved `ComputeHarness::read_back`.
fn read_back(base: NonNull<u8>) -> Vec<u32> {
    let n = N as usize;
    let mut out = Vec::with_capacity(n);
    let base = base.as_ptr().cast::<u32>();
    for i in 0..n {
        // SAFETY: the buffer is `N * 4` bytes inside the persistent host-coherent
        // mapping; `base + i` for `i < n` is in-bounds; a fence wait preceded this
        // read, so the GPU writes are complete + coherent. `read_unaligned`
        // tolerates the sub-allocated offset's alignment.
        let v = unsafe { base.add(i).read_unaligned() };
        out.push(v);
    }
    out
}

/// 0c — one compute dispatch writes the pattern; readback asserts `buffer[i] == i*2 + 1`.
#[test]
fn compute_write_pattern_round_trip() {
    let Some(ctx) = boot_or_skip("compute_write_pattern_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // One storage buffer of N u32s, host-visible+coherent (the device routes it
    // through its shared block).
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (N as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");

    // Compile the 0c shader module + build the compute pipeline.
    let module = device
        .create_shader_module(write_pattern_spirv())
        .expect("write_pattern shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("write_pattern compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    // Record: begin → bind pipeline → bind set → push N → dispatch → end.
    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &N.to_ne_bytes());
    encoder.dispatch(group_count_x(), 1, 1);
    encoder.end().expect("end");

    // Submit + fence-wait; read back the persistent mapping.
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let out = read_back(mapped);

    assert_eq!(out.len(), N as usize);
    for (i, &v) in out.iter().enumerate() {
        let want = golden_write_pattern(i as u32);
        assert_eq!(v, want, "0c mismatch at i={i}: got {v}, want {want}");
    }

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

    // Teardown in reverse resource order (no submission is pending — fence-waited).
    // SAFETY: every resource below was created on `device` and is destroyed
    // exactly once; the last submission completed (fence-waited above), so none is
    // in use by the GPU.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(buffer);
    }
    drop(ctx);
}

/// 0d — two passes chained by a buffer memory barrier; the result diffs bit-exact
/// against the CPU golden `(i*2 + 1) + 100`.
#[test]
fn compute_chained_barrier_golden() {
    let Some(ctx) = boot_or_skip("compute_chained_barrier_golden") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let buffer = device
        .create_buffer(&BufferDesc {
            size: (N as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");

    let write_module = device
        .create_shader_module(write_pattern_spirv())
        .expect("write_pattern shader module");
    let write_pattern = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &write_module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("write_pattern pipeline");

    let add_module = device
        .create_shader_module(transform_add_spirv())
        .expect("transform_add shader module");
    let transform_add = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &add_module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("transform_add pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    // Record BOTH passes chained by a buffer memory barrier: write_pattern →
    // barrier (COMPUTE_SHADER/SHADER_WRITE → COMPUTE_SHADER/SHADER_READ|WRITE) →
    // transform_add, all into ONE command buffer.
    let count = N.to_ne_bytes();
    encoder.begin().expect("begin");
    encoder.bind_storage_buffer(&buffer, 0, 0);
    // First pass.
    encoder.bind_compute_pipeline(&write_pattern);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
    encoder.dispatch(group_count_x(), 1, 1);
    // Barrier: make the first pass's writes visible to the second pass's reads +
    // writes on the same buffer (the §5.5 edge→barrier lowering in miniature).
    let barriers = [BufferBarrier {
        buffer: &buffer,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
    }];
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        buffers: &barriers,
    });
    // Second pass.
    encoder.bind_compute_pipeline(&transform_add);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
    encoder.dispatch(group_count_x(), 1, 1);
    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let out = read_back(mapped);

    assert_eq!(out.len(), N as usize);
    for (i, &v) in out.iter().enumerate() {
        let want = golden_chained(i as u32);
        assert_eq!(
            v, want,
            "0d chained-barrier golden mismatch at i={i}: got {v}, want {want}"
        );
    }

    // The bit-exact golden above is the PRIMARY oracle that the barrier is present
    // and correct: without it the second pass reads stale data and the diff fails
    // (empirically verified — a missing barrier yields `got i*2+1, want
    // (i*2+1)+100`). Synchronization validation is ALSO enabled on the instance
    // (plan G2, `VkValidationFeaturesEXT`), giving the layer a second chance to
    // flag a wrong/missing barrier — see the `negative_chained_barrier_hazard`
    // companion test for the documented hazard expectation. `assert_validation_clean`
    // additionally requires zero validation messages on this correct path.
    assert_validation_clean(&ctx);

    // SAFETY: every resource was created on `device` and is destroyed exactly
    // once; the last submission completed (fence-waited), so none is in use.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(transform_add);
        device.destroy_shader_module(add_module);
        device.destroy_compute_pipeline(write_pattern);
        device.destroy_shader_module(write_module);
        device.destroy_buffer(buffer);
    }
    drop(ctx);
}

/// G2 negative companion — the SAME chained two-pass workload but with **NO**
/// pipeline barrier between the write pass and the read-modify pass. It documents
/// the hazard expectation: the missing barrier is caught by the **bit-exact
/// golden** (the second pass reads stale data → `out[i] == i*2+1`, not
/// `(i*2+1)+100`). Synchronization validation is enabled on the instance but, on
/// the NVIDIA/host-coherent path, does not additionally flag this compute→compute
/// RAW hazard (a known sync-validation gap), so the golden is the load-bearing
/// oracle here.
///
/// `#[ignore]`d because it deliberately leaves a GPU hazard: it asserts the
/// mismatch (a passing run for this test means the hazard manifested as expected)
/// and skips the validation-clean assertion. Run explicitly with
/// `cargo test ... -- --ignored negative_chained_barrier_hazard` to reproduce.
#[test]
#[ignore = "deliberately omits a barrier to document the hazard; run with --ignored"]
fn negative_chained_barrier_hazard() {
    let Some(ctx) = boot_or_skip("negative_chained_barrier_hazard") else {
        return;
    };
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let buffer = device
        .create_buffer(&BufferDesc {
            size: (N as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");
    let write_module = device
        .create_shader_module(write_pattern_spirv())
        .expect("write_pattern shader module");
    let write_pattern = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &write_module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("write_pattern pipeline");
    let add_module = device
        .create_shader_module(transform_add_spirv())
        .expect("transform_add shader module");
    let transform_add = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &add_module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("transform_add pipeline");
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let count = N.to_ne_bytes();

    encoder.begin().expect("begin");
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.bind_compute_pipeline(&write_pattern);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
    encoder.dispatch(group_count_x(), 1, 1);
    // NO `pipeline_barrier` here — the deliberate hazard.
    encoder.bind_compute_pipeline(&transform_add);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
    encoder.dispatch(group_count_x(), 1, 1);
    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let out = read_back(mapped);
    // Expectation: at least one element shows the missing-barrier hazard (the
    // second pass observed the first pass's write incompletely). This documents
    // that the golden is what catches a missing barrier.
    let mismatches = out
        .iter()
        .enumerate()
        .filter(|&(i, &v)| v != golden_chained(i as u32))
        .count();
    assert!(
        mismatches > 0,
        "expected the missing barrier to corrupt the result, but the golden matched"
    );

    // SAFETY: every resource was created on `device`, the submission completed
    // (fence-waited), and each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(transform_add);
        device.destroy_shader_module(add_module);
        device.destroy_compute_pipeline(write_pattern);
        device.destroy_shader_module(write_module);
        device.destroy_buffer(buffer);
    }
    drop(ctx);
}

/// G3 (TD-2/C1) — multi-bind across reused recordings in ONE encoder. Binds
/// buffer A → dispatch (twice, so the cache-skip branch on the second bind of the
/// same buffer is exercised), then re-records binding B (cache re-point), then
/// re-records binding A again (cache re-point back). Each recording is a fresh
/// `begin()` (exercising C1's `bound_buffer` reset), submitted + fence-waited;
/// after each, the bound buffer's readback must equal the `write_pattern` golden
/// AND the OTHER buffer must be untouched — proving each dispatch wrote the
/// correct distinct buffer (cache-skip + cache-re-point + C1 reset). Validation
/// (incl. sync-validation) must stay clean.
#[test]
fn compute_multi_bind_distinct_buffers() {
    let Some(ctx) = boot_or_skip("compute_multi_bind_distinct_buffers") else {
        return;
    };
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let mk_buffer = || {
        device
            .create_buffer(&BufferDesc {
                size: (N as u64) * 4,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("storage buffer")
    };
    let a = mk_buffer();
    let b = mk_buffer();

    let module = device
        .create_shader_module(write_pattern_spirv())
        .expect("write_pattern shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("write_pattern compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let count = N.to_ne_bytes();

    // Records one recording binding `target` (twice, to hit the cache-skip
    // branch on the repeat bind), dispatches, submits + waits, then resets the
    // fence for the next recording.
    let mut record_into = |target: &_| {
        encoder.begin().expect("begin");
        encoder.bind_compute_pipeline(&pipeline);
        encoder.bind_storage_buffer(target, 0, 0);
        // A second identical bind in the same recording: the cache-skip branch
        // (no second `vkUpdateDescriptorSets`).
        encoder.bind_storage_buffer(target, 0, 0);
        encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
        encoder.dispatch(group_count_x(), 1, 1);
        encoder.end().expect("end");
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        device.reset_fence(&fence).expect("reset_fence");
    };

    // Recording 1: bind A → A holds the pattern, B stays zero.
    record_into(&a);
    assert_matches_pattern(device, &a, "A after recording 1");
    assert_all_zero(device, &b, "B untouched after recording 1");

    // Recording 2: re-point cache to B (different buffer → one update). B now
    // holds the pattern; A keeps its recording-1 pattern (no write to A).
    record_into(&b);
    assert_matches_pattern(device, &b, "B after recording 2");
    assert_matches_pattern(device, &a, "A unchanged after recording 2");

    // Recording 3: re-point cache back to A (C1's reset + a re-point). Both hold
    // the pattern now (A re-written, B keeps recording-2 pattern).
    record_into(&a);
    assert_matches_pattern(device, &a, "A after recording 3");
    assert_matches_pattern(device, &b, "B unchanged after recording 3");

    assert_validation_clean(&ctx);

    // SAFETY: every resource was created on `device`, the last submission
    // completed (fence-waited), and each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(a);
        device.destroy_buffer(b);
    }
    drop(ctx);
}

/// Asserts a buffer's readback equals the `write_pattern` golden for all `N`.
fn assert_matches_pattern(device: &VulkanContext, buffer: &boyko_rhi_vulkan::memory::BoundBuffer, label: &str) {
    let mapped = device
        .buffer_mapped_ptr(buffer)
        .expect("host-visible buffer is mapped");
    let out = read_back(mapped);
    for (i, &v) in out.iter().enumerate() {
        let want = golden_write_pattern(i as u32);
        assert_eq!(v, want, "{label}: mismatch at i={i}: got {v}, want {want}");
    }
}

/// Asserts a buffer's readback is all zero (never dispatched into).
fn assert_all_zero(device: &VulkanContext, buffer: &boyko_rhi_vulkan::memory::BoundBuffer, label: &str) {
    let mapped = device
        .buffer_mapped_ptr(buffer)
        .expect("host-visible buffer is mapped");
    let out = read_back(mapped);
    for (i, &v) in out.iter().enumerate() {
        assert_eq!(v, 0, "{label}: expected zero at i={i}, got {v}");
    }
}

/// G5 (T-4) — fence reuse + encoder reuse across submits. Records + submits one
/// dispatch, fence-waits, resets the fence, then re-submits the SAME encoder with
/// the SAME (reset) fence and asserts Ok — proving `reset_fence` + the
/// re-`begin()`/re-record path work. Also asserts a direct `device.wait_idle()`
/// returns Ok on the live device. Validation must stay clean.
#[test]
fn fence_reset_and_resubmit_reuse() {
    let Some(ctx) = boot_or_skip("fence_reset_and_resubmit_reuse") else {
        return;
    };
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let buffer = device
        .create_buffer(&BufferDesc {
            size: (N as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");
    let module = device
        .create_shader_module(write_pattern_spirv())
        .expect("write_pattern shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("write_pattern compute pipeline");
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let count = N.to_ne_bytes();

    let mut record_and_submit = || {
        encoder.begin().expect("begin");
        encoder.bind_compute_pipeline(&pipeline);
        encoder.bind_storage_buffer(&buffer, 0, 0);
        encoder.push_constants(ShaderStage::COMPUTE, 0, &count);
        encoder.dispatch(group_count_x(), 1, 1);
        encoder.end().expect("end");
        queue.submit(&encoder, &fence)
    };

    // First submit + wait + reset.
    record_and_submit().expect("first submit");
    device.wait_fence(&fence, u64::MAX).expect("first wait_fence");
    device.reset_fence(&fence).expect("reset_fence");

    // Re-submit the SAME encoder with the SAME (reset) fence — must be Ok.
    record_and_submit().expect("second submit (reused encoder + reset fence)");
    device
        .wait_fence(&fence, u64::MAX)
        .expect("second wait_fence after reuse");

    // A direct device-idle wait on the live device must succeed.
    device.wait_idle().expect("wait_idle on a live device");

    assert_validation_clean(&ctx);

    // SAFETY: every resource was created on `device`, the last submission
    // completed (fence-waited + wait_idle), and each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(buffer);
    }
    drop(ctx);
}
