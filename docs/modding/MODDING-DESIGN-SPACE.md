# Modding — the design space

**Status: closed at rev 6 (critique pass 6); open remarks are listed in the revision log and are carried into docs/unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md**

**Revision:** design space, **rev 6** (rev 1 + five architecture-critique passes; pass 1's two blockers,
pass 2's three, pass 3's one, pass 4's two and pass 5's two resolved — the last two by orchestrator
ruling rather than by redesign; see the Revision log at the end). Nothing here is ratified. The evidence is in
[MODDING-RESEARCH.md](MODDING-RESEARCH.md); every external claim is cited there and the short form is
repeated here only where a number decides an option. Repository facts are read at
`D:/wt/joltab` commit **`d552be05be4b4f063b6cb39ddfd63eb4688f83fd`**; paths are relative to the
repository root.

**Benchmarks cannot be run in this session; feasibility probes were.** Every quantitative claim below
is either (a) a measured number with its source, or (b) explicitly marked as a prediction with the
measurement that would confirm or refute it. §10 is the queue. Rev 2 adds four **measured**
toolchain results (M-5…M-8, MODDING-RESEARCH §13) that were taken to settle option C's feasibility
rather than to time anything: they are link-and-run experiments, each under a second, on the owner's
machine with `rustc 1.98.1 (48a229cea 2026-09-01)`, `x86_64-pc-windows-gnu`.

**Rev 3 adds no new benchmark and one new class of check — the contents of the installed toolchain,
read off disk (2026-09-16).** In
`C:\Users\flint\.rustup\toolchains\stable-x86_64-pc-windows-gnu` there is **no LLVM linker
plugin**: a glob for `*gold*` over the whole toolchain returns nothing outside rustdoc HTML. The only
plugin-capable linker it ships is `rust-lld.exe`, **211 187 378 B**; the four
`lib/rustlib/x86_64-pc-windows-gnu/bin/gcc-ld/{ld.lld, ld64.lld, lld-link, wasm-ld}.exe` are
857 088 B each and **byte-identical to one another** (same MD5), and their strings include
`rust-lld`, `-flavor` and `error running rust-lld child process` — they are shims that drive
`rust-lld`, not linkers. `bin/self-contained/` holds exactly `x86_64-w64-mingw32-gcc.exe`
(2 840 064 B), `ld.exe` (1 910 784 B), `dlltool.exe` (1 328 640 B), `libwinpthread-1.dll` and
`GCC-WARNING.txt`. That single reading decides §3's route question, which is why it sits here with
the measurements rather than inside the argument; it is recorded as **M-9** in
[MODDING-RESEARCH §13.2](MODDING-RESEARCH.md).

**Rev 4 adds two toy-scale measurements, M-10 and M-11 (research §13.3), each under two seconds,
taken because pass 3 marked a mechanism PLAUSIBLE that a probe could settle.** M-10 rebuilds the
derive's exact shape — an `#[inline]` fn holding a function-local `static ID: OnceLock` — in a
two-crate rig and follows the symbol through the rlib, through `-C lto=fat`, into a mod object
compiled against the rlib, and onto the two link lines option C's route 3b admits. **Both fail loudly,
and no flag on stable or nightly changes the binding** — so 3b, the form C was ranked first as, is
red at toy scale (§3), and the ranking is suspended on the workspace-scale M-C4 (§8.1). M-11 runs
the M-2 dylib rig on the installed nightly with `-Z dylib-lto`, the flag M-3's own error message
names: it builds, links, runs and still shares its statics, so A′'s LTO forfeit is stable's price,
not Rust's (§1.5). Neither is a benchmark; both are receipts of the M-5/M-6 kind.

**Rev 5 adds no measurement and two corrections found by tracing, both of which the earlier text
had the facts for and did not apply.** §0.3 leg (d) was derived from the two dispensers that
motivated it; re-derived from the *consumers* of each counter it has a sixth mint path
(`Assets::<T>::with_reserved` → `register_asset_layout` → `try_register_dynamic`, which returns
`None` at the cap instead of asserting), and its abort decision no longer depends on panic text.
§7.3's loader bound compared a quota against a counter read before the engine's own ids have
minted; the bound is re-opened with three traced directions and the physics census named as the
constraint it spends. Two measurements are queued (M-C5, M-K3), none taken; the rustc page cited
for route 2 was re-read and is silent on the standard library.

**Rev 6 adds no measurement and closes the document on two orchestrator rulings (R1, R2) that
resolve pass 5's two blockers by decision.** R1 re-specifies the P1 gate: M-P1 no longer diffs the
modding leg against the non-modding leg of one tree — a comparison every unconditional kernel delta
passes by construction — but compares the post-change **non-modding** build against **pins captured
on the pre-change tree** (`d552be05`, before any modding seam exists): byte size and disassembly of
named functions, three `size_of`s, the executable's export count and its `.text` size (§10 M-P1).
R2 puts mod-system registration on partition (ii): `App::add_systems` / `add_systems_in` /
`add_startup_system`, `ScheduleBuilder::add_system` and every generic that boxes a system join the
façade's forbidden set, and a mod registers a system only through a host-side seam that takes a
function pointer plus an access descriptor and allocates the `SystemBox` in the host image
(§7.9(c), §7.10 item 6). Every pass-5 remark below the blockers is resolved where the tree settles
it and carried as open where it does not (§12, rev 6).

---

## 0. What the owner asked for, restated as testable properties

> Modding that is **free in performance terms** — mod binaries are simply added and run like native
> engine code — and **optional**: a game that does not use modding pays **nothing**.

That is three separate properties, and they are tested separately.

**P1 — modding OFF costs nothing.** A build with modding not compiled in emits *the same machine
code* as an engine that never had modding support. **The two legs are two TREES, not two
configurations (rev 6, R1):** the *post-change* tree built without modding, against *pins captured
on the pre-change tree* — `d552be05`, the commit this document reads and the last one with no
modding seam in it. Test: capture the pins there (byte size and disassembly of the named functions,
`size_of` of `EcsMaster` / `ComponentPool` / `Scope`, the executable's exported-symbol count and its
`.text` size — the list is §10 M-P1), rebuild the non-modding configuration after each kernel delta,
and require every pin identical (this repository's own "0%-gate / byte-identical asm" methodology,
`Cargo.toml:84-88`), plus `cargo tree -e features` to prove the crate is absent from the resolved
closure. A diff between the modding and non-modding legs of the *same* tree is **not** this gate:
a delta that lands in `boyko_ecs` unconditionally is present in both legs and the diff is green
whether or not the delta exists (pass 5, C1).
**⚠ A whole-image symbol census cannot serve as this gate** — measured at
`D:/wt/reflect/docs/REFLECTION-ANALYSIS.md:1467-1494`: at default release the symbol reads present in
*both* legs, `--gc-sections` has "no effect whatsoever", and only `lto="fat", codegen-units=1` makes
it absent. Generic functions are the decidable case; plain ones are not.

**P2 — modding ON, no mod loaded, costs ~nothing.** Some startup work is unavoidable (a directory
scan). The test is that no *per-frame* and no *per-row* path differs, and that startup differs by a
measurable, budgeted amount. Bevy's comparable auto-registration measured **<25 ms native debug for
1614 types** ([bevy#15030](https://github.com/bevyengine/bevy/pull/15030)) — that is the order of
magnitude to budget, not to assume.

**P3 — a mod loaded, engine types untouched, costs nothing on engine paths and near-native on mod
paths.** "Near-native" has a hard denominator here: query iteration is **3.4–4.0 ns/row**
(`docs/aether-v2/DECISIONS.md:372`, `docs/archive/ENABLE-TAG-RESULTS.md:95-101`).

### 0.1 The one structural fact that makes any of this possible

Generics and `#[inline]` bodies — and, since
[rust#116505](https://github.com/rust-lang/rust/pull/116505), small call-free functions
automatically — are **instantiated in the calling crate**, even across a dylib boundary. Measured in
the research session (M-4, MODDING-RESEARCH §13): an `#[inline]` engine fn costs 0.128–0.162 ns
statically and 0.138–0.145 ns across a dylib; an `#[inline(never)]` one costs 1.72–2.22 ns vs
2.43–2.50 ns.

**Therefore: a mod's query inner loops are native regardless of the boundary, and only opaque entry
points (spawn, command apply, archetype migration, registry mints) pay indirection.** The design
question is not "how fast is the boundary" but **"how often is it crossed"**. Once per system run is
free at the 3.4 ns/row scale; once per entity is catastrophic.

### 0.2 The three hard constraints that bind every option

1. **LTO and `dylib` are mutually exclusive on stable** — measured twice in the research session
   (M-3): `lto cannot be used for 'dylib' crate type without -Zdylib-lto`, and
   `cannot prefer dynamic linking when performing LTO`. The price of giving it up is this
   repository's own matrix (`Cargo.toml:89-112`, measured 2026-09-04): geomean **0.793**,
   `query_ref_iter/10k` **0.375** (2.67×), binary **8.76 → 3.46 MB**. **On the owner's THROUGHPUT
   priority this is the single largest number in the whole document.**
2. **A `cdylib` gets its own copy of every process-global static the engine's code reads** —
   measured (M-1): two `NEXT_ID` counters at different addresses, both starting at 0. The failure
   mode is a **silently wrong id**, not a link error, and the `WorldId` gate at
   `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:286` cannot catch it.
   ⚠ **This is a class, not a list of leaks, and rev 1 under-stated it by an order of severity.**
   The worst instance is not a wrong `ComponentId` but *type confusion on the query path* — the path
   §0.1 declares free. §0.3 enumerates the class and states where its boundary is decidable.
3. **Nothing in this kernel can ever be unregistered.** Every registry slot is write-once
   `OnceLock`/atomic: `LAYOUTS` (`…/component_registry/mod.rs:208`), `HOOKS` (`:226`), `CLONE`
   (`clone.rs:100`), `MAP_ENTITIES` (`clone.rs:109`), `SERIALIZE` (`serialize.rs:164`),
   `FLAGS_DIRECT` (`flags.rs:91`), `STORAGE_KIND` (`:375`), `RESIDENCY_CLASS` (`:503`),
   plus leaked `&'static str` names in `TAG_NAMES` (`tags.rs:155`). **Loading is expressible;
   unloading is not.** `docs/editor/EDITOR-BOUNDARY.md:410` already refuses code hot reload for
   exactly this reason — that ruling covers *unloading*, and load-only is the live opening.

### 0.3 The duplicated-statics hazard is a CLASS, and its boundary is decidable

In any option where the mod links the engine's rlibs into its own image (option A, and option B
whenever it links them to get generics), the mod's image contains its own machine code for every
engine function it references, and therefore **its own copy of every process-global that code
reads**. Whether a given engine function was inlined is irrelevant: inlined or not, the code running
inside the mod resolves to the mod's copy of the static. **The only engine state a mod can share is
state reached through a pointer the host handed over.** A contract of the form "the mod receives ids
and never mints" is therefore not enforceable by convention — `EcsMaster` is `pub`, and
`world.query::<D, F>()` is one call away in mod source.

**Six instances, traced in this tree. Instance 1 ranks first by severity, instance 2 by
frequency; instance 6 (rev 5) is the one an asset-using mod reaches first.**

1. **Query type ids — silent type confusion, in release, on the path §0.1 declares free.**
   `EcsMaster::query::<D, F>()` is generic (`…/ecs_master/ecs_master.rs:825`) and is instantiated in
   the mod. It calls `<(D, F) as QueryTypeKey>::query_type_id()` — `#[inline]` and generic
   (`…/iters/query/query_type_registry.rs:235-241`) — which resolves against
   `static REGISTRY: TypeIntern<…>` (`:206`) and mints from `static QUERY_NEXT_ID` (`:98`): **the
   mod's copies, empty and starting at 0**. The id then indexes the **host's** per-world
   `QueryStateCache` through `slots.get_unchecked(id.0)` (`ecs_master.rs:385`), whose payload is a
   type-erased `NonNull<()>` that the caller `cast()`s to `UnsafeCell<QueryDataState<D, F>>`
   (`:855`) and writes through (`state_mut.update_with_world(…)`, `:880`).
   Concretely: the host registers `Query<&Transform, ()>` → host id 0 → host slot 0 holds
   `QueryDataState<&Transform, ()>`; the mod's first query mints **its own** id 0, `slot.get()`
   returns `Some`, and the mod casts and mutates the host's state as `QueryDataState<&ModComp, ()>`.
   The `debug_assert!`s at `:381` and `:843` check only the bound, and the `WorldId` gate cannot see
   it. **A load-time handshake cannot fix this**, because the id is minted inside monomorphised code
   keyed on `TypeId::of::<D>()` for types the host has never seen.
   The same shape holds for `resource_id_for<T: Resource>()`
   (`…/resources/resource_type_registry.rs:94` — `#[inline]`, generic, and its own doc notes that
   `State<S>` / `Assets<T>` / `ActionState<A>` all mint through it), for the bundle intern
   (`BUNDLE_NEXT_ID`, `…/bundle/bundle_type_registry.rs:93`), and for the non-send resource registry.

2. **Per-type component ids, and the derive's install arms — the most frequently reached instance,
   and the one the façade sketch did not cover.** Before a mod's query reaches `query_type_id()` it
   resolves each component. `Component::component_id()` is `#[inline]` and holds its
   `static ID: OnceLock<ComponentId>` **inside the fn body**
   (`crates/boyko_macros/src/component.rs:371-372`), mints through `register_new::<Self>()`
   (`:375`, landing at `component_registry/mod.rs:920`), and then runs the derive's
   `#storage_install` / `#require_install` / `#clone_install` / `#relationship_install` /
   `#residency_install` / `#serialize_install` arms (`component.rs:387-392`). For an **engine** type such as `Transform`
   that impl lives in the rlib the mod links, so the mod gets **its own `ID` cell** and installs
   **its own** `STORAGE_KIND` / `CLONE` / `SERIALIZE` rows. The id is then the mod's own mint order —
   0, 1, 2 … — and it indexes the host world's `[Column; 512]` at the wrong slot: M-1's original
   finding, restated at its real frequency. **Every engine component a mod names reaches this**,
   which is why it outranks instance 1 on frequency while instance 1 outranks it on severity (a wrong
   column is at least type-tagged in debug; a confused `QueryDataState` is not). Its consequence for
   the façade is concrete and larger than §0.3 implied: `component_id()` is called from *inside* the
   engine's own `QueryData` impls, so `boyko_mod_api` cannot re-export those impls at all — it needs
   a typed front-end whose per-type ids come from the host **for engine component types too**, not
   only for mod-defined ones.

3. **Storage-kind and residency divergence at archetype construction.** `archetype_master.rs:193,
   510, 729` decide signature membership from `component_registry::storage_kind(cid)`, and `:535`
   reads `residency_class(cid)`. Both are `[AtomicU8; MAX_COMPONENTS]` process-globals
   (`component_registry/mod.rs:375`, `:503`) whose unset slots **default to `Table` / `Cpu`**
   (`:399-410`, `:579`). In the mod's copy every engine id is unset. The source's own comment at
   `mod.rs:401-405` names the consequence for a dense id read as `Table`: it "silently re-enters the
   archetype signature, and fragments archetypes with NO compile error" — which directly undermines
   §7.4, the section that *requires* dense storage for mod components on engine entities.

4. **Allocator — rev 1 answered the wrong mechanism.** §1's allocator row cited
   `#[global_allocator]` being ignored under `-C prefer-dynamic` and dismissed it. But the adopted
   allocator decision is **per-type replacement by lifetime class**, with `#[global_allocator]`
   explicitly rejected *as an allocator* and kept only as the deny gate
   (`docs/memory/ALLOCATOR-DESIGN-SPACE.md:63`). Per-type allocator state lives in engine types and
   engine statics, which the mod duplicates. "The rule is honoured on both sides" is therefore **not
   sufficient**: it yields the same allocator *code* over two different allocator *states*, and a
   cross-boundary free is still corruption.

5. **TLS is three slots, not one, and linkage decides which copy is read.** Beside `ACTIVE_POOL`
   (`boyko_threadpool/src/tls.rs:169`) there are `LANE_DEPOSIT` (`:196`) and `IN_SYSTEM_RUN`
   (`:205`). A mod's copies read `DETACHED` / `0`: the mod's spawns miss `worker_lane_for` and fall
   through to the global injector (locality lost, no test red), and the ALLOC1 allocation-discipline
   `debug_assert!`s that the ECS makes against `is_in_system_run` read "not in a system run" and
   **pass vacuously**.

6. **The asset-layout intern and its mint — a `pub`, generic, constructor-side path the rev-4
   façade list did not name (pass 4, C1).** `Assets::<T>::with_reserved(cap)` is `pub`, `#[inline]`
   and generic (`crates/boyko_ecs/src/ecs/core/asset/assets.rs:231-232`); `Assets::<T>::default()`
   is `with_reserved(0)` (`:1052`). Its first line is `T::register_layout()`, which every
   `impl_asset_pod_backing!` impl (a `#[macro_export]`, `$crate`-qualified macro documented as
   usable "from ANY crate", `asset/backing.rs:142-160`) and the hand-written `MeshGpu` impl
   (`boyko_render/src/mesh.rs:225`) implement as `register_asset_layout::<T>(drop_fn)` — `pub` at
   `backing.rs:115`, re-exported at `asset/mod.rs:60`. That function locks its own
   `static ASSET_LAYOUTS: OnceLock<Mutex<HashMap<TypeId, ComponentId>>>` (`backing.rs:87`) and, on a
   miss, mints through `component_registry::try_register_dynamic` (`:131`) — the **same `NEXT_ID`**
   as `register_new` (`mod.rs:968` against `:214`). Instantiated in the mod, the whole chain runs on
   the mod's copies: its own empty intern, its own counter. Without leg (d) the mod mints id 0 from
   its own `NEXT_ID`, builds a `ComponentPool` whose layout comes from its own `LAYOUTS[0]`
   (internally consistent), and carries that id out as the store's `AssetKind` routing key
   (`assets.rs:155`, `:214`, `:246`; copied into every `RetireTicket`, `:166`) and, when the store
   is inserted as a resource, through the resource intern of instance 1. Every mod that constructs
   an asset store — a mesh table, a texture table, one mod-defined asset type — reaches this before
   it reaches anything else in §0.3. The façade's forbidden set therefore gains
   `Assets::<T>::with_reserved` / `Assets::<T>::default()`, `AssetBacking::register_layout`,
   `register_asset_layout` and `impl_asset_pod_backing!`, each with its own `compile_fail` fixture
   under leg (b); and leg (d)'s coverage, re-derived below, has to include this path's exhaustion
   behaviour, which is **not** an assert.

**The boundary is decidable, and the safe side is not empty.** `iters/query_state.rs` touches no
process-global registry in steady state — it walks columns whose pointers came from the host — so
§0.1's core claim (a mod's *iteration* is native) survives intact. The hazard is everything
*around* the loop. The partition to specify in Stage 1 is:

- **(i) Free to run in the mod**: row iteration and per-row work over column pointers the host
  supplied. No registry read, no mint, no TLS question.
- **(ii) Must cross through host-supplied function pointers**: every mint (component, resource,
  query, bundle, event, trigger — and the asset-layout mint of instance 6, which is a component mint
  wearing a constructor), every archetype construction, every registry classification read, every
  `par_iter` entry, every allocation whose memory can outlive the mod's call — **and the derivation
  of a mod system's `Access`** (rev 5; pass 4, question 1). The scheduler's parallel-safety contract
  is the 192 B `Access` (`system/access.rs:61`), filled in by each param's `init_access`
  (`system_param.rs:159-164`: implementations "MUST declare every read and write") from the ids the
  param resolved — `add_component_read` / `add_component_write` at `iters/query/data/read.rs:105`
  and `data/mut_.rs:234`. A `ModQuery<D, F>` that does not implement the kernel's `SystemParam` has
  no `init_access`, and an engine system and a mod system both writing `Transform` then run
  concurrently on 8 workers. So the façade's query type implements `SystemParam` and derives its
  access from **host-resolved** ids — a façade obligation, carried in §7.9(a). **And the
  registration of a mod system into the host's schedule (rev 6; pass 5, C2; ruling R2):**
  `ScheduleBuilder::add_system<F, M>` is `pub` and generic, its `Box::new(sys)`
  (`schedule_builder.rs:183`) is monomorphised in the calling crate — the mod's image — and the
  `Box<dyn System>` it produces is stored in the host's `SystemBox` (`system_box.rs:83`) and freed
  by the host's compiler-generated drop glue: mod allocates, host frees, the exact shape §1's
  allocator rule forbids. Registration therefore crosses through a host-supplied function pointer,
  and the host allocates — §7.9(c).

**How (ii) is enforced — the shape, not yet a specification.** A mod-facing façade crate
(`boyko_mod_api`) that does **not** re-export `EcsMaster`, `world.query`, `world.spawn`, the
`Assets<T>` constructors (`with_reserved`, `Default`), `AssetBacking`, `register_asset_layout`,
`impl_asset_pod_backing!`, `register_layout`, or any generic that reaches a mint — **nor (rev 6,
R2) `App`, `Plugin` / `App::add_plugin`, `App::add_systems` / `add_systems_in` /
`add_systems_cfg` / `add_systems_cfg_in` / `add_startup_system`, `ScheduleBuilder` (its
`add_system`, `insert_state` and `SystemConfig::run_if` are the production sites that box a
system or a closure: `schedule_builder.rs:183`, `:263`, `:837`; `app.rs:514`), or any generic that
boxes a system** — each with its own `compile_fail` fixture under leg (b). In its place, a façade `ModQuery<D, F>` whose id is obtained by one
`extern "C"` host call per system run — §0.1's "once per system run is free" budget — keyed on a
**stable name tuple** rather than on `TypeId`, and whose `QueryDataState<D, F>` is allocated and
owned *by the mod* and handed to the host as the type-erased `(NonNull<()>, drop_fn)` pair the cache
slot already stores (`QueryCacheSlot`, populated by `query_cold_init` with a monomorphised drop fn).
That representation exists today and is exactly the shape needed; what does not exist is the
by-stable-name mint entry point. **This is the kernel change option A actually requires, and rev 1
did not list it.**

**And how the enforcement is proven — two cooperative legs, one that cannot discriminate, and one
that can.** None of them is a whole-image symbol census (measured undecidable,
`REFLECTION-ANALYSIS.md:1467-1494`).

- **(a) `cargo tree -e features`** over a mod crate, asserting `boyko_ecs` is not a **direct**
  dependency — only `boyko_mod_api`. **Cooperative, and weak.** Under A the façade must link the
  engine rlibs to get generics at all, so `boyko_ecs` is a *transitive* dependency of every mod by
  construction; the assertion constrains only the manifest the mod author writes, in a build the mod
  author runs. One added line defeats it.
- **(b) `trybuild` `compile_fail` fixtures** per forbidden entry point, so "the mod called
  `world.query`" is a compile error in the SDK rather than a runtime corruption. **Cooperative.**
  They prove the *façade* does not re-export `world.query`; they cannot constrain a third-party crate
  that depends on `boyko_ecs` directly.
- **(c) a load-time address cross-check** — the host passes `&raw const` of its own `NEXT_ID` /
  `QUERY_NEXT_ID`, the mod compares against its own, a mismatch refuses the load.
  **⚠ This leg does not discriminate, and rev 2 was wrong to count it.** What it observes is that the
  mod's image *contains* a second copy — which under A is true of every mod, compliant or not,
  because a dependency rlib's items are carried into the image whether or not anything can reach them
  (`REFLECTION-ANALYSIS.md:1477-1494`; `--gc-sections` had "no effect whatsoever" at default
  release — the same measurement this document cites to forbid a symbol census). A check that is true
  of every mod refuses every mod; narrowed to fire only when the mod *names* the symbol, it becomes
  the census. (Strictly: the cited measurement is of a plain *function*,
  `core::ptr::drop_glue::<…>`. Whether an **unreached** `static` in a linked rlib is carried the same
  way is measured nowhere — **M-A4a**. Neither answer rescues the leg: if the copy is present the
  check fires always, and if it is absent, the façade's own probe reference materialises it anyway.)
- **(d) poisoning the mod image's mint counters — non-cooperative, discriminating, and preventive
  rather than detective.** The loader, at load time and before the mod's init runs, writes each cap
  (`MAX_COMPONENTS`, `MAX_QUERY_TYPES`, `MAX_BUNDLE_TYPES`, `RESOURCE_SLOT_COUNT`, `MAX_EVENTS`)
  into the **mod's own** copies of the five counters, at addresses it gets from an SDK probe entry
  point the handshake requires anyway. A compliant mod never mints in its own image and never
  notices. A mod that reaches a mint gets **no id**, and therefore indexes nothing the host owns —
  that half is a property of the *counters*, and it holds for every consumer. **What it does NOT
  guarantee is that the failure is loud, and rev 3–4 wrote as if it did (pass 4, C1).** Rev 3
  derived the leg from the two dispensers that motivated it — `register_new::<T>()`'s release
  `assert!` (`component_registry/mod.rs:921-926`) and `query_type_registry::register_new()`'s
  saturate-then-panic (`query_type_registry.rs:132-145`) — and rev 4 generalised "every dispenser
  asserts before it returns" to all five counters. **Coverage has to be derived from the consumers
  of each counter, not from the dispensers that happened to be looked at**, and `NEXT_ID` alone has
  three production consumers with **four** exhaustion behaviours:

  | consumer of `NEXT_ID` | reached from | at the poisoned cap |
  |---|---|---|
  | `register_new::<T>()` (`mod.rs:920-945`) | the derive's `component_id()` — every engine type a mod names | release `assert!`, `"ComponentRegistry exhausted"` (`:922-927`) — **loud** |
  | `try_register_dynamic(layout)` (`:967-990`) → `try_register_tag_by_name` (`tags.rs:182-197`; enable tags at `:134-139` go through it) | `EcsMaster::register_tag` / `register_enable_tag` (`tag_api.rs:65`, `enable_tag_api.rs:61`, both `pub`) | the dispenser returns **`None`** (`:970-972`; its own doc: it *"does NOT inherit `register_new`'s release exhaustion assert"*, `:951-953`); the two panicking wrappers convert it — `register_tag_exhausted_panic` (`tag_api.rs:245`) / `register_enable_tag_exhausted_panic` (`enable_tag_api.rs:335`) — **loud, two more texts** |
  | the same, via `try_register_tag` / `try_register_enable_tag` (`tag_api.rs:47`, `enable_tag_api.rs:73`, both `pub`) | a mod that uses the fallible form | **`None` returned to the mod — no panic at all**; the mod's own code decides what happens next |
  | `try_register_dynamic` → `register_asset_layout::<T>` (`asset/backing.rs:131`) | instance 6 — every asset-store construction | `.expect("invariant: asset ComponentId space exhausted (MAX_COMPONENTS)")` (`:132`) — loud, a **fourth text** |

  The other four counters are single-consumer today (`QUERY_NEXT_ID`: `query_type_registry.rs:132`;
  `BUNDLE_NEXT_ID`: `bundle_type_registry.rs:120`; `NEXT_RESOURCE_ID`: `resource_registry.rs:175`;
  `NEXT_EVENT_ID`: `event_registry.rs:110` — each `fetch_add` then assert, or saturate then panic;
  grep at `d552be05`), which is a fact about today's tree and not a property: the next fallible
  dispenser added to any of them re-opens this table. So the leg's guarantee is stated precisely:
  **no id is ever minted in the mod image** (all consumers), and **the attempt is observable** —
  through a panic for four of the five behaviours, through a returned `None` for the fifth. What
  turns "observable" into "the process stops" is limit (v). This is M-7's pattern — turn a silent
  failure into a loud one — applied to statics instead of to a linker section, and it gives
  **M-A4 a red control that can actually fail**, provided M-A4's red controls include an asset-store
  construction and a fallible tag mint, not only `world.query`.
  **Limits, stated rather than discovered:**
  **(i)** it covers the **mints** (instances 1, 2 and 6, plus the resource, bundle and event
  dispensers), not the **reads** of instance 3 (`STORAGE_KIND` / `RESIDENCY_CLASS` defaults consulted
  during archetype construction inside the mod), nor instance 4 (allocator state) or instance 5
  (TLS) — those stay with partition (ii), the allocator rule and the TLS install. **Nor does it cover
  ids that are never minted at all (rev 4):** `register_layout::<T>(id)`
  (`component_registry/mod.rs:1031-1056`) pins a constant id into `LAYOUTS[id]` without touching
  `NEXT_ID` — the physics scratch band's shape (`boyko_physics/src/scratch_ids.rs:653-659`,
  `:713-718`; ids from `MAX_COMPONENTS - 1` down to `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128` at
  `:685`). A mod image that calls it writes its **own** `LAYOUTS` row and the poison never fires; the
  id agrees with the host's only because it is a shared constant, not because anything checked. The
  façade must not re-export `register_layout` (one more `compile_fail` fixture under leg (b)), and
  §7.3 must never move that band.
  **(ii)** the resulting panic reads *"ComponentRegistry exhausted"* / *"enable `big_query_table`"* —
  a **lying diagnostic**. The loader must own the translation (it knows it poisoned), and M-A4 must
  check the operator-visible text, not merely that the process died.
  **(iii)** it is proof against a **mistake**, not against **malice**: the probe and the poison both
  live in an artifact the mod author compiles, and an author who wants to can restore the counter in
  three instructions.
  **(iv)** it inherits failure mode 1's ordering question, and rev 4 states the write's shape (pass
  3's question 3). The counters are `AtomicUsize` (`mod.rs:214`, `query_type_registry.rs:98`,
  `bundle_type_registry.rs:93`, `resource_registry.rs:127`, `event_registry.rs:93`), so the poison is
  an **atomic `store` through the `&'static AtomicUsize` the probe returns** — never a raw byte
  write — because Windows lets a mod's `DllMain` spawn a thread that mints concurrently, and a torn
  or non-atomic write would race the dispenser's `fetch_add`. The earliest the host can issue it is
  *after* `LoadLibrary` has run the image's initialisers, so the loader **reads before it writes**: a
  fresh image's five counters must all read `0` (every dispenser starts at `AtomicUsize::new(0)`);
  any non-zero reading is a constructor-side mint that has already happened, and the load is refused
  loudly. (Instance 6's `ASSET_LAYOUTS` is a `OnceLock<Mutex<HashMap>>`, not a counter, and needs no
  reading of its own — rev 5, pass 4's question 3: it is *downstream* of `NEXT_ID`, populated only
  with an id `try_register_dynamic` has already returned, so a constructor-side asset mint has moved
  `NEXT_ID` off zero and is caught there.) After the write, a later reading that differs from the cap is a mint that was *attempted*
  after the poison — for the three non-saturating dispensers (`register_new`, `NEXT_RESOURCE_ID`,
  `NEXT_EVENT_ID`: `fetch_add` then assert) it reads `cap + 1`; the two saturating ones
  (`QUERY_NEXT_ID`, `BUNDLE_NEXT_ID`) store the cap back before panicking
  (`query_type_registry.rs:135`, `bundle_type_registry.rs:128`) and so **cannot be told apart by a
  counter read at all** — which is why the panic itself must stay loud, limit (v). (Rev 6 notes,
  from the same code, that a poison value of `cap + 1` rather than `cap` changes this: every
  dispenser refuses at `>= cap` alike, but the saturating pair stores the cap *back*, so a read-back
  that differs from the poison value sees their refusal too; only `try_register_dynamic`'s CAS-loop
  `None` (`mod.rs:968-972`) leaves the counter untouched. §7.10 item 2b carries that as the
  boundary-side fallback if the latch cannot stay in the kernel.)
  **(v) the panic must not be catchable at the mod boundary — and §1's own panic contract would
  catch it (pass 3, W2).** The mint panic fires inside `OnceLock::get_or_init`'s closure
  (`component.rs:374-394`), which does not poison the cell (the tree's own comments:
  `bundle_type_registry.rs:106-109`, `query_type_registry.rs:51-53`), and propagates out through the
  mod's system body into the `catch_unwind` that §1's Panic-handling row requires at every mod entry
  point. A compliant mod would therefore convert leg (d)'s terminal panic into a returned status, per
  call, every frame — the mod silently does nothing from frame N onward, the exact shape this leg
  exists to abolish. **Resolution, rewritten in rev 5 (pass 4, C1): the `catch_unwind` at the
  boundary is SDK code, not author code, and it decides on a STRUCTURAL signal — never on the
  panic's text.** `boyko_mod_api` emits the `extern "C"` trampoline around each mod system (a macro;
  the author never writes the wrapper). Rev 4 had that trampoline downcast the payload and match
  five exhaustion strings. That was wrong twice over. The string set was incomplete: the asset
  path's `.expect` text (`backing.rs:132`), the two tag wrappers' texts (`tag_api.rs:245`,
  `enable_tag_api.rs:335`) and the collision texts of `register_new` (`mod.rs:938-944`),
  `dynamic_slot_occupied_panic` (`:996-1008`) and `register_layout` (`:1046-1051`) were all absent,
  so a mod constructing an asset store would have had its poison-induced panic swallowed into a
  status — silently disabled from frame N onward, at 60 fps, in release, the exact shape this limit
  exists to abolish. And it was keyed on diagnostics, which are prose, not API: any later edit to
  those strings would have degraded the abort to a status with no gate going red. **The requirement
  is therefore stated as a requirement, and the mechanism is the architect's:** the SDK boundary
  must be able to learn *that a dispenser in this image refused a mint*, from a signal the dispenser
  itself raises, independent of message text and of whether the refusal was a panic or a returned
  `None`. The shape rev 5 proposes, for the architect to ratify or replace: one per-image latch in
  the kernel — a `static` `AtomicBool` beside the five counters — that every refusal path stores
  `true` into before it asserts, panics or returns `None`: `register_new`'s assert and collision
  arms (`mod.rs:922`, `:938`), `try_register_dynamic`'s `None` and `dynamic_slot_occupied_panic`
  (`:970`, `:996`), `register_layout`'s collision arm (`:1046`), and the four single-consumer
  dispensers (`query_type_registry.rs:133-145`, `bundle_type_registry.rs:121-136`,
  `resource_registry.rs:176-180`, `event_registry.rs:112-116`) — nine cold sites, each one relaxed
  store on a path that is `#[cold]` or an assert's failing arm, unreachable from any row loop, and
  already about to unwind. The trampoline reads the latch **on the panic path** (free) and aborts
  with the loader-owned translation if it is set — which also closes limit (ii)'s lying diagnostic
  at the only place that knows a poison was written — and returns §1's status otherwise. The one
  refusal that does not panic (`try_register_tag` / `try_register_enable_tag` returning `None`) is
  closed by a second read **once per mod-system run**, which §0.1's own budget permits — "once per
  system run is free at the 3.4 ns/row scale": one relaxed load of a per-image static, beside the
  `Box<dyn System>` vtable call the schedule already makes (`schedule_builder.rs:183`). Rev 4
  rejected a per-run *counter* re-read because it could not see the two saturating dispensers; a
  latch the dispensers raise themselves has no such blind spot, so that ground is gone and only the
  per-run cost remains, inside budget. Price to **every** build: one `AtomicBool` and a store on
  nine refusal *sites* — **which is not nine emitted stores (rev 6; pass 5, C1)**: `register_new<T>`
  (`mod.rs:920`) and `register_layout<T>` (`:1031`) are generic, so their refusal arms are
  instantiated once per component type per instantiating crate, from inside the derive's
  `#[inline]` `component_id()` body (`component.rs:370-375`); against ~126 component types in use
  the emitted count is of order 130–150, and whether that moves any pin is what M-P1 measures
  rather than what this paragraph asserts. §7.10 admits the delta **conditionally** and rows it
  (item 2b): it stays in the kernel only if every M-P1 pin — `register_new::<T>` and
  `T::component_id()` for a representative engine `T` among them — is identical in the post-change
  non-modding build; if a pin moves, the delta is charged to P1 and the signal moves behind the
  modding crate boundary (item 2b names the boundary-side form and its one coverage gap). Whether to
  pay the per-run read, and whether the read site is the trampoline or a panic hook installed in the
  mod image's own `std` copy (which would abort *before* unwinding starts), are the architect's.
  **One read site is not optional (rev 6; pass 5, W1): the loader reads the latch once after the
  mod's init returns**, through the same probe it poisoned through — one load, at load time — so a
  mod that registers no scheduled system (an asset- or prototype-only mod, the kind instance 6 says
  reaches a mint first) and takes `try_register_tag`'s `None` has a read site at all; without it the
  refusal is observable by nobody and the mod runs degraded for the process lifetime. M-A4 leg (b)
  tests the abort *through the SDK boundary*, not a bare panic, and its red controls must include an
  asset-store construction and a fallible tag mint — the two refusals rev 4's classifier could not
  see — and, from rev 6, a zero-system mod that takes the `None`.
  *(The tempting alternative — the host **mirrors** its own tables into the mod's copies at load —
  does not work here: engine ids are minted lazily on first use through `component_id()`'s
  `OnceLock`, so the host's own tables are incomplete at mod-load time and drift apart afterwards.)*

**⚠ What follows from (iii) is a Stage 0 item, not a Stage 1 task.** No in-image mechanism can bind
an author who does not wish to be bound, so A's soundness against a *hostile* mod rests on **who
builds the mod** — a curated repository, signing, or an engine-owned build service that owns the
manifest and the flags. That is a trust-model decision of exactly the kind §9 Stage 0 already holds,
and it is also the only thing that would make leg (a) meaningful. Against the realistic failure — an
honest author writes `world.query::<D, F>()` — or `Assets::<MyAsset>::default()` — because it
compiles, leg (d) makes prevention structural. The honest summary for §8.1 is therefore: **A′ and C
are correct by construction; A is correct by construction against mistakes and by curation against
malice** — where "against mistakes" rests, as of rev 5, on leg (d)'s coverage being the
consumer-derived one above (six mint paths, not two dispensers) and on limit (v)'s signal being
structural; both are Stage 1 items if A is chosen, and until they are built the sentence is a
specification, not a property.

---

## 1. Option A — dynamic libraries with an exact-build contract

*Same toolchain, same flags, same engine build hash; Rust ABI inside; mod is a `cdylib` linking the
engine's rlibs; the host hands it everything it may not mint.*

### Mechanism

The engine ships as it does today: rlibs, statically linked, **fat LTO preserved**. The mod is a
`cdylib` that links the same engine rlibs. Because the mod carries its own copy of every engine
static (M-1, §0.2), the mod may not reach any minting or registry-classifying path **at all** — and
that is a *structural* requirement on the mod-facing surface, **not a contract the mod promises to
keep**. §0.3 states why the promise form is unenforceable (the mint happens inside monomorphised
generic code the mod instantiates itself), what the decidable partition is, and what the façade +
by-stable-name mint entry point must look like. Inside the mod, on the (i) side of that partition,
everything is ordinary Rust: row iteration over `&ModComp` columns, generics, SIMD.

Contract enforcement is a **build canary**, the technique stabby uses (`rustc`, `opt_level`,
`target`, `host`, `debug`, `num_jobs` canaries resolved by the linker, so a mismatch fails to
*load*, not to *run*). Here it becomes one exported symbol:

```rust
#[unsafe(no_mangle)]
pub static BOYKO_BUILD_ID: [u8; 32] = /* const hash */;
```

covering **exactly the axes `-C metadata` omits**: rustc commit hash, target triple,
`-C target-cpu`/`target-feature`, opt-level, LTO mode, panic strategy, `BOYKO_PROFILE`, the cargo
feature set, and the engine source revision. This matters because Cargo *deliberately* keeps
`RUSTFLAGS` out of `-C metadata` ([cargo#14830](https://github.com/rust-lang/cargo/pull/14830)), so
the compiler's own v0-mangling protection is **blind to `-C target-cpu=x86-64-v3`** — the flag this
engine sets workspace-wide (`.cargo/config.toml:85,88,91`).

**What the canary is and is not asked to cover (rev 6; pass 5, question 1).** The language
guarantees neither `repr(Rust)` field order nor `dyn` vtable layout across compilations; both are
deterministic for one compiler on one input, which is what the axis list above pins. Rev 6 makes
the question moot for the two structures that used to cross: `Access` and `SystemMeta` are
`#[repr(C)]` (`system/access.rs:45`, `system/system_meta.rs:86`), and under §7.9(c) no
`Box<dyn System>` crosses the image boundary at all — the host builds the system, its vtable and its
`Access` in its own image from a `#[repr(C)]` descriptor of `(ComponentId, read|write)` pairs, so the
only `repr(Rust)` value that ever crosses is the mod-owned `QueryDataState<D, F>`, which the host
never reads (it stores a `NonNull<()>` and calls the mod's `drop_fn`). What remains for M-T2 is
`TypeId` agreement, and it is informational for A (ids are host-minted by stable name).

### What "as fast as native" means here

- **Where the boundary is crossed:** once per mod at load (handshake), once per registered system per
  frame, and once per engine call the mod makes that is neither generic nor `#[inline]`. **The
  per-system crossing, priced exactly (rev 6, R2):** the schedule already reaches every system
  through one indirect call — `(*system_slot).system.run_unsafe(cell_copy)` on the worker path
  (`schedule.rs:1380`) and `.system.run_dispatcher(token)` on the dispatcher-solo path (`:1215`),
  through the `Box<dyn System<Out = ()>>` vtable in `SystemBox` (`system_box.rs:83`). A mod system
  registered through §7.9(c)'s seam is a host-allocated `System` impl over a function pointer, so
  that vtable call lands in the host image and makes **one more indirect call** into the mod's
  trampoline: two per-system-run indirect calls where an engine system pays one. That is the same
  cost class the dispatch already is — once per system run, never per row — so it is *not a new cost
  class*; it is one extra hop of M-4's kind (~2 ns) per mod system per frame, and it is paid only
  by mod systems.
- **How often:** the first two are bounded by system count. The third is the one to design against.
- **LTO/PGO/inlining reach:** LTO is preserved **inside** the engine binary and **inside** the mod,
  but **not across**. Generic and `#[inline]` engine code is instantiated in the mod and is fully
  optimised there (§0.1). Engine PGO does not cover mod code and vice versa.
- **Prediction (unmeasured):** a mod system whose loop is `Query<&A, &mut B>` over mod-defined
  components runs within noise of the same system compiled into the engine, because the loop never
  crosses. A mod system that calls `EcsMaster::spawn` per entity pays ~2 ns × N. **Measurement M-A1
  in §10.**

### Kernel features it needs

| Need | State today | Change |
|---|---|---|
| **Type identity** | `stable_name()` + `LAYOUT_FINGERPRINT` + `FORMAT_VERSION` already exist (`component.rs:174,181,195`); `STABLE_NAME_INDEX` already resolves name → id (`serialize.rs:363,383,410`) | make `stable_name` **mandatory** for mod components (the `type_name` default is documented-unstable) and add the layout check at registration, as flecs does (`ECS_INVALID_COMPONENT_SIZE`) |
| **Dynamic component registration** | `try_register_dynamic(ComponentLayout) -> Option<ComponentId>` exists at `…/component_registry/mod.rs:967` but is `pub(crate)`; `register_asset_layout<T>(drop_fn)` (`asset/backing.rs:115`) is already the exact shape — caller-supplied drop glue — **and is already `pub`** (re-exported at `asset/mod.rs:60`, reached from any crate through `Assets::<T>::with_reserved` and the `#[macro_export]` `impl_asset_pod_backing!`; §0.3 instance 6), so a foreign crate can mint a component id today, generically, by constructing an asset store; what is missing is the descriptor, not the reachability | promote to a **public Rust-ABI** descriptor path taking `(size, align, drop_fn, clone_fn, serialize_fn, map_entities_fn, stable_name, fingerprint)` — inside `boyko_ecs` a visibility change plus two by-value cold setters, **no `extern "C"` and no `#[no_mangle]` in the kernel**; the `extern "C"` entry a foreign image calls lives in `boyko_mod_host` (§7.10 item 1). Bevy's `ComponentDescriptor::new_with_layout` is the reference signature and its safety contract is the right one to copy |
| **System registration** | `Box<dyn System<Out=()>>` slots already exist (`system_box.rs:83`); the *dispatch* of a mod system is **not a new cost class** (one indirect call per system run — above). **The *registration* path is a cross-image allocation the allocator row forbids (rev 6; pass 5, C2):** `App::add_systems` (`app.rs:339`), `add_systems_in` (`:398`), `add_systems_cfg` / `add_systems_cfg_in` (`:318`, `:372`, which hand out `&mut ScheduleBuilder`), `add_startup_system` (`:508`, boxing a closure at `:514`) and `add_plugin<P: Plugin>` (`:550`, `plugin.build(&mut App)`) are all `pub`; `ScheduleBuilder::add_system<F, M>` (`schedule_builder.rs:177-191`) is `pub` and generic, its `Box::new(sys)` (`:183`) is monomorphised in the mod's image, and the box is owned by the host's `Schedule` (`SystemBox { system: Box<dyn System<Out = ()>>, … }`, `system_box.rs:80-83`; no `impl Drop for Schedule` exists — grep at `d552be05`) and freed by host drop glue | **ruling R2:** every entry point above joins the façade's forbidden set (§0.3, leg (b) fixtures). A mod registers a system **only** through a host-side seam that takes a function pointer plus an access descriptor and allocates the `SystemBox` in the **host** image — §7.9(c) — or, if a mod-side allocation is ever unavoidable, hands over the deallocation fn pointer of its own image with it, so the allocating image frees. Mod systems are still added in the config phase (schedule rebuild below); the seam is the config-phase entry the host exposes to the loader |
| **Allocator** | no `#[global_allocator]` in the engine today; `ComponentPool` storage is `VmReservation` (`memory/vm.rs:63,109,199`) — OS pages, no allocator identity, **valid across binaries**. The adopted decision is **per-type replacement by lifetime class**, with `#[global_allocator]` rejected as an allocator and kept only as the deny gate (`docs/memory/ALLOCATOR-DESIGN-SPACE.md:63`) | **the mod must never free host memory or vice versa, and "honour the rule on both sides" does not achieve that** — per-type allocator state lives in engine types and engine statics, so both sides honouring it gives the same allocator *code* over two different allocator *states* (§0.3 instance 4). **The rule, restated in rev 5 as a property of who frees (pass 4, W3): memory is freed by the image that allocated it, through that image's own code, and that code stays mapped for the allocation's whole lifetime.** Two forms satisfy it trivially — every mod allocation goes through a host call, or every allocation the mod makes dies inside the same call — and a third is sanctioned because it is the façade's own mechanism: a mod-allocated `QueryDataState<D, F>` handed to the host as the `(NonNull<()>, drop_fn)` pair, where the host *stores* the pointer for the world's lifetime but frees it by calling the **mod's** monomorphised `drop_fn` (`ecs_master.rs:335-338`, `:390-400`) — allocation and free both run mod-image code, and §7.7's load-only ruling (no `FreeLibrary`, every world dropped before the image) supplies the "stays mapped" half. Rev 4's "only two sound forms" would have forbidden the façade §0.3 prescribes two paragraphs after stating it, and a host-side allocation cannot serve that façade: the layout is monomorphised in the mod for types the host has never seen. `#[global_allocator]` being silently ignored under `-C prefer-dynamic` ([rust#100781](https://github.com/rust-lang/rust/issues/100781)) is not the mechanism at issue and does not apply. **Rev 6 applies the rule to the one path it had missed (pass 5, C2):** mod-system registration is the *first* form — the allocation is a host call — never the third, because a `Box<dyn System>` carries no mod-owned drop fn the host could call. Today the crossing is harmless *by accident*: no shipped image installs a `#[global_allocator]` (grep at `d552be05`: every hit is a bench, a test module, or `boyko_app/src/profiling/alloc_shim.rs:145`, which is `#[cfg(feature = "profiling-alloc")]` and absent from the shipped build), so both images reach `std`'s `System` allocator and one process heap. After the adopted per-type migration — whose end state puts `Box::new` in engine crates in scope (`ALLOCATOR-DESIGN-SPACE.md:59`, `:72`) — a mod-image `Box::new` freed by the host is heap corruption at world teardown, in release, on the one path every mod must take; and the `DenyAfterSteady` gate allocator (`:77`) is blind to allocations made in the mod's image, so a modded gate binary would be green from emptiness on that leg. The `name: &'static str` a `SystemBox` caches (`system_box.rs:99`) points into the mod's rodata and is covered by §7.7's no-`FreeLibrary` invariant |
| **TLS** | three slots (`ACTIVE_POOL`, `LANE_DEPOSIT`, `IN_SYSTEM_RUN`, `boyko_threadpool/src/tls.rs:169`ff) | **the sharpest hazard in option A.** The mod's copy of `ACTIVE_POOL` reads null, and invariant **PAR7** (`…/iters/query/par_iter.rs:28`) then *silently falls back to sequential iteration*. A mod's `par_iter` would be serial with no diagnostic. Fix: the mod must not call `par_iter` directly, or the handshake must install **all three slots** into the mod's TLS at each system entry. Installing `ACTIVE_POOL` alone is worse than either end: `par_iter` stops falling back to serial but deposits through the wrong lane (locality lost, still no test red), and the ALLOC1 `debug_assert!`s still pass vacuously (§0.3 instance 5). Three writes, priced in M-A2 |
| **Panic handling** | `panic = unwind`; workers `catch_unwind` and re-raise on the dispatcher; nothing above that catches (`boyko_app/src` has no `catch_unwind` outside tests) | the mod's entry points must `catch_unwind` internally and return a status — **in an SDK-emitted trampoline that aborts when the kernel's per-image mint-refusal latch is set** (§0.3 leg (d) limit (v) — a structural signal; rev 4's match on panic text is withdrawn, it missed four of the reachable texts and every future edit to them), or the poison is swallowed into a status. An unwind out of `extern "C"` aborts ([1.81](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/)); a foreign unwind across two `std` copies is **unspecified** ([`catch_unwind`](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)) |
| **Schedule rebuild** | frozen at `finish()` (`app.rs:583`); no `add_system`/`remove_system` on `Schedule` | either mods load **before** `finish()` (the §12.13 slot — no kernel change) or a rebuild API is added. Load-before-finish is strongly preferred; it costs nothing |
| **Unload** | impossible (§0.2 constraint 3) | **out of scope for A.** Load-only, process-lifetime, like abi_stable and Burst |

### Toolchain and distribution cost

Mod authors need the **byte-identical rustc** and the engine's rlibs (E0514 enforces the first;
the canary enforces the rest). That means shipping a modding SDK: the engine rlibs plus a pinned
toolchain reference — **and the cargo configuration, not only the canary**. `-C target-cpu=x86-64-v3`
comes from `.cargo/config.toml:85,88,91`, which does **not** apply to a mod built in its own
workspace; and this tree has already recorded that a `RUSTFLAGS` environment variable *replaces*
config rustflags rather than extending them. A mod author with `RUSTFLAGS` set therefore produces a
non-AVX2 binary silently. The canary catches it at load, which is good; the SDK should make the
failure **unreachable** rather than merely diagnosable. Mods are per-engine-build artifacts — the Unreal BuildId model, and SKSE's
model. **This is the honest cost: a mod compiled against engine build N does not load into build
N+1.** Every engine patch invalidates the mod ecosystem unless the SDK is versioned and mods are
rebuilt.

### Security model

**None.** Full trust, full address space, no sandbox. Same as every native path in the field
(MODDING-RESEARCH §7). Mitigation is out-of-band: signing, an allow-list, or a curated repository.
Fractureiser is the field's demonstration that mod-account compromise is a real distribution vector.

### Failure modes

1. **The canary is not checked before the mod's initializers run.** Loading a `cdylib` runs `ctor`
   code before any call — one of Bevy's stated reasons for removing dynamic plugins.
   **This is reducible, and rev 1 closed it as residual without testing that.** On Windows the PE
   export directory can be read straight from the file, or the image mapped with
   `LOAD_LIBRARY_AS_DATAFILE` / `DONT_RESOLVE_DLL_REFERENCES`, so `BOYKO_BUILD_ID` can be read
   **without executing a byte of mod code**; this tree already owns the raw `LoadLibraryA` /
   `GetProcAddress` path (`crates/boyko_rhi_vulkan/src/device.rs:1660`). The residual risk is then
   only a mod that is *loaded* after the canary passes, which is the intended order. This matters
   more than it looks: the canary is A's **only** defence against failure mode 4.
2. **A mod reaches a mint anyway.** The severe instance is not a wrong `ComponentId` but a **query
   type-id collision that type-confuses the host's query state cache** — silent, in release, on the
   path §0.1 declares free (§0.3 instance 1); the *frequent* instance is a per-type
   `component_id()` minted in the mod's own image (§0.3 instance 2). Prevention must be structural,
   and rev 3 corrects what "structural" can mean here: the façade and the `compile_fail` fixtures
   bind only a cooperating author, the load-time **address** cross-check cannot discriminate at all,
   and the leg that does is **poisoning the mod image's mint counters** so that a mint panics before
   its id is used (§0.3 leg (d)). Against a *hostile* author none of it binds, which is why A's
   trust model is a Stage 0 question. A violated convention here is memory corruption, not a panic —
   unless the poison is in place, which is precisely its point.
3. **`par_iter` silently serial** (TLS, above) — and the same TLS split makes the ALLOC1
   allocation-discipline `debug_assert!`s **pass vacuously** inside the mod, because its
   `IN_SYSTEM_RUN` copy reads `0` (§0.3 instance 5).
4. **SIMD ABI mismatch** — `-C target-cpu` is not in `-C metadata`; the intra-compilation vector-ABI
   hard error of 1.87 ([rust#116558](https://github.com/rust-lang/rust/issues/116558)) does not apply
   across two binaries. The canary is the only defence.
5. **Allocator crossing** — a `Vec` allocated in the mod and freed by the host is UB once the engine
   installs its own allocator. Rev 6 found the instance every mod would have hit — the
   `Box<dyn System>` of `ScheduleBuilder::add_system` — and closed it by ruling (R2, §7.9(c)); the
   class stays open for any future `pub` generic that boxes on the mod's side and stores on the
   host's, which is why the forbidden set is a list with fixtures rather than a principle.
6. **Windows export ceiling** — only if the engine exports; A exports one canary symbol plus the
   handshake entry, so this is not reached. (It is reached by option B if the host exports a wide
   API, and by any dylib engine.)
7. **Id budget exhaustion is a panic**, not an error (`…/component_registry/mod.rs:922`); 512
   components shared with dynamic tags **and with the 128-slot physics scratch band already reserved
   at the top** (`boyko_physics/src/scratch_ids.rs:685`, `MAX_COMPONENTS - 128`), **256 resources
   against ~145 already used**.
8. **Archetype signature divergence.** The mod's `STORAGE_KIND` / `RESIDENCY_CLASS` copies are empty,
   so every engine id reads `Table` / `Cpu` there — a dense engine component re-enters the signature
   inside the mod with no compile error (§0.3 instance 3). This is the failure mode that breaks
   §7.4, the section A depends on for mod components on engine entities.

### Cost when modding is NOT used

**Exactly zero, and it is provable at the level of code generation, provided one shape holds:**

> **Engine types keep compile-time identity and storage; only mod types go through a registration
> path; and that path lives in a crate the non-modding build does not depend on.**

Concretely:
- `#[derive(Component)]` is untouched. `component_id()` remains the per-type
  `static ID: OnceLock<ComponentId>` (`crates/boyko_macros/src/component.rs:371-372`) — **no
  `TypeId → ComponentId` map is introduced on the typed path.** (This is exactly where Bevy pays:
  its `Components::indices` hashmap is consulted by *typed* queries at `init_state`. Regressing this
  engine's `OnceLock` into a map "for mods" would violate P1 permanently.)
- Every compile-time discriminator on the `Component` trait keeps its zero-cost default
  (`component.rs:51,65,79`) — a mod-registered type is described by *data at runtime*, and the const
  arms are untouched for engine types.
- `try_register_dynamic` already exists and is already reachable (via `register_asset_layout`), so
  making it public adds **no new branch to any engine path** — the descriptor path is a cold
  registration function, not a hot-path fork. The per-item accounting — which kernel items change, by
  what mechanism, and what a non-modding build carries — is **§7.10** (rev 4): the kernel gains
  visibility and a few cold Rust-ABI functions, never an attribute, a `cfg`, a feature or a constant
  that differs per configuration — **and, from rev 6, every such item is admitted only while
  M-P1's pins, captured on the pre-change tree, stay identical in the post-change non-modding build
  (R1); "cold" and "unreachable" are arguments, the pins are the gate.**
- The loader itself lives in a **separate crate** (`boyko_mod_host`), not a feature on `boyko_ecs`.
  A game that does not depend on it has no loader code, no `libloading`, no directory scan, no
  canary symbol, no exported entry point. `cargo tree` proves absence; M-P1's pins prove codegen
  equivalence against the pre-change tree (§10).

**Why a separate crate and not a cargo feature:** feature unification. "Cargo will use the union of
all features enabled on that dependency"
([Cargo book](https://doc.rust-lang.org/cargo/reference/features.html)) — any third-party crate in
the graph can switch a `modding` feature on for a game that never wanted it, with no diff visible in
the source. A crate nothing else depends on is immune by construction. This tree already has the
precedent: `boyko_reflect` is an optional dependency behind a feature so that with it off "the crate
is not in the consumer's resolved dependency closure at all — not compiled, no rlib, no symbols"
(`D:/wt/reflect/crates/boyko_reflect/src/lib.rs:1-26`), and the absence is **demonstrated by gates,
never asserted**.

### Cost with modding compiled in, no mod loaded

- **Per-frame: zero.** No schedule entry exists, no system runs, no registry has a mod row. The
  `Box<dyn System>` slots are the same ones engine systems use.
- **Startup:** one directory scan plus a manifest parse. Budget by the Bevy analogue (<25 ms native
  debug for 1614 types) and measure (M-A3).
- **Binary:** the loader crate (`libloading` equivalent — or the in-house `LoadLibraryA` path already
  at `crates/boyko_rhi_vulkan/src/device.rs:1660`), the canary constant, the descriptor entry
  points. Small, and it is a *different build* from the non-modding one, so it does not touch P1.
- **Exports:** the handshake symbols only. Not enough to approach the PE 65535 ceiling.
- **⚠ The one non-obvious item:** the descriptor of §7.1 is an **input struct**, not a widened
  `ComponentLayout`. `ComponentLayout` stays at 56 B (`…/component_registry/mod.rs:109`, pinned by
  `const _: () = assert!(size_of::<ComponentLayout>() == 56)` at `:133`, TRIPWIRE 2), and every
  extra field the descriptor carries fans out into the **existing parallel cold tables** —
  `CLONE`/`MAP_ENTITIES` (`clone.rs:21,82`), `SERIALIZE` (`serialize.rs:16,35,122`), `STORAGE_KIND`
  (`mod.rs:363`), `RESIDENCY_CLASS` (`:503`), `HOOKS` (`:221`) — which exist *precisely* to keep that
  pin. Nor may `Access` widen past its `const _: () = assert!(size_of::<Access>() == 192)` pin
  (`system/access.rs:61`). If mods force `MAX_COMPONENTS` 512 → 1024, `ComponentMask` doubles,
  `Access` goes 192 B → 320 B (3 cache lines → 5), and **every build pays it, every system, every
  schedule build.** That is the one way option A leaks cost into P1 — and the reason §8 makes id
  budgeting a first-class decision rather than a later detail.

---

## 1.5 Option A′ — the engine as a Rust `dylib`, in the modding configuration only

*Rev 1 dropped this route silently; it belongs on the table because it dissolves §0.3 rather than
fencing it. Rev 2 called it the **only** such option; rev 3 withdraws the "only" — §1.6 prices the
one other shape that might, and cannot be dismissed without a measurement.*

### Mechanism

Two shipping configurations, which is a shape §1 already accepts and which this document already
praises in Unreal's monolithic-vs-modular switch (research §3.2):

- **non-modding build** — today's build, unchanged: static rlibs, `lto = "fat"`;
- **modding build** — the engine crates built as `crate-type = ["dylib"]`, everything
  `-C prefer-dynamic`, LTO forfeited.

**How the two configurations are expressed — and why the obvious mechanism is the dangerous one.**
Rev 2 wrote "two shipping configurations" without saying how one flips between them, and the answer
is not free. `crate-type` is a **static `[lib]` key**: it is not conditional on a feature or on a
profile, and **no `crate-type` key exists anywhere in this workspace today** (grep over
`crates/**/*.toml`, 2026-09-16 — the only related keys are `proc-macro = true` in `crates/aether`
and `crates/boyko_macros`, which are host artifacts and stay as they are). Three mechanisms exist and
only one is a real candidate:

| mechanism | verdict |
|---|---|
| **(i) `crate-type = ["rlib", "dylib"]` in the engine libs' manifests** | the obvious route, and the one with a **silent** failure mode — see below |
| (ii) a second, duplicated manifest set (an overlay applied to build the modding configuration) | works, and costs a permanent per-crate maintenance item plus a gate that the overlay stays in step with every crate added to the workspace |
| (iii) `cargo rustc --crate-type` | insufficient: "This flag only works when building a `lib` or `example` library target" and overrides only the selected package's manifest ([cargo-rustc](https://doc.rust-lang.org/cargo/commands/cargo-rustc.html)) — dependencies keep building as rlibs |

**⚠ Route (i)'s failure mode is a silent loss of LTO in the NON-modding build.** A lib with several
crate types, one of which does not support LTO, is reported to have LTO **ignored for all of them**:
*"LTO ignored for all crate types when building multiple crate types and one doesn't support it"* …
*"The `lto = true` is ignored for everything because rlibs don't support LTO"*
([rust#51009](https://github.com/rust-lang/rust/issues/51009), **still open**). Cargo has since
special-cased the `cdylib`/`staticlib` pairing — it compiles the dependency tree with embedded
bitcode and links the dynamic artifact separately ([cargo#8254](https://github.com/rust-lang/cargo/pull/8254))
— and that PR's discussion does **not** cover `rlib` + `dylib`. If the shipped build loses
`lto = "fat"` this way there is **no diagnostic**: it costs geomean **0.793** and `query_ref_iter/10k`
**0.375**, in the exact leg the owner's hard constraint protects, and nothing goes red. Whether
cargo 2026 still behaves this way for `rlib` + `dylib` is **unmeasured** — which is why M-F1 gains a
leg that measures the non-modding build itself rather than the modding one.

### Why it is on the table at all

**M-2 measured that this route makes every registry static, every TLS key and the whole `TypeId`
universe shared** — `mod NEXT_ID addr == host NEXT_ID addr`, mints interleaving correctly across the
boundary. Every one of §0.3's five instances **disappears by construction**, not by enforcement:
there is one `QUERY_NEXT_ID`, one `STORAGE_KIND` table, one allocator state, one set of TLS slots.
`par_iter` works. The mod may call `world.query::<D, F>()` and it is simply correct.

### The price, which is the largest number in this document

`lto = "fat"` is forfeited for the **modding configuration**: geomean **0.793**,
`query_ref_iter/10k` **0.375** (2.67×), binary 8.76 → 3.46 MB (`Cargo.toml:89-112`, measured
2026-09-04). **That geomean is over 7 ECS microbenchmarks, and it must be quoted with its
denominator** (rev 2's own W5 ruling, applied here to rev 2's own text): of the three ratios the
profile comment names, the spread is `add fill` **0.849** (1.18×) to `query_ref_iter` **0.375**
(2.67×), with `swap_remove` at 0.655 (1.53×). A modded *session* number is neither of these — a
frame also contains render, GPU wait and physics — and **no session-level number exists**. The
defensible statement is: *1.18×–2.67× across the bench suite, geomean 1.26×, with the worst case
sitting on `query_ref_iter`, which is the shape a mod system's inner loop most often has.* **On
stable.** Item 5 of *What is unestablished* below records that nightly's `-Z dylib-lto` keeps fat LTO
inside the dylib at toy scale (M-11), so the workspace-scale price under nightly is open (M-F1 leg (d)).

Plus: `std`/`core`/`alloc`/`compiler_builtins`/`libc` must ship and resolve as DLLs
(M-2 measured the host refusing to start until the toolchain lib dir was on `PATH`), and the PE
65535-export ceiling becomes a live question for this crate graph (research §2.1; unmeasured).

### Why the owner's constraint does **not** exclude it

The hard constraint is about **the build a non-modding game ships**. In the two-configuration world
above, the non-modding build is *intended* to be literally today's build, and the 0.793 is forgone
**only by players who load mods**.

**⚠ Rev 2 turned that intent into a waiver — "the P1 gate is trivially green and needs no asm diff at
all" — and rev 3 withdraws it.** "By construction" holds only once the configuration mechanism is
named, and the named mechanism's known failure mode (route (i) above, rust#51009) is a build that
*looks* unchanged and has silently lost fat LTO. A′ is therefore the one option where P1 could be
lost with no error message, so it gets the **same** gate as every other option, not a waiver:
M-P1's pins against the pre-change tree (rev 6) — the asm pins on a named deterministic profile,
the binary-size / `.text` pins against the 3.46 MB LTO baseline — plus `query_ref_iter` on the
non-modding build **after** the mechanism is in place (M-F1 leg (c)). With that gate the comparison is:

> **A′: 1.18×–2.67× slower on the 7-bench suite (geomean 1.26×) for modded sessions, correct by
> construction — and its non-modding build is gated, not assumed.**
> **A: 1.00× for modded sessions, correct by construction against mistakes (§0.3 leg (d)) and by
> curation against malice.**

### What is unestablished

Five items, all cheap (the fifth added in rev 4):

1. whether this workspace builds as `crate-type = ["dylib"]` at all (the `const _: () = assert!`
   layout pins and the const-generic `SpirvBlob<N>` statics are untested there);
2. whether the PE 65535-export ceiling is reachable on windows-gnu with this crate graph
   (research §2.1);
3. **whether the configuration mechanism costs the non-modding build its LTO** — route (i) above and
   rust#51009. This is the item that changed status in rev 3: it is not a modding-side risk, it is a
   P1 risk, and it is silent;
4. **`TypeId` equality across separately-*configured* builds** — research §14's open item, measured
   by **M-T2**. A′ is a shared-`TypeId` variant, so if the two configurations were ever allowed to
   differ in features or `-C metadata`, the shared query intern's `(TypeId, TypeId)` key is the thing
   that breaks. A′'s answer is the build canary of §1 (identical rustc, flags, features, engine
   revision on both sides) — which makes the configurations identical by construction and M-T2
   informational rather than blocking. **That is a claim about the canary's coverage, so it is
   stated here rather than assumed**, and §8.2's A′ row now says it.

5. **Whether the LTO forfeit is the whole price, or only stable's price (rev 4).** M-3's own error
   text names the escape — *"lto cannot be used for `dylib` crate type without `-Zdylib-lto`"* — and
   the flag exists on nightly (*"enables using LTO for the `dylib` crate type … currently only used
   for compiling `rustc` itself"*,
   [unstable book](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/dylib-lto.html)).
   **Measured at toy scale (M-11, research §13.3):** on `rustc 1.100.0-nightly (8925ea358 2026-08-20)`
   the M-2 rig builds with `--crate-type=dylib -C prefer-dynamic -C lto=fat -Z dylib-lto`, a
   `prefer-dynamic` bin links it **with and without** `-C lto=fat -Z dylib-lto` of its own (M-3's
   second error does not fire), both run, and the static is still shared (same `NEXT_ID` address —
   M-2's check). So on nightly the modding configuration can keep fat LTO **inside** the engine dylib
   and forfeit only the cross-image inlining between game code and engine — M-4's delta, not the
   0.793. Whether the workspace-scale number moves from 1.26× toward 1.00× is **M-F1 leg (d)**; the
   flag is nightly-only (owner-permitted in this tree, unstable by contract, and its own page says it
   is "currently only used for compiling `rustc` itself"), and it carries the same rust#51009
   exposure as the rest of §1.5.

**M-F1 in §10** covers (1), (2), (3) and (5); **M-T2** covers (4).

---

## 1.6 Option A″ — one shared-state artifact, everything else static and LTO'd

*Neither rev 1 nor rev 2 considered this shape. Every part of it is unestablished, and it is written
down because §1.5's "the only option that dissolves §0.3" was a strong claim resting on no argument,
and this is the shape that would refute it.*

**The observation it rests on.** §0.3's hazard is entirely about **process-global state**, never
about code. A′ pays for the state half by making the *code* dynamic as well, and the LTO forfeit is
the price of the code half alone. The question rev 2 never asked is whether the state can be shared
without the code.

**The shape.** The duplicated statics — `NEXT_ID`, `LAYOUTS`, `QUERY_NEXT_ID`, `REGISTRY`, the
resource / bundle / event dispensers, `STORAGE_KIND`, `RESIDENCY_CLASS`, the threadpool's three TLS
slots, and whatever the per-type allocator decision owns — move into one small crate with an
`extern "C"` accessor surface, built as a **`cdylib`** in the modding configuration and linked by
host and mod alike. Everything else stays static rlibs with `lto = "fat"` on **both** sides. Because
only plain data and `extern "C"` calls cross, there is no `-C prefer-dynamic` and therefore none of
A′'s `std`/`core`/`alloc` DLL shipping (M-2).

**Why it is not obviously absurd — one verified fact, and rev 4 corrects the frequency claim that was
attached to it (pass 3, W1).** The registry is **not** on the row loop: `ComponentPool` caches
`component_layout`, `drop_fn` and `component_type_id` in its own header ("Component layout (cached
from registry for performance)", `crates/boyko_ecs/src/ecs/memory/component_pool.rs:194-195`,
`:234-240`), so per-row iteration reads no registry static at all — that half holds, and the
enable/filter paths confirm it (`filter_enable.rs:166-177` and `data_is_enabled.rs:159` read
`storage_kind` in `init_state` under a `debug_assert!`, never per row). **The other half was wrong.**
Rev 3 wrote that `storage_kind` / `residency_class` are read "once per archetype construction";
`storage_kind` is in fact read on **per-entity and per-command** paths: inside the per-entity
component fetch right after the generation check (`ecs_master/component_api.rs:201`, and `:274`,
`:467`, `:657`, `:765`), in the structural-op partition (`entity_api.rs:111,123`), once per bundle id
per insert (`commands/insert_command.rs:135,256`), per remove (`remove_command.rs:87`), per bundle id
per migrated entity (`migration_helpers.rs:461` via `any_dense`, `:469`, `:660`, `:777`) and per
component per spawned row in a batch (`spawn_batch_command.rs:586`). Today each is one relaxed
`AtomicU8` load from a static array (`mod.rs:375`); under A″ each becomes a non-inlinable cross-image
call at M-4's 1.7–2.5 ns. A 4-component bundle insert crosses at least five times per entity; a
10k-entity command flush crosses ~50k times, ~100 µs of pure boundary per flush. The archetype sites
(`archetype_master.rs:193,510,729`, `:535`) are the cold ones; the command path is not.

**And a second defect, found while re-checking the first: sharing the dispensers is not enough to
share the ids.** `register_new::<T>()` is mint-only — `fetch_add`, then `LAYOUTS[raw].set`, with no
`TypeId` lookup (`mod.rs:920-945`); the de-duplication that gives `Transform` one id is the derive's
per-type `static ID: OnceLock` (`component.rs:372`), and that cell lives in the **rlib** both images
link statically under A″. M-10 leg 1 (research §13.3) shows it as a per-image global data symbol
(`D eng::component_id::ID`) that every instantiating crate references by name — so the mod's copy of
`Transform::component_id()` finds its own empty cell, calls the *shared* dispenser, and receives a
**fresh** id: Transform has two ids in one process, one per image, and the mod reads the wrong
column. Queries and resources do not have this defect — their interns are `TypeId`-keyed
(`query_type_registry.rs:206`, `resource_type_registry.rs:70`) and would move into the shared crate
whole. Components would need a `TypeId`-keyed intern **in the shared crate**, consulted by
`register_new` before it mints — a change to the mint path's semantics (lookup-or-mint) that must
exist in the modding configuration only, i.e. the same unestablished `crate-type` flip as everything
else in this section. And the derive's install arms (`component.rs:387-392`) write `STORAGE_KIND` /
`CLONE` / `SERIALIZE` from whichever image first mints a type, so those tables diverge in **both**
directions unless their setters cross the seam too.

**Why it is nonetheless unestablished, and expensively so.** Four unknowns, in descending danger:

- the **allocator** half (§0.3 instance 4) is per-type state owned by engine types
  (`docs/memory/ALLOCATOR-DESIGN-SPACE.md:63`); moving it behind an `extern "C"` seam puts a call on
  every allocation in the modding configuration, and no such design exists;
- the **TLS** half (instance 5) is read at every `par_iter` entry and by the ALLOC1 discipline
  asserts; same call cost, at a frequency nobody has measured;
- the shared query intern is keyed on `(TypeId::of::<D>(), TypeId::of::<F>())`, so it is correct only
  while both images agree on `TypeId` — the build canary's job, and **M-T2**'s question;
- the configuration flip is the **same** unestablished `crate-type` mechanism as A′ (§1.5), carrying
  the same rust#51009 exposure for the non-modding build.

**What it would buy if it worked — restated after the two corrections: A′'s correctness at A's speed
on the iteration and system-run paths, and A′'s price or worse on the command path.** One copy of
every counter and fat LTO on both sides — but every structural command pays a cross-image call per
bundle id per entity, the frequency class §0.1 calls catastrophic, **unless** `STORAGE_KIND` /
`RESIDENCY_CLASS` stay per-image and are **mirrored** rather than moved. A mirror is possible for
these two tables where §0.3 leg (d)'s closing note showed it is not for the ids: they are
`[AtomicU8; 512]` written through `set_storage_kind` / `set_residency_class` (`mod.rs:435`, `:614`),
so the seam carries the *setters* (cold, once per type) and each image keeps its own *readers*
(hot), with a 1 KiB copy per direction at each mod-system boundary. Whether the prize survives the
component-intern requirement and the two-way mirror is what **M-F2 in §10** now asks; it is a
*feasibility* probe of M-5/M-6's kind, not a benchmark, and its scope is wider than rev 3 wrote.

---

## 2. Option B — dynamic libraries behind a stable C-like ABI

*`abi_stable`, `stabby`, or a hand-written `#[repr(C)]` host function table (Godot GDExtension /
The Machinery shape).*

### Mechanism

The engine exports nothing. At load, the host hands the mod a `#[repr(C)]` struct of `extern "C"`
function pointers — the "vtable-of-host" — so all shared state stays on the host side of the
pointers. Everything crossing the line is `#[repr(C)]` or an FFI-safe wrapper. Validation is
load-time layout checking (abi_stable checks every type reachable from the root module, recursively,
**before any function can be called**) or type reports plus canaries (stabby).

### What "as fast as native" means here

- **Where the boundary is crossed:** every mod→engine call, without exception. Generics cannot cross,
  so the mod cannot instantiate `Query<&T>` from engine metadata **unless it also links the engine
  rlibs** — at which point it is option A with a worse data representation.
- **How often:** unbounded, and determined by the API shape rather than by the design.
- **LTO/PGO/inlining reach:** LTO stays inside each side. Nothing inlines across. Every host entry
  point becomes `#[no_mangle] extern "C"`, which **pins those bodies and removes them from LTO
  internalisation** — LLVM's `internalize`, `deadargelim`, `argpromotion`, `globaldce` operate on
  *internal* functions ([LLVM passes](https://llvm.org/docs/Passes.html)). This is a real, if small,
  cost to the *engine's own* codegen, and it must be measured on the specific entry-point set, not
  assumed (M-B1).
- **Measured call costs:** direct 0.82 ns, dlopen'd 1.89 ns, `abi_stable` trait object **3.23 ns**
  ([nullderef](https://nullderef.com/blog/plugin-abi-stable/)); godot-rust FFI 248–288 ns per call
  *before* caching the function pointer, **7–32 ns after**
  ([godot-rust](https://godot-rust.github.io/dev/ffi-optimizations-benchmarking/)).
  Against a 3.4–4.0 ns/row budget, **7–32 ns is 2–9 rows of work per call.** This is the option that
  makes per-row crossings impossible and per-system crossings still visible.
- **The data-representation tax is the real number, and it was measured:** Tremor slowed by **~36 %,
  reduced to ~30 %**, *with the plugins still statically linked* — the loss came from the
  `#[repr(C)]` interface and its FFI-safe types (`RHashMap` instead of `halfbrown`), not from dynamic
  loading ([nullderef, 2022-07-26](https://nullderef.com/blog/plugin-end/)).

### Kernel features it needs

Everything option A needs, **plus** an FFI-safe mirror of the ECS surface, **minus** nothing.
`RVec`/`RBox` are not `ComponentPool` columns; stabby's post-1.78 trait objects go into a leaked
global vtable set with **O(n) lookup and O(n) clone-on-insert**. Their *validation* technique
transfers; their *types* do not. A hand-written table avoids the library tax but not the
representation tax.

Additionally: no shared `TypeId` universe at all, two `std` copies, two allocator universes, two TLS
key sets — every hazard in option A, made permanent rather than contractual.

### Toolchain and distribution cost

**This is B's one genuine advantage.** Mods survive engine patch releases (GDExtension's rule:
extensions built for an older 4.x work in newer ones, not vice versa). No rlib shipping, no
toolchain pinning, and in principle other languages can target the ABI.

### Security model

None, same as A.

### Failure modes

1. **The API acquires cruft.** The Machinery's own retrospective admits APIs "acquire more and more
   'cruft'"; Half-Life's header says "INTERFACE VERSION IS FROZEN AT 138"; Source needs a wrapper
   class to keep an old interface alive. A C ABI is a decade-long commitment made at design time with
   the least information.
2. **The temptation to route engine-internal calls through the table.** The Machinery did exactly
   this ("To patch an API we just replace all the function pointers") and every user pays the
   indirection. **That is a direct violation of the owner's constraint** and must be structurally
   impossible, not merely discouraged.
3. **Per-row crossings creep in** as the API grows convenience accessors.
4. **abi_stable leaks every library by design**; unloading is an explicit non-goal.
5. Panic, allocator and SIMD hazards as in A.

### Cost when modding is NOT used

**Zero, by the same mechanism as A** — the host-table crate is absent, so no `extern "C"` entry
points exist and none are exported. **But the gate is weaker**: the entry points are `#[no_mangle]`
*plain* functions, and the measured trap (`REFLECTION-ANALYSIS.md:1467-1494`) says plain functions
are the **undecidable** case for a symbol census. The proof must be `cargo tree` + M-P1's pins
against the pre-change tree (rev 6), not a symbol grep.

### Cost with modding compiled in, no mod loaded

Higher than A, and the difference is structural rather than incidental: the exported entry points
exist in the shipped binary and are **excluded from LTO internalisation** whether or not a mod is
loaded. Startup cost is the same scan. **Prediction: small but non-zero, and it is the one item on
this page that cannot be made exactly zero by cfg-gating, because the whole point of the option is
that the symbols exist.** Measurement M-B1.

---

## 3. Option C — install-time relink

*Mods ship as objects (or rlibs); the game is relinked on the user's machine with a bundled linker.
Rev 2 described this as "full LTO across engine and mods" — see the route table below: that property
belongs to a route this design does **not** ship.*

### Mechanism — and **which of the three relink routes C ships**

**⚠ Rev 2 used one word, "relink", for three different mechanisms, and attached the performance
argument to one of them while taking the feasibility receipts in another.** Rev 3 separates them and
names the shipped one. The distinction is not pedantic: fat LTO happens **inside rustc**, at the
final crate's codegen, so an install step that does not run rustc cannot produce it — ever.

| route | what it is | what it needs | status here |
|---|---|---|---|
| **1 — rustc fat LTO at install** | the configuration that produced the measured geomean 0.793 / `query_ref_iter` 0.375 (`Cargo.toml:89-112`) | **rustc on the user's machine** | **this is option D** — the plugin-entry table further down applies the same test ("puts rustc on the user's machine ⇒ C *is* D"); excluded from C by definition |
| **2 — linker-plugin LTO** (`-Clinker-plugin-lto`) | rustc emits bitcode into the `.o`/rlib and the **linker** performs LTO ([codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html)) | *"a linker with the LLVM plugin must be used (e.g. LLD)"*; with any other linker *"the LLVM linker plugin has to be specified explicitly … `LLVMgold.so`"* ([rustc book](https://doc.rust-lang.org/rustc/linker-plugin-lto.html)) | **the plugin is not in this toolchain** (verified above: no `LLVMgold.*` anywhere; the only plugin-capable linker is `rust-lld.exe`, **211 MB**, i.e. ~4× the entire 48 MB shippable set M-8 measured). The same page also records that compiling **proc-macros** under this flag errors on Windows-like targets — and this workspace has two (`crates/aether`, `crates/boyko_macros`) — **unverified for windows-gnu** (research §13.2), and **sidestepped rather than met** if it fires: cargo withholds `build.rustflags` / `RUSTFLAGS` from host artifacts whenever `--target` is passed — *"Things being built for the host, such as build scripts or proc macros, will not receive the args"* ([cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags)) — so a vendor build of `cargo build --target x86_64-pc-windows-gnu` never hands the flag to a proc-macro. **Rev 4: after M-10 this is one of C's two live forms** |
| **3 — plain-object relink** | the installer links shipped engine objects with mod objects using an ordinary linker; **no LTO at link time in either direction** | the bundled mingw driver (M-5) | **this is what M-5 / M-6 / M-7 measured**; rev 3 chose its 3b form, **rev 4 measured 3b red at toy scale (M-10)** — what remains of route 3 is 3a |

**Rev 3: C ships route 3**, and the shipped form is route **3b** of the two the route admits (**rev 4
withdraws 3b — M-10, below**):

- **3a — ship the engine as non-LTO objects/rlibs.** The relink is trivially possible, and the modded
  binary is a **non-LTO build**: it forfeits geomean 0.793 exactly as A′ does, without A′'s dylib
  indirection but with all of C's install-time risks. **Rejected**: it pays A′'s price and keeps C's.
- **3b — ship the engine already fat-LTO'd, and plain-link the mod objects onto it.** The engine's
  own cross-crate inlining is preserved (it happened in rustc, at the engine vendor's build), the mod
  objects are compiled with `-O` by the mod author against the engine rlibs, and the final link is
  the measured M-5 line with extra objects on it. **Rev 3 wrote "this is the route to specify";
  rev 4 measured it at toy scale and it is RED — M-10, two paragraphs below.**

Under 3b the installer would have needed: the engine's post-LTO objects plus the `symbols.o` rustc
writes to a temp directory (M-5, `-C save-temps`), the mingw self-contained driver set, and one
`-Wl,--undefined=` per mod (M-7). (The "20 std rlibs" M-8 priced belong to the **non-LTO** line: a
fat-LTO bin's own `--print link-args` names **2** `.rlib`s — a temp-directory `libstd` copy and
`compiler_builtins` — against **21** on the non-LTO line, M-10 leg 7. M-8's 48 MB is route 3a's disk
row.)

**Rev 3 called two properties of 3b predictions; rev 4 measured both at toy scale (M-10, research
§13.3 — two crates, the derive's exact `#[inline]` + function-local `OnceLock` shape, stable 1.98.1,
windows-gnu), and 3b is red on both.**

*First — the LTO'd engine internalises what the mod objects reference, and nothing an export set can
write pins it back.* Under `-C lto=fat -C codegen-units=1` the dispenser, the counter and the
per-type cell all become **local** symbols (`t eng::register_new`, `b eng::NEXT_ID`,
`d eng::component_id::ID` — leg 2), as LLVM's must-preserve rule predicts
([LLVM LTO](https://llvm.org/docs/LinkTimeOptimization.html)): at engine-build time nothing names
them. A mod object compiled `-O` against the engine rlib references exactly those names — leg 4's
undefined list is `eng::register_new`, `__imp__…component_id::ID` (the per-type cell, through
windows-gnu's import-cell indirection) **and three `std` internals** (`Once::call`, `unwrap_failed`,
`panic_cannot_unwind`), because the mod instantiates `OnceLock::get_or_init` too. A `#[no_mangle]`
*function* does survive fat LTO as a global (`T boyko_probe_symbol`, leg 3), so an export set of
functions is expressible — but a **static reachable from an exported function stays local** (leg 2's
`ID` is reachable from `main`), neither stable's `-C link-dead-code` nor nightly's
`-Z export-executable-symbols` changes the binding (legs 2b/2c; the latter exports **only
`#[no_mangle]` symbols** by its own documentation,
[unstable book](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/export-executable-symbols.html)),
and the per-type cell is a function-local `static` that no sentinel crate can name in source at all.
The in-tree evidence rev 3 cited (`REFLECTION-ANALYSIS.md:1477-1494`) is the same mechanism seen from
the other side.

*Second — re-adding the rlibs does not produce a silent duplicate; it does not link either.* With the
engine rlib re-added the engine references resolve **from the rlib** (a second copy of every engine
static enters the image — the §0.3 class, as predicted), but the `std` references remain (leg 6a);
with all 27 sysroot `std` rlibs re-added in a link group, `alloc`'s code from the second `std`
references **`__rustc::__rust_alloc` / `__rust_dealloc` / `__rust_no_alloc_shim_is_unstable_v2`** —
the allocator shim rustc emits once per final artifact, which the host's fat LTO internalised
(leg 6b). The only way to satisfy those is a second allocator shim — a second allocator *state* — in
the image: §0.3 instance 4, inside the option that was supposed to make it vacuous. So the sub-case
the rev-3 text feared (a silent local+global pair) is reachable only through a line an installer
would have to construct on purpose; the sub-case an installer would actually hit is a **loud**
undefined-reference failure, on every mod that instantiates any engine `#[inline]` body or any `std`
generic — i.e. every mod.

**Consequence.** 3b's premise — "mods compiled against the rlibs, plain-linked onto fat-LTO'd
objects" — contradicts itself: compiling against the rlibs is what produces the references, and fat
LTO is what removes their definitions. No export set reconciles them, because the class of symbols to
preserve includes function-local statics and `std` internals. **C's live forms are therefore route
2** — linker-plugin LTO at install, where the mod objects exist when must-preserve is computed and the
LTO is genuinely whole-program including mod code — **or 3a**, which forfeits fat LTO for the modded
build exactly as A′ does. The workspace-scale M-C4 stays queued so that the gate is run rather than
argued, but its expected result is now stated: red for 3b, by a mechanism that does not depend on
workspace size.

### What "as fast as native" means here

**One binary, no boundary — but "LTO covers mod code too" is false for the route C ships, and rev 2
said it anyway.** Restated for route 3b, in three parts — *and rev 4 notes, since 3b is red at toy
scale (M-10): under route 2 the third bullet inverts, because linker-plugin LTO at install is exactly
cross-module optimisation between engine and mod; under 3a the first two bullets hold and the
engine's own 0.793 is forgone*:

- **What is exactly native.** There is no boundary after the relink: no indirect call through an
  import table, no duplicated statics, no `TypeId` divergence, no TLS split, no allocator split, no
  panic-ABI question. §0.2 constraint 2 is vacuous — one binary, one `NEXT_ID` — **subject to M-C4's
  duplicate-definition check above**.
- **What the mod's own code gets.** The mod is compiled by its author against the engine rlibs, so
  every generic and every `#[inline]` engine body it uses is instantiated **in the mod's own
  compilation unit** and optimised there (§0.1, M-4). A mod's `Query<&A, &mut B>` inner loop is
  therefore native in the same sense as under option A — and, unlike A, its `spawn` and registry
  calls are direct calls into the same image rather than boundary crossings.
- **What is NOT obtained, and what rev 2 claimed.** The measured geomean **0.793** and
  `query_ref_iter/10k` **0.375** describe what fat LTO buys **the engine's own code**
  (`Cargo.toml:89-112`), and under 3b the engine keeps every bit of that. What does **not** happen is
  cross-module optimisation *between* engine and mod: an engine function that is neither generic nor
  `#[inline]` is not inlined into mod code, and mod code is not inlined into the engine. Only route 2
  delivers that, at 211 MB of linker and whole-engine bitcode shipped to every player. **The claim
  "the measured geomean 0.793 applies to the whole modded binary" is withdrawn.**

PGO is route-dependent in the same way: the shipped `-Cprofile-use` profile was collected without the
mod's code either way, but under 3b the mod's code is not even in the same optimisation unit as the
profiled engine.

### Kernel features it needs

**Few — but rev 1 counted one of them as free, and it is not.** Mod components go through the
ordinary `#[derive(Component)]` path — they *are* compile-time types in the final binary. Mod systems
are ordinary systems added in the config phase. No descriptor path, no handshake, no canary (the
relink either succeeds or fails), no dynamic registration at all. §0.3 is vacuous for C: one binary,
one copy of every static.

The genuine kernel needs are the shared ones in §7 — id budget (mods consume from the same 512 /
256) — **plus a mod-plugin entry convention, which is a real mechanism with a real cost and a
measured silent-failure mode**, **plus (new in rev 3) a mod-facing export set — see the end of this
subsection.** Rev 1 asserted the convention without stating what it is. A plugin
is added by an explicit `app.add_plugin::<P>(p)` call in user code (`…/app/app.rs:550`); linking a
mod object into the binary does not create that call. There are exactly three routes, and two of
them collapse C into D:

| route | verdict |
|---|---|
| generate a stub `.rs` and compile it | **puts rustc on the user's machine ⇒ C *is* D** |
| a single weak/undefined entry symbol | handles one mod; does not generalise to N |
| **a linker-collected section** | the only route that keeps C distinct — and it is the hazard class this tree has twice refused |

The refusals are real and must be answered rather than ignored: `REFLECTION-ANALYSIS.md:307-311`
chooses lazy first-call registration through the existing `component_id()` `OnceLock` **explicitly to
"sidestep the entire `linkme` + `--gc-sections` dead-strip / init-order hazard class"** (restated at
`:742`), and `crates/boyko_log/src/target.rs:978-982` refuses a constructor-registration mechanism in
the same terms — "inventing one — a linker section, an `inventory`-style registry — would add a
runtime facility the engine does not otherwise have". There is **no** `linkme` / `inventory` /
`link_section` registry anywhere in the tree.

**What changes for C specifically, and it was measured (M-6 / M-7, research §13).** The refusals were
taken for a `bin`+rlib world where the alternative (`OnceLock` first-call) exists; here the artifact
is produced by a *relink the installer controls*, which is a genuinely different situation, and the
measurement says so:

- A PE grouped-section registry (`.boykom$a` sentinel · `.boykom$m` entries · `.boykom$z` sentinel)
  **works** on windows-gnu: 0 / 1 / 2 mod objects on the link line yielded a collected table of
  0 / 1 / 2 entries, walked by the host at run time.
- **With rustc's default `-Wl,--gc-sections` on the link line it silently collects NOTHING** —
  `mods=0`, the link succeeds, the binary runs, the mod does not exist. That is the refused hazard
  class, reproduced exactly, with the worst possible failure shape.
- **It is rescued without dropping `--gc-sections`**: the installer passes
  `-Wl,--undefined=<the mod's registration symbol>` per mod, rooting the registration *static* (not
  the entry function — rooting the function was measured **insufficient**, `mods=0`). The symbol name
  comes from the mod's manifest, so the installer already knows it.

**Therefore the convention is: one `#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]`
registration static per mod, with a conventional name, and the installer roots each one explicitly.**
That also makes the dead-strip hazard **decidable instead of silent**: the installer knows how many
roots it passed, and the host asserts at boot that the collected count equals the manifest count.

**The second kernel need, new in rev 3 and re-scoped in rev 4: a mod-facing export set that survives
the engine's own LTO — expressible for functions, NOT for the statics a mod's inline bodies touch.**
Under 3b the engine's objects are fat-LTO'd *before* any mod object exists, and LTO keeps only what
the linker declared must be preserved ([LLVM LTO](https://llvm.org/docs/LinkTimeOptimization.html)).
The configuration mechanism is a dependency edge, not a feature: the export set is a set of
`#[unsafe(no_mangle)] pub extern "C"` wrappers in `boyko_mod_registry` — the crate that already holds
the `.boykom` sentinels — which only the modding game binary depends on, so the non-modding build's
closure has no such crate and no such symbol (`cargo tree` proves it; §7.10 item 4). **M-10 measured
what that set can and cannot do:** a `#[no_mangle]` function keeps global binding through fat LTO
(leg 3), so *functions* can be pinned; the per-type `component_id()` cell, the dispensers' counters
and `std`'s internals cannot — they are function-local or foreign statics that stay local (leg 2), and
the mod's inline-instantiated bodies reference them by name (leg 4). A mod that never instantiates an
engine `#[inline]` body or a `std` generic does not exist, so under 3b the export set is necessary
and not sufficient. Under route 2 it is unnecessary (the mod objects are present at LTO time); under
3a it is unnecessary (nothing was internalised). **The item therefore survives only as B's M-B1 cost,
and only if C were built as 3b — which rev 4 no longer proposes.**

### Toolchain and distribution cost

**This is where C pays, and it is the one thing that could disqualify it.**

- **Relink time is unmeasured for this engine — and rev 3 changes which number it is.** Under the
  shipped route 3b the install step is a **plain link**, so the relevant reference class is the
  non-LTO one: Chromium 145 debug (4.51 GiB) lld **16.64 s**, release (0.58 GiB) lld **6.48 s**
  (Threadripper 7980X, [mold README](https://github.com/rui314/mold)) — seconds, for binaries three
  orders of magnitude larger than this engine's 3.46 MB. The fat-LTO figure this bullet used to quote
  — clean builds 148 s (no LTO) → 233–324 s (fat), "2–3× for fat"
  (`docs/RUST-ERGONOMICS.md:649-667`) — describes **the engine vendor's own build**, which the user
  never runs, or route 2's link, which C does not ship. So the plausible install-time risk is much
  smaller than rev 2 implied, and it is still unmeasured: **no link-only number exists for this
  engine at all**, which is M-C1. (ThinLTO with a cache reaching "incremental link-time very close to
  a non-LTO build" — [LLVM blog](https://blog.llvm.org/2016/06/thinlto-scalable-and-incremental-lto.html),
  qualitative, 2016, clang — is a route-2 mitigation, kept in M-C1 leg (c) only for the case where
  M-C4 revives that route.)
- **Is a rustc-free relink even possible on this target? YES — measured (M-5, research §13).**
  Rev 1 asserted "the installer links the shipped engine objects with the mod objects" without
  establishing that anything but rustc can assemble that link line. It can:
  `rustc --print link-args` prints the **complete** line on **stable** 1.98.1, and replaying it with
  the toolchain's own bundled `self-contained/x86_64-w64-mingw32-gcc.exe` — **no rustc in the
  process** — produced a working binary with an extra mod object linked in. One caveat, and it is the
  part a naive attempt gets wrong: the line names a `symbols.o` that rustc generates into a temp
  directory at link time, so the shipped artifact set must include it (`-C save-temps` preserves it).
  A second caveat: with `-C link-self-contained=y` the line uses the toolchain's own `crt2.o` and
  `-L …/lib/self-contained`; **without** it, rustc's default here is the *external* mingw gcc on
  `PATH`, whose sysroot the end user would also have to have. The self-contained form is the one to
  ship.
- **Shipping the linker — and the numbers invert rev 1's recommendation.** Measured on this box:
  the toolchain's `bin/self-contained/` set (`x86_64-w64-mingw32-gcc.exe` 2.84 MB, `ld.exe` 1.91 MB,
  `dlltool.exe` 1.33 MB, `libwinpthread-1.dll` 54 KB) is **6.0 MB**; `lib/self-contained/`
  (`crt2.o` + 44 mingw import/static libs) is **26.8 MB**; the 20 std rlibs the link line names total
  **15.1 MB**. **≈48 MB total**, before any engine objects — **and that is route 3's number.** Route 2 is the
  only route that needs a *plugin-capable* linker, i.e. `rust-lld`, so route 2's disk row is a
  different row. Against that, **`rust-lld.exe` alone is 211 MB** — so rev 1's "on Windows the relink must use `rust-lld`/`lld-link`" is **wrong on this
  target in both directions**: it is not required (the bundled mingw driver works, measured) and it
  is ~4× the size of the whole alternative. `rust-lld` / `lld-link` are MIT/Apache-2.0 with Rust;
  the GNU binutils/gcc binaries are GPLv3-family (**still unverified against the bundled licence
  text — a legal review item, not a technical one**), and the bundled driver ships its own
  `GCC-WARNING.txt` stating it "cannot be used for compiling C files - it is only used as a linker",
  which is precisely the use here. **The MSVC linker is not on Microsoft's VS 2022 Distributable
  list** — that list is runtime files and merge modules, not `link.exe`
  ([MS](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)).
- **Shipping engine objects.** For route 3b: the engine's **post-LTO machine-code objects** plus
  `symbols.o`, plus the mod SDK. Size is unmeasured; the LTO'd binary is 3.46 MB, and the object set
  is larger than the linked image (M-C2). **⚠ The bitcode-disclosure item is route-2-only, and rev 2
  attached it to C as a whole.** `-Clinker-plugin-lto` would ship LLVM IR of the *whole engine* to
  **every player** — a far wider exposure than the rlib SDK option A hands to registered mod authors,
  and wider than the "IP exposure" §4 flags for D. Under route 3b what ships is object code, whose
  exposure is that of any shipped binary. So this cost belongs to route 2's column, not to C's, and
  it is one more reason the route must be named before the owner is asked to weigh C's risks.
- **PGO:** possible in principle (`-Cprofile-use`, "All `rustc` flags have to be the same",
  [PGO docs](https://doc.rust-lang.org/rustc/profile-guided-optimization.html)) but the shipped
  profile was collected without the mod's code.
- **Version lock is still absolute** (E0514) — same as A. C does not buy patch-release survival.
- **No precedent found.** Searched; **no shipping game relinks its binary with mod objects at
  install.** That is a risk (unknown unknowns) and an opportunity, and it must be stated as such.

### Security model

**None, and worse in one respect:** the mod's code is linked into the game binary, so code signing
is invalidated at install. Windows Smart App Control admits only RSA-signed binaries from trusted
providers ([MS](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control));
a locally relinked binary is unsigned. **This is C's specific, non-obvious cost and it may matter
more than the link time.**

### Failure modes

1. **Install time unacceptable** (M-C1 decides).
2. **A mod that does not compile/link breaks the game**, not just the mod — no graceful degradation
   without a fallback binary.
3. **Unsigned binary** after relink (above).
4. **Antivirus/EDR reaction** to a program relinking an executable on the user's machine —
   unmeasured, plausibly severe.
5. **Diagnosing a link failure on a user's machine** is a support problem with no precedent to copy.
6. **No unload, no reload** — by construction. Changing the mod set means relinking.
7. **A mod that links but is silently absent.** Measured (M-6): with `--gc-sections` on the line and
   no explicit root, the registration section is dead-stripped, the link succeeds, the game runs, and
   the mod does nothing. This is the refused hazard class and the reason the boot-time
   "collected count == manifest count" assertion is not optional.

### Cost when modding is NOT used

**Exactly zero, and it is the cleanest zero of all five options** — because the non-modding build is
*the same build process the engine uses today*, with no mod objects on the link line and the
`boyko_mod_registry` sentinel crate absent from the dependency closure. There is no loader crate, no
descriptor path, no handshake, no canary, no exports, no feature. The kernel changes C needs are id
budgeting (§7.3 — **no constant at all** as of rev 4: mods mint from the free middle of the fixed 512
through the existing CAS dispenser and the loader enforces a quota, so a non-modding build reserves
nothing and pays no capacity) and the plugin-registry sentinels, which live in a crate the
non-modding build does not depend on, exactly as §1's separate-crate argument requires (§7.10).

### Cost with modding compiled in, no mod loaded

**The concept nearly does not apply**, which is itself the point: there is no "compiled in but no mod
loaded" state in the loader sense. A game with no mods installed is relinked with zero mod objects.
**C is the only option where P2 collapses into P1 — with one residue rev 1 did not have to account
for.** The plugin registry of §3 is a *mechanism in the engine binary*: two sentinel statics in a
`.boykom` section plus the boot-time walk between them and the collected-count assertion. A
zero-mod binary therefore carries 16 bytes of data and a loop that runs zero iterations, and — if
the chosen rescue were "drop `--gc-sections`" rather than the per-mod `--undefined` root — it would
also carry whatever that flag was removing. The per-mod root (M-7) is preferred precisely because it
keeps `--gc-sections` and confines the residue to those 16 bytes. **M-C0b measures whether that
residue is truly nothing on the real workspace**; until it does, "collapses into P1" is a prediction
for C, not a proof, and it is P1 in the P2 sense — the *non-modding* build (no registry at all)
remains exactly today's build.

**⚠ Rev 3 adds a second residue, and it is larger than the first.** Under route 3b the modding-built
binary also carries the **mod-facing export set** (§3 "Kernel features it needs"): entry points
pinned so the engine's own fat LTO cannot internalise them. That is precisely option B's structural
cost — LLVM's `internalize`/`globaldce` work on *internal* functions — arriving in C, and it is
present whether or not a mod is loaded. Two things keep it from touching the owner's hard constraint:
it exists only in the modding configuration, and its size is bounded by the SDK's entry-point list
rather than by the ECS surface. **It is not zero, it is unmeasured, and M-C4 leg (b) measures it**
with M-B1's method. The *non-modding* build is untouched by all of this. **Rev 4: this residue exists
only under 3b, which M-10 finds red at toy scale; under route 2 or 3a there is no export set, and C's
P2 residue returns to the 16-byte sentinel section alone.**

---

## 4. Option D — source mods compiled on the user's machine

*Mods ship as Rust source (or Aether); the user's machine builds them, then the game is rebuilt or
relinked.*

### Mechanism

The degenerate case of C, with the compiler moved to the user. Godot's static-module model
("modules provide deeper integration… changing a module means recompiling the engine") is the
precedent; this tree's own Aether is already a proc-macro DSL emitting
`#[derive(Component)]` structs and `pub fn` systems (`crates/aether/src/lib.rs`), so
Aether-authored mods **already require rustc** — D is the option that admits it.

### What "as fast as native" means here

Identical to C: one binary, no boundary, full LTO and PGO. Additionally, mod source can be typed
against the engine's *current* API rather than a frozen ABI, so the API-cruft failure mode of B
disappears entirely.

### Kernel features it needs

The same minimal set as C. Arguably fewer, because mod source can be required to use only the public
API, and the compiler enforces it.

### Toolchain and distribution cost

**The highest.** Shipping a full compiler: Arch's `rust 1:1.98.1` is 78.4 MB packaged /
**302.5 MB installed**, excluding LLVM ([Arch](https://archlinux.org/packages/extra/x86_64/rust/)).
The Windows rustup footprint is **not established**. Compile time is the full mod crate plus the
relink — minutes, not seconds. And the engine's *source or rlibs plus headers* must ship, which is
a licensing and IP decision, not a technical one.

Against that: it is the only option where a mod can be distributed as a small text artifact, and the
only one immune to ABI drift by construction.

### Security model

**Weakest of all.** Arbitrary code compiled and run, with a compiler on the user's machine as an
additional attack surface. No sandbox. (Factorio's Lua and Roblox's Luau sandbox precisely because
the alternative at the source level is unbounded.)

### Failure modes

All of C's, plus: compile errors surfaced to end users; build-environment variance on user machines;
a much larger install; IP exposure of engine sources or a large SDK.

### Cost when modding is NOT used / compiled in, no mod loaded

**Exactly zero, and P2 collapses into P1, exactly as in C.** A game with no mods is the ordinary
build.

---

## 5. Option E — WebAssembly, as the sandboxed contrast

*Included to price the trade, not as a serious candidate for the owner's stated goal.*

### Mechanism

Mods are Wasm modules, AOT-compiled at install (Microsoft Flight Simulator 2024's shape: add-ons
moved from DLLs to Wasm "in order to provide security and portability", modules "converted to native
code ahead of time (as DLLs)", "not interpreted", each single-threaded, no Windows API, no C++
exceptions). Zed and Veloren both ship Wasmtime plugin systems.

### What "as fast as native" means here — it does not

| | |
|---|---|
| Geomean vs native, libsodium, 2026 | Wasmtime 46.0 **2.41×** (1.46× with `wide_arithmetic`); Wasmer 7.1 1.33×; WAMR 1.57×; WasmEdge AOT 1.74× ([00f.net](https://00f.net/2026/06/23/webassembly-runtimes-2026/)) |
| SPEC CPU via browsers | +45 % Firefox / +55 % Chrome, peaks 2.08× / 2.5× (Jangda et al., ATC '19) |
| Best case, sandbox compiled into the host (wasm2c/RLBox) | **17 %** over native+SIMD (Firefox SoundTouch) — and it recovers PGO and cross-boundary inlining |
| Guest→host call | ≥10 ns (Bytecode Alliance); criterion: wasm→host typed nop 7.2–7.8 ns, host→wasm 27.7–33.8 ns |
| Bounds checks without a 4 GiB reservation + guard | 1.2×–1.8× |

**The structural disqualifier is not the percentage — it is SIMD.** Wasm SIMD is fixed **128-bit**
(phase 4); ≥256-bit "flexible vectors" is **phase 1**
([proposal](https://github.com/WebAssembly/flexible-vectors/blob/main/proposals/flexible-vectors/Overview.md)).
This engine's baseline is AVX2 (`-C target-cpu=x86-64-v3`, `.cargo/config.toml:85`). A Wasm mod
**cannot use the ISA the engine is built around**, and a guest→host call at ≥10 ns against a
3.4–4.0 ns/row budget forbids any per-row interaction.

Further: guest memory is a separate linear memory (host-backed memory is possible via `MemoryCreator`
but "any modification of them outside of wasmtime invoked routines is unsafe"), and
`threads`/shared memory is **tier 2** and unsupported by the pooling allocator — so a Wasm mod cannot
participate in the engine's work-stealing parallelism at all.

### Kernel features it needs

A completely separate seam: a host-call API, a data-copy or shared-memory protocol for component
columns, and an id translation layer. **None of it shares mechanism with A–D**, which is why E is a
fork rather than a stage.

### Toolchain, distribution, security

Cheapest distribution (a portable artifact, no toolchain pinning, works across engine versions with
a versioned host API) and **the only option with a real security model**: memory isolation,
capability-scoped host calls, epoch interruption for runaway scripts (fuel metering is ≈2× worse).
Minimal Wasmtime was 2.1 MB in 2023.

### Failure modes

Performance below the engine's own floor for any compute-shaped mod; no AVX2; no shared parallelism;
an entirely separate API surface to maintain.

### Cost when modding is NOT used / compiled in, no mod loaded

**Zero when the runtime crate is absent** — same mechanism as A/B. With the runtime compiled in and
no mod loaded, the cost is the runtime's own presence in the binary (2.1 MB class) plus its
initialisation; per-frame zero.

### 5.1 Option F — out-of-process native mods (raised in pass 3; NOT priced)

*A mod is a native Rust binary in a sandboxed child process (job object / AppContainer), crossing at
system granularity through a shared mapping. Rev 1–3 did not consider it; it is written down here
because it is E's competitor on the security axis without E's 128-bit SIMD disqualifier, and the
honest status is "not excluded on a measurement — not considered".*

- **What it keeps:** AVX2 (`-C target-cpu=x86-64-v3`), the engine's own build on the host side, and
  every §0.3 instance is moot — there is no shared image, so there are no duplicated statics, no
  `TypeId` question, no allocator or TLS split. The mod process is built like option A's mod (same
  toolchain, canary), but nothing of it is ever mapped into the host.
- **What it costs, none of it measured here:** (i) the column data a mod system reads must live in a
  **shared mapping** — `ComponentPool` storage is `VmReservation` over `VirtualAlloc` (`memory/vm.rs`;
  the crate manifest's own comment), not a file mapping — so either the pools a mod touches are
  re-backed by `CreateFileMapping` / `MapViewOfFile` (a storage-layer change, with the
  address-stability and `row_ptr`-provenance questions a second mapping brings), or the host copies
  the columns out and back per system run (a per-row copy: the frequency class §0.1 calls
  catastrophic); (ii) one cross-process hand-off per mod system per frame — an event/futex round trip
  whose cost on Windows 11 is unmeasured here and plausibly in the microseconds, i.e. ~10³ rows of
  budget per crossing; (iii) the sandbox protects the host's *code*, not its *data*: a wild write
  through the mapped view corrupts host columns as surely as an in-process one, so the security gain
  is bounded by what the mapping excludes; (iv) no `par_iter` participation and no engine `Query`
  types unless the columns are mirrored, plus a second `std` and allocator that are *meant* to be
  separate.
- **Where it belongs:** a fork beside E, not a stage — it shares no mechanism with A–D. If Stage 0
  answers "untrusted mods must run", F and E are the two candidates for the untrusted tier, and the
  measurement that decides between them is the cost of (i)+(ii) against E's 1.33×–2.41×. Nothing
  here needs a kernel change until that answer exists.

---

## 6. Comparison

| Axis | A: exact-build dylib | B: stable C-like ABI | C: install-time relink | D: source mods | E: Wasm |
|---|---|---|---|---|---|
| **Mod inner loop vs native** | ~1.00× (loop never crosses; generics instantiated in the mod — M-4) | ~1.00× if it links rlibs too, else no generics at all | **~1.00× — one binary, no boundary.** Route 2: co-optimised with the engine (LTO at install); 3a: engine loses fat LTO; **3b: red at toy scale (M-10)** | **1.00× exactly** | 1.33×–2.41× geomean; 1.45–1.55× SPEC |
| **Data-representation tax** *(measured elsewhere, NOT for an ECS column loop)* | none | **~30 %** — Tremor, whole-pipeline, `halfbrown`→`RHashMap` behind `#[repr(C)]`, **plugins still statically linked**. A hash-map-shaped hot path, not a row loop; this engine's own crossing shape is **unmeasured (M-B2)** | none | none | copy or shared-memory protocol per column |
| **Boundary cost per crossing** | ~0.3–0.7 ns over a static call (M-4) | 3.23 ns (abi_stable object); 7–32 ns (GDExtension-class, cached) | none | none | ≥10 ns guest→host |
| **Engine keeps `lto = "fat"`** | **yes** | yes | **route 2: yes, and it covers mod code too** (needs `rust-lld`, 211 MB, and ships whole-engine bitcode); **3a: no** (forfeited as under A′); 3b was "yes, for engine code" — **red at toy scale, M-10** | yes | yes |
| **SIMD available to the mod** | AVX2 / AVX-512 | AVX2 (hazardous: no cross-binary check) | AVX2 / AVX-512 | AVX2 / AVX-512 | **128-bit only** |
| **Mod can add component types** | yes, via descriptor | yes, via descriptor | yes, as ordinary Rust types | yes | yes, via host API |
| **Mod systems in the schedule** | yes — through the host-side fn-pointer seam (§7.9(c), rev 6): the existing vtable call plus one fn-pointer hop per system run, `SystemBox` host-allocated. Not a new cost class; **not** "no surcharge" — one indirect call per mod system per frame, and one kernel seam (§7.10 item 6) | yes (the same seam shape — the host table *is* the fn pointer) | yes (ordinary systems) | yes | via host trampoline |
| **Mod can use `par_iter`** | **hazard: silently serial** unless TLS is installed | no | **yes, natively** | yes | no (threads tier 2) |
| **Survives an engine patch release** | **no** (exact-build lock) | **yes** (the option's point) | no | yes (recompiled) | yes |
| **Unload / reload** | no | no | no | no | yes in principle |
| **Install-time work** | none | none | **seconds to minutes — UNMEASURED, decisive** | minutes + compiler | AOT compile once |
| **Ship on the user's disk** | mod DLLs + SDK for authors | mod DLLs | **≈48 MB linker set measured** for the shipped route 3b (bundled mingw driver 6.0 MB + mingw libs 26.8 MB + 20 std rlibs 15.1 MB) + engine objects (unmeasured, M-C2). *Not* `rust-lld` — 211 MB, and needed only by route 2 | **302 MB class compiler** (Arch figure); full windows-gnu toolchain on this box **measured 2772 MB** | 2.1 MB class runtime |
| **Code signing survives install** | yes | yes | **no** | no | yes |
| **Sandbox** | no | no | no | no | **yes** |
| **Cost when modding OFF** | **zero** (separate crate) | **zero** (separate crate) | **zero** (same build as today) | **zero** | **zero** |
| **Cost compiled in, no mod loaded** | startup scan only; per-frame zero | startup scan + **exported symbols excluded from LTO internalisation** | **collapses into OFF**, minus a 16 B sentinel section + a zero-iteration boot walk (M-C0b). (The 3b export-set residue went with 3b — rev 4) | same as C | runtime in the binary + init |
| **Kernel work required** | descriptor path, canary, TLS handshake (all three slots), id budget, **+ the §0.3 façade: a by-stable-name query/resource mint entry point covering ENGINE component types too, and the mint-counter poison that lets the gate fail for the right reason, + the host-side system-registration seam — fn pointer + access descriptor, `SystemBox` allocated in the host image (§7.9(c), rev 6)** | all of A + FFI-safe mirror of the ECS surface | id budget (loader quota, no constant — §7.3 / §7.10) + **a linker-collected plugin registry** (measured feasible; needs a per-mod `--undefined` root, plus a boot-time collected-count assertion). The 3b export set is withdrawn with 3b (M-10) | id budget + the same plugin registry | separate host API |
| **Precedent** | Bevy (removed), Fyrox (ships, "wildly unsafe"), SKSE, Unreal BuildId | Godot GDExtension, The Machinery, HL/Source, abi_stable | **none found** | Godot static modules | MSFS 2024, Zed, Veloren |

### 6.1 Option A′ against A and C, on the axes where it differs

A′ (§1.5) is kept out of the wide table because it differs from A on only five rows, and the
comparison that matters is *correctness against speed*, not a fifteen-row sweep.

| Axis | A (cdylib + rlibs) | **A′ (engine dylib, modding config)** | C (relink) |
|---|---|---|---|
| §0.3 duplicated statics | **present; fenced by the façade + the mint-counter poison (leg (d)); malice remains a trust-model question** | **absent by construction** (M-2: same static addresses) | absent under route 2 / 3a (one binary, one LTO or none). Under 3b, **M-10 measured the re-added-rlib line failing loudly on the allocator shim before any local+global pair could form** — and 3b is withdrawn |
| Mod may call `world.query::<D,F>()` | **no — corrupts** (§0.3 instance 1) | **yes, correct** | yes |
| `par_iter` from a mod | hazard: silently serial | **yes** | yes |
| Modded-session throughput *(no session-level number exists for any column — these are the 7-bench suite)* | 1.00× inside each side | **1.18×–2.67× across the suite, geomean 1.26×** (fat LTO forfeited; `query_ref_iter` is the worst and is the shape a mod system's loop most often has) | **route 2: 1.00× and co-optimised with mod code; 3a: A′'s 1.18×–2.67× without the dylib boundary** (3b withdrawn, M-10) |
| Non-modding build | today's build | **intended to be today's build — but the `crate-type` mechanism is unestablished and rust#51009 can drop fat LTO with no diagnostic (§1.5); gated by M-F1 leg (c), not waived** | today's build |
| Install-time work | none | none | **seconds to minutes, UNMEASURED** |
| Ships `std`/`core`/`alloc` DLLs | no | **yes** (M-2) | no |
| PE 65535-export ceiling | not reached | **live question, unmeasured** | not reached |

---

## 7. What the kernel must provide regardless of the option

These are needed by **every** option that lets a mod add a component type or a system, and several
are wanted by the unified-kernel plan independently. They should be specified once, in the kernel,
not per option.

1. **A public, data-driven component descriptor path.** `try_register_dynamic(ComponentLayout)` at
   `…/component_registry/mod.rs:967` already exists; `register_asset_layout<T>(drop_fn)`
   (`asset/backing.rs:115`) already proves the caller-supplied-drop-glue shape works. It must be
   promoted to `pub` — a **Rust-ABI visibility change inside `boyko_ecs`**, never an `extern "C"` or
   `#[no_mangle]` item there (the foreign-callable entry is `boyko_mod_host`'s; §7.10 item 1) — and
   given a descriptor carrying what today only flows through the derive:
   `clone_fn`, `map_entities_fn`, `serialize_fn`, `storage_kind`, `residency_class`, `stable_name`,
   `layout_fingerprint`, `format_version`.
   **⚠ The descriptor is an INPUT struct; it is not a widened `ComponentLayout`.** Every one of those
   eight fields already lives in a **parallel cold table** — `CLONE`/`MAP_ENTITIES`
   (`clone.rs:21,82`), `SERIALIZE` (`serialize.rs:16,35,122`), `STORAGE_KIND` (`mod.rs:363`),
   `RESIDENCY_CLASS` (`:503`), `HOOKS` (`:221`) — and each site's own comment says the table is
   parallel *precisely* so that `ComponentLayout` stays at 56 B / one cache line
   (`const _: () = assert!(size_of::<ComponentLayout>() == 56)`, `mod.rs:133`, TRIPWIRE 2).
   `ComponentLayout` is hot: its doc at `:104-121` records that `size` / `alignment` / `drop_fn` are
   read "on every memcpy … swap_remove/pop/set_component/Drop". Widening it would push that load onto
   a second cache line **in every build, modding or not** — the exact leak §1's warning exists to
   prevent. The descriptor therefore *fans out* into the existing tables and leaves `ComponentLayout`
   untouched. **TRIPWIRE 2 staying green is a Stage 1 gate item.**
   **Registration must validate `(size, align, fingerprint)`** — flecs aborts with
   `ECS_INVALID_COMPONENT_SIZE` on a mismatch, and a design that checks only names corrupts memory
   silently. This is a cold path; it adds no branch to any hot one.
   **The kernel delta, enumerated (rev 4):** `pub` on `try_register_dynamic` (`mod.rs:967`),
   `set_storage_kind` (`:435`), `set_residency_class` (`:614`) and `install_map_entities_fn`
   (`clone.rs:181`) — all `pub(crate)` today — plus by-value forms of `install_clone_fn` /
   `install_serialize_fn` (`clone.rs:161`, `serialize.rs:204`), which are generic over `C: Component`
   and cannot take a descriptor (`install_bind_accessor`, `serialize.rs:308`, is already the
   non-generic shape). Two cold functions and four visibility lines; the descriptor struct, its
   validation and the `extern "C"` entry are `boyko_mod_host`'s.
   **The relationship between the by-value forms and the existing generic installers is fixed
   (rev 5; pass 4, O2).** The generic `install_clone_fn::<C>` / `install_serialize_fn::<C>` are
   called by the derive from every component-defining crate (`boyko_macros/src/component.rs:387-392`)
   and are the only installers a non-modding build reaches. Whether the by-value forms stay separate
   cold functions (unreachable in that build, absent at `lto = "fat"`) or become the bodies the
   generic ones call after building their `CloneInfo` / serialize record — the obvious
   de-duplication — is an implementation choice with **no P1 consequence either way**: in the second
   form the by-value body is inlined into the same once-per-type `OnceLock::set` path the derive
   already runs, and no hot function changes. What changes is §7.10 item 1's *argument*, from
   "unreachable, therefore absent" to "reachable only from a cold path already present". The gate is
   therefore M-P1's pins against the pre-change tree — `register_new::<T>` and `T::component_id()`
   for a representative `T`, `.text` size — never the reachability claim, and item 1's cell says so.
   **Rev 6 corrects what "M-P1's asm diff" meant here:** rev 5's M-P1 diffed the modding and
   non-modding legs of the post-change tree, and both forms of item 1 are present in both legs, so
   that diff could not fail on either; R1's two-tree comparison can (pass 5, C1).
2. **Name-keyed identity as the durable key, everywhere.** Already present and already the shipped
   design (`stable_name` + `LAYOUT_FINGERPRINT` + `FORMAT_VERSION` + `STABLE_NAME_INDEX`). The
   remaining change is to make `stable_name` **mandatory for externally-supplied types** rather than
   defaulting to `type_name` (documented as not a unique identifier). Two name-keyed dynamic
   registries already exist with mods named as the motivating case:
   `boyko_log::target` (`crates/boyko_log/src/target.rs:759-763`) and `boyko_diag`'s dynamic zone
   registry (`crates/boyko_diag/src/profiling_abi/dyn_registry.rs:1-45`).
3. **An id budget policy, decided before anything is built.** 512 components shared with dynamic
   tags; **256 resources with ~145 already used**; 256 events; 1024 archetypes; 1024 systems per
   schedule; exhaustion **panics**. The options are (a) a reserved high band for mods — the shape
   `boyko_physics/src/scratch_ids.rs` already uses for scratch ids; (b) raise the caps for everyone,
   which costs `[Column; MAX_COMPONENTS]` per archetype and breaks
   `const _: () = assert!(size_of::<Access>() == 192)` (`system/access.rs:61`); (c) a hard per-mod
   quota with a diagnosable error instead of a panic.
   **⚠ Rev 1 claimed "(a) + (c) is the only combination that does not charge non-modding games". That
   is true of TIME and false of CAPACITY, and the correction matters.** `try_register_dynamic` CASes
   **the same `static NEXT_ID`** as `register_new` (`component_registry/mod.rs:968` against `:214`),
   so a reserved band is a partition of one fixed budget, not an extension of it. The occupancy is
   already tight from both ends: ~126 of 512 components in use, **128 slots already reserved at the
   top** by the physics scratch band (`scratch_ids.rs:685`, `MAX_COMPONENTS - 128`, whose own doc
   prices the margin against a measured ~142 production types), and **~145 of 256 resources**, of
   which `boyko_render` alone is ~89. A modding band therefore reduces a non-modding game's headroom
   by exactly its size, and the failure it can cause is a **release panic** the game would not have
   hit without modding support (`register_new`'s assert; `QUERY_NEXT_ID` saturate-then-panic,
   `query_type_registry.rs:133-135`).
   **Rev 3 concluded "(a) must be a per-configuration constant"; rev 4 withdrew that, because the
   only in-tree mechanism for a per-configuration kernel constant is a cargo feature
   (`MAX_QUERY_TYPES` under `#[cfg(feature = "big_query_table")]`, `query_type_registry.rs:84-90`) —
   the mechanism §8.3 forbids, and one that feature unification would switch on for a game that never
   asked (§1). That withdrawal stands: the band is the LOADER's, not the kernel's.** `MAX_COMPONENTS`
   stays `512` in every configuration (`mod.rs:63`, unconditional), `register_new`'s cap is
   untouched, nothing is reserved in a build that loads no mod, and mod components mint through the
   *existing* `try_register_dynamic` CAS (`:967`) from the shared budget in mint order, as dynamic
   tags and asset layouts do today. **Capacity for a non-modding game is unchanged by construction**:
   ids are consumed only when a mod is loaded, and the release panic rev 3 feared cannot be reached
   by a build that loads no mod.
   **What rev 5 withdraws is rev 4's BOUND (pass 4, C2).** Rev 4 had the loader refuse a load when
   `next_id + quota` would cross `SCRATCH_REGION_MIN_ID`. That check reads `NEXT_ID` at the instant
   the loader runs, and the loader runs in the config phase before `App::finish()`
   (`crates/boyko_ecs/src/ecs/core/app/app.rs:550`, `:583`; §7.5) — while engine component ids are
   still minting **lazily, on first touch** (`component_registry/mod.rs:911-914`; the counter's own
   doc, `:212-214`). At the load slot the counter holds only what plugin `build()`s have touched;
   the schedule build inside `finish()` (every `SystemParam::init_state` resolving every queried
   type) and the first frame's spawns mint the rest *afterwards*, on top of whatever the mods took.
   §0.3 leg (d)'s closing note states exactly this drift to kill the "mirror the host's tables"
   idea, and rev 4 did not apply it two sections later. A quota that passes against the
   instantaneous counter therefore bounds nothing: the production counter can climb into the band
   the check was meant to protect. **And that band is not spare capacity.**
   `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128` (`boyko_physics/src/scratch_ids.rs:685`) is sized
   from a census — *"the production side gets 384 against a measured ~142 … the census above is the
   thing to re-run before moving this number again"* (`:671-684`) — and its slots are pinned by
   `register_layout` without touching `NEXT_ID` (`mod.rs:1031-1056`), from the `ScratchColumn`
   owners' constructors (`scratch_ids.rs:31-38`), which `PhysicsPlugin::build` runs in the config
   phase (`boyko_physics/src/plugin.rs:465-471`: `SolverScratch::with_capacity` →
   `register_scratch_layouts`, `resources.rs:3779`; `BroadphaseGrid::with_capacity` →
   `register_broadphase_column_layouts`, `:569`). Every id a mod takes from the middle is one fewer
   of the 384 the physics margin was priced against — **a loader bound spends the physics census,
   and must cite it as the constraint it spends.** The failure at the meet is not the budgeted
   `"ComponentRegistry exhausted"` assert; it is a **collision** panic, and which one depends on
   plugin order (`PhysicsPlugin` is not inside `EnginePlugins`, `boyko_app/src/plugins.rs:379-547`,
   so the game decides): if physics has already pinned its slots when the production counter reaches
   one, the next engine mint dies in `register_new`'s collision arm — *"ComponentId N occupied by
   type BodyState, refused to register <EngineType>"* (`mod.rs:938-944`) — inside whichever
   `component_id()` first touched that type, plausibly mid-session; a mod's descriptor mint landing
   there dies in `dynamic_slot_occupied_panic` (`:996-1008`), whose text blames a *"test-only
   `register_layout`"* — a lying diagnostic; and if the mod loader ran **before** physics, the mod's
   mint lands on a still-unpinned band slot silently and physics dies later in a solver constructor —
   *"ComponentId 384 occupied by type ModFoo, refused to register BodyState"* (`:1046-1051`) — a
   modded session crashing in release inside physics, with a diagnostic naming physics. None of
   these texts was in rev 4's limit (v) list, which is how C1 and C2 interlock: inside a mod image
   the payload would have been swallowed into a status.
   **What is decided regardless, and what is open.** Decided: no constant, no reservation, no
   capacity charge to a build that loads no mod; the bound is the loader's; the physics census is a
   constraint the bound spends and cites; and the operator-visible failure at the bound is a
   loader-owned `Result` **before** any mint, never a collision panic **after** one. Open — **the
   architect's**, on three traced directions:
   **D1 — make the host's ids complete before the loader runs** (an eager mint pass). Not available
   in this tree as such: there is no enumeration of `#[derive(Component)]` types to walk (no
   `linkme` / `inventory` / `link_section` registry anywhere — refused at
   `REFLECTION-ANALYSIS.md:307-311` and `boyko_log/src/target.rs:978-982`), `STABLE_NAME_INDEX` is
   populated *by* the mint (`register_stable_name`, `serialize.rs:383`), and the schedule build that
   mints most ids is the thing the loader must precede (§7.5). Its only form is a hand-maintained
   list — Bevy's `register_type` shape, which this tree declined — and it would also revive the
   "mirror the host's tables" option leg (d) discarded.
   **D2 — bound the mod band from a known engine ceiling.** A `const ENGINE_COMPONENT_CEILING` in
   `boyko_mod_host` — a dependency-edge-scoped value, admitted by §7.10's rule, never in `boyko_ecs`
   — held green by a census test of the `tests/ignore_reasons_census.rs` kind over the same
   `#[derive(Component)]` count `scratch_ids.rs:671-684` already documents (142 over the
   `boyko_demo` closure, an over-count) — **a source-site count equals the id count only while no
   component type is generic (rev 6; pass 5, O1)**: a `#[derive(Component)] struct Buf<T>` mints
   one id per monomorphisation through the derive's per-type `OnceLock` (`component.rs:371-375`).
   Zero generic component derives exist under `crates/` at `d552be05` (multiline grep for a derive
   followed by `struct X<`), so the count is exact today; the census test asserts that too, and a
   generic derive turns the census into a lower bound that D3's backstop must cover. The loader refuses when
   `ceiling + quota > SCRATCH_REGION_MIN_ID`, independent of when it runs. The number is maintained,
   not derived — the discipline the physics constant already imposes on itself — and a stale census
   degrades to D3's backstop, not to silence.
   **D3 — give the mod band a floor of its own.** Mods mint **downward** from
   `SCRATCH_REGION_MIN_ID - 1` through a pinned-slot form of the dynamic dispenser —
   `try_register_dynamic_at(layout, id)`: `LAYOUTS[id].set(layout)` without touching `NEXT_ID`, the
   physics band's own shape with a `ComponentLayout` in place of a `T`; one more cold `pub` Rust-ABI
   fn in the kernel (§7.10 item 3). The two counters then never race for one slot; they can still
   *meet* when `engine_count + mod_count > 384`, and the meet is caught by the existing
   `OnceLock::set` collision check in `register_new`'s arm, with a text that names the **mod** type
   — attributable and fail-fast, the same proof `scratch_ids.rs:1-25` relies on. Alone it is a loud
   failure rather than a bound; it is the backstop D2 degrades to. **If D3 is chosen, two in-tree
   contracts move with it (rev 6; pass 5, W3):** `register_layout`'s doc names its sanctioned
   callers as *"(1) tests … (2) synthetic scratch columns"* (`mod.rs:1012-1024`, `#[doc(hidden)]`
   at `:1030`), and `dynamic_slot_occupied_panic`'s text blames *"a slot … pinned out-of-band
   (test-only `register_layout` overlapping the production counter)"* (`:998-1010`). Under D3 the
   commonest cause of that panic is a mod's legitimate downward mint meeting the engine's upward
   counter, so the text and the doc contract are part of D3's change — limit (ii)'s
   lying-diagnostic class, otherwise re-created by the remedy. §7.10's `unsafe`-seam clause does
   **not** apply to `try_register_dynamic_at`: `LAYOUTS[id].set` is a `OnceLock` set, a collision
   is loud, not unsound, so it is correctly rowed as a safe `pub` fn; only the diagnostics move.
   D2 + D3 together is the only combination in which the counters cannot meet while the census is
   current and the failure is attributable when it is not; D1 is the only one that would make the
   quota exact, and it is not available. The choice — and whether a per-mod quota is the right unit
   at all, against one budget for the whole mod set — is the architect's; §11 carries it.
   **Two host consumers of the same counter sit outside every direction and must be priced with it
   (pass 4, question 4):** `EcsMaster::register_tag` / `register_enable_tag` (`tag_api.rs:65`,
   `enable_tag_api.rs:61`; `pub`, `#[cold]`, callable at any point in a session) and
   `Assets::<T>::with_reserved` (§0.3 instance 6; the host itself constructs one in its config phase,
   `boyko_app/src/runner.rs:258`) both mint from `NEXT_ID` through `try_register_dynamic`, so the
   host's occupancy is a moving target from its own side as well, and D2's census must count the
   named tags and asset types a game mints, not `#[derive(Component)]` sites alone.
   The occupancy reader §7.10 item 3 needs is unchanged — one cold `pub fn` **value** reader per
   dispenser, values and never addresses (pass 4, O1; §7.9(b)) — and **M-K3** is queued to measure
   the drift any bound has to absorb: `NEXT_ID` at the config-phase slot, after `finish()`, and
   after the first frame, on the shipped demo. The price, once a direction is chosen, is stated with
   its number as before: ~512 − 128 (physics band) − ~126 (in use) ≈ **258** ids for mods and future
   engine types together, and ~111 resources. Option (b) above — raise the caps for everyone —
   remains rejected on M-K1's prior expectation, and rev 4's mechanism argument stands: even a
   per-configuration *raise* would have to be a feature on `boyko_ecs`. **Some ids are outside any
   band and any mint:** the physics scratch constants are pinned by `register_layout` without
   touching `NEXT_ID` (`scratch_ids.rs:653-659`, `:713-718`; §0.3 leg (d) limit (i)), so a loader
   bound never sees them through the counter and a per-configuration `MAX_COMPONENTS` would have
   *moved* them — one more reason the constant stays fixed. M-K1 should report occupancy headroom
   alongside `query_ref_iter`.
   **⚠ There is a THIRD budget, and rev 2 priced only two.** Query *shapes* have their own fixed
   table: `MAX_QUERY_TYPES = 1024` (`query_type_registry.rs:84-85`), minted saturate-then-panic
   (`:132-145`) with a 75 % warning at `:149-160`. Mod queries consume from it — and under the §0.3
   façade the **host** mints them, so they land in the *host's* table, exactly like components and
   resources. Two things follow that §7.3 must state. First, it needs the same occupancy report as
   the other two budgets (M-K1). Second, **its only in-tree remedy is a cargo feature on
   `boyko_ecs`** — `big_query_table`, which raises the cap to 4096 (`:88-90`, and the panic text at
   `:141` tells the operator to enable it) — which is the mechanism §8.3 rules out for modding
   because of feature unification, *and* it multiplies the per-world `QueryStateCache` by four: its
   own doc records "≤ 32 KB at `MAX_QUERY_TYPES = 1024` … ≤ 128 KB with the `big_query_table`
   feature" (`ecs_master.rs:327-331`), i.e. a per-world structure that fits L1d today and does not
   at 4096. So the modding decision must not be allowed to arrive at that feature by default — and
   rev 4 removes the first of rev 3's two remedies, since a per-configuration cap IS that feature: the
   remedy is a per-mod query quota, enforced at the façade's mint seam under A and by the loader
   (before/after occupancy reads through the same cold `pub fn` readers as the component budget)
   under A′ and C.
4. **Non-fragmenting storage for mod-added per-entity data.** Already present:
   `STORAGE_IS_DENSE` (`component.rs:79`). Mods adding components to *engine* entities is the
   universal archetype-fragmentation tax; flecs' answer is `DontFragment` and Bevy's is
   `StorageType::SparseSet`. **This engine already has the better answer and should require it for
   mod components on engine entities.**
5. **A mod-load slot in the boot timeline, and nothing later.** The config phase before
   `App::finish()` (`app.rs:583`) already admits plugins; a `ModLoaderPlugin` added there needs
   **no kernel change at all**. Anything after `finish()` requires a schedule rebuild API that does
   not exist and should not be built for modding alone.
6. **A failure channel that is not a panic — on the mod-facing surface only.** Today a missing
   resource is a release panic at param fetch (`missing_resource_panic::<R>()`,
   `system/params/res.rs:132`, `#[cold]`), a mod panic is an engine panic, and under the windowed
   host it presents as a frozen window (owner-session record, 2026-08-26, unverified). Mod
   registration and mod load must return `Result`; a mod system's panic should at minimum be
   attributable to the mod. `boyko_diag` already partitions `Region::Engine` vs `Region::User` with
   `User` documented as "games, plugins, **mods**, tools, benches" — the attribution substrate
   exists.
   **⚠ Scope, stated because every other item in §7 states its cost bound and this one did not.**
   The `Result` requirement covers **registration, load and attribution** — three cold paths. The
   engine's **param-fetch panic policy is out of scope and stays exactly as it is**: making param
   fetch fallible would put an error path on a per-system-run path in **every** build, modding or
   not, which is the P1 violation §7.8 exists to forbid, arriving through a different section. A
   later implementer reading item 6 as licence for a fallible `Res<T>` would be reading it wrongly.
7. **A written no-unload ruling.** `docs/editor/EDITOR-BOUNDARY.md:410` refuses code hot reload; the
   modding design must state explicitly that mods are **load-only, process-lifetime**, and that
   reload is out of scope, so the fn-ptr tables and leaked names never become live-pointer hazards.
   **Rev 5 pins the ordering the façade's drop path needs (pass 4, question 2).**
   `QueryStateCache::drop` calls the **mod's** `drop_fn` for every slot a mod system populated
   (`ecs_master.rs:390-400`), so the ruling is two invariants, not one: nothing ever calls
   `FreeLibrary` on a mod image (the tree's only `FreeLibrary` is the Vulkan loader releasing its
   own `vulkan-1.dll`, `boyko_rhi_vulkan/src/device.rs:1697`, and the mod loader gets no such call);
   and every world is dropped while the image is still mapped — which holds by construction because
   worlds are owned by the `App` the runner borrows (`run_windowed(app: &mut App, …)`,
   `boyko_app/src/runner.rs:158`) and drop before `main` returns, i.e. before process teardown
   unmaps anything. A world parked in a `static` would break the second invariant; none exists, and
   the ruling makes that a rule rather than an observation.
   abi_stable ("the library is leaked"), Burst (unload only at Play-mode exit) and Bevy
   ([#17564](https://github.com/bevyengine/bevy/issues/17564), "ids are dense and strictly
   increasing") all landed in the same place.
8. **A prohibition, stated as a kernel invariant:** *no registry lookup and no `dyn` indirection may
   be added to a **typed** path to accommodate mods.* Engine components keep the per-type
   `static ID: OnceLock<ComponentId>` (`crates/boyko_macros/src/component.rs:371-372`). Bevy's
   `TypeId → ComponentId` hashmap on the typed `init_state` path is exactly what this invariant
   forbids, and it is the single most likely way P1 gets lost quietly.
9. **A by-stable-name mint seam, for every option that puts engine code inside a second image.**
   §0.3 shows that a mod which links the engine rlibs mints from **its own** counters, and that the
   query path's type-erased cache slot then becomes a type-confusion hazard. The kernel change that
   closes it is an `extern "C"`, `TypeId`-free mint keyed on the stable-name tuple, plus an entry
   point that installs a caller-allocated, type-erased `(NonNull<()>, drop_fn)` into the host's
   query-state cache slot — the representation `query_cold_init` already uses. **This is a kernel
   item, not an option-A implementation detail**, and it is needed by A and by any B variant that
   links rlibs for generics. It is NOT needed by A′, C or D, where there is one copy of everything.
   **Two additions from rev 3, both of which make this item bigger than it looked.**
   **(a) It must cover ENGINE component types, not only mod-defined ones.** §0.3 instance 2: the
   per-type `component_id()` mint sits *inside* the engine's own `QueryData` impls, so a mod cannot
   use those impls at all and the seam must supply host-minted ids for `Transform` as readily as for
   `ModComp`. In practice that is a second monomorphised query front-end, kept in step with the
   kernel's — **and it implements the kernel's `SystemParam`** (rev 5; pass 4, question 1), so that
   its `init_access` (`system_param.rs:159-164`) declares the reads and writes the scheduler's
   `Access` is built from (`add_component_read` / `add_component_write`,
   `iters/query/data/read.rs:105`, `data/mut_.rs:234`) using the host-resolved ids; a mod system
   whose access is not derived this way is scheduled as if it touched nothing, and runs beside an
   engine system writing the same column. **The two kernel entry points this item adds are the
   largest `unsafe` surface option A puts into `boyko_ecs`, and rev 4's §7.10 table omitted them
   (pass 4, W2).** They are: a by-stable-name mint returning a `QueryTypeId`, and an installer that
   writes a caller-allocated, type-erased `(NonNull<()>, fn(NonNull<()>))` into the host's
   query-state cache slot for that id. The representation exists —
   `pub(crate) type QueryCacheSlot = (NonNull<()>, fn(NonNull<()>))` (`ecs_master.rs:317`), freed by
   `Box::from_raw` through the stored fn (`:335-338`, `:390-400`) — but the kernel cannot verify the
   *pairing*: the slot is later `cast()` to `UnsafeCell<QueryDataState<D, F>>` and written through
   (`:855`, `:880`) with only `debug_assert!` bound checks (`:381`, `:843`). An installer whose id
   and pointer disagree type-confuses the cache, and as a safe `pub fn` it would be callable by any
   game crate in every build. §7.10's rule therefore gains the clause it lacked: **a `pub` kernel
   item that is unsound if misused is an `unsafe fn` with a stated `// SAFETY:` contract, lives
   under a `#[doc(hidden)]` seam module, and is never re-exported at the crate root** — or the
   pairing is hidden behind a typed kernel helper so the unchecked form is never a public signature.
   Both entry points are rowed in §7.10 (item 5) with that scope.
   **(b) It must expose a probe, and the loader must be able to poison what the probe reports — and
   rev 4 says where each half lives, because "one cold accessor in the kernel" was an exported C
   symbol in every build (pass 3, C1 and question 2).** §0.3 leg (d) needs the addresses of *this
   image's* five mint counters. The counters are private statics in `boyko_ecs` (`mod.rs:214`,
   `query_type_registry.rs:98`, `bundle_type_registry.rs:93`, `resource_registry.rs:127`,
   `event_registry.rs:93`). **Rev 5 splits the kernel half in two, because rev 4 conflated two
   readers with different blast radii (pass 4, O1).** §7.3's loader bound needs **values** — one
   cold `pub fn … -> usize` occupancy reader per dispenser, harmless to expose, since a value cannot
   be written through. Leg (d)'s poison needs **addresses** — `&'static AtomicUsize` — and an
   address accessor is a capability no build has today: `boyko_mod_api` links the same `boyko_ecs`
   rlib every game links, so whatever the kernel exposes for the probe is reachable by any
   third-party crate in the game's graph, and `NEXT_ID.store(…)` from one of them becomes one line
   from wrecking the registry. The address accessors therefore live under a `#[doc(hidden)]` seam
   module, carry a `// SAFETY:` contract (the only sanctioned caller is a loader poisoning an image
   that is not its own, before that image's init runs), and are never re-exported at the crate
   root; the value readers are ordinary `pub`. Both are Rust-ABI, no attribute, no export. The
   **exported half**, `#[unsafe(no_mangle)] pub extern "C" fn boyko_mod_probe_counters(out: *mut
   [*const AtomicUsize; 5])`, lives in **`boyko_mod_api`**, the façade compiled into the mod's image
   and nothing else's: the host binary never contains it, the non-modding build never depends on it,
   and `boyko_ecs` still has zero `no_mangle` / `extern "C"` items (grep, 0 hits at `d552be05`).
   Pass 3 asked whether the per-type `static ID` cells — emitted into every component-defining crate
   by the derive (`component.rs:372`) — need a probe of their own. **They do not, but rev 4's reason
   was wrong (pass 4, C1):** a cell is only ever written with a dispenser's return value, and a
   poisoned counter never yields one — that much holds for every consumer. Rev 4 added "and every
   dispenser asserts before it returns", which `try_register_dynamic` does not (`mod.rs:951-953`,
   `:970-972`); the cells are protected by the *no-id* property alone, and the loudness question is
   §0.3 leg (d)'s consumer table and limit (v)'s latch, not a per-cell probe. Non-modding build:
   nothing for the readers and the probe — §7.10 item 2 — and, for the latch, one static plus one
   store per *instantiated* refusal arm — §7.10 item 2b — which M-P1's pins against the pre-change
   tree confirm or refute (rev 6; rev 5's two-leg diff could do neither).
   **(c) It must register the mod's systems on the host's side of the boundary (rev 6; pass 5, C2;
   ruling R2).** Every production path that puts a system into a schedule boxes it in the calling
   crate: `ScheduleBuilder::add_system<F, M>` (`schedule_builder.rs:177-191`, `Box::new(sys)` at
   `:183`), `insert_state` (`:263`, a boxed closure), `SystemConfig::run_if<C, M>` (`:837`, a boxed
   condition), and `App::add_startup_system` (`app.rs:508-518`, a boxed closure at `:514` drained
   in `finish()`); `App::add_systems` / `add_systems_in` (`:339`, `:398`) reach the first,
   `add_systems_cfg` / `add_systems_cfg_in` (`:318`, `:372`) hand out the builder itself, and
   `add_plugin` (`:550`) runs a foreign `build(&mut App)`. Each box is owned by the host —
   `SystemBox.system` (`system_box.rs:83`), `Schedule::systems`, `App::startup` — and freed by host
   drop glue, so under A every one is a mod-image allocation the host frees. All of them join the
   forbidden set; the seam that replaces them is the shape the query slot already has, applied to
   systems:
   - **in `boyko_ecs`**, one non-generic `System` impl over `(unsafe extern "C" fn(*mut c_void,
     UnsafeEcsCell<'_>), *mut c_void, SystemMeta)` — call it the fn-pointer system — and one cold
     entry on `ScheduleBuilder` that takes that triple plus a `#[repr(C)]` access descriptor (a
     slice of `(ComponentId, read|write)` and `(ResourceId, read|write)` pairs), builds the
     `Access` host-side through `add_component_read` / `add_component_write`
     (`system/access.rs:81`, `:87`) and their resource twins, constructs `SystemMeta::new(name,
     tick)` (`system_meta.rs:205`), and pushes `SystemDescriptor::new(SystemBox::new(Box::new(…)))`
     (`system_descriptor.rs:77`, `system_box.rs:129`) — the `Box::new` now monomorphised in the
     host image, because the entry is reached through a **host-supplied function pointer** (the
     handshake's table), never called from mod code directly. `System::initialize` on the
     fn-pointer system is a no-op: the mod's `ModQuery` state is mod-allocated and installed through
     (a)'s slot installer, and its access was derived from host-resolved ids before registration,
     which is what makes the descriptor honest. Both items are `unsafe fn` under the
     `#[doc(hidden)]` seam of §7.10's clause: a descriptor that under-declares a write makes the
     scheduler run two writers concurrently, the same "unsound if misused" shape as (a)'s installer.
   - **in `boyko_mod_host`** (the host image), the `extern "C"` wrapper the loader hands the mod.
   - **in `boyko_mod_api`** (the mod image), the trampoline macro of limit (v), now emitted as the
     `extern "C"` body the fn pointer names, with `*mut c_void` pointing at the mod's own system
     state (mod-allocated, mod-owned for the process lifetime — §7.7).
   **Price.** Per mod-system run: the vtable call the dispatch already makes (`schedule.rs:1380`,
   `:1215`) plus one fn-pointer call — two indirect calls where an engine system pays one; the same
   per-system-run class, not a per-row one, so not a new cost class (§1). Per registration: one
   host-side allocation, cold, config-phase. Non-modding build: the fn-pointer system type and the
   cold entry are unreachable in a build that never links `boyko_mod_host`; whether they are absent
   at `lto = "fat"` is what M-P1's pins say, not what this paragraph says (§7.10 item 6). **The
   fallback R2 admits** — a mod-side `Box<dyn System>` handed over together with the mod image's
   deallocation fn pointer — would need `SystemBox` (40 B, `system_box.rs:70-77`) or `Schedule` to
   carry that pointer, a field on a structure the dispatch reads in every build; it is admissible
   only if the pins prove it free, which is why the host-side form is primary.
10. **The configuration mechanism, per item — and the rule it obeys (rev 4; pass 3, C1).** Rev 3
    ruled for A′ that "two shipping configurations" is not a design until the flip is named, then
    named it for §1.5 only. Four other places in this document said "modding configuration only" and
    pointed at nothing; the obvious implementation of each is a cargo feature on `boyko_ecs`, which
    feature unification lets any third-party crate enable for a game that never asked (§1), and
    which M-P1 — a diff of *this* tree's two builds — cannot see. **The rule:** *the kernel may gain
    visibility and cold Rust-ABI functions; it may not gain an attribute (`#[no_mangle]`,
    `#[export_name]`, `#[used]`, `#[link_section]`), a `cfg`, a feature, or a constant whose value
    depends on the configuration. Everything configuration-scoped is scoped by a dependency EDGE — a
    crate the non-modding binary does not depend on — never by a switch on a crate it does.* **Rev 5
    adds the clause pass 4 (W2) found missing:** *a `pub` kernel item that is unsound if misused is
    an `unsafe fn` with a `// SAFETY:` contract under a `#[doc(hidden)]` seam, never re-exported at
    the crate root — visibility is admitted, an unchecked public signature is not.* **Rev 6 adds
    the condition R1 attaches to every admission:** *a kernel delta is admitted only while every
    M-P1 pin captured on the pre-change tree (`d552be05`) is identical in the post-change
    non-modding build; a delta that moves a pin is charged to P1 and moves behind the modding crate
    boundary. A dead `#[doc(hidden)]` seam that fat LTO drops is acceptable only if the pins prove
    it dropped — "cold", "unreachable" and "on a path about to unwind" are arguments the pins
    either confirm or refute.* Per item:

    | item | kernel delta (`boyko_ecs`, every build) | configuration-scoped part, and the crate that owns it | what a non-modding build carries | gate |
    |---|---|---|---|---|
    | **1 — §7.1 descriptor path** | `pub` on `try_register_dynamic`, `set_storage_kind`, `set_residency_class`, `install_map_entities_fn`; by-value forms of `install_clone_fn` / `install_serialize_fn` (two cold fns) | the descriptor struct, its validation, and the `extern "C"` entry — `boyko_mod_host` | no new symbol export (none is present); the two cold fns are either unreachable — absent at `lto = "fat"`, dead bytes at default release (`REFLECTION-ANALYSIS.md:1467-1494`) — or, if the generic installers are de-duplicated onto them (§7.1; pass 4, O2), inlined into the once-per-type `OnceLock::set` path the derive already runs; no hot path changes either way | M-P1 pins (rev 6): `register_new::<T>` / `T::component_id()` for a representative `T`, `.text` size, the three hot functions — captured at `d552be05`, compared on the post-change non-modding build; + the census below — **not** the reachability argument |
    | **2 — §7.9(b) counter probe** | five `pub fn` **value** readers (occupancy); five `&'static AtomicUsize` **address** accessors under a `#[doc(hidden)]` seam with a `// SAFETY:` contract (pass 4, O1) | the `#[unsafe(no_mangle)] extern "C"` probe — `boyko_mod_api`, i.e. the **mod's** image only | nothing: the host never links `boyko_mod_api` | source census: `no_mangle` / `extern "C"` / `export_name` / `link_section` over **every kernel crate a non-modding game links** — `boyko_ecs`, `boyko_utils`, `boyko_threadpool`, `boyko_log`, `boyko_diag`, `boyko_macros` (the derive's *emitted* tokens included) — stays **0** (measured 0 in each at `d552be05`; `boyko_ecs`'s own `[dependencies]` names the first four). **The `llvm-readobj --coff-exports` leg on the executable is withdrawn (pass 4, W1) — see below** |
    | **2b — §0.3 limit (v) mint-refusal latch** (rev 5; **conditional from rev 6**) | one `static AtomicBool` beside the counters; one relaxed `store(true)` on each of nine refusal paths (`mod.rs:922`, `:938`, `:970`, `:996`, `:1046`; `query_type_registry.rs:133-145`; `bundle_type_registry.rs:121-136`; `resource_registry.rs:176-180`; `event_registry.rs:112-116`) — all `#[cold]` or on an assert's failing arm; one `pub fn` reader | the read at the mod-system boundary (trampoline or panic hook) and the loader's post-init read — `boyko_mod_api` / `boyko_mod_host`, never the host's kernel | one process-global and a store per *instantiated* refusal arm — of order 130–150 emitted stores, not nine, because `register_new<T>` / `register_layout<T>` are per-type, per-crate instantiations reached from the derive's `#[inline]` `component_id()` (pass 5, C1); "nothing on any hot function" is the claim, the pins are the proof | **M-P1 pins (rev 6).** Rev 5's two-leg diff could not fail here — the delta is in both legs, and the three hot functions are reached by none of §7's items; the pins that can fail are `register_new::<T>` / `T::component_id()` for a representative `T` and `.text` size. **If a pin moves, the latch leaves the kernel.** The boundary-side form, verified against the dispensers: poison each counter with `cap + 1` instead of `cap`, and read the counters back through the probe at the trampoline's panic path and at the loader's post-init read — the three `fetch_add`-then-assert dispensers leave `cap + 2` (`mod.rs:921-922`, `resource_registry.rs:175-176`, `event_registry.rs:110-111`) and the two saturating ones store the cap *back* (`query_type_registry.rs:132-135`, `bundle_type_registry.rs:120-128`), so `counter != poison` is a structural, text-independent signal with **no kernel delta at all**. Its one gap: `try_register_dynamic`'s CAS loop returns `None` *without* moving `NEXT_ID` (`mod.rs:968-972`), so the tag and asset refusals it feeds are invisible to a counter read — reachable only outside the façade (forbidden set) and observable only by their own panic (`backing.rs:132`, `tag_api.rs:245`) or not at all (`try_register_tag`). That gap against the kernel delta is the trade the architect makes (§11) |
    | **3 — §7.3 id budget** | the five **value** readers of item 2 (values, never addresses); **no constant changes** (`MAX_COMPONENTS = 512` unconditional, `mod.rs:63`); under §7.3 direction D3, one more cold `pub` Rust-ABI fn (`try_register_dynamic_at`) | the bound (D2's census-held ceiling and/or D3's downward cursor), the per-mod quota and the `Result` — `boyko_mod_host` | nothing reserved; capacity identical | occupancy report (M-K1) + the drift measurement (M-K3) + the census + M-P1 pins (rev 6: the readers are new cold functions and must not move `.text` or any pinned body) |
    | **4 — §3 export set / sentinels** | none | `#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]` statics and any `extern "C"` wrappers — `boyko_mod_registry`, a dependency of the **modding game binary** only (a `[dependencies]` edge or a second `[[bin]]`, never a feature) | nothing: `cargo tree` shows the crate absent | M-C0b + the census |
    | **5 — §7.9(a) query seam** (A only; rev 5, pass 4 W2) | a by-stable-name `QueryTypeId` mint and a slot installer taking `(NonNull<()>, fn(NonNull<()>))` — both `unsafe fn` under a `#[doc(hidden)]` seam with `// SAFETY:` contracts, never re-exported at the crate root; or a typed helper that hides the pairing | `ModQuery<D, F>` (a `SystemParam` — §7.9(a)) and the trampoline — `boyko_mod_api` | two unreachable cold `unsafe fn`s: absent at `lto = "fat"`, dead bytes at default release; no hot path changes | M-P1 pins (rev 6) + the census; **M-A4 leg (d)** (a compliant mod's derived `Access` conflicts with an engine system's on a shared write, and the scheduler serialises them) |
    | **6 — §7.9(c) system-registration seam** (rev 6, R2; A, and any B variant that links rlibs) | one non-generic fn-pointer `System` impl and one cold `unsafe fn` entry on `ScheduleBuilder` taking `(fn ptr, ctx, name)` plus a `#[repr(C)]` access descriptor — under the `#[doc(hidden)]` seam, never re-exported at the crate root | the `extern "C"` wrapper — `boyko_mod_host` (the host image); the trampoline body — `boyko_mod_api` (the mod image) | two unreachable cold items: absent at `lto = "fat"` if the pins say so, dead bytes at default release otherwise; `SystemBox` (40 B), `Schedule` and the dispatch loop unchanged — the host-side form is primary precisely because it touches no structure the dispatch reads | M-P1 pins (schedule dispatch disassembly, `.text` size) + the census; **M-A4 leg (a)**'s new fixtures (`App::add_systems*`, `add_startup_system`, `add_plugin`, `ScheduleBuilder`) and **leg (e)** |
    | *(A′ / A″ — §1.5, §1.6)* | none | the `crate-type` flip — a manifest overlay, route (ii) of §1.5 | intended: nothing — **gated, not assumed** (rust#51009) | M-F1 leg (c) |

    **Why this census is decidable where the symbol census was not — and why the PE-export leg is
    withdrawn (rev 5; pass 4, W1).** P1's ban on a whole-image symbol census is about *presence* — an
    unreferenced item is carried at default release whether or not anything reaches it
    (`REFLECTION-ANALYSIS.md:1477-1494`). The source census is about *source* — a grep over the kernel
    crates for the attributes the rule forbids — and it can fail: it starts at 0 in every listed
    crate and a violation makes it red. Rev 4 added a second leg, `llvm-readobj --coff-exports` on
    the non-modding executable, and cited M-10 leg 3 for its decidability. **The receipt says the
    opposite** (research §13.3, leg 3): on the shipped toolchain a `#[no_mangle]` item in a
    windows-gnu **executable** is a global symbol and *not* a PE export — the exe's export directory
    is empty *with* the forbidden attribute present and *without* it, and only nightly's
    `-Z export-executable-symbols` (leg 2c) changes that. A leg that is green on the configuration it
    gates whether or not the violation is present is the repository's own "gate that could not fail",
    arriving inside the gate written to prevent it. It is withdrawn for the executable;
    `--coff-exports` stays informative for **mod DLLs and any `cdylib`** (a `cdylib`'s export
    directory is deliberate and enumerable) and belongs to M-A4's checks on the mod artifact, not to
    P1. Rev 4 also scoped the source leg to `crates/boyko_ecs/src` alone, while a non-modding game
    links `boyko_utils`, `boyko_threadpool`, `boyko_log`, `boyko_diag` and the derive's emitted code
    — a future `#[unsafe(no_mangle)]` in `boyko_threadpool`, or emitted by `boyko_macros`, would have
    passed both legs. The census now covers all six (0 hits in each at `d552be05`); the emitted-token
    check is the same grep over `boyko_macros/src`, which is where any emitted attribute has to be
    spelled (an attribute assembled from fragments at macro run time would evade it — a hazard named,
    not one the tree has). M-10 leg 2 is the strongest support the rule's *visibility* deltas have and
    is cited here directly: a `pub static` measured **local** (`b eng::NEXT_ID`) under `lto = "fat",
    codegen-units = 1`, unchanged by `-C link-dead-code` — making a kernel item `pub` changes nothing
    about what the shipped binary carries. **R1's pins are the other half (rev 6):** the census sees
    attributes in source, the pins see bytes in the binary against the pre-change tree, and neither
    is the modding-vs-non-modding diff rev 5 relied on — which no unconditional kernel delta can
    fail.

---

## 8. Recommendation

**Decided by performance first**, per the owner's standing rule, with the gate that would overturn
each decision named.

### 8.1 The ranking

**Live ordering (rev 6; pass 5, W2 — the headline now says what the body says): A′-nightly
(pending M-F1 leg (d)) > C-3a ≈ A′-stable, with C-route-2 unplaced pending M-C5, and A third with
its Stage-0 caveat. C's primacy — ranked as route 3b in rev 3 — is WITHDRAWN pending M-C4 / M-C5,
not asserted-then-suspended.** The paragraphs below are the record of how the ranking got here and
are kept as written; the headline is the decision surface, and it must not ratify a route whose
only probe (M-C5) has not run. Rev 3's headline read *"Primary: C (install-time relink). Fallback:
A′, with A demoted to third."*

**⚠ Rev 4: the primary ranking is SUSPENDED, pending the workspace-scale M-C4, and the document's own
§8.2 rule says what happens if M-C4 confirms M-10.** C was ranked first *as route 3b*. M-10 (research
§13.3) measured 3b's two "predictions" at toy scale and both are red: fat LTO internalises the
per-type cell, the dispenser and `std`'s internals; a mod object compiled against the rlibs references
all three by name; no export set can pin a function-local static; and re-adding the rlibs fails on the
allocator shim rather than producing a silent pair. The mechanism is rustc's LTO internalisation and
does not depend on workspace size, so the expected M-C4 result is red for 3b. §8.2's C row then
applies as written — *3b does not exist and C falls back to 3a, which pays A′'s LTO forfeit and keeps
C's install risks; that would drop C below A′ outright.* What rev 4 adds to that rule is C's **other**
live form, route 2 — linker-plugin LTO at install — the only form of C whose performance claim is
*stronger* than 3b's ever was (the mod objects exist at LTO time, so engine and mod are
co-optimised), at the costs the route table already prices: a 211 MB plugin-capable linker where 3a's
set is 48 MB, whole-engine bitcode shipped to every player, an install step that is a full LTO link
(unmeasured, M-C1 leg (c)), and the proc-macro concern (unverified for windows-gnu, sidestepped by
`--target`; §3). **The re-ranking is the architect's**, not this revision's; the inputs it needs are
M-C4 (to close 3b formally), M-C1 leg (c) (route 2's install time), and M-F1 leg (d) (whether
nightly's `-Z dylib-lto`, feasible at toy scale in M-11, moves A′'s price from 1.26× toward 1.00× on
the workspace). Until those exist, the candidates that are correct by construction order, on the
owner's throughput priority, as **C-route-2 ≥ A′-nightly (if M-F1(d) holds) > C-3a ≈ A′-stable**,
the two pairs separated by distribution cost rather than by throughput; A keeps 1.00× and its §0.3
trust-model caveat, unchanged.

**⚠ Rev 5 labels the head of that ordering a PREDICTION and keeps it out of the ranking until a probe
exists (pass 4, W4).** Route 2's "co-optimised with the engine" is a stronger optimisation claim than
3b's ever was, and none of the three inputs above measures its *output*: M-C4 is 3b's existence,
M-C1 leg (c) is a wall-clock, M-F1 leg (d) is A′'s number. Rev 4's own lesson (M-10) is that an
asserted optimisation property can be red at toy scale for a mechanical reason. Two mechanical
unknowns sit under route 2. **(i)** Whether the precompiled sysroot `std` participates in
linker-plugin LTO at all: the rustc book page the document cites for the route covers the linker
requirement, LLVM-version matching and the proc-macro / `prefer-dynamic` Windows error, and **says
nothing about the standard library** (re-read for this revision:
https://doc.rust-lang.org/rustc/linker-plugin-lto.html — no occurrence of "std", "sysroot" or
"precompiled"), while today's fat-LTO build demonstrably pulls a temp-directory `libstd` copy onto
its link line (M-10 leg 7). If `std` arrives at the plugin link as machine code, a route-2 binary has
*less* cross-module optimisation into `std` than today's shipped build, while charging 211 MB of
linker and whole-engine bitcode to every player — the ordering would invert after the work was done.
**(ii)** Whether an installer can drive `rust-lld` with a line whose only receipt (M-5) is for a
different driver (`x86_64-w64-mingw32-gcc`) with mingw sysroot flags. **M-C5** (§10) is the toy-scale
route-2 leg of M-10's kind — the rig exists — and until it returns, the ordering the architect
re-ranks on is **A′-nightly (if M-F1(d) holds) > C-3a ≈ A′-stable, with C-route-2 unplaced**; the
"≥" above is what M-C5 would earn, not what it has.

Rev 1 ranked A second. Two rev-2 findings move it: §0.3 shows A's central contract is unenforceable
by convention and that its failure is silent memory corruption on the query path, and §1.5 shows
that a route which dissolves that hazard *by construction* was left off the table. A is still viable
— but only with the §7.9 kernel seam and the §0.3 façade built first, which makes it the option with
the **most** kernel work rather than the second-least. **Rev 3 adds the sharper half of that
sentence**: the façade's enforcement is cooperative except for the mint-counter poison (§0.3 leg
(d)), which stops honest mistakes and not a hostile author. So the comparison is not "A is unsound"
but **A′ and C are correct by construction, while A is correct by construction against mistakes and
by curation against malice** — and "who builds the mod" therefore moves into Stage 0 beside the trust
model, where it can be answered before anything is built.

The reasoning is one number and one structural fact.

- **The number, restated for the route C actually ships (rev 3).** Every dynamic-boundary option
  costs *something* this engine has already measured and paid for. Fat LTO buys geomean **0.793** and
  `query_ref_iter/10k` **0.375** (2.67×) for **the engine's own code** (`Cargo.toml:89-112`). Under
  route 3b (§3) C keeps all of that and adds no boundary at all: the mod's inner loop is native for
  §0.1's reason (generics and `#[inline]` bodies instantiate in the mod's own compilation unit), and
  its `spawn`/registry calls are direct calls inside one image rather than crossings. What C does
  **not** buy — and rev 2 claimed it did — is co-optimisation *between* engine and mod code; that is
  route 2, at 211 MB of linker plus whole-engine bitcode shipped to every player. So the ranking
  argument is now: **C = A's inner-loop story, with A's boundary and A's §0.3 hazard both deleted,
  and the engine's 0.793 intact.** A keeps the 0.793 inside each side but not across, and pays the
  boundary and the hazard. B additionally pays a data-representation tax on anything crossing — the only measured
  figure is **~30 %**, and it is **Tremor's whole-pipeline number for a hash-map-shaped hot path with
  the plugins still statically linked**, not a number for an ECS column loop (M-B2 would price this
  engine's own shape) — and it pins host entry points out of LTO internalisation. E pays 1.33×–2.41×
  and loses AVX2 outright.
- **The structural fact.** C's "cost when modding is not used" proof is trivial, because the
  non-modding build *is* today's build process. Its kernel needs are §7 items 3, 5 and 8, plus the
  linker-collected plugin registry — which rev 1 asserted without a mechanism and rev 2 has now
  **measured into existence**: a PE grouped section rooted per mod with `-Wl,--undefined`, keeping
  `--gc-sections`, with a boot-time collected-count assertion against the manifest (§3, M-6/M-7).
  That is a genuine new mechanism of a class this tree has twice refused
  (`REFLECTION-ANALYSIS.md:307-311`, `:742`; `boyko_log/src/target.rs:978-982`), so C's kernel work
  is **not** "the fewest of any option" — D's is, and A′'s is next. What C keeps is the *performance*
  argument, which is the owner's stated priority, and the P2-collapses-into-P1 property.
- **What rev 2 removed from the ranking's risk column.** Rev 1's C rested on two unestablished steps:
  who emits the `add_plugin` call, and who constructs the link line. **Both are now measured on this
  exact toolchain** (M-5…M-7, research §13): a rustc-free relink with the toolchain's own bundled
  mingw driver produces a working binary, and the plugin registry collects. C therefore no longer
  risks collapsing into D on feasibility. It still risks being disqualified on **route existence**
  (M-C4 — whether an already-fat-LTO'd engine object set relinks against mod objects at all;
  rev 3's addition, and the one that comes first), on **install time** (M-C1, unmeasured), on
  **antivirus reaction** (M-C3) and on **code signing**. **Bitcode disclosure has left C's risk
  column**: it is route 2's cost, and rev 3 establishes that C ships route 3b (§3).

**B is not recommended** despite being the only option that survives engine patch releases. Its
advantage is real, and three of its four costs need no borrowed evidence: a permanently-exported
entry set, no generics across the boundary, and a decade-long API commitment made with the least
information. The fourth — the data-representation tax — is real in kind but **unmeasured for this
engine**; the ~30 % is Tremor's, on a different workload shape (M-B2). The rejection stands on the
first three. B would only be right if mod-ecosystem longevity outranks throughput — which inverts the
owner's stated priority.

**D is C with the compiler shipped.** Keep it as a variant of C, not a separate track: if C is built,
D is C plus `cargo build` on the user's machine plus a compiler. The Arch figure is 302 MB installed;
**measured here, the full `stable-x86_64-pc-windows-gnu` toolchain on this box is 2772 MB**
(`bin` 529 MB, `lib/rustlib` 1373 MB, docs 865 MB) — a trimmed shipment would be far smaller, but the
"302 MB class" label is a floor, not an estimate for Windows. Decide D after C's install-time number
exists.

**E is a fork, not a stage.** It shares no mechanism with A–D. It is the right answer only if
untrusted third-party mods must run, and that is a values call for the owner, not a performance one.

### 8.2 The gates that would overturn each decision

| Decision | Overturned by |
|---|---|
| **C primary** *(as route **3b** — §3; **suspended in rev 4**, §8.1)* | **M-C4** first — **and M-10 has already returned it red at toy scale (rev 4)**: if an already-fat-LTO'd engine object set cannot be relinked against mod objects — because LTO internalised the symbols the mod needs, or because adding the rlibs back produces two copies of a registry static — then 3b does not exist and C falls back to 3a, which pays A′'s LTO forfeit *and* keeps C's install risks; that would drop C below A′ outright. Then **M-C1**: link-only time for the shipped route exceeding a budget the owner sets (suggest: > 3 min for a typical mod set). Also overturned by an antivirus/EDR finding (M-C3), or by a legal finding that the bundled linker cannot ship (a licence review, not a measurement). **The whole-engine-bitcode disclosure no longer overturns C** — it is route 2's cost, and C does not ship route 2. **No longer overturnable on relink feasibility**: M-5…M-7 settled that for route 3. **Rev 5:** route 2's place at the head of the suspended ordering is a prediction; **M-C5** returning red on `std` participation or on the `rust-lld` line removes it |
| **A′ fallback** | **M-F1**: this workspace failing to build as `crate-type = ["dylib"]` (and leg (d), rev 4, cuts the other way — nightly `-Z dylib-lto` lowering A′'s price, M-11), hitting the PE 65535-export ceiling on windows-gnu, **or leg (c) showing the non-modding build losing fat LTO to the configuration mechanism** (rust#51009 — the silent one; §1.5). Also by the owner ruling that a 1.18×–2.67× penalty on modded sessions is unacceptable even though non-modding builds are untouched — in which case the fallback reverts to A, and §7.9 (including its rev-3 additions (a) and (b)) + the §0.3 façade become mandatory Stage 1 work. **`TypeId` across configurations (M-T2) is informational for A′, not blocking**: the §1 build canary makes both sides identical in rustc, flags, features and engine revision by construction, so the shared `(TypeId, TypeId)` intern key cannot diverge unless the canary's axis list is wrong — which is what M-T2 would reveal |
| **A third** | **M-A1**: a mod system's inner loop measurably slower than the same system compiled in (> 5 % on `query_ref_iter`-shaped work) would mean the "generics instantiate in the caller" premise does not survive this kernel's real code. ⚠ **M-A1 cannot see A's real defect** — a two-shape microbenchmark's id collision is benign, so A's soundness is decided by whether §7.9 + the §0.3 façade can be built and gated (**M-A4**), not by a stopwatch. **And M-A4 can only ever return "sound against mistakes"**: leg (d) binds an honest author, nothing in the image binds a hostile one, so A's remaining half is answered in Stage 0 by who builds the mod. **Rev 6:** A's Stage-1 kernel list gains §7.9(c) — the fn-pointer registration seam — so A stays the option with the most kernel work; the overturn condition is unchanged |
| **B rejected** | the owner ruling that mods must survive engine patch releases. That single requirement makes B the only candidate and the ~30 % becomes the price of the requirement, not an argument against it |
| **E rejected** | the owner ruling that untrusted mods must run without review. No native option can be made safe; E then becomes mandatory for the untrusted tier, alongside a native tier for trusted mods |
| **§7.3 loader quota** *(rev 4; was "reserved band")* | a measurement (M-K1) showing that raising `MAX_COMPONENTS` to 1024 costs < 1 % on `query_ref_iter` / `swap_remove` — in which case raising for everyone is simpler than a quota. **Prior expectation is that it does not**, because `Access` goes 192 B → 320 B and the 3-cache-line property named at `system/access.rs:56` is lost. **A per-configuration constant is no longer a candidate at all**: its only in-tree mechanism is a cargo feature on `boyko_ecs` (§7.10). **Rev 5: the quota's *bound* is re-opened (pass 4, C2)** — the instantaneous-counter form is withdrawn because engine ids mint lazily after the load slot, the direction (D1 / D2 / D3, §7.3) is the architect's, and **M-K3** measures the drift any bound must absorb; the row's overturn condition above is unchanged |

### 8.3 What is decided regardless of which option wins

- Mods are **load-only**; no unload, no reload (§7.7).
- Identity is **name + layout fingerprint**, never `TypeId`, never a raw id (§7.2).
- The modding layer is a **separate crate**, never a cargo feature on a kernel crate (feature
  unification, §1 "Cost when modding is not used") — **and, rev 4, no kernel item is
  configuration-scoped by any other switch either: no attribute, `cfg` or per-configuration constant
  in `boyko_ecs`; scoping is by dependency edge only (§7.10), with a source census over every
  kernel crate a non-modding game links as the gate — the PE-export check on the executable is
  withdrawn in rev 5 (it cannot fail on windows-gnu: M-10 leg 3), and a `pub` item that is unsound
  if misused is `unsafe` under a hidden seam, never a safe signature at the crate root.**
- **P1 is gated against the pre-change tree, never against the sibling configuration (rev 6,
  R1):** M-P1's pins are captured at `d552be05` and compared on each post-change non-modding
  build; a kernel delta present in both configurations of one tree is exactly what a
  two-configuration diff cannot see, and every §7.10 admission is conditional on the pins.
- **A mod system is registered host-side (rev 6, R2)** — for every option that puts engine code in
  a second image (A, and B when it links rlibs): a function pointer plus a `#[repr(C)]` access
  descriptor, the `SystemBox` allocated in the host image (§7.9(c)); `App::add_systems*`,
  `add_startup_system`, `add_plugin` and `ScheduleBuilder` are forbidden to a mod, with fixtures.
  C and D have one image and are untouched.
- **No typed path gains a lookup or a `dyn` hop** (§7.8).
- Mod components on engine entities use **dense/non-fragmenting storage** (§7.4) — with the
  consequence stated: a dense type is **ALWAYS `ResidencyKind::Cpu`** and is excluded from every
  archetype signature (`component/component.rs:67-79`), so **a mod component can never be
  GPU-resident**. If a mod use case needs GPU residency, it needs a different storage decision, and
  that decision is not made here.
- **Prototype-only modding is a SCOPE axis, not a sixth option** (§11) — it restricts what a mod may
  do and is composable with A, A′, C or D.
- **If C is built, it is route 2 or route 3a — not 3b** (§3, rev 4). 3b (a plain relink onto
  fat-LTO'd objects) is red at toy scale (M-10): the objects a mod compiles against the rlibs
  reference internalised statics and `std` internals that no export set can pin. Route 1 is option D
  by definition. Route 2 (linker-plugin LTO at install) is the only C form that keeps — and extends —
  the engine's LTO; its costs are a 211 MB plugin-capable linker where 3a's set is 48 MB,
  whole-engine bitcode shipped to every player, and an install-time LTO link (M-C1 leg (c)); the
  proc-macro/Windows error is **unverified for windows-gnu** (research §13.2) and sidestepped by
  building with an explicit `--target` (§3 route table).
- **A number measured on the 7-benchmark ECS suite is quoted with its denominator**, never as a
  session-level or whole-frame figure (rev 2's W5 ruling, applied in rev 3 to the 0.793 itself).

---

## 9. Staged path

Each stage is independently useful and independently abandonable. No stage commits the next.

**Stage 0 — decide the values questions.** **Three** now, and they are the owner's: (i) trust model —
curated / signed native mods, or untrusted mods needing a sandbox; (ii) whether mods must survive
engine patch releases; (iii) **who builds a mod** — the author in their own workspace, or an
engine-owned build service / curated repository that owns the manifest and the flags. (i) decides
whether E is needed at all; (ii) decides whether B becomes mandatory; (iii) is new in rev 3 and
decides how much of option A's §0.3 enforcement is even *possible*: leg (a) of the gate is
meaningful only under the second answer, and leg (d) binds mistakes rather than malice under the
first (§0.3). Nothing below is worth building until all three are answered.

**Stage 1 — the kernel work that is right regardless (§7).** Public component descriptor
(**an input struct that fans out into the existing parallel cold tables — TRIPWIRE 2 stays green**)
with layout validation; mandatory `stable_name` for externally-supplied types; the id budget policy
(**no constant, no reservation, a loader-owned `Result` before any mint — decided; the bound's
mechanism — D1 / D2 / D3 of §7.3 — is the architect's, and is the one Stage-1 item rev 5 re-opens**);
the no-unload ruling written down, with its two rev-5 invariants (no `FreeLibrary`, every world
dropped before the image; §7.7); the no-`dyn`-on-typed-paths invariant written down and gated.
**This is useful without any modding**: it is the same by-id seam the reflection lane needs
(`D:/wt/reflect/crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs:200,500` already has
`add_component_by_id` / `remove_component_by_id` in that worktree, absent here), and the same
descriptor the editor and any data-driven tooling want. Cost to non-modding games: **zero in time and zero in capacity** — nothing is reserved (§7.3), and
the kernel delta is visibility plus a handful of cold Rust-ABI functions with no attribute, `cfg`,
feature or per-configuration constant (§7.10).
**Gate list for Stage 1**: TRIPWIRE 2 green (`size_of::<ComponentLayout>() == 56`), the `Access`
192 B pin green, the occupancy headroom reported as a number, **and the §7.10 census — `no_mangle` /
`extern "C"` / `export_name` / `link_section` over `boyko_ecs`, `boyko_utils`, `boyko_threadpool`,
`boyko_log`, `boyko_diag` and `boyko_macros` = 0** (the executable's `--coff-exports` leg is
withdrawn — rev 5, it could not fail on the configuration it gated), **and M-P1's pins (rev 6)
identical to their `d552be05` capture after every kernel edit — captured BEFORE the first one.**
§7.9's by-stable-name mint seam is Stage 1 work **only if A becomes the chosen route** — and if it
is, it carries its rev-3, rev-5 and rev-6 additions with it: the seam covers **engine** component
types too and is a `SystemParam` so a mod system's `Access` is derived; the kernel exposes the five
counters' **values** as readers and their **addresses** under a hidden `unsafe` seam, the
mint-refusal latch (kernel-side only while the pins hold), the two `unsafe` query-seam entry points
and the fn-pointer system-registration seam (§7.9 (a), (b), (c); §7.10 items 2, 2b, 5, 6); and §0.3
leg (d)'s coverage is derived from every consumer of every counter — six mint paths, the
asset-store constructor among them — with limit (v) aborting on the latch, never on panic text,
and the loader reading the latch once after the mod's init.

**Stage 2 — measure (§10).** **M-C0 is done** (M-5…M-7, research §13) **for route 3**: a rustc-free
relink is feasible and the plugin registry collects. **M-C4 comes first in rev 3, and rev 4 gives it an
expected answer** — M-10 is red for 3b at toy scale, so the workspace run confirms a mechanism rather
than opens a question; it decides whether the route the recommendation rested on (3b) exists at all; **M-C1** follows and decides the
primary recommendation on time. Then M-F1 (whose leg (c) is a P1 check, not an A′ check, and whose leg (d) — nightly `-Z dylib-lto` —
may lower A′'s price rather than test it), M-C1 leg (c) (route 2's install-time LTO link), M-K1, M-A1,
and M-F2 if the owner wants §1.6 priced. None of these needs a modding implementation: M-C4 and M-C1
are relink experiments on the existing workspace, M-F1 a `crate-type` flip plus two builds, M-A1 a
two-crate microbenchmark, M-K1 a constant change and the existing bench suite.

**Stage 3 — build the winner behind `boyko_mod_host` (a separate crate).** With the P1 gate written
*first*, **and with its build profile named** (§10 M-P1): the pins captured on `d552be05` **before
the first kernel edit** — byte size and disassembly of the named functions, the three `size_of`s,
the executable's export count, `.text` size — compared on every post-change non-modding build,
plus `cargo tree -e features` proving absence. The rev-5 form — a byte-identical-asm diff between
the modding and non-modding builds of one tree — is withdrawn (pass 5, C1).
**⚠ The gate must not be a symbol census** — measured undecidable at default release
(`REFLECTION-ANALYSIS.md:1467-1494`).
**⚠ And it must say which binary it describes.** The shipped profile is `lto = "fat"` with
`codegen-units` **deliberately unset** (`Cargo.toml:104-112`; setting it to 1 under fat LTO regresses
`query_ref_iter/10k` **17.4 %** — one of this gate's own three functions). The only profile whose
determinism this tree documents is `[profile.bench]`, which sets `codegen-units = 1` **and
`lto = false`**, and whose own comment states that a number taken with `cargo bench` "does NOT
describe the shipped profile on the codegen axis, and must not be quoted as if it did"
(`Cargo.toml:113-126`). So the asm pins prove **codegen equivalence of the source change** under a
named, deterministic profile — they do not prove bit-identity of the shipped artifact. The size,
export and `.text` pins cover what they then miss, taken on `profile.release` itself, with
`cargo tree -e features` beside them (M-P1, rev 6).

**Stage 4 — one real mod, end to end**, exercising: a new component type, a system reading engine
components and writing mod components, a save round-trip with the mod absent, and a mod-authored
panic. Measure P3 against the same system compiled into the engine.

**Stage 5 — decide D and E separately**, on Stage 0's answers and Stage 2's numbers.

---

## 10. Measurements to queue

Benchmarks cannot be run now. These are specified so they can be run without re-deriving the
question. Every one states what it decides.

| ID | Measurement | Method | Decides |
|---|---|---|---|
| **M-C0** ✅ **DONE** | **Feasibility of the install step**: (a) can a Rust `bin` be relinked with an extra mod object **without rustc**, on windows-gnu; (b) does a linker-collected plugin registry work there | done in the rev-2 session — M-5/M-6/M-7, research §13 | **(a) yes** — `rustc --print link-args` on stable + the toolchain's bundled `self-contained` mingw driver; ship `symbols.o`. **(b) yes, with a trap**: `--gc-sections` silently collects nothing unless the installer roots each mod's registration **static** with `-Wl,--undefined`. **⚠ Scope (rev 3): both receipts are for route 3 — a plain link with no LTO in it.** They say nothing about route 2 (linker-plugin LTO), and nothing about relinking an object set rustc has already fat-LTO'd, which is M-C4 |
| **M-C0b** | Cost to the ENGINE of the plugin-registry mechanism: binary size and `query_ref_iter`/`swap_remove`/dispatch with the sentinel section present vs absent, and (as a control) with `--gc-sections` dropped entirely | build both legs of the real workspace; the toy delta measured in M-6 — 4 959 645 B → 4 965 583 B, **+0.12 %** from dropping `--gc-sections` on a hello-world — says nothing about the engine | whether C's registry is free for a game with zero mods, i.e. whether P2 really collapses into P1 for C |
| **M-C1** | **Install-time link, re-posed for the route C ships (rev 3).** Route 3b's install step runs **no LTO**, so the question is not "link-only time under fat LTO" but: (a) wall-clock of the installer's link — the M-5 line plus N mod objects and N `--undefined` roots — over the real workspace's post-LTO object set; (b) for comparison only, the engine vendor's own `lto = "fat"` link time, which is a *build-side* cost the user never pays; (c) route 2's link time if M-C4 ever revives it (`-Clinker-plugin-lto` + `rust-lld`) | build the workspace to objects with `-C save-temps`, then time the final link alone (not a clean build) on the owner's machine; repeat with 1 and with 8 mod-sized objects | **the primary recommendation, on time.** ⚠ Rev 2 posed this as "link-only time under fat LTO", which is not well-posed for a rustc-free installer — fat LTO happens inside rustc |
| **M-C4** ⭐ **rev 3, and it precedes M-C1** | **Does route 3b exist?** (a) take the engine's post-LTO objects (`-C save-temps` under `lto = "fat"`) and link a mod object that calls a non-generic engine function against them — does the symbol survive the engine's own LTO, or was it internalised ([LLVM LTO's must-preserve list](https://llvm.org/docs/LinkTimeOptimization.html); in-tree evidence that fat LTO removes unreferenced items: `REFLECTION-ANALYSIS.md:1477-1494`)? (b) if it did not survive, does an explicit export set (`#[used]` / `#[unsafe(no_mangle)]` / a linker `--undefined` root) bring it back, and what does that export set cost the engine's own codegen (the M-B1 question, in the modding configuration only)? (c) **rev 4, widened (pass 3, W3)**: in BOTH sub-cases — rlibs absent from the installer's line, and rlibs re-added — enumerate every symbol the mod objects leave undefined (`llvm-nm -u` over the mod objects), and for each count its definitions in the final image by demangled name **across all bindings, local (`t`/`d`/`b`) as well as global (`T`/`D`/`B`)**: exactly one, global, or red. That class — not the two hand-named counters — includes every per-type `component_id::ID` cell reachable from a mod-referenced `component_id()` (a **local** symbol after the engine's own fat LTO, M-10 leg 2, so a re-added rlib produces a local+global pair with no linker diagnostic), every `TypeIntern` and dispenser static, and `std`'s own internals. **Red control:** a line with one engine rlib deliberately re-added must make the check fire. **Toy-scale status (M-10):** sub-case 1 fails loudly (7 undefined references, three of them `std`); sub-case 2 fails loudly on the allocator shim before any pair forms | a relink experiment on the real workspace, plus `llvm-nm` (present on this box through the `llvm-tools` component, `lib/rustlib/x86_64-pc-windows-gnu/bin/llvm-nm.exe`) over the mod objects and the resulting image | **whether C's primary ranking rests on anything — expected RED for 3b after M-10 (rev 4).** Red ⇒ C degrades to route 3a, which forfeits fat LTO for the modded build exactly as A′ does while keeping C's install-time, signing and AV risks — i.e. C drops below A′ |
| **M-C2** | Size on disk of shippable engine objects/bitcode vs the 3.46 MB LTO'd binary | `cargo build --release` with `-Cembed-bitcode=yes`, measure the artifact set | whether C's download is acceptable |
| **M-C3** | Antivirus/EDR reaction to relinking an executable at install on a stock Windows 11 box | run the relink under Defender with default settings | a C disqualifier if severe |
| **M-A1** | **Mod-side monomorphisation vs compiled-in**: the same `Query<&A, &mut B>` system, once inside a `cdylib` linking the engine rlibs, once compiled into the engine, same data | criterion, 10k rows, interleaved passes per `docs/BENCHMARKING.md`; report ns/row against the 3.4–4.0 ns/row baseline | **whether A's core premise holds in this kernel.** >5 % delta pushes hard toward C |
| **M-A2** | Cost of installing **all three** TLS slots (`ACTIVE_POOL`, `LANE_DEPOSIT`, `IN_SYSTEM_RUN` — `boyko_threadpool/src/tls.rs:169,196,205`) into the mod's TLS at each system entry. ⚠ Rev 2 priced one write; installing `ACTIVE_POOL` alone leaves a `par_iter` that no longer falls back to serial but deposits through the wrong lane, and leaves the ALLOC1 asserts vacuous (§0.3 instance 5) | microbench the TLS write on windows-gnu — note this machine's measured `thread_local!` cost: 2 lock-prefixed RMWs + `FlsSetValue` under rustc ≥1.98 (`docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:87`) | whether A can offer `par_iter` to mods at all |
| **M-A3** | Startup cost of the mod directory scan + manifest parse with 0 mods present | wall-clock at boot, 20 runs | P2 budget for A/B/E |
| **M-B1** | Cost to the engine's own codegen of making N entry points `#[no_mangle] extern "C"` | build the engine with and without the export set; compare `query_ref_iter`, `swap_remove`, schedule dispatch, and binary size | the size of B's "compiled in, no mod loaded" cost, which cannot be cfg'd to zero |
| **M-B2** | **This engine's own data-representation tax**: the same row loop over a `ComponentPool` column, once as a typed Rust slice walk and once through an FFI-safe `#[repr(C)]` accessor of the shape B would force | criterion, 10k rows, same data, interleaved | replaces the borrowed Tremor ~30 % as B's inner-loop number. Tremor measured a hash-map pipeline, not an ECS column loop, and must not be quoted as if it did |
| **M-F1** | **Option A′ viability AND its P1 risk**: (a) does this workspace build as `crate-type = ["dylib"]` with `-C prefer-dynamic` (the two proc-macro crates, `crates/aether` and `crates/boyko_macros`, stay proc-macro and are not part of the flip); (b) is the PE 65535-export ceiling reached on windows-gnu with this crate graph; **(c) ⭐ rev 3 — after the configuration mechanism is in place, measure the NON-modding build**: binary size against the 3.46 MB fat-LTO baseline and `query_ref_iter/10k` against its own. Leg (c) is not an A′ check, it is the **P1** check: rust#51009's failure mode is that a multi-crate-type lib silently loses LTO for *everything*, with no diagnostic, which would cost the shipped non-modding build geomean 0.793; **(d) rev 4 — the MODDING build on nightly with `-Z dylib-lto`** (M-11 measured the flag feasible on the M-2 rig: dylib fat-LTO'd, `prefer-dynamic` bin with and without its own LTO, statics still shared): run the 7-bench suite on the modding configuration built that way and report where the 1.18×–2.67× / 1.26× moves; the flag is nightly-only and its page says it is "currently only used for compiling `rustc` itself" | flip the crate types on a worktree; count exports on the produced dylibs; then build the non-modding leg from the same tree and compare size + bench against today's | whether A′ exists as a fallback at all (§1.5, §8.2) — **and whether its configuration mechanism charges P1 in silence** |
| **M-F2** | **Option A″ feasibility** (§1.6): can a `cdylib` own one mint counter that two **statically linked, fat-LTO'd** images both mint from correctly (addresses equal, ids interleaving — M-2's check, with only the state shared)? What does the `extern "C"` accessor cost at the frequencies rev 4 re-derived — `storage_kind` per bundle id per entity on the command path (§1.6), not once per archetype — and what would the allocator and TLS halves require? **And (rev 4) how is the component id shared at all**, given `register_new` is mint-only and the derive's per-type cell is a per-image global (M-10 leg 1): the probe must include a `TypeId`-keyed component intern in the shared crate and a two-way mirror of the install tables, or A″ duplicates every engine component id | the M-1/M-2 rig, with the registry moved behind an `extern "C"` seam in a `cdylib` and both sides kept static + LTO'd | whether §1.5's withdrawn "only" should be restored, or whether a third shape offers A′'s correctness at A's speed |
| **M-A4** (rewritten in rev 3) | **The §0.3 enforcement gate itself, with the leg that can discriminate.** (a) the cooperative legs: `trybuild` `compile_fail` per forbidden entry point — from rev 6 (R2) including `App::add_systems` / `add_systems_in` / `add_systems_cfg` / `add_systems_cfg_in` / `add_startup_system` / `add_plugin`, and `ScheduleBuilder::add_system` / `insert_state` / `SystemConfig::run_if` — and `cargo tree -e features` over a mod crate — recorded as cooperative, not as proof; (b) **the mint-counter poison, over all five dispensers (rev 4; pass 3, O1)**: the loader first reads the mod image's `NEXT_ID`, `QUERY_NEXT_ID`, `BUNDLE_NEXT_ID`, `NEXT_RESOURCE_ID` and `NEXT_EVENT_ID` through the `boyko_mod_api` probe and refuses the load if any is non-zero (a constructor-side mint), then atomically stores each cap (`MAX_COMPONENTS`, `MAX_QUERY_TYPES`, `MAX_BUNDLE_TYPES`, `RESOURCE_SLOT_COUNT`, `MAX_EVENTS`) before the mod's init runs; a mod that reaches `world.query::<D, F>()`, `Transform::component_id()`, `resource_id_for::<T>()`, a bundle intern, an event mint, **an `Assets::<T>::with_reserved` / `default()` construction, or a fallible `try_register_tag` (rev 5 — the two refusals rev 4's classifier could not see; §0.3 leg (d)'s consumer table) — and, rev 6, a mod with NO scheduled system that takes `try_register_tag`'s `None` in its init, caught by the loader's post-init latch read (pass 5, W1)** — must mint nothing and must stop the process with an **operator-legible** message — **and it must stop THROUGH the SDK boundary**, which reads the kernel's mint-refusal latch and aborts (§0.3 limit (v)); a red control that keys on message text instead of the latch, or whose panic is merely caught and returned as a status, is the failure this leg exists to catch. The gate checks the operator-visible text as well, because the unmodified panic says "ComponentRegistry exhausted / enable `big_query_table`" (or, for the asset path, "asset ComponentId space exhausted"), which is a lie; (c) a compliant mod must load, run and be unaffected; **(d) (rev 5; pass 4, question 1) a compliant mod system that writes an engine component must have a derived `Access` that conflicts with an engine system's on that write, and the scheduler must serialise the two — a red control is a mod system declared through a non-`SystemParam` façade, which runs concurrently and must be caught**; **(e) (rev 6, R2) a mod system registered through §7.9(c)'s seam runs, its `SystemBox` is allocated in the host image — the host's counting allocator (the `alloc_frame_census.rs` method) sees one config-phase allocation per registered mod system and a counting shim in the mod's image sees none — and world teardown frees it host-side; the red control is the same system registered through `ScheduleBuilder::add_system` from a crate that depends on `boyko_ecs` directly, which must fail leg (a)'s fixture, and, built anyway with the fixture deleted, must show the box allocated in the mod's image** | **the red control must be a mod the engine did not build** — written against `boyko_ecs` directly, not against the façade, with the SDK's own manifest assertion deleted. A gate whose red control is a cooperating crate proves only that the SDK is self-consistent | **whether option A is sound against MISTAKES.** It cannot answer malice — the probe and the poison live in an artifact the author compiles — so the other half is Stage 0's question (iii), who builds the mod |
| **M-A4a** | **Is an UNREACHED `static` in a linked dependency rlib carried into the image?** The decidability measurement this document relies on (`REFLECTION-ANALYSIS.md:1477-1494`) is of a plain *function* (`drop_glue`); no measurement anywhere covers a `static` nothing reaches | link a mod `cdylib` against `boyko_ecs` while touching no registry path; dump the image for `NEXT_ID` / `QUERY_NEXT_ID`; repeat at default release, with `--gc-sections`, and under `lto = "fat", codegen-units = 1` | closes §0.3 leg (c) formally. **Either answer kills the address cross-check** (present ⇒ it fires for every mod; absent ⇒ the façade's own probe materialises it), which is why leg (d) exists — but the answer also tells the loader whether the probe is reading a static the mod would otherwise not have had |
| **M-K1** | **`MAX_COMPONENTS` 512 → 1024** (`ComponentMask` 8 → 16 words, `Access` 192 B → 320 B, `[Column; N]` per archetype) | flip the constant, run the 7-benchmark suite, report the geomean and `query_ref_iter` specifically | reserved band (§7.3a) vs raise-for-everyone (§7.3b) |
| **M-K2** | Dynamic (id-driven column walk) vs typed `Query<&C>` iteration over identical data | criterion, both paths over the same pool | whether an engine-side dynamic query is ever acceptable. **No published number exists in the entire field** (MODDING-RESEARCH §6) |
| **M-T1** | Does a `bin` compiled with `lto = "fat"` link a `dylib` dependency at all? | 10-minute rustc experiment | closes an open question that affects any dylib-shaped variant |
| **M-T2** | Is `TypeId` identical between separately-*configured* builds of the same crate (differing `-C metadata` / features)? | two builds, compare | the last correctness question for any shared-`TypeId` variant. M-1/M-2 could not answer it (both sides linked the same artifact) |
| **M-P1** *(rewritten in rev 6 — R1)* | **The P1 gate itself: pins captured on the PRE-change tree, compared on the POST-change NON-modding build.** Two trees, one configuration — never two configurations of one tree. **Pins, captured at `d552be05` before the first kernel edit and stored beside the gate:** (1) the byte size and the disassembly of the named hot functions — the bodies under `query_ref_iter/10k` and `ComponentPool::swap_remove/10k` (`Cargo.toml:89-99`) and `Schedule::run`'s dispatch (`schedule.rs:286`; worker path `:1380`, dispatcher path `:1215`) — resolved to symbols with `llvm-nm` on the pre-change binary and dumped with `llvm-objdump -d --disassemble-symbols=…` (both in the installed `llvm-tools`, `lib/rustlib/x86_64-pc-windows-gnu/bin/`); (2) the byte size and disassembly of every function a §7 item touches: `register_new::<T>` and `T::component_id()` for a representative engine `T` (`Transform`), `try_register_dynamic`, `register_layout::<T>`, the five dispensers (`mod.rs:920`, `query_type_registry.rs:127`, `bundle_type_registry.rs:116`, `resource_registry.rs:175`, `event_registry.rs:109`), `ScheduleBuilder::add_system::<F, M>` for one engine system; (3) `size_of::<EcsMaster>()`, `size_of::<ComponentPool>()` (pinned 128 / 144 at `component_pool.rs:57`, `:62`), `size_of::<boyko_threadpool::Scope>()` (pinned 384 at `scope.rs:1088`); (4) the count of exported symbols of the executable (`llvm-readobj --coff-exports`); (5) the `.text` size of the executable (`llvm-size`). Every §7.10 kernel delta (items 1, 2b, 3, 5, 6) must keep every pin identical; a delta that moves one is charged to P1 and moves behind the modding crate boundary | this repository's own 0%-gate methodology (`Cargo.toml:84-88`), on a **named deterministic profile** for the disassembly pins (`profile.release` leaves `codegen-units` unset, so partitioning can shift and produce a false red; `profile.bench` is deterministic but `lto = false`, and the tree's own comment forbids quoting its numbers as shipped-profile numbers, `Cargo.toml:113-126`) — so the asm pins prove **codegen equivalence of the source change**, while the size / export / `.text` pins are taken on `profile.release` itself and prove **what the shipped artifact carries**; plus `cargo tree -e features`; plus the source census (`no_mangle` / `extern "C"` / `export_name` / `link_section` over `boyko_ecs`, `boyko_utils`, `boyko_threadpool`, `boyko_log`, `boyko_diag`, `boyko_macros` = 0). **Red controls, required:** a branch that adds one relaxed store to `register_new`'s failing arm must move pin (2) or pin (5); a branch that adds one `pub extern "C"` fn to `boyko_ecs` must move the census; if neither goes red the gate is not a gate. **What pin (4) can and cannot see:** on windows-gnu an executable's export directory is empty with or without a `#[no_mangle]` item (M-10 leg 3), so pin (4) moves only for a deliberate export (a `cdylib`-style table or nightly's `-Z export-executable-symbols`); the source census is the leg that sees the attribute, and `--coff-exports` stays as an M-A4 check on mod DLLs. **Withdrawn (rev 6):** rev 5's two-leg form — byte-identical asm between the modding and non-modding builds of the post-change tree — because items 1, 2b, 3 and 5 land in `boyko_ecs` unconditionally and are present in both legs, so that diff was green by construction (pass 5, C1) | **the owner's hard constraint.** Must be red on the two controls above, or it proves nothing — and it says which claim each pin makes: codegen equivalence of the source change (asm pins) or bytes the shipped binary carries (size / export / `.text` pins) |
| **M-10** ✅ **DONE (rev 4)** | **Route 3b at toy scale, and the binding of the derive-shaped per-type cell under fat LTO** — the mechanism pass 3's W3 could not establish | two crates on stable 1.98.1 / windows-gnu: an rlib with `pub static NEXT_ID`, an `#[inline(never)] register_new`, and an `#[inline] component_id()` holding `static ID: OnceLock` in its body; bins at `-O` and at `-C lto=fat -C codegen-units=1`; a mod object compiled `-O` against the rlib; links with and without the rlib and the 27 sysroot `std` rlibs re-added; `llvm-nm` / `llvm-readobj` from the installed `llvm-tools` component. Research §13.3 has the receipts | (1) in the rlib the cell is a **global** `D eng::component_id::ID` with a `__imp_` alias — one cell per image, referenced by name from every instantiating crate; (2) under fat LTO it, `NEXT_ID` and `register_new` are **local** (`d`/`b`/`t`), unchanged by `-C link-dead-code` or nightly `-Z export-executable-symbols`; (3) a `#[no_mangle]` fn in a bin stays **global** through fat LTO and produces **no PE export entry**; (4) the mod object's undefined list: `register_new`, `__imp__…component_id::ID`, `Once::call`, `unwrap_failed`, `panic_cannot_unwind`; (5) LTO'd host + mod object: **7 undefined references, loud**; (6) + engine rlib: engine refs resolve from the rlib, `std` refs remain; + all `std` rlibs: **undefined `__rustc::__rust_alloc` / `__rust_dealloc` / `__rust_no_alloc_shim_is_unstable_v2`**, loud; (7) a fat-LTO bin's own link line names **2** `.rlib`s vs **21** without LTO. **3b is red; §3, §8.1, §8.3 and M-C4(c) follow** |
| **M-C5** *(rev 5; pass 4, W4)* | **Route 2 at toy scale** — the M-10 rig under `-Clinker-plugin-lto`: (a) does it link on windows-gnu when the installer drives `rust-lld` directly (which flavour, which line — M-5's receipt is for the bundled mingw driver, not for `rust-lld`); (b) does the proc-macro / `prefer-dynamic` error fire without `--target`, and not with it; (c) is `std` code inlined across the plugin link — compare the disassembly of a call into a `std` generic (`OnceLock::get_or_init` is already in the rig) against the fat-LTO bin's, since the rustc page is silent on whether the precompiled sysroot participates and M-10 leg 7 shows a `libstd` copy on today's link line | `eng.rs` + `modx.o` + `-Clinker-plugin-lto` + `rust-lld`, each leg under two seconds; `llvm-objdump` on the result; receipts to research §13 | **whether route 2's throughput claim is a property or a prediction.** Red on (c) drops route 2 *below* today's shipped build on `std`-touching paths while charging 211 MB and whole-engine bitcode; red on (a) means the route has no installer line. Until it runs, route 2 is unplaced in §8.1's ordering |
| **M-K3** *(rev 5; pass 4, C2)* | **The occupancy drift a §7.3 bound must absorb**: `NEXT_ID` and `NEXT_RESOURCE_ID` read at three points on the shipped demo — the config-phase mod-load slot, immediately after `App::finish()`, and after the first frame — plus the count of `register_layout`-pinned slots and the lowest pinned id | the value readers §7.10 item 3 adds, printed once at each point; one run suffices (first-call order is fixed for a fixed binary), repeated on `boyko_demo` and on the playground scene | how far the instantaneous counter sits from eventual occupancy, i.e. how much D2's ceiling or D3's floor has to cover; whether the physics census's 142 is still the number; and whether the host's own tag / asset mints (§7.3, question 4) move it mid-session |
| **M-11** ✅ **DONE (rev 4)** | **`-Z dylib-lto` on nightly — is A′'s LTO forfeit stable's price or Rust's?** | the M-2 rig on `rustc 1.100.0-nightly (8925ea358 2026-08-20)`, windows-gnu: `--crate-type=dylib -C prefer-dynamic -C lto=fat -Z dylib-lto`; a `prefer-dynamic` bin without LTO and one with `-C lto=fat -Z dylib-lto`; stable control | the dylib builds and links, both bins run, the static is shared (same `NEXT_ID` address — M-2's check); M-3's second error does not fire on the LTO'd bin; the stable control reproduces M-3's first error verbatim. **Feasibility only** — whether intra-dylib LTO recovers the 0.793 on the workspace is M-F1 leg (d) |

---

## 11. Open items this document does not decide

- **Trust model** (Stage 0). Values call.
- **Patch-release survival** (Stage 0). Values call; it is the only thing that would make B correct.
- **Whether mods add component *types* at all**, or only data prototypes of engine types (Factorio's
  model: "Prototypes can only be specified during data stage… This is fundamental limitation and its
  never going to change").
  **Rev 2 ruling on its status, though not on the question itself: prototype-only is a SCOPE axis,
  not a sixth option.** It is a restriction on what a mod may *do*, and it composes with A, A′, C or
  D rather than competing with them — which is why it has no column in §6. Its effect is large and
  worth stating precisely: a mod that adds no *types* mints no ids, needs no descriptor, and never
  executes engine registry code, so **§0.3 becomes vacuous and option A's entire enforcement problem
  disappears**. Its cost is equally plain: mod behaviour is limited to data the engine already
  models, so any mod that needs new per-entity state must express it as engine-typed data — which is
  exactly what Factorio's limitation means in practice. This is the cheapest path to a shipping mod
  system and the one that forecloses the most; it is a **scope call for the owner**, and it should be
  made before Stage 3, because choosing it removes §7.9 and most of §7.1 from the plan.
- **Asset loaders.** `HasLoaders::LOADERS` is a const table that explicitly replaced a runtime
  registry (`crates/boyko_ecs/src/ecs/core/asset/loader.rs:58,68`). A mod-provided loader for an
  *engine* asset type cannot be expressed by a const slice. The miss path is cold, so a `cfg`'d
  second scan consulted only on miss may cost nothing measurable — one measurement answers it, and it
  is not queued above because it depends on whether mods ship new file formats at all.
- **Mod shaders.** The RHI already exposes runtime `create_shader_module(&[u32])` /
  `create_compute_pipeline` (`crates/boyko_rhi/src/device.rs:513,523`), but nothing loads SPIR-V from
  a file and the variant manifest assumes a closed set. Mechanically possible, structurally
  unaddressed.
- **Save-file policy for missing mods**: flecs-style silent drop (its `strict` flag defaults to
  lenient), Unity-style hard error, or quarantine-and-preserve. This tree's
  `(FORMAT_VERSION, LAYOUT_FINGERPRINT, stable_name)` triple can express all three; none is chosen.
- **Mod load order and dependencies.** Factorio's prefix vocabulary (`!` incompatible, `?` optional,
  `+` recommended, `~` not-affecting-load-order, bare = hard, with `< <= = >= >`) is the field
  standard and is worth adopting verbatim rather than inventing.
- **Whether `docs/OPEN-QUESTIONS.md` F7** ("mods ratified out of scope 2026-08-30") is being reopened
  by owner decision. This document assumes the question is live; the ratification is not overridden
  silently.
- **The §7.3 bound's mechanism (rev 5; pass 4, C2) — the architect's.** D1 (eager mint pass; not
  available without a type enumeration the tree refuses), D2 (a census-held engine ceiling in
  `boyko_mod_host`), D3 (a downward mod band through a pinned-slot dispenser, collision-checked),
  or D2 + D3. §7.3 states what each has in the tree and which panic text the operator sees when it
  fails; M-K3 supplies the drift number. Also open inside it: whether the unit is a per-mod quota
  or one budget for the mod set. *(Rev 6: still the architect's — pass 5's question 3; if D3 is
  chosen, W3's diagnostics clause in §7.3 rides with it.)*
- **The mint-refusal signal's read site (rev 5; pass 4, C1) — the architect's, narrowed in rev 6.**
  §0.3 limit (v) fixes the requirement (a structural, text-independent signal raised by every
  refusal path) and proposes a per-image latch. Decided in rev 6 (pass 5, W1): the loader reads the
  latch once after the mod's init returns, so a zero-system mod has a read site. Still open: whether
  the SDK also reads it in the trampoline's `catch_unwind`, in a panic hook installed in the mod
  image's own `std`, and whether a per-mod-system-run read is added. **Also open (rev 6; pass 5,
  question 4): whether item 2b stays in the kernel at all.** R1 makes that a measurement — the
  latch is admitted only while M-P1's pins hold — and if they move, the boundary-side form in §7.10
  item 2b (poison at `cap + 1`, counter read-back through the probe; no kernel delta) with its
  stated gap on `try_register_dynamic`'s `None` is the fallback to ratify or replace.

---

## 12. Revision log

### rev 2 — architecture-critique pass 1 (2 critical, 5 important, 3 optional)

Every point below was re-verified against the source before the text changed; the two corrections at
the end are cases where the critique itself was wrong on a citation and the tree said otherwise.

| # | Change | Why |
|---|---|---|
| **C1** | New **§0.3** (the duplicated-statics class, its four traced instances, the decidable (i)/(ii) partition, the façade + by-stable-name mint seam, and the three-leg enforcement gate). §0.2 constraint 2 restated as a class. §1 Mechanism, allocator row, failure modes 1/2/3 and new 8 rewritten. New **§7.9** kernel item. §8.1 demotes A from fallback to third; §8.2 states that M-A1 **cannot see A's defect**. New **M-A4** | Rev 1 treated the hazard as two named leaks with two point fixes. Traced end to end: `EcsMaster::query::<D,F>` (`ecs_master.rs:825`) is generic and instantiated in the mod; `query_type_id()` (`query_type_registry.rs:235-241`) mints from the **mod's** `QUERY_NEXT_ID` (`:98`); the id then indexes the **host's** cache through `get_unchecked` (`:385`) whose payload is cast to `QueryDataState<D,F>` (`:855`) and written through (`:880`). Host slot 0 holds a different `(D,F)` ⇒ **type confusion, release, silent**. Same class: `STORAGE_KIND`/`RESIDENCY_CLASS` defaults (`mod.rs:399-410`, `:579`), the per-type allocator decision (`ALLOCATOR-DESIGN-SPACE.md:63`), and `LANE_DEPOSIT`/`IN_SYSTEM_RUN` beside `ACTIVE_POOL` (`tls.rs:196,205`) |
| **C2** | §3 "Kernel features it needs" now names the plugin mechanism, its three candidate routes and the two in-tree refusals; §3 "Toolchain and distribution" rewritten around four **measured** results; §3 failure mode 7 added; §6 cells corrected; §8.1 and §8.2 rewritten; Stage 2 no longer presupposes it; **M-C0 marked DONE**, **M-C0b** added | Rev 1's C rested on two unstated steps. Both were probed on this exact toolchain: (a) **a rustc-free relink works** — `rustc --print link-args` prints the line on stable, and the toolchain's own bundled `self-contained/x86_64-w64-mingw32-gcc.exe` replays it (ship the `symbols.o` rustc writes to a temp dir); (b) **a PE grouped-section plugin registry works**, but with `--gc-sections` on the line it **silently collects nothing** — rescued by `-Wl,--undefined=<registration static>` per mod (rooting the entry *function* was measured insufficient). C therefore does **not** collapse into D, and its kernel work is **not** "the fewest of any option" |
| **W1** | New **§1.5 Option A′** (engine as a Rust `dylib`, modding configuration only) + **§6.1** comparison + **M-F1** + a fallback row in §8.2 | The route was in research §2.1 but dropped from the option set. M-2 already measured that it makes every static shared (`mod NEXT_ID addr == host NEXT_ID addr`), so it **dissolves §0.3 by construction**. §0.2 constraint 1 binds only the build a modded session runs, and the owner's constraint is about the build a **non-modding game ships** |
| **W2** | §7.1 rewritten: the descriptor is an **input struct** that fans out into the existing parallel cold tables; `ComponentLayout` untouched; TRIPWIRE 2 added to the Stage 1 gate list. §1's ⚠ item aligned | Rev 1 said "extend the descriptor" in §7.1 and "must not widen `ComponentLayout`" in §1 without saying they are different structs. All eight named fields already live in parallel tables (`clone.rs:21,82`, `serialize.rs:16,35,122`, `mod.rs:363`, `:503`, `:221`) precisely to hold the 56 B pin (`mod.rs:133`), and `ComponentLayout` is hot (`:104-121`) |
| **W3** | §7.3 corrected: the band costs **capacity**, not time; it must be a **per-configuration** constant or the price must be stated with its number. Stage 1 and M-K1 updated | `try_register_dynamic` CASes the **same** `NEXT_ID` as `register_new` (`mod.rs:968` vs `:214`), so the band partitions one fixed budget. **128 of 512 component slots are already reserved** by the physics scratch band (`scratch_ids.rs:685`), against ~126 in use; resources are ~145 of 256 |
| **W4** | Stage 3 and M-P1 now name the profile and state which claim the gate makes; a second `profile.release` leg added | `profile.release` leaves `codegen-units` unset deliberately (cu=1 under fat LTO regresses `query_ref_iter/10k` **17.4 %**); `profile.bench` is deterministic but sets `lto = false`, and the tree's own comment forbids quoting bench numbers as shipped-profile numbers (`Cargo.toml:113-126`) |
| **W5** | §6: the Tremor ~30 % moved out of "Mod inner loop vs native" into its own row, labelled *measured elsewhere, not for an ECS column loop*; §8.1's two uses of the number qualified in place, with B's rejection restated as resting on its other three grounds; **M-B2** added | Research §10 item 7 states it correctly — whole-pipeline, `halfbrown`→`RHashMap`, plugins **statically linked**. Transcribing it into an inner-loop cell asserts something nothing measured |
| **O1** | §1 failure mode 1 rewritten: the canary **can** be read without executing mod code (PE export directory from the file, or `LOAD_LIBRARY_AS_DATAFILE` / `DONT_RESOLVE_DLL_REFERENCES`); the tree already owns raw `LoadLibraryA`/`GetProcAddress` (`device.rs:1660`) | The canary is A's only defence against the SIMD-ABI mismatch, so closing it as residual without testing was too quick |
| **O2** | §3 now states that `-Clinker-plugin-lto` ships **whole-engine LLVM bitcode to every player** — wider disclosure than the rlib SDK A gives registered authors | Rev 1 flagged IP exposure only for D |
| **O3** | §1 "Toolchain and distribution cost": the SDK must ship the **cargo configuration**, not only the canary | `-C target-cpu=x86-64-v3` lives in `.cargo/config.toml:85,88,91` and does not follow a mod built in its own workspace; a `RUSTFLAGS` env var **replaces** config rustflags |
| **Q5** | §11 rules on the *status* of Factorio's prototype-only model: a **scope axis**, composable with A/A′/C/D, not a sixth option — and notes that choosing it makes §0.3 vacuous and removes §7.9 | The critique asked why it was an open item rather than an option; the answer is that it is not a linkage route |
| **Q7** | §8.3 now states the consequence: a dense type is **always `ResidencyKind::Cpu`** and excluded from every archetype signature (`component/component.rs:67-79`), so **a mod component can never be GPU-resident** | §7.4 requires dense storage for mod components on engine entities; the GPU consequence was unstated |

**Two corrections to the critique itself**, both verified:

1. The critique cited `REFLECTION-ANALYSIS.md:172-176` for the deliberate avoidance of the
   `linkme`/`inventory` hazard class. **That is the wrong location** — `:165-185` is the
   feature-unification narrowing and a `Cargo.toml` snippet. The actual refusal is at **`:307-311`**
   ("**NOT `linkme`/`inventory`** … sidesteps the entire `linkme` + `--gc-sections` dead-strip /
   init-order hazard class"), restated at **`:742`**. The corrected citations are used above.
2. The critique's grep dismissed the `boyko_log` hit as an unrelated use of the word *inventory*. It
   is **not** unrelated: `crates/boyko_log/src/target.rs:978-982` is a **second, independent in-tree
   refusal** of exactly this mechanism ("inventing one — a linker section, an `inventory`-style
   registry — would add a runtime facility the engine does not otherwise have"). It strengthens C2
   rather than weakening it, and it is now cited in §3.

**One change rev 2 made that the critique did not ask for**, because resolving C2 created it:
§3's "P2 collapses into P1" for option C is now stated with its residue. A linker-collected
registry is a mechanism *in the shipped binary* — two sentinel statics and a boot walk — so a
zero-mod C build is not literally the non-modding build. The per-mod `--undefined` root (M-7) is
chosen over dropping `--gc-sections` precisely to confine that residue to 16 bytes, and
**M-C0b** measures whether it is truly nothing on the real workspace. The *non-modding* build is
unaffected either way: the sentinel crate is simply absent from its dependency closure.

**What rev 2 did NOT change.** The §7.8 no-`dyn`-on-typed-paths invariant, the P1/P2/P3
decomposition, the refusal of a symbol census as the P1 gate, separate-crate-not-cargo-feature, the
§0.1 reframing (re-checked: `iters/query_state.rs` touches no process-global registry in steady
state, so a mod's *iteration* really is native), and the rejection of Wasm on 128-bit SIMD rather
than on a percentage. All five survived the pass unchanged.

---

### rev 3 — architecture-critique pass 2 (3 critical, 4 important, 2 optional)

Scope was the rev-2 delta. Every point was re-verified against the source or the installed toolchain
**before** the text changed; where the verification changed the answer the critique proposed, the
difference is stated in the row rather than smoothed over.

| # | Change | Why, and what was verified |
|---|---|---|
| **C1** | **§3 now names the link route C ships.** New route table (1 rustc fat LTO ⇒ that *is* option D · 2 linker-plugin LTO · 3 plain-object relink), with the shipped form split into **3a** (rejected: it forfeits fat LTO exactly as A′ does) and **3b** (ship an already-fat-LTO'd engine object set, plain-link the mod objects onto it). "What 'as fast as native' means" rewritten in three parts; **the claim that the measured geomean 0.793 "applies to the whole modded binary" is withdrawn**. Bitcode disclosure re-scoped to route 2 only; the 48 MB disk figure labelled route 3's; §6 cells, §8.1's number, §8.2's C row and §8.3 updated; **M-C4** added and placed *before* M-C1; M-C0 marked route-3-scoped; **M-C1 re-posed** (a rustc-free installer runs no LTO, so "link-only time under fat LTO" was not well-posed) | Verified on disk, 2026-09-16 (recorded as **M-9**, research §13.2): the windows-gnu toolchain contains **no** LLVM linker plugin (`*gold*` absent), and its only plugin-capable linker is `rust-lld.exe` at **211 187 378 B** — ~4× the whole 48 MB set M-8 measured. The four `gcc-ld/*.exe` are 857 088 B, byte-identical to one another, and their strings name `rust-lld`: shims, not linkers. The rustc book supplies the requirement (*"a linker with the LLVM plugin must be used (e.g. LLD)"*, else an explicit `LLVMgold.so` path) and, separately, the proc-macro/Windows error that this workspace's two proc-macro crates would meet. M-5/M-6/M-7 are receipts for a **plain** link. Two 3b properties are *predictions* and are labelled as such: LTO internalises what the mod will reference (LLVM's must-preserve list is supplied by the linker, and the mod objects do not exist yet), and adding the rlibs back may put **two** copies of a registry static in one image |
| **C2** | **§0.3's enforcement gate rewritten.** Legs (a)/(b) reclassified as **cooperative**; **leg (c) (the load-time address cross-check) withdrawn as non-discriminating**; new **leg (d) — the loader poisons the mod image's mint counters** so an out-of-contract mint panics *before* its id is used. Limits (i)–(iii) stated, including the lying diagnostic. §1 failure mode 2, §6.1, §8.1 and §8.2's A row updated; **M-A4 rewritten** around a red control the engine did not build, **M-A4a** added; Stage 0 gains a third values question, *who builds the mod* | Leg (c) fails on the document's own citation: a dependency rlib's items are carried into the image whether or not anything reaches them (`REFLECTION-ANALYSIS.md:1477-1480` — `--gc-sections` *"no effect whatsoever"* at default release, decidable only under `lto="fat", codegen-units=1`), so the address differs for compliant and non-compliant mods alike. Leg (d) works because both dispensers already fail loudly at their cap and do so **before returning the id**: `register_new::<T>()` carries a **release** `assert!` (`component_registry/mod.rs:921-926`) and `query_type_registry::register_new()` saturates then panics (`:132-145`). Verified also that mirroring the host's tables into the mod would **not** work — engine ids are minted lazily through `component_id()`'s `OnceLock`, so the host's tables are incomplete at load and drift after it |
| **C3** | **§1.5 now states A′'s configuration mechanism and the P1 gate is restored.** Three mechanisms tabled — (i) `crate-type = ["rlib", "dylib"]`, (ii) a duplicated manifest set, (iii) `cargo rustc --crate-type` (insufficient) — with (i)'s **silent** failure mode named. The waiver *"P1 is trivially green and needs no asm diff at all"* is **withdrawn**; **M-F1 gains leg (c)**, which measures the **non-modding** build (size against the 3.46 MB fat-LTO baseline + `query_ref_iter`); §6.1's "Non-modding build" cell and §8.2's A′ row follow | Verified: **no `crate-type` key exists anywhere in the workspace** (grep over `crates/**/*.toml`; the only related keys are `proc-macro = true` in `crates/aether` and `crates/boyko_macros`). `cargo rustc --crate-type` *"only works when building a `lib` or `example` library target"* and overrides only the selected package ([cargo-rustc](https://doc.rust-lang.org/cargo/commands/cargo-rustc.html)). The hazard is real and reported open: *"LTO ignored for all crate types when building multiple crate types and one doesn't support it"* ([rust#51009](https://github.com/rust-lang/rust/issues/51009)); cargo's special case covers cdylib/staticlib, not rlib+dylib ([cargo#8254](https://github.com/rust-lang/cargo/pull/8254)) |
| **W1** | **§0.3 gains a fifth instance and it is ranked second, ahead of three of rev 2's four** — the per-type `component_id()` mint and the derive's install arms; old instances 2/3/4 renumbered to 3/4/5 and every cross-reference updated. **§7.9 widened**: the by-stable-name seam must cover **engine** component types, and must expose the counter probe leg (d) poisons | `component_id()` is `#[inline]` with its `static ID: OnceLock<…>` **inside the fn body** (`crates/boyko_macros/src/component.rs:371-372`), minting at `:375` and running six `#…_install` arms at `:387-392`. For an engine type that impl lives in the rlib the mod links, so the mod gets its own id cell *and* its own `STORAGE_KIND`/`CLONE`/`SERIALIZE` rows. It is reached from inside the engine's own `QueryData` impls, which is why the façade cannot re-export them |
| **W2** | **The 0.793 is now always quoted with its denominator.** §1.5's price paragraph, §6.1's throughput row (relabelled as the bench suite, not a session) and §8.3's decided-regardless list | 0.793 is the geomean over **7 ECS microbenchmarks** (`Cargo.toml:91-99`); of the three ratios the profile comment names the spread is 1.18× (`add fill` 0.849) to **2.67×** (`query_ref_iter` 0.375), with `swap_remove` 1.53×. Rev 2's own W5 forbade exactly this transcription and rev 2 then did it to its own number |
| **W3** | **§7.6 bounded.** The `Result` requirement covers registration, load and attribution; the engine's **param-fetch panic policy is explicitly out of scope** | The opening sentence names a per-system-run path (`missing_resource_panic::<R>()`, `system/params/res.rs:132`, `#[cold]`) while the requirement names two cold ones. Unbounded, item 6 could be read as licence for a fallible `Res<T>` — a P1 violation arriving through a section that is not §7.8 |
| **W4** | **"The only option that dissolves §0.3" is withdrawn**, and the alternative it excluded is written up as **§1.6 option A″** — the duplicated state behind one `extern "C"` `cdylib`, all engine *code* still static and fat-LTO'd on both sides — with its four unknowns and **M-F2** | Verified that the registry is **not** on the row loop: `ComponentPool` caches `component_layout`, `drop_fn` and `component_type_id` in its own header (*"Component layout (cached from registry for performance)"*, `crates/boyko_ecs/src/ecs/memory/component_pool.rs:194-195`, `:234-240`), so the accessor calls would land once per type, once per system run and once per archetype construction — M-4's ~1.7–2.5 ns is invisible there. The allocator and TLS halves remain unpriced, which is why A″ is written as a candidate, not as a fourth option |
| **O1** | **§7.3 now prices the THIRD budget** — query shapes — and states that its only in-tree remedy is a cargo feature the modding design forbids | `MAX_QUERY_TYPES = 1024` (`query_type_registry.rs:84-85`), saturate-then-panic (`:132-145`), 75 % warning (`:149-160`), and the panic text tells the operator to enable **`big_query_table`** (`:141`) — a cargo feature on `boyko_ecs`, i.e. the mechanism §8.3 rules out for modding. It also quadruples the per-world cache: *"≤ 32 KB at `MAX_QUERY_TYPES = 1024` … ≤ 128 KB with the `big_query_table` feature"* (`ecs_master.rs:327-331`) |
| **O2** | **§1's TLS prescription widened to all three slots**, with the middle state named as worse than either end; M-A2 re-scoped to three writes | `ACTIVE_POOL` / `LANE_DEPOSIT` / `IN_SYSTEM_RUN` (`boyko_threadpool/src/tls.rs:169,196,205`). Installing only the first gives a `par_iter` that no longer runs serial but deposits through the wrong lane, with the ALLOC1 asserts still vacuous |

**What rev 3 did NOT change.** The P1/P2/P3 decomposition; the refusal of a symbol census as the P1
gate (extended, not weakened — it is now also the reason leg (c) fails); separate-crate-not-cargo-
feature; §0.1's structural claim and the (i)/(ii) partition (re-checked: `ComponentPool` caches
layout and drop glue, so the row loop reads no registry static at all — the partition is if anything
better founded than rev 2 argued); the descriptor-as-input-struct ruling and TRIPWIRE 2 as a Stage 1
gate; W4's profile discipline; the rejection of Wasm on 128-bit SIMD; and the ranking itself —
**C stays primary, A′ the fallback, A third** — though C's *reason* is now a narrower and more
defensible claim than rev 2's, and both C and A′ now carry a gate that can fail (M-C4, M-F1 leg (c))
where rev 2 had an assertion.

**One thing rev 3 changed that pass 2 did not ask for**, because resolving C1 created it: §3's kernel
list gains a **mod-facing export set** for route 3b. If the engine's own fat LTO internalises what
mod objects reference, C needs the same pinned-entry-point set option B needs — with the mitigation
that it exists only in the modding configuration and therefore does not touch P1. That is B's M-B1
cost in a smaller form, and it is queued inside M-C4 rather than left to the first failed relink.

---

### rev 4 — architecture-critique pass 3 (1 critical, 3 important, 3 optional, 4 questions)

Scope was the rev-3 delta plus pass 2's blockers. Every point was re-verified against the source
before the text changed, and two of them were **measured** rather than argued (M-10, M-11 — research
§13.3), because the mechanism pass 3 marked PLAUSIBLE was a two-second probe away.

| # | Change | Why, and what was verified |
|---|---|---|
| **C1** | **New §7.10 — the configuration mechanism per item, and the rule it obeys**: the kernel may gain visibility and cold Rust-ABI functions, never an attribute, `cfg`, feature or per-configuration constant; scoping is by dependency edge only. §7.1 rewritten as a visibility change plus two by-value setters with the `extern "C"` entry in `boyko_mod_host`; §7.9(b) now places the exported probe in `boyko_mod_api` (the mod's image) over five `#[doc(hidden)] pub` counters and shows the per-type cells need no probe; §7.3's per-configuration constant **withdrawn** in favour of a loader-side quota over the fixed 512 (pass 3's question 1, the loader reading); §3's export set stated as a dependency edge from the modding binary. §1's table rows, §3 "Cost when modding is NOT used", §8.3, Stage 1 and M-P1 (a source census + `llvm-readobj --coff-exports` leg) updated | `boyko_ecs` has **0** `no_mangle` / `extern "C"` items (grep at `d552be05`); `MAX_COMPONENTS` is an unconditional `pub const` (`mod.rs:63`); the only per-configuration constant in the tree is `MAX_QUERY_TYPES` under `#[cfg(feature = "big_query_table")]` (`query_type_registry.rs:84-90`) — the forbidden mechanism; the counters are private statics (`mod.rs:214`, `query_type_registry.rs:98`, `bundle_type_registry.rs:93`, `resource_registry.rs:127`, `event_registry.rs:93`); the setters a descriptor needs are `pub(crate)` (`set_storage_kind` `:435`, `set_residency_class` `:614`, `install_map_entities_fn` `clone.rs:181`, `try_register_dynamic` `:967`) and the clone/serialize installers are generic (`clone.rs:161`, `serialize.rs:204`). **Measured (M-10 leg 3):** a `#[no_mangle]` item in a windows-gnu bin produces **no PE export entry**, so the export check is decidable where the presence census was not |
| **W1** | **§1.6 A″'s frequency claim corrected from the real call sites**, and a second defect added: sharing the dispensers does not share component ids, and the install tables diverge both ways. "What it would buy" restated; M-F2 widened | `storage_kind` is read per entity / per command, not per archetype: `component_api.rs:201,274,467,657,765`, `entity_api.rs:111,123`, `insert_command.rs:135,256`, `remove_command.rs:87`, `migration_helpers.rs:461,469,660,777`, `spawn_batch_command.rs:586` (the enable/filter reads at `filter_enable.rs:166-177` / `data_is_enabled.rs:159` are `init_state` + `debug_assert!`, so §0.1 holds). `register_new` is mint-only (`mod.rs:920-945`); the de-dup is the derive's per-type cell (`component.rs:372`), which M-10 leg 1 shows as a per-image global |
| **W2** | **§0.3 leg (d) gains limit (v)**: the SDK-emitted trampoline classifies a caught exhaustion payload and **aborts**; the per-system-run counter re-read is rejected (per-run cost, and blind to the two saturating dispensers). §1's Panic row and M-A4 leg (b) updated | The mint panic fires inside a non-poisoning `OnceLock` closure (`component.rs:374-394`; `bundle_type_registry.rs:106-109`, `query_type_registry.rs:51-53`) and would meet §1's mandated `catch_unwind`. `QUERY_NEXT_ID` and `BUNDLE_NEXT_ID` store the cap back before panicking (`:135`, `:128`); the other three `fetch_add` then assert, so only they leave `cap + 1` behind |
| **W3** | **M-C4(c) widened** to the class (every symbol the mod objects leave undefined; definitions counted across local and global bindings; both sub-cases; a red control) — **and the PLAUSIBLE mechanism was measured (M-10)**: the cell is local after fat LTO, a re-added rlib supplies a global one, and the line that would produce the pair fails first on the allocator shim. 3b withdrawn in §3; §6 / §6.1 / §8.1 / §8.2 / §8.3 / Stage 2 follow | M-10 legs 1–7 (research §13.3). The consequence — 3b contradicts itself, because compiling against the rlibs produces the references and fat LTO removes their definitions — is a property of rustc's LTO internalisation, not of workspace size; the workspace-scale M-C4 stays queued with an expected result rather than an open question |
| **O1** | M-A4 leg (b) names all five dispensers and their caps | Verified all five assert before returning: `mod.rs:922-927`, `query_type_registry.rs:133-145`, `bundle_type_registry.rs:121-136`, `resource_registry.rs:176-180`, `event_registry.rs:112-116` |
| **O2** | The proc-macro / `-Clinker-plugin-lto` error is labelled **unverified for windows-gnu** in the §3 route table and §8.3, matching research §13.2 — and the sidestep is stated with its citation: with `--target`, cargo does not pass `RUSTFLAGS` to host artifacts | [cargo config, `build.rustflags`](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags): *"Things being built for the host, such as build scripts or proc macros, will not receive the args"*. (Research §14 said the item was "on the list below"; it was not — added there) |
| **O3** | §0.3 limit (i) and §7.3 state that `register_layout`-pinned ids are outside every mint and every band | `register_layout::<T>(id)` writes `LAYOUTS[id]` without touching `NEXT_ID` (`mod.rs:1031-1056`); physics uses it for constants from `MAX_COMPONENTS - 1` down to `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128` (`scratch_ids.rs:653-659`, `:713-718`, `:685`) |
| **Q1–Q3** | Answered in §7.3 (the loader's band, no constant), §7.9(b) (probe in `boyko_mod_api`, counters `pub` in the kernel, no per-type probe needed) and §0.3 limit (iv) (an atomic `store` through the probed `&AtomicUsize`, preceded by a read-must-be-zero) | as above |
| **Q4** | **New §5.1 option F — out-of-process native mods**, written as *not considered* rather than excluded, with the four costs a measurement would have to price | `ComponentPool` storage is `VmReservation` over `VirtualAlloc` (`memory/vm.rs`; the crate manifest's own comment), not a file mapping — so shared-memory columns are a storage-layer change, not a loader feature |

**Two things rev 4 found that pass 3 did not ask for.**

1. **Route 3b is red at toy scale (M-10)**, which suspends C's primary ranking under §8.2's own rule
   and moves C's live forms to route 2 or 3a. Pass 3 asked for M-C4(c)'s scope to be widened; the
   widening was cheapest to specify by running the toy form of the check, and the toy answered the
   larger question. The re-ranking is left to the architect with the three inputs it needs named
   (§8.1).
2. **A′'s LTO forfeit is stable's price, not Rust's (M-11).** M-3's own error text names
   `-Zdylib-lto`; on the installed nightly the M-2 rig builds, links, runs and still shares its
   statics with fat LTO inside the dylib. Whether that moves the workspace number is M-F1 leg (d);
   nothing in the ranking changes until it is run.

**What rev 4 did NOT change.** The P1/P2/P3 decomposition; the refusal of a whole-image symbol census
as the P1 gate (the new census is over *source* and over the *PE export directory*, both measured
decidable); separate-crate-not-cargo-feature, now generalised to "by dependency edge only"; §0.1's
partition (re-checked a third time: the enable/filter `storage_kind` reads are `init_state` +
`debug_assert!`); leg (d) itself, including its escalation to Stage 0; the descriptor-as-input-struct
ruling and TRIPWIRE 2; and the rejection of Wasm on 128-bit SIMD. The ranking's *reasons* stand; the
ranking's *first entry* is suspended on a measurement the document already named as the one that
would overturn it.

---

### rev 5 — architecture-critique pass 4 (2 critical, 4 important, 2 optional, 4 questions)

Scope was the rev-4 delta plus pass 3's blockers. Every point was re-traced in `D:/wt/joltab` at
`d552be05` before the text changed; no measurement was taken, two are queued (M-C5, M-K3), and one
external page was re-read. Where the trace changed the critique's own claim, the row says so.

| # | Change | Why, and what was verified |
|---|---|---|
| **C1** | **§0.3 gains instance 6 — the asset-layout intern and its mint path — and leg (d) is re-derived from the CONSUMERS of each counter, not from the two dispensers that motivated it.** The façade's forbidden set gains the `Assets<T>` constructors, `AssetBacking::register_layout`, `register_asset_layout` and `impl_asset_pod_backing!` (each a `compile_fail` fixture). Leg (d) now carries a consumer table for `NEXT_ID` with its **four** exhaustion behaviours, states its guarantee precisely (no id is ever minted — all consumers; the attempt is observable — by panic for four, by a returned `None` for the fifth), and **limit (v)'s classifier keyed on five panic strings is withdrawn** for a structural signal: a per-image mint-refusal latch raised by all nine refusal paths, read at the SDK boundary, rowed in §7.10 as item 2b with its cost to every build. §7.9(b)'s "every dispenser asserts before it returns" corrected; §1's dynamic-registration and panic rows, §0.3's closing verdict, Stage 1, M-A4 legs (b)/(c) updated; limit (iv) answers question 3 | `Assets::<T>::with_reserved` is `pub`, `#[inline]`, generic, and calls `T::register_layout()` first (`assets.rs:231-232`; `default()` is `with_reserved(0)`, `:1052`); `impl_asset_pod_backing!` is `#[macro_export]` and implements it as `register_asset_layout::<T>(None)` (`backing.rs:142-160`), which is `pub` (`:115`), re-exported (`asset/mod.rs:60`), locks `ASSET_LAYOUTS` (`:87`) and mints through `try_register_dynamic` (`:131`) with `.expect("invariant: asset ComponentId space exhausted (MAX_COMPONENTS)")` (`:132`). `try_register_dynamic` returns `None` at the cap (`mod.rs:970-972`; doc `:951-953`). Its other caller is `try_register_tag_by_name` (`tags.rs:182-197`, enable tags via `:134-139`), whose `pub` wrappers either panic (`tag_api.rs:245`, `enable_tag_api.rs:335`) or **return `None` with no panic** (`tag_api.rs:47`, `enable_tag_api.rs:73`). The collision texts (`mod.rs:938-944`, `:996-1008`, `:1046-1051`) were likewise unclassified. Grep of the other four counters shows one consumer each (`query_type_registry.rs:132`, `bundle_type_registry.rs:120`, `resource_registry.rs:175`, `event_registry.rs:110`). Corrected in the critique's own favour: the poison **does** prevent corruption on the asset path (no id is returned before the `.expect`); what fails is loudness, exactly as stated |
| **C2** | **§7.3's bound is withdrawn and re-opened.** The instantaneous-counter check (`next_id + quota` at load) is replaced by: what is decided regardless (no constant, no reservation, a loader-owned `Result` before any mint, the physics census cited as the constraint any bound spends, the operator-visible text named for each failure), and three traced directions for the architect — D1 eager mint (not available: no type enumeration exists and the tree refuses one), D2 a census-held engine ceiling in `boyko_mod_host`, D3 a downward mod band through a pinned-slot dispenser with the existing collision check as backstop. Question 4 folded in (host tag and asset mints move the counter mid-session). §7.10 item 3, §8.2's row, Stage 1, §11 updated; **M-K3** queued | Engine ids mint lazily on first touch (`mod.rs:911-914`; counter doc `:212-214`); the load slot is the config phase before `finish()` (`app/app.rs:550`, `:583`), and `finish()` builds the schedules (`:614`), which is when `SystemParam::init_state` resolves queried types. `SCRATCH_REGION_MIN_ID = MAX_COMPONENTS - 128` (`scratch_ids.rs:685`) is census-sized (`:671-684`, "384 against a measured ~142 … re-run before moving this number"); its slots are pinned by `register_layout` (`mod.rs:1031-1056`) from constructors `PhysicsPlugin::build` runs at config time (`plugin.rs:465-471` → `resources.rs:3779`, `:569`), and `PhysicsPlugin` is not in `EnginePlugins` (`plugins.rs:379-547`), so the collision order is the game's. The three failure texts are `register_new`'s (`:938-944`), `dynamic_slot_occupied_panic`'s (`:996-1008`, which blames "test-only `register_layout`") and `register_layout`'s (`:1046-1051`). No enumeration of `#[derive(Component)]` types exists (`REFLECTION-ANALYSIS.md:307-311`, `target.rs:978-982`); `STABLE_NAME_INDEX` is written by the mint (`serialize.rs:383`) |
| **W1** | **The `llvm-readobj --coff-exports` leg on the executable is withdrawn from §7.10, §8.3, Stage 1 and M-P1; the source census is widened to every kernel crate a non-modding game links, the derive included.** M-10 leg 2 cited in §7.10 as the visibility deltas' support | Research §13.3 leg 3: on stable the exe's export directory is empty *with* a `#[no_mangle]` item present; only nightly's `-Z export-executable-symbols` (leg 2c) produces the one export. A leg green on both sides of the violation cannot gate it. Census re-run at `d552be05`: 0 hits in each of `boyko_ecs`, `boyko_utils`, `boyko_threadpool`, `boyko_log`, `boyko_diag`, `boyko_macros`; `boyko_ecs`'s `[dependencies]` names the first four |
| **W2** | **§7.9(a)'s two entry points are rowed in §7.10 (item 5), and the rule gains the clause for a `pub` item that is unsound if misused**: `unsafe fn`, `// SAFETY:` contract, `#[doc(hidden)]` seam, no crate-root re-export — or a typed helper that hides the pairing | `QueryCacheSlot = (NonNull<()>, fn(NonNull<()>))` (`ecs_master.rs:317`), freed via `Box::from_raw` (`:335-338`, `:390-400`), later `cast()` and written through (`:855`, `:880`) under `debug_assert!` bound checks only (`:381`, `:843`); `query_cold_init` (`:947`) is the in-kernel writer whose shape the installer copies |
| **W3** | **§1's allocator rule restated as a property of who frees** — the allocating image frees, through its own code, kept mapped for the allocation's lifetime — with the query-state hand-off named as the sanctioned instance and §7.7 supplying the "stays mapped" half | The rule and the façade it forbade were in the same section; the drop path really does run the mod's `drop_fn` from the host's `Drop` (`ecs_master.rs:390-400`), and a host-side allocation cannot serve a layout monomorphised for types the host has never seen |
| **W4** | **Route 2 is labelled a prediction and taken out of §8.1's ordering** until **M-C5** (queued: link via `rust-lld`, the proc-macro error, `std` inlining at toy scale) returns; §8.2's C row names it | The rustc linker-plugin-LTO page was re-read: it covers the linker requirement, LLVM matching and the proc-macro error and contains no occurrence of "std", "sysroot" or "precompiled"; M-10 leg 7 shows a temp-directory `libstd` rlib on today's fat-LTO link line, so whether `std` reaches a plugin link as bitcode is unestablished |
| **O1** | **§7.9(b) split**: `pub` value readers for §7.3, address accessors under a hidden `unsafe` seam for the poison; §7.10 item 2 rewritten accordingly | `boyko_mod_api` links the same `boyko_ecs` rlib every game links, so any `pub` address accessor is reachable by every crate in the graph — a `NEXT_ID.store` capability no build has today |
| **O2** | **§7.1 fixes the relationship** between the by-value installers and the generic ones: either form is admissible, neither touches a hot function, and the gate is M-P1's asm diff rather than the reachability argument; §7.10 item 1's cell rewritten | `install_clone_fn` / `install_serialize_fn` are generic (`clone.rs:161`, `serialize.rs:204`) and called by the derive (`component.rs:387-392`) once per type inside `component_id()`'s `OnceLock` |
| **Q1** | `ModQuery<D, F>` implements the kernel's `SystemParam` so a mod system's `Access` is *derived*; added to §0.3 partition (ii), §7.9(a), §7.10 item 5 and M-A4 leg (d) | `Access` is filled by each param's `init_access` (`system_param.rs:159-164`) via `add_component_read` / `add_component_write` (`iters/query/data/read.rs:105`, `data/mut_.rs:234`); the scheduler reads it through `System::access` (`system.rs:70`, `schedule_builder.rs:399`) |
| **Q2** | §7.7 pins the two invariants: no `FreeLibrary` on a mod image, every world dropped before the image | The tree's only `FreeLibrary` is the Vulkan loader's (`device.rs:1697`); the runner borrows `&mut App` (`runner.rs:158`), so worlds drop before `main` returns |
| **Q3** | §0.3 limit (iv): `ASSET_LAYOUTS` needs no zero-read | It is `OnceLock<Mutex<HashMap<TypeId, ComponentId>>>` (`backing.rs:87`), populated only with an id `try_register_dynamic` has already returned — a constructor-side asset mint has moved `NEXT_ID` first |
| **Q4** | §7.3 names the host's own tag / enable-tag / asset mints as consumers outside any bound, to be counted by D2's census and measured by M-K3 | `register_tag` / `register_enable_tag` (`tag_api.rs:65`, `enable_tag_api.rs:61`) are `pub`, `#[cold]`, callable at any time; the host constructs `Assets::<Material>` in its config phase (`runner.rs:258`) |

**Two things rev 5 changed that pass 4 did not ask for**, because resolving C1 and C2 created them.

1. **§7.10 gains a kernel delta that lands in EVERY build** — item 2b, the mint-refusal latch: one
   `AtomicBool` and nine cold stores. Rev 4's rule admits it (a Rust-ABI change with no attribute,
   `cfg`, feature or constant), and it is the first §7 item whose non-modding cost is "nine cold
   stores" rather than "nothing"; it is therefore rowed with M-P1's asm diff as the proof rather
   than asserted free. The architect may replace the mechanism; the requirement (a structural
   signal) stays.
2. **§8.1's ordering shrinks rather than re-ranks.** Pass 4 asked that route 2 be measured or
   labelled; labelling it a prediction leaves the suspended ordering with no head. That is the
   honest state: the two candidates that are correct by construction are ordered by a nightly flag
   whose workspace number does not exist yet (M-F1 leg (d)), and the one that would beat both has no
   probe. Nothing in the ranking's *reasons* changed.

**What rev 5 did NOT change.** The P1/P2/P3 decomposition; the refusal of a whole-image symbol census
as the P1 gate (narrowed, not weakened: the source census stays, only the export leg that could not
fail is gone); separate-crate-not-cargo-feature and "by dependency edge only"; §0.1's partition
(now with access derivation added to side (ii)); leg (d) as the discriminating leg, including its
escalation to Stage 0 — its *coverage* was corrected, its *mechanism* (the poison) was not; the
descriptor-as-input-struct ruling and TRIPWIRE 2; the withdrawal of a per-configuration constant;
and the rejection of Wasm on 128-bit SIMD.

---

### rev 6 — architecture-critique pass 5 (2 critical, 3 important, 1 optional, 4 questions) — CLOSING revision

Scope was the rev-5 delta plus pass 4's blockers. The two blockers were resolved by **orchestrator
ruling** (R1, R2), not by further redesign; the ruling text is applied, and each application was
re-traced in `D:/wt/joltab` at `d552be05` (still the tree's HEAD, verified `git log -1`) before the
text changed. No measurement was taken and none is newly queued; M-P1 is re-specified, M-A4 gains
a leg. **R3 closes the document:** the Status line at the top says so, and the remarks pass 6 may
still raise are listed here as open rather than resolved in place, to be carried into
`docs/unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md`.

| # | Change | Why, and what was verified |
|---|---|---|
| **C1** (R1) | **M-P1 is re-specified as two TREES, one configuration.** Pins are captured on the pre-change tree (`d552be05`) — byte size and disassembly of the named hot functions (`query_ref_iter`, `swap_remove`, `Schedule::run`'s dispatch) *and* of every function a §7 item touches (`register_new::<T>` / `T::component_id()` for a representative `T`, the five dispensers, `try_register_dynamic`, `register_layout`, `add_system::<F, M>`), `size_of` of `EcsMaster` / `ComponentPool` / `Scope`, the executable's export count and its `.text` size — and compared on the post-change **non-modding** build. Rev 5's two-leg diff is withdrawn. §7.10's rule gains R1's condition (a delta is admitted only while the pins hold; one that moves a pin is charged to P1 and moves behind the crate boundary); items 1, 2b, 3, 5 re-gated; item 2b made **conditional**, its price restated as emitted stores (order 130–150) rather than nine sites, and its boundary-side fallback named with its coverage gap. P1 (§0), §1's cost section, §7.1's O2 sentence, §8.3, Stage 1, Stage 3 and M-P1 updated; two red controls required | The two-leg diff could not fail: items 1 / 2b / 3 land in `boyko_ecs` unconditionally, so both legs of one tree carry them, and the doc's own item-2b cell said "unchanged by construction". `register_new<T>` (`mod.rs:920-947`) and `register_layout<T>` (`:1031-1056`) are generic, instantiated per type per crate from the derive's `#[inline]` `component_id()` (`component.rs:370-375`), so "nine cold stores" counted sites, not emitted stores. The three hot functions are reached by none of §7's items. `size_of` pins exist for two of the three types already (`component_pool.rs:57`, `:62`; `scope.rs:1088`); `llvm-nm` / `llvm-objdump` / `llvm-readobj` / `llvm-size` are all present in the installed `llvm-tools`. Pin (4)'s limit is M-10 leg 3's: an exe's export directory is empty with or without `#[no_mangle]` on stable, so the census stays the leg that sees the attribute. The boundary-side fallback was verified against the dispensers: `register_new`, the resource and event dispensers `fetch_add` then assert (`mod.rs:921-922`, `resource_registry.rs:175-176`, `event_registry.rs:110-111`) and leave `cap + 2` after a `cap + 1` poison; `QUERY_NEXT_ID` / `BUNDLE_NEXT_ID` store the cap back (`query_type_registry.rs:132-135`, `bundle_type_registry.rs:120-128`) so a read-back differs from `cap + 1`; `try_register_dynamic`'s CAS loop returns `None` before touching `NEXT_ID` (`mod.rs:968-972`) — the one refusal a counter read cannot see |
| **C2** (R2) | **Mod-system registration is on partition (ii), with a host-side seam — §7.9(c), §7.10 item 6.** `App`, `Plugin` / `add_plugin`, `App::add_systems` / `add_systems_in` / `add_systems_cfg` / `add_systems_cfg_in` / `add_startup_system`, `ScheduleBuilder` (its `add_system`, `insert_state`, `SystemConfig::run_if`) and every generic that boxes a system join the façade's forbidden set with `compile_fail` fixtures (M-A4 leg (a)). The seam: a non-generic fn-pointer `System` impl plus one cold `unsafe fn` entry on `ScheduleBuilder` taking `(fn ptr, ctx, name)` and a `#[repr(C)]` access descriptor, reached through a host-supplied function pointer so the `Box::new` runs in the host image; R2's fallback (mod-side box + the mod's dealloc fn pointer) recorded as admissible only if the pins prove it free. Priced: the dispatch's existing vtable call plus one fn-pointer call per mod-system run — two per-system-run indirect calls where an engine system pays one, the same class, not a new one. §0.3 (ii) and the forbidden list, §1's boundary bullet, System-registration and Allocator rows, failure mode 5, §6's two cells, §7.9(c), §7.10 item 6, §8.2's A row, §8.3, Stage 1, M-A4 legs (a) and (e) updated | `App::add_systems<F, M>` (`app.rs:339`), `add_systems_in` (`:398`), `add_systems_cfg` (`:318`), `add_systems_cfg_in` (`:372`), `add_startup_system<F, M, Out>` (`:508`, `Box::new(move \|world\| …)` at `:514` into `self.startup`), `add_plugin<P: Plugin>` (`:550`, `plugin.build(self)`) — all `pub`. `ScheduleBuilder::add_system<F, M>` (`schedule_builder.rs:177-191`) is `pub`, generic, `Box::new(sys)` at `:183`, `SystemBox::new(boxed)` at `:184`; the other production boxing sites are `insert_state` (`:263`) and `SystemConfig::run_if` (`:837`) — `:1557`, `:1627`, `:1641`, `:1686` are test code. `SystemBox { system: Box<dyn System<Out = ()>>, kind, name: &'static str }` is `pub(crate)` (`system_box.rs:80-100`), 40 B by its own doc (`:70-77`); `SystemBox::new` is `pub(crate)` (`:129`), `SystemDescriptor::new` likewise (`system_descriptor.rs:77`) — so the seam is a kernel item, not a `boyko_mod_host` one. No `impl Drop for Schedule` / `SystemBox` / `App` exists (grep). The dispatch reaches a system through `(*system_slot).system.run_unsafe(cell_copy)` (`schedule.rs:1380`) and `.system.run_dispatcher(token)` (`:1215`). `Access` and `SystemMeta` are `#[repr(C)]` (`access.rs:45`, `system_meta.rs:86`); `add_component_read` / `add_component_write` are `pub` (`access.rs:81`, `:87`); `SystemMeta::new(name, tick)` (`system_meta.rs:205`). **Correction to the critique's grep:** `#[global_allocator]` hits are benches, test modules, and one feature-gated shim — `boyko_app/src/profiling/alloc_shim.rs:145` under `#[cfg(feature = "profiling-alloc")]` — not tests and benches alone; the conclusion (no shipped image installs one) stands |
| **W1** | The loader reads the latch once after the mod's init returns — the read site a zero-system mod lacked. §0.3 limit (v), Stage 1, §11, M-A4 leg (b) (new red control: a system-less mod taking `try_register_tag`'s `None`) | `try_register_tag` returns `None` with no panic (`tag_api.rs:47`); rev 5's two read sites were the trampoline's panic path and an optional per-mod-system-run read, and a mod with no scheduled system passes through neither. One load at load time through the probe the loader already used to poison |
| **W2** | §8.1's headline restated as the live ordering rev 5 derived — *A′-nightly (pending M-F1 (d)) > C-3a ≈ A′-stable, C-route-2 unplaced pending M-C5, A third* — with C's primacy **withdrawn** pending M-C4 / M-C5; the old headline quoted for the record | The four passages the critique named are in the same document: C was ranked first as 3b (red, M-10, withdrawn); §8.2's own rule drops C-3a below A′; rev 5 labelled route 2 a prediction; rev 5's log said the ordering "shrinks rather than re-ranks" and left the bold headline standing |
| **W3** | §7.3 D3 now states that `dynamic_slot_occupied_panic`'s text and `register_layout`'s doc contract are part of D3's change; the `unsafe`-seam clause noted as not applying to `try_register_dynamic_at` | `register_layout` is `#[doc(hidden)] pub` (`mod.rs:1030-1031`) with sanctioned callers "(1) tests … (2) synthetic scratch columns" (`:1012-1024`); the panic text at `:998-1010` blames a "test-only `register_layout`"; `LAYOUTS[id].set` is a `OnceLock` set, so a collision is loud, not unsound |
| **O1** | §7.3 D2 states that its source-site census equals the id count only while no component type is generic, and asks the census test to assert it | A `#[derive(Component)] struct Buf<T>` mints once per monomorphisation (`component.rs:371-375`); a multiline grep for a derive followed by `struct X<` over `crates/` returns nothing at `d552be05` — a property of today's tree, like the four single-consumer counters |
| **Q1** | §1's canary paragraph states what it does and does not cover: `Access` / `SystemMeta` are `#[repr(C)]`, and under §7.9(c) no `Box<dyn System>` crosses, so the language's silence on `repr(Rust)` / vtable layout across compilations no longer touches any crossing value | `access.rs:45`, `system_meta.rs:86`; the only `repr(Rust)` value crossing is the mod-owned `QueryDataState<D, F>`, opaque to the host |
| **Q2** | Answered by W1 (loader post-init read); the per-run site stays open | — |
| **Q3** | Open — the architect's; §11 says so | — |
| **Q4** | Answered through R1: §7.10 admits item 2b's class **conditionally** on the pins; the mechanism that lives outside the kernel is named (item 2b) with its gap, and the choice is listed open | as C1 |

**Open remarks carried out of this document (R3):**

1. **§7.3's direction** — D1 / D2 / D3 / D2 + D3, the quota's unit, and W3's diagnostics clause if
   D3 (pass 5, Q3; M-K3 pending).
2. **Item 2b's home** — kernel latch (only while M-P1's pins hold) or the boundary-side counter
   read-back (`cap + 1` poison) with its `try_register_dynamic` gap; and the per-mod-system-run read
   (pass 5, Q2 / Q4; W1's loader read is decided).
3. **The ranking's head** — M-F1 leg (d), M-C4, M-C5 all pending; §8.1's live ordering has no
   measured head.
4. **M-P1's first run** — the pins do not exist until captured; the gate is specified, not written,
   and its two red controls are part of writing it.
5. **`docs/unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md` does not exist** at
   `D:/claude/BoykoEngine` as of this revision (the directory holds the checkpoint and the
   engine-runtime documents); the Status line names it as the carry target by ruling, and the
   transfer is the unification plan's to complete.
6. **Whatever critique pass 6 raises** is to be appended here as open, not resolved in place — the
   document is closed.

**Critique pass 6 (final) - APPROVED; its remarks, appended as open per R3.** R1 and R2 were judged applied correctly and completely; no remark establishes a cost a game without modding would pay. Carried into `docs/unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md`:

- **P6-1. The disassembly pins are taken on a build profile other than the one that ships, for a reason the document does not establish.**
- **Where:** M-P1 method cell; Stage 3.
- **Problem:**
  - The document says `profile.release` "partitioning can shift and produce a false red".
  - But the pre-change and post-change trees differ only by the modding delta. If a release build is reproducible for identical source, any change in a pinned function's release disassembly is caused by that delta. Under the owner's constraint that is a true red, not a false one.
  - `ComponentPool::swap_remove` is a non-generic function inside `boyko_ecs`. Adding a cold function to `boyko_ecs` can change how that crate is split into codegen units (CGUs), and with it the optimisation context of `swap_remove`.
  - The tree itself records how sensitive these hot paths are to that split: codegen-units = 1 under fat LTO slows `query_ref_iter` by 17.4 % (`Cargo.toml:104-107`).
  - The "named deterministic profile" is never actually named, and the document disqualifies `profile.bench` itself (`lto = false`, `Cargo.toml:114-127`).
- **Consequence:** a size-neutral change to a pinned hot function's shipped machine code passes M-P1, because only `.text` size is compared on `profile.release`. This is the only open item that touches P1.
- **Confidence:** PLAUSIBLE. Whether release builds with codegen-units unset and incremental off are reproducible is not established.
- **Settle:**
  1. Build `d552be05` twice on `profile.release` from a clean target directory.
  2. Diff `llvm-objdump --disassemble-symbols` output for the three hot functions.
  3. If the two builds are identical, add release-profile disassembly pins as well.

- **P6-2. Item 2b's in-kernel latch is ruled out by M-P1's own red control, yet the document leaves it "conditional".**
- **Problem:**
  - The red control ("one relaxed store on `register_new`'s failing arm must move pin (2) or pin (5)") is a subset of item 2b itself.
  - `register_new<T>`'s assert arm is inline in the generic body (`mod.rs:920-927`), so the store changes every instantiation, including the pinned `register_new::<Transform>`.
  - `try_register_dynamic` (`mod.rs:967-972`) is itself pinned in (2).
  - So if the gate is valid, item 2b can never be admitted.
- **Consequence:**
  - The working design is really the counter read-back at the boundary. That cannot see `try_register_dynamic`'s `None`: the CAS loop returns without touching `NEXT_ID`.
  - Pass-5 W1's post-init loader read, and M-A4 leg (b)'s rev-6 red control (a mod with no systems that takes `try_register_tag`'s `None`), therefore cannot be met.
  - M-A4 leg (b) will be red by construction on that control, unless the gap is accepted as reachable only outside the façade.
- **Confidence:** CONFIRMED (code traced).
- **Cost to games without modding:** none. The gate removes the latch.

- **P6-3. The seam passes `UnsafeEcsCell` by value, and the build canary does not cover `debug-assertions`.**
- **Where:** §7.9(c), first bullet; §1 canary paragraph and the Q1 answer.
- **Problem:**
  - `UnsafeEcsCell` has default Rust layout (`repr(Rust)`) and a `#[cfg(debug_assertions)]` field (`unsafe_ecs_cell.rs:58-81`). It is 8 bytes in release and 16 bytes with debug assertions.
  - On Win64 an 8-byte aggregate travels in a register; a 16-byte one travels by hidden reference.
  - The canary's axes name opt-level but not `debug-assertions`, which is set independently of opt-level.
  - Other debug-only fields exist: `dispatcher_token.rs:76-77`, `bundle.rs:113-118`.
  - Q1's claim that only `QueryDataState` crosses the boundary in default Rust layout is false for the seam as written.
  - Further build-time environment axes exist beside `BOYKO_PROFILE`: `BOYKO_PROFILING_TIER`, `_REGION_CAPACITY`, `_DYN_CAP` and `BOYKO_LOG_MAX_LEVEL` (`profile_axis_census.rs:325`). Whether they change any layout a mod reads is PLAUSIBLE, not checked.
- **Consequence:** a mod built at opt-level 3 with debug assertions on passes the canary. It then reads the host's `EcsMaster` pointer as a pointer to a cell, which means wild reads and writes in a release session.
- **Confidence:** CONFIRMED for the layout and the canary list. The call-convention consequence follows from the Win64 ABI.
- **Scope:** modding-only.

- **P6-4. The seam's signature cannot express an ordinary system.**
- **Problem:** the function-pointer system receives only a context pointer and a world cell. Nothing carries:
  - the change-tick window (`System::set_change_ticks`, `system.rs:267`), which a mod's `Changed` / `Added` filters need;
  - an `apply` hook for deferred command buffers (`system.rs:165`);
  - the `has_deferred`, `requires_dispatcher` and `is_gpu` flags (`system.rs:107`, `:190`), which `SystemBox::new` uses to decide the system kind;
  - ordering, sets or run conditions (`SystemConfig` is forbidden).
- **Consequence:**
  - A mod's `Changed<T>` filter works with an unset tick window.
  - A mod system's commands are never applied.
  - A mod system that touches a `NonSend` resource is classified as freely concurrent.
  - Mod systems cannot be ordered against engine systems.
  - Each hook added to fix this is one more per-run or per-barrier crossing that must be priced.
- **Confidence:** CONFIRMED (trait surface traced).
- **Scope:** modding-only.

- **P6-5. The baseline commit is fixed, while other work keeps landing.**
- **Problem:** R1 fixes `d552be05`. If any commit unrelated to modding lands in `boyko_ecs`, `boyko_threadpool` or the other linked kernel crates before a modding delta, the pins move for reasons that have nothing to do with modding.
- **Required:** pins must be captured from a clean checkout of the commit. The owner's session notes record uncommitted work in `D:/wt/joltab`; I could not run git to check.
- **Consequence:** a false red, charged to P1.
- **Confidence:** PLAUSIBLE.
- **Direction, within R1's intent:** land the modding deltas as one contiguous series on top of the pin commit, or re-capture the pins at the parent of the first modding delta.

- **P6-6. R2 citation fixes.**
- `SystemConfig::run_if` is at `system_config.rs:183-192` (the `Box::new` is at `:189`), not at `schedule_builder.rs:837`.
- `:837` is `ConfigureSet::run_if` (`:831-844`), a second boxing generic that M-A4 leg (a) does not name.
- §0.3 limit (v) still cites `schedule_builder.rs:183` (where the box is made) as "the vtable call the schedule already makes"; the call sites are `schedule.rs:1380` / `:1215`.
- `SystemMeta`'s `#[repr(C)]` is at `:85`, not `:86`.
- **Confidence:** CONFIRMED.
- **Consequence:** low. The type-level exclusion already covers both `run_if` methods.

- **P6-7. M-P1 is under-specified in three places.**
- Its list "items 1, 2b, 3, 5, 6" leaves out item 2's address accessors. The §7.10 rule header does cover them.
- It never names the executable whose exports and `.text` are pinned.
- It never names which binary provides the `query_ref_iter` symbol. Under fat LTO a game executable has no such symbol; it exists as a benchmark closure.
- **Consequence:** two people capturing the pins would produce different pins. This belongs with open remark 4 (M-P1's first run).
- **Confidence:** CONFIRMED (text).

- **P6-8. A measurement is quoted beyond its scope.**
- Items 1 and 5 say "absent at `lto = "fat"`", citing `REFLECTION-ANALYSIS.md:1477-1481`. That measurement is for fat LTO with codegen-units = 1, not for the shipped profile (fat LTO, codegen-units unset; `Cargo.toml:111-112`). Item 6 correctly says "if the pins say so".
- §6's "Cost when modding OFF — zero" cell for option A is likewise now conditional on the pins.
- **Confidence:** CONFIRMED.
- **Consequence:** none once the pins run.

- **P6-9. The per-mod-system price is a floor, not the full price.**
- The roughly 2 ns extra hop leaves out M-A2's three thread-local writes at each system entry, which the trampoline pays whenever `par_iter` is offered to mods.
- **Confidence:** PLAUSIBLE (M-A2 is unmeasured).
- **Scope:** modding-only.

- **P6-10. The cross-image allocation problem exists today, not only for "future" generics.**
- `#[inline]` lazy allocators get compiled into whichever image calls them — for example `bundle_archetype_cache`, `ecs_master.rs:517-523`.
- Every such site I found sits behind a forbidden entry point, so there is no hole today.
- Failure mode 5 should become a mechanical census over everything the façade re-exports.

**What rev 6 did NOT change.** The P1/P2/P3 decomposition (P1's *legs* changed, its property did
not); the refusal of a whole-image symbol census as the P1 gate (the pins are per-function bytes
and per-binary sizes, not a presence census); separate-crate-not-cargo-feature and "by dependency
edge only" (now with R1's condition attached); §0.1's partition (side (ii) gained system
registration); leg (d) as the discriminating leg, its coverage and its escalation to Stage 0; the
descriptor-as-input-struct ruling and TRIPWIRE 2; the allocator rule as a property of who frees (it
was applied, not changed); the ranking's *reasons*; and the rejection of Wasm on 128-bit SIMD.

