# `assets/materials/`

Local drop folder for owner-supplied PBR material packs. Everything in this folder
except this README is gitignored (see the top-level `.gitignore`) — material packs
are large, externally-sourced texture sets and are never committed to the
repository.

## Layout

Each pack lives in its own subfolder, normalized to:

```
<pack-name>/pbr/
    albedo.png             (alias: base_color.png)   -- sRGB
    normal.png                                       -- linear, tangent-space
    metallic_roughness.png (alias: mr.png)            -- linear, glTF ORM (G=roughness, B=metallic)
    ao.png                                            -- linear, R=occlusion
    emissive.png                                      -- sRGB
```

This mirrors `crates/boyko_app/assets/pbr_test/README.txt`'s convention exactly —
the same filenames the engine's texture-folder loader
(`boyko_render::load_material_folder`) resolves. Point `pbr_material_showcase.rs`
at a pack with `BOYKO_PBR_TEXTURE_DIR=<path>/pbr`.

Freshly-downloaded packs may arrive as a raw zip extraction (an un-normalized
folder tree) before being reorganized into the `pbr/` layout above — both shapes
may coexist here while a pack is staged.

## `normal.png` must be DirectX — measure it, never trust the filename

The engine's canonical tangent-space normal convention is **DirectX** (green-down:
+G = a slope facing image-bottom), the Unreal convention. The engine never re-signs
a normal map: neither the loader nor the TEXTURED shaders negate green. A pack's
`pbr/normal.png` is therefore *required* to already be DirectX; OpenGL sources are
converted once, offline, when the pack is staged:

```
python scripts/normal_ogl_to_dx.py <pack>/pbr/normal.png    # green-inverts in place, keeps a .ogl.bak
```

**Vendor filenames lie.** `alley-brick-wall` ships its map as
`alley-brick-wall_normal-ogl.png` and is measurably DirectX, while the *same
vendor's* `industrial-walls_normal-ogl.png` really is OpenGL — one vendor, mixed
conventions, identical `-ogl` naming. Converting a DirectX map because the name
says `ogl` inverts its relief, and brick is the one texture the eye cannot
adjudicate (light mortar reads as raised; bump/dent is bistable). So measure
against the pack's own height map instead of guessing:

```
python scripts/normal_ogl_to_dx.py --detect <pack>/pbr/normal.png <pack>/height.png
```

It correlates green against the height field's implied slope and prints the
verdict plus a convention-independent control correlation that must come out
strongly positive (otherwise the two maps are misaligned and the verdict is void).
Packs with no height map and a near-flat normal (e.g. `light-gold`) are
unmeasurable — leave them alone rather than guess.

## `_renders/`

Ephemeral eval-output screenshots/comparisons produced while visually judging
materials (owner/tester scratch space) — not read by any loader, not committed.

## Nothing here is committed

The whole folder is gitignored except this README (`/assets/materials/*` +
`!/assets/materials/README.md` in the top-level `.gitignore`). Durable, tracked
test fixtures live instead at `crates/boyko_app/assets/pbr_fixtures/` (see that
folder's own README).
