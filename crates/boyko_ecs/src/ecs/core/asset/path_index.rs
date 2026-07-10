//! [`PathIndex`] — a HashMap-free path→`(slot, generation)` dedup index
//! (asset-streaming plan F4): a sorted-prefix + unsorted-tail [`VmColumn`],
//! replacing [`AssetServer`](crate::ecs::core::asset::server::AssetServer)'s
//! former `HashMap<(TypeId, String), (u32, u32)>` intern.
//!
//! # Append + merge — ZERO new unsafe
//!
//! [`VmColumn`] exposes no sorted-insert-at-index primitive (only
//! `push`/`set`/`swap_remove`/index reads) — inserting into the MIDDLE of a
//! sorted array would need an unsafe shift. This index sidesteps that
//! entirely: [`insert`](PathIndex::insert) always `push`es to an UNSORTED
//! tail; once the tail grows past [`MERGE_THRESHOLD`], the WHOLE index
//! (sorted prefix + tail) is re-sorted into a scratch `Vec` and written back
//! through [`VmColumn::set`] — no unsafe shift/insert-at primitive needed.
//! [`lookup`](PathIndex::lookup) binary-searches the sorted prefix, then
//! linearly scans the (small, bounded) unsorted tail.
//!
//! # Uniqueness invariant — `insert` is first-insert-wins
//!
//! `insert` no-ops if `hash` already resolves via [`lookup`](PathIndex::lookup)
//! — a hash is stored AT MOST once. This has two consequences: (1) `lookup`
//! never needs to disambiguate multiple matches for one hash (there is at
//! most one), and (2) [`merge`](PathIndex::merge)'s `sort_unstable_by_key`
//! never needs to preserve insertion order among ties (there are none, by
//! construction) — an unstable sort is exactly as correct as a stable one
//! here.
//!
//! # Debug-only collision guard
//!
//! A 64-bit hash collision between two DISTINCT real asset paths is
//! astronomically unlikely, but a silent one would alias two different
//! assets onto the same dedup entry — a genuine bug worth catching in
//! development. `debug_paths` (compiled out entirely in release: a plain
//! `Vec`, not a `HashMap`, and never touched outside `#[cfg(debug_assertions)]`)
//! remembers the source path behind each stored hash so `insert` can
//! `debug_assert!` that a hash it already holds was stored for the SAME path.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ecs::constants::pool_reserve_rows;
use crate::ecs::memory::vm_column::VmColumn;

/// Once a [`PathIndex`]'s unsorted append tail exceeds this many entries,
/// [`PathIndex::insert`] merges the whole index into the sorted prefix. Small
/// enough that [`PathIndex::lookup`]'s tail scan stays cheap; large enough
/// that the O(n log n) merge amortizes to a rare event.
const MERGE_THRESHOLD: usize = 32;

/// One path-hash → handle binding: a path's [`DefaultHasher`] digest paired
/// with the `(slot, generation)` pair the [`Handle`](crate::ecs::core::asset::handle::Handle)
/// minted for it. `#[repr(C)]`, 16 bytes, `Copy` — the element type
/// [`PathIndex`]'s backing [`VmColumn`] stores.
#[repr(C)]
#[derive(Clone, Copy)]
struct PathEntry {
    hash: u64,
    slot: u32,
    generation: u32,
}

/// Hashes `path` to the `u64` key [`PathIndex::lookup`]/[`PathIndex::insert`]
/// index on. A plain [`DefaultHasher`] (NOT a `HashMap`) — the index is
/// rebuilt fresh every process run, so cross-version hash stability is
/// irrelevant, and hashing a string through a `Hasher` is ordinary library
/// use, not a banned collection.
#[inline]
pub(crate) fn hash_path(path: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// A HashMap-free path→`(slot, generation)` dedup index (asset-streaming plan
/// F4). See the module doc for the append+merge design and the
/// first-insert-wins uniqueness invariant.
pub(crate) struct PathIndex {
    /// `[0, sorted_len)` is sorted ascending by `hash`; `[sorted_len,
    /// entries.len())` is the unsorted append tail [`insert`](Self::insert) grows.
    entries: VmColumn<PathEntry>,
    sorted_len: usize,
    /// Debug-only collision guard — see the module doc. Logically parallel to
    /// `entries` (one entry per stored hash), never read or written outside
    /// `debug_assertions`.
    #[cfg(debug_assertions)]
    debug_paths: Vec<(u64, String)>,
}

impl PathIndex {
    /// Creates an empty index. The backing `VmColumn`'s reservation follows
    /// the same byte-targeted, row-clamped sizing every other kernel column
    /// uses ([`pool_reserve_rows`]) — VA is free and commit stays lazy, so
    /// this costs nothing until the first [`insert`](Self::insert).
    pub(crate) fn new() -> Self {
        Self {
            entries: VmColumn::new("AssetPaths.index", pool_reserve_rows(size_of::<PathEntry>())),
            sorted_len: 0,
            #[cfg(debug_assertions)]
            debug_paths: Vec::new(),
        }
    }

    /// Looks up `hash`, returning the `(slot, generation)` pair stored for
    /// it, or `None` if `hash` was never inserted.
    ///
    /// Binary-searches the sorted prefix first (`O(log n)`), then linearly
    /// scans the small unsorted tail (`O(tail len)` ≤ [`MERGE_THRESHOLD`]).
    /// `insert`'s first-insert-wins invariant guarantees at most one stored
    /// entry per hash, so there is never an ambiguous "first match" to pick
    /// among.
    #[inline]
    pub(crate) fn lookup(&self, hash: u64) -> Option<(u32, u32)> {
        let all = self.entries.as_slice();
        let prefix = &all[..self.sorted_len];
        if let Ok(i) = prefix.binary_search_by_key(&hash, |entry| entry.hash) {
            let entry = prefix[i];
            return Some((entry.slot, entry.generation));
        }
        let tail = &all[self.sorted_len..];
        tail.iter().find(|entry| entry.hash == hash).map(|entry| (entry.slot, entry.generation))
    }

    /// Records `hash` → `(slot, generation)` for `path`, appending to the
    /// unsorted tail and merging into the sorted prefix once the tail exceeds
    /// [`MERGE_THRESHOLD`].
    ///
    /// A no-op (first-insert-wins) if `hash` already resolves via
    /// [`lookup`](Self::lookup) — see the module doc's uniqueness invariant.
    /// In a debug build, a hash already stored for a DIFFERENT `path` trips a
    /// `debug_assert!` (see the module doc's collision guard).
    pub(crate) fn insert(&mut self, path: &str, hash: u64, slot: u32, generation: u32) {
        if self.lookup(hash).is_some() {
            // `debug_paths` is `#[cfg(debug_assertions)]`-only, so the whole
            // check (whose body reads `self.debug_paths`) must itself be
            // gated, or a release build fails to resolve the field.
            #[cfg(debug_assertions)]
            if let Some((_, existing_path)) = self.debug_paths.iter().find(|(h, _)| *h == hash) {
                debug_assert_eq!(
                    existing_path,
                    path,
                    "PathIndex: 64-bit hash collision between distinct paths {existing_path:?} \
                     and {path:?} (hash {hash:#x}) — the second path would silently alias the \
                     first path's handle"
                );
            }
            return;
        }
        #[cfg(debug_assertions)]
        self.debug_paths.push((hash, path.to_owned()));
        self.entries.push(PathEntry { hash, slot, generation });
        if self.entries.len() - self.sorted_len > MERGE_THRESHOLD {
            self.merge();
        }
    }

    /// Re-sorts the WHOLE index (sorted prefix + unsorted tail) by `hash`
    /// into a scratch `Vec`, then writes it back through [`VmColumn::set`] —
    /// no unsafe shift/insert-at primitive needed (see the module doc).
    ///
    /// `sort_unstable_by_key` is sound here despite being unstable:
    /// `insert`'s first-insert-wins invariant guarantees no two entries share
    /// a hash, so there are no ties whose relative order would need
    /// preserving.
    fn merge(&mut self) {
        let len = self.entries.len();
        let mut scratch: Vec<PathEntry> = self.entries.as_slice().to_vec();
        scratch.sort_unstable_by_key(|entry| entry.hash);
        for (i, entry) in scratch.into_iter().enumerate() {
            self.entries.set(i, entry);
        }
        self.sorted_len = len;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;

    /// A deterministic path string for logical key `i` — used to keep `path`
    /// and `hash` 1:1 throughout the tests below (the debug collision guard
    /// would trip if the SAME hash were ever re-inserted under a DIFFERENT
    /// path; see [`insert_debug_asserts_on_hash_collision_between_distinct_paths`]
    /// for the one test that deliberately forces that mismatch).
    fn path_for(key: usize) -> String {
        format!("proptest/path/{key}")
    }

    /// A hash not present in either the sorted prefix or the unsorted tail
    /// must resolve to `None`, not panic or return a spurious match (plan §F4
    /// unit: lookup miss).
    #[test]
    fn lookup_returns_none_for_a_hash_never_inserted() {
        let index = PathIndex::new();
        assert_eq!(index.lookup(0xDEAD_BEEF), None, "an empty index has nothing to find");
    }

    /// A freshly inserted hash resolves immediately, entirely from the
    /// unsorted tail — `sorted_len` is still `0` (plan §F4 unit: insert then
    /// lookup happy path, tail-only).
    #[test]
    fn insert_then_lookup_returns_the_stored_slot_and_generation() {
        let mut index = PathIndex::new();
        index.insert("meshes/cube.gltf", 7, 3, 1);
        assert_eq!(index.lookup(7), Some((3, 1)), "the just-inserted entry must resolve");
    }

    /// A hash inserted AFTER the last merge lives ONLY in the unsorted tail
    /// (never touched by `sort_unstable_by_key`) — `lookup` must still find
    /// it via its linear tail scan. This is a targeted regression guard: a
    /// `lookup` that only binary-searched the sorted prefix and forgot the
    /// tail scan would silently miss exactly this case.
    #[test]
    fn lookup_finds_hash_inserted_after_the_last_merge_in_the_unsorted_tail() {
        let mut index = PathIndex::new();

        // Push MERGE_THRESHOLD + 1 distinct entries: the (MERGE_THRESHOLD +
        // 1)-th push makes the tail length exceed MERGE_THRESHOLD, firing
        // exactly one merge and resetting `sorted_len` to the whole index.
        for key in 0..=MERGE_THRESHOLD {
            index.insert(&path_for(key), key as u64, key as u32, 0);
        }
        assert_eq!(index.sorted_len, MERGE_THRESHOLD + 1, "one merge must have fired by this point");

        // This entry is pushed strictly AFTER the merge above — it can only
        // ever be found via the unsorted-tail scan, never the sorted prefix.
        let tail_hash = 9_000u64;
        index.insert("post-merge/only-in-tail.bin", tail_hash, 777, 5);

        assert_eq!(
            index.lookup(tail_hash),
            Some((777, 5)),
            "a hash inserted after the last merge (living only in the unsorted tail) must be found"
        );
    }

    /// First-insert-wins holds even once the winning entry has migrated into
    /// the sorted prefix via a merge: insert hash `H` -> slot A, force a
    /// merge, insert the SAME hash `H` -> slot B, and `lookup` must still
    /// report A. Guards against a merge accidentally re-keying on the most
    /// RECENT rather than the FIRST write.
    #[test]
    fn insert_first_insert_wins_holds_across_a_merge() {
        let mut index = PathIndex::new();
        let winner_path = "winner/path.bin";
        let h = 42u64;
        index.insert(winner_path, h, 100, 0);

        // Force a merge with MERGE_THRESHOLD + 1 OTHER distinct entries — `h`
        // itself is not among them, so it does not no-op away this push. The
        // merge fires once total pushed entries reach MERGE_THRESHOLD + 1
        // (h + the first MERGE_THRESHOLD fillers); the loop's final filler
        // lands in a fresh tail afterward, which is irrelevant here — what
        // matters is that `h` (pushed first) is now inside the sorted prefix.
        for key in 0..=MERGE_THRESHOLD {
            index.insert(&path_for(key), 1_000 + key as u64, key as u32, 0);
        }
        assert_eq!(
            index.sorted_len,
            MERGE_THRESHOLD + 1,
            "one merge must have fired, migrating h's entry into the sorted prefix"
        );

        // Second insert for the SAME hash (and, per the uniqueness invariant,
        // the SAME path — see the module doc) with a DIFFERENT slot/gen.
        index.insert(winner_path, h, 200, 9);

        assert_eq!(
            index.lookup(h),
            Some((100, 0)),
            "first-insert-wins must hold even after the winning entry moved into the sorted prefix"
        );
    }

    /// Two DISTINCT paths forced to share the SAME hash trip the debug-only
    /// collision guard (module doc: "Debug-only collision guard") — reachable
    /// directly through the `(path, hash)` pair `insert` accepts, with no
    /// need for an actual `DefaultHasher` collision.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "hash collision")]
    fn insert_debug_asserts_on_hash_collision_between_distinct_paths() {
        let mut index = PathIndex::new();
        let collision_hash = 555u64;
        index.insert("path/one.bin", collision_hash, 1, 0);
        index.insert("path/two.bin", collision_hash, 2, 0);
    }

    /// One randomized operation against [`PathIndex`]: either an `insert` of
    /// a logical key (with a random candidate slot/generation payload) or a
    /// `lookup` of one. `key` indexes into the proptest's precomputed
    /// `paths`/`hashes` tables, keeping every hash tied to exactly one path
    /// string throughout (see [`path_for`]).
    #[derive(Debug, Clone)]
    enum Op {
        Insert { key: usize, slot: u32, generation: u32 },
        Lookup { key: usize },
    }

    fn op_strategy(num_keys: usize) -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (0..num_keys, any::<u32>(), any::<u32>())
                .prop_map(|(key, slot, generation)| Op::Insert { key, slot, generation }),
            2 => (0..num_keys).prop_map(|key| Op::Lookup { key }),
        ]
    }

    /// The full key space the randomized phase draws from — several times
    /// wider than [`PREAMBLE_KEYS`] so plenty of the randomized `Insert`s
    /// mint genuinely NEW distinct hashes (triggering further merges live
    /// during the randomized phase), not just no-op repeats of the preamble's
    /// keys.
    const TOTAL_KEYS: usize = 400;

    /// Keys inserted deterministically up front, before the randomized
    /// phase — 5x [`MERGE_THRESHOLD`], guaranteeing several merges fire
    /// before a single randomized op runs (rather than leaving "did we cross
    /// the merge boundary at all" to chance).
    const PREAMBLE_KEYS: usize = 5 * MERGE_THRESHOLD;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// A `BTreeMap<hash, (slot, generation)>` under first-insert-wins is
        /// the ground-truth oracle for [`PathIndex`]: every `insert` maps to
        /// `model.entry(hash).or_insert(..)` (matching `insert`'s no-op-on-
        /// existing-hash policy exactly), and after EVERY op (insert or
        /// lookup), `PathIndex::lookup` must agree with the model.
        ///
        /// The deterministic preamble (`PREAMBLE_KEYS` = 5x
        /// `MERGE_THRESHOLD`) forces several merges before the randomized
        /// phase begins, leaving a real mix of sorted-prefix-resident and
        /// unsorted-tail-resident entries for the randomized ops to probe.
        /// The randomized phase itself then draws keys from the WIDER
        /// `TOTAL_KEYS` range, so a good share of its `Insert`s mint fresh
        /// distinct hashes — driving further merges live, interleaved with
        /// `Lookup`s that hit both old (merged) and brand-new (tail) entries.
        #[test]
        fn lookup_matches_btreemap_oracle_across_merges(
            ops in prop::collection::vec(op_strategy(TOTAL_KEYS), 300..700)
        ) {
            let mut index = PathIndex::new();
            let mut model: BTreeMap<u64, (u32, u32)> = BTreeMap::new();
            let paths: Vec<String> = (0..TOTAL_KEYS).map(path_for).collect();
            let hashes: Vec<u64> = paths.iter().map(|p| hash_path(p)).collect();

            for key in 0..PREAMBLE_KEYS {
                index.insert(&paths[key], hashes[key], key as u32, 0);
                model.entry(hashes[key]).or_insert((key as u32, 0));
            }
            prop_assert!(
                index.sorted_len >= PREAMBLE_KEYS - MERGE_THRESHOLD,
                "the preamble (5x MERGE_THRESHOLD distinct inserts) must have forced multiple merges; \
                 sorted_len = {}, PREAMBLE_KEYS = {}",
                index.sorted_len,
                PREAMBLE_KEYS
            );
            for (key, &h) in hashes.iter().take(PREAMBLE_KEYS).enumerate() {
                prop_assert_eq!(
                    index.lookup(h),
                    model.get(&h).copied(),
                    "preamble mismatch for key {}",
                    key
                );
            }

            for op in ops {
                match op {
                    Op::Insert { key, slot, generation } => {
                        index.insert(&paths[key], hashes[key], slot, generation);
                        model.entry(hashes[key]).or_insert((slot, generation));
                        prop_assert_eq!(
                            index.lookup(hashes[key]),
                            model.get(&hashes[key]).copied(),
                            "post-insert mismatch for key {}",
                            key
                        );
                    }
                    Op::Lookup { key } => {
                        prop_assert_eq!(
                            index.lookup(hashes[key]),
                            model.get(&hashes[key]).copied(),
                            "lookup mismatch for key {}",
                            key
                        );
                    }
                }
            }

            // Final full sweep across the whole key space — catches any
            // entry the interleaved op stream never happened to re-check.
            for (key, &h) in hashes.iter().take(TOTAL_KEYS).enumerate() {
                prop_assert_eq!(
                    index.lookup(h),
                    model.get(&h).copied(),
                    "final sweep mismatch for key {}",
                    key
                );
            }
        }
    }
}
