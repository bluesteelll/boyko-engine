//! Slice-0 steps 0c + 0d integration tests — one compute dispatch (0c) and a
//! SECOND pass chained through a `vkCmdPipelineBarrier` (0d), both run on a real
//! Vulkan device WITH `VK_LAYER_KHRONOS_validation` enabled.
//!
//! Per plan §11: 0c writes a known pattern (`buffer[i] = i*2 + 1`), fence-waits,
//! reads back the persistent mapping, asserts the pattern. 0d records BOTH
//! `write_pattern` and `transform_add` (`buffer[i] += 100`) into ONE command
//! buffer with a buffer memory barrier between them, submits once, and diffs the
//! result bit-exact against the CPU golden `(i*2 + 1) + 100`.
//!
//! # The validation oracle (plan §6)
//!
//! Both tests boot with `InstanceConfig { enable_validation: true }` and assert
//! `ctx.debug_state().total() == 0` after the run. A validation WARNING/ERROR
//! FAILS the test — this counter is the soundness oracle that substitutes for
//! Miri on the raw-FFI path (Miri cannot run driver FFI / VRAM mapping).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or a host without the SDK's validation layer,
//! makes `VulkanContext::boot` return `Err`; both tests treat that as **skip
//! gracefully** (print + return) so a headless CI without the SDK never fails.
//! On the RTX 3060 dev box (Vulkan loader + validation layer present) they run.

use boyko_rhi_vulkan::compute::{ComputeHarness, golden_chained, golden_write_pattern};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
use boyko_rhi_vulkan::memory::HostVisibleBlock;

/// Element count for both tests. Deliberately NOT a multiple of 64 so the
/// `ceil(N/64)` dispatch + the shaders' `i < count` bounds check are exercised
/// (a non-multiple `N` must never write out of range — a validation fault would
/// flag an OOB descriptor access).
const N: u32 = 4096 + 17;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when
/// no GPU / loader / validation layer is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
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

/// 0c — one compute dispatch writes the pattern; readback asserts `buffer[i] == i*2 + 1`.
#[test]
fn compute_write_pattern_round_trip() {
    let Some(ctx) = boot_or_skip("compute_write_pattern_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    // One 1 MiB host-visible + host-coherent block; a single storage buffer of
    // N u32s bound into it.
    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        1024 * 1024,
    )
    .expect("host-visible block");

    let byte_size = (N as u64) * 4;
    let buffer = block
        .create_bound_buffer(byte_size, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT)
        .expect("storage buffer create + bind");

    // Build the compute harness, run the 0c pass, read back.
    let out = {
        let harness = ComputeHarness::new(
            ctx.device(),
            ctx.device_fns(),
            ctx.queue_family_index(),
            &buffer,
            N,
        )
        .expect("compute harness");
        harness
            .run_write_pattern(ctx.queue())
            .expect("write_pattern dispatch + fence-wait")
        // `harness` drops here (reverse-order teardown of every compute object),
        // BEFORE the buffer + block are destroyed below.
    };

    assert_eq!(out.len(), N as usize);
    for (i, &v) in out.iter().enumerate() {
        let want = golden_write_pattern(i as u32);
        assert_eq!(v, want, "0c mismatch at i={i}: got {v}, want {want}");
    }

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

    // Teardown: the buffer (freeing its sub-region), then the block + context.
    // SAFETY: `buffer` was created on `block` above and is destroyed exactly once.
    unsafe { block.destroy_bound_buffer(buffer) };
    drop(block);
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

    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        1024 * 1024,
    )
    .expect("host-visible block");

    let byte_size = (N as u64) * 4;
    let buffer = block
        .create_bound_buffer(byte_size, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT)
        .expect("storage buffer create + bind");

    let out = {
        let harness = ComputeHarness::new(
            ctx.device(),
            ctx.device_fns(),
            ctx.queue_family_index(),
            &buffer,
            N,
        )
        .expect("compute harness");
        harness
            .run_chained(ctx.queue(), buffer.buffer)
            .expect("chained write_pattern -> barrier -> transform_add + fence-wait")
    };

    assert_eq!(out.len(), N as usize);
    for (i, &v) in out.iter().enumerate() {
        let want = golden_chained(i as u32);
        assert_eq!(
            v, want,
            "0d chained-barrier golden mismatch at i={i}: got {v}, want {want}"
        );
    }

    // The barrier correctness is itself part of the oracle: without the
    // write→read barrier the validation layer's sync-validation flags a
    // WRITE-AFTER-WRITE / READ-AFTER-WRITE hazard, so a clean count also proves
    // the barrier is present + correctly scoped.
    assert_validation_clean(&ctx);

    // SAFETY: `buffer` was created on `block` above and is destroyed exactly once.
    unsafe { block.destroy_bound_buffer(buffer) };
    drop(block);
    drop(ctx);
}
