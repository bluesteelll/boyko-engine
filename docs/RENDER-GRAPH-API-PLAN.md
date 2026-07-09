# Public Render-Graph API (`rendergraph` v1) — converged plan

Status: **CONVERGED** (architect v1 → critic ITERATE (3×P0, 7×P1, 6×P2) → v2 →
focused verification (8/10 HOLD) → final deltas folded here). Base: `2df6c27`.
Extends Pillar A (`framegraph/`, docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md) into THE
public pass-declaration surface. First clients: TAA (**engine-native**, per
docs/TAA-PLAN.md — gated only on G0) and a user-shaped chain post-effect (G5).

## Goal

Engine features (CSM/SSAO/interp/TAA/post) and user passes become declared graph
nodes; users insert custom GPU passes from `boyko_app`-level code — including
their pipelines, bind sets, and per-frame parameters — without touching
`boyko_rhi_vulkan` internals. The hardcoded `declare_gbuffer_graph` line becomes
the default graph expressed in the same public vocabulary.

Budget: zero heap alloc steady-state; equiv pins (23 img + 10 buf + 22 calls,
`framegraph_gbuffer_equiv`) byte-identical with an empty registry at EVERY rung;
registry overhead < 1 µs/frame at the 32-node cap (microbenched); ≤ 3 indirect
calls per active USER pass per frame (~1–4 ns each; every `vkCmd*` a body records
is already an indirect call through the device table — dispatch is below the
noise floor of what it dispatches); ZERO indirection for engine passes.

## Non-goals (v1)

No pass culling (`is_active` is the only gate; refcount culling falls out of
Phase-2 liveness). No async-compute execution (`QueueClass` reserved; v1 panics
on `AsyncCompute`). No mid-G-buffer insertion. No per-view graphs (reserved; the
chain/extension points are per-composite concepts that replicate per view). No
`PreRaster` point (no customer; `#[non_exhaustive]` adds it non-breaking). Node
panics propagate with inherited abort semantics — no catch_unwind, no isolation.

## Key decisions

### D1 — Dispatch: static engine line + setup-boxed node registry (critic-verified)

`record_gbuffer` stays the hand-written statically-dispatched line for all 11
engine passes. User passes implement `trait RenderPassNode: Any` (dyn-safe;
trait upcasting to `dyn Any` is stable since Rust 1.86), registered ONCE at
setup as `Box<dyn RenderPassNode>`; ≤ 3 vcalls/frame per active node. No
per-frame closures/boxing — node state lives in the registered object (the
"capture IS the object" rend3 insight). Cap `MAX_USER_PASSES = 32`; all
setup-time misuse (cap, AsyncCompute, post-finalize registration) = documented
cold panic. Rejected: monomorphized registry (pass sets are frame-data-dependent;
the type parameter would infect App, killing runtime plugin composition —
industry-unanimous); enum+fn-pointer tail (same cost, worse safety); node-ifying
engine passes (churns pin-protected code for zero function).

### D2 — Resources: Transient (claim-tracked pool) / Imported / History

New `res_class: Vec<ResClass>` SoA column (u8: Transient|Imported|History) —
also the Phase-2 aliasing exclusion input.

1. **Transient** — `PassBuilder::transient_image(&TransientImageDesc)`. Pool
   owned by the host (see D10 ownership); entries `[VulkanTexture; FIF]` (D8) +
   `last_claim_frame: u64`. **Per-frame claim tracking (critic P0-1)**: a desc
   match on an entry claimed THIS frame is a miss → a SIBLING entry is allocated
   (cold) and claimed — N same-desc requests in one frame get N physical images
   (airtight for N ≥ 3: each sibling is itself claimed). Desc-dedupe is
   CROSS-FRAME memory reuse, never an intra-frame sharing channel; intra-frame
   sharing = handle passing only (chain / node-internal). Rationale: two ResIds
   on one VkImage = two independent sync machines = no derived barrier = GPU
   race — claim tracking makes it unrepresentable. Lookup: linear scan, derived
   field-wise `Eq`, ≤ pool size; overflow = cold grow + debug_assert
   (kernel-arena discipline, critic R3); v1 has no eviction (a claimed sibling
   holds ×FIF VRAM until Phase 2 — documented). Composite-relative entries are
   destroyed+recreated on the swapchain-recreate idle path; `Fixed{w,h}` survive.
2. **Imported** — the engine's fixed named set via `EngineImage` (`LitHdr`,
   `Depth`, `GbufferAlbedo/Normal/Material`; `#[non_exhaustive]`). Swapchain NOT
   importable; present blit + WSI barriers stay engine-owned.
3. **History** — NEW `ResSync::seeded_history(layout, stages, access)`: the
   layout-CARRYING seed (existing `seeded_*` pin UNDEFINED = content discard,
   fatal for history). Critic-verified against sync.rs:199-264: the transition
   state machine needs ZERO changes. `add_image_history` sets the class;
   `compile()` enforces **I5 (release-checked)**: derived `UNDEFINED` old-layout
   on a History resource is fatal unless the one-shot init is armed.
   **First-frame lifecycle (critic P1-3)**: feature-owned `history_valid: bool`
   (false at creation AND after resize recreation); on the init frame the bridge
   declares BOTH history slots `undefined()` (the sibling is physically
   UNDEFINED — a GENERAL seed would lie) + `set_first_frame(true)`, and the
   feature's record initializes BOTH slots; steady state uses `seeded_history`.
   I5 suppression is graph-global in v1 (one history feature); per-resource
   granularity = v1.1.

### D3 — Ordering: engine-owned frame line + two enumerated extension points + chain

```rust
#[non_exhaustive] #[repr(u8)]
pub enum ExtensionPoint { AfterResolve, BeforePresent }
```

Order within a point = registration order (EnginePlugins composition order —
documented contract). No arbitrary edges/anchors (the documented Bevy
"TAA-before-bloom-by-accident" conflict surface; RDG/rend3 single-owner
insertion order has none; our frame IS a line — linear compile preserved, zero
topo machinery). **Chain contract**: bridge keeps a per-frame `chain: GraphImage`
seeded to `lit[fi]` (or `hist[fi]` when engine-native TAA is ON — user post
composes after TAA naturally); chain-point nodes `chain_input()` /
`set_chain_output(img)`; present blit samples the FINAL chain value. Zero nodes
⇒ chain ≡ engine seed ⇒ declaration byte-identical ⇒ pins hold (critic-verified
reduction).

### D4 — Layering: `boyko_rhi_vulkan::rendergraph` (pub) re-exported as `boyko_app::render_graph`; borrowed-recorder ctx, restricted surface

**Recording substrate (critic P1-1)**: `VulkanCommandEncoder` owns its pool +
buffer and cannot target the frame's open swapchain cmd. Split out
`RecorderCore<'a> { fns, cmd }` carrying the moveable vkCmd bodies; the encoder
keeps pool ownership and delegates. **Four encoder methods are state-coupled and
NOT moveable** (critic-verified list for the implementer): `begin`/`end`
(own-buffer lifecycle + cache reset), `bind_storage_buffer` (updates the
encoder's own set), shared-layout `push_constants`, `dispatch` (conditional
fixed-set rebind) — these stay encoder-level as rebind-then-delegate wrappers,
byte-identical for the existing GpuColumn path.

**Restricted ctx**: `PassRecordCtx` exposes NO begin/end and NO
barriers/layout transitions — the graph owns ALL sync (a user transition
desyncs per-ResId tracking). Graphics goes through bracketed
`render_to(color, |draw| …)` deriving attachment + layout from the DECLARED
write (begin/end-rendering misuse unrepresentable; closure is a monomorphized
stack closure — no dyn, no alloc). `unsafe fn raw()` carries the clause:
recording any barrier/transition through raw() is forbidden.
`ctx.push_constants` routes the layout from the ctx-tracked last-bound pipeline
(critic R4 — the shared-vs-dedicated layout trap documented on the encoder must
not resurface). Object-safety note corrected: `RhiCommandEncoder` IS dyn-able;
the concrete ctx stands on per-vkCmd static dispatch (direct inlinable FFI
calls).

### D5 — Semantic access vocabulary (closed presets + runtime subresource)

`ImageAccess` presets: SampledFragment/SampledCompute, StorageRead/Write/
ReadWriteCompute, **StorageReadVertex** (raster interp_draw = VERTEX_SHADER +
SHADER_READ), ColorAttachment, DepthAttachment, DepthRead, TransferSrc/Dst.
`BufferAccess` analogous + Indirect/VertexInput/IndexInput. `const fn lower`
table — zero cost. **Runtime subresources (critic P1-5)**: `read_sub/write_sub`
take `Subresource { base_mip, mip_count, base_layer, layer_count }` — CSM's
`depth_layers(active)` is a runtime value a closed enum cannot carry; plain
`read/write` default single-layer mip 0. Aspect derives from the image FORMAT
(never user-specified). The two-access idiom (TRANSFER then COMPUTE on one
resource in one pass — the cluster alloc counter) is sanctioned and must be
expressed in the G1 dogfood. Sufficiency proof = G1's byte-identity gate.

### D6 — Phase 2/3 reservations

`TransientImageDesc.lifetime: ResourceLifetime { FrameLocal, Persistent }`;
`ResClass` = aliasing exclusion input; registration carries
`queue: QueueClass { Graphics, AsyncCompute }` (v1 panics on AsyncCompute;
Phase 3 = auto timeline-semaphore at first cross-queue use). Liveness derives
from the access arena — Phase 2 needs no public-surface change.

### D7 — Dogfooding: declaration-side unification; engine record stays static

`declare_gbuffer_graph` is rewritten THROUGH the public vocabulary (D5 presets,
`*_sub`, `add_image_history`, same `GbufferPassPlan` output); engine pass
names/order become enumerable (`frame_passes()`, cold). Record side stays the
static line + `record_point` hooks at the two extension points. **TAA is
engine-native per docs/TAA-PLAN.md** (config-gated declare, present-set rebind,
`[1-fi]` bridge resolution, `set_first_frame` reach — engine-internal
capabilities the public surface deliberately does not carry in v1); TAA consumes
G0 and seeds the chain; it is NOT a rung of this ladder.

### D8 — All v1 transients FIF-slotted (critic-verified)

`[VulkanTexture; FRAMES_IN_FLIGHT]` per entry; resolve through `[fi]`. The C1
fence lesson codified: the `in_flight[fi]` fence drains N−2, not the sibling;
a single-slot transient either races or serializes sibling frames. ×FIF VRAM
until Phase 2 (Frostbite reclaim reference: 1042→472 MB @4K).

### D9 — Unified ResId→physical `res_table` (critic P1-4)

One preallocated ResId-indexed `res_table: Vec<ResolvedPhys>` (capacity = engine
max + user headroom; cold regrow + debug_assert per kernel-arena discipline),
cleared + repopulated at declare: engine rows exactly as today's fixed arrays,
pool rows from claims, history/imported rows by the bridge (incl. TAA's `[1-fi]`
engine-side). `GbufferBarrierSink` indexes it uniformly; `record_point` drives
the SAME sink. **Migration oracle correction (critic verify)**: the equiv pins
are ResId-LOGICAL (CountSink counts calls; barrier PODs carry no physical
handles) — a transposed row would be pin-green and silently corrupt. The G3 gate
therefore includes a **direct row-for-row handle-equality test** between the
populated engine rows and the legacy arrays — that test is the actual migration
oracle; the pins remain the declaration oracle.

### D10 — Node registry ownership: World NonSend resource (critic v2-verify P0-2 fix)

The registry CANNOT be Renderer-owned: `Renderer` is a local of `run_windowed`
— it does not exist at `add_render_pass` time and is unreachable from `&mut App`
between frames. **The MeshRegistry precedent is the pinned shape**:
- `add_render_pass` (before `run()`) pushes into an App-side staging vec
  (boyko_app extension surface — `boyko_ecs::App` cannot name the trait).
- At runner boot, staging moves into a **World NonSend resource**
  `RenderNodeRegistry` (inserted BEFORE `finish()`, like MeshRegistry);
  `setup(RenderSetupCtx)` runs per node at that point (device live).
- The frame loop borrows it `&mut` during declare/record — which now genuinely
  yields the static between-frames confinement of `render_node_mut` (it goes
  through the World resource; the frame loop's borrow excludes concurrent
  access by construction).
- **Teardown order pinned**: `RenderNodeRegistry` is evicted (and node-held
  facade resources destroyed via the host's facade tables under the step-1
  idle) BEFORE `destroy_singleton` — exactly the MeshRegistry discipline.
Pool / `res_table` / facade handle tables stay host-side (WindowHost).

### D11 — Setup facade + bind-set policy

`RenderSetupCtx` (setup-only device facade; critic P0-3): `create_shader_module
(user SPIR-V — the shaderdsl byte-identity rule governs ENGINE shaders only)`,
`create_compute_pipeline`, `create_graphics_pipeline` (fullscreen-tri class;
target format comes from the user's own transient desc — known at setup),
`create_bind_layout/set`, `create_sampler`, `create_host_ring` (FIF-slotted).
Ids are u16 newtypes into host-owned dense tables. **ALL bind sets are
FIF-slotted in v1** (critic R1 — `write_bind_set` on a set the sibling frame's
cmd references is the update-while-pending violation; slotting everything is
trivially sound; an update-frequency flag is v1.1). `on_resize` discipline
(critic R2): prefer `write_bind_set` updates / slot-in-place recreation; table
growth bounds documented. No engine-image format query in v1; `render_to` on
engine images is NOT legal until one exists (critic P2 note).

## Node trait + channels (pinned final at G2)

```rust
pub trait RenderPassNode: Any {
    fn name(&self) -> &'static str;
    fn setup(&mut self, ctx: &mut RenderSetupCtx<'_>) {}
    fn on_resize(&mut self, ctx: &mut RenderSetupCtx<'_>) {}
    fn is_active(&mut self, frame: &FrameInfo<'_>) -> bool { true }
    fn declare(&mut self, b: &mut PassBuilder<'_>);
    fn record(&mut self, ctx: &mut PassRecordCtx<'_>);
}
#[non_exhaustive]                     // the GROWABLE channel — never the trait params
pub struct FrameInfo<'a> { pub frame_index: u32, pub frame_counter: u64,
    pub composite_extent: Extent2D, pub alpha: f32, pub time_secs: f64,
    pub camera: &'a CameraParams }
pub struct NodeHandle<N> { .. }       // typed post-setup &mut channel (D10 reachability)
impl App { pub fn add_render_pass<N: RenderPassNode>(..) -> NodeHandle<N>;
           pub fn render_node_mut<N>(..) -> &mut N; }   // Any-downcast; mismatch = panic
pub trait Plugin { fn render_passes(&self, reg: &mut RenderPassRegistrar<'_>) {} }
```

`declared: Vec<Option<PassId>>` discipline (critic P1-7): `declare_point`
visits EVERY slot EVERY frame and stores unconditionally — PassIds are
reassigned per frame; a stale `Some` drives the WRONG pass's barriers.
Stale-slot test mandatory at G2.

## Rung ladder (each green, clippy-clean, committable; pins = standing oracle)

| Rung | Scope | Gate |
|---|---|---|
| **G0** | `seeded_history` + `ResClass` + I5 (release) + `add_image_history` + `set_first_frame` + first-frame lifecycle contract + unit tests | framegraph units; pins untouched. **Unblocks engine-native TAA.** |
| **G1** | D5 vocabulary + const lowering; re-express ALL of `declare_gbuffer_graph` (CSM layered subresources + the two-access idiom included) | **Pins byte-identical 23/10/22** — the sufficiency proof. |
| **G2** | Registry + full pinned trait + `NodeHandle` downcast + `PassBuilder` + `declare_point` + unconditional `declared[]` + `res_table` declare-side | Pins hold (empty registry); registry units: bucketing, skip, cap panic, post-finalize panic, **stale-slot**. |
| **G3** *(after APP-HOST R7 — shared present/bootstrap+resize seam)* | `RecorderCore` split + restricted `PassRecordCtx`/`DrawRecorder` + `record_point` + chain + claim-tracked `TransientImagePool` + physical-keyed lazy descriptor cache + sink over `res_table` | Pins hold; **two-nodes-same-desc ⇒ two physical images**; **res_table row-equality vs legacy arrays** (the real migration oracle); cache resize-invalidation; `rendergraph_dispatch` < 1 µs; zero-alloc counter. |
| **G4** *(after R6/R7)* | D10 staging→World registry + D11 facade + `boyko_app::render_graph` + `add_render_pass`/`render_node_mut`/plugin hook + out-of-crate smoke node | Pins with node off; smoke builds using ONLY the public surface; owner visual. |
| **G5** | User-shaped dogfood: chain post-effect (vignette/tonemap-class) — transient + chain + host ring + `NodeHandle` game param + facade. *Depends on TAA-PLAN T2+ for the compose-with-TAA gate (cross-plan dependency stated).* | Owner visual; pins with effect off; TAA ON + user node ⇒ correct compose, validation-clean. |
| **G6** | Diagnostics (`frame_passes()`) + book chapter (doc-writer) | Cold; pins hold. |

**v1-core cut (owner option):** G0–G3 deliver the full mechanism (TAA unblocked,
engine-side user passes possible); G4+ is the `boyko_app` sugar — deferrable
until the first external pass demands it.

## Validation

Pins at every rung; G3 row-equality; first-frame flow tests (both-slot init,
steady no-barrier, resize re-arm); claim-tracking tests (N-sibling); stale-slot;
chain identity + redirect; descriptor-cache invalidation; runtime layered
subresource (G1); cap/AsyncCompute/post-finalize panics; `NodeHandle` mismatch
panic; `rendergraph_dispatch` microbench; zero-alloc counter (X.E methodology).

## Open questions

1. UI composite position vs `BeforePresent` (boyko_ui record site to confirm).
2. Owner VALUES: expose `Depth`/G-buffer reads to user nodes in v1 (enables
   user SSAO-class; buildable now via the facade) — or chain-only first?
3. v1.1: per-resource I5 granularity; bind-set update-frequency flag; transient
   eviction; engine-image format query + `render_to`-on-engine-image legality.
