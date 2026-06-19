//! The [`Actionlike`] trait and [`ActionKind`] enum (plan §6, Decision 4).
//!
//! Actions are a single **closed, compile-time** user enum — not an open
//! `TypeId` registry like `Component`/`Resource`. A closed enum gives a
//! `const COUNT` + dense `index()`, so [`ActionState`](crate::action::state)
//! and [`InputMap`](crate::action::map) are fixed `[…; COUNT]`-shaped arrays:
//! zero runtime registration, zero hashing, perfect cache layout (strictly
//! better than leafwing's `HashMap<A, …>` here). You rebind *inputs*, not
//! invent actions at runtime.
//!
//! Implemented via `#[derive(Actionlike)]` from `boyko_macros`.

/// The kind of an action — selects how its bindings aggregate and which value
/// accessor is valid (plan §6, Decision 4).
///
/// Per-variant via the `#[actionlike(Button|Axis1D|Axis2D)]` field attribute;
/// the default kind (no attribute) is [`ActionKind::Button`].
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    /// On/off, optionally analog `0..1`. Bindings OR (pressed) / max (value).
    Button,
    /// A single signed axis `-1..1`. Bindings sum → deadzone → clamp.
    Axis1D,
    /// A 2D vector. Bindings sum → deadzone → clamp; WASD-style composites
    /// normalize diagonals.
    Axis2D,
}

/// A typed, compile-time-closed set of game actions.
///
/// Derived via `#[derive(Actionlike)]`. The derive emits a dense
/// `index()` (the variant's declaration order, `0..COUNT`), a `from_index`
/// inverse, the per-variant `kind()` and `name()`, and a `const COUNT`. A
/// `const` assert in the derive guarantees `COUNT <= 256` (the
/// [`BitSet256`](boyko_utils::bit_mask::bit_set_256::BitSet256) cap, V8).
///
/// # Contract
/// - `index(self)` returns a value in `0..COUNT`, unique per variant.
/// - `from_index(index(a)) == Some(a)` for every variant `a` (round-trip).
/// - `from_index(i) == None` for `i >= COUNT`.
///
/// The `Copy + Eq + 'static` bound makes an action a trivially-copied key; no
/// allocation, no `dyn`.
pub trait Actionlike: Copy + Eq + 'static {
    /// The number of distinct actions in the enum — the array-sizing constant.
    const COUNT: usize;

    /// The dense `0..COUNT` index of this action (declaration order).
    fn index(self) -> usize;

    /// The action at dense index `i`, or `None` if `i >= COUNT`.
    fn from_index(i: usize) -> Option<Self>;

    /// The aggregation kind of this action (Button by default; overridable per
    /// variant via `#[actionlike(...)]`).
    fn kind(self) -> ActionKind;

    /// The action's stable name, for the `.keys` format and the rebind UI.
    fn name(self) -> &'static str;
}
