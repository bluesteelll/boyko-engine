// Module name mirrors the public `Archetype` type; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod archetype;
pub mod archetype_signature;
pub mod archetype_registry;
pub mod archetype_bundle;
pub mod archetype_master;
pub mod generation;

pub use generation::ArchetypeGeneration;