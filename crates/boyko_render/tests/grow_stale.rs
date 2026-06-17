//! Wave-B MF-7: `grow_column` rotates the handle — the OLD handle resolves stale
//! (`None`), the NEW handle resolves live + the data is preserved.
//!
//! Mint a column, upload a pattern, grow it (realloc to a larger device buffer +
//! copy old→new). The registry's take-before-register ordering bumps the reused
//! slot's generation, so the OLD `u64` resolves to no live buffer (loud
//! stale-handle safety). The NEW handle resolves Some, and a readback of the new
//! buffer returns the preserved bytes. Validation-clean throughout.

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_render::RhiContext;

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype, pattern_bytes};

const ROWS: usize = 128;
const GROWN_ROWS: u32 = 512;

#[test]
fn grow_column_stales_old_handle_and_preserves_data() {
    let Some(ctx) = boot_or_skip("grow_column_stales_old_handle_and_preserves_data") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    // A low component id (< MAX_COMPONENTS = 512); each test binary has its own
    // isolated process-global registry, so the same id across files never clashes.
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 200);

    let want = pattern_bytes(ROWS);

    // Mint + upload the initial pattern.
    let old_handle = {
        let (device, mgr) = rhi.split_mut();
        let h = mgr
            .create_column(device, &mut ecs, arch, comp, STRIDE, ROWS as u32)
            .expect("create_column");
        mgr.upload_initial(device, h, &want).expect("upload_initial");
        h
    };
    assert!(
        rhi.manager().is_handle_live(old_handle),
        "the freshly minted handle is live before the grow"
    );

    // Grow: realloc to a larger device buffer + copy old→new (MF-7).
    let new_handle = {
        let (device, mgr) = rhi.split_mut();
        mgr.grow_column(device, &mut ecs, old_handle, GROWN_ROWS)
            .expect("grow_column")
    };
    assert_ne!(new_handle, old_handle, "grow rotates the handle");

    // MF-7: the OLD handle resolves stale (None); the NEW handle resolves live.
    assert!(
        !rhi.manager().is_handle_live(old_handle),
        "MF-7: the OLD handle must resolve stale (None) after grow_column"
    );
    assert!(
        rhi.manager().is_handle_live(new_handle),
        "the NEW handle must resolve live after grow_column"
    );

    // `resolve` (keyed by (archetype, component)) now yields the NEW handle with
    // the grown capacity and preserved row count.
    let resolved = rhi.manager().resolve(arch, comp).expect("resolve the grown column");
    assert_eq!(resolved.handle, new_handle, "resolve returns the rotated handle");
    assert_eq!(resolved.device_cap, GROWN_ROWS, "capacity grew");
    assert_eq!(resolved.device_len, ROWS as u32, "row count preserved across grow");

    // The grown buffer still holds the original bytes (the old→new copy).
    {
        let (device, mgr) = rhi.split_mut();
        let got = mgr
            .readback_for_test(device, new_handle, want.len())
            .expect("readback_for_test on the grown column");
        assert_eq!(got, want, "data must be preserved across grow_column");
    }

    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
