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

> **Caveat — the carrier has no generation.** Sixteen bits of index, and nothing
> else. A freed-and-reused slot therefore renders stale content silently, with
> no generation check anywhere on the GPU side. Until a later rung carries the
> generation (or a remap) into the render path, treat render-visible `Assets<T>`
> tables as **append-only / live-forever**: do not call `remove` on a handle a
> renderer may still hold.

Refcounting exists for the streaming path — a carrier component's `on_insert` /
`on_replace` hooks feed attach/detach counts that a `boyko_render` system folds
in — and slot 0 of the material table is **pinned**: the windowed host mints the
engine default material there at boot, and pinning keeps a refcount that reaches
zero from retiring the row every entity without an explicit material points at.

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

## Performance characteristics

| Operation | Cost |
|-----------|------|
| `add` | O(1) — pop the free list or append |
| `get` / `get_mut` | O(1) — bounds check, state check, direct pointer |
| `get_by_index` | O(1), and skips the generation check by design (the render carrier's path) |
| handle | 8 bytes, `Copy`, no allocation, no refcount traffic |
| carrier component | 2 bytes on the entity |

## See also

- [Resources](resources.md) — how `Assets<T>` reaches a system.
- [Components](components.md) and [Hooks & observers](hooks-and-observers.md) —
  what the carrier components are, and how attach/detach is observed.
- [Aether materials](../aether/materials.md) and
  [Aether scenes](../aether/scenes.md) — declaring assets and minting them
  through this table.
- [Rendering overview](../rendering/overview.md) — the consumer.
- Source: `crates/boyko_ecs/src/ecs/core/asset/` (`assets.rs`, `handle.rs`,
  `asset.rs`), `crates/boyko_scene/src/render_caps.rs` (`MaterialHandle`),
  `crates/boyko_render/src/material.rs` (`MaterialId::from_handle`); design in
  `docs/ASSET-STREAMING-PLAN.md`.
