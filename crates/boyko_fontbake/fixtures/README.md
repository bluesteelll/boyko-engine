# boyko_fontbake golden fixtures

These two **libre** font files are the checked-in goldens for the T0–T3 tests
(written by the `tester`, not the developer). They are NOT committed by the
developer because (a) the developer must not write the golden tests, and (b)
only freely-redistributable fonts may live in the repo — system fonts on the
build machine are proprietary and must never be checked in.

## Checked-in files

| File | Format | Outline kind | Purpose | Status |
|------|--------|--------------|---------|--------|
| `Ubuntu-Light.ttf` | TrueType (`glyf`) | quadratic | T0/T1/T2/T3 goldens: outline + metrics + MSDF passes + atlas/.bfont | **present** (Ubuntu Font License / UFL, libre) |
| _(CFF `.otf`)_ | OpenType-CFF | cubic | T2a cubic pseudo-distance golden (CFF charstring path) | **absent** — no libre CFF/OTF available on the build machine |

`Ubuntu-Light.ttf` is the canonical TrueType fixture the test suite pins
specific glyph/segment/metric values against (`'A'`, `'o'`, `'.'`, `'8'`, …).

**CFF/OTF (cubic) note:** no `.otf` (CFF) font was available to check in, so the
end-to-end CFF charstring path is not exercised. Instead, the cubic
nearest-point solver (the `multi-seed Newton` the CFF cubic path drives) is
tested directly on a **synthetic `Segment::Cubic`** against a dense brute-force
reference (`t2a_synthetic_cubic_distance_matches_bruteforce` in
`tests/gate_goldens.rs`) — covering the same math without a CFF file. When a
libre CFF/OTF is added, an end-to-end cubic golden can be enabled.

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

If a fixture is absent the relevant golden test should `skip` (the tester's
call); the library itself never reads these files.
