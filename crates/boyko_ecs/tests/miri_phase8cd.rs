//! Phase 8c+8d Step 12 — Miri test suite for the `Command` / `CommandQueue`
//! / `Bundle` / `FunctionSystem` subsystem.
//!
//! These tests are written to be run under `cargo +nightly miri test`. They
//! exercise the unsafe code paths added in Steps 6, 7, and the apply/glue
//! pipeline; they assert that no UB (uninit reads, retag failures, double-
//! frees, aliased mutable references, padded-bytes / packed-reference
//! creation) is detected by Miri.
//!
//! They are **NOT** gated on `#[cfg(miri)]` so they also run under the
//! regular `cargo test --workspace` as smoke tests — see the lineage
//! comment at the top of `tests/miri_phase8a.rs` for the convention. The
//! tests run cheaply enough (no million-iteration loops, all sizes <= 64)
//! that the dev-profile cost is negligible.
//!
//! Plan §24 Step 12 / §15 test list:
//!
//!  1. `miri_command_queue_push_then_apply_no_ub`
//!  2. `miri_command_queue_padded_command_no_uninit_ub` (CQ1)
//!  3. `miri_command_queue_no_packed_reference_creation` (CQ-PACK1)
//!  4. `miri_command_queue_raw_command_queue_no_alias_ub` (C3)
//!  5. `miri_command_queue_panic_recovery_no_ub` (C5 + C2')
//!  6. `miri_bundle_for_each_panics_no_double_drop` (C1' / B4 NEW)
//!  7. `miri_bundle_for_each_component_bytes_callback_lifetime` (C1)
//!  8. `miri_bundle_slice_cast_arity_1_and_4` (W2'' NEW)
//!  9. `miri_function_system_initialize_then_run_no_retag_ub`
//! 10. `miri_function_system_apply_after_run_unsafe_no_aliasing`
//! 11. `miri_run_closure_once_no_turbofish_arity_4_no_ub`
//!
//! # Component-slot range
//!
//! 260..=269 + 280..=281 (Phase 8c+8d integration uses 244..=259; the
//! deleted `bundle_impls.rs` previously claimed 240..=243 — now free;
//! `params/commands.rs` claims 270..=271). Phase 8.5 migration added
//! slots 280..=281 for `PanicTrackerA` / `PanicTrackerB`, which the
//! `#[derive(Bundle)]`-flavoured rewrite of Test 6 (B4 panic safety)
//! requires as two distinct Component types to give the derived bundle
//! two distinct fields (the original `(PanicTracker, PanicTracker)` tuple
//! pattern is no longer expressible — the derive sorts by `ComponentId.0`
//! and would emit duplicate ids).
//!
//! # Phase 8.5 migration note
//!
//! Phase 8.5 (Static Bundle Cache) replaced the two-arg
//! `commands.spawn(archetype_id, (A, B, ...))` surface with a single-arg
//! `commands.spawn(MyBundle { ... })` surface backed by `#[derive(Bundle)]`.
//! All `cmds.spawn` callsites below pass a derived-bundle value; the
//! archetype id is resolved lazily via `B::cached_archetype_id` on apply.
//! Tests 6 and 7 (direct `for_each_component_bytes` exercise) wrap their
//! tuple in a derived-bundle struct so the trait method is reachable.
//!
//! # Test isolation
//!
//! The few tests that share a static counter acquire a per-file
//! `Mutex<()>` so parallel test execution does not interleave Drop
//! invocations. The pattern mirrors `tests/command_queue_panic_recovery.rs`.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::commands::command_queue::CommandQueue;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Serialisation for tests that touch a shared static counter ──────────────

static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn acquire_test_lock() -> MutexGuard<'static, ()> {
    match TEST_MUTEX.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Shared per-file counters ─────────────────────────────────────────────────

static PANIC_TRACKER_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

// ── Per-test command types ───────────────────────────────────────────────────
//
// One command type per logical test; counters live in the test bodies as
// static atomics with names scoped to the test. The types stay at module
// scope so the `Command` impl is visible to the test's `push` call.

struct PushApplyCommand;

static PUSH_APPLY_RAN: AtomicUsize = AtomicUsize::new(0);

impl Command for PushApplyCommand {
    fn apply(self, _world: &mut EcsMaster) {
        PUSH_APPLY_RAN.fetch_add(1, Ordering::Relaxed);
    }
}

/// Command with internal padding bytes between fields. `u8 + u64` forces 7
/// bytes of padding under `#[repr(C)]`. The padding bytes are uninitialised
/// in the `write_unaligned` source, but `MaybeUninit<u8>` in the queue's
/// destination is byte-pattern-agnostic. Miri's "uninit byte read"
/// detection would fire if `consume_and_drop_glue::read_unaligned` ever
/// produced an observable read of an uninit byte (it does not — the read
/// produces a `PaddedCommand` value whose own padding is again uninit
/// inside the local).
#[repr(C)]
struct PaddedCommand {
    flag: u8,
    // 7 bytes of padding under repr(C) on every supported target.
    payload: u64,
}

static PADDED_RAN: AtomicUsize = AtomicUsize::new(0);

impl Command for PaddedCommand {
    fn apply(self, _world: &mut EcsMaster) {
        // Touch both fields so the optimizer cannot elide the read of the
        // payload (which sits past the padding).
        std::hint::black_box(self.flag);
        std::hint::black_box(self.payload);
        PADDED_RAN.fetch_add(1, Ordering::Relaxed);
    }
}

/// Test 6: `PanicTrackerA` / `PanicTrackerB` — a pair of Components whose
/// Drop impls share `PANIC_TRACKER_DROP_COUNT`, allowing the derived
/// `PanicTrackerBundle` (two distinct field types) to exercise B4 panic
/// safety with two ManuallyDrop slots. Two types instead of one are
/// required because `#[derive(Bundle)]` (Phase 8.5) sorts component-ids
/// canonically — a bundle with two fields of the SAME Component type would
/// produce duplicate ids in the sorted slice, which violates the bundle's
/// archetype contract. Slot 260 retained for A (the original
/// `SLOT_PANIC_TRACKER` allocation); slot 281 added for B (within the
/// Phase 8.5 extension range 280..=339). The pair is sufficient to drive
/// B4: each tracker's Drop bumps the same counter, so the test's
/// "Drop ran 0 times" assertion still holds across both fields.
const SLOT_PANIC_TRACKER_A: ComponentId = ComponentId(260);
const SLOT_PANIC_TRACKER_B: ComponentId = ComponentId(281);

#[repr(C)]
struct PanicTrackerA {
    _marker: u32,
}

#[repr(C)]
struct PanicTrackerB {
    _marker: u32,
}

impl Drop for PanicTrackerA {
    fn drop(&mut self) {
        PANIC_TRACKER_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for PanicTrackerB {
    fn drop(&mut self) {
        PANIC_TRACKER_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

impl Component for PanicTrackerA {
    fn component_id() -> ComponentId {
        SLOT_PANIC_TRACKER_A
    }
}

impl Component for PanicTrackerB {
    fn component_id() -> ComponentId {
        SLOT_PANIC_TRACKER_B
    }
}

/// Phase 8.5 derived bundle holding both trackers. Field declaration
/// order matches canonical id order (A < B), so the derive's internal
/// sort is a no-op for this type — but it would equally hold if reversed.
#[derive(Bundle)]
struct PanicTrackerBundle {
    a: PanicTrackerA,
    b: PanicTrackerB,
}

fn register_panic_trackers() {
    register_layout::<PanicTrackerA>(SLOT_PANIC_TRACKER_A.0);
    register_layout::<PanicTrackerB>(SLOT_PANIC_TRACKER_B.0);
}

/// Test 8: arity-1 and arity-4 slice-cast soundness. Components reserve
/// slots 261..=264.
const SLOT_SC_A: ComponentId = ComponentId(261);
const SLOT_SC_B: ComponentId = ComponentId(262);
const SLOT_SC_C: ComponentId = ComponentId(263);
const SLOT_SC_D: ComponentId = ComponentId(264);

#[repr(C)]
#[derive(Clone, Copy)]
struct ScA(u32);
#[repr(C)]
#[derive(Clone, Copy)]
struct ScB(u32);
#[repr(C)]
#[derive(Clone, Copy)]
struct ScC(u32);
#[repr(C)]
#[derive(Clone, Copy)]
struct ScD(u32);

impl Component for ScA {
    fn component_id() -> ComponentId {
        SLOT_SC_A
    }
}
impl Component for ScB {
    fn component_id() -> ComponentId {
        SLOT_SC_B
    }
}
impl Component for ScC {
    fn component_id() -> ComponentId {
        SLOT_SC_C
    }
}
impl Component for ScD {
    fn component_id() -> ComponentId {
        SLOT_SC_D
    }
}

fn register_sc() {
    register_layout::<ScA>(SLOT_SC_A.0);
    register_layout::<ScB>(SLOT_SC_B.0);
    register_layout::<ScC>(SLOT_SC_C.0);
    register_layout::<ScD>(SLOT_SC_D.0);
}

/// Phase 8.5 derived bundle wrapping the four `Sc*` components. Used by
/// Tests 7, 8 (arity-4 branch) and replaces the prior anonymous tuple
/// argument shape that no longer satisfies `Bundle`.
#[derive(Bundle)]
struct ScBundle {
    a: ScA,
    b: ScB,
    c: ScC,
    d: ScD,
}

/// Phase 8.5 derived bundle wrapping the single `ScA` component. Used by
/// Test 8 (arity-1 branch) and Test 10. A dedicated single-field bundle
/// is required because each `#[derive(Bundle)]` impl owns its own
/// `BundleStaticInfo` slot — there is no implicit "subset" relationship.
#[derive(Bundle)]
struct ScABundle {
    a: ScA,
}

// Test 11 reuses the Sc* components above — no new slots needed.

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: command_queue_push_then_apply_no_ub
// ─────────────────────────────────────────────────────────────────────────────

/// Exercises the full `push` → `apply` cycle.
///
/// Miri specifically checks:
///
/// * `bytes.set_len(...)` after `write_unaligned` does not leave uninit
///   bytes in the "initialised" range that subsequent reads would observe.
/// * `consume_and_drop_glue`'s `read_unaligned::<C>` from a `*mut
///   MaybeUninit<u8>` produces a sound `C` (no provenance failure).
/// * The cursor advancement past the payload pre-`apply` (W3') keeps the
///   recovery range bounds consistent across the loop.
#[test]
fn miri_command_queue_push_then_apply_no_ub() {
    let _serial = acquire_test_lock();
    PUSH_APPLY_RAN.store(0, Ordering::Relaxed);

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();
    q.__test_push(PushApplyCommand);
    q.__test_push(PushApplyCommand);
    q.__test_push(PushApplyCommand);
    q.__test_apply(&mut world);

    assert_eq!(
        PUSH_APPLY_RAN.load(Ordering::Relaxed),
        3,
        "all three commands applied"
    );
    assert_eq!(q.__test_bytes_len(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: command_queue_padded_command_no_uninit_ub (CQ1)
// ─────────────────────────────────────────────────────────────────────────────

/// `PaddedCommand`'s 7 bytes of internal padding (between `flag: u8` and
/// `payload: u64`) are never observably read.
///
/// `write_unaligned::<PaddedCommand>` copies the entire `sizeof::<PaddedCommand>()`
/// region — including the uninit padding bytes — into `MaybeUninit<u8>`
/// slots. Miri tolerates that because the destination is `MaybeUninit`.
///
/// `read_unaligned::<PaddedCommand>` reads the same region back into a
/// stack-local `PaddedCommand`. The local's padding bytes are also
/// `MaybeUninit`-tolerant (per Rust's layout rules) so the round-trip is
/// sound. Only `self.flag` and `self.payload` are observably read — the
/// padding is never inspected.
#[test]
fn miri_command_queue_padded_command_no_uninit_ub() {
    let _serial = acquire_test_lock();
    PADDED_RAN.store(0, Ordering::Relaxed);

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();
    q.__test_push(PaddedCommand {
        flag: 0xAB,
        payload: 0x0102_0304_0506_0708,
    });
    q.__test_apply(&mut world);

    assert_eq!(PADDED_RAN.load(Ordering::Relaxed), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: command_queue_no_packed_reference_creation (CQ-PACK1)
// ─────────────────────────────────────────────────────────────────────────────

/// CQ-PACK1: the queue NEVER constructs `&C` or `&mut C` into the byte
/// slot. All access is `write_unaligned` / `read_unaligned`. This test
/// pushes a command with a 1-byte tail field that, if the queue had used
/// `&mut C`-style writes, would force a 1-byte alignment violation on
/// any `repr(packed)` retag check.
///
/// Miri's Tree Borrows + Stacked Borrows models would surface such a
/// regression as a retag failure on the `&mut C` mint. The success of
/// this test under `cargo +nightly miri test` is the CQ-PACK1 lock-down.
#[test]
fn miri_command_queue_no_packed_reference_creation() {
    let _serial = acquire_test_lock();

    /// 1-byte payload — the smallest non-empty command. If `push` /
    /// `consume_and_drop_glue` had any `&mut C` mint, the unalignment
    /// would surface here under Miri.
    struct OneByteCmd(u8);
    impl Command for OneByteCmd {
        fn apply(self, _world: &mut EcsMaster) {
            std::hint::black_box(self.0);
        }
    }

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();
    for i in 0..16u8 {
        q.__test_push(OneByteCmd(i));
    }
    q.__test_apply(&mut world);
    assert_eq!(q.__test_bytes_len(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: command_queue_raw_command_queue_no_alias_ub (C3)
// ─────────────────────────────────────────────────────────────────────────────

/// C3: the `RawCommandQueue` raw-pointer twin is minted via `&raw mut`
/// from `&mut CommandQueue` — no intermediate `&` or `&mut` references on
/// the inner fields are created.
///
/// Miri's Stacked Borrows model would surface a violation as a retag
/// failure inside `RawCommandQueue::apply_or_drop_queued`'s short-lived
/// `as_ref` / `as_mut` materialisations. Driving the queue through
/// repeated apply cycles maximises the per-loop retag count.
#[test]
fn miri_command_queue_raw_command_queue_no_alias_ub() {
    let _serial = acquire_test_lock();
    PUSH_APPLY_RAN.store(0, Ordering::Relaxed);

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();

    // Multiple apply cycles — each rebuilds the RawCommandQueue twin.
    for _ in 0..4 {
        for _ in 0..8 {
            q.__test_push(PushApplyCommand);
        }
        q.__test_apply(&mut world);
    }

    assert_eq!(PUSH_APPLY_RAN.load(Ordering::Relaxed), 32);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5: command_queue_panic_recovery_no_ub (C5 + C2')
// ─────────────────────────────────────────────────────────────────────────────

/// Exercises the `catch_unwind` recovery path under Miri. C5 (the apply
/// loop's `set_len(start)` + recovery slice copy) and C2' (the OPAQUE
/// recovery semantic between apply calls) interact tightly; this test
/// drives both in one shot.
///
/// Push `[ok, panic, ok]`; the apply catches the panic; the survivor's
/// bytes are absorbed back into `bytes` via the start==0 Err branch; the
/// second apply drains the survivor cleanly. Miri's allocator detects:
///
/// * No double-free on the panicker (the local `cmd` drops once via
///   unwind drop-elaboration; the byte slot is logically uninit because
///   `cursor` was advanced past it before the panic).
/// * No use-after-free on the survivor (the slice copy into recovery is a
///   memcpy of `MaybeUninit<u8>` — `Copy`, sound on uninit).
/// * No leak on the survivor (the second apply consumes it; `Drop` on the
///   queue at end-of-scope would also drain it.)
#[test]
fn miri_command_queue_panic_recovery_no_ub() {
    let _serial = acquire_test_lock();

    struct Panicker;
    impl Command for Panicker {
        fn apply(self, _world: &mut EcsMaster) {
            panic!("intentional panic for Miri recovery test");
        }
    }
    impl Drop for Panicker {
        fn drop(&mut self) {
            // Drop ran via unwind — counted but no apply.
        }
    }

    static A_RAN: AtomicUsize = AtomicUsize::new(0);
    static B_RAN: AtomicUsize = AtomicUsize::new(0);
    struct A;
    struct B;
    impl Command for A {
        fn apply(self, _world: &mut EcsMaster) {
            A_RAN.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Command for B {
        fn apply(self, _world: &mut EcsMaster) {
            B_RAN.fetch_add(1, Ordering::Relaxed);
        }
    }

    A_RAN.store(0, Ordering::Relaxed);
    B_RAN.store(0, Ordering::Relaxed);

    let mut q = CommandQueue::__test_new();
    let mut world = EcsMaster::new();
    q.__test_push(A);
    q.__test_push(Panicker);
    q.__test_push(B);

    let first = panic::catch_unwind(AssertUnwindSafe(|| {
        q.__test_apply(&mut world);
    }));
    assert!(first.is_err(), "apply must propagate the panic");
    assert_eq!(A_RAN.load(Ordering::Relaxed), 1);
    assert_eq!(B_RAN.load(Ordering::Relaxed), 0);

    // Survivor was absorbed into bytes (Bevy mirror) — second apply drains.
    q.__test_apply(&mut world);
    assert_eq!(B_RAN.load(Ordering::Relaxed), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6: miri_bundle_for_each_panics_no_double_drop (C1' / B4 NEW)
// ─────────────────────────────────────────────────────────────────────────────

/// B4: if the user's `for_each_component_bytes` callback panics
/// mid-iteration, the un-yielded Components LEAK (no Drop) rather than
/// double-dropping with archetype-side ownership.
///
/// Setup: `Bundle::for_each_component_bytes` on `(PanicTracker,
/// PanicTracker)`. The callback panics on the SECOND invocation.
///
/// Expectations (per plan §12.3 / B4):
///
/// * PanicTracker's Drop ran 0 times after the panic (both elements were
///   ManuallyDrop-wrapped UPFRONT, before any callback fired).
/// * Miri reports no double-drop UB on the unwind path.
///
/// Note: this test only exercises the **Bundle layer** — it does NOT go
/// through `SpawnCommand::apply`. Driving the panic from within the
/// callback isolates B4 from the rest of the apply pipeline.
#[test]
fn miri_bundle_for_each_panics_no_double_drop() {
    let _serial = acquire_test_lock();
    register_panic_trackers();
    PANIC_TRACKER_DROP_COUNT.store(0, Ordering::Relaxed);

    let bundle = PanicTrackerBundle {
        a: PanicTrackerA { _marker: 1 },
        b: PanicTrackerB { _marker: 2 },
    };

    let mut call_count = 0usize;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        bundle.for_each_component_bytes(|_id, _bytes| {
            call_count += 1;
            if call_count == 2 {
                panic!("B4 deliberate panic on second callback");
            }
        });
    }));

    assert!(result.is_err(), "callback panic must propagate");
    assert_eq!(
        PANIC_TRACKER_DROP_COUNT.load(Ordering::Relaxed),
        0,
        "B4: ManuallyDrop suppresses Drop on every element (leak < double-drop)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 7: miri_bundle_for_each_component_bytes_callback_lifetime (C1)
// ─────────────────────────────────────────────────────────────────────────────

/// C1: the `&[u8]` slices delivered to the callback are valid for the
/// callback's duration. Miri's Tree Borrows model would surface any
/// regression as a retag failure on the `from_raw_parts` re-cast in the
/// bundle impl.
///
/// This test reads each byte out of every yielded slice and stores the
/// XOR-fold into a probe so the compiler cannot elide the read.
#[test]
fn miri_bundle_for_each_component_bytes_callback_lifetime() {
    register_sc();

    let bundle = ScBundle {
        a: ScA(0x0102_0304),
        b: ScB(0x0506_0708),
        c: ScC(0x090A_0B0C),
        d: ScD(0x0D0E_0F10),
    };

    let mut fold: u8 = 0;
    bundle.for_each_component_bytes(|_id, bytes| {
        for &b in bytes {
            fold ^= b;
        }
    });

    // Sanity check: XOR-fold of every byte across the four u32 values
    // (little-endian on every Tier-1 target):
    //   ScA = [04, 03, 02, 01], ScB = [08, 07, 06, 05],
    //   ScC = [0C, 0B, 0A, 09], ScD = [10, 0F, 0E, 0D].
    // XOR = 0x04 ^ 0x03 ^ ... ^ 0x0D = 0x10 (computed by hand).
    let expected: u8 = (1u8..=16).fold(0, |acc, b| acc ^ b);
    assert_eq!(
        fold, expected,
        "byte fold across the four 32-bit values must equal the XOR of {{1..=16}}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 8: miri_bundle_slice_cast_arity_1_and_4 (W2'' NEW)
// ─────────────────────────────────────────────────────────────────────────────

/// W2'': `SpawnCommand::apply`'s `*const MaybeUninit<(ComponentId, &[u8])>`
/// → `*const (ComponentId, &[u8])` slice cast must hold under Miri's
/// strict provenance + Tree Borrows.
///
/// Drives the cast for arity 1 and arity 4 by running `SpawnCommand::apply`
/// directly through `Commands::spawn` + `CommandQueue::apply`. Miri's
/// strict-provenance checks would flag any layout-incompatibility or
/// out-of-bounds slot access (slots `[count..4]` must NEVER be read).
#[test]
fn miri_bundle_slice_cast_arity_1_and_4() {
    register_sc();

    // Arity 1 — only slot 0 is initialised; slots 1, 2, 3 stay uninit
    // until the `from_raw_parts(slots_base, count=1)` cast bounds the
    // slice at exactly 1 element. Reading slot 1 would be uninit-read UB.
    {
        let mut ecs = EcsMaster::new();
        ecs.run_system(|mut cmds: Commands| {
            cmds.spawn(ScABundle { a: ScA(0xDEAD_BEEF) });
        });
        assert_eq!(ecs.entity_count(), 1, "arity-1 spawn lands one entity");
    }

    // Arity 4 — every slot is initialised; the slice covers exactly
    // count=4. Both extremes of the W2'' cast bounds are exercised.
    {
        let mut ecs = EcsMaster::new();
        ecs.run_system(|mut cmds: Commands| {
            cmds.spawn(ScBundle {
                a: ScA(1),
                b: ScB(2),
                c: ScC(3),
                d: ScD(4),
            });
        });
        assert_eq!(ecs.entity_count(), 1, "arity-4 spawn lands one entity");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 9: miri_function_system_initialize_then_run_no_retag_ub
// ─────────────────────────────────────────────────────────────────────────────

/// `FunctionSystem::initialize` then `run_unsafe` then `apply` — every
/// unsafe stage in one call. Miri's retag check covers:
///
/// * `UnsafeEcsCell::new_mutable` mint from `&mut EcsMaster`.
/// * `SystemParam::get_param` reborrow of the param state.
/// * The closure's body call.
/// * `SystemParam::apply` reborrow of the param state for the flush.
#[test]
fn miri_function_system_initialize_then_run_no_retag_ub() {
    let mut ecs = EcsMaster::new();

    // The `()` system body is the smallest body that exercises the full
    // initialize → run_unsafe → apply pipeline without any actual
    // SystemParam reborrow inside the body — isolates the dispatch retag.
    ecs.run_system(|| {
        std::hint::black_box(());
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 10: miri_function_system_apply_after_run_unsafe_no_aliasing
// ─────────────────────────────────────────────────────────────────────────────

/// APP3 ordering: `System::apply` runs AFTER `run_unsafe` returns. Miri
/// would surface an aliasing UB if the state-slot reborrow in `apply`
/// overlapped with any borrow still live from `run_unsafe`.
///
/// This test runs a system whose body uses both `Commands` (whose `apply`
/// drains the queue) and reads back from world — the cross-borrow path
/// that aliasing UB would surface on.
#[test]
fn miri_function_system_apply_after_run_unsafe_no_aliasing() {
    register_sc();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(ScABundle { a: ScA(7) });
    });

    // Post-flush: the spawn landed. Read entity_count out for the Miri
    // run — no aliasing UB.
    assert_eq!(ecs.entity_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 11: miri_run_closure_once_no_turbofish_arity_4_no_ub
// ─────────────────────────────────────────────────────────────────────────────

/// `EcsMaster::run_closure_once` (Phase 8a alias for `run_system`) with a
/// 4-arity tuple param. Validates that the C6/W2 HRTB-based closure-arg
/// inference works AND that the resulting `FunctionSystem`'s 4-tuple
/// `Param::get_param` walk runs cleanly under Miri.
#[test]
fn miri_run_closure_once_no_turbofish_arity_4_no_ub() {
    use boyko_ecs::ecs::core::system::{Res, ResMut};
    use boyko_macros::Resource;

    #[derive(Resource)]
    struct M4A(u32);
    #[derive(Resource)]
    struct M4B(u32);
    #[derive(Resource)]
    struct M4C(u32);
    #[derive(Resource)]
    struct M4D(u32);

    let mut ecs = EcsMaster::new();
    ecs.insert_resource(M4A(1));
    ecs.insert_resource(M4B(2));
    ecs.insert_resource(M4C(3));
    ecs.insert_resource(M4D(0));

    ecs.run_closure_once(
        |(a, b, c, mut d): (Res<M4A>, Res<M4B>, Res<M4C>, ResMut<M4D>)| {
            d.0 = (*a).0 + (*b).0 + (*c).0;
        },
    );

    assert_eq!(ecs.resource::<M4D>().0, 6, "tuple-4 closure must observe + write");
}

// Trybuild / Cargo discovers the file; nothing more required here.
