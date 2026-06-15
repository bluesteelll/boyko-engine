//! Phase 22 (D4) / 22.1 (Area A) — runtime archetype-level dynamic-tag query
//! terms.
//!
//! [`TagTerms`] is the per-view term list populated by
//! `Query::with_tag` / `Query::without_tag` (and the `QueryView` mirrors).
//! It is **stack-only, `Copy`, allocation-free** and serves as the *epoch
//! fingerprint* (stamp key) for the per-state-slot
//! [`TermList`](super::term_list::TermList) memo.
//!
//! # Cost contract (Phase 22.1)
//!
//! Terms resolve **once per epoch at the driver entry**, not per archetype
//! transition and never per row. The cursors and chunk/par drivers walk a
//! plain `&[ArchetypeId]` (either the shared pre-terms slice, or the memoised
//! term-filtered slice) and contain **zero** term code — the no-terms hot
//! path is byte-identical to the pre-Phase-22 code (asm-gated). The single
//! reusable [`archetype_passes_tag_terms`] test below backs three remaining
//! consumers: [`TermList::build`](super::term_list::TermList)'s per-id pass,
//! `QueryView::get` / `get_mut` (per-lookup random access — a prefilter
//! cannot help a single in-hand archetype), and
//! [`count_term_matched`] / [`any_term_matched`] (read-only count/any).
//!
//! # QS1 invariant
//!
//! Terms NEVER mutate the shared interned `QueryState` archetype-match cache
//! (it is shared across all instances of a `(D, F)` query type). The cache
//! stays term-agnostic; every accessor that exposes it carries the
//! `_pre_terms` suffix (see `query_state.rs`), and the term test below is the
//! single post-cache filter applied during the per-epoch prefilter build.

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

    /// Live-prefix equality — the epoch fingerprint compare used by the
    /// [`TermList`](super::term_list::TermList) memo fast path (Phase 22.1
    /// D-D).
    ///
    /// Compares `len`, `polarity`, and only the live `ids[..len]` prefix.
    /// The trailing `[len, MAX_DYN_TAG_TERMS)` slots are placeholders that
    /// `push` never reads; comparing only the live prefix is both correct
    /// today (trailing slots are always [`TagId(ComponentId(0))`]) and robust
    /// against any future mutation path that leaves stale data past `len`.
    #[inline]
    pub(crate) fn same(&self, other: &TagTerms) -> bool {
        if self.len != other.len || self.polarity != other.polarity {
            return false;
        }
        let len = self.len as usize;
        self.ids[..len] == other.ids[..len]
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

/// THE single term test (Phase 22.1 Area A). Three consumers only — NONE on
/// a row loop:
///
/// * [`TermList::build`](super::term_list::TermList) — one call per matched
///   archetype during the per-epoch prefilter build (cold, once per epoch).
/// * `QueryView::get` / `get_mut` — per-lookup on the single in-hand
///   archetype (random access; a prefilter slice cannot help a point query).
/// * [`count_term_matched`] / [`any_term_matched`] — read-only count/any.
///
/// The cursors (`QueryIter::next` / `QueryIterMut::next`) and the
/// chunk/par drivers no longer call this at all — they walk a pre-resolved
/// `&[ArchetypeId]` and carry zero term code (the Phase 22 F1 cold/inline
/// scan asymmetry is gone with the per-transition test that needed it).
///
/// ≤ [`MAX_DYN_TAG_TERMS`] signature bit tests against the archetype mask on
/// safe references — **zero unsafe**. `len == 0` short-circuits with one
/// predicted not-taken branch.
#[inline]
pub(crate) fn archetype_passes_tag_terms(terms: &TagTerms, arch: &Archetype) -> bool {
    if terms.len == 0 {
        return true;
    }
    term_scan_body(terms, arch)
}

/// The shared scan body — ≤ [`MAX_DYN_TAG_TERMS`] signature bit tests.
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
