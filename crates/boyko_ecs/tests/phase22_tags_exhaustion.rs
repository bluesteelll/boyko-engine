//! Phase 22 (Tags) — dynamic-tag budget exhaustion (plan D3 mint protocol,
//! Wave 1B).
//!
//! DELIBERATELY a separate test binary: driving the process-global `NEXT_ID`
//! to `MAX_COMPONENTS` is permanent (the registry is write-once, ids are
//! never recycled), and any later typed mint in the same process —
//! `register_new` via a `#[derive(Component)]` type's first `component_id()`
//! — panics on the release exhaustion assert. Isolating the drain in its own
//! process keeps `phase22_tags.rs` (including its Wave-2A typed-component
//! additions) deterministic regardless of test scheduling.

use boyko_ecs::ecs::core::component::component_registry::MAX_COMPONENTS;
use boyko_ecs::prelude::EcsMaster;

#[test]
fn minting_past_the_shared_budget_returns_none_then_register_tag_panics() {
    let mut world = EcsMaster::new();

    // Interned BEFORE the drain; re-checked after it (name-keyed idempotency
    // must survive exhaustion).
    let survivor = world
        .try_register_tag("phase22_exhaust_survivor")
        .expect("budget must be available at test start");

    // Drain the shared ComponentId budget. Some slots may already be taken by
    // typed registrations in this process, so the ceiling can arrive early;
    // MAX_COMPONENTS + 1 fresh names always overshoot it.
    let mut exhausted = false;
    for i in 0..=MAX_COMPONENTS {
        let name = format!("phase22_exhaust_{i}");
        if world.try_register_tag(&name).is_none() {
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "minting MAX_COMPONENTS + 1 fresh names must exhaust the shared budget"
    );

    // Fallible mint at the ceiling: None, repeatably.
    assert!(
        world.try_register_tag("phase22_exhaust_post").is_none(),
        "a fresh name past the ceiling must return None"
    );

    // NAME-keyed idempotency survives exhaustion: an interned name still
    // resolves (idempotent re-mint is success, never None — plan D3 step 1).
    assert_eq!(
        world.try_register_tag("phase22_exhaust_survivor"),
        Some(survivor),
        "an interned name must keep resolving after exhaustion"
    );
    assert_eq!(
        world.tag_by_name("phase22_exhaust_survivor"),
        Some(survivor),
        "tag_by_name must keep resolving after exhaustion"
    );

    // Panicking sugar: pin the message (it must name the shared 512 budget).
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        world.register_tag("phase22_exhaust_panics");
    }))
    .expect_err("register_tag on a fresh name past the ceiling must panic");
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("<non-string panic payload>");
    assert!(
        msg.contains("shared component-id budget is exhausted"),
        "panic message must name the shared budget; got: {msg}"
    );
    assert!(
        msg.contains("512"),
        "panic message must name the 512-slot ceiling; got: {msg}"
    );
}
