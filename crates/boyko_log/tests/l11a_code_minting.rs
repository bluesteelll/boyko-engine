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
    CODE_IDX_EXHAUSTED, CodeIdx, DOWNSTREAM_IDX_BASE, MAX_CODES, code_occupancy, resolve_idx,
};

/// One `#[test]`, for the reason the module header gives: every claim reads one global counter.
#[test]
fn the_mint_hands_out_one_index_per_code_and_never_aliases() {
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

    static PAST: AtomicU16 = AtomicU16::new(0);
    assert_eq!(
        resolve_idx(CodeIdx::Dynamic(&PAST)),
        CODE_IDX_EXHAUSTED,
        "past the space the mint must return the SENTINEL, never wrap into an occupied slot"
    );
}
