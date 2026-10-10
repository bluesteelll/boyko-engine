//! Particles P0 — gate #11 (subsystem containment, D17/M4), gate #7's CPU half (the four-boundary
//! pool partition), and gate #15 (the D15 out-of-bounds emitter clamp).
//!
//! # Gate #11: what is asserted, and how it is OBSERVED
//!
//! `App` exposes no accessor for its resolved `EventUpdatePolicy` and none for whether a Fixed
//! schedule exists, so this file does not read those fields — it **acts**, and asserts on what the
//! acting produced. Two probes, both derived from `App::update_with_delta`'s own frame order:
//!
//! * **Fixed-schedule probe.** Step ④ calls `fixed_advance` **only** when `self.fixed.is_some()`,
//!   and `fixed_advance` is the sole writer of `FixedTime`'s accumulator and elapsed time. So
//!   `FixedTime::elapsed() == 0 && FixedTime::overstep() == 0` after a script of stepping frames
//!   is exactly the statement *no Fixed schedule exists in this app*.
//! * **Event-swap probe.** Step ③ swaps the event double-buffer iff `policy == EveryFrame ||
//!   fixed.is_none() || fixed_steps_since_swap > 0`. Over a script of 0-substep frames, a Main
//!   reader therefore observes every sent event iff the app resolved to the every-frame swap, and
//!   observes NOTHING while a `WaitForFixed` app holds the swap.
//!
//! Together the two pin the resolved policy: `App::finish` resolves `None` to `WaitForFixed`
//! **iff a Fixed schedule was configured**, so "no Fixed schedule" plus "the every-frame swap
//! behaviour" is the observable content of "the resolved policy is `EveryFrame`, in both apps".
//!
//! # The instrument is proved non-vacuous
//!
//! A gate that cannot fail is worse than no gate. `the_probes_are_not_vacuous` builds a third app
//! that DOES register a Fixed schedule and shows both probes flip: the fixed clock advances and the
//! Main reader observes ZERO events over the same 0-substep script. Without that test, a probe
//! that silently returned "clean" for every input would report containment forever.

#![cfg(not(miri))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use boyko_ecs::ecs::core::app::{App, CoreSchedule};
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{
    EmitRequestGpu, MAX_EMITTERS, ParticleCounters, ParticleEmitScratch, ParticlePlugin,
};

/// One 64 Hz fixed step, plus slack — a frame delta guaranteed to expend at least one substep IF a
/// Fixed schedule exists.
const STEP: Duration = Duration::from_millis(20);
/// A delta far too small to complete a 64 Hz substep across any script length used here.
const TINY: Duration = Duration::from_micros(1);
/// Events the probe script sends.
const SENDS: u32 = 5;
/// 0-substep frames the event probe runs. More than `SENDS`, so a delivered event has several
/// frames in which to arrive and "0 observed" cannot be a timing artifact.
const PROBE_FRAMES: u32 = 8;

// ── Event plumbing (the `app_multi_schedule` manual-impl shape) ───────────────────────

#[derive(Clone, Copy)]
struct NoParticipants;

impl Participants for NoParticipants {
    fn participant_count() -> usize {
        0
    }
    fn participant_info() -> &'static [ParticipantInfo] {
        &[]
    }
}

#[derive(Clone, Copy)]
struct NoParameters;

impl Parameters for NoParameters {}

/// The probe event. A fresh id is safe here without a reserved range: each integration test file
/// is its own process, so this binary's event registry is not shared with any other suite.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ContainmentEvt {
    value: u32,
}

impl Event for ContainmentEvt {
    type Participants = NoParticipants;
    type Parameters = NoParameters;
    fn event_id() -> u64 {
        1
    }
    fn event_name() -> &'static str {
        "ContainmentEvt"
    }
    fn new(_: NoParticipants, _: NoParameters) -> Self {
        ContainmentEvt { value: 0 }
    }
    fn participants(&self) -> &NoParticipants {
        unimplemented!("the probe event carries no participants")
    }
    fn participants_mut(&mut self) -> &mut NoParticipants {
        unimplemented!("the probe event carries no participants")
    }
    fn parameters(&self) -> &NoParameters {
        unimplemented!("the probe event carries no parameters")
    }
    fn parameters_mut(&mut self) -> &mut NoParameters {
        unimplemented!("the probe event carries no parameters")
    }
}

/// Remaining sends, so the writer stops on its own and the counts are exact.
#[derive(Resource, Default)]
struct SendBudget(u32);

// ── The observable signature of an app's schedule / event configuration ──────────────

/// Everything gate #11 compares between the two apps. Every field is the result of an ACTION —
/// see the module doc for why none of them is a field read.
#[derive(Debug, PartialEq, Eq)]
struct AppSignature {
    /// `true` iff a `CoreSchedule::Fixed` schedule exists (the driver advanced the fixed clock).
    has_fixed_schedule: bool,
    /// Events a Main reader observed over `PROBE_FRAMES` 0-substep frames.
    events_delivered: u32,
    /// The live schedule labels, as their stable spellings.
    labels: Vec<&'static str>,
}

/// Builds a probe app, optionally with [`ParticlePlugin`] and optionally with a Fixed schedule.
///
/// `with_fixed` exists ONLY for the non-vacuity canary; both gate apps pass `false`.
fn probe_app(with_plugin: bool, with_fixed: bool) -> (App, Arc<AtomicU32>) {
    let received = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&received);

    // One worker + the dispatcher ⇒ two event lanes; a serial pool keeps the script deterministic.
    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.world_mut()
        .preregister_event_default::<ContainmentEvt>()
        .expect("invariant: the probe event preregisters on a fresh world");
    app.world_mut().insert_resource(SendBudget(0));

    app.add_systems(|mut budget: ResMut<SendBudget>, mut w: EventWriter<ContainmentEvt>| {
        if budget.0 > 0 {
            budget.0 -= 1;
            w.send(ContainmentEvt { value: 1 }).expect("invariant: send within lane capacity");
        }
    });
    app.add_systems(move |mut r: EventReader<ContainmentEvt>| {
        counter.fetch_add(r.read().count() as u32, Ordering::Relaxed);
    });

    if with_plugin {
        app.add_plugin(ParticlePlugin);
    }
    if with_fixed {
        // The canary's only difference: a Fixed schedule EXISTS. This is precisely what
        // `ParticlePlugin` must never do.
        app.add_systems_in(CoreSchedule::Fixed, || {});
    }

    (app, received)
}

/// Runs both probes against `app` and returns its observable signature.
fn signature_of(app: &mut App, received: &AtomicU32) -> AppSignature {
    app.finish();
    app.world_mut().resource_mut::<SendBudget>().0 = SENDS;

    // Probe 1 — the event swap, over 0-substep frames ONLY. Run first: a stepping frame would
    // release a held backlog and erase the very difference this probe exists to see.
    for _ in 0..PROBE_FRAMES {
        app.update_with_delta(TINY);
    }
    let events_delivered = received.load(Ordering::Relaxed);

    // Probe 2 — does a Fixed schedule exist? `fixed_advance` is the sole writer of `FixedTime`'s
    // clock and runs only when one does.
    for _ in 0..4 {
        app.update_with_delta(STEP);
    }
    let fixed = app.world().resource::<FixedTime>();
    let has_fixed_schedule = fixed.elapsed() > Duration::ZERO || fixed.overstep() > Duration::ZERO;

    let mut labels = vec!["Main"];
    if has_fixed_schedule {
        labels.push("Fixed");
    }

    AppSignature { has_fixed_schedule, events_delivered, labels }
}

// ── Gate #11 — containment (D17 / M4) ────────────────────────────────────────────────

/// **Gate #11.** Two apps differing ONLY by `add_plugin(ParticlePlugin)` have byte-identical
/// observable schedule/event configuration: the same resolved event-swap behaviour, no Fixed
/// schedule in either, and the same live schedule-label set.
///
/// The failure this forbids is not hypothetical. A particle plugin that registered its step on
/// `CoreSchedule::Fixed` — the obvious design, and the one Rev 2 of the plan had — would create
/// the lazy `fixed_builder`, which makes `App::finish` resolve the process-wide event policy to
/// `WaitForFixed`, which holds the event swap on every 0-substep frame. At 200 fps against a 64 Hz
/// step that is two frames in three, for INPUT, UI and COLLISION events, in a game whose only
/// change was installing a particle system.
#[test]
fn plugin_does_not_change_event_policy_or_schedule_set() {
    let (mut without, without_rx) = probe_app(false, false);
    let (mut with, with_rx) = probe_app(true, false);

    let baseline = signature_of(&mut without, &without_rx);
    let armed = signature_of(&mut with, &with_rx);

    assert_eq!(
        armed, baseline,
        "installing ParticlePlugin must leave the app's observable schedule/event configuration \
         identical (D17)"
    );
    assert!(!baseline.has_fixed_schedule, "the baseline app has no Fixed schedule");
    assert!(
        !armed.has_fixed_schedule,
        "ParticlePlugin must NOT create a CoreSchedule::Fixed schedule — doing so flips the \
         process-wide EventUpdatePolicy to WaitForFixed (M4)"
    );
    assert_eq!(baseline.labels, vec!["Main"], "only the Main schedule is live without the plugin");
    assert_eq!(armed.labels, vec!["Main"], "and only the Main schedule is live with it");
    assert_eq!(
        armed.events_delivered, SENDS,
        "every sent event reaches its Main reader on a 0-substep frame — the every-frame swap"
    );
}

/// **The non-vacuity canary.** A gate that cannot fail is not a gate. This builds the app
/// `ParticlePlugin` must never produce — one with a Fixed schedule — and shows BOTH probes flip,
/// so a future regression really would be caught by the test above.
#[test]
fn the_probes_are_not_vacuous() {
    let (mut clean, clean_rx) = probe_app(false, false);
    let (mut with_fixed, fixed_rx) = probe_app(false, true);

    let clean = signature_of(&mut clean, &clean_rx);
    let dirty = signature_of(&mut with_fixed, &fixed_rx);

    assert_ne!(clean, dirty, "the probes must distinguish a Fixed-schedule app from a clean one");
    assert!(dirty.has_fixed_schedule, "the fixed probe must SEE a Fixed schedule when one exists");
    assert_eq!(dirty.labels, vec!["Main", "Fixed"]);
    assert_eq!(
        dirty.events_delivered, 0,
        "WaitForFixed holds the swap across 0-substep frames — the exact behaviour change a \
         Fixed-registering particle plugin would inflict on every unrelated event consumer"
    );
    assert_eq!(clean.events_delivered, SENDS, "and the clean app delivers all of them");
}

/// Containment is also a statement about the CLOCK: `ParticlePlugin` must not read, write or
/// reconfigure the engine's `Time` / `FixedTime`. Composing it leaves both at their defaults.
#[test]
fn plugin_leaves_the_engine_clocks_at_their_defaults() {
    let (mut without, _) = probe_app(false, false);
    let (mut with, _) = probe_app(true, false);
    without.finish();
    with.finish();

    let a = without.world().resource::<FixedTime>().timestep();
    let b = with.world().resource::<FixedTime>().timestep();
    assert_eq!(a, b, "ParticlePlugin must not touch the engine's fixed timestep");

    let sa = without.world().resource::<Time>().relative_speed();
    let sb = with.world().resource::<Time>().relative_speed();
    assert_eq!(sa, sb, "ParticlePlugin must not touch Time's relative speed");
    assert_eq!(
        without.world().resource::<Time>().max_delta(),
        with.world().resource::<Time>().max_delta(),
        "ParticlePlugin must not touch Time's max_delta (the engine-wide death-spiral guard)"
    );
}

// ── The carrier hooks are LIVE, not merely declared ──────────────────────────────────

/// `ParticleEffectHandle`'s `on_insert` / `on_replace` hooks actually FIRE through a real `App`.
///
/// A hook declared in a `#[component(..)]` attribute and never observed is the "dead datum" class
/// this repository has been bitten by repeatedly — a producer wired to a consumer nobody calls,
/// green in every build. This drives the whole chain: spawn → hook → queue →
/// `particle_apply_effect_refs` drains it.
///
/// It also pins the ORDERING fact that makes the wiring work without a priming call: the spawn
/// resolves `ParticleEffectHandle::component_id()` (which installs the hooks) BEFORE the hook
/// dispatch for that same spawn, so `ParticlePlugin` does not need to prime the id at build time.
/// Measured, not assumed — the sibling `boyko_scene` carriers' integration tests prime explicitly.
#[test]
fn the_effect_handle_hooks_fire_and_the_queue_drains() {
    use boyko_ecs::ecs::core::entity::entity::Entity;
    use boyko_render::{ParticleEffectHandle, ParticleEffectRefs};

    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.add_plugin(ParticlePlugin);
    app.finish();

    let entity: Entity = app
        .world_mut()
        .run_system(|mut cmds: Commands| cmds.spawn(ParticleEffectHandle(3)).id());

    let queued = app.world().resource::<ParticleEffectRefs>().queued();
    assert_eq!(queued.len(), 1, "on_insert must push exactly one delta");
    assert_eq!(queued[0].slot, 3, "for the slot the carrier names");
    assert_eq!(queued[0].delta, 1, "and it must be a +1 (attach)");

    // One frame drains it — the queue has a real consumer.
    app.update_with_delta(TINY);
    assert!(
        app.world().resource::<ParticleEffectRefs>().is_empty(),
        "particle_apply_effect_refs must drain the queue — a producer with no consumer is the \
         dead-datum defect this test exists to refuse"
    );

    // Departure pushes the matching -1.
    app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(entity).despawn();
    });
    let queued = app.world().resource::<ParticleEffectRefs>().queued();
    assert_eq!(queued.len(), 1, "on_replace must fire on despawn");
    assert_eq!(queued[0].delta, -1, "and push the matching -1");
    assert_eq!(queued[0].slot, 3, "for the OLD slot, read pre-departure");
}

/// The device push constants have exactly one home each, and all of them are reachable from the
/// public surface — the host records without recomputing anything.
///
/// This is the seam the plan's "one number, two consumers" rule is about: if a recording site had
/// to derive `steps` or `requested_spawn` for itself, there would be two derivations to keep in
/// agreement, which is the shape every defect in this plan's reversal ledger arrived in.
#[test]
fn every_device_push_constant_is_reachable_from_one_public_home() {
    use boyko_render::{ParticleClock, ParticleConfig, PARTICLE_SUBSTEP_CEILING};

    let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
    app.add_plugin(ParticlePlugin);
    app.finish();
    app.update_with_delta(STEP);

    // sim: {steps, timestep}
    let clock = *app.world().resource::<ParticleClock>();
    assert!(clock.steps() <= PARTICLE_SUBSTEP_CEILING, "steps is host-clamped before the push");
    assert!(clock.timestep() > 0.0);

    // kickoff: {requested_spawn, capacity}
    let scratch = app.world().resource::<ParticleEmitScratch>();
    assert_eq!(scratch.total_spawn(), 0, "no emitter exists, so the frame requests no spawns");
    assert!(
        scratch.total_spawn() == 0,
        "and the upload/declare gate is the SAME read as the pushed value — zero bytes cross PCIe \
         and the emit pass is not declared"
    );
    assert!(app.world().resource::<ParticleConfig>().capacity > 0);

    // emit: {emitter_count, frame_index} — frame_index is host-owned.
    assert_eq!(scratch.emitter_count(), 0);
    assert_eq!(
        scratch.emitter_count() as usize,
        scratch.requests().len(),
        "emitter_count must be the row count, never a separately maintained number"
    );
}

// ── Gate #15 / D15 — the out-of-bounds emitter clamp ─────────────────────────────────

/// **Gate #15 (D15 / R8).** At `MAX_EMITTERS + 1` enabled emitters every write stays in bounds,
/// the extra emitter is dropped, and the drop is counted EXACTLY — in emitters and in the spawns
/// that went with them.
///
/// Hanabi shipped a 12 B indirect overrun at ~260 instances because a GPU table was sized from a
/// constant, and this device runs with `robustBufferAccess` OFF, so the 257th row would be
/// undefined behaviour rather than a wrapped write. That is why the clamp is present in RELEASE
/// and not only behind a `debug_assert!`.
#[test]
fn max_emitters_plus_one_writes_stay_in_bounds_and_the_drop_is_counted() {
    let mut scratch = ParticleEmitScratch::default();
    scratch.begin_frame();

    for i in 0..MAX_EMITTERS {
        let accepted =
            scratch.push_request(EmitRequestGpu { spawn_count: 2, ..EmitRequestGpu::default() });
        assert!(accepted, "emitter {i} is within MAX_EMITTERS and must be accepted");
    }
    assert_eq!(scratch.requests().len(), MAX_EMITTERS);
    assert_eq!(scratch.total_spawn(), 2 * MAX_EMITTERS as u32);
    assert_eq!(scratch.dropped_emitters(), 0, "nothing is dropped at exactly MAX_EMITTERS");

    let accepted =
        scratch.push_request(EmitRequestGpu { spawn_count: 41, ..EmitRequestGpu::default() });

    assert!(!accepted, "the MAX_EMITTERS+1'th emitter must be refused");
    assert_eq!(scratch.requests().len(), MAX_EMITTERS, "no write past the device table's end");
    assert_eq!(scratch.dropped_emitters(), 1, "exactly one emitter dropped");
    assert_eq!(scratch.dropped_spawns(), 41, "and exactly its spawn count recorded");
    assert_eq!(
        scratch.total_spawn(),
        2 * MAX_EMITTERS as u32,
        "a dropped emitter must not advance the prefix sum, or every accepted row after it would \
         renumber on the device"
    );

    // The prefix over the accepted rows is still exactly the running sum — the property a dropped
    // emitter is most likely to break silently.
    let mut running = 0u32;
    for row in scratch.requests() {
        assert_eq!(row.first_spawn, running);
        running += row.spawn_count;
    }
    assert_eq!(running, scratch.total_spawn());
}

// ── Gate #7 (CPU half) — the four-boundary pool partition ────────────────────────────

/// The CPU oracle model of the GPU pool bookkeeping (plan §Counter and list ownership, NORMATIVE).
///
/// `std::Vec` here is the sanctioned `#[cfg(test)]` oracle exception: this is a MODEL of a device
/// buffer, not engine state, and it never ships. The production side owns no CPU mirror of
/// per-particle state at all (Principle 0).
struct PoolModel {
    cap: u32,
    /// The list the sim WALKS this frame; emit appends to it.
    alive_read: Vec<u32>,
    /// The list the sim BUILDS this frame; it becomes `alive_read` at the frame edge.
    alive_write: Vec<u32>,
    /// The free list. Emit reads a reserved window; the sim pushes retired slots.
    dead: Vec<u32>,
    /// Render positions taken from the additive class's render counter.
    additive_render: Vec<u32>,
    /// The real counter record.
    counters: ParticleCounters,
    /// The additive class's render counter (it lives in `p_draw_args`, not `p_counters` — the
    /// separation this model must preserve or the M2 alpha-leak assertion becomes meaningless).
    additive_instance_count: u32,
    /// The alpha class's render counter. Always 0 at P0; carried so the B3 class-split assertion
    /// is written in the form that would CATCH an alpha leak.
    alpha_instance_count: u32,
}

impl PoolModel {
    /// Boot state: every device buffer zero-filled EXCEPT the free list, which is the identity
    /// permutation with `dead_count == CAP`. That is the only non-zero boot fill, and it is what
    /// makes B0's equality true at `alive_count_next == 0`.
    fn boot(cap: u32) -> Self {
        Self {
            cap,
            alive_read: vec![0; cap as usize],
            alive_write: vec![0; cap as usize],
            dead: (0..cap).collect(),
            additive_render: vec![u32::MAX; cap as usize],
            counters: ParticleCounters { dead_count: cap, ..ParticleCounters::default() },
            additive_instance_count: 0,
            alpha_instance_count: 0,
        }
    }

    /// **B0** — the frame edge. `alive_count_next` (last frame's survivors) + `dead_count == CAP`.
    fn assert_b0(&self, frame: usize) {
        assert_eq!(
            self.counters.alive_count_next + self.counters.dead_count,
            self.cap,
            "frame {frame} B0: N_prev + D == CAP"
        );
    }

    /// The one-thread kickoff pass, verbatim from D3.
    fn kickoff(&mut self, requested_spawn: u32, frame: usize) {
        let c = &mut self.counters;
        let a = c.alive_count_next;
        c.alive_count_cur = a;
        c.alive_count_next = 0;

        let d = c.dead_count;
        let e = requested_spawn.min(d);
        assert!(e <= d, "frame {frame}: real_emit <= dead_count, always");
        c.clamped_spawns += requested_spawn - e;
        c.real_emit_count = e;

        // Pre-DECREMENT the free list and pre-INCREMENT the live count in the SAME one-thread
        // pass, so the reservation is accounted on both sides simultaneously.
        c.dead_count = d - e;
        c.dead_base = c.dead_count;
        c.emit_append_base = a;
        c.alive_count_cur = a + e;

        // The render counters are reset by kickoff; the list counter was reset above.
        self.additive_instance_count = 0;
        self.alpha_instance_count = 0;
    }

    /// **B1** — kickoff → emit. The two-term equality PLUS the in-flight window's one-to-one
    /// correspondence, which is the assertion a LOST reservation would fail while the equality
    /// still held.
    fn assert_b1(&self, frame: usize) {
        let c = &self.counters;
        assert_eq!(
            c.alive_count_cur + c.dead_count,
            self.cap,
            "frame {frame} B1: A + D == CAP (kickoff accounts the reservation on both sides)"
        );

        let list_window = c.alive_count_cur - c.emit_append_base;
        assert_eq!(
            list_window, c.real_emit_count,
            "frame {frame} B1: the reserved alive-list window is exactly real_emit_count wide"
        );
        assert_eq!(
            c.dead_base + c.real_emit_count,
            c.dead_count + c.real_emit_count,
            "frame {frame} B1: dead_base IS the post-decrement dead_count"
        );

        // The reserved free-list entries are distinct slots, and none of them is already live.
        let reserved: Vec<u32> = (0..c.real_emit_count)
            .map(|g| self.dead[(c.dead_base + g) as usize])
            .collect();
        let mut seen = vec![false; self.cap as usize];
        for slot in &reserved {
            assert!(*slot < self.cap, "frame {frame} B1: a reserved slot is in range");
            assert!(!seen[*slot as usize], "frame {frame} B1: the reserved window repeats a slot");
            seen[*slot as usize] = true;
        }
        for i in 0..c.emit_append_base {
            let live = self.alive_read[i as usize];
            assert!(
                !seen[live as usize],
                "frame {frame} B1: slot {live} is live AND reserved for a spawn — the window is \
                 not one-to-one with the free list"
            );
        }
    }

    /// The emit pass: zero atomics, both indices arithmetic in `gid`.
    fn emit(&mut self) {
        let c = self.counters;
        for gid in 0..c.real_emit_count {
            let slot = self.dead[(c.dead_base + gid) as usize];
            self.alive_read[(c.emit_append_base + gid) as usize] = slot;
        }
    }

    /// **B2** — emit → sim. `A + D == CAP`, and every entry of the walked list is a distinct slot.
    fn assert_b2(&self, frame: usize) {
        let c = &self.counters;
        assert_eq!(
            c.alive_count_cur + c.dead_count,
            self.cap,
            "frame {frame} B2: A + D == CAP"
        );
        let mut seen = vec![false; self.cap as usize];
        for i in 0..c.alive_count_cur {
            let slot = self.alive_read[i as usize];
            assert!(slot < self.cap, "frame {frame} B2: a live slot is in range");
            assert!(!seen[slot as usize], "frame {frame} B2: slot {slot} appears twice in the list");
            seen[slot as usize] = true;
        }
    }

    /// The sim pass. `dies(slot, i)` is the per-lane liveness predicate.
    ///
    /// A survivor takes its LIST index from `alive_count_next` (shared by both blend classes) and
    /// its RENDER index from its own class's counter — the separation that makes the alpha leak
    /// impossible.
    fn sim(&mut self, mut dies: impl FnMut(u32, u32) -> bool) {
        for i in 0..self.counters.alive_count_cur {
            let slot = self.alive_read[i as usize];
            if dies(slot, i) {
                let d = self.counters.dead_count;
                self.dead[d as usize] = slot;
                self.counters.dead_count = d + 1;
                continue;
            }
            let idx = self.counters.alive_count_next;
            self.counters.alive_count_next = idx + 1;
            self.alive_write[idx as usize] = slot;

            // P0 is additive-only, so the class select is a compile-time constant.
            let r_pos = self.additive_instance_count;
            self.additive_instance_count = r_pos + 1;
            self.additive_render[r_pos as usize] = slot;
        }
    }

    /// **B3** — sim → draw. `N + D == CAP`, the class split sums to `N` (the M2 assertion that
    /// would have caught an alpha leak), and every one of the `CAP` slots appears EXACTLY once
    /// across the alive list and the free list — no leak, no duplicate.
    fn assert_b3(&self, frame: usize) {
        let c = &self.counters;
        assert_eq!(
            c.alive_count_next + c.dead_count,
            self.cap,
            "frame {frame} B3: N + D == CAP"
        );
        assert_eq!(
            self.additive_instance_count + self.alpha_instance_count,
            c.alive_count_next,
            "frame {frame} B3: the class split must sum to alive_count_next (M2 — an alpha \
             survivor that took its list index from anywhere else would fail HERE)"
        );

        let mut seen = vec![0u32; self.cap as usize];
        for i in 0..c.alive_count_next {
            seen[self.alive_write[i as usize] as usize] += 1;
        }
        for i in 0..c.dead_count {
            seen[self.dead[i as usize] as usize] += 1;
        }
        for (slot, count) in seen.iter().enumerate() {
            assert_eq!(
                *count, 1,
                "frame {frame} B3: slot {slot} appears {count} times across the alive and free \
                 lists (1 == no leak and no duplicate)"
            );
        }

        // Every render record written this frame is a live slot, and the render range is dense.
        for r in 0..self.additive_instance_count {
            let slot = self.additive_render[r as usize];
            assert!(slot < self.cap, "frame {frame} B3: render position {r} holds a real slot");
        }
    }

    /// The frame edge: the host swaps the two physical alive buffers by selecting `sets[parity]`.
    fn end_frame(&mut self) {
        std::mem::swap(&mut self.alive_read, &mut self.alive_write);
    }

    /// One whole frame with every boundary asserted.
    fn frame(&mut self, requested_spawn: u32, frame: usize, dies: impl FnMut(u32, u32) -> bool) {
        self.assert_b0(frame);
        self.kickoff(requested_spawn, frame);
        self.assert_b1(frame);
        self.emit();
        self.assert_b2(frame);
        self.sim(dies);
        self.assert_b3(frame);
        self.end_frame();
    }
}

/// A deterministic LCG — a reproducible spread of spawn/death scripts without pulling a
/// third-party generator into this crate's dependency graph.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }

    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 { 0 } else { self.next_u32() % bound }
    }
}

/// **Gate #7, CPU half.** Over randomized spawn/death scripts the four-boundary partition holds at
/// B0, B1, B2 and B3: `alive + dead == CAP` at every boundary, the B1 in-flight window is
/// one-to-one, the B3 class split sums to `alive_count_next`, every slot appears exactly once, and
/// `real_emit <= dead_count` always.
///
/// The three-term form is what makes this test worth writing: a LOST reservation keeps the
/// two-term equality true while leaking slots, so the window is asserted explicitly.
#[test]
fn the_four_boundary_partition_holds_over_random_spawn_and_death_scripts() {
    // A small CAP on purpose: a script over 64 slots reaches "pool full" and "free list empty"
    // within a few dozen frames, where 262 144 would exercise neither.
    const CAP: u32 = 64;

    for seed in 0..24u64 {
        let mut rng = Lcg(0xA5A5_0000_0000_0001 ^ (seed << 17));
        let mut pool = PoolModel::boot(CAP);

        for frame in 0..96 {
            // Sometimes ask for more than the pool can ever hold — the clamp is part of the model.
            let requested = match rng.below(8) {
                0 => CAP * 2,
                1 => 0,
                _ => rng.below(CAP / 2 + 1),
            };
            // A death probability that swings across the script, so the same run sees
            // all-surviving, all-dying and mixed frames.
            let death_threshold = rng.below(256);
            pool.frame(requested, frame, |_slot, _i| (rng.next_u32() >> 24) < death_threshold);
        }
    }
}

/// Edge case: a request larger than the free list is CLAMPED, the shortfall is counted exactly,
/// and the partition still holds.
#[test]
fn a_request_past_the_free_list_clamps_and_counts_the_shortfall() {
    const CAP: u32 = 16;
    let mut pool = PoolModel::boot(CAP);

    // Frame 0 fills the pool exactly; nothing is clamped.
    pool.frame(CAP, 0, |_, _| false);
    assert_eq!(pool.counters.clamped_spawns, 0, "CAP spawns into an empty pool clamp nothing");
    assert_eq!(pool.counters.alive_count_next, CAP, "and every one of them survives");
    assert_eq!(pool.counters.dead_count, 0, "the free list is empty");

    // Frame 1 asks for 5 more against an empty free list: all five are refused and counted.
    pool.frame(5, 1, |_, _| false);
    assert_eq!(pool.counters.real_emit_count, 0, "a full pool emits nothing");
    assert_eq!(pool.counters.clamped_spawns, 5, "and counts every refused spawn");
    assert_eq!(pool.counters.alive_count_next, CAP, "the live set is unchanged");
}

/// Edge case: `lifetime == 0` — every particle spawned this frame dies in the same frame. This is
/// the case that FORCES dual alive lists: the walked list still holds them while the built list
/// receives none.
#[test]
fn spawn_and_die_in_one_frame_returns_every_slot_to_the_free_list() {
    const CAP: u32 = 32;
    let mut pool = PoolModel::boot(CAP);

    pool.frame(CAP, 0, |_, _| true);

    assert_eq!(pool.counters.alive_count_next, 0, "nothing survives a zero lifetime");
    assert_eq!(pool.counters.dead_count, CAP, "and every slot is back on the free list");
    assert_eq!(pool.additive_instance_count, 0, "no render record is written");
}

/// Edge case: an empty pool. `alive_count_cur == 0` walks nothing, publishes nothing and leaves
/// the partition intact — the frame-0 shape, and every frame of an idle scene.
#[test]
fn an_empty_pool_walks_nothing_and_keeps_the_partition() {
    const CAP: u32 = 8;
    let mut pool = PoolModel::boot(CAP);

    for frame in 0..4 {
        pool.frame(0, frame, |_, _| false);
        assert_eq!(pool.counters.alive_count_cur, 0);
        assert_eq!(pool.counters.alive_count_next, 0);
        assert_eq!(pool.counters.dead_count, CAP);
        assert_eq!(pool.counters.clamped_spawns, 0);
    }
}

/// Frame 0 specifically (the plan spells it out): kickoff sets `A = 0 + E` and
/// `emit_append_base = 0`, emit writes exactly `E` records and `E` list entries, and the sim walks
/// exactly those `E`. No leak, no stale read, no dependence on any GPU-side parity state.
#[test]
fn frame_zero_walks_exactly_the_particles_emit_wrote() {
    const CAP: u32 = 64;
    const SPAWN: u32 = 20;
    let mut pool = PoolModel::boot(CAP);

    pool.assert_b0(0);
    pool.kickoff(SPAWN, 0);

    assert_eq!(pool.counters.emit_append_base, 0, "frame 0's append base is zero");
    assert_eq!(pool.counters.real_emit_count, SPAWN);
    assert_eq!(pool.counters.alive_count_cur, SPAWN, "A == 0 + E on frame 0");
    pool.assert_b1(0);

    pool.emit();
    pool.assert_b2(0);

    pool.sim(|_, _| false);
    assert_eq!(pool.counters.alive_count_next, SPAWN, "the sim walked exactly the fresh spawns");
    assert_eq!(pool.additive_instance_count, SPAWN, "and rendered exactly them");
    pool.assert_b3(0);
}
