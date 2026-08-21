//! [`fixed_advance`] — the unified fixed-timestep catch-up driver
//! (Phase 20 plan D3).
//!
//! ONE shared function drives the accumulator on every target: `App` calls it
//! with `|w| fixed.run(w)`, a pool-less (wasm) runner calls it with its
//! sequential step closure, and Miri tests call it with a counting closure.
//! Monomorphized over the closure — no `dyn`, no indirect call on the substep
//! path.

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::time::fixed_time::FixedTime;
use crate::ecs::core::time::time::Time;

/// Runs the fixed-timestep catch-up loop once for the current frame and
/// returns the substep count.
///
/// Reads `Time::delta` (the already-clamped, already-scaled virtual delta),
/// accumulates it into `FixedTime::overstep`, then expends whole timesteps —
/// calling `step(world)` once per substep — until the accumulator drops below
/// one step. Writes [`FixedTime::steps_this_frame`] after the loop.
///
/// # Contract (plan ★m6)
///
/// Call EXACTLY ONCE per [`Time::advance_with`]: the function consumes the
/// frame's `Time::delta` by value into the accumulator, so a second call in
/// the same frame would re-accumulate the same delta and double the simulated
/// time.
///
/// # Timestep snapshot (plan ★M3)
///
/// The timestep is snapshotted ONCE at entry and threaded through every
/// expend; a fixed-schedule system calling `set_timestep` via
/// `ResMut<FixedTime>` mid-loop therefore takes effect on the NEXT frame's
/// loop, preserving the `⌈(max_delta × speed + timestep) / timestep⌉` substep
/// bound. A mid-loop mutation of `overstep` itself (e.g.
/// [`FixedTime::discard_overstep`]) IS observable on the next loop iteration
/// — the same exposure Bevy has.
///
/// # Bound
///
/// At the defaults (250 ms `max_delta`, 64 Hz, speed 1.0) the loop runs at
/// most 16 substeps per frame; a paused or zero-delta frame runs 0 (the
/// accumulator is `< timestep` between frames). Raising `max_delta`, the
/// speed, or lowering the timestep raises the bound proportionally (plan D4).
///
/// # Panics
///
/// Panics if the world is missing the `Time` or `FixedTime` resource —
/// `App::finish` inserts both; a hand-rolled (pool-less) runner must insert
/// them itself before the first frame.
pub fn fixed_advance<F: FnMut(&mut EcsMaster)>(world: &mut EcsMaster, mut step: F) -> u32 {
    let delta = world
        .try_resource::<Time>()
        .expect(
            "fixed_advance requires the `Time` resource — insert `Time::default()` \
             before the first frame (App::finish does this automatically)",
        )
        .delta();

    // Accumulate this frame's virtual delta and snapshot the timestep ONCE
    // (★M3). The `resource_mut` borrow ends at the end of the block, so the
    // loop below re-borrows freely (the critic-verified borrow choreography).
    let ts = {
        let fixed = world.try_resource_mut::<FixedTime>().expect(
            "fixed_advance requires the `FixedTime` resource — insert `FixedTime::default()` \
             before the first frame (App::finish does this automatically)",
        );
        fixed.accumulate(delta);
        fixed.timestep()
    };

    // One `resource_mut` re-borrow per substep: a slab-indexed lookup,
    // single-digit ns against the µs-scale `step` it brackets (plan D3
    // trade-off). The re-borrow is REQUIRED — `step` takes `&mut EcsMaster`,
    // so no `&mut FixedTime` may live across it.
    let mut steps: u32 = 0;
    while world.resource_mut::<FixedTime>().expend(ts) {
        step(world);
        steps += 1;
    }

    let fixed = world.resource_mut::<FixedTime>();
    fixed.set_steps_this_frame(steps);
    debug_assert!(
        fixed.overstep() < ts,
        "invariant: post-loop overstep must be below the loop-entry timestep snapshot"
    );
    steps
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Fresh world with both clock resources at engine defaults
    /// (250 ms clamp, 64 Hz).
    fn world_with_clocks() -> EcsMaster {
        let mut world = EcsMaster::new();
        world.insert_resource(Time::default());
        world.insert_resource(FixedTime::default());
        world
    }

    /// Advances `Time` by `raw` and runs `fixed_advance` with a counting
    /// closure; returns the substep count.
    fn advance_and_count(world: &mut EcsMaster, raw: Duration) -> u32 {
        world.resource_mut::<Time>().advance_with(raw);
        let mut counted = 0u32;
        let steps = fixed_advance(world, |_| counted += 1);
        assert_eq!(counted, steps, "the closure fires exactly once per counted substep");
        steps
    }

    // ── T3 — `fixed_advance` with a counting closure ─────────────────────────

    /// 0-step case: a sub-timestep delta accumulates without stepping.
    #[test]
    fn sub_timestep_delta_runs_zero_steps() {
        let mut world = world_with_clocks();
        let steps = advance_and_count(&mut world, Duration::from_millis(1));
        assert_eq!(steps, 0);
        let ft = world.resource::<FixedTime>();
        assert_eq!(ft.steps_this_frame(), 0);
        assert_eq!(ft.overstep(), Duration::from_millis(1));
        assert_eq!(ft.elapsed(), Duration::ZERO);
    }

    /// 1-step case: exactly one timestep expends once with a ZERO remainder.
    #[test]
    fn exact_timestep_runs_one_step() {
        let mut world = world_with_clocks();
        let ts = world.resource::<FixedTime>().timestep();
        let steps = advance_and_count(&mut world, ts);
        assert_eq!(steps, 1);
        let ft = world.resource::<FixedTime>();
        assert_eq!(ft.steps_this_frame(), 1);
        assert_eq!(ft.overstep(), Duration::ZERO);
        assert_eq!(ft.elapsed(), ts);
    }

    /// 16-step case (P20-B3 shape): 250 ms at 64 Hz is EXACTLY 16 steps with
    /// a ZERO remainder — and a 1 s hitch clamps to the same 16.
    #[test]
    fn quarter_second_at_64_hz_is_exactly_16_steps() {
        let mut world = world_with_clocks();
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(250)), 16);
        assert_eq!(world.resource::<FixedTime>().overstep(), Duration::ZERO);

        // 1 s raw ⇒ inflow-clamped to 250 ms ⇒ the same 16 steps; the real
        // clock keeps the full second.
        let mut world = world_with_clocks();
        assert_eq!(advance_and_count(&mut world, Duration::from_secs(1)), 16);
        assert_eq!(world.resource::<Time>().real_delta(), Duration::from_secs(1));
    }

    /// Accumulate-across-frames: two 8 ms frames step 0 then 1, with the
    /// exact 375 µs remainder.
    #[test]
    fn overstep_accumulates_across_frames() {
        let mut world = world_with_clocks();
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(8)), 0);
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(8)), 1);
        let ft = world.resource::<FixedTime>();
        assert_eq!(ft.overstep(), Duration::from_micros(375));
        assert_eq!(ft.elapsed(), ft.timestep());
    }

    /// Paused ⇒ zero virtual delta ⇒ zero substeps; `steps_this_frame`
    /// re-records 0.
    #[test]
    fn paused_frame_runs_zero_steps() {
        let mut world = world_with_clocks();
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(20)), 1);
        world.resource_mut::<Time>().pause();
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(100)), 0);
        assert_eq!(world.resource::<FixedTime>().steps_this_frame(), 0);
    }

    /// ★M3: a step closure re-setting the timestep mid-loop does NOT change
    /// the running loop's expend amount — the entry snapshot governs; the new
    /// timestep takes effect on the next frame's loop.
    #[test]
    fn mid_loop_set_timestep_takes_effect_next_frame() {
        let mut world = world_with_clocks();
        world.resource_mut::<Time>().advance_with(Duration::from_millis(250));
        let steps = fixed_advance(&mut world, |w| {
            // 1 ms would mean 250 steps if it took effect immediately.
            w.resource_mut::<FixedTime>().set_timestep(Duration::from_millis(1));
        });
        assert_eq!(steps, 16, "the loop-entry snapshot governs the whole loop");

        // Next frame the staged 1 ms timestep governs: 10 ms ⇒ 10 steps.
        assert_eq!(advance_and_count(&mut world, Duration::from_millis(10)), 10);
    }

    /// The step closure observes the world mid-catch-up (it IS the substep).
    #[test]
    fn step_closure_receives_the_world() {
        let mut world = world_with_clocks();
        world.resource_mut::<Time>().advance_with(Duration::from_millis(40));
        let steps = fixed_advance(&mut world, |w| {
            // Touch a resource through the world borrow per substep.
            let _ = w.resource::<FixedTime>().overstep();
        });
        assert_eq!(steps, 2);
    }

    // ── T3 — missing-resource panics (★m6) ───────────────────────────────────

    #[test]
    #[should_panic(expected = "fixed_advance requires the `Time` resource")]
    fn missing_time_panics() {
        let mut world = EcsMaster::new();
        world.insert_resource(FixedTime::default());
        let _ = fixed_advance(&mut world, |_| {});
    }

    #[test]
    #[should_panic(expected = "fixed_advance requires the `FixedTime` resource")]
    fn missing_fixed_time_panics() {
        let mut world = EcsMaster::new();
        world.insert_resource(Time::default());
        let _ = fixed_advance(&mut world, |_| {});
    }

    // ── Proptest — the accumulator invariants (plan §Test matrix) ────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            // Each case spins up a fresh world; cap the case count to keep
            // wall-time bounded (the math under test is pure integer ns).
            #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

            /// For arbitrary raw-delta sequences (0..400 ms — straddling the
            /// 250 ms clamp), every frame upholds:
            /// `steps == floor((overstep_prev + delta) / timestep)`,
            /// post-loop `overstep < timestep`, and
            /// `elapsed == steps_total × timestep`.
            #[test]
            #[cfg_attr(
                miri,
                ignore = "Miri wall-time: 64 cases x up to 39 frames on a fresh world each; \
                          the same accumulate/expend math is covered there by \
                          miri_fixed_loop's M-P20-1"
            )]
            fn fixed_advance_invariants(
                raw_deltas in proptest::collection::vec(0u64..400_000_000, 1..40)
            ) {
                let mut world = world_with_clocks();
                let ts = world.resource::<FixedTime>().timestep();
                let mut steps_total: u32 = 0;

                for raw_ns in raw_deltas {
                    let overstep_prev = world.resource::<FixedTime>().overstep();
                    world
                        .resource_mut::<Time>()
                        .advance_with(Duration::from_nanos(raw_ns));
                    let delta = world.resource::<Time>().delta();
                    let expected =
                        u32::try_from((overstep_prev + delta).as_nanos() / ts.as_nanos())
                            .expect("invariant: the clamp bounds steps far below u32::MAX");

                    let steps = fixed_advance(&mut world, |_| {});
                    steps_total += steps;

                    let ft = world.resource::<FixedTime>();
                    prop_assert_eq!(steps, expected, "steps == floor((prev + delta) / ts)");
                    prop_assert_eq!(ft.steps_this_frame(), steps);
                    prop_assert!(ft.overstep() < ts, "post-loop overstep < timestep");
                    prop_assert_eq!(ft.elapsed(), ts * steps_total, "elapsed == steps_total x ts");
                }
            }
        }
    }
}
