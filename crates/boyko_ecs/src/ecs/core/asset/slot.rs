//! [`Slot<T>`] — the occupancy discriminant for one row of an
//! [`Assets<T>`](crate::ecs::core::asset::assets::Assets) table.

/// One row of an [`Assets<T>`](crate::ecs::core::asset::assets::Assets) table:
/// either a live asset value, or a vacated slot.
///
/// This is a safe `Vec`-slotmap row, NOT a hand-rolled `VmColumn`-style
/// primitive: the architecture-critic rejected a bespoke `SlotColumn` for
/// this rung because dropping a `T: !Copy` through a raw byte column
/// reintroduces double-free / drop-uninit UB — `VmColumn` is sound only
/// because it *never* drops. Assets are integer-indexed (never
/// pointer-addressed; see `MeshRegistry`/`MaterialRegistry`'s identical
/// `Vec`-is-correct precedent in `boyko_render`), so a plain enum row with
/// Rust's own `Drop` is both simpler and sound with ZERO `unsafe`: the
/// compiler only drops the `Occupied` payload, never a `Vacant` row.
pub(crate) enum Slot<T> {
    /// A live asset value.
    Occupied(T),
    /// A row minted by [`Assets::reserve`](crate::ecs::core::asset::assets::Assets::reserve)
    /// that has no value yet: either still in flight
    /// ([`AssetLoadState::Loading`](crate::ecs::core::asset::asset::AssetLoadState::Loading))
    /// or the load failed
    /// ([`AssetLoadState::Failed`](crate::ecs::core::asset::asset::AssetLoadState::Failed) —
    /// the row STAYS `Reserved`, it never regains a value). Carries no `T`:
    /// there is nothing to drop, and no placeholder value needs constructing
    /// for a type that has no meaningful default.
    Reserved,
    /// A freed row. `next_free` mirrors what was on top of
    /// [`Assets::free`](crate::ecs::core::asset::assets::Assets)'s LIFO
    /// stack at the moment this row was vacated — an intrusive echo of the
    /// flat free-list, read back only as a `debug_assert` cross-check in
    /// [`Assets::add`](crate::ecs::core::asset::assets::Assets::add) when a
    /// row is reused. `Assets::free` (the flat `Vec<u32>`) remains the sole
    /// mechanism actually driving O(1) reuse; this field costs nothing in
    /// release (the assert reading it compiles out) and catches free-list
    /// corruption in debug builds.
    Vacant { next_free: u32 },
}
