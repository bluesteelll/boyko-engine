# Meshes & Loaders

> How triangle meshes reach the GPU: the vertex format, the `Assets<MeshGpu>` table, the built-in
> primitives, and the engine's own `.obj`, `.glb`, `.png` and `.mat` decoders.

## Meshes as assets

A mesh is a `MeshGpu` asset in the world's `Assets<MeshGpu>` table, a non-send resource because
each record owns its GPU vertex and index buffers. An entity draws a mesh through the
`MeshHandle` component, which `MeshBundle` carries together with the transform, the material and
the visibility.

`MeshAssetsExt` (in the `boyko_app` prelude) adds the mesh-specific calls to `Assets<MeshGpu>`:

| Call | What it does |
|------|--------------|
| `cube(ctx, size)` | An axis-aligned cube centred at the origin, with per-face normals. |
| `plane(ctx, size)` | A flat square in the XZ plane, centred at the origin. |
| `register_mesh(ctx, &vertices, &indices)` | Uploads your own triangles; 16-bit indices are chosen when the vertex count allows. |
| `mesh(handle)`, `try_get(handle)` | Look up a registered mesh. |

```rust,ignore
use boyko_app::prelude::*;

fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    commands.spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))));
}
```

Register meshes in a startup system: it runs after the device exists, which the upload needs.

## The vertex format

Every mesh uses one `Vertex` layout
([`mesh.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/mesh.rs)):

| Field | Type | Notes |
|-------|------|-------|
| `position` | `[f32; 3]` | Model space. |
| `normal` | `[f32; 3]` | Model space. |
| `color` | `[f32; 4]` | Linear vertex colour, multiplied into the albedo. |
| `uv` | `[f32; 2]` | `[0, 0]` when the source has none. |
| `tangent` | `[f32; 4]` | `xyz` tangent, `w` the bitangent sign. Needed by normal maps. |

`generate_tangents` derives tangents from positions, normals and UVs (Lengyel's method). The
built-in primitives and the `.obj` loader run it once at load time; it never runs per frame.

## Loading files

Each loader is an `AssetLoader` keyed by file extension. They decode bytes into a CPU form
(`MeshData` for meshes); [Assets & Handles](../concepts/assets.md) describes the `AssetServer`
calls that read a path, decode it and stage the result. You can also call a decoder directly and
register the result yourself:

```rust,ignore
use boyko_ecs::ecs::core::asset::AssetLoader;
use boyko_render::loaders::ObjMeshLoader;

let bytes = std::fs::read("assets/models/rock.obj").expect("read the model");
let data = ObjMeshLoader::decode(&bytes).expect("a valid .obj");
let rock = meshes.register_mesh(dev.get(), &data.vertices, &data.indices);
```

| Loader | Extension | Produces | Supported |
|--------|-----------|----------|-----------|
| `ObjMeshLoader` | `.obj` | a mesh | `v`, `vn`, `vt`, faces of 3+ corners (fan-triangulated), negative indices. Materials (`mtllib`, `usemtl`) are skipped. |
| `GlbMeshLoader` | `.glb` | a mesh | Binary glTF 2.0, triangle primitives with `POSITION`, `NORMAL`, `TEXCOORD_0`, `TANGENT`, `COLOR_0` and 8/16/32-bit indices. Node transforms are baked in. |
| `PngTextureLoader` | `.png` | a texture | See [PNG decoding](#png-decoding-boyko_image). |
| `RonMaterialLoader` | `.mat` | a material | The key-value text format in [Materials & Textures](materials-and-textures.md). |

### glTF details

`GlbMeshLoader::decode` merges every primitive of the file into one mesh. Two more entry points
give more control:

- `decode_scene` keeps the primitives apart (`GlbScene::parts`) and returns the file's materials
  (base colour, metallic and roughness factors, emissive, and the indices of the base-colour,
  normal, metallic-roughness and occlusion images) and its embedded images. Wiring those into
  `Material` and `TextureGpu` assets is up to the caller, and only PNG images can be decoded.
- `decode_static_pose` and `decode_scene_static_pose` accept rigged files and return the rest
  shape, dropping joints, weights and morph targets.

A plain `decode` refuses skins and animations, as well as sparse accessors, Draco and meshopt
compression (any `extensionsRequired`), non-triangle modes and non-indexed primitives. Each
refusal is an `AssetError`, never a silent fallback.

## PNG decoding (`boyko_image`)

[`boyko_image`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_image) is the
engine's own PNG decoder, with zlib and DEFLATE written from the RFCs and no third-party
dependencies. Its one entry point is `decode_png`, which expands every image to RGBA.

| Supported | Refused |
|-----------|---------|
| Grayscale, RGB, grayscale + alpha, RGBA | Palette images |
| 8 and 16 bits per channel (16-bit is reduced to 8 for textures) | 1, 2 and 4 bits per channel |
| Non-interlaced images | Adam7 interlacing |

The refused forms are not used by PBR texture sets, so the decoder stays small. There is no JPEG
decoder.

## What is not supported

- Skeletal skinning and animation: rigged models load in their rest pose only.
- Virtual geometry or streaming of mesh detail.
- Mesh LODs.

## See also

- [Materials & Textures](materials-and-textures.md): what a mesh is shaded with.
- [Assets & Handles](../concepts/assets.md): `Assets<T>`, the `AssetServer` and reference counts.
- [GPU-Resident Columns](gpu-columns.md): how instance transforms reach the GPU.
- [Render Paths & Visibility Buffer](render-paths.md): how meshes are rasterised.
- Source: [`loaders/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_render/src/loaders),
  [`mesh_assets.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/mesh_assets.rs),
  [`crates/boyko_image`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_image).
