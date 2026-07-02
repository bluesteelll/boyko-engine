//! `ui_focus_system` — the exclusive pointer hit-test + interaction writer (GUI
//! P4 focus algorithm).
//!
//! Exclusive (`&mut EcsMaster`) because the hit-test reads a node's ancestors'
//! `ComputedClip`/`FocusPolicy` while writing *other* nodes' `Interaction` — not
//! a conflict-free parallel `Query` (mirrors `ui_layout_apply`).
//!
//! Per-frame flow (Decisions 1/2/11/12/13):
//! 1. Blur/leave short-circuit: `!cursor_inside || !window_focused` → reset all
//!    interaction, cancel pending clicks, clear focus; return.
//! 2. Resolve the cursor to LOGICAL px via `UiViewport.scale_factor` (one narrow).
//! 3. Total-Z-order hover resolution (StackIndex desc, paint order desc, Entity),
//!    `Block` stops who-becomes-hovered.
//! 4. UNCONDITIONAL reset+write pass over EVERY interactive node (so a node
//!    occluded by a `Block` node this frame is still reset to `None`).
//! 5. `RelativeCursorPosition` set-if-changed; stamp-at-press click resolution;
//!    same-frame press+release deferral.
//! 6. Keyboard focus + total cross-root tab order.

use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_input::{KeyCode, MouseButton, PhysicalInput};
use boyko_macros::Resource;

use crate::components::{ComputedClip, ComputedRect, StackIndex, UiRoot};
use crate::interaction::action::{OnClick, OnSubmit, NO_ACTION};
use crate::interaction::components::{
    Focusable, FocusPolicy, Interaction, RelativeCursorPosition,
};
use crate::resources::UiViewport;

/// Maximum simultaneous pointers (v1 = mouse only). Shaped as a fixed array so
/// multi-touch is a non-breaking later extension (Decision 11).
pub const MAX_POINTERS: usize = 1;

/// Per-pointer interaction bookkeeping (Decision 11). POD `Copy`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerSlot {
    /// The `(origin, resolved_action_index)` stamped at press time; fired at
    /// release when the release lands over the SAME origin entity. `None` when
    /// no press is in flight (Decision 2).
    pub pending_click: Option<(Entity, u16)>,
    /// Set at release-inside-same-node; consumed by `ui_dispatch_system` for
    /// exactly one frame (Decision 2).
    pub click_fired: Option<(Entity, u16)>,
}

/// Single-pointer-aware pointer interaction resource (Decision 11). Fixed POD
/// array; no per-frame allocation.
#[derive(Resource, Clone, Debug)]
pub struct UiPointerState {
    /// Per-pointer slots; v1 uses slot 0 (mouse).
    pub slots: [PointerSlot; MAX_POINTERS],
    /// An `OnSubmit` action stamped by the focus system on an Enter edge while a
    /// node is focused; fired (O(1)) by `ui_dispatch_system` (focus step / submit
    /// edge).
    pub pending_submit: Option<u16>,
}

impl Default for UiPointerState {
    #[inline]
    fn default() -> Self {
        Self {
            slots: [PointerSlot::default(); MAX_POINTERS],
            pending_submit: None,
        }
    }
}

/// The single keyboard-focus resource: which entity holds keyboard focus, plus
/// the cached total-ordered focusable list (focus step 7).
#[derive(Resource, Default)]
pub struct UiInputFocus {
    /// The currently keyboard-focused entity, or `None`.
    pub focused: Option<Entity>,
}

/// The registered EnableTag ids for the interaction bit surface (Decision 1).
/// Cached once at plugin setup; `ui_focus_system` toggles the bits ONLY on a
/// genuine `Interaction` transition (transition-gated).
#[derive(Resource, Clone, Copy, Debug)]
pub struct UiInteractionConfig {
    /// `UiHovered` enable-tag id.
    pub hovered_tag: boyko_ecs::ecs::core::component::component_registry::EnableTagId,
    /// `UiPressed` enable-tag id.
    pub pressed_tag: boyko_ecs::ecs::core::component::component_registry::EnableTagId,
    /// `UiFocused` enable-tag id.
    pub focused_tag: boyko_ecs::ecs::core::component::component_registry::EnableTagId,
}

/// One hit-test candidate, collected in paint order. POD `Copy`.
#[derive(Clone, Copy)]
struct Candidate {
    entity: Entity,
    /// Paint/document sequence (DFS order = paint order). Higher = painted later
    /// = on top.
    paint_seq: u32,
    stack_index: u32,
    rect: ComputedRect,
    /// Optional clip rect (when the node carries `ComputedClip`).
    clip: Option<ComputedClip>,
    block: bool,
}

/// Reused per-frame scratch for the focus pass (a `Resource` — engine storage,
/// allocated once, capacity retained). Frame-transient. Every buffer is
/// `clear()`-then-extend so the steady-state path allocates nothing (Principle
/// 1/5).
#[derive(Resource, Default)]
pub struct UiInteractionScratch {
    /// Cached interactive-node set in paint order for the current pass (filled by
    /// `collect_candidates` into retained capacity).
    candidates: Vec<Candidate>,
    /// DFS stack of `(entity, clip)` for the paint-order walk.
    stack: Vec<(Entity, Option<ComputedClip>)>,
    /// Cached focusable list `(tab_index, paint_seq, Entity)` for the total tab
    /// order.
    focusables: Vec<(u32, u32, Entity)>,
    /// Nodes that transitioned INTO `Hovered` from `None`/`Pressed` this frame
    /// (the None→Hovered hover-enter edge). Stamped by `write_interactions`,
    /// consumed once by `ui_dispatch_system::fire_hover_actions` — the
    /// edge-triggered `OnHover` signal (no per-row tick window in an exclusive
    /// body, so the edge is materialized here at the transition site).
    pub hover_entered: Vec<Entity>,
    /// `query_entities` candidate buffer for the root walk (extend-into-retained-
    /// capacity, never a fresh `Vec`).
    query_buf: Vec<Entity>,
    /// Retained archetype-id scratch backing `query_entities_buf` (alloc-free).
    arch_ids: Vec<boyko_ecs::ecs::identifiers::primitives::ArchetypeId>,
    /// Reused `(entity, rect)` snapshot for `write_interactions` (the loop must
    /// not borrow `candidates` while it mutates the world).
    write_nodes: Vec<(Entity, ComputedRect)>,
}

/// Exclusive pointer hit-test + interaction writer (GUI P4).
//
// `clippy::needless_pass_by_ref_mut`: the body calls `&mut self` engine methods
// (`get_component_mut`, `enable_id`/`disable_id`, `resource_mut`) through
// cross-crate boundaries clippy cannot see through. Mirrors `ui_layout_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_focus_system(world: &mut EcsMaster) {
    let viewport = *world.resource::<UiViewport>();
    let config = *world.resource::<UiInteractionConfig>();

    // PhysicalInput edges (mouse left button + cursor levels).
    let physical = world.resource::<PhysicalInput>().clone();
    let cursor_active = physical.cursor_inside && physical.window_focused;

    // Move the scratch out so the recursion's `&mut world` calls do not conflict
    // with a held resource borrow (the `mem::take` borrow protocol).
    let mut scratch = mem::take(world.resource_mut::<UiInteractionScratch>());

    // Collect the interactive node set in paint order (also used by the reset
    // pass and the tab-order refresh). Done first so blur can reset every node.
    collect_candidates(world, &mut scratch);

    if !cursor_active {
        blur_reset(world, &mut scratch, &config);
        *world.resource_mut::<UiInteractionScratch>() = scratch;
        return;
    }

    // Cursor in LOGICAL px (one narrow after the divide — Decision 13).
    let scale = if viewport.scale_factor > 0.0 {
        viewport.scale_factor
    } else {
        1.0
    };
    debug_assert!(viewport.scale_factor > 0.0, "UiViewport.scale_factor must be > 0");
    let cursor = [
        (physical.cursor_pos[0] / scale as f64) as f32,
        (physical.cursor_pos[1] / scale as f64) as f32,
    ];

    let left = MouseButton::Left.dense_index().map(|i| 1u8 << i).unwrap_or(1);
    let clicked = physical.mouse_just_pressed & left != 0;
    let released = physical.mouse_just_released & left != 0;
    let held = physical.mouse_pressed & left != 0;

    // ── 3a: total-Z-order hover resolution ─────────────────────────────────
    let hovered = resolve_hovered(&scratch, cursor);

    // ── 3b: unconditional reset + write pass over EVERY interactive node ────
    write_interactions(world, &mut scratch, &config, hovered, cursor, clicked, released, held);

    // ── 5/6: click resolution (stamp-at-press / release) + same-frame defer ─
    resolve_pointer(world, hovered, clicked, released, held);

    // ── 7: keyboard focus + total tab order + submit edge ──────────────────
    update_focus(world, &mut scratch, &config, &physical);

    *world.resource_mut::<UiInteractionScratch>() = scratch;
}

/// Refreshes the cached root list and collects every interactive node (one that
/// carries an `Interaction` column) in paint order (DFS over `Children`).
///
/// Fully allocation-free in steady state: the root query, the DFS stack, and the
/// candidate list are all retained scratch buffers cleared-and-refilled here.
fn collect_candidates(world: &mut EcsMaster, scratch: &mut UiInteractionScratch) {
    // Root walk through the retained buffers (no fresh `Vec`).
    world.query_entities_buf(&[UiRoot::component_id()], &mut scratch.query_buf, &mut scratch.arch_ids);

    scratch.candidates.clear();
    scratch.stack.clear();
    let mut paint_seq: u32 = 0;

    // Roots in a deterministic order (Entity id) for a stable cross-root paint
    // sequence; children are pushed in reverse so they pop in document order.
    scratch.query_buf.sort_unstable_by_key(|e| e.id().0);
    for &root in scratch.query_buf.iter().rev() {
        scratch.stack.push((root, None));
    }

    while let Some((node, inherited_clip)) = scratch.stack.pop() {
        // The node's own clip narrows the inherited clip for its subtree.
        let own_clip = world.get_component::<ComputedClip>(node).copied();
        let effective_clip = own_clip.or(inherited_clip);

        if let Some(rect) = world.get_component::<ComputedRect>(node).copied()
            && world.get_component::<Interaction>(node).is_some()
        {
            let stack_index = world
                .get_component::<StackIndex>(node)
                .map(|s| s.0)
                .unwrap_or(0);
            let block = matches!(
                world.get_component::<FocusPolicy>(node).copied(),
                Some(FocusPolicy::Block)
            );
            scratch.candidates.push(Candidate {
                entity: node,
                paint_seq,
                stack_index,
                rect,
                clip: effective_clip,
                block,
            });
            paint_seq += 1;
        }

        // DFS children in reverse so they are visited in document (paint) order.
        // Push directly from the `Children` borrow onto the (disjoint) scratch
        // stack — no `to_vec` copy. `Children` is an immutable column borrow of
        // `world`; `scratch.stack` lives outside `world`, so the two borrows do
        // not conflict.
        if let Some(children) = world.get_component::<Children>(node) {
            for &child in children.as_slice().iter().rev() {
                scratch.stack.push((child, effective_clip));
            }
        }
    }
}

/// Returns whether `key` had a rising edge this frame (event-stream edge).
#[inline]
fn key_just_pressed(physical: &PhysicalInput, key: KeyCode) -> bool {
    match key.dense_index() {
        Some(idx) => physical.keys_just_pressed.get(idx),
        None => false,
    }
}

/// Returns whether `cursor` is inside `rect` (logical px, half-open on the far
/// edge).
#[inline]
fn point_in_rect(cursor: [f32; 2], rect: ComputedRect) -> bool {
    cursor[0] >= rect.x
        && cursor[0] < rect.x + rect.w
        && cursor[1] >= rect.y
        && cursor[1] < rect.y + rect.h
}

/// Returns whether `cursor` is inside the optional clip rect (no clip ⇒ true).
#[inline]
fn point_in_clip(cursor: [f32; 2], clip: Option<ComputedClip>) -> bool {
    match clip {
        Some(c) => {
            cursor[0] >= c.x && cursor[0] < c.x + c.w && cursor[1] >= c.y && cursor[1] < c.y + c.h
        }
        None => true,
    }
}

/// Resolves the top-most hit by the TOTAL order `(StackIndex desc, paint_seq
/// desc, Entity desc)` (Decision Z-order / focus step 3a).
///
/// `FocusPolicy::Block` semantics: a hit `Block` node painted on top is the
/// hovered node and occludes every lower node — which the single-top-most-hit
/// selection already enforces (a lower node can only win if it is strictly
/// higher in the total order, but a node painted below the Block node has a
/// lower `paint_seq`, so it never replaces it). The reset pass (step 3b) then
/// sets every non-hovered node — including those occluded by the Block node — to
/// `None`. The `block` flag is asserted-consistent here to document the
/// invariant that the chosen node, when it is a Block, is genuinely the
/// top-most.
fn resolve_hovered(scratch: &UiInteractionScratch, cursor: [f32; 2]) -> Option<Entity> {
    let mut best: Option<&Candidate> = None;
    for c in &scratch.candidates {
        if !point_in_rect(cursor, c.rect) || !point_in_clip(cursor, c.clip) {
            continue;
        }
        best = match best {
            None => Some(c),
            Some(b) if candidate_is_above(c, b) => Some(c),
            Some(b) => Some(b),
        };
    }
    if let Some(b) = best {
        debug_assert!(
            !b.block
                || scratch.candidates.iter().all(|c| {
                    !(point_in_rect(cursor, c.rect)
                        && point_in_clip(cursor, c.clip)
                        && candidate_is_above(c, b))
                }),
            "a hit Block node must be the total-order top-most hit (occlusion invariant)"
        );
    }
    best.map(|c| c.entity)
}

/// Total order: `a` is above `b` iff higher StackIndex, then higher paint_seq
/// (later paint = on top), then higher Entity id (final stable tie-break).
#[inline]
fn candidate_is_above(a: &Candidate, b: &Candidate) -> bool {
    (a.stack_index, a.paint_seq, a.entity.id().0) > (b.stack_index, b.paint_seq, b.entity.id().0)
}

/// The unconditional reset+write pass (focus step 3b): every interactive node is
/// visited. The hovered node becomes `Pressed`/`Hovered`/`None`; every other
/// node becomes `None`. Each genuine transition toggles the matching EnableTag
/// bit (transition-gated, Decision 1) and writes `RelativeCursorPosition`
/// set-if-changed.
#[allow(clippy::too_many_arguments)]
fn write_interactions(
    world: &mut EcsMaster,
    scratch: &mut UiInteractionScratch,
    config: &UiInteractionConfig,
    hovered: Option<Entity>,
    cursor: [f32; 2],
    clicked: bool,
    released: bool,
    held: bool,
) {
    // Snapshot the candidate (entity, rect) list into a RETAINED buffer so the
    // loop does not borrow `candidates` while mutating the world (no fresh `Vec`).
    scratch.write_nodes.clear();
    scratch
        .write_nodes
        .extend(scratch.candidates.iter().map(|c| (c.entity, c.rect)));
    // Reset the per-frame hover-enter edge buffer (Decision 2 — OnHover edge).
    scratch.hover_entered.clear();

    for i in 0..scratch.write_nodes.len() {
        let (entity, rect) = scratch.write_nodes[i];
        let prev = world
            .get_component::<Interaction>(entity)
            .copied()
            .unwrap_or(Interaction::None);

        let next = if Some(entity) == hovered {
            if released && prev == Interaction::Pressed {
                // Release over the press origin transitions back out of Pressed;
                // the click is resolved in `resolve_pointer`. Hovered (cursor is
                // still over the node) after release.
                Interaction::Hovered
            } else if clicked || (held && prev == Interaction::Pressed) {
                Interaction::Pressed
            } else {
                Interaction::Hovered
            }
        } else {
            Interaction::None
        };

        if next != prev {
            if let Some(mut guard) = world.get_component_mut::<Interaction>(entity) {
                *guard = next;
            }
            // EnableTag bits — transition-gated (Decision 1).
            apply_enable_bits(world, config, entity, prev, next);
            // Hover-enter edge: stamp nodes that became `Hovered` from a
            // non-`Hovered` state so `ui_dispatch_system` fires `OnHover` ONCE on
            // the None→Hovered edge (not every held-hover frame). `Pressed`→
            // `Hovered` on release is NOT a hover-enter (the node was already
            // hovered through the press), so it is excluded.
            if next == Interaction::Hovered && prev == Interaction::None {
                scratch.hover_entered.push(entity);
            }
        }

        // RelativeCursorPosition (set-if-changed; canonical on leave).
        let rel = if Some(entity) == hovered && rect.w > 0.0 && rect.h > 0.0 {
            RelativeCursorPosition {
                cursor_over: true,
                normalized: [
                    (cursor[0] - rect.x) / rect.w - 0.5,
                    (cursor[1] - rect.y) / rect.h - 0.5,
                ],
            }
        } else {
            RelativeCursorPosition {
                cursor_over: false,
                normalized: [0.0, 0.0],
            }
        };
        write_relative_cursor(world, entity, rel);
    }
}

/// Toggles the EnableTag bits to match the `prev → next` interaction transition.
/// Only the bits that actually change are touched (the transition is already
/// gated by the caller).
fn apply_enable_bits(
    world: &mut EcsMaster,
    config: &UiInteractionConfig,
    entity: Entity,
    prev: Interaction,
    next: Interaction,
) {
    let was_hovered = prev != Interaction::None;
    let is_hovered = next != Interaction::None;
    if was_hovered != is_hovered {
        if is_hovered {
            world.enable_id(entity, config.hovered_tag);
        } else {
            world.disable_id(entity, config.hovered_tag);
        }
    }
    let was_pressed = prev == Interaction::Pressed;
    let is_pressed = next == Interaction::Pressed;
    if was_pressed != is_pressed {
        if is_pressed {
            world.enable_id(entity, config.pressed_tag);
        } else {
            world.disable_id(entity, config.pressed_tag);
        }
    }
}

/// Writes `RelativeCursorPosition` set-if-changed (only on a node that hosts the
/// opt-in column).
fn write_relative_cursor(world: &mut EcsMaster, entity: Entity, rel: RelativeCursorPosition) {
    match world.get_component::<RelativeCursorPosition>(entity) {
        Some(cur) if *cur == rel => {}
        Some(_) => {
            if let Some(mut guard) = world.get_component_mut::<RelativeCursorPosition>(entity) {
                *guard = rel;
            }
        }
        None => {}
    }
}

/// Stamp-at-press / release-up click resolution (Decision 2). A press stamps
/// `(origin, action)` into `pending_click`; a release over the SAME origin fires
/// `click_fired` (a one-frame output cleared at the next call), so a same-frame
/// press+release is observed exactly once with no deferral bookkeeping.
fn resolve_pointer(
    world: &mut EcsMaster,
    hovered: Option<Entity>,
    clicked: bool,
    released: bool,
    _held: bool,
) {
    let state = world.resource_mut::<UiPointerState>();
    let slot = &mut state.slots[0];

    // Clear last frame's transient output.
    slot.click_fired = None;

    // Stamp at press: read the origin's OnClick.0 now (Decision 2). We must do
    // this without holding the `state` borrow.
    if clicked && let Some(origin) = hovered {
        let action = read_on_click(world, origin);
        let state = world.resource_mut::<UiPointerState>();
        let slot = &mut state.slots[0];
        slot.pending_click = Some((origin, action));
    }

    // Release: fire iff release lands over the SAME origin entity (Decision 2).
    // The `hovered == Some(origin)` compare is generation-safe on its own —
    // `Entity` equality includes the generation and `hovered` is resolved from
    // live entities this frame, so a recycled slot can never masquerade as the
    // pressed origin. No separate re-validation is needed.
    if released {
        let state = world.resource_mut::<UiPointerState>();
        let slot = &mut state.slots[0];
        if let Some((origin, action)) = slot.pending_click {
            if hovered == Some(origin) {
                slot.click_fired = Some((origin, action));
            }
            slot.pending_click = None;
        }
    }

    // Same-frame press+release over the same node needs no extra work: the press
    // branch stamped `pending_click` and the release branch above already fired
    // `click_fired` from it this same frame. `click_fired` is a one-frame output
    // (cleared at the top of the next call), so dispatch observes the click
    // exactly this frame with no deferral bookkeeping.
}

/// Reads `entity`'s `OnClick.0` action index, or [`NO_ACTION`] if absent.
fn read_on_click(world: &EcsMaster, entity: Entity) -> u16 {
    world.get_component::<OnClick>(entity).map(|c| c.0).unwrap_or(NO_ACTION)
}

/// Keyboard focus + total cross-root tab order + Enter submit edge (focus step
/// 7).
fn update_focus(
    world: &mut EcsMaster,
    scratch: &mut UiInteractionScratch,
    config: &UiInteractionConfig,
    physical: &PhysicalInput,
) {
    // Drop a despawned focused entity first (mirrors the blur-clears-focus rule).
    let mut focused = world.resource::<UiInputFocus>().focused;
    if let Some(f) = focused
        && world.get_component_raw(f, Focusable::component_id()).is_none()
    {
        world.disable_id(f, config.focused_tag);
        focused = None;
    }

    // Build the total-ordered focusable list: (tab_index, paint_seq, Entity).
    scratch.focusables.clear();
    for c in &scratch.candidates {
        if let Some(focusable) = world.get_component::<Focusable>(c.entity) {
            scratch.focusables.push((focusable.tab_index, c.paint_seq, c.entity));
        }
    }
    scratch
        .focusables
        .sort_unstable_by_key(|a| (a.0, a.1, a.2.id().0));

    // Tab edge advances focus cyclically.
    let tab = key_just_pressed(physical, KeyCode::Tab);
    if tab && !scratch.focusables.is_empty() {
        let next = advance_focus(&scratch.focusables, focused);
        if let Some(prev) = focused {
            world.disable_id(prev, config.focused_tag);
        }
        if let Some(n) = next {
            world.enable_id(n, config.focused_tag);
        }
        focused = next;
    }

    // Enter while focused → stamp the submit action for dispatch.
    let enter = key_just_pressed(physical, KeyCode::Enter);
    if enter && let Some(f) = focused {
        let submit = world.get_component::<OnSubmit>(f).map(|s| s.0);
        world.resource_mut::<UiPointerState>().pending_submit = submit;
    } else {
        world.resource_mut::<UiPointerState>().pending_submit = None;
    }

    world.resource_mut::<UiInputFocus>().focused = focused;
}

/// Advances the focus to the next focusable in the total order, cyclically.
fn advance_focus(
    focusables: &[(u32, u32, Entity)],
    current: Option<Entity>,
) -> Option<Entity> {
    if focusables.is_empty() {
        return None;
    }
    let idx = match current {
        Some(c) => focusables.iter().position(|(_, _, e)| *e == c),
        None => None,
    };
    let next = match idx {
        Some(i) => (i + 1) % focusables.len(),
        None => 0,
    };
    Some(focusables[next].2)
}

/// Blur/leave reset (Decision 12): every interactive node → `None`, all
/// interaction bits cleared, all pending clicks cancelled, focus cleared.
fn blur_reset(
    world: &mut EcsMaster,
    scratch: &mut UiInteractionScratch,
    config: &UiInteractionConfig,
) {
    // Snapshot the node list into the retained `write_nodes` buffer (alloc-free);
    // also clear the hover-enter edge buffer (a blurred frame fires no OnHover).
    scratch.write_nodes.clear();
    scratch
        .write_nodes
        .extend(scratch.candidates.iter().map(|c| (c.entity, c.rect)));
    scratch.hover_entered.clear();
    for i in 0..scratch.write_nodes.len() {
        let entity = scratch.write_nodes[i].0;
        let prev = world
            .get_component::<Interaction>(entity)
            .copied()
            .unwrap_or(Interaction::None);
        if prev != Interaction::None {
            if let Some(mut guard) = world.get_component_mut::<Interaction>(entity) {
                *guard = Interaction::None;
            }
            apply_enable_bits(world, config, entity, prev, Interaction::None);
        }
        write_relative_cursor(
            world,
            entity,
            RelativeCursorPosition {
                cursor_over: false,
                normalized: [0.0, 0.0],
            },
        );
    }

    // Cancel every pointer slot's pending click + clear transient outputs.
    let state = world.resource_mut::<UiPointerState>();
    for slot in &mut state.slots {
        slot.pending_click = None;
        slot.click_fired = None;
    }
    state.pending_submit = None;

    // Clear keyboard focus + bit.
    let focused = world.resource::<UiInputFocus>().focused;
    if let Some(f) = focused {
        world.disable_id(f, config.focused_tag);
    }
    world.resource_mut::<UiInputFocus>().focused = None;
}
