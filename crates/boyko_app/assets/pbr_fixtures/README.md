# `pbr_fixtures/`

Tracked, in-repo ground-truth oracle texture sets — small (~600 KB total, unlike
`assets/materials/`, which is gitignored). Each fixture exists to visually verify
ONE specific convention the textured-PBR pipeline depends on. Point
`pbr_material_showcase.rs` at one via `BOYKO_PBR_TEXTURE_DIR`.

## `synth_bumps/`

A synthetic bump grid: known-protruding geometry baked into `normal.png` (plus a
matching `albedo.png` / `ao.png` / `metallic_roughness.png`). The normal-map
GREEN-CHANNEL CONVENTION oracle — under correct green-channel handling the bumps
must visibly light FROM the sun side; an inverted green channel renders them as
dents instead of bumps (see `gbuffer_mrt.fs.hlsl`'s GREEN-CHANNEL CONVENTION
block).

The baker emitted OpenGL green-up; the committed `normal.png` is the **DirectX**
conversion of it (`scripts/normal_ogl_to_dx.py`, 2026-07-16), because DirectX is
the engine's canonical convention and the engine re-signs nothing at runtime. This
fixture is a valid oracle only while it stays canonical: if the baker is ever
re-run, convert its output again before committing.

Unlike a vendor pack, this fixture's convention is *known* rather than measured —
it was produced in-repo. For third-party packs the filename is not evidence (one
vendor ships mixed conventions under identical `-ogl` names); measure them against
their height map with `normal_ogl_to_dx.py --detect`, as `assets/materials/README.md`
describes.

## `synth_marker/`

The same bump grid plus an UP-arrow / letter-"R" marker baked into `albedo.png`.
Verifies PNG row-order (a vertically-flipped decode reads the arrow upside-down)
and UV mirroring/winding (a mirrored U reads the "R" as its own mirror image) — a
second, independent oracle from `synth_bumps/`'s green-channel proof.

## Running

```
BOYKO_PBR_TEXTURE_DIR=<repo>/crates/boyko_app/assets/pbr_fixtures/synth_bumps \
    cargo test -p boyko-app --test pbr_material_showcase -- --ignored --test-threads=1
```

(mirrors `pbr_material_showcase.rs`'s own module-doc windowed-eval conventions —
`BOYKO_DISABLE_VALIDATION=1`, `BOYKO_HOST_DUMP=<path.bmp>` to capture the frame.)
