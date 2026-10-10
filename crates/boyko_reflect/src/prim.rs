//! The `prim::` accessor library (CORE C4): one monomorphic `get`/`set` pair per
//! [`ScalarKind`], and the **release** kind check that makes a stale write a refusal
//! instead of a corruption.
//!
//! These twenty-four functions are what a `Prim` [`FieldInfo`]'s `get`/`set` slots are
//! filled from. Each is baked for one concrete field type; there is no generic
//! dispatch, no `dyn`, and no downcast — the fn-pointer slot *is* the dispatch, and
//! §8 is explicit that this is an indirect call rather than a free one.
//!
//! # The read/write asymmetry, stated where both halves live side by side
//!
//! * **Reads take a shared reborrow** — `&*(p as *const T)` — the `Bindable`
//!   trampoline's precedented, Miri-clean pattern (CORE F11).
//! * **Every writer stays raw** — `ptr::write`, **never** an intermediate `&mut T`.
//!
//! That asymmetry is deliberate (analysis B.7). A `&mut T` materialized inside an
//! accessor is a unique retag over memory the ECS may hold other live raw pointers
//! into; the read side has no such exposure because a shared reborrow that dies inside
//! the same statement cannot invalidate anything. `FieldMut<'a>` — an accessor that
//! *hands out* a `&mut` into a field — is deferred to v2 behind a full Tree-Borrows
//! analysis, precisely because it is the "cached pointer + reborrow" class TB caught in
//! this engine after three critic rounds had approved it.
//!
//! # The kind check is a `bool`, and it is checked BEFORE memory is touched
//!
//! CORE **D11**: the legitimate `--release --features reflect` editor build compiles
//! `debug_assert!` out, and that build is exactly where an editor passes a stale
//! `(ComponentId, field)` triple after a hot-reload. So the check is a release `-> bool`
//! and its red is shown in a **release-profile** test (`tests/c4_prim.rs`, gate 3), not
//! a debug one. The extractor doing the checking is [`Scalar`]'s own — so a
//! non-canonical payload (a hand-built `Scalar { kind: U8, bits: 300 }`) is refused by
//! the same branch that refuses a wrong kind, and neither reaches the store.
//!
//! # Why a macro, and why no `#[inline]`
//!
//! The twenty-four bodies are generated from one `prim_accessors!` arm so they
//! **cannot drift**: C1's second RED mutation was a single per-kind extractor typo
//! (a zero-extending `as_i8`), invisible except to a per-kind test, and a library of
//! twenty-four hand-copied bodies is twenty-four chances to write it again. The gates
//! stay per-kind regardless — the macro removes the *drift*, not the *coverage*.
//!
//! No `#[inline]`: in production every one of these is reached through a `FieldInfo`
//! fn-pointer slot, where the attribute cannot help, and principle 7 forbids
//! doctrine-driven inlining. If a direct-call consumer ever appears and a profile shows
//! it matters, that is a measurement, and it changes this paragraph with it.
//!
//! # The by-kind dispatch, and why it is a `match` rather than a dense table
//!
//! [`getter_for`] / [`setter_for`] map a [`ScalarKind`] onto its pair. They exist for
//! the [`array`] element accessors (CORE C5), which are handed an
//! [`ArrayInfo::elem`][ai] rather than a baked fn-pointer, and for C7's derive, which
//! picks a slot from the kind it inferred.
//!
//! A dense `[unsafe fn(…); 12]` indexed by `kind as usize` would be the house shape for
//! a *runtime* table, but this one is resolved at compile time and a jump table is what
//! the `match` lowers to anyway — while the `match`, written with **no wildcard arm and
//! generated from the same macro rows as the accessors themselves**, additionally makes
//! a newly added [`ScalarKind`] a *compile error* until it has a pair. That property is
//! the reason for the shape; the discriminant-indexed array would silently answer a new
//! kind with whatever sat at that index.
//!
//! [`FieldInfo`]: crate::type_info::FieldInfo
//! [`array`]: crate::array
//! [ai]: crate::type_info::ArrayInfo::elem

use std::ptr;

use boyko_ecs::ecs::identifiers::primitives::EntityId;

use crate::scalar::{Scalar, ScalarKind};

/// Generates one monomorphic `get`/`set` pair per [`ScalarKind`], plus the two
/// wildcard-free dispatch functions over the same rows.
///
/// The single `// SAFETY:` comment inside each half covers **every** expansion: the
/// invariants are identical across kinds because the bodies are, and stating them once
/// at the generating site is what keeps them from drifting apart the way twenty-four
/// copies would.
macro_rules! prim_accessors {
    ($($kind:ident, $get:ident, $set:ident, $t:ty, $extract:ident, $ty_name:literal;)*) => {
        /// Returns the reader baked for `kind`.
        ///
        /// The `match` carries **no wildcard arm** and is generated from the same rows
        /// as the accessors, so a new [`ScalarKind`] fails to compile here until it has
        /// a pair — see the module header for why this is not a discriminant-indexed
        /// dense table.
        pub fn getter_for(kind: ScalarKind) -> unsafe fn(*const u8) -> Scalar {
            match kind { $( ScalarKind::$kind => $get, )* }
        }

        /// Returns the writer baked for `kind`. See [`getter_for`].
        pub fn setter_for(kind: ScalarKind) -> unsafe fn(*mut u8, Scalar) -> bool {
            match kind { $( ScalarKind::$kind => $set, )* }
        }

        $(
        #[doc = concat!("Reads a `", $ty_name, "` field as a [`Scalar`].")]
        ///
        /// # Safety
        ///
        /// `p` points at a live, initialized,
        #[doc = concat!("`align_of::<", $ty_name, ">()`-aligned instance of the field")]
        /// type this fn-pointer was installed for; `offset_of!` guarantees in-bounds
        /// and field-aligned; provenance is inherited from the arena-rooted base the
        /// caller derived. The value must not be concurrently written.
        pub unsafe fn $get(p: *const u8) -> Scalar {
            // SAFETY: the caller guarantees `p` addresses a live, initialized,
            // correctly aligned value of this exact type, with provenance covering it.
            // The shared reborrow is read-only and dies inside this statement, so it
            // can invalidate no other pointer the caller holds (CORE F11's pattern).
            let this: &$t = unsafe { &*(p as *const $t) };
            Scalar::from(*this)
        }

        #[doc = concat!("Writes a `", $ty_name, "` field, returning `false` — **before")]
        /// touching memory** — when `v` does not carry this kind.
        ///
        /// The refusal is a **release** `bool` (CORE D11), never a `debug_assert!`: the
        /// build where the stale-triple case actually happens is the one where a
        /// `debug_assert!` no longer exists. A non-canonical payload is refused by the
        /// same branch, because the check *is* [`Scalar`]'s own checked extractor.
        ///
        /// # Safety
        ///
        #[doc = concat!("As [`", stringify!($get), "`], with write permission: `p` must")]
        /// be writable for the field's size, correctly aligned, and no reference into
        /// it may be live across this call.
        pub unsafe fn $set(p: *mut u8, v: Scalar) -> bool {
            let Some(value) = v.$extract() else {
                return false;
            };
            // SAFETY: the kind check above proved `v` carries exactly this type's
            // value; the caller guarantees `p` is a writable, correctly aligned,
            // in-bounds field of that type. The store is RAW — no intermediate
            // `&mut $t` is ever created, which is the module header's asymmetry.
            unsafe { ptr::write(p as *mut $t, value) };
            true
        }
    )*};
}

prim_accessors! {
    Bool,     get_bool,      set_bool,      bool,     as_bool,      "bool";
    U8,       get_u8,        set_u8,        u8,       as_u8,        "u8";
    U16,      get_u16,       set_u16,       u16,      as_u16,       "u16";
    U32,      get_u32,       set_u32,       u32,      as_u32,       "u32";
    U64,      get_u64,       set_u64,       u64,      as_u64,       "u64";
    I8,       get_i8,        set_i8,        i8,       as_i8,        "i8";
    I16,      get_i16,       set_i16,       i16,      as_i16,       "i16";
    I32,      get_i32,       set_i32,       i32,      as_i32,       "i32";
    I64,      get_i64,       set_i64,       i64,      as_i64,       "i64";
    F32,      get_f32,       set_f32,       f32,      as_f32,       "f32";
    F64,      get_f64,       set_f64,       f64,      as_f64,       "f64";
    EntityId, get_entity_id, set_entity_id, EntityId, as_entity_id, "EntityId";
}
