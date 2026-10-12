//! Render wave RD1 (defect R0b): **the transparent-depth rule as data and pure functions** — which
//! consumer classes can appear on a boot, which are declared on a frame, when the SDF depth merge
//! is declared, and the two debug guards that keep the merge and its consumers paired.
//!
//! # Two layers (the rung's arming rule)
//!
//! - [`TransparentDepthCapability`] — **boot-frozen**: a class bit is set iff the boot built that
//!   class's GPU bundle (the `ParticleConfig::mode` → `particle: Option<…>` idiom), never because a
//!   plugin or resource merely exists. It decides the marcher's VIEWT variant
//!   ([`sdf_forward_writes_viewt`]) and whether the merge's GPU objects exist at all.
//! - [`TransparentDepthFrame`] — **per frame**: a class bit is set iff that class's pass is
//!   declared this frame, read from the SAME per-frame field the consumer's own declaration reads.
//!   Always a subset of the capability ([`frame_within_capability`]), so a consumer that turns on
//!   mid-run on an armed boot finds the marcher already writing `t`.
//!
//! RD1 defines bit [`TRANSPARENT_CLASS_PARTICLES`] only; R1 (translucent meshes), FX9 (weather) and
//! FX12 (decals) each add their bit, their frame field and their pairing row.
//!
//! # The merge predicate ([`sdf_depth_merge`])
//!
//! `Draw` iff the frame has an SDF leg, no HZB dump pass is declared, some consumer class is
//! declared, and the test-only `disarm_sdf_merge` key is not armed. Under that key the
//! visibility-buffer SDF-only leg keeps a CLEAR-only pass ([`SdfDepthMerge::ClearOnly`]): there
//! the merge is the depth image's first writer, and removing it would make `particle_draw` read an
//! unwritten transient — the red control could not render in a dev build.
//!
//! # The guards (classification by DECLARED ACCESS, never by name)
//!
//! - [`transparent_depth_order_violation`] — a pass declared after the frame's last SDF producer
//!   whose access to the path depth is a depth-attachment READ without WRITE, or a fragment-shader
//!   read, is a depth-tested transparent consumer; it must come after a declared merge. A later
//!   rung's consumer is classified with no registration step. Frames that declare the HZB dump, and
//!   frames under the disarm key, are exempt by name.
//! - [`transparent_depth_pairing_violation`] — every consumer class is declared iff its frame bit
//!   is set: the "consumer narrowed, merge not" direction the order guard cannot see.
//! - [`merge_clear_violation`] — the merge recorder's `CLEAR` load op is tied to the pass's
//!   derived first touch of the path depth (an `UNDEFINED` old layout), so a recorder that loads a
//!   first-touched depth, or clears a written one, is a debug red.
//!
//! The `debug_*` wrappers exist only under `debug_assertions`; their panic paths are `#[cold]`.
//! Every pure function is available in every build, so a host test asserts the rule itself in a
//! release test build too.

use crate::ffi::{
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
};
use crate::framegraph::{FrameGraph, PassId, ResId};

/// The bit index of the particle class (`particle_draw`, Particles P0). The first and, at RD1,
/// only consumer class.
pub const TRANSPARENT_CLASS_PARTICLES: u8 = 0;

/// The boot-frozen set of depth-tested transparent consumer classes a boot CAN declare. See the
/// module doc for why a bit is an arming value and never a resource's existence.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TransparentDepthCapability(u8);

impl TransparentDepthCapability {
    /// No consumer class armed — every default `EnginePlugins` boot.
    pub const NONE: Self = Self(0);
    /// The particle class alone.
    pub const PARTICLES: Self = Self(1 << TRANSPARENT_CLASS_PARTICLES);

    /// The raw bitset (bit `k` = class `k`).
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// `true` iff some class is armed.
    #[inline]
    #[must_use]
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// `true` iff class `class` is armed.
    #[inline]
    #[must_use]
    pub const fn has(self, class: u8) -> bool {
        self.0 & (1 << class) != 0
    }

    /// `self` with class `class` armed iff `armed` — the builder each class's bundle site calls.
    #[inline]
    #[must_use]
    pub const fn with(self, class: u8, armed: bool) -> Self {
        if armed { Self(self.0 | (1 << class)) } else { self }
    }
}

/// The per-frame set of depth-tested transparent consumer classes DECLARED this frame. Same bit
/// layout as [`TransparentDepthCapability`]; always a subset of it.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TransparentDepthFrame(u8);

impl TransparentDepthFrame {
    /// No consumer declared this frame.
    pub const NONE: Self = Self(0);

    /// The frame set `bits` on a boot of capability `cap`.
    ///
    /// # Panics
    /// In debug builds, when `bits` names a class `cap` does not arm — a consumer declared on a
    /// boot whose marcher never writes `t` would merge a stale `gViewT`.
    #[inline]
    #[must_use]
    pub fn within(cap: TransparentDepthCapability, bits: u8) -> Self {
        let frame = Self(bits);
        debug_assert!(
            frame_within_capability(frame, cap),
            "invariant: the transparent-depth frame set {bits:#04b} is not within the boot \
             capability {:#04b}",
            cap.bits()
        );
        frame
    }

    /// The raw bitset (bit `k` = class `k`).
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// `true` iff some class is declared this frame.
    #[inline]
    #[must_use]
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// `true` iff class `class` is declared this frame.
    #[inline]
    #[must_use]
    pub const fn has(self, class: u8) -> bool {
        self.0 & (1 << class) != 0
    }
}

/// `true` iff every class `frame` declares is armed in `cap` (the two layers' subset rule).
#[inline]
#[must_use]
pub const fn frame_within_capability(
    frame: TransparentDepthFrame,
    cap: TransparentDepthCapability,
) -> bool {
    frame.0 & !cap.0 == 0
}

/// What the frame declares for the SDF depth merge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SdfDepthMerge {
    /// No merge pass this frame.
    Absent,
    /// The merge pass draws the SDF depth (the production state on every SDF-leg frame with a
    /// declared consumer).
    Draw,
    /// Test-only (`disarm_sdf_merge` on the visibility-buffer SDF-only leg): the pass is declared
    /// and clears the depth it first-touches, but draws nothing — the pre-RD1 regime with a
    /// defined depth.
    ClearOnly,
}

/// The merge predicate (module doc). `sdf_leg`: the frame has an SDF producer; `vb_sdf_only`: the
/// visibility-buffer path without a mesh leg (where the merge is the path depth's first writer);
/// `hzb_dump_declared`: the HZB dump pass is declared this frame; `disarmed`: the test-only
/// `disarm_sdf_merge` key.
#[inline]
#[must_use]
pub const fn sdf_depth_merge(
    sdf_leg: bool,
    vb_sdf_only: bool,
    hzb_dump_declared: bool,
    frame: TransparentDepthFrame,
    disarmed: bool,
) -> SdfDepthMerge {
    debug_assert!(!vb_sdf_only || sdf_leg, "invariant: the VB SDF-only leg carries the SDF leg");
    if frame.any() && disarmed && vb_sdf_only {
        SdfDepthMerge::ClearOnly
    } else if sdf_leg && !hzb_dump_declared && frame.any() && !disarmed {
        SdfDepthMerge::Draw
    } else {
        SdfDepthMerge::Absent
    }
}

/// Whether the SDF forward marcher writes the `gViewT` lane: it marches this frame AND either the
/// TAA resolve reads the lane or the boot armed a transparent consumer class (the merge reads it).
/// Keyed on the CAPABILITY, never the frame set, so the marcher never flips variants frame to frame.
#[inline]
#[must_use]
pub const fn sdf_forward_writes_viewt(
    sdf_forward_marched: bool,
    taa: bool,
    cap: TransparentDepthCapability,
) -> bool {
    sdf_forward_marched && (taa || cap.any())
}

/// `true` iff pass `p` is a depth-tested transparent consumer of `path_depth`: one of its accesses
/// to that resource is a depth-attachment read WITHOUT a depth-attachment write, or a
/// fragment-shader read. The merge (read + write), the opaque producers (write), compute readers
/// (HZB, SSAO, resolves) and transfer reads (the dumps) are all excluded by the access itself.
#[must_use]
pub fn is_transparent_depth_consumer(g: &FrameGraph, p: PassId, path_depth: ResId) -> bool {
    g.pass_accesses(p).any(|a| {
        a.res == path_depth
            && ((a.access & VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT != 0
                && a.access & VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT == 0)
                || (a.stage & VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT != 0
                    && a.access & VK_ACCESS_SHADER_READ_BIT != 0))
    })
}

/// The order guard: the first depth-tested transparent consumer declared after
/// `last_sdf_producer` that is not preceded by the declared `merge`, or `None` when the frame is
/// in order. `None` also when the frame has no SDF producer, declares the HZB dump, or runs under
/// the disarm key (both exempt by name).
#[must_use]
pub fn transparent_depth_order_violation(
    g: &FrameGraph,
    path_depth: ResId,
    last_sdf_producer: Option<PassId>,
    merge: Option<PassId>,
    hzb_dump_declared: bool,
    disarmed: bool,
) -> Option<PassId> {
    let producer = last_sdf_producer?;
    if hzb_dump_declared || disarmed {
        return None;
    }
    let first = producer.index() + 1;
    (first..g.pass_count())
        .map(|i| PassId(i as u16))
        .filter(|&p| is_transparent_depth_consumer(g, p, path_depth))
        .find(|p| merge.is_none_or(|m| m.index() > p.index()))
}

/// The pairing check: the first class whose consumer pass is declared on a value other than its
/// frame bit, or `None`. `declared` lists `(class, consumer pass declared this frame)` for every
/// class the declarator knows; a frame bit with no row is itself a violation (a class added to the
/// frame set without its pairing row).
#[must_use]
pub fn transparent_depth_pairing_violation(
    frame: TransparentDepthFrame,
    declared: &[(u8, bool)],
) -> Option<u8> {
    if let Some(&(class, _)) = declared.iter().find(|&&(class, d)| frame.has(class) != d) {
        return Some(class);
    }
    (0..8u8).find(|&class| frame.has(class) && !declared.iter().any(|&(c, _)| c == class))
}

/// The merge recorder's load-op check (critique W1): `clears` must equal "the frame graph derived
/// the merge's access to `path_depth` as a FIRST TOUCH" — a barrier on `path_depth` with an
/// `UNDEFINED` old layout in the merge's derived barrier range. Returns `true` on a mismatch:
/// loading a first-touched depth reads undefined memory; clearing a written one erases the
/// opaque depth. Requires `g` compiled.
#[must_use]
pub fn merge_clear_violation(
    g: &FrameGraph,
    merge: PassId,
    path_depth: ResId,
    clears: bool,
) -> bool {
    let range = g.pass_barriers()[merge.index()];
    let begin = range.img_begin as usize;
    let first_touch = g.img_barriers()[begin..begin + range.img_count as usize]
        .iter()
        .any(|b| b.res == path_depth && b.old_layout == VK_IMAGE_LAYOUT_UNDEFINED);
    clears != first_touch
}

/// Debug wrapper over [`transparent_depth_order_violation`]: panics naming the offending pass.
#[cfg(debug_assertions)]
pub fn debug_check_transparent_depth_order(
    g: &FrameGraph,
    path_depth: ResId,
    last_sdf_producer: Option<PassId>,
    merge: Option<PassId>,
    hzb_dump_declared: bool,
    disarmed: bool,
) {
    if let Some(p) = transparent_depth_order_violation(
        g,
        path_depth,
        last_sdf_producer,
        merge,
        hzb_dump_declared,
        disarmed,
    ) {
        order_violation(g.pass_name(p));
    }
}

/// Debug wrapper over [`transparent_depth_pairing_violation`]: panics naming the class.
#[cfg(debug_assertions)]
pub fn debug_check_transparent_depth_pairing(
    frame: TransparentDepthFrame,
    declared: &[(u8, bool)],
) {
    if let Some(class) = transparent_depth_pairing_violation(frame, declared) {
        pairing_violation(class, frame);
    }
}

/// Debug wrapper over [`merge_clear_violation`]: panics naming the load op the recorder chose.
#[cfg(debug_assertions)]
pub fn debug_check_merge_clear(g: &FrameGraph, merge: PassId, path_depth: ResId, clears: bool) {
    if merge_clear_violation(g, merge, path_depth, clears) {
        clear_violation(g.res_name(path_depth), clears);
    }
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn order_violation(pass: &str) -> ! {
    panic!(
        "transparent-depth order: pass '{pass}' tests the path depth after the frame's SDF \
         producer with no sdf_depth_merge declared before it — SDF surfaces would not occlude it"
    )
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn pairing_violation(class: u8, frame: TransparentDepthFrame) -> ! {
    panic!(
        "transparent-depth pairing: consumer class {class} is declared on a value other than its \
         frame bit (frame set {:#04b}) — the merge and the consumer read different values",
        frame.bits()
    )
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn clear_violation(depth: &str, clears: bool) -> ! {
    if clears {
        panic!(
            "sdf_depth_merge clears '{depth}', which an earlier pass wrote this frame — the CLEAR \
             would erase the opaque depth"
        )
    } else {
        panic!(
            "sdf_depth_merge loads '{depth}' on its first touch this frame — LOAD after an \
             UNDEFINED transition reads undefined memory; the merge must CLEAR it"
        )
    }
}
