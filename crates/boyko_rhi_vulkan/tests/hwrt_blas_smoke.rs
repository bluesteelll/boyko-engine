//! HW-RT rung R2a-2 — the BLAS/TLAS GPU smoke test.
//!
//! The first LIVE-GPU ray-tracing test: it proves the R2a-1 FFI sequence + the R2a-2
//! `accel_build` orchestration build a REAL acceleration structure on hardware. It creates
//! two trivial single-triangle meshes, builds a BLAS per mesh, then a TLAS over the two
//! BLASes, and asserts every AS reports a non-zero device address — the empirical proof that
//! `SHADER_DEVICE_ADDRESS` usage + the `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` alloc flag +
//! the scratch-alignment trick + the record/submit/fence sequence all line up. The test
//! passing == no device-lost (+ a clean validation messenger when validation is on).
//!
//! This crate cannot depend on `boyko_render`, so the meshes are built directly through
//! `ctx.create_buffer` (NOT `MeshRegistry`). A position-only 12-byte vertex stride is fine —
//! the BLAS build reads only `R32G32B32_SFLOAT` position at offset 0, and no shader ever
//! reads these vertices (R2a-2 builds the AS but traces nothing).
//!
//! `#[cfg(feature = "hwrt")]`: the whole test compiles ONLY under `--features hwrt`. It is
//! `#[ignore]` — it runs on real RT hardware via
//! `cargo test -p boyko_rhi_vulkan --features hwrt -- --ignored --test-threads=1`
//! (the orchestrator runs it; subagents cannot run fresh GPU exes). On a non-RT GPU / no
//! loader it SKIPs (prints a `SKIP` line + returns), never fails.
#![cfg(feature = "hwrt")]

use core::ptr::NonNull;

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::accel_build::{BlasBuildInput, build_blas, build_tlas, destroy_blas, destroy_tlas};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::memory::BoundBuffer;

/// Position-only vertex stride (three `f32`). The BLAS triangle format is
/// `R32G32B32_SFLOAT` at offset 0, so a 12-byte stride is a valid build input.
const VERTEX_STRIDE: u64 = 12;

/// Boots a headless validation-on context, or returns `None` (skip) on a GPU-less /
/// loader-less / validation-less host (mirrors the rung tests' skip convention).
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig { enable_validation: true, ..InstanceConfig::default() })
    {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: GPU / loader / validation unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages (the soundness oracle); a no-op
/// with a note when validation is disabled via `BOYKO_DISABLE_VALIDATION`.
fn assert_validation_clean(ctx: &VulkanContext) {
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — messenger oracle skipped");
        return;
    }
    let state = ctx.debug_state().expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the R2a-2 AS build — see the [vk-validation] log",
        state.total()
    );
}

/// Creates a host-visible, device-addressable, AS-build-input buffer of `data.len()` bytes
/// and memcpies `data` into its mapping.
fn upload_buffer(ctx: &VulkanContext, data: &[u8], base_usage: BufferUsage) -> BoundBuffer {
    let buffer = ctx
        .create_buffer(&BufferDesc {
            size: data.len() as u64,
            usage: base_usage | BufferUsage::ACCEL_BUILD_INPUT | BufferUsage::SHADER_DEVICE_ADDRESS,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("AS-input buffer create");
    let ptr: NonNull<u8> = ctx.buffer_mapped_ptr(&buffer).expect("host-visible buffer is mapped");
    // SAFETY: `ptr` points to `data.len()` mapped host-coherent bytes; `data` is a distinct
    // equally-sized slice (no overlap with the fresh device allocation); the copy completes
    // before the build submit references the buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), ptr.as_ptr(), data.len());
    }
    buffer
}

/// A single-triangle mesh: 3 position-only vertices + `[0, 1, 2]` indices. Returns the two
/// owned buffers (vertex, index).
fn make_triangle(ctx: &VulkanContext) -> (BoundBuffer, BoundBuffer) {
    let verts: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let indices: [u32; 3] = [0, 1, 2];
    // SAFETY: `[f32; 9]` / `[u32; 3]` are `#[repr]`-plain POD arrays; re-viewing them as
    // byte slices of the same length reads their well-defined bytes (any bit pattern is a
    // valid u8), and the slices live for the `upload_buffer` calls below.
    let vbytes: &[u8] = unsafe {
        core::slice::from_raw_parts(verts.as_ptr().cast::<u8>(), core::mem::size_of_val(&verts))
    };
    let ibytes: &[u8] = unsafe {
        core::slice::from_raw_parts(indices.as_ptr().cast::<u8>(), core::mem::size_of_val(&indices))
    };
    let vb = upload_buffer(ctx, vbytes, BufferUsage::VERTEX);
    let ib = upload_buffer(ctx, ibytes, BufferUsage::INDEX);
    (vb, ib)
}

/// R2a-2 GPU smoke: build 2 BLAS + 1 TLAS on real hardware; assert every device address is
/// non-zero and teardown is device-lost-free.
#[test]
#[ignore = "requires a real RT GPU (run: --features hwrt -- --ignored --test-threads=1)"]
fn hwrt_blas_tlas_smoke() {
    let Some(ctx) = boot_or_skip("hwrt_blas_tlas_smoke") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    if !ctx.ray_query_enabled() {
        eprintln!(
            "SKIP hwrt_blas_tlas_smoke: device '{}' does not expose ray query (non-RT GPU)",
            ctx.device_name()
        );
        return;
    }

    // Two trivial single-triangle meshes.
    let (vb0, ib0) = make_triangle(&ctx);
    let (vb1, ib1) = make_triangle(&ctx);

    let queue = ctx.rhi_queue();

    // Build a BLAS per mesh.
    let blas0 = build_blas(
        &ctx,
        &queue,
        &BlasBuildInput {
            vertex_buffer: &vb0,
            index_buffer: &ib0,
            vertex_count: 3,
            index_count: 3,
            vertex_stride: VERTEX_STRIDE,
        },
    )
    .expect("BLAS 0 build");
    let blas1 = build_blas(
        &ctx,
        &queue,
        &BlasBuildInput {
            vertex_buffer: &vb1,
            index_buffer: &ib1,
            vertex_count: 3,
            index_count: 3,
            vertex_stride: VERTEX_STRIDE,
        },
    )
    .expect("BLAS 1 build");

    assert_ne!(blas0.device_address, 0, "BLAS 0 must report a non-zero device address");
    assert_ne!(blas1.device_address, 0, "BLAS 1 must report a non-zero device address");

    // Build a TLAS over the two BLAS addresses (a separate submit — the BLAS fences already
    // ordered them ahead of this read).
    let tlas = build_tlas(&ctx, &queue, &[blas0.device_address, blas1.device_address])
        .expect("TLAS build");
    assert_ne!(tlas.device_address, 0, "TLAS must report a non-zero device address");

    // Self-evidencing proof for the manual run: the three live AS device addresses on hardware.
    println!(
        "R2a-2 OK: BLAS0=0x{:016x} BLAS1=0x{:016x} TLAS=0x{:016x} (scratch_align={}) — real AS built on GPU",
        blas0.device_address, blas1.device_address, tlas.device_address, ctx.as_scratch_align()
    );

    // Idle before teardown (the AS/backing lifetimes require no pending GPU use).
    ctx.wait_idle().expect("device wait idle");

    // Teardown in reverse dependency order: TLAS, then the two BLAS, then the mesh buffers.
    // SAFETY: `wait_idle` above guarantees the GPU no longer uses any of these; each was
    // created on `ctx` and is destroyed exactly once by-value.
    unsafe {
        destroy_tlas(&ctx, tlas);
        destroy_blas(&ctx, blas0);
        destroy_blas(&ctx, blas1);
        ctx.destroy_buffer(vb0);
        ctx.destroy_buffer(ib0);
        ctx.destroy_buffer(vb1);
        ctx.destroy_buffer(ib1);
    }

    // The oracle: a clean run records zero validation messages (== no device-lost / no
    // mis-formed AS build info).
    assert_validation_clean(&ctx);
    drop(ctx);
}
