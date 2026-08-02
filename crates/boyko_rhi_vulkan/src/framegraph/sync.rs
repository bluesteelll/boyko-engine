//! The per-resource Vulkan synchronization state machine (Granite-style, D2) and
//! the Vk-valued, `Copy`/`Eq` barrier PODs the compiler derives.
//!
//! Each frame-graph resource carries a running [`ResSync`] state
//! `{layout, pending-write access + stages, made-visible access + stages}`. A
//! pass access consults it via [`transition`], which emits a barrier IFF a
//! genuine hazard requires one — a layout mismatch, a read-after-write that must
//! be flushed + made visible, a write-after-read/write that must be ordered, or a
//! read at a stage/access the last flush never reached. Otherwise the access is
//! free (an already-visible read). This is the minimal-barrier core (D2): over
//! UE5-style dependency-levels it never over-synchronizes reads.
//!
//! Barriers are **Vk-VALUED** (raw `VkPipelineStageFlags`/`VkAccessFlags`/
//! `VkImageLayout`), but reference resources by the logical [`ResId`], not a raw
//! `VkImage` — the handle is bound at RECORD time (Step 1c), so the same derived
//! plan is valid for any per-frame physical slot (Step 1d).

use super::ids::ResId;
use crate::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_IMAGE_ASPECT_COLOR_BIT,
    VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
};

/// The union of every `VkAccessFlags` WRITE bit the frame path can perform. An
/// access intersecting this is a WRITE (leaves a flush-pending hazard); anything
/// else is a pure READ. (No `MEMORY_WRITE`/`ALL` — only bits the frame uses.)
pub const WRITE_ACCESS_MASK: u32 = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
    | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT
    | VK_ACCESS_SHADER_WRITE_BIT
    | VK_ACCESS_TRANSFER_WRITE_BIT;

/// A Vk-valued image subresource range (aspect + mip span + array-layer span),
/// `Copy`/`Eq` so derived barriers diff cleanly. Layered ranges (CSM/atlas depth,
/// W3) are first-class from Step 1b via [`SubRange::depth_layers`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SubRange {
    /// `VkImageAspectFlags` (COLOR or DEPTH here).
    pub aspect: u32,
    pub base_mip: u32,
    pub mip_count: u32,
    pub base_layer: u32,
    pub layer_count: u32,
}

impl SubRange {
    /// The single-layer COLOR range (mirrors `COLOR_SUBRESOURCE_RANGE`).
    pub const COLOR: Self = Self {
        aspect: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };
    /// The single-layer DEPTH range (mirrors `DEPTH_SUBRESOURCE_RANGE`).
    pub const DEPTH: Self = Self {
        aspect: VK_IMAGE_ASPECT_DEPTH_BIT,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };

    /// A whole-array DEPTH range over `[0, layers)` — the CSM cascade / spot atlas
    /// layered depth pass (W3: subresource ranges are first-class, not a hack).
    #[inline]
    pub const fn depth_layers(layers: u32) -> Self {
        Self {
            aspect: VK_IMAGE_ASPECT_DEPTH_BIT,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: layers,
        }
    }

    /// A whole-array COLOR range over `[0, layers)` — the SDFDDGI I2 probe-update pass's
    /// storage WRITE covers ALL `DDGI_ATLAS_LAYERS` array layers of each atlas (the boot
    /// SHADER_READ_ONLY_OPTIMAL → GENERAL transition + the update→resolve barrier must span
    /// every layer, since the resolve samples the whole array). The COLOR analogue of
    /// [`depth_layers`](Self::depth_layers).
    #[inline]
    pub const fn color_layers(layers: u32) -> Self {
        Self {
            aspect: VK_IMAGE_ASPECT_COLOR_BIT,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: layers,
        }
    }

    /// A whole-MIP-CHAIN COLOR range over `[0, mips)`, one array layer — the range a pass
    /// that touches an entire mipped image declares (VG R3: the HZB depth pyramid, whose
    /// build seeds mip 0 and whose consumers sample the chain). The mip analogue of
    /// [`color_layers`](Self::color_layers) — and the first constructor here that varies
    /// `mip_count` at all; every other range in this module pins it to 1.
    ///
    /// The range is copied VERBATIM into the derived [`ImgBarrier`] by
    /// [`FrameGraph::compile`](super::graph::FrameGraph::compile), so this constructor is
    /// the whole of what a whole-chain access needs to emit a correct barrier. It does NOT
    /// make the sync state machine subresource-aware: [`transition`] is keyed per resource
    /// and never reads a `SubRange`. The boundary that keeps that sound is
    /// `INVARIANT HZB-SUBRESOURCE-UNIFORM`, documented and mechanically checked in
    /// `FrameGraph::compile`.
    #[inline]
    pub const fn color_mips(mips: u32) -> Self {
        Self {
            aspect: VK_IMAGE_ASPECT_COLOR_BIT,
            base_mip: 0,
            mip_count: mips,
            base_layer: 0,
            layer_count: 1,
        }
    }

    /// `true` iff `self` and `other` select the SAME set of subresources — equal
    /// `(base_mip, mip_count, base_layer, layer_count)`.
    ///
    /// `aspect` is deliberately NOT compared. The four span fields are what a per-ResId
    /// tracked layout can be wrong ABOUT (two passes touching different mips/layers of one
    /// image), which is the question `INVARIANT HZB-SUBRESOURCE-UNIFORM` in
    /// [`FrameGraph::compile`](super::graph::FrameGraph::compile) is stated over. Aspect
    /// selects which PLANES a barrier covers and is a separate axis this predicate makes no
    /// claim about; today no resource declares two aspects (color images declare
    /// [`COLOR`](Self::COLOR)/[`color_layers`](Self::color_layers), depth images
    /// [`DEPTH`](Self::DEPTH)/[`depth_layers`](Self::depth_layers), and buffers always the
    /// `COLOR` placeholder).
    /// `cfg(debug_assertions)` + `pub(crate)`: its only caller is the debug-only invariant
    /// check, so it is not permanent public API. A `pub` spelling would have made it one
    /// forever in exchange for silencing a dead-code warning that this attribute removes at
    /// the source.
    #[cfg(debug_assertions)]
    #[inline]
    pub(crate) const fn same_span(&self, other: &Self) -> bool {
        self.base_mip == other.base_mip
            && self.mip_count == other.mip_count
            && self.base_layer == other.base_layer
            && self.layer_count == other.layer_count
    }
}

/// A derived image-memory barrier (Vk-valued, resource-logical). Lowered to a
/// `VkImageMemoryBarrier` at record time by binding `res` → the current physical
/// `VkImage`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImgBarrier {
    pub res: ResId,
    pub src_stage: u32,
    pub dst_stage: u32,
    pub src_access: u32,
    pub dst_access: u32,
    pub old_layout: i32,
    pub new_layout: i32,
    pub subresource: SubRange,
}

/// A derived buffer-memory barrier (Vk-valued, resource-logical). Buffers have no
/// layout; ordering is driven purely by the flush/visibility hazards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufBarrier {
    pub res: ResId,
    pub src_stage: u32,
    pub dst_stage: u32,
    pub src_access: u32,
    pub dst_access: u32,
}

/// The running synchronization state of one resource across the frame. Reset to
/// [`ResSync::undefined`] each compile (transient/history resources are re-
/// `UNDEFINED`'d per frame — the prior content is discarded before its producer).
#[derive(Clone, Copy, Debug)]
pub struct ResSync {
    /// Current `VkImageLayout` (buffers keep the UNDEFINED sentinel forever).
    pub layout: i32,
    /// Write access performed but NOT yet flushed (made available). 0 = clean.
    pub flush_access: u32,
    /// Pipeline stages that produced `flush_access` (the barrier's src stage).
    pub flush_stages: u32,
    /// Access already made visible by prior barriers, in `visible_stages`.
    pub visible_access: u32,
    /// Pipeline stages the visible access has been made visible to.
    pub visible_stages: u32,
}

impl ResSync {
    /// Fresh state for a transient/history image (layout UNDEFINED, no pending).
    #[inline]
    pub const fn undefined() -> Self {
        Self {
            layout: VK_IMAGE_LAYOUT_UNDEFINED,
            flush_access: 0,
            flush_stages: 0,
            visible_access: 0,
            visible_stages: 0,
        }
    }

    /// Cross-frame seed for a NON-RINGED resource whose SIBLING in-flight frame
    /// ends with READS at `(stages, access)` (light table / tiles / cluster
    /// grid+index / CSM cascade / shadow atlas — each ends its frame consumed by
    /// the resolve or marcher). This frame's first WRITE then derives a WAR
    /// execution dependency (`src = stages`, no availability — the src is a read)
    /// ordering it after those still-pipelined reads; a first READ at a covered
    /// stage+access stays FREE (already visible), exactly as it is within a
    /// frame. Layout stays UNDEFINED — content is re-rendered, only ordering
    /// matters (audit B-002/B-003).
    #[inline]
    pub const fn seeded_readers(stages: u32, access: u32) -> Self {
        Self {
            layout: VK_IMAGE_LAYOUT_UNDEFINED,
            flush_access: 0,
            flush_stages: 0,
            visible_access: access,
            visible_stages: stages,
        }
    }

    /// Cross-frame seed for a NON-RINGED, CONTENT-PERSISTENT image whose start-of-frame
    /// layout is a REAL layout (NOT re-rendered from scratch), and whose sibling in-flight
    /// frame ends with READS at `(stages, access)`. Unlike [`seeded_readers`](Self::seeded_readers)
    /// — which leaves the layout UNDEFINED because its resource is re-rendered every frame
    /// (content-discard legal) — this seeds the ACTUAL `layout` so the first write's derived
    /// transition is `layout → GENERAL`, PRESERVING the existing contents (Vulkan discards on a
    /// `UNDEFINED` oldLayout, which would wipe a persistent accumulator).
    ///
    /// The SDFDDGI I2 probe atlases use this: they are boot-initialized to
    /// `SHADER_READ_ONLY_OPTIMAL` and are PERSISTENT accumulators (Decision D2 — the round-robin
    /// update writes only 1/N tiles per frame; the other (N-1)/N MUST survive), so the first
    /// storage write each frame needs a content-preserving `SHADER_READ_ONLY_OPTIMAL → GENERAL`
    /// transition, not a discarding `UNDEFINED → GENERAL` (plan §2.5/§7). The reader-visibility
    /// half still orders this frame's write after the sibling's still-pipelined resolve reads
    /// (the WAR seed).
    #[inline]
    pub const fn seeded_readers_at_layout(layout: i32, stages: u32, access: u32) -> Self {
        Self {
            layout,
            flush_access: 0,
            flush_stages: 0,
            visible_access: access,
            visible_stages: stages,
        }
    }

    /// Cross-frame seed for a NON-RINGED resource whose SIBLING frame ends with
    /// an UNDRAINED WRITE at `(stages, access)` — no same-frame read follows to
    /// flush it (the cluster `alloc` counter: the cull atomics are its last
    /// touch). This frame's first access then derives a full memory dependency
    /// (`src = stages/access`) — the WAW/RAW ordering + availability the sibling
    /// write needs. `access` must be a WRITE bit.
    #[inline]
    pub const fn seeded_writer(stages: u32, access: u32) -> Self {
        Self {
            layout: VK_IMAGE_LAYOUT_UNDEFINED,
            flush_access: access,
            flush_stages: stages,
            visible_access: 0,
            visible_stages: 0,
        }
    }

    /// Cross-frame seed for a NON-RINGED, CONTENT-PERSISTENT image whose start-of-frame layout is a
    /// REAL layout (NOT re-rendered) and whose SIBLING in-flight frame ends with an UNDRAINED WRITE
    /// at `(stages, access)`. This frame's first access then derives a full memory dependency
    /// (`src = stages/access`) — the RAW/WAW ordering + availability the sibling write needs —
    /// WITHOUT discarding content (unlike [`seeded_writer`](Self::seeded_writer), whose UNDEFINED
    /// layout would wipe a persistent accumulator). `access` must be a WRITE bit.
    ///
    /// This is the content-preserving RAW analogue of [`seeded_readers_at_layout`](Self::seeded_readers_at_layout)
    /// (which is the content-preserving WAR seed). The Rung-3b temporal shadow-vis history uses it on
    /// the READ image of its cross-frame ping-pong: the sibling frame WROTE that physical image, and
    /// this frame's FIRST access is a READ, so a genuine `SHADER_WRITE → SHADER_READ` availability +
    /// visibility barrier is required (the reader WAR seed would leave the read FREE/already-visible,
    /// which is exactly the race). The `layout` is preserved (GENERAL for life). Ordering reaches the
    /// sibling's still-pipelined write via single-queue submission order.
    #[inline]
    pub const fn seeded_writer_at_layout(layout: i32, stages: u32, access: u32) -> Self {
        Self {
            layout,
            flush_access: access,
            flush_stages: stages,
            visible_access: 0,
            visible_stages: 0,
        }
    }
}

/// The two halves of a required barrier + its layout transition, returned by
/// [`transition`]. The caller wraps it into an [`ImgBarrier`] (with subresource)
/// or a [`BufBarrier`] (dropping the layout).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trans {
    pub src_stage: u32,
    pub dst_stage: u32,
    pub src_access: u32,
    pub dst_access: u32,
    pub old_layout: i32,
    pub new_layout: i32,
}

/// Advance `state` by one pass access `(stage, access, layout)` and return
/// `Some(Trans)` iff a barrier is required, else `None` (a free, already-visible
/// read). For buffers the caller passes `layout == state.layout` so the layout
/// arm never fires.
///
/// Minimality (D2): a barrier fires only on a real hazard —
/// - **layout mismatch** (`layout != state.layout`), or
/// - **RAW/WAW** (`state.flush_access != 0`: a pending write to flush), or
/// - **WAR** (this access writes and prior reads must be ordered before it), or
/// - **visibility extension** (this read touches a stage/access the last flush
///   never reached).
///
/// The src half is the pending write (RAW/WAW), or an execution-only dependency
/// on prior reads (WAR / visibility extend), or `TOP_OF_PIPE`/0 on first touch.
pub fn transition(state: &mut ResSync, stage: u32, access: u32, layout: i32) -> Option<Trans> {
    let is_write = access & WRITE_ACCESS_MASK != 0;
    let layout_change = layout != state.layout;

    let need = if is_write {
        // A write must order after any prior producer (flush) or reader (visible),
        // and transition the layout. A first-touch write to a fresh IMAGE always
        // changes layout (UNDEFINED→X); a first-touch BUFFER write has no hazard
        // yet and correctly emits NO barrier — but still records its pending flush
        // below (uniform state advance), so the next reader gets a real RAW.
        layout_change || state.flush_access != 0 || state.visible_stages != 0
    } else {
        // A read is free only when the layout already matches AND there is no
        // pending write to flush AND this exact stage+access is already visible.
        layout_change
            || state.flush_access != 0
            || (stage & !state.visible_stages != 0)
            || (access & !state.visible_access != 0)
    };

    // Derive the barrier (if needed) from the CURRENT state, BEFORE advancing.
    let result = if need {
        let (src_stage, src_access) = if state.flush_access != 0 {
            // RAW / WAW: flush + make the pending write available.
            (state.flush_stages, state.flush_access)
        } else if state.visible_stages != 0 {
            // WAR / visibility extend: an execution dependency on the prior
            // readers (no memory to make available — the src is a read).
            (state.visible_stages, 0)
        } else {
            // First touch: nothing to make available; TOP_OF_PIPE is the sound src.
            (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, 0)
        };
        Some(Trans {
            src_stage,
            dst_stage: stage,
            src_access,
            dst_access: access,
            old_layout: state.layout,
            new_layout: layout,
        })
    } else {
        None
    };

    // ALWAYS advance the state — whether or not a barrier was emitted. This is the
    // correctness keystone: a first-touch write emits no barrier yet MUST leave a
    // pending flush so its reader sees a real availability (RAW) hazard, not a bare
    // execution dependency (`src_access = 0` = stale-read UB).
    state.layout = layout;
    if is_write {
        state.flush_access = access & WRITE_ACCESS_MASK;
        state.flush_stages = stage;
        state.visible_access = 0;
        state.visible_stages = 0;
    } else {
        // A read (barriered or free) clears the pending flush — it is now visible to
        // this read — and accumulates the covered (stage, access) monotonically.
        state.flush_access = 0;
        state.flush_stages = 0;
        state.visible_access |= access;
        state.visible_stages |= stage;
    }

    result
}
