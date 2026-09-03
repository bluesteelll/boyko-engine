//! A read-only snapshot of the live UI subtree (P3 §9, Decision 7 / 10).
//!
//! The reconcile and the serializer need to read live components AND issue
//! structural `Commands`. Because the engine has no entity-yielding `Query`
//! (see `layout.rs`'s deviation note) and a `Commands`-using FunctionSystem
//! cannot also call `get_component`, the watch system runs as an EXCLUSIVE
//! `&mut EcsMaster` system: it builds this OWNED snapshot from the world FIRST
//! (one `query_entities` call + `get_component` reads, scoped to the document's
//! roots), releasing the world borrow, then replays structural changes through a
//! `Commands` closure (`world.run_system`). The snapshot owns its data, so it
//! never aliases the world while commands are emitted.
//!
//! Scope (Decision 10): the view is built by walking DOWN from the document's
//! roots via `Children`, so it covers exactly the document's own subtree and
//! never foreign `UiName`-bearing entities.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;

use crate::binding::components::{BindText, BindValue};
use crate::components::{
    Bar, BarFill, Button, ComputedClip, ContentSize, StackIndex, UiAbsolute, UiAlign, UiAnchor,
    UiGrid, UiImage, UiLayout, UiName, UiRoot, UiSourceOrder, UiSpacing,
};
use crate::interaction::action::{OnClick, OnHover, OnSubmit};
use crate::text::components::UiText;

/// The owned, text-relevant component snapshot of one live node.
///
/// `Option` per component = presence (the archetype hosts it); a ZST marker is a
/// `bool`, since there is no value to carry.
///
/// # Scope: the whole `.ui` vocabulary, not just the reconcile's patch set
///
/// This snapshot is the serializer's ONLY input, so whatever it omits, a save
/// cannot write. It previously carried 8 members of a 21-member vocabulary, which
/// put a STRUCTURAL ceiling under `serialize_ui`: twelve author-owned components
/// were unwritable no matter what the writer did. It now carries every member the
/// parser accepts except `ComputedRect` (layout output — Decision 14, and
/// `WriterPolicy::Omit` in [`vocab`](crate::text::vocab) records the reason) and
/// the private `UiSourceOrder`.
///
/// The added `get_component` reads are the cost of a lossless save. They land on
/// the reconcile/serialize path, which runs on a file-watch event, not per frame.
#[derive(Clone, Debug)]
pub struct LiveNode {
    /// The node's entity handle.
    pub entity: Entity,
    /// The node's live parent (its `ChildOf`), or `None` for a document root.
    pub parent: Option<Entity>,
    /// The node's live children, in `Children` slice order (unspecified order;
    /// the reconcile keys by `UiName`/`UiSourceOrder`, never by this order).
    pub children: Vec<Entity>,
    /// The node's `UiName`, if present (the primary diff key).
    pub name: Option<UiName>,
    /// The node's `UiSourceOrder`, if present (the anonymous-reconcile key).
    /// Crate-private: `UiSourceOrder` is a private reload-only bookkeeping key.
    pub(crate) source_order: Option<UiSourceOrder>,

    // Text-owned component presence + value (Decision 14: `ComputedRect` is
    // layout output, excluded from the patch set, so it is not snapshotted).
    pub layout: Option<UiLayout>,
    pub spacing: Option<UiSpacing>,
    pub align: Option<UiAlign>,
    pub absolute: Option<UiAbsolute>,
    pub content_size: Option<ContentSize>,
    pub stack_index: Option<StackIndex>,
    pub clip: Option<ComputedClip>,
    pub is_root: bool,

    // The style / widget / interaction half of the vocabulary (P5b, P6a, P4,
    // GUI #27). Snapshotted so a save can write them back.
    pub text: Option<UiText>,
    pub image: Option<UiImage>,
    pub grid: Option<UiGrid>,
    pub anchor: Option<UiAnchor>,
    pub is_button: bool,
    pub is_bar: bool,
    pub is_bar_fill: bool,
    pub on_click: Option<OnClick>,
    pub on_hover: Option<OnHover>,
    pub on_submit: Option<OnSubmit>,
    pub bind_text: Option<BindText>,
    pub bind_value: Option<BindValue>,
}

/// A read-only view over the live UI subtree rooted at the document's roots.
///
/// Built once per reconcile from one `query_entities` call + scoped descent
/// (Decision 7). Lookups are by `Entity` (a linear scan over the snapshot — the
/// subtree is small and this is a cold path); the per-parent diff indices the
/// reconcile builds on top are the binary-search / ordinal keys.
#[derive(Clone, Debug, Default)]
pub struct UiTreeView {
    /// All live nodes in the document's subtree, parents before children
    /// (pre-order from the roots).
    pub nodes: Vec<LiveNode>,
}

impl UiTreeView {
    /// Builds the view by walking DOWN from `roots` via `Children`, snapshotting
    /// each reachable node's text-relevant components. Scoped to the document's
    /// own subtree (Decision 10); a stale/despawned root is skipped.
    pub fn build(world: &EcsMaster, roots: &[Entity]) -> Self {
        let mut nodes = Vec::new();
        // Iterative pre-order walk (no recursion depth concern on a cold path).
        let mut stack: Vec<(Entity, Option<Entity>)> = Vec::new();
        // Push roots in reverse so the pop order is declaration order.
        for &r in roots.iter().rev() {
            stack.push((r, None));
        }
        while let Some((entity, parent)) = stack.pop() {
            if !world.has_entity(entity) {
                continue;
            }
            let children: Vec<Entity> = world
                .get_component::<Children>(entity)
                .map(|c| c.as_slice().to_vec())
                .unwrap_or_default();
            let node = LiveNode {
                entity,
                parent,
                children: children.clone(),
                name: world.get_component::<UiName>(entity).copied(),
                source_order: world.get_component::<UiSourceOrder>(entity).copied(),
                layout: world.get_component::<UiLayout>(entity).copied(),
                spacing: world.get_component::<UiSpacing>(entity).copied(),
                align: world.get_component::<UiAlign>(entity).copied(),
                absolute: world.get_component::<UiAbsolute>(entity).copied(),
                content_size: world.get_component::<ContentSize>(entity).copied(),
                stack_index: world.get_component::<StackIndex>(entity).copied(),
                clip: world.get_component::<ComputedClip>(entity).copied(),
                is_root: world.has_component(entity, UiRoot::component_id()),
                text: world.get_component::<UiText>(entity).copied(),
                image: world.get_component::<UiImage>(entity).copied(),
                grid: world.get_component::<UiGrid>(entity).copied(),
                anchor: world.get_component::<UiAnchor>(entity).copied(),
                is_button: world.has_component(entity, Button::component_id()),
                is_bar: world.has_component(entity, Bar::component_id()),
                is_bar_fill: world.has_component(entity, BarFill::component_id()),
                on_click: world.get_component::<OnClick>(entity).copied(),
                on_hover: world.get_component::<OnHover>(entity).copied(),
                on_submit: world.get_component::<OnSubmit>(entity).copied(),
                bind_text: world.get_component::<BindText>(entity).copied(),
                bind_value: world.get_component::<BindValue>(entity).copied(),
            };
            nodes.push(node);
            // Children pushed in reverse so the pre-order is preserved on pop.
            for &child in children.iter().rev() {
                stack.push((child, Some(entity)));
            }
        }
        Self { nodes }
    }

    /// Looks up a node by entity (linear scan; the subtree is small, cold path).
    #[inline]
    pub fn get(&self, entity: Entity) -> Option<&LiveNode> {
        self.nodes.iter().find(|n| n.entity == entity)
    }

    /// The document's root nodes (those with no scoped parent), in pre-order.
    pub fn roots(&self) -> impl Iterator<Item = &LiveNode> + '_ {
        self.nodes.iter().filter(|n| n.parent.is_none())
    }
}
