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

use boyko_macros::Component;

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
/// At S4 there is exactly ONE legal value, and that is pinned mechanically by
/// the one-variant `const` match below rather than asserted in prose: S5's
/// `Tile` cannot arrive without turning that match into
/// `error[E0004]: non-exhaustive patterns`, which walks the author to every
/// site that assumes a single mode. (`std::mem::variant_count` would be the
/// obvious spelling; MEASURED on rustc 1.97.1 it is `E0658` *and* "not yet
/// stable as a const fn" — two errors on one line — so it does not exist on
/// this toolchain.)
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NineSliceMode {
    /// The edges and the centre are STRETCHED to fill their destination
    /// region. The only mode at S4; `Tile` arrives with S5's sub-rect `frac`
    /// mechanism (`docs/UI-PLAN-SPRITES.md` S-D11).
    #[default]
    Stretch = 0,
}

// S4's "exactly one legal value", made mechanical. NO outer braces: MEASURED —
// `const _: () = { match … };` emits `unused_braces`, which the project's
// `clippy --all-targets -- -D warnings` gate turns into an error.
const _: () = match NineSliceMode::Stretch {
    NineSliceMode::Stretch => (),
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
    /// How the edges and the centre fill their destination. One legal value at
    /// S4 ([`NineSliceMode::Stretch`]).
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
