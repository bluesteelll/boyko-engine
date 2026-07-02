//! Render **P1a GPU gate** (scaffold) — proves the descriptor *vocabulary* + the
//! COMPUTE bind point against a **multi-resource** bind group, BEFORE the P1b MRT
//! G-buffer rewrite.
//!
//! # What this proves (the P1a milestone)
//!
//! A compute pipeline bound to a vocabulary set
//! `{ StorageBuffer (edit-list) @ binding 0 + StorageImage (R8G8B8A8 output) @ binding 1 }`
//! reproduces the rung-8/9 packed-buffer golden — the marcher color is written to a
//! **storage image** instead of the packed word in the buffer's pixel region; depth
//! still comes from the packed buffer / no mesh — within `+/-2/255` per channel vs
//! [`golden_editlist_pixel`](boyko_rhi_vulkan::goldens::golden_editlist_pixel). This
//! exercises the new RHI seam end to end:
//!
//! * [`RhiDevice::create_bind_group_layout`] with a **heterogeneous** entry slice
//!   (a `StorageBuffer` + a `StorageImage` binding) — the multi-resource vocabulary;
//! * [`ComputePipelineDesc::bind_group_layout`] `= Some(..)` — a compute pipeline
//!   whose `set 0` is the vocabulary set (a dedicated layout, NOT the device-shared
//!   fixed single-STORAGE_BUFFER layout the packed-buffer offscreen path uses);
//! * [`RhiDevice::create_bind_group`] with a `StorageBuffer` + a `StorageImage`
//!   [`BindGroupEntry`], the pool sized per the per-kind histogram, the set written
//!   ONCE at create (NO per-frame `vkUpdateDescriptorSets`);
//! * [`RhiCommandEncoder::bind_descriptor_set_compute`] — the COMPUTE bind point.
//!
//! # SCAFFOLD STATUS — the GPU run + the storage-image readback golden is the
//! tester's (see the `#[ignore]`d `p1a_vocabulary_storage_image_matches_golden`).
//!
//! This file compiles + the host-side RHI-surface smoke (`p1a_vocabulary_*_host`)
//! runs today, but the **golden GPU assertion is gated behind `#[ignore]`** because
//! it needs a NEW compute shader that does not exist yet:
//!
//! * a `sdf_editlist_storage_image` HLSL/SPIR-V variant that reads the edit-list
//!   from `StorageBuffer` @ `binding 0` (the existing `sdf_editlist` packed-header
//!   format) and `RWTexture2D<float4>` STOREs the marcher color to `StorageImage` @
//!   `binding 1` (instead of packing it into the buffer's pixel region);
//! * a `sdf_editlist_storage_image_spirv()` accessor in `boyko_rhi_vulkan::compute`.
//!
//! The tester: (1) author + compile that shader, (2) un-`#[ignore]` the golden test,
//! (3) create the R8G8B8A8 `STORAGE | TRANSFER_SRC` texture, transition it
//! `UNDEFINED -> GENERAL` (a storage-image compute store), dispatch through the
//! vocabulary set, barrier `GENERAL -> TRANSFER_SRC_OPTIMAL`, `copy_image_to_buffer`
//! into a host-visible readback buffer, and assert each readback texel within
//! `+/-2/255` of `golden_editlist_pixel`, plus a hit!=miss guard and
//! `assert_validation_clean` (NO per-frame `vkUpdateDescriptorSets`).

use core::ptr::NonNull;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, ComputePipelineDesc, DescriptorKind, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage, TextureDesc, TextureDimension,
};
use boyko_rhi_vulkan::compute::{EDITLIST_BUFFER_WORDS, LOCAL_SIZE_X, SDF_IMG_H, SDF_IMG_W, SdfEdit, editlist_pixel_hits, encode_edit_list, sdf_editlist_spirv, sdf_editlist_storage_image_spirv, sdf_op};
use boyko_rhi_vulkan::goldens::{golden_editlist_pixel};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 8/9).
const CHANNEL_TOL: i32 = 2;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when no
/// GPU / loader / validation layer is available.
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
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the P1a vocabulary run — see the [vk-validation] log",
        state.total()
    );
}

/// The P1a vocabulary set layout: a `StorageBuffer` (the edit-list) @ binding 0 + a
/// `StorageImage` (the marcher output) @ binding 1, both visible to COMPUTE.
fn p1a_layout_entries() -> [BindGroupLayoutEntry; 2] {
    [
        BindGroupLayoutEntry {
            binding: 0,
            count: 1,
            kind: DescriptorKind::StorageBuffer,
            stage: ShaderStage::COMPUTE,
        },
        BindGroupLayoutEntry {
            binding: 1,
            count: 1,
            kind: DescriptorKind::StorageImage,
            stage: ShaderStage::COMPUTE,
        },
    ]
}

/// **Host-side RHI-surface smoke** (runs today): builds the multi-resource
/// vocabulary set + a compute pipeline bound to it via
/// [`ComputePipelineDesc::bind_group_layout`], then tears everything down. Proves the
/// new descriptor-vocabulary create path (heterogeneous layout, StorageBuffer +
/// StorageImage bind group written ONCE, COMPUTE-bind-point-capable pipeline layout)
/// is sound on a real device WITHOUT yet needing the storage-image marcher shader.
/// The validation messenger must stay clean (a wrong descriptor type / pool size /
/// layout mismatch would fault here, at create).
#[test]
fn p1a_vocabulary_set_creates_clean_host() {
    let Some(ctx) = boot_or_skip("p1a_vocabulary_set_creates_clean_host") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    let device: &VulkanContext = &ctx;

    // The edit-list StorageBuffer (the existing packed-header layout; reused as the
    // vocabulary set's binding-0 input).
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");

    // The R8G8B8A8 output StorageImage (`STORAGE` for the compute store + `TRANSFER_SRC`
    // so the tester's golden readback can copy it out).
    let output = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
        })
        .expect("R8G8B8A8 storage image");

    // The heterogeneous vocabulary layout (StorageBuffer + StorageImage @ COMPUTE).
    let layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &p1a_layout_entries(),
        })
        .expect("P1a vocabulary bind-group layout");

    // A compute pipeline whose `set 0` IS the vocabulary set (a dedicated layout, not
    // the device-shared fixed STORAGE_BUFFER one). The `sdf_editlist` SPIR-V is bound
    // here only to drive pipeline creation; the storage-image marcher shader is the
    // tester's (the dispatch + golden are `#[ignore]`d below). Pipeline creation
    // validates the layout against the bound stage interface.
    let module = device
        .create_shader_module(sdf_editlist_spirv())
        .expect("sdf_editlist shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
        })
        .expect("P1a vocabulary compute pipeline (set 0 = vocabulary set)");

    // The bind group: a StorageBuffer + a StorageImage entry, written ONCE at create.
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::StorageImage { texture: &output },
            ],
        })
        .expect("P1a vocabulary bind group (storage buffer + storage image)");

    assert_validation_clean(&ctx);

    // SAFETY: every resource below was created on `device` and is destroyed exactly
    // once; no GPU submission was issued (host-only smoke), so none is in use.
    unsafe {
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_bind_group_layout(layout);
        device.destroy_texture(output);
        device.destroy_buffer(buffer);
    }
}

/// Total pixel count (the dispatch element count; the shader bounds `idx < IMG_W*IMG_H`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;
/// R8G8B8A8 readback byte size.
const READBACK_BYTES: u64 = (PIXELS as u64) * 4;

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D dispatch group count (rung-9 convention).
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Writes `words` `u32`s into a host-coherent mapping (seeds the edit-list header).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is `EDITLIST_BUFFER_WORDS * 4` bytes in the persistent
        // host-coherent mapping; `dst + i` for `i < words.len() <= EDITLIST_BUFFER_WORDS`
        // is in-bounds. No GPU work is in flight yet (submit follows), so the host write
        // is unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Splits an R8G8B8A8 texel's first three bytes into `[r, g, b]`.
fn unpack_rgb_bytes(rgba: &[u8]) -> [i32; 3] {
    [rgba[0] as i32, rgba[1] as i32, rgba[2] as i32]
}

/// The packed `0xAABBGGRR` golden split into `[r, g, b]` (R in the low byte).
fn golden_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// The rung-9 "crater" CSG scene (base sphere minus a smaller sphere), reused so the
/// storage-image marcher renders a non-trivial field the host golden also predicts.
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// **P1a GPU gate (TESTER):** dispatch the storage-image marcher through the
/// vocabulary set { StorageBuffer @ b0 + StorageImage @ b1 } via the COMPUTE bind
/// point and assert the readback matches the rung-9 packed-buffer golden within
/// `+/-2/255` — proving a storage-image WRITE through the vocabulary set works on the
/// GPU (the one genuinely-new P1a capability).
///
/// The flow: seed the edit-list StorageBuffer, transition the output StorageImage
/// `UNDEFINED -> GENERAL`, `bind_descriptor_set_compute(&group, &pipeline)` +
/// `dispatch`, barrier `GENERAL -> TRANSFER_SRC_OPTIMAL`, `copy_image_to_buffer` into
/// a host-visible readback buffer, then assert each scanned texel within `CHANNEL_TOL`
/// of `golden_editlist_pixel`, a hit!=miss guard, and `assert_validation_clean` (the
/// set is written ONCE at create — NO per-frame `vkUpdateDescriptorSets`).
#[test]
fn p1a_vocabulary_storage_image_matches_golden() {
    let Some(ctx) = boot_or_skip("p1a_vocabulary_storage_image_matches_golden") else {
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

    let edits = crater();

    // Pick discriminating texels host-side BEFORE the GPU run (independent of the GPU):
    // a guaranteed HIT (the lit CSG surface, preferring the center) and a guaranteed
    // MISS (the (0,0) corner background). The storage-image store must reproduce both.
    let (hit_px, hit_py) = {
        let (cx, cy) = (SDF_IMG_W / 2, SDF_IMG_H / 2);
        if editlist_pixel_hits(&edits, cx, cy) {
            (cx, cy)
        } else {
            let mut found = None;
            'scan: for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    if editlist_pixel_hits(&edits, px, py) {
                        found = Some((px, py));
                        break 'scan;
                    }
                }
            }
            found.expect("invariant: the CSG body must hit at least one texel")
        }
    };
    let (miss_px, miss_py) = (0u32, 0u32);
    assert!(
        !editlist_pixel_hits(&edits, miss_px, miss_py),
        "invariant: the (0,0) corner must MISS the CSG field"
    );

    // --- The edit-list StorageBuffer (binding 0), seeded with the packed header. ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The R8G8B8A8 output StorageImage (binding 1): STORAGE for the compute store,
    // TRANSFER_SRC so the golden readback can copy it out. ---
    let output = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
        })
        .expect("R8G8B8A8 storage image");

    // --- The host-visible readback buffer. ---
    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback buffer");

    // --- The vocabulary layout + pipeline + bind group (the genuinely-new P1a seam). ---
    let layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &p1a_layout_entries(),
        })
        .expect("P1a vocabulary bind-group layout");
    let module = device
        .create_shader_module(sdf_editlist_storage_image_spirv())
        .expect("sdf_editlist_storage_image shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // The vocabulary pipeline's dedicated layout declares the shared compute push
            // range; the shader takes NO push (bounded by the static extent), so nothing is
            // pushed (review O1: no push on the vocabulary path). `4` keeps the create-time
            // "non-empty multiple of 4" contract.
            push_constant_bytes: 4,
            bind_group_layout: Some(&layout),
        })
        .expect("P1a vocabulary compute pipeline");
    // The bind group: StorageBuffer + StorageImage, written ONCE at create.
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::StorageImage { texture: &output },
            ],
        })
        .expect("P1a vocabulary bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // UNDEFINED -> GENERAL: the storage image must be in GENERAL for a compute store
    // (TOP_OF_PIPE -> COMPUTE_SHADER, NONE -> SHADER_WRITE).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &output,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::SHADER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::General,
        range: ImageSubresourceRange::COLOR,
    });

    encoder.bind_compute_pipeline(&pipeline);
    // The COMPUTE bind point: bind the vocabulary set 0 against the pipeline's OWN
    // dedicated layout (NOT the fixed single-STORAGE_BUFFER set). No `bind_storage_buffer`
    // here, so the dispatch's fixed-set rebind is skipped (bound_buffer == NULL).
    encoder.bind_descriptor_set_compute(&bind_group, &pipeline);
    encoder.dispatch(group_count_x(), 1, 1);

    // GENERAL -> TRANSFER_SRC_OPTIMAL so the readback copy can read the stored image
    // (COMPUTE_SHADER write -> TRANSFER read).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &output,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&output, ImageLayout::TransferSrcOptimal, &readback, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the R8G8B8A8 bytes.
    let dst_ptr = device
        .buffer_mapped_ptr(&readback)
        .expect("host-visible readback buffer is mapped");
    let mut out = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `dst_ptr` points to `READBACK_BYTES` mapped host-coherent bytes; a fence
    // wait preceded this read, so the GPU store + copy are complete + coherent; reading
    // `READBACK_BYTES` bytes is in-bounds; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), READBACK_BYTES as usize);
    }

    let texel = |px: u32, py: u32| -> &[u8] {
        let base = ((py * SDF_IMG_W + px) as usize) * 4;
        &out[base..base + 4]
    };
    let close = |got: [i32; 3], want: [i32; 3]| (0..3).all(|c| (got[c] - want[c]).abs() <= CHANNEL_TOL);

    // Scan EVERY texel against the host golden: the storage-image store must reproduce
    // the rung-9 field across the whole image, within +/-2/255.
    let mut max_delta = 0i32;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let got = unpack_rgb_bytes(texel(px, py));
            let want = golden_rgb(golden_editlist_pixel(&edits, px, py));
            for c in 0..3 {
                let d = (got[c] - want[c]).abs();
                if d > max_delta {
                    max_delta = d;
                }
            }
            assert!(
                close(got, want),
                "texel ({px},{py}) channel mismatch: got {got:?}, want {want:?} (tol {CHANNEL_TOL}, max so far {max_delta})"
            );
        }
    }
    println!("P1a storage-image golden: max per-channel delta = {max_delta}/255 (tol {CHANNEL_TOL})");

    // hit != miss guard: the lit surface texel and the background corner MUST differ
    // beyond the tolerance — proving the marcher actually ran a field, not a constant.
    let hit_got = unpack_rgb_bytes(texel(hit_px, hit_py));
    let miss_got = unpack_rgb_bytes(texel(miss_px, miss_py));
    assert!(
        !close(hit_got, miss_got),
        "hit texel ({hit_px},{hit_py}) {hit_got:?} must differ from miss texel ({miss_px},{miss_py}) {miss_got:?} beyond +/-{CHANNEL_TOL} — proving the storage-image store rendered the field"
    );

    // The oracle: a clean run records zero validation messages (the set was written
    // ONCE at create — no per-frame vkUpdateDescriptorSets in the recorded stream).
    assert_validation_clean(&ctx);

    // SAFETY: every resource was created on `device`; the last submission completed
    // (fence-waited above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_bind_group_layout(layout);
        device.destroy_buffer(readback);
        device.destroy_texture(output);
        device.destroy_buffer(buffer);
    }
    drop(ctx);
}
