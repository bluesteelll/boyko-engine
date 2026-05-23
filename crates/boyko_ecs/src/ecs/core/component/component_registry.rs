use std::alloc::Layout;
use std::any::TypeId;
use std::sync::OnceLock;

/// Maximum number of components supported by the ECS system.
pub const MAX_COMPONENTS: usize = 512;

/// Holds layout information for a specific component type.
///
/// Filled in by `register_layout::<T>(id)` (currently invoked from the
/// `#[derive(Component)]`-generated `#[ctor::ctor]` initializer). Each entry
/// is written exactly once via `OnceLock::set` and read lock-free via
/// `OnceLock::get` — fixes the race / `static mut` UB called out by audit
/// findings M-002 / C-002 / Q-010 in `docs/AUDIT-2026-05-23.md`.
#[derive(Clone, Copy, Debug)]
pub struct ComponentLayout {
    /// Size of the component in bytes.
    pub size: usize,
    /// Alignment requirement for the component.
    pub alignment: usize,
    /// The component's type name (for debugging).
    pub type_name: &'static str,
    /// Unique type identifier.
    pub type_id: TypeId,
}

impl ComponentLayout {
    /// Creates a new `ComponentLayout` with static information about type `T`.
    #[inline]
    pub fn new_static<T: 'static>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            alignment: std::mem::align_of::<T>(),
            type_name: std::any::type_name::<T>(),
            type_id: TypeId::of::<T>(),
        }
    }

    /// Returns a memory layout object for this component.
    #[inline]
    pub fn layout(&self) -> Layout {
        // SAFETY: size/alignment originated from `size_of::<T>()` /
        // `align_of::<T>()` for some `T: 'static`. Those are valid by the
        // language definition — alignment is a power of two and size fits in
        // `isize::MAX` (otherwise `T` would not have a layout).
        unsafe { Layout::from_size_align_unchecked(self.size, self.alignment) }
    }
}

/// Static storage for component layouts. Each slot is independent and
/// initialized at most once via `OnceLock::set`. Read path is a single
/// acquire-load + branch — no `Mutex`, no `static mut`, no data race.
static LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Registers a component's layout information in the global registry.
/// Typically called during program initialization from the
/// `#[derive(Component)]`-generated `#[ctor::ctor]` block.
///
/// Idempotent: if the slot is already initialized (e.g. duplicate registration
/// from a second translation unit) the call is silently ignored. The audit
/// finding C-003 (ID collision detection across compilation units) is tracked
/// separately — this function only fixes the storage race.
pub fn register_layout<T: 'static>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );

    if component_id >= MAX_COMPONENTS {
        return;
    }

    let layout = ComponentLayout::new_static::<T>();
    // `OnceLock::set` returns Err if the slot was already initialized; we
    // ignore that — see doc-comment above for the rationale.
    let _ = LAYOUTS[component_id].set(layout);
}

/// Retrieves layout information for a component by its ID.
/// Returns `None` if the component hasn't been registered yet.
#[inline]
pub fn get_layout(component_id: usize) -> Option<&'static ComponentLayout> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );

    if component_id >= MAX_COMPONENTS {
        return None;
    }

    LAYOUTS[component_id].get()
}

/// Optimized function to get the size of a component by ID.
#[inline]
pub fn get_component_size(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.size)
}

/// Optimized function to get the alignment of a component by ID.
#[inline]
pub fn get_component_alignment(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.alignment)
}

/// Creates a memory layout for a component without going through `ComponentLayout`.
#[inline]
pub fn get_component_memory_layout(component_id: usize) -> Option<Layout> {
    Some(get_layout(component_id)?.layout())
}

/// Ultra-fast access to component size when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that
/// `register_layout::<T>(component_id)` has already completed for the
/// corresponding type `T`. Violating either yields UB.
#[inline(always)]
pub unsafe fn get_component_size_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().size }
}

/// Ultra-fast access to component alignment when you're confident the component exists.
///
/// # Safety
/// See [`get_component_size_unchecked`].
#[inline(always)]
pub unsafe fn get_component_alignment_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().alignment }
}

/// Ultra-fast access to component layout when you're confident the component exists.
///
/// # Safety
/// See [`get_component_size_unchecked`].
#[inline(always)]
pub unsafe fn get_layout_unchecked(component_id: usize) -> &'static ComponentLayout {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked() }
}

/// Ultra-fast access to component memory layout when you're confident the component exists.
///
/// # Safety
/// See [`get_component_size_unchecked`].
#[inline(always)]
pub unsafe fn get_component_memory_layout_unchecked(component_id: usize) -> Layout {
    // SAFETY: forwarded to the unchecked accessor below.
    let layout = unsafe { get_layout_unchecked(component_id) };
    // SAFETY: size/alignment come from a registered `ComponentLayout`, valid by construction.
    unsafe { Layout::from_size_align_unchecked(layout.size, layout.alignment) }
}

/// Ultra-fast access to component type ID when you're confident the component exists.
///
/// # Safety
/// See [`get_component_size_unchecked`].
#[inline(always)]
pub unsafe fn get_component_type_id_unchecked(component_id: usize) -> TypeId {
    // SAFETY: forwarded to the unchecked accessor.
    unsafe { get_layout_unchecked(component_id).type_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use large IDs in this module to avoid colliding with other test modules
    // that register components under low IDs (0-10). OnceLock slots are global
    // and persist for the lifetime of the test binary.
    const TEST_BASE: usize = 450;

    // --- register_layout + get_layout ---

    #[test]
    fn register_layout_then_get_returns_matching_fields() {
        let id = TEST_BASE;
        register_layout::<u32>(id);
        let layout = get_layout(id).expect("layout must be present after register");
        assert_eq!(layout.size, std::mem::size_of::<u32>(), "size must match u32");
        assert_eq!(
            layout.alignment,
            std::mem::align_of::<u32>(),
            "alignment must match u32"
        );
        assert_eq!(
            layout.type_id,
            TypeId::of::<u32>(),
            "type_id must match TypeId::of::<u32>()"
        );
    }

    #[test]
    fn get_layout_unregistered_returns_none() {
        // ID 499 is unused by any other test in this crate.
        assert!(
            get_layout(499).is_none(),
            "unregistered component must return None"
        );
    }

    #[test]
    fn register_layout_idempotent_same_type() {
        // Registering the same type twice must keep the first registration.
        let id = TEST_BASE + 1;
        register_layout::<u64>(id);
        register_layout::<u64>(id); // second call — must be silent no-op
        let layout = get_layout(id).expect("slot must remain populated");
        assert_eq!(layout.size, std::mem::size_of::<u64>());
    }

    #[test]
    fn register_layout_idempotent_different_type_keeps_first() {
        // A second registration with a *different* type is silently ignored
        // (known limitation — see audit C-003: ID collision not yet detected).
        // We verify that the first registration wins and no panic occurs.
        let id = TEST_BASE + 2;
        register_layout::<u8>(id);
        register_layout::<u64>(id); // collision — ignored
        let layout = get_layout(id).expect("first registration must survive");
        // The first type (u8, size=1) must have been preserved.
        assert_eq!(
            layout.size,
            std::mem::size_of::<u8>(),
            "first registration (u8) must not be overwritten by second (u64)"
        );
    }

    #[test]
    fn register_layout_at_max_components_boundary_is_safe() {
        // MAX_COMPONENTS = 512 is out-of-range (valid indices: 0..511).
        // In debug, debug_assert fires; in release, `if` guard returns silently.
        // catch_unwind handles the debug panic; get_layout with the same OOB ID
        // is *also* wrapped because it has its own debug_assert.
        let _register = std::panic::catch_unwind(|| {
            register_layout::<u32>(MAX_COMPONENTS);
        });
        // Even if register panicked, the slot must not exist.
        // Also wrap get_layout since it has a debug_assert on the same ID.
        let slot_is_none = std::panic::catch_unwind(|| {
            get_layout(MAX_COMPONENTS)
        });
        // Either it panicked (debug_assert) or returned None — both are acceptable.
        if let Ok(result) = slot_is_none {
            assert!(result.is_none(), "out-of-range ID must yield None");
        }
        // If catch_unwind returned Err the debug_assert fired — also acceptable.
    }

    #[test]
    fn get_layout_at_index_zero_is_none_before_register() {
        // Index 0 is a valid index but we never register a component there
        // in this test module. It may have been registered by another module —
        // we can only assert that get_layout does not panic on index 0.
        let _ = get_layout(0); // must not panic regardless of value
    }

    // --- get_component_size / get_component_alignment ---

    #[test]
    fn get_component_size_matches_registered_layout() {
        let id = TEST_BASE + 3;
        register_layout::<f32>(id);
        assert_eq!(
            get_component_size(id),
            Some(std::mem::size_of::<f32>()),
            "get_component_size must agree with size_of::<f32>()"
        );
    }

    #[test]
    fn get_component_alignment_matches_registered_layout() {
        let id = TEST_BASE + 4;
        register_layout::<f64>(id);
        assert_eq!(
            get_component_alignment(id),
            Some(std::mem::align_of::<f64>()),
            "get_component_alignment must agree with align_of::<f64>()"
        );
    }

    #[test]
    fn get_component_size_unregistered_returns_none() {
        assert!(
            get_component_size(498).is_none(),
            "unregistered component must return None from get_component_size"
        );
    }

    // --- get_layout_unchecked (unsafe hot path) ---

    #[test]
    fn get_layout_unchecked_after_register_returns_correct_size() {
        let id = TEST_BASE + 5;
        register_layout::<f32>(id);
        // SAFETY: `id` is < MAX_COMPONENTS and `register_layout` was just called.
        let layout = unsafe { get_layout_unchecked(id) };
        assert_eq!(
            layout.size,
            std::mem::size_of::<f32>(),
            "unchecked accessor must return the registered layout"
        );
    }

    // --- get_component_memory_layout ---

    #[test]
    fn get_component_memory_layout_produces_valid_layout() {
        let id = TEST_BASE + 6;
        register_layout::<u128>(id);
        let mem_layout =
            get_component_memory_layout(id).expect("must return Some after register");
        assert_eq!(mem_layout.size(), std::mem::size_of::<u128>());
        assert_eq!(mem_layout.align(), std::mem::align_of::<u128>());
    }
}
