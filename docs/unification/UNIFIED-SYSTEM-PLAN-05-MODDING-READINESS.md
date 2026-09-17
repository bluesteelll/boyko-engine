# Unified system plan — 05 Modding readiness (rev 5.1)

This is the carry target named by `docs/modding/MODDING-DESIGN-SPACE.md:3` (R3).

Tree tags (`[J]`, `[M]`) and the citation verification record are in
[00 Overview](UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md). The citations rev 5.1 adds are verified in 00 §10
(rev 5.1 changelog). Bare `:N` citations below are lines of
`[M]docs/modding/MODDING-DESIGN-SPACE.md` (closed at rev 6) unless a file is named.

**Rev 5.1** reconciles this file with the closed modding design (rev 6, including critique pass 6's
remarks, `:2423-2517`) and with the owner's hard requirement H-1 (§1). It changes three things:
- The kernel contract carries only what **every live modding option** needs (§3.1). Option-specific
  items move to modding Stage 3 and are built only for the chosen option (§3.2).
- Every kernel modding item is **not compiled** in a game that does not use modding. Rev 5 said
  "dropped at fat LTO"; rule S-1 (§1) replaces that.
- The no-cost gate gains the checks that prove "not compiled" (§6).

## 1. The requirement as testable properties

**H-1, the owner's hard requirement (2026-09-16).**
- Modding is optional. A game that does not use it pays no overhead of any kind: no indirection,
  lookup, branch, registry, lock, allocation, exported symbol, binary size or startup work.
- The target is code generation identical to an engine without modding support: the modding layer
  is, for example, a cargo feature or a crate that is simply not compiled.
- Consequence for the kernel: compile-time type identity, storage and dispatch stay in force for the
  engine's own types, and any dynamic registration path for mod types is additive and compiled out
  when unused.

**Properties.**
- **P1 — modding off costs nothing.** The non-modding build is identical on every UG-15 leg to its
  parent without the modding delta.
- **P2 — modding on, no mod loaded.** No per-frame or per-row difference; the startup delta is
  budgeted (MD:M-A3, queued in MQ-09). This configuration is not an arm of UG-15 leg (4) (03 §6).
  P2 differs by option; §3.1 states it per option.
- **P3 — a mod loaded.** Engine paths are unchanged; mod inner loops are native
  (`MODDING-DESIGN-SPACE.md:104-116`).

**The mechanism has two halves.**

**(a) The modding layer is crates, never a feature.**
- Modding is the crates `boyko_mod_host` (host image), `boyko_mod_api` (mod image) and
  `boyko_mod_registry` (modding game binary only).
- H-1 admits either a feature or a crate. The plan takes crates: Cargo unifies features across the
  graph, so any dependency can switch a `modding` feature on for a game that never asked, with no
  diff in the game's source (`:637-641`). A crate the game does not depend on is not compiled.
- There is no cargo feature on any kernel crate (`:2003-2010`).
- `boyko_modding` (the allocator's name) is retired in favour of these three.

**(b) Rule S-1: the kernel half is not compiled either.**
- **The rule.** Every modding item that adds code to a kernel crate (a function body, a type with
  methods, a trait impl) is generic over `ModSeam` (01 KC-19b). `ModSeam` is a `#[doc(hidden)]
  pub unsafe trait` with no methods and **no implementor in any kernel crate**. Its implementors
  live in `boyko_mod_host`, `boyko_mod_api`, `boyko_mod_registry` and census test crates.
- **Why it holds at every profile.** A generic function is compiled only where its type arguments
  are known ("the compiler can only compile a generic function when it knows the specific type
  arguments it is instantiated with", matklad, the source cited at
  `[M]docs/modding/MODDING-RESEARCH.md:214-223`; re-read 2026-09-17). A game that links no crate
  naming a `ModSeam` implementor instantiates none of these bodies. They then have no object code
  in the kernel rlibs or in the game binary, whatever the profile, LTO mode or codegen-unit count.
- **What it replaces.** Rev 5 argued "no engine caller, dropped at fat LTO". Critique pass 6 found
  that claim quoted beyond its measurement, which was taken with `codegen-units = 1`, not with the
  shipped profile (P6-8, `:2503-2507`). S-1 needs no linker or LTO argument.
- **Why not `#[inline]` alone.** The same source says an `#[inline]` body is compiled "with every
  crate it is used in". Whether it is also emitted in the defining crate when unused there is not
  stated, and is not measured here. A generic body leaves no such question.
- **What S-1 forbids in a kernel crate, when made for modding:** a new non-generic body, a new
  `static`, a new field on an existing type, and a visibility change. Each is admitted only where
  §3.2 names it and UG-15 admits it. The recommended options need none (§3.1).
- **Residual (a prediction, not measured).** A `pub` generic body makes the items it names
  reachable from other crates even when it is never instantiated.
  - `NEXT_ID`, `NEXT_RESOURCE_ID` and `NEXT_EVENT_ID` are already named by generic engine bodies:
    `register_new::<T>` (`[J]crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:920-921`),
    `resource_registry::register_new::<R>` (`[J]…/resources/resource_registry.rs:164`, `:175`) and
    `register_event_new::<E>` (`[J]…/events/event_registry.rs:109-110`). MS-03's readers change
    nothing for them.
  - `QUERY_NEXT_ID` and `BUNDLE_NEXT_ID` are named today only by non-generic `#[inline(never)]`
    dispensers (`[J]…/iters/query/query_type_registry.rs:98`, `:125-132`;
    `[J]…/bundle/bundle_type_registry.rs:93`, `:114-120`). MS-03's readers make them reachable.
  - The shipped binary is fat-LTO'd, which internalises every symbol it does not export. UG-15 legs
    (7) and (7b) on D-S1(ii) are the check.
- **Decision, by performance.**
  - S-1 costs a non-modding build 0 instructions and 0 bytes at every profile. A modding build pays
    one copy per token, i.e. one per image.
  - The rejected alternatives: a dead non-generic body costs bytes at default release
    (`REFLECTION-ANALYSIS.md:1467-1494`, quoted at `:89-92`); a cargo feature re-opens feature
    unification.
  - **Overturned by:** leg (7b) finding a seam symbol in a kernel rlib's object code, or leg (7)
    moving on a seam commit. The item then leaves the kernel for `boyko_mod_host` (modding R1,
    `:1818-1823`).

## 2. What stays compile-time in the contract

| Shape | Engine path (unchanged) | Evidence |
|---|---|---|
| Component identity | per-type `static ID: OnceLock` | `[J]crates/boyko_macros/src/component.rs:372-375` |
| Storage choice | associated consts, per monomorphisation | `[J]…/component/component.rs:51-126` |
| Pool construction | layout-erased `ComponentPool::new(id, rows)`; one path for engine and mod | `[J]…/component_pool.rs:279` |
| Scratch storage | KC-10, registry-free (no id at all) | 01 |
| Query / iteration | generic, instantiated in the caller | modding §0.1 |
| System dispatch | one indirect call per system run (existing). No modding item adds a field to `SystemBox` or `Schedule`, or a branch or hop to the dispatch loop; option A's extra hop is inside its own `System` impl (MS-12) | `[J]…/schedule.rs:1380`, `:1215` (modding `:1795-1804`) |
| Save identity | the derive's `stable_name` (default `type_name`), indexed at mint | `[J]…/component/component.rs:195-197`; `[J]…/component_registry/serialize.rs:383-398` |
| Kernel modding seam | generic over `ModSeam`; no object code without an implementor (S-1) | 01 KC-19b |
| Generic components | KC-20 hashes on uncached paths only; not a mod path | engine ED20 |
| Groups / chain | ChainKey authority is a type (pub(crate)); mods cannot open or close engine chains | physics rev 5 |

**Invariant I-1** (01 §1) is gated by UG-15 leg (2). **Invariant I-4** (01 §1) carries S-1.

## 3. Additive modding paths

### 3.1 What each live option needs from the kernel

**The live ordering** (`:1874-1877`): A′-nightly (pending MD:M-F1 (d)) > C-3a ≈ A′-stable, with C
route 2 unplaced pending MD:M-C5, and A third. No head is measured (R3-3, §5).

| Option | Place | Kernel delta it needs | Configuration-scoped part, and its crate | P2: modding built in, no mod loaded | Decided by |
|---|---|---|---|---|---|
| **A′** — the engine as a Rust `dylib`, modding configuration only | head on nightly; ties with C-3a on stable | **MS-03 only** | MS-14: the `crate-type` flip as a manifest overlay used only to build the modding executable (route (ii), `:697`); the loader → `boyko_mod_host`; the mod entry and build canary → `boyko_mod_api` | The modding executable forfeits fat LTO whether or not a mod is loaded: 1.18×–2.67× on the 7-bench suite, geomean 1.26×, on stable (`:723-733`). "Forgone only by players who load mods" (`:741-743`) holds only if the static executable runs when no mod is installed | MD:M-F1 (a)–(d); M-T2 informational |
| **C** — install-time relink, route 3a or route 2 | 3a ties with A′-stable; route 2 unplaced | **MS-03 only** | MS-15: the `.boykom` registry, per-mod `--undefined` roots and the collected-count assertion → `boyko_mod_registry`, a dependency of the modding game binary only | 16 B of sentinels and a zero-iteration boot walk (`:1242-1255`), measured by MD:M-C0b | MD:M-C4, M-C5, M-C1, M-C3, M-C0b |
| **D** — source mods | after C (`:1979-1984`) | **MS-03 only** | as C, plus a compiler | as C | C's MD:M-C1 |
| **A** — exact-build `cdylib` | third | MS-03, plus MS-01, MS-02b and MS-08..MS-13 at Stage 3, and RM-1's item | façade, trampoline and probe → `boyko_mod_api`; host seams → `boyko_mod_host` | startup scan only; 0 per frame (`:647-656`) | MD:M-A1, M-A2, M-A4; Stage 0 (iii) |
| **B**, **E**, **F** | rejected; fork; not priced (`:1971-1977`, `:1986-1987`, `:1380-1407`) | not planned | — | — | Stage 0 (ii) makes B mandatory; Stage 0 (i) makes E or F mandatory |

**Result.**
- **Phase D builds only the common set:** `ModSeam` and MS-03's four readers (01 KC-19b, rung
  D-S1(ii)).
- Choosing among A′, C and D changes no kernel rung. Choosing A adds MS-01, MS-02b and MS-08..MS-13
  at Stage 3, each under S-1.

**Why the common set is this small.**
- A′ shares every static, so a mod may call `world.query` and it is correct (`:715-719`). Its
  modding §7.10 row has no kernel delta (`:1834`).
- Under C, mod components go through the ordinary derive and nothing is registered dynamically
  (`:1083-1087`). C's kernel needs are modding §7 items 3, 5 and 8 plus the registry
  (`:1952-1954`):
  - item 5 needs no kernel change (`:1662-1665`);
  - item 8 is I-1;
  - the registry is MS-15, outside the kernel.
- Modding §7.9's seams are not needed by A′, C or D (`:1707-1708`).
- What remains is item 3's occupancy reads, which the loader takes under A′ and C (`:1653-1656`).
- **The descriptor path (modding §7.1) is not common.** Under A′, C and D a mod component is a Rust
  type with the ordinary derive. Only A (and B, E) needs a runtime descriptor. Stage 0's
  prototype-only scope (00 §7 Q-1 (iv)) removes it under A too (`:2164-2170`).

### 3.2 The paths

"Options" names the options that need the path. "Kernel delta" is written in S-1's form.

| MS | Path | Options | Kernel delta | Owning crate for the rest | Non-modding cost | Gate | Lands |
|---|---|---|---|---|---|---|---|
| MS-01 | Descriptor path (modding §7.1): an input struct fanned out into the existing parallel tables | A (B, E) | one `unsafe fn` generic over `ModSeam` that writes the clone, map-entities, serialize and residency rows for a minted id, by value, from inside `boyko_ecs`. **No visibility change:** rev 5's `pub` on `try_register_dynamic` (`[J]…/component_registry/mod.rs:967`), `set_residency_class` (`:614`) and `install_map_entities_fn` (`[J]…/component_registry/clone.rs:181`) is withdrawn; the mint is MS-02b's. **`set_storage_kind` (`:435`) stays `pub(crate)`:** P40 makes the kind a parameter of the mint (`ALLOCATOR-DESIGN-SPACE.md:3692`) | struct, validation, `extern "C"` → `boyko_mod_host` | nothing compiled (S-1); `ComponentLayout` stays 56 B (`[J]…/component_registry/mod.rs:133`) | UG-15 strict with legs (7) and (7b) | Stage 3, only if A is chosen and Stage 0 keeps mod-defined component types |
| MS-02a | Name-keyed identity for mod types: a mandatory `stable_name`, refused when the full name is already registered | all | none. **Precondition, an engine defect (RM-2):** a duplicate full name is appended, not refused (`[J]…/component_registry/serialize.rs:394-397`), and resolution returns the first match (`:415-421`) | mandatory-name check in the SDK's registration macro (`boyko_mod_api` / `boyko_mod_registry`); refusal as a loader `Result` (MS-06) | 0 | RM-2's UG-19 row | RM-2's fix as an engine rung; the SDK half at Stage 3 |
| MS-02b | Dynamic sized mint by name: `intern_or_mint_sized(name, size, align, kind)` (P40's rule in a separate sized body); P32 R1 (`TAG_NAMES` → `DYN_NAMES`, a rename); class-A entries `ComponentLayout::new_dynamic`, `try_register_dynamic_by_name`, `dynamic_by_name` | A (B, E) | the sized body and the entries generic over `ModSeam`; the rename is a rename | mandatory-name check → host | nothing compiled | UG-19 sized-name rows; UG-15 strict with (7) and (7b) | Stage 3, with MS-01 |
| MS-03 | Id budget: P39's skip mint (KC-19a, a bug fix, D-S1(i)) and occupancy value reads | all | `ModSeam`, plus four `pub fn`s generic over it that return the query, bundle, resource and event counters **by value**. Component occupancy is KC-19a's `id_space_census()`, an engine item (`ALLOCATOR-DESIGN-SPACE.md:3657`) | the bound (D2's `ENGINE_COMPONENT_CEILING`, held by a census over derives, tags, asset kinds and group columns) and the quota for the whole mod set → `boyko_mod_host` (A′, A) or `boyko_mod_registry`'s boot walk (C, D). Under A′ and C, query ids mint at schedule build, after the load slot, so the query quota is read after `App::finish()`, against the whole set (R3-1) | nothing compiled; residual in §1 (b) | UG-15 strict with (7) and (7b) on D-S1(ii); MD:M-K3 | **Stage 1**, D-S1(ii) |
| MS-04 | Dense storage required for mod components on engine entities | all | none (`STORAGE_IS_DENSE`, `[J]…/component/component.rs:79`) | A: the façade. A′, C, D: a const assertion over `STORAGE_IS_DENSE` emitted by the SDK's registration macro | 0 | A: MD:M-A4 (a). A′, C, D: a `compile_fail` fixture in the SDK | Stage 3 |
| MS-05 | Config-phase load slot | all | none | A′, A: `ModLoaderPlugin` → `boyko_mod_host`. C, D: the boot walk → `boyko_mod_registry` | 0 | — | Stage 3 |
| MS-06 | `Result` on register / load / attribution; param-fetch panic policy **unchanged** | all | none | host / registry | 0 | — | Stage 3 |
| MS-07 | Load-only ruling: no `FreeLibrary`; every world dropped before the image | all (C, D: by construction) | none | host | 0 | written rule | Stage 3 |
| MS-08 | By-id structural ops: consume KF-47 (KC-21), marked `MOD-SEAM` | A (B, E) | doc lines only | — | 0 (engine clients exist) | UG-15 (5) | markers with A6, as 02 schedules (no code) |
| MS-09 | Mod components are POD in v1: `drop_fn` and `clone_fn` refused at the seam; `map_entities_fn` and the SerPod blit allowed | A | none | seam assert → host | 0 | `mod_seam_pins` (P26.2 rows ii–iii) | Stage 3 |
| MS-10 | By-stable-name `QueryTypeId` mint + slot installer (`#[doc(hidden)]`) | A | 2 `unsafe fn`s generic over `ModSeam` | `ModQuery` `SystemParam` → `boyko_mod_api` | nothing compiled | UG-15 with (7) and (7b); MD:M-A4 (d) | Stage 3 |
| MS-11 | Counter probe: address accessors under a hidden `unsafe` seam; exported probe in `boyko_mod_api`. The value readers are MS-03's | A | 5 address accessors generic over `ModSeam` | probe → api | nothing compiled | UG-15 (1), (2), (7), (7b) | Stage 3 |
| MS-12 | fn-pointer system seam, re-specified for P6-4: `run(ctx, cell, last_run, this_run)`, optional `apply(ctx, world)`, descriptor `{flags: has_deferred / requires_dispatcher / is_gpu, access[], sets[], before[], after[]}` lowered host-side into `SystemConfig` | A | a fn-pointer system type generic over `ModSeam` with its `System` impl, and one `unsafe fn` entry generic over `ModSeam`. The `Box::new` and the vtable are emitted where the token is named, i.e. in `boyko_mod_host`, the host image (`:1776-1790`) | wrapper → host; trampoline → api | nothing compiled; `SystemBox` stays 40 B | UG-15 with (7) and (7b); MD:M-A4 (a), (e) | Stage 3 |
| MS-13 | Mint-refusal signal: **boundary-side** counter read-back (`cap + 1` poison). No kernel latch (P6-2). **Re-derived at its Stage-3 rung against the post-D-S1 dispensers:** the signal was derived at `mod.rs:938` and `:996` (two of the nine refusal paths listed at `MODDING-DESIGN-SPACE.md:1829`), and P39 changes both arms | A | none | api / host | 0 | MD:M-A4 (b), re-scoped (§5) | Stage 3 |
| MS-14 | A′'s configuration flip: the engine crates built as `crate-type = ["dylib"]` through a manifest overlay applied only when building the modding executable (route (ii), `:697`). Route (i), `["rlib", "dylib"]` in the shipped manifests, is refused: rust#51009 can drop fat LTO from the non-modding build with no diagnostic (`:700-711`) | A′ | none | the overlay and its in-step check, outside every shipped manifest | 0: the shipped manifests are unchanged. No `Cargo.toml` in `[J]` has a `crate-type` key (grep, 2026-09-17) | a manifest census (`crate-type` keys in shipped manifests = 0); MD:M-F1 (c); UG-15 strict | Stage 3, if A′ is chosen |
| MS-15 | C's plugin registry: one `.boykom$m` registration static per mod, the installer's per-mod roots, and the boot-time count assertion (`:1116-1130`) | C, D | none | `boyko_mod_registry`, a dependency of the modding game binary only (`:1831`) | 0: the crate is absent | MD:M-C0b; UG-15 leg (6) | Stage 3, if C or D is chosen |

## 4. Reconciling allocator §7 with modding rev 6

| Allocator item | Disposition | Reason |
|---|---|---|
| K-MOD-1/2/3: `HeapRef` as the mod allocator; one `Heap` per mod | **withdrawn** with U-1 | Mods use engine storage forms. The per-mod heap's reason (iii), O(1) unload, is void under load-only. Reasons (i) and (ii) apply equally to any engine structure a mod writes and are not solved by a heap. Revival: modding Stage 3 shows a need for mod-private freed memory; the heap then lives in `boyko_mod_host`. |
| K-MOD-4 (erased pool constructor) | kept = §2 row 3 | — |
| K-MOD-5 / P26 / P39 | kept = MS-02b (option A, Stage 3) / MS-03 | rev 5.1: the sized mint is not common to the live options (§3.1) |
| K-MOD-6 (`drop_fn` scoped to the seam, P44 O1) | kept = MS-09 | — |
| K-MOD-7 (not-exposed list) | kept; `FrameArena` and Heap removed from it; allowlist gate = UG-15 (1) plus a `raw` import census | — |
| K-MOD-8 (registry is a `Resource` in the modding crate) | kept | — |
| K-MOD-9 (no C symbol in the kernel) | kept = UG-15 (1): a `syn` census of export and retention attributes, non-Rust-ABI fn definitions and global assembly. It is never re-blessed; its owner-held allowlist is empty (03 §6) | — |
| K-MOD-10 `remove_component_type` | **deleted** (U-10) | load-only (modding §7.7, `:1680-1681`); kernel registries are write-once (`MODDING-DESIGN-SPACE.md:133-139`) |
| K-MOD-11 | already consumed (P38) = MS-08 | — |
| G6 (a)–(f) | merged into UG-15 | two-tree baseline (R1); parent-of-delta capture (P6-5) |
| `MAX_MOD_TYPES = 128` | replaced by MS-03's D2 bound | computed against MD:M-K3 after D-S2 frees the physics band |
| D1 / D2 / D3 (modding §7.3) | **D2 + P39 skip.** D1 is unavailable; D3 is refused. | Once U-2 deletes the band nothing pins production slots, and P39 makes collisions skip; D3's pinned-slot dispenser buys nothing. Overturn: MD:M-K3 shows engine late mints beyond the census → D3 as backstop. |

## 5. Carried open remarks: dispositions

| Remark | Disposition |
|---|---|
| R3-1, §7.3 direction | decided in §4 (D2 + P39). MD:M-K3 is a structural check at B2, after D-S3(ii) (group stores and their column ids) and after D-S3(iii). Rev 5.1: under A′ and C the quota unit is the whole mod set, read after `App::finish()` (MS-03) |
| R3-2, item 2b home | decided: boundary side (MS-13). The `try_register_dynamic` `None` gap is accepted as reachable only outside the façade: `try_register_tag` / `register_*tag` and asset construction join MD:M-A4 leg (a)'s forbidden set as `compile_fail` fixtures. MD:M-A4 leg (b)'s rev-6 system-less red control becomes a fixture, not a runtime latch read. |
| R3-3, ranking head | open; decided by MQ-09 (MD:M-F1 (d), MD:M-C4, MD:M-C5) and Stage 0. **Rev 5.1: the choice changes no Phase-D rung**, because A′, C and D need only the common set (§3.1) |
| R3-4, M-P1 first run | UG-15 at B3, then per delta; red controls required |
| R3-5, this file | exists |
| P6-1, profile reproducibility | a structural check at B3; if release builds are reproducible, release asm pins are added (UG-15 (2)) |
| P6-2, latch unadmittable | accepted → MS-13 |
| P6-3, `UnsafeEcsCell` layout / canary | build canary axes gain `debug-assertions`, `overflow-checks`, `panic` strategy, and every `BOYKO_*` build-env axis from `profile_axis_census.rs`; the host exports a layout fingerprint (`size_of` / `offset_of` of `EcsMaster`, `UnsafeEcsCell`) checked at load; the seam passes the cell only when fingerprints match (`[J]…/system/unsafe_ecs_cell.rs:65`) |
| P6-4, seam cannot express an ordinary system | MS-12's widened descriptor. Added cost per mod-system run: one argument pair. Per apply window: one optional indirect call. Modding-only. |
| P6-5, fixed baseline | pins captured at each modding delta's parent (UG-15) |
| P6-6, citation fixes | erratum in the modding doc: `run_if` at `system_config.rs:183-192`; `ConfigureSet::run_if` at `schedule_builder.rs:831-844` added to MD:M-A4 (a); `SystemMeta` `repr(C)` at `system_meta.rs:85` (all three re-read in `[J]`) |
| P6-7, M-P1 under-specified | UG-15 legs name the binaries (03 §6) and include item 2's accessors |
| P6-8, "absent at fat LTO" quoted beyond its measurement | **answered by S-1 (rev 5.1):** no kernel modding item relies on LTO to be absent |
| P6-9, per-mod-system price is a floor | option A only; carried with RM-1, which re-poses the TLS part of that price |
| P6-10, cross-image allocation exists today | option A only; failure mode 5 becomes a mechanical census over everything the façade re-exports, run as part of MD:M-A4 (a) |

**Remarks raised by rev 5.1.**

| # | Remark | Scope | Disposition |
|---|---|---|---|
| RM-1 | **KC-04 re-poses option A's TLS hazard.** Under A the mod image carries its own copies of KC-04's statics `THREAD_RECORDS`, `THREAD_BUSY` and `CTX_IDX` (01 §6 items 1 and 3). On its first lookup the mod's copy of `CTX_IDX` reads 0, so the mod allocates a second TLS index and claims a record in its own table, which reads detached: `par_iter` runs serially and the ALLOC1 checks pass vacuously, the symptoms of §0.3 instance 5 (`:212-217`). Handing the mod the host's index does not help, because the mod's code still indexes its own table. So MD:M-A2's "three TLS writes per system entry" (`:2134`) no longer describes the fix. Each mod image also spends one of the 64 fast TLS indices (00 RK-14, hazard 2) | option A only. A′, C and D have one image; a non-modding build is untouched | open, the architect's; MD:M-A2 is re-posed before A is chosen. A candidate, modding-only: the trampoline copies the host record's pool and lane fields into the mod image's own record at each system entry. Its item is generic over `ModSeam` (S-1) |
| RM-2 | **A duplicate full `stable_name` is not refused.** `register_stable_name` appends a second id under an existing name (`[J]…/component_registry/serialize.rs:394-397`), and `resolve_stable_name` returns the first match (`:415-421`). Two component types that declare the same `stable_name` therefore resolve silently to whichever minted first, at every load. The default name is `type_name` (`[J]…/component/component.rs:195-197`), so reaching this takes an explicit attribute. The call sits inside the derive's `component_id()` closure (`[J]crates/boyko_macros/src/component.rs:318-321`, `:392`) | engine defect (bugs first). MS-02a depends on it | proposed: refuse in `register_stable_name` when the bucket holds a different id with the same full name. The cost is one compare loop on a once-per-type cold path, in every build, because it is an engine fix, not a modding item. It is an attributed rung naming `Transform::component_id`; the architect places it |
| RM-3 | **Knock-ons in files this revision does not edit.** **02 D-S1(ii):** scope becomes KC-19b rev 5.1 ("MS-03" instead of "MS-02, MS-03"); its red-first test "a sized name re-minted with a different layout → `Err`" moves to MS-02b's Stage-3 rung; its rename list (`TAG_NAMES → DYN_NAMES`, 02 §2) empties. **03 UG-15 row, leg (7) and UG-16:** "MS-01's rung" is now a Stage-3 rung, option A only. **03 red control (vii):** `dynamic_by_name` no longer lands in Phase D; from D-S1(ii) on, its item is an MS-03 reader instantiated from `boyko_demo` with a control-branch token. **03 leg (5) and new leg (7b):** as §6 below, with controls (xi) and (xii). **03 UG-19:** the sized-name rows move from D-S1(ii) to MS-02b's rung. **03 MQ-09 and the structural list:** since A′ and C head the ordering, MQ-09 gains the timing legs of MD:M-F1 (c) and MD:M-C0b, and the structural list gains MD:M-F1 (a), (b) and the size legs of (c) and M-C0b | plan files 02, 03 | for the architect's next patch |
| RM-4 | **A′'s P2 needs a launcher rule.** A′'s modding executable pays A′'s price with or without a mod (§3.1), so "only players who load mods pay" requires shipping both executables and starting the static one when no mod is installed | A′ only | a Stage-0 product question beside Q-1; no kernel consequence |

## 6. The no-cost gate

This is UG-15 (03 §6).
- **Strict mode** runs on every commit of a modding delta, and on every kernel commit that touches
  a seam item.
  - A **seam item** is any kernel item named in §3.2's "Kernel delta" column, or any body in UG-15
    leg (2)'s pinned set.
  - A **seam commit** is a kernel commit that adds or widens an item named in §3.2's "Kernel delta"
    column:
    - in Phase D, D-S1(ii) only (rev 5.1: MS-01's rung moved to Stage 3);
    - in Stage 3, each kernel half of the chosen option's items (MS-01, MS-02b, MS-10 to MS-12, and
      RM-1's item under A).

    Every seam commit runs strict.
  - D-S1(i) touches pinned bodies as a bug fix, so it runs in **attributed** mode.
  - D-S1(ii) runs strict with its cut commit as its parent, and that commit contains D-S1(i)
    (02 §2). This split is what lets a moved pin be attributed.
- **Every seam commit also runs leg (7), the linked seam census.** The linked `boyko_demo`'s
  section sizes and defined-symbol multiset must equal the parent's, under the commit's rename
  list.
  - Leg (7) is what proves modding R1 (`MODDING-DESIGN-SPACE.md:1818-1823`) for the Stage-1 items.
  - Legs (1)–(3) cannot see a seam body that survives fat LTO without an attribute.
  - Leg (6) cannot see it either, because both of its arms contain the kernel
    (`ALLOCATOR-DESIGN-SPACE.md:3825`).
- **Rev 5.1 adds the checks that prove S-1** (to be written into 03 §6, RM-3):
  - **Leg (5), S-1 shape.** A `syn` check over the census crates. Every item that carries a
    `MOD-SEAM` marker and adds code is generic over a parameter bounded by `ModSeam`; MS-08's
    doc-only markers on engine items are exempt. No census crate implements `ModSeam` outside
    `#[cfg(test)]`. `ModSeam` is not re-exported at any crate root.
  - **Leg (7b), rlib object census.** On every seam commit: `llvm-nm --defined-only --demangle`
    over the **object members** of every census crate's rlib, as built for `boyko_demo` under
    `[profile.seam-census]`. Pass: no defined symbol whose demangled path names a `MOD-SEAM` item.
    - Metadata members (`lib.rmeta`) are not objects and are skipped.
    - Anti-vacuity: the check reports how many object members it read, and zero read is red.
  - **Red control (xi).** One MS-03 reader with its `ModSeam` parameter removed → leg (7b) red.
    Leg (7) may stay green, because fat LTO can drop the dead body. This proves that (7b) sees what
    (7) cannot.
  - **Green control (xii).** The committed generic readers, with no implementor anywhere in the
    build → leg (7b) green. This proves the census does not fire on names carried only in
    metadata.
- It is green only if every red control that 03 §6 lists for the commit, (i)–(viii) and (xi), is
  red in its own branch, and green controls (ix), (x) and (xii) stay green.

## 7. Modding sequence against the kernel plan

| Stage | When | Content |
|---|---|---|
| 0 | any time | owner values (00 §7 Q-1, Q-2; RM-4) |
| 1 | inside Phase D | **Only the common set.** D-S1(ii) lands `ModSeam` and MS-03's four readers (01 KC-19b) under strict UG-15, including legs (7) and (7b). Its parent is D-S1(ii)'s cut commit, which contains D-S1(i), the bug-fix commit (P39, P40's tag path, 32.2, NW4). MS-08's markers land with A6 (doc lines, no code). RM-2's fix is an engine rung, placed by the architect. Nothing else is built for modding in Phase D |
| 2 | quiet windows; structural checks any time | MQ-09, plus the timing legs of MD:M-F1 (c) and MD:M-C0b. Structural: MD:M-F1 (a), (b) and the size legs of (c) and M-C0b; MD:M-C4, MD:M-C5, MD:M-K3 |
| 3 | after Phase-D exit | **Only the chosen option's items**, behind the modding crates, gated by UG-15: A′ → MS-14; C or D → MS-15; A → MS-01, MS-02b, MS-09..MS-13 and RM-1's item. Every option → MS-02a's SDK half and MS-04..MS-07. Waiting for D-exit means Stage 3 is not fighting a moving kernel |
| 4 | after Stage 3 | one real mod end to end (modding §9) |

## 8. Owner values questions (Stage 0)

These are duplicated in 00 §7 Q-1 and Q-2.
- Trust model.
- Patch-release survival.
- Who builds a mod.
- Prototype-only scope or mod-defined component types.
- Unload / hot reload as a product requirement.

The answers decide whether options E or B become mandatory, and whether MS-01, MS-02b and MS-09 to
MS-13 are built at all. Under A′, C and D, the prototype-only answer removes nothing from the
kernel, because those options need no descriptor (§3.1).
