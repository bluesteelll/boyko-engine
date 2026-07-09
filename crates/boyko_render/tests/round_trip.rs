//! Wave-B oracle: `upload_initial` → `readback_for_test` bit-exact round-trip on
//! a REAL device-resident column, validation-clean.
//!
//! Write a deterministic byte pattern, stage + copy it into the device-local
//! column, read it back through the test-only device→staging copy, and assert
//! every byte matches. The device buffer is never CPU-mapped, so the bytes can
//! only have arrived through real `vkCmdCopyBuffer`s — proving the device-local
//! column + upload/readback path. Then assert the validation layer reported 0
//! messages (the §6 soundness oracle).

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_render::{GpuColumnManager, RhiContext};

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype, pattern_bytes};

const ROWS: usize = 1024;

#[test]
fn upload_then_readback_is_bit_exact() {
    let Some(ctx) = boot_or_skip("upload_then_readback_is_bit_exact") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    // A low component id (< MAX_COMPONENTS = 512); each test binary has its own
    // isolated process-global registry, so the same id across files never clashes.
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 200);

    let want = pattern_bytes(ROWS);

    let handle = {
        let (device, mgr): (_, &mut GpuColumnManager) = rhi.split_mut();
        let handle = mgr
            .create_column(device, &mut ecs, arch, comp, STRIDE, ROWS as u32)
            .expect("create_column");
        mgr.upload_initial(device, handle, &want).expect("upload_initial");
        let got = mgr
            .readback_for_test(device, handle, want.len())
            .expect("readback_for_test");
        assert_eq!(got, want, "device column readback must be bit-exact after upload");
        handle
    };

    // The resolved column reports the uploaded geometry.
    let resolved = rhi.manager().resolve(arch, comp).expect("resolve a live column");
    assert_eq!(resolved.handle, handle, "resolve returns the current handle");
    assert_eq!(resolved.stride, STRIDE);
    assert_eq!(resolved.device_len, ROWS as u32, "uploaded rows recorded");
    assert_eq!(resolved.device_cap, ROWS as u32);

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
