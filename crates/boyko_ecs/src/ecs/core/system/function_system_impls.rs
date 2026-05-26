//! Variadic blanket impls of [`SystemParamFunction`] for arity 0..=12.
//!
//! See Phase 8c+8d plan §4 (Decision C2 — double-`FnMut` HRTB bound) and
//! §7 (Decision C5 — variadic tuple impls). This file is paired with the
//! trait declaration in [`function_system`](super::function_system); the
//! split keeps the blanket impls (mechanical, ~13 entries) separate from
//! the trait + struct surface (small, hand-edited).
//!
//! # Why a macro
//!
//! Each blanket impl follows the same skeleton:
//!
//! ```ignore
//! impl<Out, P0, P1, ..., Pn, F> SystemParamFunction<fn(P0, P1, ..., Pn) -> Out> for F
//! where
//!     Out: 'static,
//!     P0: SystemParam + 'static,
//!     ..,
//!     Pn: SystemParam + 'static,
//!     F: Send + Sync + 'static
//!        + FnMut(P0, P1, ..., Pn) -> Out
//!        + for<'w, 's> FnMut(
//!             <P0 as SystemParam>::Item<'w, 's>,
//!             ..,
//!             <Pn as SystemParam>::Item<'w, 's>,
//!         ) -> Out,
//! { ... }
//! ```
//!
//! Hand-writing thirteen near-identical impls invites copy-paste drift; a
//! `macro_rules!` site keeps the bound list and `run` body in lock-step
//! across all arities. The empty-tuple base case (arity 0) is emitted as a
//! standalone impl because the macro cannot synthesise the `()` `Param`
//! type without a separate branch.
//!
//! # The double-`FnMut` HRTB bound (plan §4.7, invariant FS3)
//!
//! Each non-empty impl carries TWO `FnMut` bounds:
//!
//! * `FnMut(P0, P1, ..., Pn) -> Out` — the direct call shape.
//! * `for<'w, 's> FnMut(<P0 as SystemParam>::Item<'w, 's>, ...) -> Out` —
//!   the GAT-projected call shape that the actual `run` body uses.
//!
//! The double bound is **load-bearing** for rustc's closure-argument
//! inference (see plan §4.7 and the `tests/into_system_closure_inference.rs`
//! reproducer). Removing either bound regresses the user-facing ergonomic
//! claim that `|q: Query<&Position>|` compiles without explicit `<'_, '_>`
//! annotations. Do NOT simplify.
//!
//! # Why the `call_inner` helper
//!
//! The `run` body invokes `self` through a tiny inner fn whose signature
//! takes a single `F: FnMut(P0, P1, ..., Pn) -> Out`. This indirection
//! steers rustc into resolving the HRTB-projected call against the
//! `for<'w, 's> FnMut(...)` bound (whose return type is `Out`) rather than
//! the direct `FnMut(P0, ..., Pn) -> Out` bound. Both bounds resolve to the
//! same machine code; the helper exists for inference, not for codegen.

use crate::ecs::core::system::function_system::SystemParamFunction;
use crate::ecs::core::system::system_param::SystemParam;

// ── Arity 0 — empty tuple base case ────────────────────────────────────────

impl<Out, F> SystemParamFunction<fn() -> Out> for F
where
    Out: 'static,
    F: Send + Sync + 'static + FnMut() -> Out,
{
    type In = ();
    type Out = Out;
    type Param = ();

    #[inline]
    fn run(
        &mut self,
        _input: Self::In,
        _params: <Self::Param as SystemParam>::Item<'_, '_>,
    ) -> Self::Out {
        // The inner-fn dispatch used by arities 1..=12 (see the macro
        // below) is unnecessary for arity 0 — there is no `Param` tuple
        // to destructure and no HRTB-projected call shape to disambiguate.
        // Calling `self` directly is both simpler and what clippy expects.
        (self)()
    }
}

// ── Arity 1..=12 — variadic macro ──────────────────────────────────────────

/// Emits a `SystemParamFunction` blanket impl for an arity-N function shape.
///
/// Each invocation passes the type-parameter names (`P0`, `P1`, ...) and
/// matching destructure-binding names (`p0`, `p1`, ...). The macro stamps
/// out the double-`FnMut` HRTB bound, the tuple `Param` projection, and
/// the `run` body's destructure + `call_inner` dispatch.
macro_rules! impl_system_param_function {
    ($(($p:ident, $b:ident)),+ $(,)?) => {
        impl<Out, $($p,)+ F> SystemParamFunction<fn($($p,)+) -> Out> for F
        where
            Out: 'static,
            $($p: SystemParam + 'static,)+
            F: Send + Sync + 'static
               + FnMut($($p,)+) -> Out
               + for<'w, 's> FnMut(
                    $(<$p as SystemParam>::Item<'w, 's>,)+
                 ) -> Out,
        {
            type In = ();
            type Out = Out;
            type Param = ($($p,)+);

            #[inline]
            fn run(
                &mut self,
                _input: Self::In,
                params: <Self::Param as SystemParam>::Item<'_, '_>,
            ) -> Self::Out {
                // Clippy `too_many_arguments` fires on arities >= 8; the
                // variadic shape REQUIRES these signatures (the trait's
                // public surface promises 0..=12 arity). The lint is a
                // style suggestion for hand-written code, not a soundness
                // signal — the helper is private to this macro expansion.
                // Identical posture to Phase 8a's `tuple_impl` macro.
                #[allow(clippy::too_many_arguments)]
                fn call_inner<Out, $($p,)+ F: FnMut($($p,)+) -> Out>(
                    mut f: F,
                    $($b: $p,)+
                ) -> Out {
                    f($($b,)+)
                }
                let ($($b,)+) = params;
                call_inner(self, $($b,)+)
            }
        }
    };
}

impl_system_param_function!((P0, p0));
impl_system_param_function!((P0, p0), (P1, p1));
impl_system_param_function!((P0, p0), (P1, p1), (P2, p2));
impl_system_param_function!((P0, p0), (P1, p1), (P2, p2), (P3, p3));
impl_system_param_function!((P0, p0), (P1, p1), (P2, p2), (P3, p3), (P4, p4));
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
    (P7, p7),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
    (P7, p7),
    (P8, p8),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
    (P7, p7),
    (P8, p8),
    (P9, p9),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
    (P7, p7),
    (P8, p8),
    (P9, p9),
    (P10, p10),
);
impl_system_param_function!(
    (P0, p0),
    (P1, p1),
    (P2, p2),
    (P3, p3),
    (P4, p4),
    (P5, p5),
    (P6, p6),
    (P7, p7),
    (P8, p8),
    (P9, p9),
    (P10, p10),
    (P11, p11),
);
