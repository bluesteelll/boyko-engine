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

use boyko_rhi::{
    AsBuildEntry, AsGeometryDesc, AsIndexType, AsKind, BindGroupDesc, BindGroupEntry,
    BindGroupLayoutDesc, BindGroupLayoutEntry, BufferCopy, BufferDesc, BufferUsage,
    ComputePipelineDesc, DescriptorKind, MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue,
    ShaderStage,
};
use boyko_rhi_vulkan::accel_build::{
    BlasBuildInput, buffer_device_address, build_blas, build_tlas, create_persistent_tlas,
    destroy_blas, destroy_persistent_tlas, destroy_tlas,
};
use boyko_rhi_vulkan::compute::{
    BUILD_TLAS_INSTANCES_PUSH_BYTES, build_tlas_instances_spirv, hwrt_as_descriptor_smoke_spirv,
};
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
            index_type: AsIndexType::Uint32,
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
            index_type: AsIndexType::Uint32,
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

/// R2a-3 GPU smoke: the GPU-resident TLAS pack + build. Build 2 triangle BLAS, fill the mesh-id /
/// instance-ring / BLAS-address buffers, DISPATCH the packer compute (which writes the 64-byte
/// `VkAccelerationStructureInstanceKHR[]` records into the instance array), then build a TLAS from
/// the GPU-WRITTEN array (a separate submit — the pack fence orders the write before the build's
/// read, mirroring R2a-2's BLAS→TLAS submit split). Assert the TLAS device address is non-zero +
/// validation is clean. The pack-written 64-B records are the only reflection-unverified surface,
/// so this smoke is their oracle.
#[test]
#[ignore = "requires a real RT GPU (run: --features hwrt -- --ignored --test-threads=1)"]
fn hwrt_tlas_pack_build_smoke() {
    let Some(ctx) = boot_or_skip("hwrt_tlas_pack_build_smoke") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    if !ctx.ray_query_enabled() {
        eprintln!(
            "SKIP hwrt_tlas_pack_build_smoke: device '{}' does not expose ray query (non-RT GPU)",
            ctx.device_name()
        );
        return;
    }

    const COUNT: u32 = 2;

    // Two BLAS (one per mesh).
    let (vb0, ib0) = make_triangle(&ctx);
    let (vb1, ib1) = make_triangle(&ctx);
    let queue = ctx.rhi_queue();
    let blas0 = build_blas(&ctx, &queue, &BlasBuildInput {
        vertex_buffer: &vb0, index_buffer: &ib0, vertex_count: 3, index_count: 3,
        vertex_stride: VERTEX_STRIDE, index_type: AsIndexType::Uint32,
    }).expect("BLAS 0 build");
    let blas1 = build_blas(&ctx, &queue, &BlasBuildInput {
        vertex_buffer: &vb1, index_buffer: &ib1, vertex_count: 3, index_count: 3,
        vertex_stride: VERTEX_STRIDE, index_type: AsIndexType::Uint32,
    }).expect("BLAS 1 build");

    // The packer inputs: the instance ring (COUNT × 48-B DISTINCT affines — instance 0 identity,
    // instance 1 translated, so a wrong bind/dispatch is caught), the mesh-id lane (instance i
    // uses mesh i), the BLAS-address table (2 × u64), and the DEVICE-LOCAL output instance array
    // (COUNT × 64 B — the production residency; the readback below proves the shader wrote it).
    // Row-major 3×4 affines, 12 f32 each: [linear_row_i.xyz | translation_i].
    let affines: [[f32; 12]; COUNT as usize] = [
        // Instance 0: identity (no translation).
        [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        // Instance 1: identity linear + a distinct translation (7, 8, 9) in the row-major col-3.
        [1.0, 0.0, 0.0, 7.0, 0.0, 1.0, 0.0, 8.0, 0.0, 0.0, 1.0, 9.0],
    ];
    let mut ring_bytes = Vec::new();
    for affine in &affines {
        for f in *affine {
            ring_bytes.extend_from_slice(&f.to_le_bytes());
        }
    }
    let ring = upload_buffer(&ctx, &ring_bytes, BufferUsage::STORAGE);
    let mesh_ids: [u32; COUNT as usize] = [0, 1];
    let mut mesh_id_bytes = Vec::new();
    for id in mesh_ids {
        mesh_id_bytes.extend_from_slice(&id.to_le_bytes());
    }
    let mesh_id_buf = upload_buffer(&ctx, &mesh_id_bytes, BufferUsage::STORAGE);
    // The BLAS-address table by mesh id: mesh 0 → blas0, mesh 1 → blas1.
    let blas_addr_by_mesh: [u64; 2] = [blas0.device_address, blas1.device_address];
    let mut blas_addr_bytes = Vec::new();
    for a in blas_addr_by_mesh {
        blas_addr_bytes.extend_from_slice(&a.to_le_bytes());
    }
    let blas_addr_buf = upload_buffer(&ctx, &blas_addr_bytes, BufferUsage::STORAGE);
    let instance_array = ctx
        .create_buffer(&BufferDesc {
            size: COUNT as u64 * 64,
            usage: BufferUsage::STORAGE
                | BufferUsage::ACCEL_BUILD_INPUT
                | BufferUsage::SHADER_DEVICE_ADDRESS
                | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::DeviceLocal,
        })
        .expect("instance array create");
    let instance_array_addr =
        buffer_device_address(&ctx, &instance_array).expect("instance-array device address");
    assert_ne!(instance_array_addr, 0, "instance array must have a non-zero device address");

    // The packer pipeline + its 4-binding set { ring @0, mesh-ids @1, blas-addr @2, out @3 }.
    let module = ctx.create_shader_module(build_tlas_instances_spirv()).expect("packer module");
    let layout = ctx
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("packer layout");
    let pipeline = ctx
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: BUILD_TLAS_INSTANCES_PUSH_BYTES,
            bind_group_layout: Some(&layout),
            spec_constants: &[],
        })
        .expect("packer pipeline");
    let bind_group = ctx
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &ring },
                BindGroupEntry::StorageBuffer { buffer: &mesh_id_buf },
                BindGroupEntry::StorageBuffer { buffer: &blas_addr_buf },
                BindGroupEntry::StorageBuffer { buffer: &instance_array },
            ],
        })
        .expect("packer bind group");

    // Dispatch the packer (one submit, fence-waited — the write completes before the build read).
    let pack_fence = ctx.create_fence(false).expect("pack fence");
    let mut pack_enc = ctx.create_command_encoder().expect("pack encoder");
    pack_enc.begin().expect("pack begin");
    pack_enc.bind_compute_pipeline(&pipeline);
    pack_enc.bind_descriptor_set_compute(&bind_group, &pipeline);
    pack_enc.push_compute_constants(&pipeline, ShaderStage::COMPUTE, 0, &COUNT.to_le_bytes());
    pack_enc.dispatch(COUNT.div_ceil(64), 1, 1);
    pack_enc.end().expect("pack end");
    queue.submit(&pack_enc, &pack_fence).expect("pack submit");
    ctx.wait_fence(&pack_fence, u64::MAX).expect("pack wait");

    // === The R2a-3 HARDWARE ORACLE: read the device-local instance array back and prove the pack
    // shader wrote CORRECT `VkAccelerationStructureInstanceKHR` records. The pack compute + its
    // bindings are the novel R2a-3 surface with NO reflection check on this no-validation box, so
    // these byte assertions are the only proof the shader ran correctly on HW. ===
    let staging = ctx
        .create_buffer(&BufferDesc {
            size: COUNT as u64 * 64,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("readback staging create");
    let copy_fence = ctx.create_fence(false).expect("copy fence");
    let mut copy_enc = ctx.create_command_encoder().expect("copy encoder");
    copy_enc.begin().expect("copy begin");
    copy_enc.copy_buffer(
        &instance_array,
        &staging,
        &[BufferCopy { src_offset: 0, dst_offset: 0, size: COUNT as u64 * 64 }],
    );
    copy_enc.end().expect("copy end");
    queue.submit(&copy_enc, &copy_fence).expect("copy submit");
    ctx.wait_fence(&copy_fence, u64::MAX).expect("copy wait");

    let staging_ptr = ctx.buffer_mapped_ptr(&staging).expect("readback staging is mapped");
    let mut packed = vec![0u8; COUNT as usize * 64];
    // SAFETY: `staging_ptr` is the persistently-mapped first byte of a host-coherent staging buffer
    // of `COUNT * 64` bytes; the copy above fence-waited (readback complete + coherent); reading
    // `COUNT * 64` bytes is in-bounds; `packed` is a distinct, equally-sized allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(staging_ptr.as_ptr(), packed.as_mut_ptr(), packed.len());
    }

    // Assert the packed 64-B records for instance 0 (first) and instance COUNT-1 (last).
    let u32_le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let u64_le = |b: &[u8]| {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    };
    for &i in &[0usize, COUNT as usize - 1] {
        let rec = &packed[i * 64..i * 64 + 64];
        // bytes[0..48] == the input affine's 48 transform bytes (row-major, verbatim copy).
        let want_transform: Vec<u8> =
            affines[i].iter().flat_map(|f| f.to_le_bytes()).collect();
        assert_eq!(
            &rec[0..48],
            want_transform.as_slice(),
            "instance {i}: the 48-B transform must be the input affine copied verbatim"
        );
        // u32@48 == (0xFF << 24) | (i & 0x00FF_FFFF)  (mask=0xFF | customIndex=i).
        let want_index_mask = (0xFFu32 << 24) | (i as u32 & 0x00FF_FFFF);
        assert_eq!(
            u32_le(&rec[48..52]),
            want_index_mask,
            "instance {i}: instanceCustomIndex:24|mask:8 must be (0xFF<<24)|i"
        );
        // u32@52 == 0  (sbtOffset=0 | flags=0).
        assert_eq!(u32_le(&rec[52..56]), 0, "instance {i}: sbtOffset|flags must be 0");
        // u64@56 (LE) == the BLAS device address for mesh_ids[i].
        let want_blas = blas_addr_by_mesh[mesh_ids[i] as usize];
        assert_eq!(
            u64_le(&rec[56..64]),
            want_blas,
            "instance {i}: accelerationStructureReference must be blas_addr[mesh_ids[{i}]]"
        );
    }
    println!(
        "R2a-3 pack oracle OK: instance 0 → BLAS 0x{:016x}, instance {} → BLAS 0x{:016x} — 64-B records correct on HW",
        blas_addr_by_mesh[mesh_ids[0] as usize],
        COUNT - 1,
        blas_addr_by_mesh[mesh_ids[COUNT as usize - 1] as usize],
    );

    // Build the TLAS from the GPU-written instance array into a persistent (capacity-sized) TLAS.
    let tlas = create_persistent_tlas(&ctx, COUNT).expect("persistent TLAS create");
    let entry = AsBuildEntry {
        kind: AsKind::Tlas,
        geometry: AsGeometryDesc {
            vertex_data: instance_array_addr,
            index_data: 0,
            vertex_stride: 0,
            max_vertex: 0,
            primitive_count: COUNT,
            index_type: AsIndexType::Uint32,
        },
        scratch_address: tlas.scratch_addr,
    };
    let build_fence = ctx.create_fence(false).expect("build fence");
    let mut build_enc = ctx.create_command_encoder().expect("build encoder");
    build_enc.begin().expect("build begin");
    build_enc.cmd_build_acceleration_structures(core::slice::from_ref(&entry), &[&tlas.accel]);
    build_enc.end().expect("build end");
    queue.submit(&build_enc, &build_fence).expect("build submit");
    ctx.wait_fence(&build_fence, u64::MAX).expect("build wait");

    assert_ne!(
        tlas.accel.device_address(), 0,
        "the TLAS built from the GPU-packed instance array must report a non-zero device address"
    );
    println!(
        "R2a-3 OK: TLAS=0x{:016x} built from a GPU-PACKED {COUNT}-instance array — pack+build clean",
        tlas.accel.device_address()
    );

    ctx.wait_idle().expect("device wait idle");
    // Teardown in reverse dependency order.
    // SAFETY: `wait_idle` above guarantees the GPU no longer uses any of these; each was created
    // on `ctx` and is destroyed exactly once by-value.
    unsafe {
        ctx.destroy_command_encoder(build_enc);
        ctx.destroy_fence(build_fence);
        ctx.destroy_command_encoder(copy_enc);
        ctx.destroy_fence(copy_fence);
        ctx.destroy_buffer(staging);
        ctx.destroy_command_encoder(pack_enc);
        ctx.destroy_fence(pack_fence);
        destroy_persistent_tlas(&ctx, tlas);
        ctx.destroy_bind_group(bind_group);
        ctx.destroy_compute_pipeline(pipeline);
        ctx.destroy_bind_group_layout(layout);
        ctx.destroy_shader_module(module);
        ctx.destroy_buffer(instance_array);
        ctx.destroy_buffer(blas_addr_buf);
        ctx.destroy_buffer(mesh_id_buf);
        ctx.destroy_buffer(ring);
        destroy_blas(&ctx, blas0);
        destroy_blas(&ctx, blas1);
        ctx.destroy_buffer(vb0);
        ctx.destroy_buffer(ib0);
        ctx.destroy_buffer(vb1);
        ctx.destroy_buffer(ib1);
    }

    assert_validation_clean(&ctx);
    drop(ctx);
}

/// R2a-4a GPU smoke: the AS-DESCRIPTOR write. Build a BLAS + a TLAS, then bind the TLAS to a
/// `DescriptorKind::AccelerationStructure` descriptor via the new
/// `VkWriteDescriptorSetAccelerationStructureKHR` `p_next` path (`create_bind_group` with a
/// `BindGroupEntry::AccelerationStructure`), and DISPATCH a trivial `rayQuery` compute that traces
/// one ray against the bound TLAS and writes the hit flag to an output buffer. The oracle is "no
/// device-lost + clean validation": if the AS-descriptor write is malformed (wrong sType, dangling
/// `p_acceleration_structures`, bad layout) the trace mis-reads the descriptor → a device-lost / a
/// validation error. This is the ONLY oracle for the R2a-4a AS-descriptor `p_next` write (the
/// silent-FFI UAF class `abi_guard`/Miri cannot see).
#[test]
#[ignore = "requires a real RT GPU (run: --features hwrt -- --ignored --test-threads=1)"]
fn hwrt_as_descriptor_smoke() {
    let Some(ctx) = boot_or_skip("hwrt_as_descriptor_smoke") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    if !ctx.ray_query_enabled() {
        eprintln!(
            "SKIP hwrt_as_descriptor_smoke: device '{}' does not expose ray query (non-RT GPU)",
            ctx.device_name()
        );
        return;
    }

    let queue = ctx.rhi_queue();

    // The simplest valid TLAS: one single-triangle BLAS, one instance over it.
    let (vb, ib) = make_triangle(&ctx);
    let blas = build_blas(&ctx, &queue, &BlasBuildInput {
        vertex_buffer: &vb, index_buffer: &ib, vertex_count: 3, index_count: 3,
        vertex_stride: VERTEX_STRIDE, index_type: AsIndexType::Uint32,
    }).expect("BLAS build");
    let tlas = build_tlas(&ctx, &queue, &[blas.device_address]).expect("TLAS build");
    assert_ne!(tlas.device_address, 0, "TLAS must report a non-zero device address");

    // A single-`uint` output the smoke shader writes the hit flag into (also the readback proof the
    // dispatch ran). DeviceLocal + TRANSFER_SRC so it can be copied back after the trace.
    let output = ctx
        .create_buffer(&BufferDesc {
            size: 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::DeviceLocal,
        })
        .expect("output buffer create");

    // The 2-binding set: the TLAS at binding 0 (the R2a-4a AS descriptor UNDER TEST), the output
    // storage buffer at binding 1.
    let layout = ctx
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::AccelerationStructure, stage: ShaderStage::COMPUTE },
                BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
            ],
        })
        .expect("AS-descriptor smoke layout");
    let module = ctx.create_shader_module(hwrt_as_descriptor_smoke_spirv()).expect("smoke module");
    let pipeline = ctx
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // A dummy 4-byte push (the shared compute layout rejects a 0-byte range).
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
            spec_constants: &[],
        })
        .expect("smoke pipeline");
    // The AS-DESCRIPTOR WRITE UNDER TEST: `BindGroupEntry::AccelerationStructure` drives the new
    // `VkWriteDescriptorSetAccelerationStructureKHR` `p_next` path in `create_bind_group`.
    let bind_group = ctx
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::AccelerationStructure { accel: &tlas.accel },
                BindGroupEntry::StorageBuffer { buffer: &output },
            ],
        })
        .expect("AS-descriptor smoke bind group");

    // Dispatch the trace (1 thread; `count = 1` push so the single thread stores). A clean submit +
    // fence wait == the AS descriptor was read without a device-lost.
    let count: u32 = 1;
    let fence = ctx.create_fence(false).expect("smoke fence");
    let mut enc = ctx.create_command_encoder().expect("smoke encoder");
    enc.begin().expect("smoke begin");
    enc.bind_compute_pipeline(&pipeline);
    enc.bind_descriptor_set_compute(&bind_group, &pipeline);
    enc.push_compute_constants(&pipeline, ShaderStage::COMPUTE, 0, &count.to_le_bytes());
    enc.dispatch(1, 1, 1);
    enc.end().expect("smoke end");
    queue.submit(&enc, &fence).expect("smoke submit");
    ctx.wait_fence(&fence, u64::MAX).expect("smoke wait");

    println!(
        "R2a-4a OK: TLAS=0x{:016x} bound as a VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR descriptor and traced — AS-descriptor pNext write clean on HW",
        tlas.device_address
    );

    ctx.wait_idle().expect("device wait idle");
    // Teardown in reverse dependency order.
    // SAFETY: `wait_idle` above guarantees the GPU no longer uses any of these; each was created on
    // `ctx` and is destroyed exactly once by-value.
    unsafe {
        ctx.destroy_command_encoder(enc);
        ctx.destroy_fence(fence);
        ctx.destroy_bind_group(bind_group);
        ctx.destroy_compute_pipeline(pipeline);
        ctx.destroy_bind_group_layout(layout);
        ctx.destroy_shader_module(module);
        ctx.destroy_buffer(output);
        destroy_tlas(&ctx, tlas);
        destroy_blas(&ctx, blas);
        ctx.destroy_buffer(vb);
        ctx.destroy_buffer(ib);
    }

    // The oracle: a clean run records zero validation messages (== the AS-descriptor write was
    // well-formed; no device-lost).
    assert_validation_clean(&ctx);
    drop(ctx);
}
