//! The record header, the payload encoding, and `dsp!`.
//!
//! # Deferred formatting
//!
//! A record on the ring is a 20-byte header plus a **TAGGED POD payload**. Nothing is formatted on
//! the emitting thread: the header carries a `&'static LogSite` with the format literal, each
//! value carries a one-byte [`ValueTag`], and the sink walks the pair later, off the hot path.
//! That is what keeps an enabled site in the tens of nanoseconds instead of the hundreds.
//!
//! # Why the payload is tagged, and what it replaces *(measured at L6)*
//!
//! The predecessor design gave [`LogSite`](crate::site::LogSite) a `decode` function pointer
//! "monomorphised per argument-tuple type". **It was never filled with one and could not be**, and
//! the field's own doc comment said why: the site is a `static` and Rust has no generic statics,
//! so the tuple type cannot be named at the initialiser. Every site in the workspace therefore
//! carried the same placeholder, `decode_opaque`, which reported a byte count — and **no drain
//! path ever called it**. The sink printed the *format literal*, placeholders and all: a `warn!`
//! carrying a set name rendered `ordering references set '{}' which has no members`.
//!
//! That is not a gap this rung could route around, because L6's entire content is call sites whose
//! value **is** their arguments. Two facts decided the replacement rather than a preference:
//!
//! 1. A per-site function pointer cannot be named at a `static`, so the only way to install one is
//!    to publish it at run time into a mutable site — atomics and a transmuted `fn` pointer on the
//!    emission path, to carry data the emitting thread is specified never to touch.
//! 2. A per-site function pointer **cannot decode a file**. `logdec` (L13b) reads a `.blog` in
//!    another process, where a pointer into the producing binary means nothing. That rung would
//!    have had to introduce tags or a shape table regardless, so the fn-pointer design was never
//!    going to survive it.
//!
//! One tag byte per value costs a store adjacent to the value it describes, makes the payload
//! self-describing for **both** consumers, and leaves the site a pure immutable `'static`. Gate
//! **G5** — the distinct-`decode`-symbol census — loses its subject with the field and is struck
//! rather than restated; there is exactly one walker, by construction, forever.
//!
//! # Length is a RUNTIME quantity
//!
//! [`LogValue`] has `encoded_len(&self)`, not a `const ENCODED_LEN`. A `&str`'s length is not a
//! constant, and the space reservation that the entire overflow protocol rests on was undefined
//! for exactly the implementors that produce large records. For an all-POD tuple every
//! `encoded_len` **is** a constant and the compiler folds the sum to an immediate, so the const
//! case loses nothing.

use core::fmt::Write as _;

/// Total bytes of a record header. The walk step for both producer and consumer.
pub(crate) const HEADER_BYTES: usize = 20;

/// Hard cap on one record, checked at **run time** against the computed length.
///
/// Not a `debug_assert!` of unreachability: five 256-byte strings exceed a kilobyte and the
/// argument list may hold twelve of them, so "unreachable" described a debug-build panic
/// reachable from safe user code. An over-cap record is **dropped** with [`flags::TOO_LARGE`] and
/// counted, in every profile.
pub const MAX_RECORD_BYTES: usize = 2048;

/// Cap on one inline `&str`. Longer strings are truncated **at a character boundary** and the
/// record is flagged.
pub const MAX_STR_BYTES: usize = 256;

/// Stack budget for one **rendered** line: the level, the code, `file:line` and the interleaved
/// message.
///
/// Derived rather than picked: a payload is at most [`MAX_RECORD_BYTES`] − `HEADER_BYTES` bytes,
/// and rendering a string costs at most its own bytes while rendering a number costs at most 20
/// characters for 9 payload bytes — so 2 KiB of payload cannot exceed 2 KiB of text by more than
/// the numeric ratio, and the site's own `file` and `fmt` add at most one path and one literal.
/// The value is generous on purpose: overflow **truncates at a character boundary** rather than
/// corrupting, but a truncated diagnostic is still a diagnostic that lost its tail.
pub const MAX_RENDERED_BYTES: usize = 3072;

/// Bytes a **dynamic** record carries ahead of its values: the target id, little-endian *(L10)*.
///
/// A `dyn_*!` site has no compile-time target — the id is an argument, and the same site may be
/// reached with a different one on every call — so the id has to travel with the record. It goes
/// here rather than in [`RecordHeader`] because the header is exactly 20 bytes and pinned as such,
/// and adding a field would charge **every** record two bytes to describe a case almost none of
/// them are in. Static records carry zero of these.
pub const DYN_PREFIX_BYTES: usize = 2;

/// Split a payload into its dynamic-target prefix and the values behind it.
///
/// `site_target` is the emitting site's own `target` field, and it is the whole discriminant:
/// `Some` means the site named its target at compile time and the payload is values from byte
/// zero; `None` means the first [`DYN_PREFIX_BYTES`] are the id. The caller passes the field
/// rather than a flag so that the writer and the reader cannot disagree — there is one fact and
/// one place it lives.
///
/// A prefix that is short or names an id outside the dynamic band yields `(None, values)`: the
/// record still renders, and only its *attribution* is lost. That direction is deliberate. A torn
/// or truncated prefix is a symptom of something already wrong, and the reader's best outcome is
/// the message it was carrying — not a panic inside the drain, and not a fabricated `TargetId`,
/// which is the hazard the closed constructor set exists to prevent.
#[must_use]
pub fn split_dynamic_target(
    site_target: Option<crate::target::TargetId>,
    payload: &[u8],
) -> (Option<crate::target::TargetId>, &[u8]) {
    if let Some(t) = site_target {
        return (Some(t), payload);
    }
    if payload.len() < DYN_PREFIX_BYTES {
        return (None, payload);
    }
    let (head, rest) = payload.split_at(DYN_PREFIX_BYTES);
    let raw = u16::from_le_bytes([head[0], head[1]]);
    (crate::target::TargetId::from_dynamic_raw(raw), rest)
}

/// Record header flags.
pub mod flags {
    /// At least one `&str` argument was truncated at [`super::MAX_STR_BYTES`].
    pub const STR_TRUNCATED: u8 = 1 << 0;
    /// A record immediately after this one was suppressed by a rate policy.
    pub const SUPPRESSED_FOLLOWS: u8 = 1 << 1;
    /// The record exceeded [`super::MAX_RECORD_BYTES`] and was dropped. Carried so a sink that
    /// sees a truncated stream can say why.
    pub const TOO_LARGE: u8 = 1 << 2;
}

/// 20 B, **packed**: the ring is byte-oriented and records are never aligned, so alignment
/// padding would be pure waste.
///
/// `code`, `level` and `lane` are deliberately **not** duplicated here — the sink holds the site
/// pointer and knows which lane it is draining, so carrying them again bought three bytes of
/// nothing.
///
/// `site` is a real pointer field so **provenance round-trips**; a null `site` is the PAD
/// sentinel that fills a ring tail too short for a record.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub(crate) struct RecordHeader {
    /// The call site. Null means PAD.
    pub site: *const crate::site::LogSite,
    /// Raw `boyko_diag::clock::ticks()`. Scaled by the sink, never on this thread.
    pub tsc: u64,
    /// **Total** record bytes including this header — the walk step.
    pub len: u16,
    /// See [`flags`].
    pub flags: u8,
    /// Low 8 bits of `boyko_diag::clock::clock_epoch()`.
    ///
    /// Spends what used to be padding, so the header does not grow. Eight bits suffice: the sink
    /// is at most one park interval behind the producer, so at most one epoch boundary can lie
    /// between them and the full counter is reconstructed against the live epoch. Rendering it
    /// beside the timestamp is what makes a record that straddles a suspend **legible** rather
    /// than merely wrong.
    pub clock_epoch_lo: u8,
}

const _: () = assert!(core::mem::size_of::<RecordHeader>() == HEADER_BYTES);
// A PAD record must fit in a header alone, or the wrap rule cannot express "fill the tail".
const _: () = assert!(HEADER_BYTES <= MAX_RECORD_BYTES);
// One header plus one maximal string must fit, or a single-argument site could never emit. The
// `3` is the string's own envelope: one tag byte plus the `u16` length.
const _: () = assert!(HEADER_BYTES + 3 + MAX_STR_BYTES <= MAX_RECORD_BYTES);

/// What a payload value is, so **one** non-generic walker can render it.
///
/// Written by [`LogValue::encode`] immediately before the value's bytes and read by
/// [`render_payload`]. The discriminants are a **wire format**: `logdec` (L13b) decodes a file
/// written by another process, so a value may be renumbered only by a `schema_version` bump.
///
/// `Usize`/`Isize` are distinct from `U64`/`I64` on purpose. Their width is
/// `size_of::<usize>()`, which is the producing target's, and collapsing them into the 64-bit
/// tags would let a 32-bit producer's file decode as 64-bit values that were never written. The
/// in-process sink cannot tell the difference; a cross-process reader can, and that is the reader
/// this distinction exists for.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueTag {
    /// `u8`.
    U8 = 1,
    /// `u16`.
    U16 = 2,
    /// `u32`.
    U32 = 3,
    /// `u64`.
    U64 = 4,
    /// `usize`, at the producing target's width.
    Usize = 5,
    /// `i8`.
    I8 = 6,
    /// `i16`.
    I16 = 7,
    /// `i32`.
    I32 = 8,
    /// `i64`.
    I64 = 9,
    /// `isize`, at the producing target's width.
    Isize = 10,
    /// `f32`.
    F32 = 11,
    /// `f64`.
    F64 = 12,
    /// `bool`, one byte.
    Bool = 13,
    /// `char`, as its `u32` scalar value.
    Char = 14,
    /// `&str`: a `u16` byte length followed by that many UTF-8 bytes.
    Str = 15,
}

impl ValueTag {
    /// The tag for a raw byte, or `None` when it names no value.
    ///
    /// A `None` here is a **corrupt or truncated payload**, which the walker reports rather than
    /// guesses at: a tag it does not recognise means it no longer knows where the next value
    /// starts, so continuing would render arbitrary bytes as if they were data.
    #[must_use]
    pub const fn from_raw(b: u8) -> Option<ValueTag> {
        Some(match b {
            1 => ValueTag::U8,
            2 => ValueTag::U16,
            3 => ValueTag::U32,
            4 => ValueTag::U64,
            5 => ValueTag::Usize,
            6 => ValueTag::I8,
            7 => ValueTag::I16,
            8 => ValueTag::I32,
            9 => ValueTag::I64,
            10 => ValueTag::Isize,
            11 => ValueTag::F32,
            12 => ValueTag::F64,
            13 => ValueTag::Bool,
            14 => ValueTag::Char,
            15 => ValueTag::Str,
            _ => return None,
        })
    }
}

/// One value that can travel in a record payload.
///
/// # Safety
///
/// [`encode`](LogValue::encode) writes exactly [`encoded_len`](LogValue::encoded_len) bytes to
/// `dst` and returns that count, **the first of which is `TAG`**. An implementation that writes
/// more overruns the ring; one that writes fewer leaves the payload misaligned with the header's
/// `len` and every subsequent field decodes from the wrong offset; one whose first byte is not
/// `TAG` desynchronises the walker for the rest of the record.
pub unsafe trait LogValue {
    /// This value's wire tag, written as the first payload byte.
    const TAG: ValueTag;

    /// Upper bound on [`encoded_len`](LogValue::encoded_len), for the argument-list cap.
    const MAX_ENCODED_LEN: usize;

    /// Bytes this value will occupy. A constant for every POD implementor, so a POD tuple's sum
    /// folds to an immediate.
    fn encoded_len(&self) -> usize;

    /// Flags this value contributes — currently only [`flags::STR_TRUNCATED`]. Constant `0` for
    /// every POD implementor.
    #[inline]
    fn value_flags(&self) -> u8 {
        0
    }

    /// Write the value.
    ///
    /// # Safety
    ///
    /// `dst` must be valid for writes of [`encoded_len`](LogValue::encoded_len) bytes.
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}

macro_rules! impl_pod_value {
    ($($t:ty => $tag:ident),* $(,)?) => {$(
        // SAFETY: `encoded_len` is `1 + size_of::<$t>()` and `encode` writes one tag byte followed
        //   by exactly that many bytes with one `copy_nonoverlapping` from the value's
        //   little-endian byte array. The two cannot disagree because both are derived from the
        //   same `size_of`, and the first byte written is `TAG`.
        unsafe impl LogValue for $t {
            const TAG: ValueTag = ValueTag::$tag;
            const MAX_ENCODED_LEN: usize = 1 + core::mem::size_of::<$t>();

            #[inline]
            fn encoded_len(&self) -> usize {
                1 + core::mem::size_of::<$t>()
            }

            #[inline]
            unsafe fn encode(&self, dst: *mut u8) -> usize {
                let bytes = self.to_le_bytes();
                // SAFETY: the caller guarantees `dst` is valid for `encoded_len()` writes, which
                //   is exactly `1 + bytes.len()`. The source is a local array, so the ranges
                //   cannot overlap.
                unsafe {
                    dst.write(Self::TAG as u8);
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(1), bytes.len());
                }
                1 + bytes.len()
            }
        }
    )*};
}

impl_pod_value!(
    u8 => U8, u16 => U16, u32 => U32, u64 => U64, usize => Usize,
    i8 => I8, i16 => I16, i32 => I32, i64 => I64, isize => Isize,
    f32 => F32, f64 => F64,
);

// SAFETY: tag plus one byte, written directly; `encoded_len` is 2 and `encode` writes 2, tag first.
unsafe impl LogValue for bool {
    const TAG: ValueTag = ValueTag::Bool;
    const MAX_ENCODED_LEN: usize = 2;

    #[inline]
    fn encoded_len(&self) -> usize {
        2
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        // SAFETY: the caller guarantees `dst` is valid for two writes.
        unsafe {
            dst.write(Self::TAG as u8);
            dst.add(1).write(u8::from(*self));
        }
        2
    }
}

// SAFETY: tag plus its `u32` scalar value; `encoded_len` and `encode` both use 5, tag first.
unsafe impl LogValue for char {
    const TAG: ValueTag = ValueTag::Char;
    const MAX_ENCODED_LEN: usize = 5;

    #[inline]
    fn encoded_len(&self) -> usize {
        5
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        let bytes = (*self as u32).to_le_bytes();
        // SAFETY: the caller guarantees `dst` is valid for five writes; source is a local array.
        unsafe {
            dst.write(Self::TAG as u8);
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(1), 4);
        }
        5
    }
}

/// Bytes of `s` that fit in `MAX_STR_BYTES` **without splitting a character**.
///
/// Truncating mid-codepoint would hand the sink invalid UTF-8, which it would then either render
/// as replacement characters or refuse — a corruption introduced by the logger itself, on the
/// path that exists to report corruption. `str::floor_char_boundary` is unstable, so the walk is
/// here: continuation bytes are `0b10xx_xxxx`, and there are at most three of them.
#[inline]
fn str_fit(s: &str) -> usize {
    if s.len() <= MAX_STR_BYTES {
        return s.len();
    }
    let b = s.as_bytes();
    let mut i = MAX_STR_BYTES;
    while i > 0 && (b[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

// SAFETY: `encoded_len` is `3 + str_fit(self)` and `encode` writes the tag, a `u16` length and
//   exactly `str_fit(self)` bytes -- the same quantity, computed by the same function.
unsafe impl LogValue for &str {
    const TAG: ValueTag = ValueTag::Str;
    const MAX_ENCODED_LEN: usize = 3 + MAX_STR_BYTES;

    #[inline]
    fn encoded_len(&self) -> usize {
        3 + str_fit(self)
    }

    #[inline]
    fn value_flags(&self) -> u8 {
        if self.len() > MAX_STR_BYTES { flags::STR_TRUNCATED } else { 0 }
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        let n = str_fit(self);
        let len_bytes = (n as u16).to_le_bytes();
        // SAFETY: the caller guarantees `dst` is valid for `encoded_len()` == `3 + n` writes.
        //   Every write stays inside that span, and neither source can alias the ring.
        unsafe {
            dst.write(Self::TAG as u8);
            core::ptr::copy_nonoverlapping(len_bytes.as_ptr(), dst.add(1), 2);
            core::ptr::copy_nonoverlapping(self.as_ptr(), dst.add(3), n);
        }
        3 + n
    }
}

/// An argument tuple.
///
/// # Safety
///
/// As [`LogValue`]: `encode` writes exactly `encoded_len` bytes and returns that count.
pub unsafe trait LogArgs {
    /// Upper bound on the payload, used by the `MAX_RECORD_BYTES` check's constant folding.
    const MAX_ENCODED_LEN: usize;

    /// Payload bytes.
    fn encoded_len(&self) -> usize;

    /// Flags contributed by the arguments. Constant `0` for an all-POD tuple.
    fn args_flags(&self) -> u8;

    /// Write the payload.
    ///
    /// # Safety
    ///
    /// `dst` must be valid for writes of [`encoded_len`](LogArgs::encoded_len) bytes.
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}

macro_rules! impl_args_tuple {
    ($($name:ident : $idx:tt),*) => {
        // SAFETY: `encoded_len` is the sum of the fields' `encoded_len`, and `encode` walks the
        //   fields in the same order advancing the cursor by each field's own return value. The
        //   two therefore agree by construction for any correct `LogValue` implementation, which
        //   is that trait's own safety contract.
        unsafe impl<$($name: LogValue),*> LogArgs for ($($name,)*) {
            const MAX_ENCODED_LEN: usize = 0 $(+ $name::MAX_ENCODED_LEN)*;

            #[inline]
            fn encoded_len(&self) -> usize {
                0 $(+ self.$idx.encoded_len())*
            }

            #[inline]
            fn args_flags(&self) -> u8 {
                0 $(| self.$idx.value_flags())*
            }

            #[inline]
            unsafe fn encode(&self, dst: *mut u8) -> usize {
                let mut off = 0usize;
                $(
                    // SAFETY: the caller guarantees `dst` is valid for `encoded_len()` bytes,
                    //   which is the sum of the fields' lengths; `off` is the sum of the lengths
                    //   already written, so `dst.add(off)` is in range with at least this field's
                    //   length remaining.
                    off += unsafe { self.$idx.encode(dst.add(off)) };
                )*
                off
            }
        }
    };
}

// The empty tuple is written out rather than generated: the macro's body would produce a `mut`
// cursor that is never advanced and an unused `dst`, i.e. two warnings on the one arity that
// cannot have a field. Silencing them with `#[allow]` inside the macro would silence them for
// EVERY arity, including one where an unused `dst` would be a real defect.
//
// SAFETY: `encoded_len` is 0 and `encode` writes nothing, so the two agree.
unsafe impl LogArgs for () {
    const MAX_ENCODED_LEN: usize = 0;

    #[inline]
    fn encoded_len(&self) -> usize {
        0
    }

    #[inline]
    fn args_flags(&self) -> u8 {
        0
    }

    #[inline]
    unsafe fn encode(&self, _dst: *mut u8) -> usize {
        0
    }
}

impl_args_tuple!(A0: 0);
impl_args_tuple!(A0: 0, A1: 1);
impl_args_tuple!(A0: 0, A1: 1, A2: 2);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6, A7: 7);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6, A7: 7, A8: 8);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6, A7: 7, A8: 8, A9: 9);
impl_args_tuple!(A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6, A7: 7, A8: 8, A9: 9, A10: 10);
impl_args_tuple!(
    A0: 0, A1: 1, A2: 2, A3: 3, A4: 4, A5: 5, A6: 6, A7: 7, A8: 8, A9: 9, A10: 10, A11: 11
);

// ─────────────────────────────── the one payload walker ───────────────────────────────

/// Take `n` bytes, or `None` when the payload ends first.
///
/// A failure **ends the walk**: `cur` jumps to the end rather than staying put. A value whose
/// bytes are missing means the walker no longer knows where the next value begins, so resuming
/// inside the remains of this one would render the tail of a truncated integer as a fresh value —
/// the same "invent data rather than admit the loss" failure a corrupt tag has.
#[inline]
fn take<'a>(p: &'a [u8], cur: &mut usize, n: usize) -> Option<&'a [u8]> {
    match cur.checked_add(n) {
        Some(end) if end <= p.len() => {
            let out = &p[*cur..end];
            *cur = end;
            Some(out)
        }
        _ => {
            *cur = p.len();
            None
        }
    }
}

/// Render one tagged value, advancing `cur`. `false` means the payload was exhausted **before**
/// this value, which is the ordinary "no more arguments" answer rather than an error.
fn write_value(p: &[u8], cur: &mut usize, f: &mut crate::site::LogFormatter) -> bool {
    let Some(&raw) = p.get(*cur) else { return false };
    *cur += 1;
    let Some(tag) = ValueTag::from_raw(raw) else {
        // The walker no longer knows where the next value starts, so it stops rather than
        // rendering arbitrary bytes as if they were data. A diagnostic that invents values is
        // worse than one that says it lost the thread.
        f.write_str("<corrupt tag>");
        *cur = p.len();
        return true;
    };

    macro_rules! num {
        ($t:ty, $n:expr) => {
            match take(p, cur, $n) {
                Some(x) => {
                    let mut a = [0u8; $n];
                    a.copy_from_slice(x);
                    f.write_fmt(format_args!("{}", <$t>::from_le_bytes(a)));
                }
                None => f.write_str("<truncated>"),
            }
        };
    }

    match tag {
        ValueTag::U8 => num!(u8, 1),
        ValueTag::U16 => num!(u16, 2),
        ValueTag::U32 => num!(u32, 4),
        ValueTag::U64 => num!(u64, 8),
        ValueTag::I8 => num!(i8, 1),
        ValueTag::I16 => num!(i16, 2),
        ValueTag::I32 => num!(i32, 4),
        ValueTag::I64 => num!(i64, 8),
        ValueTag::F32 => num!(f32, 4),
        ValueTag::F64 => num!(f64, 8),
        ValueTag::Usize => {
            const N: usize = core::mem::size_of::<usize>();
            num!(usize, N);
        }
        ValueTag::Isize => {
            const N: usize = core::mem::size_of::<isize>();
            num!(isize, N);
        }
        ValueTag::Bool => match take(p, cur, 1) {
            Some(x) => f.write_str(if x[0] == 0 { "false" } else { "true" }),
            None => f.write_str("<truncated>"),
        },
        ValueTag::Char => match take(p, cur, 4) {
            Some(x) => {
                let mut a = [0u8; 4];
                a.copy_from_slice(x);
                match char::from_u32(u32::from_le_bytes(a)) {
                    Some(c) => f.write_fmt(format_args!("{c}")),
                    None => f.write_str("<invalid char>"),
                }
            }
            None => f.write_str("<truncated>"),
        },
        ValueTag::Str => {
            let n = match take(p, cur, 2) {
                Some(x) => usize::from(u16::from_le_bytes([x[0], x[1]])),
                None => {
                    f.write_str("<truncated>");
                    return true;
                }
            };
            match take(p, cur, n) {
                // Checked, not `from_utf8_unchecked`: the producer cuts on a character boundary,
                // but this walker also runs over bytes read back from a ring that a defect could
                // have torn, and a diagnostic path must not be the one that hands the sink
                // invalid UTF-8.
                Some(x) => match core::str::from_utf8(x) {
                    Ok(s) => f.write_str(s),
                    Err(_) => f.write_str("<invalid utf-8>"),
                },
                None => f.write_str("<truncated>"),
            }
        }
    }
    true
}

/// Interleave a record's tagged payload with its site's format literal.
///
/// **The one walker.** There is no per-site decoder and no monomorphisation: the payload says what
/// it holds, so this function serves every site in the workspace and every `.blog` `logdec` will
/// ever read.
///
/// What it supports, stated rather than implied: `{}` consumes the next value; `{{` and `}}` are
/// literal braces; **any other `{…}` group also consumes the next value and its format spec is
/// ignored**, so `{:?}` and `{:.2}` render as `{}` would. That is a real limitation and it is
/// written here instead of being discovered at a call site — the spec lives in the caller's source
/// and the values on the wire are already rendered-agnostic bytes.
///
/// Neither disagreement is silent. A placeholder with no value left renders `{missing}`; values
/// left over when the literal runs out are appended in `[+ …]`, so a mis-shaped call site is
/// visible in its own output rather than losing data.
pub fn render_payload(payload: &[u8], fmt: &str, f: &mut crate::site::LogFormatter) {
    let b = fmt.as_bytes();
    let mut cur = 0usize;
    let mut i = 0usize;
    let mut text_start = 0usize;

    while i < b.len() {
        // Scanning bytes is safe against multi-byte UTF-8: every continuation byte is >= 0x80, so
        // neither brace can be matched inside a character, and every slice boundary taken below
        // lands on one.
        let doubled = i + 1 < b.len() && b[i + 1] == b[i];
        match b[i] {
            b'{' | b'}' if doubled => {
                f.write_str(&fmt[text_start..=i]);
                i += 2;
                text_start = i;
            }
            b'{' => {
                f.write_str(&fmt[text_start..i]);
                let mut j = i + 1;
                while j < b.len() && b[j] != b'}' {
                    j += 1;
                }
                i = if j < b.len() { j + 1 } else { b.len() };
                text_start = i;
                if !write_value(payload, &mut cur, f) {
                    f.write_str("{missing}");
                }
            }
            _ => i += 1,
        }
    }
    f.write_str(&fmt[text_start..]);

    if cur < payload.len() {
        f.write_str(" [+");
        let mut first = true;
        while cur < payload.len() {
            if !first {
                f.write_str(",");
            }
            first = false;
            f.write_str(" ");
            if !write_value(payload, &mut cur, f) {
                break;
            }
        }
        f.write_str("]");
    }
}

/// A stack buffer that renders a [`Display`](core::fmt::Display) and yields `&str`.
///
/// **It exists to keep user code out of the open-record window.** The predecessor design ran
/// `Display::fmt` while the ring tail held a partially written record and `write` had not yet
/// advanced. A nested emit from that `Display` — or from anything it called — would overwrite the
/// outer record and publish one `len` for two interleaved payloads, to be decoded by the wrong
/// function pointer. An unwind through the same window left the ring in the same state. The
/// "two producers on one lane is unrepresentable" argument is about *threads* and does not touch
/// this at all.
///
/// [`dsp!`](crate::dsp) expands in **argument position**, so it runs to completion before
/// `emit_impl` is entered and before any lane state is touched. Overflow truncates; it can never
/// overrun a ring.
pub struct DspBuf<const N: usize> {
    buf: [u8; N],
    len: u16,
    truncated: bool,
}

impl<const N: usize> DspBuf<N> {
    /// An empty buffer.
    ///
    /// Exists so a drain loop can hoist **one** buffer out of the per-record closure instead of
    /// zeroing `N` bytes of stack for every record it renders.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        DspBuf { buf: [0; N], len: 0, truncated: false }
    }

    /// Forget the contents, keeping the storage.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.truncated = false;
    }

    /// Render `v`.
    #[inline]
    #[must_use]
    pub fn render(v: &impl core::fmt::Display) -> Self {
        let mut me = Self::new();
        // `write!` cannot fail for this writer: `write_str` truncates instead of erroring, so a
        // formatting *error* here can only come from the user's own `Display` impl. Either way
        // the buffer holds whatever was rendered before the failure, which is what a diagnostic
        // wants — so the result is deliberately dropped rather than turned into a panic on the
        // logging path.
        let _ = write!(&mut me, "{v}");
        me
    }

    /// The rendered text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // SAFETY: `write_str` only ever appends whole `&str` prefixes cut at a character
        //   boundary (see the impl below), so `buf[..len]` is always valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len as usize]) }
    }

    /// Whether rendering hit the buffer's end.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

impl<const N: usize> Default for DspBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> core::fmt::Write for DspBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let free = N - self.len as usize;
        if free == 0 {
            self.truncated = !s.is_empty();
            return Ok(());
        }
        let mut take = s.len().min(free);
        if take < s.len() {
            self.truncated = true;
            // Cut at a character boundary; continuation bytes are `0b10xx_xxxx`.
            let b = s.as_bytes();
            while take > 0 && (b[take] & 0xC0) == 0x80 {
                take -= 1;
            }
        }
        let at = self.len as usize;
        self.buf[at..at + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take as u16;
        Ok(())
    }
}

/// Render a `Display` value into a caller-owned stack buffer and yield `&str`.
///
/// `dsp!(x)` uses a 256-byte buffer; `dsp!(x, 64)` picks the size. The expansion is a **by-value
/// temporary in argument position**, whose `as_str()` borrow lives to the end of the enclosing
/// statement by Rust's temporary-lifetime rules for call arguments. The obvious block form —
/// `{ let mut buf = …; &buf }` — returns a reference to a local and does not compile, which is
/// why the form is pinned here rather than left to the reader.
#[macro_export]
macro_rules! dsp {
    ($e:expr) => {
        $crate::DspBuf::<256>::render(&$e).as_str()
    };
    ($e:expr, $n:literal) => {
        $crate::DspBuf::<$n>::render(&$e).as_str()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tuple into a scratch buffer and return the bytes actually written.
    fn enc<A: LogArgs>(a: &A) -> Vec<u8> {
        let mut v = vec![0u8; a.encoded_len() + 16];
        // SAFETY: the buffer is longer than `encoded_len()`.
        let n = unsafe { a.encode(v.as_mut_ptr()) };
        assert_eq!(n, a.encoded_len(), "encode must write exactly encoded_len bytes");
        v.truncate(n);
        v
    }

    #[test]
    fn pod_values_encode_little_endian_at_their_declared_width() {
        // The tag byte comes FIRST, then the little-endian value. Written out rather than
        // computed, because the pair is a wire format `logdec` decodes without this source.
        assert_eq!(enc(&(0x1234_5678u32,)), vec![3, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(enc(&(true,)), vec![13, 1]);
        assert_eq!(enc(&(-1i16,)), vec![7, 0xFF, 0xFF]);
        let mut f32_row = vec![11u8];
        f32_row.extend_from_slice(&1.0f32.to_le_bytes());
        assert_eq!(enc(&(1.0f32,)), f32_row);
    }

    #[test]
    fn encoded_len_and_encode_agree_for_every_arity_used() {
        // The whole overflow protocol rests on these two never disagreeing, so the check is over
        // the shape of the tuple rather than over one example.
        let _ = enc(&());
        let _ = enc(&(1u8,));
        let _ = enc(&(1u8, 2u16, 3u32, 4u64));
        let _ = enc(&(1u8, "a", 2i32, "bb", 3.5f64, true, 'c', 7usize));
        let _ = enc(&(0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, 11u8));
    }

    #[test]
    fn a_string_carries_its_length_first() {
        let out = enc(&("hi",));
        assert_eq!(out, vec![ValueTag::Str as u8, 2, 0, b'h', b'i']);
    }

    #[test]
    fn an_over_cap_string_truncates_and_flags() {
        let s = "x".repeat(MAX_STR_BYTES + 50);
        let t = (s.as_str(),);
        assert_eq!(t.encoded_len(), 3 + MAX_STR_BYTES);
        assert_eq!(t.args_flags(), flags::STR_TRUNCATED);
        assert_eq!(enc(&t).len(), 3 + MAX_STR_BYTES);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // A 3-byte codepoint straddling the cap. Cutting mid-sequence would hand the sink invalid
        // UTF-8 -- corruption introduced by the logger, on the path that reports corruption.
        let mut s = "a".repeat(MAX_STR_BYTES - 1);
        s.push('€'); // 3 bytes, so bytes 255..258 -- the cap lands inside it
        let t = (s.as_str(),);
        let out = enc(&t);
        let payload = &out[3..];
        assert!(
            core::str::from_utf8(payload).is_ok(),
            "the encoded prefix must remain valid UTF-8"
        );
        assert_eq!(payload.len(), MAX_STR_BYTES - 1, "the whole codepoint must be dropped");
        assert_eq!(t.args_flags(), flags::STR_TRUNCATED);
    }

    /// Render `payload` against `fmt`, the way a sink does.
    fn rend(fmt: &str, payload: &[u8]) -> String {
        let mut out = String::new();
        {
            let mut f = crate::site::LogFormatter::new(&mut out);
            render_payload(payload, fmt, &mut f);
        }
        out
    }

    #[test]
    fn the_walker_interleaves_values_with_the_format_literal() {
        // THE PROPERTY THIS RUNG EXISTS FOR. Until L6 every sink printed `fmt` verbatim, so this
        // assertion's expected string was `"set '{}' has {} members"` -- the literal, with the
        // arguments transported across the ring and discarded at the far end.
        //
        // RED STATE: make `render_payload` write `fmt` and return. Every assertion below fails,
        // and so does `the_unlaned_renderer_carries_the_arguments_and_never_overflows` in `lane`.
        assert_eq!(
            rend("set '{}' has {} members", &enc(&("render", 0u32))),
            "set 'render' has 0 members"
        );
        assert_eq!(rend("no placeholders", &enc(&())), "no placeholders");
        assert_eq!(rend("{}", &enc(&(-7i64,))), "-7");
        assert_eq!(rend("{}", &enc(&(true,))), "true");
        assert_eq!(rend("{}", &enc(&('q',))), "q");
        // A format SPEC is ignored rather than honoured, which is a stated limitation: the spec
        // lives in the caller's source and the wire carries rendered-agnostic bytes.
        assert_eq!(rend("{:?} {:.3}", &enc(&(1u8, 2u8))), "1 2");
    }

    #[test]
    fn a_mis_shaped_call_site_is_visible_in_its_own_output() {
        // Neither disagreement may be silent: a diagnostic that quietly drops half its arguments
        // is the failure this rung was opened to fix, one level down.
        assert_eq!(rend("{} {}", &enc(&(1u8,))), "1 {missing}");
        assert_eq!(rend("just text", &enc(&(1u8, 2u8))), "just text [+ 1, 2]");
    }

    #[test]
    fn doubled_braces_are_literal_and_consume_no_value() {
        assert_eq!(rend("{{}} {}", &enc(&(5u8,))), "{} 5");
        assert_eq!(rend("{{{}}}", &enc(&(5u8,))), "{5}");
    }

    #[test]
    fn a_corrupt_tag_stops_the_walk_instead_of_inventing_values() {
        // The walker no longer knows where the next value starts, so continuing would render
        // arbitrary ring bytes as if they were data -- which is worse than saying it lost the
        // thread, because the reader cannot tell.
        assert_eq!(rend("{}", &[200u8, 1, 2, 3, 4]), "<corrupt tag>");
        // A tag with too few bytes behind it is a different failure and says so.
        assert_eq!(rend("{}", &[ValueTag::U64 as u8, 1, 2]), "<truncated>");
    }

    #[test]
    fn tags_are_a_wire_format_and_their_numbers_are_pinned() {
        // `logdec` (L13b) decodes a file written by a binary it was not compiled with, so these
        // discriminants may move only behind a `schema_version` bump. Pinned as literals, because
        // deriving them from the enum would agree with any renumbering.
        for (tag, raw) in [
            (ValueTag::U8, 1u8),
            (ValueTag::U16, 2),
            (ValueTag::U32, 3),
            (ValueTag::U64, 4),
            (ValueTag::Usize, 5),
            (ValueTag::I8, 6),
            (ValueTag::I16, 7),
            (ValueTag::I32, 8),
            (ValueTag::I64, 9),
            (ValueTag::Isize, 10),
            (ValueTag::F32, 11),
            (ValueTag::F64, 12),
            (ValueTag::Bool, 13),
            (ValueTag::Char, 14),
            (ValueTag::Str, 15),
        ] {
            assert_eq!(tag as u8, raw, "{tag:?} moved on the wire");
            assert_eq!(ValueTag::from_raw(raw), Some(tag));
        }
        assert_eq!(ValueTag::from_raw(0), None, "0 is not a tag, so a zeroed byte is corruption");
        assert_eq!(ValueTag::from_raw(16), None);
    }

    #[test]
    fn every_value_type_declares_the_tag_it_writes() {
        // The trait's safety contract says the first byte written IS `TAG`; a type that declared
        // one and wrote another would desynchronise the walker for the rest of the record, and
        // nothing else in the crate compares the two.
        macro_rules! check {
            ($v:expr, $t:ty) => {
                assert_eq!(
                    enc(&($v,))[0],
                    <$t as LogValue>::TAG as u8,
                    "{} writes a tag it does not declare",
                    stringify!($t)
                );
            };
        }
        check!(1u8, u8);
        check!(1u16, u16);
        check!(1u32, u32);
        check!(1u64, u64);
        check!(1usize, usize);
        check!(1i8, i8);
        check!(1i16, i16);
        check!(1i32, i32);
        check!(1i64, i64);
        check!(1isize, isize);
        check!(1.0f32, f32);
        check!(1.0f64, f64);
        check!(true, bool);
        check!('c', char);
        check!("s", &str);
    }

    #[test]
    fn an_exactly_capped_string_is_not_flagged() {
        // The boundary case, because `>` and `>=` are one keystroke apart and both compile.
        let s = "y".repeat(MAX_STR_BYTES);
        let t = (s.as_str(),);
        assert_eq!(t.args_flags(), 0);
        assert_eq!(t.encoded_len(), 3 + MAX_STR_BYTES);
    }

    #[test]
    fn a_pod_tuple_contributes_no_flags() {
        assert_eq!((1u32, 2i64, 3.0f32, false, 'z').args_flags(), 0);
    }

    #[test]
    fn dsp_renders_and_truncates_at_a_character_boundary() {
        assert_eq!(DspBuf::<32>::render(&42u32).as_str(), "42");
        assert!(!DspBuf::<32>::render(&42u32).is_truncated());

        let long = "€".repeat(20); // 60 bytes
        let b = DspBuf::<10>::render(&long);
        assert!(b.is_truncated());
        assert_eq!(b.as_str().len() % 3, 0, "only whole 3-byte codepoints may survive");
        assert!(b.as_str().chars().all(|c| c == '€'));
    }

    #[test]
    fn dsp_macro_yields_a_borrow_that_outlives_the_call() {
        // The property the expansion form exists for: a block form returning `&local` does not
        // compile, and this call is the shape that proves the temporary lives long enough.
        fn takes(s: &str) -> usize {
            s.len()
        }
        assert_eq!(takes(dsp!(12345u32)), 5);
        assert_eq!(takes(dsp!(7u8, 8)), 1);
    }

    #[test]
    fn header_is_packed_to_twenty_bytes() {
        // Restated at run time as well as in the `const` assert: the `const` proves the size, and
        // this proves the field offsets a `repr(C, packed)` promise implies -- a reordering that
        // preserved the size would pass the const assert alone.
        assert_eq!(core::mem::size_of::<RecordHeader>(), 20);
        assert_eq!(core::mem::offset_of!(RecordHeader, site), 0);
        assert_eq!(core::mem::offset_of!(RecordHeader, tsc), 8);
        assert_eq!(core::mem::offset_of!(RecordHeader, len), 16);
        assert_eq!(core::mem::offset_of!(RecordHeader, flags), 18);
        assert_eq!(core::mem::offset_of!(RecordHeader, clock_epoch_lo), 19);
    }
}
