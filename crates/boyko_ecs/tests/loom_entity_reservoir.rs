//! EM2′ — loom models of the `EntityReservoir` claim protocol.
//!
//! These models drive the **real** production code (Phase 9.1 lesson C1)
//! through `boyko_ecs::ecs::core::entity::reservoir_loom_exports`, a
//! `cfg(loom)`-only shim whose functions each forward to one production method.
//! Under `--cfg loom`, `entity_reservoir.rs` aliases its `AtomicIsize` /
//! `AtomicUsize` to `loom::sync::atomic`, so loom schedules the genuine
//! `fetch_sub` of a claim, the `fetch_add` of a mint, and the preset load that
//! `EntityCounter::from_ptr` performs. The one transcription is
//! `register_claimed` (steps 3 and 7 of `SpawnAtCommand::apply`); the one piece
//! of modelled non-production code is `forbidden_settle_during_phase`, the
//! negative control of `n1_…`.
//!
//! # What the models check
//!
//! Each model builds a real `EntityMaster` whose recycled stack holds a small,
//! fixed set of entities (`stack`), runs one or two *phases* in which 2–3 loom
//! threads claim from it (one `EntityCounter` per claimer, like one `Commands`
//! per system; the model's main thread is always one of them), and runs the
//! dispatcher's `&mut` window between phases through the real
//! `deallocate_entity` / `allocate_entity_ticketed` / `rewind_allocate`. Every
//! phase is checked for:
//!
//! * **U** — no entity id is handed out twice (panic text `was issued twice`);
//! * **S** — the handed-out `(id, generation)` multiset is exactly the expected
//!   one: the right stack entries, generations carried unchanged, fresh ids
//!   minted in order;
//! * **L** — per claimer, recycled results come before fresh ones (the
//!   EXHAUSTED bit never clears) and recycled ids strictly decrease (LIFO);
//! * **D** — the final raw `free_top` equals `start - recycled - failed`, where
//!   `failed` (the `fetch_sub`s that found the stack empty) lies between the
//!   claimers that provably paid one and those that could have;
//! * **P** — a probed claimer never observes `free_top` rise (F2 monotone);
//! * **W0** — the phase did not change the physical stack length (F2);
//! * **W1** — after the settling `&mut` operation, the physical length, the
//!   claimable length and the settled `free_top` agree (F1), and
//!   `EntityMaster::check_invariants` passes (F3).
//!
//! The build is the debug profile on purpose: the production `debug_assert!`s
//! (F2 in `try_claim_recycled` and `settle`, F3 in `allocate_entity_ticketed`,
//! the refusal panic in `rewind_allocate`) are extra oracles here.
//!
//! Each model also requires a **witness mask**: a set of interleaving classes
//! (which claimer got a given entry, which drift value occurred) that loom must
//! have reached. A model whose exploration collapsed to one schedule, or whose
//! mutation made a class unreachable while breaking no safety invariant (for
//! example a counter that never presets EXHAUSTED), is red. The `model` helper
//! also requires at least 2 executions and refuses `LOOM_MAX_PERMUTATIONS`,
//! `LOOM_MAX_DURATION` and `LOOM_CHECKPOINT_FILE`, which end exploration
//! early without loom's completion. It prints one receipt per model
//! (`--nocapture`): the execution count, the witness mask and the preemption
//! bound in force.
//!
//! # What the models do NOT check
//!
//! * **Atomic `Ordering`.** loom does not model weak-memory reorderings, so
//!   changing any `Ordering` in the protocol leaves every model green. A single
//!   RMW is correct under every ordering anyway.
//! * **The recycled entries themselves.** The `VmColumn` behind the stack is
//!   plain memory, not a `loom::cell::UnsafeCell`, so loom cannot see a write
//!   racing a claim's read. The Miri `claim_race_*` tests in
//!   `entity_reservoir.rs` are the authority for that.
//! * **EM2′-K in production.** No `&mut EntityMaster` runs during a phase here
//!   because the borrow checker forbids it (the counter borrows the master);
//!   in the engine the guarantee is the scheduler's apply-window gate, gated
//!   natively by `tests/em_reservoir_parallel_claims.rs`. `n1_…` only shows
//!   that the atomics do not make EM2′-K unnecessary.
//! * `mint_fresh_batch`, `sort_free_low_ids_first`, `clear`, generation
//!   wrap-around, and the choreography of `EcsMaster::create_entity` around
//!   its ticket (covered natively).
//!
//! # Run
//!
//! ```bash
//! # Measured on the x86_64-pc-windows-msvc host; on Linux the key would be
//! # target."cfg(unix)" (not measured). Debug profile, no preemption bound.
//! # Do not set RUSTFLAGS: it replaces the [target.*] rustflags of
//! # .cargo/config.toml (see the loom_term_list.rs header).
//! cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   test -p boyko-ecs --test loom_entity_reservoir -- --list
//! cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   test -p boyko-ecs --test loom_entity_reservoir -- --test-threads=1 --nocapture
//! ```
//!
//! `--list` must print 8 `models::…: test` lines before a run counts. Without
//! `--cfg loom` this binary contains no tests and prints `running 0 tests`
//! with exit 0: that is by design and is not a pass of anything, and it is also
//! what any `--config` key that fails to reach rustc produces.
//!
//! The file wraps its contents in `#[cfg(loom)] mod models` rather than using a
//! file-level `#![cfg(loom)]`.
//!
//! # Reading, 2026-09-17 (not a pin)
//!
//! Base `d552be05` plus the uncommitted `loom_exports` shim, msvc host, the
//! command above. Execution counts: r1 70, r2 1769, r3 2306, r4 7, r5 49,
//! r6 49, r7 28; every witness mask complete; n1 panicked with
//! `was issued twice`. The counts are reported so a collapse of exploration is
//! visible; the witness masks, not the counts, are the gate.
//!
//! Mutation probes run against the production code at that reading (each
//! applied alone, the file restored byte-identical afterwards):
//!
//! * `fetch_sub` replaced by `load` + `store(r - 1)`: all 7 positive models red
//!   on U.
//! * claim index `r` instead of `r - 1`: all 8 red on the F2 debug assertion in
//!   `try_claim_recycled` (n1 because its panic text changes).
//! * the EXHAUSTED bit never set: r1 and r2 red on D (r2: −3 against a lower
//!   bound of −2).
//! * the preset load replaced by `false`: r1, r2 and r5 red on an incomplete
//!   witness mask, r7 on its exact phase-2 drift. No safety check sees it.
//! * `settle` without `truncate`: r1–r6 red on W1 (r4: physical 3, settled 1),
//!   r7 on its window's physical-length assertion.
//!   `EntityMaster::check_invariants`, which r1–r4 call before W1, passes this
//!   mutation.
//! * `settle` without `.max(0)`: r1–r3 and r5–r7 red on the F2 debug assertion
//!   in `settle`.
//! * `push_free` without `settle`: r5 red on W1. With r5's W1 checks also
//!   removed, phase 2 hands out the live `E(0,2)` and S catches it; the same
//!   holds for `settle` without `truncate`.
//! * a claim that returns generation 0: all 7 red on S.
//! * the Recycled rewind without `unpop`: r6 red (`recycled_entity_count` 0).
//! * the Fresh rewind without `unmint_fresh_mut`: r7 red (`next_entity_id` 3).
//! * the ungated claim minting before it tries the stack: r3 red on S.
//!
//! Exploration probes: `LOOM_MAX_PREEMPTIONS=1` turns n1 red (3 executions, no
//! double issue found) and `=2` keeps it green; `LOOM_MAX_PREEMPTIONS=0` turns
//! all 8 red (r1–r7 on their witness masks); `LOOM_MAX_PERMUTATIONS` set makes
//! `model` refuse to run.

#[cfg(loom)]
mod models {
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicU64 as StdAtomicU64, AtomicUsize as StdAtomicUsize};
    use std::sync::atomic::Ordering as StdOrdering;

    use loom::sync::Arc;
    use loom::thread;

    use boyko_ecs::ecs::core::entity::entity::Entity;
    use boyko_ecs::ecs::core::entity::entity_master::EntityMaster;
    use boyko_ecs::ecs::core::entity::reservoir_loom_exports as rx;
    use boyko_ecs::ecs::identifiers::primitives::EntityId;

    // ── harness ─────────────────────────────────────────────────────────────

    /// Out-of-band record of what exploration reached. Plain `std` atomics:
    /// loom does not see them, and it runs every model thread as a coroutine
    /// on one OS thread, so they are only ever touched from that thread (and
    /// in fact only from the model's main thread).
    #[derive(Default)]
    struct Witness {
        executions: StdAtomicUsize,
        mask: StdAtomicU64,
    }

    impl Witness {
        fn hit(&self, bit: u64) {
            self.mask.fetch_or(bit, StdOrdering::Relaxed);
        }
    }

    /// Runs `f` under loom's default builder (no preemption bound unless
    /// `LOOM_MAX_PREEMPTIONS` is set) and checks the exploration itself: at
    /// least 2 executions, and every bit of `required` reached.
    fn model(name: &'static str, required: u64, f: impl Fn(&Witness) + Send + Sync + 'static) {
        for var in ["LOOM_MAX_PERMUTATIONS", "LOOM_MAX_DURATION", "LOOM_CHECKPOINT_FILE"] {
            assert!(
                std::env::var_os(var).is_none(),
                "{name}: refusing to run with {var} set: it stops exploration early without \
                 loom's completion, so a green would come from partial work"
            );
        }
        let builder = loom::model::Builder::new();
        let witness = StdArc::new(Witness::default());
        let inner = StdArc::clone(&witness);
        builder.check(move || {
            inner.executions.fetch_add(1, StdOrdering::Relaxed);
            f(&inner);
        });
        let executions = witness.executions.load(StdOrdering::Relaxed);
        let mask = witness.mask.load(StdOrdering::Relaxed);
        eprintln!(
            "[loom receipt] {name}: executions={executions} witness_mask={mask:#b} \
             required={required:#b} preemption_bound={:?} max_branches={}",
            builder.preemption_bound, builder.max_branches
        );
        assert!(
            executions >= 2,
            "{name}: loom ran {executions} execution(s); a model with one schedule explores nothing"
        );
        assert_eq!(
            mask & required,
            required,
            "{name}: witness incomplete: reached {mask:#b}, required {required:#b} \
             (an interleaving class the protocol allows was never observed)"
        );
    }

    fn e(id: usize, generation: u32) -> Entity {
        Entity::new(EntityId(id), generation)
    }

    /// Fresh ⇔ generation 0: every fixture entry and every push carries a
    /// generation of at least 1.
    fn is_fresh(x: Entity) -> bool {
        x.generation() == 0
    }

    /// An `EntityMaster` whose recycled stack is `[E(0,g0) … E(n-1,g(n-1))]`
    /// (top = `E(n-1, g(n-1))`), with `next_entity_id == n` and no live entity.
    /// `gens[i] >= 1`; the models use distinct values so a wrong generation
    /// cannot coincide with a right one.
    fn stack(gens: &[u32]) -> EntityMaster {
        let n = gens.len();
        let mut em = EntityMaster::new();
        for i in 0..n {
            let ticket = rx::allocate_ticketed(&mut em);
            let got = ticket.entity();
            assert_eq!(got, e(i, 0), "fixture: fresh allocation {i}");
            rx::register_claimed(&mut em, got);
        }
        for (i, &g) in gens.iter().enumerate() {
            assert!(g >= 1, "fixture: stack generations start at 1");
            for generation in 0..g - 1 {
                assert!(em.deallocate_entity(e(i, generation)), "fixture: despawn {i}");
                let ticket = rx::allocate_ticketed(&mut em);
                assert_eq!(
                    ticket.entity(),
                    e(i, generation + 1),
                    "fixture: the LIFO pop returns the id just pushed, generation bumped"
                );
                rx::register_claimed(&mut em, ticket.entity());
            }
        }
        for (i, &g) in gens.iter().enumerate() {
            assert!(em.deallocate_entity(e(i, g - 1)), "fixture: final despawn {i}");
        }
        em.check_invariants();
        assert_eq!(rx::physical_len(&mut em), n, "fixture: physical stack length");
        assert_eq!(em.recycled_entity_count(), n, "fixture: claimable length");
        assert_eq!(em.free_top_raw(), n as isize, "fixture: raw free_top");
        assert_eq!(em.next_entity_id(), EntityId(n), "fixture: next fresh id");
        assert_eq!(em.entity_count(), 0, "fixture: no live entity");
        em
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        /// One real `EntityCounter`, `claims` gated claims.
        Gated,
        /// As `Gated`, and loads `free_top` before and after each claim.
        GatedProbed,
        /// `claims` ungated claims (`EntityMaster::reserve_entity`).
        Ungated,
        /// The negative control: one `forbidden_settle_during_phase`.
        ForbiddenSettle,
    }

    #[derive(Clone, Copy, Debug)]
    struct Claimer {
        kind: Kind,
        claims: usize,
    }

    const fn gated(claims: usize) -> Claimer {
        Claimer { kind: Kind::Gated, claims }
    }
    const fn gated_probed(claims: usize) -> Claimer {
        Claimer { kind: Kind::GatedProbed, claims }
    }
    const fn ungated(claims: usize) -> Claimer {
        Claimer { kind: Kind::Ungated, claims }
    }
    const FORBIDDEN_SETTLE: Claimer = Claimer { kind: Kind::ForbiddenSettle, claims: 0 };

    /// What one claimer observed, in order.
    struct Record {
        results: Vec<Entity>,
        /// `GatedProbed` only: raw `free_top` before and after each claim.
        probes: Vec<isize>,
    }

    /// The claimer body. It only records; every assertion runs on the model's
    /// main thread once no loom `Arc` is alive.
    fn run_claimer(em: &EntityMaster, c: Claimer) -> Record {
        let mut rec = Record { results: Vec::with_capacity(c.claims), probes: Vec::new() };
        match c.kind {
            Kind::Ungated => {
                for _ in 0..c.claims {
                    rec.results.push(rx::claim_ungated(em));
                }
            }
            Kind::Gated | Kind::GatedProbed => {
                let probed = matches!(c.kind, Kind::GatedProbed);
                // Created on the claimer thread, as `Commands::get_param` runs
                // on the worker: the preset load interleaves with the other
                // claimers.
                let counter = rx::counter(em);
                for _ in 0..c.claims {
                    if probed {
                        rec.probes.push(em.free_top_raw());
                    }
                    rec.results.push(counter.reserve_entity());
                    if probed {
                        rec.probes.push(em.free_top_raw());
                    }
                }
            }
            Kind::ForbiddenSettle => rx::forbidden_settle_during_phase(em),
        }
        rec
    }

    fn payload_text(payload: &(dyn Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_owned()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_owned()
        }
    }

    fn unwrap_claimer(index: usize, result: std::thread::Result<Record>) -> Record {
        match result {
            Ok(rec) => rec,
            Err(payload) => panic!("claimer {index} panicked: {}", payload_text(&*payload)),
        }
    }

    /// One phase: `claimers[0]` runs on the model's main thread, every other
    /// claimer on its own loom thread. A panic inside a claimer is caught on
    /// that claimer's thread (loom's `spawn` does not catch, and a panic that
    /// escapes a coroutine while another still holds a loom `Arc` aborts the
    /// binary instead of reporting); it is re-raised here, after the joins and
    /// after the `Arc` has been unwrapped.
    fn spawn_phase(em: EntityMaster, claimers: &[Claimer]) -> (EntityMaster, Vec<Record>) {
        let em = Arc::new(em);
        let mut handles = Vec::with_capacity(claimers.len() - 1);
        for &c in &claimers[1..] {
            let em = Arc::clone(&em);
            handles.push(thread::spawn(move || {
                catch_unwind(AssertUnwindSafe(|| run_claimer(&em, c)))
            }));
        }
        let main_result = catch_unwind(AssertUnwindSafe(|| run_claimer(&em, claimers[0])));
        let joined: Vec<std::thread::Result<std::thread::Result<Record>>> =
            handles.into_iter().map(|h| h.join()).collect();
        let em = match Arc::try_unwrap(em) {
            Ok(em) => em,
            Err(still_shared) => {
                drop(still_shared);
                panic!("model harness: the EntityMaster is still shared after every join");
            }
        };
        let mut records = Vec::with_capacity(claimers.len());
        records.push(unwrap_claimer(0, main_result));
        for (i, j) in joined.into_iter().enumerate() {
            let inner = j.unwrap_or_else(|p| {
                panic!("claimer {} escaped its own catch: {}", i + 1, payload_text(&*p))
            });
            records.push(unwrap_claimer(i + 1, inner));
        }
        (em, records)
    }

    /// U. Checked first so a double issue is reported as such.
    fn check_unique(tag: &str, records: &[Record]) {
        let mut ids: Vec<usize> =
            records.iter().flat_map(|r| r.results.iter().map(|x| x.id().0)).collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            let all: Vec<&[Entity]> = records.iter().map(|r| r.results.as_slice()).collect();
            panic!("{tag}: U violated: entity id {} was issued twice (per claimer: {all:?})", w[0]);
        }
    }

    /// S.
    fn check_set(tag: &str, records: &[Record], expected: &[(usize, u32)]) {
        let mut got: Vec<(usize, u32)> = records
            .iter()
            .flat_map(|r| r.results.iter().map(|x| (x.id().0, x.generation())))
            .collect();
        got.sort_unstable();
        let mut want = expected.to_vec();
        want.sort_unstable();
        assert_eq!(got, want, "{tag}: S violated: the issued (id, generation) multiset differs");
    }

    /// L.
    fn check_lifo(tag: &str, records: &[Record]) {
        for (i, r) in records.iter().enumerate() {
            let first_fresh = r.results.iter().position(|x| is_fresh(*x)).unwrap_or(r.results.len());
            assert!(
                r.results[first_fresh..].iter().all(|x| is_fresh(*x)),
                "{tag}: L violated: claimer {i} got a recycled entity after a fresh one: {:?}",
                r.results
            );
            assert!(
                r.results[..first_fresh].windows(2).all(|w| w[0].id() > w[1].id()),
                "{tag}: L violated: claimer {i}'s recycled ids do not strictly decrease: {:?}",
                r.results
            );
        }
    }

    /// P: a probed claimer's view of `free_top` never rises (F2 monotone).
    fn check_probes(tag: &str, records: &[Record]) {
        for (i, r) in records.iter().enumerate() {
            assert!(
                r.probes.windows(2).all(|w| w[1] <= w[0]),
                "{tag}: F2 violated: claimer {i} saw free_top rise within the phase: {:?}",
                r.probes
            );
        }
    }

    /// D. A failed `fetch_sub` is always followed by a mint, so `failed <=`
    /// (gated counters with a fresh result) + (ungated fresh results). A gated
    /// counter with a recycled result was preset non-exhausted (the bit never
    /// clears), so if it also has a fresh result it paid exactly one failed
    /// `fetch_sub`; an ungated fresh result always paid one.
    fn check_drift(tag: &str, claimers: &[Claimer], records: &[Record], start_top: isize, end_top: isize) {
        let (mut f, mut b, mut u, mut recycled) = (0isize, 0isize, 0isize, 0isize);
        for (c, r) in claimers.iter().zip(records) {
            let fresh = r.results.iter().filter(|x| is_fresh(**x)).count() as isize;
            let rec = r.results.len() as isize - fresh;
            recycled += rec;
            match c.kind {
                Kind::Gated | Kind::GatedProbed => {
                    if fresh > 0 {
                        f += 1;
                        if rec > 0 {
                            b += 1;
                        }
                    }
                }
                Kind::Ungated => u += fresh,
                Kind::ForbiddenSettle => {}
            }
        }
        let lo = start_top - recycled - (f + u);
        let hi = start_top - recycled - (b + u);
        assert!(
            lo <= end_top && end_top <= hi,
            "{tag}: D violated: free_top ended at {end_top}, outside [{lo}, {hi}] \
             (start {start_top}, recycled {recycled}, f {f}, b {b}, u {u})"
        );
    }

    struct PhaseOut {
        em: EntityMaster,
        records: Vec<Record>,
        /// Raw `free_top` right after the phase (negative drift included).
        end_top: isize,
    }

    /// A checked phase: runs it, then U, S, L, P, D and W0.
    fn run_phase(tag: &str, mut em: EntityMaster, claimers: &[Claimer], expected: &[(usize, u32)]) -> PhaseOut {
        let start_len = rx::physical_len(&mut em);
        let start_top = em.free_top_raw();
        let (mut em, records) = spawn_phase(em, claimers);
        let end_top = em.free_top_raw();
        check_unique(tag, &records);
        check_set(tag, &records, expected);
        check_lifo(tag, &records);
        check_probes(tag, &records);
        check_drift(tag, claimers, &records, start_top, end_top);
        let len = rx::physical_len(&mut em);
        assert_eq!(len, start_len, "{tag}: W0 violated: the phase changed the physical stack length");
        PhaseOut { em, records, end_top }
    }

    /// The apply step for every claimed entity (`SpawnAtCommand::apply`).
    fn register_all(em: &mut EntityMaster, records: &[Record]) {
        for r in records {
            for &x in &r.results {
                rx::register_claimed(em, x);
            }
        }
    }

    /// W1: call after a settling `&mut` operation.
    fn check_settled(tag: &str, em: &mut EntityMaster) {
        let physical = rx::physical_len(em);
        let claimable = em.recycled_entity_count();
        let top = em.free_top_raw();
        assert!(
            top >= 0 && physical == claimable && claimable == top as usize,
            "{tag}: W1/F1 violated: physical_len {physical}, recycled_entity_count {claimable}, \
             settled free_top {top}"
        );
        em.check_invariants();
    }

    fn got(r: &Record, x: Entity) -> bool {
        r.results.contains(&x)
    }

    // ── models ──────────────────────────────────────────────────────────────

    /// Two counters, two claims each, against `[E(0,2), E(1,1)]`.
    ///
    /// Red when: a claim loses an update (U), hands out a wrong entry or
    /// generation (S), a failed claim is not remembered (D), the preset load is
    /// skipped (witness `drift 0` unreachable), or a claim reads above the
    /// physical top (F2 debug assertion).
    #[test]
    fn r1_two_counters_drain_a_two_entry_stack() {
        const MAIN_GOT_E1: u64 = 1 << 0;
        const T_GOT_E1: u64 = 1 << 1;
        const DRIFT_0: u64 = 1 << 2;
        const DRIFT_M1: u64 = 1 << 3;
        const DRIFT_M2: u64 = 1 << 4;
        model("r1", 0b1_1111, |w| {
            let claimers = [gated(2), gated(2)];
            let PhaseOut { mut em, records, end_top } =
                run_phase("r1", stack(&[2, 1]), &claimers, &[(0, 2), (1, 1), (2, 0), (3, 0)]);
            if got(&records[0], e(1, 1)) {
                w.hit(MAIN_GOT_E1);
            }
            if got(&records[1], e(1, 1)) {
                w.hit(T_GOT_E1);
            }
            match end_top {
                0 => w.hit(DRIFT_0),
                -1 => w.hit(DRIFT_M1),
                -2 => w.hit(DRIFT_M2),
                _ => {}
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r1 window", &mut em);
            assert_eq!(em.next_entity_id(), EntityId(4), "r1 window: next fresh id");
            assert_eq!(em.free_top_raw(), 0, "r1 window: settled free_top");
            assert_eq!(rx::physical_len(&mut em), 0, "r1 window: physical length");
            assert_eq!(em.entity_count(), 4, "r1 window: live entities");
        });
    }

    /// Two probed counters, two claims each, against `[E(0,1)]`: whoever claims
    /// the entry pays one failed `fetch_sub`; the other pays one or none.
    ///
    /// Red when: a counter keeps calling `fetch_sub` after a failure (D: drift
    /// −3), the preset load is skipped (witness `drift −1` unreachable), or a
    /// probe sees `free_top` rise.
    #[test]
    fn r2_exhaustion_bit_caps_failing_claims_at_one_per_counter() {
        const MAIN_GOT_E0: u64 = 1 << 0;
        const T_GOT_E0: u64 = 1 << 1;
        const DRIFT_M1: u64 = 1 << 2;
        const DRIFT_M2: u64 = 1 << 3;
        model("r2", 0b1111, |w| {
            let claimers = [gated_probed(2), gated_probed(2)];
            let PhaseOut { mut em, records, end_top } =
                run_phase("r2", stack(&[1]), &claimers, &[(0, 1), (1, 0), (2, 0), (3, 0)]);
            if got(&records[0], e(0, 1)) {
                w.hit(MAIN_GOT_E0);
            }
            if got(&records[1], e(0, 1)) {
                w.hit(T_GOT_E0);
            }
            match end_top {
                -1 => w.hit(DRIFT_M1),
                -2 => w.hit(DRIFT_M2),
                _ => {}
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r2 window", &mut em);
            assert_eq!(em.next_entity_id(), EntityId(4), "r2 window: next fresh id");
            assert_eq!(em.free_top_raw(), 0, "r2 window: settled free_top");
            assert_eq!(rx::physical_len(&mut em), 0, "r2 window: physical length");
        });
    }

    /// One ungated and two gated claimers (one claim each) against
    /// `[E(0,2), E(1,1)]`: exactly one of the three mints, and any of them can
    /// be the one.
    ///
    /// Red when: a claim loses an update (U), or the ungated claim mints before
    /// trying the stack (S: a fresh id is burnt).
    #[test]
    fn r3_gated_and_ungated_claimers_take_each_entry_once() {
        const MAIN_FRESH: u64 = 1 << 0;
        const T1_FRESH: u64 = 1 << 1;
        const T2_FRESH: u64 = 1 << 2;
        model("r3", 0b111, |w| {
            let claimers = [ungated(1), gated(1), gated(1)];
            let PhaseOut { mut em, records, .. } =
                run_phase("r3", stack(&[2, 1]), &claimers, &[(0, 2), (1, 1), (2, 0)]);
            for (i, bit) in [MAIN_FRESH, T1_FRESH, T2_FRESH].into_iter().enumerate() {
                if records[i].results.iter().any(|x| is_fresh(*x)) {
                    w.hit(bit);
                }
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r3 window", &mut em);
            assert_eq!(em.next_entity_id(), EntityId(3), "r3 window: next fresh id");
            assert_eq!(em.free_top_raw(), 0, "r3 window: settled free_top");
            assert_eq!(rx::physical_len(&mut em), 0, "r3 window: physical length");
        });
    }

    /// Two claims against `[E(0,3), E(1,1), E(2,2)]` take exactly the top two;
    /// the window's settle truncates them and the dispatcher pops the rest.
    ///
    /// Red when: a claim loses an update (U), or `settle` does not truncate
    /// (W1: physical length 3, settled length 1).
    #[test]
    fn r4_partial_drain_takes_exactly_the_top() {
        const MAIN_GOT_E2: u64 = 1 << 0;
        const T_GOT_E2: u64 = 1 << 1;
        model("r4", 0b11, |w| {
            let claimers = [gated(1), gated(1)];
            let PhaseOut { mut em, records, end_top } =
                run_phase("r4", stack(&[3, 1, 2]), &claimers, &[(2, 2), (1, 1)]);
            assert_eq!(end_top, 1, "r4: free_top after two successful claims from 3");
            if got(&records[0], e(2, 2)) {
                w.hit(MAIN_GOT_E2);
            }
            if got(&records[1], e(2, 2)) {
                w.hit(T_GOT_E2);
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r4 window", &mut em);
            assert_eq!(rx::physical_len(&mut em), 1, "r4 window: physical length after settle");
            let ticket = rx::allocate_ticketed(&mut em);
            assert_eq!(ticket.entity(), e(0, 3), "r4 window: the dispatcher pops the remaining entry");
            rx::register_claimed(&mut em, ticket.entity());
            check_settled("r4 window after pop", &mut em);
            assert_eq!(rx::physical_len(&mut em), 0, "r4 window: physical length after pop");
            assert_eq!(em.next_entity_id(), EntityId(3), "r4 window: no fresh id minted");
            assert_eq!(em.entity_count(), 3, "r4 window: live entities");
        });
    }

    /// Phase 1 drains `[E(0,2)]` (one claimer mints `E(1,0)`); the window
    /// despawns `E(1,0)`, which settles lazily and pushes `E(1,1)`; phase 2
    /// must hand out `E(1,1)` exactly once and never a live id.
    ///
    /// Red when: `settle` does not truncate, `push_free` does not settle
    /// (W1 or S), or `settle` does not clamp a negative drift (F2 debug
    /// assertion).
    #[test]
    fn r5_lazy_settle_between_phases_never_reissues_a_claim() {
        const P1_DRIFT_0: u64 = 1 << 0;
        const P1_DRIFT_M1: u64 = 1 << 1;
        const P2_MAIN_GOT_E1: u64 = 1 << 2;
        const P2_T2_GOT_E1: u64 = 1 << 3;
        model("r5", 0b1111, |w| {
            let claimers = [gated(1), gated(1)];
            let PhaseOut { mut em, records, end_top } =
                run_phase("r5 phase 1", stack(&[2]), &claimers, &[(0, 2), (1, 0)]);
            match end_top {
                0 => w.hit(P1_DRIFT_0),
                -1 => w.hit(P1_DRIFT_M1),
                _ => {}
            }
            register_all(&mut em, &records);
            assert!(em.deallocate_entity(e(1, 0)), "r5 window: despawn E(1,0)");
            check_settled("r5 window", &mut em);
            assert_eq!(em.recycled_entity_count(), 1, "r5 window: one recycled entry");
            assert_eq!(rx::physical_len(&mut em), 1, "r5 window: physical length");

            let PhaseOut { mut em, records, .. } =
                run_phase("r5 phase 2", em, &claimers, &[(1, 1), (2, 0)]);
            for r in &records {
                for &x in &r.results {
                    assert!(!em.is_entity_valid(x), "r5 phase 2: handed out the live entity {x:?}");
                }
            }
            assert!(!em.is_entity_valid(e(1, 0)), "r5: the despawned handle stays stale");
            if got(&records[0], e(1, 1)) {
                w.hit(P2_MAIN_GOT_E1);
            }
            if got(&records[1], e(1, 1)) {
                w.hit(P2_T2_GOT_E1);
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r5 window 2", &mut em);
            assert_eq!(em.entity_count(), 3, "r5 window 2: live entities");
            assert_eq!(em.next_entity_id(), EntityId(3), "r5 window 2: next fresh id");
        });
    }

    /// Phase 1 claims the top two of `[E(0,3), E(1,1), E(2,2)]`; the window
    /// allocates `E(0,3)` and rewinds it (a rejected create); phase 2 must hand
    /// it out exactly once with its generation, and mint only one fresh id.
    ///
    /// Red when: the Recycled rewind does not restore the entry (window
    /// `recycled == 1`), or restores it with another generation (S).
    #[test]
    fn r6_rejected_create_restores_a_recycled_ticket_once() {
        const P1_MAIN_TOP: u64 = 1 << 0;
        const P1_T_TOP: u64 = 1 << 1;
        const P2_MAIN_GOT_E0: u64 = 1 << 2;
        const P2_T2_GOT_E0: u64 = 1 << 3;
        model("r6", 0b1111, |w| {
            let claimers = [gated(1), gated(1)];
            let PhaseOut { mut em, records, .. } =
                run_phase("r6 phase 1", stack(&[3, 1, 2]), &claimers, &[(2, 2), (1, 1)]);
            if got(&records[0], e(2, 2)) {
                w.hit(P1_MAIN_TOP);
            }
            if got(&records[1], e(2, 2)) {
                w.hit(P1_T_TOP);
            }
            register_all(&mut em, &records);
            let ticket = rx::allocate_ticketed(&mut em);
            assert_eq!(ticket.entity(), e(0, 3), "r6 window: the allocation pops E(0,3)");
            assert!(rx::rewind(&mut em, ticket), "r6 window: the rewind restores the ticket");
            check_settled("r6 window", &mut em);
            assert_eq!(em.recycled_entity_count(), 1, "r6 window: the rewound entry is back");
            assert_eq!(rx::physical_len(&mut em), 1, "r6 window: physical length");
            assert_eq!(em.next_entity_id(), EntityId(3), "r6 window: the rejected create minted nothing");

            let PhaseOut { mut em, records, .. } =
                run_phase("r6 phase 2", em, &claimers, &[(0, 3), (3, 0)]);
            if got(&records[0], e(0, 3)) {
                w.hit(P2_MAIN_GOT_E0);
            }
            if got(&records[1], e(0, 3)) {
                w.hit(P2_T2_GOT_E0);
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r6 window 2", &mut em);
            assert_eq!(em.next_entity_id(), EntityId(4), "r6 window 2: next fresh id");
            assert_eq!(em.entity_count(), 4, "r6 window 2: live entities");
        });
    }

    /// Phase 1 drains `[E(0,2)]`; the window's allocation settles the drift,
    /// finds the stack empty and mints `E(2,0)`, which is rewound; phase 2 (both
    /// presets exhausted) must mint id 2 exactly once, then id 3.
    ///
    /// Red when: the Fresh rewind does not roll the counter back (window
    /// `next == 2`), or `settle` does not clamp a negative drift (F2 debug
    /// assertion).
    #[test]
    fn r7_rejected_create_restores_a_fresh_ticket_once() {
        const P1_DRIFT_0: u64 = 1 << 0;
        const P1_DRIFT_M1: u64 = 1 << 1;
        const P2_MAIN_GOT_2: u64 = 1 << 2;
        const P2_T2_GOT_2: u64 = 1 << 3;
        model("r7", 0b1111, |w| {
            let claimers = [gated(1), gated(1)];
            let PhaseOut { mut em, records, end_top } =
                run_phase("r7 phase 1", stack(&[2]), &claimers, &[(0, 2), (1, 0)]);
            match end_top {
                0 => w.hit(P1_DRIFT_0),
                -1 => w.hit(P1_DRIFT_M1),
                _ => {}
            }
            register_all(&mut em, &records);
            let ticket = rx::allocate_ticketed(&mut em);
            assert_eq!(ticket.entity(), e(2, 0), "r7 window: the allocation mints id 2");
            assert!(rx::rewind(&mut em, ticket), "r7 window: the rewind restores the ticket");
            assert_eq!(em.next_entity_id(), EntityId(2), "r7 window: the counter is rolled back");
            assert_eq!(rx::physical_len(&mut em), 0, "r7 window: physical length");
            assert_eq!(em.free_top_raw(), 0, "r7 window: the drift is settled");
            check_settled("r7 window", &mut em);

            let PhaseOut { mut em, records, end_top } =
                run_phase("r7 phase 2", em, &claimers, &[(2, 0), (3, 0)]);
            assert_eq!(end_top, 0, "r7 phase 2: both presets exhausted, so no fetch_sub ran");
            if got(&records[0], e(2, 0)) {
                w.hit(P2_MAIN_GOT_2);
            }
            if got(&records[1], e(2, 0)) {
                w.hit(P2_T2_GOT_2);
            }
            register_all(&mut em, &records);
            em.check_invariants();
            check_settled("r7 window 2", &mut em);
            assert_eq!(em.next_entity_id(), EntityId(4), "r7 window 2: next fresh id");
            assert_eq!(em.entity_count(), 4, "r7 window 2: live entities");
        });
    }

    /// NEGATIVE CONTROL. `settle`'s clamp-and-store run through the shared
    /// atomic while a counter claims (what EM2′-K forbids) erases a
    /// `fetch_sub`, and the same entry is claimed twice: main loads 2, T claims
    /// `E(1,1)` (2 → 1), main stores 2, T claims `E(1,1)` again. Two
    /// preemptions.
    ///
    /// This model passing means loom found that interleaving and the U oracle
    /// fired. It turns red if exploration is weakened below two preemptions,
    /// if loom stops scheduling the claim's `fetch_sub`, or if U cannot fail.
    #[test]
    #[should_panic(expected = "was issued twice")]
    fn n1_settle_racing_live_claims_issues_an_entity_twice() {
        model("n1", 0, |_w| {
            let (_em, records) = spawn_phase(stack(&[2, 1]), &[FORBIDDEN_SETTLE, gated(2)]);
            check_unique("n1", &records);
        });
    }
}
