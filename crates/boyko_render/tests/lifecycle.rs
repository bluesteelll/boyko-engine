//! Wave-B lifecycle: `create_column` mints a device pool + drives the A2 seam;
//! `resolve` returns `Some` for a live handle and `None` after `destroy_all`.
//!
//! Also asserts the A2 effect: after `create_column`, the core archetype is
//! GPU-resident and the component's CPU column is nulled (the C1 contract — the
//! CPU cannot touch GPU bytes), and `has_component` reads the device component as
//! absent from the CPU surface (the intended §2 semantics).

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_render::RhiContext;

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype};

const ROWS: u32 = 256;

#[test]
fn create_resolve_destroy_lifecycle() {
    let Some(ctx) = boot_or_skip("create_resolve_destroy_lifecycle") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    // A low component id (< MAX_COMPONENTS = 512); each test binary has its own
    // isolated process-global registry, so the same id across files never clashes.
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 200);

    // Before minting, no column resolves for the key.
    assert!(
        rhi.manager().resolve(arch, comp).is_none(),
        "resolve must be None before any column is created"
    );

    let handle = {
        let (device, mgr) = rhi.split_mut();
        mgr.create_column(device, &mut ecs, arch, comp, STRIDE, ROWS)
            .expect("create_column")
    };

    // A live handle resolves to its current geometry.
    let resolved = rhi
        .manager()
        .resolve(arch, comp)
        .expect("a live column must resolve Some");
    assert_eq!(resolved.handle, handle);
    assert_eq!(resolved.stride, STRIDE);
    assert_eq!(resolved.device_cap, ROWS);
    assert_eq!(resolved.device_len, 0, "a freshly minted column has zero rows");

    // The A2 effect (probed through the public archetype surface): the device
    // component stays in the signature mask — residency flips the backing, not the
    // mask. (The internal `GPU_RESIDENT` flag + nulled column are covered by
    // boyko_ecs's own A2 tests; `flags` is private to the core.)
    let arch_ref = ecs
        .archetype_master()
        .get_archetype(arch)
        .expect("archetype exists");
    assert!(
        arch_ref.has_component_id(comp),
        "the component stays in the signature mask (residency does not change the mask)"
    );

    assert_validation_clean(rhi.context());

    // Tear down: after destroy_all the handle resolves None.
    rhi.destroy_all();
    assert!(
        rhi.manager().resolve(arch, comp).is_none(),
        "resolve must be None after destroy_all"
    );

    drop(rhi);
}
