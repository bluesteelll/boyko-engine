//! The Render Dependency Graph bridge for the whole G-buffer frame: the
//! [`GbufferPassPlan`] (per-frame `PassId` map), the [`GbufferBarrierSink`] that
//! resolves each derived barrier to a physical `VkImage`/`VkBuffer` and records it,
//! and the `Renderer` methods that (re)declare the graph + drive one pass's derived
//! barriers. Split out of the former monolithic `swapchain.rs` (audit W4).

use core::ptr;

use crate::device::DeviceFns;
use crate::ffi::*;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::frame_driver::Renderer;
use super::scene_types::GBufferScene;
use super::targets::GBufferTargets;
use super::COLOR_SUBRESOURCE_RANGE;

/// The per-frame [`PassId`](crate::framegraph::PassId) of each G-buffer pass declared
/// into [`Renderer::frame_graph`] by [`Renderer::render_gbuffer_frame`] (Steps 1d/1e).
///
/// `record_gbuffer` reads this (via `as_ref().expect(...)`) at each barrier site to
/// drive that pass's derived barriers when the framegraph flag is ON. The optional
/// members mirror `record_gbuffer`'s config gates EXACTLY: a member is `Some` iff its
/// pass body is recorded this frame, so a graph barrier is never emitted for a pass
/// whose GPU work was skipped (and vice versa — no double-barrier, no missing one).
#[derive(Clone, Copy)]
pub(crate) struct GbufferPassPlan {
    /// Pillar B B3: the per-instance TRS interpolation compute PRE-PASS (`scene.interp.
    /// is_some()`). Recorded BEFORE the raster pass; its `record_pass` is a no-op (the interp
    /// pass reads the FIF-private pair SSBO + writes the FIF-private draw SSBO — both frame-
    /// private, so the graph derives NO input barrier). The COMPUTE→VERTEX RAW barrier on the
    /// draw SSBO is derived at the raster pass (the draw-buffer READER) and emitted by
    /// `record_pass(raster)` — after this pass's dispatch wrote the draw columns.
    pub(crate) interp: Option<crate::framegraph::PassId>,
    /// The always-present 3-MRT + depth raster pass (sites 0/1).
    pub(crate) raster: crate::framegraph::PassId,
    /// The async light-table re-upload (`scene.light_dirty && light_upload_bytes>0`).
    pub(crate) light_upload: Option<crate::framegraph::PassId>,
    /// The P0 coarse tile-cull (`scene.coarse.is_some()`).
    pub(crate) coarse: Option<crate::framegraph::PassId>,
    /// The always-present marcher pass. Its `record_pass` emits the collapsed input
    /// transitions (depth→sampled, color→general, lit/viewt first-touch — sites 3/3b/4).
    pub(crate) marcher: crate::framegraph::PassId,
    /// The SSAO pass (`scene.ssao.is_some()`).
    pub(crate) ssao: Option<crate::framegraph::PassId>,
    /// SDFDDGI I2: the probe-update pass (`scene.ddgi_update.is_some()`). Writes the two atlas
    /// storage images + the classification buffer; the update→resolve barrier is DERIVED at the
    /// resolve (the atlas reader), NOT hand-written.
    pub(crate) ddgi_update: Option<crate::framegraph::PassId>,
    /// The L1 clustered light-cull pass (`scene.cluster_cull.is_some()` + the buffers).
    pub(crate) light_cull: Option<crate::framegraph::PassId>,
    /// The CSM cascade depth pass (`scene.csm.is_some()`).
    pub(crate) csm: Option<crate::framegraph::PassId>,
    /// The sparse spot/point atlas depth pass (`scene.atlas_punctual.is_some()`).
    pub(crate) atlas: Option<crate::framegraph::PassId>,
    /// HW-RT rung R2a-3: the TLAS-instance PACK compute pre-pass (`scene.tlas.is_some()`). Runs
    /// after interp, before the raster `begin_rendering`; writes the `tlas_instances` array
    /// (COMPUTE/SHADER_WRITE) and, when interp ran, reads the shared ring (COMPUTE/SHADER_READ).
    /// `Some` iff `scene.tlas.is_some()` (the "member is Some iff its body is recorded" invariant).
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_pack: Option<crate::framegraph::PassId>,
    /// HW-RT rung R2a-3: the per-frame TLAS BUILD pass (`scene.tlas.is_some()`), right after
    /// `tlas_pack`. Reads the `tlas_instances` array at the AS-build stage
    /// (AS_BUILD/SHADER_READ), deriving the pack-write → build-read barrier; the AS write into
    /// the UNTRACKED backing/scratch is invisible to the graph.
    #[cfg(feature = "hwrt")]
    pub(crate) tlas_build: Option<crate::framegraph::PassId>,
    /// The always-present deferred resolve pass.
    pub(crate) resolve: crate::framegraph::PassId,
    /// The always-present present-sample pass: only the `lit` GENERAL→SHADER_READ_ONLY
    /// transition (site 5c). The swapchain WSI barriers stay hand-recorded.
    pub(crate) present_sample: crate::framegraph::PassId,
}

/// The number of IMAGE resources the whole-frame graph declares (Steps 1d/1e), in the
/// FIXED ResId order the sink resolves by: albedo=0, normal=1, material=2, depth=3,
/// viewt=4, lit=5, ssao=6, cascade=7, atlas=8, ddgi_irr=9, ddgi_depth=10. Buffer ResIds
/// follow, offset by this. SDFDDGI I2 appended the two DDGI atlas storage images (9/10) —
/// declared UNCONDITIONALLY (seeded with the boot `SHADER_READ_ONLY_OPTIMAL` layout) but only
/// ACCESSED on the `ddgi_update`/`resolve` passes that name them, so the OFF-path barrier set
/// (which never routes a barrier at ResId 9/10) is byte-unchanged.
pub(crate) const FRAMEGRAPH_IMAGE_COUNT: usize = 11;

/// The REAL [`BarrierSink`](crate::framegraph::BarrierSink) for the whole G-buffer
/// frame (Steps 1c–1e): it resolves each derived barrier's logical `res` → the current
/// physical `VkImage`/`VkBuffer` (images indexed by the ResId `0..9`, buffers by
/// `res.index() - FRAMEGRAPH_IMAGE_COUNT` — the graph's fixed declaration order) and
/// records ONE batched sync1 `vkCmdPipelineBarrier` per barrier group, exactly as the
/// hand path did.
///
/// Lives only for the duration of one `record_pass` call; borrows the device fn-table
/// and the open command buffer.
pub(crate) struct GbufferBarrierSink<'a> {
    pub(crate) fns: &'a DeviceFns,
    pub(crate) cmd: VkCommandBuffer,
    /// The physical images resolved by image `ResId` index `0..FRAMEGRAPH_IMAGE_COUNT`
    /// — `[albedo, normal, material, depth, viewt, lit, ssao, cascade, atlas, ddgi_irr,
    /// ddgi_depth]` for the current frame slot (the last two SDFDDGI I2, single-instance
    /// world-fixed atlases — NOT ringed). MUST match the graph's declaration order. A pass
    /// that does NOT declare an optional image (e.g. cascade when CSM is off, or the DDGI
    /// atlases when the update pass is off) never routes a barrier naming that ResId, so its
    /// slot may hold [`VkImage::NULL`] harmlessly.
    pub(crate) images: [VkImage; FRAMEGRAPH_IMAGE_COUNT],
    /// The physical buffers resolved by `res.index() - FRAMEGRAPH_IMAGE_COUNT` (the graph's
    /// FIXED buffer declaration order): `[light_table, tiles, grid, index, alloc,
    /// ddgi_classification, ddgi_ray_table, interp_pairs, interp_out_slot, interp_model_out]`.
    /// The two SDFDDGI I2 buffers (classification + Fibonacci ray-table) are single-instance,
    /// named by the `ddgi_update` pass ONLY when `scene.ddgi_update.is_some()`. The interp trio
    /// (Pillar B B3, refined-B) is the CURRENT frame slot's FIF-ringed interpolation SSBOs,
    /// bound ONLY when `scene.interp.is_some()`.
    ///
    /// Under `hwrt` the array grows by ONE: `tlas_instances` (HW-RT rung R2a-3) is declared
    /// UNCONDITIONALLY right AFTER the DDGI buffers (so its ResId is FIXED regardless of the
    /// conditional interp trio, which then shifts one slot later), landing at index 7 — the
    /// order becomes `[.., ddgi_ray_table, tlas_instances, interp_pairs, interp_out_slot,
    /// interp_model_out]`. On a `not(hwrt)` build the array is exactly `[VkBuffer; 10]` with
    /// unchanged indices (RISK-2: cfg-gated ⇒ no OFF-path ResId shift). On any OFF path an
    /// ungated slot is never named by a derived barrier, so its [`VkBuffer::NULL`] is inert
    /// (same NULL-when-ungated rule as [`Self::images`]).
    #[cfg(not(feature = "hwrt"))]
    pub(crate) buffers: [VkBuffer; 10],
    /// See the `not(hwrt)` variant's doc: `hwrt` grows this by one (`tlas_instances` at index 7).
    #[cfg(feature = "hwrt")]
    pub(crate) buffers: [VkBuffer; 11],
}

impl crate::framegraph::BarrierSink for GbufferBarrierSink<'_> {
    fn image_barriers(
        &mut self,
        src_stage: u32,
        dst_stage: u32,
        group: &[crate::framegraph::ImgBarrier],
    ) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "image barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
        // Build the `VkImageMemoryBarrier` array on the stack (alloc-free) from the
        // logical group, resolving each `res` → the bound physical image. Field style
        // mirrors the hand `to_color`/`to_depth` barriers this replaces. (`VkImageMemoryBarrier`
        // is not `Copy` — it carries a `p_next` pointer — so `from_fn` fills each slot;
        // the tail `[n..]` slots are inert placeholders never handed to the driver.)
        let n = group.len();
        let arr: [VkImageMemoryBarrier; crate::framegraph::MAX_PASS_BARRIERS] =
            core::array::from_fn(|i| {
                if i < n {
                    let b = group[i];
                    VkImageMemoryBarrier {
                        s_type: VkStructureType::ImageMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: b.src_access,
                        dst_access_mask: b.dst_access,
                        old_layout: b.old_layout,
                        new_layout: b.new_layout,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        image: self.images[b.res.index()],
                        subresource_range: VkImageSubresourceRange {
                            aspect_mask: b.subresource.aspect,
                            base_mip_level: b.subresource.base_mip,
                            level_count: b.subresource.mip_count,
                            base_array_layer: b.subresource.base_layer,
                            layer_count: b.subresource.layer_count,
                        },
                    }
                } else {
                    VkImageMemoryBarrier {
                        s_type: VkStructureType::ImageMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: 0,
                        dst_access_mask: 0,
                        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        new_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        image: VkImage::NULL,
                        subresource_range: COLOR_SUBRESOURCE_RANGE,
                    }
                }
            });
        // SAFETY: the command buffer is open (this sink is only driven inside the open
        // `record_gbuffer` recording). Every `arr[i].image` was resolved from the
        // `images[res.index()]` slot (a live G-buffer image for the current frame);
        // `res.index()` is in `0..FRAMEGRAPH_IMAGE_COUNT` for every image barrier the
        // whole-frame graph derives (images are ResId `0..11`). The masks/layouts/
        // subresource are the graph-derived Vk values that reproduce the hand path's
        // transitions. `arr[..n]` (a stack array) outlives the call; the count == `n`.
        // No memory or buffer barriers (`0, ptr::null(), 0, ptr::null()`), matching the
        // hand image calls.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                n as u32,
                arr.as_ptr().cast(),
            );
        }
    }

    fn buffer_barriers(
        &mut self,
        src_stage: u32,
        dst_stage: u32,
        group: &[crate::framegraph::BufBarrier],
    ) {
        debug_assert!(
            group.len() <= crate::framegraph::MAX_PASS_BARRIERS,
            "buffer barrier group ({}) exceeds MAX_PASS_BARRIERS",
            group.len()
        );
        // Build the `VkBufferMemoryBarrier` array on the stack (alloc-free), resolving
        // each `res` → the bound physical buffer via `res.index() - FRAMEGRAPH_IMAGE_COUNT`
        // (buffers are declared AFTER the images, so a buffer ResId is offset by the
        // image count). Field style mirrors the hand `to_shader_read`/`tiles_barrier`/
        // `cull_to_resolve` barriers this replaces. Whole-buffer range (`offset: 0`,
        // `size: VK_WHOLE_SIZE`) matches every hand buffer barrier. `VkBufferMemoryBarrier`
        // is `Copy` (no `p_next` payload), but `from_fn` keeps the tail slots inert
        // placeholders never handed to the driver (count == `n`).
        let n = group.len();
        let arr: [VkBufferMemoryBarrier; crate::framegraph::MAX_PASS_BARRIERS] =
            core::array::from_fn(|i| {
                if i < n {
                    let b = group[i];
                    VkBufferMemoryBarrier {
                        s_type: VkStructureType::BufferMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: b.src_access,
                        dst_access_mask: b.dst_access,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        buffer: self.buffers[b.res.index() - FRAMEGRAPH_IMAGE_COUNT],
                        offset: 0,
                        size: VK_WHOLE_SIZE,
                    }
                } else {
                    VkBufferMemoryBarrier {
                        s_type: VkStructureType::BufferMemoryBarrier,
                        p_next: ptr::null(),
                        src_access_mask: 0,
                        dst_access_mask: 0,
                        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                        buffer: VkBuffer::NULL,
                        offset: 0,
                        size: VK_WHOLE_SIZE,
                    }
                }
            });
        // SAFETY: the command buffer is open (this sink is only driven inside the open
        // `record_gbuffer` recording). Every `arr[i].buffer` was resolved from the
        // `buffers[res.index() - FRAMEGRAPH_IMAGE_COUNT]` slot (a live scene buffer for
        // this frame); a buffer barrier's `res.index()` is always `>= FRAMEGRAPH_IMAGE_COUNT`
        // (buffers are declared after the 11 images) and `< FRAMEGRAPH_IMAGE_COUNT +
        // buffers.len()` (the 5 core + 2 DDGI + 3 interp buffer ResIds, plus the 1 R2a-3
        // `tlas_instances` ResId under `hwrt`).
        // The masks are the graph-derived Vk values that reproduce the hand path's
        // TRANSFER→COMPUTE / COMPUTE→COMPUTE flush/visibility hazards; the whole-buffer
        // range matches every hand buffer barrier. `arr[..n]` (a stack array) outlives
        // the call; the count == `n`. No memory or image barriers, matching the hand
        // buffer calls.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                n as u32,
                arr.as_ptr().cast(),
                0,
                ptr::null(),
            );
        }
    }
}

impl Renderer<'_> {
    /// Steps 1d/1e: re-declare the WHOLE G-buffer frame into `self.frame_graph`
    /// (`reset` + declare + `compile`), config-gated from `scene`, and store the
    /// per-pass [`GbufferPassPlan`] in `self.gbuffer_pass_plan`. Called by
    /// `render_gbuffer_frame` every frame, immediately before the `&self`
    /// `record_gbuffer`, which drives each pass's derived barriers through it.
    ///
    /// The declared accesses MUST mirror `record_gbuffer`'s real `(stage, access,
    /// layout, subresource)` for the MAXIMAL permutation — this is the reference
    /// `tests/framegraph_gbuffer_equiv.rs::build_maximal_frame` (minus the swapchain
    /// image, whose WSI barriers stay hand-recorded). Resources are declared in a FIXED
    /// order that pins the ResIds the [`GbufferBarrierSink`] resolves by: images
    /// albedo=0..atlas=8, then SDFDDGI I2 ddgi_irr=9/ddgi_depth=10, then buffers
    /// light_table=11..alloc=15, ddgi_classification=16/ddgi_ray_table=17, then the
    /// (conditional) interp trio=18/19/20.
    ///
    /// Zero heap allocation (the arenas keep capacity across `reset`); the per-frame
    /// `compile` walks a ~11-pass line (cheap).
    pub(crate) fn declare_gbuffer_graph(&mut self, scene: &GBufferScene<'_>) {
        use crate::framegraph::{ResSync, SubRange};

        // The (EARLY|LATE)_FRAGMENT_TESTS stage pair the depth-write barriers use.
        const FRAG: u32 =
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
        // The marcher/SSAO storage-image read|write access on the G-buffer attributes.
        const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

        let g = &mut self.frame_graph;
        g.reset();

        // --- Images (FIXED ResId order: albedo=0..atlas=8). The G-buffer attributes +
        // depth + lit + ssao are RINGED per frame-in-flight (`ring[fi]`), so each slot's
        // reuse is already fence-ordered two frames back — they start `undefined()`.
        // The CSM cascade + shadow atlas are SINGLE instances shared by BOTH in-flight
        // frames: their depth passes re-render them every armed frame, so the re-render
        // must ORDER after the sibling frame's still-pipelined resolve reads — the
        // cross-frame seed supplies that WAR src (audit B-003; the world-fixed viewer
        // masked it because identical content made the torn read benign).
        let albedo = g.add_image("albedo");
        let normal = g.add_image("normal");
        let material = g.add_image("material");
        let depth = g.add_image("depth");
        let viewt = g.add_image("viewt");
        let lit = g.add_image("lit");
        let ssao = g.add_image("ssao");
        let cascade = g.add_image_seeded(
            "cascade",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let atlas = g.add_image_seeded(
            "atlas",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // SDFDDGI I2 (ResIds 9/10): the two DDGI probe atlases (irradiance + depth). SINGLE
        // world-fixed instances (Decision D1/D2 — probe i is the same world point every frame, one
        // persistent atlas, NO ring, NO reprojection). Boot-initialized to SHADER_READ_ONLY_OPTIMAL
        // in `DdgiAtlas::create` (unlike SSAO's UNDEFINED first-touch), so SEED them with that layout
        // (the light_table `add_buffer_seeded` cross-frame-seed pattern) — the RDG then derives the
        // correct SHADER_READ_ONLY_OPTIMAL → GENERAL transition for the update's storage WRITE, and
        // the update-write → resolve-read GENERAL→GENERAL barrier at the resolve (the atlas reader).
        // Declared UNCONDITIONALLY (fixed ResId order) but ACCESSED only by the `ddgi_update` /
        // `resolve` passes that name them, so the OFF path routes no barrier here (byte-identical).
        // Seed the REAL boot layout (SHADER_READ_ONLY_OPTIMAL), NOT the discard-legal UNDEFINED the
        // cascade/atlas use: the DDGI atlases are PERSISTENT accumulators (Decision D2 — round-robin
        // writes 1/N tiles/frame, the rest MUST survive), so the first storage write each frame needs
        // a CONTENT-PRESERVING SHADER_READ_ONLY_OPTIMAL → GENERAL transition. A UNDEFINED oldLayout
        // would let the driver DISCARD the un-updated tiles (plan §2.5/§7).
        let ddgi_irr = g.add_image_seeded(
            "ddgi_irr",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        );
        let ddgi_depth = g.add_image_seeded(
            "ddgi_depth",
            ResSync::seeded_readers_at_layout(
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        );
        // --- Buffers (ResId 11..15) — ALL single instances shared by both in-flight
        // frames (audit B-002). light_table/tiles/grid/index end their frame consumed
        // by a COMPUTE read (resolve / marcher), so a dirty-frame re-write must order
        // after those sibling reads (WAR seed). `alloc` ends its frame on the cull's
        // atomic WRITES with no draining read, so its per-frame TRANSFER reset needs
        // the full memory dependency (writer seed).
        let light_table = g.add_buffer_seeded(
            "light_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let tiles = g.add_buffer_seeded(
            "tiles",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let grid = g.add_buffer_seeded(
            "grid",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let index = g.add_buffer_seeded(
            "index",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let alloc = g.add_buffer_seeded(
            "alloc",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        );
        // --- SDFDDGI I2 buffers (ResIds 16/17 — sink slots 8/9). Declared UNCONDITIONALLY (so the
        // interp trio below always follows at ResIds 18/19/20 → its fixed sink slots 5/6/7) but
        // ACCESSED only by the `ddgi_update` pass that names them, so the OFF path routes no barrier
        // here. Both are single device-local instances (the classification buffer 1 u32/probe and the
        // boot-static Fibonacci ray table): the update pass reads the ray table, read/writes the
        // classification. Seeded like the other single-instance buffers so a cross-frame re-touch
        // orders after the sibling frame's still-pipelined update reads (WAR seed).
        let ddgi_classification = g.add_buffer_seeded(
            "ddgi_classification",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        let ddgi_ray_table = g.add_buffer_seeded(
            "ddgi_ray_table",
            ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
        );
        // --- HW-RT rung R2a-3 `tlas_instances` (ResId 18 under `hwrt`; RISK-2) — the compute-
        // written `VkAccelerationStructureInstanceKHR[]` array, the ONLY new framegraph-tracked
        // resource. Declared UNCONDITIONALLY (so its ResId is FIXED regardless of the conditional
        // interp trio, which then shifts to 19/20/21 → sink slots 8/9/10). Per-FIF frame-private
        // (`add_buffer`, undefined seed — the pack fully overwrites `[0..count)` each frame, and
        // the sibling in-flight frame touches the OTHER slot; no cross-frame WAR/WAW hazard).
        // Because the whole declaration is `#[cfg(feature = "hwrt")]`-gated, a `not(hwrt)` build
        // leaves every existing ResId unchanged. On a tlas-off frame no pass declares a
        // `buffer_access` on it, so the graph routes zero barriers naming it (byte-identical OFF).
        #[cfg(feature = "hwrt")]
        let tlas_instances = g.add_buffer("tlas_instances");
        // --- Pillar B B3 interp SSBOs (ResIds 18/19/20; 19/20/21 under `hwrt`, refined-B) —
        // declared ONLY when the
        // interp pass is wired, so the OFF path's ResId + barrier counts are byte-unchanged (the
        // equiv pins). All three are FIF-RINGED (frame-private, like the G-buffer ring): the host
        // writes this frame's slot of `interp_pairs` + `interp_out_slot`, the interp compute
        // writes the DYNAMIC slots of the SHARED `interp_model_out` (the instance ring), and the
        // raster/shadow VS read the SAME `interp_model_out` slot — a sibling in-flight frame
        // touches a DIFFERENT slot. So they start `undefined()` (plain `add_buffer`, NOT seeded):
        // no cross-frame WAR/WAW hazard, only the intra-frame COMPUTE→VERTEX RAW the graph derives
        // at the raster (the model_out reader).
        let (interp_pairs, interp_out_slot, interp_model_out) = if scene.interp.is_some() {
            (
                Some(g.add_buffer("interp_pairs")),
                Some(g.add_buffer("interp_out_slot")),
                Some(g.add_buffer("interp_model_out")),
            )
        } else {
            (None, None, None)
        };

        // Pass `interp` (Pillar B B3, refined-B) — gated `scene.interp.is_some()`. Runs FIRST
        // (before raster): reads the pair + out-slot SSBOs (COMPUTE/SHADER_READ — first touch, no
        // barrier needed on fresh frame-private slots) + writes the SHARED model-out ring
        // (COMPUTE/SHADER_WRITE — the dynamic slots). The COMPUTE→VERTEX barrier ordering this
        // write before the raster VS read is derived at the raster pass (the model_out reader),
        // NOT here.
        let interp = if let (Some(pairs), Some(out_slot), Some(model_out)) =
            (interp_pairs, interp_out_slot, interp_model_out)
        {
            let p = g.add_pass("interp");
            g.buffer_access(
                pairs,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                out_slot,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                model_out,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // HW-RT rung R2a-3: the TLAS pack + build passes — declared ONLY when `scene.tlas.is_some()`
        // (armed under hwrt + ray_query + count > 0), so the OFF path's ResId + barrier counts are
        // byte-unchanged. Run after interp, BEFORE the raster `begin_rendering` (the pack reads the
        // shared instance ring; the build reads the pack output).
        #[cfg(feature = "hwrt")]
        let (tlas_pack, tlas_build) = if scene.tlas.is_some() {
            // Pass `tlas_pack`: writes the `tlas_instances` array (COMPUTE/SHADER_WRITE); when the
            // interp pass ran it ALSO reads the shared `interp_model_out` ring the interp compute
            // wrote (COMPUTE/SHADER_READ — deriving the interp-WRITE → pack-READ RAW on the ring),
            // mirroring the raster pass's conditional ring read. When interp is OFF the ring is
            // host-CPU-scattered into host-coherent memory and the submit's host-write → device
            // domain dependency orders it (exactly as the raster VS reads the host-scattered ring),
            // so the pack declares ONLY its `tlas_instances` write.
            let pack = g.add_pass("tlas_pack");
            if let Some(model_out) = interp_model_out {
                g.buffer_access(
                    model_out,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                );
            }
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            // Pass `tlas_build`: reads the `tlas_instances` array at the AS-build stage — the graph
            // derives the pack(COMPUTE/SHADER_WRITE) → build(AS_BUILD/SHADER_READ) barrier. The build
            // writes the AS into the UNTRACKED backing/scratch (invisible to the graph), so ONLY this
            // instance-array read is declared. `VK_ACCESS_SHADER_READ_BIT & WRITE_ACCESS_MASK == 0`,
            // so `sync.rs` classifies it a READ with no `WRITE_ACCESS_MASK`/sync-engine change.
            let build = g.add_pass("tlas_build");
            g.buffer_access(
                tlas_instances,
                VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
                VK_ACCESS_SHADER_READ_BIT,
            );
            (Some(pack), Some(build))
        } else {
            (None, None)
        };

        // Pass `raster` (sites 0/1): the 3-MRT G-buffer + depth.
        let raster = g.add_pass("raster");
        // Pillar B B3 (refined-B): when the interp pass ran, the raster VS READS the SHARED
        // model-out ring the compute wrote — the graph derives the COMPUTE(WRITE)→VERTEX(READ)
        // RAW barrier here (the reader). The ring is consumed at the VERTEX stage (the raster +
        // shadow VS index `instances[...]`), so declare a VERTEX_SHADER/SHADER_READ access.
        // Declared ONLY when the interp pass exists, so the OFF path derives nothing.
        if let Some(model_out) = interp_model_out {
            g.buffer_access(
                model_out,
                VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        for &c in &[albedo, normal, material] {
            g.image_access(
                c,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                SubRange::COLOR,
            );
        }
        g.image_access(
            depth,
            FRAG,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            SubRange::DEPTH,
        );

        // Pass `light_upload` (async light-table re-upload) — gated exactly as the hand
        // site: `light_dirty && light_upload_bytes > 0`.
        let light_upload = if scene.light_dirty && scene.light_upload_bytes > 0 {
            let p = g.add_pass("light_upload");
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `coarse` (P0 coarse tile-cull) — gated `scene.coarse.is_some()`. Samples
        // depth (transitions it to SHADER_READ_ONLY) + writes the tiles buffer.
        let coarse = if scene.coarse.is_some() {
            let p = g.add_pass("coarse");
            g.image_access(
                depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.buffer_access(
                tiles,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `marcher` (sites 3/3b/4 collapsed): reads depth (→sampled if coarse did
        // not already) + tiles, read|writes the 3 attributes + gViewT (→GENERAL). Its
        // `record_pass` emits depth→sampled, color→general, and viewt's first-touch
        // UNDEFINED→GENERAL. (lit/ssao first-touch UNDEFINED→GENERAL are placed by the
        // graph at their own true first-use — resolve / ssao — a sound-superset re-order
        // of the hand path's eager site-(4) batch; see the module docs + equiv tests.)
        let marcher = g.add_pass("marcher");
        g.image_access(
            depth,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::DEPTH,
        );
        // The marcher READS the coarse tile bounds ONLY when the coarse pass ran (its
        // push gates the read off otherwise, and the hand path emits NO tiles barrier
        // when coarse is off). Declaring it unconditionally would derive a spurious
        // first-touch tiles barrier the hand path never records.
        if coarse.is_some() {
            g.buffer_access(
                tiles,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        for &c in &[albedo, normal, material, viewt] {
            g.image_access(
                c,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                RW,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }

        // Pass `ssao` — gated `scene.ssao.is_some()`. Reads normal/material/viewt (all
        // already SHADER_READ-visible from the marcher store→load), writes ssao.
        let ssao_pass = if scene.ssao.is_some() {
            let p = g.add_pass("ssao");
            for &c in &[normal, material, viewt] {
                g.image_access(
                    c,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::COLOR,
                );
            }
            g.image_access(
                ssao,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
            Some(p)
        } else {
            None
        };

        // Pass `ddgi_update` (SDFDDGI I2 probe update) — gated `scene.ddgi_update.is_some()`, i.e.
        // ONLY when `ResolvedDdgi::enabled()`. Recorded AFTER the marcher (edit-list warm) + AFTER
        // the L0 light-table copy (`LightBuf` COMPUTE-read-visible), BEFORE the resolve. Reads the
        // light table + the boot-static ray table (COMPUTE/SHADER_READ), read/writes the
        // classification (COMPUTE/RW), and WRITES the two atlas storage images (COMPUTE/SHADER_WRITE,
        // GENERAL) — the RDG derives the SHADER_READ_ONLY_OPTIMAL → GENERAL transition here (the
        // content-preserving seed layout, so the round-robin's un-updated tiles survive) and the
        // update-write → resolve-read GENERAL → SHADER_READ_ONLY_OPTIMAL barrier at the resolve (the
        // atlas reader), so NO `cmd_pipeline_barrier` is hand-written. The edit-list SSBO (`Buf` @0)
        // is NOT a graph resource (host-seeded once, like the marcher's read — no per-frame barrier),
        // so it is not named here.
        let ddgi_update = if scene.ddgi_update.is_some() {
            let p = g.add_pass("ddgi_update");
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                ddgi_ray_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(ddgi_classification, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            for &img in &[ddgi_irr, ddgi_depth] {
                g.image_access(
                    img,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_WRITE_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    SubRange::color_layers(crate::ddgi::DDGI_ATLAS_LAYERS),
                );
            }
            Some(p)
        } else {
            None
        };

        // Pass `light_cull` (L1 clustered cull) — gated exactly as the hand site: the
        // pipeline AND all three cluster buffers are `Some`. Resets alloc (transfer),
        // reads the table, writes grid/index.
        let light_cull = if scene.cluster_cull.is_some()
            && scene.cluster_grid.is_some()
            && scene.light_index.is_some()
            && scene.light_index_alloc.is_some()
        {
            let p = g.add_pass("light_cull");
            g.buffer_access(
                alloc,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            g.buffer_access(alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            g.buffer_access(
                index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
            Some(p)
        } else {
            None
        };

        // Pass `csm` (cascade depth) — gated `scene.csm.is_some()`. Layered depth over
        // `[0..active_count)` cascades, clamped `[1, MAX_CASCADES]` exactly as the hand
        // site. The barrier-out (→SHADER_READ_ONLY) is placed by the graph at the
        // resolve's cascade read (a sound-superset re-order; matches build_maximal_frame).
        let csm = if let Some(csm_act) = &scene.csm {
            let active = (csm_act.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            let p = g.add_pass("csm_depth");
            g.image_access(
                cascade,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(active),
            );
            Some(p)
        } else {
            None
        };

        // Pass `atlas` (spot/point atlas depth) — gated `scene.atlas_punctual.is_some()`.
        // Layered depth over `[0..active_layers)`, clamped `[1, MAX_TEXTURE_LAYERS]`.
        let atlas_pass = if let Some(atlas_act) = &scene.atlas_punctual {
            let active =
                (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            let p = g.add_pass("atlas_depth");
            g.image_access(
                atlas,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::depth_layers(active),
            );
            Some(p)
        } else {
            None
        };

        // Pass `resolve`: reads albedo/normal/material/viewt/ssao (GENERAL) + the L1
        // buffers + the layered shadow maps (→sampled), writes lit (first-touch
        // UNDEFINED→GENERAL). The optional reads are declared ONLY when their producer
        // pass ran, so the graph derives their →sampled / →read barriers exactly when the
        // hand path does. Declaring them conditionally keeps a resource the frame never
        // touched (e.g. cascade when CSM is off) out of the compiled barrier set.
        let resolve = g.add_pass("resolve");
        for &c in &[albedo, normal, material, viewt] {
            g.image_access(
                c,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        if ssao_pass.is_some() {
            g.image_access(
                ssao,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        if light_cull.is_some() {
            g.buffer_access(
                grid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            g.buffer_access(
                index,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        // The resolve reads the light table whenever it was uploaded this frame (the
        // hand path's marcher/resolve read the table after the L0-r0 copy). The graph
        // only needs a barrier here if a producer left a pending flush; declaring the
        // read is harmless (free when already visible) but we mirror the hand site: the
        // table's producer is the light_upload copy (TRANSFER_WRITE), consumed at COMPUTE.
        if light_upload.is_some() {
            g.buffer_access(
                light_table,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
        }
        if csm.is_some() {
            let active =
                (scene.csm.as_ref().map(|c| c.active_count).unwrap_or(1) as usize)
                    .clamp(1, MAX_CASCADES) as u32;
            g.image_access(
                cascade,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(active),
            );
        }
        if atlas_pass.is_some() {
            let active = (scene
                .atlas_punctual
                .as_ref()
                .map(|a| a.active_layers)
                .unwrap_or(1) as usize)
                .clamp(1, MAX_TEXTURE_LAYERS) as u32;
            g.image_access(
                atlas,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::depth_layers(active),
            );
        }
        // SDFDDGI I2: when the update pass ran, the resolve READS the two atlas storage images the
        // update WROTE. Declaring the read here (the atlas READER) is what makes the RDG DERIVE the
        // update-write → resolve-read barrier. The resolve SAMPLES the atlases through a
        // COMBINED_IMAGE_SAMPLER, so it reads them at SHADER_READ_ONLY_OPTIMAL — deriving a
        // GENERAL → SHADER_READ_ONLY_OPTIMAL, COMPUTE→COMPUTE, SHADER_WRITE→SHADER_READ transition
        // over all `DDGI_ATLAS_LAYERS` layers. This is the CONTENT-PRESERVING CYCLE that keeps the
        // persistent accumulator consistent across frames: boot SHADER_READ_ONLY_OPTIMAL → update
        // GENERAL (write) → resolve SHADER_READ_ONLY_OPTIMAL (sample), so the image ENDS each frame
        // at SHADER_READ_ONLY_OPTIMAL == the cross-frame seed layout (`seeded_readers_at_layout`
        // above) — no layout desync on the next frame's update write. Declared ONLY when
        // `ddgi_update.is_some()`, so the OFF path derives nothing (byte-identical).
        if ddgi_update.is_some() {
            for &img in &[ddgi_irr, ddgi_depth] {
                g.image_access(
                    img,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::color_layers(crate::ddgi::DDGI_ATLAS_LAYERS),
                );
            }
        }
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );

        // Pass `present_sample` (site 5c): lit GENERAL→SHADER_READ_ONLY for the
        // present-blit's FRAGMENT sample. The swapchain WSI barriers (sites 7/9) stay
        // hand-recorded, so the swapchain image is NOT a graph resource here.
        let present_sample = g.add_pass("present_sample");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );

        g.compile();

        self.gbuffer_pass_plan = Some(GbufferPassPlan {
            interp,
            raster,
            light_upload,
            coarse,
            marcher,
            ssao: ssao_pass,
            ddgi_update,
            light_cull,
            csm,
            atlas: atlas_pass,
            #[cfg(feature = "hwrt")]
            tlas_pack,
            #[cfg(feature = "hwrt")]
            tlas_build,
            resolve,
            present_sample,
        });
    }

    /// Steps 1d/1e: drive one declared pass's derived barriers (Step 1c's
    /// [`GbufferBarrierSink`], now whole-frame) into the open `cmd`. Builds the sink
    /// resolving the graph's FIXED ResIds → the current frame slot's physical
    /// `VkImage`/`VkBuffer` handles, then calls `record_pass`, which emits the minimum
    /// number of batched sync1 `vkCmdPipelineBarrier` calls for `pass`.
    ///
    /// An optional buffer/image the frame did not wire resolves to a NULL handle: the
    /// declaration never routed a barrier naming its ResId, so the NULL is never handed
    /// to the driver (a pass records ONLY the barriers derived from ITS declared
    /// accesses).
    pub(crate) fn record_graph_pass(
        &self,
        pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        scene: &GBufferScene<'_>,
        fi: usize,
    ) {
        let mut sink = GbufferBarrierSink {
            fns: self.fns,
            cmd,
            images: [
                targets.albedo[fi].image,
                targets.normal[fi].image,
                targets.material[fi].image,
                targets.depth[fi].image,
                targets.viewt[fi].image,
                targets.lit[fi].image,
                targets.ssao[fi].image,
                // cascade / atlas are single-instance (NOT ringed) world-fixed maps.
                scene.csm_cascade_texture.image,
                scene.shadow_atlas_texture.image,
                // SDFDDGI I2 (ResIds 9/10): the two DDGI atlas storage images — single-instance
                // (NOT ringed) world-fixed atlases, the SAME textures the resolve set samples. Only
                // named by a derived barrier on the `ddgi_update` ON path; on the OFF path they are
                // unreferenced (their slots inert, like cascade/atlas when those maps are off).
                scene.ddgi_irr_texture.image,
                scene.ddgi_depth_texture.image,
            ],
            #[cfg(not(feature = "hwrt"))]
            buffers: [
                scene.light_table.buffer,
                scene.tiles_buffer.buffer,
                scene.cluster_grid.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index_alloc.map_or(VkBuffer::NULL, |b| b.buffer),
                // SDFDDGI I2 (ResIds 16/17 → slots 5/6): the single-instance classification +
                // Fibonacci ray-table buffers, ALWAYS resolved (declared unconditionally in the
                // graph). Named by a derived barrier ONLY on the `ddgi_update` ON path.
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                // Pillar B B3 (ResIds 18/19/20 → slots 7/8/9, refined-B): the CURRENT frame slot's
                // FIF-ringed interp SSBOs, NULL on the interp-OFF path (never named by a derived
                // barrier there). On the ON path the ONLY derived barrier is the COMPUTE→VERTEX RAW on
                // `interp_model_out` (the shared instance ring) at the raster pass; `interp_pairs`
                // and `interp_out_slot` are declared but never barriered (first-touch reads).
                scene.interp.map_or(VkBuffer::NULL, |a| a.pair_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.out_slot_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.model_out_buffer.buffer),
            ],
            // HW-RT rung R2a-3 (RISK-2): `tlas_instances` is declared UNCONDITIONALLY right after
            // the DDGI buffers (ResId 18 → slot 7), so its slot is FIXED regardless of the
            // conditional interp trio (which shifts to ResIds 19/20/21 → slots 8/9/10). NULL on the
            // tlas-OFF path (never named by a derived barrier there); on the ON path the pack write
            // → build read barrier is derived on this slot.
            #[cfg(feature = "hwrt")]
            buffers: [
                scene.light_table.buffer,
                scene.tiles_buffer.buffer,
                scene.cluster_grid.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.light_index_alloc.map_or(VkBuffer::NULL, |b| b.buffer),
                scene.ddgi_classification.buffer,
                scene.ddgi_ray_table.buffer,
                scene.tlas.map_or(VkBuffer::NULL, |t| t.instance_array.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.pair_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.out_slot_buffer.buffer),
                scene.interp.map_or(VkBuffer::NULL, |a| a.model_out_buffer.buffer),
            ],
        };
        self.frame_graph.record_pass(pass, &mut sink);
    }
}
