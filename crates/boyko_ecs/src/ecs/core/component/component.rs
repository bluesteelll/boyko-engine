use std::any::TypeId;
use crate::ecs::identifiers::primitives::ComponentId;

/// Marker trait for ECS component types.
///
/// Implemented automatically via `#[derive(Component)]`. Each type gets a
/// unique [`ComponentId`] assigned on first call to [`component_id`] — see
/// the [`component_registry`] module docs for the startup warm-up contract
/// and collision-detection semantics.
///
/// # Panic safety
///
/// `<Self as Drop>::drop` (if any) **must not panic**. If a panic escapes
/// `Component::drop` during `ComponentPool::Drop` (executed during archetype
/// or `EcsMaster` teardown), and any other panic is already in progress,
/// the process aborts. This matches `Vec<T>`'s policy for its element type.
///
/// Concretely:
/// - Do not call user code that may panic from a `Drop` impl on a `Component`.
/// - Do not use `expect` / `unwrap` / `assert!` in `Drop`.
/// - Prefer owning heap data via `Vec` / `Box` / `Arc`, which have
///   well-defined non-panicking drop semantics.
///
/// `ComponentPool::Drop` is NOT wrapped in `catch_unwind`; the overhead
/// (~20-30 ns per element) is unacceptable on a teardown path. The escape
/// hatch for users who cannot guarantee panic-free `Drop`: wrap the entire
/// `EcsMaster` lifetime in `std::panic::catch_unwind` at the application
/// boundary, and on catch, discard the master and never touch it again.
///
/// [`component_id`]: Component::component_id
/// [`component_registry`]: crate::ecs::core::component::component_registry
pub trait Component: 'static + Sized {
    /// Returns the unique identifier for this component type.
    ///
    /// The first call mints the ID via the global registry; subsequent calls
    /// return the cached value from a per-type `OnceLock` — no atomic on the
    /// hot path after initialization.
    fn component_id() -> ComponentId;

    #[inline]
    fn debug_type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    #[inline]
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }

    #[inline]
    fn mem_size() -> usize {
        std::mem::size_of::<Self>()
    }

    #[inline]
    fn alignment() -> usize {
        std::mem::align_of::<Self>()
    }
}
