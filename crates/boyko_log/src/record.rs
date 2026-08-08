//! The record header, the payload encoding, and `dsp!`.
//!
//! # Deferred formatting
//!
//! A record on the ring is a 20-byte header plus a **POD payload**. Nothing is formatted on the
//! emitting thread: the header carries a `&'static LogSite` whose `decode` function pointer is
//! monomorphised per argument-tuple type, and the sink calls it later, off the hot path. That is
//! what keeps an enabled site in the tens of nanoseconds instead of the hundreds.
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
// One header plus one maximal string must fit, or a single-argument site could never emit.
const _: () = assert!(HEADER_BYTES + 2 + MAX_STR_BYTES <= MAX_RECORD_BYTES);

/// One value that can travel in a record payload.
///
/// # Safety
///
/// [`encode`](LogValue::encode) writes exactly [`encoded_len`](LogValue::encoded_len) bytes to
/// `dst` and returns that count. An implementation that writes more overruns the ring; one that
/// writes fewer leaves the payload misaligned with the header's `len` and every subsequent field
/// decodes from the wrong offset.
pub unsafe trait LogValue {
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
    ($($t:ty),* $(,)?) => {$(
        // SAFETY: `encoded_len` is `size_of::<$t>()` and `encode` writes exactly that many bytes
        //   with one `copy_nonoverlapping` from the value's little-endian byte array. The two
        //   cannot disagree because both are derived from the same `size_of`.
        unsafe impl LogValue for $t {
            const MAX_ENCODED_LEN: usize = core::mem::size_of::<$t>();

            #[inline]
            fn encoded_len(&self) -> usize {
                core::mem::size_of::<$t>()
            }

            #[inline]
            unsafe fn encode(&self, dst: *mut u8) -> usize {
                let bytes = self.to_le_bytes();
                // SAFETY: the caller guarantees `dst` is valid for `encoded_len()` writes, which
                //   is exactly `bytes.len()`. The source is a local array, so the ranges cannot
                //   overlap.
                unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len()) };
                bytes.len()
            }
        }
    )*};
}

impl_pod_value!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64);

// SAFETY: one byte, written directly; `encoded_len` is 1 and `encode` writes 1.
unsafe impl LogValue for bool {
    const MAX_ENCODED_LEN: usize = 1;

    #[inline]
    fn encoded_len(&self) -> usize {
        1
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        // SAFETY: the caller guarantees `dst` is valid for one write.
        unsafe { dst.write(u8::from(*self)) };
        1
    }
}

// SAFETY: encoded as its `u32` scalar value; `encoded_len` and `encode` both use 4.
unsafe impl LogValue for char {
    const MAX_ENCODED_LEN: usize = 4;

    #[inline]
    fn encoded_len(&self) -> usize {
        4
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        let bytes = (*self as u32).to_le_bytes();
        // SAFETY: the caller guarantees `dst` is valid for four writes; source is a local array.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, 4) };
        4
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

// SAFETY: `encoded_len` is `2 + str_fit(self)` and `encode` writes a `u16` length followed by
//   exactly `str_fit(self)` bytes -- the same quantity, computed by the same function.
unsafe impl LogValue for &str {
    const MAX_ENCODED_LEN: usize = 2 + MAX_STR_BYTES;

    #[inline]
    fn encoded_len(&self) -> usize {
        2 + str_fit(self)
    }

    #[inline]
    fn value_flags(&self) -> u8 {
        if self.len() > MAX_STR_BYTES { flags::STR_TRUNCATED } else { 0 }
    }

    #[inline]
    unsafe fn encode(&self, dst: *mut u8) -> usize {
        let n = str_fit(self);
        let len_bytes = (n as u16).to_le_bytes();
        // SAFETY: the caller guarantees `dst` is valid for `encoded_len()` == `2 + n` writes.
        //   Both copies stay inside that span, and neither source can alias the ring.
        unsafe {
            core::ptr::copy_nonoverlapping(len_bytes.as_ptr(), dst, 2);
            core::ptr::copy_nonoverlapping(self.as_ptr(), dst.add(2), n);
        }
        2 + n
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
    /// Render `v`.
    #[inline]
    #[must_use]
    pub fn render(v: &impl core::fmt::Display) -> Self {
        let mut me = DspBuf { buf: [0; N], len: 0, truncated: false };
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
        assert_eq!(enc(&(0x1234_5678u32,)), vec![0x78, 0x56, 0x34, 0x12]);
        assert_eq!(enc(&(true,)), vec![1]);
        assert_eq!(enc(&(-1i16,)), vec![0xFF, 0xFF]);
        assert_eq!(enc(&(1.0f32,)), 1.0f32.to_le_bytes().to_vec());
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
        assert_eq!(out, vec![2, 0, b'h', b'i']);
    }

    #[test]
    fn an_over_cap_string_truncates_and_flags() {
        let s = "x".repeat(MAX_STR_BYTES + 50);
        let t = (s.as_str(),);
        assert_eq!(t.encoded_len(), 2 + MAX_STR_BYTES);
        assert_eq!(t.args_flags(), flags::STR_TRUNCATED);
        assert_eq!(enc(&t).len(), 2 + MAX_STR_BYTES);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // A 3-byte codepoint straddling the cap. Cutting mid-sequence would hand the sink invalid
        // UTF-8 -- corruption introduced by the logger, on the path that reports corruption.
        let mut s = "a".repeat(MAX_STR_BYTES - 1);
        s.push('€'); // 3 bytes, so bytes 255..258 -- the cap lands inside it
        let t = (s.as_str(),);
        let out = enc(&t);
        let payload = &out[2..];
        assert!(
            core::str::from_utf8(payload).is_ok(),
            "the encoded prefix must remain valid UTF-8"
        );
        assert_eq!(payload.len(), MAX_STR_BYTES - 1, "the whole codepoint must be dropped");
        assert_eq!(t.args_flags(), flags::STR_TRUNCATED);
    }

    #[test]
    fn an_exactly_capped_string_is_not_flagged() {
        // The boundary case, because `>` and `>=` are one keystroke apart and both compile.
        let s = "y".repeat(MAX_STR_BYTES);
        let t = (s.as_str(),);
        assert_eq!(t.args_flags(), 0);
        assert_eq!(t.encoded_len(), 2 + MAX_STR_BYTES);
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
