/// Concurrency integration tests for component_id() and register_event_new.
///
/// Verifies that the OnceLock-based lazy ID minting is safe under concurrent access:
/// - All threads calling T::component_id() for the same T get the same ID.
/// - Threads calling component_id() for distinct types get pairwise distinct IDs.
///
/// These tests use std::thread::scope (stable since Rust 1.63) — no external deps.
use boyko_macros::Component;
use boyko_ecs::ecs::core::component::component::Component;

// Types must be declared at module scope so their TypeId is unambiguous.

/// Shared component type: all 8 threads will call this type's component_id().
#[allow(dead_code)]
#[derive(Component)]
struct SharedComponent {
    value: u64,
}

/// 8 distinct component types — one per thread in the distinct-types test.
/// Using numeric suffixes avoids name conflicts.
#[allow(dead_code)] #[derive(Component)] struct ThreadComp0 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp1 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp2 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp3 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp4 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp5 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp6 { v: u8 }
#[allow(dead_code)] #[derive(Component)] struct ThreadComp7 { v: u8 }

/// N threads all calling the same type's component_id() must all receive the
/// same value. Repeat RUNS times to expose nondeterminism in scheduling.
#[test]
fn concurrent_register_new_for_same_type() {
    const N: usize = 8;
    const RUNS: usize = 10;

    for run in 0..RUNS {
        let results: Vec<usize> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..N)
                .map(|_| s.spawn(SharedComponent::component_id))
                .collect();
            handles.into_iter().map(|h| h.join().expect("thread must not panic")).collect()
        });

        let first = results[0];
        for (i, &id) in results.iter().enumerate() {
            assert_eq!(
                id,
                first,
                "run {run}: thread {i} returned id={id}, expected {first} (all threads same type)"
            );
        }
    }
}

/// 8 threads, each calling a distinct type's component_id(), must produce
/// pairwise distinct IDs.
#[test]
fn concurrent_register_new_for_distinct_types() {
    // Collect IDs: each closure captures a different type's component_id() fn.
    let ids: Vec<usize> = std::thread::scope(|s| {
        let h0 = s.spawn(ThreadComp0::component_id);
        let h1 = s.spawn(ThreadComp1::component_id);
        let h2 = s.spawn(ThreadComp2::component_id);
        let h3 = s.spawn(ThreadComp3::component_id);
        let h4 = s.spawn(ThreadComp4::component_id);
        let h5 = s.spawn(ThreadComp5::component_id);
        let h6 = s.spawn(ThreadComp6::component_id);
        let h7 = s.spawn(ThreadComp7::component_id);
        vec![
            h0.join().expect("thread 0 must not panic"),
            h1.join().expect("thread 1 must not panic"),
            h2.join().expect("thread 2 must not panic"),
            h3.join().expect("thread 3 must not panic"),
            h4.join().expect("thread 4 must not panic"),
            h5.join().expect("thread 5 must not panic"),
            h6.join().expect("thread 6 must not panic"),
            h7.join().expect("thread 7 must not panic"),
        ]
    });

    // All IDs must be pairwise distinct.
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(
                ids[i],
                ids[j],
                "ThreadComp{i} and ThreadComp{j} must have distinct component IDs \
                 (got ids[{i}]={}, ids[{j}]={})",
                ids[i],
                ids[j]
            );
        }
    }
}

/// Verify that a type's component_id() is stable even after concurrent first-call
/// racing: call component_id() on SharedComponent from N threads simultaneously,
/// THEN call it again from the main thread — must return the same value.
#[test]
fn component_id_stable_after_concurrent_first_call() {
    const N: usize = 16;

    // First, run N concurrent callers.
    let concurrent_ids: Vec<usize> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..N)
            .map(|_| s.spawn(SharedComponent::component_id))
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread must not panic")).collect()
    });

    // All concurrent IDs must be equal.
    let expected = concurrent_ids[0];
    for (i, &id) in concurrent_ids.iter().enumerate() {
        assert_eq!(id, expected, "concurrent thread {i}: id={id} != expected {expected}");
    }

    // Post-race call from main thread must return the same value.
    let post_race_id = SharedComponent::component_id();
    assert_eq!(
        post_race_id,
        expected,
        "post-race main-thread call returned {post_race_id}, expected {expected}"
    );
}
