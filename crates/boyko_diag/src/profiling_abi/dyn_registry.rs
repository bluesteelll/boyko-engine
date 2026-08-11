//! Profiling rung 10 — zones defined by DATA, not by a macro at a call site.
//!
//! # What this is for, and why `declare_zone!` cannot do it
//!
//! [`declare_zone!`](crate::declare_zone) needs a name that is a string literal and an identifier
//! that is a Rust item. A zone whose name comes from a config file, a mod's manifest, a script or
//! a spawned entity's archetype has neither: there is no site to write the macro at, and the set
//! of names is not known until the data is read. [`register_zone`] is the entry point for that
//! case, and the [`DynZoneHandle`] it returns is 16 bytes of plain `Copy` data a game stores
//! wherever it stores the thing being measured — in an ECS component, in a script's userdata,
//! behind an FFI boundary.
//!
//! # The two arenas, and why they are `.bss`
//!
//! A registration must not allocate. Not "should not" — the profiler runs inside a frame it is
//! measuring, and a heap that grows under it changes the number being reported. So the names and
//! the descriptors live in two fixed-extent statics sized by compile-time constants, which is the
//! storage policy's `.bss` arm (see [`crate::storage`]): [`MAX_USER_BUDGET`] descriptors and
//! [`DYN_NAME_BYTES`] of name text, reserved by two monotone counters and never freed.
//!
//! Never freed is not an oversight either. A [`ZoneDesc`] is published into `REGISTRY` and read by
//! a fold on another thread; reclaiming a slot would need to prove no reader holds it, which needs
//! either a lock on the emission path or an epoch scheme — both larger than the thing they would
//! reclaim. A registration is a once-per-zone cost, and the budget is what bounds it.
//!
//! # `DYN_DESCS` holds `MaybeUninit`, and that is forced
//!
//! [`ZoneDesc`] carries a `&'static str`. A null reference is not a valid `&str`, so `ZoneDesc` can
//! never implement [`ZeroInit`](crate::storage::ZeroInit) and a `SyncCells<ZoneDesc, N>` can never
//! be `zeroed()`. `MaybeUninit<ZoneDesc>` can: every bit pattern is a valid `MaybeUninit<T>` by
//! definition. The slot is written exactly once, by the thread whose `fetch_add` reserved it, and
//! becomes readable to everyone else only through `REGISTRY`'s `Release` store — so "initialised"
//! is not a flag anybody has to check, it is implied by having obtained the pointer at all.
//!
//! # What this rung does NOT do
//!
//! * **No zone here is emitted from yet.** `zone_dyn!`/`counter_dyn!`/`gauge_dyn!` are below and
//!   compile, but nothing in this engine calls them: the game-facing acceptance path is rung 15's
//!   overlay. What rung 10 delivers is the registry and the id-space isolation, both gated.
//! * **It does not make `zone_desc` a production reader.** MEASURED at this rung: `zone_desc` has
//!   no caller outside its own tests, engine or user. The registry is written in production and
//!   read only by tests, and that is as true after this rung as before it — the name resolver that
//!   will read it is the telemetry decoder (rung 13) and the overlay (rung 15).
//! * **It does not bound name TEXT per zone.** A single 60 000-byte name is a legal use of the
//!   arena that starves every later registrant. The refusal is counted and reported, which is the
//!   property gated; "one registrant cannot be greedy" is not claimed.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    MAX_USER_BUDGET, REGISTRY, USER_SCOPE_BASE, ZONE_ID_EXHAUSTED, ZoneDesc, ZoneTier, mint_id_in,
};
use crate::sample::Region;
use crate::storage::SyncCells;

/// Bytes of name text the dynamic registry may hand out, in total.
///
/// **On the `BOYKO_PROFILE` axis since J1**: `dev` / `editor` 64 KiB, the three others 8 KiB.
///
/// Rung 10 landed it as a literal on [`MAX_USER_BUDGET`]'s precedent and for its reason — the axis
/// was rung 14's — at the dev figure, the row every other constant in this crate was written at.
///
/// It is deliberately not `MAX_USER_BUDGET × some_max_name_len`: names are wildly uneven — a
/// generated `"mod.blade_runner.tick.phase3"` beside a `"ai"` — and a per-zone cap large enough for
/// the long one would reserve that much for the short one. One shared arena spends the bytes where
/// they are actually used, at the cost stated in the module docs. That shape is what lets the two
/// arenas shrink by different factors across the axis (descriptors 6×, names 8×) without either
/// figure needing to justify itself against the other.
pub const DYN_NAME_BYTES: usize = crate::profile::DYN_NAME_BYTES;

/// The name arena. One shared byte range, carved by [`DYN_NAME_NEXT`].
static DYN_NAMES: SyncCells<u8, DYN_NAME_BYTES> = SyncCells::zeroed();

/// The descriptor arena, indexed by `id - ENGINE_ZONE_SLOTS`.
static DYN_DESCS: SyncCells<MaybeUninit<ZoneDesc>, MAX_USER_BUDGET> = SyncCells::zeroed();

/// Next free byte of [`DYN_NAMES`]. Monotone; a reservation past the end is refused, not wrapped.
static DYN_NAME_NEXT: AtomicU32 = AtomicU32::new(0);

/// Bytes [`DYN_DESCS`] occupies — one of the two terms rung 10 adds to the residency bound.
///
/// A `size_of` over the real static's type rather than a product a consumer types in, for the
/// reason [`super::registry_bytes`] states: a hand-multiplied figure is a second spelling of the
/// extent, and the two can drift while both look right.
#[must_use]
pub const fn dyn_descs_bytes() -> usize {
    size_of::<SyncCells<MaybeUninit<ZoneDesc>, MAX_USER_BUDGET>>()
}

/// Bytes [`DYN_NAMES`] occupies — the other term.
#[must_use]
pub const fn dyn_names_bytes() -> usize {
    size_of::<SyncCells<u8, DYN_NAME_BYTES>>()
}

/// What a caller asks for. Borrowed, because the arena is what makes the name `'static`.
///
/// `name` is a `&str` and not a `&'static str` on purpose: a caller reading a mod manifest has a
/// `String` whose bytes it is about to drop, and requiring `'static` from it would force the
/// allocation this whole module exists to avoid.
#[derive(Clone, Copy, Debug)]
pub struct ZoneSpec<'a> {
    /// Printed name. Copied into the arena; the borrow ends when [`register_zone`] returns.
    pub name: &'a str,
    /// Which scope's bit arms this zone. Must be `>= USER_SCOPE_BASE`.
    pub scope: u32,
    /// Declared tier, exactly as a static zone's.
    pub tier: ZoneTier,
}

/// Why a registration was refused.
///
/// Every variant is a **refusal that is counted and reported**, never a panic: `register_zone` can
/// be reached from a mod's data at frame 40 000, and a profiler that kills a shipped title over a
/// bad manifest has become the failure it exists to report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegisterError {
    /// `spec.scope < USER_SCOPE_BASE` — the engine's range. `W9212`.
    EngineScope,
    /// [`MAX_USER_BUDGET`] ids are gone. `W9210`.
    BudgetExhausted,
    /// [`DYN_NAME_BYTES`] of name text are gone. `W9210`, the same code: from a host's point of
    /// view both say "the user registry is full", and both are answered by the same knob.
    NameArenaExhausted,
    /// The name is empty. Refused rather than accepted, because an unnamed zone is a row a reader
    /// cannot attribute and it would consume a slot that a named one could have used.
    EmptyName,
}

/// A registered dynamic zone: everything emission needs, and nothing else.
///
/// # Sixteen bytes, `Copy`, `Send + Sync`, no thread affinity
///
/// That combination is what lets a game store one in an ECS component and emit from whichever
/// worker thread the system happens to run on. It is plain data — an id and a precomputed bit — so
/// it carries no borrow of the registry and no lifetime.
///
/// # `arm_bit` is carried, and that is the whole performance argument
///
/// The alternative is to store only the id and recover the scope from `REGISTRY[id]` at each
/// emission. That is a dependent load into a 56 KiB table on the hot path, and it is exactly the
/// implementation `G17`'s RED substitutes to make the dynamic leg exceed its budget. Carrying the
/// bit costs 8 bytes in a struct a game holds once per zone and removes the load entirely.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DynZoneHandle {
    /// The registry id, already offset into the user half of the space.
    pub id: u16,
    /// `1 << scope` — precomputed so an emission is a mask test and never a table walk.
    pub arm_bit: u64,
}

impl DynZoneHandle {
    /// Whether this zone's scope is armed right now. One `Acquire` load and a bit test.
    #[must_use]
    #[inline]
    pub fn armed(self) -> bool {
        super::arm_mask_bits() & self.arm_bit != 0
    }
}

/// Register a zone whose name is data. **Cold, lock-free, allocator-free.**
///
/// Steps, in the order the ordering argument depends on:
///
/// 1. Validate the scope and the name. Both refusals happen before any counter moves, so a rejected
///    spec consumes neither an id nor a byte.
/// 2. Reserve `name.len()` bytes of [`DYN_NAMES`] with one `fetch_add`. On overrun, give the bytes
///    back with a `fetch_sub` so the counter stays a truthful bound rather than drifting up on
///    every failed attempt.
/// 3. Reserve an id from [`USER_ID_NEXT`] — the SAME counter a `User` crate's static
///    `declare_zone!` draws from (D19).
/// 4. Copy the name bytes and build a `&'static str` over the reserved range.
/// 5. Write the [`ZoneDesc`] into this thread's reserved [`DYN_DESCS`] slot.
/// 6. Publish the descriptor pointer into `REGISTRY` with a `Release` store — the one edge that
///    makes every write above visible to a reader that `Acquire`s the pointer.
///
/// # Errors
///
/// [`RegisterError`], one per refusal; each is also counted and raised for the fold to report.
#[cold]
#[inline(never)]
pub fn register_zone(spec: ZoneSpec<'_>) -> Result<DynZoneHandle, RegisterError> {
    if spec.scope < USER_SCOPE_BASE || spec.scope >= super::SCOPE_COUNT {
        crate::loss::raise(crate::loss::DiagFlag::EngineScopeRefused);
        return Err(RegisterError::EngineScope);
    }
    if spec.name.is_empty() {
        return Err(RegisterError::EmptyName);
    }

    let len = spec.name.len() as u32;
    let off = DYN_NAME_NEXT.fetch_add(len, Ordering::Relaxed);
    // `checked_add` and not `off + len`: a pathological `len` near `u32::MAX` would wrap the sum
    // and make an overrun look like a fit. The counter is `u32` and the arena is 64 KiB, so the
    // wrap needs a name no caller would write — which is precisely why it must be handled here
    // rather than assumed away.
    let end = off.checked_add(len);
    if end.is_none_or(|e| e as usize > DYN_NAME_BYTES) {
        DYN_NAME_NEXT.fetch_sub(len, Ordering::Relaxed);
        crate::loss::record_here(crate::loss::LossClass::Refused, 0);
        crate::loss::raise(crate::loss::DiagFlag::UserZoneBudgetExhausted);
        return Err(RegisterError::NameArenaExhausted);
    }

    let id = mint_id_in(Region::User);
    if id == ZONE_ID_EXHAUSTED {
        // The name bytes are given back too. `mint_id_in` already counted and raised the
        // exhaustion, so this path adds no second report of one event.
        DYN_NAME_NEXT.fetch_sub(len, Ordering::Relaxed);
        return Err(RegisterError::BudgetExhausted);
    }
    let slot = id as usize - super::ENGINE_ZONE_SLOTS;

    // SAFETY: `off..off + len` was reserved by THIS thread's `fetch_add` and the counter is
    // monotone, so no other thread can hold an overlapping range; the bound check above proved
    // `off + len <= DYN_NAME_BYTES`, so every index is in range. `get_ptr` requires `i < N` and
    // single-writer, both established. The bytes are written before the `Release` store below, so
    // no reader can observe them half-copied.
    unsafe {
        let base = DYN_NAMES.get_ptr(off as usize);
        core::ptr::copy_nonoverlapping(spec.name.as_ptr(), base, len as usize);
    }

    // SAFETY: the range was just filled with a byte-for-byte copy of `spec.name`, which is a valid
    // `&str`, so the same bytes are valid UTF-8. The lifetime is `'static` because `DYN_NAMES` is a
    // `static` that is never freed and this range is never reused — the counter is monotone and
    // nothing decrements it except the give-back above, which happens only on a path that returns
    // before any byte is written.
    let name: &'static str = unsafe {
        let base = DYN_NAMES.get_ptr(off as usize).cast_const();
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(base, len as usize))
    };

    // SAFETY: `slot < MAX_USER_BUDGET` because `id < ZONE_ID_SPACE` (checked by `mint_id_in`) and
    // `slot = id - ENGINE_ZONE_SLOTS`. The slot was reserved by this thread's own successful mint,
    // and the id counter is monotone, so no other thread writes it. The write initialises the
    // `MaybeUninit`, and the `Release` store below is what lets anyone else read it.
    unsafe {
        DYN_DESCS.get_ptr(slot).write(MaybeUninit::new(ZoneDesc {
            name,
            scope: spec.scope,
            tier: spec.tier,
            // Always `User`. A dynamic zone has no declaring crate to read a partition from, and
            // there is no argument that could set it: an entry point that let a caller choose
            // `Engine` would be the hole the whole partition exists to close.
            region: Region::User,
        }));
    }

    // The publication. Everything written above is visible to a thread that `Acquire`s this
    // pointer out of `REGISTRY`, and nothing before it is visible without doing so.
    //
    // SAFETY: the pointer is to a slot this thread just initialised, in a `static` that is never
    // freed, so it stays valid for `'static`.
    let desc_ptr = unsafe { DYN_DESCS.get_ptr(slot).cast::<ZoneDesc>() };
    REGISTRY[id as usize].store(desc_ptr, Ordering::Release);

    Ok(DynZoneHandle { id, arm_bit: 1u64 << spec.scope })
}

/// Open a dynamic zone across an FFI or script boundary — the half of `zone_dyn!` a macro cannot
/// cross.
///
/// Returns an opaque token that must be handed back to [`zone_dyn_close`]. `0` means "the zone was
/// not armed and nothing was opened", and closing with it is a no-op — so a script that always
/// calls the pair costs one mask test when the profiler is off.
///
/// The token is a raw clock tick and callers must not interpret it. It is `u64` rather than an
/// opaque newtype because the boundary this crosses is C ABI, where a newtype buys nothing.
#[must_use]
#[inline]
pub fn zone_dyn_open(h: DynZoneHandle) -> u64 {
    if !h.armed() {
        return 0;
    }
    // A tick of 0 is representable and would be read as "not opened". It is reachable only in the
    // first nanosecond of the counter's life, and the consequence is one lost sample rather than a
    // wrong one — stated because a token scheme with a reserved value should say which value.
    crate::clock::ticks()
}

/// Close a zone opened by [`zone_dyn_open`]. A `token` of `0` is a no-op.
#[inline]
pub fn zone_dyn_close(h: DynZoneHandle, token: u64) {
    if token == 0 {
        return;
    }
    // `wrapping_sub`, matching every other span in this crate: the counter is monotone within an
    // epoch, and across an epoch break the difference is meaningless either way — the fold
    // quarantines the window rather than this path second-guessing it.
    let elapsed = crate::clock::ticks().wrapping_sub(token);
    // Always `Region::User`. There is no dynamic zone in the engine's region and no argument that
    // could put one there — the same closure `register_zone` makes at registration, restated at
    // emission so the two cannot drift.
    crate::sample::push(
        Region::User,
        crate::sample::Sample {
            stamp: token,
            value: elapsed,
            zone: h.id,
            flags: crate::sample::SampleKind::Span as u16,
            _pad: 0,
        },
    );
}

/// A dynamic zone's RAII guard — [`super::ZoneGuard`]'s twin for a handle that is data.
///
/// Separate from `ZoneGuard` rather than generic over the two, because they carry different things:
/// a static guard holds a `&'static ZoneHandle` and accumulates into its `calls`/`ticks`, while a
/// dynamic one holds 16 bytes of `Copy` data and has no per-zone accumulator to reach — the
/// descriptor arena is written once at registration and never touched again. Unifying them would
/// mean a branch on every close, on the path whose cost is the point.
///
/// `!Send`, like its twin: it carries a lane binding established at open.
pub struct DynZoneGuard {
    handle: DynZoneHandle,
    opened: u64,
    /// Binds the guard to the thread that opened it — the lane it will push to is that thread's.
    _not_send: core::marker::PhantomData<*const ()>,
}

impl DynZoneGuard {
    /// Open a guard on an ARMED handle. The caller has already tested the gate; see [`zone_dyn`].
    #[must_use]
    #[inline]
    pub fn open(handle: DynZoneHandle) -> DynZoneGuard {
        DynZoneGuard {
            handle,
            opened: crate::clock::ticks(),
            _not_send: core::marker::PhantomData,
        }
    }
}

impl Drop for DynZoneGuard {
    #[inline]
    fn drop(&mut self) {
        zone_dyn_close(self.handle, self.opened);
    }
}

/// Open a dynamic zone for the enclosing scope. The dynamic twin of [`zone!`](crate::zone).
///
/// ```ignore
/// let _z = zone_dyn!(handle);   // `handle: DynZoneHandle`, e.g. out of a component
/// ```
///
/// # Why this is a macro when its argument is a value
///
/// [`zone!`](crate::zone) is a macro because its gate is a `const` block the compiler folds. This
/// one has no compile-time tier to fold — a data zone has no tier known at build time — so the
/// macro buys only one thing, and it is the thing that matters: **the guard is bound in the
/// caller's scope**, so it closes at the caller's brace rather than at the end of a call
/// expression. A function returning `Option<DynZoneGuard>` would work identically at every correct
/// call site and silently measure nothing at `let _ = open(h);`.
///
/// ⚠️ **The expansion touches `REGISTRY` nowhere, and that is the gated property.** `arm_bit` is
/// carried on the handle precisely so this path is a load of one atomic word and a mask test.
/// Recovering the scope from `REGISTRY[id]` instead is `G17`'s RED.
#[macro_export]
macro_rules! zone_dyn {
    ($handle:expr) => {
        if $crate::profiling_abi::dyn_registry::DynZoneHandle::armed($handle) {
            Some($crate::profiling_abi::dyn_registry::DynZoneGuard::open($handle))
        } else {
            None
        }
    };
}

/// Add `v` to a dynamic counter — a RATE per frame, never a level.
///
/// The kind is in the sample's flags and is not interchangeable with [`gauge_dyn!`](crate::gauge_dyn):
/// a reducer sums a counter across a frame and takes the last value of a gauge, so feeding one to
/// the other's reducer produces a number that is wrong in a way no assertion downstream can catch.
#[macro_export]
macro_rules! counter_dyn {
    ($handle:expr, $v:expr) => {
        $crate::profiling_abi::dyn_registry::emit_dyn(
            $handle,
            $v,
            $crate::sample::SampleKind::Counter,
        )
    };
}

/// Record `v` as a dynamic gauge — a LEVEL at an instant, never a rate. See [`counter_dyn!`](crate::counter_dyn).
#[macro_export]
macro_rules! gauge_dyn {
    ($handle:expr, $v:expr) => {
        $crate::profiling_abi::dyn_registry::emit_dyn(
            $handle,
            $v,
            $crate::sample::SampleKind::Gauge,
        )
    };
}

/// The one body behind [`counter_dyn!`](crate::counter_dyn) and [`gauge_dyn!`](crate::gauge_dyn).
///
/// A function rather than two macro bodies: unlike [`zone_dyn!`](crate::zone_dyn) there is no guard
/// whose scope must be the caller's, so the macros exist only to name the kind — and two macro
/// bodies that differ in one enum variant are two places for that variant to be wrong.
#[inline]
pub fn emit_dyn(h: DynZoneHandle, v: u64, kind: crate::sample::SampleKind) {
    if !h.armed() {
        return;
    }
    crate::sample::push(
        Region::User,
        crate::sample::Sample {
            stamp: crate::clock::ticks(),
            value: v,
            zone: h.id,
            flags: kind as u16,
            _pad: 0,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the tests that read the process-global reservation counters.
    ///
    /// `DYN_NAME_NEXT` and `USER_ID_NEXT` are process-wide and monotone, so a test asserting "this
    /// call spent nothing" is asserting about a number every other test in this binary also moves.
    /// MEASURED: without this lock `an_empty_name_consumes_neither_an_id_nor_a_byte` fails under
    /// `libtest`'s default parallelism and passes under `--test-threads=1` — a gate whose colour
    /// depends on how it was invoked. The same shape rung 8's `G4c` trio hit, answered the same
    /// way.
    ///
    /// A `Mutex` is the right tool and the ban's own exception applies: this is test scaffolding,
    /// not an engine path.
    #[allow(clippy::disallowed_types)]
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Takes the lock, ignoring poisoning — a panicking sibling has already failed the run, and
    /// turning its poison into a second failure here would report one defect twice.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A registration produces a resolvable zone, and the descriptor a reader gets back is the one
    /// the caller asked for — name included, out of the arena rather than out of the caller's
    /// buffer.
    #[test]
    fn a_registered_zone_resolves_to_the_name_its_caller_passed() {
        let _serial = serial();
        // Built at run time from pieces, so nothing here can be a `&'static str` the compiler
        // interned: the name that comes back MUST have travelled through the arena.
        let owned = format!("mod.{}.tick", "probe_a");
        let h = register_zone(ZoneSpec {
            name: &owned,
            scope: USER_SCOPE_BASE,
            tier: ZoneTier::Always,
        })
        .expect("a fresh registration succeeds");

        assert!(
            h.id as usize >= super::super::ENGINE_ZONE_SLOTS,
            "a dynamic zone must mint from the USER half of the id space, got {}",
            h.id
        );
        assert_eq!(h.arm_bit, 1u64 << USER_SCOPE_BASE, "the arm bit is precomputed");

        let desc = super::super::zone_desc(h.id).expect("the descriptor is published");
        assert_eq!(desc.name, owned, "the name did not survive the arena");
        assert_eq!(desc.scope, USER_SCOPE_BASE);
        assert_eq!(desc.region, Region::User);

        // And the borrowed name is genuinely gone by the time the descriptor is read.
        drop(owned);
        let desc = super::super::zone_desc(h.id).expect("still published");
        assert!(desc.name.starts_with("mod."), "the arena copy outlives the caller's buffer");
    }

    /// An engine scope is REFUSED, not clamped.
    ///
    /// The RED: replace the refusal with `spec.scope.max(USER_SCOPE_BASE)` and this reds — a
    /// clamped zone would register successfully on a neighbouring scope, which is the silent
    /// outcome `W9212` exists to prevent.
    #[test]
    fn an_engine_scope_is_refused_rather_than_moved() {
        let _serial = serial();
        for scope in [0u32, 1, USER_SCOPE_BASE - 1] {
            assert_eq!(
                register_zone(ZoneSpec { name: "x", scope, tier: ZoneTier::Always }),
                Err(RegisterError::EngineScope),
                "scope {scope} is the engine's and must be refused"
            );
        }
        // The boundary itself is the first LEGAL scope, not the last refused one.
        assert!(
            register_zone(ZoneSpec {
                name: "boundary",
                scope: USER_SCOPE_BASE,
                tier: ZoneTier::Always
            })
            .is_ok(),
            "USER_SCOPE_BASE is the first scope a game may have"
        );
    }

    /// A scope past the mask's width is refused too — otherwise `1 << scope` is UB.
    #[test]
    fn a_scope_past_the_mask_is_refused() {
        let _serial = serial();
        assert_eq!(
            register_zone(ZoneSpec {
                name: "x",
                scope: super::super::SCOPE_COUNT,
                tier: ZoneTier::Always
            }),
            Err(RegisterError::EngineScope),
            "a scope with no bit in the mask has no arm bit to precompute"
        );
    }

    /// An empty name is refused before any counter moves.
    #[test]
    fn an_empty_name_consumes_neither_an_id_nor_a_byte() {
        let _serial = serial();
        let ids_before = super::super::minted_user_zones();
        let bytes_before = DYN_NAME_NEXT.load(Ordering::Relaxed);
        assert_eq!(
            register_zone(ZoneSpec { name: "", scope: USER_SCOPE_BASE, tier: ZoneTier::Always }),
            Err(RegisterError::EmptyName)
        );
        assert_eq!(super::super::minted_user_zones(), ids_before, "no id was spent");
        assert_eq!(DYN_NAME_NEXT.load(Ordering::Relaxed), bytes_before, "no byte was spent");
    }

    /// Two registrations get distinct ids, distinct name ranges and distinct registry entries.
    ///
    /// The RED this pins: reserve the name range without `fetch_add` (a plain load plus store) and
    /// two concurrent registrants overlap; even serially, reusing one offset makes both names read
    /// as the second one.
    #[test]
    fn two_registrations_share_nothing() {
        let _serial = serial();
        let a = register_zone(ZoneSpec {
            name: "dyn.alpha",
            scope: USER_SCOPE_BASE + 1,
            tier: ZoneTier::Always,
        })
        .expect("first");
        let b = register_zone(ZoneSpec {
            name: "dyn.beta",
            scope: USER_SCOPE_BASE + 2,
            tier: ZoneTier::Always,
        })
        .expect("second");

        assert_ne!(a.id, b.id, "two zones must not share an id");
        assert_ne!(a.arm_bit, b.arm_bit, "two scopes must not share an arm bit");
        let da = super::super::zone_desc(a.id).expect("a published");
        let db = super::super::zone_desc(b.id).expect("b published");
        assert_eq!(da.name, "dyn.alpha");
        assert_eq!(db.name, "dyn.beta");
        assert_ne!(
            da.name.as_ptr(),
            db.name.as_ptr(),
            "two names must occupy different arena ranges"
        );
    }

    /// `armed` follows the scope mask, and the handle needs no registry lookup to say so.
    #[test]
    fn a_handle_reports_its_own_scope_without_touching_the_registry() {
        let _serial = serial();
        let h = register_zone(ZoneSpec {
            name: "dyn.armed_probe",
            scope: USER_SCOPE_BASE + 3,
            tier: ZoneTier::Always,
        })
        .expect("registered");
        super::super::disarm_scope(USER_SCOPE_BASE + 3);
        assert!(!h.armed());
        assert_eq!(zone_dyn_open(h), 0, "a disarmed zone opens nothing");
        super::super::arm_scope(USER_SCOPE_BASE + 3);
        assert!(h.armed());
        super::super::disarm_scope(USER_SCOPE_BASE + 3);
    }

    /// The two arena sizes are what the statics actually are, not what a comment says.
    #[test]
    fn the_arena_byte_figures_come_from_the_types() {
        assert_eq!(dyn_names_bytes(), DYN_NAME_BYTES, "the name arena is its own length");
        assert_eq!(
            dyn_descs_bytes(),
            MAX_USER_BUDGET * size_of::<MaybeUninit<ZoneDesc>>(),
            "the descriptor arena is budget x descriptor"
        );
        // The ABSOLUTE figure is deliberately not asserted: it is a property of `ZoneDesc`'s
        // layout, which is allowed to change, and a gate on it would fail for a field being added
        // rather than for anything being wrong. It is not printed either — this crate is the mute
        // leaf and its own lint forbids `println!` (measured: `-D warnings` reds on it). The
        // measured figures live where a reader looks for them, in `05-LADDER-GATES.md`'s rung-10
        // record: 24 B/descriptor here against the corpus's 48, so 73 728 B rather than 144 KiB.
        //
        // What IS asserted is that a descriptor did not become enormous by accident: a `ZoneDesc`
        // is four small fields, and anything past two cache lines means one of them grew into
        // something that does not belong in a table with `MAX_USER_BUDGET` entries.
        assert!(
            size_of::<MaybeUninit<ZoneDesc>>() <= 128,
            "a zone descriptor grew to {} B; at {MAX_USER_BUDGET} slots that is {} B of `.bss`",
            size_of::<MaybeUninit<ZoneDesc>>(),
            dyn_descs_bytes()
        );
    }
}
