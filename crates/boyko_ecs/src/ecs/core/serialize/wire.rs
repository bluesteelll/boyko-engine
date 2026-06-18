//! Per-element value codec for the `SerializeViaFn` encode path (Phase S1.5).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.1 / §3.7 (the owning / bit-restricted
//! decode path). This module defines the in-house [`Wire`] trait — a leaf value
//! encoder/decoder over the cursors in this module's parent
//! ([`SaveCursor`] / [`LoadCursor`]) — plus
//! the [`WireTuple`] field-aggregate the derive routes a component's fields through.
//!
//! # Why an in-house trait (no serde / no third-party)
//!
//! The boyko serialization stack is codegen-driven and dependency-free (the
//! §3.2 crate-boundary decision). [`Wire`] is the boyko analog of a serde
//! `Serialize + Deserialize` pair, but specialized to the single binary wire
//! format: little-endian fixed-width scalars, `u32`-length-prefixed variable
//! data, and a strict "validate, never transmute blindly" read contract (the C3
//! obligation). Every [`Wire::wire_read`] is bounds-checked through the cursor and
//! returns [`DecodeError`] on malformed input — it NEVER panics or invokes UB on
//! hostile bytes.
//!
//! # Encoding (the wire format)
//!
//! - Integers / `f32` / `f64`: their fixed-width little-endian bytes (`to_le_bytes`
//!   / `from_le_bytes`). Floats are encoded by their raw IEEE-754 bytes (no
//!   normalization), so a NaN bit pattern round-trips verbatim.
//! - `bool`: one byte, `0` or `1`; `wire_read` rejects any other value with
//!   [`DecodeError::InvalidBitPattern`].
//! - `char`: the scalar value as a `u32`; `wire_read` validates it through
//!   [`char::from_u32`] (rejects surrogates / out-of-range with
//!   [`DecodeError::InvalidBitPattern`]).
//! - `String`: a `u32` byte length then the UTF-8 bytes; `wire_read` validates the
//!   UTF-8 ([`DecodeError::InvalidBitPattern`] on malformed bytes).
//! - `Vec<T>`: a `u32` element count then each element; the count is bounded by the
//!   remaining input (a hostile count cannot over-allocate — see [`Vec<T>`]'s impl).
//! - `Option<T>`: one tag byte (`0` = `None`, `1` = `Some`), then the payload for
//!   `Some`; any other tag is [`DecodeError::InvalidBitPattern`].
//! - `[T; N]`: exactly `N` elements (no length prefix — the count is the const).
//! - [`Entity`]: the raw saved id (`EntityId.0` as `u64`) followed by the
//!   generation (`u32`). The id is written verbatim; the saved→new remap is S2's
//!   `map_entities_fn` pass, NOT this codec.

use crate::ecs::core::serialize::{DecodeError, LoadCursor, SaveCursor};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::EntityId;

/// The per-element value codec for the `SerializeViaFn` encode path (plan §3.1).
///
/// A component classified `SerializeViaFn` is encoded field-by-field: the derive
/// routes each field through `Wire`, so any owning (`String` / `Vec`) or
/// bit-restricted (`bool` / `char`) field is length-prefixed / validated on read.
/// Implement this (or a manual [`serialize_fn`] /
/// [`deserialize_fn`]) for a field type the derive cannot encode automatically;
/// `#[component(no_serialize)]` opts the whole component out.
///
/// # Read contract (the C3 obligation)
///
/// [`wire_read`](Wire::wire_read) MUST be sound on arbitrary bytes: every read is
/// bounds-checked through the [`LoadCursor`] and every bit-restricted value is
/// validated, returning [`DecodeError`] rather than panicking or producing an
/// invalid value. A malformed stream leaves no half-built state the caller must
/// clean up (the value is returned by-value only on full success).
///
/// [`serialize_fn`]: crate::ecs::core::component::component::Component::serialize_fn
/// [`deserialize_fn`]: crate::ecs::core::component::component::Component::deserialize_fn
pub trait Wire: Sized {
    /// Appends this value's wire bytes to `c` (little-endian / length-prefixed per
    /// the module encoding).
    fn wire_write(&self, c: &mut SaveCursor<'_>);

    /// Reads one value of this type from `c`, advancing the read head. Returns
    /// [`DecodeError`] on a short read or an invalid bit pattern — never panics or
    /// produces an invalid value on hostile input.
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError>;
}

// ── Fixed-width LE scalars: integers + floats ───────────────────────────────────
//
// A macro keeps the integer/float impls byte-identical: write `to_le_bytes`, read
// exactly `size_of` bytes (bounds-checked by the cursor) then `from_le_bytes`. Every
// bit pattern of these widths is a valid value, so no validation beyond the bounds
// check is needed (this is exactly the `SerPod` class, but `Wire` re-states it for
// the per-element path).

macro_rules! impl_wire_le_scalar {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Wire for $ty {
                #[inline]
                fn wire_write(&self, c: &mut SaveCursor<'_>) {
                    c.write_bytes(&self.to_le_bytes());
                }

                #[inline]
                fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
                    let bytes = c.read_bytes(::core::mem::size_of::<$ty>())?;
                    // `read_bytes(size_of)` returns exactly `size_of` bytes, so the
                    // array conversion is infallible; `try_into` keeps it panic-free.
                    let arr = bytes
                        .try_into()
                        .map_err(|_| DecodeError::UnexpectedEof)?;
                    Ok(<$ty>::from_le_bytes(arr))
                }
            }
        )*
    };
}

impl_wire_le_scalar!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64);

// ── Bit-restricted scalars: bool, char ──────────────────────────────────────────

impl Wire for bool {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        c.write_bytes(&[*self as u8]);
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let byte = c.read_bytes(1)?[0];
        match byte {
            0 => Ok(false),
            1 => Ok(true),
            // Any other byte is not a valid `bool` (C3 validate-on-read).
            _ => Err(DecodeError::InvalidBitPattern),
        }
    }
}

impl Wire for char {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        (*self as u32).wire_write(c);
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let scalar = u32::wire_read(c)?;
        // `char::from_u32` rejects surrogates (0xD800..=0xDFFF) and values above
        // 0x10FFFF — the only `u32`s that are not valid `char`s (C3).
        char::from_u32(scalar).ok_or(DecodeError::InvalidBitPattern)
    }
}

// ── Owning: String ──────────────────────────────────────────────────────────────

impl Wire for String {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        write_u32_len(c, self.len());
        c.write_bytes(self.as_bytes());
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let len = c.read_u32()? as usize;
        let bytes = c.read_bytes(len)?;
        // Validate UTF-8 on read (C3): a corrupt byte run is rejected, never
        // transmuted into a `String` (`from_utf8` copies the validated bytes).
        core::str::from_utf8(bytes)
            .map(|s| s.to_owned())
            .map_err(|_| DecodeError::InvalidBitPattern)
    }
}

// ── Containers: Vec<T>, Option<T>, [T; N] ────────────────────────────────────────

impl<T: Wire> Wire for Vec<T> {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        write_u32_len(c, self.len());
        for item in self {
            item.wire_write(c);
        }
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let count = c.read_u32()? as usize;
        // Bound the pre-allocation by the bytes that remain: an element occupies at
        // least one byte on the wire, so a `count` larger than `remaining()` is a
        // hostile/corrupt length and cannot be satisfied — reject it BEFORE
        // `with_capacity` so a forged count never triggers a huge allocation.
        if count > c.remaining() {
            return Err(DecodeError::BadLengthPrefix);
        }
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(T::wire_read(c)?);
        }
        Ok(out)
    }
}

impl<T: Wire> Wire for Option<T> {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        match self {
            None => c.write_bytes(&[0u8]),
            Some(value) => {
                c.write_bytes(&[1u8]);
                value.wire_write(c);
            }
        }
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let tag = c.read_bytes(1)?[0];
        match tag {
            0 => Ok(None),
            1 => Ok(Some(T::wire_read(c)?)),
            // Any other tag is not a valid `Option` discriminant (C3).
            _ => Err(DecodeError::InvalidBitPattern),
        }
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        // No length prefix: `N` is a compile-time constant the reader also knows.
        for item in self {
            item.wire_write(c);
        }
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        // Build the array via `try_from_fn`-style accumulation without `unsafe`:
        // read into a `Vec` (already length-validated per element by the cursor)
        // then convert. `N` is small (a fixed component field), so the transient
        // `Vec` is negligible and avoids a `MaybeUninit` drop-on-error dance.
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::wire_read(c)?);
        }
        // The loop pushed exactly `N` elements, so `try_into` cannot fail; map the
        // (unreachable) error to a decode error to stay panic-free.
        items.try_into().map_err(|_| DecodeError::UnexpectedEof)
    }
}

// ── Entity (raw saved id; remap is S2) ───────────────────────────────────────────

impl Wire for Entity {
    #[inline]
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        // Write the raw saved id (the value the S2 remap pass rewrites) + the
        // generation. `EntityId.0` is a `usize`; encode it as a fixed `u64` so the
        // wire width is target-independent (plan O2 rejects a ptr-width mismatch on
        // load, so the `u64`-on-32-bit case never round-trips a truncated id).
        (self.id().0 as u64).wire_write(c);
        self.generation().wire_write(c);
    }

    #[inline]
    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        let id = u64::wire_read(c)?;
        let generation = u32::wire_read(c)?;
        // The saved id is stored verbatim; S2's `map_entities_fn` rewrites it to a
        // freshly-allocated `Entity`. On a 64-bit target `id as usize` is lossless;
        // a load on a narrower target is rejected earlier by the ptr-width header
        // check (O2), so the cast cannot silently truncate a live id here.
        Ok(Entity::new(EntityId(id as usize), generation))
    }
}

/// Writes a length as a `u32` little-endian prefix (the `String` / `Vec` wire
/// length, plan §3.1). A length that exceeds `u32::MAX` is clamped to `u32::MAX` on
/// write; such a value is impossible for an in-memory component field on any
/// realistic target (a single `Vec`/`String` over 4 GiB), and the reader's
/// remaining-input bound rejects a forged oversized prefix regardless.
#[inline]
fn write_u32_len(c: &mut SaveCursor<'_>, len: usize) {
    let prefix = u32::try_from(len).unwrap_or(u32::MAX);
    prefix.wire_write(c);
}

/// "Every element of this field tuple is [`Wire`]" — the field-aggregate the derive
/// routes a `SerializeViaFn` component's fields through (plan §3.7), mirroring the
/// `SerPodTuple` field-validity proof for the POB arm.
///
/// The derive emits a `WireBridge` mapping the struct to its field tuple `(F0, F1,
/// …)`; the generic encode/decode glue (`serialize_via_wire` / `deserialize_via_wire`
/// in the registry) is bounded `C::Owned: WireTuple`, so it is instantiable ONLY when
/// every field is `Wire`. A field that is not `Wire` (e.g. `Box<u32>`, `Rc<u32>`)
/// fails the bound, the encode-fn autoref arm defers, and the component installs
/// `None` (a graceful demotion — the same deferral the `SerPodTuple` POB gate uses,
/// never a hard compile error). The unit tuple `()` is vacuously `WireTuple` (a ZST
/// tag with no field bytes).
pub trait WireTuple: Sized {
    /// Writes every element in declaration order (the per-field encode, plan §3.7).
    fn tuple_write(&self, c: &mut SaveCursor<'_>);

    /// Reads every element in the SAME order, returning the reconstructed tuple by
    /// value (so a partial read on a malformed stream leaves no half-built state —
    /// the W5 partial-row contract). Returns [`DecodeError`] on the first failure.
    fn tuple_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError>;
}

impl WireTuple for () {
    #[inline]
    fn tuple_write(&self, _c: &mut SaveCursor<'_>) {}

    #[inline]
    fn tuple_read(_c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        Ok(())
    }
}

macro_rules! impl_wire_tuple {
    ($($name:ident),+) => {
        impl<$($name: Wire),+> WireTuple for ($($name,)+) {
            #[inline]
            #[allow(non_snake_case)]
            fn tuple_write(&self, c: &mut SaveCursor<'_>) {
                let ($($name,)+) = self;
                $( $name.wire_write(c); )+
            }

            #[inline]
            #[allow(non_snake_case)]
            fn tuple_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
                // Read each element in declaration order; `?` short-circuits on the
                // first malformed field and the tuple is only returned on full
                // success (no partial-init to clean up).
                $( let $name = <$name as Wire>::wire_read(c)?; )+
                Ok(($($name,)+))
            }
        }
    };
}

impl_wire_tuple!(A);
impl_wire_tuple!(A, B);
impl_wire_tuple!(A, B, C);
impl_wire_tuple!(A, B, C, D);
impl_wire_tuple!(A, B, C, D, E);
impl_wire_tuple!(A, B, C, D, E, F);
impl_wire_tuple!(A, B, C, D, E, F, G);
impl_wire_tuple!(A, B, C, D, E, F, G, H);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_wire_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

/// The WRITE-side counterpart of [`WireTuple`] over a tuple of field BORROWS (plan
/// §3.7). The derive's `WireBridge::as_refs` produces `(&F0, &F1, …)`, which this
/// encodes in declaration order — so the save path never moves a field out of the
/// live component (a `Drop` component forbids that — `E0509`) and never needs
/// `Clone`. Each element borrow must be `Wire`; a non-`Wire` field fails the bound
/// and the encode-fn autoref arm defers to `None` (the same graceful demotion as
/// [`WireTuple`]). The unit tuple `()` is vacuously `WireRefTuple` (a ZST tag).
pub trait WireRefTuple {
    /// Writes every borrowed element in declaration order.
    fn ref_tuple_write(&self, c: &mut SaveCursor<'_>);
}

impl WireRefTuple for () {
    #[inline]
    fn ref_tuple_write(&self, _c: &mut SaveCursor<'_>) {}
}

macro_rules! impl_wire_ref_tuple {
    ($($name:ident),+) => {
        impl<'a, $($name: Wire),+> WireRefTuple for ($(&'a $name,)+) {
            #[inline]
            #[allow(non_snake_case)]
            fn ref_tuple_write(&self, c: &mut SaveCursor<'_>) {
                let ($($name,)+) = self;
                $( $name.wire_write(c); )+
            }
        }
    };
}

impl_wire_ref_tuple!(A);
impl_wire_ref_tuple!(A, B);
impl_wire_ref_tuple!(A, B, C);
impl_wire_ref_tuple!(A, B, C, D);
impl_wire_ref_tuple!(A, B, C, D, E);
impl_wire_ref_tuple!(A, B, C, D, E, F);
impl_wire_ref_tuple!(A, B, C, D, E, F, G);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_wire_ref_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
