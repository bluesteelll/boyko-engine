> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

The codebase facts confirm the critics' findings. I have enough verified context (exclusive system is `FnMut(&mut EcsMaster)`, the consistency window, the hierarchy model). I'll now produce the full revised plan addressing every critical and major finding.

The key redesigns:
1. **Split discovery (SystemParam, has the tick window) from layout (exclusive)** — fixes the unbuildable Changed-scan.
2. **Normal marker component for `UiRoot`** + cache roots in scratch, refresh on `Added` — fixes un-enumerable bitset and per-frame alloc.
3. **Depth-indexed scratch pools** — fixes recursion clobbering.
4. **Explicit pass structure** (intrinsic measure → definite-size resolve → distribute) with Auto-cross fold, Pct/Stretch ordering — fixes layout-algorithm criticals.
5. **`mem::take` borrow protocol** for scratch resource.
6. **Single pointer-resolution per node**, honest cost model.

Here is the full revised plan.

---

# Architecture: boyko_ui Phase P1 — Layout components + the in-house layout system over ECS

## Goal

Stand up the `boyko_ui` crate and deliver the **layout foundation**: a set of ECS components (layout inputs + computed outputs) and a **two-system pair** (a change-detecting *discovery* system + an exclusive *tree-walk* system) that runs an in-house reimplementation of a CSS-flexbox-style multi-pass layout algorithm over the existing `ChildOf`/`Children` tree, writing `ComputedRect{x,y,w,h}` to every laid-out widget.

Performance targets:
- **0 heap allocations per frame** in the steady state of BOTH systems — every scratch buffer is preallocated in a `Resource`-owned `LayoutScratch` (seeded with `with_capacity`, reused, only `clear()`/index-reset per frame). The discovery system writes its dirty set into that preallocated buffer. **No `query_entities` on any per-frame path** (it allocates a fresh `Vec` per call — `ecs_master.rs:2096`).
- **~0 cost when nothing changed** — the discovery system is a real scheduled `FunctionSystem`; its `Changed<T>` scan yields nothing in steady state (Phase-10 0%-overhead path), it writes an empty dirty set, and the exclusive layout system early-returns on an empty dirty set.
- **Hot path**: a dirty root subtree of N nodes lays out in O(N) node-resolutions plus the per-container stretch freeze loop (≤ S iterations for S stretch items per container) plus the documented re-layout cost of nested Auto/Stretch/Pct subtrees (bounded by **depth × stretch-nesting**, see §Honest pass count). No `dyn`, no `HashMap`, no per-frame heap.
- **Honest per-node cost model**: each node visit is a **multi-hop pointer chase** through the sparse `entities_inland` store (keyed by entity id, no spatial correlation to tree order — `ecs_master.rs:1759-1996`), NOT an O(1) free access. We minimize this by resolving each node's archetype/row pointer **once per visit** and reading all of its columns from that single resolution (via the batched raw accessor), instead of N independent `get_component` lookups. Archetype-order SoA traversal is the documented future optimization seam.
- The output column `ComputedRect` is `#[repr(C)]`, 16 B; each per-node write is **one aligned 16 B store** (good for the later P5a GPU upload). Streaming/non-temporal writes are NOT claimed in P1 — the targets are scattered across archetypes (each `ComputedRect` at `column_ptr + idx*stride` in whatever archetype the node lives in), so a streaming run does not exist; it is a future contiguous archetype-order pass.

## Context and constraints

- **Affected subsystems**: NEW crate `boyko_ui` (no changes to `boyko_ecs` core). Consumes `ChildOf`/`Children` (Phase 19), `Changed`/`Added` (Phase 10), `Query`/`Res`/`ResMut` SystemParams, exclusive systems, `ScheduleBuilder`.
- **Invariants preserved**: Principle 0 (no parallel data system — the tree is `ChildOf`/`Children`, props/outputs are ECS columns, scratch is a `Resource`-owned engine buffer that is **strictly frame-transient** — it holds only `Entity` handles + POD work items, is fully reset every frame, and **never** caches per-node durable layout state across frames; any persisted state is an ECS column). Principle 1/5 (no per-frame alloc after warmup, no `dyn` on the hot path). Principle 7 (rare/diagnostic arms marked `#[cold]`/`#[inline(never)]`). Principle 8 (`unsafe` justified — **this phase needs none**).
- **Consistency window** (`hierarchy/mod.rs:22-27`): `Children` is consistent with `ChildOf` only after the deferred-command drain at the apply window. The layout pair reads `Children`/`ChildOf` and therefore observes structural changes from **prior** apply windows. The pair is scheduled AFTER the structural-mutation systems. Documented.
- **Sibling order is non-deterministic** (`Children` uses `swap_remove`). Layout MUST sort children to a stable flow order itself (Decision 4).
- **Coordinate space**: top-left origin, +x right, +y down, units = logical pixels (DPI applied at upload by P5a, not by layout).

## Key decisions

### Decision 1: A two-system pair — a scheduled change-detecting **discovery** system + an **exclusive** tree-walk system

**What**: Layout is **two** systems, scheduled in order:

1. `ui_layout_discovery(changed: Query<Entity, Or<(Changed<UiLayout>, Changed<UiSpacing>, Changed<UiAlign>, Changed<UiAbsolute>, Changed<ContentSize>, Changed<Children>, Changed<ChildOf>, Added<UiRoot>)>>, q_childof: Query<&ChildOf>, q_root: Query<(), With<UiRoot>>, mut scratch: ResMut<LayoutScratch>, viewport: Res<UiViewport>)` — a **normal `FunctionSystem`** (SystemParams give it the `(last_run, this_run]` window that `Changed`/`Added` require). It computes the dirty-root set into `scratch.dirty_roots` (preallocated buffer) and never allocates.
2. `ui_layout_apply(world: &mut EcsMaster)` — an **exclusive system** (`FnMut(&mut EcsMaster)`, the only form `ExclusiveFunctionSystem` accepts — `exclusive_function_system.rs:209`). It reads `scratch.dirty_roots`, then runs the recursive tree walk with nested parent↔child mutable row access.

**Why the split**: The critic correctly identified that Decisions 1 and 2 of the original plan were mutually exclusive. Verified:
- An exclusive body `(self.func)(world_ref)` has **no access to its own `(last_run, this_run]` window** (it lives on `self.meta`, not on `EcsMaster`), so it cannot evaluate `Changed`.
- `EcsMaster::query` is **compile-time-forbidden** from containing `Ref/Mut/Added/Changed` (the `const { eval_query_no_change_detection::<D,F>() }` guard, `ecs_master.rs:3635-3659`) — change detection "requires Schedule context; use `Query<D,F>` as a SystemParam inside a system body."

Therefore the change scan MUST live in a scheduled (non-exclusive) `FunctionSystem` where SystemParams supply the window. The recursive tree walk MUST be exclusive (only `&mut EcsMaster` expresses nested parent↔child mutable row access without unsafe aliasing). Splitting them satisfies both constraints with zero unsafe and zero compromise.

**How the two communicate without a parallel data system**: through the `LayoutScratch` **Resource** (`ResMut` in discovery, fetched via `get_resource_mut` in the exclusive walk). This is engine-owned `Resource` storage (Principle-0 legitimate), not a side `std::Vec`. The dirty set is **frame-transient**: discovery clears and refills `dirty_roots` every frame; the exclusive walk consumes it; nothing persists per-node across frames.

**Why the recursion is still exclusive (not a `Query`)**: the algorithm is a recursive tree walk with read-then-write interleaving on the SAME entities (a parent reads a child's measured size, then writes the child's rect). A parallel `Query` cannot hold `&mut ComputedRect` to a child while recursing into that child. Exclusive `&mut EcsMaster` makes each `get_component_mut` a fresh, short-lived borrow with zero unsafe. Parallelism (over independent roots) is a deferred, measured-need optimization.

**Alternatives rejected**:
- Single exclusive system doing its own Changed scan — **does not compile and has no tick window** (the verified contradiction above).
- A persisted `EnableTag LayoutDirty` bit read by the exclusive walk via `is_enabled` — viable (the exclusive body CAN probe a bitset bit), but it pushes dirty-marking into every structural/prop-mutation site, requires a clear pass, and adds a Phase-22 dependency. The discovery-system split gets the dirty set "for free" from Phase-10 ticks with no new component and no scattered marking. Kept as the documented fallback if discovery ever shows up in a profile.

**Trade-off**: two systems and one cross-system Resource handoff instead of one system. The handoff is a single `Resource` (engine storage), and the two-system shape is the *only* buildable form given the verified API constraints.

### Decision 2: Dirty-root discovery via Changed-scan + mark-up-to-root, relayout whole dirty root subtrees

**What**: Each frame `ui_layout_discovery` iterates its `Changed`/`Added` query. For each changed node it walks `ChildOf` up to the `UiRoot`-tagged ancestor and pushes that root into `scratch.dirty_roots` (deduplicated by linear scan — root count is tiny). On viewport resize it marks **all** roots dirty (see §Changed-gating). `ui_layout_apply` fully re-lays-out each dirty root's subtree. Unchanged roots are never visited.

**Why whole-root granularity** (the correctness backbone, must survive the redesign): a change at any depth can affect ancestors (Auto sizing flows **up**) and descendants (Stretch/Pct flows **down**). The only correct cheap unit is "the enclosing root subtree." Marking up to root and relaying the whole root subtree is correct by construction — there is **no per-node Changed gate inside the walk**, so no up/down ripple can be missed. Sub-root incremental layout is fragile (up-propagation of Auto) and is explicitly deferred.

**Why it is cheap enough**: per-node work is a single pointer resolution + a handful of column reads + f32 math + one `ComputedRect` store. A HUD root of ≤ a few hundred nodes re-lays in single-digit-to-tens of µs, and only on frames where that root's subtree changed. Steady-state frames cost only the discovery scan (Phase-10 0%-overhead). **Updated estimate with the corrected pass structure** (§Honest pass count): the extra intrinsic-measure / definite-size passes multiply the per-node constant by a small factor (≈2–3× for subtrees with nested Auto/Stretch/Pct, 1× for fixed-size subtrees); a 1000-node mixed root therefore lands in the low tens of µs, still well within budget for a change-frame.

**Structural-change soundness** (critic's gap closed):
- **Child add/remove**: link/unlink mutates the parent's `Children` at the apply window → `Changed<Children>` on the parent row → up-walk from the parent reaches its root. Covered.
- **Reparent across roots** (the gap): moving a subtree from parent under root A to parent under root B mutates **both** the old and new parent's `Children` (`Changed<Children>` on both), AND the moved child's `ChildOf` (`Changed<ChildOf>` on the child). The scan term set therefore includes **both** `Changed<Children>` and `Changed<ChildOf>`. The up-walk from each changed endpoint reaches the correct root: from old parent → root A (reclaims vacated flow space), from new parent → root B (accommodates the arrival), and from the moved child via its NEW `ChildOf` → root B. Both roots get marked. **An explicit reparent-across-roots test is mandatory** (§Tests).
- **Despawn of a middle node**: recursive despawn (Phase 19) unlinks the node from its parent's `Children` (mutating it) → `Changed<Children>` on the parent → root marked. **An explicit despawn-of-middle-node test is mandatory** (§Tests).

**Why `set-if-changed` on the output never masks a needed relayout**: layout's dirty detection keys off **input** components (`UiLayout`/`UiSpacing`/`UiAlign`/`UiAbsolute`/`ContentSize`/`Children`/`ChildOf`), never off `Changed<ComputedRect>`. So suppressing a bit-identical `ComputedRect` write (Decision 5) cannot hide a relayout from the discovery system. And because the relayout unit is the whole subtree (no per-node gate), an Auto-content up-propagation that lands a child at an identical rect still re-walks that child. This property is the correctness backbone and is preserved exactly by the two-system design.

**Alternatives rejected**: `EnableTag LayoutDirty` persisted bit (fallback, Decision 1); per-node incremental (deferred, fragile).

**Trade-off**: a one-node change relays its whole root subtree. Mitigated by small node counts and branch-light f32 math. Sub-root incremental is a future phase.

### Decision 3: Inline the morphorm/flexbox trait split — no `Node`/`Cache`/`Store` traits, no `dyn`

**What**: The algorithm is written directly against ECS columns. "Store" = a single per-node pointer resolution then column reads; "Tree" = `Children::as_slice()` (sorted); "Cache" = a `ComputedRect` write. Axis folding (main/cross ↔ x/y/w/h) is a private inline helper, not a trait.

**Why**: those traits decouple the algorithm from any host's storage. Our host is fixed (the ECS), so they are pure indirection. Inlining yields monomorphic direct calls (Principle 1), a smaller I-cache footprint, and no generic-trait boilerplate.

**Trade-off**: the algorithm is boyko-specific, not a reusable standalone crate. Intended.

### Decision 4: Stable flow order = sort children by `Entity::id()` into depth-indexed reused scratch

**What**: Because `Children` order is non-deterministic, each container's children are copied into a **depth-indexed** reused scratch slice (Decision 6) and sorted by `Entity::id()` ascending for a deterministic flow order.

**Why**: layout must be deterministic frame-to-frame (a `swap_remove` of an unrelated sibling must not reorder the visual flow). `Entity::id()` is stable for a live entity. The sort happens into preallocated scratch, not a fresh `Vec`.

**Trade-off**: flow order = id (spawn) order, not insertion order, not author-controllable beyond spawn order in P1. An explicit `UiOrder(u32)` sort key is filed for P2. Documented.

### Decision 5: Component granularity — split by churn; output split `ComputedRect`/`StackIndex`/`ComputedClip`

**What**: Three hot input components (`UiLayout`, `UiSpacing`, `UiAlign`), one cold opt-in input (`UiAbsolute`, only on absolutely-positioned nodes), output `ComputedRect` (hot, every node) + `StackIndex` (cold, author-owned) + `ComputedClip` (cold, author-owned in P1).

**Why**: change detection is per-component-per-row. Splitting by churn profile means a node animating only its size (`UiLayout`) does not bump `UiSpacing`/`UiAlign` ticks, keeping the discovery scan tight. Absolute offsets live on a separate `UiAbsolute` so the 99% relative nodes carry no dead fields (smaller rows, better D-cache density).

**Set-if-changed scope**: P1 layout **only writes `ComputedRect`**, and writes it set-if-changed. `StackIndex` and `ComputedClip` are **author-owned** in P1 (layout never writes them), so layout never bumps their ticks. The 0%-overhead guarantee is scoped to exactly `ComputedRect`.

**Set-if-changed mechanism (tick-correct)**: `get_component_mut` returns a `Mut` guard that bumps `changed_tick` on **any mutable deref** (`ecs_master.rs:1936`). To avoid bumping on an equal write, the compare reads through a **shared** accessor first: `let new = compute(...); if world.get_component::<ComputedRect>(child) != Some(&new) { *world.get_component_mut::<ComputedRect>(child) = new; }`. The mutable guard is acquired **only** when the values differ. (Implemented as: resolve the node's row pointer once, read the current rect by shared ref, compare, and take the mut guard solely on inequality.)

**Trade-off**: more component ids and archetype variety. Acceptable — these are the canonical churn boundaries and serve the 0%-overhead gate.

### Decision 6: Depth-indexed scratch pools — the working buffers must survive recursion

**What**: `child_buf` and `stretch_buf` are **NOT** single shared `Vec`s. They are **depth-indexed pools**: `child_pool: Vec<Vec<Entity>>` and `stretch_pool: Vec<Vec<StretchItem>>`, each preallocated to `MAX_LAYOUT_DEPTH` inner `Vec`s. `layout_node(node, depth, ...)` uses `child_pool[depth]` and `stretch_pool[depth]` for its own working set.

**Why (critic critical)**: `layout_node` recurses **while its working set is live** — Phase 1 resolves an Auto child by recursing, and that recursion would clear-and-refill the parent's shared `child_buf`/`stretch_buf`, corrupting the parent's loop on any tree deeper than two Auto/Stretch levels. Depth-indexing gives each recursion level its own buffer. Each inner `Vec` is reused across frames (cleared on entry, capacity retained), so there is still **zero per-frame alloc** after warmup. `MAX_LAYOUT_DEPTH` is a fixed const (e.g. 128); the depth guard `debug_assert!(depth < MAX_LAYOUT_DEPTH)` doubles as the cycle/pathological-depth guard. If `depth >= MAX_LAYOUT_DEPTH` in release, the node is treated as a leaf (no further recursion) to avoid OOB — a `#[cold]` diagnostic path.

**Why a pool, not stack arrays**: fanout is unbounded (a container can have thousands of children), so a fixed stack array would either overflow or cap fanout. A depth-indexed heap pool, grown once to high-water and reused, is the alloc-free way to give each level an unbounded-but-reused buffer.

**Trade-off**: `MAX_LAYOUT_DEPTH` inner `Vec`s preallocated (most empty most of the time — a few KB of pointers, negligible). A tree deeper than `MAX_LAYOUT_DEPTH` is clamped (debug-asserted). Acceptable: real UI trees are shallow.

### Decision 7: `mem::take` borrow protocol for the scratch resource

**What**: `ui_layout_apply` does `let mut scratch = std::mem::take(world.get_resource_mut::<LayoutScratch>())` at entry (moving the buffers out into a stack local, leaving an empty `LayoutScratch` in the resource slab), runs the **entire** recursion against the stack-local `scratch`, then `*world.get_resource_mut::<LayoutScratch>() = scratch` before return (moving the buffers — with retained capacity — back).

**Why (critic minor → load-bearing)**: a `&mut LayoutScratch` borrowed from the resource slab cannot be held across `world.get_component_mut(...)` calls on the same `&mut EcsMaster` (borrow conflict). `mem::take` frees the world borrow for the recursion's `get_component`/`get_component_mut` while keeping the buffers (and their capacities) alive on the stack. The take/put-back is two `Resource` accesses per frame (negligible) and moves only the `Vec` headers (no element copy, no alloc — capacity travels with the moved `Vec`). `LayoutScratch: Default` (all-empty) makes `mem::take` valid.

**Trade-off**: the buffers are unavailable in the resource slab during the walk (irrelevant — only this system touches them). Clean, zero-unsafe, alloc-free.

---

## Crate skeleton

`crates/boyko_ui/Cargo.toml`:
```toml
[package]
name = "boyko-ui"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
name = "boyko_ui"
path = "src/lib.rs"

[dependencies]
boyko-ecs    = { path = "../boyko_ecs" }
boyko-macros = { path = "../boyko_macros" }
boyko-utils  = { path = "../boyko_utils" }
# NOTE: boyko-threadpool is NOT a dependency in P1 — the apply pass is exclusive;
# the demo already owns the pool to build the schedule.
```
- No `[lints] workspace = true` block (matches the leaf-crate convention).
- Add `"crates/boyko_ui"` to the root `Cargo.toml` `[workspace].members`.

`crates/boyko_ui/src/lib.rs` module layout:
```rust
//! boyko_ui — ECS-native UI. P1: layout components + the in-house layout systems.
//! Widgets are entities; layout inputs/outputs are components; the tree is
//! ChildOf/Children; layout is two systems over the ECS. No parallel data system.

pub mod units;       // Unit, LayoutType, PositionType, AlignMain, AlignCross
pub mod components;  // UiLayout, UiSpacing, UiAlign, UiAbsolute, ComputedRect,
                     //   StackIndex, ComputedClip, UiRoot, ContentSize
pub mod layout;      // ui_layout_discovery + ui_layout_apply + LayoutScratch + algo
pub mod resources;   // UiViewport, LayoutScratch

pub mod prelude {
    //! Crate-local convenience re-exports (no engine-wide prelude).
    pub use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};
    pub use crate::components::{
        ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign,
        UiLayout, UiRoot, UiSpacing,
    };
    pub use crate::layout::{ui_layout_apply, ui_layout_discovery};
    pub use crate::resources::{LayoutScratch, UiViewport};
}
```
- `LayoutScratch`'s internal buffers and the private `fn layout_node(...)` are not re-exported (internal). `LayoutScratch` the type is exported only so the host can `insert_resource(LayoutScratch::with_seeds())` at setup.

---

## Component & type definitions

### Units (`units.rs`)

```rust
/// A length on one axis. 4-unit flexbox model (no viewport units in P1).
/// `Copy` + `repr(C)` (a tag byte + an f32 payload, 8 B). Hot: read for every
/// sized node on every measurement.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    /// Logical pixels.
    Px(f32),
    /// Percentage of the parent's resolved DEFINITE axis extent (content box).
    /// `%` of an indefinite (Auto) parent axis resolves to Auto/content per CSS,
    /// NOT zero-of-final (see §Auto/Pct ordering). Not clamped to 0..=100.
    Pct(f32),
    /// Flex-grow factor. Consumes free main-axis space proportional to factor;
    /// also valid on gaps (parent-applied stretch spacing).
    Stretch(f32),
    /// Intrinsic: hug content (container = fold of children; leaf = ContentSize).
    Auto,
}
impl Default for Unit { fn default() -> Self { Unit::Auto } }
```
**Why `repr(C)` enum**: deterministic 8-byte layout (1-byte discriminant padded + f32), keeps `UiLayout` compact. **No `Vw/Vh/VMin/VMax`** in P1 — purely additive later (new variants resolved against `UiViewport`).

```rust
/// Container layout direction. `Grid` RESERVED — P1 falls back to Column with a
/// `#[cold]` debug_assert (see §Out of scope). 1-byte repr.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayoutType {
    Row,            // main = x (width), cross = y (height)
    #[default]
    Column,         // main = y (height), cross = x (width)
    Overlay,        // children share the container box; positioned by align
    Grid,           // RESERVED — P1 falls back to Column
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PositionType { #[default] Relative, Absolute }

/// Main-axis distribution of leftover free space (only when no Stretch consumes
/// it — see §AlignMain precedence). 1-byte repr.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignMain {
    #[default] Start, Center, End, SpaceBetween, SpaceAround, SpaceEvenly,
}

/// Cross-axis placement of each child within the container's cross extent.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignCross { #[default] Start, Center, End, Stretch }
```

### Input components (`components.rs`)

```rust
/// Primary layout input. HOT: read for every node every pass. ~68 B, one line.
/// `Changed<UiLayout>` is the primary relayout trigger.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct UiLayout {
    pub layout_type: LayoutType,
    pub position_type: PositionType,
    pub width: Unit,      pub height: Unit,
    pub min_width: Unit,  pub min_height: Unit,
    pub max_width: Unit,  pub max_height: Unit,
}
impl Default for UiLayout {
    fn default() -> Self {
        Self {
            layout_type: LayoutType::Column,
            position_type: PositionType::Relative,
            width: Unit::Auto, height: Unit::Auto,
            min_width: Unit::Auto, min_height: Unit::Auto,  // Auto min = content
            // Unbounded sentinel is f32::MAX (FINITE), NOT INFINITY — see below.
            max_width: Unit::Px(f32::MAX), max_height: Unit::Px(f32::MAX),
        }
    }
}
```
**Unbounded-max sentinel is `f32::MAX`, not `f32::INFINITY`** (critic minor, adopted): an `INFINITY` upper bound can produce `NaN` through `0.0 * INFINITY` in a degenerate stretch round or `Pct`-derived intermediate; a `NaN` rect would never compare-equal under derived `PartialEq` (so set-if-changed would bump the tick **every frame forever**, defeating 0%-overhead) and would corrupt geometry. `f32::MAX` is clamp-equivalent for all realistic sizes, is finite, and cannot introduce `INFINITY`-arithmetic `NaN`. A `#[cfg(debug_assertions)]` `debug_assert!(rect.x.is_finite() && rect.y.is_finite() && rect.w.is_finite() && rect.h.is_finite())` precedes every `ComputedRect` write as a backstop.

```rust
/// Parent-applied spacing. HOT on containers, cold/absent on leaves. ~96 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]
pub struct UiSpacing {
    pub padding_left: Unit,  pub padding_right: Unit,
    pub padding_top: Unit,   pub padding_bottom: Unit,
    pub border_left: Unit,   pub border_right: Unit,    // layout inset only
    pub border_top: Unit,    pub border_bottom: Unit,
    pub row_gap: Unit,       // gap between children when main = y (Column)
    pub column_gap: Unit,    // gap between children when main = x (Row)
    // Gaps may be Stretch (parent-applied stretch spacing) — handled by the
    // freeze loop. `margin` (child-applied) is DEFERRED (§Out of scope).
}
impl Default for UiSpacing { /* all Unit::Px(0.0) */ }
```
**Border is a layout inset** (insets the content box) independent of the visual border (P5a `UiBackground`). Default `Px(0.0)` = branch-free no-op.

```rust
/// Alignment of children. COLD: read once per container; often default. 2 tag B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiAlign { pub main: AlignMain, pub cross: AlignCross }
```

```rust
/// Absolute (self-directed) offsets. COLD + OPT-IN: present ONLY on nodes whose
/// UiLayout.position_type == Absolute. before (left/top) wins over after.
/// Auto = "unset". `Changed<UiAbsolute>` is a scan term.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiAbsolute {
    pub left: Unit,  pub right: Unit,
    pub top: Unit,   pub bottom: Unit,
}
```
Default for all four = `Unit::Auto` (unset).

```rust
/// Leaf intrinsic size (the content_size seam). COLD, OPT-IN. In P1 (no text
/// shaping — P5b) this is an authored/image-derived fixed size that Auto leaves
/// hug. Layout only READS it. `Changed<ContentSize>` triggers relayout.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ContentSize { pub width: f32, pub height: f32 } // 0.0 if none
```
**P1 Auto-without-ContentSize**: an Auto leaf with no `ContentSize` and no children → `0×0` on that axis (documented). With `ContentSize` → hugs it. P5b replaces the *source* of `ContentSize`; the layout code is unchanged.

### Output components (`components.rs`)

```rust
/// The resolved screen-space rectangle — the ONLY geometry the renderer reads
/// (P5a). HOT: written for every laid-out node. `#[repr(C)]`, 16 B = one aligned
/// store target. NaN never appears (clamps + f32::MAX sentinel + finite-assert).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ComputedRect {
    pub x: f32,  // top-left x, logical px (+x right)
    pub y: f32,  // top-left y (+y down)
    pub w: f32,
    pub h: f32,
}
```
**Why x,y,w,h together (vs Bevy's split)**: the renderer reads one 16 B component and the writer does one aligned 16 B store — clean for the instanced-quad upload (P5a) and hit-test (P4). **Write discipline**: one whole-struct assignment per node, **set-if-changed** (Decision 5), with the compare reading through a shared accessor so the `Mut` guard is acquired only on inequality.

```rust
/// Draw-order / z key within a root. COLD, OPT-IN (default 0). AUTHOR-OWNED in
/// P1 — layout never reads/writes it. The renderer (P5a) sorts by it.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackIndex(pub u32);
```

```rust
/// Clip rectangle for overflow. COLD, OPT-IN. AUTHOR-OWNED in P1 (not computed);
/// consumed by P5a's scissor. P1 overflow policy = "allow overflow" (§Risks).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ComputedClip { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
```

```rust
/// Marks a screen-space root: the layout entry points. NORMAL marker component
/// (NOT bitset) so it is ENUMERABLE via Query<(), With<UiRoot>> / archetype
/// signature scan. A root's ChildOf (if any) is ignored for layout; it is seeded
/// from the viewport rect.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct UiRoot;
```
**Why a NORMAL marker, not bitset** (critic critical): the bitset/EnableTag backend is a **separate registry** (`enable_tag_api.rs:14-15`) — bitset tags are filtered out of every archetype signature and expose only per-entity `enable`/`disable`/`is_enabled`, with **no enumeration/iterator** over tagged entities. Root discovery requires enumeration (the up-walk needs to recognize a root, and `Added<UiRoot>` must be queryable). A normal ZST marker component:
- Is enumerable via `Query<(), With<UiRoot>>` (used by `q_root` in discovery to test "is this node a root?" during the up-walk, and by `Added<UiRoot>` in the scan).
- Pays only the tick-bearing ZST pool cost (the same cost the demo's mode-tag markers pay) — acceptable; there are few roots.

The original plan conflated `#[derive(Component)]` (mints a `ComponentId`, archetype-stored) with the EnableTag registry (`EnableTagId`, bitset, un-enumerable) and wrongly cited the `freeze_pulse` precedent (which enumerates a **normal** `ParticleTag` then toggles the **bitset** `Frozen` — the opposite of what root discovery needs). Corrected: `UiRoot` is a normal marker.

### Resource (`resources.rs`)

```rust
/// Screen-space root seed. Set by the host (window/swapchain) on surface create
/// and resize. `scale_factor` folds DPI (logical = physical / scale); P1 layout
/// is logical-px, P5a applies scale_factor at upload.
#[derive(Clone, Copy, Debug)]
pub struct UiViewport {
    pub width: f32, pub height: f32, pub scale_factor: f32,
    /// Bumped by the host on every resize. The discovery system compares it to
    /// its last-seen generation; a mismatch marks ALL roots dirty.
    pub generation: u32,
}
impl Default for UiViewport {
    fn default() -> Self { Self { width: 0.0, height: 0.0, scale_factor: 1.0, generation: 0 } }
}
```
**Viewport-resize signal** (critic open-q resolved): `UiViewport` is a `Resource` (no per-row tick, so `Changed` does not apply to it). The host bumps `generation` on resize. The discovery system stores `last_viewport_generation` in `LayoutScratch`; if it differs from `viewport.generation`, discovery marks **all** roots dirty (a resize affects every root). Marking all roots requires enumerating roots — done **without** `query_entities` (which allocates): the discovery system holds a `Query<Entity, With<UiRoot>>` param `q_all_roots` and, **only on a resize frame**, iterates it into `scratch.dirty_roots`. A resize is rare, so this iteration is off the steady-state path entirely. (The `Query` iterator does not allocate — it iterates archetype columns in place; only `query_entities`, which collects into a fresh `Vec`, allocates.)

---

## The layout systems

### Signatures

```rust
/// Discovery pass: a NORMAL scheduled system (SystemParams supply the
/// (last_run, this_run] window that Changed/Added require). Writes the dirty-root
/// set into the preallocated LayoutScratch. Zero per-frame alloc.
pub fn ui_layout_discovery(
    changed: Query<Entity, Or<(
        Changed<UiLayout>, Changed<UiSpacing>, Changed<UiAlign>, Changed<UiAbsolute>,
        Changed<ContentSize>, Changed<Children>, Changed<ChildOf>, Added<UiRoot>,
    )>>,
    q_childof: Query<&ChildOf>,
    q_is_root: Query<(), With<UiRoot>>,        // "is this node a root?" probe
    q_all_roots: Query<Entity, With<UiRoot>>,  // resize-frame full enumeration
    mut scratch: ResMut<LayoutScratch>,
    viewport: Res<UiViewport>,
);

/// Apply pass: an EXCLUSIVE system (the only form that gives nested parent↔child
/// mutable row access). Consumes scratch.dirty_roots; runs the recursive walk.
pub fn ui_layout_apply(world: &mut EcsMaster);
```

```rust
/// Reused per-frame scratch (Resource — engine storage, allocated ONCE at setup
/// via `with_seeds`, capacity persists, only clear()/index-reset per frame).
pub struct LayoutScratch {
    /// Dedup'd dirty roots for THIS frame (discovery writes, apply reads+clears).
    dirty_roots: Vec<Entity>,
    /// DEPTH-INDEXED child working sets (Decision 6). child_pool[d] is the sorted
    /// child list for the container being laid out at recursion depth d. Each
    /// inner Vec reused across frames (clear on entry, capacity retained).
    child_pool: Vec<Vec<Entity>>,    // len == MAX_LAYOUT_DEPTH
    /// DEPTH-INDEXED stretch working sets (Decision 6).
    stretch_pool: Vec<Vec<StretchItem>>, // len == MAX_LAYOUT_DEPTH
    /// Discovery's up-walk dedup scratch (frame-local linear set; root count tiny).
    /// Reused; cleared each discovery run.
    marked: Vec<Entity>,
    /// Last viewport generation discovery acted on (resize detection).
    last_viewport_generation: u32,
    /// #[cfg(test)] relayout counter for the 0%-overhead test hook.
    #[cfg(test)] pub relayout_count: u32,
}

impl Default for LayoutScratch {
    fn default() -> Self {
        // Default == fully EMPTY (valid mem::take target, Decision 7).
        Self {
            dirty_roots: Vec::new(),
            child_pool: Vec::new(), stretch_pool: Vec::new(),
            marked: Vec::new(), last_viewport_generation: 0,
            #[cfg(test)] relayout_count: 0,
        }
    }
}

impl LayoutScratch {
    /// Host calls this ONCE at setup and inserts the result as a Resource. Seeds
    /// every buffer with a modest capacity so the first frames after a new
    /// high-water container do not reallocate (critic minor adopted).
    pub fn with_seeds() -> Self {
        const SEED_ROOTS: usize = 8;
        const SEED_FANOUT: usize = 32;   // typical max children per container
        const SEED_STRETCH: usize = 16;  // typical max stretch items per container
        let mut child_pool = Vec::with_capacity(MAX_LAYOUT_DEPTH);
        let mut stretch_pool = Vec::with_capacity(MAX_LAYOUT_DEPTH);
        for _ in 0..MAX_LAYOUT_DEPTH {
            child_pool.push(Vec::with_capacity(SEED_FANOUT));
            stretch_pool.push(Vec::with_capacity(SEED_STRETCH));
        }
        Self {
            dirty_roots: Vec::with_capacity(SEED_ROOTS),
            child_pool, stretch_pool,
            marked: Vec::with_capacity(SEED_ROOTS),
            last_viewport_generation: 0,
            #[cfg(test)] relayout_count: 0,
        }
    }
}

/// Max recursion depth = depth-pool length = cycle/pathological-depth guard.
pub(crate) const MAX_LAYOUT_DEPTH: usize = 128;

/// One participant in a container's main-axis stretch freeze loop. POD. ~40 B.
#[derive(Clone, Copy)]
struct StretchItem {
    target: StretchTarget,
    factor: f32,
    min: f32,        // Auto-min resolved to measured CONTENT main before freeze
    max: f32,        // f32::MAX sentinel if unbounded
    base_share: f32, // factor * free / sum for THIS round (= "measured")
    computed: f32,   // clamp(base_share, min, max)
    frozen: bool,
}
#[derive(Clone, Copy)]
enum StretchTarget { Child(u32), GapAfter(u32) }
```
**Why `marked` is a frame-local linear `Vec`, NOT a persisted `Entity::id()`-keyed bitset** (critic major adopted): the original plan's "OR a reused bitset keyed by Entity::id() if root count grows" is the slippery slope toward a shadow entity→node index (a forbidden cross-frame side store under Principle 0). Root count is tiny; a linear-scan dedup over `dirty_roots`/`marked` is O(roots²) but roots ≤ single digits in practice, so it is negligible. **The persisted-bitset option is dropped entirely.** If root count ever grows pathologically, the documented escalation is a normal ECS approach (a per-root `RootDirty` ZST tag toggled and cleared), never a scratch-resident cross-frame bitset.

### Discovery algorithm (`ui_layout_discovery`)

```
clear scratch.dirty_roots; clear scratch.marked
if viewport.generation != scratch.last_viewport_generation:
    scratch.last_viewport_generation = viewport.generation
    for root in q_all_roots.iter():            # resize-frame ONLY, off hot path
        push_dedup(dirty_roots, root)
    return                                       # all roots dirty; done
for node in changed.iter():                      # Phase-10 0%-overhead in steady state
    # up-walk to the enclosing root
    let mut cur = node
    loop {
        if q_is_root.get(cur).is_some(): push_dedup(dirty_roots, cur); break
        match q_childof.get(cur) { Some(co) => cur = co.0, None => break }  # orphan: no root
        debug_assert!(walk_steps < MAX_LAYOUT_DEPTH)  # cycle guard
    }
```
- `push_dedup` = linear scan of `dirty_roots`; push if absent. (`marked` reserved for any future two-phase dedup; in P1 `dirty_roots` itself is the dedup set — both are tiny.)
- **Steady state**: `changed.iter()` yields nothing (no input tick in the window) and `viewport.generation` unchanged → `dirty_roots` stays empty, **zero alloc, zero up-walks**. This is the 0%-overhead point, now in a system that actually compiles.
- **Zero alloc**: `dirty_roots`/`marked` are preallocated; `push_dedup` reuses capacity; `changed.iter()`/`q_*.get()`/`q_all_roots.iter()` are in-place archetype iteration (no `query_entities`).

### Apply algorithm (`ui_layout_apply`)

```
let mut scratch = mem::take(world.get_resource_mut::<LayoutScratch>())  # Decision 7
#[cfg(test)] scratch.relayout_count = 0
let viewport = *world.get_resource::<UiViewport>()
for root in scratch.dirty_roots.drain(..):       # drain: clears for next frame
    #[cfg(test)] scratch.relayout_count += 1
    layout_root(world, &mut scratch, root, viewport)
*world.get_resource_mut::<LayoutScratch>() = scratch  # put back, capacity retained
```
- `mem::take` frees the world borrow so the recursion can call `get_component`/`get_component_mut`.
- `dirty_roots.drain(..)` consumes and clears in one pass; capacity retained.
- Early-return is implicit: empty `dirty_roots` → the loop body never runs → put-back → return. **Zero recursion, zero writes, zero alloc** when nothing is dirty.

### How the walk reads/writes a node (honest cost model — critic major)

Per node visit, resolve the node's **(archetype, row) pointer ONCE** (the engine's batched raw accessor `get_components_raw` at `ecs_master.rs:2121` does the single sparse `entities_inland[id]` lookup + null/gen check + archetype deref), then read `UiLayout`/`UiSpacing`/`UiAlign` and (if present) `UiAbsolute`/`ContentSize` from that single resolution. This replaces N independent `get_component` calls (each a full multi-hop pointer chase) with one resolution + column offsets.

**Cost model statement**: each node visit is a **cache-miss-bound pointer chase** into the sparse `entities_inland` store (keyed by entity id, NO spatial correlation to tree-walk order) then into the archetype slab — NOT O(1)-free. The O(N) figure counts *node visits*; the *constant* is a pointer chase per visit. The engine's SoA strength (contiguous column iteration) is **not** exploited by a per-entity tree walk; **archetype-order SoA traversal is the documented future optimization seam** (it would require flattening the tree into archetype-sorted order, deferred). This honest model replaces the original "O(1) cheap access" claim.

**Writes** are scattered (each `ComputedRect` at `column_ptr + idx*stride` in whatever archetype the node lives in). Each write is **one aligned 16 B store**, set-if-changed; there is **no streaming run** and **no non-temporal store** (that framing is dropped).

### The algorithm — HONEST pass structure (critics: criticals on Auto/Pct/cross + pass count)

The algorithm is **NOT** a single down→up→down with one touch per child. It is a **multi-pass** structure because several quantities depend on later-phase outputs (Auto size depends on children; Pct/Stretch depend on the resolved container size). The true structure per `layout_node`:

```
layout_node(node, depth, avail_main, avail_cross, parent_def_main, parent_def_cross) -> Size{main, cross}
```
where `avail_*` is the space the parent offers, `parent_def_*` is the parent's DEFINITE (non-Auto) extent on each axis (used as the Pct base; `NaN`/sentinel if the parent axis is indefinite). `child_buf = scratch.child_pool[depth]`; `stretch_buf = scratch.stretch_pool[depth]`.

**Pass A — gather & partition** (uses `child_pool[depth]`):
1. Resolve `layout_type` (`Grid` → `#[cold]` `debug_assert!` + treat as `Column`).
2. Read this node's `UiLayout`/`UiSpacing`/`UiAlign` from one pointer resolution.
3. Copy `Children` slice → `child_buf`; sort by `Entity::id()`.
4. Partition: relative children kept in front, absolute appended to a tail region (stable).
5. Resolve this node's own definite main/cross if the author set `Px`/`Pct(definite-parent)`: a `Px` size is definite; a `Pct` size is definite **iff** `parent_def_*` on that axis is definite. Record `self_def_main`/`self_def_cross` (or "indefinite" if Auto / Pct-of-indefinite).

**Pass B — intrinsic content measure** (the "down" pass; loops relative children; recurses into `depth+1`):
For each **relative** child, measure its **intrinsic content size** on both axes by recursing `layout_node(child, depth+1, intrinsic_avail, ...)` where the main-axis avail is unconstrained for the purpose of content measurement. Resolve the child's contribution per its main-axis `Unit`:
- `Px(v)` → main contribution `v`.
- `Auto` → recurse; if leaf (no children) use `ContentSize` on that axis (0 if absent); if container use the recursion's returned `Size.main`.
- `Pct(v)` → if `self_def_main` is definite: `v*0.01*self_main_content`; **else (Auto/indefinite container): the Pct child resolves to its CONTENT size for intrinsic purposes** (CSS: a percentage against an indefinite container behaves as auto) — recurse for content, and it contributes its content to the parent's Auto total. The purely-circular case (Pct child is the sole determinant of an Auto parent) bottoms out at the child's content (0 if none). **This fixes the critic's Pct-of-Auto-main critical: a Pct-main child of an Auto-main container resolves against content, contributing to the final total, not against an incomplete mid-loop value.**
- `Stretch(f)` → **measure the child at its intrinsic content size now** (recurse with unconstrained main) to obtain its content-main floor; push `StretchItem{ target: Child(i), factor: f, min: (min_unit==Auto ? measured_content_main : resolve(min_unit)), max, ... }`; add `f` to `main_flex_sum`. **This fixes the critic's "Stretch min=Auto unmeasured" major: Stretch children ARE measured for content in Pass B, so the freeze loop has a real Auto-min floor.** A `Stretch` gap pushes `StretchItem{ target: GapAfter(i) }`.
- Clamp each non-stretch contribution to `[min, max]`.
- Accumulate `relative_main_sum += contribution + resolved_fixed_gap` (Stretch contributions excluded — they are 0 until distribution).
- **Cross fold (fixes the critic's "Auto-cross never computed" critical)**: track `max_child_cross = max(max_child_cross, child_cross_intrinsic)` across all relative children, where `child_cross_intrinsic` = the child's resolved cross (Px/Pct-of-definite/Auto-content). This is the basis for the container's Auto cross.

**Pass C — resolve this container's definite extents** (no child recursion):
- **Main**: `target_main = self_def_main` if definite; else (Auto main) `target_main = relative_main_sum + padding_main + border_main` (hug content); clamp to `[min_main, max_main]`.
- **Cross**: `target_cross = self_def_cross` if definite; else (Auto cross) `target_cross = max_child_cross + padding_cross + border_cross` (hug widest child); clamp to `[min_cross, max_cross]`.
- `content_main = target_main - padding_main - border_main`; `content_cross = target_cross - padding_cross - border_cross`.
- Now (and only now) is the container's size DEFINITE, so Pct/Stretch children that needed it can resolve.

**Pass D — main-axis stretch freeze** (iterative, exact CSS resolve-flexible-lengths; loops `stretch_buf`):
- `free_main = content_main − relative_main_sum` (non-stretch children + fixed gaps already in `relative_main_sum`). May be negative (over-constrained, §AlignMain).
- Freeze loop:
  1. `sum = Σ factor` over **unfrozen** items.
  2. each unfrozen: `base_share = factor * free_main / sum`; `computed = clamp(base_share, min, max)`; `violation = computed − base_share`; `total_violation += violation`.
  3. freeze rule: `total > 0` → freeze items with `violation > 0` (hit min); `total < 0` → freeze items with `violation < 0` (hit max); `total == 0` → freeze ALL.
  4. **Subtract each newly-frozen item's COMPUTED (clamped) size from `free_main`, and its factor from the running sum** (NOT the base_share — critic major adopted; subtracting computed is the correct CSS rule). Repeat with the reduced `free_main`/`sum`.
  5. Terminates: each round freezes ≥1 item (the violating set is non-empty when `total ≠ 0`) or all on `total == 0` ⇒ ≤ S rounds, O(S²) total. **Termination-soundness note**: a pathological pair (item A `min` > item B `max`) still freezes ≥1 per round because the violating set is determined by the SIGN of net violation, and at least the most-violating item is in it; the `debug_assert!(rounds <= stretch_count)` cannot fire on legal min/max combinations (proven: every round removes ≥1 item from the unfrozen set). A `#[cfg(test)]` test exercises the A.min>B.max pair.
- For each stretch item whose `computed ≠ its first-round content measure`, **re-layout that child** at `computed` main so descendants reflow (the documented nested-stretch redundant work).

**Pass E — cross-axis stretch + descendant reflow** (loops cross-stretch children, then containers):
- `AlignCross::Stretch` child (or child cross `Unit::Stretch`): `child_cross = content_cross`, clamped to the child's cross min/max; re-layout against it. **Auto-cross-container + Stretch interaction (critic feedback-loop concern)**: when the container's cross is Auto, `content_cross` was already computed in Pass C as `max_child_cross` (the unstretched content max). Stretching children to that value does NOT change `max_child_cross` (it equals the max content), so there is **no feedback loop** — the stretched cross equals the already-finalized content cross. Documented.
- Any child that itself has children gets a final `layout_node(child, depth+1, resolved_main, resolved_cross, target_main, target_cross)` so the now-definite container size propagates as the child's Pct base.

**Pass F — positioning** (loops relative children; streaming-shaped but scattered writes):
- main cursor = `padding_main_before + border_main_before` + the `AlignMain` leading offset (computed from `free_main` leftover — see §AlignMain precedence).
- per child: `main_pos = cursor + leading; cross_pos = padding_cross_before + border_cross_before + cross_align_fraction * (content_cross − child_cross)`.
- map `(main_pos, cross_pos, child_main, child_cross)` → `(x,y,w,h)` via axis-fold; **add the parent's absolute screen origin** (passed down the recursion) so children get absolute coords; `debug_assert!` finite; write `ComputedRect` set-if-changed.
- `cursor += child_main + gap` (+ SpaceBetween/Around/Evenly increments).

**Pass G — absolute children** (loops the absolute tail; consume no flow space):
- size against the **padding box** of this container; resolve `UiAbsolute` before/after (before wins; both unset → span if size Auto, else align); recurse if it has children; `debug_assert!` finite; write `ComputedRect`.

**Return** `Size{ main: target_main, cross: target_cross }` (clamped to this node's own min/max) for the parent's accumulation.

**Overlay**: Pass A + Pass B-as-measure (each child measured against the full content box) + Pass C + Pass E (children may cross/main-stretch to the box) + Pass F positioning by `UiAlign` only (each child placed independently within the box; no main accumulation, no main freeze across siblings). Documented as a distinct positioning rule.

#### AlignMain precedence (critic major adopted)

1. **Stretch present** ⇒ stretch consumes all positive free space ⇒ `free_main` leftover after Pass D is ~0 ⇒ `AlignMain` distribution (SpaceBetween/Around/Evenly, Center, End) is a **no-op**. **P1 documents: when any relative child or gap is `Stretch`, `AlignMain` is ignored.** (Tested.)
2. **No Stretch, `free_main > 0`** ⇒ `AlignMain` distributes leftover normally.
3. **`free_main < 0` (over-constrained / overflow)** ⇒ the leading offset is **clamped to ≥ 0** (content packs at the before-edge and overflows the after-edge; Center/SpaceAround do NOT shift content off the before-edge). **P1 documented overflow-alignment policy.** (Tested.)

#### Honest pass count & complexity

Per container, the worst case touches its children across Passes B (intrinsic measure, including Stretch content pre-measure), D/E (re-layout of clamped/cross-stretch children), and F (positioning). A subtree with nested Auto/Stretch/Pct can therefore be re-entered: **the redundant-work bound is `O(N × stretch_nesting_depth)`, NOT `O(N)` flat** — a Pct-cross child of an Auto-cross parent of a Stretch grandparent needs the content-measure round + the definite-resolve round + the distribution round on that subtree. This is stated honestly. For fixed-size subtrees (no Auto/Stretch/Pct), it collapses to a single touch per child. The freeze loop adds `O(S²)` per container (S = stretch items, typically ≤ a few). **Net headline**: O(N) node visits × a small (≤ ~3) re-layout factor for nested-flexible subtrees, + Σ O(S²) over containers. The "tens of µs for a 1000-node root" estimate holds with this ≤3× factor.

---

## Roots & coordinate space

- **Viewport source**: `UiViewport` resource (logical w/h + `scale_factor` + `generation`). The host (windowing/swapchain — eframe in `boyko_demo`, Vulkan swapchain in the engine) updates it on create/resize via `get_resource_mut`, bumping `generation` on resize. Layout reads it; it does NOT query a window directly (Principle 0, no winit dep on the engine path).
- **Root seed rect**: each dirty `UiRoot` is laid out with `avail_main/avail_cross = viewport extent`, `parent_def_* = viewport extent` (definite), origin `(0,0)`. A root with explicit `width/height` uses those (clamped); default Auto root → fills viewport.
- **Origin**: top-left, +x right, +y down (matches the instanced-quad raster and Vulkan framebuffer handling at upload).
- **Units**: logical pixels in `ComputedRect`. **DPI**: `scale_factor` is stored but NOT multiplied into layout in P1 — layout is logical-px; P5a applies `scale_factor` at upload. Resolution-independent layout. Documented seam.

---

## Multithreading model

- **Single-threaded** in P1. `ui_layout_discovery` is a normal system but reads only `Changed`/`ChildOf`/`UiRoot` and writes only the `LayoutScratch` resource; `ui_layout_apply` is exclusive (`&mut EcsMaster`, full world). They run sequentially (discovery before apply) by schedule order.
- **No shared mutable state across threads**, no atomics, no locks. The `LayoutScratch` handoff is single-threaded (discovery writes, apply reads) within one schedule frame.
- **`Send`/`Sync`**: `LayoutScratch` holds only `Vec<Entity>`/`Vec<Vec<...>>`/POD — `Send + Sync + Default`, valid as a `Resource` and a `mem::take` target. All components are POD `Copy` — trivially `Send + Sync`.
- **Parallel-over-independent-roots** is the documented future optimization (each root subtree is disjoint, embarrassingly parallel), gated behind a measured need; it would require per-root sub-borrows (unsafe row partitioning) and is out of scope for P1.

## Integration

- **New crate** `crates/boyko_ui/` (Cargo.toml + `src/{lib,units,components,layout,resources}.rs`); register in root `Cargo.toml` `[workspace].members`.
- **No `boyko_ecs` core changes.** Consumes existing public API: `#[derive(Component)]`, `Query`/`Res`/`ResMut`/`With`/`Or`/`Changed`/`Added` SystemParams, `ChildOf`/`Children`, exclusive systems, `ScheduleBuilder::add_system`, `EcsMaster::{get_component, get_component_mut, get_components_raw, get_resource, get_resource_mut, insert_resource}`.
- **Host wiring** (mirrors `boyko_demo/src/sim/runner.rs:166-325`): `insert_resource(UiViewport::default())`, `insert_resource(LayoutScratch::with_seeds())` at setup; `add_system(ui_layout_discovery)` then `add_system(ui_layout_apply)` AFTER all structural/prop-mutation systems (consistency window). Host updates `UiViewport` on resize.
- **Schedule order constraint**: discovery → apply, both after structural mutators. If the engine's schedule needs an explicit edge, use `before`/`after` (Phase 15) to pin `ui_layout_discovery` before `ui_layout_apply`.

## Implementation plan (for the developer)

1. `crates/boyko_ui/Cargo.toml` + workspace registration; empty `lib.rs` with the module declarations. **Verify build.**
2. `units.rs` — `Unit` (`f32::MAX` sentinel doc), `LayoutType`, `PositionType`, `AlignMain`, `AlignCross`, all `repr` + `Default`.
3. `components.rs` — input components (`UiLayout` default uses `f32::MAX` max), `UiSpacing`, `UiAlign`, `UiAbsolute`, `ContentSize`; outputs `ComputedRect`, `StackIndex`, `ComputedClip`; `UiRoot` **normal marker** (not bitset).
4. `resources.rs` — `UiViewport` (with `generation`), `LayoutScratch` (`Default` = empty, `with_seeds`), `MAX_LAYOUT_DEPTH`, `StretchItem`/`StretchTarget`.
5. `layout.rs` — `ui_layout_discovery` (scan + up-walk + dedup + resize-all-roots branch).
6. `layout.rs` — `ui_layout_apply` (`mem::take` protocol + dirty-root loop).
7. `layout.rs` — the private `layout_node` with the explicit Passes A–G + axis-fold helper + freeze loop. Mark rare arms `#[cold]`/`#[inline(never)]`: Grid fallback, missing-`ComputedRect`, depth-clamp, non-convergence assert.
8. `prelude` re-exports.
9. Hand off to the tester (the dev does not run tests).

**Inlining discipline (Principle 7)**: NO blanket `#[inline(always)]`. The only `#[inline]` candidate is the trivial axis-fold helper, and only if assembly shows it is not inlined. `layout_node` is a large monomorphic body — keep the Unit-resolution `match` (4-way, LUT-friendly) and the freeze inner loop as the only hot region; push Grid/missing-rect/depth/assert arms to `#[cold]` out-of-line functions to keep the hot path's I-cache footprint small.

## Metrics and validation

### Mandatory unit tests (driven via a hand-built schedule with `ui_layout_discovery` + `ui_layout_apply`; helper `spawn_ui(world, UiLayout, parent: Option<Entity>) -> Entity` that also inserts a default `ComputedRect`)

**Row / Column**: Column 200×Auto, three `Px(50)` h children, `row_gap=Px(10)` → y = 0,60,120; Auto h = 170. Row Auto×100, two `Px(80)` w children, `column_gap=Px(20)` → x = 0,100. Axis-fold transposition (same set Row vs Column → transposed coords).

**Auto-cross (new — critic critical)**: Column Auto×Auto with children of differing widths (`Px(40)`, `Px(120)`, `Px(80)`) → container Auto cross = 120 (+ cross padding/border). Asserts the Pass-C cross fold.

**Pct-of-Auto-main (new — critic critical)**: Column **Auto** main with a `Px(100)` h child and a `Pct(50)` h child whose own content is `Px(30)` → the Pct child resolves to its content (30) for the Auto total (CSS indefinite-container behavior); container Auto main = 130. Separately: Column **Px(200)** main, `Pct(50)` h child → 100 (definite base). Both asserted distinctly.

**Overlay**: 300×300, two `Px(100)²` children, `{Start,Start}` and `{Center,Center}` → (0,0) and (100,100); both share the box.

**Stretch**: Row 300, `Stretch(1)`+`Stretch(2)` → 100,200. Row 300, `Px(100)`+`Stretch(1)`+`Stretch(1)` → 100,100,100. **min clamp**: Row 300, `Stretch(1) min_width=Px(150)` + `Stretch(1)` → 150,150 (freeze converged, NOT 100/200). **max clamp**: `Stretch(1) max_width=Px(50)` + `Stretch(1)` in 300 → 50,250. **Stretch gap**: Row 300, `Px(100)`+`Px(100)`+`column_gap=Stretch(1)` → gap 100. **Zero free**: Row 0, two `Stretch(1)` → 0,0. **Computed-not-base subtraction (new — critic major)**: Row 300, `Stretch(1) max=Px(50)` + `Stretch(2)` → first 50, second 250 (verifies frozen item's COMPUTED 50, not base_share, is subtracted). **A.min>B.max pathological (new — critic major)**: `Stretch(1) min=Px(200)` + `Stretch(1) max=Px(50)` in Row 100 → 200,50 (both freeze; assert ≤ S rounds, assert fired no spurious non-convergence).

**Stretch min=Auto content floor (new — critic major)**: `Stretch(1) min_width=Auto` with `ContentSize{120,_}` in an undersized Row 80 → child = 120 (never crushed below content; Pass-B pre-measure populated the floor).

**Auto/content (P1, no text)**: Auto leaf, no `ContentSize`, no children → 0×0. Auto leaf + `ContentSize{40,12}` → 40×12. Auto container hugging: Column Auto, two `Px(30)` h children → h = 60 (+ padding/border).

**Px/Pct**: Pct(50) of `Px(200)` parent → 100. Pct base reduced by parent padding (`padding_left=Px(10)` → base = 200−10).

**min/max non-stretch**: `Px(500) max_width=Px(200)` → 200. `Px(10) min_width=Px(50)` → 50.

**AlignMain (new — critic major)**: Row 300, `Px(50)`+`Px(50)`, `AlignMain::Center` → packed centered (leading = 100). Row 300 with a `Stretch(1)` child + `AlignMain::SpaceBetween` → AlignMain IGNORED (stretch consumed free space). Over-constrained Row 100 with two `Px(80)`, `AlignMain::Center` → leading clamped to 0 (content overflows after-edge, not negative before-edge).

**Nesting**: Column(root 400×600) → Row(`Stretch`×`Stretch`) → each a Column with `Px` children. Assert inner reflow (≥2 `layout_node` calls on the inner subtree). **Depth-pool isolation (new — critic critical)**: a ≥3-level Auto/Stretch tree where a parent is mid-loop while a child recurses → assert the parent's children positions are NOT corrupted by the grandchildren (directly exercises Decision 6). Build a 4-deep tree with siblings at each level and verify every leaf's rect.

**Absolute**: Container 200×200, relative `Px(50)` child + absolute `UiAbsolute{left:Px(10),top:Px(20)}` `Px(30)²` → relative at flow pos, absolute at (10,20), relative flow UNAFFECTED. `left & right` both set → before (left) wins.

**Empty container**: Column Auto, no children, `padding=Px(5)` all → 10×10. No stretch loop, no panic.

**Structural (new — critic major)**:
- **Reparent across roots**: two roots A, B; move a subtree from a child of A to a child of B; run discovery+apply; assert BOTH roots relaid (A reclaims vacated flow, B accommodates arrival). Verify `Changed<ChildOf>` + `Changed<Children>` on both endpoints drove it.
- **Despawn of a middle node**: spawn a 3-child column, despawn the middle, run; assert the remaining two reflow (parent's `Children` change drove the root mark).

**Determinism**: spawn children, remove a middle sibling (triggers `swap_remove`), relayout → flow order unchanged (id-sorted).

**0%-overhead (scoped — critic minor)**: run once; record each `ComputedRect` tick. Run again, NO mutation → assert no `ComputedRect` tick advanced AND `scratch.relayout_count == 0` (the `#[cfg(test)]` counter). **Identical-geometry mutation**: mutate an input so the recomputed geometry is bit-identical → assert the `ComputedRect` tick does NOT advance (set-if-changed via shared-deref compare held). Mutate one child's `UiLayout` to a DIFFERENT size → assert exactly its root relaid (`relayout_count == 1`), other roots untouched.

**Zero-per-frame-alloc (re-derived against real APIs — critic majors)**: counting global allocator behind a `#[cfg(test)]` feature (mirror Phase X.E `bench-alloc`). Warm up M frames so all depth-pool inner `Vec`s + `dirty_roots` reach high-water. Reset counter. (a) Run discovery+apply on an UNCHANGED tree → assert **0 allocations** (steady state: `changed.iter()` empty, no `query_entities`, `mem::take` moves headers only). (b) Run after a NON-structural size tweak (within high-water) → assert **0 allocations**. (c) A structural growth beyond high-water may allocate once → asserted as the documented amortized exception. Explicitly assert NO call path invokes `query_entities` (code-review check + the alloc test on the resize path: trigger a resize after warmup with the same root count → 0 allocations because `q_all_roots.iter()` is in-place).

### Mandatory `debug_assert!` invariants

- `depth < MAX_LAYOUT_DEPTH` (cycle/pathological-depth guard; release clamps to leaf).
- Freeze-loop `rounds <= stretch_count` (termination).
- `rect.{x,y,w,h}.is_finite()` before every `ComputedRect` write (NaN backstop).
- `!matches!(layout_type, Grid)` reachable only via the `#[cold]` fallback.
- Node has `ComputedRect` before a positioning write (else `#[cold]` `debug_assert!(false)` + skip, no mid-walk insert).

### Miri

Full unit suite under Miri-TB. P1 has **no `unsafe`**, so Miri is a cheap UB/correctness backstop (alias rules around `mem::take` + repeated `get_component_mut`), not the primary gate.

## Risks / edge cases

| Risk | Handling |
|---|---|
| **`Children` non-deterministic order** | Sort `child_pool[depth]` by `Entity::id()` (Decision 4). |
| **`Children` staleness window** | Pair scheduled after structural mutators; changes seen next run. Documented. |
| **Recursion clobbering working sets** | Depth-indexed `child_pool`/`stretch_pool` (Decision 6); each level its own buffer. |
| **Auto-cross never computed** | Pass-C cross fold (`max_child_cross`); finalized before Pass-E cross-stretch. |
| **Pct-of-Auto-main mid-loop read** | Pct-of-indefinite resolves to content in Pass B; container size finalized in Pass C before any Pct-of-definite resolution. |
| **Stretch min=Auto floor** | Pass-B pre-measures Stretch children at content; floor populated before Pass-D freeze. |
| **Freeze subtract computed-vs-measured** | Subtract frozen COMPUTED size + factor (CSS-correct). Tested. |
| **Freeze non-convergence** | Net-violation freeze rule freezes ≥1/round; `debug_assert!(rounds<=S)`; A.min>B.max tested. |
| **AlignMain vs Stretch / negative free** | Stretch ⇒ AlignMain ignored; `free_main<0` ⇒ leading clamped ≥0. Documented + tested. |
| **Over-constrained (no flex-shrink)** | Allow overflow; `ComputedClip` author-owned for P5a scissor; P1 does not clip. |
| **Deep `ChildOf` cycle** | `MAX_LAYOUT_DEPTH` guard (debug_assert + release leaf-clamp); consistent with Phase-19 footgun. |
| **Node missing `ComputedRect`** | `#[cold]` `debug_assert!(false)` + skip (no mid-walk insert → no archetype migration). DSL guarantees presence. |
| **Grid in P1** | `#[cold]` `debug_assert!(false)` + fall back to Column. |
| **Scratch growth beyond high-water** | One amortized `Vec` growth that frame; seeded `with_capacity` minimizes it; steady state 0. Documented exception. |
| **`f32::MAX` max sentinel** | Finite; clamp no-op for realistic sizes; cannot produce INFINITY-arithmetic NaN. |
| **NaN rect** | Excluded by clamps + finite sentinel + finite-`debug_assert!`; would otherwise bump tick forever — explicitly guarded. |
| **`query_entities` per-frame alloc** | FORBIDDEN on all per-frame paths; root discovery via `Query::iter` + scratch, not `query_entities`. |
| **Scratch resource borrow conflict** | `mem::take` out at entry, put back at exit (Decision 7). |
| **Persisted shadow index** | FORBIDDEN: scratch is frame-transient (Entity handles + POD only), reset every frame; no cross-frame per-node cache; `marked` stays a frame-local linear Vec (no persisted Entity-id bitset). |

## Out of scope for P1 — seams left for later phases

- **Text measurement** → P5b (writes `ContentSize`; layout unchanged).
- **Rendering** → P5a (consumes `ComputedRect`/`StackIndex`/`ComputedClip`; 16 B `repr(C)` upload seam).
- **Interaction/hit-test/focus** → P4 (`Interaction`/`Focusable`/… hit-test `ComputedRect`).
- **World-space/diegetic UI** → P7 (writes a root's screen origin into the seed before the pass; `layout_root`'s explicit seed rect is the seam).
- **Grid internals** → later sub-phase (variant reserved).
- **`margin` (child-applied spacing)** → later (P1 freeze handles size + gap stretch only).
- **Viewport units (`Vw/Vh/VMin/VMax`)** → later (additive `Unit` variants).
- **`UiOrder(u32)` flow-order key** → P2 (DSL assigns; P1 = id order).
- **Parallel-over-independent-roots** → future, measured-need (per-root sub-borrows).
- **Sub-root incremental layout** → future (P1 relays whole dirty root subtrees).
- **Archetype-order SoA layout traversal** → future (the cache-locality optimization that would exploit SoA; P1 is per-entity pointer-chase).
- **DPI scaling into pixels** → P5a applies `scale_factor` at upload.

## Open questions (for the critic / owner)

1. **Schedule-edge mechanism**: I assume `before`/`after` (Phase 15) can pin `ui_layout_discovery` → `ui_layout_apply` and both after structural mutators. If the engine instead requires the apply (exclusive) system to sit in a dedicated exclusive stage, the ordering is still expressible — confirm the preferred mechanism (it does not change the design, only the wiring call).
2. **`get_components_raw` ergonomics**: I plan to resolve each node's row pointer once via the batched raw accessor (`ecs_master.rs:2121`) then read columns by offset. If that accessor's signature is awkward for the per-node read set, the fallback is grouped `get_component` calls (correct, marginally slower); confirm the accessor is suitable for a read-set of `{UiLayout, UiSpacing?, UiAlign?, UiAbsolute?, ContentSize?}`. This is an implementation-detail confirmation, not a design fork.

Relevant files for the developer: new crate at `D:\claude\BoykoEngine\crates\boyko_ui\` (`Cargo.toml`, `src/lib.rs`, `src/units.rs`, `src/components.rs`, `src/layout.rs`, `src/resources.rs`); register in `D:\claude\BoykoEngine\Cargo.toml` workspace members; wiring example to mirror at `D:\claude\BoykoEngine\crates\boyko_demo\src\sim\runner.rs:166-325`; component-derive precedent at `D:\claude\BoykoEngine\crates\boyko_demo\src\sim\components.rs:21`; exclusive-system precedent at `D:\claude\BoykoEngine\crates\boyko_demo\src\sim\systems\particles.rs:145`; exclusive-system internals at `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\exclusive_function_system.rs:209`; `query`/change-detection guard at `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\...:3635-3659`; `query_entities` (alloc, forbidden on hot path) at `:2096`; batched raw accessor at `:2121`; hierarchy API at `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\hierarchy\mod.rs:121-173`.

## Changes from review

**Critical — exclusive system cannot run a `Changed` scan (lens 1)**: Split into a two-system pair (Decision 1): a normal scheduled `ui_layout_discovery` (SystemParams supply the `(last_run,this_run]` window; `Changed`/`Added` legal there) that writes the dirty-root set into the `LayoutScratch` resource, and an exclusive `ui_layout_apply` that consumes it. Verified the contradiction (`exclusive_function_system.rs:209`, `query` guard `ecs_master.rs:3635-3659`) and resolved it at the design level.

**Critical — bitset `UiRoot` not enumerable (lens 1)**: Changed `UiRoot` to a **normal marker component** (enumerable via `Query<(), With<UiRoot>>`, `Added<UiRoot>`, archetype scan). Dropped the wrong `freeze_pulse` precedent (it enumerates a normal tag, not a bitset). Documented the registry distinction.

**Critical — `child_buf`/`stretch_buf` clobbered by recursion (lens 3)**: Decision 6 — depth-indexed pools `child_pool`/`stretch_pool` (`Vec<Vec<…>>` of length `MAX_LAYOUT_DEPTH`), each inner buffer reused across frames; each recursion level owns its buffer. Added a 4-deep isolation test.

**Critical — Auto-cross never computed (lens 3)**: Added an explicit Pass-C cross fold (`max_child_cross` → container Auto cross), finalized before Pass-E cross-stretch; documented the no-feedback-loop interaction with `AlignCross::Stretch`. Added an Auto-cross test.

**Critical — Pct-of-Auto-main reads incomplete size (lens 3)**: Explicit pass ordering — Pct-of-indefinite resolves to content in Pass B (CSS indefinite-container rule), container size finalized in Pass C, Pct-of-definite resolved afterward. Added both Pct-of-Auto and Pct-of-definite tests.

**Major — Stretch min=Auto unmeasured (lens 3)**: Pass B now pre-measures Stretch children at content size to populate the Auto-min floor before the freeze loop. Added a content-floor test.

**Major — freeze redistribution computed-vs-measured (lens 3)**: Pinned the CSS rule — subtract each frozen item's COMPUTED (clamped) size and factor; stated the ≤S-round / O(S²) termination invariant and proved the assert cannot fire on legal min/max (A.min>B.max test added).

**Major — AlignMain vs Stretch / negative free undefined (lens 3)**: Documented precedence — Stretch ⇒ AlignMain ignored; `free_main<0` ⇒ leading clamped ≥0. Added both tests.

**Major — honest pass count (lens 3)**: Replaced "3 phases down→up→down" with the true Passes A–G; stated the redundant-work bound as `O(N × stretch_nesting)`, not flat O(N); re-derived the perf estimate with a ≤3× factor.

**Major — per-node cost model / pointer chase (lens 2)**: Dropped the O(1)-free-access claim; restated each node visit as a cache-miss-bound multi-hop pointer chase through sparse `entities_inland`; resolve each node's row pointer ONCE via the batched raw accessor; documented archetype-order SoA traversal as the future seam.

**Major — `query_entities` allocates / zero-alloc claim false (lenses 1+2)**: Forbade `query_entities` on all per-frame paths. Root discovery is the discovery-system up-walk into preallocated `dirty_roots`; resize-frame full enumeration uses `Query::iter` (in-place, no alloc), off the steady-state path. Re-derived the zero-alloc guarantee against real APIs and added a resize-path alloc test.

**Major — Principle 0 / persisted shadow store risk (lens 1)**: Dropped the "persisted Entity-id-keyed bitset" dedup option; `marked`/`dirty_roots` are frame-transient linear `Vec`s, reset every frame, holding only Entity handles + POD; explicitly forbade any cross-frame per-node cache in scratch (must be an ECS column). Escalation path (if root count grows) is a normal ECS `RootDirty` tag, never a scratch-resident cross-frame structure.

**Major — structural invalidation gap (lens 1)**: Added `Changed<ChildOf>` to the scan terms; documented that reparent-across-roots marks both endpoints' roots and despawn-of-middle marks the parent's root; confirmed `set-if-changed` keys off inputs (never `Changed<ComputedRect>`) so it cannot mask a relayout, and the whole-subtree relayout unit (no per-node gate) is preserved. Added reparent-across-roots and despawn-of-middle tests.

**Minor — INF max → NaN (lens 1)**: Changed the unbounded sentinel from `f32::INFINITY` to `f32::MAX` (finite, clamp-equivalent, no INF-arithmetic NaN); added a finite-`debug_assert!` before every `ComputedRect` write.

**Minor — set-if-changed tick semantics (lens 2)**: Specified the compare reads through a shared accessor (`get_component`) and acquires the `Mut` guard only on inequality (the guard bumps on any mutable deref); added an identical-geometry no-tick-advance test.

**Minor — scratch resource borrow lifetime (lens 3)**: Decision 7 — `mem::take` the scratch onto the stack at entry, run the recursion against the local (freeing the world borrow for `get_component_mut`), put it back at exit; `Default`=empty makes the take valid; capacity travels with the moved `Vec`s.

**Minor — scratch growth / quadratic dedup (lens 2)**: Added `LayoutScratch::with_seeds()` with documented `with_capacity` seeds; stated `marked`/`dirty_roots` dedup is linear over a tiny root set, with the escalation trigger being a normal ECS tag (not the dropped bitset).

**Minor — I-cache / cold arms (lens 2)**: Added Principle-7 inlining discipline — rare/diagnostic arms (Grid, missing-rect, depth-clamp, non-convergence) marked `#[cold]`/`#[inline(never)]`; only the axis-fold helper is an `#[inline]` candidate (if measured); no blanket `#[inline(always)]`.

**Minor — SIMD/NT framing oversold (lens 2)**: Demoted to "each per-node write is one aligned 16 B store"; dropped the streaming/non-temporal claim (targets are scattered across archetypes); kept `repr(C)`/16 B for the P5a upload.

**Minor — set-if-changed scope / StackIndex/ComputedClip (lens 3)**: Scoped the 0%-overhead guarantee to exactly `ComputedRect`; stated `StackIndex`/`ComputedClip` are author-owned in P1 (layout never writes them). Reframed the risk-table NaN note as "bit-identical reflow intentionally suppressed."