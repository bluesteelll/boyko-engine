// crates/boyko_ecs/src/ecs/constants.rs

/// Default arena size in bytes (64MB)
pub const DEFAULT_ARENA_SIZE: usize = 64 * 1024 * 1024;

/// Typical CPU cache line size in bytes
pub const CACHE_LINE_SIZE: usize = 64;

/// Default number of components per chunk
/// This value is a balance between memory efficiency and performance
/// Smaller values offer better memory utilization but worse cache performance
/// Larger values provide better cache locality but may waste memory
pub const DEFAULT_COMPONENTS_PER_CHUNK: usize = 1024;

/// Default chunk size for tiny components (<16 bytes)
pub const TINY_COMPONENTS_PER_CHUNK: usize = 2048;

/// Default chunk size for small components (16-64 bytes)
pub const SMALL_COMPONENTS_PER_CHUNK: usize = 1024;

/// Default chunk size for medium components (65-256 bytes)
pub const MEDIUM_COMPONENTS_PER_CHUNK: usize = 512;

/// Default chunk size for large components (>256 bytes)
pub const LARGE_COMPONENTS_PER_CHUNK: usize = 256;

/// Default number of chunks per component pool
/// INCREASED from original 32 to 128 for better scalability
pub const DEFAULT_CHUNKS_PER_POOL: usize = 128;

/// Minimum alignment for components (8 bytes)
pub const MIN_ALIGNMENT: usize = 8;

/// Initial entity capacity for archetypes
pub const INITIAL_ENTITY_CAPACITY: usize = 1024;

/// Size threshold for different component size categories (in bytes)
pub const TINY_COMPONENT_THRESHOLD: usize = 16;
pub const SMALL_COMPONENT_THRESHOLD: usize = 64;
pub const MEDIUM_COMPONENT_THRESHOLD: usize = 256;

/// Threshold for chunk compaction (as a percentage of fragmentation)
/// When fragmentation exceeds this ratio, compaction will be triggered
pub const COMPACTION_THRESHOLD: f32 = 0.25; // 25% fragmentation

/// Minimum number of components for triggering auto-compaction
pub const MIN_COMPONENTS_FOR_COMPACTION: usize = 16;

/// When dynamic expansion is enabled, grow by this factor
pub const POOL_GROWTH_FACTOR: f32 = 1.5; // Grow by 50%

/// Maximum potential expansion factor for pool size
/// Limits how much a pool can grow beyond its initial size
pub const MAX_POOL_EXPANSION_FACTOR: usize = 8; // Can grow to 8x initial size