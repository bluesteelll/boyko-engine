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
        }
    }
}

impl std::error::Error for EcsError {}

/// Convenience alias mirroring `anyhow::Result` but with the domain error.
pub type EcsResult<T> = Result<T, EcsError>;
