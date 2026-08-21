//! CPU-side pack helpers + the reused render scratch — GUI P5a Decision 6 / A1.
//!
//! The pack is `O(N)` sequential SoA into a reused `Vec<UiInstance>` (`clear()` +
//! `extend`, never `Vec::new`), sorted `O(N log N)` in place by
//! `(StackIndex, append_order)` — an unstable sort over a TOTAL key (the unique
//! append index breaks ties), so the result is painter's order with zero per-frame
//! allocation — then bulk-memcpy'd once into the mapped ring slot.
//! There is NO mirror column and NO per-chunk `cast_slice` — a global z-sort across
//! archetypes forbids a per-chunk blit (Decision 6).

use boyko_macros::Resource;

use crate::ui::instance::{
    premultiply_rgba8, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, FLAG_TEXT, FLAG_TEXTURED, UiInstance,
    UI_SLOT_MASK, UI_SLOT_SHIFT,
};

/// The SPRITE half of one node's pack inputs (UI-ADVANCED S3): the `UiImage`
/// component's three render-relevant values, flattened so the pack stays free of
/// any `boyko_ui` type (`boyko-render` reads the component in the gather and passes
/// values here — the same shape `text_uv` already takes).
///
/// Its presence is the capability: a node WITHOUT `UiImage` emits no sprite record
/// at all (structural skip), and one WITH it emits a sprite quad whose default tint
/// is fully transparent, so an authored-but-untextured Image costs one invisible
/// instance and ZERO pixels (S-D8's default-OFF row for this rung).
#[derive(Clone, Copy, Debug)]
pub struct UiImageInput {
    /// The bindless texture slot (`UiImage.texture`) — MUST be
    /// `< BINDLESS_TEXTURE_CAPACITY`; it is packed into `flags` bits
    /// [`UI_SLOT_SHIFT`]`..32` and `debug_assert!`ed at the pack (gate G3-5).
    pub slot: u32,
    /// The sprite's normalized UV sub-rect `(u0, v0, u1, v1)` in `[0, 1]`
    /// (`UiImage.uv_min`/`uv_max`), written VERBATIM into [`UiInstance::uv`] —
    /// never scale-folded, exactly like the glyph UV.
    pub uv: [f32; 4],
    /// The tint, STRAIGHT RGBA8 (`UiImage.tint`); premultiplied at pack into
    /// [`UiInstance::color`], the same convention `UiBackground.color` follows.
    pub tint: u32,
}

/// The NINE-SLICE half of one node's pack inputs (UI-ADVANCED S4): the
/// `UiNineSlice` component's four render-relevant values, flattened so the pack
/// stays free of any `boyko_ui` type (the [`UiImageInput`] shape, one rung on).
///
/// Its presence is HALF the capability: nine sub-quads are emitted only when
/// this AND [`PackInput::image`] are both present — a nine-sliced node with no
/// image is a structural no-op that emits its background alone (S-D12 (3)).
#[derive(Clone, Copy, Debug)]
pub struct UiNineSliceInput {
    /// DESTINATION inset per side, logical px, `[l, t, r, b]`. `debug_assert!`ed
    /// non-negative and finite. An axis whose two sides exceed the node's extent
    /// is shrunk proportionally at pack (a chrome tweened below its own border is
    /// ordinary, not an error); a NEGATIVE side is clamped to zero.
    pub border_px: [f32; 4],
    /// SOURCE inset per side as a fraction of [`UiImageInput::uv`], `[l, t, r, b]`.
    /// `debug_assert!`ed into `[0, 1)` with `l + r < 1` / `t + b < 1`.
    ///
    /// In release the domain's two edges get the two remedies the pack's axis
    /// split carries, and they are not the same remedy: an axis whose
    /// sides **sum to 1 or more** is scaled down proportionally, so the centre
    /// source region degenerates to zero width; a side **below 0** is clamped to
    /// zero, because a negative inset is not a proportion of anything and the sum
    /// test cannot see it. Both land on the same guarantee — degenerate, never
    /// invert into a negative-extent UV rect.
    pub border_uv: [f32; 4],
    /// The `NineSliceMode` discriminant as a RAW `u8` — the
    /// [`UiImageInput::slot`] precedent, and for the same reason: a typed
    /// one-variant enum cannot carry an out-of-range value without a
    /// `transmute`, which is instant UB and therefore cannot be a gate. The
    /// AUTHORED component keeps the typed enum, where the type system forbids
    /// the value; this raw byte is `debug_assert!`ed
    /// `< `[`UI_NINE_SLICE_MODE_COUNT`] at the pack boundary.
    pub mode: u8,
    /// Emit the centre sub-quad (region 4 / sub [`UI_NINE_SLICE_CENTER_SUB`])?
    pub fill_center: bool,
}

/// The number of legal [`UiNineSliceInput::mode`] values — the bound the pack
/// `debug_assert!`s a raw discriminant against (gate G4-5).
///
/// It is `1` at S4 and it is BOUND to `boyko_ui`'s `NineSliceMode` by the
/// EXHAUSTIVE conversion match in
/// [`gather_ui_nodes`](crate::ui::gather::gather_ui_nodes) — the one site that
/// turns the authored enum into this raw byte. Adding a variant there is
/// `error[E0004]`, which is what walks the author to this line. (The count
/// cannot be derived: `std::mem::variant_count` is nightly-only on 1.97.1, and
/// the enum lives in the crate this module is deliberately type-free of.)
pub const UI_NINE_SLICE_MODE_COUNT: u8 = 1;

/// One source node's pack inputs (logical-px component values + the node's z key),
/// the testable boundary of [`pack_ui_instance`] (no Arena/world dependency, so the
/// pack is unit-testable in isolation per the testability rule).
#[derive(Clone, Copy, Debug)]
pub struct PackInput {
    /// `ComputedRect` (logical px): top-left x, y, width, height.
    pub rect: [f32; 4],
    /// `UiBackground.color` (STRAIGHT RGBA8).
    pub color: u32,
    /// `UiBackground.border_color` (STRAIGHT RGBA8).
    pub border_color: u32,
    /// `UiBackground.corner_radius` (logical px): tl, tr, br, bl.
    pub corner_radius: [f32; 4],
    /// `UiBackground.border_width` (logical px): l, t, r, b. P5a uses the UNIFORM
    /// width (`[0]`) and `debug_assert!`s the four sides equal.
    pub border_width: [f32; 4],
    /// `ComputedClip` (logical px) if the node carries one: x, y, w, h.
    pub clip: Option<[f32; 4]>,
    /// GUI P5b text lane (Decision T4-G): when `Some`, this node is a GLYPH quad, not
    /// a rect. The value is the glyph's NORMALIZED atlas UV rect `(left, top, right,
    /// bottom)` in `[0, 1]`, written verbatim (NOT scale-folded) into
    /// [`UiInstance::uv`] with `FLAG_TEXT` set (its OWN field since the UI-ADVANCED
    /// S2 widening — the `corner_radius` alias is retired, and a glyph packs
    /// `corner_radius` ZERO); `rect` is then the glyph quad (already
    /// physical-or-logical px, scale-folded like a rect), `color` the
    /// premultiplied-at-pack foreground, and `border_*` are ignored. `None` ⇒ the
    /// rect path (P5a, unchanged; packs the identity `uv = (0, 0, 1, 1)`).
    pub text_uv: Option<[f32; 4]>,
    /// UI-ADVANCED S3 sprite lane: `Some` iff the node carries a `UiImage`. It does
    /// NOT change what [`pack_ui_instance`] returns — the node's background rect is
    /// packed exactly as before — it makes the node emit a SECOND record via
    /// [`pack_ui_image_instance`], per D4's per-node emission contract
    /// (*background rect → … → image → glyphs*).
    pub image: Option<UiImageInput>,
    /// UI-ADVANCED S4 nine-slice lane: `Some` iff the node carries a
    /// `UiNineSlice`. Together with [`image`](Self::image) it selects the node's
    /// emission from S-D12 (1)'s four-row truth table — and when BOTH are
    /// present it SUPPRESSES the whole-rect image record, because the nine
    /// sub-quads ARE that image, sliced.
    pub nine_slice: Option<UiNineSliceInput>,
}

/// Folds one node's logical-px inputs into a physical-px, premultiplied
/// [`UiInstance`] (Decision 6 / A1 step 2): scale folding, premultiply, sentinel-free
/// `CLIP_PRESENT`, `BORDER_ANY` from the uniform border width.
///
/// `scale_factor` is the logical→physical DPI scale (folded into every length so the
/// shader works in physical px and `fwidth` AA is one device pixel). The four border
/// sides MUST be equal in P5a (a `debug_assert!` traps an asymmetric author — the
/// uniform case is exact; asymmetric per-side is a deferred phase).
pub fn pack_ui_instance(input: &PackInput, scale_factor: f32) -> UiInstance {
    debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
    debug_assert!(
        input.rect.iter().all(|v| v.is_finite()),
        "invariant: ComputedRect is finite before pack"
    );

    let s = scale_factor;
    let min_px = [input.rect[0] * s, input.rect[1] * s];
    let size_px = [input.rect[2] * s, input.rect[3] * s];

    // Clip is shared by rects AND glyphs (text clips too). Physical-px AABB.
    let mut flags = 0u32;
    let clip = match input.clip {
        Some(c) => {
            debug_assert!(
                c.iter().all(|v| v.is_finite()),
                "invariant: ComputedClip is finite when CLIP_PRESENT"
            );
            flags |= FLAG_CLIP_PRESENT;
            [c[0] * s, c[1] * s, (c[0] + c[2]) * s, (c[1] + c[3]) * s]
        }
        // Unclipped: leave clip zero, flag clear — the shader never reads it (no
        // sentinel arithmetic to ill-condition).
        None => [0.0; 4],
    };

    // GUI P5b text branch (Decision T4-G): a glyph quad. The UV rect goes into the
    // record's OWN `uv` field (UI-ADVANCED S2 — the `corner_radius` alias is retired;
    // a glyph packs the radius ZERO, gate G2-5), written verbatim, NOT scale-folded —
    // it is already normalized. `FLAG_TEXT` selects the MSDF branch in the FS. Border
    // is N/A for a glyph.
    if let Some(uv) = input.text_uv {
        debug_assert!(
            uv.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invariant: FLAG_TEXT glyph UV is finite and within [0, 1]"
        );
        flags |= FLAG_TEXT;
        return UiInstance {
            min_px,
            size_px,
            clip,
            corner_radius: [0.0; 4],
            uv,
            color: premultiply_rgba8(input.color),
            border_color: 0,
            border_width: 0.0,
            flags,
        };
    }

    // P5a rect branch (unchanged).
    debug_assert!(
        input.corner_radius.iter().all(|v| v.is_finite()),
        "invariant: corner_radius is finite before pack"
    );
    // P5a uniform-border invariant: the four sides must match (asymmetric deferred).
    let bw = input.border_width[0];
    debug_assert!(
        input.border_width.iter().all(|&w| w == bw),
        "invariant: P5a renders a UNIFORM border — the four border_width sides must be equal"
    );

    let corner_radius = [
        input.corner_radius[0] * s,
        input.corner_radius[1] * s,
        input.corner_radius[2] * s,
        input.corner_radius[3] * s,
    ];
    let border_width = bw * s;
    if border_width > 0.0 {
        flags |= FLAG_BORDER_ANY;
    }

    UiInstance {
        min_px,
        size_px,
        clip,
        corner_radius,
        // The identity UV (S-D8): a plain rect's shader branch never reads it, so
        // every pre-S2 node packs a constant and the widening is pixel-invisible
        // (gate G2-3); when S3's textured lane lands, `(0,0,1,1)` is also the
        // correct whole-texture default.
        uv: [0.0, 0.0, 1.0, 1.0],
        color: premultiply_rgba8(input.color),
        border_color: premultiply_rgba8(input.border_color),
        border_width,
        flags,
    }
}

/// The number of nine-slice SUB-QUADS a sliced node can emit — the sub space
/// `UI_NINE_SLICE_SUB_BASE ..= UI_NINE_SLICE_SUB_BASE + UI_NINE_SLICE_REGIONS - 1`,
/// **row-major**: TL, T, TR, L, **C**, R, BL, B, BR.
pub const UI_NINE_SLICE_REGIONS: u32 = 9;

/// The sub code of the FIRST nine-slice sub-quad (TL). Sub `0` is always the
/// node's background rect, so the slices start at `1`.
pub const UI_NINE_SLICE_SUB_BASE: u32 = 1;

/// The sub code of the CENTRE sub-quad (region 4 of 9), skipped when
/// `UiNineSliceInput::fill_center` is `false`.
pub const UI_NINE_SLICE_CENTER_SUB: u32 = UI_NINE_SLICE_SUB_BASE + 4;

/// The sub code of the WHOLE-RECT image record. It is emitted only when the node
/// carries a `UiImage` and NO `UiNineSlice` — when both are present the nine
/// sub-quads ARE that image and this code is not pushed (S-D12 (1)).
pub const UI_IMAGE_SUB: u32 = UI_NINE_SLICE_SUB_BASE + UI_NINE_SLICE_REGIONS;

/// The STRIDE of the `(node, sub)` append code
/// [`UiUploadSystem::gather_into_staging`](crate::ui::upload::UiUploadSystem::gather_into_staging)
/// sorts on — the one loop that packs directly in SORTED order and therefore has to
/// find each record's SOURCE node from its key alone.
///
/// **It is DERIVED from the largest sub code, and it is a stride rather than a
/// per-node emission count.** UI-ADVANCED S4 severed the two: the sub space is a
/// fixed layout with a HOLE in it (a sliced node uses `0` and `1..=9` and never
/// `10`; an unsliced imaged node uses `0` and `10` and never `1..=9`), so the
/// per-node emission is 10 / 9 / 2 / 1 by S-D12 (1)'s truth table while the stride
/// stays `UI_IMAGE_SUB + 1`. The hole costs nothing: the key push only pushes codes
/// for records that exist, and the decode is `append % UI_RECORDS_PER_NODE`.
///
/// Deriving it from [`UI_IMAGE_SUB`] pins the one relation the hole made
/// non-obvious — a rung that adds a sub code moves the stride with it, and every
/// record count in the S4 gates is an expression over these constants rather than
/// a literal. (The pre-S4 doc claimed "S4's nine-slice raises this constant;
/// nothing else changes at that call site". The second half was false: the key
/// push was hard-coded to at most two sub-records and the decode was a BINARY
/// `if`, both of which S4 replaces.)
pub const UI_RECORDS_PER_NODE: u32 = UI_IMAGE_SUB + 1;

// The sub space is CONTIGUOUS and the image record sits directly above the last
// slice: the three literals above are not independently choosable, and this is
// the relation that says so.
const _: () = assert!(UI_NINE_SLICE_SUB_BASE + UI_NINE_SLICE_REGIONS == UI_IMAGE_SUB);
const _: () = assert!(UI_NINE_SLICE_CENTER_SUB > UI_NINE_SLICE_SUB_BASE);
const _: () = assert!(UI_NINE_SLICE_CENTER_SUB < UI_IMAGE_SUB);

/// Folds one node's SPRITE half into the second [`UiInstance`] that node emits
/// (UI-ADVANCED S3), or `None` when the node carries no `UiImage` — absence is the
/// structural skip, so an image-less world's record stream is byte-identical to S2's.
///
/// The sprite quad covers the SAME `ComputedRect` as the node's background (D4's
/// contract paints it directly over the background, and layout is untouched by the
/// image), so `min_px`/`size_px`/`clip` are the background record's verbatim — only
/// the flags, the UV and the color differ:
///
/// * `FLAG_TEXTURED` + the slot in `flags` bits [`UI_SLOT_SHIFT`]`..32` (S-D2),
/// * `uv` = the image's normalized sub-rect, written verbatim (never scale-folded),
/// * `color` = the premultiplied tint; `corner_radius`/`border_*` are N/A for a
///   sprite and pack ZERO (a rounded sprite is nine-slice's job, S4).
///
/// The default `UiImage` tint is alpha 0, so this record is INVISIBLE until an
/// author writes an opaque tint — the rung's default-OFF guarantee (gate G3-2, red
/// mutation M3-e).
pub fn pack_ui_image_instance(input: &PackInput, scale_factor: f32) -> Option<UiInstance> {
    let image = input.image?;
    debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
    debug_assert!(
        input.text_uv.is_none(),
        "invariant: a GLYPH row carries no sprite — FLAG_TEXT and FLAG_TEXTURED are \
         different quads with different shader branches, never one record wearing both"
    );
    debug_assert!(
        image.slot < boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY,
        "invariant: a UI sprite slot is a live bindless slot (< BINDLESS_TEXTURE_CAPACITY); \
         flags bits {UI_SLOT_SHIFT}..32 hold only {} of them",
        UI_SLOT_MASK + 1
    );
    debug_assert!(
        image.uv.iter().all(|v| v.is_finite()),
        "invariant: a UI sprite UV rect is finite"
    );

    let s = scale_factor;
    let mut flags = FLAG_TEXTURED | ((image.slot & UI_SLOT_MASK) << UI_SLOT_SHIFT);
    let clip = match input.clip {
        Some(c) => {
            flags |= FLAG_CLIP_PRESENT;
            [c[0] * s, c[1] * s, (c[0] + c[2]) * s, (c[1] + c[3]) * s]
        }
        None => [0.0; 4],
    };

    Some(UiInstance {
        min_px: [input.rect[0] * s, input.rect[1] * s],
        size_px: [input.rect[2] * s, input.rect[3] * s],
        clip,
        corner_radius: [0.0; 4],
        uv: image.uv,
        color: premultiply_rgba8(image.tint),
        border_color: 0,
        border_width: 0.0,
        flags,
    })
}

/// Splits one axis into its three extents from an inset pair. **Its whole
/// contract is that the three extents it returns are non-negative and sum to
/// `extent`** — S-D12 (2)'s ruled release behaviour, "degenerate, never invert",
/// for an inset pair outside the domain the pack `debug_assert!`s.
///
/// The domain has TWO edges and each needs its own remedy, which is the
/// correction this function carries:
///
/// * **A side BELOW zero is clamped to zero.** A negative inset is not a
///   proportion of anything, so there is nothing to scale: `-0.5` and `0.25` sum
///   to `-0.25`, which no `sum > extent` test can see, and the raw value would
///   put the first cut BEHIND the axis's own origin — a negative-extent
///   destination rect and a `u1 < u0` source rect, the exact picture S-D12 (2)
///   exists to forbid. (MEASURED in `--release` before this clamp existed, and
///   pinned by `ui_s4_nine_slice.rs`'s
///   `s_d12_2_a_negative_inset_degenerates_in_release_instead_of_inverting`,
///   which is release-only because in debug the pack's `debug_assert!` fires
///   first.)
/// * **A PAIR that overruns `extent` is shrunk proportionally.** Not optional and
///   not an error path: a 96×96 chrome animated to 8×8 is an ordinary tween, and
///   without it the corners overlap and the edges invert. Unity and Godot both do
///   exactly this. At `lo + hi == extent` the middle degenerates to zero rather
///   than inverting.
///
/// Used for BOTH sides of the split — the destination against the rect's extent
/// in logical px, and the source against `1.0`, because `border_uv` is already a
/// fraction of the sub-rect.
#[inline]
fn split_axis(lo: f32, hi: f32, extent: f32) -> [f32; 3] {
    // Clamp FIRST: the proportional shrink below is a proportion, and a negative
    // side has none. `max` also maps NaN to `0.0` here (`f32::max` returns the
    // non-NaN operand), so no NaN inset can reach the cumulative cuts.
    let lo = lo.max(0.0);
    let hi = hi.max(0.0);
    let sum = lo + hi;
    let (lo, hi) = if sum > extent && sum > 0.0 {
        let k = extent / sum;
        (lo * k, hi * k)
    } else {
        (lo, hi)
    };
    [lo, extent - lo - hi, hi]
}

/// Folds one nine-slice REGION of a node into the [`UiInstance`] that draws it
/// (UI-ADVANCED S4), or `None` when the node is missing either half of the
/// capability — absence is the structural skip, exactly as in
/// [`pack_ui_image_instance`], so a node carrying `UiNineSlice` and no `UiImage`
/// emits its background and nothing else.
///
/// `region` is `0..`[`UI_NINE_SLICE_REGIONS`], **row-major**: TL, T, TR, L, C, R,
/// BL, B, BR. It is the sub code minus [`UI_NINE_SLICE_SUB_BASE`].
///
/// The record is the sprite record's shape — `FLAG_TEXTURED` + the slot in
/// `flags`, the premultiplied tint in `color`, zero `corner_radius`/`border_*` —
/// narrowed to one of nine destination sub-rects sampling one of nine source
/// sub-rects:
///
/// * **destination**: the node's rect cut by [`UiNineSliceInput::border_px`],
///   `[l, t, r, b]` in logical px, scale-folded like any other length. A corner
///   is exactly `border_px` in size, NOT a fraction of the rect — that is the
///   whole of what nine-slicing is.
/// * **source**: the image's UV sub-rect cut by
///   [`UiNineSliceInput::border_uv`], per side as a FRACTION of that sub-rect.
///   Written verbatim into [`UiInstance::uv`], never scale-folded.
///
/// The three cuts on each axis come from cumulative boundaries with the OUTER
/// edges pinned to the node's own rect and the image's own UV, so the nine
/// regions TILE their parents exactly — no seam, no overlap, no accumulated
/// drift at the far edge.
pub fn pack_ui_nine_slice_instance(
    input: &PackInput,
    region: u32,
    scale_factor: f32,
) -> Option<UiInstance> {
    let image = input.image?;
    let ns = input.nine_slice?;
    debug_assert!(scale_factor > 0.0, "invariant: UI scale_factor is positive");
    debug_assert!(
        region < UI_NINE_SLICE_REGIONS,
        "invariant: a UI nine-slice region is one of the {UI_NINE_SLICE_REGIONS} \
         row-major sub-quads"
    );
    debug_assert!(
        ns.mode < UI_NINE_SLICE_MODE_COUNT,
        "invariant: a UI nine-slice mode is a legal NineSliceMode discriminant \
         (< {UI_NINE_SLICE_MODE_COUNT}); the authored component carries the typed enum, \
         this is the raw byte that crossed the crate boundary"
    );
    debug_assert!(
        input.text_uv.is_none(),
        "invariant: a GLYPH row is never nine-sliced — FLAG_TEXT and FLAG_TEXTURED are \
         different quads with different shader branches, never one record wearing both"
    );
    debug_assert!(
        image.slot < boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY,
        "invariant: a UI sprite slot is a live bindless slot (< BINDLESS_TEXTURE_CAPACITY); \
         flags bits {UI_SLOT_SHIFT}..32 hold only {} of them",
        UI_SLOT_MASK + 1
    );
    debug_assert!(
        ns.border_px.iter().all(|v| v.is_finite() && *v >= 0.0),
        "invariant: a nine-slice destination border is finite and non-negative"
    );
    debug_assert!(
        ns.border_uv.iter().all(|v| v.is_finite() && (0.0..1.0).contains(v)),
        "invariant: each nine-slice source inset is a fraction of the sub-rect in [0, 1)"
    );
    debug_assert!(
        ns.border_uv[0] + ns.border_uv[2] < 1.0 && ns.border_uv[1] + ns.border_uv[3] < 1.0,
        "invariant: a nine-slice source split does not invert — each axis's two insets \
         sum to less than the whole sub-rect (release scales the axis down instead)"
    );

    let s = scale_factor;
    let col = (region % 3) as usize;
    let row = (region / 3) as usize;

    // Destination: cumulative cuts, outer edges pinned to the node's own rect.
    let dw = split_axis(ns.border_px[0], ns.border_px[2], input.rect[2]);
    let dh = split_axis(ns.border_px[1], ns.border_px[3], input.rect[3]);
    let xs = [
        input.rect[0],
        input.rect[0] + dw[0],
        input.rect[0] + dw[0] + dw[1],
        input.rect[0] + input.rect[2],
    ];
    let ys = [
        input.rect[1],
        input.rect[1] + dh[0],
        input.rect[1] + dh[0] + dh[1],
        input.rect[1] + input.rect[3],
    ];

    // Source: the same construction against the image's UV sub-rect, with the
    // insets read as fractions OF THAT SUB-RECT (never of the whole texture —
    // which is what keeps this correct when S5 makes the sub-rect a flipbook
    // frame that moves every tick).
    let du = image.uv[2] - image.uv[0];
    let dv = image.uv[3] - image.uv[1];
    let fu = split_axis(ns.border_uv[0], ns.border_uv[2], 1.0);
    let fv = split_axis(ns.border_uv[1], ns.border_uv[3], 1.0);
    let us = [
        image.uv[0],
        image.uv[0] + du * fu[0],
        image.uv[0] + du * (fu[0] + fu[1]),
        image.uv[2],
    ];
    let vs = [
        image.uv[1],
        image.uv[1] + dv * fv[0],
        image.uv[1] + dv * (fv[0] + fv[1]),
        image.uv[3],
    ];

    let mut flags = FLAG_TEXTURED | ((image.slot & UI_SLOT_MASK) << UI_SLOT_SHIFT);
    let clip = match input.clip {
        Some(c) => {
            flags |= FLAG_CLIP_PRESENT;
            [c[0] * s, c[1] * s, (c[0] + c[2]) * s, (c[1] + c[3]) * s]
        }
        None => [0.0; 4],
    };

    Some(UiInstance {
        min_px: [xs[col] * s, ys[row] * s],
        size_px: [(xs[col + 1] - xs[col]) * s, (ys[row + 1] - ys[row]) * s],
        clip,
        corner_radius: [0.0; 4],
        uv: [us[col], vs[row], us[col + 1], vs[row + 1]],
        color: premultiply_rgba8(image.tint),
        border_color: 0,
        border_width: 0.0,
        flags,
    })
}

/// The largest number of records ONE node can emit — the size of the sub-code
/// scratch [`ui_node_sub_codes`] fills. It is the EMISSION maximum (background +
/// every region), not the stride: the sub space has a hole in it, because
/// `UiNineSlice`'s presence suppresses the image record.
pub const UI_MAX_SUBS_PER_NODE: usize = 1 + UI_NINE_SLICE_REGIONS as usize;

/// **The SOLE authority on which sub-records a node emits** (S-D12 (3)) — the
/// one place S-D12 (1)'s truth table is written as code, and the one thing every
/// pack loop asks before it packs anything.
///
/// Writes the node's sub codes into `out` in D4's per-node emission order and
/// returns how many. Reading it is the whole of the truth table:
///
/// | `UiNineSlice` | `UiImage` | emits | subs |
/// |---|---|---|---|
/// | absent | absent | 1 | `0` |
/// | absent | present | 2 | `0`, [`UI_IMAGE_SUB`] |
/// | present | absent | 1 | `0` |
/// | present | present | 10 (9 without the centre) | `0`, `1..=9` |
///
/// # Why the PUSH and not the decode carries this
///
/// Because every decode arm's precondition is then established here, and no arm
/// can fail for any authored component set. The pre-S4 loop dispatched on a
/// BINARY `if` and ended its else-arm in `.expect(..)` over
/// [`pack_ui_image_instance`], which opens `let image = input.image?` — so a node
/// carrying `UiNineSlice` and no `UiImage` panicked in RELEASE as well as debug.
/// Widening the decode's `match` would have left the push free to emit a code
/// whose arm still had to cope; making the push the authority removes the
/// possibility instead of handling it (gate G4-8, red mutation M4-g).
pub fn ui_node_sub_codes(input: &PackInput, out: &mut [u32; UI_MAX_SUBS_PER_NODE]) -> usize {
    // Sub 0 — the node's own background rect. Every packable node has one.
    out[0] = 0;
    let mut n = 1;

    match (input.nine_slice, input.image) {
        // Sliced AND imaged: the nine sub-quads ARE the image, sliced, so the
        // whole-rect image record is NOT emitted (S-D12 (1)).
        (Some(ns), Some(_)) => {
            for r in 0..UI_NINE_SLICE_REGIONS {
                let sub = UI_NINE_SLICE_SUB_BASE + r;
                if sub == UI_NINE_SLICE_CENTER_SUB && !ns.fill_center {
                    continue;
                }
                out[n] = sub;
                n += 1;
            }
        }
        // Imaged only: S3's behaviour, byte-identical.
        (None, Some(_)) => {
            out[n] = UI_IMAGE_SUB;
            n += 1;
        }
        // Sliced only: a structural NO-OP. With no image there is no texture, no
        // source rect, and nothing for nine quads to be (S-D12 (3)).
        (Some(_), None) => {}
        (None, None) => {}
    }
    n
}

/// **The decode**: packs ONE of a node's sub-records, chosen by its sub code.
///
/// `sub` MUST be one [`ui_node_sub_codes`] emitted for this same `input` — which
/// is what makes every arm total and every `.expect` below unreachable for all
/// four component combinations (gate G4-8). It is a PURE function of
/// `(input, sub, scale_factor)`, which is what lets the in-schedule loop pack
/// directly in SORTED order, recovering each record's source from its key alone.
pub fn pack_ui_sub_record(input: &PackInput, sub: u32, scale_factor: f32) -> UiInstance {
    match sub {
        0 => pack_ui_instance(input, scale_factor),
        UI_IMAGE_SUB => pack_ui_image_instance(input, scale_factor).expect(
            "invariant: the image sub code is emitted only for a node carrying UiImage and \
             no UiNineSlice — ui_node_sub_codes is the sole authority",
        ),
        s => pack_ui_nine_slice_instance(input, s - UI_NINE_SLICE_SUB_BASE, scale_factor).expect(
            "invariant: a nine-slice sub code is emitted only for a node carrying BOTH \
             UiNineSlice and UiImage — ui_node_sub_codes is the sole authority",
        ),
    }
}

/// The **loop-agnostic emitter**: appends every record of one node, in D4's
/// per-node emission order, into a caller-supplied sink; returns how many.
///
/// This is the seam the expansion policy lives behind. **There are THREE routes
/// through the pack, and two of them are correct:**
///
/// 1. **This one.** It `push`es in `subs[..n]` order and never sorts, so its push
///    order IS the emission order — the only route on which that is observable,
///    and the reason `ui_no_realloc.rs`'s
///    `ui_nine_slice_emitter_pushes_in_d4_order` exists.
/// 2. **The in-schedule loop**
///    ([`UiUploadSystem::gather_into_staging`](crate::ui::upload::UiUploadSystem::gather_into_staging)).
///    It does not call this one — it must write into a FIXED box by sorted index,
///    so it drives the same two functions ([`ui_node_sub_codes`] then
///    [`pack_ui_sub_record`]) directly. Same authority, same decode, which is what
///    lets a test drive the production expansion into its own scratch instead of
///    hand-rolling it (gate G4-4). Because it sorts on the sub CODE, its output is
///    invariant to the order the codes were pushed in.
/// 3. ⚠️ **[`UiUploadSystem::pack_sort_upload`](crate::ui::upload::UiUploadSystem::pack_sort_upload),
///    which re-implements the expansion at S3 semantics and is therefore WRONG** —
///    a node with both `UiNineSlice` and `UiImage` gets an unsliced whole-rect
///    image, the picture S-D12 (1) rules out. It has no caller in the workspace
///    and whether it is deleted or wired is an owner SCOPE call already filed
///    (`docs/OPEN-QUESTIONS.md`, entry 2026-08-21), so it is left standing rather
///    than given a second, unrunnable copy of this policy. If it is ever wired it
///    must be replaced by a call to THIS function.
///
/// Allocation-free in steady state: it only `push`es, so a warmed sink never
/// grows.
pub fn emit_ui_node_records(
    input: &PackInput,
    scale_factor: f32,
    sink: &mut Vec<UiInstance>,
) -> usize {
    let mut subs = [0u32; UI_MAX_SUBS_PER_NODE];
    let n = ui_node_sub_codes(input, &mut subs);
    for &sub in &subs[..n] {
        sink.push(pack_ui_sub_record(input, sub, scale_factor));
    }
    n
}

/// Reused per-frame UI render scratch (Principle 0 storage — a `Resource`, NOT a
/// side store). Allocated/grown ONLY at setup or on a capacity-crossing frame; a
/// steady-state frame only `clear()`s + `extend`s + sorts in place (capacity
/// persists), so there is zero steady-state allocation.
#[derive(Resource)]
pub struct UiRenderScratch {
    /// Packed records, sorted by `StackIndex`; `clear()` + `extend`, never `Vec::new`.
    pub pack: Vec<UiInstance>,
    /// Parallel sort-key lane `(stack_index, append_order)` — capacity-stable,
    /// reused; `sort_unstable_by_key` then gather (both `O(N log N)`, zero alloc).
    /// Append order is the natural tie-break (filled in traversal order); because
    /// `append_order` is unique the key is a TOTAL order, so an unstable sort is a
    /// permutation identical to a stable one (and avoids timsort's per-call merge
    /// buffer — keeping the per-frame allocation count at zero).
    pub keys: Vec<(u32, u32)>,
    /// The instance count uploaded last frame (for the change gate / debug).
    pub last_count: u32,
    /// DIAGNOSTIC (S0 item 6, deliberately NOT `#[cfg(test)]` — the §10.4
    /// `relayout_count` lesson): repacks ever executed by
    /// [`pack_sort_upload`](crate::ui::upload::UiUploadSystem::pack_sort_upload)
    /// (the LEGACY path — which has NO caller in the workspace, so this counter
    /// reads zero in every process; see that method's doc), wrapping. Sample
    /// before/after a frame for
    /// a per-frame count. The in-schedule two-phase seam keeps its OWN census
    /// on the system ([`UiUploadSystem::repacks`]) — the D6a per-slot gate and
    /// its `[u64; FRAMES_IN_FLIGHT]` state live there too, because Phase 1
    /// reads the world through a read-only [`WorldView`] that cannot project
    /// `&mut` to this `Resource`.
    ///
    /// [`UiUploadSystem::repacks`]: crate::ui::upload::UiUploadSystem::repacks
    /// [`WorldView`]: boyko_ecs::ecs::core::system::dispatcher_token::WorldView
    pub repacks: u64,
}

impl Default for UiRenderScratch {
    /// Empty buffers; capacity arrives with the first pack and persists.
    fn default() -> Self {
        UiRenderScratch {
            pack: Vec::new(),
            keys: Vec::new(),
            last_count: 0,
            repacks: 0,
        }
    }
}

impl UiRenderScratch {
    /// Sorts the packed records by `(StackIndex, append_order)` using the parallel
    /// key lane, in place, zero alloc (A1 step 3). `keys[i]` must hold
    /// `(stack_index, i)` for each packed record `pack[i]` before the call.
    ///
    /// The result is painter's order: `StackIndex` ascending, ties broken by
    /// append (query-traversal) order. The key `(stack, append_idx)` is a TOTAL
    /// order because `append_idx` is unique per record, so an UNSTABLE sort yields
    /// exactly the stable ordering while avoiding timsort's per-call n/2 merge-buffer
    /// allocation (the per-frame allocation budget is zero — Decision 5 / A1).
    /// The gather then materializes `pack` in key order via a reused scratch swap;
    /// because the key lane encodes the append index, it is a permutation — no
    /// record is dropped or duplicated.
    pub fn sort_by_stack(&mut self, gather: &mut Vec<UiInstance>) {
        debug_assert_eq!(
            self.keys.len(),
            self.pack.len(),
            "invariant: the key lane has one entry per packed record"
        );
        // Unstable sort by (stack, append_idx): append_idx makes the key a total
        // order, so the unstable result == the stable result, with zero allocation.
        self.keys.sort_unstable_by_key(|&(stack, idx)| (stack, idx));
        gather.clear();
        gather.reserve(self.pack.len());
        for &(_, idx) in &self.keys {
            gather.push(self.pack[idx as usize]);
        }
        core::mem::swap(&mut self.pack, gather);
    }
}

/// The monotonic UI-render generation counter (A1 step 1) — a `Resource` bumped
/// once per changed frame by
/// [`ui_render_discovery`](crate::ui::gather::ui_render_discovery) (the ONE
/// production bump site since UI-ADVANCED S0; the host additionally bumps on a
/// DPI/scale change, which no component carries). The two-phase upload seam's
/// gate is one `u64` compare PER frame-in-flight slot, hoisted AHEAD of the
/// gather in Phase 1 of
/// [`UiUploadSystem::run_dispatcher`](crate::ui::upload::UiUploadSystem):
/// a static frame costs one compare and ZERO component probes — an O(1) skip,
/// not an O(N) Changed scan (the discovery system pays that scan once,
/// archetype-filtered, for the whole set).
#[derive(Resource, Default)]
pub struct UiRenderGeneration {
    /// The current generation; bumped on any pack-input change.
    pub generation: u64,
}

impl UiRenderGeneration {
    /// Bumps the generation, forcing the next frame's upload to repack. Cheap and
    /// alloc-free; called by every writer of a pack input.
    #[inline]
    pub fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}
