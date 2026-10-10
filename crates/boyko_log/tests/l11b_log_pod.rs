//! `LogPod`, and B10's property stated as a number rather than as a promise.
//!
//! The design this replaces used a blanket `copy_nonoverlapping` of `size_of::<Self>()`. That is UB
//! **independent of whether `POD_LEN` is honest**: a `#[repr(C)]` struct has padding, `size_of`
//! includes it, padding bytes are uninitialised, and copying them lets the sink materialise a
//! `&[u8]` over uninitialised memory.
//!
//! So the test that matters is not "encoding round-trips" — a `size_of` copy round-trips too. It is
//! that **`POD_LEN` is strictly less than `size_of` for a padded struct**, which is only true if the
//! length is a field sum. `Padded` below exists to make those two numbers differ.

use boyko_log::record::{DspBuf, LogPod};

#[repr(C)]
#[derive(Clone, Copy)]
struct Hit {
    dmg: u32,
    target: u32,
}
boyko_log::impl_log_pod!(Hit { dmg: u32, target: u32 });

/// `u8` then `u64` under `#[repr(C)]`: seven bytes of padding after the first field.
#[repr(C)]
#[derive(Clone, Copy)]
struct Padded {
    tag: u8,
    at: u64,
}
boyko_log::impl_log_pod!(Padded { tag: u8, at: u64 });

#[test]
fn pod_len_is_a_field_sum_and_never_size_of() {
    // ── the unpadded case: the two happen to agree, which is why it cannot be the test ───────
    assert_eq!(<Hit as LogPod>::POD_LEN, 8);
    assert_eq!(core::mem::size_of::<Hit>(), 8);

    // ── THE PROPERTY. A padded struct is where a `size_of` copy would put uninitialised bytes
    //    on the wire, and where a field sum provably does not.
    assert_eq!(<Padded as LogPod>::POD_LEN, 9, "1 + 8 = the FIELDS, not the layout");
    assert_eq!(core::mem::size_of::<Padded>(), 16, "the layout is 16 with seven padding bytes");
    assert!(
        <Padded as LogPod>::POD_LEN < core::mem::size_of::<Padded>(),
        "if these were equal the encoder would be copying padding -- the exact B10 defect, and \
         the only assertion here that a size_of-based impl would fail"
    );
}

#[test]
fn encode_pod_writes_exactly_pod_len_initialised_bytes_and_no_more() {
    // A canary byte past the end: a blanket `size_of` copy would write seven padding bytes over it.
    let mut buf = [0xAAu8; <Padded as LogPod>::POD_LEN + 1];
    let v = Padded { tag: 3, at: 0x0102_0304_0506_0708 };
    // SAFETY: `buf` is longer than `POD_LEN`, so it is valid for that many writes, and a fresh
    //   local cannot overlap `v`.
    unsafe { v.encode_pod(buf.as_mut_ptr()) };

    assert_eq!(buf[0], 3, "the first field is written at offset 0");
    assert_eq!(&buf[1..9], &0x0102_0304_0506_0708u64.to_le_bytes(), "the second follows IMMEDIATELY");
    assert_eq!(
        buf[<Padded as LogPod>::POD_LEN],
        0xAA,
        "encode_pod wrote past POD_LEN -- with a size_of copy this byte is padding from the struct"
    );
}

#[test]
fn fmt_pod_renders_named_fields_and_refuses_to_read_past_a_short_slice() {
    let v = Hit { dmg: 12, target: 7 };
    let mut bytes = [0u8; <Hit as LogPod>::POD_LEN];
    // SAFETY: `bytes` is exactly `POD_LEN` long and does not overlap `v`.
    unsafe { v.encode_pod(bytes.as_mut_ptr()) };

    let mut out = DspBuf::<128>::new();
    let mut f = boyko_log::LogFormatter::new(&mut out);
    <Hit as LogPod>::fmt_pod(&bytes, &mut f);
    assert_eq!(out.as_str(), "{dmg: 12, target: 7}");

    // The sink decodes bytes off a shared ring and must not trust the length it was handed.
    let mut short = DspBuf::<128>::new();
    let mut f2 = boyko_log::LogFormatter::new(&mut short);
    <Hit as LogPod>::fmt_pod(&bytes[..3], &mut f2);
    assert!(
        short.as_str().contains("{truncated}"),
        "a short slice must render a marker, not read past its end: {:?}",
        short.as_str()
    );
}
