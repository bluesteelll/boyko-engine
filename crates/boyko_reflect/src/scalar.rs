//! `Scalar` + `ScalarKind` — the POD value cell every field accessor traffics in
//! (CORE C1, taxonomy §3).
//!
//! EnTT's `meta_any`-with-SOO idea specialized to POD: one 16-byte, `Copy`,
//! heap-free tagged cell. The tag doubles as the `ValueKind` guard on `set`
//! (CORE D11), so it is not extra cost.
//!
//! # The payload encodings, in one place
//!
//! * **`Ux` / `Bool`** — the value zero-extended into `bits`.
//! * **`Ix`** — the value **sign-extended** to 64 bits ([`ix_to_bits`] /
//!   [`ix_from_bits`] — THE sign rule, written once, used by every signed
//!   constructor and extractor).
//! * **`F32` / `F64`** — the IEEE-754 bit pattern (`to_bits`), zero-extended for
//!   `F32`. Round-trips are therefore **bit-exact**: NaN payloads, `-0.0` and
//!   subnormals survive.
//! * **`EntityId`** — the kernel's [`EntityId`] index, zero-extended.
//!
//! Extractors are CHECKED (`Option`, CORE D10's shape): a kind mismatch is `None`,
//! never a reinterpretation — and a non-canonical `bits` (constructible, since the
//! fields are `pub` per the taxonomy sketch) is also `None` rather than a silently
//! truncated wrong value.

use boyko_ecs::ecs::identifiers::primitives::EntityId;

/// Discriminates the payload encoding of a [`Scalar`] (CORE C1, taxonomy §3).
///
/// One variant per primitive the v1 value model can carry. The variant order is
/// load-bearing for nothing (no index arithmetic keys on it); the `#[repr(u8)]`
/// keeps the tag one byte so `Scalar` hits its 16-byte target.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarKind {
    /// `bool`, stored as `0`/`1`.
    Bool,
    /// `u8`, zero-extended.
    U8,
    /// `u16`, zero-extended.
    U16,
    /// `u32`, zero-extended.
    U32,
    /// `u64`, as-is.
    U64,
    /// `i8`, sign-extended (see the module header's sign rule).
    I8,
    /// `i16`, sign-extended.
    I16,
    /// `i32`, sign-extended.
    I32,
    /// `i64`, reinterpreted two's-complement.
    I64,
    /// `f32`, by IEEE-754 bit pattern (zero-extended).
    F32,
    /// `f64`, by IEEE-754 bit pattern.
    F64,
    /// The kernel's [`EntityId`] slot index, zero-extended.
    EntityId,
}

/// The 16-byte-target `#[repr(C)]` POD tagged union (CORE C1): a kind tag plus a
/// 64-bit payload cell. `Copy`, no heap, no drop.
///
/// Layout is pinned at the bottom of this file at the values MEASURED on the
/// mandated toolchain (C1 gate 1): size 16, align 8.
///
/// Fields are `pub` per the taxonomy sketch (§3); the checked extractors below
/// are the supported read path, and they answer a hand-built non-canonical
/// payload with `None` rather than a truncated value.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scalar {
    /// Which encoding `bits` holds.
    pub kind: ScalarKind,
    /// The payload, encoded per the module header's table.
    pub bits: u64,
}

/// THE sign-extension rule, store side (CORE C1 Lands: "written once").
///
/// A signed payload is widened to `i64` at the constructor (Rust's `as i64` on a
/// narrower signed type sign-extends) and reinterpreted as `u64` two's-complement
/// here. [`ix_from_bits`] is its inverse; nothing else touches signed payloads.
#[inline]
const fn ix_to_bits(v: i64) -> u64 {
    v as u64
}

/// THE sign-extension rule, read side: the whole `bits` reinterpreted as `i64`
/// (two's complement), then narrowed LOSSLESSLY by the per-kind extractor.
///
/// A zero-extending reader (`try_from` on the raw `u64`) misreads every negative
/// payload — C1's second RED mutation exists to show exactly that, and the
/// round-trip tests' names say their ranges include negatives for this reason.
#[inline]
const fn ix_from_bits(bits: u64) -> i64 {
    bits as i64
}

impl Scalar {
    /// Returns `bits` iff the tag matches `kind` — the one kind guard every
    /// checked extractor goes through.
    #[inline]
    fn payload(self, kind: ScalarKind) -> Option<u64> {
        if self.kind == kind { Some(self.bits) } else { None }
    }

    /// Checked extractor: `Some(bool)` iff `kind == Bool` and the payload is a
    /// canonical `0`/`1`.
    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        match self.payload(ScalarKind::Bool)? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// Checked extractor: `Some(u8)` iff `kind == U8` and the payload fits.
    #[inline]
    pub fn as_u8(self) -> Option<u8> {
        u8::try_from(self.payload(ScalarKind::U8)?).ok()
    }

    /// Checked extractor: `Some(u16)` iff `kind == U16` and the payload fits.
    #[inline]
    pub fn as_u16(self) -> Option<u16> {
        u16::try_from(self.payload(ScalarKind::U16)?).ok()
    }

    /// Checked extractor: `Some(u32)` iff `kind == U32` and the payload fits.
    #[inline]
    pub fn as_u32(self) -> Option<u32> {
        u32::try_from(self.payload(ScalarKind::U32)?).ok()
    }

    /// Checked extractor: `Some(u64)` iff `kind == U64`.
    #[inline]
    pub fn as_u64(self) -> Option<u64> {
        self.payload(ScalarKind::U64)
    }

    /// Checked extractor: `Some(i8)` iff `kind == I8` and the sign-extended
    /// payload fits (module header's sign rule; negatives are the load-bearing
    /// half of its range).
    #[inline]
    pub fn as_i8(self) -> Option<i8> {
        i8::try_from(ix_from_bits(self.payload(ScalarKind::I8)?)).ok()
    }

    /// Checked extractor: `Some(i16)` iff `kind == I16` and the sign-extended
    /// payload fits.
    #[inline]
    pub fn as_i16(self) -> Option<i16> {
        i16::try_from(ix_from_bits(self.payload(ScalarKind::I16)?)).ok()
    }

    /// Checked extractor: `Some(i32)` iff `kind == I32` and the sign-extended
    /// payload fits.
    #[inline]
    pub fn as_i32(self) -> Option<i32> {
        i32::try_from(ix_from_bits(self.payload(ScalarKind::I32)?)).ok()
    }

    /// Checked extractor: `Some(i64)` iff `kind == I64`.
    #[inline]
    pub fn as_i64(self) -> Option<i64> {
        Some(ix_from_bits(self.payload(ScalarKind::I64)?))
    }

    /// Checked extractor: `Some(f32)` iff `kind == F32` and the payload is a
    /// canonical zero-extended 32-bit pattern. Bit-exact: NaN payloads, `-0.0`
    /// and subnormals survive the trip.
    #[inline]
    pub fn as_f32(self) -> Option<f32> {
        u32::try_from(self.payload(ScalarKind::F32)?).ok().map(f32::from_bits)
    }

    /// Checked extractor: `Some(f64)` iff `kind == F64`. Bit-exact.
    #[inline]
    pub fn as_f64(self) -> Option<f64> {
        self.payload(ScalarKind::F64).map(f64::from_bits)
    }

    /// Checked extractor: `Some(EntityId)` iff `kind == EntityId` and the payload
    /// fits the platform's `usize` (always, on the 64-bit targets this engine
    /// ships on; checked rather than truncated on 32-bit ones).
    #[inline]
    pub fn as_entity_id(self) -> Option<EntityId> {
        usize::try_from(self.payload(ScalarKind::EntityId)?).ok().map(EntityId)
    }
}

impl From<bool> for Scalar {
    #[inline]
    fn from(v: bool) -> Self {
        Self { kind: ScalarKind::Bool, bits: u64::from(v) }
    }
}

impl From<u8> for Scalar {
    #[inline]
    fn from(v: u8) -> Self {
        Self { kind: ScalarKind::U8, bits: u64::from(v) }
    }
}

impl From<u16> for Scalar {
    #[inline]
    fn from(v: u16) -> Self {
        Self { kind: ScalarKind::U16, bits: u64::from(v) }
    }
}

impl From<u32> for Scalar {
    #[inline]
    fn from(v: u32) -> Self {
        Self { kind: ScalarKind::U32, bits: u64::from(v) }
    }
}

impl From<u64> for Scalar {
    #[inline]
    fn from(v: u64) -> Self {
        Self { kind: ScalarKind::U64, bits: v }
    }
}

impl From<i8> for Scalar {
    #[inline]
    fn from(v: i8) -> Self {
        Self { kind: ScalarKind::I8, bits: ix_to_bits(i64::from(v)) }
    }
}

impl From<i16> for Scalar {
    #[inline]
    fn from(v: i16) -> Self {
        Self { kind: ScalarKind::I16, bits: ix_to_bits(i64::from(v)) }
    }
}

impl From<i32> for Scalar {
    #[inline]
    fn from(v: i32) -> Self {
        Self { kind: ScalarKind::I32, bits: ix_to_bits(i64::from(v)) }
    }
}

impl From<i64> for Scalar {
    #[inline]
    fn from(v: i64) -> Self {
        Self { kind: ScalarKind::I64, bits: ix_to_bits(v) }
    }
}

impl From<f32> for Scalar {
    #[inline]
    fn from(v: f32) -> Self {
        Self { kind: ScalarKind::F32, bits: u64::from(v.to_bits()) }
    }
}

impl From<f64> for Scalar {
    #[inline]
    fn from(v: f64) -> Self {
        Self { kind: ScalarKind::F64, bits: v.to_bits() }
    }
}

impl From<EntityId> for Scalar {
    #[inline]
    fn from(v: EntityId) -> Self {
        Self { kind: ScalarKind::EntityId, bits: v.0 as u64 }
    }
}

// C1 gate 1 — the layout pin, at the values MEASURED on the mandated toolchain
// (stable-x86_64-pc-windows-gnu 1.97.1) and printed by
// tests/c1_scalar.rs::scalar_layout_measured_and_pinned. The 16-byte target
// (§3) is met: 1-byte tag + 7 padding + 8-byte payload, align 8.
const _: () = assert!(
    size_of::<Scalar>() == 16 && align_of::<Scalar>() == 8,
    "Scalar layout moved off its measured pin (16 bytes, align 8) -- re-measure and re-pin \
     deliberately, in the same change that moved it"
);
