//! Slice-0 NO-SDK integration test: boot a real Vulkan device and round-trip a
//! host-visible buffer through the sub-allocator.
//!
//! This is the single integration test described in §11 step 2 (sub-allocator
//! in isolation) extended to a real device-memory write/read: create several
//! `VkBuffer`s, query their `VkMemoryRequirements`, sub-allocate aligned
//! offsets into one host-visible + host-coherent block, bind, map (persistent),
//! write a known pattern per buffer, read it back, and assert equality + that
//! the buffers occupy distinct, non-overlapping offsets. Teardown destroys
//! every buffer and the block (which unmaps + frees the memory), then the
//! context (device + instance + loader).
//!
//! # CI gate
//!
//! Device/loader/GPU absence returns `Err` from `VulkanContext::boot`, which
//! this test treats as **skip gracefully** (print + return) so a GPU-less CI
//! does not fail. On a machine with a Vulkan loader + GPU it runs and asserts.

use boyko_rhi_vulkan::device::{BootError, InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
};
use boyko_rhi_vulkan::memory::HostVisibleBlock;

#[test]
fn host_visible_buffer_round_trip() {
    // NO-SDK: validation layers are NOT requested (the SDK ships them
    // separately). The flag exists but stays off.
    let ctx = match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => ctx,
        Err(e) => {
            // Skip gracefully on a GPU-less / loader-less host.
            eprintln!(
                "SKIP host_visible_buffer_round_trip: no Vulkan device available ({e:?})"
            );
            return;
        }
    };

    println!("Vulkan device: {}", ctx.device_name());
    println!("queue family index: {}", ctx.queue_family_index());

    // 16 MiB host-visible + host-coherent block.
    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        16 * 1024 * 1024,
    )
    .expect("invariant: a Vulkan device always exposes a host-visible+coherent memory type");

    // Three buffers of distinct sizes + usages → distinct sub-allocated offsets.
    let sizes_usages = [
        (4096u64, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT),
        (1024u64, VK_BUFFER_USAGE_TRANSFER_SRC_BIT),
        (8192u64, VK_BUFFER_USAGE_TRANSFER_DST_BIT),
    ];

    let mut bound = Vec::with_capacity(sizes_usages.len());
    for &(size, usage) in &sizes_usages {
        let b = block
            .create_bound_buffer(size, usage)
            .expect("buffer create + sub-alloc + bind");
        bound.push(b);
    }

    // Distinct, non-overlapping offsets.
    for i in 0..bound.len() {
        for j in (i + 1)..bound.len() {
            let (a_off, a_size) = (bound[i].offset, bound[i].size);
            let (b_off, b_size) = (bound[j].offset, bound[j].size);
            assert!(
                a_off + a_size <= b_off || b_off + b_size <= a_off,
                "buffers {i} and {j} overlap: [{a_off},{}) vs [{b_off},{})",
                a_off + a_size,
                b_off + b_size
            );
        }
    }

    // Write a per-buffer known pattern, then read it back. Host-coherent
    // memory needs no explicit flush/invalidate.
    for (idx, b) in bound.iter().enumerate() {
        let len = b.size as usize;
        let pattern = pattern_byte(idx);
        // SAFETY: `b.mapped` points to `b.size` contiguous mapped bytes inside
        // the persistently-mapped, host-coherent block (the sub-allocator
        // guarantees `[offset, offset+size)` is in-bounds); writing `len` bytes
        // is in-bounds. No other live alias touches this sub-region (distinct,
        // non-overlapping offsets, asserted above).
        unsafe {
            std::ptr::write_bytes(b.mapped.as_ptr(), pattern, len);
            // Also stamp a marker word at the front so we verify a real
            // value, not just a fill.
            let head = b.mapped.as_ptr() as *mut u32;
            head.write_unaligned(0xDEAD_0000 | idx as u32);
        }
    }

    for (idx, b) in bound.iter().enumerate() {
        let len = b.size as usize;
        let pattern = pattern_byte(idx);
        // SAFETY: same in-bounds, single-aliased mapped region as the write
        // loop; host-coherent memory makes the prior CPU writes visible without
        // a flush.
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(b.mapped.as_ptr(), len) };
        // The first 4 bytes are the marker word; the rest are the fill pattern.
        let marker = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(marker, 0xDEAD_0000 | idx as u32, "marker word mismatch (buffer {idx})");
        for (k, &byte) in bytes.iter().enumerate().skip(4) {
            assert_eq!(
                byte, pattern,
                "byte {k} of buffer {idx} mismatched: got {byte:#x}, want {pattern:#x}"
            );
        }
    }

    // Clean teardown: destroy every buffer (freeing its sub-region), then drop
    // the block (unmap + free memory) and the context (device + instance +
    // loader).
    for b in bound {
        // SAFETY: each `b` was produced by `block.create_bound_buffer` above on
        // this block and is destroyed exactly once here.
        unsafe { block.destroy_bound_buffer(b) };
    }
    drop(block);
    drop(ctx);
}

/// A distinct non-zero fill byte per buffer index.
fn pattern_byte(idx: usize) -> u8 {
    [0xA5u8, 0x3C, 0x77, 0x18, 0xE2][idx % 5]
}

/// A second, smaller test that exercises sub-alloc reuse on a real block: fill,
/// free the middle, re-alloc into the hole, verify the re-used buffer maps to
/// the freed offset. Skips gracefully without a GPU.
#[test]
fn sub_alloc_reuse_on_real_block() {
    let ctx = match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP sub_alloc_reuse_on_real_block: no Vulkan device ({e:?})");
            return;
        }
    };

    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        4 * 1024 * 1024,
    )
    .expect("host-visible block");

    let usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
    let a = block.create_bound_buffer(65536, usage).expect("a");
    let b = block.create_bound_buffer(65536, usage).expect("b");
    let c = block.create_bound_buffer(65536, usage).expect("c");
    let b_offset = b.offset;

    // Free the middle buffer, then a same-size alloc should reuse its offset
    // (first-fit into the coalesced hole, assuming uniform driver alignment).
    let a_kept = a;
    let c_kept = c;
    // SAFETY: `b` was created on this block and is destroyed exactly once.
    unsafe { block.destroy_bound_buffer(b) };
    let reused = block.create_bound_buffer(65536, usage).expect("reuse");
    assert_eq!(reused.offset, b_offset, "freed offset should be reused first-fit");

    // SAFETY: each remaining buffer was created on this block, destroyed once.
    unsafe {
        block.destroy_bound_buffer(a_kept);
        block.destroy_bound_buffer(c_kept);
        block.destroy_bound_buffer(reused);
    }
    drop(block);
    drop(ctx);
}

/// Slice-0 step 0a — the validation-layer oracle. Boots WITH
/// `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger enabled, runs
/// real device ops (allocate a host-visible block + a buffer, then tear them
/// down) under the layer, and asserts the messenger recorded ZERO warning/error
/// validation messages. A validation fault FAILS this test — this counter is the
/// soundness oracle that substitutes for Miri on the raw-FFI path (plan §6).
/// Skips gracefully when the SDK's validation layer is absent (boot returns the
/// `ValidationLayerUnavailable` error) or there is no GPU.
#[test]
fn validation_layer_clean_on_device_ops() {
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
    }) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!(
                "SKIP validation_layer_clean_on_device_ops: validation layer / GPU unavailable ({e:?})"
            );
            return;
        }
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(
        ctx.validation_enabled(),
        "validation must be active when InstanceConfig::enable_validation is set"
    );

    // Exercise a real device-memory path under the validation layer.
    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        1024 * 1024,
    )
    .expect("host-visible block");
    let b = block
        .create_bound_buffer(4096, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT)
        .expect("buffer create + bind under validation");
    // SAFETY: `b` was created on this block and is destroyed exactly once.
    unsafe { block.destroy_bound_buffer(b) };
    drop(block);

    // The oracle: a clean run records zero validation messages. A non-zero count
    // means the layer caught a real API misuse (the `[vk-validation]` log lines
    // identify it) — fail loudly.
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during device ops — see the [vk-validation] log",
        state.total()
    );

    // Teardown (Drop) destroys the messenger before the instance; any
    // destroy-time fault is logged by the create-time messenger threaded through
    // `p_next` (it logs but does not count, so it cannot be asserted here).
    drop(ctx);
}

/// Surfaces the variant names so an `unused` lint does not fire if a future
/// refactor stops constructing one (keeps the public error enum honest).
#[allow(dead_code)]
fn _boot_error_is_debug(e: BootError) -> String {
    format!("{e:?}")
}
