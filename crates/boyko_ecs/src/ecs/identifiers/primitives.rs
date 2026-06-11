//! Strongly-typed identifier wrappers for the ECS.
//!
//! All IDs are `#[repr(transparent)]` newtypes around `usize` — zero
//! runtime cost, identical layout — but type-distinct so the compiler
//! refuses to confuse e.g. `EntityId` with `ComponentId`. Audit C-017.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Process-global monotonic counter backing [`WorldId::mint`]. Metadata-class
/// global (like the component/event registries): it stores no world-derived
/// state, only hands out unique numbers. `u64` cannot realistically wrap.
static NEXT_WORLD_ID: AtomicU64 = AtomicU64::new(0);

/// Process-unique world identifier (Phase 21).
///
/// Minted once per [`EcsMaster`] construction (`new` / `with_capacity`) from a
/// process-global atomic counter; never reused within a process, even after
/// the world is dropped. Its purpose is the world-binding gate on
/// [`Schedule::run`]: a `Schedule` records the id of the world it was built on
/// and release-panics when handed a different world (Bevy
/// `Schedule::run` parity), closing the cross-world UB surface of cached
/// per-world pointers (event-buffer `NonNull`s, `QueryState` generations).
///
/// The inner value is private: `WorldId`s are only minted by the engine, so
/// two equal ids always mean "the same world" (no forgeable constructor).
///
/// [`EcsMaster`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster
/// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldId(u64);

impl WorldId {
    /// Mints the next process-unique world id.
    ///
    /// Relaxed: only uniqueness matters (a single `fetch_add` counter); no
    /// payload is published through this atomic.
    #[inline]
    pub(crate) fn mint() -> Self {
        Self(NEXT_WORLD_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Unwraps to the underlying `u64` (diagnostics only — the value carries
    /// no meaning beyond process-wide uniqueness).
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WorldId {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WorldId({})", self.0)
    }
}
