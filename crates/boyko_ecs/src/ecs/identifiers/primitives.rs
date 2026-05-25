//! Strongly-typed identifier wrappers for the ECS.
//!
//! All IDs are `#[repr(transparent)]` newtypes around `usize` — zero
//! runtime cost, identical layout — but type-distinct so the compiler
//! refuses to confuse e.g. `EntityId` with `ComponentId`. Audit C-017.

use std::fmt;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub usize);

        impl $name {
            /// Constructs from a raw `usize`. Same as `$name(raw)` — provided
            /// for explicit-conversion call sites where the tuple form is
            /// less readable.
            #[inline]
            pub const fn new(raw: usize) -> Self {
                Self(raw)
            }

            /// Unwraps to the underlying `usize`.
            #[inline]
            pub const fn get(self) -> usize {
                self.0
            }
        }

        impl From<usize> for $name {
            #[inline]
            fn from(raw: usize) -> Self {
                Self(raw)
            }
        }

        impl From<$name> for usize {
            #[inline]
            fn from(id: $name) -> usize {
                id.0
            }
        }

        impl fmt::Display for $name {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

define_id!(/// Public entity identifier.
    EntityId);
define_id!(/// Public archetype identifier.
    ArchetypeId);
define_id!(/// Chunk identifier (top-level, within an archetype's pool bundle).
    ChunkId);
define_id!(/// Internal chunk index inside one ComponentPool.
    InlandChunkId);
define_id!(/// Component type identifier (assigned at registration).
    ComponentId);
define_id!(/// Resource type identifier (assigned at registration in the
    /// global resource registry; distinct from `ComponentId` because the
    /// same Rust type cannot be registered as both — see M6).
    ResourceId);
define_id!(/// Internal unit index inside one ComponentPool.
    InlandUnitId);
define_id!(/// Internal pool index inside one ComponentPoolBundle.
    InlandPoolId);
define_id!(/// Internal component index inside one Archetype.
    InlandComponentId);
define_id!(/// Internal archetype index inside one ArchetypeBundle.
    InlandArchetypeId);

/// Generation counter for entity-recycling safety. Kept as a plain alias
/// to `usize` — generations have no type-distinct callers (only used
/// inside `Entity` and `EntityInland`, never crossed with another ID).
pub type Generation = usize;
