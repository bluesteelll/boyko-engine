//! Shared harness for the GUI P4 interaction + data-bind integration tests.
//!
//! The interaction systems are EXCLUSIVE (`&mut EcsMaster`), so the natural way
//! to drive them in a test is `world.run_system_once(&mut sys)` per frame — but a
//! per-call `run_system` resets the `(last_run, this_run]` change-detection
//! window every call, which the change-gated bind discovery relies on. The bind
//! tests that need a real tick window therefore build a hand-rolled `Schedule`
//! (mirroring the P1 `common::Ui` harness); the pure-interaction tests, which do
//! not depend on `Changed`, drive the exclusive systems directly via a closure
//! that calls the free function.
//!
//! Spawning goes through the same `Commands`/`Arc<Mutex<…>>`-probe pattern as the
//! P1 harness so the `ChildOf`/`Children` hooks maintain the reverse collection
//! the focus DFS reads.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_input::{Actionlike, ActionKind, ActionState, MouseButton, PhysicalInput};

use boyko_ui::components::{ComputedClip, ComputedRect, StackIndex, UiRoot};
use boyko_ui::interaction::components::{FocusPolicy, Focusable, Interaction, RelativeCursorPosition};
use boyko_ui::interaction::focus::{
    ui_focus_system, UiInputFocus, UiInteractionConfig, UiInteractionScratch, UiPointerState,
};
use boyko_ui::interaction::action::{OnClick, OnHover, OnSubmit};
use boyko_ui::interaction::plugin::{TAG_UI_FOCUSED, TAG_UI_HOVERED, TAG_UI_PRESSED};
use boyko_ui::resources::UiViewport;

/// A minimal three-action enum for the dispatch tests. Dense indices: Jump=0,
/// Fire=1, Menu=2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestAction {
    Jump,
    Fire,
    Menu,
}

impl Actionlike for TestAction {
    const COUNT: usize = 3;
    fn index(self) -> usize {
        match self {
            TestAction::Jump => 0,
            TestAction::Fire => 1,
            TestAction::Menu => 2,
        }
    }
    fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(TestAction::Jump),
            1 => Some(TestAction::Fire),
            2 => Some(TestAction::Menu),
            _ => None,
        }
    }
    fn kind(self) -> ActionKind {
        ActionKind::Button
    }
    fn name(self) -> &'static str {
        match self {
            TestAction::Jump => "Jump",
            TestAction::Fire => "Fire",
            TestAction::Menu => "Menu",
        }
    }
}

/// Builder for an interaction-test world: spawns interactive nodes, drives the
/// exclusive `ui_focus_system` with a scripted `PhysicalInput`, and exposes the
/// resulting `Interaction`/pointer/focus state.
pub struct InterWorld {
    pub world: EcsMaster,
    pub config: UiInteractionConfig,
}

impl InterWorld {
    /// Builds a world with the interaction resources + a scale-1.0 1000×800
    /// viewport, registering the three interaction EnableTags exactly as the
    /// plugin does.
    pub fn new() -> Self {
        Self::with_scale(1.0)
    }

    /// Builds a world with a custom viewport scale factor (HiDPI tests).
    pub fn with_scale(scale: f32) -> Self {
        let mut world = EcsMaster::new();
        let hovered_tag = world.register_enable_tag(TAG_UI_HOVERED);
        let pressed_tag = world.register_enable_tag(TAG_UI_PRESSED);
        let focused_tag = world.register_enable_tag(TAG_UI_FOCUSED);
        let config = UiInteractionConfig {
            hovered_tag,
            pressed_tag,
            focused_tag,
        };
        world.insert_resource(config);
        world.insert_resource(UiPointerState::default());
        world.insert_resource(UiInputFocus::default());
        world.insert_resource(UiInteractionScratch::default());
        world.insert_resource(UiViewport {
            width: 1000.0,
            height: 800.0,
            scale_factor: scale,
            generation: 0,
        });
        world.insert_resource(PhysicalInput::default());
        world.insert_resource(ActionState::<TestAction>::new());
        Self { world, config }
    }

    /// Spawns an interactive root node (carries `Interaction` + `ComputedRect` +
    /// `UiRoot`) at the given logical rect, returning its live handle.
    pub fn spawn_node(&mut self, x: f32, y: f32, w: f32, h: f32) -> Entity {
        self.spawn_node_cfg(x, y, w, h, NodeOpts::default(), None)
    }

    /// Spawns an interactive node with full options, optionally under a parent.
    pub fn spawn_node_cfg(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opts: NodeOpts,
        parent: Option<Entity>,
    ) -> Entity {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        self.world.run_system(move |mut cmds: Commands| {
            let mut ec = cmds.spawn(Interaction::None);
            ec.insert(ComputedRect { x, y, w, h });
            if opts.root {
                ec.insert(UiRoot);
            }
            if let Some(s) = opts.stack_index {
                ec.insert(StackIndex(s));
            }
            if let Some(c) = opts.clip {
                ec.insert(c);
            }
            if let Some(b) = opts.block {
                ec.insert(if b { FocusPolicy::Block } else { FocusPolicy::Pass });
            }
            if let Some(a) = opts.on_click {
                ec.insert(OnClick(a));
            }
            if let Some(a) = opts.on_hover {
                ec.insert(OnHover(a));
            }
            if let Some(a) = opts.on_submit {
                ec.insert(OnSubmit(a));
            }
            if opts.relative_cursor {
                ec.insert(RelativeCursorPosition::default());
            }
            if let Some(tab) = opts.focusable {
                ec.insert(Focusable { tab_index: tab });
            }
            if let Some(p) = parent {
                ec.set_parent(p);
            }
            *probe.lock().expect("probe") = Some(ec.id());
        });
        let e = sink.lock().expect("probe").expect("spawned handle");
        assert!(self.world.has_entity(e), "spawned interactive node is live");
        e
    }

    /// Runs one `ui_focus_system` frame against the current `PhysicalInput`.
    pub fn focus(&mut self) {
        ui_focus_system(&mut self.world);
    }

    /// Sets the cursor (PHYSICAL px) for the next frame.
    pub fn set_cursor(&mut self, x: f64, y: f64) {
        self.world.resource_mut::<PhysicalInput>().cursor_pos = [x, y];
    }

    /// Sets the per-frame mouse-button edges/level for the next frame. The left
    /// button is bit 0.
    pub fn set_mouse(&mut self, just_pressed: bool, just_released: bool, held: bool) {
        let left = MouseButton::Left.dense_index().map(|i| 1u8 << i).unwrap_or(1);
        let p = self.world.resource_mut::<PhysicalInput>();
        p.mouse_just_pressed = if just_pressed { left } else { 0 };
        p.mouse_just_released = if just_released { left } else { 0 };
        p.mouse_pressed = if held { left } else { 0 };
    }

    /// Reads a node's `Interaction`.
    pub fn interaction(&self, e: Entity) -> Interaction {
        self.world
            .get_component::<Interaction>(e)
            .copied()
            .unwrap_or(Interaction::None)
    }

    /// Reads a node's `RelativeCursorPosition`.
    pub fn rel(&self, e: Entity) -> Option<RelativeCursorPosition> {
        self.world.get_component::<RelativeCursorPosition>(e).copied()
    }

    /// `UiHovered` enable-bit state.
    pub fn is_ui_hovered(&self, e: Entity) -> bool {
        self.world.is_enabled_id(e, self.config.hovered_tag)
    }

    /// `UiPressed` enable-bit state.
    pub fn is_ui_pressed(&self, e: Entity) -> bool {
        self.world.is_enabled_id(e, self.config.pressed_tag)
    }

    /// `UiFocused` enable-bit state.
    pub fn is_ui_focused(&self, e: Entity) -> bool {
        self.world.is_enabled_id(e, self.config.focused_tag)
    }

    /// The currently keyboard-focused entity.
    pub fn focused(&self) -> Option<Entity> {
        self.world.resource::<UiInputFocus>().focused
    }

    /// The slot-0 `click_fired` output (what dispatch would consume).
    pub fn click_fired(&self) -> Option<(Entity, u16)> {
        self.world.resource::<UiPointerState>().slots[0].click_fired
    }

    /// The slot-0 `pending_click` (a press in flight).
    pub fn pending_click(&self) -> Option<(Entity, u16)> {
        self.world.resource::<UiPointerState>().slots[0].pending_click
    }
}

impl Default for InterWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-node options for `spawn_node_cfg`.
#[derive(Clone, Copy, Default)]
pub struct NodeOpts {
    pub root: bool,
    pub stack_index: Option<u32>,
    pub clip: Option<ComputedClip>,
    pub block: Option<bool>,
    pub on_click: Option<u16>,
    pub on_hover: Option<u16>,
    pub on_submit: Option<u16>,
    pub relative_cursor: bool,
    pub focusable: Option<u32>,
}

impl NodeOpts {
    pub fn root() -> Self {
        Self { root: true, ..Self::default() }
    }
    pub fn with_stack(mut self, s: u32) -> Self {
        self.stack_index = Some(s);
        self
    }
    pub fn with_block(mut self, b: bool) -> Self {
        self.block = Some(b);
        self
    }
    pub fn with_click(mut self, a: u16) -> Self {
        self.on_click = Some(a);
        self
    }
    pub fn with_hover(mut self, a: u16) -> Self {
        self.on_hover = Some(a);
        self
    }
    pub fn with_submit(mut self, a: u16) -> Self {
        self.on_submit = Some(a);
        self
    }
    pub fn with_relative_cursor(mut self) -> Self {
        self.relative_cursor = true;
        self
    }
    pub fn with_focusable(mut self, tab: u32) -> Self {
        self.focusable = Some(tab);
        self
    }
    pub fn with_clip(mut self, c: ComputedClip) -> Self {
        self.clip = Some(c);
        self
    }
    pub fn with_root(mut self) -> Self {
        self.root = true;
        self
    }
}

/// Marks an entity as a root after spawn (the spawn helper always makes a fresh
/// entity; this is sugar to read clearer at the call site).
pub fn opts_root() -> NodeOpts {
    NodeOpts::root()
}
