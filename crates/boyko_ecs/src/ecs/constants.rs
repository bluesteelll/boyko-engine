/// Default arena size in bytes (64MB)
/// This controls the total amount of memory initially allocated for the ECS system
pub const DEFAULT_ARENA_SIZE: usize = 64 * 1024 * 1024;

/// Typical CPU cache line size in bytes
/// Used for memory alignment to optimize cache usage
pub const CACHE_LINE_SIZE: usize = 64;

/// Minimum alignment for components (8 bytes)
/// Ensures components have at least this alignment even if their actual type
/// requires less alignment
pub const MIN_ALIGNMENT: usize = 8;

//
// Chunk configuration
//

/// Default number of components per chunk
/// This value is a balance between memory efficiency and performance
/// Smaller values offer better memory utilization but worse cache performance
/// Larger values provide better cache locality but may waste memory
pub const DEFAULT_COMPONENTS_PER_CHUNK: usize = 1024;

/// Default number of chunks per component pool
/// This controls initial capacity of the pool vector
pub const DEFAULT_CHUNKS_PER_POOL: usize = 128;

//
// Component size categories
//

/// Default chunk size for tiny components (<16 bytes)
/// Tiny components can be densely packed for better cache utilization
pub const TINY_COMPONENTS_PER_CHUNK: usize = 2048;

/// Default chunk size for small components (16-64 bytes)
/// Small components still benefit from dense packing
pub const SMALL_COMPONENTS_PER_CHUNK: usize = 1024;

/// Default chunk size for medium components (65-256 bytes)
/// Medium-sized components require more memory but can still be efficiently cached
pub const MEDIUM_COMPONENTS_PER_CHUNK: usize = 512;

/// Default chunk size for large components (>256 bytes)
/// Large components are stored in smaller chunks to avoid excessive memory waste
pub const LARGE_COMPONENTS_PER_CHUNK: usize = 256;

/// Size threshold for different component size categories (in bytes)
pub const TINY_COMPONENT_THRESHOLD: usize = 16;
pub const SMALL_COMPONENT_THRESHOLD: usize = 64;
pub const MEDIUM_COMPONENT_THRESHOLD: usize = 256;

//
// Archetype and entity configuration
//

/// Initial entity capacity for archetypes
/// Controls how many entities an archetype can initially store
pub const INITIAL_ENTITY_CAPACITY: usize = 1024;

/// When dynamic expansion is enabled, grow by this factor
/// This is used when containers need to be resized
pub const GROWTH_FACTOR: f32 = 1.5; // Grow by 50%

/// Maximum potential expansion factor for pool size
/// Limits how much a pool can grow beyond its initial size
pub const MAX_EXPANSION_FACTOR: usize = 8; // Can grow to 8x initial size

//
// Memory management
//

/// Threshold for chunk compaction (as a percentage of fragmentation)
/// When fragmentation exceeds this ratio, compaction will be triggered
pub const COMPACTION_THRESHOLD: f32 = 0.25; // 25% fragmentation

/// Minimum number of components for triggering auto-compaction
/// Prevents unnecessary compaction for small component collections
pub const MIN_COMPONENTS_FOR_COMPACTION: usize = 16;

/// Default initial capacity for free slots tracking
/// Controls the initial size of vectors used to track free component slots
pub const INITIAL_FREE_SLOTS_CAPACITY: usize = 1024;

/// Maximum percentage of empty chunks before pool reorganization is triggered
pub const MAX_EMPTY_CHUNKS_RATIO: f32 = 0.2; // 20% empty chunks