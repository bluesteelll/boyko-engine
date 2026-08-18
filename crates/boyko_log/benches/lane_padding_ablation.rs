//! `lane_padding_ablation` — **padded+cached vs padded-only vs neither**.
//!
//! `LogLane` spends 112 bytes of padding and two `Cell<u32>` cursor caches to keep the producer's
//! and the consumer's lines apart. This row asks what that buys.
//!
//! # It has to be two threads, and a single-threaded version would be worse than none
//!
//! False sharing is a phenomenon of two cores writing one cache line. A single-threaded ablation
//! would show the three layouts costing the same, print a verdict, and mean nothing — the vacuous
//! gate this campaign exists to remove. So this bench runs a producer and a consumer on two
//! threads and measures ns per item handed across.
//!
//! # Three layouts, and what each PAIR isolates
//!
//! * **A — padded + cached** (`LogLane`'s real shape): cursors on separate 64-byte lines, and each
//!   side keeps a stale local copy of the other's cursor, refreshed only when it looks full/empty.
//! * **B — padded, no cache**: same lines, but each side loads the other's atomic every iteration.
//!   `A vs B` is what the CURSOR CACHE buys.
//! * **C — cached, no padding**: both cursors in one cache line. `A vs C` is what the PADDING buys.
//!
//! Two pairs, two questions, one sitting. A three-way league table without the pairing would say
//! which layout is fastest without saying which of the two mechanisms made it so.
//!
//! # The replica is a replica, and that is stated rather than glossed
//!
//! These rings are not `LogLane`; they are its cursor protocol with the payload removed, because
//! the payload copy is identical in all three layouts and would dilute every reading with a
//! constant both legs pay. The transfer to the real lane rests on the claim that the cursors are
//! what contend — which is the claim the padding was added for, and is therefore the thing under
//! test rather than an assumption smuggled past it.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::thread;
use std::time::Instant;

#[path = "instrument.rs"]
mod instrument;
use instrument::{med_and_floor, resolution_ns};

/// Items handed across per timed run. Large enough that thread start-up is noise.
const ITEMS: u32 = 400_000;

/// Rounds in the sitting; every round runs A, B, A-twin and C so drift lands on all three.
///
/// MEASURED: 41 rounds is WORSE than 15. A longer sitting drifts further, and the A-vs-A twin
/// rejected three sittings of four at 41 against one of three at 15. The answer does not change
/// between them -- padding resolves at ~0.75-0.89 ns/item either way and the cursor cache resolves
/// at neither -- so the sample is set where valid sittings are commoner, not where the number is
/// nicer. Averaging more rounds does not help when the thing that grows with the rounds is drift.
const ROUNDS: usize = 15;

/// Ring capacity in items. Small enough that the two sides actually meet and contend.
const CAP: u32 = 256;

/// Cursors on separate 64-byte lines — `LogLane`'s shape.
#[repr(C, align(64))]
struct Padded {
    write: AtomicU32,
    _pad0: [u8; 60],
    read: AtomicU32,
    _pad1: [u8; 60],
}

/// Cursors adjacent in ONE line — the layout the padding exists to avoid.
#[repr(C, align(64))]
struct Packed {
    write: AtomicU32,
    read: AtomicU32,
}

/// Run one producer/consumer pass over a cursor pair and return ns per item.
///
/// `cached` selects whether each side keeps a stale local copy of the other's cursor or loads the
/// atomic every iteration. Both sides are treated symmetrically, as they are in `LogLane`.
fn run(items: u32, cached: bool, write: &'static AtomicU32, read: &'static AtomicU32) -> f64 {
    let started = Arc::new(AtomicUsize::new(0));
    let s2 = Arc::clone(&started);

    let consumer = thread::spawn(move || {
        s2.fetch_add(1, Ordering::AcqRel);
        let mut seen = 0u32;
        let mut w_cache = 0u32;
        while seen < items {
            // The cached side refreshes only when its stale copy says empty — which is exactly
            // what `write_cached` does in the real lane.
            let avail = if cached {
                if w_cache == seen {
                    w_cache = write.load(Ordering::Acquire);
                }
                w_cache
            } else {
                write.load(Ordering::Acquire)
            };
            if avail == seen {
                std::hint::spin_loop();
                continue;
            }
            seen += 1;
            read.store(seen, Ordering::Release);
        }
    });

    // Started before the clock, so thread creation is never inside a reading.
    while started.load(Ordering::Acquire) == 0 {
        std::hint::spin_loop();
    }

    let t0 = Instant::now();
    let mut published = 0u32;
    let mut r_cache = 0u32;
    while published < items {
        let free = if cached {
            if published.wrapping_sub(r_cache) >= CAP {
                r_cache = read.load(Ordering::Acquire);
            }
            r_cache
        } else {
            read.load(Ordering::Acquire)
        };
        if published.wrapping_sub(free) >= CAP {
            std::hint::spin_loop();
            continue;
        }
        published += 1;
        write.store(published, Ordering::Release);
        black_box(published);
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(items);
    consumer.join().expect("the consumer thread must not panic");
    ns
}

/// A fresh pair per run, leaked so the consumer closure is `'static`.
///
/// Fresh rather than reset: a reused pair carries the previous run's cache-line state into the
/// next one, and the whole subject here is cache-line state.
fn fresh_padded() -> &'static Padded {
    Box::leak(Box::new(Padded {
        write: AtomicU32::new(0),
        _pad0: [0; 60],
        read: AtomicU32::new(0),
        _pad1: [0; 60],
    }))
}

fn fresh_packed() -> &'static Packed {
    Box::leak(Box::new(Packed { write: AtomicU32::new(0), read: AtomicU32::new(0) }))
}

fn main() {
    let mut a = Vec::with_capacity(ROUNDS);
    let mut b = Vec::with_capacity(ROUNDS);
    let mut a2 = Vec::with_capacity(ROUNDS);
    let mut c = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        let p = fresh_padded();
        a.push(run(ITEMS, true, &p.write, &p.read));
        let p = fresh_padded();
        b.push(run(ITEMS, false, &p.write, &p.read));
        let p = fresh_padded();
        a2.push(run(ITEMS, true, &p.write, &p.read));
        let q = fresh_packed();
        c.push(run(ITEMS, true, &q.write, &q.read));
    }

    let resolution = resolution_ns(ITEMS);
    let (med_a, se_a) = med_and_floor(&mut a, resolution);
    let (med_b, se_b) = med_and_floor(&mut b, resolution);
    let (med_a2, _) = med_and_floor(&mut a2, resolution);
    let (med_c, se_c) = med_and_floor(&mut c, resolution);

    let twin_gap = (med_a - med_a2).abs();

    println!("instrument: resolution {resolution:.6} ns/item over {ITEMS}-item runs");
    println!("A  padded + cached (the real lane) : {med_a:7.3} ns/item  (se {se_a:.4})");
    println!("B  padded, no cursor cache         : {med_b:7.3} ns/item  (se {se_b:.4})");
    println!("C  cached, cursors in ONE line     : {med_c:7.3} ns/item  (se {se_c:.4})");
    println!("  A-vs-A twin gap = {twin_gap:.4} ns");

    // The twin decides whether the sitting measured anything. Two threads on a loaded box drift
    // far more than one, so it is checked before either pair is read.
    if twin_gap > med_a * 0.10 {
        println!("  verdict: NOT MEASURABLE (instrument): the A-vs-A twin drifted over 10% of the leg");
        return;
    }

    // PAIRS, NOT A LEAGUE TABLE. Each pair isolates one mechanism; a ranking of three layouts says
    // which is fastest without saying which mechanism made it so.
    let cache_gain = med_b - med_a;
    let pad_gain = med_c - med_a;
    let pairs = [
        ("cursor cache (A vs B)", cache_gain, se_a + se_b),
        ("padding      (A vs C)", pad_gain, se_a + se_c),
    ];
    for (name, gain, se) in pairs {
        if gain.abs() < se {
            println!("  {name}: NOT RESOLVED -- {gain:+.3} ns/item is inside the combined floor {se:.3}");
        } else if gain > 0.0 {
            println!("  {name}: buys {gain:.3} ns/item ({:.1}%)", gain / med_a * 100.0);
        } else {
            println!(
                "  {name}: COSTS {:.3} ns/item -- the ablated layout is FASTER, which the mechanism does not explain",
                -gain
            );
        }
    }
}
