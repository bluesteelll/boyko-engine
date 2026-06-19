//! The action-map layer (plan §6): typed actions, the binding map, the SoA
//! action state, and the per-frame aggregation.
//!
//! # I4 seams
//! `clash.rs` is crate-internal (used by `process`). The `Resource` impls for
//! `ActionState<A>`/`InputMap<A>` (via the TypeId registry, plan C1), contexts
//! (plan §6 V3), the `update_action_state` system, and the fixed-step snapshot
//! (plan §7.3) land in I4. Persistence (`.keys`, plan §9) and the rebind state
//! machine (plan §9.4) land in I5.

pub mod actionlike;
pub mod map;
pub mod process;
pub mod state;

pub(crate) mod clash;
