# The `.blog` binary log format

Written by `crates/boyko_log/src/sink/binary.rs`, read by `crates/boyko_log/src/bin/logdec.rs`.
This document is the wire contract; the code is the authority and the two are kept together by
`crates/boyko_log/tests/l13b_binary_format.rs` (codec), `l13b_binary_sink_wired.rs` (the sink
writes it) and `l13b_logdec_roundtrip.rs` (the tool reads it back).

## Why a second format exists at all

The text sink renders every record on the drain thread through `core::fmt`. The binary sink writes
the payload **verbatim** and names the site by a two-byte dictionary id, so the formatting happens
offline in `logdec` instead of in the process being measured. Measured on this box, over eleven
sittings: `render_payload` against `encode_record`, same payload, interleaved — **4.29× – 4.68×**.
`docs/OPEN-QUESTIONS.md` records the owner ruling that kept the rung when its `≥ 5×` clause fired.

## Conventions

* **Little-endian throughout.**
* **Length-prefixed strings**: a `u16` byte count followed by that many UTF-8 bytes. No NUL
  terminator, no fixed cap — a `file` and a `fmt` are arbitrary strings and a fixed width would
  silently truncate the one thing that makes a record locatable.
* **Every frame starts with a one-byte kind**, so a reader can tell what it is holding before it
  reads anything else.
* **A short read is an ordinary outcome, not corrupt input.** The file a reader most wants is the
  one a crash cut off mid-write. The decoder stops at the first frame that does not decode and the
  caller compares consumed bytes against the file length; `logdec` prints `RAGGED TAIL`.

## Frame kinds

| byte | kind | when |
|---|---|---|
| `1` | `Dictionary` | first time a site is written to this file |
| `2` | `Record` | the common case |
| `3` | `Anchor` | at `open`, and whenever a delta would overflow or go backwards |
| `4` | `InlineSite` | the dictionary is full; the site travels with the record |

### `Anchor` — 17 bytes, fixed

| offset | width | field |
|---|---|---|
| 0 | 1 | kind = `3` |
| 1 | 8 | `ticks` — absolute, from `boyko_diag::clock::ticks` |
| 9 | 8 | `ticks_per_ns` — IEEE-754 `f64` bits |

**The scale is in the file because the reader is on another machine.** A tick count is a property
of the CPU that produced it. The first draft wrote ticks alone, and a decoder could print
`+41231 ticks` and nothing better — for a format whose entire purpose is to be read offline.
`logdec` prints `t=NNN` and a `NO ANCHOR` note when the scale is missing or zero, rather than
computing a plausible-looking millisecond figure against a scale of one.

**A file must open with an anchor.** One that opens with a record has no absolute time to add its
deltas to: it decodes to a session that started at zero, which is worse than one that refuses to
decode.

### `Dictionary` — variable

| offset | width | field |
|---|---|---|
| 0 | 1 | kind = `1` |
| 1 | 2 | `site_id` |
| 3 | 4 | `line` |
| 7 | 2 + n | `file` |
| … | 2 + n | `fmt` |

**Per file, not per process.** `open` resets the dictionary, because a decoder replays it from the
frames in the file it is reading — ids carried over from a previous file would decode this one
under the wrong sites.

**One frame per SITE, not per record.** A `Dictionary` frame emitted per record would be a
file/line pair per record wearing a dictionary's name, and the format's whole saving would be gone.

### `Record` — 11-byte header plus payload

| offset | width | field |
|---|---|---|
| 0 | 1 | kind = `2` |
| 1 | 2 | `site_id` — into the dictionary replayed from **this file** |
| 3 | 4 | `tsc_delta` — ticks **since this file's anchor** |
| 7 | 2 | payload length |
| 9 | 1 | `flags` — carried verbatim from the ring |
| 10 | 1 | `epoch_lo` — low 8 bits of `clock_epoch`, so a record straddling a suspend is legible |
| 11 | n | payload |

**`tsc_delta` is measured from the anchor**, and saying so is not redundant: the first
implementation wrote the low 32 bits of the *absolute* counter into this field, and `logdec` printed
`+425.840ms` for three records emitted microseconds apart. Every frame round-tripped byte for byte,
because the bytes were faithfully the wrong number.

**`u32` spans ~1.4 s at 3 GHz**, so a file re-anchors before a delta could reach it. The bound is
`u32::MAX / 2` **ticks** — expressed in ticks, not seconds, so the write path needs no clock scale;
that is roughly 0.7 s at 3 GHz and 2.1 s at 1 GHz, and it is exact against the wire width on every
machine.

**A delta also re-anchors when it would go BACKWARDS.** The drain walks lanes in index order and a
lane is per thread, so `tsc` is *not* monotone across records within one pass: a record from lane 5
may be older than one from lane 2. Subtracting a later anchor would underflow into a wrapped `u32`
— about four billion ticks of nothing.

**A re-anchor is stamped at the triggering record's own tick**, so that record's delta is exactly
zero. Stamping it at a fresh clock reading would put the anchor *after* the record it was written
for, and the record's own delta would underflow — the bug the rule exists to avoid, reintroduced by
the fix for it.

The cost is bounded and worth naming: a pass whose lanes interleave badly can emit one anchor per
lane transition, 17 bytes each. A wrong stamp is silent; a few extra anchors are not.

### `InlineSite` — variable

| offset | width | field |
|---|---|---|
| 0 | 1 | kind = `4` |
| 1 | 4 | `line` |
| 5 | 2 + n | `file` |
| … | 2 + n | `fmt` |
| … | 2 + n | payload |

Written when `intern_site` refuses because the 4096-entry table is full. `boyko-W0116` reports the
condition **once**: past a full table every later site writes inline, so the stream grows, no record
is lost, and no id is reused. Reusing an id would decode a later record under an earlier site's
file and line — a log that lies about where it came from, which is worse than a large one.

**It carries `fmt`, and the first implementation did not.** Without the format literal an inline
record was locatable and not renderable: the decoder could say where it came from and could only
dump its values as raw tags.

## The payload

Self-describing, one `record::ValueTag` byte per value, **and those discriminants are a wire
format**. `record::render_payload` is the one walker on both sides: `{}` consumes the next value,
`{{` and `}}` are literal braces, and any other `{…}` group also consumes the next value **with its
format spec ignored** — a real limitation, written here rather than discovered at a call site.
Neither direction of disagreement is silent: a placeholder with no value renders `{missing}`.

## `logdec`

```
logdec <file.blog> [more.blog ...]
```

Prints one line per record: the stamp, the source location, and the rendered message. Exit `2` for
usage, `1` for a file that could not be read, `0` otherwise — **including a ragged tail**, which is
reported and is not a failure.

It has no parsing of its own: `boyko_log::sink::binary::frames` is the one walker, shared with the
format tests. A tool with a private copy of the decode would go on working after the shipped
decoder broke, and the test that proved the decoder would go on passing after the tool broke.

## What this format does not do yet

* **No rotation.** `02-SINK-LIFECYCLE.md` specifies a retained-file set and `logdec --merge`;
  neither exists.
* **No `SessionId` in the file.** Two files from one run cannot yet be proved to belong together,
  which is what `06-DISPOSITIONS.md` assigns to the cross-process case.
