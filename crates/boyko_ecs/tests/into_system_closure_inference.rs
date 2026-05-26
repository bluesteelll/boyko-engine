//! Closure-argument inference reproducer for Phase 8c (Step 2, W2').
//!
//! Validates the three forms the `SystemParamFunction<Marker>` blanket impl
//! (see `crates/boyko_ecs/src/ecs/core/system/function_system_impls.rs`)
//! must accept under stable rustc:
//!
//! 1. `|p: StubParam<'_, '_>|`   — explicit anonymous lifetimes.
//! 2. `fn body(p: StubParam<'_, '_>)` — fn-pointer / fn-item form.
//! 3. `|p: StubParam|`           — fully elided (W2' acceptance gate).
//!
//! Form 3 is the canonical user-facing shape the Phase 8c headline
//! ergonomic promise depends on (e.g. `|q: Query<&Position>|` without
//! `<'_, '_>`). If rustc cannot infer it, the fallback documented in plan
//! §4.7.1 applies — users must spell out the lifetime annotations and the
//! `lifetimeless::SQuery` migration path opens.
//!
//! Each test is a `--no-run` compile gate: the runtime body is empty and
//! never executes the closure. We use a free function
//! [`accepts_system_param_function`] to anchor the bounds without bringing
//! the full `EcsMaster` / scheduling stack into scope.
//!
//! The stub `SystemParam` (`StubParam<'w, 's>`) deliberately threads BOTH
//! lifetimes through `PhantomData`, mirroring `Query<'w, 's, D, F>` (Phase
//! 8b). The single-lifetime variant tested in §4.7's reproducer is NOT
//! sufficient — a `Res<'w, R>`-style param hides the `'s` slot and lets
//! rustc cheat. Two real lifetimes are required to catch the regression.

use std::marker::PhantomData;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::SystemParamFunction;
use boyko_ecs::ecs::core::system::filtered_access_set::FilteredAccessSet;
use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
use boyko_ecs::ecs::core::system::system_param::SystemParam;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

// ── Stub `SystemParam` ─────────────────────────────────────────────────────

/// Per-system state for [`StubParam`]. Trivial — the param has no real
/// access surface and no per-invocation work. `Send + Sync + 'static` so
/// it satisfies [`SystemParam::State`]'s bounds.
struct StubState;

// SAFETY: ZST with no references; thread-safe by construction.
unsafe impl Send for StubState {}
// SAFETY: same — ZST cannot be observed across threads.
unsafe impl Sync for StubState {}

/// Two-lifetime stub `SystemParam`. `'w` is the world-access scope; `'s`
/// is the state scope (mirrors `Query<'w, 's, D, F>`). Both lifetimes are
/// threaded through `PhantomData` so rustc cannot elide them away during
/// closure inference — this is the property the W2' reproducer is gating.
#[allow(dead_code)]
struct StubParam<'w, 's> {
    _w: PhantomData<&'w ()>,
    _s: PhantomData<&'s mut ()>,
}

// SAFETY (SP1, SP2, SP4):
//   - SP1: `init_access` declares no reads or writes (the stub touches
//     nothing on the world).
//   - SP2: `get_param` returns an inert `StubParam` value; the body is
//     vacuously aliasing-safe.
//   - SP4: `init_state` performs no world mutation — `StubState` is
//     zero-sized.
unsafe impl<'a, 'b> SystemParam for StubParam<'a, 'b> {
    type State = StubState;
    type Item<'w, 's> = StubParam<'w, 's>;

    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        StubState
    }

    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
    }

    unsafe fn get_param<'w, 's>(
        _state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        StubParam {
            _w: PhantomData,
            _s: PhantomData,
        }
    }
}

// ── Bound anchor ───────────────────────────────────────────────────────────

/// Anchors the `SystemParamFunction<Marker>` bound without invoking it.
///
/// The function is never called at runtime — it exists only to drag the
/// trait obligation through type inference. If `body` does not satisfy
/// `SystemParamFunction<Marker>` for some `Marker`, compilation fails at
/// the call site, which is what the three tests below assert.
#[allow(dead_code)]
fn accepts_system_param_function<F, Marker>(_body: F)
where
    F: SystemParamFunction<Marker>,
    Marker: 'static,
{
}

// ── Acceptance tests ───────────────────────────────────────────────────────

/// Test 1 (§4.7 reproducer 1) — explicit `<'_, '_>` annotations on the
/// closure's parameter type. This is the minimum-noise form for users
/// who cannot rely on full elision (see Test 3 and the §4.7.1 fallback).
#[test]
fn closure_with_explicit_lifetimes_compiles() {
    accepts_system_param_function(|_p: StubParam<'_, '_>| {});
}

/// Test 2 (§4.7 reproducer 2) — fn-item form. Bevy's documentation
/// emphasises that named fn items must work as freely as closures; this
/// test exists so a regression in the fn-pointer coercion path is caught
/// independently from closure-specific inference logic.
#[test]
fn closure_with_implicit_anonymous_lifetimes_compiles() {
    fn body(_p: StubParam<'_, '_>) {}
    accepts_system_param_function(body);
}

/// Test 3 (§4.7 reproducer 3, W2' NEW) — FULLY ELIDED parameter type.
/// No `<'_, '_>` annotation; the closure simply binds `_p: StubParam`.
///
/// # Why this matters
///
/// This is the canonical user-facing form: `|q: Query<&Position>|`. If
/// this compiles, the Phase 8c headline ergonomic promise holds — users
/// pay zero lifetime-annotation noise at the closure boundary. If this
/// FAILS to compile, plan §4.7.1's fallback documents the residual
/// gap: users must write `|q: Query<'_, '_, &Position>|` (or adopt the
/// future `lifetimeless::SQuery` shim).
///
/// On stable rustc 1.85+ this test is expected to pass (Bevy ships the
/// same pattern). The test is the W2' acceptance criterion from plan
/// Round 3.
#[test]
fn closure_with_elided_param_type_compiles() {
    accepts_system_param_function(|_p: StubParam| {});
}
