//! SDF brick-atlas campaign M2 — the on-device atlas plumbing SMOKE check (`#[ignore]`,
//! RTX-only). NOT the correctness gate (the tester writes the baker-tile-correctness +
//! atlas-round-trip + offscreen golden); this only verifies that the device probe + the 3D
//! `R8_SNORM`/`R16_SFLOAT` atlas create + the CPU bake + the staged `TRANSFER_DST` upload run
//! VALIDATION-CLEAN on a real GPU (the M2-plumbing acceptance the developer self-runs).
//!
//! Run with `cargo test -p boyko_rhi_vulkan --test m2_brick_atlas_smoke -- --ignored`.

use boyko_rhi_vulkan::brick_atlas::BrickAtlas;
use boyko_rhi_vulkan::compute::{AtlasEncoding, SdfEdit, sdf_op};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi::RhiDevice;
use boyko_sdf_math::SdfEditField;

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP log) when no
/// GPU / loader / validation layer / dynamic-rendering is available.
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

/// The crater demo scene (a sphere with a subtracted dimple) — a SURFACE-rich field so the
/// atlas bake covers multiple SURFACE cells.
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// Builds the authority `SdfEditField` from a slice of edits (bumping its gen, like the
/// render path).
fn field_of(edits: &[SdfEdit]) -> SdfEditField {
    let mut field = SdfEditField::new();
    for e in edits {
        assert!(field.push(*e), "scene must fit MAX_SDF_EDITS");
    }
    field.bump_gen();
    field
}

/// M2 on-device atlas SMOKE: the device probe picks a linear-filterable atlas format, the 3D
/// atlas creates, the CPU bake covers SURFACE cells, and the staged upload runs
/// validation-clean. Asserts the messenger recorded ZERO messages across the create + upload.
#[test]
#[ignore = "GPU on-device smoke — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn m2_brick_atlas_creates_and_uploads_on_device() {
    let Some(ctx) = boot_or_skip("m2_brick_atlas_creates_and_uploads_on_device") else {
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

    let caps = ctx.device_caps();
    let chosen = AtlasEncoding::from_linear_filter_ok(caps.atlas_linear_filter_ok);
    println!(
        "[m2-smoke] atlas_linear_filter_ok={} chosen_encoding={chosen:?} atlas_format={:?}",
        caps.atlas_linear_filter_ok,
        caps.atlas_format()
    );

    let field = field_of(&crater());

    // Create the atlas (image + sampler) + bake + upload — the whole M2 plumbing path.
    let atlas = BrickAtlas::create(&ctx, &field)
        .expect("M2 brick atlas should create + bake + upload on a Vulkan device");
    assert_eq!(atlas.encoding(), chosen, "atlas encoding must match the device probe");

    // A re-bake (the per-gen full-rebuild path) must also run clean.
    atlas
        .rebake(&ctx, &field)
        .expect("M2 brick atlas rebake should upload validation-clean");

    // M3 — the INCREMENTAL dirty rebake (the dynamic-edit fast path). Move the carver edit, bump
    // the gen, and re-bake ONLY the dirty cells via the sub-region upload. The prior full bake
    // seeded `prev_aabb`, so moving edit 1 dirties the swept old+new region (the union-dirty rule
    // clears the ghost at the old location). It MUST run validation-clean.
    let mut moved = field;
    moved.move_edit(1, [0.6, 0.0, 0.0]);
    moved.bump_gen();
    let uploaded = atlas
        .rebake_dirty(&ctx, &moved)
        .expect("M2 brick atlas rebake_dirty should upload validation-clean");
    assert!(uploaded, "moving an edit must dirty at least one cell (an upload must run)");
    // Snapshot the dirty ledger so a subsequent edit diffs against the freshly-baked state.
    moved.clear_dirty();

    // A `rebake_dirty` with NO change must be a no-op (no submit, atlas already current).
    let noop = atlas
        .rebake_dirty(&ctx, &moved)
        .expect("M2 brick atlas rebake_dirty (no change) should not error");
    assert!(!noop, "an unchanged authority must not upload");

    // The validation messenger must be silent across the create + the full + the dirty uploads.
    if !ctx.validation_enabled() {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation reported {} message(s) during the M2 atlas create/upload",
        state.total()
    );

    // Drain the device before teardown (the upload fences are already waited; this is
    // belt-and-braces so the destroy never races an in-flight submission).
    RhiDevice::wait_idle(&ctx).expect("wait_idle before atlas teardown");
    // SAFETY: `ctx` is the live context the atlas was created on; the device is drained
    // (wait_idle above + the fenced uploads completed), so nothing references the image; the
    // by-value move destroys the image + sampler exactly once.
    unsafe { atlas.destroy(&ctx) };

    println!("[m2-smoke] OK — atlas created, baked, uploaded, validation-clean");
}
