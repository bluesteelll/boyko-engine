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
// Read only by the `#[cfg(debug_assertions)]` read-has-producer check in `compile`, so a
// release build would see this import as unused.
#[cfg(debug_assertions)]
use super::sync::WRITE_ACCESS_MASK;
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

/// The DEBUG-ONLY per-resource accumulator behind `INVARIANT HZB-SUBRESOURCE-UNIFORM`
/// (documented at its check site in [`FrameGraph::compile`]).
///
/// It records the resource's FIRST declared `(SubRange, layout)` plus one bit per axis
/// saying that some later access disagreed with it. Two bits suffice because "some two
/// accesses differ" is equivalent to "some access differs from the first" — if every
/// access equals the first, no pair differs; if any differs from the first, that pair
/// does. So the invariant is decidable in one linear pass, with no per-resource set.
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct SubWitness {
    /// The resource's first declared subresource range — the span every later access to it
    /// must match, per INVARIANT HZB-SUBRESOURCE-UNIFORM.
    first_sub: SubRange,
    /// Some later access declared a different mip/layer SPAN than `first_sub`.
    ///
    /// Latched rather than compared pairwise on the spot so the assert can name BOTH the
    /// offending span and the one it should have matched.
    span_varied: bool,
}

/// A declarative render-dependency graph. Declare resources + passes + their
/// accesses, call [`compile`](FrameGraph::compile), then read the derived
/// per-pass barrier plan. Rebuilt every frame; [`reset`](FrameGraph::reset)
/// retains capacity for zero per-frame allocation.
pub struct FrameGraph {
    // --- resource arena (SoA) ---
    res_is_image: Vec<bool>,
    res_name: Vec<&'static str>,
    /// The per-resource sync state `compile` STARTS from. `ResSync::undefined()`
    /// for ringed/transient resources (prior content discarded, the per-slot fence
    /// already orders slot reuse). For a NON-RINGED resource shared by both
    /// in-flight frames (light table, CSM cascade, shadow atlas) the declare site
    /// seeds `visible_stages/visible_access` with the resource's steady-state
    /// consumer scopes, so this frame's FIRST write derives a WAR execution
    /// dependency on the SIBLING frame's still-pipelined reads instead of a
    /// no-op `TOP_OF_PIPE` src — the cross-frame torn-read fix (audit B-002/B-003).
    res_seed: Vec<ResSync>,
    /// `true` iff the resource was declared via
    /// [`add_image_seeded`](FrameGraph::add_image_seeded) /
    /// [`add_buffer_seeded`](FrameGraph::add_buffer_seeded) — content is
    /// intentionally cross-frame, so `compile`'s DEBUG-ONLY unwritten-transient-
    /// read guard exempts it. Populated at declare time in every profile
    /// (negligible once-per-resource-per-frame cost, matching `res_seed`/
    /// `res_name`); only ever READ under `cfg(debug_assertions)`.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    res_seeded: Vec<bool>,

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
    /// DEBUG-ONLY per-resource "written-or-seeded" bit for `compile`'s authoring
    /// guard: a non-seeded transient IMAGE must be written by a prior pass before
    /// its first read, or a mis-authored pass silently derives a hazard-free
    /// `TOP_OF_PIPE` barrier instead of a caught bug (see `compile`). Cleared to
    /// `res_seeded` every compile; entirely compiled out in release — `compile`
    /// runs every frame, so the tracking must cost nothing there (Principle 1/7).
    #[cfg(debug_assertions)]
    res_written: Vec<bool>,
    /// DEBUG-ONLY per-resource witness for `INVARIANT HZB-SUBRESOURCE-UNIFORM` (stated in
    /// full at its check site in `compile`): the first declared `(SubRange, layout)` of each
    /// resource plus the two "varied since" bits the invariant is decided from. `None` until
    /// a resource's first access; refilled with `None` every compile, mirroring how
    /// `res_written` is refilled from `res_seeded`. Entirely compiled out in release —
    /// `compile` runs every frame, so the tracking must cost nothing there (Principle 1/7).
    #[cfg(debug_assertions)]
    res_sub_witness: Vec<Option<SubWitness>>,

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
            res_seed: Vec::with_capacity(max_res),
            res_seeded: Vec::with_capacity(max_res),
            pass_name: Vec::with_capacity(max_pass),
            pass_access_begin: Vec::with_capacity(max_pass),
            pass_access_count: Vec::with_capacity(max_pass),
            acc_res: Vec::with_capacity(max_acc),
            acc_stage: Vec::with_capacity(max_acc),
            acc_access: Vec::with_capacity(max_acc),
            acc_layout: Vec::with_capacity(max_acc),
            acc_sub: Vec::with_capacity(max_acc),
            state: Vec::with_capacity(max_res),
            #[cfg(debug_assertions)]
            res_written: Vec::with_capacity(max_res),
            #[cfg(debug_assertions)]
            res_sub_witness: Vec::with_capacity(max_res),
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
        self.res_seed.clear();
        self.res_seeded.clear();
        self.pass_name.clear();
        self.pass_access_begin.clear();
        self.pass_access_count.clear();
        self.acc_res.clear();
        self.acc_stage.clear();
        self.acc_access.clear();
        self.acc_layout.clear();
        self.acc_sub.clear();
        self.state.clear();
        #[cfg(debug_assertions)]
        self.res_written.clear();
        #[cfg(debug_assertions)]
        self.res_sub_witness.clear();
        self.img_barriers.clear();
        self.buf_barriers.clear();
        self.pass_barriers.clear();
    }

    /// Declare a transient/history IMAGE resource (layout starts UNDEFINED).
    #[inline]
    pub fn add_image(&mut self, name: &'static str) -> ResId {
        self.push_res(true, name, ResSync::undefined(), false)
    }

    /// Declare a BUFFER resource (no layout; ordering is flush/visibility only).
    #[inline]
    pub fn add_buffer(&mut self, name: &'static str) -> ResId {
        self.push_res(false, name, ResSync::undefined(), false)
    }

    /// Declare a NON-RINGED IMAGE shared by both in-flight frames (CSM cascade,
    /// shadow atlas), seeding its start-of-frame sync state with the sibling
    /// frame's end-of-frame scopes (see [`ResSync::seeded_readers`] /
    /// [`ResSync::seeded_writer`]). The first write this frame then orders after
    /// the sibling's still-pipelined accesses instead of the hazard-free
    /// `TOP_OF_PIPE` a fresh `undefined()` state yields (audit B-002/B-003).
    #[inline]
    pub fn add_image_seeded(&mut self, name: &'static str, seed: ResSync) -> ResId {
        self.push_res(true, name, seed, true)
    }

    /// Declare a NON-RINGED BUFFER shared by both in-flight frames (light table,
    /// tiles, cluster grid/index/alloc): same cross-frame seeding as
    /// [`add_image_seeded`](FrameGraph::add_image_seeded) (buffers have no
    /// layout; the seed only strengthens the first access's src scope).
    #[inline]
    pub fn add_buffer_seeded(&mut self, name: &'static str, seed: ResSync) -> ResId {
        self.push_res(false, name, seed, true)
    }

    #[inline]
    fn push_res(&mut self, is_image: bool, name: &'static str, seed: ResSync, seeded: bool) -> ResId {
        debug_assert!(
            self.res_is_image.len() < u16::MAX as usize,
            "framegraph resource count exceeds u16 index space"
        );
        let id = ResId(self.res_is_image.len() as u16);
        self.res_is_image.push(is_image);
        self.res_name.push(name);
        self.res_seed.push(seed);
        self.res_seeded.push(seeded);
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

        // Per-resource start state: ringed/transient resources start UNDEFINED
        // (re-`UNDEFINED`'d each frame — prior content discarded); NON-RINGED
        // shared resources start from their declared cross-frame seed (visible
        // consumer scopes), so their first write orders after the sibling
        // in-flight frame's reads (see `add_image_seeded`).
        self.state.clear();
        self.state.extend_from_slice(&self.res_seed);
        // DEBUG-ONLY authoring-guard scratch: starts from the declare-time seeded
        // bit (a seeded resource is exempt everywhere), then latches `true` at
        // each write encountered below. Entirely compiled out in release.
        #[cfg(debug_assertions)]
        {
            self.res_written.clear();
            self.res_written.extend_from_slice(&self.res_seeded);
            // No declare-time seed exists for the subresource witness: it is derived purely
            // from this compile's accesses, so every resource starts unwitnessed.
            self.res_sub_witness.clear();
            self.res_sub_witness.resize(self.res_is_image.len(), None);
        }

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

                // DEBUG-ONLY authoring guard (release-neutral): a non-seeded
                // transient IMAGE must be written by a prior pass before its first
                // read, or a mis-authored pass silently derives a hazard-free
                // `TOP_OF_PIPE` barrier below instead of surfacing as a caught bug.
                // Ringed/seeded resources (`res_seeded`) are exempt — cross-frame
                // content is intentional for them (the shadow-temporal history pool,
                // the DDGI atlas, CSM cascade / shadow atlas, the shared buffers).
                #[cfg(debug_assertions)]
                {
                    let is_write = access & WRITE_ACCESS_MASK != 0;
                    debug_assert!(
                        !is_image || is_write || self.res_written[ri],
                        "framegraph: pass '{}' reads transient image '{}' with no prior \
                         producer or seed (add_image_seeded) — this would silently derive \
                         a hazard-free TOP_OF_PIPE barrier",
                        self.pass_name[p],
                        self.res_name[ri],
                    );
                    if is_write {
                        self.res_written[ri] = true;
                    }

                    // === INVARIANT HZB-SUBRESOURCE-UNIFORM (debug-only, release-neutral) ===
                    //
                    // EVERY access to one `ResId` must declare the SAME subresource span —
                    // the same `(base_mip, mip_count, base_layer, layer_count)`. Aspect is
                    // excluded: it is a property of the image's format, not a selection.
                    //
                    // WHY THE SPAN ALONE, AND NOT "SPAN *AND* LAYOUT BOTH VARY". The weaker
                    // two-axis condition is the one this guard was first written with, and it
                    // is UNSOUND. A uniform DECLARED layout does not give a uniform ACTUAL
                    // layout, because only the union of the spans that have actually appeared
                    // in an emitted barrier has been transitioned — every other subresource is
                    // still in the image's start layout. Concretely, and this passed the weaker
                    // guard silently:
                    //
                    //     let pyr = g.add_image("pyr");            // starts UNDEFINED
                    //     pass A: SHADER_WRITE, GENERAL, SubRange::COLOR       // mips [0,1)
                    //     pass B: SHADER_READ,  GENERAL, color_mips(4)         // mips [0,4)
                    //
                    // A is a first touch, so its barrier transitions mips [0,1) only. B needs
                    // no layout change but does need a flush, so `transition` returns a barrier
                    // over [0,4) claiming `oldLayout = GENERAL` — for mips 1..3, which were
                    // never transitioned and are still UNDEFINED
                    // (VUID-VkImageMemoryBarrier-oldLayout-01197). Undefined, and invisible to
                    // the validation layers, which see a well-formed barrier and cannot follow
                    // its provenance back to the state machine that invented the `oldLayout`.
                    //
                    // Note the hazard is ORDER-DEPENDENT: declaring the superset FIRST is fine,
                    // the subset first is UB. A guard whose verdict depends on declaration order
                    // is not a guard, which is the second reason the two-axis form is rejected
                    // rather than merely tightened.
                    //
                    // This condition is therefore an OVER-APPROXIMATION on purpose: some
                    // varying-span declarations are safe (superset-first ones), and the guard
                    // fires on them anyway. A fire means "this declaration is outside the region
                    // this state machine can prove safe", NOT "this declaration is broken".
                    //
                    // PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this assert
                    // is its TRIGGER. Keying `state` by `(ResId, mip, layer)` is what lifts the
                    // restriction, and it is what a mip pyramid wants: the HZB build writes mip
                    // k while reading mip k-1. When that pass is authored, it trips this assert.
                    // That is the INTENDED way to discover the work — a mechanical, unmissable
                    // notice at the moment the first declaration needs it. The response is to
                    // build per-subresource tracking, never to relax the condition until it
                    // goes quiet.
                    //
                    // WHY RELEASE IS UNGUARDED. Not "the declaration surface is compile-time
                    // fixed" — WHICH accesses run is heavily data-conditional here (leg and
                    // path predicates gate whole pass families). What IS compile-time fixed is
                    // the span argument at every `image_access` site: they are literal
                    // `SubRange` constructors, so a debug run that REACHES a pass settles that
                    // pass's spans for the release build of the same source. CI runs its tests
                    // as a debug x release matrix, so the debug leg reaches everything the
                    // release leg does. Paying for the check in the release frame path would
                    // buy no information the debug leg did not already have, on a `compile`
                    // that runs every frame (Principle 1/7). What this does NOT cover is a pass
                    // no debug run ever reaches; covering that is the gate's problem, not this
                    // assert's.
                    //
                    // Buffers are structurally exempt without a branch: `buffer_access` always
                    // passes `SubRange::COLOR`, so their span is uniform by construction.

                    // `SubWitness` is `Copy`: read it out, fold this access in, write it
                    // back — no borrow of `self` outlives the update, so the assert below
                    // can still name the pass and resource.
                    let witness = match self.res_sub_witness[ri] {
                        Some(mut w) => {
                            w.span_varied |= !sub.same_span(&w.first_sub);
                            w
                        }
                        None => SubWitness { first_sub: sub, span_varied: false },
                    };
                    self.res_sub_witness[ri] = Some(witness);
                    debug_assert!(
                        !witness.span_varied,
                        "framegraph: INVARIANT HZB-SUBRESOURCE-UNIFORM violated at pass '{}' \
                         on resource '{}' — its accesses declare DIFFERENT subresource spans \
                         (this access: {:?}; first declared: {:?}), and this state machine \
                         tracks ONE layout per ResId, so a subresource that never appeared in \
                         an emitted barrier is still in the image's start layout while a later \
                         barrier claims otherwise. Fix by giving this resource per-subresource \
                         sync state, not by making the declarations agree by hand",
                        self.pass_name[p],
                        self.res_name[ri],
                        sub,
                        witness.first_sub,
                    );
                }

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

    /// **FROZEN REFERENCE — a verbatim copy of [`compile`](FrameGraph::compile) as it stood at
    /// VG R3 P1-5a commit C1**, i.e. the per-`ResId` sync state machine BEFORE the state was
    /// re-keyed to `(ResId, mip)`.
    ///
    /// It writes into the same three output arenas as `compile` (`img_barriers`,
    /// `buf_barriers`, `pass_barriers`), so a differential can run one, snapshot, run the
    /// other, and compare the streams element for element.
    ///
    /// # Its whole purpose
    ///
    /// To be DIFFED against the new `compile` over the SINGLE-MIP regime — every declaration
    /// that exists today, where each resource's accesses all name one mip. Over that regime
    /// the two must agree EXACTLY: `(ResId, mip)` keying collapses to `ResId` keying when
    /// there is only ever one mip, and P1-5a's claim is precisely that nothing existing moves.
    /// The differential is what makes that claim testable rather than asserted, because the
    /// baseline it compares against is code that cannot have been edited to agree.
    ///
    /// # NEVER PATCHED
    ///
    /// This body is not maintained. If a later, legitimate change to `compile` makes the
    /// differential fail, **the DIFFERENTIAL is deleted, not this function** — the moment
    /// someone "fixes" the reference to match the new behaviour, it stops being a record of
    /// the old one and the differential starts asserting that the code equals itself. A
    /// frozen copy that drifts is worse than no copy, because it still looks like evidence.
    ///
    /// # Deletion condition
    ///
    /// Delete this function together with its differential at **the end of piece 1** of
    /// P1-5a — once the re-keyed `compile` has been proved equivalent over the single-mip
    /// regime and the first genuinely multi-mip declaration lands, at which point the two
    /// machines are no longer expected to agree and the reference has said everything it can.
    ///
    /// `pub` rather than `pub(crate)`: it has no caller until the differential lands, and only
    /// a `pub` method on a `pub` type is seeded as reachable by the dead-code pass, so this
    /// spelling is what keeps the interim commit warning-clean without an `#[allow]`. The
    /// precedent is in this crate: `goldens::golden_deferred_resolve_clustered_shadowed` is a
    /// `pub fn` in a `#[cfg(any(test, feature = "goldens"))]` module whose only occurrence in
    /// the workspace is its own definition, and the `--all-targets -D warnings` gate is green.
    #[cfg(test)]
    pub fn compile_per_resource_reference(&mut self) {
        self.img_barriers.clear();
        self.buf_barriers.clear();
        self.pass_barriers.clear();

        // Per-resource start state: ringed/transient resources start UNDEFINED
        // (re-`UNDEFINED`'d each frame — prior content discarded); NON-RINGED
        // shared resources start from their declared cross-frame seed (visible
        // consumer scopes), so their first write orders after the sibling
        // in-flight frame's reads (see `add_image_seeded`).
        self.state.clear();
        self.state.extend_from_slice(&self.res_seed);
        // DEBUG-ONLY authoring-guard scratch: starts from the declare-time seeded
        // bit (a seeded resource is exempt everywhere), then latches `true` at
        // each write encountered below. Entirely compiled out in release.
        #[cfg(debug_assertions)]
        {
            self.res_written.clear();
            self.res_written.extend_from_slice(&self.res_seeded);
            // No declare-time seed exists for the subresource witness: it is derived purely
            // from this compile's accesses, so every resource starts unwitnessed.
            self.res_sub_witness.clear();
            self.res_sub_witness.resize(self.res_is_image.len(), None);
        }

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

                // DEBUG-ONLY authoring guard (release-neutral): a non-seeded
                // transient IMAGE must be written by a prior pass before its first
                // read, or a mis-authored pass silently derives a hazard-free
                // `TOP_OF_PIPE` barrier below instead of surfacing as a caught bug.
                // Ringed/seeded resources (`res_seeded`) are exempt — cross-frame
                // content is intentional for them (the shadow-temporal history pool,
                // the DDGI atlas, CSM cascade / shadow atlas, the shared buffers).
                #[cfg(debug_assertions)]
                {
                    let is_write = access & WRITE_ACCESS_MASK != 0;
                    debug_assert!(
                        !is_image || is_write || self.res_written[ri],
                        "framegraph: pass '{}' reads transient image '{}' with no prior \
                         producer or seed (add_image_seeded) — this would silently derive \
                         a hazard-free TOP_OF_PIPE barrier",
                        self.pass_name[p],
                        self.res_name[ri],
                    );
                    if is_write {
                        self.res_written[ri] = true;
                    }

                    // === INVARIANT HZB-SUBRESOURCE-UNIFORM (debug-only, release-neutral) ===
                    //
                    // EVERY access to one `ResId` must declare the SAME subresource span —
                    // the same `(base_mip, mip_count, base_layer, layer_count)`. Aspect is
                    // excluded: it is a property of the image's format, not a selection.
                    //
                    // WHY THE SPAN ALONE, AND NOT "SPAN *AND* LAYOUT BOTH VARY". The weaker
                    // two-axis condition is the one this guard was first written with, and it
                    // is UNSOUND. A uniform DECLARED layout does not give a uniform ACTUAL
                    // layout, because only the union of the spans that have actually appeared
                    // in an emitted barrier has been transitioned — every other subresource is
                    // still in the image's start layout. Concretely, and this passed the weaker
                    // guard silently:
                    //
                    //     let pyr = g.add_image("pyr");            // starts UNDEFINED
                    //     pass A: SHADER_WRITE, GENERAL, SubRange::COLOR       // mips [0,1)
                    //     pass B: SHADER_READ,  GENERAL, color_mips(4)         // mips [0,4)
                    //
                    // A is a first touch, so its barrier transitions mips [0,1) only. B needs
                    // no layout change but does need a flush, so `transition` returns a barrier
                    // over [0,4) claiming `oldLayout = GENERAL` — for mips 1..3, which were
                    // never transitioned and are still UNDEFINED
                    // (VUID-VkImageMemoryBarrier-oldLayout-01197). Undefined, and invisible to
                    // the validation layers, which see a well-formed barrier and cannot follow
                    // its provenance back to the state machine that invented the `oldLayout`.
                    //
                    // Note the hazard is ORDER-DEPENDENT: declaring the superset FIRST is fine,
                    // the subset first is UB. A guard whose verdict depends on declaration order
                    // is not a guard, which is the second reason the two-axis form is rejected
                    // rather than merely tightened.
                    //
                    // This condition is therefore an OVER-APPROXIMATION on purpose: some
                    // varying-span declarations are safe (superset-first ones), and the guard
                    // fires on them anyway. A fire means "this declaration is outside the region
                    // this state machine can prove safe", NOT "this declaration is broken".
                    //
                    // PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this assert
                    // is its TRIGGER. Keying `state` by `(ResId, mip, layer)` is what lifts the
                    // restriction, and it is what a mip pyramid wants: the HZB build writes mip
                    // k while reading mip k-1. When that pass is authored, it trips this assert.
                    // That is the INTENDED way to discover the work — a mechanical, unmissable
                    // notice at the moment the first declaration needs it. The response is to
                    // build per-subresource tracking, never to relax the condition until it
                    // goes quiet.
                    //
                    // WHY RELEASE IS UNGUARDED. Not "the declaration surface is compile-time
                    // fixed" — WHICH accesses run is heavily data-conditional here (leg and
                    // path predicates gate whole pass families). What IS compile-time fixed is
                    // the span argument at every `image_access` site: they are literal
                    // `SubRange` constructors, so a debug run that REACHES a pass settles that
                    // pass's spans for the release build of the same source. CI runs its tests
                    // as a debug x release matrix, so the debug leg reaches everything the
                    // release leg does. Paying for the check in the release frame path would
                    // buy no information the debug leg did not already have, on a `compile`
                    // that runs every frame (Principle 1/7). What this does NOT cover is a pass
                    // no debug run ever reaches; covering that is the gate's problem, not this
                    // assert's.
                    //
                    // Buffers are structurally exempt without a branch: `buffer_access` always
                    // passes `SubRange::COLOR`, so their span is uniform by construction.

                    // `SubWitness` is `Copy`: read it out, fold this access in, write it
                    // back — no borrow of `self` outlives the update, so the assert below
                    // can still name the pass and resource.
                    let witness = match self.res_sub_witness[ri] {
                        Some(mut w) => {
                            w.span_varied |= !sub.same_span(&w.first_sub);
                            w
                        }
                        None => SubWitness { first_sub: sub, span_varied: false },
                    };
                    self.res_sub_witness[ri] = Some(witness);
                    debug_assert!(
                        !witness.span_varied,
                        "framegraph: INVARIANT HZB-SUBRESOURCE-UNIFORM violated at pass '{}' \
                         on resource '{}' — its accesses declare DIFFERENT subresource spans \
                         (this access: {:?}; first declared: {:?}), and this state machine \
                         tracks ONE layout per ResId, so a subresource that never appeared in \
                         an emitted barrier is still in the image's start layout while a later \
                         barrier claims otherwise. Fix by giving this resource per-subresource \
                         sync state, not by making the declarations agree by hand",
                        self.pass_name[p],
                        self.res_name[ri],
                        sub,
                        witness.first_sub,
                    );
                }

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
