//! [`Handle<T>`] — a typed, `Copy` reference into an
//! [`Assets<T>`](crate::ecs::core::asset::assets::Assets) table.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use static_assertions::{assert_impl_all, const_assert_eq};

/// A typed reference into an [`Assets<T>`](crate::ecs::core::asset::assets::Assets)
/// table: a slot `index` plus a `generation` counter.
///
/// # Layout
///
/// `#[repr(C)]`, 8 bytes (`u32` + `u32`), `Copy`. `PhantomData<fn() -> T>` is
/// used instead of `PhantomData<T>` so `Handle<T>` is `Send + Sync + Copy`
/// for **every** `T` regardless of `T`'s own auto-trait profile — a bare
/// `PhantomData<T>` would make `Handle<T>` inherit `T`'s variance and
/// auto-trait sensitivity (a `!Send` or invariant `T` would poison the
/// handle), even though a `Handle` never actually stores or drops a `T`.
/// Function-pointer types (`fn() -> T`) are `Send + Sync` unconditionally
/// in `std`, and covariant in `T`.
///
/// `Clone` / `Copy` / `PartialEq` / `Eq` / `Hash` / `Debug` are
/// hand-implemented (not `#[derive(..)]`) for the same reason: a derive on a
/// generic struct adds a `T: Trait` bound to the generated impl, which would
/// again wrongly tie `Handle<T>`'s traits to `T`'s.
///
/// # Generational reuse caveat
///
/// [`Assets::remove`](crate::ecs::core::asset::assets::Assets::remove) frees
/// the slot and bumps its generation, so a stale `Handle` is rejected by
/// [`Assets::get`](crate::ecs::core::asset::assets::Assets::get) /
/// [`get_mut`](crate::ecs::core::asset::assets::Assets::get_mut) /
/// [`contains`](crate::ecs::core::asset::assets::Assets::contains) after
/// reuse. **This reuse is UNSAFE for render-referenced assets** until a
/// later rung carries the generation (or a remap) into the render path: the
/// planned render carrier stores only a 16-bit index, so a freed-and-reused
/// slot renders stale content silently, with no generation check on the GPU
/// side. Until that rung lands, treat render-visible `Assets<T>` tables as
/// append-only/live-forever — do not call `remove` on a handle a renderer
/// may still hold.
#[repr(C)]
pub struct Handle<T> {
    /// Slot index into the owning `Assets<T>`'s parallel arrays.
    index: u32,
    /// Generation stamped at mint time; must match the slot's current
    /// generation for the handle to resolve.
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// Constructs a handle from a raw `index` + `generation` pair.
    ///
    /// Restricted to the crate: only [`Assets::add`](crate::ecs::core::asset::assets::Assets::add)
    /// / [`Assets::remove`](crate::ecs::core::asset::assets::Assets::remove)
    /// (on reuse) and [`AssetServer`](crate::ecs::core::asset::server::AssetServer)
    /// mint handles — an externally-fabricated `Handle` could name a slot it
    /// never legitimately owns.
    #[inline]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            _marker: PhantomData,
        }
    }

    /// Returns the slot index this handle addresses.
    #[inline]
    pub fn index(self) -> u32 {
        self.index
    }

    /// Returns the generation this handle was minted with.
    #[inline]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

// `Handle<T>`'s layout and auto-trait profile do not depend on `T` (see the
// struct doc) — `()` stands in for an arbitrary, possibly `!Send`/`!Sync`/
// invariant `T` to prove the claim holds regardless of the concrete asset
// type.
const_assert_eq!(std::mem::size_of::<Handle<()>>(), 8);
assert_impl_all!(Handle<()>: Send, Sync, Copy);

#[cfg(test)]
mod tests {
    use super::*;

    /// `Handle<T>` is exactly 8 bytes and `Copy` (plan §A0 unit:
    /// `handle_is_8_bytes_copy_send_sync`). The module-level `const_assert_eq!`
    /// / `assert_impl_all!` already pin this at compile time; this test keeps
    /// the property visible in a normal test run.
    #[test]
    fn handle_is_8_bytes_copy_send_sync() {
        assert_eq!(std::mem::size_of::<Handle<()>>(), 8);
        let h = Handle::<()>::new(3, 7);
        let copied = h;
        assert_eq!(h, copied, "Copy must not move out of `h`");
    }

    #[test]
    fn handle_equality_is_index_and_generation() {
        let a = Handle::<()>::new(1, 2);
        let b = Handle::<()>::new(1, 2);
        let c = Handle::<()>::new(1, 3);
        let d = Handle::<()>::new(2, 2);
        assert_eq!(a, b);
        assert_ne!(a, c, "differing generation must compare unequal");
        assert_ne!(a, d, "differing index must compare unequal");
    }
}
