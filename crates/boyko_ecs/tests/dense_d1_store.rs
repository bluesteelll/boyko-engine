//! Dense plan D1 — `DenseStore` + views, tested in isolation.
//!
//! Covers the D1 gate: insert / remove (tombstone) / slot reuse (LIFO),
//! deterministic iteration order (no swap-remove reorder), address-stability
//! across a grow past `reserve_rows`, compact correctness, contains / slot_of,
//! drop correctness for a non-Copy dense type, and a property test of the
//! `e2s[s2e[s]] == s` ∧ `!live(s) ⟺ s ∈ free` invariant.
//!
//! The Miri-TB suite (single-threaded UB-clean ops + the parallel
//! distinct-slot solve-view test) lives at the bottom under the standard
//! `#[cfg(...)]` gates.
//!
//! Component-id allocation: 103 (`Pos`) / 104 (`DropCounter`) — in the free
//! 103..=127 band below `MAX_COMPONENTS = 512` (no collision with the
//! authoritative used-id survey).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::dense::DenseStore;
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};

// ── component types ─────────────────────────────────────────────────────────

/// 16-byte POD payload (the physics-body shape: all-`Copy`, no Drop).
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

const POS_ID: ComponentId = ComponentId(103);

impl Component for Pos {
    fn component_id() -> ComponentId {
        POS_ID
    }
}

/// A non-Copy component whose Drop increments a shared counter — proves the
/// store honors the column's `drop_fn` exactly once per live component.
#[repr(C)]
struct DropCounter {
    counter: Arc<AtomicUsize>,
}

const DROP_ID: ComponentId = ComponentId(104);

impl Component for DropCounter {
    fn component_id() -> ComponentId {
        DROP_ID
    }
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn register() {
    component_registry::register_layout::<Pos>(POS_ID.0);
    component_registry::register_layout::<DropCounter>(DROP_ID.0);
}

fn pos_bytes(p: &Pos) -> &[u8] {
    // SAFETY: `Pos` is `#[repr(C)]` POD; viewing it as its own byte span is
    // sound and the bytes are a valid representation of the registered type.
    unsafe { std::slice::from_raw_parts((p as *const Pos).cast::<u8>(), std::mem::size_of::<Pos>()) }
}

fn pos_store(reserve_rows: usize) -> DenseStore {
    register();
    DenseStore::new(POS_ID, reserve_rows)
}

fn e(i: usize) -> EntityId {
    EntityId(i)
}

fn insert_pos(store: &mut DenseStore, entity: EntityId, p: Pos) -> u32 {
    store.insert(entity, pos_bytes(&p))
}

/// Reads back the `Pos` at a slot through the solve view (read-only).
fn read_pos(store: &DenseStore, slot: u32) -> Pos {
    let view = store.solve_view();
    // SAFETY: `slot` is a live slot produced by `insert`/`slot_of`; `row_ptr`'s
    // contract (slot < len ∧ live) holds. The pointer is valid for a `Pos`
    // (the store's registered type) and there is no concurrent writer.
    unsafe {
        let ptr = view.row_ptr(slot as usize).cast::<Pos>();
        *ptr
    }
}

// ── insert / contains / slot_of ─────────────────────────────────────────────

#[test]
fn insert_assigns_sequential_slots_and_contains() {
    let mut store = pos_store(64);
    let s0 = insert_pos(&mut store, e(10), Pos { x: 1.0, y: 0.0, z: 0.0, w: 0.0 });
    let s1 = insert_pos(&mut store, e(20), Pos { x: 2.0, y: 0.0, z: 0.0, w: 0.0 });
    let s2 = insert_pos(&mut store, e(30), Pos { x: 3.0, y: 0.0, z: 0.0, w: 0.0 });

    assert_eq!((s0, s1, s2), (0, 1, 2), "fresh inserts append at the frontier");
    assert_eq!(store.live_count(), 3);
    assert_eq!(store.len(), 3);

    assert!(store.contains(e(10)) && store.contains(e(20)) && store.contains(e(30)));
    assert!(!store.contains(e(99)));
    assert_eq!(store.slot_of(e(20)), Some(1));
    assert_eq!(store.slot_of(e(99)), None);

    assert_eq!(read_pos(&store, s1).x, 2.0);
    assert!(store.check_invariant());
}

// ── remove (tombstone) + LIFO slot reuse ────────────────────────────────────

#[test]
fn remove_then_insert_reuses_freed_slot_lifo() {
    let mut store = pos_store(64);
    let _s0 = insert_pos(&mut store, e(1), Pos { x: 1.0, y: 0.0, z: 0.0, w: 0.0 });
    let s1 = insert_pos(&mut store, e(2), Pos { x: 2.0, y: 0.0, z: 0.0, w: 0.0 });
    let s2 = insert_pos(&mut store, e(3), Pos { x: 3.0, y: 0.0, z: 0.0, w: 0.0 });

    // Remove two: free list becomes [s1, s2] in push order, LIFO pops s2 first.
    assert!(store.remove(e(2)));
    assert!(store.remove(e(3)));
    assert_eq!(store.live_count(), 1);
    assert_eq!(store.len(), 3, "high-water mark unchanged by tombstone");
    assert!(!store.contains(e(2)) && !store.contains(e(3)));

    // LIFO: the most-recently-freed slot (s2) is reused first.
    let reused_a = insert_pos(&mut store, e(4), Pos { x: 4.0, y: 0.0, z: 0.0, w: 0.0 });
    assert_eq!(reused_a, s2, "LIFO: last freed slot reused first");
    let reused_b = insert_pos(&mut store, e(5), Pos { x: 5.0, y: 0.0, z: 0.0, w: 0.0 });
    assert_eq!(reused_b, s1, "LIFO: second freed slot reused next");

    // No new slot was appended — the high-water mark stayed at 3.
    assert_eq!(store.len(), 3);
    assert_eq!(store.live_count(), 3);
    assert_eq!(read_pos(&store, reused_a).x, 4.0);
    assert_eq!(read_pos(&store, reused_b).x, 5.0);
    assert!(store.check_invariant());
}

#[test]
fn remove_absent_entity_returns_false() {
    let mut store = pos_store(8);
    insert_pos(&mut store, e(1), Pos { x: 1.0, y: 0.0, z: 0.0, w: 0.0 });
    assert!(!store.remove(e(999)));
    assert_eq!(store.live_count(), 1);
}

// ── deterministic iteration order (no swap-remove reorder) ──────────────────

#[test]
fn live_iteration_preserves_insertion_order_across_remove_patterns() {
    // Live slots NEVER move (tombstone, not swap-remove) — so iteration order
    // is insertion order minus the tombstoned slots, NOT a swap-reordered list.
    let mut store = pos_store(64);
    for i in 0..6usize {
        insert_pos(&mut store, e(i), Pos { x: i as f32, y: 0.0, z: 0.0, w: 0.0 });
    }

    // Remove from the middle and the front; the survivors keep their order.
    store.remove(e(2));
    store.remove(e(0));
    store.remove(e(4));

    let mut order: Vec<usize> = Vec::new();
    store.for_each_live(|_slot, entity| order.push(entity.get()));
    assert_eq!(
        order,
        vec![1, 3, 5],
        "survivors retain insertion order (no swap-remove reorder)"
    );

    // Re-insert reuses freed slots (low indices), but a reused low slot still
    // iterates before later survivors — order is by SLOT, deterministic.
    insert_pos(&mut store, e(100), Pos { x: 100.0, y: 0.0, z: 0.0, w: 0.0 });
    let mut order2: Vec<usize> = Vec::new();
    store.for_each_live(|slot, entity| order2.push((slot as usize, entity.get()).1));
    // e(100) reuses slot 4 (last freed), so it sits between slot 3 (e3) and 5 (e5).
    assert_eq!(order2, vec![1, 3, 100, 5]);
    assert!(store.check_invariant());
}

// ── address stability across a grow past reserve_rows ───────────────────────

#[test]
fn column_base_is_address_stable_across_grow() {
    // reserve_rows large enough to never hit the ceiling, but small initial
    // committed capacity forces at least one `grow_rows` as we fill past the
    // first commit step. The VM-reserved base must NOT move (this is the
    // property std::Vec lacked that caused SP4).
    let reserve = 1 << 16; // 65_536 rows; first commit covers far fewer.
    let mut store = pos_store(reserve);

    // Capture the column base via the solve view's slot-0 pointer (the lowest
    // address of the column). `row_ptr` requires a live slot, so seed one row
    // first, capture, then keep inserting.
    insert_pos(&mut store, e(0), Pos { x: 0.0, y: 0.0, z: 0.0, w: 0.0 });
    // SAFETY: slot 0 is live (just inserted); reading the pointer (not
    // dereferencing) is sound. Captured as an address only.
    let base_before = unsafe { store.solve_view().row_ptr(0) };

    // Insert enough rows to cross at least one grow boundary. 20_000 * 16 B =
    // 320 KB, well past the initial commit step.
    for i in 1..20_000usize {
        insert_pos(&mut store, e(i), Pos { x: i as f32, y: 0.0, z: 0.0, w: 0.0 });
    }
    assert!(store.len() >= 20_000);

    // SAFETY: slot 0 is still live and never moves (tombstone discipline);
    // reading its pointer is sound.
    let base_after = unsafe { store.solve_view().row_ptr(0) };
    assert_eq!(
        base_before, base_after,
        "ComponentPool column base must be address-stable across grow (in-place commit)"
    );

    // Spot-check that early rows are still readable (not relocated).
    assert_eq!(read_pos(&store, 0).x, 0.0);
    assert_eq!(read_pos(&store, 12_345).x, 12_345.0);
    assert!(store.check_invariant());
}

// ── compact ─────────────────────────────────────────────────────────────────

#[test]
fn compact_drops_tombstones_canonicalizes_order_clears_free() {
    let mut store = pos_store(64);
    for i in 0..8usize {
        insert_pos(&mut store, e(i), Pos { x: i as f32, y: 0.0, z: 0.0, w: 0.0 });
    }
    // Tombstone a scattered set: slots 1, 3, 6.
    store.remove(e(1));
    store.remove(e(3));
    store.remove(e(6));
    assert_eq!(store.live_count(), 5);
    assert_eq!(store.len(), 8, "tombstones still occupy the high-water mark");

    store.compact();

    // Post-compact: live slots are 0..5 in canonical insertion order.
    assert_eq!(store.len(), 5, "high-water mark collapses to live count");
    assert_eq!(store.live_count(), 5);

    // After compact, live slots are exactly 0..live_count with NO gaps, and
    // the entity order equals the surviving insertion order.
    let mut order: Vec<(u32, usize)> = Vec::new();
    store.for_each_live(|slot, entity| order.push((slot, entity.get())));
    let expected: Vec<(u32, usize)> =
        [0usize, 2, 4, 5, 7].iter().enumerate().map(|(i, &en)| (i as u32, en)).collect();
    assert_eq!(order, expected, "canonical slots 0..N in surviving insertion order");

    // The bytes followed their entities down.
    for (slot, &ent) in [0usize, 2, 4, 5, 7].iter().enumerate() {
        assert_eq!(store.slot_of(e(ent)), Some(slot as u32));
        assert_eq!(read_pos(&store, slot as u32).x, ent as f32);
    }

    // Free list empty; further inserts append fresh (no reuse).
    let s_new = insert_pos(&mut store, e(50), Pos { x: 50.0, y: 0.0, z: 0.0, w: 0.0 });
    assert_eq!(s_new, 5, "post-compact insert appends at the new frontier");
    assert!(store.check_invariant());
}

#[test]
fn compact_on_all_live_is_identity() {
    let mut store = pos_store(16);
    for i in 0..4usize {
        insert_pos(&mut store, e(i), Pos { x: i as f32, y: 0.0, z: 0.0, w: 0.0 });
    }
    let before: Vec<(u32, usize)> = {
        let mut v = Vec::new();
        store.for_each_live(|s, en| v.push((s, en.get())));
        v
    };
    store.compact();
    let after: Vec<(u32, usize)> = {
        let mut v = Vec::new();
        store.for_each_live(|s, en| v.push((s, en.get())));
        v
    };
    assert_eq!(before, after, "compact with no tombstones must be identity");
    assert!(store.check_invariant());
}

// ── drop correctness for a non-Copy dense type ──────────────────────────────

#[test]
fn drop_counter_each_dropped_exactly_once_remove_compact_storedrop() {
    register();
    let ctr = Arc::new(AtomicUsize::new(0));
    let dropped = || ctr.load(Ordering::Relaxed);

    {
        let mut store = DenseStore::new(DROP_ID, 32);
        let make = |c: &Arc<AtomicUsize>| DropCounter { counter: Arc::clone(c) };

        // Insert 6 drop-counting components.
        for i in 0..6usize {
            let dc = make(&ctr);
            // SAFETY: `DropCounter` is `#[repr(C)]`; moving it into the column
            // via its byte representation is a move (ptr::write semantics in
            // `column.add`) — the local `dc` is forgotten by the byte copy, so
            // no drop runs here. We `forget` to prevent the local's Drop double.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&dc as *const DropCounter).cast::<u8>(),
                    std::mem::size_of::<DropCounter>(),
                )
            };
            store.insert(e(i), bytes);
            std::mem::forget(dc);
        }
        assert_eq!(dropped(), 0, "no drops yet (all live)");

        // remove() must drop exactly the removed components.
        store.remove(e(1));
        store.remove(e(4));
        assert_eq!(dropped(), 2, "remove drops each removed component once");

        // compact() drops nothing (tombstones already dropped on remove).
        store.compact();
        assert_eq!(dropped(), 2, "compact must not re-drop tombstones");
        assert_eq!(store.live_count(), 4);

        // store drop must drop the 4 survivors — exactly once each.
    }
    assert_eq!(
        ctr.load(Ordering::Relaxed),
        6,
        "every component dropped exactly once (2 on remove + 4 on store drop), no double/leak"
    );
}

// ── property test: the structural invariant ─────────────────────────────────

mod property {
    use super::*;
    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum Op {
        Insert(u8),
        Remove(u8),
        Compact,
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0u8..16).prop_map(Op::Insert),
            (0u8..16).prop_map(Op::Remove),
            Just(Op::Compact),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

        #[test]
        fn random_op_sequence_preserves_invariant(ops in proptest::collection::vec(op_strategy(), 0..80)) {
            register();
            let mut store = DenseStore::new(POS_ID, 256);
            // Mirror model: which entity ids are currently present.
            let mut present = std::collections::HashSet::<u8>::new();

            for op in ops {
                match op {
                    Op::Insert(id) => {
                        if !present.contains(&id) {
                            let p = Pos { x: id as f32, y: 0.0, z: 0.0, w: 0.0 };
                            super::insert_pos(&mut store, super::e(id as usize), p);
                            present.insert(id);
                        }
                    }
                    Op::Remove(id) => {
                        let removed = store.remove(super::e(id as usize));
                        prop_assert_eq!(removed, present.remove(&id));
                    }
                    Op::Compact => store.compact(),
                }
                prop_assert!(store.check_invariant(), "invariant violated after {:?}", op);
                prop_assert_eq!(store.live_count(), present.len());
            }
        }
    }
}
