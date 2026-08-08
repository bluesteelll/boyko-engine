//! The sample transport — two SPSC regions per lane, and the isolation that makes one of them
//! a game's problem rather than the engine's.
//!
//! # Two regions, not one, and the reason is not tidiness
//!
//! A runaway game scope — a zone opened per entity, per draw, per particle — fills its region and
//! drops **its own** samples. `engine_overflow` stays zero, the engine's frame timings stay whole,
//! and the profile a developer is reading does not silently lose the rows that would have explained
//! the game's problem. One region would have made a game's mistake indistinguishable from an engine
//! regression, in the artifact whose whole job is to tell them apart.
//!
//! The region is a **compile-time constant of the declaring crate** ([`crate::profiling_partition`]),
//! so there is no runtime branch and no per-site escape: a crate is one partition or the other, in
//! one greppable line at its root.
//!
//! It is also a false-sharing fix. Four distinct cache lines per lane — engine writer, engine
//! reader, user writer, user reader — so a game's `write` cursor never invalidates the engine's.
//!
//! # What is here and what is rung 2's
//!
//! Here: the record, the rings, their cursors, the push, the drain, and the overflow accounting.
//! **Not** here: where the buffers come from. `buf` is an `AtomicPtr` published once at arm by the
//! ECS-side store, which owns a `VmReservation` and a stride chosen at arm time. Until something
//! publishes a buffer, every push is refused and counted — which is the honest state of a process
//! that has a profiler compiled in and has not armed it.

use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};

use crate::lane::LANE_COUNT;
use crate::profile::REGION_CAPACITY;

/// What a sample records.
///
/// Two bits of [`Sample::flags`]. A fourth value is reserved rather than used, so a decoder that
/// meets one from a future writer can say "unknown kind" instead of guessing.
#[repr(u16)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleKind {
    /// A closed interval. `value` is its duration in clock ticks.
    Span = 0,
    /// An increment, summed within a frame. `value` is the increment.
    Counter = 1,
    /// A level. `value` is the level; the last one in a frame wins.
    Gauge = 2,
}

impl SampleKind {
    /// Bits `[0..1]` of [`Sample::flags`].
    const MASK: u16 = 0b11;

    /// The kind a raw `flags` word encodes, or `None` for the reserved encoding.
    #[must_use]
    pub const fn from_flags(flags: u16) -> Option<SampleKind> {
        match flags & Self::MASK {
            0 => Some(SampleKind::Span),
            1 => Some(SampleKind::Counter),
            2 => Some(SampleKind::Gauge),
            _ => None,
        }
    }
}

/// The sample bit that marks a GPU-origin record.
pub const FLAG_GPU: u16 = 1 << 2;

/// One record. **Exactly 24 bytes**, pinned below.
///
/// # The payload has its own 64 bits, and that is a correction
///
/// An earlier revision overloaded `stamp` with three meanings — a timestamp, a counter value, and
/// the high bits of a long duration — and the fold read it *before* dispatching on kind. Counters,
/// gauges and long spans were therefore all attributed by a field that was not a time. Separating
/// them costs 8 bytes per sample and removes a whole class of silently-wrong attribution.
///
/// `stamp` is an **absolute** `u64` tick count rather than a frame-relative `u32`: a `u32` of ticks
/// overflows on a frame longer than about 1.4 s, which is exactly the hitch most worth recording.
/// Absolute stamps also make frame attribution a merge rather than an epoch comparison.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// `Span`: the clock at OPEN. `Counter`/`Gauge`: the clock at the emit call. The attribution
    /// key — the frame a sample belongs to is decided by this and nothing else.
    pub stamp: u64,
    /// `Span`: duration in clock ticks — a `u64`, so nothing saturates and no continuation record
    /// exists. `Counter`: the increment. `Gauge`: the level.
    pub value: u64,
    /// The zone id this sample belongs to.
    pub zone: u16,
    /// `[0..1]` kind · `[2]` GPU origin · `[3..15]` reserved.
    ///
    /// There is deliberately **no `saturated` bit** — nothing saturates — and **no depth field**:
    /// nesting is reconstructed from the stamps, and a depth written by the producer would be one
    /// more thing that can disagree with them.
    pub flags: u16,
    /// Named rather than implicit, so the layout is pinned instead of incidental.
    pub _pad: u32,
}

const _: () = assert!(size_of::<Sample>() == 24);
const _: () = assert!(align_of::<Sample>() == 8);

impl Sample {
    /// The kind this sample encodes, or `None` for the reserved encoding.
    #[must_use]
    pub const fn kind(&self) -> Option<SampleKind> {
        SampleKind::from_flags(self.flags)
    }
}

/// Which region of a lane a sample goes to.
///
/// A compile-time property of the declaring crate, never a runtime choice — see the module doc.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    /// Zones declared by engine crates.
    Engine = 0,
    /// Zones declared by games, plugins, mods, tools and benches.
    User = 1,
}

/// Workspace members whose crates may declare `Engine` zones.
///
/// The list is a `const` here rather than a naming convention, so `profiling_partition!(Engine)`
/// **fails to compile** outside it — the const assert compares `CARGO_PKG_NAME` against this array.
/// An out-of-workspace crate can still write the line; it just does not build.
///
/// Residual, named rather than hidden: a workspace member that lies is one greppable line, and a
/// tidy test pins this list against the actual member set. What there is **no** escape from is the
/// per-site level — a crate is one partition for all of its zones.
pub const ENGINE_PACKAGES: &[&str] = &[
    "boyko-app",
    "boyko-diag",
    "boyko-ecs",
    "boyko-fontbake",
    "boyko-image",
    "boyko-input",
    "boyko-log",
    "boyko-macros",
    "boyko-math",
    "boyko-physics",
    "boyko-render",
    "boyko-rhi",
    "boyko-rhi-vulkan",
    "boyko-scene",
    "boyko-sdf-math",
    "boyko-serialize",
    "boyko-shaderdsl",
    "boyko-threadpool",
    "boyko-ui",
    "boyko-utils",
];

/// `true` when `name` is an engine package. A `const fn` so the check is a compile error.
#[must_use]
pub const fn is_engine_package(name: &str) -> bool {
    let mut i = 0;
    while i < ENGINE_PACKAGES.len() {
        if const_str_eq(ENGINE_PACKAGES[i], name) {
            return true;
        }
        i += 1;
    }
    false
}

/// `str` equality in a `const` context. `==` on `&str` is not `const` on stable.
const fn const_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Declare which lane region a crate's zones belong to. **One line at the crate root.**
///
/// ```ignore
/// boyko_diag::profiling_partition!(Engine);   // engine crates
/// boyko_diag::profiling_partition!(User);     // games, plugins, mods, tools, benches
/// ```
///
/// `profiling_partition!(Engine)` carries a `const` assert against [`ENGINE_PACKAGES`], so a
/// downstream crate claiming the engine's region does not compile. `declare_zone!` then reads the
/// constant this defines, for both the id counter and the ring region — neither is a per-site
/// choice, which is what makes a game's runaway scope cost a game's samples and nothing else.
#[macro_export]
macro_rules! profiling_partition {
    (Engine) => {
        /// This crate's lane region. Read by `declare_zone!`.
        pub const __BOYKO_ZONE_PARTITION: $crate::sample::Region = $crate::sample::Region::Engine;
        const _: () = assert!(
            $crate::sample::is_engine_package(env!("CARGO_PKG_NAME")),
            "profiling_partition!(Engine) is for engine crates; use profiling_partition!(User)"
        );
    };
    (User) => {
        /// This crate's lane region. Read by `declare_zone!`.
        pub const __BOYKO_ZONE_PARTITION: $crate::sample::Region = $crate::sample::Region::User;
    };
}

/// The producer's half of one region. Its own cache line.
///
/// **All mutable state is atomic.** No `UnsafeCell`, no plain field mutated through a `&'static` —
/// an earlier revision had both, and neither is expressible here without a lie about who writes
/// what.
#[repr(C, align(64))]
struct RegionWriter {
    /// Published **once** at first arm with a `Release`, and never nulled afterwards. A buffer that
    /// could be taken away would make every producer's pointer a use-after-free the instant a host
    /// disarmed.
    buf: AtomicPtr<Sample>,
    /// Samples ever published. Read `Relaxed` by the sole owner, `Release`-stored after the bytes.
    write: AtomicU32,
    /// Samples refused for want of space. **Monotone: never cleared, by anybody.**
    ///
    /// This is `substrate/loss-fold`'s Q2(b) shape, and it landed here after the first version of
    /// this file used `fetch_sub(observed)` instead. That version was not *wrong* — the producer
    /// here increments with an RMW, so the lost-update window Q2 describes (an owner's
    /// `load; add; store` overwriting a consumer's subtract) cannot open. It was **a second shape
    /// for a question the substrate had already answered**, which is the duplication class this
    /// crate exists to delete: a reader who learnt the rule from `boyko_diag::loss` would have had
    /// to re-derive it here.
    ///
    /// `u64` rather than `u32` for the same reason the logging plan widened its loss counters: a
    /// monotone `u32` at a region's refusal rate has a wrap that is *unlikely*, and "unlikely" is
    /// not the kind of statement a loss counter may rest on. At 2^64 it is a proof.
    overflow: AtomicU64,
    _pad: [u8; 40],
}

/// The consumer's half. Its own line, so a drain never invalidates a producer's.
#[repr(C, align(64))]
struct RegionReader {
    read: AtomicU32,
    _pad: [u8; 60],
}

#[repr(C, align(64))]
struct RegionLane {
    w: RegionWriter,
    r: RegionReader,
}

/// One lane's transport: two regions, four cache lines.
#[repr(C, align(64))]
struct ZoneLane {
    engine: RegionLane,
    user: RegionLane,
}

impl RegionLane {
    const fn new() -> RegionLane {
        RegionLane {
            w: RegionWriter {
                buf: AtomicPtr::new(core::ptr::null_mut()),
                write: AtomicU32::new(0),
                overflow: AtomicU64::new(0),
                _pad: [0; 40],
            },
            r: RegionReader { read: AtomicU32::new(0), _pad: [0; 60] },
        }
    }
}

impl ZoneLane {
    const fn new() -> ZoneLane {
        ZoneLane { engine: RegionLane::new(), user: RegionLane::new() }
    }

    #[inline]
    fn region(&self, region: Region) -> &RegionLane {
        match region {
            Region::Engine => &self.engine,
            Region::User => &self.user,
        }
    }
}

const _: () = assert!(size_of::<ZoneLane>() == 256);
const _: () = assert!(core::mem::offset_of!(ZoneLane, user) == 128);
const _: () = assert!(core::mem::offset_of!(RegionLane, r) == 64);

// SAFETY (manual `Sync` for `ZoneLane`):
//   1. WRITE side: exactly one thread ever writes a region's `write` or its buffer -- the one whose
//      `crate::lane::lane()` returns this index. That uniqueness is the substrate's and is
//      single-writer by construction on both of its paths: a pool worker's index IS its dense
//      `worker_id`, and every other thread's index comes from `claim_lane()`'s load-then-CAS.
//   2. READ side: exactly one thread reads the buffer and writes `read` -- whichever holds the
//      consumer role. At this rung that role is the caller of `drain_region`, and the ECS fold
//      takes it under a `ResMut` at rung 2.
//   3. Visibility: bytes written before `write.store(_, Release)` are visible to a consumer that
//      loads `write` with `Acquire`. The consumer never reads past its observed `w` and never
//      advances `read` over samples it has not copied out.
//   4. `buf` is write-once with a `Release` store and is read with `Acquire`, so a producer that
//      observes a non-null pointer observes a buffer the publisher had finished preparing. It is
//      never nulled, so no producer's pointer can be invalidated by a disarm.
//   5. `overflow` is written by the producer alone and is MONOTONE -- no consumer ever clears it,
//      so there is no window for a clear to race an increment at all. A reader takes differences
//      against its own last-seen value, which is `substrate/loss-fold`'s Q2(b) contract and the
//      same one `crate::loss::delta_since` implements for the per-lane cells.
unsafe impl Sync for ZoneLane {}

/// The transports. `.bss`, never freed, address-stable for the process.
///
/// 256 B per lane × 80 lanes = **20 KiB in every profile**, and it is a *reserved* extent: `.bss`
/// is demand-zero, and a process that never arms writes no page of it.
static LANES: [ZoneLane; LANE_COUNT as usize] = [const { ZoneLane::new() }; LANE_COUNT as usize];

/// Publish a region's buffer. Called once per lane per region, at arm.
///
/// # Safety
///
/// `buf` must point to at least [`REGION_CAPACITY`] writable, correctly-aligned `Sample` slots that
/// stay valid for the rest of the process. The transport never frees it and never nulls the
/// pointer, which is what lets a producer hold it without a lifetime.
///
/// Returns `false` when this region already has a buffer — publication is once, not idempotent-
/// with-replacement, because a replacement would strand every producer that had already read the
/// old pointer.
pub unsafe fn publish_region(lane: u16, region: Region, buf: *mut Sample) -> bool {
    let Some(l) = LANES.get(lane as usize) else { return false };
    let w = &l.region(region).w;
    w.buf
        .compare_exchange(core::ptr::null_mut(), buf, Ordering::Release, Ordering::Relaxed)
        .is_ok()
}

/// Whether a region has a buffer yet.
#[must_use]
pub fn region_armed(lane: u16, region: Region) -> bool {
    LANES
        .get(lane as usize)
        .is_some_and(|l| !l.region(region).w.buf.load(Ordering::Acquire).is_null())
}

/// Append one sample to this thread's lane, in `region`. Returns `false` when it was refused.
///
/// A refusal is **counted, never silent**: the region's `overflow` rises, and the fold reports it
/// as a shortfall rather than letting the artifact read as complete.
///
/// The three refusal causes are deliberately not distinguished here — no lane, no buffer, no room.
/// They differ in what a host should do about them, and that is a question for the report the fold
/// writes, which has the lane table and the arm state to answer it with. A producer on the hot path
/// has neither.
#[inline]
pub fn push(region: Region, sample: Sample) -> bool {
    let lane_id = crate::lane::lane();
    let Some(l) = LANES.get(lane_id as usize) else {
        // No lane: nothing to charge the loss to either, which is why this is the one refusal the
        // transport cannot count. `crate::loss`'s un-laned row is where it lands instead.
        crate::loss::record_here(crate::loss::LossClass::Unclaimed, 0);
        return false;
    };
    let w = &l.region(region).w;

    let buf = w.buf.load(Ordering::Acquire);
    if buf.is_null() {
        w.overflow.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let write = w.write.load(Ordering::Relaxed);
    let read = l.region(region).r.read.load(Ordering::Acquire);
    // Monotone cursors: the live count is the DIFFERENCE, not the write cursor. `write` wraps at
    // 2^32 and only differences are ever taken -- the invariant that a `write <= CAPACITY` check
    // would have got wrong the moment a long session lapped the counter.
    if write.wrapping_sub(read) >= REGION_CAPACITY {
        w.overflow.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let slot = (write % REGION_CAPACITY) as usize;
    // SAFETY: `buf` is non-null (checked above) and, by `publish_region`'s contract, points to at
    //   least `REGION_CAPACITY` writable, aligned `Sample` slots valid for the process. `slot <
    //   REGION_CAPACITY` by the modulo. This thread is the region's only producer (`Sync` clause 1),
    //   and the admission check above proved this slot holds no sample the consumer has yet to copy.
    unsafe { buf.add(slot).write(sample) };

    // Publishes the bytes above to a consumer that loads `write` with `Acquire`.
    w.write.store(write.wrapping_add(1), Ordering::Release);
    true
}

/// Hand every published sample in `lane`'s `region` to `on_sample`, oldest first.
///
/// Returns how many were handed over.
///
/// # The two properties this upholds
///
/// 1. **It never reads past its observed `write`.** One `Acquire` load fixes the horizon.
/// 2. **It never advances `read` over samples it has not copied.** The cursor moves once, at the
///    end, to the value the walk actually reached.
///
/// # Safety
///
/// The caller must be this region's single consumer. At this rung that is enforced by convention
/// and by the tests; the ECS fold takes it under a `ResMut` at rung 2, where the scheduler's
/// exclusivity analysis becomes the proof.
pub unsafe fn drain_region(
    lane: u16,
    region: Region,
    mut on_sample: impl FnMut(Sample),
) -> u32 {
    let Some(l) = LANES.get(lane as usize) else { return 0 };
    let rl = l.region(region);
    let buf = rl.w.buf.load(Ordering::Acquire);
    if buf.is_null() {
        return 0;
    }
    let w = rl.w.write.load(Ordering::Acquire);
    let mut r = rl.r.read.load(Ordering::Relaxed);
    let mut moved = 0;

    while r != w {
        let slot = (r % REGION_CAPACITY) as usize;
        // SAFETY: `slot < REGION_CAPACITY` by the modulo, and `buf` is the published buffer of at
        //   least that many slots. Every index below the observed `write` was initialised by
        //   `push` before the `Release` this walk's `Acquire` paired with. `Sample` is `Copy` POD.
        on_sample(unsafe { buf.add(slot).read() });
        r = r.wrapping_add(1);
        moved += 1;
    }

    // Published only after every sample above has been copied out. Moving this inside the loop
    // would let the producer overwrite a slot between the callback and the store.
    rl.r.read.store(r, Ordering::Release);
    moved
}

/// Samples this region has refused, **for the life of the process**.
///
/// Monotone and never cleared. A consumer that wants "since my last fold" keeps its own last-seen
/// value and subtracts — the delta lives at the consumer, which is what makes a clear, and the
/// window a clear opens, unnecessary. `substrate/loss-fold`'s Q2 resolution (b), applied here
/// rather than re-derived.
#[must_use]
pub fn overflow(lane: u16, region: Region) -> u64 {
    LANES
        .get(lane as usize)
        .map_or(0, |l| l.region(region).w.overflow.load(Ordering::Relaxed))
}

/// Refused samples since `last`, advancing `last` to the current total.
///
/// The consumer-side delta, in the one shape both subsystems use. Saturating rather than
/// wrapping: a caller that passes a `last` from a different region gets 0, not a number near 2^64
/// that would read as a catastrophic loss.
pub fn overflow_since(lane: u16, region: Region, last: &mut u64) -> u64 {
    let now = overflow(lane, region);
    let delta = now.saturating_sub(*last);
    *last = now;
    delta
}

/// Samples this region holds that the consumer has not taken.
#[must_use]
pub fn pending(lane: u16, region: Region) -> u32 {
    LANES.get(lane as usize).map_or(0, |l| {
        let rl = l.region(region);
        rl.w.write.load(Ordering::Acquire).wrapping_sub(rl.r.read.load(Ordering::Relaxed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lane index no other test uses, so this file's process-global writes stay its own.
    const LANE_A: u16 = LANE_COUNT - 1;
    const LANE_B: u16 = LANE_COUNT - 2;

    fn slab() -> Box<[Sample]> {
        vec![
            Sample { stamp: 0, value: 0, zone: 0, flags: 0, _pad: 0 };
            REGION_CAPACITY as usize
        ]
        .into_boxed_slice()
    }

    fn span(zone: u16, stamp: u64, value: u64) -> Sample {
        Sample { stamp, value, zone, flags: SampleKind::Span as u16, _pad: 0 }
    }

    /// Publication is ONCE. A second buffer would strand every producer holding the first.
    #[test]
    fn a_region_takes_one_buffer_and_refuses_a_second() {
        let mut a = slab();
        let mut b = slab();
        assert!(!region_armed(LANE_A, Region::Engine));
        // SAFETY: `a` outlives the calls below and holds REGION_CAPACITY slots. It is leaked at the
        //   end of this test rather than dropped, because the transport never un-publishes.
        assert!(unsafe { publish_region(LANE_A, Region::Engine, a.as_mut_ptr()) });
        assert!(region_armed(LANE_A, Region::Engine));
        // SAFETY: same contract; the call is expected to REFUSE and never store the pointer.
        assert!(
            !unsafe { publish_region(LANE_A, Region::Engine, b.as_mut_ptr()) },
            "a second publication would strand producers holding the first pointer"
        );
        core::mem::forget(a);
        drop(b);
    }

    /// The engine region fills and refuses while the user region is untouched — and vice versa.
    ///
    /// This is the isolation the two-region split exists for: a runaway game scope must not cost
    /// the engine a single sample, and one region's `overflow` must not move the other's.
    #[test]
    fn the_two_regions_are_isolated_in_capacity_and_in_loss() {
        // `LANE` is a thread-local, so pinning it here isolates this test's producer
        // side by construction; the LANE index is this test's alone, which isolates the
        // process-global `LANES` row.
        crate::lane::set_lane(LANE_B);
        let mut engine = slab();
        let mut user = slab();
        // SAFETY: both slabs hold REGION_CAPACITY slots and are leaked below; the transport never
        //   frees or nulls a published pointer.
        unsafe {
            assert!(publish_region(LANE_B, Region::Engine, engine.as_mut_ptr()));
            assert!(publish_region(LANE_B, Region::User, user.as_mut_ptr()));
        }

        // Fill the USER region exactly, then one past it.
        for i in 0..REGION_CAPACITY {
            assert!(push(Region::User, span(7, u64::from(i), 1)), "slot {i} of a fresh region");
        }
        assert!(!push(Region::User, span(7, 0, 1)), "a full region must refuse");
        assert_eq!(overflow(LANE_B, Region::User), 1);

        // The engine region is untouched by any of that.
        assert_eq!(overflow(LANE_B, Region::Engine), 0, "a game's overflow reached the engine's");
        assert_eq!(pending(LANE_B, Region::Engine), 0);
        assert!(push(Region::Engine, span(3, 100, 42)), "the engine region is still empty");

        // The drain returns exactly what was pushed, oldest first, and frees the space.
        let mut seen = Vec::new();
        // SAFETY: this thread is the only consumer of these regions in this test.
        let moved = unsafe { drain_region(LANE_B, Region::User, |s| seen.push(s)) };
        assert_eq!(moved, REGION_CAPACITY);
        assert_eq!(seen.len(), REGION_CAPACITY as usize);
        assert_eq!(seen[0].stamp, 0);
        assert_eq!(seen[REGION_CAPACITY as usize - 1].stamp, u64::from(REGION_CAPACITY - 1));
        assert_eq!(seen[0].kind(), Some(SampleKind::Span));
        assert!(push(Region::User, span(7, 999, 1)), "a drained region has room again");

        // The counter is MONOTONE and the delta lives at the consumer: a second read of the same
        // total yields 0 without anything having been cleared.
        let mut seen_loss = 0u64;
        assert_eq!(overflow_since(LANE_B, Region::User, &mut seen_loss), 1);
        assert_eq!(overflow_since(LANE_B, Region::User, &mut seen_loss), 0);
        assert_eq!(overflow(LANE_B, Region::User), 1, "the total must NOT have been cleared");

        core::mem::forget(engine);
        core::mem::forget(user);
    }

    /// A region with no buffer refuses and COUNTS. An un-armed profiler is not a silent one.
    #[test]
    fn an_unarmed_region_refuses_and_counts() {
        crate::lane::set_lane(LANE_COUNT - 3);
        let before = overflow(LANE_COUNT - 3, Region::Engine);
        assert!(!region_armed(LANE_COUNT - 3, Region::Engine));
        assert!(!push(Region::Engine, span(1, 0, 0)));
        assert_eq!(overflow(LANE_COUNT - 3, Region::Engine), before + 1);
        // SAFETY: no buffer was published, so the drain returns before touching one.
        assert_eq!(unsafe { drain_region(LANE_COUNT - 3, Region::Engine, |_| {}) }, 0);
    }

    /// A closed zone lands in the transport, in ITS CRATE'S region, with its own id.
    ///
    /// The wiring, end to end: without it the guard would still accumulate into its handle and
    /// every gate would stay green while the transport carried nothing — a measured interval with
    /// no observable effect is indistinguishable from a guard that measures nothing.
    #[test]
    fn a_closed_zone_reaches_the_transport() {
        const LANE_C: u16 = LANE_COUNT - 4;
        crate::lane::set_lane(LANE_C);
        crate::clock::calibrate();
        let mut engine = slab();
        // SAFETY: the slab holds REGION_CAPACITY slots and is leaked below.
        assert!(unsafe { publish_region(LANE_C, Region::Engine, engine.as_mut_ptr()) });

        crate::declare_zone!(TRANSPORT_PROBE, name = "transport-probe", scope = 3, tier = crate::profiling_abi::ZoneTier::Always);
        crate::profiling_abi::arm_scope(3);
        {
            let _z = crate::zone!(TRANSPORT_PROBE);
        }
        crate::profiling_abi::disarm_scope(3);

        let want = crate::profiling_abi::zone_id(&TRANSPORT_PROBE);
        let mut seen = Vec::new();
        // SAFETY: this thread is the only consumer of this region in this test.
        let moved = unsafe { drain_region(LANE_C, Region::Engine, |s| seen.push(s)) };
        assert_eq!(moved, 1, "one closed zone must produce exactly one sample");
        assert_eq!(seen[0].zone, want, "the sample must carry the zone's minted id");
        assert_eq!(seen[0].kind(), Some(SampleKind::Span));
        // `boyko_diag` writes `profiling_partition!(Engine)` at its root, so a zone declared inside
        // it is an ENGINE zone -- which is the whole claim, and it is checked rather than assumed.
        assert_eq!(pending(LANE_C, Region::User), 0, "an engine zone reached the user region");

        core::mem::forget(engine);
    }

    /// The reserved kind encoding is reported as unknown rather than guessed.
    #[test]
    fn the_reserved_kind_is_not_guessed() {
        assert_eq!(SampleKind::from_flags(0b11), None);
        assert_eq!(SampleKind::from_flags(FLAG_GPU), Some(SampleKind::Span));
    }
}
