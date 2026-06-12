//! Phase 22 (D4) — runtime archetype-level dynamic-tag query terms.
//!
//! [`TagTerms`] is the per-view term list populated by
//! `Query::with_tag` / `Query::without_tag` (and the `QueryView` mirrors).
//! It is **stack-only, `Copy`, allocation-free** and is threaded by value /
//! `&TagTerms` through every matched-list driver (the D4 disposition table).
//!
//! # Cost contract (plan D4)
//!
//! `len == 0` costs **one predicted not-taken branch per archetype
//! transition** — outside the row loop, never per row. The inner row loop of
//! every driver must remain byte-identical to the pre-Phase-22 code
//! (asm-gated in Wave 3).
//!
//! # QS1 invariant
//!
//! Terms NEVER mutate the shared interned `QueryState` archetype-match cache
//! (it is shared across all instances of a `(D, F)` query type). The cache
//! stays term-agnostic; every accessor that exposes it carries the
//! `_pre_terms` suffix (see `query_state.rs`), and the term test below is the
//! single post-cache filter applied at each driver's archetype-transition
//! point.

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_registry::TagId;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Maximum number of dynamic-tag terms (`with_tag` + `without_tag` combined)
/// a single query view may carry. Exceeding it is a **loud release panic at
/// term-add time** (setup-time, cold) — never a silent truncation.
///
/// Cheap to raise (the storage is stack-only); 8 covers every anticipated
/// use (plan open question 2).
pub const MAX_DYN_TAG_TERMS: usize = 8;

/// Runtime archetype-level tag terms (Phase 22 D4).
///
/// Carried by BOTH `Query<D, F>` (SystemParam) and `QueryView<D, F>` (direct
/// API); never stored in the shared interned `QueryState` (QS1 stays
/// term-agnostic). `polarity` bit `i` is `1` for a `with` term and `0` for a
/// `without` term over `ids[i]`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TagTerms {
    /// Term tag ids; only `[0, len)` is meaningful.
    ids: [TagId; MAX_DYN_TAG_TERMS],
    /// Bit `i`: `1` = with (tag must be present), `0` = without (absent).
    polarity: u8,
    /// Number of live terms; `<= MAX_DYN_TAG_TERMS`.
    len: u8,
}

impl TagTerms {
    /// The no-terms value — the state of every freshly minted query view.
    pub(crate) const EMPTY: Self = Self {
        // Slot 0 is a placeholder, never read: only `[0, len)` is meaningful.
        ids: [TagId(ComponentId(0)); MAX_DYN_TAG_TERMS],
        polarity: 0,
        len: 0,
    };

    /// `true` when no terms are set — the byte-identical fast-path gate every
    /// driver checks once per archetype transition.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of live terms.
    #[inline]
    #[allow(dead_code)] // symmetry with is_empty; consumed by debug asserts/tests
    pub(crate) fn len(&self) -> usize {
        self.len as usize
    }

    /// Appends a `with` term (archetype must carry `tag`).
    ///
    /// # Panics
    /// Release-active panic past [`MAX_DYN_TAG_TERMS`] terms (setup-time).
    #[inline]
    pub(crate) fn push_with(&mut self, tag: TagId) {
        self.push(tag, true);
    }

    /// Appends a `without` term (archetype must NOT carry `tag`).
    ///
    /// # Panics
    /// Release-active panic past [`MAX_DYN_TAG_TERMS`] terms (setup-time).
    #[inline]
    pub(crate) fn push_without(&mut self, tag: TagId) {
        self.push(tag, false);
    }

    fn push(&mut self, tag: TagId, with: bool) {
        let idx = self.len as usize;
        if idx >= MAX_DYN_TAG_TERMS {
            tag_terms_overflow_panic();
        }
        self.ids[idx] = tag;
        if with {
            self.polarity |= 1 << idx;
        }
        self.len += 1;
    }
}

/// THE single term test (plan D4) — every matched-list driver except
/// `QueryIterMut::next` (see the F1 exception below) calls this at its
/// archetype-transition point, enforced by the `_pre_terms` rename sweep
/// over ALL `QueryState` matched-list accessors.
///
/// ≤ [`MAX_DYN_TAG_TERMS`] signature bit tests against the archetype mask on
/// safe references — **zero unsafe**. `len == 0` short-circuits with one
/// predicted not-taken branch.
///
/// # Phase 22 F1 (I-cache)
///
/// Only the `len == 0` test is inlined at the call sites; the scan body is
/// `#[cold]`-outlined in [`term_scan_cold`]. Inlining the scan loop into
/// `QueryIter::next` pushed that body past LLVM's inline threshold —
/// `next()` stopped inlining into caller loops and `query_ref_iter_10k`
/// regressed +247% (asm-verified). The split keeps the no-terms cost
/// contract (one branch) byte-identical and moves the term-bearing scan off
/// the callers' inline budget.
///
/// `QueryIterMut::next` is the ONE exception — it must use
/// [`archetype_passes_tag_terms_inline_scan`] instead (see its doc).
#[inline]
pub(crate) fn archetype_passes_tag_terms(terms: &TagTerms, arch: &Archetype) -> bool {
    if terms.len == 0 {
        return true;
    }
    term_scan_cold(terms, arch)
}

/// Inline-scan variant of [`archetype_passes_tag_terms`] — used ONLY by
/// `QueryIterMut::next` (Phase 22 F1).
///
/// The mut cursor's monomorphisations sit on the opposite side of LLVM's
/// inline threshold from the read-only cursor's: `next()` stays inlined even
/// with the scan body inline, but a reachable `call` inside its loop nest
/// (the `#[cold] #[inline(never)]` scan) forces the register allocator to
/// spill the row cursor (`current_row` / `current_len` / fetch base) to the
/// stack — `query_mut_iter_10k` +47% with the cold call vs +26% with the
/// inline scan (criterion-verified, same session, p = 0.00). Do not "unify"
/// the two variants without re-running that A/B.
#[inline]
pub(crate) fn archetype_passes_tag_terms_inline_scan(
    terms: &TagTerms,
    arch: &Archetype,
) -> bool {
    if terms.len == 0 {
        return true;
    }
    term_scan_body(terms, arch)
}

/// Cold term-scan wrapper — runs only on term-bearing query views: once per
/// archetype transition on the iteration drivers, once per lookup on
/// `QueryView::get`/`get_mut` (their term test guards a single random-access
/// row, not a loop). Outlined so the inline cost of
/// [`archetype_passes_tag_terms`] at every driver's transition point is one
/// compare-and-branch (see the F1 note above).
#[cold]
#[inline(never)]
fn term_scan_cold(terms: &TagTerms, arch: &Archetype) -> bool {
    term_scan_body(terms, arch)
}

/// The shared scan body — `caller` decides inline vs cold-outlined dispatch
/// (see [`archetype_passes_tag_terms`] / [`term_scan_cold`]).
#[inline]
fn term_scan_body(terms: &TagTerms, arch: &Archetype) -> bool {
    let len = terms.len as usize;
    for (i, tag) in terms.ids[..len].iter().enumerate() {
        let want = (terms.polarity >> i) & 1 != 0;
        if arch.has_component_id(tag.component_id()) != want {
            return false;
        }
    }
    true
}

/// Term-filtered archetype count over a pre-terms matched-id list — backs the
/// term-aware `Query::archetype_count` / `QueryView::archetype_count` paths.
///
/// Archetype-level membership only (no `entity_count` consultation — same
/// semantics as the no-terms path). Stale ids (archetypes removed after the
/// state's last sync) do not count: the term test needs the live signature,
/// so liveness is consulted on this (term-bearing, cold-ish) path only.
#[inline]
pub(crate) fn count_term_matched(
    terms: &TagTerms,
    master: &ArchetypeMaster,
    ids: &[ArchetypeId],
) -> usize {
    ids.iter()
        .filter(|&&id| {
            master
                .get_archetype(id)
                .is_some_and(|arch| archetype_passes_tag_terms(terms, arch))
        })
        .count()
}

/// Short-circuiting "any archetype passes the terms" — backs the term-aware
/// `is_empty` paths. Same membership semantics as [`count_term_matched`].
#[inline]
pub(crate) fn any_term_matched(
    terms: &TagTerms,
    master: &ArchetypeMaster,
    ids: &[ArchetypeId],
) -> bool {
    ids.iter().any(|&id| {
        master
            .get_archetype(id)
            .is_some_and(|arch| archetype_passes_tag_terms(terms, arch))
    })
}

/// Cold panic site for the >[`MAX_DYN_TAG_TERMS`] overflow — loud, release-
/// active, fired at term-add time (setup), never on the iteration hot path.
#[cold]
#[inline(never)]
fn tag_terms_overflow_panic() -> ! {
    panic!(
        "with_tag/without_tag: more than MAX_DYN_TAG_TERMS = {MAX_DYN_TAG_TERMS} dynamic-tag \
         terms on a single query; raise the constant if the use case is real"
    );
}
