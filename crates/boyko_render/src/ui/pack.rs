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
    premultiply_rgba8, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, FLAG_TEXT, UiInstance,
};

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
    /// bottom)` in `[0, 1]`, written verbatim (NOT scale-folded) into the
    /// `corner_radius` alias with `FLAG_TEXT` set; `rect` is then the glyph quad
    /// (already physical-or-logical px, scale-folded like a rect), `color` the
    /// premultiplied-at-pack foreground, and `border_*` are ignored. `None` ⇒ the
    /// rect path (P5a, unchanged).
    pub text_uv: Option<[f32; 4]>,
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

    // GUI P5b text branch (Decision T4-G): a glyph quad. The UV rect aliases
    // `corner_radius` (written verbatim, NOT scale-folded — it is already normalized);
    // `FLAG_TEXT` selects the MSDF branch in the FS. Border is N/A for a glyph.
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
            corner_radius: uv,
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
        color: premultiply_rgba8(input.color),
        border_color: premultiply_rgba8(input.border_color),
        border_width,
        flags,
    }
}

/// Reused per-frame UI render scratch (Principle 0 storage — a `Resource`, NOT a
/// side store). Allocated/grown ONLY at setup or on a capacity-crossing frame; a
/// steady-state frame only `clear()`s + `extend`s + sorts in place (capacity
/// persists), so there is zero steady-state allocation.
#[derive(Resource, Default)]
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
    /// The last generation seen — the O(1) change gate (A1 step 1): a static frame
    /// short-circuits on `gen == last_seen_generation`.
    pub last_seen_generation: u64,
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

/// The monotonic UI-render generation counter (A1 step 1) — a `Resource` bumped by
/// any writer of the pack inputs (`ComputedRect` via the layout system,
/// `UiBackground` / `StackIndex` / `ComputedClip` via authoring/commands, and the
/// viewport/swapchain extent). The upload system's gate is one `u64` compare:
/// `if gen == scratch.last_seen_generation { return; }` — the 0%-when-static
/// guarantee is an O(1) compare, not an O(N) Changed scan.
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
