//! VG R3 step S1 — the explicit `TextureView` RHI capability, exercised by
//! NEW-capability acceptance tests. NO behavior change to any existing resource: these
//! tests create the NEW shape (an image view whose subresource range starts above mip 0,
//! or at an interior array layer, or in a reinterpreted format) and prove it creates +
//! binds CLEANLY through the validation layer (the GPU-half oracle, zero messages),
//! gracefully skipping on a GPU-less / loader-less / validation-less host.
//!
//! These do NOT build a depth pyramid, dispatch a reduction, or bind a view into any
//! render path. They validate ONLY the RHI capability:
//!
//! 1. **Per-mip views** — a mipped `R32_SFLOAT` `STORAGE | SAMPLED` image gets one view
//!    per level, each `base_mip: k, mip_count: 1`. That range is unreachable through
//!    `create_texture`: every view it builds starts at mip 0.
//! 2. **`BindGroupEntry::StorageImageView`** — an explicit view written into a
//!    `STORAGE_IMAGE` descriptor, the same `GENERAL` + NULL-sampler write the implicit
//!    `StorageImage` arm performs.
//! 3. **Interior-layer + format-reinterpreting views** — `base_layer: k` on an array
//!    depth image, a `D2Array` slice of it, and (on a MUTABLE image whose own views are
//!    `R8G8B8A8_SRGB`) an explicit view back in the image's own `R8G8B8A8_UNORM`.
//!
//! # Test discipline
//!
//! This file follows `csm_inc0_capabilities.rs` — the RHI crate's convention for
//! device-needing capability tests: a graceful `boot_or_skip` (so a GPU-less CI host
//! skips instead of failing) plus an `assert_validation_clean` oracle that no-ops under
//! `BOYKO_DISABLE_VALIDATION`. It is deliberately NOT `#[ignore]`d, and the reason is
//! `csm_inc0_capabilities.rs` itself: a device-needing capability test that SKIPS
//! gracefully needs no `#[ignore]`, because the skip already does the job `#[ignore]`
//! would. Marking these `#[ignore]` would remove the only automated coverage of the
//! create path in exchange for nothing.
//!
//! Every teardown here obeys THE OWNERSHIP RULE (`texture.rs` module docs): each view is
//! destroyed BEFORE the texture it views.

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, DescriptorKind,
    Format, ImageUsage, RhiDevice, ShaderStage, TextureDesc, TextureDimension, TextureViewDesc,
    TextureViewDimension,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The test image's base extent — big enough to carry a real mip chain.
const DIM: u32 = 64;
/// The mip-chain length of the pyramid-shaped test image (`64 → 32 → 16 → 8 → 4`).
const MIPS: u32 = 5;
/// The array-layer count of the multi-layer test image (the CSM cascade count).
const LAYERS: u32 = 4;

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP log)
/// when no GPU / loader / validation layer / dynamic-rendering is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages (the GPU-half oracle).
///
/// A no-op (with a one-line note) when validation is disabled via
/// `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW process on this box):
/// there is no messenger to read, but the create paths still run.
fn assert_validation_clean(ctx: &VulkanContext, what: &str) {
    if !ctx.validation_enabled() {
        eprintln!(
            "NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the {what} clean-oracle assert"
        );
        return;
    }
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the {what} capability test — see the [vk-validation] log",
        state.total()
    );
}

/// The mipped single-layer storage image the per-mip views are taken of — the shape a
/// depth pyramid has.
fn pyramid_desc() -> TextureDesc {
    TextureDesc {
        width: DIM,
        height: DIM,
        depth: 1,
        format: Format::R32Sfloat,
        dimension: TextureDimension::D2,
        usage: ImageUsage::STORAGE | ImageUsage::SAMPLED,
        array_layers: 1,
        mip_levels: MIPS,
        view_format: None,
    }
}

// ===========================================================================
// Capability 1 — one view per mip level.
// ===========================================================================

/// Every level of a mip chain gets its own `base_mip: k, mip_count: 1` view, and all of
/// them create cleanly. `create_texture` cannot produce this: its single-layer view spans
/// `[0, mip_levels)` and its per-layer views span `[0, 1)` — both start at 0.
#[test]
fn per_mip_views_create_for_every_level() {
    let Some(ctx) = boot_or_skip("per_mip_views_create_for_every_level") else {
        return;
    };

    let texture = ctx
        .create_texture(&pyramid_desc())
        .expect("mipped R32_SFLOAT storage texture creates");

    // One owned view per level, in a fixed-size array — no heap, and the arity is the
    // mip count by construction rather than by a length assert.
    let views: [_; MIPS as usize] = core::array::from_fn(|k| {
        ctx.create_texture_view(
            &texture,
            &TextureViewDesc {
                base_mip: k as u32,
                mip_count: 1,
                ..TextureViewDesc::default()
            },
        )
        .expect("per-mip view creates")
    });

    assert_validation_clean(&ctx, "per-mip views");

    // SAFETY: every view and the texture were created on `ctx`, no GPU work referenced
    // them (nothing was submitted), and each is destroyed exactly once. THE OWNERSHIP
    // RULE: the views go first, the image they view second.
    unsafe {
        for view in views {
            ctx.destroy_texture_view(view);
        }
        ctx.destroy_texture(texture);
    }
}

// ===========================================================================
// Capability 2 — an explicit view written into a STORAGE_IMAGE descriptor.
// ===========================================================================

/// A `BindGroupEntry::StorageImageView` writes an EXPLICIT view into a `STORAGE_IMAGE`
/// binding, validation-clean — the same descriptor write the implicit `StorageImage` arm
/// performs, differing only in which `VkImageView` handle it names.
///
/// The view bound is the LAST mip, a level the texture owns no view of. What this asserts
/// is that the create + write path accepts such a view; it does NOT distinguish the
/// handle written (there is no dispatch and no readback here), so it is a create-path
/// gate, not a routing gate.
#[test]
fn storage_image_view_binds_an_interior_mip() {
    let Some(ctx) = boot_or_skip("storage_image_view_binds_an_interior_mip") else {
        return;
    };

    let texture = ctx
        .create_texture(&pyramid_desc())
        .expect("mipped R32_SFLOAT storage texture creates");
    let view = ctx
        .create_texture_view(
            &texture,
            &TextureViewDesc {
                base_mip: MIPS - 1,
                mip_count: 1,
                ..TextureViewDesc::default()
            },
        )
        .expect("last-mip view creates");

    let layout = ctx
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::StorageImage,
                stage: ShaderStage::COMPUTE,
            }],
        })
        .expect("single STORAGE_IMAGE layout creates");
    let group = ctx
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[BindGroupEntry::StorageImageView { view: &view }],
        })
        .expect("bind group with an explicit storage-image view creates");

    assert_validation_clean(&ctx, "explicit storage-image view binding");

    // SAFETY: each resource was created on `ctx`, none was ever submitted (so no GPU work
    // references them), and each is destroyed exactly once, in reverse creation order.
    // THE OWNERSHIP RULE: the view is destroyed before the texture it views.
    unsafe {
        ctx.destroy_bind_group(group);
        ctx.destroy_bind_group_layout(layout);
        ctx.destroy_texture_view(view);
        ctx.destroy_texture(texture);
    }
}

// ===========================================================================
// Capability 3 — the layer axis and the format axis.
// ===========================================================================

/// An INTERIOR array layer (`base_layer: 2, layer_count: 1`) and a multi-layer
/// `D2Array` slice both create cleanly over a 4-layer depth image — the layer half of
/// the desc, on the one texture shape in the engine that has layers.
#[test]
fn interior_layer_and_array_slice_views_create() {
    let Some(ctx) = boot_or_skip("interior_layer_and_array_slice_views_create") else {
        return;
    };

    let texture = ctx
        .create_texture(&TextureDesc {
            width: DIM,
            height: DIM,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: LAYERS,
            mip_levels: 1,
            view_format: None,
        })
        .expect("4-layer D32 array depth texture creates");

    // One interior layer, as a plain 2D view.
    let layer_view = ctx
        .create_texture_view(
            &texture,
            &TextureViewDesc {
                base_layer: 2,
                layer_count: 1,
                ..TextureViewDesc::default()
            },
        )
        .expect("interior single-layer view creates");
    // A contiguous slice of layers, as a 2D-ARRAY view.
    let slice_view = ctx
        .create_texture_view(
            &texture,
            &TextureViewDesc {
                base_layer: 1,
                layer_count: 2,
                dimension: TextureViewDimension::D2Array,
                ..TextureViewDesc::default()
            },
        )
        .expect("interior 2D-array slice view creates");

    assert_validation_clean(&ctx, "interior-layer and array-slice views");

    // SAFETY: both views and the texture were created on `ctx`, nothing was submitted,
    // and each is destroyed exactly once — views first (THE OWNERSHIP RULE).
    unsafe {
        ctx.destroy_texture_view(slice_view);
        ctx.destroy_texture_view(layer_view);
        ctx.destroy_texture(texture);
    }
}

/// A format-REINTERPRETING view creates cleanly over a MUTABLE image. The texture
/// declares `format: R8G8B8A8_UNORM` + `view_format: R8G8B8A8_SRGB` (so the backend sets
/// `VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT` and its own views are sRGB); the explicit view
/// then asks for the image's own UNORM format back — the reinterpretation direction the
/// texture itself cannot express.
#[test]
fn format_reinterpreting_view_creates_on_a_mutable_image() {
    let Some(ctx) = boot_or_skip("format_reinterpreting_view_creates_on_a_mutable_image") else {
        return;
    };

    let texture = ctx
        .create_texture(&TextureDesc {
            width: DIM,
            height: DIM,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: 1,
            // Declaring a different view format is what makes the image MUTABLE.
            view_format: Some(Format::R8G8B8A8Srgb),
        })
        .expect("mutable-format texture creates");

    // Inheriting (`format: None`) picks up the texture's own view format (sRGB).
    let inherited = ctx
        .create_texture_view(&texture, &TextureViewDesc::default())
        .expect("inheriting view creates");
    // Reinterpreting back to the image's own UNORM format.
    let reinterpreted = ctx
        .create_texture_view(
            &texture,
            &TextureViewDesc {
                format: Some(Format::R8G8B8A8Unorm),
                ..TextureViewDesc::default()
            },
        )
        .expect("reinterpreting view creates");

    assert_validation_clean(&ctx, "format-reinterpreting view");

    // SAFETY: both views and the texture were created on `ctx`, nothing was submitted,
    // and each is destroyed exactly once — views first (THE OWNERSHIP RULE).
    unsafe {
        ctx.destroy_texture_view(reinterpreted);
        ctx.destroy_texture_view(inherited);
        ctx.destroy_texture(texture);
    }
}
