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

## `_renders/`

Ephemeral eval-output screenshots/comparisons produced while visually judging
materials (owner/tester scratch space) — not read by any loader, not committed.

## Nothing here is committed

The whole folder is gitignored except this README (`/assets/materials/*` +
`!/assets/materials/README.md` in the top-level `.gitignore`). Durable, tracked
test fixtures live instead at `crates/boyko_app/assets/pbr_fixtures/` (see that
folder's own README).
