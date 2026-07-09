//! The [`Bindable`] codegen trait (GUI P4 Decision 7).
//!
//! `#[derive(Bindable)]` (in `boyko_macros`) implements this for a component: a
//! `u8`-indexed `fmt_field` / `value_field` over the struct's fields, a
//! parse-time `field_id` name→index resolver, and a `register_bind_accessor`
//! that installs the type-erased fn-pointer pair into the registry's
//! `BIND_ACCESSORS` table.
//!
//! Reflection-free: field identity is a compile-time `u8` resolved once at
//! parse/spawn — no runtime string compare, no `HashMap`, no `TypeId` / `Any`,
//! no `Box<dyn Fn>` (Principle 1).
//!
//! Two call paths (Decision 7):
//! * the `ui!` monomorphized path calls `fmt_field(field, &mut UiTextBuffer)`
//!   with a CONCRETE sink (zero accessor indirection, no `dyn Write` vtable);
//! * the `.ui` dynamic path goes through the installed `BindAccessor` fn-pointer
//!   + a `&mut dyn fmt::Write` trampoline (both only when the source changed).

use boyko_ecs::ecs::core::component::component::Component;

/// A component whose fields can be read by a `u8` id and formatted/valued for
/// data binding (Decision 7). Implemented by `#[derive(Bindable)]`.
pub trait Bindable: Component {
    /// The number of bindable fields (declaration order assigns `0..FIELD_COUNT`).
    const FIELD_COUNT: u8;

    /// Formats field `field` into `out` (the `ui!` concrete-sink and the
    /// type-erased trampoline both call this). An out-of-range `field` is a
    /// silent no-op.
    fn fmt_field(&self, field: u8, out: &mut dyn core::fmt::Write) -> core::fmt::Result;

    /// Returns field `field` as an `f32` (the `BindValue` path). An out-of-range
    /// `field` returns `0.0`.
    fn value_field(&self, field: u8) -> f32;

    /// Resolves a field NAME to its `u8` id (cold, parse-time). `None` for an
    /// unknown name.
    fn field_id(name: &str) -> Option<u8>;

    /// Installs this type's type-erased [`BindAccessor`] into the registry table
    /// (once per type per process; write-once). Called at plugin/registration
    /// setup so the `.ui` dynamic path can resolve the accessor by `ComponentId`.
    ///
    /// [`BindAccessor`]: boyko_ecs::ecs::core::component::component_registry::BindAccessor
    fn register_bind_accessor();
}
