//! The UI upload system (`UiUploadSystem`) — GUI P5a Rung 4.
//!
//! A `GpuSystem`-shaped consumer (EMPTY [`Access`], [`is_gpu`](System::is_gpu) →
//! `SystemKind::GpuCompute`, dispatcher-solo) that, on a UI-change frame:
//!
//! 1. **O(1) generation gate** — short-circuits on `gen == last_seen_generation`
//!    (the [`UiRenderGeneration`] resource, A1 step 1); a static frame does nothing.
//! 2. **packs** every visible node `(ComputedRect, UiBackground, ComputedClip?,
//!    StackIndex?)` into the reused [`UiRenderScratch`] (`clear()` + `extend`,
//!    never `Vec::new`),
//! 3. **stable z-sorts** by `(StackIndex, append_order)` in place (zero alloc),
//! 4. **uploads** the packed scratch into the current-FIF host-mapped ring via
//!    [`RhiContext::ui_upload`], and
//! 5. **stashes** the POD-by-value [`UiFramePlan`] the swapchain recorder reads
//!    (Decision 9: it borrows NO RHI handle, so nothing `!Send` crosses the token
//!    drop; the recorder re-resolves the pipeline + bind-group by `frame_index`).
//!
//! # The world-access seam (Rung-4 integration boundary)
//!
//! The pure pack→sort→upload pipeline is the world-AGNOSTIC core
//! [`UiUploadSystem::pack_sort_upload`], which takes the per-node inputs as an
//! iterator plus the scratch, the ortho, the current `frame_index`, and an
//! `&mut RhiContext`. It is fully unit-/Miri-testable (no Arena / world) and is what
//! the host render loop and the goldens drive directly.
//!
//! The shipped end-to-end ON-SCREEN path is the render host calling
//! [`host_upload_frame`](UiUploadSystem::host_upload_frame) (read the swapchain slot →
//! fence it → [`pack_sort_upload`](UiUploadSystem::pack_sort_upload)), which returns
//! the plan AND the minted [`FrameWriteToken`]; then
//! [`RhiContext::ui_pass`](crate::RhiContext::ui_pass) on the plan, then
//! [`Renderer::present_sampled`](boyko_rhi_vulkan::swapchain::Renderer::present_sampled)
//! with the token (consumed BY VALUE — R0b) and `Some(&pass)` — the
//! `record_present_sampled` UI sub-pass records the one draw.
//!
//! The host-drivable
//! [`host_upload_frame_from_world`](UiUploadSystem::host_upload_frame_from_world)
//! seam (#31) runs the D6a per-slot generation gate FIRST — one `u64` compare
//! against `scratch.last_seen_generation[slot]`, AHEAD of the gather, so a
//! static frame costs zero component probes and zero repacks (UI-ADVANCED S0
//! item 5) — then, on a changed slot, gathers the visible nodes from a
//! [`DispatcherToken::world`] [`WorldView`] (a read-only ECS projection, #30),
//! ends that borrow, and delegates to `host_upload_frame` with only the `!Send`
//! borrows live. `WorldView` (#30) supplies the column/resource-read HALF of
//! the world access the in-schedule site needs. The canonical gather to wire in
//! is [`gather_ui_nodes`](crate::ui::gather::gather_ui_nodes).
//!
//! The `impl System` shell ([`System::run_dispatcher`]) is registered for its
//! scheduler SHAPE only (EMPTY access, `is_gpu()`, dispatcher-solo) and is an honest
//! no-op here: it does NOT project-and-drop the `!Send` [`RhiContext`]. The one
//! capability still missing for the in-schedule upload is the swapchain `Renderer`
//! slot index + in-flight fence — the `Renderer` is not yet an ECS resource — so the
//! host drives the path through `host_upload_frame_from_world` until an ECS-resident
//! swapchain handle exists (the remaining architectural decision for the
//! orchestrator).

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::dispatcher_token::{DispatcherToken, WorldView};
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use boyko_rhi_vulkan::swapchain::{FrameWriteToken, Renderer};

use crate::error::GpuColumnError;
use crate::gpu_column::RhiContext;
use crate::ui::instance::{UiInstance, UiOrtho};
use crate::ui::pack::{pack_ui_instance, PackInput, UiRenderScratch};
// `UiRenderGeneration` is referenced only in doc links across this module; importing
// it lets the bare `[`UiRenderGeneration`]` intra-doc links resolve.
#[allow(unused_imports)]
use crate::ui::pack::UiRenderGeneration;
use crate::ui::plan::UiFramePlan;

/// One source node's pack inputs plus its painter's-order z key — the world-agnostic
/// row the [`UiUploadSystem::pack_sort_upload`] core consumes (so the pack is driven
/// by a host-owned `Query` without this crate naming the query types).
#[derive(Clone, Copy, Debug)]
pub struct UiNode {
    /// The per-node pack input (logical-px component values + style + optional clip).
    pub input: PackInput,
    /// The node's `StackIndex` (0 if the node has none) — the painter's-order key.
    pub stack: u32,
}

/// The UI upload system (Rung 4): a `GpuSystem`-shaped `impl System` (EMPTY access,
/// `is_gpu()`, dispatcher-solo) carrying the SETUP-class state the upload needs.
///
/// The per-frame pack→sort→upload is the world-agnostic core
/// [`pack_sort_upload`](Self::pack_sort_upload); the `impl System` shell projects the
/// `!Send` [`RhiContext`] through the [`DispatcherToken`]. See the module docs for
/// the world-access seam.
pub struct UiUploadSystem {
    /// The logical→physical DPI scale folded into every length at pack (so the shader
    /// works in physical px and `fwidth` AA is one device pixel). The host updates it
    /// when the viewport scale factor changes (and bumps the generation).
    scale_factor: f32,
    /// Per-system metadata (name, EMPTY access, tick snapshots). The `Access` stays
    /// EMPTY — the GpuSystem-shaped consumer adds no conflict-graph edges (MF-5).
    meta: SystemMeta,
}

impl UiUploadSystem {
    /// Constructs the upload system with the initial logical→physical `scale_factor`.
    ///
    /// The system declares EMPTY [`Access`] (it touches no CPU column through the
    /// conflict graph) and is expected to be registered `SystemConfig::gpu()` so the
    /// scheduler resolves it to `SystemKind::GpuCompute` (dispatcher-solo), like
    /// [`GpuSystem`](crate::GpuSystem).
    pub fn new(scale_factor: f32) -> Self {
        debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
        // `Tick::new(1)` is the construction sentinel (the dispatcher overwrites the
        // snapshot before the first run); the system consumes no change ticks (empty
        // access — it gates on the explicit `UiRenderGeneration` counter instead).
        let meta = SystemMeta::new(std::any::type_name::<UiUploadSystem>(), Tick::new(1));
        Self {
            scale_factor,
            meta,
        }
    }

    /// Updates the logical→physical scale factor (the host calls this on a viewport
    /// DPI change, then bumps [`UiRenderGeneration`]).
    #[inline]
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
        self.scale_factor = scale_factor;
    }

    /// The world-AGNOSTIC pack → stable z-sort → upload core (A1 steps 2-6). Drives
    /// the whole per-frame UI build given the visible nodes (in any order), the reused
    /// `scratch`, the `ortho` for the swapchain extent, a BORROW of the
    /// [`FrameWriteToken`] write proof for the target slot (R0b: the upload is a
    /// mid-frame write — the caller keeps the token for the frame-ending
    /// `present_sampled` consume), and the `&mut RhiContext` that owns the rings.
    ///
    /// - **pack**: `scratch.pack.clear()` + `extend` one [`UiInstance`] per node
    ///   (preallocated; never `Vec::new`), folding `scale_factor` + premultiply +
    ///   `CLIP_PRESENT` / `BORDER_ANY` via [`pack_ui_instance`]; the `(stack, append)`
    ///   key lane is filled in iteration order.
    /// - **sort**: [`UiRenderScratch::sort_by_stack`] — an in-place, zero-alloc stable
    ///   sort by `(StackIndex, append_order)` (the append index makes the key a total
    ///   order, so an unstable sort is the stable permutation without a merge buffer).
    /// - **upload**: [`RhiContext::ui_upload`] memcpys the packed bytes into the
    ///   current-FIF mapped ring (growing pow2 + rebuilding that slot's bind-group on
    ///   overflow), returning the POD-by-value [`UiFramePlan`].
    ///
    /// `gather` is a reused scratch buffer for the sort gather (capacity-stable; pass
    /// the same `Vec` each frame so it never reallocates in steady state).
    ///
    /// Returns the [`UiFramePlan`] to stash for the swapchain recorder. An empty node
    /// set yields an empty plan (the recorder draws nothing).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a ring grow / mapping failure (or if
    /// [`RhiContext::ui_setup`](crate::RhiContext::ui_setup) was never called).
    pub fn pack_sort_upload<I: IntoIterator<Item = UiNode>>(
        &self,
        nodes: I,
        scratch: &mut UiRenderScratch,
        gather: &mut Vec<UiInstance>,
        ortho: UiOrtho,
        token: &FrameWriteToken,
        ctx: &mut RhiContext,
    ) -> Result<UiFramePlan, GpuColumnError> {
        // DIAGNOSTIC (S0 item 6): one repack executed. Counted HERE — at the
        // pack itself, not at the gate — so a wrongly-placed gate that still
        // gathers but skips the pack keeps this at zero while the probe counter
        // moves (the two counters split exactly on the M0-b mutation).
        scratch.repacks = scratch.repacks.wrapping_add(1);

        // (2) pack — clear + extend into the preallocated scratch, never Vec::new.
        scratch.pack.clear();
        scratch.keys.clear();
        for (append, node) in nodes.into_iter().enumerate() {
            scratch.pack.push(pack_ui_instance(&node.input, self.scale_factor));
            scratch.keys.push((node.stack, append as u32));
        }

        // (3) stable z-sort in place (zero alloc — the (stack, append) key is total).
        scratch.sort_by_stack(gather);

        // (4)+(5) upload the contiguous packed bytes into the current-FIF ring and
        // return the POD-by-value plan (no RHI handle escapes — Decision 9).
        let plan = ctx.ui_upload(&scratch.pack, ortho, token)?;
        scratch.last_count = plan.instance_count;
        Ok(plan)
    }

    /// The complete ON-SCREEN host driver (GUI P5a Rung 5) — the single call the
    /// render host makes each frame BEFORE [`Renderer::present_sampled`]. It ties the
    /// whole upload path together correctly, in order:
    ///
    /// 1. **fences the swapchain's NEXT frame-in-flight slot**
    ///    ([`Renderer::wait_frame_in_flight`]) so the GPU's last read of that
    ///    per-frame UI ring (the submit two presents back) is complete — closing the
    ///    write-after-read race on the persistently-mapped, host-coherent ring — and
    ///    minting the [`FrameWriteToken`] write proof for that slot, and
    /// 2. drives [`pack_sort_upload`](Self::pack_sort_upload) (pack → stable z-sort →
    ///    memcpy into that slot) with the token — the upload cannot name any other
    ///    slot.
    ///
    /// The host then calls [`RhiContext::ui_pass`](crate::RhiContext::ui_pass) on the
    /// returned plan to build the concrete
    /// [`UiPass`](boyko_rhi_vulkan::swapchain::UiPass) and passes it — together with
    /// the returned [`FrameWriteToken`], BY VALUE — to
    /// `present_sampled(token, ..., Some(&pass))`: the frame-ending submit consumes
    /// the token (R0b), so the write discipline does not fork between the G-buffer
    /// and UI-composite paths. Because the plan is POD (borrows no RHI handle) the
    /// host may hold it across the pass build; the pass re-resolves the pipeline +
    /// bind-group by `plan.frame_index` (MF-7).
    ///
    /// `ortho` MUST be [`UiOrtho::for_extent`](crate::UiOrtho::for_extent) of the
    /// swapchain extent this frame presents into (Decision 9). `gather` is the reused
    /// sort-gather scratch (pass the same `Vec` each frame so it never reallocates).
    ///
    /// # Errors
    /// [`GpuColumnError::Swapchain`] on the fence wait, or any
    /// [`pack_sort_upload`](Self::pack_sort_upload) upload error.
    pub fn host_upload_frame<I: IntoIterator<Item = UiNode>>(
        &self,
        nodes: I,
        scratch: &mut UiRenderScratch,
        gather: &mut Vec<UiInstance>,
        ortho: UiOrtho,
        renderer: &Renderer<'_>,
        ctx: &mut RhiContext,
    ) -> Result<(UiFramePlan, FrameWriteToken), GpuColumnError> {
        // (1) Fence the slot BEFORE the memcpy — the GPU finished reading this ring
        // slot two presents back; uploading before its in-flight fence signals would
        // race the GPU's read. The returned token is the write proof the upload
        // requires (and the only source of the slot index).
        let token = renderer.wait_frame_in_flight()?;
        // (2) pack → sort → upload into the now-free slot (a mid-frame BORROW of the
        // token); the token itself is returned for the frame-ending
        // `present_sampled` consume.
        let plan = self.pack_sort_upload(nodes, scratch, gather, ortho, &token, ctx)?;
        Ok((plan, token))
    }

    /// Host-drivable per-frame upload with the D6a generation gate HOISTED ahead
    /// of the gather (UI-ADVANCED S0 item 5): a static frame costs ONE `u64`
    /// compare and ZERO component probes — the gather closure is never entered,
    /// the pack never runs, and the slot's existing ring contents are re-served.
    ///
    /// Per frame, in order:
    ///
    /// 1. read the target ring slot ([`Renderer::frame_index`] — the slot the
    ///    frame-ending present will use) and the current
    ///    [`UiRenderGeneration`] from the world;
    /// 2. **the gate**: if the generation equals
    ///    `scratch.last_seen_generation[slot]`, mint the [`FrameWriteToken`]
    ///    (present still needs the fence proof) and return a plan re-serving
    ///    the slot's uploaded count with THIS frame's `ortho` — no gather, no
    ///    pack, no upload;
    /// 3. otherwise gather the visible nodes from the read-only [`WorldView`]
    ///    (`gather_nodes` fills `node_buf` using ONLY the view's `&self` read
    ///    surface — [`WorldView::resource`], [`WorldView::get_component_raw`],
    ///    [`WorldView::query_entities_buf`]), let that borrow end, then drive
    ///    [`host_upload_frame`](Self::host_upload_frame) and record the
    ///    generation + count for the slot.
    ///
    /// The gate is PER ring slot (`[u64; FRAMES_IN_FLIGHT]`): after one change,
    /// each slot repacks once (its ring holds stale bytes until its own repack)
    /// and only then skips — a single scalar would skip the second slot onto
    /// stale contents (rung S0 gate G0-3 / red mutation M0-a).
    ///
    /// The swapchain `Renderer` is still host-supplied (it is not yet an ECS
    /// resource), so this is a host driver, not the in-schedule site — see the
    /// module docs' world-access seam.
    ///
    /// > **UI-ADVANCED S0 status (2026-08-21): this seam currently has NO
    /// > possible caller** — a `WorldView` is mintable only inside a
    /// > `System: Send + Sync + 'static` body, where `&mut RhiContext` is
    /// > M1-exclusive with the view (E0502) and host locals are unreachable
    /// > (E0277/E0521). The gate below is therefore landed per the plan's item
    /// > 5 but UNGATED until the callability fork is resolved — see
    /// > `docs/OPEN-QUESTIONS.md`, entry 2026-08-21.
    ///
    /// Like [`host_upload_frame`](Self::host_upload_frame), returns the minted
    /// [`FrameWriteToken`] alongside the plan — the host passes it BY VALUE to the
    /// frame-ending `present_sampled` consume (R0b).
    ///
    /// # Panics
    /// If the world has no [`UiRenderGeneration`] resource. The gate refuses to
    /// guess: a host that never registered
    /// [`ui_render_discovery`](crate::ui::gather::ui_render_discovery) (and its
    /// resource) would otherwise silently repack every frame — the "gate that
    /// cannot fail" shape this project keeps recording.
    ///
    /// # Errors
    /// [`GpuColumnError::Swapchain`] on the fence wait, or any
    /// [`pack_sort_upload`](Self::pack_sort_upload) upload error.
    #[allow(clippy::too_many_arguments)]
    pub fn host_upload_frame_from_world<F>(
        &self,
        world: WorldView<'_>,
        node_buf: &mut Vec<UiNode>,
        gather_nodes: F,
        scratch: &mut UiRenderScratch,
        gather: &mut Vec<UiInstance>,
        ortho: UiOrtho,
        renderer: &Renderer<'_>,
        ctx: &mut RhiContext,
    ) -> Result<(UiFramePlan, FrameWriteToken), GpuColumnError>
    where
        F: FnOnce(WorldView<'_>, &mut Vec<UiNode>),
    {
        // (1) The slot this frame's present will fence + bind (round-robin; the
        // fence wait below and `present_sampled` both use it), and the current
        // generation — read BEFORE the gather so the gate can skip it (D6a).
        let slot = renderer.frame_index();
        debug_assert!(
            slot < crate::ui::FRAMES_IN_FLIGHT,
            "invariant: the swapchain frame index addresses a UI ring slot"
        );
        let generation = world.resource::<UiRenderGeneration>().generation;

        // (2) The per-slot gate: nothing changed since this SLOT last packed ⇒
        // its ring bytes are current — skip the gather AND the pack. The token
        // is still minted (the frame-ending present consumes it), and the plan
        // re-serves the slot's count under THIS frame's ortho (the ortho is
        // extent-derived per frame; the packed bytes do not depend on it).
        if generation == scratch.last_seen_generation[slot] {
            let token = renderer.wait_frame_in_flight()?;
            debug_assert_eq!(
                token.slot(),
                slot,
                "invariant: the fenced slot is the slot the gate compared"
            );
            return Ok((
                UiFramePlan {
                    instance_count: scratch.last_counts[slot],
                    ortho,
                    frame_index: slot,
                },
                token,
            ));
        }

        // (3) Changed for this slot: gather, then upload. The `world` view (the
        // read borrow) is consumed by the closure; only the `!Send`
        // `&mut RhiContext` + `&Renderer` borrows are live during the upload.
        node_buf.clear();
        gather_nodes(world, node_buf);
        let (plan, token) =
            self.host_upload_frame(node_buf.drain(..), scratch, gather, ortho, renderer, ctx)?;
        debug_assert_eq!(
            token.slot(),
            slot,
            "invariant: the uploaded slot is the slot the gate compared"
        );
        scratch.last_seen_generation[slot] = generation;
        scratch.last_counts[slot] = plan.instance_count;
        Ok((plan, token))
    }
}

// SAFETY (S1' + MF-5 / Option C): this system records NO CPU component access (the
//   declared `Access` is empty) and, in P5a, its `run_dispatcher` body is an honest
//   no-op that mints no reference (the on-screen upload is host-driven via
//   `host_upload_frame`). The S1' aliasing contract is therefore vacuously upheld: the
//   scheduler runs a `GpuCompute` system dispatcher-solo at `running == 0`, the token
//   is mintable only there, and the body neither projects the token nor touches world
//   state. `run_unsafe` is unreachable-by-design (a worker holds no token, so the
//   `!Send` `RhiContext` is structurally unreachable on the worker path).
unsafe impl System for UiUploadSystem {
    type Out = ();

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name()
    }

    /// EMPTY component/resource access — the upload touches no CPU column through the
    /// conflict graph (MF-5). It is the `GpuSystem`-shaped consumer shape.
    #[inline]
    fn access(&self) -> &Access {
        self.meta.access()
    }

    /// Phase 5 Option C — the GPU marker, so the schedule resolves this to
    /// `SystemKind::GpuCompute` (dispatcher-solo) even without the explicit
    /// `SystemConfig::gpu()` opt-in.
    #[inline]
    fn is_gpu(&self) -> bool {
        true
    }

    /// No two-phase init — the access surface is EMPTY by construction.
    fn initialize(&mut self, _world: &mut EcsMaster) {}

    /// The worker path. A `UiUploadSystem` is `SystemKind::GpuCompute`, dispatched
    /// SOLO on the dispatcher via [`run_dispatcher`](System::run_dispatcher); it must
    /// NEVER run on a worker (it has no [`DispatcherToken`], so the `!Send`
    /// `RhiContext` is structurally unreachable here). A loud debug panic flags a
    /// scheduler bug; a benign release no-op.
    ///
    /// # Safety
    /// **S1** — Vacuous: this body touches no world state.
    unsafe fn run_unsafe(&mut self, _cell: UnsafeEcsCell<'_>) -> Self::Out {
        debug_assert!(
            false,
            "UiUploadSystem ran on a worker via run_unsafe; it must be \
             SystemKind::GpuCompute and dispatched solo via run_dispatcher \
             (register it with SystemConfig::gpu() or rely on is_gpu())"
        );
    }

    /// The dispatcher-solo entry point. This shell is REGISTERED for its scheduler
    /// SHAPE (EMPTY access, `is_gpu()`, dispatcher-solo) but, in P5a, performs NO work
    /// here — and deliberately does NOT project-and-drop the `!Send` [`RhiContext`]
    /// (a project-then-discard would be a misleading "looks wired, does nothing").
    ///
    /// # Why the upload is host-driven, not done here (Rung-4 world-access seam)
    ///
    /// The pack→sort→upload reads the world's CPU columns (`ComputedRect`, …) and the
    /// [`UiRenderScratch`] / [`UiRenderGeneration`] `Resource`s AND the swapchain's
    /// per-frame slot index + in-flight fence (for the write-after-read upload
    /// contract). The column/resource-read HALF is now reachable through
    /// [`DispatcherToken::world`]'s
    /// [`WorldView`] (#30); the host-drivable
    /// [`host_upload_frame_from_world`](Self::host_upload_frame_from_world) (#31)
    /// gathers nodes through it before the `!Send` upload. The one capability still
    /// missing in-schedule is the swapchain `Renderer` slot index + in-flight fence —
    /// the `Renderer` is not yet an ECS resource — so the on-screen path is still
    /// driven by the render host through `host_upload_frame_from_world` (gather via the
    /// view → [`host_upload_frame`](Self::host_upload_frame): fence the slot →
    /// [`pack_sort_upload`](Self::pack_sort_upload)) + [`RhiContext::ui_pass`] +
    /// [`Renderer::present_sampled`]. This shell becomes the in-schedule upload site
    /// once an ECS-resident swapchain handle exists (tracked for the orchestrator).
    ///
    /// # Safety
    /// **S1'** — Vacuous: this body touches no world state and mints no aliasing
    /// reference (it does not even project the token).
    unsafe fn run_dispatcher(&mut self, _token: DispatcherToken<'_>) -> Self::Out {}

    /// No deferred mutations — the upload's effects live entirely in the VRAM ring +
    /// the stashed POD `UiFramePlan`. No-op `apply` (MF-5).
    #[inline]
    fn apply(&mut self, _world: &mut EcsMaster) {}

    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    #[inline]
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.set_change_ticks(last_run, this_run);
    }

    #[inline]
    fn check_change_tick(&mut self, current: Tick) {
        self.meta.clamp_change_ticks(current);
    }
}
