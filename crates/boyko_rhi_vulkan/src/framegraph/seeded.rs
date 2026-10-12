//! OBS production observers (render-wave plan H0 Scope 4, AD-15): the derived **seed class** of a
//! declared resource, the cross-frame wrappers [`SeededImage`] / [`SeededBuffer`] with the
//! initialising-access discipline, and the two debug-only invariants `compile` runs after its pass
//! walk — the **seeded-layout** invariant and the **trailing-scope** seed-coverage check.
//!
//! # Why these are production invariants, not replica tests
//!
//! Synchronization validation is blind to the cross-frame write-after-read class on this machine
//! (measured: zero messages with a narrowed `material_table` seed, `graph_bridge.rs`'s
//! `MaterialTableRes` doc). A replica of a declarator diverges silently from the production one
//! (`tests/particle_barrier_stream.rs`'s own header). So the checks run INSIDE
//! [`FrameGraph::compile`] on the production declarators, debug-only: every golden run is a
//! dev-profile run, so every golden exercises them, and release pays nothing.
//!
//! # The seed classes, derived — never stored
//!
//! A resource's class is a function of the two things `push_res` already records, its seed and
//! its seeded (provenance) bit, so `FrameGraph` gains no per-resource field:
//!
//! | seeded bit | seed       | class                                                        |
//! |------------|------------|--------------------------------------------------------------|
//! | `true`     | stateful   | [`SeedClass::CrossFrame`] — content and ordering cross frames |
//! | `false`    | stateful   | [`SeedClass::FrameLocal`] — WAR seed kept, content rebuilt    |
//! | `false`    | undefined  | [`SeedClass::Bare`] — per-FIF ring or transient               |
//! | `true`     | undefined  | [`SeedClass::ProvenanceOnly`] — host-filled, no ordering      |
//!
//! # The trailing scope
//!
//! A seed is "the sibling frame's end-of-frame scopes" (`ResSync::seeded_readers` /
//! `seeded_writer`), so that is where its coverage is checked. Per mip, the trailing scope is:
//!
//! - the stages of every access declared AFTER the frame's last write-carrying access to that mip
//!   (two unordered trailing readers both count — neither is ordered before the other);
//! - that write's own stage when nothing follows it — and the seed must then be a WRITER seed
//!   (`flush_stages != 0`), or the next frame's first access orders after the sibling's reads
//!   only and never makes the sibling's write available;
//! - every access's stage on a frame with no write to that mip.
//!
//! Every access before the last write is ordered before it by the frame's own barriers, so only
//! the trailing scope can race the next frame's first access. Checked per access instead, the
//! rule would fire on legitimate in-frame writes outside a reader seed (`taa_cov`'s blended draws,
//! `csm_trans_draw`, `lit_prev`'s copy), none of which is a cross-frame gap.
//!
//! Per MIP, not per resource: two different mips of one image are different subresources, so a
//! read of mip 3 and a later write of mip 0 are not ordered by any in-frame barrier, and a
//! per-resource walk would wrongly treat the read as covered by the write.

use super::graph::FrameGraph;
use super::ids::ResId;
use super::sync::{ResSync, SubRange, WRITE_ACCESS_MASK};
use crate::ffi::{
    VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_TRANSFER_BIT,
};

/// How many [`SeededImage`] / [`SeededBuffer`] declarations one frame graph tracks. The debug
/// registry is an inline array (no heap field on `FrameGraph`, UG-02); overflow is a
/// `debug_assert`, never a silent drop.
pub const MAX_SEEDED_WRAPPERS: usize = 32;

/// How many [`FrameGraph::add_image_seeded_restored`] opt-outs one frame graph tracks.
pub const MAX_RESTORED_IMAGES: usize = 4;

/// The derived seed class of a declared resource (see the module doc's table).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeedClass {
    /// Seeded bit set, stateful seed: content AND ordering cross frames (`taa_hist`, the light
    /// table, `material_table`, the particle counters).
    CrossFrame,
    /// Seeded bit clear, stateful seed: the WAR seed is kept, but the content is rebuilt every
    /// frame, so `compile`'s unwritten-read guard stays live.
    FrameLocal,
    /// Seeded bit clear, undefined seed: a per-FIF ring slot or an in-frame transient.
    Bare,
    /// Seeded bit set, undefined seed: content from outside the graph (a host-filled ring), no
    /// cross-frame ordering.
    ProvenanceOnly,
}

impl SeedClass {
    /// Derives the class from the seeded bit and the seed `push_res` recorded.
    #[inline]
    pub const fn derive(seeded: bool, seed: ResSync) -> Self {
        match (seeded, seed_is_stateful(seed)) {
            (true, true) => Self::CrossFrame,
            (false, true) => Self::FrameLocal,
            (false, false) => Self::Bare,
            (true, false) => Self::ProvenanceOnly,
        }
    }

    /// The class's name as the receipt and the failure messages spell it.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CrossFrame => "CrossFrame",
            Self::FrameLocal => "FrameLocal",
            Self::Bare => "Bare",
            Self::ProvenanceOnly => "ProvenanceOnly",
        }
    }
}

/// `true` iff `seed` differs from [`ResSync::undefined`] in any field — it carries a layout, a
/// visible scope or a pending write the next frame's first access must respect.
#[inline]
pub const fn seed_is_stateful(seed: ResSync) -> bool {
    seed.layout != VK_IMAGE_LAYOUT_UNDEFINED
        || seed.flush_access != 0
        || seed.flush_stages != 0
        || seed.visible_access != 0
        || seed.visible_stages != 0
}

/// Every pipeline stage `seed` orders the next frame's first access after: the sibling frame's
/// visible readers and its pending writer.
#[inline]
pub const fn seed_stages(seed: ResSync) -> u32 {
    seed.visible_stages | seed.flush_stages
}

/// What a wrapper's content does across frames — the class it declares (see [`SeedClass`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeedContent {
    /// Content crosses frames ([`SeedClass::CrossFrame`]): exempt from the unwritten-read guard.
    CrossFrame,
    /// Content is rebuilt every frame ([`SeedClass::FrameLocal`]): the first access of the frame
    /// must be an `init_*` access.
    FrameLocal,
}

/// ORs the image-dump instrument's `TRANSFER` read into `seed` (H0 Scope 6). Armed at boot only
/// when the instrument is: frame N+1's first write to a single dumped resource must then order
/// after frame N's dump copy, which is a `TRANSFER` read in that resource's trailing scope. With
/// the instrument off the seed is returned unchanged, so no shipped barrier moves.
#[inline]
pub const fn widen_for_dump(seed: ResSync, dump_widen: bool) -> ResSync {
    if dump_widen {
        ResSync {
            visible_stages: seed.visible_stages | VK_PIPELINE_STAGE_TRANSFER_BIT,
            visible_access: seed.visible_access | VK_ACCESS_TRANSFER_READ_BIT,
            ..seed
        }
    } else {
        seed
    }
}

// --- the debug-only registry `FrameGraph` carries -------------------------------------------

/// One registered wrapper: its resource, and whether an `init_*` access of it was declared yet
/// this frame. The init bit lives HERE, in the graph, rather than
/// in the wrapper value, so a `Copy` wrapper used by value at many declare sites has one bit per
/// declaration, not one per copy.
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
pub(crate) struct WrapperSlot {
    pub(crate) res: ResId,
    pub(crate) init_seen: bool,
}

/// The debug-only registries behind the seeded-layout invariant and the trailing-scope check:
/// fixed inline arrays, cleared by `FrameGraph::reset`, so the graph gains no heap field.
#[cfg(debug_assertions)]
pub(crate) struct ObsRegistry {
    wrappers: [WrapperSlot; MAX_SEEDED_WRAPPERS],
    wrapper_count: u8,
    restored: [(ResId, &'static str); MAX_RESTORED_IMAGES],
    restored_count: u8,
}

#[cfg(debug_assertions)]
impl ObsRegistry {
    pub(crate) const fn new() -> Self {
        Self {
            wrappers: [WrapperSlot { res: ResId(0), init_seen: false };
                MAX_SEEDED_WRAPPERS],
            wrapper_count: 0,
            restored: [(ResId(0), ""); MAX_RESTORED_IMAGES],
            restored_count: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.wrapper_count = 0;
        self.restored_count = 0;
    }

    pub(crate) fn register_wrapper(&mut self, res: ResId) -> u8 {
        let n = self.wrapper_count as usize;
        debug_assert!(
            n < MAX_SEEDED_WRAPPERS,
            "framegraph: more than {MAX_SEEDED_WRAPPERS} SeededImage/SeededBuffer declarations in \
             one frame — raise MAX_SEEDED_WRAPPERS (the registry is a fixed inline array)"
        );
        self.wrappers[n] = WrapperSlot { res, init_seen: false };
        self.wrapper_count += 1;
        n as u8
    }

    pub(crate) fn register_restored(&mut self, res: ResId, site: &'static str) {
        let n = self.restored_count as usize;
        debug_assert!(
            n < MAX_RESTORED_IMAGES,
            "framegraph: more than {MAX_RESTORED_IMAGES} add_image_seeded_restored opt-outs in one \
             frame — raise MAX_RESTORED_IMAGES"
        );
        self.restored[n] = (res, site);
        self.restored_count += 1;
    }

    pub(crate) fn wrappers(&self) -> &[WrapperSlot] {
        &self.wrappers[..self.wrapper_count as usize]
    }

    pub(crate) fn slot_mut(&mut self, slot: u8) -> &mut WrapperSlot {
        &mut self.wrappers[slot as usize]
    }

    pub(crate) fn is_restored(&self, res: ResId) -> bool {
        self.restored[..self.restored_count as usize].iter().any(|&(r, _)| r == res)
    }
}

// --- the invariants, as pure functions returning a violation by value -----------------------

/// A seeded image that ends the frame, on some mip, in a layout other than its seed layout. The
/// next frame's `compile` re-creates the start state from the static seed and nothing restores
/// layouts between frames, so the next frame's first barrier would claim a false `oldLayout`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeededLayoutViolation {
    pub res: ResId,
    pub name: &'static str,
    pub mip: u32,
    pub seed_layout: i32,
    pub end_layout: i32,
}

/// Which half of the trailing-scope rule a resource broke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrailingScopeRule {
    /// A stage of the trailing scope lies outside the seed's stages.
    ScopeOutsideSeed,
    /// The frame ends on a write while the seed is a reader seed (no `flush_stages`).
    EndsOnWriteUnderReaderSeed,
}

/// A wrapper-declared resource whose frame-ending accesses the seed does not cover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrailingScopeViolation {
    pub res: ResId,
    pub name: &'static str,
    pub mip: u32,
    pub rule: TrailingScopeRule,
    pub scope_stages: u32,
    pub seed_stages: u32,
}

/// The seeded-layout invariant over every image of `g` (call after `compile`): every image
/// declared with a layout seed ends the frame, on every mip, in that layout — unless it was
/// declared through [`FrameGraph::add_image_seeded_restored`] (debug builds only; a release build
/// has no opt-out list and checks every such image).
pub fn seeded_layout_violation(g: &FrameGraph) -> Option<SeededLayoutViolation> {
    for i in 0..g.res_count() {
        let res = ResId(i as u16);
        let seed = g.res_seed(res);
        if !g.res_is_image(res) || seed.layout == VK_IMAGE_LAYOUT_UNDEFINED {
            continue;
        }
        #[cfg(debug_assertions)]
        if g.obs_registry().is_restored(res) {
            continue;
        }
        for mip in 0..g.res_mips(res) {
            let end_layout = g.resolved_layout_mip(res, mip);
            if end_layout != seed.layout {
                return Some(SeededLayoutViolation {
                    res,
                    name: g.res_name(res),
                    mip,
                    seed_layout: seed.layout,
                    end_layout,
                });
            }
        }
    }
    None
}

/// The trailing scope of mip `mip` of `res`: `(stages, ends_on_write)`, or `None` when no access
/// of this frame touches that mip. Walks the flat access arena backwards, O(accesses).
pub fn trailing_scope(g: &FrameGraph, res: ResId, mip: u32) -> Option<(u32, bool)> {
    let mut scope = 0u32;
    let mut touched = false;
    for p in (0..g.pass_count()).rev() {
        for a in g.pass_accesses(super::ids::PassId(p as u16)).rev() {
            if a.res != res || !sub_covers(a.sub, mip) {
                continue;
            }
            let is_write = a.access & WRITE_ACCESS_MASK != 0;
            if is_write {
                return Some(if touched { (scope, false) } else { (a.stage, true) });
            }
            scope |= a.stage;
            touched = true;
        }
    }
    touched.then_some((scope, false))
}

#[inline]
const fn sub_covers(sub: SubRange, mip: u32) -> bool {
    mip >= sub.base_mip && mip - sub.base_mip < sub.mip_count
}

/// The trailing-scope check for one resource, every mip: the first mip whose trailing scope the
/// seed does not cover.
pub fn trailing_scope_violation_of(g: &FrameGraph, res: ResId) -> Option<TrailingScopeViolation> {
    let seed = g.res_seed(res);
    let covered = seed_stages(seed);
    for mip in 0..g.res_mips(res) {
        let Some((scope, ends_on_write)) = trailing_scope(g, res, mip) else {
            continue;
        };
        let rule = if ends_on_write && seed.flush_stages == 0 {
            TrailingScopeRule::EndsOnWriteUnderReaderSeed
        } else if scope & !covered != 0 {
            TrailingScopeRule::ScopeOutsideSeed
        } else {
            continue;
        };
        return Some(TrailingScopeViolation {
            res,
            name: g.res_name(res),
            mip,
            rule,
            scope_stages: scope,
            seed_stages: covered,
        });
    }
    None
}

/// The trailing-scope check over every wrapper-declared resource of `g` (debug builds: the
/// registry exists only there).
#[cfg(debug_assertions)]
pub fn trailing_scope_violation(g: &FrameGraph) -> Option<TrailingScopeViolation> {
    g.obs_registry()
        .wrappers()
        .iter()
        .find_map(|w| trailing_scope_violation_of(g, w.res))
}

#[cold]
#[inline(never)]
pub(crate) fn fire_seeded_layout(v: SeededLayoutViolation) -> ! {
    panic!(
        "framegraph: INVARIANT SEEDED-LAYOUT violated — image '{}' (mip {}) was declared with the \
         seed layout {} but ends the frame in layout {}. The next frame's compile re-creates its \
         start state from the static seed and nothing restores layouts between frames, so its \
         first barrier would claim a false oldLayout (the recorded-vs-actual divergence the C2 \
         fix calls spec-UB). End the frame in the seed layout (declare a final read at the seed \
         layout), or, for a legitimate restore outside the graph, declare the image with \
         add_image_seeded_restored(name, seed, \"<restoring site>\")",
        v.name, v.mip, v.seed_layout, v.end_layout
    )
}

#[cold]
#[inline(never)]
pub(crate) fn fire_trailing_scope(v: TrailingScopeViolation) -> ! {
    match v.rule {
        TrailingScopeRule::ScopeOutsideSeed => panic!(
            "framegraph: SEED COVERAGE violated — the trailing scope of '{}' (mip {}) holds stages \
             {:#x}, but its seed covers only {:#x}. The trailing scope is the accesses after the \
             frame's last write (or that write, or every access on a frame without one); the next \
             frame's first access orders only after the seed's stages, so a trailing access \
             outside them races it (a cross-frame WAR/WAW gap validation cannot see). Widen the \
             seed to the trailing scope",
            v.name, v.mip, v.scope_stages, v.seed_stages
        ),
        TrailingScopeRule::EndsOnWriteUnderReaderSeed => panic!(
            "framegraph: SEED COVERAGE violated — '{}' (mip {}) ENDS THE FRAME ON A WRITE (stages \
             {:#x}) but is declared with a READER seed (stages {:#x}, no flush stages). The next \
             frame's first access would order after the sibling's reads only and never make the \
             sibling's write available. Declare a writer seed, or end the frame on a read",
            v.name, v.mip, v.scope_stages, v.seed_stages
        ),
    }
}

// --- the wrappers ------------------------------------------------------------------------------

/// A single cross-frame-shared IMAGE declared with its class (H0 Scope 4): the one declare site
/// every new seeded image goes through, so the trailing-scope check and (for
/// [`SeedContent::FrameLocal`]) the initialising-access discipline cover it.
///
/// `Copy`, like the `MaterialTableRes` shape it generalises: the per-frame init bit lives in the
/// graph's debug registry, keyed by the declaration, not in the value.
#[derive(Clone, Copy, Debug)]
pub struct SeededImage {
    id: ResId,
    content: SeedContent,
    seed_stages: u32,
    #[cfg(debug_assertions)]
    slot: u8,
}

/// A single cross-frame-shared BUFFER declared with its class — see [`SeededImage`].
#[derive(Clone, Copy, Debug)]
pub struct SeededBuffer {
    id: ResId,
    content: SeedContent,
    seed_stages: u32,
    #[cfg(debug_assertions)]
    slot: u8,
}

impl SeededImage {
    /// Declares `name` with `mips` levels and `seed` (widened with `TRANSFER` when `dump_widen`),
    /// as [`SeedContent::CrossFrame`] (`add_image_seeded` / `add_image_mipped`) or
    /// [`SeedContent::FrameLocal`] (the frame-local twins), and registers it for the debug checks.
    ///
    /// # Panics
    ///
    /// If `mips == 0` (as `add_image_mipped` does), in every profile.
    pub fn declare(
        g: &mut FrameGraph,
        name: &'static str,
        seed: ResSync,
        mips: u32,
        content: SeedContent,
        dump_widen: bool,
    ) -> Self {
        let seed = widen_for_dump(seed, dump_widen);
        let id = match (content, mips) {
            (SeedContent::CrossFrame, 1) => g.add_image_seeded(name, seed),
            (SeedContent::CrossFrame, _) => g.add_image_mipped(name, mips, seed),
            (SeedContent::FrameLocal, 1) => g.add_image_seeded_frame_local(name, seed),
            (SeedContent::FrameLocal, _) => g.add_image_mipped_frame_local(name, mips, seed),
        };
        #[cfg(debug_assertions)]
        let slot = g.obs_registry_mut().register_wrapper(id);
        Self {
            id,
            content,
            seed_stages: seed_stages(seed),
            #[cfg(debug_assertions)]
            slot,
        }
    }

    /// The declared resource.
    #[inline]
    pub fn id(self) -> ResId {
        self.id
    }

    /// A colour or depth ATTACHMENT access that initialises the image (its recorder clears on
    /// load); it may also blend (`READ | WRITE`). The only kind, with the other `init_*`, a
    /// FrameLocal image may take as its first access of the frame.
    pub fn init_attachment(
        self,
        g: &mut FrameGraph,
        stage: u32,
        access: u32,
        layout: i32,
        sub: SubRange,
    ) {
        self.mark_init(g);
        g.image_access(self.id, stage, access, layout, sub);
    }

    /// A whole-image `vkCmdClearColorImage` at `TRANSFER` (layout `GENERAL` or
    /// `TRANSFER_DST_OPTIMAL`).
    pub fn init_clear(self, g: &mut FrameGraph, layout: i32, sub: SubRange) {
        self.mark_init(g);
        g.image_access(self.id, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, layout, sub);
    }

    /// A `vkCmdCopyImage` whose region covers the whole destination.
    pub fn init_copy_full(self, g: &mut FrameGraph, layout: i32, sub: SubRange) {
        self.mark_init(g);
        g.image_access(self.id, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, layout, sub);
    }

    /// A compute storage write whose dispatch covers the whole extent (layout `GENERAL`).
    pub fn init_storage_full(self, g: &mut FrameGraph, stage: u32, sub: SubRange) {
        self.mark_init(g);
        g.image_access(self.id, stage, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, sub);
    }

    /// A read. FrameLocal: an `init_*` access must precede it this frame. CrossFrame: the stage
    /// must lie inside the seed (the `MaterialTableRes::read` per-read assert).
    pub fn read(self, g: &mut FrameGraph, stage: u32, access: u32, layout: i32, sub: SubRange) {
        self.check_read(g, stage, "read");
        g.image_access(self.id, stage, access, layout, sub);
    }

    /// A read-modify-write (e.g. a `LOAD`-ing blend). FrameLocal: an `init_*` access must precede
    /// it this frame, even though it carries a WRITE bit `compile`'s guard would accept.
    pub fn rmw(self, g: &mut FrameGraph, stage: u32, access: u32, layout: i32, sub: SubRange) {
        self.check_rmw(g, "rmw");
        g.image_access(self.id, stage, access, layout, sub);
    }

    /// A write of a CrossFrame image (its content persists; no initialising discipline).
    pub fn write(self, g: &mut FrameGraph, stage: u32, access: u32, layout: i32, sub: SubRange) {
        debug_assert_eq!(
            self.content,
            SeedContent::CrossFrame,
            "invariant (OBS): a FrameLocal SeededImage '{}' is written through an init_* access, \
             never a bare write",
            g.res_name(self.id)
        );
        g.image_access(self.id, stage, access, layout, sub);
    }

    #[inline]
    fn mark_init(self, g: &mut FrameGraph) {
        debug_assert_eq!(
            self.content,
            SeedContent::FrameLocal,
            "invariant (OBS): init_* is the FrameLocal initialising access; '{}' is CrossFrame",
            g.res_name(self.id)
        );
        #[cfg(debug_assertions)]
        {
            g.obs_registry_mut().slot_mut(self.slot).init_seen = true;
        }
    }

    #[inline]
    fn check_read(self, g: &FrameGraph, stage: u32, what: &'static str) {
        match self.content {
            SeedContent::FrameLocal => self.check_rmw(g, what),
            SeedContent::CrossFrame => debug_assert_eq!(
                stage & !self.seed_stages,
                0,
                "invariant (OBS): a reader of '{}' at stage {stage:#x} lies outside its \
                 cross-frame seed {:#x} — next frame's first write would not wait for this read",
                g.res_name(self.id),
                self.seed_stages
            ),
        }
    }

    #[inline]
    fn check_rmw(self, g: &FrameGraph, what: &'static str) {
        #[cfg(debug_assertions)]
        if self.content == SeedContent::FrameLocal {
            let seen = g.obs_registry().wrappers()[self.slot as usize].init_seen;
            debug_assert!(
                seen,
                "invariant (OBS): INITIALISING ACCESS missing — FrameLocal '{}' is touched by \
                 {what} before any init_* access this frame. Its content is rebuilt every frame, \
                 so the first access must be its declared initialiser (init_attachment with \
                 loadOp CLEAR, init_clear, init_copy_full or init_storage_full); a LOAD-ing \
                 blend would accumulate across frames while compile's guard accepts its WRITE bit",
                g.res_name(self.id)
            );
        }
        let _ = (g, what);
    }
}

impl SeededBuffer {
    /// Declares `name` with `seed` (widened with `TRANSFER` when `dump_widen`), as
    /// [`SeedContent::CrossFrame`] (`add_buffer_seeded`) or [`SeedContent::FrameLocal`]
    /// (`add_buffer_seeded_frame_local`), and registers it for the debug checks.
    pub fn declare(
        g: &mut FrameGraph,
        name: &'static str,
        seed: ResSync,
        content: SeedContent,
        dump_widen: bool,
    ) -> Self {
        let seed = widen_for_dump(seed, dump_widen);
        let id = match content {
            SeedContent::CrossFrame => g.add_buffer_seeded(name, seed),
            SeedContent::FrameLocal => g.add_buffer_seeded_frame_local(name, seed),
        };
        #[cfg(debug_assertions)]
        let slot = g.obs_registry_mut().register_wrapper(id);
        Self {
            id,
            content,
            seed_stages: seed_stages(seed),
            #[cfg(debug_assertions)]
            slot,
        }
    }

    /// The declared resource.
    #[inline]
    pub fn id(self) -> ResId {
        self.id
    }

    /// The seed's stages (visible ∪ flush), after any dump widening.
    #[inline]
    pub fn seed_stages(self) -> u32 {
        self.seed_stages
    }

    /// A whole-buffer `vkCmdFillBuffer` at `TRANSFER` — a FrameLocal initialising access.
    pub fn init_clear(self, g: &mut FrameGraph) {
        self.mark_init(g);
        g.buffer_access(self.id, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    }

    /// A `vkCmdCopyBuffer` whose regions cover the whole buffer — a FrameLocal initialising
    /// access.
    pub fn init_copy_full(self, g: &mut FrameGraph) {
        self.mark_init(g);
        g.buffer_access(self.id, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    }

    /// A compute storage write whose dispatch covers the whole buffer — a FrameLocal
    /// initialising access.
    pub fn init_storage_full(self, g: &mut FrameGraph, stage: u32) {
        self.mark_init(g);
        g.buffer_access(self.id, stage, VK_ACCESS_SHADER_WRITE_BIT);
    }

    /// A read. FrameLocal: an `init_*` access must precede it this frame. CrossFrame: the stage
    /// must lie inside the seed.
    pub fn read(self, g: &mut FrameGraph, stage: u32, access: u32) {
        match self.content {
            SeedContent::FrameLocal => self.check_init(g, "read"),
            SeedContent::CrossFrame => debug_assert_eq!(
                stage & !self.seed_stages,
                0,
                "invariant (OBS): a reader of '{}' at stage {stage:#x} lies outside its \
                 cross-frame seed {:#x} — next frame's first write would not wait for this read",
                g.res_name(self.id),
                self.seed_stages
            ),
        }
        g.buffer_access(self.id, stage, access);
    }

    /// A read-modify-write (atomics). FrameLocal: an `init_*` access must precede it this frame.
    pub fn rmw(self, g: &mut FrameGraph, stage: u32, access: u32) {
        self.check_init(g, "rmw");
        g.buffer_access(self.id, stage, access);
    }

    /// A write of a CrossFrame buffer (the `material_upload` copy).
    pub fn write(self, g: &mut FrameGraph, stage: u32, access: u32) {
        debug_assert_eq!(
            self.content,
            SeedContent::CrossFrame,
            "invariant (OBS): a FrameLocal SeededBuffer '{}' is written through an init_* access, \
             never a bare write",
            g.res_name(self.id)
        );
        g.buffer_access(self.id, stage, access);
    }

    #[inline]
    fn mark_init(self, g: &mut FrameGraph) {
        debug_assert_eq!(
            self.content,
            SeedContent::FrameLocal,
            "invariant (OBS): init_* is the FrameLocal initialising access; '{}' is CrossFrame",
            g.res_name(self.id)
        );
        #[cfg(debug_assertions)]
        {
            g.obs_registry_mut().slot_mut(self.slot).init_seen = true;
        }
    }

    #[inline]
    fn check_init(self, g: &FrameGraph, what: &'static str) {
        #[cfg(debug_assertions)]
        if self.content == SeedContent::FrameLocal {
            let seen = g.obs_registry().wrappers()[self.slot as usize].init_seen;
            debug_assert!(
                seen,
                "invariant (OBS): INITIALISING ACCESS missing — FrameLocal '{}' is touched by \
                 {what} before any init_* access this frame. Its content is rebuilt every frame, \
                 so the first access must be its declared initialiser (init_clear, init_copy_full \
                 or init_storage_full)",
                g.res_name(self.id)
            );
        }
        let _ = (g, what);
    }
}
