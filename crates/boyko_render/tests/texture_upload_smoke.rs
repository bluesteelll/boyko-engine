//! Textured-PBR T2 — headless smoke test for [`build_texture_gpu`]: uploads a small
//! checkerboard [`TextureData`] into a fresh [`BindlessTextureTable`], proving the
//! mip-generating staged upload (create → stage level 0 → blit chain → bindless
//! register) round-trips with NO validation-layer complaint.
//!
//! # Scope
//!
//! T2 delivers the mip-generating upload + bindless-registration INFRASTRUCTURE
//! only — no material references a texture and no pipeline samples the bindless set
//! yet (T5/T6). This test proves the STRUCTURAL claim: a multi-mip `SAMPLED |
//! TRANSFER_DST | TRANSFER_SRC` `R8G8B8A8_UNORM` image with a mutable `R8G8B8A8_SRGB`
//! view builds, its mip chain blits cleanly (no VUID violation from the interleaved
//! barrier/blit/barrier sequence), and it registers into a real bindless slot.
//!
//! `#[ignore]`: a headless GPU test the orchestrator runs explicitly (this repo's
//! convention for GPU/screenshot tests — see `bindless_smoke.rs`).

mod common;

use boyko_rhi::RhiDevice;
use boyko_render::bindless::BindlessTextureTable;
use boyko_render::{ColorSpace, TextureData, build_texture_gpu, mip_levels_for};

use common::{assert_validation_clean, boot_or_skip};

#[test]
#[ignore]
fn texture_upload_smoke_checkerboard_8x8_builds_mip_chain_and_bindless_slot() {
    let Some(ctx) =
        boot_or_skip("texture_upload_smoke_checkerboard_8x8_builds_mip_chain_and_bindless_slot")
    else {
        return;
    };

    let mut table = match BindlessTextureTable::new(&ctx) {
        Ok(t) => t,
        Err(e) => panic!("BindlessTextureTable::new failed: {e:?}"),
    };

    // An 8x8 black/white checkerboard — small, visually distinct pixel data that
    // exercises every mip level down to 1x1 (8 -> 4 -> 2 -> 1).
    const DIM: u32 = 8;
    let mut rgba8 = Vec::with_capacity((DIM * DIM * 4) as usize);
    for y in 0..DIM {
        for x in 0..DIM {
            let v: u8 = if (x + y) % 2 == 0 { 0xFF } else { 0x00 };
            rgba8.extend_from_slice(&[v, v, v, 0xFF]);
        }
    }
    let data = TextureData {
        width: DIM,
        height: DIM,
        rgba8,
        color_space: ColorSpace::Srgb,
    };

    let gpu = build_texture_gpu(&ctx, &mut table, &data);

    assert!(gpu.bindless_slot >= 1, "slot 0 is reserved for the error texture");
    assert_eq!(gpu.mip_levels, mip_levels_for(DIM, DIM));
    assert_eq!(gpu.mip_levels, 4, "8x8 -> 4x4 -> 2x2 -> 1x1 is 4 mip levels");
    assert_eq!(gpu.width, DIM);
    assert_eq!(gpu.height, DIM);

    assert_validation_clean(&ctx);

    // Teardown: the texture is owned by this test (not by the table — real textures
    // live in `Assets<TextureGpu>`, T2); the table owns only the error texture + the
    // descriptor set.
    let _ = ctx.wait_idle();
    // SAFETY: `wait_idle` above drains the device; `gpu.texture` was created on
    // `ctx`, owned exclusively here, and its only submission (the staged mip-chain
    // upload) already fence-waited inside `build_texture_gpu` / `upload_texture_2d`;
    // the by-value move destroys it exactly once.
    unsafe {
        ctx.destroy_texture(gpu.texture);
    }
    table.destroy(&ctx);
}
