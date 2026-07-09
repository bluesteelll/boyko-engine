//! The [`Plugin`] trait — a modular unit of [`App`] configuration.
//!
//! A plugin bundles a coherent slice of setup (its systems, resources, states,
//! and sub-plugins) behind one [`Plugin::build`] call, so a consumer composes a
//! world out of `app.add_plugins((RenderPlugin, PhysicsPlugin, InputPlugin))`
//! instead of hand-wiring every registration.

use crate::ecs::core::app::app::App;

/// A modular unit of [`App`] configuration. Implement [`build`](Plugin::build)
/// to register your systems / resources / states / sub-plugins in one call.
///
/// # Lifecycle — consumed at `build`
///
/// Unlike Bevy, a `Plugin` is **consumed when it is added**: [`App::add_plugin`]
/// calls [`build`](Plugin::build) immediately and then drops the plugin value.
/// The `App` never retains the instance. Bevy keeps every plugin alive in a
/// `Vec<Box<dyn Plugin>>` for its deferred `finish`/`cleanup` async-init
/// lifecycle (sub-apps, render world); boyko has no such lifecycle, so there is
/// no reason to retain the instance — and therefore **no `Send + Sync`
/// supertrait** is required (Bevy needs it only because the retained instances
/// may cross the sub-app thread boundary). Dropping that bound is strictly more
/// permissive: a plugin may capture `!Send` setup data (e.g. an `Rc`).
///
/// The only supertrait is `'static`, required so [`App::add_plugin`] can key
/// duplicate detection on `TypeId::of::<P>()`.
pub trait Plugin: 'static {
    /// Configures the `App`. Called exactly ONCE, immediately, when the plugin
    /// is added via [`App::add_plugin`] / [`App::add_plugins`].
    fn build(&self, app: &mut App);

    /// Human-readable name for duplicate-plugin diagnostics. Defaults to the
    /// fully-qualified type name, which distinguishes generic instantiations
    /// (`Foo<A>` vs `Foo<B>`) just as their distinct `TypeId`s do.
    fn name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }
}
