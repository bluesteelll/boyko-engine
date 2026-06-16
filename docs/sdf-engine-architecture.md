# Dynamic SDF Game Engine — Architecture & Techniques Reference

> A technical breakdown of the rendering/world architecture used in Mike Turitzin's
> dynamic Signed Distance Field (SDF) game engine, intended as an implementation
> reference for building a similar engine. Physics-library specifics (Jolt) are
> intentionally excluded; the collision-mesh *generation* step is kept because it is
> an engine concern.
>
> Source video: *"I'm making a game engine based on dynamic signed distance fields (SDFs)"*
> by Mike Turitzin — https://www.youtube.com/watch?v=il-TXbn5iMA

---

## 1. Goal & Design Constraints

The engine exists to enable **high-fidelity, real-time modification of world geometry
during gameplay**. This is the single requirement that drives every architectural choice.

Key constraints derived from that goal:

- The **entire world is represented as SDFs** — terrain, props, and player-made edits alike.
- All geometry must be **dynamically modifiable at runtime**, not just authored offline
  (this is the explicit departure from *Dreams*, where SDF geometry is sculpted in-editor
  but not freely mutable during play).
- Modifications must support both **smooth** and **sharp-edged** add/remove of matter.
- Modifications must be **non-destructive** when desired (e.g. a moving hole, a tunnel that
  can be created, walked through, then removed).
- Must hold **high performance** and support **vast / open-world** spaces.

Design trade-off that falls out of this: the engine **optimizes for cheap recomputation of
changes**, accepting a somewhat higher rendering cost in exchange. (This is the reasoning
behind *not* using a triangle mesh for rendering — see §6.)

---

## 2. Core Data Model: Ordered List of "SDF Edits"

The scene is an **ordered list of SDF edits** (terminology borrowed from *Dreams*).

An **SDF edit** = a primitive shape with:

- **position, rotation, scale**
- a **boolean operation** applied against the accumulated world: `union`, `subtraction`,
  or `intersection`
- optionally a **smoothed** variant of the operation (smooth-min / smooth blend) to produce
  soft transitions instead of hard edges
- optional **material** information, including the ability to **volumetrically stamp material
  underground** (e.g. ore deposits revealed by digging)

Example: a sphere edit with a `subtraction` op cuts a spherical chunk out of the world at a
position.

Properties of this representation:

- Boolean ops are trivial and exact in SDF space — this is the core advantage of SDFs.
- The number of edits is effectively **unbounded**, because the engine only ever recomputes
  the spatial region a given edit touches (see §8).
- Both **additive** (building walls from cubes, drawing on surfaces) and **subtractive**
  (digging, tunneling) workflows are just edits with different ops.

---

## 3. Rendering Foundation: SDFs + Ray Marching (Sphere Tracing)

Baseline rendering is **ray marching / sphere tracing** of the signed distance function,
the standard Shadertoy-style technique. The surface is the zero-distance isosurface
(distance == 0); positive = outside, negative = inside.

**The problem this engine is built to solve:** brute-force ray marching evaluates a complex
distance function *many times per pixel*. For a scene with dozens of overlapping edits
influencing a single point, this hits a hard performance wall, even with culling of
irrelevant edits. Everything in §4–§7 exists to avoid re-evaluating the full distance
function per pixel.

---

## 4. Distance Caching on a Grid + Interpolation

Instead of evaluating the scene distance function from scratch per pixel, **evaluate it once
per grid point on a 3D grid and cache the results**, then reuse cached values when rendering.

**Reconstruction at an arbitrary point:**

1. Find which grid cell the point falls in.
2. Gather the distances at that cell's corner grid points (4 in 2D, 8 in 3D).
3. **Bilinear (2D) / trilinear (3D) interpolation** of those corner distances gives the
   distance estimate. In 3D this is a **single GPU texture fetch**.

**Accuracy characteristics:**

- Reconstruction is accurate where the surface is straight/near-straight at cell granularity.
- It is inaccurate near corners — corners visibly "wobble" as edits move; higher grid
  resolution reduces this.
- Trilinear interpolation of cached distances can reconstruct **surprisingly complex, curved
  surfaces with nontrivial topology** from a relatively coarse grid — which is what makes a
  lower grid resolution viable. (See the NVIDIA SDF-grid paper in References.)

This is the same family of technique used for **two decades to render sharp font glyphs at
any size** (Valve's 2007 alpha-tested magnification paper). For fonts the glyph SDF is cached
once because it never changes; here the challenge is that the field changes at runtime.

---

## 5. Compact Distance Storage

Cached distances are stored compactly rather than as full 32-bit floats:

- **1 byte per distance value.**
- Distances are clamped to the **minimum useful range = half the diagonal length of a grid
  cell**. Only the surface neighborhood matters (see §6), so representing larger distances is
  wasteful.

Even so, density is the killer: a dense `1024³` grid at 1 byte/value already costs **~1 GB**.
Dense storage only works for a tiny world → motivates sparse storage (§6).

---

## 6. Sparse Storage: Brick Map + Brick Atlas

Since only the **surface** is rendered, only cache cells that **contain the surface** matter —
specifically cells whose corner grid points evaluate to **both positive and negative**
distances (i.e. the isosurface passes through them). All other cells can be discarded.

Rather than an **octree** (the common choice for sparse grids), this engine uses a
**brick map**, chosen because it maps particularly well to the GPU:

- **Brick map** = a dense grid of **pointers to bricks**.
- A **brick** = a small cubic region of the cached distance grid. Bricks are **8×8×8**
  (size inspired by *Dreams*; works well in practice).
- Bricks are allocated from a large **3D texture atlas** (the "brick atlas"). Each texel =
  one cached grid point.
- Because bricks are 8³, the **pointer grid itself is trivially small** relative to the atlas.

Bricks are allocated/freed dynamically as geometry moves (only surface-containing regions
keep bricks).

### Why not generate a triangle mesh (marching cubes / dual contouring)?

Considered and rejected **for rendering**:

- A mesh is **cheap to render** but **more expensive to (re)generate** for a given fidelity.
- The engine's primary goal is a **high-fidelity dynamic world**, so it optimizes for **cheap
  recomputation on change**, accepting a higher render cost.
- (Note: a mesh *is* still used for collision — see §9 — where low resolution is acceptable.)

---

## 7. Vast Spaces: Level of Detail via Geometry Clipmaps

Sparse bricks reduce memory a lot but still don't scale to open worlds. LOD is required —
and critically, here LOD reduces not just geometry loaded but **how often the SDF is
evaluated** (far regions must be evaluated far less frequently).

Approach: **geometry clipmaps** (nested regular grids), adapted to brick-pointer grids:

- Instead of one brick-pointer grid, use a **set of nested grids**. Each successive level is
  **2× the size in every dimension** of the previous level.
- **All levels are centered on the player and follow player movement.**
- Net effect: the SDF is evaluated at **coarser resolution farther from the player**, keeping
  the **on-screen size of bricks roughly constant** regardless of distance. Bricks visibly
  double in size at each clipmap transition.

**Memory impact (engine's own figures):** for a scene with ~2.5 km draw distance, the
near-player resolution would need **~200 trillion** brick-map cells if applied uniformly;
with clipmaps only **~20 million** cells are needed — roughly a **10-million-fold reduction**.

---

## 8. Fast Incremental Updates (the core differentiator)

To support runtime edits cheaply, the engine **only regenerates the world regions that
actually changed**, never the whole world.

Mechanism:

- All SDF edits are tracked **spatially in a Bounding Volume Hierarchy (BVH)** — specifically
  a **tree of axis-aligned bounding boxes (AABB tree)**.
- The BVH is **shared between CPU and GPU** and updates dynamically as edits move.

The AABB tree is used for three things:

1. **Raycasts** — narrow down which edits to test for exact intersection as a ray sweeps.
2. **Evaluation culling** — query which edits can affect a region so the SDF evaluator does
   less work.
3. **Dirty-region detection** — determine exactly **which distance bricks must be
   re-evaluated** when an edit changes. Frame-to-frame, the vast majority of bricks are
   reused even while geometry is actively changing.

> Caveat stated by the author: this incremental scheme is **not strictly mathematically
> correct** in all cases, but is good enough for the interactions the engine targets. Worth
> keeping in mind if you push it into edge cases.

---

## 9. Collision Mesh Generation (physics library itself out of scope)

Physics needs collision geometry. Doing collision directly against the SDF is possible but
non-trivial and not supported out of the box by typical physics engines, so instead:

- Generate a **triangle collision mesh from the SDF**, fed to the physics engine **in chunks**.
- A mesh works fine **for physics** as long as **resolution is low** (this is the one place a
  mesh is used; rendering never uses one — see §6).
- Mesh generation uses **marching cubes** (chosen for simplicity and easy parallelization).
- Rendering-side SDF evaluation runs on the **GPU**; the **collision mesh is generated across
  multiple CPU threads** for low-latency updates to physics.
- Any SDF edit can be made a **dynamic physics body** that collides with the rest of the scene.
- Because the collision mesh updates **dynamically**, moving/subtracting geometry stays
  physically consistent (e.g. a moving hole can physically capture objects; a freshly dug
  tunnel is immediately walkable).

> The downstream physics engine integration (Jolt) is intentionally excluded from this
> document. Treat the collision mesh as an output handed to "some external physics engine".

---

## 10. Terrain (also an SDF)

Terrain is **not a heightfield** — it is **fully 3D**, generated as a sequence of SDF edits,
but **special-cased for efficiency**.

- Generated **progressively using octaves of noise** (fractal/fBm-style accumulation).
- Technique heavily inspired by an Inigo Quilez article (noise / fBm terrain).
- The **same algorithm with different parameters** yields anything from gentle to wild
  landscapes.
- Because terrain is just SDF edits, **all other edits interact/blend with it correctly**
  (e.g. cubes blending into a hillside).

---

## 11. Capabilities Unlocked (validation targets)

Useful as a feature checklist / acceptance tests for a similar engine:

- **Effectively unbounded number of edits** (via incremental recompute + LOD), enabling e.g.
  **digging kilometers below the surface**.
- **Non-destructive edits**: a "tunnel gun" = a capsule `subtraction` locked to the player's
  view; create → walk through → remove. Physics collision mesh updates with it, not just visuals.
- **Moving holes** that capture objects/enemies (collision mesh updates dynamically). Directly
  inspired by *Donut County*.
- **Volumetric material stamping** underground (ore deposits, layered materials).
- **Additive building** (e.g. wall out of cubes) and **surface drawing/painting**.
- **SDF modeling** at high enough fidelity to sculpt detailed objects, despite not being built
  as a modeling tool.
- Extensive **debug visualizations** (brick allocation, clipmap levels, regenerated bricks,
  BVH structure, raycast hits) — strongly recommended to build these early; they double as
  intuition-builders for each subsystem.

---

## 12. Subsystem → Technique Map (quick index)

| Subsystem | Technique / Data structure |
|---|---|
| World representation | Ordered list of SDF edits (shape + transform + boolean op) |
| Boolean combine | union / subtraction / intersection, with smooth-min variants |
| Base rendering | Ray marching / sphere tracing of the zero isosurface |
| Per-pixel cost reduction | Cache distances on a grid; bi/trilinear interpolation (1 texture fetch) |
| Distance storage | 1 byte/value, clamped to half cell-diagonal |
| Sparsity | Brick map (dense grid of pointers) + brick atlas (3D texture), 8³ bricks |
| Surface selection | Keep only cells straddling sign change (pos & neg corners) |
| Open-world LOD | Geometry clipmaps (nested grids, 2× per level, player-centered) |
| Edit tracking | CPU/GPU-shared BVH (AABB tree) |
| Runtime updates | Incremental regen of dirty bricks only |
| Collision | Marching-cubes mesh from SDF, low-res, multi-threaded CPU, chunked |
| Terrain | Fully-3D SDF from octaves of noise (fBm), special-cased |

---

## 13. References & Sources

**Primary source (the engine itself)**
- Mike Turitzin — *"I'm making a game engine based on dynamic signed distance fields (SDFs)"* (video): https://www.youtube.com/watch?v=il-TXbn5iMA
- Mike Turitzin — channel: https://www.youtube.com/@Mike-Turitzin · site: https://miketuritzin.com/ · X: https://x.com/miketuritzin

**SDF & ray-marching foundations**
- Sebastian Lague — *Coding Adventure: Ray Marching* (intro to SDFs + sphere tracing): https://www.youtube.com/watch?v=Cp5WWtMoeKg
- Inigo Quilez — articles index (SDFs, smooth-min, noise, terrain, etc.): https://iquilezles.org/articles/
- Inigo Quilez — 3D SDF primitive distance functions: https://iquilezles.org/articles/distfunctions/
- Inigo Quilez — fBm / value noise (terrain generation basis): https://iquilezles.org/articles/fbm/
- paniq — *"SDF Tracing Visualization"* (Shadertoy): https://www.shadertoy.com/view/lslXD8

**Cached-distance / interpolation rendering**
- Chris Green (Valve), 2007 — *Improved Alpha-Tested Magnification for Vector Textures and Special Effects* (sharp glyph rendering from cached SDFs): https://steamcdn-a.akamaihd.net/apps/valve/2007/SIGGRAPH2007_AlphaTestedMagnification.pdf
- Söderlund, Evans, Akenine-Möller (NVIDIA), 2022 — *Ray Tracing of Signed Distance Function Grids* (JCGT, Vol. 11 No. 3): https://jcgt.org/published/0011/03/06/

**Open-world LOD**
- Losasso & Hoppe, 2004 — *Geometry Clipmaps: Terrain Rendering Using Nested Regular Grids*: https://hhoppe.com/geomclipmap.pdf

**Inspiration / prior art (no direct paper, search terms given)**
- *Dreams* (Media Molecule, PS4) — SDF-based world; origin of "SDF edit" terminology and 8³ brick size. See Alex Evans' talk *"Learning From Failure"* (Umbra Ignite / SIGGRAPH 2015) for the rendering tech.
- *Donut County* — inspiration for the "moving hole captures objects" mechanic.
- Marching cubes — Lorensen & Cline, 1987 (classic isosurface polygonization; used here only for collision meshes).

---

## 14. Notes for Implementation Agents

- The architecture's novelty is **integration, not new algorithms** — every individual
  technique above is established. The defensible value is the *combination*: BVH-tracked edits
  → incremental dirty-brick regeneration → synchronized dynamic collision mesh, all under a
  brick-map + geometry-clipmap memory hierarchy, in service of a fully mutable world.
- **Order of implementation that de-risks early:** (1) basic SDF + sphere-tracing renderer →
  (2) grid distance cache + trilinear interpolation → (3) brick map + atlas (sparse) →
  (4) BVH over edits + incremental dirty-region regen → (5) geometry clipmap LOD →
  (6) marching-cubes collision mesh → (7) terrain as noise-driven SDF edits.
- Build **debug visualizations alongside each subsystem**, not after.
- Remember the explicit correctness caveat on incremental updates (§8): validate behavior on
  overlapping/rapidly-changing edits.

---

## 15. Hybrid Mesh + SDF: Render & Physics Integration

This section extends the engine to **combine classical mesh rendering/physics with SDF
geometry** for maximum flexibility at minimum overhead. The guiding principle:

> **Keep both representations native. Never fuse them into one. Convert lazily, locally, and
> only at the boundary where it is actually consumed. Route every draw/query to whichever
> representation answers it cheapest.**

Overhead is never literally zero (any hybrid pays at the implicit↔explicit boundary), but with
this principle it becomes **pay-for-what-you-use** — proportional to *changes* and *visible
area*, not world size.

### 15.1 Unified rendering (shared depth / deferred G-buffer)

- Rasterize meshes normally into a deferred **G-buffer** (depth, albedo, normal, material).
- In a following pass, **raymarch the SDF reading the same depth buffer** as a ray-distance
  bound → correct mesh↔SDF occlusion for free, no special-case logic.
- One shared lighting/shadow/post pass over both.
- **Tile-cull the raymarch:** using the brick map / clipmap, mark only screen tiles where SDF
  is actually present and march only those. Avoids paying for a full-screen raymarch.
- Bonus (hybrid is cheaper here, not just cheaper to combine): SDF gives **soft shadows and AO
  nearly free** via cone/sphere tracing, so **mesh objects can sample the SDF** for soft
  shadowing/occlusion that a pure mesh pipeline pays dearly for.

### 15.2 Mesh characters with SDF-cut holes

Goal: a **detailed mesh character** in which you can cut **arbitrary SDF holes**, performantly,
**without ever converting the full character to SDF**. That avoidance is the source of the speed.

**Core trick — cut in bind-space (rest pose), not world space:**

1. On hit, transform the impact point **back into the character's undeformed bind-space** via
   inverse skinning (which bones/weights at that point).
2. Store a **cutter SDF** there (typically a union of spheres/capsules).
3. Skinning then carries **both the mesh and the holes** into the animated pose simultaneously,
   so a hole in the arm stays in the arm as it bends — no extra logic. All boolean work happens
   where the character is rigid.

**Rendering the holes (cheapest layer):**

- Mesh rasterizes as usual. In the fragment shader, transform fragment position back to
  bind-space (pass bind-space coords as a vertex attribute and interpolate) and sample the
  cutter SDF. If `distance < 0` (inside hole) → `discard`. This is per-pixel boolean
  subtraction: one small-field eval + discard, **no mesh regeneration**, **constant in mesh
  complexity**.
- Naive discard gives hollow see-through holes. Two cost tiers:
  - **Cheap / hollow:** render backfaces (two-sided) so the cut shows the far inner wall. Great
    for robots, armor, thin-walled shells.
  - **Solid cross-section (visible "fill", like cut flesh):** bake a **low-res interior volume
    SDF** of the character **once, offline**, in bind-space (mesh→SDF via jump-flooding /
    winding number). In hole pixels only, **raymarch `subtract(charSDF, cutterSDF)`** to find
    the cap surface; normal = field gradient; shade with a separate "interior" material.
    Restrict the raymarch to hole pixels via stencil/discard mask → near-free. Low resolution
    is fine: outer detail comes from the mesh, the SDF is visible only on the cut cap.

**Physics of the holes — cost ladder (use the lowest tier that satisfies the gameplay):**

1. **Queries (near-free):** raycasts, hit tests, "did the shot pass through an existing hole",
   character controller → evaluate `subtract(charSDF, cutterSDF)` analytically in bind-space.
   No mesh.
2. **Local collision regen:** if things must physically fall through the hole, re-run marching
   cubes **only on dirty bricks near the hole** (same incremental BVH/dirty-region scheme as
   §8/§9) and hand to the solver. A chunk rebuilds, not the whole body.
3. **Proxy colliders:** animated characters usually use per-bone **capsules**, not the full
   mesh. A hole affects physics only if "significant": cut a capsule → either ignore (visual-
   only hole) or, if the hole is through-and-through, **detect disconnection**
   (connected-components / flood-fill on the field) and **split the character into separate
   bodies** = limb severing.
4. **Volumetric soft body (XPBD/FEM on tetrahedra):** holes genuinely alter the sim by removing
   particles/tetrahedra inside the cutter. Most physically honest, **most expensive** — only if
   body destructibility is central.

Typical real choice: **tier 1 for everything analytic + tier 3 for severing**, dropping to 2/4
only when required.

**Where the honest overhead lives:** the one-time interior-SDF bake (small 3D texture per
character, memory); the **seam** where mesh surface meets SDF cap (blend normals or a hard edge
shows); approximate bind-space evaluation in heavily-skinned joint zones (fine for visuals, can
lie for precise joint physics). Through-cuts (topological body splitting) are the only genuinely
expensive case, because the **number of bodies changes**.

### 15.3 Two-way conversion on demand

- **mesh → SDF** (distance bake, jump-flooding): make an imported polygonal model carvable.
- **SDF → mesh** (marching cubes / dual contouring): for static decorative geometry that never
  changes and is cheaper to rasterize once than to raymarch every frame.

---

## 16. Destructible Routing: Hybrid vs. Fragment-Based (e.g. UE Chaos)

**There is no single "hybrid is X% faster than Chaos" number — none is published, and any flat
figure is fabricated.** The correct comparison is **per-component and per-scenario**, because
the two approaches stress *different resources* and win in *different cases*.

### 16.1 Critical methodological note (resource budget)

- **Fragment-based rigid destruction (Chaos) is mostly CPU-bound** (rigid-body constraint solver).
- **SDF carving + raymarch is mostly GPU-bound.**
- A hybrid can be "faster" simply by **moving work off a saturated CPU onto a free GPU**, a real
  systemic win even when total operations aren't fewer. Always ask *which budget is scarce*.

### 16.2 Cost decomposition (where each wins)

| Component | SDF / hybrid | Fragment (Chaos) | Winner |
|---|---|---|---|
| Cut-geometry generation | Cutter-SDF in shader, ~const in mesh complexity, sub-ms GPU, arbitrary shape | Fracture **prebaked offline** (0 runtime) but **shape fixed in advance**; runtime refracture = tens of ms, not its profile | **Hybrid** for arbitrary cuts (categorical) |
| Narrow-phase (contact generation) | depth = field value, normal = gradient; great for particles/soft/fluid | GJK/EPA on convex pieces, well optimized | Hybrid for continuum/soft; ~parity for convex rigid |
| Broad-phase (which pairs are near) | still needs BVH/spatial hash | same | **Parity** |
| Constraint solver (resolve many-body contacts) | **not accelerated by SDF at all** | mature: clustering, declustering on-demand, sleeping, baked caches | **Chaos** |
| Rendering the result | raymarch (per-pixel) or shell + SDF cap | rasterize many small shards (draw calls, overdraw) | ~comparable |

**Key insight:** SDF can cheapen *contact generation* but does **nothing** for *contact
resolution* among many bodies — and large rigid destruction is **dominated by the solver**.

### 16.3 Per-scenario verdict (reasoned order-of-magnitude, NOT measured)

| Scenario | Faster | Order |
|---|---|---|
| Clean arbitrary hole, body stays whole (drill, burn, tunnel) | **Hybrid** | Categorical: hybrid ~O(1)/frame on GPU; Chaos needs runtime refracture (tens of ms) or can't do it → effectively 10–100×+ or "Chaos N/A" |
| Continuum: melting, digging, smearing, soft-body | **Hybrid** | Large; Chaos is paradigmatically wrong for this (would emulate via thousands of shards → solver blows up) |
| Shatter into many rigid shards, pile, stacking | **Chaos** | Tuned hybrid ~parity (0.5–1×) at best; **naive hybrid is realistically 2–5× slower** — bottlenecked by an immature solver/broad-phase |
| Topological body splitting (body count changes) | Parity | Expensive for both; representation-independent |

### 16.4 Honesty disclaimer (do not skip)

Chaos is a **mature, years-optimized, shipped-in-production** system. "Faster in a microbenchmark
I designed" ≠ "faster in a real project." On **Chaos's home turf** (rigid shatter) a custom
hybrid will almost certainly lose until comparable years go into its solver. The hybrid's real,
large advantage is **outside** that turf — carving and continuum — not inside it.

### 16.5 Routing rule for the engine (the actual takeaway)

Do **not** pick one system. **Route by destruction type:**

- **Carving / burning / drilling / digging / clean holes in whole bodies → SDF path.**
- **Continuum (melt / soft / fluid / smear) → SDF/particle path.**
- **Shatter into free rigid shards with inter-shard physics → mature rigid-body solver path.**
- **Chain them:** let the SDF define *where* the cut passes (near-free), then **spawn fragments
  along the cut line** into the rigid solver where flying debris is needed.

This "SDF decides *where*, solver decides *how debris behaves*" split is the strongest design —
it uses each tool only on the task it dominates.

### 16.6 References for this section

- Unreal Engine — Chaos Destruction (fracture, Geometry Collections, clustering): https://dev.epicgames.com/documentation/en-us/unreal-engine/destruction-in-unreal-engine
- Jump Flooding Algorithm (mesh→SDF baking) — Rong & Tan, 2006: https://www.comp.nus.edu.sg/~tants/jfa/i3d06.pdf
- Gradient of an SDF as contact normal / general SDF collision — see Inigo Quilez articles (§13).
- XPBD (position-based soft/rigid solving) — Macklin et al., 2016: https://matthias-research.github.io/pages/publications/XPBD.pdf
