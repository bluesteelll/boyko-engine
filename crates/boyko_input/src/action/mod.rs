//! The action-map layer (plan §6): typed actions, the binding map, the SoA
//! action state, and the per-frame aggregation.
//!
//! # I4 (this round)
//! `resource_id` mints a distinct `ResourceId` per concrete `A` for the generic
//! `ActionState<A>` / `InputMap<A>` resources (plan §7.1, C1); the `Resource`
//! impls live next to each type; the `update_action_state` ingest system and the
//! frame-stable fixed snapshot (plan §7.3) land in `process` / `state`.
//!
//! # I5 (this round)
//! `rebind` adds the [`RebindSession`](rebind::RebindSession) state machine
//! (plan §9.4); the `.keys` persistence lives in the sibling
//! [`persist`](crate::persist) module (plan §9).
//!
//! # Remaining seams
//! `clash.rs` is crate-internal (used by `process`). Contexts / the priority
//! stack (plan §6 V3) land in a later round.

pub mod actionlike;
pub mod map;
pub mod process;
pub mod rebind;
pub mod resource_id;
pub mod state;

pub(crate) mod clash;
