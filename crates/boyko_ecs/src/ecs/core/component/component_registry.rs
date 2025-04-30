use std::alloc::Layout;
use std::any::TypeId;
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum number of components supported by the ECS system
const MAX_COMPONENTS: usize = 512;

/// Holds layout information for a specific component type
#[derive(Clone, Copy, Debug)]
pub struct ComponentLayout {
    /// Size of the component in bytes
    pub size: usize,
    /// Alignment requirement for the component
    pub alignment: usize,
    /// The component's type name (for debugging)
    pub type_name: &'static str,
    /// Unique type identifier
    pub type_id: TypeId,
}

impl ComponentLayout {
    /// Creates a new ComponentLayout with static information about type T
    pub const fn new_static<T: 'static>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            alignment: std::mem::align_of::<T>(),
            type_name: std::any::type_name::<T>(),
            type_id: TypeId::of::<T>(),
        }
    }
    
    /// Returns a memory layout object for this component
    /// This is optimized to avoid unnecessary validation
    #[inline(always)]
    pub fn layout(&self) -> Layout {
        unsafe { Layout::from_size_align_unchecked(self.size, self.alignment) }
    }
}

/// Static registry for component layouts
/// Uses fixed-size arrays to avoid dynamic allocation
struct StaticLayoutRegistry {
    /// Array of component layouts
    /// The index corresponds to the component ID
    layouts: [ComponentLayout; MAX_COMPONENTS],
    
    /// Initialization flags for each component
    /// Each flag indicates whether the corresponding layout has been initialized
    initialized: [AtomicBool; MAX_COMPONENTS],
}

// Create a zeroed ComponentLayout for initialization
const ZEROED_LAYOUT: ComponentLayout = ComponentLayout {
    size: 0,
    alignment: 0,
    type_name: "",
    type_id: TypeId::of::<()>(),
};

// Create a zeroed array of AtomicBool
const ZEROED_ATOMIC_BOOL: AtomicBool = AtomicBool::new(false);

/// The global static registry instance
static REGISTRY: StaticLayoutRegistry = StaticLayoutRegistry {
    layouts: [ZEROED_LAYOUT; MAX_COMPONENTS],
    initialized: [ZEROED_ATOMIC_BOOL; MAX_COMPONENTS],
};

/// Registers a component's layout information in the global registry
/// This is typically called during program initialization
pub fn register_layout<T: 'static>(component_id: usize) {
    // Check bounds only in debug builds
    debug_assert!(component_id < MAX_COMPONENTS, 
        "Component ID {} exceeds maximum allowed ({})", component_id, MAX_COMPONENTS);
    
    // First check if the layout is already initialized (fast path)
    if !REGISTRY.initialized[component_id].load(Ordering::Relaxed) {
        // Create the layout - this is a completely static operation with no allocation
        let layout = ComponentLayout::new_static::<T>();
        
        // Try to mark as initialized using atomic swap
        if !REGISTRY.initialized[component_id].swap(true, Ordering::AcqRel) {
            // We won the race - write the layout to the static array
            unsafe {
                std::ptr::write(
                    &REGISTRY.layouts[component_id] as *const ComponentLayout as *mut ComponentLayout,
                    layout
                );
            }
        }
    }
}

/// Retrieves layout information for a component by its ID
/// Returns None if the component hasn't been registered yet
#[inline]
pub fn get_layout(component_id: usize) -> Option<&'static ComponentLayout> {
    // Check bounds only in debug builds
    debug_assert!(component_id < MAX_COMPONENTS, 
        "Component ID {} is out of bounds", component_id);
    
    // In release builds, out-of-bounds access is caller's responsibility
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    
    // Check if the layout has been initialized
    if REGISTRY.initialized[component_id].load(Ordering::Acquire) {
        // Safe to return a reference since the data is static
        Some(&REGISTRY.layouts[component_id])
    } else {
        None
    }
}

/// Optimized function to get the size of a component by ID
#[inline]
pub fn get_component_size(component_id: usize) -> Option<usize> {
    let layout = get_layout(component_id)?;
    Some(layout.size)
}

/// Optimized function to get the alignment of a component by ID
#[inline]
pub fn get_component_alignment(component_id: usize) -> Option<usize> {
    let layout = get_layout(component_id)?;
    Some(layout.alignment)
}

/// Creates a memory layout for a component without going through ComponentLayout
/// This is the most efficient way to get a layout when the component ID is known
#[inline]
pub fn get_component_memory_layout(component_id: usize) -> Option<Layout> {
    let layout = get_layout(component_id)?;
    Some(unsafe { Layout::from_size_align_unchecked(layout.size, layout.alignment) })
}

/// Ultra-fast access to component size when you're confident the component exists
/// Will cause undefined behavior if component_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_component_size_unchecked(component_id: usize) -> usize {
    debug_assert!(component_id < MAX_COMPONENTS && 
        REGISTRY.initialized[component_id].load(Ordering::Relaxed),
        "Component ID {} is invalid or not initialized", component_id);
    
    REGISTRY.layouts[component_id].size
}

/// Ultra-fast access to component alignment when you're confident the component exists
/// Will cause undefined behavior if component_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_component_alignment_unchecked(component_id: usize) -> usize {
    debug_assert!(component_id < MAX_COMPONENTS && 
        REGISTRY.initialized[component_id].load(Ordering::Relaxed),
        "Component ID {} is invalid or not initialized", component_id);
    
    REGISTRY.layouts[component_id].alignment
}


/// Ultra-fast access to component layout when you're confident the component exists
/// Will cause undefined behavior if component_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_layout_unchecked(component_id: usize) -> &'static ComponentLayout {
    debug_assert!(component_id < MAX_COMPONENTS && 
        REGISTRY.initialized[component_id].load(Ordering::Relaxed),
        "Component ID {} is invalid or not initialized", component_id);
    
    &REGISTRY.layouts[component_id]
}

/// Ultra-fast access to component memory layout when you're confident the component exists
/// Will cause undefined behavior if component_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_component_memory_layout_unchecked(component_id: usize) -> Layout {
    debug_assert!(component_id < MAX_COMPONENTS && 
        REGISTRY.initialized[component_id].load(Ordering::Relaxed),
        "Component ID {} is invalid or not initialized", component_id);
   unsafe {
    Layout::from_size_align_unchecked(
        REGISTRY.layouts[component_id].size,
        REGISTRY.layouts[component_id].alignment
    )
   }
}

/// Ultra-fast access to component type ID when you're confident the component exists
/// Will cause undefined behavior if component_id is invalid - use with caution!
#[inline(always)]
pub unsafe fn get_component_type_id_unchecked(component_id: usize) -> TypeId {
    debug_assert!(component_id < MAX_COMPONENTS && 
        REGISTRY.initialized[component_id].load(Ordering::Relaxed),
        "Component ID {} is invalid or not initialized", component_id);
    
    REGISTRY.layouts[component_id].type_id
}