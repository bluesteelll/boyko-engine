//! Wave-B C2 regression: dropping an [`RhiContext`] WITHOUT a manual `destroy_all`
//! frees every device resource (the production drop path).
//!
//! In production the ECS world's `NonSend` slab drops the `RhiContext` resource;
//! nobody calls `destroy_all` manually. Before the `impl Drop for RhiContext` fix
//! that meant EVERY device buffer leaked: the `VulkanContext` field dropped first
//! (freeing device memory + the device), then the registry's `Drop` found live
//! buffers → a release leak / a debug-build `debug_assert!` panic.
//!
//! This test mints a column + uploads, asserts the registry is NOT yet drained,
//! then lets the `RhiContext` drop WITHOUT a manual `destroy_all`. The `Drop` impl
//! must drain the registry; the registry's own `Drop` leak guard (a debug
//! `debug_assert!(false)`) would fail this test if anything leaked. We additionally
//! assert `is_fully_drained()` from inside a manual second `destroy_all` BEFORE the
//! implicit drop to prove idempotency does not double-free.

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_render::RhiContext;

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype, pattern_bytes};

const ROWS: usize = 256;

#[test]
fn drop_without_manual_destroy_all_frees_everything() {
    let Some(ctx) = boot_or_skip("drop_without_manual_destroy_all_frees_everything") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 203);

    let want = pattern_bytes(ROWS);

    let handle = {
        let (device, mgr) = rhi.split_mut();
        let h = mgr
            .create_column(device, &mut ecs, arch, comp, STRIDE, ROWS as u32)
            .expect("create_column");
        mgr.upload_initial(device, h, &want).expect("upload_initial");
        h
    };

    // A live column exists, so the registry is NOT drained — proving the implicit
    // drop below has real work to do (it is not a vacuous pass).
    assert!(
        rhi.manager().is_handle_live(handle),
        "the column is live before drop"
    );
    assert!(
        !rhi.manager().is_fully_drained(),
        "the registry holds the live column + staging before drop"
    );

    assert_validation_clean(rhi.context());

    // The crux: drop WITHOUT calling `destroy_all`. `RhiContext::Drop` must run
    // `manager.destroy_all(&self.context)` BEFORE the fields drop. If it does not,
    // the registry's own `Drop` leak guard trips the debug `debug_assert!(false)`
    // and fails this test (the regression). A clean drop here proves the production
    // teardown path frees everything.
    drop(rhi);
}

/// Idempotency (C2): an explicit `destroy_all` followed by the implicit `Drop`
/// must NOT double-free. The explicit call drains the registry; the `Drop` call
/// early-returns on the already-drained registry.
#[test]
fn explicit_destroy_all_then_drop_is_idempotent() {
    let Some(ctx) = boot_or_skip("explicit_destroy_all_then_drop_is_idempotent") else {
        return;
    };

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 204);

    let want = pattern_bytes(ROWS);
    {
        let (device, mgr) = rhi.split_mut();
        let h = mgr
            .create_column(device, &mut ecs, arch, comp, STRIDE, ROWS as u32)
            .expect("create_column");
        mgr.upload_initial(device, h, &want).expect("upload_initial");
    }

    // First teardown (explicit): drains the registry.
    rhi.destroy_all();
    assert!(
        rhi.manager().is_fully_drained(),
        "explicit destroy_all drained the registry"
    );

    assert_validation_clean(rhi.context());

    // Second teardown (implicit, via Drop): idempotent — early-returns on the
    // already-drained registry, no double-free. A clean drop proves it.
    drop(rhi);
}
