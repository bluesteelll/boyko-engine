//! Commit granularity of the virtual-memory primitives, and the slab bounds of the commit ladders
//! that step by it.
//!
//! Moved here from `boyko_ecs::ecs::constants` at rung C1 (unified plan KC-01) together with the
//! reservation primitive that interprets them; `boyko_ecs` re-exports every item under its old
//! path. The pool policy built on them (the base stagger, `pool_byte_layout`, `pool_commit_step`)
//! stays in `boyko_ecs`, beside the `ComponentPool` that owns it.

/// Virtual-memory commit granularity (Phase X.F, renamed from
/// `ARENA_COMMIT_GRANULE` when Phase X.J retired the shared Arena): 64 KiB —
/// the Windows reservation granularity, and a multiple of the 4 KiB
/// commit/`mprotect` page size everywhere. Every `VmReservation` length is
/// rounded up to this (`os_len = align_up(len, COMMIT_GRANULE)`) so a
/// frontier commit can never overrun the kernel's page-rounded mapping.
pub const COMMIT_GRANULE: usize = 64 * 1024;

/// OS commit page (packing plan D1,
/// `docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md`): `MEM_COMMIT` inside an
/// existing reservation and `mprotect` are page-granular; only `MEM_RESERVE`
/// is bound by the 64 KiB allocation granularity ([`COMMIT_GRANULE`]).
///
/// 4 KiB on `x86_64`, the stated target platform on both OSes.
///
/// It is the commit quantum: `ComponentPool`'s page-floor ladder,
/// `VmColumn::grow_to` and [`POOL_MIN_SLAB`] step by it, while every
/// reservation (`os_len`, `pool_byte_layout`, `VmReservation::reserve`) stays
/// granule-rounded. A constant rather than a runtime query because
/// `pool_byte_layout`, `pool_commit_step` and the `const _` proofs are
/// `const fn`; a debug-only belt in `VmReservation::reserve` checks the OS page
/// divides it.
#[cfg(target_arch = "x86_64")]
pub const COMMIT_PAGE: usize = 4096;
/// Every other architecture keeps the granule as its commit page, so a
/// 16 KiB- or 64 KiB-page kernel is never handed a page-misaligned `mprotect`
/// base (packing plan D1).
#[cfg(not(target_arch = "x86_64"))]
pub const COMMIT_PAGE: usize = COMMIT_GRANULE;

// A granule is a whole number of commit pages on every arm, so every
// granule-rounded reservation length ends on a page boundary.
const _: () = assert!(COMMIT_GRANULE.is_multiple_of(COMMIT_PAGE));

/// Minimum pool commit step (Phase X.I D4, retargeted by packing plan D2):
/// one [`COMMIT_PAGE`], the first rung of the `ComponentPool` and `VmColumn`
/// commit ladders — the floor that keeps sparse archetypes cheap. The ladder
/// doubles from here (×2, not ×4, plan D3), so on `x86_64` a one-row-at-a-time
/// fill of a 40 B column to 1 M rows is 15 growth events; a batch request is
/// one.
pub const POOL_MIN_SLAB: usize = COMMIT_PAGE;

// One knob, not two (packing plan D2): the first ladder rung IS the commit
// page. Removing this assert is half of the recorded G2(c) mutation.
const _: () = assert!(POOL_MIN_SLAB == COMMIT_PAGE);

/// Maximum pool data-commit step (Phase X.I D4): 64 MiB — bounds
/// commit-charge overshoot by one slab (the X.F overshoot-honesty bound);
/// one max-step costs ≤ ~50 µs (the Phase X.F B4 envelope). A larger
/// REQUEST is not clamped (the request-dominant `max` in
/// `pool_commit_step` always covers it).
pub const POOL_MAX_SLAB: usize = 64 * 1024 * 1024;
