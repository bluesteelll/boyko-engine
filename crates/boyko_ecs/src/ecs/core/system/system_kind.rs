//! Phase 4 Seam 4 — [`SystemKind`], the 3-valued dispatch classifier that
//! replaces `SystemBox::is_exclusive` (D5 + CR-B).
//!
//! A schedule-internal, 1-byte tag carried in the same slot the old
//! `is_exclusive: bool` occupied, so `SystemBox` keeps its exact size /
//! field offsets. The dispatcher reads it once per system per round at the
//! apply-window gate; `runs_on_dispatcher()` is the single hot predicate.
//!
//! # The CpuExclusive axis is byte-identical to today's `is_exclusive`
//!
//! `CpuExclusive` is resolved from `access().is_universal()` exactly as
//! `is_exclusive` was (the widening to 3 values only ADDS the
//! `GpuCompute` marker carve-out — set by an explicit
//! `SystemDescriptor::is_gpu` flag, never derived from access). A CPU-only
//! world that mints no GPU system never produces a `GpuCompute`, so its
//! dispatch decisions are unchanged (the 0%-gate).
//!
//! # SCH15 (CR-B)
//!
//! `Schedule::run` keeps the SCH15 invariant an **equality** on the
//! CpuExclusive axis: `(kind == CpuExclusive) == access().is_universal()`
//! for every system. `GpuCompute` is the one marker-set variant that does
//! NOT imply universal access (it is Phase-5-scheduled), so it is asserted
//! separately. The withdrawn D5 "implication" form is NOT used.

/// Schedule-internal dispatch classification for one system (D5 + CR-B).
///
/// `#[repr(u8)]` so it occupies the exact 1-byte slot the previous
/// `is_exclusive: bool` did inside [`SystemBox`]; `SystemBox` keeps its
/// size, alignment, and field offsets.
///
/// [`SystemBox`]: super::super::schedule::system_box::SystemBox
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SystemKind {
    /// A normal CPU system that may run concurrently with other
    /// non-conflicting systems on a worker thread. Resolved when the
    /// system declares neither the GPU marker nor universal access. This
    /// is the overwhelming majority case — the discriminant is `0` so a
    /// zero-initialised tag is the cheap default.
    CpuConcurrent = 0,

    /// A CPU system that must run alone on the dispatcher inside the
    /// apply window (`running == 0`). Resolved iff `access().is_universal()`
    /// — byte-identical to the previous `is_exclusive` derivation. The
    /// conflict graph already serializes it (universal access conflicts
    /// with everything); EXC2 runs it solo.
    CpuExclusive = 1,

    /// A GPU-compute system that records/submits through the NonSend RHI,
    /// so it runs dispatcher-only (touchable only when `running == 0`). Set
    /// by an explicit `SystemDescriptor::is_gpu` marker, NOT derived from
    /// access — it carries no access constraint (Phase-5-scheduled). The
    /// conflict graph still orders it; [`runs_on_dispatcher`] forces it
    /// solo via EXC2.
    ///
    /// [`runs_on_dispatcher`]: SystemKind::runs_on_dispatcher
    GpuCompute = 2,
}

impl SystemKind {
    /// Returns `true` iff a system of this kind must run on the dispatcher
    /// thread inside the apply window (`running == 0`), never on a worker.
    ///
    /// Both [`CpuExclusive`](SystemKind::CpuExclusive) and
    /// [`GpuCompute`](SystemKind::GpuCompute) take the dispatcher path;
    /// [`CpuConcurrent`](SystemKind::CpuConcurrent) does not. This is a
    /// `matches!(.., 1 | 2)` — one range check, the same cost class as the
    /// old `is_exclusive` bool load.
    #[inline]
    pub(crate) fn runs_on_dispatcher(self) -> bool {
        matches!(self, SystemKind::CpuExclusive | SystemKind::GpuCompute)
    }
}

#[cfg(test)]
mod tests {
    use super::SystemKind;

    /// `runs_on_dispatcher` truth table — the load-bearing dispatch
    /// predicate. `CpuConcurrent` is the only worker-eligible kind.
    #[test]
    fn runs_on_dispatcher_truth_table() {
        assert!(
            !SystemKind::CpuConcurrent.runs_on_dispatcher(),
            "CpuConcurrent must be worker-eligible (off-dispatcher)"
        );
        assert!(
            SystemKind::CpuExclusive.runs_on_dispatcher(),
            "CpuExclusive must run on the dispatcher"
        );
        assert!(
            SystemKind::GpuCompute.runs_on_dispatcher(),
            "GpuCompute must run on the dispatcher"
        );
    }

    /// The discriminants are pinned to the plan's D5 values so any code that
    /// transmutes / matches on the raw byte stays in agreement.
    #[test]
    fn discriminants_match_plan() {
        assert_eq!(SystemKind::CpuConcurrent as u8, 0);
        assert_eq!(SystemKind::CpuExclusive as u8, 1);
        assert_eq!(SystemKind::GpuCompute as u8, 2);
    }

    /// `SystemKind` fits the 1-byte slot the previous `is_exclusive: bool`
    /// occupied so `SystemBox`'s layout is preserved.
    #[test]
    fn system_kind_is_one_byte() {
        assert_eq!(core::mem::size_of::<SystemKind>(), 1);
    }
}
