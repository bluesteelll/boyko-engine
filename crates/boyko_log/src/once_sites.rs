//! The `Once` register: which sites actually fired, and how many times each did.
//!
//! # Why this exists, and what its absence cost
//!
//! `RatePolicy::Once` is the one policy the emission macros deliberately do NOT apply — the latch
//! is a named [`OnceSite`](crate::codes::OnceSite) the site declares, because a `static` inside a
//! macro expansion cannot be named and an observer must be able to reset the latch it is about to
//! test. That leaves the declaration honoured by human diligence, and **nothing enumerated `Once`
//! sites, so nothing could notice when the diligence lapsed**: measured on this tree, 45 `Live`
//! rows declare `Once`/`OnceCounted` and 39 (identifier, file) pairs carry no `OnceSite` at all.
//!
//! Three documents in the corpus specify this register — an intrusive list plus one `LOG-ONCE`
//! census row per fired site — and two doc comments in the tree referred to it by name
//! (`ONCE_SITES`) as though it existed. It did not. This module is that register.
//!
//! # `fired > 1` IS THE DEFECT, and that is the whole design
//!
//! A `Once` site that has a latch fires exactly once per process. So a row reading `fired=1` is a
//! site keeping its declaration, and a row reading `fired=17` is a site whose registry row promises
//! `Once` and whose code delivers seventeen. The audit stops being a grep over identifier uses —
//! which cannot tell an emitter from a doc link — and becomes a number the census prints.
//!
//! # The accounting is the DRAIN's, not the emitting thread's
//!
//! The consumer already holds each record's `&'static LogSite`, and `LogSite::rate` carries the
//! policy as cold compile-time data. So the hash, the probe and the counter all run off the
//! emitting thread, on a path that is already cold. Putting it in the emission macro would have
//! spent an RMW per `Once` occurrence on the producer to learn something the consumer can work out
//! at leisure.
//!
//! # Counted at EMISSION, not at delivery
//!
//! [`note`] runs before the per-sink filters. A latch is spent when the site emits, whatever a sink
//! then does with the record — and a register that only counted delivered records would read
//! `fired=0` for every site in a process whose sinks are all off, which is the exact silence
//! `W0111` exists to refuse.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::codes::RatePolicy;
use crate::site::LogSite;

/// Distinct fired sites the register can hold. A power of two, so the probe wraps by mask.
///
/// 128 × 16 B = 2 KiB of `.bss`, demand-zero. The engine declares 45 `Once` rows and a row may have
/// several emitters, so this is roughly double the sites that can ever fire — and the overflow is
/// counted rather than silently dropped, because a register that quietly forgot a site would be
/// the same defect it exists to detect.
pub const MAX_ONCE_SITES: usize = 128;

const _: () = assert!(MAX_ONCE_SITES.is_power_of_two());

/// One fired site.
struct OnceEntry {
    /// The site's address, `0` while free. Published with `Release`.
    ptr: AtomicUsize,
    /// Emissions observed from it. `1` for a site that keeps its declaration.
    fired: AtomicU32,
}

impl OnceEntry {
    const fn new() -> OnceEntry {
        OnceEntry { ptr: AtomicUsize::new(0), fired: AtomicU32::new(0) }
    }
}

static SITES: [OnceEntry; MAX_ONCE_SITES] = [const { OnceEntry::new() }; MAX_ONCE_SITES];

/// Emissions from a `Once` site the register had no room for.
///
/// Counted rather than given a diagnostic code: a code is a promise of a documented page, and the
/// condition is already reported where a reader looks for it — the census's own `LOG-ONCE` header.
static OVERFLOW: AtomicU64 = AtomicU64::new(0);

/// Record one emission from a site whose code declares `Once` or `OnceCounted`.
///
/// **Called by the drain, under the drain token**, so there is exactly one writer. The atomics are
/// for the CENSUS's benefit: [`walk`] may run on any thread.
///
/// A site with any other policy is not the register's business and returns immediately — the
/// branch is on cold `'static` data the consumer already holds.
pub(crate) fn note(site: &'static LogSite) {
    if !matches!(site.rate, RatePolicy::Once | RatePolicy::OnceCounted) {
        return;
    }
    let key = core::ptr::from_ref(site) as usize;
    // Fibonacci hashing on the ADDRESS, the same mix the binary sink's site dictionary uses: site
    // statics are laid out consecutively, so the low bits alone would collide in runs.
    let mut i = ((key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize) & (MAX_ONCE_SITES - 1);
    for _ in 0..MAX_ONCE_SITES {
        let seen = SITES[i].ptr.load(Ordering::Acquire);
        if seen == key {
            // `Relaxed` load/store rather than `fetch_add`: the drain role is single, so this is
            // not a contended increment. Saturating, because a site storming for hours must read
            // as "far more than once" rather than wrap back to a number that looks disciplined.
            let n = SITES[i].fired.load(Ordering::Relaxed);
            SITES[i].fired.store(n.saturating_add(1), Ordering::Relaxed);
            return;
        }
        if seen == 0 {
            SITES[i].fired.store(1, Ordering::Relaxed);
            // The count is written BEFORE the key is published, so a walker that observes the key
            // observes a count of at least one. The other order would show `fired=0` for a site
            // that had just fired -- a row saying the opposite of what happened.
            SITES[i].ptr.store(key, Ordering::Release);
            return;
        }
        i = (i + 1) & (MAX_ONCE_SITES - 1);
    }
    OVERFLOW.fetch_add(1, Ordering::Relaxed);
}

/// One row of the register, as the census prints it.
#[derive(Clone, Copy, Debug)]
pub struct OnceRow {
    /// `b'W'` or `b'E'`.
    pub class: u8,
    /// The printed code number.
    pub code: u16,
    /// Source file of the emitting site.
    pub file: &'static str,
    /// Source line of the emitting site.
    pub line: u32,
    /// Emissions observed. **Anything above `1` is a site not keeping its `Once` declaration.**
    pub fired: u32,
    /// `true` for [`RatePolicy::OnceCounted`], whose suppressions are a real number rather than
    /// `UNCOUNTED(by policy)`.
    pub counted: bool,
}

/// Every fired site, in table order.
///
/// Table order rather than fire order: the register is a hash table and keeping insertion order
/// would cost a second structure to answer a question nobody asked. A reader looking for one site
/// looks by code and file, not by position.
pub fn walk() -> impl Iterator<Item = OnceRow> {
    SITES.iter().filter_map(|e| {
        let key = e.ptr.load(Ordering::Acquire);
        if key == 0 {
            return None;
        }
        // SAFETY: `key` was written by `note` from a `&'static LogSite`, and the `Acquire` above
        //   pairs with the `Release` that published it. A site static outlives the process, so the
        //   reference is valid for `'static`. Nothing ever writes a non-address into `ptr`.
        let site: &'static LogSite = unsafe { &*(key as *const LogSite) };
        Some(OnceRow {
            class: site.class,
            code: site.code,
            file: site.file,
            line: site.line,
            fired: e.fired.load(Ordering::Relaxed),
            counted: matches!(site.rate, RatePolicy::OnceCounted),
        })
    })
}

/// Emissions the register had no room for. Cumulative for the process.
#[must_use]
pub fn overflowed() -> u64 {
    OVERFLOW.load(Ordering::Relaxed)
}

/// How many distinct sites the register holds.
#[must_use]
pub fn len() -> usize {
    SITES.iter().filter(|e| e.ptr.load(Ordering::Acquire) != 0).count()
}

/// Empty the register. **Test builds only**, behind `test-probe`.
///
/// For the same reason `OnceSite::reset` exists: the register is process state, a test binary is
/// one process, and an observer that cannot control its preconditions is one whose green means
/// "in this order, this time".
#[cfg(feature = "test-probe")]
pub fn reset() {
    for e in &SITES {
        e.ptr.store(0, Ordering::Release);
        e.fired.store(0, Ordering::Relaxed);
    }
    OVERFLOW.store(0, Ordering::Relaxed);
}
