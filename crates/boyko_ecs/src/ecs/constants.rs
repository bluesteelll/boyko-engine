/// Default virtual-address reservation for the component-data arena
/// (Phase X.F): 4 GiB on 64-bit OS-syscall arms. Virtual address space is
/// effectively free (Windows user VA is 128 TB; no commit charge is paid
/// until slabs are committed at the frontier), so the reserve is sized for
/// headroom — ~64x the pre-X.F 64 MB ceiling.
///
/// Note for tooling: large `PROT_NONE`/`PAGE_NOACCESS` reservations show up
/// in ASan/valgrind-class tooling as *address space*, not memory.
/// Memory-constrained embedders should use
/// `EcsMaster::with_arena_reserve` to pick a smaller ceiling.
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const DEFAULT_ARENA_RESERVE: usize = 4 * 1024 * 1024 * 1024;

/// Default arena reservation on the fallback arm (Miri / wasm32 / 32-bit /
/// exotic targets): 64 MiB. The fallback backing eagerly allocates the full
/// reserve from the global allocator (no reserve/commit split exists there),
/// so a multi-GB default would be fatal — the pre-X.F default is kept.
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const DEFAULT_ARENA_RESERVE: usize = 64 * 1024 * 1024;

/// Arena commit granularity (Phase X.F): 64 KiB — the Windows reservation
/// granularity, and a multiple of the 4 KiB commit/`mprotect` page size
/// everywhere. The reservation length itself is rounded up to this
/// (`os_reserve = align_up(reserve, ARENA_COMMIT_GRANULE)`) so a frontier
/// commit can never overrun the kernel's page-rounded mapping.
pub const ARENA_COMMIT_GRANULE: usize = 64 * 1024;

/// Minimum arena commit slab (Phase X.F): 2 MiB — one slab covers a default
/// ~3 MB component pool in <= 2 growth events. 2 MiB is also the Linux THP
/// size (the base alignment is page-only, so any THP benefit is
/// opportunistic — a documented non-goal).
pub const ARENA_MIN_SLAB: usize = 2 * 1024 * 1024;

/// Maximum arena commit slab (Phase X.F): 64 MiB — commit-charge overshoot
/// never exceeds the size of the entire pre-X.F arena. Filling a 4 GiB
/// reserve takes ~70 events lifetime, so the bound optimizes for overshoot
/// honesty, not event count. A single request larger than this is NOT
/// clamped (one request = one event; see `Arena::grow_then_retry`).
pub const ARENA_MAX_SLAB: usize = 64 * 1024 * 1024;

/// Typical CPU cache line size in bytes
/// Used for memory alignment to optimize cache usage
pub const CACHE_LINE_SIZE: usize = 64;

/// Minimum alignment for components (8 bytes)
/// Ensures components have at least this alignment even if their actual type
/// requires less alignment
pub const MIN_ALIGNMENT: usize = 8;

/// Minimum alignment for `ComponentPool` backing buffers.
///
/// Lifted from `align_of::<T>()` to ensure column-start addresses are AVX2-loadable
/// without an unaligned-prologue. Required by `Query::for_each_chunk` SIMD-amenable
/// inner loops (Phase X.A). 32 = AVX2 baseline; AVX-512 (64-byte) is opt-in via a
/// future `SIMD_BUFFER_ALIGN_AVX512` cfg-gated constant if needed.
///
/// See `docs/PHASE-X.A-PLAN.md` §6 for the design rationale and cost analysis.
pub const SIMD_BUFFER_ALIGN: usize = 32;

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

/// Default virtual-address reservation for the entity-metadata store
/// (`InlandStore`, Phase X.G): 1 GiB = 67,108,864 16-byte `EntityInland`
/// slots on the 64-bit OS-syscall arms — aligned with the 4 GiB component
/// arena ceiling (the arena exhausts long before 67 M real entities).
/// Reservation is address space only (no commit charge, no resident pages);
/// see `DEFAULT_ARENA_RESERVE` for the tooling note.
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const DEFAULT_INLAND_RESERVE: usize = 1024 * 1024 * 1024;
/// Fallback default (Miri / wasm32 / exotic / 32-bit): the reservation is
/// eagerly allocated ZEROED from the global allocator, so it must stay small.
/// 16 MiB = 1,048,576 entity slots — no Miri/wasm workload approaches that.
/// Native wasm32 cost: one eager zeroed 16 MiB per world, accepted because
/// the shipping wasm demo creates exactly one world (Phase X.G R2-W3).
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const DEFAULT_INLAND_RESERVE: usize = 16 * 1024 * 1024;

/// Smallest commit slab for the entity-metadata store (Phase X.G D2):
/// 256 KiB = 16,384 slots — covers the 9192-slot first request (1000 +
/// MAX_BATCH_HINT) in a single event. Granule (`ARENA_COMMIT_GRANULE`)
/// multiple.
pub const INLAND_MIN_SLAB: usize = 256 * 1024;

/// Largest geometric commit step for the entity-metadata store (Phase X.G
/// D2): 16 MiB = 1,048,576 slots — one max-step covers a 1 M-entity world;
/// bounds commit-charge overshoot by one slab.
pub const INLAND_MAX_SLAB: usize = 16 * 1024 * 1024;

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

//
// Event dispatch configuration
//

/// Maximum number of worker threads that can send events concurrently.
/// Controls the number of per-type writer lanes in `EventBuffer<E>`.
pub const MAX_EVENT_THREADS: u32 = 64;

/// Maximum events per lane per frame in `EventBuffer<E>`.
/// Bounds the per-lane write buffer allocation at preregister time.
pub const MAX_EVENT_CAPACITY: u32 = 16384;