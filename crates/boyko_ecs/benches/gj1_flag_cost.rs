//! `GJ1` / `sched_cpu_flag_on_off` — what the runtime flags cost while they are ON.
//!
//! Profiling rung 16 (= joint rung J2). The rung was **refused on a measurement** at rung 15's
//! close, and the refusal enumerated its own preconditions so it could be re-entered rather than
//! re-argued. Re-measured today, every one of them has changed:
//!
//! | precondition | at rung 15 | now |
//! |---|---|---|
//! | production emission sites | **2**, and neither real (a comment and the rung's own fixture) | **71**, across `boyko_app`, `boyko_ecs`, `boyko_render`, `boyko_physics`, `boyko_image`, `boyko_demo` |
//! | manifests depending on `boyko-log` | 2 | **12**, including `boyko_app`, `boyko_render`, `boyko_rhi_vulkan` |
//! | a non-test `Profiler::arm` | none | `boyko_app/src/plugins.rs`, behind `BOYKO_PROFILE_ON` |
//! | a non-test `boyko_log::enable` | none | `boyko_app/src/plugins.rs`, behind `BOYKO_LOG` |
//!
//! So leg **A** — "profiler armed, logger enabled" — is constructible, and the rung is entered.
//!
//! # What this bench measures, and the leg it deliberately does NOT build
//!
//! GJ1 specifies three legs. **(A)** flags on. **(B)** the *same binary*, flags off. **(C)** a
//! build with the const ceiling forced permissive and the flags off, so every site the shipping
//! ceiling deleted survives and pays one `.bss` load and one predicted branch.
//!
//! **(A vs B) is measured here, in one sitting, ABBA-counterbalanced, against an interleaved zero
//! control.** That is the half of the claim this instrument can settle.
//!
//! **(C) is not built, and the reason is a measurement rather than effort.** Leg C is a DIFFERENT
//! BUILD — `BOYKO_PROFILE` is a whole-build axis — so (B vs C) is cross-build by construction, and
//! this campaign measured today what a cross-build absolute is worth on this box: the same
//! unchanged bench leg read **10.16 / 10.94 / 11.72 / 12.11 ns** across four sittings, a spread
//! wider than anything the comparison could have found. A number taken that way would be drift
//! wearing a verdict's name.
//!
//! The half leg C exists to prove — *that the residual per-site floor is real and only the compile
//! ceiling removes it* — **is already measured in-sitting**, by `boyko_log/benches/log_gate_cost`:
//! a site the ceiling kept and the runtime refused, against a site the ceiling deleted, in one
//! process. That is the same question with none of the cross-build noise.
//!
//! # Why the instrument is `boyko_log`'s and not a copy
//!
//! `instrument.rs` encodes a rule that was got wrong once and would be got wrong again in each
//! copy: a spread floor must be the clock's RESOLUTION, never a fraction of the reading. This box's
//! clock ticks at 100 ns, so a per-call figure from an N-call block cannot express anything finer
//! than `100 / N`.

use std::hint::black_box;
use std::time::Instant;

#[path = "../../boyko_log/benches/instrument.rs"]
mod instrument;
use instrument::{med_and_floor, resolution_ns};

use boyko_diag::profiling_abi::ZoneTier;
use boyko_ecs::ecs::core::profiling::{Profiler, ProfilerConfig, fold};
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, trace};

// A bench is a `User` crate by the partition macro's own list ("games, plugins, mods, tools,
// benches"). Claiming `Engine` here would not compile, which is the mechanism working.
boyko_diag::profiling_partition!(User);

// `Always`, so no profile's compile tier can delete the site. A bench whose subject vanished in
// the profile under test would report the absence of its own instrument as a result.
boyko_diag::declare_zone!(GJ1_BODY, name = "gj1.body", scope = 0, tier = ZoneTier::Always);

/// Iterations per timed block.
///
/// Bounded by the LANE, not by taste: a lane ring is 16 KiB and holds a few hundred records, so a
/// longer block would spend most of its iterations on the refusal path — measuring what a full
/// ring costs under the name of what an emission costs. The drain runs between blocks, never
/// inside one. The same bound applies to the sample region, which `fold` empties beside it.
const CALLS: u32 = 256;

/// Rounds in the sitting. The legs interleave, so drift lands on all of them.
const ROUNDS: usize = 41;

/// One iteration of the "schedule body": one profiling zone and one log site.
///
/// Both subsystems in one body on purpose. GJ1's subject is the JOINT configuration — the whole
/// reason rung 16 exists is that a flag-off number taken without the other subsystem present is
/// not a number about the both-present configuration.
#[inline(never)]
fn body(i: u32) {
    let _z = boyko_diag::zone!(GJ1_BODY);
    trace!(Log, "gj1 body {}", black_box(i));
}

/// Time one block, with the drain and the fold OUTSIDE the reading.
#[inline(never)]
fn time_block(profiler: &mut Profiler) -> f64 {
    let mut barrier = 0u8;
    let t0 = Instant::now();
    for i in 0..CALLS {
        black_box(&mut barrier);
        body(i);
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(CALLS);
    black_box(barrier);
    // Outside the reading, and both of them: a block that left the lane full would make the NEXT
    // block measure refusals, and one that left the sample region full would do the same one
    // subsystem over.
    if let DrainResult::Ran(_) = drain() {}
    fold(profiler);
    ns
}

/// The empty control: the same loop shape with no zone and no log site.
#[inline(never)]
fn time_empty() -> f64 {
    let mut barrier = 0u8;
    let t0 = Instant::now();
    for i in 0..CALLS {
        black_box(&mut barrier);
        black_box(i);
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(CALLS);
    black_box(barrier);
    ns
}

/// Turn BOTH subsystems on, and **assert that both actually came on**.
///
/// Without the assertions this bench could measure the logger alone and report it as the joint
/// configuration -- which is the whole thing rung 16 exists to prevent, reproduced inside the rung.
/// `ARM_MASK == 0` with a green verdict would be a number about one subsystem wearing the name of
/// two. GJ1's own leg (B) is specified as "`ARM_MASK == 0`, every `CONTROL` byte `Off`", so both
/// sides of the pair are checked rather than assumed.
fn flags_on(profiler: &mut Profiler) {
    profiler.arm(ProfilerConfig::default());
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    assert!(
        boyko_diag::profiling_abi::arm_mask_bits() != 0,
        "leg A asked for the profiler and did not get it: ARM_MASK is 0, so every number below would be the logger's alone under a joint name"
    );
    assert!(
        boyko_diag::profiling_abi::scope_armed(GJ1_BODY::SCOPE),
        "leg A armed the profiler but not THIS zone's scope, so its site is still a closed gate"
    );
    assert!(
        boyko_log::runtime_ceiling(<Log as LogTarget>::ID) >= Level::Trace as u8,
        "leg A asked for the logger and did not get it"
    );
}

/// Turn both off, and assert the state GJ1 specifies for leg (B).
/// Profiler on, logger off. **This is logging's `P1` (`sched_cpu_logger_on_off`) subject**, and it
/// is taken in this sitting rather than its own because J2's whole instruction is to re-take the
/// baselines *in the both-present configuration*: a logger cost measured with no profiler present
/// is not a number about the configuration a shipped title runs.
fn profiler_only(profiler: &mut Profiler) {
    profiler.arm(ProfilerConfig::default());
    set_target_control(<Log as LogTarget>::ID, TargetControl::OFF);
    assert!(boyko_diag::profiling_abi::arm_mask_bits() != 0, "leg D wants the profiler armed");
    assert_eq!(
        boyko_log::runtime_ceiling(<Log as LogTarget>::ID),
        Level::Off as u8,
        "leg D wants the logger off"
    );
}

fn flags_off(profiler: &mut Profiler) {
    profiler.disarm();
    set_target_control(<Log as LogTarget>::ID, TargetControl::OFF);
    assert_eq!(
        boyko_diag::profiling_abi::arm_mask_bits(),
        0,
        "leg B is specified as ARM_MASK == 0; a residual bit leaves a subsystem half on"
    );
    assert_eq!(
        boyko_log::runtime_ceiling(<Log as LogTarget>::ID),
        Level::Off as u8,
        "leg B is specified as every CONTROL byte Off"
    );
}

fn main() {
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        binary: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    boyko_log::sink::slot::reset();
    let mut profiler = Profiler::new();

    let mut on = Vec::with_capacity(ROUNDS);
    let mut off = Vec::with_capacity(ROUNDS);
    let mut on2 = Vec::with_capacity(ROUNDS);
    let mut ctl = Vec::with_capacity(ROUNDS);
    let mut prof_only = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        // A-B-A': the twin is the same leg measured twice around the other, so a drift that would
        // otherwise look like a difference between flag states shows up as a difference between A
        // and A' instead — where it cannot be mistaken for the result.
        flags_on(&mut profiler);
        on.push(time_block(&mut profiler));
        flags_off(&mut profiler);
        off.push(time_block(&mut profiler));
        flags_on(&mut profiler);
        on2.push(time_block(&mut profiler));
        profiler_only(&mut profiler);
        prof_only.push(time_block(&mut profiler));
        ctl.push(time_empty());
    }
    flags_off(&mut profiler);

    let resolution = resolution_ns(CALLS);
    let (med_on, se_on) = med_and_floor(&mut on, resolution);
    let (med_off, se_off) = med_and_floor(&mut off, resolution);
    let (med_on2, _) = med_and_floor(&mut on2, resolution);
    let (med_ctl, se_ctl) = med_and_floor(&mut ctl, resolution);
    let (med_prof, se_prof) = med_and_floor(&mut prof_only, resolution);

    let twin = (med_on - med_on2).abs();
    let cost = med_on - med_off;

    println!("instrument: resolution {resolution:.4} ns/call over {CALLS}-call blocks");
    println!("GJ1 (A) flags ON               : {med_on:7.2} ns/site  (se {se_on:.2})");
    println!("GJ1 (B) flags OFF, same binary : {med_off:7.2} ns/site  (se {se_off:.2})");
    println!("        zero control           : {med_ctl:7.2} ns/site  (se {se_ctl:.2})");
    println!("        A-vs-A twin gap        : {twin:.3} ns");
    println!("GJ1 (D) profiler ON, logger OFF: {med_prof:7.2} ns/site  (se {se_prof:.2})");
    println!("        A - B                  : {cost:+.2} ns/site");

    // The twin decides whether the sitting measured anything, and it is PROPORTIONAL to the leg
    // rather than to the separation: a drift far below the separation can still be wider than the
    // band a later run would have to fall inside.
    const MAX_TWIN_DRIFT: f64 = 0.02;
    if twin > med_on * MAX_TWIN_DRIFT {
        println!("  verdict: NOT MEASURABLE (instrument): the A-vs-A twin drifted over 2% of the leg");
        return;
    }
    // The control must resolve apart from the OFF leg, or the instrument saw nothing. This is the
    // same shape as GJ1's own "control inert" red, applied to the leg this bench actually has.
    if (med_off - med_ctl).abs() < (se_off + se_ctl) {
        println!(
            "  verdict: NOT RESOLVED (control inert): the flag-off body is indistinguishable from \
             an empty loop, so this sitting could not have seen the flags either"
        );
        return;
    }
    // ── THE FLAG-OFF RESIDUAL IS BOUNDED, AND GJ1's SECOND RED IS WHY THIS EXISTS ──────────
    //
    // GJ1 specifies: "delete the runtime gate from the emission macros so B becomes the same code
    // as A => B collapses onto A and (A vs B) stops resolving." **Run against a JOINT body, that
    // red only half fires.** Deleting the LOGGER's gate leaves the PROFILER's in place, so B rose
    // from 3.12 to 12.11 ns and the pair went on resolving at 16.02 ns — a green verdict over a
    // subsystem whose runtime gate had been deleted. The specified red assumes a body with one
    // gate; rung 16's body has two, which is the entire point of the joint rung.
    //
    // So the residual is bounded as well as the saving. The form is scale-free rather than an
    // absolute in nanoseconds: an absolute would be a fact about this box's clock rate wearing a
    // correctness claim, and this bound holds wherever the mechanism does.
    //
    // MEASURED, both directions: gates present -> 2.73 vs a 6.54 allowance; the logger's gate
    // deleted -> 11.72 vs 6.93, which reds.
    const MAX_RESIDUAL_FRACTION: f64 = 0.25;
    let residual = med_off - med_ctl;
    let total = med_on - med_ctl;
    if residual > total * MAX_RESIDUAL_FRACTION {
        println!(
            "  verdict: GATE NOT DOING ITS JOB -- the flag-off leg keeps {residual:.2} ns over the control, more than a quarter of the {total:.2} ns the flags cost. A deleted runtime gate looks exactly like this."
        );
        return;
    }

    // ── `sched_cpu_logger_on_off` (logging P1), from the same sitting ───────────────────────
    //
    // (A - D) is the LOGGER's cost with the profiler present, which is P1's subject stated the way
    // J2 requires it: in the both-present configuration. Reported beside GJ1 rather than in its own
    // bench, because a second sitting is a second drift.
    let logger_cost = med_on - med_prof;
    let logger_floor = se_on + se_prof;
    if logger_cost.abs() < logger_floor {
        println!(
            "  sched_cpu_logger_on_off: NOT RESOLVED -- {logger_cost:+.2} ns is inside the combined floor {logger_floor:.2}"
        );
    } else {
        println!("  sched_cpu_logger_on_off: the logger costs {logger_cost:.2} ns/site with the profiler armed");
    }

    if cost.abs() < (se_on + se_off) {
        println!("  verdict: NOT RESOLVED: A and B are within their combined spread floor");
    } else if cost > 0.0 {
        println!("  verdict: RESOLVED — turning the flags off saves {cost:.2} ns per site");
        println!("           (B still pays the per-site floor; only the compile ceiling removes it,");
        println!("            which `boyko_log/benches/log_gate_cost` measures in its own sitting)");
    } else {
        println!(
            "  verdict: INVERTED — the flag-ON leg is FASTER by {:.2} ns, which no mechanism here \
             explains; read as drift and re-run on an idle box",
            -cost
        );
    }
}
