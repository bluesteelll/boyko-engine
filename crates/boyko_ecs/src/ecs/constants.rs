// crates/boyko_ecs/src/ecs/constants.rs

/// Default arena size in bytes (64MB)
pub const DEFAULT_ARENA_SIZE: usize = 64 * 1024 * 1024;

/// Typical CPU cache line size in bytes
pub const CACHE_LINE_SIZE: usize = 64;

/// Default number of components per chunk
pub const DEFAULT_COMPONENTS_PER_CHUNK: usize = 1024;

/// Default number of chunks per component pool
pub const DEFAULT_CHUNKS_PER_POOL: usize = 32;

/// Minimum alignment for components (8 bytes)
pub const MIN_ALIGNMENT: usize = 8;

/// Initial entity capacity for archetypes
pub const INITIAL_ENTITY_CAPACITY: usize = 1024;