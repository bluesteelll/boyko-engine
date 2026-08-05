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
//!
//! # State keying (VG R3 P1-5a)
//!
//! Sync state is keyed `(ResId, mip)`, not `ResId`. Every resource owns a
//! CONTIGUOUS block of [`ResSync`] entries in one flat `state` arena, located by
//! its `ResShape::state_base` and sized by its declared mip count (one for every
//! resource declared through [`FrameGraph::add_image`] /
//! [`add_image_seeded`](FrameGraph::add_image_seeded) /
//! [`add_buffer`](FrameGraph::add_buffer) /
//! [`add_buffer_seeded`](FrameGraph::add_buffer_seeded); `mips` for one declared
//! through [`add_image_mipped`](FrameGraph::add_image_mipped)). A mip chain whose
//! levels are in different states — the HZB build writes mip `k` while reading
//! mip `k-1` — is therefore tracked exactly, and one access over several mips
//! emits one barrier per RUN of adjacent mips that agreed. Over the single-mip
//! regime the keying collapses to the old one entry-for-entry, which is what makes
//! the existing barrier streams byte-identical.

use super::ids::{PassId, ResId};
use super::sync::{transition, BufBarrier, ImgBarrier, ResSync, SubRange, Trans};
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

/// Where one resource's block of per-mip [`ResSync`] entries lives in the flat `state`
/// arena, and how long it is.
///
/// AoS rather than the file's usual SoA, deliberately: both fields are read TOGETHER at
/// every use — the range check reads `mip_count` and then `compile` reads `state_base` for
/// the same resource — so one 8-byte record is one cache line touch where two parallel
/// `Vec<u32>`s would be two. (Recorded in `docs/VG-R3-P1-PYRAMID-PLAN.md` §11.)
///
/// `state_base` is a running prefix sum maintained by [`FrameGraph::push_res`], the SOLE
/// writer of every per-resource arena; resource `i`'s mip `m` lives at
/// `res_shape[i].state_base + m`.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResShape {
    /// Index of this resource's mip 0 in the flat `state` / `res_written` arenas.
    state_base: u32,
    /// How many mips this resource was DECLARED with (1 for every non-`add_image_mipped`
    /// resource, including every buffer). The bound `image_access` range-checks against.
    mip_count: u32,
}

/// The open, still-extendable run of adjacent mips inside ONE image access whose derived
/// transitions were all identical — the unit `compile` emits as a single [`ImgBarrier`].
///
/// Merging matters for more than barrier count: a whole-chain access on a uniform chain must
/// keep emitting the ONE barrier the per-`ResId` machine emitted, or every existing stream
/// moves. `mip_count` is the run's length so far; `trans` is what every mip in it derived.
#[derive(Clone, Copy)]
struct MipRun {
    base_mip: u32,
    mip_count: u32,
    trans: Trans,
}

/// The DEBUG-ONLY per-resource accumulator behind `INVARIANT SUBRESOURCE-LAYER-UNIFORM`
/// (documented at its check site in [`FrameGraph::compile`]).
///
/// It records the resource's FIRST declared `SubRange` plus one bit saying that some later
/// access disagreed with it on the LAYER axis. One bit suffices because "some two accesses
/// differ" is equivalent to "some access differs from the first" — if every access equals
/// the first, no pair differs; if any differs from the first, that pair does. So the
/// invariant is decidable in one linear pass, with no per-resource set.
///
/// This arena stays keyed per `ResId` while `state` and `res_written` went per `(ResId, mip)`:
/// the invariant it decides is now the LAYER one, and layers are not keyed by anything.
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct SubWitness {
    /// The resource's first declared subresource range. Only its `(base_layer, layer_count)`
    /// are compared; the whole range is kept so the assert can print what was declared.
    first_sub: SubRange,
    /// Some later access declared a different ARRAY-LAYER span than `first_sub`.
    ///
    /// Latched rather than compared pairwise on the spot so the assert can name BOTH the
    /// offending span and the one it should have matched.
    layers_varied: bool,
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
    /// Where each resource's block of per-mip sync entries starts in `state` /
    /// `res_written`, and how long it is. See [`ResShape`].
    res_shape: Vec<ResShape>,
    /// The running prefix sum `push_res` maintains: the TOTAL number of `(ResId, mip)`
    /// entries declared so far, i.e. the length `compile` fills `state` to and the
    /// `state_base` the NEXT resource will be given. Cleared with `res_shape` in
    /// [`reset`](FrameGraph::reset) — the two are one datum split in two places.
    res_state_total: u32,

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
    /// The per-`(ResId, mip)` running sync state, FLAT and MIP-WEIGHTED: resource `i`'s
    /// mip `m` is at `res_shape[i].state_base + m`, and the whole arena is
    /// `res_state_total` entries long. Refilled from `res_seed` (each resource's seed
    /// replicated across its mips) every compile.
    state: Vec<ResSync>,
    /// DEBUG-ONLY per-`(ResId, mip)` "written-or-seeded" bit for `compile`'s authoring
    /// guard: a non-seeded transient IMAGE must be written by a prior pass before
    /// its first read, or a mis-authored pass silently derives a hazard-free
    /// `TOP_OF_PIPE` barrier instead of a caught bug (see `compile`). Cleared to
    /// `res_seeded` every compile; entirely compiled out in release — `compile`
    /// runs every frame, so the tracking must cost nothing there (Principle 1/7).
    ///
    /// MIP-WEIGHTED for the same reason `state` is, and it must stay in step with it: left
    /// per-`ResId`, a pure-read consumer of a mip its writer never wrote would be silent (the
    /// guard would see the *resource* as written), `transition` would take the first-touch arm
    /// on that mip, and the emitted `UNDEFINED → GENERAL` is verbatim the failure this guard
    /// exists to prevent.
    #[cfg(debug_assertions)]
    res_written: Vec<bool>,
    /// DEBUG-ONLY per-RESOURCE witness for `INVARIANT SUBRESOURCE-LAYER-UNIFORM` (stated in
    /// full at its check site in `compile`): the first declared `SubRange` of each resource
    /// plus the "layers varied since" bit the invariant is decided from. `None` until a
    /// resource's first access; refilled with `None` every compile, mirroring how
    /// `res_written` is refilled from `res_seeded`. Entirely compiled out in release —
    /// `compile` runs every frame, so the tracking must cost nothing there (Principle 1/7).
    ///
    /// Per `ResId`, NOT per `(ResId, mip)` — the one debug arena that did not get
    /// mip-weighted, because the invariant it decides is the LAYER one and layers are not
    /// keyed by anything.
    #[cfg(debug_assertions)]
    res_sub_witness: Vec<Option<SubWitness>>,

    // --- compile output ---
    img_barriers: Vec<ImgBarrier>,
    buf_barriers: Vec<BufBarrier>,
    pass_barriers: Vec<PassBarrierRange>,
}

/// The declared mip span of an `image_access` does not lie inside the resource's declared mip
/// count. Out of line + `#[cold]`: the check is on the declare path of every image access in
/// the frame, so its failure arm must not sit in the I-cache beside the check.
///
/// The message deliberately shares NO phrase with `INVARIANT SUBRESOURCE-LAYER-UNIFORM`'s: the
/// two are distinct failures with distinct fixes (declare the resource with the right mip
/// count, versus give the resource per-layer state), and a shared substring would let one
/// `should_panic(expected = ...)` be satisfied by either — a gate that cannot tell the two
/// apart is not a gate.
#[cold]
#[inline(never)]
fn mip_range_out_of_shape(name: &'static str, sub: SubRange, declared_mips: u32) -> ! {
    panic!(
        "framegraph: image_access on resource '{name}' declares a mip span OUTSIDE THE \
         DECLARED SHAPE — base_mip {}, mip_count {} — but '{name}' was declared with \
         {declared_mips} mip level(s). `add_image`/`add_image_seeded` declare ONE level; \
         a chain must be declared with `add_image_mipped(name, mips, seed)`, and \
         `mip_count == 0` selects nothing. This is checked in release because the barrier \
         path bounds nothing: `base_mip`/`mip_count` are copied verbatim into \
         `VkImageSubresourceRange`, and the sync state is keyed (ResId, mip), so an \
         out-of-shape span silently indexes a NEIGHBOURING resource's state",
        sub.base_mip, sub.mip_count,
    )
}

/// A mipped image was declared with zero levels. Out of line + `#[cold]` for the same reason
/// as [`mip_range_out_of_shape`], though this one is once per declared resource.
#[cold]
#[inline(never)]
fn zero_mip_declaration(name: &'static str) -> ! {
    panic!(
        "framegraph: add_image_mipped('{name}', 0, ..) — a resource with zero mip levels owns \
         no sync entries, so the NEXT resource declared would be handed the same `state_base` \
         and every access to it would advance this resource's state instead. Declare at least \
         one level (`add_image` if the resource has no chain)"
    )
}

impl FrameGraph {
    /// Preallocate all arenas ONCE. `max_acc` bounds the total accesses across
    /// all passes in a frame. Sized generously (the frame declares tens of each);
    /// exceeding a cap in `reset`-then-declare only regrows the `Vec` (cold), it
    /// is never a correctness issue.
    ///
    /// `max_res` counts RESOURCES, while the sync arenas are indexed per
    /// `(ResId, mip)`, so a graph declaring mipped resources fills `state` past
    /// `max_res` and regrows it once. Size `max_res` against the mip-weighted total
    /// ([`res_state_total`](FrameGraph::res_state_total)) if a mipped declarator is
    /// meant to stay allocation-free.
    pub fn with_capacity(max_res: usize, max_pass: usize, max_acc: usize) -> Self {
        Self {
            res_is_image: Vec::with_capacity(max_res),
            res_name: Vec::with_capacity(max_res),
            res_seed: Vec::with_capacity(max_res),
            res_seeded: Vec::with_capacity(max_res),
            res_shape: Vec::with_capacity(max_res),
            res_state_total: 0,
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
    ///
    /// `res_shape` and `res_state_total` are ONE datum stored in two places — the shapes and
    /// the running prefix sum they were cut from — and are cleared together, beside `state`,
    /// which is derived from both. Clearing either alone silently corrupts the next frame:
    ///
    /// * shapes cleared, total kept → the next frame's FIRST resource is handed
    ///   `state_base = <last frame's total>` while `compile` sizes `state` from the NEW total,
    ///   so every index lands past the end (a panic if far enough out, and a read of some
    ///   other resource's mip if not).
    /// * total cleared, shapes kept → `push_res` derives fresh `ResId`s from the (cleared)
    ///   `res_is_image` while `res_shape` still holds LAST frame's entries at those indices,
    ///   so every shape lookup returns a stale `state_base`/`mip_count`: the release-live
    ///   range check validates against last frame's mip count and the state machine advances
    ///   last frame's layouts.
    ///
    /// The `debug_assert_eq!` at the top of `compile` (`state.len() == res_state_total`) is
    /// what turns a future edit that clears only one of them into a caught bug.
    pub fn reset(&mut self) {
        self.res_is_image.clear();
        self.res_name.clear();
        self.res_seed.clear();
        self.res_seeded.clear();
        self.res_shape.clear();
        self.res_state_total = 0;
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

    /// Declare a transient/history IMAGE resource (layout starts UNDEFINED), with ONE mip
    /// level — every access to it must declare `base_mip: 0, mip_count: 1`
    /// ([`SubRange::COLOR`] / [`DEPTH`](SubRange::DEPTH) /
    /// [`color_layers`](SubRange::color_layers) / [`depth_layers`](SubRange::depth_layers)).
    /// For a real mip chain use [`add_image_mipped`](FrameGraph::add_image_mipped).
    #[inline]
    pub fn add_image(&mut self, name: &'static str) -> ResId {
        self.push_res(true, name, ResSync::undefined(), false, 1)
    }

    /// Declare a BUFFER resource (no layout; ordering is flush/visibility only).
    #[inline]
    pub fn add_buffer(&mut self, name: &'static str) -> ResId {
        self.push_res(false, name, ResSync::undefined(), false, 1)
    }

    /// Declare a NON-RINGED IMAGE shared by both in-flight frames (CSM cascade,
    /// shadow atlas), seeding its start-of-frame sync state with the sibling
    /// frame's end-of-frame scopes (see [`ResSync::seeded_readers`] /
    /// [`ResSync::seeded_writer`]). The first write this frame then orders after
    /// the sibling's still-pipelined accesses instead of the hazard-free
    /// `TOP_OF_PIPE` a fresh `undefined()` state yields (audit B-002/B-003).
    #[inline]
    pub fn add_image_seeded(&mut self, name: &'static str, seed: ResSync) -> ResId {
        self.push_res(true, name, seed, true, 1)
    }

    /// Declare a MIPPED IMAGE — `mips` levels, each tracked as its own `(ResId, mip)` sync
    /// entry, so a pass may write mip `k` while reading mip `k-1` (the HZB depth pyramid).
    /// Accesses select levels with [`SubRange::color_mips`] or a hand-built `SubRange`; the
    /// span is range-checked against `mips` in EVERY profile (see
    /// [`image_access`](FrameGraph::image_access)).
    ///
    /// # Why the seed is a REQUIRED argument and not an `Option`
    ///
    /// This is the only route by which a mipped resource can be declared, and a mip chain
    /// deep enough to be worth building is one no engine here rings per frame in flight — the
    /// pyramid is a SINGLE image shared by both in-flight frames. Defaulting its seed to
    /// `ResSync::undefined()` would not be a compile error the day it is wrong; it would be a
    /// silent one-way door: frame N+1's first write to mip `d` would derive a `TOP_OF_PIPE`
    /// src with no dependency on frame N's still-pipelined reads — the cross-frame WAR race
    /// this engine has already shipped and fixed once (audit B-002/B-003), invisible to every
    /// golden because it is "wrong only in motion". Requiring the argument makes the question
    /// unavoidable at the declare site. [`add_image_seeded`](FrameGraph::add_image_seeded)
    /// cannot substitute: it declares one mip, and the range check then rejects `base_mip > 0`.
    ///
    /// Pass [`ResSync::undefined`] explicitly for a chain that genuinely has no cross-frame
    /// hazard (nothing reads it while the sibling frame writes it) — the point is that this is
    /// then a stated decision rather than a default.
    ///
    /// # Panics
    ///
    /// If `mips == 0`, in every profile: a zero-mip resource owns no sync entries, so the next
    /// resource would alias its `state_base` and every access to it would index a neighbour.
    #[inline]
    pub fn add_image_mipped(&mut self, name: &'static str, mips: u32, seed: ResSync) -> ResId {
        if mips == 0 {
            zero_mip_declaration(name);
        }
        // `seeded = true`: a mipped image is by construction the non-ringed, cross-frame kind
        // (that is what forces the seed argument), so it takes the same exemption from
        // `compile`'s unwritten-transient-read guard that every `add_image_seeded` resource
        // takes — its content across frames is intentional.
        self.push_res(true, name, seed, true, mips)
    }

    /// Declare a NON-RINGED BUFFER shared by both in-flight frames (light table,
    /// tiles, cluster grid/index/alloc): same cross-frame seeding as
    /// [`add_image_seeded`](FrameGraph::add_image_seeded) (buffers have no
    /// layout; the seed only strengthens the first access's src scope).
    #[inline]
    pub fn add_buffer_seeded(&mut self, name: &'static str, seed: ResSync) -> ResId {
        self.push_res(false, name, seed, true, 1)
    }

    /// The SOLE writer of every per-resource arena — all five public declarators funnel
    /// through it, and `ResId` is constructed nowhere else in the workspace. `mips` is 1 for
    /// every declarator but [`add_image_mipped`](FrameGraph::add_image_mipped) (buffers
    /// included: they own exactly one sync entry, at their `state_base`).
    #[inline]
    fn push_res(
        &mut self,
        is_image: bool,
        name: &'static str,
        seed: ResSync,
        seeded: bool,
        mips: u32,
    ) -> ResId {
        debug_assert!(
            self.res_is_image.len() < u16::MAX as usize,
            "framegraph resource count exceeds u16 index space"
        );
        debug_assert!(mips >= 1, "invariant: push_res mips must be >= 1");
        let id = ResId(self.res_is_image.len() as u16);
        self.res_is_image.push(is_image);
        self.res_name.push(name);
        self.res_seed.push(seed);
        self.res_seeded.push(seeded);
        self.res_shape.push(ResShape { state_base: self.res_state_total, mip_count: mips });
        // `checked_add` (not a bare `+`) because release builds run with overflow-checks OFF:
        // a wrapped total would hand the NEXT resource a `state_base` that aliases an earlier
        // one, and the aliasing index stays IN BOUNDS — a silent wrong-resource transition
        // rather than a panic. Once per declared resource, tens per frame.
        self.res_state_total = self
            .res_state_total
            .checked_add(mips)
            .expect("invariant: framegraph total mip count overflows u32");
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
    ///
    /// # Panics
    ///
    /// In EVERY profile, if `sub`'s mip span is not contained in the mip count `res` was
    /// declared with — including the empty span `mip_count == 0`.
    ///
    /// # Why that check is release-LIVE while the subresource invariant below it is not
    ///
    /// `INVARIANT SUBRESOURCE-LAYER-UNIFORM` may be debug-only because every span it judges is
    /// a compile-time literal, so a debug run that REACHES a pass settles that pass's spans for
    /// the release build of the same source. A mip span is the first one that is NOT: the
    /// pyramid's level count is derived from the render extent at runtime, so a release-only
    /// resolution can produce a span no debug run ever formed. And the consequence is not a
    /// caught bug — `graph_bridge.rs` copies `base_mip`/`mip_count` verbatim into
    /// `VkImageSubresourceRange` and the sink holds bare `VkImage` handles with no mip count
    /// to check against, while `compile` indexes `state` at `state_base + mip`, where an
    /// out-of-shape mip lands on a NEIGHBOURING resource's entry and stays in bounds.
    ///
    /// This is a DECLARE-time function: it runs before `compile`, so the check dominates every
    /// mip-weighted index the state machine will form (`push_access` is the sole writer of
    /// `acc_sub`, and its only other caller — `buffer_access` — hardcodes `SubRange::COLOR`).
    #[inline]
    pub fn image_access(&mut self, res: ResId, stage: u32, access: u32, layout: i32, sub: SubRange) {
        debug_assert!(
            self.res_is_image[res.index()],
            "image_access on a buffer resource '{}'",
            self.res_name[res.index()]
        );
        let shape = self.res_shape[res.index()];
        // Written so the CHECK cannot overflow: the third test subtracts only after the second
        // has established `base_mip < mip_count`, and it never adds — so `mip_count: u32::MAX`
        // (this engine defines no `VK_REMAINING_MIP_LEVELS` sentinel, and a caller reaching for
        // one would spell that) panics rather than wrapping into acceptance.
        if sub.mip_count == 0
            || sub.base_mip >= shape.mip_count
            || sub.mip_count > shape.mip_count - sub.base_mip
        {
            mip_range_out_of_shape(self.res_name[res.index()], sub, shape.mip_count);
        }
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

    /// Emit one derived image barrier for a COMPLETED run of adjacent mips: the run's
    /// `Trans` verbatim, plus the access's own `sub` with ONLY the mip span replaced by the
    /// run's. `aspect`/`base_layer`/`layer_count` are the access's — the mip axis is the only
    /// one this machine derives.
    fn push_img_run(&mut self, res: ResId, sub: SubRange, run: MipRun) {
        self.img_barriers.push(ImgBarrier {
            res,
            src_stage: run.trans.src_stage,
            dst_stage: run.trans.dst_stage,
            src_access: run.trans.src_access,
            dst_access: run.trans.dst_access,
            old_layout: run.trans.old_layout,
            new_layout: run.trans.new_layout,
            subresource: SubRange {
                aspect: sub.aspect,
                base_mip: run.base_mip,
                mip_count: run.mip_count,
                base_layer: sub.base_layer,
                layer_count: sub.layer_count,
            },
        });
    }

    /// Derive the minimal per-pass barrier plan from the declared accesses,
    /// walking passes in declaration (execution) order and running the Granite
    /// sync state machine, keyed `(ResId, mip)`. Idempotent given the same
    /// declarations.
    pub fn compile(&mut self) {
        self.img_barriers.clear();
        self.buf_barriers.clear();
        self.pass_barriers.clear();

        // Per-`(ResId, mip)` start state: ringed/transient resources start UNDEFINED
        // (re-`UNDEFINED`'d each frame — prior content discarded); NON-RINGED
        // shared resources start from their declared cross-frame seed (visible
        // consumer scopes), so their first write orders after the sibling
        // in-flight frame's reads (see `add_image_seeded`). One resource's seed
        // applies to EVERY mip it declared: a chain shared across frames is shared
        // whole, and its per-mip states diverge only through this frame's accesses.
        self.state.clear();
        for i in 0..self.res_shape.len() {
            let mips = self.res_shape[i].mip_count as usize;
            let seed = self.res_seed[i];
            let len = self.state.len();
            self.state.resize(len + mips, seed);
        }
        debug_assert_eq!(
            self.state.len(),
            self.res_state_total as usize,
            "framegraph: the per-mip state arena disagrees with the prefix sum `push_res` \
             maintained — `reset` must clear `res_shape` and `res_state_total` together"
        );
        // DEBUG-ONLY authoring-guard scratch: starts from the declare-time seeded
        // bit (a seeded resource is exempt everywhere), replicated across the
        // resource's mips, then latches `true` per MIP at each write encountered
        // below. Entirely compiled out in release.
        #[cfg(debug_assertions)]
        {
            self.res_written.clear();
            for i in 0..self.res_shape.len() {
                let mips = self.res_shape[i].mip_count as usize;
                let seeded = self.res_seeded[i];
                let len = self.res_written.len();
                self.res_written.resize(len + mips, seeded);
            }
            // No declare-time seed exists for the subresource witness: it is derived purely
            // from this compile's accesses, so every resource starts unwitnessed. Per
            // `ResId`, not per `(ResId, mip)` — see `res_sub_witness`.
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
                // This resource's block of per-mip sync entries. `image_access` has already
                // range-checked `sub`'s span against the declared shape IN EVERY PROFILE, and
                // `buffer_access` hardcodes `SubRange::COLOR` against the single entry every
                // buffer owns — so `mip_base + m` is in this resource's own block by
                // construction, never a neighbour's.
                let state_base = self.res_shape[ri].state_base as usize;
                let mip_base = state_base + sub.base_mip as usize;

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
                    // PER MIP, in step with `state`: a consumer of mip k is not made safe by
                    // a producer of mip 0. Left per-`ResId` this guard would see the RESOURCE
                    // as written, stay silent, and let `transition` take the first-touch arm
                    // on mip k — emitting exactly the `UNDEFINED → GENERAL` it exists to stop.
                    for m in 0..sub.mip_count as usize {
                        debug_assert!(
                            !is_image || is_write || self.res_written[mip_base + m],
                            "framegraph: pass '{}' reads transient image '{}' mip {} with no \
                             prior producer or seed (add_image_seeded / add_image_mipped) — \
                             this would silently derive a hazard-free TOP_OF_PIPE barrier",
                            self.pass_name[p],
                            self.res_name[ri],
                            sub.base_mip as usize + m,
                        );
                        if is_write {
                            self.res_written[mip_base + m] = true;
                        }
                    }

                    // === INVARIANT SUBRESOURCE-LAYER-UNIFORM (debug-only, release-neutral) ===
                    //
                    // EVERY access to one `ResId` must declare the SAME ARRAY-LAYER span — the
                    // same `(base_layer, layer_count)`. The MIP span is free. Aspect is
                    // excluded: it is a property of the image's format, not a selection.
                    //
                    // WHY THE MIP AXIS IS NO LONGER ASSERTED — the MECHANISM, not a relaxation.
                    // Sync state is keyed `(ResId, mip)`: `ResShape::state_base` gives each
                    // resource a contiguous block of `ResSync` entries, this loop advances mip
                    // `m` at `state_base + m`, and a barrier's `base_mip`/`mip_count` are
                    // DERIVED from the run of adjacent mips that agreed. There is no longer one
                    // tracked layout for several mips to disagree with; there is one per mip.
                    // This is the answer the OLD form of this comment prescribed by name:
                    // "PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this
                    // assert is its TRIGGER … The response is to build per-subresource
                    // tracking, never to relax the condition until it goes quiet."
                    //
                    // THE DISCRIMINATOR AGAINST "RELAXING UNTIL IT GOES QUIET". Both moves —
                    // building the machine and widening the condition — end with the assert
                    // silent on a mip-varying declaration, so silence is not the evidence. The
                    // evidence is that the machine now derives a real, DISTINCT transition per
                    // mip, and the fixtures that used to trip this axis assert exactly that: in
                    // `tests/framegraph_gbuffer_equiv.rs`,
                    // `compile_allows_two_mip_spans_on_one_resource` and its two neighbours no
                    // longer assert "no panic" — they assert the derived barrier list field by
                    // field, plus `resolved_layout_mip` reporting DIFFERENT layouts on
                    // different mips of one image. A widened condition would leave those
                    // fixtures emitting the old single whole-chain barrier under a single
                    // tracked layout, and they would fail. Quiet because the machine answers
                    // the question, not because the question stopped being asked.
                    //
                    // WHY THE LAYER AXIS KEEPS THE GUARD, and the hazard it still describes.
                    // One `ResSync` block covers ALL of a resource's layers, so a uniform
                    // DECLARED layout does not give a uniform ACTUAL layout across layers: only
                    // the union of the layer spans that have actually appeared in an emitted
                    // barrier has been transitioned, and every other layer is still in the
                    // image's start layout. Concretely:
                    //
                    //     let atlas = g.add_image("atlas");                    // UNDEFINED
                    //     pass A: DEPTH_WRITE, DEPTH_ATTACHMENT, depth_layers(1)  // layers [0,1)
                    //     pass B: SHADER_READ, DEPTH_ATTACHMENT, depth_layers(4)  // layers [0,4)
                    //
                    // A is a first touch, so its barrier transitions layer [0,1) only. B needs
                    // no layout change but does need a flush, so `transition` returns a barrier
                    // over [0,4) claiming `oldLayout = DEPTH_ATTACHMENT_OPTIMAL` — for layers
                    // 1..3, which were never transitioned and are still UNDEFINED
                    // (VUID-VkImageMemoryBarrier-oldLayout-01197). Undefined, and invisible to
                    // the validation layers, which see a well-formed barrier and cannot follow
                    // its provenance back to the state machine that invented the `oldLayout`.
                    //
                    // The hazard is ORDER-DEPENDENT: declaring the superset FIRST is fine, the
                    // subset first is UB. A guard whose verdict depends on declaration order is
                    // not a guard, so this condition is an OVER-APPROXIMATION on purpose — some
                    // varying-layer declarations are safe (superset-first ones) and it fires on
                    // them anyway. A fire means "this declaration is outside the region this
                    // state machine can prove safe", NOT "this declaration is broken". The fix
                    // is per-layer state, exactly as per-mip state was the fix on the other
                    // axis — never making the declarations agree by hand.
                    //
                    // WHY THE TWO AXES ARE TREATED DIFFERENTLY, argued rather than asserted.
                    // (1) The mip axis has a live consumer that is expressible no other way:
                    // the HZB build writes mip k while reading mip k-1, in one frame, on one
                    // image — there is no declaration of that which a per-`ResId` layout can
                    // describe. The layer axis has none: every layered resource in the tree
                    // (CSM cascades, the punctual atlas, the DDGI probe atlases) is written and
                    // read WHOLE-ARRAY, at one compile-time-constant span. Paying for an axis
                    // with no consumer is speculation. (2) The flat `state_base + m` keying
                    // cannot collide with a layered resource, because `texture.rs:227-231`
                    // (`debug_assert!(!(is_array && desc.mip_levels > 1))`) makes mipped and
                    // layered DISJOINT at image creation — no image in this engine is both, so
                    // no resource needs a `mips x layers` product of entries.
                    //
                    // WHY RELEASE IS UNGUARDED — and why that argument SURVIVES the narrowing,
                    // which is the same fact as (1) above rather than a second one. The
                    // argument was never "the declaration surface is compile-time fixed":
                    // WHICH accesses run is heavily data-conditional here (leg and path
                    // predicates gate whole pass families). It was that the SPAN ARGUMENT at
                    // every `image_access` site is a literal `SubRange` constructor, so a debug
                    // run that REACHES a pass settles that pass's spans for the release build
                    // of the same source, and CI's debug x release matrix reaches everything.
                    // The pyramid's MIP span is the first one in the tree that is DATA-DERIVED
                    // (its level count comes from the render extent) — and the mip axis is
                    // exactly the axis that stopped being asserted here and became a
                    // release-live range check in `image_access` instead. Every LAYER span
                    // still in the tree is a constant constructor (`depth_layers(CASCADE_LAYERS)`,
                    // `color_layers(DDGI_ATLAS_LAYERS)`), so the axis that kept the debug-only
                    // assert is exactly the axis the debug-only argument still covers.
                    //
                    // Buffers are structurally exempt without a branch: `buffer_access` always
                    // passes `SubRange::COLOR`, so their layer span is uniform by construction.

                    // `SubWitness` is `Copy`: read it out, fold this access in, write it
                    // back — no borrow of `self` outlives the update, so the assert below
                    // can still name the pass and resource.
                    let witness = match self.res_sub_witness[ri] {
                        Some(mut w) => {
                            w.layers_varied |= !sub.same_layers(&w.first_sub);
                            w
                        }
                        None => SubWitness { first_sub: sub, layers_varied: false },
                    };
                    self.res_sub_witness[ri] = Some(witness);
                    debug_assert!(
                        !witness.layers_varied,
                        "framegraph: INVARIANT SUBRESOURCE-LAYER-UNIFORM violated at pass '{}' \
                         on resource '{}' — its accesses declare DIFFERENT array-layer spans \
                         (this access: {:?}; first declared: {:?}), and this state machine \
                         tracks ONE layout per (ResId, mip) across ALL layers, so a layer that \
                         never appeared in an emitted barrier is still in the image's start \
                         layout while a later barrier claims otherwise. Fix by giving this \
                         resource per-layer sync state, not by making the declarations agree \
                         by hand",
                        self.pass_name[p],
                        self.res_name[ri],
                        sub,
                        witness.first_sub,
                    );
                }

                if is_image {
                    // Advance each SELECTED mip's own state, then emit one barrier per
                    // maximal RUN of adjacent mips whose derived transition was IDENTICAL.
                    // A chain whose mips are all in the same state therefore still emits the
                    // single whole-span barrier the per-`ResId` machine emitted — that is the
                    // byte-identity fold, and it is why every single-mip stream in the tree is
                    // unmoved (one mip, one transition, one barrier carrying the access's own
                    // span). A chain whose mips diverged emits one barrier per distinct state.
                    //
                    // A mip deriving `None` — a free, already-visible read — BREAKS the run.
                    // It needs no barrier at all, and folding it into a neighbouring run would
                    // emit a barrier over a subresource that did not ask for one, which is the
                    // widening this step exists to remove.
                    //
                    // Split-borrow, as before: `transition` mutates the mip's state inside a
                    // scoped borrow, which is released before anything is pushed.
                    let mut run: Option<MipRun> = None;
                    for m in 0..sub.mip_count {
                        let trans = {
                            let st = &mut self.state[mip_base + m as usize];
                            transition(st, stage, access, layout)
                        };
                        match (&mut run, trans) {
                            // Same transition as the open run: widen it, emit nothing yet.
                            (Some(open), Some(t)) if open.trans == t => open.mip_count += 1,
                            // A different transition, or none at all: close whatever was open
                            // and start a new run iff this mip needs one.
                            (slot, t) => {
                                if let Some(done) = slot.take() {
                                    self.push_img_run(res, sub, done);
                                }
                                *slot = t.map(|trans| MipRun {
                                    base_mip: sub.base_mip + m,
                                    mip_count: 1,
                                    trans,
                                });
                            }
                        }
                    }
                    if let Some(done) = run {
                        self.push_img_run(res, sub, done);
                    }
                } else {
                    // BUFFERS — deliberately NOT the per-mip loop. A buffer owns exactly ONE
                    // sync entry, at its `state_base`, so the loop would run once with a run
                    // accumulator that can never merge anything. This arm is the pre-P1-5a
                    // code unchanged: the byte-identity argument for every buffer stream in
                    // the tree, and the I-cache one on the arm that carries most of the
                    // frame's accesses.
                    let trans = {
                        let st = &mut self.state[state_base];
                        // Pass the current (sentinel) layout so the layout arm never fires.
                        let want_layout = st.layout;
                        transition(st, stage, access, want_layout)
                    };

                    if let Some(t) = trans {
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
    /// # The ONE difference from the C1 text, and why it is not a patch
    ///
    /// Commit C2 renamed `SubWitness::span_varied` to `layers_varied` (the live guard is now
    /// the layer-only `INVARIANT SUBRESOURCE-LAYER-UNIFORM`). That struct is SHARED with this
    /// body, so the field's new spelling appears here three times — and nothing else does.
    /// The PREDICATE is untouched: this body still calls
    /// [`SubRange::same_span`](super::sync::SubRange::same_span), which still compares all
    /// four span fields and is kept alive in `sync.rs` for this caller alone, precisely so
    /// that no behavioural drift is possible. A `bool` that is latched from the same
    /// expression, compared by the same assert and never read by anything else decides the
    /// same thing under either name, on every input — mipped graphs included, where this body
    /// therefore still panics exactly as the C1 machine did. Everything the differential
    /// reads — `img_barriers`, `buf_barriers`, `pass_barriers` — is downstream of code this
    /// rename does not touch.
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
                            w.layers_varied |= !sub.same_span(&w.first_sub);
                            w
                        }
                        None => SubWitness { first_sub: sub, layers_varied: false },
                    };
                    self.res_sub_witness[ri] = Some(witness);
                    debug_assert!(
                        !witness.layers_varied,
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

    /// The total number of `(ResId, mip)` sync entries declared so far — the length
    /// `compile` fills the state arena to. Equals the resource count while every resource is
    /// single-mip, and exceeds it by `mips - 1` for each
    /// [`add_image_mipped`](FrameGraph::add_image_mipped) declaration.
    #[inline]
    pub fn res_state_total(&self) -> u32 {
        self.res_state_total
    }

    /// The GROUND-TRUTH final layout MIP `mip` of a resource reaches after `compile` — read
    /// directly from that mip's running sync state (NOT reconstructed from the barrier list,
    /// which would miss a final layout-preserving free access and report a stale layout).
    /// Call after `compile`; panics if called before (the state arena is only populated by
    /// `compile`).
    #[inline]
    pub fn resolved_layout_mip(&self, res: ResId, mip: u32) -> i32 {
        let shape = self.res_shape[res.index()];
        debug_assert!(
            mip < shape.mip_count,
            "resolved_layout_mip: mip {} is outside '{}'s {} declared level(s)",
            mip,
            self.res_name[res.index()],
            shape.mip_count
        );
        let entry = shape.state_base as usize + mip as usize;
        debug_assert!(
            entry < self.state.len(),
            "resolved_layout called before compile (or out-of-range resource)"
        );
        self.state[entry].layout
    }

    /// The GROUND-TRUTH final layout a SINGLE-MIP resource reaches after `compile` — the
    /// [`resolved_layout_mip`](FrameGraph::resolved_layout_mip) of its mip 0.
    ///
    /// The index is REBASED through the resource's `ResShape::state_base`, not `res.index()`:
    /// with a mip-weighted state arena `state[res.index()]` is a different entry as soon as ANY
    /// earlier resource declared a chain, and it stays in bounds (the arena only grew), so the
    /// wrong read is silent — and against a corpus where most layouts are `GENERAL` it often
    /// agrees by coincidence. The debug-assert below is an ergonomic "which mip did you mean"
    /// guard for mipped callers, NOT the argument that this index is right; that argument is
    /// the rebase.
    #[inline]
    pub fn resolved_layout(&self, res: ResId) -> i32 {
        debug_assert_eq!(
            self.res_shape[res.index()].mip_count, 1,
            "resolved_layout on '{}', which was declared with several mip levels — its mips \
             can hold DIFFERENT layouts, so say which one with resolved_layout_mip",
            self.res_name[res.index()]
        );
        self.resolved_layout_mip(res, 0)
    }
}
