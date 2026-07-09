//! Wave-B X1/X2 regression: the pair-keyed `meta` table survives staging/grow
//! churn — a grow of one column AND a staging regrow never null or alias a LIVE
//! column's meta entry.
//!
//! The single-column tests (`round_trip`, `grow_stale`, `lifecycle`) never
//! exercise the buffer-slot-index COLLISION between the staging buffer and a live
//! column, nor a second live column whose slot index a grow/staging-regrow could
//! reuse. This test mints TWO device columns (in two GPU-pure archetypes), uploads
//! distinct patterns, grows ONE column, and forces a staging regrow — then asserts
//! BOTH columns still `resolve()` to `Some` with the correct geometry AND bytes.
//! Under the old index-keyed table this would silently null a live column's entry
//! (resolve → `None`) or alias it; the pair-keyed table makes it impossible.

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_render::RhiContext;

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype, pattern_bytes};

const ROWS_A: usize = 64;
const ROWS_B: usize = 256;
const GROWN_A: u32 = 1024;

#[test]
fn two_columns_survive_grow_and_staging_regrow() {
    let Some(ctx) = boot_or_skip("two_columns_survive_grow_and_staging_regrow") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();

    // Two GPU-pure single-component archetypes (distinct ids => distinct
    // archetypes => two device columns). Each test binary has its own isolated
    // process-global registry, so these ids are safe.
    let (arch_a, comp_a) = gpu_pure_archetype(&mut ecs, 201);
    let (arch_b, comp_b) = gpu_pure_archetype(&mut ecs, 202);

    // Distinct byte patterns. B is larger, so uploading B regrows the SHARED
    // staging buffer (and frees the old staging slot — the X2 collision trigger).
    let want_a = pattern_bytes(ROWS_A);
    let mut want_b = pattern_bytes(ROWS_B);
    // Make B's bytes obviously distinct from A's so an alias would be detected.
    for byte in &mut want_b {
        *byte ^= 0x5A;
    }

    // Mint + upload BOTH columns. The second upload regrows staging.
    let (handle_a, handle_b) = {
        let (device, mgr) = rhi.split_mut();
        let ha = mgr
            .create_column(device, &mut ecs, arch_a, comp_a, STRIDE, ROWS_A as u32)
            .expect("create_column A");
        mgr.upload_initial(device, ha, &want_a).expect("upload A");

        let hb = mgr
            .create_column(device, &mut ecs, arch_b, comp_b, STRIDE, ROWS_B as u32)
            .expect("create_column B");
        // This upload's staging need (ROWS_B * STRIDE) exceeds A's staging cap, so
        // `ensure_staging` frees the old staging buffer and registers a new one —
        // the exact churn that, under the old index-keyed table, could null a live
        // column's meta entry.
        mgr.upload_initial(device, hb, &want_b).expect("upload B");
        (ha, hb)
    };

    // Both columns resolve to their CURRENT handles before the grow.
    assert_eq!(
        rhi.manager().resolve(arch_a, comp_a).expect("A resolves").handle,
        handle_a,
        "column A resolves before grow"
    );
    assert_eq!(
        rhi.manager().resolve(arch_b, comp_b).expect("B resolves").handle,
        handle_b,
        "column B resolves before grow"
    );

    // Grow column A: rotates A's handle, reallocs A's device buffer (and frees A's
    // OLD slot index — which a later allocation may reuse).
    let grown_a = {
        let (device, mgr) = rhi.split_mut();
        mgr.grow_column(device, &mut ecs, handle_a, GROWN_A)
            .expect("grow_column A")
    };
    assert_ne!(grown_a, handle_a, "grow rotated A's handle");

    // Force a staging regrow by reading B back through the (now possibly smaller)
    // staging buffer at its full byte length — exercises the staging take/register
    // churn AGAIN after the grow rotated slot indices.
    let got_b = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, handle_b, want_b.len())
            .expect("readback B after grow")
    };

    // ===== The X1/X2 assertions: BOTH columns are still live + correct. =====

    // Column B's entry was NOT nulled or aliased by A's grow or the staging churn.
    let resolved_b = rhi
        .manager()
        .resolve(arch_b, comp_b)
        .expect("X2: column B must still resolve Some after A's grow + staging regrow");
    assert_eq!(resolved_b.handle, handle_b, "B's handle is unchanged (B was not grown)");
    assert_eq!(resolved_b.device_len, ROWS_B as u32, "B's row count intact");
    assert_eq!(resolved_b.device_cap, ROWS_B as u32, "B's capacity intact");
    assert_eq!(got_b, want_b, "B's bytes intact (no alias overwrote them)");

    // Column A's entry was upserted in place by the grow — exactly one entry, the
    // rotated handle, with the grown capacity and preserved rows + bytes.
    let resolved_a = rhi
        .manager()
        .resolve(arch_a, comp_a)
        .expect("X1: column A must resolve Some to its rotated handle");
    assert_eq!(resolved_a.handle, grown_a, "A resolves to the rotated handle");
    assert_eq!(resolved_a.device_cap, GROWN_A, "A's capacity grew");
    assert_eq!(resolved_a.device_len, ROWS_A as u32, "A's row count preserved across grow");

    let got_a = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, grown_a, want_a.len())
            .expect("readback A after grow")
    };
    assert_eq!(got_a, want_a, "A's bytes preserved across grow");

    // The OLD A handle is stale (loud MF-7), and resolves are still single-valued.
    assert!(
        !rhi.manager().is_handle_live(handle_a),
        "MF-7: A's OLD handle is stale after grow"
    );

    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
