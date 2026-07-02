//! `framegraph` — the in-house Render Dependency Graph (RDG) for the on-screen
//! G-buffer frame (industrial frame-system, Pillar A).
//!
//! Passes declare which resources they read/write, at which pipeline stage +
//! access + layout; [`FrameGraph::compile`] runs a Granite-style per-resource
//! synchronization state machine that **auto-derives** the minimal Vulkan
//! `vkCmdPipelineBarrier` set — replacing the ~18–30 hand-authored, hand-batched
//! barriers in `swapchain::record_gbuffer`. See
//! `docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md` for the full plan + the de-risked
//! 1a→1f sequence.
//!
//! # Where this lives, and why (architecture decisions — plan divergences)
//!
//! - **Crate = `boyko_rhi_vulkan`, not `boyko_render`.** The barriers reproduce
//!   `record_gbuffer`'s exact `Vk*` stages/access/layouts and are verified by a
//!   CPU diff against it in this crate. A backend-agnostic `boyko_render` layer
//!   would need the full graphics-pipeline stage/layout surface added to
//!   `boyko_rhi`'s buffer-only `BarrierStage`/`BarrierAccess`, then re-lowered —
//!   a speculative abstraction for a single (Vulkan) backend, against Principle 0.
//!   The graph is still ECS-native at the frame seam (Step 1c wires it as a
//!   `NonSend`-style resource reached via the dispatcher token, like `RhiContext`).
//! - **Substrate = build-time preallocated `Vec`s** (the `boyko_render::barrier`
//!   precedent), not `VmReservation` (which is `pub(crate)` in `boyko_ecs`).
//!   Zero per-frame allocation via `reset` + re-declare. Resolves critic C4.
//! - **Gate = sound-superset + hazard coverage**, not byte-identical: the machine
//!   places each barrier at true first-use, whereas the hand path batches some
//!   transitions eagerly — so the streams differ positionally but enforce the
//!   same dependencies (plan open-decision #1 / C5). The equivalence tests assert
//!   layout trajectories + producer→consumer coverage + minimality (`count ≤`
//!   hand), on the moving/maximal-permutation scene.
//! - **Ordering = linear** (declaration order); DAG topo/SCC deferred until a
//!   branching frame exists (see [`graph`]).
//!
//! # Step status
//!
//! Step 1b (here): the module + the CPU capture + the equivalence diff — the
//! graph does NOT drive the GPU yet (zero deletion risk). Steps 1c–1f flip it to
//! drive subsets, add history-rotation + FIF slotting, layered subresource
//! barriers, and finally delete `record_gbuffer` + the ring.

pub mod graph;
pub mod ids;
pub mod record;
pub mod sync;

pub use graph::{FrameGraph, PassBarrierRange};
pub use ids::{PassId, ResId};
pub use record::{BarrierSink, MAX_PASS_BARRIERS};
pub use sync::{BufBarrier, ImgBarrier, ResSync, SubRange, Trans, WRITE_ACCESS_MASK};
