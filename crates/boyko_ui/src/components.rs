//! ECS components — the layout inputs and computed outputs.
//!
//! Every component is its own SoA column inside the archetype; change detection
//! is per-component-per-row, so the inputs are split by churn profile (a node
//! animating only its size bumps only [`UiLayout`]'s tick). All are POD `Copy`
//! (`Send + Sync`), so they are trivially safe to read on the layout pass.
//!
//! The `boyko` `Component` derive is a pure marker: it adds no fields and only
//! assigns a lazily-allocated `ComponentId`, so it coexists with `#[repr(C)]`.

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
    pub const fn new(s: &str) -> Self {
        debug_assert!(s.len() <= Self::CAP, "invariant: ui name exceeds CAP");
        let src = s.as_bytes();
        let mut bytes = [0u8; Self::CAP];
        // `const fn` cannot use `copy_from_slice`/iterators with `?`; a manual
        // index loop is the const-compatible byte copy. `n` is clamped to CAP so
        // an over-length name (which the macro already rejects) cannot overrun.
        let n = if src.len() < Self::CAP { src.len() } else { Self::CAP };
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
        // SAFETY: `new` only ever writes valid UTF-8 bytes copied verbatim from a
        // `&str` into `bytes[..len]`, and `len <= CAP` (debug-asserted above and
        // clamped in `new`). The slice `bytes[..len]` is therefore the exact
        // valid-UTF-8 prefix that was stored, so `from_utf8_unchecked` is sound.
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
