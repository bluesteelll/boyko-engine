//! **VG-R0 staging rung S1 — the memory pools grow instead of capping at 64 MiB.**
//!
//! # The defect this rung removes
//!
//! Every buffer in this engine is sub-allocated from a shared block per memory
//! location. Until S1 there was exactly **one** block per location, created
//! lazily at a fixed `64 MiB`, first-fit, **with no growth path** — so 64 MiB was
//! a hard ceiling on everything that location backed.
//!
//! For mesh geometry the ceiling is reached by ordinary content. `build_mesh_gpu`
//! creates BOTH the vertex and the index buffer as
//! `MemoryLocation::HostVisibleCoherent`, and at 64 B/vertex with 0.5
//! vertices/triangle plus `u32` indices a mesh costs ~44 B/triangle. So the whole
//! engine could hold about `67.11e6 / 44 ≈ 1.5 M triangles` of mesh at once — and
//! the failure arrived as a `vkCreateBuffer` `.expect` **panic**, outside every
//! gate that was supposed to bound the corpus. The device-local block carried the
//! identical 64 MiB constant, which is why merely moving mesh data to VRAM would
//! have relocated the ceiling rather than removed it.
//!
//! # What this gate asserts
//!
//! [`the_old_single_block_refuses_past_its_capacity`] runs the **pre-S1
//! mechanism** — one `HostVisibleBlock` — against requests that exceed it and
//! shows it fail. [`a_pool_grows_where_a_single_block_refused`] runs the **same
//! requests** through a [`BlockPool`] and shows them succeed by appending a block.
//! That pairing is the demonstration: same inputs, old mechanism red, new
//! mechanism green, on this box rather than in an argument.
//!
//! The remaining tests cover the two properties growth must not break: a single
//! request LARGER than the default block size gets a block sized to fit, and
//! freeing returns space to the block that minted it so a
//! free/alloc cycle reuses capacity instead of growing without bound.

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
use boyko_rhi_vulkan::memory::{BlockPool, HostVisibleBlock, MemoryError};

/// A deliberately small default block, so growth is exercised without allocating
/// hundreds of megabytes. The production constants are 64 MiB for both pools;
/// the property under test is structural, not tied to their magnitude.
const SMALL_BLOCK: u64 = 8 * 1024 * 1024;

/// Four requests of 3 MiB against an 8 MiB block: the first two fit, the third
/// cannot. Sized so the pre-S1 refusal happens on a known request rather than
/// depending on driver alignment padding.
const CHUNK: u64 = 3 * 1024 * 1024;
const CHUNKS: usize = 4;

fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// **The defect, executed.** One block is all the engine had before S1, and it
/// refuses the third 3 MiB request with `SubAllocExhausted` — which the shipped
/// mesh path turned into a panic.
#[test]
fn the_old_single_block_refuses_past_its_capacity() {
    let Some(ctx) = boot_or_skip("the_old_single_block_refuses_past_its_capacity") else {
        return;
    };
    let mut block = HostVisibleBlock::new(
        ctx.device(),
        ctx.device_fns(),
        ctx.memory_properties(),
        SMALL_BLOCK,
        false,
    )
    .expect("invariant: an 8 MiB host-visible block allocates");

    let mut accepted = 0usize;
    let mut refusal = None;
    for _ in 0..CHUNKS {
        match block.create_bound_buffer(CHUNK, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT) {
            Ok(b) => {
                accepted += 1;
                // Dropped, NOT destroyed — and that is the point. `BoundBuffer`
                // has no `Drop`; a sub-allocation is returned only by an explicit
                // `destroy_bound_buffer`, so the space stays OCCUPIED and the
                // next request meets a genuinely fuller block.
                let _ = b;
            }
            Err(e) => {
                refusal = Some(e);
                break;
            }
        }
    }

    assert!(
        matches!(refusal, Some(MemoryError::SubAllocExhausted)),
        "RED: a single {SMALL_BLOCK}-byte block accepted {accepted} x {CHUNK} bytes without \
         refusing — this test asserts the PRE-S1 defect still reproduces, so if it stops \
         refusing, the pairing below no longer demonstrates anything and must be re-derived"
    );
    assert!(
        accepted * (CHUNK as usize) <= SMALL_BLOCK as usize,
        "invariant: the block cannot have handed out more than it has"
    );
    eprintln!("pre-S1 single block: accepted {accepted} of {CHUNKS} x {CHUNK} B, then refused");

    // The buffers were forgotten, not destroyed; the block's `Drop` frees the
    // whole `VkDeviceMemory`. Their `VkBuffer` handles are released when the
    // device is destroyed with the context.
    drop(block);
}

/// **The repair, executed on the same inputs.** The pool accepts every request
/// the single block refused, by appending a block.
#[test]
fn a_pool_grows_where_a_single_block_refused() {
    let Some(ctx) = boot_or_skip("a_pool_grows_where_a_single_block_refused") else {
        return;
    };
    let mut pool: BlockPool<HostVisibleBlock> = BlockPool::new(SMALL_BLOCK);
    assert_eq!(pool.block_count(), 0, "invariant: a fresh pool allocates nothing");

    let mut held = Vec::with_capacity(CHUNKS);
    for i in 0..CHUNKS {
        let bound = pool
            .alloc(
                ctx.device(),
                ctx.device_fns(),
                ctx.memory_properties(),
                false,
                CHUNK,
                VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "RED: request {i} of {CHUNKS} x {CHUNK} B failed ({e:?}) — the pool must GROW \
                     where the single block refused, which is the whole of rung S1"
                )
            });
        held.push(bound);
    }

    assert!(
        pool.block_count() > 1,
        "RED: every request was served from ONE block ({} total capacity) — the requests no \
         longer exceed a block, so this test is not exercising growth",
        pool.total_capacity()
    );
    assert!(
        pool.total_capacity() >= (CHUNKS as u64) * CHUNK,
        "invariant: the pool must hold at least what it handed out"
    );
    // Every buffer must name the block that minted it, or freeing routes wrong.
    assert!(
        held.iter().any(|b| b.block > 0),
        "RED: no buffer names a block past the first, so the index is not being stamped"
    );
    eprintln!(
        "pooled: {CHUNKS} x {CHUNK} B served from {} blocks, {} B total",
        pool.block_count(),
        pool.total_capacity()
    );

    for bound in held {
        // SAFETY: each `bound` came from `pool.alloc` above, is destroyed exactly
        // once here, and no GPU work was ever submitted against it.
        unsafe { pool.free(bound) };
    }
    pool.clear();
}

/// A single request larger than the default block size must still be served — the
/// pool sizes a fresh block to fit rather than refusing. This is the case the
/// corpus actually needs: one multi-million-triangle mesh's vertex buffer alone
/// exceeds the default.
#[test]
fn one_request_larger_than_the_default_block_is_served() {
    let Some(ctx) = boot_or_skip("one_request_larger_than_the_default_block_is_served") else {
        return;
    };
    let mut pool: BlockPool<HostVisibleBlock> = BlockPool::new(SMALL_BLOCK);
    let oversized = SMALL_BLOCK * 3 + 7; // deliberately not a multiple
    let bound = pool
        .alloc(
            ctx.device(),
            ctx.device_fns(),
            ctx.memory_properties(),
            false,
            oversized,
            VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        )
        .expect("RED: a request larger than the default block must mint a block sized to fit");

    assert_eq!(bound.size, oversized, "invariant: the buffer is the requested size");
    assert!(
        pool.total_capacity() >= oversized,
        "RED: the minted block ({} B) is smaller than the request ({oversized} B)",
        pool.total_capacity()
    );
    // SAFETY: produced by the `alloc` directly above, destroyed exactly once.
    unsafe { pool.free(bound) };
    pool.clear();
}

/// Freeing must return space to the block that minted it, so a free/alloc cycle
/// REUSES capacity. Without this, growth would be a leak wearing a feature's
/// clothes.
#[test]
fn freeing_returns_capacity_to_its_own_block() {
    let Some(ctx) = boot_or_skip("freeing_returns_capacity_to_its_own_block") else {
        return;
    };
    let mut pool: BlockPool<HostVisibleBlock> = BlockPool::new(SMALL_BLOCK);
    let alloc = |pool: &mut BlockPool<HostVisibleBlock>| {
        pool.alloc(
            ctx.device(),
            ctx.device_fns(),
            ctx.memory_properties(),
            false,
            CHUNK,
            VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        )
        .expect("invariant: a 3 MiB request is servable")
    };

    let mut held: Vec<_> = (0..CHUNKS).map(|_| alloc(&mut pool)).collect();
    let grown = pool.block_count();
    assert!(grown > 1, "invariant: the fixture must have grown for the reuse check to mean anything");

    for bound in held.drain(..) {
        // SAFETY: each came from `alloc` above and is destroyed exactly once.
        unsafe { pool.free(bound) };
    }

    // The same workload again must fit in the blocks already held.
    let again: Vec<_> = (0..CHUNKS).map(|_| alloc(&mut pool)).collect();
    assert_eq!(
        pool.block_count(),
        grown,
        "RED: re-running the same workload after freeing it grew the pool from {grown} to {} \
         blocks — freed space is not being reused",
        pool.block_count()
    );
    for bound in again {
        // SAFETY: as above.
        unsafe { pool.free(bound) };
    }
    pool.clear();
}

/// The shipped path: `RhiDevice::create_buffer` must serve a request past the
/// production 64 MiB constant. This is the property the corpus depends on, on the
/// real seam rather than on a fixture pool.
#[test]
fn the_shipped_create_buffer_path_serves_past_the_production_ceiling() {
    let Some(ctx) = boot_or_skip("the_shipped_create_buffer_path_serves_past_the_production_ceiling")
    else {
        return;
    };
    // 5 x 20 MiB = 100 MiB, comfortably past the 64 MiB a single block held.
    const BIG: u64 = 20 * 1024 * 1024;
    const COUNT: usize = 5;

    let mut held = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let buffer = ctx
            .create_buffer(&BufferDesc {
                size: BIG,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .unwrap_or_else(|e| {
                panic!(
                    "RED: host-visible allocation {i} of {COUNT} x {BIG} B failed ({e:?}). Before \
                     rung S1 this is exactly where the engine stopped: one 64 MiB block, no \
                     growth, and the mesh path turned the failure into a panic."
                )
            });
        held.push(buffer);
    }

    let (host_blocks, _) = ctx.pool_block_counts();
    let (host_bytes, _) = ctx.pool_total_capacities();
    assert!(
        host_blocks > 1,
        "RED: {COUNT} x {BIG} B was served from {host_blocks} block(s) of {host_bytes} B — that is \
         more than the 64 MiB default, so either the constant changed or growth did not happen"
    );
    eprintln!("shipped path: {COUNT} x {BIG} B -> {host_blocks} host blocks, {host_bytes} B");

    for buffer in held {
        // SAFETY: each came from `create_buffer` on this context, no GPU work was
        // ever submitted against it, and it is destroyed exactly once.
        unsafe { ctx.destroy_buffer(buffer) };
    }
}
