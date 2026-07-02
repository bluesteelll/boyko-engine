//! The frame graph itself: SoA arenas for resources / passes / accesses, the
//! declarative build API, and the linear-order barrier-derivation compile.
//!
//! # Substrate (resolves critic C4)
//!
//! The arenas are **build-time preallocated `Vec`s** — the exact same accepted
//! Principle-0 exception as `boyko_render::barrier`'s build-time Vec and the
//! threadpool deques (transient GPU-orchestration scratch, not a durable
//! per-entity store). Allocated ONCE via [`FrameGraph::with_capacity`]; every
//! frame calls [`FrameGraph::reset`] (`Vec::clear`, capacity retained) then re-
//! declares — so the per-frame path performs **zero heap allocation** as long as
//! the caps are not exceeded (debug-asserted). This sidesteps making
//! `boyko_ecs`'s `pub(crate)` `VmReservation` public; a single-reservation
//! co-location is a drop-in behind this identical index API if profiling ever
//! wants it.
//!
//! # Ordering (linear, by design)
//!
//! The in-house G-buffer frame is a straight **line** of passes (raster → cull →
//! marcher → ssao → light-cull → shadow depth → resolve → present), authored in
//! dependency order. A topological sort of a line is the identity, so Step 1b
//! compiles in **declaration order** — provably optimal for a linear frame. The
//! alloc-free `u16` topo/SCC scaffolding (plan C4) is deferred until a genuinely
//! branching pass graph exists (YAGNI / anti-speculation, Principle 0); the
//! per-resource sync state machine — the actual industrial win — is fully here.

use super::ids::{PassId, ResId};
use super::sync::{transition, BufBarrier, ImgBarrier, ResSync, SubRange};
use crate::ffi::VK_IMAGE_LAYOUT_UNDEFINED;

/// The per-pass slice into the flat derived-barrier arenas: the image + buffer
/// barriers that must be recorded (as batched `vkCmdPipelineBarrier` array calls)
/// immediately BEFORE the pass's GPU work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PassBarrierRange {
    pub img_begin: u32,
    pub img_count: u32,
    pub buf_begin: u32,
    pub buf_count: u32,
}

/// A declarative render-dependency graph. Declare resources + passes + their
/// accesses, call [`compile`](FrameGraph::compile), then read the derived
/// per-pass barrier plan. Rebuilt every frame; [`reset`](FrameGraph::reset)
/// retains capacity for zero per-frame allocation.
pub struct FrameGraph {
    // --- resource arena (SoA) ---
    res_is_image: Vec<bool>,
    res_name: Vec<&'static str>,

    // --- pass arena (SoA) ---
    pass_name: Vec<&'static str>,
    pass_access_begin: Vec<u32>,
    pass_access_count: Vec<u32>,

    // --- access arena (SoA), flat; each pass owns a [begin, begin+count) slice ---
    acc_res: Vec<ResId>,
    acc_stage: Vec<u32>,
    acc_access: Vec<u32>,
    acc_layout: Vec<i32>,
    acc_sub: Vec<SubRange>,

    // --- compile scratch (reused; preallocated to res cap) ---
    state: Vec<ResSync>,

    // --- compile output ---
    img_barriers: Vec<ImgBarrier>,
    buf_barriers: Vec<BufBarrier>,
    pass_barriers: Vec<PassBarrierRange>,
}

impl FrameGraph {
    /// Preallocate all arenas ONCE. `max_acc` bounds the total accesses across
    /// all passes in a frame. Sized generously (the frame declares tens of each);
    /// exceeding a cap in `reset`-then-declare only regrows the `Vec` (cold), it
    /// is never a correctness issue.
    pub fn with_capacity(max_res: usize, max_pass: usize, max_acc: usize) -> Self {
        Self {
            res_is_image: Vec::with_capacity(max_res),
            res_name: Vec::with_capacity(max_res),
            pass_name: Vec::with_capacity(max_pass),
            pass_access_begin: Vec::with_capacity(max_pass),
            pass_access_count: Vec::with_capacity(max_pass),
            acc_res: Vec::with_capacity(max_acc),
            acc_stage: Vec::with_capacity(max_acc),
            acc_access: Vec::with_capacity(max_acc),
            acc_layout: Vec::with_capacity(max_acc),
            acc_sub: Vec::with_capacity(max_acc),
            state: Vec::with_capacity(max_res),
            img_barriers: Vec::with_capacity(max_acc),
            buf_barriers: Vec::with_capacity(max_acc),
            pass_barriers: Vec::with_capacity(max_pass),
        }
    }

    /// Clear all arenas for a fresh frame, RETAINING capacity (no dealloc). The
    /// per-frame build path then re-declares with zero heap allocation.
    pub fn reset(&mut self) {
        self.res_is_image.clear();
        self.res_name.clear();
        self.pass_name.clear();
        self.pass_access_begin.clear();
        self.pass_access_count.clear();
        self.acc_res.clear();
        self.acc_stage.clear();
        self.acc_access.clear();
        self.acc_layout.clear();
        self.acc_sub.clear();
        self.state.clear();
        self.img_barriers.clear();
        self.buf_barriers.clear();
        self.pass_barriers.clear();
    }

    /// Declare a transient/history IMAGE resource (layout starts UNDEFINED).
    #[inline]
    pub fn add_image(&mut self, name: &'static str) -> ResId {
        debug_assert!(
            self.res_is_image.len() < u16::MAX as usize,
            "framegraph resource count exceeds u16 index space"
        );
        let id = ResId(self.res_is_image.len() as u16);
        self.res_is_image.push(true);
        self.res_name.push(name);
        id
    }

    /// Declare a BUFFER resource (no layout; ordering is flush/visibility only).
    #[inline]
    pub fn add_buffer(&mut self, name: &'static str) -> ResId {
        debug_assert!(
            self.res_is_image.len() < u16::MAX as usize,
            "framegraph resource count exceeds u16 index space"
        );
        let id = ResId(self.res_is_image.len() as u16);
        self.res_is_image.push(false);
        self.res_name.push(name);
        id
    }

    /// Begin a new pass in declaration (execution) order; subsequent
    /// [`image_access`](FrameGraph::image_access) /
    /// [`buffer_access`](FrameGraph::buffer_access) calls attach to it until the
    /// next `add_pass`.
    #[inline]
    pub fn add_pass(&mut self, name: &'static str) -> PassId {
        debug_assert!(
            self.pass_name.len() < u16::MAX as usize,
            "framegraph pass count exceeds u16 index space"
        );
        let id = PassId(self.pass_name.len() as u16);
        self.pass_name.push(name);
        self.pass_access_begin.push(self.acc_res.len() as u32);
        self.pass_access_count.push(0);
        id
    }

    /// Record that the current pass touches IMAGE `res` at `(stage, access,
    /// layout)` over `sub`. `access` may combine read + write bits (e.g. the
    /// marcher's `SHADER_READ | SHADER_WRITE` on a G-buffer attribute).
    #[inline]
    pub fn image_access(&mut self, res: ResId, stage: u32, access: u32, layout: i32, sub: SubRange) {
        debug_assert!(
            self.res_is_image[res.index()],
            "image_access on a buffer resource '{}'",
            self.res_name[res.index()]
        );
        self.push_access(res, stage, access, layout, sub);
    }

    /// Record that the current pass touches BUFFER `res` at `(stage, access)`.
    #[inline]
    pub fn buffer_access(&mut self, res: ResId, stage: u32, access: u32) {
        debug_assert!(
            !self.res_is_image[res.index()],
            "buffer_access on an image resource '{}'",
            self.res_name[res.index()]
        );
        // Layout/subresource are unused for buffers; the UNDEFINED sentinel keeps
        // the layout arm quiet in `transition`.
        self.push_access(res, stage, access, VK_IMAGE_LAYOUT_UNDEFINED, SubRange::COLOR);
    }

    fn push_access(&mut self, res: ResId, stage: u32, access: u32, layout: i32, sub: SubRange) {
        debug_assert!(
            !self.pass_name.is_empty(),
            "access declared before any add_pass"
        );
        self.acc_res.push(res);
        self.acc_stage.push(stage);
        self.acc_access.push(access);
        self.acc_layout.push(layout);
        self.acc_sub.push(sub);
        // `expect` (not a bare `- 1`) so a misuse (access before add_pass) surfaces
        // loudly even in release, where the debug_assert above is compiled out.
        let last = self
            .pass_access_count
            .len()
            .checked_sub(1)
            .expect("invariant: access declared before any add_pass");
        self.pass_access_count[last] += 1;
    }

    /// Derive the minimal per-pass barrier plan from the declared accesses,
    /// walking passes in declaration (execution) order and running the Granite
    /// per-resource sync state machine. Idempotent given the same declarations.
    pub fn compile(&mut self) {
        self.img_barriers.clear();
        self.buf_barriers.clear();
        self.pass_barriers.clear();

        // Fresh per-resource state: every transient/history resource starts
        // UNDEFINED (re-`UNDEFINED`'d each frame — prior content discarded).
        self.state.clear();
        self.state
            .resize(self.res_is_image.len(), ResSync::undefined());

        for p in 0..self.pass_name.len() {
            let img_begin = self.img_barriers.len() as u32;
            let buf_begin = self.buf_barriers.len() as u32;

            let begin = self.pass_access_begin[p] as usize;
            let count = self.pass_access_count[p] as usize;
            for a in begin..begin + count {
                let res = self.acc_res[a];
                let stage = self.acc_stage[a];
                let access = self.acc_access[a];
                let layout = self.acc_layout[a];
                let sub = self.acc_sub[a];
                let ri = res.index();
                let is_image = self.res_is_image[ri];

                // Split-borrow: read the access scalars above, mutate state here,
                // release the borrow before pushing into the barrier arenas.
                let trans = {
                    let st = &mut self.state[ri];
                    // Buffers: pass the current (sentinel) layout so the layout
                    // arm never fires.
                    let want_layout = if is_image { layout } else { st.layout };
                    transition(st, stage, access, want_layout)
                };

                if let Some(t) = trans {
                    if is_image {
                        self.img_barriers.push(ImgBarrier {
                            res,
                            src_stage: t.src_stage,
                            dst_stage: t.dst_stage,
                            src_access: t.src_access,
                            dst_access: t.dst_access,
                            old_layout: t.old_layout,
                            new_layout: t.new_layout,
                            subresource: sub,
                        });
                    } else {
                        self.buf_barriers.push(BufBarrier {
                            res,
                            src_stage: t.src_stage,
                            dst_stage: t.dst_stage,
                            src_access: t.src_access,
                            dst_access: t.dst_access,
                        });
                    }
                }
            }

            self.pass_barriers.push(PassBarrierRange {
                img_begin,
                img_count: self.img_barriers.len() as u32 - img_begin,
                buf_begin,
                buf_count: self.buf_barriers.len() as u32 - buf_begin,
            });
        }
    }

    // --- read-back accessors (for the record step + the equivalence tests) ---

    /// All derived image barriers, in emission order.
    #[inline]
    pub fn img_barriers(&self) -> &[ImgBarrier] {
        &self.img_barriers
    }

    /// All derived buffer barriers, in emission order.
    #[inline]
    pub fn buf_barriers(&self) -> &[BufBarrier] {
        &self.buf_barriers
    }

    /// The per-pass barrier plan (one entry per declared pass, in order).
    #[inline]
    pub fn pass_barriers(&self) -> &[PassBarrierRange] {
        &self.pass_barriers
    }

    /// The debug name of a resource (for diagnostics + test assertions).
    #[inline]
    pub fn res_name(&self, res: ResId) -> &'static str {
        self.res_name[res.index()]
    }

    /// The GROUND-TRUTH final layout a resource reaches after `compile` — read
    /// directly from the resource's running sync state (NOT reconstructed from the
    /// barrier list, which would miss a final layout-preserving free access and
    /// report a stale layout). Call after `compile`; panics if called before (the
    /// state vector is only populated by `compile`).
    #[inline]
    pub fn resolved_layout(&self, res: ResId) -> i32 {
        debug_assert!(
            res.index() < self.state.len(),
            "resolved_layout called before compile (or out-of-range resource)"
        );
        self.state[res.index()].layout
    }
}
