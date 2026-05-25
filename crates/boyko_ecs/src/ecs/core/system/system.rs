//! The `System` trait — type-erased handle every concrete system implements.
//!
//! See Phase 8a plan §8 (Decision D6). Phase 8a ships exactly one impl —
//! [`FnOnceSystem`] in [`super::fn_once_system`]. Phase 8c will add
//! `FunctionSystem<F, M>` via the `IntoSystem` adapter; both will plug into
//! [`EcsMaster::run_system_once`] generically, no virtual dispatch.
//!
//! # Why `unsafe trait`
//!
//! Implementations of [`System::run_unsafe`] take an [`UnsafeEcsCell`] —
//! the by-value, raw-pointer-mediated handle on the world. Calling
//! `run_unsafe` while another `System` is live on the same world is UB
//! (raw-pointer aliasing without the borrow checker's protection). The
//! invariant cannot be expressed in the type system, so the trait carries
//! it as a safety contract — see invariant S1 below.
//!
//! [`FnOnceSystem`]: super::fn_once_system::FnOnceSystem
//! [`UnsafeEcsCell`]: super::unsafe_ecs_cell::UnsafeEcsCell
//! [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Type-erased system handle. Every concrete system implements this trait;
/// [`EcsMaster::run_system_once`] is generic over `S: System` so the caller's
/// system survives across calls without `Box<dyn System>`.
///
/// # `Send + Sync + 'static`
///
/// Required so the Phase 9 scheduler can migrate systems across worker
/// threads. The bound is non-negotiable — every Phase 8a impl satisfies it
/// trivially (closures captured by [`FnOnceSystem`] inherit `Send + Sync`
/// from their captures; [`SystemMeta`] and `P::State` are `Send + Sync` by
/// construction).
///
/// # Safety
///
/// **S1** — The caller of [`run_unsafe`](Self::run_unsafe) must assert that
/// no other `System::run_unsafe` is in flight on the same [`EcsMaster`].
/// Phase 9's scheduler enforces this via the [`Access`] conflict graph;
/// Phase 8a's [`EcsMaster::run_system_once`] enforces it trivially by
/// taking `&mut EcsMaster` (the borrow checker forbids re-entry while the
/// system holds the exclusive borrow).
///
/// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
/// [`SystemMeta`]: super::system_meta::SystemMeta
pub unsafe trait System: Send + Sync + 'static {
    /// Output of the system body — typically `()`, occasionally a value
    /// extracted by the body for the caller (e.g. a probe in tests).
    type Out;

    /// Returns the diagnostic name of the system. For
    /// [`FnOnceSystem`](super::fn_once_system::FnOnceSystem) this is
    /// `std::any::type_name::<F>()`; for the Phase 8c `FunctionSystem<F>`
    /// it will be the same.
    fn name(&self) -> &'static str;

    /// Returns the declared [`Access`] surface. Empty until
    /// [`initialize`](Self::initialize) runs; populated thereafter by the
    /// system's `SystemParam` chain.
    fn access(&self) -> &Access;

    /// Two-phase initialisation: builds the per-system state and declares
    /// the access surface. Idempotent — calling twice has no additional
    /// effect.
    ///
    /// Called once per system before the first [`run_unsafe`](Self::run_unsafe)
    /// invocation. The Phase 8a [`EcsMaster::run_system_once`] calls it
    /// implicitly; Phase 9's scheduler will call it at system registration
    /// time.
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    fn initialize(&mut self, world: &mut EcsMaster);

    /// Runs the system body once.
    ///
    /// # Safety
    ///
    /// **S1** — The caller must guarantee that no other `System::run_unsafe`
    /// is in flight on the same world for the duration of this call. See
    /// the trait-level safety section for the full rationale; Phase 8a's
    /// [`EcsMaster::run_system_once`] enforces the invariant trivially by
    /// taking `&mut EcsMaster`.
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::system::system_meta::SystemMeta;

    /// Minimal compile-only impl proving the trait shape is buildable. The
    /// concrete `FnOnceSystem` impl lives in
    /// [`super::super::fn_once_system`]; this one exists so unit tests in
    /// the same crate can exercise the trait's bare contract without
    /// pulling in `SystemParam` machinery.
    struct NoopSystem {
        meta: SystemMeta,
    }

    // SAFETY (S1): `run_unsafe` performs no world access; the contract is
    //   vacuously upheld for any caller.
    unsafe impl System for NoopSystem {
        type Out = ();

        fn name(&self) -> &'static str {
            self.meta.name()
        }

        fn access(&self) -> &Access {
            self.meta.access()
        }

        fn initialize(&mut self, _world: &mut EcsMaster) {}

        unsafe fn run_unsafe(&mut self, _world: UnsafeEcsCell<'_>) -> Self::Out {}
    }

    /// Compile-only assertion that `NoopSystem: System` (and therefore
    /// `Send + Sync + 'static`).
    fn assert_impl<S: System>() {
        let _ = std::marker::PhantomData::<S>;
    }

    #[test]
    fn noop_system_implements_trait() {
        assert_impl::<NoopSystem>();
        let sys = NoopSystem {
            meta: SystemMeta::new("noop"),
        };
        assert_eq!(sys.name(), "noop");
    }
}
