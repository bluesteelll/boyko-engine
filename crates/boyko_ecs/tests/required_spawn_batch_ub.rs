//! Feature 1 (required components) — the CRITICAL spawn_batch UB regression.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` Soundness: `SpawnBatchCommand::apply`
//! constructs the required columns through the same construct-into-uninit pass
//! as the single spawn. The UB the fix closes is committing a required column's
//! slot WITHOUT constructing it (a logically-uninit slot that the archetype then
//! drops on teardown → drop-of-uninitialized = UB). This file proves the
//! required columns are CONSTRUCTED, not committed-uninitialized:
//!
//! * every entity in the batch has B == B::default();
//! * at teardown B's `Drop` runs EXACTLY N times (one per constructed row,
//!   neither 0 — uninit/never-constructed — nor 2N — double-construct).
//!
//! # Pre-existing caveat (do NOT contradict)
//!
//! `SpawnBatchCommand::apply` fires NO on_add/on_insert hooks/observers for ANY
//! component (a pre-existing gap, filed separately). So this file asserts only
//! CONSTRUCTION correctness (values + drop count) — it does NOT assert hook
//! firing on the spawn_batch path (which fires for nobody).
//!
//! # Why a heap-wrapping drop counter (not Copy)
//!
//! B wraps a `Box<u32>` and increments a global `static AtomicUsize` in `Drop`.
//! A `Copy` type cannot have a `Drop`, and a trivial-drop type would let a
//! drop-of-uninit slip past unobserved. The `Box` payload also gives Miri a real
//! heap allocation to track — a never-constructed (uninit) slot dropped as a
//! `Box` would be a use of uninitialized memory Miri flags loudly.
//!
//! BUG-REQ-SNAKE-1 is FIXED at the macro layer (the derive emits its own
//! `#[allow(non_snake_case)]` on the generated ctor fns), so no crate-level mask
//! is needed here.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// Global drop counter for the required component B. Incremented once per
/// `Drop::drop`. Reset at the start of the test (the test is the only writer
/// for this component type).
static B_DROPS: AtomicUsize = AtomicUsize::new(0);

/// Required component with a NON-trivial `Drop` wrapping a heap value. Its
/// `Default` produces a sentinel value (777) so a constructed row is
/// distinguishable from zeroed/uninit bytes.
#[derive(Component)]
#[repr(C)]
struct UbB {
    payload: Box<u32>,
}

impl Default for UbB {
    fn default() -> Self {
        UbB {
            payload: Box::new(777),
        }
    }
}

impl Drop for UbB {
    fn drop(&mut self) {
        // A drop-of-uninit slot would either never reach here (drop count 0) or
        // deref a garbage `Box` pointer (Miri UB). Reading the payload forces a
        // real heap deref so a bad pointer is observed, not silently ignored.
        let _ = *self.payload;
        B_DROPS.fetch_add(1, SEQ);
    }
}

/// The user-spawned component whose bundle requires `UbB`. `UbB` is ABSENT from
/// the bundle, so the constructor pass must materialize it.
#[derive(Component)]
#[require(UbB)]
#[repr(C)]
struct UbA(u32);

#[derive(Bundle)]
struct UbABundle {
    a: UbA,
}

/// N entities, kept small so Miri can run it (the spawn_batch path is the
/// soundness surface). 64 is enough to prove "exactly N, not 0 / not 2N".
const N: usize = 64;

#[test]
fn spawn_batch_a_requires_b_constructs_b_dropped_exactly_n_times() {
    let _ = UbA::component_id();
    let _ = UbB::component_id();
    B_DROPS.store(0, SEQ);

    {
        let mut ecs = EcsMaster::new();

        ecs.run_system(|mut cmds: Commands| {
            let _ = cmds
                .spawn_batch((0..N as u32).map(|i| UbABundle { a: UbA(i) }))
                .expect("N ≤ MAX_BATCH_HINT");
        });

        assert_eq!(
            ecs.entity_count(),
            N,
            "spawn_batch landed all N entities"
        );

        // Every entity must have a CONSTRUCTED B holding the sentinel default.
        // Collect entity handles first (releases the iter borrow) so the
        // per-entity `get_component` (&self) does not alias the iterator.
        let entities: Vec<_> = ecs.iter_entities().collect();
        assert_eq!(entities.len(), N, "iterated every batch entity");
        for e in entities {
            assert!(
                ecs.has_component(e, UbA::component_id()),
                "each batch entity has the explicit A"
            );
            assert!(
                ecs.has_component(e, UbB::component_id()),
                "spawn_batch UB: required B is CONSTRUCTED for every batch row"
            );
            let b = ecs
                .get_component::<UbB>(e)
                .expect("required B present on every row");
            assert_eq!(
                *b.payload, 777,
                "spawn_batch UB: B holds the CONSTRUCTED default sentinel (777), \
                 NOT uninitialized/garbage bytes"
            );
        }

        assert_eq!(
            B_DROPS.load(SEQ),
            0,
            "no B dropped while the entities are still alive"
        );
        // `ecs` drops here → archetype teardown drops every committed row's B.
    }

    assert_eq!(
        B_DROPS.load(SEQ),
        N,
        "spawn_batch UB REGRESSION: B's Drop runs EXACTLY N times \
         (proves N constructed rows — not 0 uninit/never-constructed, not 2N double-construct)"
    );
}
