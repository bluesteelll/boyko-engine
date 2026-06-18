//! Domain-specific error type for `boyko-ecs` operations.
//!
//! All fallible public APIs return `Result<T, EcsError>`. Consumers can
//! pattern-match on variants to handle individual failure modes. The library
//! deliberately does NOT depend on `anyhow` (application-level error
//! crate) — see audit C-019.

use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};

/// Failures that can arise from `boyko-ecs` operations.
///
/// New variants may be added in minor versions; consumers should treat the
/// enum as `#[non_exhaustive]` (literally marked below).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EcsError {
    /// `ArchetypeId` does not exist in the archetype master.
    ArchetypeNotFound(ArchetypeId),

    /// `EntityId` is not registered (or was deleted) — generation may have advanced.
    EntityNotFound(EntityId),

    /// Component pool for `ComponentId` is full and cannot accept new entries.
    ComponentPoolFull { component_id: ComponentId },

    /// One of the requested components is not known to the archetype.
    UnknownComponentForArchetype {
        archetype_id: ArchetypeId,
        component_id: ComponentId,
    },

    /// Generic "archetype rejected the entity push" — for cases not covered by
    /// the more specific variants above.
    ArchetypeRejectedEntity { archetype_id: ArchetypeId },

    /// A swap-remove operation failed on a component pool within a bundle.
    PoolSwapRemoveFailed,

    /// An event buffer lane is full; the send was rejected.
    EventBufferFull {
        /// Name of the event type (for diagnostics).
        type_name: &'static str,
        /// Index of the thread lane that overflowed.
        thread_index: u32,
        /// Number of events the call tried to write.
        attempted: u32,
        /// Number of events rejected (equals `attempted` for all-or-nothing sends).
        dropped: u32,
    },

    /// `send` or `events` called for an event type that was never preregistered.
    EventNotRegistered {
        /// Name of the event type (for diagnostics).
        type_name: &'static str,
    },

    /// `preregister` called twice for the same event type on the same dispatcher.
    EventAlreadyRegistered {
        /// Name of the event type (for diagnostics).
        type_name: &'static str,
    },

    /// An `EventConfig` value failed validation.
    InvalidEventConfig {
        /// Human-readable description of the failing constraint.
        reason: &'static str,
    },

    /// Phase 12.5 Opt-A2 (SBO17): a `spawn_batch` call exceeded the
    /// hard-coded per-call cap (`MAX_BATCH_HINT = 8 192`).
    ///
    /// The counter was NOT advanced — the caller can retry after chunking
    /// the batch into pieces ≤ `MAX_BATCH_HINT`.
    SpawnBatchExceedsCapacity {
        /// The `n` that was requested.
        requested: usize,
        /// The hard cap (`MAX_BATCH_HINT`).
        max: usize,
    },

    /// Phase 12.5 Opt-A2 (SBO4) / Phase X.I: an archetype's pool reserve
    /// ceiling (rows) was insufficient to accept `requested` more rows.
    ///
    /// Returned by `Archetype::reserve_capacity` when at least one owned
    /// pool's `count + requested` exceeds its reserve ceiling
    /// (`reserve_rows`). Committed capacity BELOW the ceiling grows on
    /// demand (Phase X.I `grow_rows`) and never produces this error — it
    /// fires only when the archetype outgrows the pool's whole
    /// reservation. The archetype state is unchanged — callers may either
    /// reduce the batch size or chunk across multiple `apply` calls.
    ArchetypePoolCapacityExceeded {
        /// Identifier of the archetype that rejected the reserve.
        archetype_id: ArchetypeId,
        /// The pool's reserve ceiling in rows (`reserve_rows`; the field
        /// name predates Phase X.I and is kept for compatibility).
        pool_capacity: usize,
        /// Number of rows the caller asked to reserve.
        requested: usize,
    },

    /// Phase 12.5 Opt-A2 (SBO17b / I-N1 / W2): a `spawn_batch` aggregate
    /// would advance `EntityMaster::next_entity_id` past the world's
    /// pre-sized entity-fast-store (`MAX_ENTITIES_HINT + MAX_BATCH_HINT`).
    ///
    /// On the direct path (`EcsMaster::spawn_batch`) this is detected via
    /// a Relaxed pre-load and returned as `Err` — the counter is NOT
    /// advanced. On the queued path (`SpawnBatchCommand::apply`) the same
    /// condition manifests as a hard panic at apply time (workers cannot
    /// pre-check because they race each other).
    WorldEntityCapacityExceeded {
        /// The would-be `end_id` (= `start + n`).
        end_id: usize,
        /// The world's current pre-sized fast-store length.
        capacity: usize,
    },
}

impl std::fmt::Display for EcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EcsError::ArchetypeNotFound(id) => {
                write!(f, "archetype {} not found", id)
            }
            EcsError::EntityNotFound(id) => {
                write!(f, "entity {} not found or stale", id)
            }
            EcsError::ComponentPoolFull { component_id } => {
                write!(f, "component pool for id {} is full", component_id)
            }
            EcsError::UnknownComponentForArchetype {
                archetype_id,
                component_id,
            } => {
                write!(
                    f,
                    "archetype {} does not contain component {}",
                    archetype_id, component_id
                )
            }
            EcsError::ArchetypeRejectedEntity { archetype_id } => {
                write!(f, "archetype {} rejected entity push", archetype_id)
            }
            EcsError::PoolSwapRemoveFailed => {
                write!(f, "component pool bundle swap_remove failed")
            }
            EcsError::EventBufferFull { type_name, thread_index, attempted, dropped } => {
                write!(
                    f,
                    "event buffer full for '{}' on thread {}: attempted {}, dropped {}",
                    type_name, thread_index, attempted, dropped
                )
            }
            EcsError::EventNotRegistered { type_name } => {
                write!(f, "event type '{}' is not registered; call preregister_event first", type_name)
            }
            EcsError::EventAlreadyRegistered { type_name } => {
                write!(f, "event type '{}' is already registered on this dispatcher", type_name)
            }
            EcsError::InvalidEventConfig { reason } => {
                write!(f, "invalid EventConfig: {}", reason)
            }
            EcsError::SpawnBatchExceedsCapacity { requested, max } => {
                write!(
                    f,
                    "spawn_batch requested {} entities; per-call cap is {} \
                     (chunk by the caller)",
                    requested, max
                )
            }
            EcsError::ArchetypePoolCapacityExceeded {
                archetype_id,
                pool_capacity,
                requested,
            } => {
                write!(
                    f,
                    "archetype {}: pool reserve ceiling ({} rows) cannot accept {} more rows",
                    archetype_id, pool_capacity, requested
                )
            }
            EcsError::WorldEntityCapacityExceeded { end_id, capacity } => {
                write!(
                    f,
                    "spawn_batch aggregate overshoot: end_id {} exceeds \
                     pre-sized capacity {} (SBO16+SBO17b); reduce concurrent \
                     workers or chunk further",
                    end_id, capacity
                )
            }
        }
    }
}

impl std::error::Error for EcsError {}

/// Convenience alias mirroring `anyhow::Result` but with the domain error.
pub type EcsResult<T> = Result<T, EcsError>;
