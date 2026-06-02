//! The [`Plugins`] trait — variadic plugin registration for
//! [`App::add_plugins`].
//!
//! A sealed marker trait that lets `add_plugins` accept either a single
//! [`Plugin`] or a heterogeneous tuple of `1..=12` items (each of which is
//! itself `Plugins`, so tuples nest). The `Marker` type parameter disambiguates
//! the single-plugin blanket impl from the tuple impls — the same technique
//! `IntoSystem<_, _, M>` uses elsewhere in the tree.

use crate::ecs::core::app::app::App;
use crate::ecs::core::app::plugin::Plugin;

/// Sealing module — prevents external `impl Plugins` so the only blessed inputs
/// to [`App::add_plugins`] are "a [`Plugin`]" or "a tuple of [`Plugins`]".
mod sealed {
    /// Sealed supertrait of [`Plugins`](super::Plugins). Parameterised by the
    /// same disambiguation `Marker` so the leaf and tuple impls do not overlap.
    pub trait Sealed<Marker> {}
}

/// Disambiguation marker for the single-[`Plugin`] leaf impl of [`Plugins`].
///
/// Without a distinct marker, the blanket `impl<P: Plugin> Plugins for P` and
/// the tuple impls would overlap when a tuple element is itself a plugin.
pub struct PluginMarker;

/// A group of plugins accepted by [`App::add_plugins`]. Implemented for any
/// single [`Plugin`] and for tuples of `1..=12` `Plugins` (which nest, since a
/// tuple is itself `Plugins`).
pub trait Plugins<Marker>: sealed::Sealed<Marker> {
    /// Adds every plugin in this group to the `App`, in declaration order.
    fn add_to_app(self, app: &mut App);
}

impl<P: Plugin> sealed::Sealed<PluginMarker> for P {}

impl<P: Plugin> Plugins<PluginMarker> for P {
    #[inline]
    fn add_to_app(self, app: &mut App) {
        app.add_plugin(self);
    }
}

/// Emits the `Plugins` impl for a tuple of the given type-parameter names, each
/// paired with its own disambiguation marker. Structurally mirrors the
/// `SystemParam` tuple macro (`system/params/tuple_impl.rs`) but emits a safe
/// `impl` with a behavioral body.
macro_rules! impl_plugins_tuple {
    ($(($p:ident, $m:ident)),*) => {
        impl<$($p: Plugins<$m>),*, $($m),*> sealed::Sealed<($($m,)*)> for ($($p,)*) {}

        impl<$($p: Plugins<$m>),*, $($m),*> Plugins<($($m,)*)> for ($($p,)*) {
            #[inline]
            fn add_to_app(self, app: &mut App) {
                #[allow(non_snake_case)]
                let ($($p,)*) = self;
                $(
                    $p.add_to_app(app);
                )*
            }
        }
    };
}

impl_plugins_tuple!((P0, M0));
impl_plugins_tuple!((P0, M0), (P1, M1));
impl_plugins_tuple!((P0, M0), (P1, M1), (P2, M2));
impl_plugins_tuple!((P0, M0), (P1, M1), (P2, M2), (P3, M3));
impl_plugins_tuple!((P0, M0), (P1, M1), (P2, M2), (P3, M3), (P4, M4));
impl_plugins_tuple!((P0, M0), (P1, M1), (P2, M2), (P3, M3), (P4, M4), (P5, M5));
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6)
);
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6),
    (P7, M7)
);
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6),
    (P7, M7),
    (P8, M8)
);
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6),
    (P7, M7),
    (P8, M8),
    (P9, M9)
);
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6),
    (P7, M7),
    (P8, M8),
    (P9, M9),
    (P10, M10)
);
impl_plugins_tuple!(
    (P0, M0),
    (P1, M1),
    (P2, M2),
    (P3, M3),
    (P4, M4),
    (P5, M5),
    (P6, M6),
    (P7, M7),
    (P8, M8),
    (P9, M9),
    (P10, M10),
    (P11, M11)
);

impl App {
    /// Adds a group of plugins: either a single [`Plugin`] or a tuple of up to
    /// 12 `Plugins` (which nest, since a tuple is itself `Plugins`).
    ///
    /// Each plugin is added via [`add_plugin`](App::add_plugin) in declaration
    /// order, so duplicate detection and [`Plugin::build`] apply uniformly.
    pub fn add_plugins<M, G: Plugins<M>>(&mut self, plugins: G) -> &mut Self {
        plugins.add_to_app(self);
        self
    }
}
