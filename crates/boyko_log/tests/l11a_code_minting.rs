//! L11a's mint: one index per code, no leaks, no aliasing, and a sentinel that is never an index.
//!
//! **Its own integration binary, and forced rather than tidy.** `CODE_OCCUPANCY` is process-global
//! and insert-only. The leak claim is `occupancy grew by EXACTLY ONE`, which is only checkable when
//! nothing else in the process is minting — and `cargo test` runs a binary's tests CONCURRENTLY.
//! The first draft lived in `codes.rs`'s `#[cfg(test)] mod` beside two sibling tests that also
//! mint, and it failed on exactly that: a leak assertion racing the neighbours it shares a counter
//! with. Same shape, same reason, same remedy as `l10_dynamic_targets.rs`.

use std::sync::atomic::AtomicU16;

use boyko_log::codes::{
    CODE_IDX_EXHAUSTED, CodeIdx, DOWNSTREAM_IDX_BASE, E0115, MAX_CODES, W0114, code_occupancy,
    code_space_nearly_full, resolve_idx,
};
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log};

/// One `#[test]`, for the reason the module header gives: every claim reads one global counter.
#[test]
fn the_mint_hands_out_one_index_per_code_and_never_aliases() {
    // A real manual file sink, because `W0114`/`E0115` are asserted off DISK below and a default
    // run leaves every ceiling `Off` -- which is exactly why this is a test and not a run.
    let path = std::env::temp_dir().join("boyko_l11a_code_minting.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    // ── an engine index is its ROW, and costs no mint ────────────────────────────────────────
    //
    // The L11a invariant in one line: nothing a downstream crate does can move an engine code's
    // index, because the `Static` arm never consults the counter.
    let before = code_occupancy();
    assert_eq!(resolve_idx(CodeIdx::Static(7)), 7);
    assert_eq!(code_occupancy(), before, "a Static arm must not spend downstream budget");

    // ── SIXTEEN THREADS, ONE CODE: exactly one index, and NONE LEAKED ────────────────────────
    //
    // The leak half is what a `fetch_add`-per-caller design fails silently: every racer takes a
    // slot, all but one are abandoned, and nothing notices until the space is spent 16x early. So
    // occupancy must move by EXACTLY ONE -- "non-zero" would pass on the broken design.
    static CELL: AtomicU16 = AtomicU16::new(0);
    let before = code_occupancy();
    let seen: Vec<u32> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..16).map(|_| s.spawn(|| resolve_idx(CodeIdx::Dynamic(&CELL)))).collect();
        hs.into_iter().map(|h| h.join().expect("no minter panics")).collect()
    });
    let first = seen[0];
    assert!(seen.iter().all(|&i| i == first), "racers disagreed on the index: {seen:?}");
    assert_eq!(
        code_occupancy(),
        before + 1,
        "sixteen racers consumed {} indices for ONE code -- a mint that leaks spends the space \
         sixteen times faster than the census reports",
        code_occupancy() - before
    );
    assert!(first >= u32::from(DOWNSTREAM_IDX_BASE), "a downstream index sits above the engine rows");
    assert!(first < u32::from(MAX_CODES), "and inside the slot array");

    // ── a second code gets a DIFFERENT slot ──────────────────────────────────────────────────
    //
    // Aliasing is what `fetch_add(1) % MAX_CODES` introduces, and it is worse than exhaustion: a
    // rate limiter throttling an unrelated code's storm, with nothing reporting it.
    static B: AtomicU16 = AtomicU16::new(0);
    let ib = resolve_idx(CodeIdx::Dynamic(&B));
    assert_ne!(ib, first, "two codes resolved to one rate slot");
    assert_eq!(resolve_idx(CodeIdx::Dynamic(&CELL)), first, "re-resolving must not mint again");

    // ── EXHAUSTION returns the sentinel, and the sentinel is NEVER an index ──────────────────
    //
    // Driven by real mints rather than by poking the counter, so the boundary arithmetic is the
    // one production takes. Every index handed out before the wall must still be distinct.
    let capacity = MAX_CODES - DOWNSTREAM_IDX_BASE;
    // `Box::leak` and nothing else: a cell handed to `CodeIdx::Dynamic` must be `&'static`, and the
    // first draft leaked one and then rebuilt a `Box` from the same pointer -- two owners for one
    // allocation, in a test whose whole subject is not handing one thing out twice.
    let mut handed: Vec<u32> = Vec::new();
    while code_occupancy() < capacity {
        let cell: &'static AtomicU16 = Box::leak(Box::new(AtomicU16::new(0)));
        let idx = resolve_idx(CodeIdx::Dynamic(cell));
        if idx != CODE_IDX_EXHAUSTED {
            handed.push(idx);
        }
    }
    let mut sorted = handed.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), handed.len(), "the mint handed the same slot to two codes");

    // ── W0114 AND E0115 REACH A READER ───────────────────────────────────────────────────────
    //
    // The observers, and they are not an extra: the exhaustion above already drives both
    // conditions, and a code whose emission nothing reads is this campaign's signature defect.
    // Read back off a real manual file sink rather than inferred from the sentinel.
    assert!(code_space_nearly_full(), "the fill above must have crossed the 90 % threshold");
    let text = sink_text(&path);
    let w0114 = format!("boyko-W{:04}", W0114.number());
    assert!(
        text.contains(&w0114),
        "crossing 90 % emitted no {w0114} -- the threshold report nobody reads: {text:?}"
    );

    // THREE failed mints, not one. `Once` suppresses the SECOND and later reports, so a test that
    // exhausts the space exactly once cannot tell a latch from no latch -- the first draft of this
    // block did exactly that, and deleting the latch left it green.
    static PAST: AtomicU16 = AtomicU16::new(0);
    static PAST2: AtomicU16 = AtomicU16::new(0);
    static PAST3: AtomicU16 = AtomicU16::new(0);
    assert_eq!(resolve_idx(CodeIdx::Dynamic(&PAST2)), CODE_IDX_EXHAUSTED);
    assert_eq!(resolve_idx(CodeIdx::Dynamic(&PAST3)), CODE_IDX_EXHAUSTED);
    assert_eq!(
        resolve_idx(CodeIdx::Dynamic(&PAST)),
        CODE_IDX_EXHAUSTED,
        "past the space the mint must return the SENTINEL, never wrap into an occupied slot"
    );

    let text = sink_text(&path);
    let e0115 = format!("boyko-E{:04}", E0115.number());
    assert!(text.contains(&e0115), "exhaustion emitted no {e0115}: {text:?}");
    assert_eq!(
        text.matches(&e0115).count(),
        1,
        "{e0115} is `Once`; past exhaustion EVERY later mint fails, and reporting each one turns          one budget problem into a storm of reports about it"
    );
}

/// Everything the sink has been given so far, drained and read back off disk.
fn sink_text(path: &std::path::Path) -> String {
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    std::fs::read_to_string(path).expect("the sink's file is readable")
}
