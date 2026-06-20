//! Layout scalar/enum primitives (the 4-unit flexbox model).
//!
//! All types here are POD `Copy` with explicit `repr` so their byte layout is
//! stable and the layout pass reads them branch-light. No viewport units in P1
//! (`Vw`/`Vh`/`VMin`/`VMax` are a purely additive future extension).

/// A length on one axis. 4-unit flexbox model (no viewport units in P1).
///
/// `Copy` + `repr(C)` (a tag byte + an `f32` payload, 8 B). Hot: read for every
/// sized node on every measurement pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    /// Logical pixels.
    Px(f32),
    /// Percentage of the parent's resolved DEFINITE axis extent (content box).
    ///
    /// `%` of an indefinite (Auto) parent axis resolves to Auto/content per CSS,
    /// NOT zero-of-final. Not clamped to `0..=100`.
    Pct(f32),
    /// Flex-grow factor. Consumes free main-axis space proportional to factor;
    /// also valid on gaps (parent-applied stretch spacing).
    Stretch(f32),
    /// Intrinsic: hug content (container = fold of children; leaf = `ContentSize`).
    Auto,
}

impl Default for Unit {
    #[inline]
    fn default() -> Self {
        Unit::Auto
    }
}

/// Container layout direction.
///
/// `Grid` is RESERVED — P1 falls back to [`LayoutType::Column`] with a `#[cold]`
/// `debug_assert!` (the variant is reserved so the public API is stable when the
/// grid sub-phase lands).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayoutType {
    /// main = x (width), cross = y (height).
    Row,
    /// main = y (height), cross = x (width).
    #[default]
    Column,
    /// Children share the container box; each positioned by its align only.
    Overlay,
    /// RESERVED — P1 falls back to `Column`.
    Grid,
}

/// Whether a node participates in its parent's flow (`Relative`) or is taken out
/// of flow and positioned against the parent's padding box (`Absolute`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PositionType {
    /// In-flow: consumes main-axis space, placed by the container's algorithm.
    #[default]
    Relative,
    /// Out-of-flow: consumes no flow space, placed by [`crate::components::UiAbsolute`].
    Absolute,
}

/// Main-axis distribution of leftover free space.
///
/// Only applies when no `Stretch` consumes the free space (see the layout
/// module's AlignMain precedence) and `free_main > 0`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignMain {
    /// Pack at the before-edge.
    #[default]
    Start,
    /// Pack centered.
    Center,
    /// Pack at the after-edge.
    End,
    /// First child at the before-edge, last at the after-edge, gaps even between.
    SpaceBetween,
    /// Equal space around each child (half-gap at the edges).
    SpaceAround,
    /// Equal space between children and edges.
    SpaceEvenly,
}

/// Cross-axis placement of each child within the container's cross extent.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignCross {
    /// Place at the cross before-edge.
    #[default]
    Start,
    /// Center on the cross axis.
    Center,
    /// Place at the cross after-edge.
    End,
    /// Fill the container's cross content extent (clamped to the child's cross
    /// min/max).
    Stretch,
}
