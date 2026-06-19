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
use crate::action::map::InputMap;
use crate::action::process::{clear_consumed_fixed_edges, update_action_state};
use crate::action::state::ActionState;
use crate::constants::RAW_QUEUE_CAP;
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
/// # I5 seam
/// `keys_path` reserves the `.keys` override location: in I5 a present override
/// file is loaded as a delta over the default map at build. For I4 it is stored
/// but unused — the default map is inserted verbatim.
pub struct InputPlugin<A: Actionlike> {
    /// The template map copied into the world at build.
    default_map: InputMap<A>,
    /// Reserved `.keys` override path (loaded in I5; ignored in I4).
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

    /// Sets a `.keys` override path. Reserved for I5 (`.keys` persistence); in I4
    /// the path is stored but the default map is inserted unchanged.
    #[inline]
    pub fn with_keys_path(mut self, path: &'static str) -> Self {
        self.keys_path = Some(path);
        self
    }

    /// The configured `.keys` override path, if any (I5 seam).
    #[inline]
    pub fn keys_path(&self) -> Option<&'static str> {
        self.keys_path
    }
}

impl<A: Actionlike> Plugin for InputPlugin<A> {
    fn build(&self, app: &mut App) {
        // Cold path: allocate every fixed buffer ONCE.
        app.insert_resource(RawInputQueue::with_capacity(RAW_QUEUE_CAP));
        app.insert_resource(PhysicalInput::default());
        app.insert_resource(ActionState::<A>::with_count(A::COUNT));
        // I5 seam: a present `keys_path` override would be loaded here as a delta
        // over `default_map`; for I4 the default map is inserted verbatim.
        app.insert_resource(self.default_map.clone_arena());

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
