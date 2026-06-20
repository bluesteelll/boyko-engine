//! Canonical node bundles.
//!
//! [`UiNodeBundle`] is the always-present node base. Bundling exactly the two
//! components every laid-out node carries lets the common node hit the Phase-8.5
//! static archetype cache as a single 2-component unit in one spawn — the
//! `ui!` macro spawns this base whenever a node's component set contains both
//! `UiLayout` and `ComputedRect`.

use boyko_macros::Bundle;

use crate::components::{ComputedRect, UiLayout};

/// The always-present node base: `UiLayout` (primary layout input) + the
/// `ComputedRect` output every laid-out node carries.
///
/// Hits the Phase-8.5 static archetype cache as a single 2-component unit (one
/// per-world `OnceLock<ArchetypeId>` slot). The `ui!` macro spawns this base
/// when a node's component set contains BOTH `UiLayout` and `ComputedRect`
/// (set-based recognition); otherwise it spawns `UiLayout` and injects
/// `ComputedRect::default()`.
///
/// NOTE: this is the node BASE, not its final archetype. The hierarchy hooks
/// migrate archetypes on linking — a child gains `ChildOf`, a parent gains
/// `Children` — so the final archetype is `UiNodeBundle (+ opts) (+ ChildOf)
/// (+ Children)`.
#[derive(Bundle)]
pub struct UiNodeBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle (the renderer's only geometry input).
    pub rect: ComputedRect,
}
