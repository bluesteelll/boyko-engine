boyko_engine PBR texture showcase — texture folder convention
===============================================================

This folder is read by `crates/boyko_app/tests/pbr_material_showcase.rs`
(`pbr_material_showcase_screenshot_dump`, `#[ignore]`) to build the owner-facing
material showcase scene: a real PBR texture set applied to a sphere so the
materials can be judged visually.

Where to put files
-------------------
By default the test reads THIS folder (`crates/boyko_app/assets/pbr_test/`,
resolved relative to the `boyko_app` crate at compile time). To point the test
at a different folder instead, set the environment variable
`BOYKO_PBR_TEXTURE_DIR` to an absolute path before running the test.

Filenames (ALL optional — a missing file falls back gracefully, no crash)
---------------------------------------------------------------------------
  albedo.png               (alias: base_color.png)   -- sRGB color space
  normal.png                                          -- linear color space, tangent-space
  metallic_roughness.png   (alias: mr.png)            -- linear color space, glTF ORM
                                                            channels: G = roughness, B = metallic
  ao.png                                              -- linear color space, R = occlusion
  emissive.png                                        -- sRGB color space

Any subset of these may be present. An empty folder renders a plain default
material (white base color, metallic 1.0, roughness 0.5). A missing/unreadable/
undecodable file for a given slot falls back to that channel's material default
and prints a one-line note to stderr — the scene never panics on a bad or
partial texture set.

Do NOT commit PNG files here
-----------------------------
This folder's `.gitignore` excludes `*.png` — only this README (and the
`.gitignore` itself) are tracked. The owner supplies real texture files locally;
they are never checked into the repository.
