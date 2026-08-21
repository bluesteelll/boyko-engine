//! The canonical UI render gather + discovery — UI-ADVANCED rung S0
//! (`docs/UI-PLAN-SPRITES.md`, architecture D31 + D6b).
//!
//! Three things live here, and they are one mechanism:
//!
//! 1. [`ui_pack_inputs!`] — the SINGLE spelling of the pack-input component set.
//!    It expands to BOTH the `Or<(Changed<…>, …)>` filter type of
//!    [`ui_render_discovery`] AND the gather's per-node read list, from one
//!    component list (`__ui_pack_inputs_list!`). Adding a pack input edits that
//!    one list; a component present in the gather but absent from the discovery
//!    filter (or vice versa) is UNREPRESENTABLE — deleting a component from the
//!    list changes the read tuple's arity and the gather below fails to compile
//!    (rung S0 gate G0-1 / red mutation M0-c).
//! 2. [`gather_ui_nodes`] — the canonical gather: a DFS over `UiRoot`/`Children`
//!    carrying the inherited clip on its stack, mirroring the interaction
//!    hit-test's `collect_candidates` (`boyko_ui/src/interaction/focus.rs`)
//!    traversal exactly — sort roots by entity id, push reversed, pop, children
//!    reversed — so the renderer's pre-order (which IS the D4 paint order) and
//!    the hit-test's `paint_seq` are ONE traversal discipline rather than two
//!    that must be kept in agreement (gate G0-4).
//! 3. [`ui_render_discovery`] — ONE normal scheduled system whose
//!    `Query<(), ui_pack_inputs!(changed)>` bumps [`UiRenderGeneration`] once
//!    per changed frame. One bump site, not one per writer.
//!
//! # The probe counter (S0 item 6 — a DIAGNOSTIC, not `#[cfg(test)]`)
//!
//! [`UiGatherScratch::probes`] counts every component probe the gather issues
//! (the four pack inputs plus `Children`, per visited node). It exists because
//! the gather is the one cost this campaign adds to every node of every frame
//! (§10.8), and because the cheaper-but-wrong gate placement — the compare
//! INSIDE the pack instead of ahead of the gather — is invisible to a repack
//! counter and visible only here (red mutation M0-b). The `relayout_count`
//! lesson (§10.4): a `#[cfg(test)]` counter cannot be read by the observer rung,
//! so this one is unconditional (one `u64` add per probe).
//!
//! # Scratch ownership
//!
//! [`UiGatherScratch`] follows the seam's established host-owned-scratch idiom
//! (`node_buf` / `gather` in [`UiUploadSystem::host_upload_frame_from_world`]):
//! retained buffers the host passes in each frame, cleared and refilled, never
//! reallocated in steady state. It is deliberately NOT an ECS `Resource`: the
//! gather runs against a read-only [`WorldView`], which cannot project `&mut`
//! to a resource — the host (or the test harness) owns it beside the other
//! seam scratch.
//!
//! [`UiUploadSystem::host_upload_frame_from_world`]: crate::ui::upload::UiUploadSystem::host_upload_frame_from_world

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::dispatcher_token::WorldView;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_ui::components::UiRoot;

use crate::ui::pack::{PackInput, UiRenderGeneration};
use crate::ui::upload::UiNode;

/// The ONE component list behind [`ui_pack_inputs!`] — every expansion routes
/// through this arm, so the discovery filter and the gather read list cannot
/// drift (rung S0 gate G0-1: only one spelling can fail to compile).
///
/// At S0 the list is the four pack inputs that exist today. Animation adds
/// `UiVisual` HERE; sprites add theirs at S3–S5; interaction adds its scroll
/// datum HERE — never directly in the gather or the filter (§6 of the plan).
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_pack_inputs_list {
    ($apply:ident $extra:tt) => {
        $crate::$apply! { $extra [
            ::boyko_ui::components::ComputedRect,
            ::boyko_ui::components::UiBackground,
            ::boyko_ui::components::ComputedClip,
            ::boyko_ui::components::StackIndex
        ] }
    };
}

/// Expands the pack-input set into the discovery filter type
/// (`ui_pack_inputs!(changed)`) or the gather's per-node read tuple
/// (`ui_pack_inputs!(read <view>, <entity>, <probes>)`), from the ONE component
/// list in [`__ui_pack_inputs_list!`].
///
/// - `changed` → the type `Or<(Changed<C1>, …, Changed<Cn>)>`, usable in a
///   `Query<(), …>` filter position.
/// - `read view, entity, probes` → the expression
///   `(probe_component::<C1>(view, entity, probes), …)` — an `Option<&C>` per
///   component, in list order. `view: &WorldView<'_>`, `entity: Entity`,
///   `probes: &mut u64`; each argument expression is evaluated once per
///   component, so pass plain borrows.
///
/// Deleting a component from the list changes the read tuple's arity, which
/// fails [`gather_ui_nodes`]'s destructuring at compile time — the M0-c red.
#[macro_export]
macro_rules! ui_pack_inputs {
    (changed) => {
        $crate::__ui_pack_inputs_list! { __ui_pack_inputs_changed () }
    };
    (read $view:expr, $entity:expr, $probes:expr) => {
        $crate::__ui_pack_inputs_list! { __ui_pack_inputs_read ($view, $entity, $probes) }
    };
}

/// [`ui_pack_inputs!`] applier: the `Or<(Changed<…>, …)>` filter TYPE.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_pack_inputs_changed {
    (() [$($c:ty),* $(,)?]) => {
        ::boyko_ecs::ecs::core::iters::query::filter::Or<(
            $(::boyko_ecs::ecs::core::iters::query::filter::Changed<$c>,)*
        )>
    };
}

/// [`ui_pack_inputs!`] applier: the per-node read TUPLE (one `Option<&C>` per
/// pack input, in list order).
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_pack_inputs_read {
    (($view:expr, $entity:expr, $probes:expr) [$($c:ty),* $(,)?]) => {
        ($( $crate::ui::gather::probe_component::<$c>($view, $entity, $probes), )*)
    };
}

/// One typed component probe through the [`WorldView`] read surface, counted.
///
/// Increments `probes` (the S0 diagnostic counter) and forwards to
/// [`WorldView::get_component_raw`], casting the untyped column pointer back to
/// `&C`. This is the ONLY read verb the gather uses, so the probe counter is
/// exact by construction.
#[inline]
pub fn probe_component<'v, C: Component>(
    view: &'v WorldView<'_>,
    entity: Entity,
    probes: &mut u64,
) -> Option<&'v C> {
    *probes = probes.wrapping_add(1);
    let ptr = view.get_component_raw(entity, C::component_id())?;
    // SAFETY: `get_component_raw` returned `Some`, so `ptr` points at a live,
    // initialized component row in the column minted for `C::component_id()` —
    // the id is type-bound to `C` (the registry mints one id per type), so the
    // bytes are a valid `C` at `C`'s alignment (the pool stores rows at natural
    // alignment). The reference is tied to `'v` (the `WorldView` borrow): a
    // `WorldView` exists only inside a `DispatcherToken`'s `&self` window, and
    // every world mutation path needs `&mut` of the token or of `EcsMaster`,
    // which borrowck excludes while `'v` is live — so the row cannot move or be
    // written for the reference's lifetime.
    Some(unsafe { &*(ptr.cast::<C>()) })
}

/// Host-owned retained scratch for [`gather_ui_nodes`] (the seam's
/// `node_buf`/`gather` idiom — see the module doc's "Scratch ownership").
/// All buffers are cleared-and-refilled per gather; capacity persists, so a
/// steady-state gather allocates nothing after warmup.
#[derive(Default)]
pub struct UiGatherScratch {
    /// `UiRoot` entity buffer for the root walk (refilled per gather).
    pub roots: Vec<Entity>,
    /// Archetype-id scratch backing `query_entities_buf` (alloc-free walk).
    pub arch_ids: Vec<ArchetypeId>,
    /// DFS stack of `(entity, inherited clip)` — the clip rides the stack, so
    /// inheritance costs no second pass (G0-4).
    pub stack: Vec<(Entity, Option<[f32; 4]>)>,
    /// DIAGNOSTIC (S0 item 6): component probes issued by the gather, ever
    /// (wrapping). Sample before/after a frame for a per-frame count. A static
    /// frame under the hoisted D6a gate must not advance it at all (G0-2);
    /// §10.8 reports it per node per frame.
    pub probes: u64,
}

/// The canonical UI render gather (S0 item 3): fills `node_buf` with one
/// [`UiNode`] per visible node (a node carrying BOTH `ComputedRect` and
/// `UiBackground`), in D4 paint order, with the inherited clip resolved.
///
/// The traversal mirrors the interaction hit-test's `collect_candidates`
/// exactly (one discipline, two consumers — G0-4):
///
/// - roots = every `UiRoot` entity, sorted by entity id (deterministic
///   cross-root paint order), pushed in reverse so they pop in id order;
/// - DFS: children pushed in reverse so they pop in document order;
/// - the node's own `ComputedClip` NARROWS the inherited clip for its subtree
///   (own wins; absent ⇒ inherit), and the EFFECTIVE clip is what packs;
/// - pre-order IS the emission order IS the `append` sort key downstream.
///
/// Reads go through [`ui_pack_inputs!`]'s `read` arm — the one spelling — plus
/// one `Children` probe per node for the traversal itself. Every probe counts
/// into [`UiGatherScratch::probes`].
///
/// The signature matches the
/// [`host_upload_frame_from_world`](crate::ui::upload::UiUploadSystem::host_upload_frame_from_world)
/// gather seam up to the scratch: wire it as
/// `|view, buf| gather_ui_nodes(&view, &mut gather_scratch, buf)`.
pub fn gather_ui_nodes(
    view: &WorldView<'_>,
    scratch: &mut UiGatherScratch,
    node_buf: &mut Vec<UiNode>,
) {
    // Root walk through the retained buffers (no fresh `Vec`).
    view.query_entities_buf(
        &[UiRoot::component_id()],
        &mut scratch.roots,
        &mut scratch.arch_ids,
    );

    scratch.stack.clear();

    // Roots in a deterministic order (entity id) for a stable cross-root paint
    // sequence; pushed in reverse so they pop in id order (collect_candidates'
    // exact discipline).
    scratch.roots.sort_unstable_by_key(|e| e.id().0);
    for &root in scratch.roots.iter().rev() {
        scratch.stack.push((root, None));
    }

    while let Some((node, inherited_clip)) = scratch.stack.pop() {
        // The ONE spelling of the pack-input reads (G0-1): arity-locked to the
        // component list — an edit there that does not land here fails to build.
        let (rect, background, clip, stack_index) =
            ui_pack_inputs!(read view, node, &mut scratch.probes);

        // The node's own clip narrows the inherited clip for its subtree.
        let own_clip = clip.map(|c| [c.x, c.y, c.w, c.h]);
        let effective_clip = own_clip.or(inherited_clip);

        // Visible ⇔ rect AND background (the pack's own definition of a
        // packable node); a layout-only / style-less node still forwards the
        // clip to its subtree but emits nothing.
        if let (Some(rect), Some(bg)) = (rect, background) {
            node_buf.push(UiNode {
                input: PackInput {
                    rect: [rect.x, rect.y, rect.w, rect.h],
                    color: bg.color,
                    border_color: bg.border_color,
                    corner_radius: bg.corner_radius,
                    border_width: bg.border_width,
                    clip: effective_clip,
                    text_uv: None,
                },
                stack: stack_index.map(|s| s.0).unwrap_or(0),
            });
        }

        // DFS children in reverse so they are visited in document (paint)
        // order. `Children` is a traversal read, not a pack input; it still
        // counts as a probe (it is a per-node component read the gather pays).
        if let Some(children) = probe_component::<Children>(view, node, &mut scratch.probes) {
            for &child in children.as_slice().iter().rev() {
                scratch.stack.push((child, effective_clip));
            }
        }
    }
}

/// The render-discovery system (S0 item 4): ONE normal scheduled system that
/// bumps [`UiRenderGeneration`] when any pack input changed this frame — one
/// bump site for the whole pack-input set, instead of a bump call in every
/// writer.
///
/// The filter type is [`ui_pack_inputs!`]`(changed)` — the same spelling the
/// gather reads, so the discovery set and the gather set cannot drift (G0-1).
/// `iter().next().is_some()` is the per-row change signal (the iterator honors
/// the per-row `Changed` window; `is_empty()` is archetype-level only — the
/// `ui_layout_discovery` precedent). In steady state the scan yields nothing
/// and the generation holds, which is what arms the D6a per-slot gate's skip.
//
// `clippy::type_complexity`: the `Query<(), Or<(Changed<…>, …)>>` change-set
// type IS the SystemParam signature (the `ui_layout_discovery` precedent) —
// it cannot be a `type` alias without losing the SystemParam impl. The macro
// keeps the spelling single; the type stays structural.
#[allow(clippy::type_complexity)]
pub fn ui_render_discovery(
    changed: Query<(), ui_pack_inputs!(changed)>,
    mut generation: ResMut<UiRenderGeneration>,
) {
    if changed.iter().next().is_some() {
        generation.bump();
    }
}
