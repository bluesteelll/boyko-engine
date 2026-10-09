# Materials & Textures

> Metallic-roughness PBR materials stored as assets, five optional bindless textures per material,
> and the tonemapping curve at the end of the frame.

## What it is

A material in `boyko_render` has two halves
([`material.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/material.rs)):

- **`MaterialGpu`**, 48 bytes: base colour and alpha, metallic, roughness, reflectance, flags and
  emissive radiance, all in linear space. This is the element of the material table the shaders
  index.
- **`MaterialTextures`**: five bindless texture slots (`albedo`, `normal`, `metal_rough`, `ao`,
  `emissive`). Slot `0` means "no texture, use the scalar".

`Material { gpu, textures }` is the asset. The world owns every material in an `Assets<Material>`
resource, and the renderer mirrors that table into a GPU buffer (`MaterialTable`). An entity references a
material through the `MaterialHandle` component, a 16-bit index into that table. Slot 0 is the
engine's default material.

## Creating a material

```rust,ignore
use boyko_app::prelude::*;
use boyko_ecs::prelude::ResMut;
use boyko_render::Material;

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    // base colour (linear RGBA), metallic, roughness, reflectance, emissive, flags
    let red_plastic = materials.add(Material::new([0.8, 0.1, 0.1, 1.0], 0.0, 0.4, 0.5, [0.0; 3], 0));

    let cube = meshes.cube(dev.get(), 1.0);
    let mut bundle = MeshBundle::new(cube, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)));
    bundle.material = MaterialHandle::from_handle(red_plastic);
    commands.spawn(bundle);
}
```

The `.mat` loader reads the same scalars from a small text file, one `key = value` pair per line:

```text
# red plastic
base_color = 0.8, 0.1, 0.1, 1.0
metallic = 0.0
roughness = 0.4
reflectance = 0.5
emissive = 0.0 0.0 0.0
```

A missing key keeps its default, and an unknown key is ignored. The loader is
`RonMaterialLoader` in [`loaders/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_render/src/loaders),
despite its name an in-house key-value format rather than RON. See
[Assets & Handles](../concepts/assets.md) for the loading API.

## Textures

Textures are `TextureGpu` assets: a GPU image with a full mip chain, registered in the bindless
texture table (`BindlessTextureTable`). Every texture is sampled with one shared trilinear,
anisotropic sampler, so a material refers to a texture by its bindless slot number alone.

The quickest route is a texture folder. `load_material_folder` uploads whichever of these files
exist and returns the slots:

| Slot | File | Colour space |
|------|------|--------------|
| `albedo` | `albedo.png` or `base_color.png` | sRGB |
| `normal` | `normal.png` | linear, tangent space |
| `metal_rough` | `metallic_roughness.png` or `mr.png` | linear; G = roughness, B = metallic (the glTF packing) |
| `ao` | `ao.png` | linear; R = occlusion |
| `emissive` | `emissive.png` | sRGB |

```rust,ignore
use boyko_app::prelude::*;
use boyko_ecs::prelude::ResMut;
use boyko_render::{BindlessTextureTable, Material, MaterialGpu, TextureGpu, load_material_folder};

fn setup(
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
) {
    let slots = load_material_folder(&mut textures, dev.get(), &mut bindless,
                                     std::path::Path::new("assets/materials/brick/pbr"));
    // With a texture bound, the scalar multiplies it (base colour) or stands in where it is absent.
    let brick = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.9, 0.5, [0.0; 3], 0),
        slots,
    ));
    // ... spawn meshes with MaterialHandle::from_handle(brick) ...
}
```

A missing or undecodable file leaves its slot at `0` and prints a one-line note; the call never
panics. Textures are decoded by the engine's own PNG decoder (`boyko_image`); see
[Meshes & Loaders](meshes-and-loaders.md).

Normal maps need a tangent basis. The `.obj` loader and the built-in `cube` / `plane` primitives
generate per-vertex tangents at load time (`generate_tangents`, Lengyel's method); `.glb` files
can carry their own.

### Where textures are drawn

| Geometry / path | Textured materials |
|-----------------|--------------------|
| Meshes, Deferred | yes: a separate textured raster pipeline |
| Meshes, Visibility Buffer | yes: the classified (or split) textured shaders |
| Meshes, Forward and Forward+ | scalars only: the Forward shaders do not sample the texture table yet |
| SDF shapes | scalars only, from the material id an edit carries |

## Shading model

The deferred resolve evaluates a Cook-Torrance metallic-roughness BRDF from the material
parameters. Ambient diffuse and ambient specular are occluded separately: the specular term uses a
roughness-aware specular occlusion, so polished metal does not darken like matte paint.

`LightingConfig::terminator_softening` (default `0.0`) wraps the diffuse falloff, which softens
the hard shadow line that normal maps produce under grazing light. Values around `0.15` to `0.3`
give a gentle falloff.

## Exposure and tonemapping

At the end of the resolve the accumulated linear radiance is multiplied by
`LightingConfig::exposure` (default `1.0`), mapped to `[0, 1]` by a tonemapper and gamma-encoded.

| `Tonemapper` | Character |
|--------------|-----------|
| `Aces` (default) | Stephen Hill's fitted ACES curve. |
| `Neutral` | Khronos PBR Neutral: hue-preserving, gentle toe, no shadow crush. |
| `ReinhardJodie` | A cheap hue-preserving Reinhard variant. |

```rust,ignore
use boyko_render::{LightingConfig, Tonemapper};

app.insert_resource(LightingConfig {
    tonemapper: Tonemapper::Neutral,
    exposure: 1.2,
    ..LightingConfig::default()
});
```

There is no automatic exposure.

## See also

- [Meshes & Loaders](meshes-and-loaders.md): the vertex format and the file loaders.
- [Lighting](lighting.md): the lights and `LightingConfig`.
- [Assets & Handles](../concepts/assets.md): `Assets<T>`, handles and reference counts.
- [Aether: Materials](../aether/materials.md): declaring materials in the Aether language.
- Source: [`material.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/material.rs),
  [`texture.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/texture.rs),
  [`bindless.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/bindless.rs).
