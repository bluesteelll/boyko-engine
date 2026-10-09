# Assets & Handles

An **asset** is a value too big, too shared, or too expensive to duplicate per
entity: a material, a GPU mesh, a texture. The engine stores each asset type in
one kernel resource, `Assets<T>`, and entities reference rows in it by
**handle** rather than by value.

```rust,ignore
use boyko_ecs::ecs::core::asset::{Assets, Handle};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;

fn setup(mut materials: ResMut<Assets<Material>>) {
    let gold: Handle<Material> = materials.add(Material::default());
    // …put `gold` on the entities that should render with it
}
```

`Assets<T>` is an ordinary [resource](resources.md), so it reaches your systems
through `Res` / `ResMut` like any other, and it obeys the same conflict rules in
the scheduler.

## The table

`Assets<T>` is not a `Vec<T>`. It is the kernel's own dense-storage recipe: a
standalone [`ComponentPool`](../memory/arena.md) for the values, plus an
occupancy bitmap, a per-slot state/generation word, a refcount column, and a
LIFO free list. The same machinery dense components ride, reused rather than
re-invented — which is why an asset table inherits the pool's virtual-memory
behavior: a large reserved address ceiling with lazy commit.

`with_reserved(cap)` is a **pre-touch hint, not a ceiling**. The table grows past
it; the number only decides how much is prepared up front.

```rust,ignore
app.insert_resource(Assets::<Material>::with_reserved(8));
```

The kernel core here is deliberately **render-agnostic**: no device, no upload,
no GPU-resident table. GPU residency for a given asset type lives in
`boyko_render`, because `boyko_ecs` cannot depend on it.

## The handle

`Handle<T>` is a `#[repr(C)]`, 8-byte, `Copy` pair: a slot `index` and a
`generation`.

| Property | Why |
|----------|-----|
| `Copy`, `Send`, `Sync` for **every** `T` | the marker is `PhantomData<fn() -> T>`, so a `!Send` or invariant `T` cannot poison the handle it never stores |
| traits hand-written, not derived | a derive on a generic struct adds a `T: Trait` bound, which would tie the handle's traits to `T`'s all over again |
| minted only by the table | `Handle::new` is crate-private: `Assets::add` / `reserve` and the asset server mint them, so a fabricated handle cannot name a slot it never owned |

Resolution goes back through the table:

```rust,ignore
let material: Option<&Material> = materials.get(gold);
```

`get`, `get_mut` and `contains` all return "nothing" for a handle that is out of
range, **stale**, or not `Loaded`. Stale is the generation's job: `remove` frees
the row and bumps its generation, so a handle minted before the free stops
resolving even after the slot is reused.

## Load states

A row is `Loading`, `Loaded` or `Failed`. `add` inserts a value that is
immediately `Loaded`; `reserve` mints a handle for a row that is still
`Loading`, and `fill` / `fail` complete it. That split is what lets a loader
hand out a handle before the bytes have arrived.

`get` resolves only `Loaded` rows, so a system holding a handle to an in-flight
asset simply sees `None` until it lands — there is no partially-initialized `T`
to observe.

## The render carrier

The renderer cannot afford an 8-byte handle per entity per lane, so a
render-visible reference is narrowed to a 16-bit row index at the point the
component is written:

```rust,ignore
use boyko_scene::MaterialHandle;

commands.spawn(bundle).insert(MaterialHandle(handle.index() as u16));
```

`MaterialHandle` is `#[repr(transparent)]` over a `u16` and is an ordinary
component with lifecycle hooks; `boyko_render`'s `MaterialId::from_handle` does
the same narrowing on the GPU-side sibling. Both debug-assert the row index fits
16 bits — the material table is documented to stay under 65 536 rows.

### Freed and reused rows

The carrier holds only the 16-bit index, so the GPU never sees a generation. The
CPU side checks it instead:

- `MeshHandle` and `MaterialHandle` each `#[require]` a generation lane,
  `MeshRefGen` / `MaterialRefGen`, which records the row generation the carrier
  was bound against.
- Under the windowed host, `AssetRefcountPlugin` (part of `EnginePlugins`) runs
  `boyko_render::validate_asset_refs` every frame, before the gathers that read
  the carriers.
- When a row's generation or load state no longer matches a carrier's lane — the
  row was freed, reused, or is not `Loaded` — the system marks the carrier stale.
  A stale mesh carrier is not drawn. A stale material carrier is drawn with the
  pinned default material (slot 0). A carrier with no lane at all (a row loaded
  from an older save) is treated as stale.

Refcounting keeps a row alive while carriers reference it. A carrier's
`on_insert` hook adds one reference and its `on_replace` hook (which also fires
on removal and despawn) drops one; a `boyko_render` system folds the counts in.
A row whose count reaches zero is retired only after the frames in flight have
finished with it. So the normal way to free a render-visible asset is to drop its
last carrier. An explicit `remove` is still caught by the validation above, but
every carrier on that row stops drawing (or falls back to the default material).

Slot 0 of the material table is **pinned**: the windowed host mints the engine
default material there at boot, and pinning keeps a refcount that reaches zero
from retiring the row every entity without an explicit material points at.

## Where the tables come from

Under [the windowed host](../app/windowed-host.md), boot inserts
`Assets<Material>`, mints and pins the default material, and wires the GPU-side
material table. A bare `App` inserts nothing — you create the tables you need.

Assets that live on the GPU (`Assets<MeshGpu>`) are `NonSend` resources, because
they own device-tied buffers, and mesh registration takes the device:

```rust,ignore
use boyko_ecs::ecs::core::system::{NonSendRes, NonSendResMut};
use boyko_render::{MeshAssetsExt, MeshGpu};
use boyko_app::GpuDevice;

fn spawn_world(
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = MeshAssetsExt::plane(&mut *meshes, dev.get(), 22.0);
    // …
}
```

That signature is exactly what [an Aether `scene`](../aether/scenes.md) writes
for you, and the reason a scene with mesh bindings needs a live device while one
without runs headless.

## Loading from bytes and paths

Loading is split in two: a pure-CPU **decode** and a device-side **upload**.

- **`AssetLoader`** decodes raw bytes into an asset's CPU intermediate
  (`Asset::Cpu`). It names the file extensions it claims (lowercase, no dot).
- **`HasLoaders`** is the per-asset-type dispatch table: a `const LOADERS` slice
  built with `LoaderEntry::of::<YourLoader>()`. It is fixed at compile time, so
  there is no runtime loader registry and no type erasure.
- **`AssetServer`** is a zero-sized resource with two methods.
  `decode_bytes::<A>(ext, bytes)` picks the loader by extension and decodes.
  `load(path, &mut assets, &mut staging, &mut paths)` reads the file, decodes
  it, reserves a `Loading` row in `Assets<A>`, and queues the decoded value on
  `AssetStaging<A>`.
- **`AssetPaths<A>`** is the per-type path index that makes `load` dedupe: the
  same path returns the same handle while that handle still resolves.

```rust,ignore
use boyko_ecs::prelude::*; // AssetServer, AssetLoader, Assets, Handle, ...
use boyko_ecs::ecs::core::asset::{AssetPaths, AssetStaging};
use boyko_render::Material; // its loader claims the "mat" extension

fn decode_only(server: &AssetServer, bytes: &[u8]) {
    // CPU-only: no file system, no device.
    let _cpu = server.decode_bytes::<Material>("mat", bytes);
}

fn load_one(
    server: &AssetServer,
    assets: &mut Assets<Material>,
    staging: &mut AssetStaging<Material>,
    paths: &mut AssetPaths<Material>,
) -> Handle<Material> {
    // Returns at once; the row is `Loading` until the upload pass fills it.
    server.load("assets/gold.mat", assets, staging, paths)
}
```

A read or decode failure does not panic. `load` still returns a well-formed
handle, and its row is marked `Failed`; poll `Assets::state(handle)` to see the
outcome. The upload half lives in `boyko_render`: an upload pass drains
`AssetStaging<A>` and `fill`s each reserved row. The windowed host inserts
`AssetServer` as a regular resource and, for `Material`, `MeshGpu` and
`TextureGpu`, the per-type `AssetStaging` / `AssetPaths` as **non-send**
resources. A system reaches those two through `NonSendResMut`, not `ResMut`, so
it runs on the dispatcher thread; see
[Non-`Send` resources](resources.md#non-send-resources). The host runs the
upload drains once at boot, after the startup systems.

## Change signals

Four counters let a GPU mirror decide what to re-upload without diffing the
table:

| Signal | Moves when |
|--------|-----------|
| `dirty_gen` | any `get_mut` resolves a live row |
| `high_water` | the table appends a fresh row |
| `install_epoch` | `add` installs a value — including into a **reused** row, which leaves `high_water` unchanged |
| `free_epoch` | a row is freed |

`install_epoch` is the one worth remembering: a mirror gated on row-count growth
alone cannot see free-list reuse, and would keep serving the old contents.

### The edited-row set

The counters say *that* something changed; the **edited set** says *which rows*.
It is one bit per row meaning "a GPU copy of this row may differ from the CPU
value", and it is the preferred re-upload signal:

- `get_mut`, `add` (fresh or reused row), `fill`, `remove` and `retire` mark the
  row. `fail`, `reserve` and a refcount reaching zero do not.
- `edited_any()` and `edited_count()` are O(1), so an idle frame costs one load.
- `drain_edited(|row, value| …)` visits every marked row in ascending order and
  clears the marks. `value` is `Some(&T)` for a `Loaded` row and `None` for any
  other state, which a mirror writes as zeros.

```rust,ignore
fn sync_mirror(mut materials: ResMut<Assets<Material>>) {
    if !materials.edited_any() {
        return; // nothing changed since the last drain
    }
    materials.drain_edited(|row, value| {
        // upload `value` (or zeros for `None`) into GPU row `row`
        let _ = (row, value);
    });
}
```

Marks persist until drained, however many frames pass.

### Other table methods

`iter` walks the `Loaded` rows with their handles. `state_of_index` and
`try_generation` read a row by raw index. `pin`, `inc_ref`, `dec_ref` and
`retire` are the refcount and retirement surface that `boyko_render` drives; you
rarely call them yourself.

## Performance characteristics

| Operation | Cost |
|-----------|------|
| `add` | O(1) — pop the free list or append |
| `get` / `get_mut` | O(1) — bounds check, state check, direct pointer |
| `get_by_index` | O(1), and skips the generation check by design (the render carrier's path) |
| handle | 8 bytes, `Copy`, no allocation, no refcount traffic |
| carrier component | `MaterialHandle` 2 bytes, `MeshHandle` 4 bytes, each plus a 4-byte generation lane |
| `edited_any` / `edited_count` | O(1) |
| `drain_edited` | O(words up to the last marked row) |

## See also

- [Resources](resources.md) — how `Assets<T>` reaches a system.
- [Components](components.md) and [Hooks & observers](hooks-and-observers.md) —
  what the carrier components are, and how attach/detach is observed.
- [Aether materials](../aether/materials.md) and
  [Aether scenes](../aether/scenes.md) — declaring assets and minting them
  through this table.
- [Rendering overview](../rendering/overview.md) — the consumer.
- Source: `crates/boyko_ecs/src/ecs/core/asset/` (`assets.rs`, `handle.rs`,
  `asset.rs`, `server.rs`, `loader.rs`, `paths.rs`),
  `crates/boyko_scene/src/render_caps.rs` (`MaterialHandle`, `MeshHandle`, the
  generation lanes), `crates/boyko_render/src/asset_refcount.rs`
  (`validate_asset_refs`, `AssetRefcountPlugin`),
  `crates/boyko_render/src/material.rs` (`MaterialId::from_handle`); design in
  `docs/ASSET-STREAMING-PLAN.md`.
