//! The UI upload system (`UiUploadSystem`) — GUI P5a Rung 4 + UI-ADVANCED S0.
//!
//! A `GpuSystem`-shaped consumer (EMPTY [`Access`], [`is_gpu`](System::is_gpu) →
//! `SystemKind::GpuCompute`, dispatcher-solo) that, on a UI-change frame:
//!
//! 1. **O(1) generation gate** — short-circuits on `gen == last_seen_generation`
//!    (the [`UiRenderGeneration`] resource, A1 step 1); a static frame does nothing.
//! 2. **packs** every visible node `(ComputedRect, UiBackground, ComputedClip?,
//!    StackIndex?)` into a reused scratch (`clear()` + `extend`, never `Vec::new`),
//! 3. **stable z-sorts** by `(StackIndex, append_order)` in place (zero alloc),
//! 4. **uploads** the packed records into the current-FIF host-mapped ring via
//!    [`RhiContext::ui_upload`], and
//! 5. **stashes** the POD-by-value [`UiFramePlan`] the swapchain recorder reads
//!    (Decision 9: it borrows NO RHI handle, so nothing `!Send` crosses the token
//!    drop; the recorder re-resolves the pipeline + bind-group by `frame_index`).
//!
//! # The world-access seam — TWO PHASES, SEQUENCED, NEVER FUSED (UI-ADVANCED S0)
//!
//! The in-schedule seam is [`System::run_dispatcher`], and it is the mirror of
//! the shipped [`GpuSystem`](crate::GpuSystem) ordering — world read first,
//! `!Send` projection second, never both at once:
//!
//! * **Phase 1 (shared borrow):** [`DispatcherToken::world`]'s read-only
//!   [`WorldView`] carries the D6a generation gate (structural skip: an
//!   unchanged generation returns before ONE component is probed) and, on a
//!   changed frame, [`gather_into_staging`](UiUploadSystem::gather_into_staging)
//!   — gather + pack + z-sort into the system-owned `staging` box. The view is
//!   dropped at the phase's closing brace; only the packed COUNT crosses.
//! * **Phase 2 (exclusive borrow):** [`DispatcherToken::nonsend_resource_mut`]
//!   projects the `!Send` [`RhiContext`] and
//!   [`upload_staging`](UiUploadSystem::upload_staging) memcpys
//!   `staging[..n]` into the fenced ring slot. No world type appears in this
//!   phase's signature, so the fusion cannot be re-written (rung S0 gate G0-5).
//!
//! The two phases borrow the SAME token — `world()` takes `&self`,
//! `nonsend_resource_mut` takes `&mut self` — so a body holding both at once is
//! the M1 conflict borrowck refuses (`dispatcher_token.rs:185-190`, and the
//! per-route probes in `docs/OPEN-QUESTIONS.md`, entry 2026-08-21). The fused
//! predecessor of this seam (`host_upload_frame_from_world`, whose parameter
//! list demanded a live `WorldView` AND a `&mut RhiContext` at one call site)
//! had NO possible caller and is DELETED, not re-signed.
//!
//! The pure pack→sort→upload pipeline additionally remains available as the
//! world-AGNOSTIC core [`UiUploadSystem::pack_sort_upload`], which takes the
//! per-node inputs as an iterator plus the scratch, the ortho, the current
//! `frame_index`, and an `&mut RhiContext`. It is fully unit-/Miri-testable
//! (no Arena / world) and is what the render host and the goldens drive
//! directly (host-time world reads go through `EcsMaster::run_closure_once` /
//! a query system — never through a smuggled view).
//!
//! # Driving the seam from a host (until the `Renderer` is an ECS resource)
//!
//! The swapchain [`Renderer`] is host-held, so the host mints the write proof
//! and stages it BEFORE dispatching the system:
//!
//! 1. `let token = renderer.wait_frame_in_flight()?;` — fence the slot (the
//!    write-after-read contract) and mint the [`FrameWriteToken`];
//! 2. `sys.stage_frame(token, ortho);` — stage the POD frame inputs into the
//!    system (both are `Send + 'static`; no `!Send` state enters the system);
//! 3. `ecs.run_system_once(&mut sys);` — the dispatcher-solo run: Phase 1 then
//!    Phase 2;
//! 4. `let (plan, token) = sys.take_frame_output()…;` — the plan for
//!    [`RhiContext::ui_pass`] and the token for the frame-ending
//!    `present_sampled` consume (BY VALUE — R0b).
//!
//! A run with no staged frame (a bare [`EcsMaster`], the scheduler, a
//! device-free test) executes Phase 1 alone and returns at Phase 2's
//! projection — which is exactly what makes the S0 observer and gates
//! G0-2/G0-3 device-free: Phase 1 is unit-testable with a bare `EcsMaster`
//! through [`EcsMaster::run_system_once`], no graphics type in sight.

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
use crate::ui::gather::{gather_ui_nodes, UiGatherScratch};
use crate::ui::instance::{UiInstance, UiOrtho};
use crate::ui::pack::{pack_ui_instance, PackInput, UiRenderGeneration, UiRenderScratch};
use crate::ui::plan::UiFramePlan;
use crate::ui::FRAMES_IN_FLIGHT;

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

/// Rows in the preallocated [`UiUploadSystem`] staging box (sized at
/// [`System::initialize`], never grown in the frame loop). 4096 × 80 B = 320 KiB
/// (the S2-widened stride) — 2× the plan's own N = 2048 measurement scene, so
/// steady state never touches the overflow clamp.
pub const UI_STAGING_ROWS: usize = 4096;

/// The all-zero [`UiInstance`] the staging box is seeded with at initialize.
const UI_INSTANCE_ZERO: UiInstance = UiInstance {
    min_px: [0.0; 2],
    size_px: [0.0; 2],
    clip: [0.0; 4],
    corner_radius: [0.0; 4],
    uv: [0.0; 4],
    color: 0,
    border_color: 0,
    border_width: 0.0,
    flags: 0,
};

/// Host-staged per-frame inputs for Phase 2 (see the module doc's "Driving the
/// seam from a host"): the fenced slot's write proof + this frame's ortho. Both
/// POD and `Send + 'static` — nothing `!Send` enters the system's state.
struct PendingFrame {
    /// The write proof minted by [`Renderer::wait_frame_in_flight`] — the ONLY
    /// source of the ring slot index (R0b), returned to the host with the plan
    /// for the frame-ending `present_sampled` consume.
    token: FrameWriteToken,
    /// [`UiOrtho::for_extent`] of the swapchain extent this frame presents into.
    ortho: UiOrtho,
}

/// The UI upload system (Rung 4 + UI-ADVANCED S0): a `GpuSystem`-shaped
/// `impl System` (EMPTY access, `is_gpu()`, dispatcher-solo) carrying the
/// SETUP-class state the two-phase upload needs — the staging box (the one
/// staging mirror for the GPU-contiguity ring write; Principle 0's named
/// legitimate exception), the gather scratch, and the per-slot generation gate.
///
/// The per-frame seam is the two-phase [`System::run_dispatcher`]; the
/// world-agnostic core [`pack_sort_upload`](Self::pack_sort_upload) remains the
/// host/golden driver. See the module docs.
pub struct UiUploadSystem {
    /// The logical→physical DPI scale folded into every length at pack (so the shader
    /// works in physical px and `fwidth` AA is one device pixel). The host updates it
    /// when the viewport scale factor changes (and bumps the generation).
    scale_factor: f32,
    /// Phase 1's pack target: the preallocated staging mirror for the ring
    /// memcpy — sized ONCE at [`System::initialize`] ([`UI_STAGING_ROWS`]),
    /// never grown in the frame loop. Durable per-entity data stays in ECS
    /// columns; this box holds one frame's packed, z-sorted GPU records only.
    staging: Box<[UiInstance]>,
    /// Records staged by the LAST gather (the prefix of `staging` that is live).
    staged: usize,
    /// Gather output scratch (cleared + refilled per changed frame; capacity
    /// persists — zero steady-state allocation).
    node_buf: Vec<UiNode>,
    /// Parallel `(stack, append)` sort-key lane — the same total-order key
    /// [`UiRenderScratch::sort_by_stack`] uses, reused per changed frame.
    keys: Vec<(u32, u32)>,
    /// The DFS gather scratch + the S0 probe census
    /// ([`UiGatherScratch::probes`]).
    gather_scratch: UiGatherScratch,
    /// DIAGNOSTIC (S0 item 6): packs executed by
    /// [`gather_into_staging`](Self::gather_into_staging), ever (wrapping).
    /// With [`UiGatherScratch::probes`] this is the seam's COMMAND CENSUS: over
    /// a static run both counters must record ZERO work (gate G0-2 asserts the
    /// census, not a timing delta).
    repacks: u64,
    /// DIAGNOSTIC: frames whose gather emitted more than [`UI_STAGING_ROWS`]
    /// records (release-clamped to the box; loud in debug). A non-zero value
    /// means the staging box is undersized for the scene.
    staging_overflows: u64,
    /// The last generation seen, PER frame-in-flight ring slot — the O(1) D6a
    /// change gate, hoisted ahead of the gather (S0 item 5). Per slot because a
    /// skip re-serves the SLOT's ring contents: a single scalar would skip the
    /// second slot onto stale bytes. `u64::MAX` = "never seen", so a fresh
    /// system always packs. A run with no staged frame gates on lane 0 — one
    /// system instance serves ONE driving mode (host-staged or device-free);
    /// do not interleave them on one instance.
    last_seen_generation: [u64; FRAMES_IN_FLIGHT],
    /// The record count last uploaded into each ring slot — what a skipped
    /// frame's [`UiFramePlan`] re-serves (the ortho is rebuilt from the live
    /// extent each frame; only the count is slot-resident state).
    last_counts: [u32; FRAMES_IN_FLIGHT],
    /// The host-staged frame inputs (write proof + ortho), taken by the next
    /// `run_dispatcher`. `None` ⇒ Phase 1 only (device-free / in-schedule run).
    pending_frame: Option<PendingFrame>,
    /// The last host-driven run's output: the upload verdict (the plan, or the
    /// ring-grow error the host must see) + the write proof handed back for the
    /// frame-ending `present_sampled` consume.
    frame_output: Option<(Result<UiFramePlan, GpuColumnError>, FrameWriteToken)>,
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
    /// [`GpuSystem`](crate::GpuSystem). The staging box is allocated at
    /// [`System::initialize`], not here — construction stays allocation-free.
    pub fn new(scale_factor: f32) -> Self {
        debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
        // `Tick::new(1)` is the construction sentinel (the dispatcher overwrites the
        // snapshot before the first run); the system consumes no change ticks (empty
        // access — it gates on the explicit `UiRenderGeneration` counter instead).
        let meta = SystemMeta::new(std::any::type_name::<UiUploadSystem>(), Tick::new(1));
        Self {
            scale_factor,
            staging: Box::new([]),
            staged: 0,
            node_buf: Vec::new(),
            keys: Vec::new(),
            gather_scratch: UiGatherScratch::default(),
            repacks: 0,
            staging_overflows: 0,
            last_seen_generation: [u64::MAX; FRAMES_IN_FLIGHT],
            last_counts: [0; FRAMES_IN_FLIGHT],
            pending_frame: None,
            frame_output: None,
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

    /// Stages the host-minted frame inputs for the NEXT dispatch (module doc's
    /// "Driving the seam from a host"): the [`FrameWriteToken`] from
    /// [`Renderer::wait_frame_in_flight`] (the fenced slot's write proof — the
    /// only source of the slot index, R0b) and this frame's `ortho`.
    #[inline]
    pub fn stage_frame(&mut self, token: FrameWriteToken, ortho: UiOrtho) {
        debug_assert!(
            self.pending_frame.is_none(),
            "invariant: one staged frame per dispatch — the previous frame was \
             never dispatched"
        );
        self.pending_frame = Some(PendingFrame { token, ortho });
    }

    /// Takes the last host-driven run's output: the upload verdict (the
    /// [`UiFramePlan`], or the ring error the host must handle) and the
    /// [`FrameWriteToken`] handed back for the frame-ending `present_sampled`
    /// consume (BY VALUE — R0b). `None` if the last run had no staged frame.
    #[inline]
    pub fn take_frame_output(
        &mut self,
    ) -> Option<(Result<UiFramePlan, GpuColumnError>, FrameWriteToken)> {
        self.frame_output.take()
    }

    /// The records staged by the last [`gather_into_staging`](Self::gather_into_staging)
    /// — packed, z-sorted, ready for [`upload_staging`](Self::upload_staging).
    /// The S0 observer + gate G0-3 read the packed-count (and the rows) here.
    #[inline]
    pub fn staged(&self) -> &[UiInstance] {
        &self.staging[..self.staged]
    }

    /// COMMAND CENSUS, gather half: component probes ever issued by this
    /// system's gathers (forwards [`UiGatherScratch::probes`]). A static run
    /// must not advance it (G0-2).
    #[inline]
    pub fn probes(&self) -> u64 {
        self.gather_scratch.probes
    }

    /// COMMAND CENSUS, pack half: packs ever executed by
    /// [`gather_into_staging`](Self::gather_into_staging). A static run must
    /// not advance it (G0-2).
    #[inline]
    pub fn repacks(&self) -> u64 {
        self.repacks
    }

    /// DIAGNOSTIC: staging-box overflow clamps (see the field doc).
    #[inline]
    pub fn staging_overflows(&self) -> u64 {
        self.staging_overflows
    }

    /// **Phase 1 (device-free):** gather + pack + z-sort into the system-owned
    /// staging box, returning the packed COUNT — the only thing that crosses
    /// the seam to Phase 2 (never the view's borrow; red mutation M0-b).
    ///
    /// No `!Send` type in the signature: the phase reads the world exclusively
    /// through the [`WorldView`] `&self` surface and writes only `self`'s own
    /// buffers, so it is unit-testable with a bare [`EcsMaster`] (through
    /// [`EcsMaster::run_system_once`], which mints the token) — no device, no
    /// graphics type (rung S0 gate G0-5 pins this signature).
    ///
    /// A gather that emits more than the staging box holds is clamped to the
    /// box in release (loud `debug_assert!` in dev, counted in
    /// [`staging_overflows`](Self::staging_overflows)) — the S-D7 house
    /// pattern: fail loudly in dev, degrade visibly-but-safely in release.
    pub fn gather_into_staging(&mut self, view: &WorldView<'_>) -> usize {
        // COMMAND CENSUS (S0 item 6): one pack executed. Counted at the pack
        // itself, not at the gate, so a wrongly-placed gate that still gathers
        // but skips the pack keeps this at zero while the probe census moves.
        self.repacks = self.repacks.wrapping_add(1);

        self.node_buf.clear();
        gather_ui_nodes(view, &mut self.gather_scratch, &mut self.node_buf);

        let emitted = self.node_buf.len();
        let n = if emitted > self.staging.len() {
            debug_assert!(
                false,
                "UiUploadSystem staging box overflow: gather emitted {emitted} \
                 records into a {}-row box (raise UI_STAGING_ROWS)",
                self.staging.len()
            );
            self.staging_overflows = self.staging_overflows.wrapping_add(1);
            self.staging.len()
        } else {
            emitted
        };

        // z-sort via the (stack, append) key lane — the same TOTAL order
        // `UiRenderScratch::sort_by_stack` argues (append is unique, so an
        // unstable sort IS the stable permutation, zero alloc) — then pack the
        // records into `staging` directly in sorted order.
        let Self {
            staging,
            node_buf,
            keys,
            scale_factor,
            ..
        } = self;
        keys.clear();
        for (append, node) in node_buf[..n].iter().enumerate() {
            keys.push((node.stack, append as u32));
        }
        keys.sort_unstable_by_key(|&k| k);
        for (dst, &(_, append)) in keys.iter().enumerate() {
            staging[dst] = pack_ui_instance(&node_buf[append as usize].input, *scale_factor);
        }

        self.staged = n;
        n
    }

    /// **Phase 2 (exclusive):** memcpy the staged records into the fenced ring
    /// slot via [`RhiContext::ui_upload`], returning the POD-by-value plan.
    ///
    /// No world type in the signature — no [`WorldView`], no [`EcsMaster`] —
    /// so the gather/upload fusion cannot be re-written here (rung S0 gate
    /// G0-5 pins this signature; the trybuild fixture
    /// `tests/ui_s0_seam_fusion/` pins that a call site holding both borrows
    /// does not compile). Deliberately an associated fn with no `self`: Phase 2
    /// cannot reach the gather state at all.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a ring grow / mapping failure (or if
    /// [`RhiContext::ui_setup`](crate::RhiContext::ui_setup) was never called).
    pub fn upload_staging(
        rhi: &mut RhiContext,
        packed: &[UiInstance],
        ortho: UiOrtho,
        token: &FrameWriteToken,
    ) -> Result<UiFramePlan, GpuColumnError> {
        rhi.ui_upload(packed, ortho, token)
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
        // DIAGNOSTIC (S0 item 6): one repack executed on the LEGACY host path.
        // Counted HERE — at the pack itself, not at a gate — so a wrongly-placed
        // gate that still gathers but skips the pack keeps this at zero while
        // the probe counter moves.
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
}

// SAFETY (S1' + MF-5 / Option C, mirroring `GpuSystem`): `run_dispatcher` reaches
//   the world ONLY through the token's blessed projections, in sequence — Phase 1
//   reads through the read-only `WorldView` (`&self` of the token), whose borrow
//   ends at the phase's closing brace; Phase 2 projects the `!Send` `RhiContext`
//   through `nonsend_resource_mut` (`&mut self` of the token). Borrowck forbids
//   the two coexisting (M1, dispatcher_token.rs:185-190), and the scheduler runs
//   a `GpuCompute` system dispatcher-solo at `running == 0`, where the token is
//   mintable — so no worker aliases either projection. The declared `Access` is
//   EMPTY: the world reads go through the view, which is dispatcher-solo by
//   construction, not through the conflict graph. `run_unsafe` is
//   unreachable-by-design (a worker holds no token, so both the view and the
//   `!Send` `RhiContext` are structurally unreachable on the worker path).
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

    /// Allocates the staging box ONCE ([`UI_STAGING_ROWS`] rows) — the one
    /// setup-time allocation the seam owns; the frame loop never grows it.
    /// Idempotent (re-`initialize` keeps the existing box).
    fn initialize(&mut self, _world: &mut EcsMaster) {
        if self.staging.is_empty() {
            self.staging = vec![UI_INSTANCE_ZERO; UI_STAGING_ROWS].into_boxed_slice();
        }
    }

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

    /// The dispatcher-solo entry point — the UI-ADVANCED S0 two-phase seam
    /// (sequence, never fuse; the shipped `GpuSystem::run_dispatcher` ordering):
    ///
    /// * **Phase 1 (shared):** the D6a generation gate, hoisted ahead of the
    ///   gather (a static frame costs one `u64` compare and ZERO component
    ///   probes — the structural skip), then
    ///   [`gather_into_staging`](Self::gather_into_staging) on a changed frame.
    ///   The [`WorldView`] lives only inside the phase's braces.
    /// * **Phase 2 (exclusive):** project the `!Send` [`RhiContext`] and
    ///   [`upload_staging`](Self::upload_staging) into the host-staged fenced
    ///   slot. With no staged frame (bare world / scheduler / device-free
    ///   test) or no registered `RhiContext`, the phase returns — Phase 1
    ///   alone IS the device-free observer surface (S0).
    ///
    /// The gate is PER ring slot: after one change, each slot repacks once
    /// (its ring holds stale bytes until its own repack) and only then skips.
    /// The gate commit is rolled back on an upload error so the next frame
    /// retries instead of re-serving a count the ring never received.
    ///
    /// # Panics
    /// If the world has no [`UiRenderGeneration`] resource. The gate refuses to
    /// guess: a host that never registered
    /// [`ui_render_discovery`](crate::ui::gather::ui_render_discovery) (and its
    /// resource) would otherwise silently repack every frame — the "gate that
    /// cannot fail" shape this project keeps recording.
    ///
    /// # Safety
    /// **S1'** — The token witnesses `running == 0` (dispatcher-solo mint); the
    /// body holds at most ONE token projection at a time (see the `unsafe impl`
    /// SAFETY block).
    unsafe fn run_dispatcher(&mut self, mut token: DispatcherToken<'_>) -> Self::Out {
        // The host-staged frame inputs, if any. Taken FIRST so a panic or an
        // early return never leaves a stale write proof armed for a later
        // frame. No staged frame ⇒ the device-free lane (slot 0).
        let pending = self.pending_frame.take();
        let slot = pending.as_ref().map_or(0, |f| f.token.slot());
        debug_assert!(
            slot < FRAMES_IN_FLIGHT,
            "invariant: the staged write proof addresses a UI ring slot"
        );

        // ── Phase 1 (shared borrow): gate, then gather into staging. ──
        let n = {
            let view = token.world();
            let generation = view.resource::<UiRenderGeneration>().generation;

            // The per-slot gate, AHEAD of the gather (D6a): nothing changed
            // since this SLOT last packed ⇒ its ring bytes are current — skip
            // the gather AND the pack (zero probes, zero packs, zero upload
            // commands — the G0-2 census). A host-staged frame still gets a
            // plan re-serving the slot's count under THIS frame's ortho (the
            // ortho is extent-derived per frame; the packed bytes are not).
            if generation == self.last_seen_generation[slot] {
                if let Some(frame) = pending {
                    self.frame_output = Some((
                        Ok(UiFramePlan {
                            instance_count: self.last_counts[slot],
                            ortho: frame.ortho,
                            frame_index: slot,
                        }),
                        frame.token,
                    ));
                }
                return;
            }
            self.last_seen_generation[slot] = generation;
            self.gather_into_staging(&view)
        };
        // ^ This closing brace drops `view`, ending the token's shared borrow
        // BEFORE Phase 2's `&mut` projection — the M1 discipline
        // (dispatcher_token.rs:185-190: a `WorldView` cannot coexist with
        // `nonsend_resource_mut`). Only the packed COUNT `n` crosses the seam —
        // never the view's borrow: a view-read placed AFTER Phase 2's
        // projection is E0502 (red mutation M0-b, ledger 2026-08-21). NOTE the
        // brace alone is NOT compile-load-bearing — NLL already ends the
        // borrow at the view's last use (M0-a's ruled E0502 was probed
        // 2026-08-21 and the hoisted-brace form COMPILES) — so the brace is
        // scope hygiene against a future edit that HOLDS the view, and the
        // compile-time tripwire is the M0-b shape + the G0-5 trybuild fixture
        // (`tests/ui_s0_seam_fusion/`), which does red. A brace whose purpose
        // is invisible is a brace someone deletes; this comment is its purpose.
        self.last_counts[slot] = n as u32;

        // ── Phase 2 (exclusive borrow): project the !Send context, upload. ──
        let Some(rhi) = token.nonsend_resource_mut::<RhiContext>() else {
            // Device-free world (the S0 observer / G0-2 / G0-3 harness, or a
            // host that never inserted the context): Phase 1 already did all
            // the device-free work. Not a defect — the seam's honest floor.
            return;
        };
        let Some(frame) = pending else {
            // In-schedule run with a live context but no host-staged write
            // proof: the swapchain `Renderer` is not yet an ECS resource, so
            // there is no fenced slot to write. The staging is packed and
            // waiting; the host drive (stage_frame → run_system_once →
            // take_frame_output) is the shipped route.
            return;
        };
        let verdict = match Self::upload_staging(rhi, &self.staging[..n], frame.ortho, &frame.token)
        {
            Ok(plan) => Ok(plan),
            Err(e) => {
                // Roll back the gate commit: the ring never received this
                // generation's bytes, so the next frame must retry rather than
                // skip onto stale contents.
                self.last_seen_generation[slot] = u64::MAX;
                Err(e)
            }
        };
        self.frame_output = Some((verdict, frame.token));
    }

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
