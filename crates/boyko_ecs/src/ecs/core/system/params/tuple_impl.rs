//! Tuple `SystemParam` impls and arity-overflow diagnostic stubs.
//!
//! See Phase 8a plan §7 (D5 / M7 / C-NEW-2 resolution).
//!
//! # Strategy
//!
//! A single `macro_rules!` site emits the `SystemParam` impl for every
//! tuple arity in `0..=MAX_SYSTEM_PARAM_ARITY` (Bevy's `all_tuples!`
//! does the same internally). The empty tuple `()` is the base case;
//! arities `1..=12` come from twelve explicit macro invocations.
//!
//! # Arity overflow (M7 + C-NEW-2)
//!
//! For tuples with arity `13..=24` a second macro emits stub impls
//! whose method bodies contain `const { panic!(...) }`. The const
//! block evaluates ONLY at monomorphization (when a user actually
//! instantiates the impl), so the wider crate compiles cleanly even
//! though twelve oversized impls are registered. Users who try to use
//! a 13-arity tuple system get a focused error pointing them at
//! `MAX_SYSTEM_PARAM_ARITY` instead of a wall of "trait not satisfied"
//! diagnostics. `compile_error!` was rejected (C-NEW-2): it fires at
//! macro-expand time and would break every downstream crate.
//!
//! `const { panic!(...) }` requires `rustc >= 1.79`; boyko targets the
//! Rust 2024 edition (`rustc >= 1.85`) so the dependency is
//! unconditionally satisfied.

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Maximum tuple arity for which a working `SystemParam` impl is
/// emitted. Tuples with arity in `13..=24` carry a stub impl whose
/// methods `const { panic!(...) }` at monomorphization time; anything
/// beyond arity 24 simply has no impl and falls back to the standard
/// "trait not satisfied" error.
///
/// Raising the cap is a one-line patch (add a macro invocation in
/// `tuple_impl.rs`); the cost is roughly four extra trait
/// monomorphisations per concrete tuple used (`SystemParam`,
/// `ReadOnlySystemParam`, etc. in later phases).
pub const MAX_SYSTEM_PARAM_ARITY: usize = 12;

// ── Working impls (arity 0..=12) ───────────────────────────────────────────

// Empty-tuple base case. Required because the variadic macro below
// cannot emit a parameterless `impl<>` block cleanly under Rust's
// generic-parameter syntax.
//
// SAFETY (SP1, SP2, SP4): the empty tuple has no access surface, no
//   state, and no per-invocation view. Every invariant holds vacuously.
unsafe impl SystemParam for () {
    type State = ();
    type Item<'w, 's> = ();

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {}

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        _state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
    }
}

/// Emits a `SystemParam` impl for a tuple of the given type-parameter
/// names. Used for arity `1..=MAX_SYSTEM_PARAM_ARITY` — see the
/// invocations below.
macro_rules! impl_system_param_tuple {
    ($($p:ident),*) => {
        // SAFETY (SP1, SP2, SP4): each per-param impl already upholds
        //   SP1/SP2/SP4 by its own contract; the tuple impl simply
        //   forwards `init_state` / `init_access` / `get_param` to each
        //   element in declaration order. Intra-system aliasing among
        //   the elements is caught by `FilteredAccessSet::add_*` during
        //   the threaded `init_access` walk.
        unsafe impl<$($p: SystemParam),*> SystemParam for ($($p,)*) {
            type State = ($($p::State,)*);
            type Item<'w, 's> = ($($p::Item<'w, 's>,)*);

            #[inline]
            fn init_state(
                world: &mut EcsMaster,
                system_meta: &mut SystemMeta,
            ) -> Self::State {
                ($(<$p as SystemParam>::init_state(world, system_meta),)*)
            }

            #[inline]
            fn init_access(
                state: &Self::State,
                system_meta: &mut SystemMeta,
                access_set: &mut FilteredAccessSet,
                world: &mut EcsMaster,
            ) {
                #[allow(non_snake_case)]
                let ($($p,)*) = state;
                $(
                    <$p as SystemParam>::init_access(
                        $p, system_meta, access_set, world,
                    );
                )*
            }

            #[inline]
            unsafe fn get_param<'w, 's>(
                state: &'s mut Self::State,
                system_meta: &SystemMeta,
                world: UnsafeEcsCell<'w>,
            ) -> Self::Item<'w, 's> {
                #[allow(non_snake_case)]
                let ($($p,)*) = state;
                // SAFETY (SP1, SP2, SP3): every element's `get_param`
                //   contract is upheld by the caller (sibling-aliasing
                //   resolved at `init_access` via FilteredAccessSet,
                //   cross-system aliasing resolved by the scheduler).
                //   `UnsafeEcsCell` is `Copy`; passing it by value to
                //   each element preserves the raw-pointer provenance
                //   (C1 / by-value receivers).
                ($(
                    unsafe {
                        <$p as SystemParam>::get_param($p, system_meta, world)
                    },
                )*)
            }
        }
    };
}

impl_system_param_tuple!(P0);
impl_system_param_tuple!(P0, P1);
impl_system_param_tuple!(P0, P1, P2);
impl_system_param_tuple!(P0, P1, P2, P3);
impl_system_param_tuple!(P0, P1, P2, P3, P4);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10);
impl_system_param_tuple!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11);

// ── Diagnostic stubs (arity 13..=24) — M7 + C-NEW-2 ────────────────────────
//
// Each stub impl has `State = ()` / `Item<'w, 's> = ()` so it type-checks
// in isolation. Its method bodies all contain `const { panic!(...) }`,
// which evaluates only at monomorphization — when a user actually
// instantiates the impl. Code that never touches a 13+ arity tuple as
// `SystemParam` compiles cleanly.

/// Emits a stub `SystemParam` impl for an oversized tuple. Every method
/// body is `const { panic!(...) }`; the panic fires at monomorphization
/// (not at macro-expand) so the host crate keeps compiling.
macro_rules! impl_system_param_tuple_too_large {
    ($($p:ident),*) => {
        // SAFETY: stub impl whose every method body is
        //   `const { panic!(...) }`. The impl is never *successfully*
        //   used at runtime — the const block fails at monomorphization
        //   with the diagnostic in `init_state`. SP1/SP2/SP4 are
        //   vacuously upheld because no code path that respects the
        //   contract ever observes the impl's effects.
        unsafe impl<$($p: SystemParam),*> SystemParam for ($($p,)*) {
            type State = ();
            type Item<'w, 's> = ();

            fn init_state(
                _world: &mut EcsMaster,
                _system_meta: &mut SystemMeta,
            ) -> Self::State {
                const {
                    panic!(
                        "tuple has too many SystemParam elements. \
                         boyko-engine supports up to \
                         MAX_SYSTEM_PARAM_ARITY = 12. Split your system \
                         into smaller systems or wrap related params in \
                         a struct that implements SystemParam."
                    )
                }
            }

            fn init_access(
                _state: &Self::State,
                _system_meta: &mut SystemMeta,
                _access_set: &mut FilteredAccessSet,
                _world: &mut EcsMaster,
            ) {
                const { panic!("tuple too large: see init_state diagnostic") }
            }

            unsafe fn get_param<'w, 's>(
                _state: &'s mut Self::State,
                _system_meta: &SystemMeta,
                _world: UnsafeEcsCell<'w>,
            ) -> Self::Item<'w, 's> {
                const { panic!("tuple too large: see init_state diagnostic") }
            }
        }
    };
}

impl_system_param_tuple_too_large!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12);
impl_system_param_tuple_too_large!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18, P19
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18, P19, P20
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18, P19, P20,
    P21
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18, P19, P20,
    P21, P22
);
impl_system_param_tuple_too_large!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16, P17, P18, P19, P20,
    P21, P22, P23
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::marker::PhantomData;

    /// Compile-only shim: instantiating `assert_impl::<T>()` proves `T`
    /// satisfies `SystemParam`. Used by the test bodies below.
    fn assert_impl<T: SystemParam>() {}

    /// Minimal in-test `SystemParam` impl so tuple impls can be checked
    /// without depending on Step 7's `Res<R>` / `ResMut<R>`. Has no
    /// access surface and a unit state.
    struct DummyParam;

    /// State for [`DummyParam`] — `Send + Sync + 'static`.
    #[derive(Default)]
    struct DummyState {
        _marker: PhantomData<fn() -> ()>,
    }

    // SAFETY (SP1, SP2, SP4): test-only stub. `DummyParam` declares no
    //   access in `init_access`, performs no `world` mutation in
    //   `init_state`, and returns a unit `Item`. Every invariant holds
    //   vacuously.
    unsafe impl SystemParam for DummyParam {
        type State = DummyState;
        type Item<'w, 's> = DummyParam;

        fn init_state(_world: &mut EcsMaster, _meta: &mut SystemMeta) -> Self::State {
            DummyState::default()
        }

        fn init_access(
            _state: &Self::State,
            _meta: &mut SystemMeta,
            _access_set: &mut FilteredAccessSet,
            _world: &mut EcsMaster,
        ) {
        }

        unsafe fn get_param<'w, 's>(
            _state: &'s mut Self::State,
            _meta: &SystemMeta,
            _world: UnsafeEcsCell<'w>,
        ) -> Self::Item<'w, 's> {
            DummyParam
        }
    }

    /// The empty tuple implements `SystemParam` (base case).
    #[test]
    fn empty_tuple_is_system_param() {
        assert_impl::<()>();
    }

    /// A 1-arity tuple over an in-test `SystemParam` compiles.
    #[test]
    fn single_dummy_param_tuple_is_system_param() {
        assert_impl::<(DummyParam,)>();
    }

    /// A 12-arity tuple — the documented cap — compiles.
    #[test]
    fn arity_12_tuple_is_system_param() {
        assert_impl::<(
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
            DummyParam,
        )>();
    }

    /// `MAX_SYSTEM_PARAM_ARITY` carries the documented cap.
    #[test]
    fn max_arity_constant_value() {
        assert_eq!(MAX_SYSTEM_PARAM_ARITY, 12);
    }
}
