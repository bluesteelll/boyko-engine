//! OBS (render-wave plan H0 Scopes 3 and 4; OPEN-ITEMS 1, `critique_verify_r5.md` I3): the
//! wave's **seed class table held as data**, its production (P) check, and the metadata of the
//! **name-checked sinks**.
//!
//! # The class table
//!
//! [`SEED_CLASS_TABLE`] names every SINGLE resource shared by both frames in flight — HEAD's, and
//! every one the wave's rungs will create — with its class, its kind and the minimum stages its
//! seed must name. The idiom a developer copies for a screen-sized per-frame target is the per-FIF
//! ring declared with a bare `add_image` (`targets.rs`'s rings, `graph_bridge.rs`'s `lit`), and
//! the same line on a SINGLE image is the B-002/B-003 race: frame N+1's first write does not wait
//! for frame N's reads. A table row makes that a caught bug three ways:
//!
//! - **(P)** [`class_table_violation`] runs after every declarator's `compile` in debug builds —
//!   every golden run is a dev-profile run, so every golden frame checks every declared row;
//! - **(R)** the `BOYKO_GRAPH_RECEIPT` device census re-derives the class from each receipt's
//!   `[[resource]]` rows against this data, independently of this function;
//! - **host** — `tests/seed_class_table.rs` declares EVERY row bare, on every declarator of its
//!   mask, and requires a violation.
//!
//! # Rules for a row (they are what makes the table a gate rather than a list)
//!
//! - `name` is the **literal graph name** the owning rung must declare. Renaming a resource is a
//!   row edit in the same commit.
//! - A row registered ahead of its rung (`expect_present: false`) carries the
//!   **all-declarators mask** [`DECL_ALL`] until the owning rung narrows it, in its own commit —
//!   so a bare declaration on an unforeseen declarator cannot hide outside the mask.
//! - The owning rung flips `expect_present` and fills `seed_stages_min` in the commit that
//!   declares the resource. `seed_stages_min == 0` means "the owning rung fills it". A landed row
//!   the census's max-config frame does not declare keeps `expect_present: false` and is named in
//!   [`LANDED_OFF_MAX_CONFIG`].
//! - `kind` is {image, buffer} only: a mip chain is an image (a mip count is a frame-dependent
//!   shape — `hzb_pyramid` has one level on an HZB-off frame — not a class property).
//!
//! # The name-checked sinks
//!
//! Each `record_*_pass` resolves a derived barrier's `ResId` to a physical handle POSITIONALLY:
//! `images[res.index()]`, `buffers[res.index() − IMAGE_COUNT]`. A drift between the declarator's
//! order and the fill's order lands a barrier on a LIVE WRONG resource with no validation message.
//! In debug builds each fill writes, beside every handle expression, the slot's name and its
//! frame-in-flight shape ([`SinkMeta`]); release builds keep the handle-only arrays (the metadata
//! does not exist there — zero cost). [`sink_violation`] then checks, once per frame:
//!
//! - **(a)** the sink name equals the graph's `res_name` for every resource the frame touches;
//! - **(b)** a [`Fif::Single`] slot the frame WRITES has a seed with stages — the name-independent
//!   arm: a single target declared with the per-FIF idiom fires even under a name the class table
//!   does not know;
//! - **(c)** the [`Fif`] column is honest: against the same fill for the sibling slot `fi ^ 1`, a
//!   `Ring` slot's handle differs and a `Single` slot's is equal.

use crate::ffi::{
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT as COMPUTE, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT as INDIRECT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT as FRAGMENT, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT as VERTEX,
};
use crate::framegraph::{seed_stages, FrameGraph, PassId, ResId, SeedClass, WRITE_ACCESS_MASK};

/// The Deferred declarator (`declare_deferred_graph`).
pub const DECL_DEFERRED: u8 = 1;
/// The Forward declarator (`declare_forward_graph`), shared by Forward and ForwardPlus.
pub const DECL_FORWARD: u8 = 2;
/// The visibility-buffer declarator (`declare_vb_graph`).
pub const DECL_VB: u8 = 4;
/// Every declarator — the mask of a row registered ahead of its owning rung.
pub const DECL_ALL: u8 = DECL_DEFERRED | DECL_FORWARD | DECL_VB;
/// The individual declarator bits, for iterating a mask.
pub const DECLARATORS: [u8; 3] = [DECL_DEFERRED, DECL_FORWARD, DECL_VB];

/// The declarator a bit names, as the receipt and the failure messages spell it.
pub const fn declarator_name(declarator: u8) -> &'static str {
    match declarator {
        DECL_DEFERRED => "Deferred",
        DECL_FORWARD => "Forward",
        DECL_VB => "VisibilityBuffer",
        _ => "<mask>",
    }
}

/// What a row's resource is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeedKind {
    /// An image (single-level or a mip chain).
    Image,
    /// A buffer.
    Buffer,
}

/// One row of [`SEED_CLASS_TABLE`] (see the module doc for the rules a row follows).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeedClassRow {
    /// The literal graph name.
    pub name: &'static str,
    /// The declarators the row applies to ([`DECL_DEFERRED`] / [`DECL_FORWARD`] / [`DECL_VB`]).
    pub declarators: u8,
    /// Image or buffer.
    pub kind: SeedKind,
    /// [`SeedClass::CrossFrame`] or [`SeedClass::FrameLocal`].
    pub class: SeedClass,
    /// The stages the seed must name at least; `0` until the owning rung fills it.
    pub seed_stages_min: u32,
    /// The frame may end on a write, so the seed must be a writer seed (`flush_stages != 0`).
    pub writer_seed: bool,
    /// The rung that owns the resource (`"HEAD"` for the shipped ones).
    pub owner: &'static str,
    /// The resource is declared on every max-config frame of each declarator of the mask (the
    /// device census requires it); `false` until the owning rung lands it.
    pub expect_present: bool,
}

const D: u8 = DECL_DEFERRED;
const F: u8 = DECL_FORWARD;
const V: u8 = DECL_VB;
const HWRT: bool = cfg!(feature = "hwrt");

const fn head(name: &'static str, declarators: u8, kind: SeedKind, min: u32, writer: bool) -> SeedClassRow {
    SeedClassRow {
        name,
        declarators,
        kind,
        class: SeedClass::CrossFrame,
        seed_stages_min: min,
        writer_seed: writer,
        owner: "HEAD",
        expect_present: true,
    }
}

const fn head_hwrt(name: &'static str, declarators: u8, min: u32, writer: bool) -> SeedClassRow {
    SeedClassRow { expect_present: HWRT, ..head(name, declarators, SeedKind::Image, min, writer) }
}

const fn wave(
    name: &'static str,
    owner: &'static str,
    kind: SeedKind,
    class: SeedClass,
    min: u32,
) -> SeedClassRow {
    SeedClassRow {
        name,
        declarators: DECL_ALL,
        kind,
        class,
        seed_stages_min: min,
        writer_seed: false,
        owner,
        expect_present: false,
    }
}

/// A wave row its owning rung has landed: `FrameLocal` image, the narrowed mask and stages.
const fn landed(name: &'static str, declarators: u8, min: u32, expect_present: bool) -> SeedClassRow {
    SeedClassRow { declarators, seed_stages_min: min, expect_present, ..wave(name, "PX1a", SeedKind::Image, SeedClass::FrameLocal, min) }
}

use SeedClass::{CrossFrame as CF, FrameLocal as FL};
use SeedKind::{Buffer as B, Image as I};

/// The wave's seed class table (H0 Scope 4, rev 5; the 04-AMENDMENTS-R6 H0 row). HEAD's single
/// cross-frame resources are transcribed from the three declarators' seeds; every rev-5 wave row
/// is registered ahead of its rung (`expect_present: false`, all-declarators mask). The R9 / V9
/// rows (`csm_trans_depth`, `atlas_trans{,_depth}`, `atmo_*`) are appended by those rungs.
pub const SEED_CLASS_TABLE: &[SeedClassRow] = &[
    // --- HEAD: images ---
    head("cascade", D | V, I, COMPUTE, false),
    head("cascade", F, I, FRAGMENT, false),
    head("atlas", D | V, I, COMPUTE, false),
    head("atlas", F, I, FRAGMENT, false),
    head("ddgi_irr", D | V, I, COMPUTE, false),
    head("ddgi_depth", D | V, I, COMPUTE, false),
    head("taa_hist", D | V, I, COMPUTE, false),
    head("taa_hist_read", D | V, I, COMPUTE, true),
    head_hwrt("shadow_temporal_hist", D | V, COMPUTE, false),
    head_hwrt("shadow_temporal_hist_read", D | V, COMPUTE, true),
    head("hzb_pyramid", V, I, COMPUTE, true),
    // --- HEAD: buffers ---
    head("light_table", D | V, B, COMPUTE, false),
    head("light_table", F, B, FRAGMENT, false),
    head("tiles", D, B, COMPUTE, false),
    head("grid", D, B, COMPUTE, false),
    head("index", D, B, COMPUTE, false),
    head("alloc", D, B, COMPUTE, true),
    head("ddgi_classification", D | V, B, COMPUTE, false),
    head("ddgi_ray_table", D | V, B, COMPUTE, false),
    head("cluster_grid", V, B, COMPUTE, false),
    head("cluster_grid", F, B, FRAGMENT, false),
    head("light_index", V, B, COMPUTE, false),
    head("light_index", F, B, FRAGMENT, false),
    head("light_index_alloc", F | V, B, COMPUTE, true),
    head("material_table", D | V, B, COMPUTE, false),
    head("material_table", F, B, FRAGMENT | COMPUTE, false),
    head("p_counters", D | F | V, B, COMPUTE, true),
    head("p_dispatch_args", D | F | V, B, INDIRECT, false),
    head("p_draw_args", D | F | V, B, INDIRECT, false),
    head("p_dead", D | F | V, B, COMPUTE, true),
    head("p_alive_read", D | F | V, B, COMPUTE, true),
    head("p_alive_write", D | F | V, B, COMPUTE, false),
    head("p_particle", D | F | V, B, COMPUTE, true),
    head("p_render", D | F | V, B, VERTEX, false),
    head("p_emit_req", D | F | V, B, COMPUTE, false),
    head("p_effects", D | F | V, B, COMPUTE, false),
    head("p_render_sorted", D | F | V, B, VERTEX, false),
    head("p_sort_bins", D | F | V, B, COMPUTE, true),
    // --- the wave (H0 rev-5 table; 04 §B H0 row) ---
    // Render wave PX1a (D-G1): the post seam's two single targets, landed — `display` on every
    // declarator's every frame (`post_final` writes it, `present_sample` ends it at
    // SHADER_READ_ONLY for the FRAGMENT blit; the seed keeps COMPUTE for the CAS read render wave
    // PX1b retired — a sound superset that keeps every barrier stream, D-R13); `lit_native`
    // on Deferred and VB, written by the SSAA downsample and read by `post_final` at COMPUTE.
    // `lit_native` stays off the census's max-config (TAA) frame, so `expect_present: false`.
    landed("display", D | F | V, COMPUTE | FRAGMENT, true),
    landed("lit_native", D | V, COMPUTE, false),
    wave("lit_prev", "R4", I, FL, 0),
    wave("scene_color_mips", "R4", I, FL, 0),
    wave("post_chain", "PX3", I, FL, 0),
    wave("taa_cov", "R6", I, FL, 0),
    wave("csm_trans", "R9", I, FL, COMPUTE | FRAGMENT),
    wave("fog_integrated", "V2", I, FL, COMPUTE),
    wave("fog_sun_far", "V2", I, FL, COMPUTE),
    wave("fog_scatter[0]", "V2", I, FL, COMPUTE),
    wave("exposure_hist", "PX3", B, FL, COMPUTE),
    wave("oit_b0", "R7", I, FL, FRAGMENT | COMPUTE),
    wave("oit_moments", "R7", I, FL, FRAGMENT),
    wave("oit_accum", "R7", I, FL, COMPUTE),
    wave("easu_out", "PX8s", I, FL, FRAGMENT),
    wave("fog_media_acc", "V10", B, FL, COMPUTE),
    wave("fog_inj_lanes", "V10", B, FL, COMPUTE),
    wave("fog_inj_scan", "V10", B, FL, COMPUTE),
    wave("post_lut", "PX5", I, CF, 0),
    wave("exposure_state", "PX3", B, CF, 0),
];

/// Rows their rung has LANDED — mask narrowed, `seed_stages_min` filled — that the census's
/// max-config frame still does not declare, so `expect_present` stays `false` (render wave PX1a
/// Q10: `lit_native` is SSAA-only and the max config is TAA). The pre-registration rule (the
/// all-declarators mask while `expect_present` is false) does not apply to them; each must carry
/// non-zero minimum stages, the mark of a landed row.
pub const LANDED_OFF_MAX_CONFIG: &[&str] = &["lit_native"];

/// The row `name` has for `declarator`, if any.
pub fn row_for(name: &str, declarator: u8) -> Option<&'static SeedClassRow> {
    SEED_CLASS_TABLE.iter().find(|r| r.declarators & declarator != 0 && r.name == name)
}

/// Which rule a declared row broke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassRule {
    /// The derived class differs from the row's (`Bare` and `ProvenanceOnly` always differ).
    WrongClass,
    /// An image row declared as a buffer, or the reverse.
    WrongKind,
    /// The seed names no stage at all.
    SeedWithoutStages,
    /// The seed lacks some of the row's minimum stages.
    SeedMissingMinStages,
    /// The row may end its frame on a write, but the seed has no flush stages.
    ReaderSeedOnWriterRow,
}

/// A declared resource whose name is a row for this declarator, declared against the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClassViolation {
    /// The offending resource.
    pub res: ResId,
    /// Its graph name.
    pub name: &'static str,
    /// The declarator it was declared on.
    pub declarator: u8,
    /// The rule broken.
    pub rule: ClassRule,
    /// The row's class.
    pub row_class: SeedClass,
    /// The class the declaration derives.
    pub derived: SeedClass,
    /// The declaration's seed stages.
    pub seed_stages: u32,
    /// The row's minimum stages.
    pub min_stages: u32,
    /// The rung that owns the row.
    pub owner: &'static str,
}

/// The class-table check over one compiled frame of `declarator`: the first declared resource
/// whose name is a row for that declarator and whose declaration breaks the row.
pub fn class_table_violation(g: &FrameGraph, declarator: u8) -> Option<ClassViolation> {
    for i in 0..g.res_count() {
        let res = ResId(i as u16);
        let name = g.res_name(res);
        let Some(row) = row_for(name, declarator) else {
            continue;
        };
        let seed = g.res_seed(res);
        let stages = seed_stages(seed);
        let derived = g.res_class(res);
        let kind = if g.res_is_image(res) { SeedKind::Image } else { SeedKind::Buffer };
        let rule = if derived != row.class {
            ClassRule::WrongClass
        } else if kind != row.kind {
            ClassRule::WrongKind
        } else if stages == 0 {
            ClassRule::SeedWithoutStages
        } else if row.seed_stages_min & !stages != 0 {
            ClassRule::SeedMissingMinStages
        } else if row.writer_seed && seed.flush_stages == 0 {
            ClassRule::ReaderSeedOnWriterRow
        } else {
            continue;
        };
        return Some(ClassViolation {
            res,
            name,
            declarator,
            rule,
            row_class: row.class,
            derived,
            seed_stages: stages,
            min_stages: row.seed_stages_min,
            owner: row.owner,
        });
    }
    None
}

/// The (P) site: panics with the class-table message when [`class_table_violation`] finds one.
/// Called right after each declarator's `compile`, in debug builds.
#[cfg(debug_assertions)]
#[inline]
pub(crate) fn assert_class_table(g: &FrameGraph, declarator: u8) {
    if let Some(v) = class_table_violation(g, declarator) {
        fire_class_violation(v);
    }
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn fire_class_violation(v: ClassViolation) -> ! {
    panic!(
        "OBS CLASS TABLE violated ({:?}) — '{}' is a row of SEED_CLASS_TABLE (class {}, owner {}, \
         minimum seed stages {:#x}) but the {} declarator declares it {} with seed stages {:#x}. \
         A single resource shared by both frames in flight declared with the per-FIF idiom (a \
         bare add_image / add_buffer) lets frame N+1's first write race frame N's reads; declare \
         it through SeededImage / SeededBuffer with its class, or edit the row in the same commit",
        v.rule,
        v.name,
        v.row_class.name(),
        v.owner,
        v.min_stages,
        declarator_name(v.declarator),
        v.derived.name(),
        v.seed_stages
    )
}

// --- the name-checked sinks ------------------------------------------------------------------

/// How a sink slot's handle relates to the frame-in-flight slot `fi`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fif {
    /// One handle per frame-in-flight slot, chosen by `fi` in the fill (`targets.lit[fi]`, or the
    /// sibling `[fi ^ 1]` read slot): the fill for `fi ^ 1` yields a different handle.
    Ring,
    /// A ring the runner already sliced to this frame's slot before the scene reached the fill
    /// (the interp trio, `tlas_instances`): the fill cannot re-slice it, so (c) skips it.
    RingPreSliced,
    /// One handle shared by both frames in flight: the fill for `fi ^ 1` yields the same one.
    Single,
}

impl Fif {
    /// The column value as the receipt spells it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ring => "Ring",
            Self::RingPreSliced => "RingPreSliced",
            Self::Single => "Single",
        }
    }
}

/// The debug-only metadata written beside one sink slot's handle expression.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SinkMeta {
    /// The slot's name, spelled independently of the declarator's `add_*` call.
    pub name: &'static str,
    /// The slot's frame-in-flight shape.
    pub fif: Fif,
}

/// One sink slot: its metadata beside its handle in debug builds, the bare handle in release.
#[cfg(debug_assertions)]
pub(crate) type Slot<H> = (SinkMeta, H);
/// One sink slot: its metadata beside its handle in debug builds, the bare handle in release.
#[cfg(not(debug_assertions))]
pub(crate) type Slot<H> = H;

/// A sink slot written beside its handle expression: `slot!("lit", Ring, targets.lit[fi].image)`.
/// In release builds it is the handle expression alone, so the fills compile to today's
/// handle-only arrays.
#[cfg(debug_assertions)]
macro_rules! slot {
    ($name:literal, $fif:ident, $handle:expr) => {
        (
            $crate::present::seed_class::SinkMeta {
                name: $name,
                fif: $crate::present::seed_class::Fif::$fif,
            },
            $handle,
        )
    };
}
/// A sink slot written beside its handle expression (release: the handle expression alone).
#[cfg(not(debug_assertions))]
macro_rules! slot {
    ($name:literal, $fif:ident, $handle:expr) => {
        $handle
    };
}
pub(crate) use slot;

/// A positional sink fill: the handle array the barrier sink resolves `ResId`s through, and — in
/// debug builds only — the metadata array written slot for slot beside it.
pub(crate) struct SinkFill<H: Copy, const N: usize> {
    pub(crate) handles: [H; N],
    #[cfg(debug_assertions)]
    pub(crate) meta: [Option<SinkMeta>; N],
    next: usize,
}

impl<H: Copy, const N: usize> SinkFill<H, N> {
    /// An empty fill: every slot `null` (inert — no derived barrier names an undeclared `ResId`).
    #[inline]
    pub(crate) fn new(null: H) -> Self {
        Self {
            handles: [null; N],
            #[cfg(debug_assertions)]
            meta: [None; N],
            next: 0,
        }
    }

    /// A fill from a whole literal array of slots.
    #[inline]
    pub(crate) fn from_slots(slots: [Slot<H>; N]) -> Self {
        #[cfg(debug_assertions)]
        {
            Self {
                handles: slots.map(|s| s.1),
                meta: slots.map(|s| Some(s.0)),
                next: N,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            Self { handles: slots, next: N }
        }
    }

    /// Appends `slots` at the next positions, in order.
    #[inline]
    pub(crate) fn push_all(&mut self, slots: &[Slot<H>]) {
        for &s in slots {
            #[cfg(debug_assertions)]
            {
                self.meta[self.next] = Some(s.0);
                self.handles[self.next] = s.1;
            }
            #[cfg(not(debug_assertions))]
            {
                self.handles[self.next] = s;
            }
            self.next += 1;
        }
    }
}

/// What a sink check found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinkRule {
    /// (a) the slot's name differs from the graph's name for the `ResId`.
    NameMismatch,
    /// (a) the frame touches a `ResId` the fill wrote no slot for.
    MissingSlot,
    /// (b) a `Single` slot is written this frame under a seed with no stages.
    SingleDeclaredPerFif,
    /// (c) a `Ring` slot resolves to the same handle for `fi` and `fi ^ 1`.
    RingNotRinged,
    /// (c) a `Single` slot resolves to different handles for `fi` and `fi ^ 1`.
    SingleNotShared,
}

/// A sink slot that disagrees with the graph or with its own `Fif` column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SinkViolation {
    /// The rule broken.
    pub rule: SinkRule,
    /// The `ResId` concerned.
    pub res: ResId,
    /// The graph's name for it.
    pub graph_name: &'static str,
    /// The slot's name (`""` for a missing slot).
    pub sink_name: &'static str,
}

/// The per-resource facts the sink check reads from the compiled graph: touched this frame, and
/// written this frame.
fn touched_and_written(g: &FrameGraph, res: ResId) -> (bool, bool) {
    let mut touched = false;
    for p in 0..g.pass_count() {
        for a in g.pass_accesses(PassId(p as u16)) {
            if a.res == res {
                touched = true;
                if a.access & WRITE_ACCESS_MASK != 0 {
                    return (true, true);
                }
            }
        }
    }
    (touched, false)
}

/// Checks one sink (images then buffers, each as raw handle values for `fi` and for `fi ^ 1`)
/// against the compiled graph: arms (a), (b) and (c) of the module doc, over every resource the
/// frame touches.
pub fn sink_violation(
    g: &FrameGraph,
    images: (&[Option<SinkMeta>], &[u64], &[u64]),
    buffers: (&[Option<SinkMeta>], &[u64], &[u64]),
) -> Option<SinkViolation> {
    let image_count = images.0.len();
    for i in 0..g.res_count() {
        let res = ResId(i as u16);
        let (touched, written) = touched_and_written(g, res);
        if !touched {
            continue;
        }
        let graph_name = g.res_name(res);
        let (meta, now, other) = if g.res_is_image(res) {
            (images.0.get(i).copied().flatten(), images.1.get(i), images.2.get(i))
        } else {
            let b = i.wrapping_sub(image_count);
            (buffers.0.get(b).copied().flatten(), buffers.1.get(b), buffers.2.get(b))
        };
        let fail = |rule, sink_name| Some(SinkViolation { rule, res, graph_name, sink_name });
        let Some(meta) = meta else {
            return fail(SinkRule::MissingSlot, "");
        };
        if meta.name != graph_name {
            return fail(SinkRule::NameMismatch, meta.name);
        }
        if meta.fif == Fif::Single && written && seed_stages(g.res_seed(res)) == 0 {
            return fail(SinkRule::SingleDeclaredPerFif, meta.name);
        }
        let (Some(&now), Some(&other)) = (now, other) else {
            continue;
        };
        if now == 0 || other == 0 {
            continue;
        }
        match meta.fif {
            Fif::Ring if now == other => return fail(SinkRule::RingNotRinged, meta.name),
            Fif::Single if now != other => return fail(SinkRule::SingleNotShared, meta.name),
            _ => {}
        }
    }
    None
}

/// The (P) site: panics with the sink message when [`sink_violation`] finds one. Called once per
/// frame, at the frame's first `record_*_pass`, in debug builds.
#[cfg(debug_assertions)]
#[inline]
pub(crate) fn assert_sink(
    g: &FrameGraph,
    declarator: u8,
    images: (&[Option<SinkMeta>], &[u64], &[u64]),
    buffers: (&[Option<SinkMeta>], &[u64], &[u64]),
) {
    if let Some(v) = sink_violation(g, images, buffers) {
        fire_sink_violation(v, declarator);
    }
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn fire_sink_violation(v: SinkViolation, declarator: u8) -> ! {
    match v.rule {
        SinkRule::NameMismatch | SinkRule::MissingSlot => panic!(
            "OBS SINK NAME violated ({:?}) — the {} sink resolves ResId {} ('{}' in the graph) \
             through the slot named '{}'. The fill and the declarator disagree on the positional \
             order, so this resource's barriers would land on a LIVE WRONG handle with no \
             validation message; fix the fill or the declaration order",
            v.rule,
            declarator_name(declarator),
            v.res.0,
            v.graph_name,
            v.sink_name
        ),
        SinkRule::SingleDeclaredPerFif => panic!(
            "OBS SINK FIF violated — a single resource shared by both frames in flight is declared \
             with the per-FIF idiom: the {} sink's slot '{}' is Fif::Single and written this frame, \
             but its graph declaration '{}' carries a seed with no stages, so frame N+1's first \
             write races frame N's reads. Declare it seeded (SeededImage / SeededBuffer)",
            declarator_name(declarator),
            v.sink_name,
            v.graph_name
        ),
        SinkRule::RingNotRinged | SinkRule::SingleNotShared => panic!(
            "OBS SINK FIF violated ({:?}) — the {} sink's slot '{}' does not match its Fif \
             column against the sibling frame-in-flight slot; the column is what arm (b) trusts, \
             so a wrong one hides a per-FIF declaration of a shared resource",
            v.rule,
            declarator_name(declarator),
            v.sink_name
        ),
    }
}
