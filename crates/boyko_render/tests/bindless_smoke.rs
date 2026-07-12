//! T4 — headless smoke test for [`BindlessTextureTable`]: creates the bindless
//! descriptor set (layout + UPDATE_AFTER_BIND pool + set + shared sampler), writes
//! two distinct textures into it via [`BindlessTextureTable::register`], and
//! asserts the validation layer recorded zero messages.
//!
//! # Scope (why this is a smoke test, not the full `NonUniformResourceIndex` proof)
//!
//! T4 (this rung) delivers the bindless descriptor INFRASTRUCTURE only — no
//! pipeline binds this set yet (T6 wires the textured pipeline). Proving
//! `NonUniformResourceIndex` genuinely round-trips a dynamically-indexed array read
//! requires a shader that samples `gTextures[push_constant_index]` and a
//! SPIR-V-disassembly check for the `NonUniform` decoration on the array access (the
//! DXC hazard the T4 plan calls out) — that shader + disassembly harness is a
//! follow-up the orchestrator drives once T6 lands a real bindless-sampling shader.
//! This test instead proves the STRUCTURAL claim: the bindless set can be created,
//! written incrementally at two different slots with NO validation-layer complaint
//! (no VUID violation from the UPDATE_AFTER_BIND / VARIABLE_DESCRIPTOR_COUNT /
//! PARTIALLY_BOUND declaration or the per-slot write), and torn down cleanly.
//!
//! `#[ignore]`: a headless GPU test the orchestrator runs explicitly (this repo's
//! convention for GPU/screenshot tests — see `ui_hud_screenshot.rs`).

mod common;

use boyko_rhi::RhiDevice;
use boyko_render::bindless::{BindlessTextureTable, create_solid_color_texture};

use common::{assert_validation_clean, boot_or_skip};

#[test]
#[ignore]
fn bindless_smoke_create_write_two_slots_and_teardown() {
    let Some(ctx) = boot_or_skip("bindless_smoke_create_write_two_slots_and_teardown") else {
        return;
    };

    let table = BindlessTextureTable::new(&ctx);
    let mut table = match table {
        Ok(t) => t,
        Err(e) => panic!("BindlessTextureTable::new failed: {e:?}"),
    };
    assert!(
        table.capacity() > 1,
        "the table must reserve slot 0 plus at least one real slot"
    );

    // Two visually-distinct solid-color textures (red / green) so slots 1 and 2
    // hold genuinely different content — the future sample+readback follow-up can
    // assert on these exact colors without changing this setup.
    let red = create_solid_color_texture(&ctx, 4, 4, [0xFF, 0x00, 0x00, 0xFF])
        .expect("red test texture create+upload");
    let green = create_solid_color_texture(&ctx, 4, 4, [0x00, 0xFF, 0x00, 0xFF])
        .expect("green test texture create+upload");

    let slot_red = table.register(&ctx, red.view());
    let slot_green = table.register(&ctx, green.view());

    assert_eq!(slot_red, 1, "the first registration must take the lowest real slot");
    assert_eq!(slot_green, 2, "the second registration must take the next real slot");
    assert_ne!(slot_red, 0, "slot 0 is reserved for the error texture");
    assert_ne!(slot_green, 0, "slot 0 is reserved for the error texture");

    assert_validation_clean(&ctx);

    // Teardown: the two test textures are owned by this test (not by the table —
    // real textures live in `Assets<TextureGpu>`, T2); the table owns only the
    // error texture + the descriptor set.
    let _ = ctx.wait_idle();
    // SAFETY: `wait_idle` above drains the device; `red`/`green` were created on
    // `ctx`, owned exclusively here, and never bound to any command buffer beyond
    // their own upload submit (already fence-waited inside
    // `create_solid_color_texture`); each is moved by value ⇒ destroyed exactly
    // once.
    unsafe {
        ctx.destroy_texture(red);
        ctx.destroy_texture(green);
    }
    table.destroy(&ctx);
}
