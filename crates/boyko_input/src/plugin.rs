//! [`InputPlugin<A>`] — the Phase-18 [`Plugin`] that wires `boyko_input` into an
//! [`App`] (plan §7.4).
//!
//! One plugin per [`Actionlike`] enum `A`. It inserts the source-agnostic raw
//! resources ([`RawInputQueue`] + [`PhysicalInput`]) plus the per-`A`
//! [`ActionState`] + [`InputMap`] (allocating their fixed buffers ONCE on the
//! cold build path), and registers the ingest system
//! [`update_action_state::<A>`] on [`CoreSchedule::Main`] ordered
//! `.before_set(`[`GameplaySet`]`)` — the variable/render step, NOT the fixed
//! step (C3: input is sampled once per frame; the fixed loop reads the
//! frame-stable snapshot).
//!
//! [`Plugin`]: boyko_ecs::ecs::core::app::Plugin
//! [`App`]: boyko_ecs::ecs::core::app::App
//! [`CoreSchedule`]: boyko_ecs::ecs::core::app::CoreSchedule

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::schedule::system_set::SystemSet;

use crate::action::actionlike::Actionlike;
use crate::action::map::{InputMap, InputMapBuilder};
use crate::action::process::{clear_consumed_fixed_edges, update_action_state};
use crate::action::state::ActionState;
use crate::constants::RAW_QUEUE_CAP;
use crate::persist::load_keys;
use crate::raw::queue::{PhysicalInput, RawInputQueue};

/// The default schedule set that gameplay systems join so the input ingest can
/// be ordered **before** them.
///
/// [`InputPlugin`] registers [`update_action_state`] `.before_set(GameplaySet)`,
/// so any system placed `.in_set(GameplaySet)` observes a freshly-updated
/// [`ActionState`] for the frame. A game that uses its own set can reproduce the
/// ordering with an explicit `.after(input_key)` instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GameplaySet;

impl SystemSet for GameplaySet {}

/// The input plugin for action enum `A` (plan §7.4).
///
/// Build it from a default [`InputMap`] — typically via
/// [`InputMap::builder`](crate::action::map::InputMap::builder) — and add it with
/// `app.add_plugin(InputPlugin::new(map))`. The plugin owns the template map and
/// inserts a copy into the world at [`build`](Plugin::build), so one template can
/// seed several worlds.
///
/// # `.keys` override (I5)
/// If [`with_keys_path`](Self::with_keys_path) set an override path and the file
/// is present and readable, it is loaded as an **override-delta** over the
/// default map at build (plan §9.3): actions the file omits keep their defaults;
/// actions it mentions are fully overridden. A missing/unreadable file falls back
/// to the default map (a fresh install has no override yet), and per-line parse
/// errors are recoverable — a hand-edited config never bricks the game.
pub struct InputPlugin<A: Actionlike> {
    /// The template map copied into the world at build.
    default_map: InputMap<A>,
    /// Optional `.keys` override path; loaded as a delta at build (I5).
    keys_path: Option<&'static str>,
}

impl<A: Actionlike> InputPlugin<A> {
    /// Creates a plugin from a default binding map.
    #[inline]
    pub fn new(default_map: InputMap<A>) -> Self {
        Self {
            default_map,
            keys_path: None,
        }
    }

    /// Sets a `.keys` override path. At [`build`](Plugin::build), if the file is
    /// present and readable, its bindings are loaded as an override-delta over the
    /// default map (plan §9.3); otherwise the default map is used unchanged.
    #[inline]
    pub fn with_keys_path(mut self, path: &'static str) -> Self {
        self.keys_path = Some(path);
        self
    }

    /// The configured `.keys` override path, if any.
    #[inline]
    pub fn keys_path(&self) -> Option<&'static str> {
        self.keys_path
    }

    /// Resolves the map to insert: the default map, or — if a `keys_path` is set
    /// and the file is present and readable — the default map with the `.keys`
    /// override-delta applied (plan §9.3).
    ///
    /// A missing/unreadable file silently falls back to the default map (a fresh
    /// install has no override). Per-line parse errors are recoverable and folded
    /// into the loaded map; they do not abort the build. This is the cold build
    /// path, so the file read + parse allocations are off the per-frame path.
    fn resolve_map(&self) -> InputMap<A> {
        let Some(path) = self.keys_path else {
            return self.default_map.clone_arena();
        };
        let Ok(src) = std::fs::read_to_string(path) else {
            // No override file yet (or unreadable) — use the default map.
            return self.default_map.clone_arena();
        };
        let mut builder = InputMapBuilder::<A>::from_map(&self.default_map);
        let _report = load_keys(&src, &mut builder);
        builder.build()
    }
}

impl<A: Actionlike> Plugin for InputPlugin<A> {
    fn build(&self, app: &mut App) {
        // Cold path: allocate every fixed buffer ONCE.
        app.insert_resource(RawInputQueue::with_capacity(RAW_QUEUE_CAP));
        app.insert_resource(PhysicalInput::default());
        app.insert_resource(ActionState::<A>::with_count(A::COUNT));
        // Load the `.keys` override-delta if a path is set and the file reads;
        // otherwise insert the default map verbatim (plan §9.3 / §7.4).
        app.insert_resource(self.resolve_map());

        // Register the ingest on the Main (variable) step, before the gameplay
        // set — NOT the fixed step (C3, plan §7.3). `before_set` is the
        // set-level ordering API (`before` takes a `SystemKey`).
        //
        // The sticky-edge clear (`clear_consumed_fixed_edges`) runs FIRST on
        // Main, so it observes the current frame's `FixedTime::steps_this_frame`
        // (the fixed loop already ran this frame — fixed-loop-first order) and
        // clears the accumulated frozen edges only when a batch consumed them,
        // BEFORE `update_action_state` OR-accumulates the next batch (C3,
        // BUG-I4-C3 no-miss / no-double-count). `before` takes the clear
        // system's `SystemKey`, so the two are strictly ordered.
        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            let clear_key = b.add_system(clear_consumed_fixed_edges::<A>).key();
            b.add_system(update_action_state::<A>)
                .after(clear_key)
                .before_set(GameplaySet);
        });
    }
}
