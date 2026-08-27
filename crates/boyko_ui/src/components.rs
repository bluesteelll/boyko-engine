//! ECS components — the layout inputs and computed outputs.
//!
//! Every component is its own SoA column inside the archetype; change detection
//! is per-component-per-row, so the inputs are split by churn profile (a node
//! animating only its size bumps only [`UiLayout`]'s tick). All are POD `Copy`
//! (`Send + Sync`), so they are trivially safe to read on the layout pass.
//!
//! The `boyko` `Component` derive is a pure marker: it adds no fields and only
//! assigns a lazily-allocated `ComponentId`, so it coexists with `#[repr(C)]`.

use core::cmp::Ordering;

use boyko_macros::{Bundle, Component};

use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};

/// Primary layout input. HOT: read for every node every pass.
///
/// `Changed<UiLayout>` is the primary relayout trigger.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct UiLayout {
    /// Container direction (or `Overlay`).
    pub layout_type: LayoutType,
    /// In-flow vs out-of-flow.
    pub position_type: PositionType,
    /// Preferred width.
    pub width: Unit,
    /// Preferred height.
    pub height: Unit,
    /// Minimum width (`Auto` = content floor).
    pub min_width: Unit,
    /// Minimum height (`Auto` = content floor).
    pub min_height: Unit,
    /// Maximum width (default `Px(f32::MAX)` — finite unbounded sentinel).
    pub max_width: Unit,
    /// Maximum height (default `Px(f32::MAX)` — finite unbounded sentinel).
    pub max_height: Unit,
}

impl Default for UiLayout {
    #[inline]
    fn default() -> Self {
        Self {
            layout_type: LayoutType::Column,
            position_type: PositionType::Relative,
            width: Unit::Auto,
            height: Unit::Auto,
            // Auto min = content floor (resolved during the intrinsic measure).
            min_width: Unit::Auto,
            min_height: Unit::Auto,
            // Unbounded-max sentinel is f32::MAX (FINITE), NOT INFINITY: an
            // INFINITY upper bound can produce NaN through `0.0 * INFINITY` in a
            // degenerate stretch round; a NaN rect never compares equal under
            // derived PartialEq, so set-if-changed would bump the tick every
            // frame forever (defeating 0%-overhead). f32::MAX is clamp-equivalent
            // for all realistic sizes and finite.
            max_width: Unit::Px(f32::MAX),
            max_height: Unit::Px(f32::MAX),
        }
    }
}

/// Parent-applied spacing (padding, layout-inset border, gaps). HOT on
/// containers, cold/absent on leaves.
///
/// `border_*` is a pure layout inset (it shrinks the content box); the visual
/// border is a separate render concern (P5a). `margin` (child-applied spacing)
/// is deferred. Gaps may be `Stretch` (parent-applied stretch spacing) and are
/// then handled by the freeze loop.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct UiSpacing {
    /// Left padding.
    pub padding_left: Unit,
    /// Right padding.
    pub padding_right: Unit,
    /// Top padding.
    pub padding_top: Unit,
    /// Bottom padding.
    pub padding_bottom: Unit,
    /// Left layout-inset border.
    pub border_left: Unit,
    /// Right layout-inset border.
    pub border_right: Unit,
    /// Top layout-inset border.
    pub border_top: Unit,
    /// Bottom layout-inset border.
    pub border_bottom: Unit,
    /// Gap between children when main = y (Column).
    pub row_gap: Unit,
    /// Gap between children when main = x (Row).
    pub column_gap: Unit,
}

impl Default for UiSpacing {
    #[inline]
    fn default() -> Self {
        let zero = Unit::Px(0.0);
        Self {
            padding_left: zero,
            padding_right: zero,
            padding_top: zero,
            padding_bottom: zero,
            border_left: zero,
            border_right: zero,
            border_top: zero,
            border_bottom: zero,
            row_gap: zero,
            column_gap: zero,
        }
    }
}

/// Alignment of children within a container. COLD: read once per container;
/// often default.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiAlign {
    /// Main-axis distribution of leftover free space.
    pub main: AlignMain,
    /// Cross-axis placement of each child.
    pub cross: AlignCross,
}

/// Absolute (self-directed) offsets. COLD + OPT-IN: present ONLY on nodes whose
/// `UiLayout.position_type == Absolute`.
///
/// `before` (left/top) wins over `after` (right/bottom) when both are set.
/// `Auto` means "unset". `Changed<UiAbsolute>` is a relayout scan term.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiAbsolute {
    /// Offset from the parent's left content edge.
    pub left: Unit,
    /// Offset from the parent's right content edge.
    pub right: Unit,
    /// Offset from the parent's top content edge.
    pub top: Unit,
    /// Offset from the parent's bottom content edge.
    pub bottom: Unit,
}

/// Leaf intrinsic size (the content-size seam). COLD, OPT-IN.
///
/// In P1 (no text shaping — P5b) this is an authored/image-derived fixed size
/// that `Auto` leaves hug. Layout only READS it. `Changed<ContentSize>` triggers
/// relayout. P5b replaces the *source* of this component; the layout code is
/// unchanged.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ContentSize {
    /// Intrinsic content width (0.0 if none).
    pub width: f32,
    /// Intrinsic content height (0.0 if none).
    pub height: f32,
}

/// The resolved screen-space rectangle — the ONLY geometry the renderer reads
/// (P5a). HOT: written for every laid-out node.
///
/// `#[repr(C)]`, 16 B = one aligned store target, clean for the instanced-quad
/// upload. NaN never appears (clamps + `f32::MAX` sentinel + finite-assert before
/// every write). Layout writes this set-if-changed.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ComputedRect {
    /// Top-left x, logical px (+x right).
    pub x: f32,
    /// Top-left y, logical px (+y down).
    pub y: f32,
    /// Width, logical px.
    pub w: f32,
    /// Height, logical px.
    pub h: f32,
}

/// Draw-order / z key within a root. COLD, OPT-IN (default 0).
///
/// AUTHOR-OWNED in P1 — layout never reads/writes it. The renderer (P5a) sorts
/// by it.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackIndex(pub u32);

/// Clip rectangle for overflow. COLD, OPT-IN.
///
/// AUTHOR-OWNED in P1 (not computed); consumed by P5a's scissor. P1's overflow
/// policy is "allow overflow" (layout does not clip).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ComputedClip {
    /// Clip rect top-left x.
    pub x: f32,
    /// Clip rect top-left y.
    pub y: f32,
    /// Clip rect width.
    pub w: f32,
    /// Clip rect height.
    pub h: f32,
}

/// Visual fill + border style for a node (GUI P5a). AUTHOR-OWNED, OPT-IN: a node
/// WITHOUT this component is layout-only / invisible; a node with the [`Default`]
/// (transparent, no border) is likewise invisible — authors OPT IN to a visible
/// fill/border. Read by the P5a pack system together with [`ComputedRect`].
///
/// `#[repr(C)]`, 44 B align 4 (`u32×2 + f32×4 + f32×4 + u32`, no tail pad — const-
/// asserted). POD `Copy`; its own SoA column; the change gate covers its writers.
///
/// Colors are authored STRAIGHT RGBA8 (`byte0=R .. byte3=A`); the pack system
/// premultiplies them into the GPU record. `corner_radius` is `tl, tr, br, bl`
/// (the `sdRoundedBox` per-corner select order). `border_width` is per-side `l, t,
/// r, b` for forward-compat, but **P5a renders a UNIFORM border only**: the pack
/// uses `border_width[0]` and `debug_assert!`s the four sides are equal (asymmetric
/// per-side borders are a deferred phase with the correct per-side inner-SDF).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiBackground {
    /// Fill color, STRAIGHT RGBA8 (`byte0=R .. byte3=A`); premultiplied at pack.
    pub color: u32,
    /// Border color, STRAIGHT RGBA8; premultiplied at pack.
    pub border_color: u32,
    /// Per-corner radius `tl, tr, br, bl`, logical px.
    pub corner_radius: [f32; 4],
    /// Per-side border width `l, t, r, b`, logical px (0 = no border that side).
    /// P5a uses a UNIFORM width (`border_width[0]`); per-side is deferred.
    pub border_width: [f32; 4],
    /// Reserved flags (P5a derives the GPU `UiInstance.flags` at pack time).
    pub flags: u32,
}

const _: () = assert!(size_of::<UiBackground>() == 44);
const _: () = assert!(align_of::<UiBackground>() == 4);

impl Default for UiBackground {
    /// Transparent fill, no border, zero radius — an invisible node (authors opt in
    /// to a visible fill/border).
    #[inline]
    fn default() -> Self {
        UiBackground {
            color: 0,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            flags: 0,
        }
    }
}

/// Marks a screen-space root: the layout entry points.
///
/// A NORMAL marker component (NOT a bitset tag) so it is ENUMERABLE via
/// `query_entities(&[UiRoot::component_id()])` and `Added<UiRoot>`. A root's
/// `ChildOf` (if any) is ignored for layout; the root rect is seeded from the
/// viewport.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiRoot;

/// Stable author-assigned name for a node. COLD, OPT-IN: present only on
/// `#named` nodes authored through the [`ui!`](boyko_macros::ui) macro (P2).
///
/// It is the diff key for the P3 `.ui` hot-reload pass: two `UiName` columns are
/// compared with a fixed-size `memcmp` (`Copy`/`Eq`), with no interner and no
/// global string table — Principle 1/5. The name is stored inline (no heap), so
/// it is POD `Copy`/`Send`/`Sync` like every other layout component and stores
/// in a contiguous SoA column with no indirection.
///
/// The buffer is 60 bytes; the `ui!` macro rejects longer names at compile time
/// with a span on the offending `#name`. The struct is one cache line (64 B,
/// `align(64)`) for a clean column stride.
#[repr(C, align(64))]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiName {
    /// UTF-8 bytes of the name; only the first `len` are meaningful, the rest
    /// are zero.
    bytes: [u8; Self::CAP],
    /// Valid byte count in `bytes` (`<= CAP`).
    len: u8,
    /// Pad to one full cache line (64 B).
    _pad: [u8; 3],
}

impl UiName {
    /// Maximum name length in bytes (UTF-8). 60 B keeps the struct at exactly
    /// one cache line; the `ui!` macro enforces this at compile time.
    pub const CAP: usize = 60;

    /// Copies `s` into the inline buffer.
    ///
    /// `s.len()` MUST be `<= CAP`; the `ui!` macro enforces this at compile time
    /// on the string literal, so the runtime path only `debug_assert!`s it (the
    /// P3 text path is the other caller and is responsible for its own bound
    /// check before calling). `const` so the `ui!` literal path is const-foldable.
    ///
    /// If an over-length name reaches this path in release (the `debug_assert!`
    /// is gone), the copy is truncated at the last UTF-8 char boundary at or
    /// before `CAP` — never at a partial multi-byte char — so `as_str` stays
    /// sound.
    pub const fn new(s: &str) -> Self {
        debug_assert!(s.len() <= Self::CAP, "invariant: ui name exceeds CAP");
        let src = s.as_bytes();
        let mut bytes = [0u8; Self::CAP];
        // `const fn` cannot use `copy_from_slice`/iterators with `?`; a manual
        // index loop is the const-compatible byte copy. `n` is clamped to CAP so
        // an over-length name (which the macro already rejects) cannot overrun.
        let mut n = if src.len() < Self::CAP { src.len() } else { Self::CAP };
        // If the clamp cut inside a multi-byte UTF-8 char (only possible when we
        // truncated, i.e. `n < src.len()`), back `n` off to the preceding char
        // boundary: continuation bytes match `(b & 0xC0) == 0x80`, so we drop the
        // trailing partial sequence entirely. Without this a straddling char would
        // leave an invalid-UTF-8 prefix that `as_str`'s `from_utf8_unchecked` reads
        // as UB. `n < src.len()` guards `src[n]` in bounds; `n > 0` guards the
        // decrement. A single boundary is at most 3 continuation bytes back (UTF-8
        // scalar ≤ 4 bytes), so this loop cannot underflow a valid CAP-sized cut.
        while n < src.len() && n > 0 && (src[n] & 0xC0) == 0x80 {
            n -= 1;
        }
        let mut i = 0;
        while i < n {
            bytes[i] = src[i];
            i += 1;
        }
        Self { bytes, len: n as u8, _pad: [0; 3] }
    }

    /// The name as a string slice. COLD (debug / diff display only).
    #[inline]
    pub fn as_str(&self) -> &str {
        debug_assert!(self.len as usize <= Self::CAP, "invariant: UiName len exceeds CAP");
        // SAFETY: `new` copies bytes verbatim from a `&str` (already valid UTF-8) into
        // `bytes[..len]`, and when it truncates it backs `len` off to a UTF-8 char
        // boundary (never leaving a partial multi-byte sequence). So `bytes[..len]` is
        // always a valid-UTF-8 prefix of the source, and `len <= CAP` (debug-asserted
        // above and clamped in `new`). `from_utf8_unchecked` is therefore sound.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }

    /// The name length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the name is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

// A documented total order over the meaningful UTF-8 prefix, tie-broken on
// length (P3 Decision 9). This is the diff key for the `.ui` hot-reload
// reconcile: a per-parent index is a `sorted Vec<(UiName, Entity)>` + binary
// search, which is not expressible without `Ord`.
//
// The order is consistent with the derived `Eq`: two `UiName`s are `Eq` iff
// `len` is equal and `bytes[..len]` are equal, and `cmp` returns `Equal`
// EXACTLY then — the trailing `bytes[len..]` are always zero (written by
// `new`) and `_pad` is always `[0; 3]`, so they never perturb the prefix
// comparison. A blanket `#[derive(Ord)]` would instead order by the full fixed
// buffer (including `_pad`), so the comparison is written by hand to compare
// only the live prefix. memcmp over a POD column slice — reflection-free
// (Principle 1/5).
impl Ord for UiName {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes[..self.len as usize]
            .cmp(&other.bytes[..other.len as usize])
            .then(self.len.cmp(&other.len))
    }
}

impl PartialOrd for UiName {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ───────────────────────── GUI P6a widget components ──────────────────────
//
// The 6 HUD widgets are realized ECS-natively (Principle 0): a widget is NOT a
// `Box<dyn>` object or a runtime `Widget` enum but a deterministic SET of
// components composed on the P1–P5b substrate. Where a widget needs an identity
// beyond its component set, it carries a ZST marker (the `UiRoot` precedent) so
// it is enumerable (`query_entities(&[Button::component_id()])`) and filterable
// (`Added<Button>`); where it needs config, it carries a small POD struct.
//
// The CANONICAL authorable form is this component set — identical across
// `ui!` (which lowers each literal as an `.insert`), `.ui` (the closed-match
// dispatch, extended for these type-names), and a hand-spawn. The preset bundles
// in `bundles.rs` are a Rust-only ergonomic convenience that expands to the SAME
// component set; they are NOT `ui!`/`.ui` type-names (the `ui!` macro requires a
// `UiLayout` literal per node, which a bundle name is not — C1).

/// Marks a node as a Button: an interactive styled panel
/// (`UiLayout` + `UiBackground` + `Interaction` + `Focusable` + `OnClick`).
///
/// A ZST marker (the [`UiRoot`] precedent) so a Button is ENUMERABLE /
/// `Added<Button>`-filterable distinctly from a plain interactive panel, and is a
/// stable `.ui` type-name. Carries no fields.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Button;

/// Marks a node as a Bar TRACK: a progress/health bar whose single
/// [`BarFill`]-marked child's main-axis size tracks a `0..1` value.
///
/// The track hosts the value in [`UiValue`](crate::binding::UiValue) (the P4
/// `BindValue` sink, reused verbatim). A ZST marker so the bar driver can
/// enumerate tracks; carries no fields.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar;

/// Marks the fill child of a [`Bar`]: the node whose main-axis `Unit::Pct` is
/// driven by `bar_fill_system` from the track's `UiValue`.
///
/// A ZST marker so the driver finds the fill child among the track's children;
/// carries no fields.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BarFill;

/// Image fill for a node (GUI P6a). AUTHOR-OWNED, OPT-IN. `#[repr(C)]`, POD.
///
/// `texture` is a DENSE `u32` handle into the (future) UI texture table — NOT a
/// string / `HashMap` key (the [`FontId`](crate::text::FontId) dense-handle
/// discipline, Principle 1). `tint` is STRAIGHT RGBA8 (premultiplied at pack,
/// the [`UiBackground`] convention).
///
/// # Render seam (P6a vs P5a)
///
/// P6a creates this component so an Image node is layout-complete and authorable
/// in `ui!`/`.ui`; the P5a pack path does NOT yet consume it, so an Image renders
/// nothing until a P5a follow-up learns `UiImage`. The [`Default`] is texture 0
/// with a FULLY TRANSPARENT tint (alpha 0) — an authored-but-untextured Image is
/// invisible (it never flashes a white box when P5a lands), mirroring
/// `UiBackground`'s transparent default (m1).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiImage {
    /// Dense handle into the (future) UI texture table (`0` = no texture).
    pub texture: u32,
    /// Atlas sub-rect min (`u, v`).
    pub uv_min: [f32; 2],
    /// Atlas sub-rect max (`u, v`).
    pub uv_max: [f32; 2],
    /// Tint, STRAIGHT RGBA8 (`byte0=R .. byte3=A`); premultiplied at pack.
    pub tint: u32,
}

const _: () = assert!(size_of::<UiImage>() == 24);
const _: () = assert!(align_of::<UiImage>() == 4);

impl Default for UiImage {
    /// Texture 0, full UV (`0..1`), FULLY TRANSPARENT tint (alpha 0) — an
    /// invisible node until both a texture and an opaque tint are authored (m1:
    /// never a white flash once P5a packs it).
    #[inline]
    fn default() -> Self {
        UiImage {
            texture: 0,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: 0,
        }
    }
}

/// How a [`UiNineSlice`]'s EDGE and CENTRE regions fill their destination
/// (UI-ADVANCED S4). `#[repr(u8)]`, so the render side can carry the
/// discriminant as a raw `u8` across the crate boundary.
///
/// At S4 there was exactly ONE legal value, pinned mechanically by the
/// one-variant `const` match below rather than asserted in prose. **S5 added
/// [`Tile`](NineSliceMode::Tile), and the mechanism worked exactly as
/// designed**: the match went `error[E0004]: non-exhaustive patterns` and
/// walked the author to the gather's narrowing site, which is the one place
/// that must bump `UI_NINE_SLICE_MODE_COUNT`.
/// (`std::mem::variant_count` would be the obvious spelling; MEASURED on rustc
/// 1.97.1 it is `E0658` *and* "not yet stable as a const fn" — two errors on
/// one line — so it does not exist on this toolchain.)
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NineSliceMode {
    /// The edges and the centre are STRETCHED to fill their destination
    /// region — one copy of the source region, scaled.
    #[default]
    Stretch = 0,
    /// The edges and the centre REPEAT their source region across the
    /// destination, at a count DERIVED from the two borders (UI-ADVANCED S5,
    /// `docs/UI-PLAN-SPRITES.md` S-D15).
    ///
    /// # The count is derived, not authored, and it needs no texture size
    ///
    /// The engine records a texture's dimensions nowhere, so the reference
    /// engines' `dest_px / source_px` is unavailable. It is not needed: a
    /// nine-slice already states its own source→destination scale twice —
    /// [`border_px`](UiNineSlice::border_px) is a corner's destination size
    /// and [`border_uv`](UiNineSlice::border_uv) is the same corner's source
    /// extent — and their ratio IS the scale. The sub-rect's own extent
    /// CANCELS out of that ratio, which is what makes the count identical
    /// under a whole texture and under a [`UiSpriteSheet`] frame.
    ///
    /// A nine-slice with a zero border on an axis has no scale to read and
    /// does not get to guess one: that axis renders as `Stretch`.
    ///
    /// # It composes with a sprite sheet, rather than colliding with one
    ///
    /// The wrap is `frac` of the QUAD PARAMETER, applied inside the record's
    /// own UV sub-rect, so the sample never leaves that sub-rect for any
    /// count. Under a sheet the sub-rect is one frame, so `Tile` repeats
    /// within the frame and cannot reach its neighbours.
    Tile = 1,
}

// S4's "exactly one legal value" made mechanical, and S5's "exactly two". NO
// outer braces: MEASURED — `const _: () = { match … };` emits `unused_braces`,
// which the project's `clippy --all-targets -- -D warnings` gate turns into an
// error.
const _: () = match NineSliceMode::Stretch {
    NineSliceMode::Stretch => (),
    NineSliceMode::Tile => (),
};

/// Nine-slice (nine-patch) rendering for a node's [`UiImage`] (UI-ADVANCED S4).
/// AUTHOR-OWNED, OPT-IN, COLD table storage. `#[repr(C)]`, POD, 36 B.
///
/// # Presence IS the statement "draw my image sliced"
///
/// This component does not add a layer on top of the sprite — it changes HOW
/// the sprite is drawn. A node carrying both `UiNineSlice` and [`UiImage`]
/// emits its background rect plus NINE sub-quads and **no whole-rect image
/// record**; the slices *are* the image (`docs/UI-PLAN-SPRITES.md` S-D12 (1),
/// which is what Unity's `Image{type: Sliced}`, Godot's `NinePatchRect` and
/// Bevy's `NodeImageMode::Sliced` all do — none of them draws the image twice).
/// A node carrying `UiNineSlice` and NO `UiImage` is a structural **no-op**: it
/// emits its background and nothing else (S-D12 (3)) — never nine invisible
/// quads.
///
/// A frame that wants a nine-sliced border AND an unsliced picture inside it is
/// TWO nodes — a nine-sliced parent with an imaged child — exactly as it is
/// expressed in Unity, Godot and Bevy, and the hierarchy for it already exists.
///
/// # The two borders
///
/// * [`border_px`](Self::border_px) is the DESTINATION inset, in logical px.
/// * [`border_uv`](Self::border_uv) is the SOURCE inset, as a FRACTION of the
///   node's current [`UiImage`] UV sub-rect — not in source texels, because the
///   engine never records a texture's dimensions anywhere (the bindless table
///   registers a bare image view and stores no size). A fraction needs no
///   dimensions by construction, and it stays correct when S5 makes the
///   sub-rect a flipbook frame that changes every tick.
///
/// Both are `[l, t, r, b]` — [`UiBackground::border_width`]'s side order, NOT
/// [`UiBackground::corner_radius`]'s `tl, tr, br, bl`.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiNineSlice {
    /// DESTINATION inset per side, logical px, `[l, t, r, b]`.
    ///
    /// **The [`Default`] is `[0.0; 4]`, and the picture that produces is
    /// ACCEPTED rather than guarded**: with a zero destination inset every
    /// corner and edge region has zero extent, so the only visible sub-quad is
    /// the centre — the node renders **the middle ninth of its texture, zoomed
    /// to fill**. Because presence SUPPRESSES the whole-rect image record, a
    /// defaulted `UiNineSlice` does NOT degrade to an unsliced sprite. That is
    /// the same shape as [`UiImage`]'s alpha-0 default tint: the
    /// zero-configuration value of an authored component is the null one and
    /// the author sees the result immediately. It is stated here because an
    /// unstated degenerate default is the datum an author discovers by acting
    /// on it.
    ///
    /// If `l + r` exceeds the node's width (a chrome tweened below its own
    /// border) the axis's two sides are shrunk PROPORTIONALLY at pack, per
    /// axis — what Unity and Godot both do — so the corners never overlap and
    /// the edges never invert. That is ordinary, not an error, and is silent.
    ///
    /// A NEGATIVE side is out of domain (`debug_assert!`ed at pack) and is
    /// clamped to zero in release — the sum test above cannot see it, and left
    /// alone it produces a negative-extent destination rect.
    pub border_px: [f32; 4],
    /// SOURCE inset per side as a fraction of the node's current UV sub-rect,
    /// `[l, t, r, b]`. `Default` = equal thirds (`1/3` each side), which is the
    /// zero-configuration split and NOT the rule — a 32×32 chrome with an 8 px
    /// border wants `1/4`, and a 64×64 panel with a 6 px border wants `3/32`.
    ///
    /// Valid domain: each side in `[0, 1)` with `l + r < 1` and `t + b < 1`
    /// (`debug_assert!`ed at pack). In release an axis whose sides sum to `1`
    /// or more is scaled down proportionally, so the centre SOURCE region
    /// degenerates to zero width instead of inverting into a negative-extent
    /// UV rect; a side below `0` is clamped to zero, which is the OTHER edge of
    /// the domain and needs the other remedy — a negative inset is not a
    /// proportion of anything, and no sum test sees it.
    pub border_uv: [f32; 4],
    /// How the edges and the centre fill their destination —
    /// [`NineSliceMode::Stretch`] (the `Default`) or
    /// [`NineSliceMode::Tile`] (UI-ADVANCED S5). The four CORNERS are
    /// unaffected by either: a corner is exactly `border_px` in both spaces,
    /// so it has nothing to stretch or repeat, and it packs byte-identically
    /// under both modes.
    pub mode: NineSliceMode,
    /// Emit the CENTRE sub-quad (region 4, sub code 5)? `false` leaves the
    /// centre hole unpainted — the node's background shows through — and the
    /// node emits 9 records rather than 10.
    ///
    /// **The [`Default`] is `true`**, and it is ruled rather than inherited
    /// from `bool::default()`: `false` combined with `border_px`'s `[0.0; 4]`
    /// default would make a defaulted `UiNineSlice` emit a background plus
    /// EIGHT zero-extent slices and render nothing at all, while the image it
    /// suppressed went undrawn. Every record count in this rung's gates is
    /// stated for the `true` row of S-D12 (1)'s truth table.
    pub fill_center: bool,
    /// Explicit tail padding to 36 B. SPELLED rather than implicit: MEASURED on
    /// rustc 1.97.1, the field list without it is *also* 36 B / align 4, i.e.
    /// the two bytes are implicit tail padding — which is precisely what "the
    /// padding is spelled" forbids leaving unwritten in a `#[repr(C)]` POD.
    pub _pad: [u8; 2],
}

const _: () = assert!(size_of::<UiNineSlice>() == 36);
const _: () = assert!(align_of::<UiNineSlice>() == 4);

impl Default for UiNineSlice {
    /// Zero destination inset, equal-thirds source split, `Stretch`, centre ON.
    ///
    /// See [`border_px`](UiNineSlice::border_px) for the picture the zero
    /// destination inset produces — it is accepted, not guarded.
    #[inline]
    fn default() -> Self {
        UiNineSlice {
            border_px: [0.0; 4],
            border_uv: [1.0 / 3.0; 4],
            mode: NineSliceMode::Stretch,
            fill_center: true,
            _pad: [0; 2],
        }
    }
}

/// Draw a [`UiImage`] as ONE FRAME of a sprite sheet (UI-ADVANCED S5).
/// AUTHOR-OWNED, OPT-IN, TABLE storage. `#[repr(C)]`, POD, 4 B.
///
/// # The sheet OVERRIDES `UiImage`; it does not replace it
///
/// [`UiImage`] remains the capability — a node carrying `UiSpriteSheet` and no
/// `UiImage` draws its background alone, the same structural skip
/// `UiNineSlice` alone takes. What this component changes is what the sprite
/// record SAMPLES: the render gather substitutes the frame's computed UV
/// sub-rect and the sheet's own bindless slot into the image inputs, so
/// [`UiImage::uv_min`]/[`uv_max`](UiImage::uv_max)/[`texture`](UiImage::texture)
/// stop being read while [`tint`](UiImage::tint) still is.
///
/// That substitution site is why [`UiNineSlice::border_uv`]'s "a fraction of
/// the node's CURRENT `UiImage` UV sub-rect" stays literally true, and why a
/// nine-sliced sheet-framed node slices THE FRAME rather than the atlas — the
/// two components compose with no code between them.
///
/// # `index` is what the flipbook writes, and that is deliberate
///
/// [`ui_sprite_flipbook`](crate::sprite::ui_sprite_flipbook) writes this field
/// (through `Mut::set_if_neq`) rather than a cursor field, because THIS write's
/// change tick is the repaint signal: it is what `ui_render_discovery`'s
/// `Or<(Changed<…>, …)>` filter sees. A dense component in that filter would be
/// invisible to it — MEASURED, `docs/UI-PLAN-SPRITES.md` S-D16 (1) — so the
/// per-frame write has to land on a TABLE column, and this is it.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiSpriteSheet {
    /// The sheet's dense handle — a [`SheetId`](crate::sprite::SheetId) index
    /// into the [`UiSheetTable`](crate::sprite::UiSheetTable) resource, never a
    /// string or a map key (the [`FontId`](crate::text::FontId) discipline).
    ///
    /// An id no sheet was registered for leaves the sheet INERT: the node draws
    /// its `UiImage` exactly as it would without this component. An absent
    /// table does the same — the gather reads it through the non-panicking
    /// resource verb, because eight in-tree harnesses build UI worlds by hand
    /// and never insert one.
    pub sheet: u16,
    /// Which frame, in ROW-MAJOR order (`col = index % cols`,
    /// `row = index / cols`).
    ///
    /// An `index` at or above the sheet's
    /// [`frame_count`](crate::sprite::UiSheet::frame_count) is CLAMPED to the
    /// last frame and counted in the render gather's `sheet_index_clamps`
    /// diagnostic, rather than panicking or sampling a trailing cell that
    /// holds nothing.
    pub index: u16,
}

const _: () = assert!(size_of::<UiSpriteSheet>() == 4);
const _: () = assert!(align_of::<UiSpriteSheet>() == 2);

/// How [`UiSpriteAnim`] walks its frame range (UI-ADVANCED S5).
///
/// A TYPED enum on the authored component rather than a raw `u8`: S-D13 (4)(3)
/// ruled that the authored component keeps the type system's guarantee and only
/// a byte that CROSSES a crate boundary is `debug_assert!`ed against a count.
/// This one never crosses — the flipbook and the component both live in
/// `boyko_ui`, and the render pack never sees it — so no count const and no
/// conversion site are minted for it.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SpriteAnimMode {
    /// `first → last`, then wrap to `first`. One cycle = one pass.
    #[default]
    Forward = 0,
    /// `last → first`, then wrap to `last`. One cycle = one pass.
    Reverse = 1,
    /// `first → last → first`. One cycle = one ROUND TRIP; the two endpoints
    /// are each shown once per cycle, not twice (the turn does not repeat the
    /// end frame).
    PingPong = 2,
    /// Exactly [`Forward`](SpriteAnimMode::Forward) with
    /// [`repeats`](UiSpriteAnim::repeats)` == 1`: one pass, then HOLD `last`
    /// forever. It is spelled as its own variant because that is how authors
    /// reach for it, and the equivalence is stated so the two knobs cannot
    /// disagree.
    Once = 3,
}

/// A flipbook animation over a [`UiSpriteSheet`]'s frames (UI-ADVANCED S5).
/// AUTHOR-OWNED, OPT-IN, TABLE storage, **COLD** — author-written, never
/// system-written. `#[repr(C)]`, POD, 12 B.
///
/// # The churn split, which is the whole point of two components
///
/// This component is the animation's CONFIGURATION and nothing writes it per
/// frame, so `Changed<UiSpriteAnim>` means "an author retargeted the
/// animation" and never "a frame ticked". The per-frame state lives in
/// [`UiSpriteCursor`] (dense, flipbook-private) and the per-frame RESULT in
/// [`UiSpriteSheet::index`] (table, the repaint signal).
///
/// # The cursor arrives on its own — an `on_add` HOOK, not `#[require]`
///
/// The flipbook queries all three components, so an authored `UiSpriteAnim` with
/// no [`UiSpriteCursor`] would silently never tick. `#[require(UiSpriteCursor)]`
/// is the obvious remedy and it does not work: **MEASURED on this kernel, a
/// `#[require]` whose target is a DENSE component panics at insert** — the
/// require pass resolves the required id's `ComponentPool` in the target
/// ARCHETYPE (`migration_helpers.rs`: *"invariant: target hosts every required id
/// (expanded archetype)"*), and a dense id is excluded from every archetype
/// signature and owns no per-archetype pool by construction (dense plan D0). The
/// panic names an expansion that never happened, so the message does not say what
/// is wrong. Filed as a kernel defect in `docs/OPEN-QUESTIONS.md`.
///
/// **UI-ADVANCED S6 closes the hazard at this component anyway**, through the
/// route the require pass could not take:
/// `sprite::ui_sprite_anim_on_add` deferred-inserts
/// the cursor at its `Default` through a one-field `Bundle` wrapper, and
/// `InsertCommand` already PARTITIONS dense ids off the table path. One landing,
/// at the component, inherited by every construction site — the `.ui` dispatch,
/// the reload reconcile, `ui!`, a hand-spawn, and
/// [`AnimatedSpriteBundle`](crate::bundles::AnimatedSpriteBundle). See that hook's
/// doc for why it is `on_add` rather than `on_insert`, why the insert being
/// DEFERRED matters, and why there is deliberately no symmetric `on_remove`.
///
/// [`AnimatedSpriteBundle`](crate::bundles::AnimatedSpriteBundle) remains the
/// ergonomic one-spawn form (the layout base, the image, the sheet, the animation
/// and the cursor together); it is no longer the only thing standing between an
/// author and a frozen sprite. Gate G5-12 pins both halves — the bundle ticks,
/// and so do the components spawned one at a time.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(on_add = crate::sprite::ui_sprite_anim_on_add)]
pub struct UiSpriteAnim {
    /// First frame of the range, INCLUSIVE (a [`UiSpriteSheet::index`]).
    pub first: u16,
    /// Last frame of the range, INCLUSIVE. `last < first` is a degenerate
    /// range: the flipbook holds `first` and never advances.
    pub last: u16,
    /// Frames per second. `<= 0` or non-finite ⇒ the flipbook never advances
    /// (a paused animation an author can express without removing a
    /// component).
    pub fps: f32,
    /// How the range is walked.
    pub mode: SpriteAnimMode,
    /// How many CYCLES to run before holding, or `0` for INFINITE.
    ///
    /// The two knobs are defined against each other rather than left to
    /// collide: [`SpriteAnimMode::Once`] is exactly `Forward` with
    /// `repeats == 1`, and every mode holds the frame its LAST cycle ended on —
    /// `Forward` holds [`last`](Self::last), `Reverse` and `PingPong` hold
    /// [`first`](Self::first). Cycles completed are counted in
    /// [`UiSpriteCursor::loops_done`], which saturates at `u8::MAX` so an
    /// infinite animation cannot wrap the counter back under a budget.
    pub repeats: u8,
    /// Explicit tail padding to 12 B. SPELLED rather than implicit, the
    /// [`UiNineSlice::_pad`] rule: MEASURED on rustc 1.97.1 the field list
    /// without it is *also* 12 B / align 4, i.e. the two bytes are implicit
    /// tail padding — which is precisely what "the padding is spelled" forbids
    /// leaving unwritten in a `#[repr(C)]` POD.
    pub _pad: [u8; 2],
}

const _: () = assert!(size_of::<UiSpriteAnim>() == 12);
const _: () = assert!(align_of::<UiSpriteAnim>() == 4);

impl Default for UiSpriteAnim {
    /// Frames `0..=0` at 12 fps, `Forward`, infinite — a one-frame animation
    /// that moves nothing until an author sets a range. The zero-configuration
    /// value is the null one, `UiImage`'s alpha-0 default tint one component
    /// over.
    #[inline]
    fn default() -> Self {
        UiSpriteAnim {
            first: 0,
            last: 0,
            fps: 12.0,
            mode: SpriteAnimMode::Forward,
            repeats: 0,
            _pad: [0; 2],
        }
    }
}

/// [`ui_sprite_flipbook`](crate::sprite::ui_sprite_flipbook)'s PRIVATE per-frame
/// state (UI-ADVANCED S5). **DENSE** storage; read by no other system and by no
/// pack input. `#[repr(C)]`, POD, 8 B.
///
/// # Why dense, and why NOT in the pack-input list
///
/// Dense because it is written every frame on every animated node and read by
/// exactly one system: a table column would migrate the archetype on
/// insert/remove and would put per-frame churn in the same store as the cold
/// authored data. And it is deliberately absent from `ui_pack_inputs!` — the
/// gather probes every listed component on every visited node, so listing it
/// would charge a dead probe to every node of every changed frame, and a dense
/// term inside the discovery filter's `Or<..>` was MEASURED never to fire
/// (S-D16 (1)).
///
/// There is no `frame` field: the frame index lives in
/// [`UiSpriteSheet::index`], where the pack reads it. A cursor-local copy would
/// be written by the flipbook and read by nobody.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
pub struct UiSpriteCursor {
    /// Seconds accumulated toward the next frame step; always less than one
    /// frame's duration after a tick.
    pub elapsed: f32,
    /// [`SpriteAnimMode::PingPong`]'s current direction: `+1` forward, `-1`
    /// backward. Ignored by the other three modes.
    pub dir: i8,
    /// Completed cycles, saturating at `u8::MAX` — what makes
    /// [`UiSpriteAnim::repeats`] and [`SpriteAnimMode::Once`] expressible at
    /// all. Nothing in `{elapsed, dir}` counts cycles, so without this field
    /// both were unimplementable.
    pub loops_done: u8,
    /// Explicit tail padding to 8 B (the [`UiSpriteAnim::_pad`] rule).
    pub _pad: [u8; 2],
}

const _: () = assert!(size_of::<UiSpriteCursor>() == 8);
const _: () = assert!(align_of::<UiSpriteCursor>() == 4);

impl Default for UiSpriteCursor {
    /// Zero elapsed, direction FORWARD, zero completed cycles.
    ///
    /// `dir: 1`, not `0`, and that is why this is written rather than derived:
    /// every path that materializes the cursor materializes it through
    /// `Default`, and a derived `dir: 0` would make every `PingPong` animation
    /// stand still on frame `first` with nothing to say so.
    ///
    /// The materializing path is `UiSpriteAnim`'s
    /// `on_add` hook `sprite::ui_sprite_anim_on_add` (UI-ADVANCED S6,
    /// `docs/UI-PLAN-SPRITES.md` S-D20), with
    /// [`AnimatedSpriteBundle`](crate::bundles::AnimatedSpriteBundle) as the
    /// ergonomic one-spawn form. It is NOT `#[require]`: that attribute panics on
    /// a dense target on this kernel (see [`UiSpriteAnim`]'s doc and
    /// `docs/OPEN-QUESTIONS.md`), and the spelling this comment previously
    /// showed — `#[require(A => B)]` — has never existed; the derive parses
    /// `#[require(B)]`, `#[require(C = expr)]` and `#[require(D(args))]` only
    /// (`boyko_macros/src/component.rs:901-909`).
    #[inline]
    fn default() -> Self {
        UiSpriteCursor {
            elapsed: 0.0,
            dir: 1,
            loops_done: 0,
            _pad: [0; 2],
        }
    }
}

/// Uniform grid track config (GUI P6a). Pairs with
/// [`UiLayout`]`{ layout_type: Grid }`. AUTHOR-OWNED, OPT-IN.
///
/// The layout solver places relative child at flow index `i` into cell
/// `(col = i % columns, row = i / columns)` and sizes each child to the uniform
/// cell extent (the container content box divided by the track counts). A
/// bounded, `O(children)` placement — no super-linear scan (the complexity guard
/// holds). `columns == 0` is coerced to `1`; `rows == 0` derives the row count
/// from the child count.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct UiGrid {
    /// Track count on the cross axis (number of columns). `0` ⇒ treated as `1`.
    pub columns: u8,
    /// Track count on the main axis (number of rows). `0` ⇒
    /// `ceil(child_count / columns)`.
    pub rows: u8,
}

/// The 9 screen-anchor positions for a [`UiAnchor`] (the corners, the edge
/// centers, and the center). `#[repr(u8)]`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnchorEdge {
    /// Pin the node's top-left to the screen top-left.
    #[default]
    TopLeft,
    /// Pin the node's top edge, centered horizontally.
    TopCenter,
    /// Pin the node's top-right to the screen top-right.
    TopRight,
    /// Pin the node's left edge, centered vertically.
    CenterLeft,
    /// Pin the node centered on both axes.
    Center,
    /// Pin the node's right edge, centered vertically.
    CenterRight,
    /// Pin the node's bottom-left to the screen bottom-left.
    BottomLeft,
    /// Pin the node's bottom edge, centered horizontally.
    BottomCenter,
    /// Pin the node's bottom-right to the screen bottom-right.
    BottomRight,
}

/// Screen-edge anchor for a [`UiRoot`] (GUI P6a). COLD, OPT-IN, AUTHOR-OWNED.
///
/// Pins a root's resolved rectangle to a screen edge/corner with an inset
/// `offset` and (optionally) the [`UiSafeArea`](crate::resources::UiSafeArea)
/// inset. Resolved INSIDE `ui_layout_apply` (after the root is measured, before
/// its rect is written) so the layout pass stays the SINGLE `ComputedRect`
/// writer (no pre-pass write race). A node WITHOUT this component lays out at the
/// viewport top-left as before.
///
/// P6a scopes anchoring to ROOTS only; a `UiAnchor` on a non-root in-tree node is
/// ignored by the layout pass (a non-root is positioned by its parent's box, a
/// different code path — Open Question 3). The system NEVER writes this component
/// (author-only, like [`StackIndex`] / [`UiAbsolute`]).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiAnchor {
    /// Which corner/edge to pin to.
    pub edge: AnchorEdge,
    /// Inset toward the screen interior from the pinned edge, x (logical px).
    pub offset_x: f32,
    /// Inset toward the screen interior from the pinned edge, y (logical px).
    pub offset_y: f32,
    /// Subtract the [`UiSafeArea`](crate::resources::UiSafeArea) inset when true.
    pub use_safe_area: bool,
    /// Explicit tail pad → const-asserted size.
    pub _pad: [u8; 3],
}

const _: () = assert!(size_of::<UiAnchor>() == 16);
const _: () = assert!(align_of::<UiAnchor>() == 4);

impl Default for UiAnchor {
    /// Top-left, zero offset, no safe-area — equivalent to no anchor.
    #[inline]
    fn default() -> Self {
        UiAnchor {
            edge: AnchorEdge::TopLeft,
            offset_x: 0.0,
            offset_y: 0.0,
            use_safe_area: false,
            _pad: [0; 3],
        }
    }
}

/// Stable per-sibling positional key for the `.ui` hot-reload reconcile of
/// ANONYMOUS (unnamed) nodes (P3 Decision 11).
///
/// `Children` sibling order is explicitly unspecified and order-perturbing on
/// removal (`Vec::swap_remove`), so an anonymous node can NOT be matched against
/// the live `Children` slice by position. Instead every node (named and
/// anonymous) is stamped at spawn with its **declaration ordinal among its
/// siblings** in the parse tree (0-based, per-parent); the reconcile matches an
/// anonymous node to the live child carrying the same `UiSourceOrder`. Named
/// nodes match by the stronger [`UiName`] key; this ordinal is the fallback.
///
/// Private to the crate: it is a P3-reload-only bookkeeping key the author never
/// writes and the `ui!` macro never stamps, so it is excluded from the
/// `.ui`-vs-`ui!` equivalence comparison (Decision 12).
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UiSourceOrder(pub u32);

// ─────────────────────────────────────────────────────────────────────────────
// UI-ADVANCED rung A1 — the animation sink and the four tween channels
// (`docs/UI-PLAN-ANIMATION.md` A1, AD3, AD6, AD10, AD11, AD12, AM5, AM8).
// ─────────────────────────────────────────────────────────────────────────────

/// An easing curve identifier (AD2): a `u8` with a reserved custom half.
///
/// `0..=29` are the built-in `family * 3 + direction` ids (RmlUi's ten families
/// × in/out/in-out); `30..=127` are reserved; `128..=255` are custom curves,
/// whose LUT index is `id - 128`. The custom test is therefore `id & 0x80` — one
/// branchless bit, never a comparison against a mutable table length.
///
/// # A1 lands the TYPE, not the table
///
/// Rung A1 is **linear only**: the field exists, every tween carries one, and
/// [`ui_visual_tick`](crate::animation::ui_visual_tick) applies the raw
/// normalized `t`. Rung A2 lands the thirty built-in bodies and the const-assert
/// that holds the custom boundary. The split is deliberate — A1's gates test the
/// machinery and A2's gates test the curves, so a red at A2 cannot be blamed on
/// A1.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct EasingId(u8);

impl EasingId {
    /// `linear` (family 0, direction 0). The only curve rung A1 evaluates.
    pub const LINEAR: EasingId = EasingId(0);

    /// The bit that separates a built-in id from a custom LUT index (AD2).
    pub const CUSTOM_BIT: u8 = 0x80;

    /// Wraps a raw id. No validation: `128..=255` is the custom half by
    /// construction and `30..=127` is reserved, so there is no invalid `u8`.
    #[inline]
    pub const fn from_raw(raw: u8) -> Self {
        EasingId(raw)
    }

    /// The raw id.
    #[inline]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Whether this id addresses the custom curve table (`id & 0x80`).
    #[inline]
    pub const fn is_custom(self) -> bool {
        self.0 & Self::CUSTOM_BIT != 0
    }
}

/// Bit 0 of a tween row's `flags`: advance this row on the UI clock's VIRTUAL
/// delta instead of its real one (D15's per-row opt-in, AD1 reason (4),
/// AD9 (1)).
///
/// The tween lane is the ONE lane that carries this bit, and it is what makes
/// [`UiClock::dt_real`](crate::animation::UiClock::dt_real) reachable at all: a
/// consumer WITHOUT the bit reads `dt_virtual`, full stop (AM7 / AD9). Set it on
/// a tween that should pause with the game and slow in slow-motion; leave it
/// clear on a tween whose whole purpose is to run while the game is paused — the
/// pause-menu fade D15 exists for.
pub const TWEEN_FLAG_VIRTUAL_CLOCK: u8 = 1 << 0;

/// The animation SINK: one node's composed visual transform, read by the pack
/// fold (A4) and the hit-test fold (A6).
///
/// # It is a TABLE component, and that is the rung's headline decision (AD10)
///
/// A dense sink is written correctly, read correctly — and is INVISIBLE to
/// `ui_render_discovery`'s `Or<(Changed<C1>, …)>`, because the kernel's `Or`
/// overrides none of the dense hooks (`HAS_DENSE` takes the trait default
/// `false` and the inner term's fetch stays null). MEASURED: the same
/// `Mut::set_if_neq` write is seen by a bare `Changed` (1 row) and not by the
/// discovery filter's `Or` (0 rows) for a dense sink; a table sink is seen by
/// both. The failure mode is a frozen picture with no panic, no error and no
/// failing assertion — so the storage kind is const-asserted below rather than
/// merely documented.
///
/// The four `Tween*` channels stay DENSE for the opposite reason: nothing
/// filters them, `AnyOf` DOES forward `HAS_DENSE`/`resolve_dense` to its arms,
/// and they churn once per animation. The sink never churns — it is inserted
/// once per element that ever animates and is never removed (its last value IS
/// the resting appearance), so a table sink costs exactly one archetype
/// migration per animated element, ever. This is the shipped
/// [`UiSpriteSheet`] : [`UiSpriteCursor`] split, one rung later.
///
/// # Its `Default` is the IDENTITY and its `PartialEq` is BYTEWISE
///
/// Neither is derived, and each absence is a decision — see [`UiVisual::IDENTITY`]
/// (AD6) and the hand-written [`PartialEq`] impl (AD11).
///
/// # Layout (AM5)
///
/// 24 B, `#[repr(C)]`, POD `Copy`. D9's `uv_shift: [f32; 2]` is deliberately
/// ABSENT: no v1 channel writes it, and removing it takes this plan's shader
/// exposure to exactly zero.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct UiVisual {
    /// Straight (non-premultiplied) RGBA8 tint, multiplied component-wise into
    /// the node's colour before the pack's premultiply (AD3).
    pub tint_mul: u32,
    /// Scalar opacity, multiplied down the inheritance stack (AD3/AD4) and
    /// folded into the existing premultiply.
    pub opacity: f32,
    /// Origin-relative translation in logical pixels (AD3's `o`).
    pub offset_px: [f32; 2],
    /// Origin-relative scale about the node's rect centre (AD3's `s`).
    pub scale: [f32; 2],
}

const _: () = assert!(size_of::<UiVisual>() == 24);
const _: () = assert!(align_of::<UiVisual>() == 4);
const _: () = assert!(core::mem::offset_of!(UiVisual, tint_mul) == 0);
const _: () = assert!(core::mem::offset_of!(UiVisual, opacity) == 4);
const _: () = assert!(core::mem::offset_of!(UiVisual, offset_px) == 8);
const _: () = assert!(core::mem::offset_of!(UiVisual, scale) == 16);

// A1 gate 11 (AD10 / AD13), the `occlusion_marker.rs` idiom verbatim: the
// storage kind is a BUILD ERROR when it is wrong, not a comment. `dense` is the
// non-signature backend — it drops the id from every archetype signature, leaves
// it without a per-archetype `ComponentPool`, and (this is the one that matters
// here) is invisible to every `Changed<C>` nested inside an `Or<..>`.
const _: () = assert!(
    !<UiVisual as ::boyko_ecs::ecs::core::component::component::Component>::STORAGE_IS_DENSE,
    "UiVisual MUST be a table component (AD10): a dense Changed<C> inside Or<..> was MEASURED \
     never to fire on this kernel, and UiVisual is a term of ui_render_discovery's Or from rung \
     A4 onwards — a dense sink renders a frozen picture with nothing saying so"
);

impl UiVisual {
    /// The identity visual: no tint, fully opaque, no offset, unit scale.
    ///
    /// The SECOND route into the default value (AD6). `Default` returns this
    /// const, and gate 4 compares the two against literals written into the
    /// test — two spellings that neither derives from the other, the
    /// `default_mode_is_off` precedent. A node with NO `UiVisual` row folds by
    /// exactly these bytes, which is A4's disarmed gate.
    pub const IDENTITY: UiVisual = UiVisual {
        tint_mul: 0xFFFF_FFFF,
        opacity: 1.0,
        offset_px: [0.0; 2],
        scale: [1.0; 2],
    };
}

impl Default for UiVisual {
    /// [`UiVisual::IDENTITY`] — hand-written, never derived (AD6).
    ///
    /// A derived `Default` gives `tint = 0` (transparent black), `opacity = 0`
    /// and `scale = [0, 0]`: an element that inserts a `UiVisual` and animates
    /// nothing becomes an invisible zero-sized node. That is a two-line decision
    /// which costs an afternoon when it is discovered from a screenshot instead.
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl PartialEq for UiVisual {
    /// BITWISE, not `f32`'s `PartialEq` (AD11).
    ///
    /// `Mut::set_if_neq` is the render gate's only throttle, and the derived
    /// float equality has an exception nothing states: `NaN != NaN`, so ONE NaN
    /// anywhere in the sink makes `set_if_neq(THE SAME BYTES)` write and bump on
    /// EVERY frame. One such row bumps `UiRenderGeneration` — a single global
    /// counter — so the per-slot upload skip is disarmed for the WHOLE UI,
    /// permanently. MEASURED on a plateau tween (`from == to`, which an author
    /// writes whenever a transition targets the state it is already in) over a
    /// sink carrying one NaN: `[0, 0, 0]` bytewise vs `[1, 1, 1]` derived.
    ///
    /// A NaN is reachable in RELEASE without any kernel bug: the `debug_assert!`s
    /// at the authoring sites all compile out and the public start helpers take
    /// author `from`/`to` values with no release-side filter.
    ///
    /// The `± 0.0` trade is on the record: bytewise equality calls `+0.0` and
    /// `−0.0` different, so a channel landing on `−0.0` where `+0.0` stood costs
    /// ONE extra bump — one frame, not a state. The derived form's NaN case costs
    /// every frame, forever. This is the direction the trade must run.
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        self.tint_mul == o.tint_mul
            && self.opacity.to_bits() == o.opacity.to_bits()
            && self.offset_px[0].to_bits() == o.offset_px[0].to_bits()
            && self.offset_px[1].to_bits() == o.offset_px[1].to_bits()
            && self.scale[0].to_bits() == o.scale[0].to_bits()
            && self.scale[1].to_bits() == o.scale[1].to_bits()
    }
}

/// Generates one `Tween*` channel column: the struct, its layout pins, its
/// dense-storage const-assert, and its `#[derive(Bundle)]` wrapper.
///
/// Four channels with identical bookkeeping (`elapsed`, `inv_duration`,
/// `easing`, `flags`) and one differing payload type. The macro is here so the
/// four cannot drift: a field added to the bookkeeping is added to all four, and
/// the storage-kind assert is emitted per channel rather than remembered per
/// channel.
macro_rules! tween_channel {
    (
        $(#[$meta:meta])*
        $name:ident, $payload:ty, $size:expr, $bundle:ident, $bundle_field:ident
    ) => {
        $(#[$meta])*
        ///
        /// # Storage: DENSE, and safe here for one stated reason (AD10)
        ///
        /// **Nothing filters a `Tween*`.** No channel is a member of
        /// `ui_pack_inputs!`, nothing reads `Changed<Tween*>`, and no channel
        /// appears in any `Or<..>` in this plan — so the kernel's `Or` blindness
        /// to dense terms (AM8) cannot reach them. `AnyOf`, which the fused tick
        /// DOES use, forwards `HAS_DENSE` and `resolve_dense` to every arm, so a
        /// dense arm resolves its store and yields `Some`/`None` per row
        /// correctly. Dense also keeps the per-animation insert/reap churn out of
        /// every archetype signature, which is exactly what
        /// [`UiSpriteCursor`]'s own dense decision was for.
        ///
        /// # `#[require]` may NOT point at this type
        ///
        /// MEASURED on this kernel: the require pass resolves the required id's
        /// `ComponentPool` in the target ARCHETYPE, and a dense id owns none, so
        /// `#[require(<a dense type>)]` PANICS at insert. The sink arrives by the
        /// `on_add` hook below instead — the `UiSpriteAnim` → `UiSpriteCursor`
        /// route, in the opposite direction.
        #[repr(C)]
        #[derive(Component, Clone, Copy, Debug, PartialEq)]
        #[component(storage = "dense", on_add = crate::animation::ui_visual_sink_on_add)]
        pub struct $name {
            /// The value at `elapsed == 0`.
            pub from: $payload,
            /// The value at `elapsed >= duration`, ASSIGNED exactly at the
            /// endpoint (never `to ± ULP` — the endpoint is assigned, not
            /// interpolated).
            pub to: $payload,
            /// Seconds since the tween started.
            pub elapsed: f32,
            /// `1.0 / duration_secs`. Stored reciprocal so the per-row tick is a
            /// multiply rather than a divide; the zero-duration trap it creates
            /// is caught by a `debug_assert!` at the authoring site, where the
            /// mistake was made.
            pub inv_duration: f32,
            /// Which curve (AD2). Rung A1 evaluates LINEAR only.
            pub easing: EasingId,
            /// Per-row flags. Bit 0 is [`TWEEN_FLAG_VIRTUAL_CLOCK`].
            pub flags: u8,
            /// Explicit tail padding (the [`UiNineSlice::_pad`] rule).
            pub _pad: [u8; 2],
        }

        const _: () = assert!(size_of::<$name>() == $size);
        const _: () = assert!(align_of::<$name>() == 4);
        const _: () = assert!(core::mem::offset_of!($name, from) == 0);

        // A1 gate 11 (AD10): the channels are the half of the split that MUST be
        // dense. Plain storage here would put four ids into every animated
        // node's archetype signature and pay four migrations per animation start
        // and four more per completion, on the pass that starts every hover.
        const _: () = assert!(
            <$name as ::boyko_ecs::ecs::core::component::component::Component>::STORAGE_IS_DENSE,
            "the Tween* channels MUST be dense (AD10): they are inserted and reaped per \
             animation, nothing filters them, and AnyOf — unlike Or — forwards the dense hooks"
        );

        /// A one-field `#[derive(Bundle)]` wrapper, so an insert verb can take
        /// the channel at all.
        ///
        /// Not a style choice, and MEASURED one file over: dense storage
        /// SUPPRESSES the single-component `Bundle` impl the derive normally
        /// emits (`boyko_macros`' `component.rs` gates `bundle_items` on
        /// `no_bundle || storage_bitset || storage_dense`), so
        /// `insert(TweenTint { .. })` is `error[E0277]: the trait bound
        /// TweenTint: Bundle is not satisfied`. A wrapper bundle is the only
        /// spelling that compiles — the same
        /// [`SpriteCursorBundle`](crate::sprite) idiom.
        #[derive(Bundle, Clone, Copy, Debug)]
        pub struct $bundle {
            /// The channel row.
            pub $bundle_field: $name,
        }
    };
}

tween_channel!(
    /// Tween channel: the node's [`UiVisual::tint_mul`], interpolated
    /// component-wise in STRAIGHT RGBA8 (AD3).
    TweenTint,
    u32,
    20,
    TweenTintBundle,
    tween
);

tween_channel!(
    /// Tween channel: the node's [`UiVisual::opacity`].
    TweenOpacity,
    f32,
    20,
    TweenOpacityBundle,
    tween
);

tween_channel!(
    /// Tween channel: the node's [`UiVisual::offset_px`] (AD3's `o`).
    TweenOffset,
    [f32; 2],
    28,
    TweenOffsetBundle,
    tween
);

tween_channel!(
    /// Tween channel: the node's [`UiVisual::scale`] (AD3's `s`).
    TweenScale,
    [f32; 2],
    28,
    TweenScaleBundle,
    tween
);
