> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §4 rungs S0–S2. Ladder gate: see the index.

### S0 — the seam, the gate, the observer — **size L**

*Architecture D31 + D6a + D6b + D32. This is the first landable rung.*

**Lands.**

1. `boyko-ui` moves from `[dev-dependencies]` to `[dependencies]` in `boyko_render/Cargo.toml`; the
   `TEST-ONLY` annotation at `:106-118` is rewritten to record the promotion and its reason (the
   layering rule at `boyko_render/Cargo.toml:7-13` names this crate as the home of per-entity GPU
   data paths).
2. **`ui_pack_inputs!`** — a `macro_rules!` in `boyko_render::ui` that expands to **both** the
   `Or<(Changed<…>, …)>` filter type of `ui_render_discovery` **and** the gather's per-node read list,
   from **one** spelling. At S0 the list is the four that exist today:
   `ComputedRect`, `UiBackground`, `ComputedClip`, `StackIndex`. Animation adds `UiVisual` here;
   sprites add four here at S3–S5; interaction adds its scroll datum here.
3. **`boyko_render::ui::gather_ui_nodes`** — the canonical gather, a DFS over `UiRoot`/`Children`
   carrying the inherited clip on its stack, mirroring `collect_candidates` (`focus.rs:204-257`)
   line for line. Its pre-order **is** the paint order D4 pins, so the renderer's z-order and the
   hit-test's `paint_seq` are one traversal rather than two that must be kept in agreement.
4. **`ui_render_discovery`** — one normal system whose `Query<(), ui_pack_inputs!(changed)>` bumps
   `UiRenderGeneration` once per changed frame. One site, not fifteen.
5. **The two-phase seam** (the architect's 2026-08-21 WorldView ruling — sequence, never fuse,
   inside one `UiUploadSystem::run_dispatcher`, mirroring the shipped `GpuSystem` ordering):
   **Phase 1 (shared borrow)** — the per-slot `[u64; FRAMES_IN_FLIGHT]` generation gate, hoisted
   AHEAD of the gather (D6a; a static frame costs one `u64` compare and **zero** component probes —
   the structural skip), then `gather_into_staging(&mut self, view: &WorldView<'_>) -> usize`
   (gather + pack + z-sort into the system-owned staging `Box`, no `!Send` type in the signature,
   device-free, unit-testable with a bare `EcsMaster`), the view dropped at the phase's closing
   brace; **Phase 2 (exclusive borrow)** — `token.nonsend_resource_mut::<RhiContext>()`, then
   `upload_staging(rhi, packed, ortho, token)` (no `WorldView` in the signature, so the fusion
   cannot be re-written). `self.staging` is a preallocated `Box<[UiInstance]>` sized at
   `initialize`, never grown in the frame loop (the Principle-0 named legitimate exception: the
   staging mirror for a GPU-contiguity write; durable data stays in ECS columns). The fused
   predecessor `host_upload_frame_from_world` is **DELETED, not re-signed** — its parameter list
   WAS the defect (see the fact table).
6. Two **diagnostic** counters (not `#[cfg(test)]` — the §10.4 `relayout_count` lesson): probes/frame
   in `gather_ui_nodes` and repacks/frame in `pack_sort_upload`. The two-phase seam carries its own
   pack census on the system (`UiUploadSystem::repacks`), because Phase 1 reads the world through a
   read-only `WorldView` that cannot project `&mut` to a `Resource`.
7. **The observer — re-pointed at Phase 1 alone** (the 2026-08-21 ruling): a device-free test on a
   bare `EcsMaster`, no graphics type in sight — one hardcoded panel, discovery + the two-phase
   dispatch, the staged records asserted **by value** (scale folding, z-order, packed count) in
   `tests/ui_s0_seam.rs`. Cheaper than a windowed rung AND unit-testable. The windowed `boyko_app`
   UI rung (D32's floor) is deferred to the rung that makes the swapchain `Renderer` an ECS
   resource — until then the host drives the seam via
   `stage_frame(token, ortho)` → `run_system_once` → `take_frame_output()`.

**No instance change. No shader change. No `.spv` change.**

**Gate.**

| # | Claim | How |
|---|---|---|
| **G0-1** | The gather reads every pack-input component, and cannot drift | Compile-level: `ui_pack_inputs!` is the only spelling. Plus a behavioural test — a world with one node, mutating each pack-input component in turn, asserting the generation bumps **exactly once** per mutation and **zero** times on an unrelated component. |
| **G0-2** | The structural skip is Phase 1's contract | Asserted on the **COMMAND CENSUS**, not a timing delta: 10 consecutive static dispatches of the two-phase `run_dispatcher` record **zero** on both census counters — zero component probes, zero packs — i.e. zero recorded work on an unchanged generation (`ui_s0_seam.rs::g0_2_static_frames_record_zero_census`). |
| **G0-3** | The packed-count return crosses the seam | Phase 1's contract: mutate one node once ⇒ the next dispatch packs **exactly once**, `gather_into_staging`'s packed-count is observable off the system (the count AND the repacked row carrying the new value — not a stale re-serve), and the dispatch after that skips again (`ui_s0_seam.rs::g0_3_one_mutation_one_repack_count_returned`). |
| **G0-4** | The DFS carries the inherited clip and its pre-order is paint order | A three-level tree with a clip at the middle level: the leaf's packed `clip` is the ancestor's, and the emitted `append` order equals `collect_candidates`'s `paint_seq` for the same tree. |
| **G0-5** | **THE SEAM GATE: the fusion is unrepresentable** | Two halves. (1) Signature pins: Phase 1's signature names **no** `!Send`/graphics type, Phase 2's names **no** world type — pinned by fn-pointer coercions that stop compiling if either signature grows the other phase's type (`ui_s0_seam.rs::g0_5_seam_signatures_do_not_cross`). (2) A trybuild fixture where **re-fusing them fails to compile**: one call site holding Phase 1's `WorldView` live across Phase 2's `&mut RhiContext` projection is E0502 (`tests/ui_s0_seam_fusion/refused_refusion.rs`, blessed `.stderr`). That makes the fusion unrepeatable rather than fixed. |
| **G5-11** | The repeat count is DERIVED, the bit layout is what the shader reads, and every degenerate input is `Stretch` | *(added 2026-08-26 at the landing.)* **The rung's headline mechanism had NO device-free gate**: an eight-input ratio with a `round`, two clamps and four degenerate cases, exercised only by G5-7 and G5-8 — two `BOYKO_UI_GOLDEN_REQUIRE_DEVICE` goldens that `boot_or_skip` past on a GPU-less box. A CPU table over `pack_ui_nine_slice_instance`'s FLAGS word on G4-3's own scene: the derivation COMPUTES `(4, 2)`; the four corners pack **no tile bits at all** (the ABSOLUTE form of S-D15's byte-identity claim — see S-D19 (6)); the top/bottom edges pack `(4, 1)`, the left/right `(1, 2)`, the centre `(4, 2)`; `Tile` moves NO source and NO destination coordinate; and each of `ui_nine_slice_tiles_axis`'s degenerate inputs yields `1`. It also gates the bit LAYOUT (bit 5, 6..=12, 13..=19), which no picture can see. CPU, device-free. |
| **G5-12** | The cursor pairing is structural at the AUTHORING site | *(added 2026-08-26 at the landing — S-D19 (5): `#[require(UiSpriteCursor)]` PANICS, because the require pass resolves the required id's pool in the target ARCHETYPE and a dense id owns none.)* Two legs: a node spawned from **`AnimatedSpriteBundle`** animates; and a node hand-spawned with `UiSpriteAnim` and NO cursor is FROZEN — asserted, because stating the hazard as an assertion is the difference between a documented hazard and a claimed one. CPU, device-free. |

**Red mutations.**

* **M0-a — hoist Phase 1's braces (delete the view drop).** The 2026-08-21 ruling re-specified this
  as "⇒ E0502 AT COMPILE TIME — a build failure, stronger than a runtime red; pins the brace as
  load-bearing". **That claim is REFUTED BY THE COMPILER** (probed the same day, ledger in the
  landing report and `docs/OPEN-QUESTIONS.md`): under NLL the view's borrow ends at its **last
  use** (`gather_into_staging(&view)`), so the hoisted-brace form **compiles clean — exit 0, no
  E0502, no warning**. What the brace actually is: scope hygiene against a future edit that HOLDS
  the view across Phase 2 — and the compile-time tripwire on *that* shape is real and demonstrated:
  it is M0-b below and G0-5's trybuild fixture, both of which red. The brace's comment in
  `upload.rs` records exactly this so the brace is not deleted as decorative. *(The pre-ruling
  M0-a — collapse the per-slot gate to a scalar ⇒ a skip serves the sibling slot's stale ring — is
  retired as a plan mutation; its rationale is pinned at the gate's field doc in `upload.rs`.)*

  **Disposition (orchestrator, same day): M0-a is RETIRED as a mutation.** Its premise is false and
  its property is covered twice over — at compile time by M0-b (the borrow crossing the seam) and
  structurally by G0-5's re-fusion fixture. A mutation kept alive after its predicted red is proven
  unfireable would be this plan's own gate-that-cannot-fail class, curated rather than caught.
* **M0-b — read the generation from the view across Phase 2 instead of from self.** ⇒ **E0502**,
  demonstrated 2026-08-21 (ledger): `cannot borrow 'token' as mutable because it is also borrowed
  as immutable` — the view minted in Phase 1, the `&mut` projection at Phase 2's head, the
  view-read after it named as "immutable borrow later used here". *Pins that the gate's RESULT
  crosses the seam — the packed count, a copied `u64` — never its borrow.* *(The pre-ruling M0-b —
  move the compare into `pack_sort_upload`, splitting the two counters — is retired as a plan
  mutation; the two-counter split it argued for is now G0-2's census design, asserted on both
  halves.)*
* **M0-c — delete `ComputedClip` from `ui_pack_inputs!`.** The build fails at the gather. *A
  completeness test that checks a hand-kept list against the gather is checking a list against itself;
  only one spelling can fail to compile.*
* **M0-d — make `ui_render_discovery` bump unconditionally.** G0-2 reds (`repacks == 10`). *Proves the
  discovery filter is doing work rather than the counter being incidentally zero.*

**Measurement.** §10.8 legs **(a)** baseline probes/node/frame and gather µs at N ∈ {256, 2048}, and
**(d)** the static frame with the compare hoisted — which must read **zero probes** or D6a is not
wired where it claims to be. §10.3: repacks avoided on a static frame **and** the unchanged full cost
of a changing frame, both reported, so the module doc never again claims more than the mechanism
delivers. **Both previously-blocked legs are re-pointed at `run_dispatcher` as the bracket** (the
two-phase seam, driven through `run_system_once` — headless; `ui_s0_measure.rs`). **Scope note:**
Phase 2's upload cost is DRAW-adjacent host+transfer on the windowed device — compare only within
one scene, and name the GPU zone id before quoting a number; that half is the owner-run windowed
leg, not the headless harness. *Landed 2026-08-21 (this box, debug profile, Instant/QPC ~0.1 µs
floor): leg (d) static dispatch median 0.2 µs @ N=256 / 0.4 µs @ N=2048, probes = 0 asserted,
repacks avoided 100/100; §10.3 changed-frame full cost median 231.7 µs @ N=256 / 2250.3 µs @
N=2048 — the gate does not reduce it, and now the number says so.*

---

### S1 — the UI shader into the eDSL; both sync gates; manifest rows — **size M**

*Architecture D30. Sequenced before S2 per **R1**: the re-DXC gate must exist before the shader is
edited, or nothing notices a `.spv` that stops matching its source while the shader is being changed
repeatedly.*

**Lands.**

1. `boyko_shaderdsl/src/ui.rs` — the leaves, each one generic `C: Cf` body instantiated over `f32`
   (the host oracle) and `Emit` (the HLSL printer):
   `ui_sd_rounded_box` (the per-corner Quilez/Bevy SDF), `ui_clip_coverage`, `ui_median3`,
   `ui_screen_px_range`, `ui_premultiplied_over` (the border-over-fill composite),
   `ui_unpack_rgba8`.
2. `boyko_shaderdsl/src/bin/emit_ui.rs` — owns **both whole files** (S-D10), with the `UiInstance`
   field offsets, `UI_INSTANCE_SIZE`, and the three flag-bit constants as **generator inputs**, so no
   shader ever spells a byte offset that a host `offset_of!` also spells.
3. Sentinels in `ui_rect.vs.hlsl` / `ui_rect.fs.hlsl`.
4. `crates/boyko_render/tests/ui_rect_edsl_sync.rs` and `ui_rect_spv_sync.rs` — in `boyko_render`,
   because that is where the shaders live (`boyko_render/shaders/`), following the
   `particle_edsl_sync.rs` plumbing verbatim (`shaders_dir()`, LF normalization, `find_dxc()`,
   graceful SKIP with an `eprintln`).
5. Two rows in `docs/SHADER-VARIANT-MANIFEST.md` — `ui_rect.vs.spv` and `ui_rect.fs.spv`, both
   **no-`-D`-axis** rows, stated as such, with the frozen dxc recipe from each source's header
   (`-T vs_6_0` / `-T ps_6_0`, `-fspv-target-env=vulkan1.3`).

**Gate.**

| # | Claim | How |
|---|---|---|
| **G1-1** | Every generated span **is** the printer's output, and is inside the right function | `ui_rect_{vs,fs}_matches_edsl_emit` |
| **G1-2** | Each committed `.spv` **is** the re-DXC of its own source under the frozen recipe | `ui_rect_{vs,fs}_spv_byte_identical` — SKIPs without dxc, **and the skip is reported** (`PARTICLES-PLAN.md` F15) |
| **G1-3** | The `f32` oracle agrees with the emitted math | `ui_sd_rounded_box::<f32>` against a table of `(p, half_size, r)` points including all four quadrants and the degenerate `r == 0`; `ui_median3::<f32>` against the three orderings |
| **G1-4** | The `.spv` did not move | `SpirvBlob<2368>` / `SpirvBlob<7060>` unchanged |
| **G1-5** | The four UI goldens are unchanged | S-D6's hashes (landed at S2) do not exist yet at S1 — so at S1 the existing **texel** assertions plus G1-4's byte lengths are the gate, and this weakness is why S-D6 lands with S2 and not later |

**Red mutations.**

* **M1-a — change the `1e-5` `fwidth` floor to `1e-4` in the leaf and re-emit.** G1-2 reds (`.spv`
  bytes move). *Proves the byte gate is live and not a length check.*
* **M1-b — hand-edit one character INSIDE a sentinel span without re-emitting.** G1-1 reds. *Proves
  the eDSL owns the span.*
* **M1-c — hand-edit one character OUTSIDE every sentinel, in the skeleton.** G1-1 stays **green** and
  G1-2 reds. *This is the mutation that says why both tests exist: neither alone covers the file.*
* **M1-d — remove `dxc` from `PATH`.** G1-2 **SKIPs**. *This is not a defect; it is the rung's
  vacuum condition, and the rung is not called done on a skipped run. The mutation exists so that the
  skip is seen at least once by the person who has to report it.*

**Measurement.** §10.7 — byte identity of the re-emitted HLSL and the re-DXC'd `.spv`. This gate does
not exist today in **any** form.

**Honest note.** S1 delivers no user-visible change and is the rung most likely to be skipped under
pressure. It is sequenced third and not last because S2 is a six-site lockstep edit to a file that
today has *no gate proving its binary matches its source*.

---

### S2 — `UiInstance` 64 B → 80 B, all ten sites, one commit — **size M**

*Architecture D1. No feature lands here. This rung exists so that neither feature half widens the
record twice.*

**Lands.** The field list from D1, verbatim:

```
@0   min_px        [f32; 2]
@8   size_px       [f32; 2]
@16  clip          [f32; 4]
@32  corner_radius [f32; 4]   // ALWAYS the radius — the alias is retired
@48  uv            [f32; 4]   // NEW: normalized (u0,v0,u1,v1) — glyphs AND sprites
@64  color         u32
@68  border_color  u32
@72  border_width  f32
@76  flags         u32        // S-D2's bit assignment
                              // = 80 B, multiple of 16, no tail pad
```

**Lockstep sites — all ten, in one commit:** the Rust struct; `UI_INSTANCE_SIZE`; the ~~**ten**~~
**nine per-field** `offset_of!` const-asserts *(amended 2026-08-21 at landing: D1's field list has
NINE fields, so nine per-field asserts — plus the `size_of == 80` and `align_of == 16` pins beside
them. The "ten" carried forward the fact table's counting convention, which called today's eight
per-field asserts "9"; the same off-by-one, twice)*; S-D2's `BINDLESS_TEXTURE_CAPACITY` const-assert;
the `UiInstance` mirror in
`ui_rect.vs.hlsl`; the mirror in `ui_rect.fs.hlsl` (both now via `emit_ui.rs` — the offsets are
generator inputs, so this is one edit, not two, which is S1's dividend); the two `SpirvBlob<N>` byte
lengths; `pack_ui_instance`'s text branch (writes `uv`, leaves `corner_radius` zero); the Miri
byte-view test.

**Also lands: S-D6's four image hashes**, blessed on the 64 B build **first**, in a preparatory commit,
so "identical" has a referent. The two-commit protocol is the rung:

* **commit A** — add the SHA-256 assertion to each of the four UI goldens, blessed against the
  **current 64 B** build, with the BMP dumped and looked at;
* **commit B** — the widening, which must reproduce all four hashes.

**Gate.**

| # | Claim | How |
|---|---|---|
| **G2-1** | Rust layout is exactly D1's | ~~ten~~ **nine** `offset_of!` asserts *(one per field — the 2026-08-21 count amendment above)* + `size_of == 80` + `align_of == 16` |
| **G2-2** | The 12-bit slot field cannot silently truncate | S-D2's `BINDLESS_TEXTURE_CAPACITY <= 1 << 12` const-assert |
| **G2-3** | The widening is pixel-invisible | The four UI goldens reproduce commit A's hashes exactly |
| **G2-4** | The byte view is sound at the new size | the Miri byte-view test over an 80 B record |
| **G2-5** | The text lane really migrated | a CPU test asserting a `FLAG_TEXT` instance now carries the UV in `uv` and **zero** in `corner_radius` |
| **G2-6** | The reserved bits are actually zero | a CPU test asserting `flags & 0xFFFF_FFF8 == 0` for every packed instance at S2 |

**Red mutations.**

* **M2-a — swap `uv` and `corner_radius` in the *Rust* struct only.** G2-1 reds.
* **M2-b — swap them in the *HLSL* mirror only.** G2-1 stays **green**; G2-3 reds. *This is the
  mutation **R1** says nothing in the tree can currently see. It is visible only because S1 put the
  `.spv` under a byte gate and commit A put the image under a hash. If M2-b does not red, the rung's
  gate is decorative and the rung is not done.*
* **M2-c — raise `BINDLESS_TEXTURE_CAPACITY` to 8192.** G2-2 reds. *Proves the assert is against the
  live constant and not a copy of it.*
* **M2-d — leave the text lane writing `corner_radius`.** G2-5 reds and G2-3 reds (the glyph samples
  `uv == (0,0,0,0)`). *Proves the un-aliasing is complete rather than additive.*

**Measurement.** §10.2 — ring traffic 128 KB → 160 KB at N = 2048 (arithmetic, stated as such) **plus**
wall-clock pack+sort at N ∈ {256, 2048}, same scene, 64 B vs 80 B, criterion, median of the two builds.
The bytes are arithmetic; the time is not, and only the time can say whether the sort's gather (which
touches every record twice) notices.

*Landed 2026-08-21 (this box, RTX 3060 host, criterion 0.5 bench profile,
`benches/ui_pack_sort.rs` — one deterministic scene, 16 stack strata, every 4th node clipped, half
rounded): arithmetic 2048 × 64 B = 128 KiB → 2048 × 80 B = 160 KiB (+25 % bytes, touched twice by
the sort gather). Wall-clock medians 64 B → 80 B: **5.62 µs → 5.71 µs @ N=256 (+4.3 %)**,
**49.12 µs → 51.16 µs @ N=2048 (+4.7 %)** — the sort's gather notices the widening at a fifth of
the byte growth; the pack is compute-, not bandwidth-, bound at these N. Affordable; D1 stands.*

*Landing notes, 2026-08-21, recorded per S-D10/SR1 (the reconciliations the rung surfaced):*

* *The `.spv` MOVED, deliberately — this rung is the shader-surface edit S1's gates exist for.
  `ui_rect.vs.spv` 2368 → 2408 B (the mirror gains the `uv` member the VS declares and never
  reads); `ui_rect.fs.spv` 7060 → 7136 B (the mirror + the `FLAG_TEXT` branch reading `inst.uv`
  instead of the retired alias — the rung's whole semantic delta). The generated-HLSL diff was read
  before the re-bless and contains exactly those two changes; the reason is recorded at both
  `SpirvBlob<N>` pins (`src/ui/mod.rs`).*
* *Commit A blessed the four S-D6 hashes on the 64 B build (BMPs dumped and looked at); commit B
  reproduced all four EXACTLY on the 80 B build — G2-3 held with zero re-blessing.*
* *S-D6 refinement for the swapchain golden: its "full readback" is the WSI-owned frame, whose
  extent and byte order the driver decides (this box clamps the requested 64×64 window to
  **120×64 BGRA**), so that pin carries `(extent, is_bgra)` qualifiers beside the hash and is
  asserted only when the live shape matches the blessed one — a mismatched WSI shape prints a loud
  NOTE rather than silently passing-as-checked. The three offscreen goldens pin fixed 64×64 /
  128×128 RGBA frames and carry the bare hash, as written.*
* *`UI_STAGING_ROWS`' doc arithmetic followed the stride (4096 × 80 B = 320 KiB — the S0 seam's
  staging box, untouched otherwise).*
* *An ELEVENTH lockstep site the ten-site list missed, found by the unconditional full-suite gate
  (`cargo test -p boyko-render --all-targets --no-fail-fast`), not by the enumerated ten:
  `ui_hud_screenshot.rs::hud_glyph_packing_golden` — a device-free test in the (otherwise
  `#[ignore]`d ×8) HUD binary that PINNED the retired contract verbatim ("the GPU pack lane
  carries that same UV into the `corner_radius` alias"). Its pin migrated with the field
  (`inst.uv == expected`, `corner_radius == [0;4]`). The lesson is the fact table's own: a
  grep-shaped site census misses a consumer that names the CONTRACT rather than the field.*

---

