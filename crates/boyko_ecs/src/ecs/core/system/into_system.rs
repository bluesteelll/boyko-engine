//! The `IntoSystem` trait — convert any value into a runnable [`System`].
//!
//! See Phase 8c+8d plan §3 (Decision C1) for the trait shape and §6
//! (Decision C4) for the blanket-impl strategy. The `Marker` type
//! parameter resolves trait-impl ambiguity between the identity blanket
//! (`S: System`) and the function-system blanket (`F: SystemParamFunction`).
//!
//! This module ships the trait declaration plus the [`IsFunctionSystem`]
//! marker type. The blanket impls are wired in Step 2 (identity) and
//! Step 3 (`FunctionSystem`); see plan §24 for the step ordering.
//!
//! [`System`]: super::system::System

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::exclusive_function_system::ExclusiveFunctionSystem;
use crate::ecs::core::system::function_system::{FunctionSystem, SystemParamFunction};
use crate::ecs::core::system::system::System;

/// Convert any value into a runnable [`System`].
///
/// `IntoSystem` is the bridge between user-written function bodies and
/// the runtime's [`System`] trait. The trait is keyed by a `Marker`
/// type parameter so distinct blanket impls (identity vs. function-system)
/// can coexist without overlap — see plan §3 and §6 for the rationale.
///
/// # Type parameters
///
/// * `In` — input type plumbed through to [`System::run_unsafe`]. Phase 8c
///   uses `()` exclusively; Phase 9's chained-system support will exercise
///   non-unit `In`.
/// * `Out` — output of the system body.
/// * `Marker` — disambiguator for trait impls. The identity impl uses the
///   system type itself as the marker; the function-system impl uses
///   [`IsFunctionSystem`].
///
/// # Invariants (plan §18, IS1 / IS2)
///
/// * **IS1** — `Marker` is unique per arity for function-system impls.
/// * **IS2** — identity and function-system blankets are disjoint by
///   construction (distinct `Marker` shapes).
///
/// [`System`]: super::system::System
/// [`System::run_unsafe`]: super::system::System::run_unsafe
pub trait IntoSystem<In, Out, Marker>: Sized {
    /// Concrete [`System`] type produced by [`into_system`](Self::into_system).
    ///
    /// [`System`]: super::system::System
    type System: System<Out = Out>;

    /// Consumes `this` and yields the runnable system.
    ///
    /// The associated-function form (rather than `&self`) mirrors Bevy's
    /// `IntoSystem::into_system`; it leaves the impl free to move captured
    /// state into the produced [`Self::System`] without re-borrowing.
    fn into_system(this: Self) -> Self::System;
}

/// Marker type selecting the function-system blanket impl of
/// [`IntoSystem`].
///
/// Wired in Phase 8c Step 2 (forward declaration) and Step 3 (functional
/// body); see plan §6 and §24. The type is zero-sized and exists solely
/// as a disambiguator inside the [`IntoSystem`] trait bound.
pub struct IsFunctionSystem;

/// Function-system blanket: any `F: SystemParamFunction<Marker>` with
/// `In = ()` is convertible into a [`FunctionSystem<F, Marker>`].
///
/// The two-element marker tuple `(IsFunctionSystem, Marker)` is the
/// standard Bevy disambiguation trick (plan §3, §6) — the `IsFunctionSystem`
/// ZST disjoints the blanket from any future identity `impl<S: System>
/// IntoSystem<..., S> for S` (IS2). `Marker` retains the per-arity
/// disambiguator inherited from [`SystemParamFunction`]'s blanket impls
/// (IS1).
///
/// `In = ()` matches the current [`System::run_unsafe`] signature, which
/// takes no input. Phase 9's chained-system support will widen the bound.
impl<F, Out, Marker> IntoSystem<(), Out, (IsFunctionSystem, Marker)> for F
where
    F: SystemParamFunction<Marker, In = (), Out = Out>,
    Marker: 'static,
{
    type System = FunctionSystem<F, Marker>;

    #[inline]
    fn into_system(this: Self) -> Self::System {
        FunctionSystem::new(this)
    }
}

/// Marker type selecting the exclusive-system blanket impl of
/// [`IntoSystem`].
///
/// Phase 9 Wave 3 Step 8 introduces this marker so the Phase 8c
/// `SystemParamFunction`-based blanket (keyed by
/// `(IsFunctionSystem, Marker)`) and the new exclusive-system blanket
/// (keyed by `(ExclusiveSystemMarker, fn(&mut EcsMaster))`) coexist
/// without overlapping per the coherence checker. The marker is a
/// zero-sized unit struct in our crate; no fields, no behaviour — purely
/// a nominal disambiguator.
///
/// # Coherence proof (Phase 9 plan §3 Q9.1 / Q9.2)
///
/// Two `IntoSystem` blanket impls could in principle conflict if a
/// single `F` could satisfy both at the same `Marker`. The Phase 9
/// design proves this impossible by construction:
///
/// 1. The Phase 8c blanket bounds `F: SystemParamFunction<Marker>`. The
///    `SystemParamFunction` trait is implemented for closures whose
///    parameters all satisfy [`SystemParam`]. `&mut EcsMaster` is **not**
///    a `SystemParam` (intentional design — the dedicated
///    `Commands` / `Res` / `Query` params route through the access
///    graph instead).
/// 2. The new blanket bounds `F: FnMut(&mut EcsMaster) + Send + Sync +
///    'static`. The single parameter type is `&mut EcsMaster`.
///
/// No closure can simultaneously satisfy "every param is a
/// `SystemParam`" (point 1) AND "exactly one param is `&mut EcsMaster`"
/// (point 2), because `&mut EcsMaster` fails point 1. The third type
/// argument (`(IsFunctionSystem, Marker)` vs `(ExclusiveSystemMarker,
/// fn(&mut EcsMaster))`) is therefore non-overlapping at the type
/// level; the coherence checker accepts both impls.
///
/// [`SystemParam`]: super::system_param::SystemParam
pub struct ExclusiveSystemMarker;

/// Exclusive-system blanket: any `F: FnMut(&mut EcsMaster) + Send + Sync
/// + 'static` is convertible into an [`ExclusiveFunctionSystem<F>`].
///
/// The marker tuple `(ExclusiveSystemMarker, fn(&mut EcsMaster))`
/// mirrors the Phase 8c marker shape — a ZST disambiguator paired with
/// a function-pointer signature that captures the "shape" of the call.
/// The function-pointer half is load-bearing for inference: spelling it
/// `fn(&mut EcsMaster)` lets rustc pick this impl on a bare
/// `|w: &mut EcsMaster| { ... }` closure without explicit turbofish.
///
/// See Phase 9 plan §3 Q9.1 for the full coherence proof; the
/// [`ExclusiveSystemMarker`] doc-comment summarises it.
impl<F> IntoSystem<(), (), (ExclusiveSystemMarker, fn(&mut EcsMaster))> for F
where
    F: FnMut(&mut EcsMaster) + Send + Sync + 'static,
{
    type System = ExclusiveFunctionSystem<F>;

    #[inline]
    fn into_system(this: Self) -> Self::System {
        ExclusiveFunctionSystem::new(this)
    }
}
