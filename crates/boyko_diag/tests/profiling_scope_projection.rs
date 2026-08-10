//! **Profiling rung 11 — the split between the channel half of `ARM_MASK` and the projected half.**
//!
//! # Why this is an integration test and not a unit test
//!
//! `ARM_MASK` is process-global, and `project_scopes` writes **every** bit in
//! `PROJECTED_SCOPE_BASE..SCOPE_COUNT` on every call. `profiling_abi`'s in-file test module arms
//! and disarms scopes 10, 11, 20, 30, 31, 40, 41 and 42 — all inside that range — so a projection
//! test sharing that binary would clear their gates mid-assertion under `libtest`'s default
//! parallelism. Serialising ten existing tests on a new lock is the alternative; a separate binary
//! is one file and no edit to code that already works.
//!
//! # What is actually at stake here
//!
//! The projection could have owned the whole word. It does not, and the reason is a one-way switch:
//! the fold's entry gate is `any_armed()`, and the projection is a **step of the fold**. With the
//! channel bits projectable, disabling every scope clears the mask, the fold stops running, and the
//! projection never runs again — so re-enabling a scope writes a bit that nothing will ever read.
//! The game would have turned its profiler off permanently by turning one scope off, with no
//! diagnostic. The first test below is that property, stated directly rather than through the ECS.

use boyko_diag::profiling_abi::{
    PROJECTED_SCOPE_BASE, PROJECTED_SCOPE_MASK, SCOPE_COUNT, USER_SCOPE_BASE, any_armed, arm_scope,
    arm_mask_bits, disarm_scope, project_scopes, scope_armed,
};

/// The channel bit `Profiler::arm` holds — `ROOT_SCOPE`, spelled here without depending on
/// `boyko_ecs` (this crate is the bottom of the graph and depends on nothing).
const ROOT: u32 = 0;

/// One lock over the one global these four tests share.
///
/// `ARM_MASK` is process-global and every test here writes all 56 projected bits, so under
/// `libtest`'s default parallelism each would clear the others' gates mid-assertion. This is the
/// third time this campaign has needed the rule and the second time in this crate — `dyn_registry`
/// serialises its own six tests on the same shape, for the same reason.
///
/// A `Mutex` is the right tool and the ban's own exception applies: test scaffolding, not a hot
/// path.
#[allow(clippy::disallowed_types)]
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the lock, tolerating a poisoning from an earlier failure — the second test's result is
/// still worth having, and a `PoisonError` reported instead of the real assertion would hide it.
#[allow(clippy::disallowed_types)]
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Leave the mask in a known state: no channels, no scopes.
fn quiesce() {
    project_scopes(0);
    for c in 0..PROJECTED_SCOPE_BASE {
        disarm_scope(c);
    }
    assert!(!any_armed(), "the fixture failed to quiesce the mask: {:#x}", arm_mask_bits());
}

/// **The trap the split exists to avoid.** A projection of zero must not clear the channel bit, so
/// the fold keeps running and the toggle stays two-sided.
///
/// RED, run at implementation: replace `project_scopes`'s `fetch_update` body with a plain
/// `store(scopes)`. MEASURED — it fires **three assertions earlier than predicted**, on
/// *"publishing scopes cleared the channel the profiler is armed on"*: the channel is lost by the
/// FIRST projection, not by the projection of zero, because a plain store overwrites the whole word
/// whatever it is publishing. The `any_armed()` clause below is the one that names the consequence;
/// this one is the one that catches it. (It also reds
/// `an_unchanged_projection_reports_that_it_wrote_nothing`, which a plain store cannot satisfy at
/// all.)
#[test]
fn projecting_zero_scopes_leaves_the_channel_half_alone() {
    let _g = serial();
    quiesce();
    arm_scope(ROOT);
    assert!(scope_armed(ROOT));

    project_scopes(1u64 << USER_SCOPE_BASE);
    assert!(scope_armed(ROOT), "publishing scopes cleared the channel the profiler is armed on");
    assert!(scope_armed(USER_SCOPE_BASE));

    project_scopes(0);
    assert!(
        !scope_armed(USER_SCOPE_BASE),
        "a projection of zero must disarm the scopes it no longer names"
    );
    assert!(
        scope_armed(ROOT) && any_armed(),
        "disabling every scope switched the whole instrument off — the fold's own entry gate is \
         any_armed(), and the projection is a step of the fold, so this is a one-way switch"
    );

    disarm_scope(ROOT);
    quiesce();
}

/// The projection replaces the scope half wholesale — it is not an accumulator.
///
/// RED: `let next = live | scopes;` ⇒ a bit once projected never clears ⇒ the second assertion
/// fires. (Measured in the engine too: that injection lands on `G12`'s zero-A assertion,
/// `left: 1, right: 0`, in both write-path clauses.)
#[test]
fn a_projection_replaces_the_scope_half_rather_than_accumulating_into_it() {
    let _g = serial();
    quiesce();

    let a = 1u64 << USER_SCOPE_BASE;
    let b = 1u64 << (SCOPE_COUNT - 1);

    project_scopes(a | b);
    assert_eq!(arm_mask_bits(), a | b);

    project_scopes(b);
    assert_eq!(
        arm_mask_bits(),
        b,
        "the scope half must be REPLACED; an OR would leave a scope armed after the ECS said off"
    );

    quiesce();
}

/// A bit outside the projected half is ignored on the way IN, not honoured and not an error.
///
/// A `ProfilingScope` hand-built on a channel bit is the case: it names a bit the ECS does not own,
/// and the honest answer is that the projection does not write it. Silently arming a channel from
/// a game's component would be the worse outcome, and it is the one this asserts against.
#[test]
fn a_channel_bit_in_the_input_is_dropped_rather_than_armed() {
    let _g = serial();
    quiesce();

    project_scopes(u64::MAX);
    assert_eq!(
        arm_mask_bits(),
        PROJECTED_SCOPE_MASK,
        "an all-ones projection must arm every scope and no channel"
    );

    project_scopes((1u64 << ROOT) | (1u64 << (PROJECTED_SCOPE_BASE - 1)));
    assert_eq!(arm_mask_bits(), 0, "a projection naming only channels must write nothing at all");

    quiesce();
}

/// The store happens **only on change** — the corpus's *"one store only on change"*, expressed as
/// the return value so a caller can assert it rather than infer it.
#[test]
fn an_unchanged_projection_reports_that_it_wrote_nothing() {
    let _g = serial();
    quiesce();

    let bits = 1u64 << (USER_SCOPE_BASE + 1);
    assert!(project_scopes(bits), "the first projection of a new value must change the mask");
    assert!(!project_scopes(bits), "re-projecting the same value must not write the line again");
    assert!(!project_scopes(bits | 1), "a channel bit in the input is not a change");
    assert!(project_scopes(0), "clearing is a change");
    assert!(!project_scopes(0), "and clearing twice is not");

    quiesce();
}
