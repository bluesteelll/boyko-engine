//! The sinks — where a drained record actually goes.
//!
//! # State of this module
//!
//! **Rung L5.** One sink exists: [`ecs`], the transport that carries formatted lines from whoever
//! holds the drain token to the ECS. The console route is not here — it predates this module and
//! lives in [`crate::sync_out`], because it is the *synchronous* channel rather than a sink, and
//! moving it would put one destination behind two mechanisms.
//!
//! What does not exist yet: `console`, `file`, `binary`, `callback`, `crash`, `request`. Each
//! arrives with its own rung (L4, L13b, L14, L15), and each is absent rather than stubbed — a
//! `SinkKind` variant that returns `Ok(())` is indistinguishable from a working sink at every
//! call site, which is the failure mode this crate's gates exist to make impossible.

pub mod ecs;
pub mod binary;
pub mod file;
pub mod request;
