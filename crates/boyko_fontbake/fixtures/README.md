# boyko_fontbake golden fixtures

These **libre** font files are the checked-in goldens for the T0–T3 tests
(written by the `tester`, not the developer). They are NOT committed by the
developer because (a) the developer must not write the golden tests, and (b)
only freely-redistributable fonts may live in the repo — system fonts on the
build machine are proprietary and must never be checked in.

## Checked-in files

| File | Format | Outline kind | Purpose | Status |
|------|--------|--------------|---------|--------|
| `Ubuntu-Light.ttf` | TrueType (`glyf`) | quadratic | T0/T1/T2/T3 goldens: outline + metrics + MSDF passes + atlas/.bfont | **present** (Ubuntu Font License / UFL, libre) |
| `SourceCodePro-Regular.otf` | OpenType-CFF (`OTTO`) | cubic | T2a end-to-end cubic golden (CFF charstring path) | **present** (SIL Open Font License 1.1 — see `SourceCodePro-OFL.txt`) |

`Ubuntu-Light.ttf` is the canonical TrueType fixture the test suite pins
specific glyph/segment/metric values against (`'A'`, `'o'`, `'.'`, `'8'`, …).

**CFF/OTF (cubic) note:** `SourceCodePro-Regular.otf` is the OpenType-CFF
(`OTTO`, Type-2 charstring) fixture that exercises the **end-to-end cubic path**
the `glyf` (quadratic) Ubuntu font never reaches. The `t2a_cff_*` goldens in
`tests/gate_goldens.rs` load it, assert the extracted outline is genuinely
**cubic** (SourceCodePro `'o'` decodes to 8 `Segment::Cubic`, zero quads — the
`curve_to` → `cubic_to` charstring path, NOT a quad fallback), pin the exact
decoded control points, and cross-check the MSDF `.a` (true SDF) channel against
an independent brute-force nearest-point reference. The complementary unit check
`t2a_synthetic_cubic_distance_matches_bruteforce` still exercises the cubic
nearest-point solver (the `multi-seed Newton`) directly on a **synthetic
`Segment::Cubic`**, so the math is covered both in isolation and end-to-end.

Any libre TrueType and any libre CFF/OTF will do as fixtures. Recommended libre
sources:

- TrueType: DejaVu Sans, Roboto, or Liberation Sans (all OFL/Apache/libre).
- CFF/OTF: Source Serif / Source Sans (OFL), or any Adobe-Source family `.otf`.

A subset (ASCII + Latin-1) keeps the fixture small; the full font also works.

## How the code consumes them

`boyko_fontbake::TtfFace::from_bytes(&std::fs::read(path)?)` parses either
format (the `ttf-parser` backend handles `glyf` and CFF/CFF2). The whole bake
pipeline is font-agnostic, so the goldens only pin specific glyph/segment/field
values, not the file identity.

Both required fixtures (the TTF quadratic and the CFF cubic) are now checked in,
so the goldens hard-fail on a missing fixture rather than skipping. The library
itself never reads these files; only the tests do.
