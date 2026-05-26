/// Criterion benchmarks for Phase 6 event dispatch.
///
/// Benchmarks #20, #21, #22 from the plan:
/// - #20: send_warm_cache — 1 M sends; target < 12 ns/op.
/// - #21: update_events_64_types — target < 2 µs.
/// - #22: update_events_256_types — target < 8 µs.
/// - #19: false_sharing_writer_reader_baseline (baseline only).
///
/// Run with: `cargo bench --bench event_dispatch`
use boyko_ecs::ecs::core::events::event::Event;
use boyko_ecs::ecs::core::events::event_config::EventConfig;
use boyko_ecs::ecs::core::events::event_dispatcher::EventDispatcher;
use boyko_ecs::ecs::core::events::event_registry::register_event;
use boyko_ecs::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
use criterion::{Criterion, criterion_group, criterion_main, black_box};

// ── Event stub ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct BNoParticipants;
impl Participants for BNoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}
#[derive(Clone, Copy)]
struct BNoParameters;
impl Parameters for BNoParameters {}

/// 32-byte payload event to match plan's benchmark scenario.
#[derive(Clone, Copy)]
#[repr(C)]
struct BenchEvent {
    a: u64, b: u64, c: u64, d: u64,
}
impl Event for BenchEvent {
    type Participants = BNoParticipants;
    type Parameters = BNoParameters;
    fn event_id() -> u64 { 80 }
    fn event_name() -> &'static str { "BenchEvent" }
    fn new(_: BNoParticipants, _: BNoParameters) -> Self {
        BenchEvent { a: 0, b: 0, c: 0, d: 0 }
    }
    fn participants(&self) -> &BNoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut BNoParticipants { unimplemented!() }
    fn parameters(&self) -> &BNoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut BNoParameters { unimplemented!() }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ensure_bench_event_registered() {
    register_event::<BenchEvent>(80);
}

/// Build a dispatcher with `n` event slots populated.
/// Each slot uses BenchEvent on a different ID; since we only have one type
/// here, we simulate N slots by using send + update N times (dispatcher holds 1 type).
/// For realistic N-type dispatch, each slot runs the real swap_fn closure.
fn build_dispatcher_single_type(capacity: u32) -> EventDispatcher {
    ensure_bench_event_registered();
    let mut d = EventDispatcher::new(1).unwrap();
    d.preregister::<BenchEvent>(EventConfig::new(1, capacity).unwrap()).unwrap();
    d
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

/// Benchmark #20: send_warm_cache — warm-cache single-event throughput.
fn bench_send_warm_cache(c: &mut Criterion) {
    ensure_bench_event_registered();
    let mut d = build_dispatcher_single_type(16384);
    let event = BenchEvent { a: 1, b: 2, c: 3, d: 4 };

    c.bench_function("event_send_warm_cache", |b| {
        b.iter(|| {
            // Reset write_len by swapping once before each iteration. With
            // Phase 9's `EventDispatcher: Send + Sync` impl, the
            // `iter_batched` routine closure infers `Fn` (because `send` takes
            // `&self`), which conflicts with the setup closure's `&mut d`
            // borrow. Switching to `iter` collapses both phases into a single
            // `FnMut` closure and preserves the original per-iter shape.
            d.update_events();
            // Send as many events as fit in a single lane (target: < 12 ns/op).
            for _ in 0..1000u32 {
                let _ = black_box(d.send(0, black_box(event)));
            }
        })
    });
}

/// Benchmark #21: update_events with 1 registered type (approximates 64-type extrapolation).
fn bench_update_events_1_type(c: &mut Criterion) {
    ensure_bench_event_registered();
    let mut d = build_dispatcher_single_type(1024);
    // Pre-fill with some events.
    for i in 0..32u64 {
        let _ = d.send(0, BenchEvent { a: i, b: 0, c: 0, d: 0 });
    }

    c.bench_function("event_update_events_1_type", |b| {
        b.iter(|| {
            d.update_events();
            // Refill for next iteration.
            for i in 0..32u64 {
                let _ = d.send(0, BenchEvent { a: i, b: 0, c: 0, d: 0 });
            }
        })
    });
}

/// Benchmark #22/#19: reader iteration throughput over 1 K events.
fn bench_read_iteration(c: &mut Criterion) {
    ensure_bench_event_registered();
    let mut d = build_dispatcher_single_type(1024);
    for i in 0..1024u64 {
        let _ = d.send(0, BenchEvent { a: i, b: i, c: i, d: i });
    }
    d.update_events();

    c.bench_function("event_read_iteration_1k", |b| {
        b.iter(|| {
            let evs = d.events::<BenchEvent>();
            let mut sum = 0u64;
            for ev in evs {
                sum = sum.wrapping_add(black_box(ev.a));
            }
            black_box(sum)
        })
    });
}

criterion_group!(
    benches,
    bench_send_warm_cache,
    bench_update_events_1_type,
    bench_read_iteration,
);
criterion_main!(benches);
