//! R2 windowed smoke: drives the FULL runner path headlessly — device
//! singleton boot → `WindowHost` boot → NonSend World residents → `finish()` →
//! ~5 presented frames → D2 teardown (world eviction + `destroy_singleton`) —
//! with the exit requested by an ordinary `AppExit`-setting system, proving the
//! loop AND the teardown work without a human at the window.
//!
//! Windowed-test conventions: `#[ignore]` (needs a real windowed GPU device),
//! graceful SKIP when boot fails, run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::RhiContext;

/// Frames left before the test requests exit. Decremented once per Main run.
#[derive(Resource)]
struct FrameBudget(u32);

/// Counts the budget down and requests exit on the last frame. The runner
/// observes `AppExit` after the frame completes (before the present), so the
/// loop ends deterministically after ~5 frames.
fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

const BUDGET: u32 = 5;

#[test]
#[ignore = "needs a real windowed GPU device; run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn windowed_smoke_five_frames_then_clean_teardown() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.add_systems(exit_after_budget);
    app.add_plugins(EnginePlugins::window("boyko_app R2 smoke", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner
    // exits BEFORE the frame loop, so the budget is untouched — SKIP (the
    // runner already logged the boot failure).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP windowed_smoke_five_frames_then_clean_teardown: windowed boot unavailable");
        return;
    }

    // The loop ran the exact budget: the exit system fired on frame 5.
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // D2 teardown left the World GPU-evicted: no device-referencing NonSend
    // resident may survive `destroy_singleton` (the `'static` fiction ended).
    assert!(
        !app.world().contains_non_send_resource::<RhiContext>(),
        "teardown must evict the shared-mode RhiContext"
    );
    assert!(
        !app.world().contains_non_send_resource::<boyko_app::GpuDevice>(),
        "teardown must evict the GpuDevice handle"
    );
}
