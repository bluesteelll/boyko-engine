//! Slice-0 buffer round-trip + GPU-backed handle-registry integration tests,
//! driven through the `boyko_rhi` trait surface (Phase 1, Wave D).
//!
//! `host_visible_buffer_round_trip` boots a real device, creates host-visible
//! storage buffers through [`RhiDevice::create_buffer`], writes a known pattern
//! through [`RhiDevice::buffer_mapped_ptr`], reads it back, asserts equality +
//! distinct non-overlapping offsets, and tears them down via
//! [`RhiDevice::destroy_buffer`]. `registry_register_resolve_take_destroy_all`
//! drives the backend-agnostic [`ResourceRegistry`] over REAL `Vulkan` resources:
//! register a `BoundBuffer` / fence / shader / pipeline, resolve them, take one,
//! destroy the taken one explicitly, then `destroy_all` the rest.
//! `validation_layer_clean_on_device_ops` runs trait-driven device ops under the
//! validation layer and asserts ZERO messages (the soundness oracle, plan §6).
//!
//! # CI gate
//!
//! Device/loader/GPU absence returns `Err` from `VulkanContext::boot`, which
//! these tests treat as **skip gracefully** (print + return).

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, ResourceRegistry, RhiDevice,
    RhiError, TextureDesc,
};

use boyko_rhi_vulkan::compute::write_pattern_spirv;
use boyko_rhi_vulkan::device::{BootError, InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::rhi_impl::Vulkan;

#[test]
fn host_visible_buffer_round_trip() {
    // NO-SDK: validation layers are NOT requested (the SDK ships them separately).
    let ctx = match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP host_visible_buffer_round_trip: no Vulkan device available ({e:?})");
            return;
        }
    };

    println!("Vulkan device: {}", ctx.device_name());
    println!("queue family index: {}", ctx.queue_family_index());

    let device: &VulkanContext = &ctx;

    // Three buffers of distinct sizes + usages → distinct sub-allocated offsets
    // (the device routes them through its one shared host-visible block).
    let sizes_usages = [
        (4096u64, BufferUsage::STORAGE),
        (1024u64, BufferUsage::TRANSFER_SRC),
        (8192u64, BufferUsage::TRANSFER_DST),
    ];

    let mut bound = Vec::with_capacity(sizes_usages.len());
    for &(size, usage) in &sizes_usages {
        let b = device
            .create_buffer(&BufferDesc {
                size,
                usage,
                location: MemoryLocation::HostVisibleCoherent,
            })
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

    // Write a per-buffer known pattern, then read it back. Host-coherent memory
    // needs no explicit flush/invalidate.
    for (idx, b) in bound.iter().enumerate() {
        let len = b.size as usize;
        let pattern = pattern_byte(idx);
        let mapped = device
            .buffer_mapped_ptr(b)
            .expect("host-visible buffer is mapped");
        // SAFETY: `mapped` points to `b.size` contiguous mapped bytes inside the
        // persistent host-coherent block (the sub-allocator guarantees
        // `[offset, offset+size)` is in-bounds); writing `len` bytes is in-bounds;
        // no other live alias touches this sub-region (distinct offsets above).
        unsafe {
            std::ptr::write_bytes(mapped.as_ptr(), pattern, len);
            let head = mapped.as_ptr() as *mut u32;
            head.write_unaligned(0xDEAD_0000 | idx as u32);
        }
    }

    for (idx, b) in bound.iter().enumerate() {
        let len = b.size as usize;
        let pattern = pattern_byte(idx);
        let mapped = device
            .buffer_mapped_ptr(b)
            .expect("host-visible buffer is mapped");
        // SAFETY: same in-bounds, single-aliased mapped region as the write loop;
        // host-coherent memory makes the prior CPU writes visible without a flush.
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(mapped.as_ptr(), len) };
        let marker = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(marker, 0xDEAD_0000 | idx as u32, "marker word mismatch (buffer {idx})");
        for (k, &byte) in bytes.iter().enumerate().skip(4) {
            assert_eq!(
                byte, pattern,
                "byte {k} of buffer {idx} mismatched: got {byte:#x}, want {pattern:#x}"
            );
        }
    }

    // Teardown: destroy every buffer (freeing its sub-region), then the context.
    for b in bound {
        // SAFETY: each `b` was produced by `create_buffer` above on this device
        // and is destroyed exactly once here (no GPU work touched it).
        unsafe { device.destroy_buffer(b) };
    }
    drop(ctx);
}

/// A distinct non-zero fill byte per buffer index.
fn pattern_byte(idx: usize) -> u8 {
    [0xA5u8, 0x3C, 0x77, 0x18, 0xE2][idx % 5]
}

/// T-1 (plan G1) — re-added sub-allocator reuse test, now driven through the
/// device. Create A/B/C (same size + usage so they carve contiguous, equal-sized
/// regions from the shared block), destroy B (the middle hole), then create a
/// buffer of B's size: first-fit must refill B's freed offset. Proves the
/// device's shared host-visible block recycles a freed sub-allocation. Skips
/// gracefully without a GPU.
#[test]
fn suballocator_reuse_through_device() {
    let ctx = match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP suballocator_reuse_through_device: no Vulkan device available ({e:?})");
            return;
        }
    };
    let device: &VulkanContext = &ctx;

    let mk = |size: u64| {
        device
            .create_buffer(&BufferDesc {
                size,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("buffer create + sub-alloc + bind")
    };

    // Equal sizes → equal driver memory requirements → equal carved extents, so
    // a freed middle hole is an exact fit for a same-size request.
    const SIZE: u64 = 4096;
    let a = mk(SIZE);
    let b = mk(SIZE);
    let c = mk(SIZE);

    // A, B, C are distinct, contiguous, non-overlapping regions.
    assert_ne!(a.offset, b.offset, "A and B distinct");
    assert_ne!(b.offset, c.offset, "B and C distinct");
    let b_offset = b.offset;

    // Free B (the middle). Its sub-region returns to the allocator's free list.
    // SAFETY: `b` was created on `device`, no GPU work touched it, destroyed once.
    unsafe { device.destroy_buffer(b) };

    // A same-size request must first-fit into B's freed hole (the only free range
    // before the tail), so the new buffer's offset equals B's freed offset.
    let recycled = mk(SIZE);
    assert_eq!(
        recycled.offset, b_offset,
        "first-fit must recycle B's freed offset (got {}, want {b_offset})",
        recycled.offset
    );

    // Teardown the survivors.
    // SAFETY: each was created on `device`, untouched by the GPU, destroyed once.
    unsafe {
        device.destroy_buffer(a);
        device.destroy_buffer(c);
        device.destroy_buffer(recycled);
    }
    drop(ctx);
}

/// Drives the backend-agnostic [`ResourceRegistry`] over REAL `Vulkan` resources:
/// register a buffer / fence / shader / pipeline, resolve each, take the buffer
/// and destroy it explicitly, then `destroy_all` the rest. Proves the registry's
/// generation-checked resolve + the structural `destroy_all` teardown work over a
/// live backend (plan D6/W4). Skips gracefully without a GPU.
#[test]
fn registry_register_resolve_take_destroy_all() {
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP registry_register_resolve_take_destroy_all: no GPU / validation ({e:?})");
            return;
        }
    };
    let device: &VulkanContext = &ctx;

    let mut reg: ResourceRegistry<Vulkan> = ResourceRegistry::new();

    // Register a real buffer + fence + shader + pipeline.
    let buffer = device
        .create_buffer(&BufferDesc {
            size: 4096,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("buffer");
    let buf_handle = reg.register_buffer(buffer);

    let fence = device.create_fence(false).expect("fence");
    let fence_handle = reg.register_fence(fence);

    let module = device
        .create_shader_module(write_pattern_spirv())
        .expect("shader module");
    // The shader handle is also taken so it does not outlive its pipeline; we keep
    // the module live in the registry across pipeline creation (create only reads
    // it) and destroy_all frees it in reverse order (pipeline before shader).
    let shader_handle = reg.register_shader(module);

    let pipeline = {
        let module_ref = reg
            .resolve_shader(shader_handle)
            .expect("resolve shader for pipeline build");
        device
            .create_compute_pipeline(&ComputePipelineDesc {
                module: module_ref,
                entry: c"main",
                push_constant_bytes: 4,
            })
            .expect("compute pipeline")
    };
    let pipe_handle = reg.register_compute_pipeline(pipeline);

    // Resolve every live handle.
    assert!(reg.resolve_buffer(buf_handle).is_some(), "buffer resolves");
    assert!(reg.resolve_fence(fence_handle).is_some(), "fence resolves");
    assert!(reg.resolve_shader(shader_handle).is_some(), "shader resolves");
    assert!(
        reg.resolve_compute_pipeline(pipe_handle).is_some(),
        "pipeline resolves"
    );

    // Take the buffer out (for an explicit destroy) — the stale handle must then
    // resolve to None (generation bumped on take).
    let taken = reg.take_buffer(buf_handle).expect("take returns the buffer");
    assert!(
        reg.resolve_buffer(buf_handle).is_none(),
        "a taken handle resolves to None"
    );
    // SAFETY: `taken` was created on `device`, no GPU work touched it, and it is
    // destroyed exactly once here.
    unsafe { device.destroy_buffer(taken) };

    // Destroy everything still registered, in reverse resource order, after a
    // device-idle wait (the structural teardown, plan W4). After this every map is
    // empty, so the registry can drop without tripping its leak debug-assert.
    reg.destroy_all(device);
    // Plan G4 (T-3): the reverse-order teardown must be load-bearing in RELEASE
    // too — assert every kind resolves None after `destroy_all` (not just a
    // debug-only tripwire). The buffer handle was already taken; the rest were
    // freed by `destroy_all`.
    assert!(
        reg.resolve_buffer(buf_handle).is_none(),
        "destroy_all leaves the taken buffer handle stale"
    );
    assert!(
        reg.resolve_fence(fence_handle).is_none(),
        "destroy_all empties the fence map"
    );
    assert!(
        reg.resolve_shader(shader_handle).is_none(),
        "destroy_all empties the shader map"
    );
    assert!(
        reg.resolve_compute_pipeline(pipe_handle).is_none(),
        "destroy_all empties the pipeline map"
    );
    assert!(
        reg.is_fully_drained(),
        "destroy_all leaves every map empty"
    );

    drop(reg);
    drop(ctx);
}

/// Slice-0 step 0a — the validation-layer oracle, trait-driven. Boots WITH
/// `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger, runs real
/// device ops (create a buffer through the trait, then destroy it) under the
/// layer, and asserts the messenger recorded ZERO warning/error messages. A
/// validation fault FAILS this test — the soundness oracle that substitutes for
/// Miri on the raw-FFI path (plan §6). Skips gracefully without the SDK / GPU.
#[test]
fn validation_layer_clean_on_device_ops() {
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
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

    let device: &VulkanContext = &ctx;
    let buffer = device
        .create_buffer(&BufferDesc {
            size: 4096,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("buffer create + bind under validation");
    // SAFETY: `buffer` was created on `device` and is destroyed exactly once.
    unsafe { device.destroy_buffer(buffer) };

    // The oracle: a clean run records zero validation messages.
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during device ops — see the [vk-validation] log",
        state.total()
    );

    drop(ctx);
}

/// COVER-1 (plan G6) — a declared-but-unimplemented seam returns `Unsupported`
/// through the backend, and that category projects losslessly to
/// [`RhiError::Unsupported`]. Exercises the `#[cold]` default stub bodies + the
/// `VulkanError` → `RhiError` projection (and, transitively, the plan-C3 verbatim
/// `Rhi(_)` round-trip is exercised by the unit tests in `error.rs`). Skips
/// gracefully without a GPU.
#[test]
fn seam_stub_returns_unsupported_through_backend() {
    let ctx = match VulkanContext::boot(InstanceConfig::default()) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP seam_stub_returns_unsupported_through_backend: no GPU ({e:?})");
            return;
        }
    };
    let device: &VulkanContext = &ctx;

    // `create_texture` is a Phase-6+ seam — the default stub returns Unsupported.
    let tex = device.create_texture(&TextureDesc::default());
    let err = tex.expect_err("create_texture is a seam stub — must return Err");
    assert_eq!(
        RhiError::from(err),
        RhiError::Unsupported("create_texture"),
        "seam error must project to RhiError::Unsupported(create_texture)"
    );

    // `map_buffer` is a Phase-5 seam — same shape.
    let buffer = device
        .create_buffer(&BufferDesc {
            size: 256,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("buffer");
    let mapped = device.map_buffer(&buffer);
    let err = mapped.expect_err("map_buffer is a seam stub — must return Err");
    assert_eq!(
        RhiError::from(err),
        RhiError::Unsupported("map_buffer"),
        "seam error must project to RhiError::Unsupported(map_buffer)"
    );

    // SAFETY: `buffer` was created on `device` and is destroyed exactly once.
    unsafe { device.destroy_buffer(buffer) };
    drop(ctx);
}

/// Surfaces the variant names so an `unused` lint does not fire if a future
/// refactor stops constructing one (keeps the public error enum honest).
#[allow(dead_code)]
fn _boot_error_is_debug(e: BootError) -> String {
    format!("{e:?}")
}
