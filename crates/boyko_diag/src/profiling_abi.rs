//! Layer B — the profiling ABI, **hosted** here, not shared.
//!
//! # Hosted, and the word is load-bearing
//!
//! This crate's growth rule admits a thing only if **both** subsystems write it and a disagreement
//! between two copies would show up in a joined artifact. `profiling_abi` fails that test: the
//! profiler writes it and `boyko_log` never names it. It is here for a **graph** reason instead.
//!
//! The ABI must sit below `boyko_threadpool` and `boyko_rhi_vulkan`, because both open zones.
//! Before this crate the only thing below everything was `boyko_utils`, which must keep an empty
//! `[dependencies]` and must not become a second one. So the ABI is *hosted* by the bottom crate
//! and namespaced to say so — it is `profiling_abi`, not `abi`, and this paragraph is why.
//!
//! **`boyko_log` must never name anything in this module.** That is a mechanical check, not a
//! convention: a grep for `ZoneTier`, `ARM_MASK` or `declare_zone` in the logger's sources must
//! return nothing.
//!
//! # The two-axis gate
//!
//! ```text
//! const { $handle::TIER as u8 <= GLOBAL_TIER as u8 }   // (a) compile: folds, deletes codegen
//!     && ARM_MASK.load(Acquire) & scope_bit != 0       // (b) runtime: one Acquire load, one bt
//! ```
//!
//! **The tier is read from the `mod` companion, never through the handle static**, and that is not
//! a style choice: the handle carries an `AtomicU16`, and a `const` block that reads through it is
//! `error[E0080]: constant accesses mutable global memory`. The obvious spelling does not compile.
//! [`declare_zone!`] therefore emits **two** items per zone — a `static` in the value namespace and
//! a `mod` in the type namespace, sharing one name — and the gate reads the module's `const`.
//!
//! # What the two axes buy, and what neither does
//!
//! Axis (a) is the **compile ceiling**: `const false` short-circuits the `&&` and the arm and its
//! operands vanish. Axis (b) is the **runtime flag** and is the site's floor — one `Acquire` load
//! of a cache-padded global plus one statically-predicted-not-taken branch, at every surviving
//! site, forever. A flag has to be read in order to be a flag, so disarming cannot drive (b) to
//! zero. Only the tier removes the site.
//!
//! **The tier fold deletes CODEGEN, not TOKENS.** The expansion names its handle twice — once in
//! the gate's `const` block and once in the guard body — and name resolution runs on both whichever
//! way the const folds. A mistyped zone identifier is therefore `E0425` in **every** profile,
//! retail included. Only the feature axis, which deletes the macro definition before name
//! resolution, can hide one.

use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};

/// How much a build is willing to compile in.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ZoneTier {
    /// Survives every profile, including retail. For zones a shipped title still wants.
    Always = 0,
    /// Development builds.
    Dev = 1,
    /// Deep instrumentation: per-item, per-draw, per-entity.
    Deep = 2,
}

impl ZoneTier {
    /// Build a tier from its raw discriminant.
    ///
    /// # Panics
    ///
    /// On a value outside `0..=2`. Used only from a `const` context, where that is a **compile
    /// error** — a build script that emitted a nonsense tier must stop the build, not silently
    /// instrument the wrong amount of a shipped title.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u8) -> ZoneTier {
        match raw {
            0 => ZoneTier::Always,
            1 => ZoneTier::Dev,
            2 => ZoneTier::Deep,
            _ => panic!("invariant: a tier is 0..=2; see boyko_diag::profile::PROFILING_TIER"),
        }
    }
}

/// The compile-time tier ceiling for this build. A zone above it does not exist.
pub const GLOBAL_TIER: ZoneTier = ZoneTier::from_raw(crate::profile::PROFILING_TIER);

/// Number of scopes a build can arm independently.
pub const SCOPE_COUNT: u32 = 64;

/// The runtime flag word: bit *s* set means scope *s* is armed.
///
/// **Its own cache line.** It is read by every surviving site on every frame and written only when
/// a scope is armed or disarmed, so sharing a line with anything that changes would invalidate it
/// for every reader on every write. `.bss`-zero means disarmed, which is what makes a process that
/// never arms the profiler free of it without an initialiser.
#[repr(C, align(64))]
struct ArmMask {
    bits: AtomicU64,
    _pad: [u8; 56],
}

static ARM_MASK: ArmMask = ArmMask { bits: AtomicU64::new(0), _pad: [0; 56] };

const _: () = assert!(core::mem::size_of::<ArmMask>() == 64);
const _: () = assert!(core::mem::align_of::<ArmMask>() == 64);

/// Immutable per-zone metadata, one `'static` per [`declare_zone!`].
pub struct ZoneDesc {
    /// Printed name.
    pub name: &'static str,
    /// Which scope's bit arms this zone.
    pub scope: u32,
    /// Declared tier. Duplicated into the `mod` companion, which is what the gate reads —
    /// see the module docs on why the gate cannot read it from here.
    pub tier: ZoneTier,
    /// Which lane region this zone's samples go to.
    ///
    /// A property of the DECLARING CRATE, not of the site: `declare_zone!` reads
    /// `crate::__BOYKO_ZONE_PARTITION`, which [`profiling_partition!`](crate::profiling_partition)
    /// puts at that crate's root. So a game cannot mint one engine zone by accident, and the choice
    /// is a compile-time constant rather than a branch on the sample path.
    pub region: crate::sample::Region,
}

/// A declared zone: its metadata and the id the registry assigns on first use.
pub struct ZoneHandle {
    /// Cold metadata.
    pub desc: &'static ZoneDesc,
    /// Registry id, minted once. **This field is why the gate cannot read the tier through the
    /// handle**: an `AtomicU16` makes the whole static mutable global memory, and a `const` block
    /// that reads through it is `E0080`.
    pub id: AtomicU16,
    /// Closed intervals. See [`ZoneGuard`] on why the accumulators live here at this rung.
    calls: AtomicU64,
    /// Raw clock ticks accumulated across those intervals.
    ticks: AtomicU64,
}

impl ZoneHandle {
    /// Declare a handle. `const`, so the static costs no initialiser.
    #[must_use]
    pub const fn new(desc: &'static ZoneDesc) -> ZoneHandle {
        ZoneHandle {
            desc,
            id: AtomicU16::new(0),
            calls: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
        }
    }

    /// Closed intervals recorded on this zone.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    /// Raw clock ticks accumulated across those intervals. Scaled by a reader, never here.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

/// Engine zone slots. Sized here rather than per profile at this rung: nothing reads a profile
/// value for it yet, and a constant nothing reads is a value nothing can prove wrong.
pub const ENGINE_ZONE_SLOTS: usize = 4096;

/// The id a zone gets when the registry is full.
///
/// **Not a panic and not a silent alias.** A profiler that aborts a shipped title because it ran
/// out of name slots has become the failure it exists to report; a profiler that hands out a
/// duplicate id merges two zones' numbers, which is worse than losing one because the result still
/// looks like data. The zone runs unregistered, its interval still accumulates on its own handle,
/// and the exhaustion is counted.
pub const ZONE_ID_EXHAUSTED: u16 = u16::MAX;

/// Next free slot. `.bss`-zero, so slot 0 is the first minted and an un-minted handle's `id` of 0
/// is distinguishable only by the claimed flag below — which is why minting uses a CAS on the
/// handle's own id rather than a bare "is it zero" test.
static NEXT_SLOT: AtomicU16 = AtomicU16::new(1);

/// Registered descriptors, indexed by zone id. `.bss`, never freed, address-stable.
static REGISTRY: [core::sync::atomic::AtomicPtr<ZoneDesc>; ENGINE_ZONE_SLOTS] =
    [const { core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()) }; ENGINE_ZONE_SLOTS];

/// This zone's registry id, minting one on first use.
///
/// Ids start at **1**: zero is the un-minted state of the handle's own field, so using it as a
/// valid id would make "never minted" and "minted first" indistinguishable without a second flag.
///
/// Concurrent first uses race on one `compare_exchange`; the loser adopts the winner's id rather
/// than minting a second. Two ids for one zone would split its samples across two rows, and the
/// split would look like two quiet zones instead of one busy one.
pub fn zone_id(handle: &'static ZoneHandle) -> u16 {
    let cur = handle.id.load(Ordering::Acquire);
    if cur != 0 {
        return cur;
    }
    mint_cold(handle)
}

/// The once-per-zone mint, out of line so the hot path is a load and a compare.
#[cold]
#[inline(never)]
fn mint_cold(handle: &'static ZoneHandle) -> u16 {
    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
    if slot == 0 || (slot as usize) >= ENGINE_ZONE_SLOTS {
        // Wrapped or past the end. Count it and give every later caller the same answer, so the
        // exhaustion is reported once per zone rather than once per call.
        crate::loss::record_here(crate::loss::LossClass::Refused, 0);
        let _ = handle.id.compare_exchange(
            0,
            ZONE_ID_EXHAUSTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return handle.id.load(Ordering::Acquire);
    }
    match handle.id.compare_exchange(0, slot, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            REGISTRY[slot as usize].store(
                core::ptr::from_ref(handle.desc).cast_mut(),
                Ordering::Release,
            );
            slot
        }
        // Another thread minted first. Its id is the zone's; ours is simply abandoned, which costs
        // one slot out of `ENGINE_ZONE_SLOTS` and is bounded by the number of zones times the
        // number of threads that raced on their first use.
        Err(won) => won,
    }
}

/// The descriptor registered under `id`, if any.
#[must_use]
pub fn zone_desc(id: u16) -> Option<&'static ZoneDesc> {
    let p = REGISTRY.get(id as usize)?.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        // SAFETY: only `mint_cold` writes this slot, and it writes a pointer derived from a
        //   `&'static ZoneDesc` reached through a `&'static ZoneHandle`. The store is `Release`
        //   and this load is `Acquire`, so a non-null observation sees a fully published pointer.
        //   Descriptors are `'static` and never freed, so the reference cannot dangle.
        Some(unsafe { &*p })
    }
}

/// An open zone. Closes on `Drop`, **including the unwinder's**.
///
/// # Why the guard, and not `open()`/`close()`
///
/// A zone that a panic unwinds through must still close, or its interval is lost and — worse —
/// every enclosing zone's interval silently absorbs it. `Drop` is the only closing discipline the
/// language enforces.
///
/// # Where the sample goes at this rung
///
/// Into the handle's own two atomics. **The lane-based sample transport is the larger half of the
/// profiler's own rung**, and until it exists a zone that accumulated nowhere would make the gate
/// and the guard untestable — a measured interval with no observable effect is indistinguishable
/// from a guard that measures nothing. The accumulators are `Relaxed` adds on a cold-ish path and
/// are **not** the design's final storage; they are what makes this rung provable on its own.
pub struct ZoneGuard {
    handle: &'static ZoneHandle,
    opened: u64,
}

impl ZoneGuard {
    /// Open a zone. One clock read.
    ///
    /// Callers reach this through [`zone!`](crate::zone), which puts both gates in front of it —
    /// so by the time this runs, the tier admitted the site and the scope is armed.
    #[inline]
    #[must_use]
    pub fn open(handle: &'static ZoneHandle) -> ZoneGuard {
        ZoneGuard { handle, opened: crate::clock::ticks() }
    }
}

impl Drop for ZoneGuard {
    #[inline]
    fn drop(&mut self) {
        // `wrapping_sub`, because the clock is a raw counter and a wrap must yield the interval
        // rather than a number near 2^64. A plain subtraction would panic in debug and produce a
        // nonsense total in release -- once per counter lap, which is to say never during a test
        // and eventually in a session.
        let elapsed = crate::clock::ticks().wrapping_sub(self.opened);
        self.handle.calls.fetch_add(1, Ordering::Relaxed);
        self.handle.ticks.fetch_add(elapsed, Ordering::Relaxed);
        // The transport takes the same interval. A refusal here is already counted by the region's
        // `overflow`, so the return value is dropped rather than escalated -- and the accumulators
        // above stay, because they are what a caller can read before rung 2's fold exists. When it
        // does, they become the transport's cross-check rather than its substitute.
        crate::sample::push(
            self.handle.desc.region,
            crate::sample::Sample {
                stamp: self.opened,
                value: elapsed,
                zone: zone_id(self.handle),
                flags: crate::sample::SampleKind::Span as u16,
                _pad: 0,
            },
        );
    }
}

/// Open a zone if both gates admit it.
///
/// ```ignore
/// let _z = zone!(VB_EARLY_RASTER);
/// ```
///
/// Expands to an `Option<ZoneGuard>`: `None` when either gate refuses. **This is the second place
/// the expansion names the handle identifier** — the first is the gate's `const` block — which is
/// why a mistyped zone is `E0425` in every feature-on profile, retail included, whichever way the
/// tier folds.
#[macro_export]
macro_rules! zone {
    ($handle:ident) => {
        if $crate::zone_enabled!($handle) {
            Some($crate::profiling_abi::ZoneGuard::open(&$handle))
        } else {
            None
        }
    };
}

/// Whether scope `s` is armed. One `Acquire` load and one bit test.
///
/// `Acquire` rather than `Relaxed`: arming publishes the buffers a sample will be written into, so
/// a site that observes the bit must observe those too.
#[inline]
#[must_use]
pub fn scope_armed(scope: u32) -> bool {
    debug_assert!(scope < SCOPE_COUNT, "invariant: a scope index is below SCOPE_COUNT");
    ARM_MASK.bits.load(Ordering::Acquire) & (1u64 << (scope % SCOPE_COUNT)) != 0
}

/// Arm a scope. Runs on the enable path, never at process start.
pub fn arm_scope(scope: u32) {
    ARM_MASK.bits.fetch_or(1u64 << (scope % SCOPE_COUNT), Ordering::Release);
}

/// Disarm a scope.
pub fn disarm_scope(scope: u32) {
    ARM_MASK.bits.fetch_and(!(1u64 << (scope % SCOPE_COUNT)), Ordering::Release);
}

/// Whether anything at all is armed — the one load a caller needs to skip a whole subsystem.
#[inline]
#[must_use]
pub fn any_armed() -> bool {
    ARM_MASK.bits.load(Ordering::Acquire) != 0
}

/// Declare a zone: **two items sharing one name**, in two namespaces.
///
/// ```ignore
/// declare_zone!(VB_EARLY_RASTER, name = "vb.early_raster", scope = 3, tier = ZoneTier::Dev);
/// ```
///
/// expands to a `static VB_EARLY_RASTER: ZoneHandle` (value namespace) **and** a
/// `mod VB_EARLY_RASTER { pub const TIER: ZoneTier = …; }` (type namespace). A `static` and a `mod`
/// may share a name; a `struct` and a `static` may not, which is why the companion is a module and
/// not a marker type.
///
/// **The companion exists because the gate cannot read the tier from the handle.** The handle
/// carries an `AtomicU16`, so a `const` block reading through it is `E0080: constant accesses
/// mutable global memory`. Measured, not assumed — the obvious spelling was specified for four
/// revisions and does not compile.
///
/// The `use super::*;` inside the module is also load-bearing: a macro-emitted `mod` is a fresh
/// scope that inherits none of the caller's imports, so without it `ZoneTier` is unresolvable at
/// every expansion site that did not happen to glob-import it.
// `clippy::crate_in_macro_def` is exactly the behaviour wanted here and nowhere else in this
// crate: `crate::__BOYKO_ZONE_PARTITION` MUST resolve in the CALLER's crate root, because the
// partition is a property of the declaring crate. `$crate` would resolve to `boyko_diag` and make
// every zone in the workspace an engine zone -- silently, and in the one field whose whole job is
// to isolate a game's samples from the engine's. The lint's usual case is a bug; this is the
// mechanism.
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! declare_zone {
    ($ident:ident, name = $name:literal, scope = $scope:expr, tier = $tier:expr $(,)?) => {
        #[doc = concat!("Zone `", $name, "`.")]
        pub static $ident: $crate::profiling_abi::ZoneHandle =
            $crate::profiling_abi::ZoneHandle::new(&$crate::profiling_abi::ZoneDesc {
                name: $name,
                scope: $scope,
                tier: $tier,
                // The DECLARING crate's partition, not this site's. A crate that never wrote
                // `profiling_partition!` fails here with an unresolved path, which is the intended
                // outcome: an unpartitioned zone has no region to be isolated in.
                region: crate::__BOYKO_ZONE_PARTITION,
            });

        #[doc = concat!("Compile-time facts about zone `", $name, "`.")]
        #[allow(non_snake_case)]
        pub mod $ident {
            // Present so `$tier`/`$scope` expressions naming caller-scope items resolve. A caller
            // that writes two literals does not need it, hence the allow rather than a second
            // macro arm.
            #[allow(unused_imports)]
            use super::*;
            /// The declared tier, readable from a `const` block — which the handle static is not.
            pub const TIER: $crate::profiling_abi::ZoneTier = $tier;
            /// The arming scope.
            pub const SCOPE: u32 = $scope;
        }
    };
}

/// The gate a zone site expands to.
///
/// Written as a macro rather than a function so gate (a) is a `const` block the compiler folds and
/// short-circuits: a function call would evaluate its arguments, which is the entire property the
/// `&&` chain exists to prevent.
#[macro_export]
macro_rules! zone_enabled {
    ($handle:ident) => {
        const { $handle::TIER as u8 <= $crate::profiling_abi::GLOBAL_TIER as u8 }
            && $crate::profiling_abi::scope_armed($handle::SCOPE)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    declare_zone!(TEST_ALWAYS, name = "t.always", scope = 1, tier = ZoneTier::Always);
    declare_zone!(TEST_DEEP, name = "t.deep", scope = 2, tier = ZoneTier::Deep);

    #[test]
    fn the_handle_and_the_companion_share_a_name_in_two_namespaces() {
        // The two-item expansion, exercised through BOTH namespaces at once. A `struct` companion
        // would be `E0428` here; a `mod` is not.
        assert_eq!(TEST_ALWAYS.desc.name, "t.always");
        assert_eq!(TEST_ALWAYS::TIER, ZoneTier::Always);
        assert_eq!(TEST_ALWAYS::SCOPE, 1);
        assert_eq!(TEST_ALWAYS.desc.tier, TEST_ALWAYS::TIER, "the two copies must agree");
    }

    /// THE reason the companion exists — and the proof is that this **compiles**, not that any
    /// assertion in it passes.
    ///
    /// Reading `TEST_ALWAYS.desc.tier` in this `const` block instead would be `E0080: constant
    /// accesses mutable global memory`, because the handle carries an `AtomicU16`. So the const
    /// block IS the test; an `assert!(OK)` beneath it would be `assert!(true)`, which clippy
    /// correctly refuses and which would teach a reader that the assertions here are decoration.
    const _COMPANION_IS_CONST_READABLE: bool = TEST_ALWAYS::TIER as u8 <= GLOBAL_TIER as u8;

    #[test]
    fn a_disarmed_scope_gates_everything_out() {
        // `.bss`-zero is disarmed, and that is what makes a process that never arms free of the
        // profiler without an initialiser.
        disarm_scope(1);
        assert!(!scope_armed(1));
        assert!(!zone_enabled!(TEST_ALWAYS));
    }

    #[test]
    fn arming_one_scope_does_not_arm_its_neighbours() {
        disarm_scope(10);
        disarm_scope(11);
        arm_scope(10);
        assert!(scope_armed(10));
        assert!(!scope_armed(11), "scopes must be independent bits, not a single flag");
        disarm_scope(10);
        assert!(!scope_armed(10));
    }

    #[test]
    fn the_tier_gate_folds_independently_of_the_runtime_flag() {
        // Arm the scope as wide as it goes, so the ONLY thing that can refuse a site is its tier.
        // In the `dev` profile `GLOBAL_TIER` is `Deep`, so both survive; the assertion that
        // matters is the SHAPE -- a site above the ceiling is refused with the flag fully armed.
        arm_scope(2);
        assert!(scope_armed(2));
        // The handle static is referenced here on purpose. `zone_enabled!` reads ONLY the module
        // companion, so at this rung nothing touches the static at all -- clippy said so, and it
        // is right: the second naming of the identifier lives in the guard body, which arrives
        // with the sample path. Until then this is where the static is proved to exist and to
        // carry the same tier the gate reads.
        assert_eq!(TEST_DEEP.desc.tier, TEST_DEEP::TIER);
        assert_eq!(TEST_DEEP.desc.scope, TEST_DEEP::SCOPE);
        assert!(zone_enabled!(TEST_DEEP), "the dev profile's Deep ceiling admits a Deep zone");
        disarm_scope(2);
        assert!(!zone_enabled!(TEST_DEEP), "disarming must refuse regardless of tier");
    }

    #[test]
    fn any_armed_is_the_one_load_that_skips_a_subsystem() {
        disarm_scope(20);
        let quiet = !any_armed();
        arm_scope(20);
        assert!(any_armed());
        disarm_scope(20);
        assert_eq!(!any_armed(), quiet || !any_armed());
    }

    // One zone and one scope PER TEST. Both are process-global, and two tests sharing either one
    // arm and disarm each other's gate while reading each other's counters -- measured: the first
    // draft shared `TEST_GUARD` between the two tests below and failed on the second run of the
    // suite, not the first.
    declare_zone!(TEST_GUARD, name = "t.guard", scope = 30, tier = ZoneTier::Always);
    declare_zone!(TEST_UNWIND, name = "t.unwind", scope = 31, tier = ZoneTier::Always);

    #[test]
    fn an_armed_zone_records_an_interval_and_a_disarmed_one_records_nothing() {
        disarm_scope(30);
        let before = (TEST_GUARD.calls(), TEST_GUARD.ticks());
        {
            let _z = zone!(TEST_GUARD);
            std::hint::spin_loop();
        }
        assert_eq!(
            (TEST_GUARD.calls(), TEST_GUARD.ticks()),
            before,
            "a disarmed zone must not open a guard, so nothing may accumulate"
        );

        arm_scope(30);
        {
            let z = zone!(TEST_GUARD);
            assert!(z.is_some(), "an armed zone in an admitting tier must open");
            for _ in 0..2000 {
                std::hint::spin_loop();
            }
        }
        assert_eq!(TEST_GUARD.calls(), before.0 + 1, "exactly one interval must be recorded");
        assert!(TEST_GUARD.ticks() > before.1, "the interval must have a positive duration");
        disarm_scope(30);
    }

    #[test]
    fn a_zone_closes_on_the_unwinding_path() {
        // A zone a panic unwinds through must still close, or its interval is lost AND every
        // enclosing zone silently absorbs it -- a wrong number rather than a missing one.
        arm_scope(31);
        let before = TEST_UNWIND.calls();
        let caught = std::panic::catch_unwind(|| {
            let _z = zone!(TEST_UNWIND);
            panic!("deliberate, inside a zone");
        });
        assert!(caught.is_err());
        assert_eq!(TEST_UNWIND.calls(), before + 1, "the unwinder must still close the zone");
        disarm_scope(31);
    }

    declare_zone!(TEST_ID_A, name = "t.id.a", scope = 40, tier = ZoneTier::Always);
    declare_zone!(TEST_ID_B, name = "t.id.b", scope = 41, tier = ZoneTier::Always);

    #[test]
    fn minting_is_idempotent_and_distinct_zones_get_distinct_ids() {
        let a1 = zone_id(&TEST_ID_A);
        let a2 = zone_id(&TEST_ID_A);
        let b = zone_id(&TEST_ID_B);

        assert_ne!(a1, 0, "ids start at 1; zero is the un-minted state of the handle's field");
        assert_eq!(a1, a2, "a second use must adopt the first id, not mint another");
        assert_ne!(a1, b, "two zones sharing an id would merge their numbers");

        assert_eq!(zone_desc(a1).map(|d| d.name), Some("t.id.a"));
        assert_eq!(zone_desc(b).map(|d| d.name), Some("t.id.b"));
        assert!(zone_desc(0).is_none(), "slot 0 is never minted into");

        // Minting is INDEPENDENT of the gate: a zone gets its id whether or not its scope is
        // armed, because the id is identity and the gate is admission. Asserting it here also
        // keeps the module companion's consts live, which clippy noticed were otherwise unread by
        // these three zones -- a fair observation, since nothing but `zone!` reads them.
        disarm_scope(40);
        disarm_scope(41);
        assert!(!zone_enabled!(TEST_ID_A), "a disarmed zone is still registrable");
        assert!(!zone_enabled!(TEST_ID_B));
        assert_eq!(zone_id(&TEST_ID_A), a1, "arming state must not change identity");
        assert_eq!(zone_id(&TEST_ID_B), b);
    }

    #[test]
    fn racing_first_uses_agree_on_one_id() {
        // Two ids for one zone would split its samples across two rows, and the split reads as two
        // quiet zones rather than one busy one -- a wrong picture, not a missing one.
        use std::sync::Arc;
        use std::sync::atomic::AtomicU32;

        declare_zone!(TEST_ID_RACE, name = "t.id.race", scope = 42, tier = ZoneTier::Always);

        let seen = Arc::new([const { AtomicU32::new(0) }; 8]);
        let mut hs = Vec::new();
        for k in 0..8usize {
            let s = Arc::clone(&seen);
            hs.push(std::thread::spawn(move || {
                s[k].store(u32::from(zone_id(&TEST_ID_RACE)), Ordering::SeqCst);
            }));
        }
        for h in hs {
            h.join().expect("minting thread panicked");
        }
        let first = seen[0].load(Ordering::SeqCst);
        assert_ne!(first, 0);
        disarm_scope(42);
        assert!(!zone_enabled!(TEST_ID_RACE));
        for k in 1..8 {
            assert_eq!(seen[k].load(Ordering::SeqCst), first, "racing minters disagreed");
        }
    }

    #[test]
    fn the_arm_mask_owns_its_cache_line() {
        // Asserted at run time as well as in the `const` asserts: a reordering that preserved the
        // size would pass those alone, and the whole point is that no neighbour shares the line a
        // hot reader loads every frame.
        assert_eq!(core::mem::size_of::<ArmMask>(), 64);
        assert_eq!(std::ptr::from_ref(&ARM_MASK) as usize % 64, 0);
    }
}
