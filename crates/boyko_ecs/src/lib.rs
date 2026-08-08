pub mod ecs;
pub mod prelude;

pub use ecs::core::app::{App, AppExit, Plugin, Plugins};
pub use ecs::error::{EcsError, EcsResult};
// This crate declares profiling zones (the frame driver's four, plus the fold's own), so it must
// name its lane region -- the same one line every engine crate writes. `declare_zone!` reads
// `crate::__BOYKO_ZONE_PARTITION` from the DECLARING crate's root, which is what keeps a game's
// samples out of the engine's region; a crate that omits this line fails to compile at its first
// zone, which is the intended outcome rather than a silent default.
boyko_diag::profiling_partition!(Engine);
