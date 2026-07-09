//! Subset-clash resolution (plan §6, Decision 8).
//!
//! `PrioritizeLongest`: if active binding A's key-set is a strict subset of
//! active binding B's key-set, A's action is suppressed (`Ctrl+S` suppresses
//! bare `S`). O(active²) over only the *active* chord/key candidates (typically
//! < 10), off every hot loop, no allocation (works over fixed-size stack
//! buffers).

use crate::action::map::BindSpec;
use crate::constants::{CLASH_LIMIT, MAX_CHORD_KEYS};
use crate::raw::keycode::KeyCode;

/// One active button-like candidate considered by the clash pass: the action it
/// drives and the sorted set of keys it requires.
#[derive(Clone, Copy)]
pub(crate) struct Candidate {
    /// Dense action index this binding belongs to.
    pub action: usize,
    /// The keys this binding requires (only `len` are valid), sorted ascending
    /// by raw discriminant so subset testing is a merge walk.
    pub keys: [u16; MAX_CHORD_KEYS],
    pub len: u8,
}

/// A fixed-capacity stack collector for clash candidates — no heap allocation.
pub(crate) struct CandidateSet {
    items: [Candidate; CLASH_LIMIT],
    len: usize,
}

impl CandidateSet {
    #[inline]
    pub(crate) fn new() -> Self {
        let empty = Candidate {
            action: 0,
            keys: [0u16; MAX_CHORD_KEYS],
            len: 0,
        };
        Self {
            items: [empty; CLASH_LIMIT],
            len: 0,
        }
    }

    /// Pushes the key-set of an active button/chord binding. Composite axes and
    /// the reserved/none variants carry no clash key-set and are ignored. Over
    /// `CLASH_LIMIT` the push is dropped (debug-asserted).
    #[inline]
    pub(crate) fn push_active(&mut self, action: usize, spec: &BindSpec) {
        let mut keys = [0u16; MAX_CHORD_KEYS];
        let len: u8 = match spec {
            BindSpec::Key(code) => {
                keys[0] = encode_key(*code);
                1
            }
            BindSpec::Chord { keys: ck, len } => {
                let n = (*len as usize).min(MAX_CHORD_KEYS);
                for (dst, src) in keys.iter_mut().zip(ck.iter()).take(n) {
                    *dst = encode_key(*src);
                }
                // Sort the valid prefix so subset testing is a merge walk.
                keys[..n].sort_unstable();
                n as u8
            }
            // Mouse buttons share no key-set domain with keyboard chords; v1
            // clash resolution operates on keyboard key-sets only (matches the
            // `Ctrl+S` vs `S` motivating case). Other variants carry no set.
            BindSpec::Mouse(_)
            | BindSpec::Axis1 { .. }
            | BindSpec::Axis2 { .. }
            | BindSpec::Stick
            | BindSpec::None => return,
        };

        debug_assert!(
            self.len < CLASH_LIMIT,
            "clash candidate set exceeded CLASH_LIMIT ({CLASH_LIMIT})"
        );
        if self.len >= CLASH_LIMIT {
            return;
        }
        self.items[self.len] = Candidate { action, keys, len };
        self.len += 1;
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[Candidate] {
        &self.items[..self.len]
    }
}

impl Default for CandidateSet {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes a `KeyCode` into a `u16` key id for set comparison. Canonical keys
/// use their dense index; `Unidentified` keys use a high-range id derived from
/// the low payload bits so they compare distinctly without colliding with
/// canonical ids.
#[inline]
fn encode_key(code: KeyCode) -> u16 {
    match code.dense_index() {
        Some(i) => i as u16,
        // Canonical ids are < CANONICAL_KEY_COUNT (<= 256); push exotic keys
        // into the high half of the u16 space to avoid collision. The low 15
        // bits of the raw scancode disambiguate them for clash comparison.
        None => {
            if let KeyCode::Unidentified(raw) = code {
                0x8000u16 | (raw as u16 & 0x7FFF)
            } else {
                // Unreachable: `dense_index` returns `None` only for
                // `Unidentified`.
                0x8000u16
            }
        }
    }
}

/// Returns `true` iff `sub`'s key-set is a **strict** subset of `sup`'s.
/// Both key arrays are sorted ascending; this is a linear merge walk.
#[inline]
fn is_strict_subset(sub: &Candidate, sup: &Candidate) -> bool {
    if sub.len >= sup.len {
        return false;
    }
    let sub_keys = &sub.keys[..sub.len as usize];
    let sup_keys = &sup.keys[..sup.len as usize];
    let mut j = 0usize;
    for &k in sub_keys {
        // Advance `j` until we find `k` in the superset (sorted merge).
        while j < sup_keys.len() && sup_keys[j] < k {
            j += 1;
        }
        if j >= sup_keys.len() || sup_keys[j] != k {
            return false;
        }
        j += 1;
    }
    true
}

/// Runs `PrioritizeLongest` suppression over the active candidate set, invoking
/// `suppress(action_index)` for every candidate whose key-set is a strict
/// subset of another active candidate's key-set.
///
/// O(active²); `active` is the count of active keyboard bindings this frame
/// (typically < 10). No allocation.
#[inline]
pub(crate) fn resolve_prioritize_longest(
    set: &CandidateSet,
    mut suppress: impl FnMut(usize),
) {
    let items = set.as_slice();
    for (i, sub) in items.iter().enumerate() {
        for (k, sup) in items.iter().enumerate() {
            if i == k {
                continue;
            }
            if is_strict_subset(sub, sup) {
                suppress(sub.action);
                break;
            }
        }
    }
}
