# Modding — the survey

**Status:** research only, **rev 4**. No design is ratified here; the options and the recommendation
live in [MODDING-DESIGN-SPACE.md](MODDING-DESIGN-SPACE.md). Rev 2 added **§13.1** (four measured
install-time-relink and linker-registry probes, M-5…M-8), closed one §14 item, and corrected two
claims in §15 that the design space's rev-2 critique pass found too strong. Rev 3 adds **§13.3**
(M-10 — the derive-shaped per-type static under fat LTO and route 3b's two link lines, both loud
failures; M-11 — nightly `-Z dylib-lto` on the M-2 rig), closes one more §14 item at toy scale, adds
three, and annotates §15's questions 2, 6 and 7 with the design space's rev-4 answers. (§13.2, M-9,
was written during the design space's rev 3 without a status bump here.) **Rev 4 of this file (with
the design space's rev 5)** takes no measurement; it corrects §12.4's reachability claim for the
asset-layout mint, withdraws the `--coff-exports` gate from §15's question 6 on the strength of
§13.3 leg 3's own receipt, re-opens question 7's bound, and adds two §14 items the design space's
M-C5 and M-K3 would close.

**The question.** The owner's goal is modding that is **free in performance terms** — a mod binary is
added and runs like native engine code — under a hard constraint: **modding is optional, and a game
that does not use it pays nothing**: no extra indirection, lookup, branch, registry, lock,
allocation, symbol export, binary size or startup work on any path, ideally *identical code
generation* to an engine with no modding support at all.

**The answer this survey reaches, before any design.** Nobody has shipped that. Every native modding
path in the field pays at least one of three prices — a dynamic-link boundary (lost inlining, not
the call), a stable-ABI data representation, or runtime binary patching — and the only separation
mechanism that is genuinely free when unused is a **build-time choice**, never a runtime check.
Rust adds two specific aggravations (no stable ABI; LTO and `dylib` are mutually exclusive on
stable) and one specific mitigation (generics and `#[inline]` bodies are instantiated in the
*calling* crate, so a mod's inner loops are native even across a boundary). This engine is at the
hard end of the spectrum: every id it uses is minted into a process-global static, storage
provenance is baked into query state, and nothing can ever be unregistered.

Repository facts in §12 are read from `D:/wt/joltab` at commit **`d552be05be4b4f063b6cb39ddfd63eb4688f83fd`**
(branch `merge/ke16-into-ecsnative`); paths are relative to the repository root.

---

## 1. The language floor: what Rust guarantees and what it does not

Nothing above this section can be designed around these facts.

### 1.1 No stable ABI; symbol names are the *only* automatic protection

- `repr(Rust)` guarantees alignment, non-overlap and field alignment and nothing else; field order
  "does not have to be the same as the order in which the fields are specified", and **"Type layout
  can be changed with each compilation"**
  ([Reference — Type layout](https://doc.rust-lang.org/reference/type-layout.html)).
  `-Zrandomize-layout` exists specifically to break code assuming otherwise
  ([Unstable Book](https://doc.rust-lang.org/unstable-book/compiler-flags/randomize-layout.html)),
  and its own documentation warns it is not a safety oracle.
- With **v0 mangling** (default on stable since 1.97, 2026-07-09
  — [release notes](https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/);
  [nightly switch 2025-11-20](https://blog.rust-lang.org/2025/11/20/switching-to-v0-mangling-on-nightly))
  every Rust symbol embeds `StableCrateId`, which hashes the crate name, the crate type, the sorted
  `-C metadata` values and **the rustc version**
  ([`rustc_span/src/def_id.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_span/src/def_id.rs),
  [`rustc_symbol_mangling/src/v0.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_symbol_mangling/src/v0.rs)).
  A mod built by a different rustc therefore **fails to link** rather than corrupting memory.
- **Two holes in that protection, both load-bearing here.** (i) `#[no_mangle] extern "C"` discards it
  entirely — which is exactly what Bevy's removed `_bevy_create_plugin` did. (ii) Cargo
  *deliberately* excludes `RUSTFLAGS` from `-C metadata`, because "profile-guided optimizations need
  to swap `RUSTFLAGS` between runs, but need to keep the same symbol names"
  ([`Metadata` docs](https://doc.rust-lang.org/stable/nightly-rustc/cargo/core/compiler/struct.Metadata.html),
  [cargo#14830](https://github.com/rust-lang/cargo/pull/14830), 2024-12-06). So
  `-C target-cpu=x86-64-v3` — the flag this engine sets workspace-wide — is **invisible** to the
  compiler's own mismatch check.
- rlib metadata *is* version-locked: mixing compiler versions is E0514
  ([error index](https://doc.rust-lang.org/error_codes/E0514.html)).
- rustc is **not** bit-for-bit reproducible today
  ([rust#129080](https://github.com/rust-lang/rust/issues/129080), open; a `--reproducible` flag is
  only [proposed](https://github.com/rust-lang/rust/pull/160448)). "Same toolchain + same flags ⇒
  identical layout" is what every shipping scheme relies on and no document guarantees it — only the
  absence of a counterexample for layout specifically.

### 1.2 `TypeId`, `type_name`, vtables and fn pointers are not cross-binary identities

- `TypeId`: size and layout are unstable, and "the hashes and ordering will vary between Rust
  releases" ([std docs](https://doc.rust-lang.org/std/any/struct.TypeId.html)). It is computed from
  the type structure via `StableHasher` and inherits `StableCrateId` through `DefPathHash`
  ([`rustc_middle/src/ty/util.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_middle/src/ty/util.rs));
  historic report [rust#61553](https://github.com/rust-lang/rust/issues/61553).
  Two builds of the same crate with different `-C metadata` are *different types* to the compiler
  ([rustc-dev-guide, libs and metadata](https://rustc-dev-guide.rust-lang.org/backend/libs-and-metadata.html)).
- `type_name`: "must not be considered to be a unique identifier of a type … the output may change
  between versions of the compiler"
  ([std docs](https://doc.rust-lang.org/std/any/fn.type_name.html)).
- Vtable pointers are neither unique nor stable: `ptr::eq` documents that pointers to the same
  underlying type "can compare inequal (because vtables are duplicated in multiple codegen units)"
  and that different types can compare equal after dedup
  ([std docs](https://doc.rust-lang.org/std/ptr/fn.eq.html)).
- Even **function-pointer identity** broke real code when automatic cross-crate inlining landed
  ([rust#116505](https://github.com/rust-lang/rust/pull/116505) →
  [rust#117047](https://github.com/rust-lang/rust/issues/117047)).

### 1.3 Panics

- Since 1.81, an uncaught unwind out of an `extern "C"` function **aborts the process**
  ([release notes](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/),
  [RFC 2945](https://rust-lang.github.io/rfcs/2945-c-unwind-abi.html)).
- With `extern "C-unwind"` unwinding is allowed, but a `panic=abort` receiver catches and aborts.
- Worse across two `std` copies: a panic from "a different runtime" is a *foreign exception* and
  `catch_unwind` on it is **unspecified** — either abort or an opaque `Err`
  ([`catch_unwind` docs](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)).
- **Consequence:** a mod must catch its own panics and return a status across any C boundary.

### 1.4 Allocator

- `cdylib`s are "guaranteed to use the `System` allocator" by default
  ([`std::alloc::System`](https://doc.rust-lang.org/std/alloc/struct.System.html)); one
  `#[global_allocator]` per crate graph
  ([`std::alloc`](https://doc.rust-lang.org/std/alloc/index.html)). `dealloc` on memory not
  allocated "via this allocator" is UB
  ([`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html)).
- `#[global_allocator]` is **silently ignored** under `-C prefer-dynamic`
  ([rust#100781](https://github.com/rust-lang/rust/issues/100781), open since 2022).
- Field evidence: Bevy has an open, S-Blocked segfault from `mimalloc` + `dynamic_linking`
  ([bevy#18251](https://github.com/bevyengine/bevy/issues/18251)).
- **Consequence for the owner's "one allocator — ours" rule:** the moment the engine installs its own
  global allocator, any allocation crossing a `cdylib` boundary becomes UB unless both sides route
  to the same allocator.

### 1.5 TLS and unloading

- std, verbatim: when dynamically unloading a Rust `cdylib`, "pending TLS destructors may run during
  the unload or may be leaked"; and a process loading a Rust cdylib "must not cause the Rust TLS
  destructor support to be initialized for the first time during process shutdown"
  ([`LocalKey`](https://doc.rust-lang.org/std/thread/struct.LocalKey.html)).
- On glibc, pending TLS destructors set `DF_1_NODELETE` and make `dlclose` a **no-op**
  ([rust#59629](https://github.com/rust-lang/rust/issues/59629), open since 2019;
  [MaskRay, TLS](https://maskray.me/blog/2021-02-14-all-about-thread-local-storage)).
- A `dlopen`ed object's TLS uses general-dynamic (`__tls_get_addr`, lazy allocation on glibc) rather
  than local-exec — a model change, not a constant factor (MaskRay, above).
- On Windows, `DllMain` runs under the loader lock with a long prohibited-call list
  ([MS best practices](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-best-practices)).
  On `x86_64-pc-windows-gnu` rustc does not use native TLS at all (`target_thread_local` unset → OS-key
  backend) — measured in this repository at `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md:87`.
- **The requirements for a sound `dlclose` are an open question in the Unsafe Code Guidelines**
  ([UCG#526](https://github.com/rust-lang/unsafe-code-guidelines/issues/526), open since 2024-08-13):
  no surviving fn pointers, vtables, `&'static` data or statics from the library.

### 1.6 SIMD across the boundary

The ABI of `__m256`/vector types depends on enabled target features; a mismatch yields a
half-filled vector. Rust made this a lint in 1.84 and a **hard error in 1.87**
([rust#116558](https://github.com/rust-lang/rust/issues/116558)) — but that check is
*intra-compilation*. Across two separately built binaries nothing checks it, and `-C target-cpu`
is not in `-C metadata` (§1.1).

---

## 2. Linkage economics: where "free" is actually decided

### 2.1 The four routes and what each costs

| Route | Buys | Costs |
|---|---|---|
| Engine as Rust `dylib`, `-C prefer-dynamic` | one copy of every registry static; coherent `TypeId` | **LTO forfeited for the whole product**; ships `std`/`core`/`alloc` as DLLs; "rdylibs expose all symbols as rlibs do" → PE 65535-export ceiling |
| Mod as `cdylib` linking the engine as rlib | LTO inside the mod; native mod-local monomorphisation | **two copies** of every engine static, TLS key, `TypeId` universe and allocator |
| Export symbols from the `.exe` | keeps LTO for the game | `-Zexport-executable-symbols` is **unstable** ([rust#84161](https://github.com/rust-lang/rust/issues/84161)); requires `#[no_mangle]`; grows every build's export table |
| `#[repr(C)]` function table handed to the mod at load | no exports, no dylib, engine keeps LTO | every mod→engine call is indirect; generics cannot cross |

- **LTO and dylib are mutually exclusive on stable.** `cgcx.prefer_dynamic && !cgcx.dylib_lto` →
  `DynamicLinkingWithLTO`; `CrateType::Dylib && !cgcx.dylib_lto` → `LtoDylib`
  ([`rustc_codegen_ssa/src/back/lto.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_codegen_ssa/src/back/lto.rs)).
  `-Zdylib-lto` is nightly and "currently only used for compiling `rustc` itself"
  ([Unstable Book](https://doc.rust-lang.org/stable/unstable-book/compiler-flags/dylib-lto.html));
  the general issue is [rust#31854](https://github.com/rust-lang/rust/issues/31854) (open since 2016).
  Cargo also **silently drops** LTO for a `cdylib` that also emits an rlib
  ([cargo#4611](https://github.com/rust-lang/cargo/issues/4611), open).
- **A dylib always dynamically links `std`** (bjorn3, 2024-04-17,
  [users.rust-lang.org](https://users.rust-lang.org/t/crate-type-rlib-dylib-and-ldd-exe-std-dll-not-found/110103)):
  "If you create a rust dylib, it will always dynamically link libstd."
  The Reference's uniqueness rule is the cause: "a library never appears more than once in any
  artifact" ([Linkage](https://doc.rust-lang.org/reference/linkage.html)).
- **The Windows export ceiling is real and has been hit.** PE export ordinals are 16-bit. Bevy set
  `-Zshare-generics=n` on Windows to build with `dynamic`
  ([bevy#2016](https://github.com/bevyengine/bevy/pull/2016), 2021-08-30) and its
  `config_fast_builds.toml` still carries the warning; `LNK1189` at
  [bevy#14930](https://github.com/bevyengine/bevy/issues/14930) and
  [bevy#1110](https://github.com/bevyengine/bevy/issues/1110); rust-lld "too many exported symbols
  (max 65535)" at [bevy_egui#22](https://github.com/vladbat00/bevy_egui/issues/22). On windows-gnu the
  same wall reads `ld.exe: error: export ordinal too large`
  ([forum, 2022-11-10](https://users.rust-lang.org/t/gnu-ld-linker-errror-export-ordinal-too-large-xxxxx/84092)).
  Note `share-generics` defaults **on at opt-level 0/1/s/z and off at 2/3**
  ([`rustc_session/src/config.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_session/src/config.rs)) —
  a release engine dylib does not share generic instantiations with mods, so every mod
  re-instantiates them.
- **Every exported symbol is a symbol LTO cannot touch.** LLVM's `internalize`, `deadargelim`,
  `argpromotion` and `globaldce` operate on *internal* functions
  ([LLVM passes](https://llvm.org/docs/Passes.html)).

### 2.2 The cost that actually bites is lost inlining, not the call

| Measurement | Number | Source |
|---|---|---|
| CPython, `-fno-semantic-interposition` (re-enables inlining *inside* one `.so`) | +5 % to +27 % (scimark_lu 1.38×, nbody 1.32×) | [Fedora](https://fedoraproject.org/wiki/Changes/PythonNoSemanticInterpositionSpeedup) |
| CPython, static vs shared libpython | 5–27 % (nbody 238 → 174 ms) | [Fedora](https://fedoraproject.org/wiki/Changes/PythonStaticSpeedup) |
| Trivial `add` in a loop, inlined vs not (M4/LLVM) | 0.7 → **0.03 ns/int** (23×, because only the inlined form vectorises) | [Lemire, 2026-02-08](https://lemire.me/blog/2026/02/08/the-cost-of-a-function-call/) |
| Short virtual call, 20 M objects | 153 ms vs 126 ms (18 %); inline vs not 242 → 136 ms | [johnnysswlab](https://johnnysswlab.com/the-true-price-of-virtual-functions-in-c/) |
| **This repository, fat LTO vs cargo defaults** (5 configs × 7 benchmarks, 10 interleaved + 12 rotated passes) | `query_ref_iter/10k` **0.375** (2.67×), `swap_remove/10k` 0.655, `add fill/10k` 0.849, **geomean 0.793**, binary **8.76 → 3.46 MB** | `Cargo.toml:89-112` (measured 2026-09-04) |
| This repository, rlib boundary without LTO | 1.55× on `swap_remove`/`add fill` at 10k rows; ≈17 % geomean over 7; +5.3 MB binary | `docs/RUST-ERGONOMICS.md:649-667` |

The call itself is cheap. Windows: with `dllimport` linkage the call is
`call DWORD PTR __imp_func` — one indirect call; without it the linker inserts a thunk that
Microsoft documents as larger and cache-degrading
([MS Learn](https://learn.microsoft.com/en-us/cpp/build/importing-function-calls-using-declspec-dllimport)).
ELF: default PLT is `call foo@plt` + `jmp *GOT` (2 branches); `-fno-plt` collapses it to one 6-byte
`call *GOT(%rip)` ([MaskRay, PLT](https://maskray.me/blog/2021-09-19-all-about-procedure-linkage-table);
explicitly no measured numbers there).

**Control-flow guard**, if ever enabled on Windows, adds 0–8 % (geomean 2.9 % SPEC2017int) to
indirect-call-heavy code and is nightly-only
([Unstable Book](https://doc.rust-lang.org/stable/unstable-book/compiler-flags/control-flow-guard.html)).

### 2.3 The Rust mitigation that makes native mods plausible at all

Generics and `#[inline]` bodies — plus, since
[rust#116505](https://github.com/rust-lang/rust/pull/116505) (merged 2023-10-18), automatic
cross-crate inlining of small call-free functions — are **instantiated in the calling crate**
([matklad on `#[inline]`](https://matklad.github.io/2021/07/09/inline-in-rust.html)). A mod compiled
against the engine's rlib metadata gets native-quality query and storage codegen *inside itself*,
even across a dylib boundary. Only opaque, non-inlinable entry points (spawn, command apply,
archetype migration, registry mints) pay the indirection.

This is the single most important structural fact in the whole survey: **it is the reason
"one crossing per system run" can be free while "one crossing per entity" cannot.**

### 2.4 Link and relink times

| Measurement | Number | Source |
|---|---|---|
| Chromium 145 debug (4.51 GiB), non-LTO relink | lld **16.64 s** / mold 1.65 s (Threadripper 7980X, 2026-08-28) | [mold README](https://github.com/rui314/mold) |
| Chromium 145 release (0.58 GiB) | lld **6.48 s** / mold 0.64 s | same |
| This repository, clean build | 148 s (no LTO) → 233–324 s (fat); "2–3× for fat, ≈+15 % for thin" | `docs/RUST-ERGONOMICS.md:649-667` |
| ThinLTO with a cache | "incremental link-time very close to a non-LTO build" (qualitative, 2016, clang) | [LLVM blog](https://blog.llvm.org/2016/06/thinlto-scalable-and-incremental-lto.html) |
| dylib split, incremental link | 2.353 s → 0.202 s (compile-time only) | [Kra.hn, 2022-09-09](https://robert.kra.hn/posts/2022-09-09-speeding-up-incremental-rust-compilation-with-dylibs/) |

**Not established anywhere:** link-only (as opposed to clean-build) time for a Rust game binary
under fat LTO. No number for this engine exists.

### 2.5 Toolchain shipping

- `-Clinker-plugin-lto` makes rustc emit bitcode instead of machine code in `.o`/rlib;
  `-Cembed-bitcode` keeps rlibs usable both ways
  ([codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html)). LLVM promises
  bitcode readability back to 3.0 ([developer policy](https://llvm.org/docs/DeveloperPolicy.html)).
- A bundled linker already exists in the toolchain: `rust-lld.exe`, `gcc-ld/lld-link.exe`, and
  self-contained `ld.exe` / `dlltool.exe` / `x86_64-w64-mingw32-gcc.exe` under
  `lib/rustlib/x86_64-pc-windows-gnu/bin` (observed locally on toolchains 1.97.1 / 1.98.0 /
  stable / nightly).
- Licence asymmetry: Rust is MIT/Apache-2.0
  ([Arch package](https://archlinux.org/packages/extra/x86_64/rust/)); GNU binutils/gcc binaries are
  GPLv3-family (**unverified** against the bundled `COPYING`); **the MSVC linker is not on
  Microsoft's VS 2022 Distributable list** — that list is runtime files, merge modules and
  `pgort140.dll`, not `link.exe`
  ([MS redistribution](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)).
  An install-time relink on Windows must use `rust-lld` / `lld-link`.
- Shipping a full compiler: Arch's `rust 1:1.98.1` is 78.4 MB packaged / **302.5 MB installed**,
  excluding LLVM (system `llvm-libs`). The Windows rustup footprint is **not established**.
- PGO across engine + mods is possible in principle (`-Cprofile-use`, "All `rustc` flags have to be
  the same", [PGO docs](https://doc.rust-lang.org/rustc/profile-guided-optimization.html)) but the
  shipped profile was collected without the mod's code.

---

## 3. Precedents

### 3.1 The table

| Engine / project | Loading model | ABI contract | New data types | New systems | Overhead paid | Sandbox | Failure history |
|---|---|---|---|---|---|---|---|
| **Quake 2** | `Sys_GetGameAPI(&import)`, game DLL *is* the game | C function tables; `GAME_API_VERSION 3`; version mismatch rejected | yes (owns game logic) | yes | every engine↔game call indirect, **always** | no | — |
| **Quake 3** | QVM bytecode + x86 JIT | syscall dispatch | yes | yes | interpreter "performances were disappointing" → JIT added | yes (VM) | — |
| **Half-Life 1** | `enginefuncs_t` + `DLL_FUNCTIONS`, `INTERFACE_VERSION 140` | append-only C tables; "INTERFACE VERSION IS FROZEN AT 138" | yes | yes | indirect, always | no | single game DLL → Metamod exists to multiplex add-ons |
| **Source 1** | `CreateInterface()` per DLL | C++ pure-virtual interfaces, version in the name (`MyInterface001`); no data members | yes | yes | every cross-DLL call virtual, **always** | no | incompatible change needs a wrapper for the old interface |
| **Unreal Engine** | Monolithic (static) *or* Modular (DLL per module); `IMPLEMENT_MODULE` registers a static registrant in monolithic, a DLL initializer in modular | C++; **BuildId GUID** — "only DLLs compiled at the same time as the executable" | yes (plugins, full engine access) | yes | **nothing if Monolithic — but then mods are impossible** | no | legacy Hot Reload left "HOTRELOADED" classes and corrupted Blueprint assets |
| **Unreal / Live Coding (Live++)** | runtime binary patching | needs `/FUNCTIONPADMIN`, `/OPT:NOREF`, `/OPT:NOICF` | no | no | link flags in every shipped binary; "impact should be minimal", no numbers | no | without Object Reinstancing "large-scale changes … behave unpredictably"; not available on consoles/mobile |
| **Godot GDExtension** | shared library, `get_proc_address(name)` per function since 4.1 (modelled on OpenGL/Vulkan) | C, lookup by name; forward-compatible only ("4.0 extensions will not work with 4.1+") | yes (classes) | yes | **248–288 ns/call before caching → 7–32 ns after** (measured) | no | GDScript→extension static methods fall back to Variant `varcall` ([godot#91282](https://github.com/godotengine/godot/issues/91282)) |
| **Unity (Mono/IL2CPP + BepInEx/Harmony)** | managed DLL; IL2CPP needs Cpp2IL + Il2CppInterop + P/Invoke | .NET IL; none for binary patching | **no** — cannot add fields or extend enums | via hooks | unpatched code pays nothing | no | inlined methods are unpatchable; BepInEx 6 IL2CPP failed on Unity 2021.2+ |
| **Unity Burst modding** | `BurstRuntime.LoadAdditionalLibrary` before first Burst use | native lib, Unity-Editor-built | no | no | not documented | no | documented example is "a proof of concept" |
| **Skyrim SKSE** | DLL exports `SKSEPlugin_Query` / `SKSEPlugin_Load` | C++ against the game binary; **one SKSE build per game version** | via hooks | via hooks | detours only where patched | no | "Every update breaks old SKSE mods because the addresses … change"; Address Library / signature scanning exist to survive it |
| **Minecraft Fabric / NeoForge** | Mixin bytecode transform before class load | JVM bytecode | yes | yes | **no reliable measurement found** | no | redirectors and overwrites "inherently incompatible" when two mods target one point |
| **Factorio** | Lua, three stages (settings → prototype → runtime) | scripted; prototypes only in the data stage | **no engine classes** — prototypes of engine types only | scripted | per-mod tick time shown; ">1 ms may be worth looking into" | yes (Lua) | staff: "This is fundamental limitation and its never going to change" |
| **Roblox / Luau** | bytecode VM, ~16 KB core loop, fastcalls | — | no | scripted | native codegen is **server-only**, adds startup compile + memory, no speedup numbers given | yes (`io`/`package` removed, read-only globals, interrupt) | — |
| **The Machinery** | DLL per plugin; every API a fn-pointer struct from `tm_api_registry` | C ("C has a stable ABI"); semver per API | yes | yes | **engine-internal calls go through the same structs — everyone pays**, no numbers published | no | "hot-reload code paths aren't exercised in normal operation"; company ended the engine 2022 |
| **flecs** | module import fn; `ecs_import_from_library` | C; component resolved by **C++ symbol string**, size/alignment checked | yes (runtime components) | yes | id-based by construction — no static mode to be cheaper than; **addons excludable at compile time** (`FLECS_CUSTOM_BUILD` / `FLECS_NO_<addon>`) | no | [#685](https://github.com/SanderMertens/flecs/issues/685): static ids not registered in other UE DLLs; [#1034](https://github.com/SanderMertens/flecs/issues/1034) module registration failed in a shared library |
| **EnTT** | `ENTT_API_EXPORT`/`IMPORT`; meta context shared via `locator<meta_ctx>` | C++; `type_hash` from pretty-printed name, sequential fallback | yes (named pools) | n/a | typed views bypass the virtual base; base `basic_sparse_set` is virtual for everyone | no | ids "not guaranteed stable across different runs"; cross-library name collisions; pools created one side and freed the other "can quickly lead to crashes" |
| **Bevy (removed)** `bevy_dynamic_plugin` | cdylib exporting `_bevy_create_plugin` → `Box<dyn Plugin>` | none | yes | yes | — | no | **deprecated 0.14, removed 0.15**: `ctor` runs on load, "VTable layouts are not guaranteed", identical Bevy *and* rustc required, `TypeId` unvouched |
| **Bevy `dynamic_linking`** | `bevy_dylib` = `crate-type=["dylib"]` + `-C prefer-dynamic` | same-build only | n/a | n/a | LTO forfeited; ships `libstd.so` | no | "Do not enable this feature for release builds" |
| **Bevy 0.17 `hotpatching`** | Dioxus `subsecond` jump table | n/a | no | no | **zero when cfg'd out** — the trait method itself is removed | no | "`hotpatching` should never be enabled in release" |
| **Fyrox CHR** | `fyrox-dylib` + `game_dylib`, both `-C prefer-dynamic`; shadow-copy + `notify` watcher; `fyrox_plugin: fn() -> Box<dyn Plugin>` | same build, **no compatibility check at all** | yes | yes | **`PluginContainer::{Static,Dynamic}` is NOT cfg-gated — every Fyrox game pays the enum match and the `dyn` hop** | no | own docs: "wildly unsafe functionality which could result in memory corruption"; dangling objects "could be overlooked … crash or memory corruption" |
| **MS Flight Simulator 2024** | Wasm, AOT-compiled to DLLs | Wasm | yes | yes | sandbox tax (§5); single-threaded per module, no Win32, no C++ exceptions | **yes** | — |
| **abi_stable 0.11.3** | `cdylib`, `RootModule::load_from*` | `StableAbi`-derived layout, checked recursively **before any call** | via FFI-safe types | via `sabi_trait` | load-time only, not per call | no | **library is leaked by design**; unloading removed because "possibly unsound" |
| **stabby 72.1** | `cdylib`/`dylib`, `#[stabby::export]` + `_stabbied` checker | pinned layout + type reports + **canaries** (`rustc`, `opt_level`, `target`, `host`, `debug`, `num_jobs`) | opt-in per type | — | post-1.78 trait objects use a leaked global vtable set with **O(n) lookup and O(n) insert** | no | — |
| **rustc `sdylib` / `#[export_stable]`** | nightly, experimental ([PR #134767](https://github.com/rust-lang/rust/pull/134767), merged 2025-05-05) | stable mangling + **128-bit type hash** in symbols; interface file emitted | `extern "C"` and stable-repr types only | — | — | no | **no generics** — cannot carry a monomorphised `Query<&T>`; no stabilisation found |

### 3.2 Notes the table cannot hold

**Unreal is the industry precedent for the owner's constraint.** `MODULE_API` expands to
`__declspec(dllexport)` in modular builds, `__declspec(dllimport)` when importing, and **"empty,
when compiling in monolithic mode"**
([Epic docs](https://dev.epicgames.com/documentation/unreal-engine/module-api-specifiers-in-unreal-engine));
in `IS_MONOLITHIC` builds `FModuleManager` uses statically registered module delegates instead of
per-DLL `InitializeModule` exports. The same source tree compiles to a DLL-boundary build or a
boundary-free build, chosen by one build-system switch — the C++ analogue of a cargo feature.
The price is visible in practice: Satisfactory switched to Modular at modders' request; a community
guide (not a primary source) says it took "roughly a year" and that shipped PDBs "doubled the
download size", and modders must use Coffee Stain's modified engine build — a stock installation
"**will not work**".

**Quake 3's VM decision is the oldest statement of the trade-off**: QVM was built to combine "the
security/portability of Quake1 Virtual Machine with the high performances of Quake2's native DLLs",
and an x86 JIT was added because interpreted "performances were disappointing"
([Sanglard, 2012](https://fabiensanglard.net/quake3/qvm.php)).

**Bevy's field reports are the cheapest available preview of the naive Rust design.**
[discussion #8390](https://github.com/bevyengine/bevy/discussions/8390): segfaults,
`Requested resource ... Schedules does not exist`, TypeId mismatches — and the resolution was
"compile game and mod in the same workspace", which makes mods non-distributable. The crate docs'
own verdict: the technique "works best when plugins and the main application are built
simultaneously (like DLC packs) rather than in modding scenarios"
([docs.rs 0.14.2](https://docs.rs/bevy_dynamic_plugin/0.14.2/bevy_dynamic_plugin/)).

**Could not establish:** whether GDExtension support can be compiled out of Godot; per-cause
breakdown of the Jangda Wasm slowdowns beyond the abstract; any primary account of Source-specific
ABI breakages (the SourceMod "HL2SDK Mistakes" post returns 403; metamod.org returns 521);
Minecraft Mixin's startup or steady-state cost.

---

## 4. Type identity across binaries — the ECS-specific hazard

Everyone who solved it used the same shape: **a stable string or hash key plus a layout check, never
a language type id.**

| Engine | Key | Layout check |
|---|---|---|
| **flecs** | component *entity*, resolved by **C++ symbol string** via `ecs_lookup_symbol` when a second binary's static id cache is cold | yes — aborts `ECS_INVALID_COMPONENT_SIZE` on "already registered with mismatching size/alignment"; wrong symbol → `ECS_NAME_IN_USE` ([`src/addons/flecs_cpp.c`](https://github.com/SanderMertens/flecs/blob/master/src/addons/flecs_cpp.c)) |
| **Unity DOTS** | `TypeIndex` (runtime, "should not be considered deterministic across builds") + **`StableTypeHash`** for serialization: FNV1a-64 over namespace, type name, assembly name, recursive field type hashes, field index, explicit offsets | partial — **struct size and pack are commented out (TODO)** in [`TypeHash.cs`](https://github.com/needle-mirror/com.unity.entities/blob/master/Unity.Entities/Types/TypeHash.cs) (package 1.4.8); `[TypeVersion]` can override; the file's own comment: "This hash is NOT expected to remain the same across versions" |
| **EnTT** | `type_hash<T>` = compile-time hash of the compiler's pretty-printed name; **sequential runtime counter** under `ENTT_STANDARD_CPP`, where "there is no guarantee that identifiers remain stable across executions" | none built in |
| **Bevy** | `ComponentId` = per-`World` `fetch_add(1)`; `TypeId → ComponentId` in a `TypeIdHashMap`; serialization key is **`TypePath`** (a string) — `TypeUuid` was replaced because `type_name` is not stable | none across binaries (descriptor carries `Layout`, nothing compares it) |
| **UE Mass** | `UScriptStruct*` + runtime `StructTracker` bit index | engine-wide reflection guarantees layout |
| **boyko-engine** | `Component::stable_name()` + `LAYOUT_FINGERPRINT` + `FORMAT_VERSION` — **already present**, see §12.3 | fingerprint is "best-effort hash of (size, align, repr, per-field offsets, field_count)" |

**The failure is the same bug four times:** per-binary statics for type ids. flecs #685
(UE5.0EA, 2022-03-22: reflection components' "static IDs [were not] registered yet in these other
DLLs"; workaround — construct a `flecs::world()` in each DLL); EnTT's sequential `type_index`;
Bevy's `TypeId`; and this engine's per-type `OnceLock<ComponentId>` (§12.1). It was **measured**
here in M-1 (§13).

---

## 5. Registering what the engine never saw at build time

**Bevy** is the closest comparable surface:

```rust
pub unsafe fn ComponentDescriptor::new_with_layout(
    name, storage_type, layout: Layout,
    drop: Option<for<'a> unsafe fn(OwningPtr<'a>)>,
    mutable: bool, clone_behavior: ComponentCloneBehavior,
    relationship_accessor: Option<RelationshipAccessorInitializer>) -> Self
```

Safety: "the `drop` fn must be usable on a pointer with a value of the layout `layout`"; "the
component type must be safe to access from any thread"
([docs.rs 0.19.1](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/component/struct.ComponentDescriptor.html)).
The in-tree example builds `Layout::array::<u64>(size)` components, inserts with
`insert_by_ids(&ids, OwningPtr…)` and reads back with `get_by_id` + raw casts
([`examples/ecs/dynamic.rs`](https://github.com/bevyengine/bevy/blob/main/examples/ecs/dynamic.rs)).

**flecs**: `ecs_component_init` with `.type = { .size, .alignment }` returns a component entity;
lifecycle via `ecs_set_hooks` with `ecs_type_hooks_t` (`ctor, dtor, copy, move, copy_ctor,
move_ctor, ctor_move_dtor, move_dtor, cmp, equals, on_add, on_set, on_remove, on_replace, …`), and
"when type hooks are configured and a hook is left to NULL, it is assumed that the type has no
behavior for that hook".

**Unity DOTS is the opposite**: `TypeManager.Initialize()` scans assemblies once, before use;
`MaximumTypesCount = 1 << 13` (8192); `TypeInfo` in a `static NativeArray<TypeInfo>` plus a
`SharedStatic` for Burst. Late registration exists only as `TypeManager.AddNewComponentTypes`, and
the changelog scopes it to **"(in Editor builds)"**. Community DOTS modding therefore ships
Burst-compiled native libs.

**Registration cost, measured:** Bevy's `inventory`-based reflect auto-registration is
"**<40 ms on debug wasm build and <25 ms on debug native build for 1614 registered types**"; wasm
size +6.75 % (`-Oz`) / +11.46 % (full wasm-bindgen)
([bevy#15030](https://github.com/bevyengine/bevy/pull/15030), merged 2025-08-06). `linkme` is the
no-life-before-main alternative — "static elements are gathered into a contiguous section of the
binary by the linker" ([docs.rs 0.3.37](https://docs.rs/linkme/latest/linkme/)) — **but it was
already rejected in this tree**, see §12.1.

---

## 6. Queries over mod-defined types

Two designs, priced differently.

**(a) Mod-side monomorphisation (the "free" path).** The mod links the engine as an rlib and
instantiates `Query<&ModComp>` itself. Iteration is native, inlined, SIMD-able — identical to engine
code (§2.3). The cost is not runtime but *binding*: same rustc, same crate metadata, same features,
same codegen flags, and per-binary duplicated statics. `sdylib` cannot help: it is `extern "C"` and
stable-repr only, **no generics**.

**(b) Engine-side dynamic query.** Bevy's `QueryBuilder` + `FilteredEntityRef/Mut`
([bevy#9774](https://github.com/bevyengine/bevy/pull/9774), 0.13, 2024-01-16). Its authors state the
cost: "currently they have to determine the accesses a query has in a given archetype during
iteration which is far from ideal". [bevy#16396](https://github.com/bevyengine/bevy/pull/16396) made
`QueryBuilder::is_dense()` depend on `D::IS_DENSE && F::IS_DENSE`. The open follow-up
([bevy#25797](https://github.com/bevyengine/bevy/issues/25797), chescock, 2026-09-15) lists what
still costs: `get_by_id` must "look up the index of the column" with nowhere to cache it; no
non-archetypal (`Changed<C>`) dynamic filters; and it concedes structured builders "won't fully
recapture static query efficiency" because `Fetch` is dynamically sized and `StorageType` must be
branched at runtime.

EnTT documents the gap qualitatively: compile-time views "can make several optimizations because of
that"; `runtime_view` "is constructed at runtime using numerical type identifiers … and is a bit
slower to iterate", and exposes no `get`. flecs makes (b) the only path, so there is nothing to
compare against.

> **Published numbers for dynamic vs static component access: none found.** Searched Bevy PRs/issues
> (#9774, #16396, #25797, #2495), EnTT docs, flecs docs/FAQ, and the ECS benchmark suites
> (`rust-gamedev/ecs_bench_suite`, `abeimler/ecs_benchmark`). **The dynamic-query penalty in this
> field is undocumented in numbers and must be measured locally, not assumed.**

**The scale anchor that decides granularity.** This engine's query iteration is **3.4–4.0 ns/row**
(`docs/aether-v2/DECISIONS.md:372`, `docs/archive/ENABLE-TAG-RESULTS.md:95-101`). A Wasmtime
guest→host call is ≥10 ns (§7). So a per-row boundary crossing is catastrophic and a per-system-run
crossing is free by comparison. **The whole design question reduces to: can the mod's hot loop live
entirely inside the mod?**

---

## 7. Sandboxing, priced

| Runtime / measurement | Number | Source |
|---|---|---|
| Wasmtime 46.0 vs native, libsodium geomean, 2026 | **2.41×** (1.46× with `wide_arithmetic`) | [00f.net, 2026-06-23](https://00f.net/2026/06/23/webassembly-runtimes-2026/) |
| Wasmer 7.1 | 1.33× | same |
| WAMR | 1.57× | same |
| WasmEdge AOT | 1.74× | same |
| SPEC CPU via browsers | **+45 % (Firefox) / +55 % (Chrome)**, peaks 2.08× / 2.5× | Jangda et al., USENIX ATC '19 ([paper](https://www.usenix.org/conference/atc19/presentation/jangda)) |
| Firefox RLBox/wasm2c, SoundTouch | **17 %** over native+SIMD (≈200 % before SIMD) | [bugzilla 1829765](https://bugzilla.mozilla.org/show_bug.cgi?id=1829765) |
| Wasmtime explicit bounds checks (no 4 GiB reservation + guard) | **1.2×–1.8×**; disabling signal-based traps up to 2× | [Wasmtime docs](https://docs.wasmtime.dev/examples-fast-execution.html) |
| `memory64` | 10 % to >100 %, because bounds checks can no longer be elided | [SpiderMonkey, 2025-01-15](https://spidermonkey.dev/blog/2025/01/15/is-memory64-actually-worth-using.html) |
| Wasmtime guest→host call | "as little as **10 nanoseconds**"; minimal runtime 2.1 MB (2023) | [Bytecode Alliance](https://bytecodealliance.org/articles/wasmtime-and-cranelift-in-2023) |
| Wasmtime criterion (machine unstated) | wasm→host typed nop **7.2–7.8 ns**; host→wasm **27.7–33.8 ns**; component-model async guest→host ~280 ns, host→guest ~1.4 µs | [wasmtime#14280](https://github.com/bytecodealliance/wasmtime/pull/14280) |
| Fuel vs epoch metering | fuel ≈2× worse | [Wasmtime 10](https://bytecodealliance.org/articles/wasmtime-10-performance) |
| SFI (NaCl era) | <5 % ARM, 7 % x86-64 | Sehr et al., USENIX Security 2010 ([paper](https://cliffle.com/pub/native-client-paper/)) |
| LFI (ASPLOS '24) | ~7 % ARM64 for the compatible subset | [Yedidia](https://www.scs.stanford.edu/~zyedidia/docs/papers/lfi.pdf) |
| Segue & ColorGuard (ASPLOS '25) | "speedups ranging from 13.8 % for SPECint 2006" via x86-64 segmentation | [abstract](https://vahldiek.github.io/publication/narayan-2022/) |
| bpftime / EIM (OSDI '25), nginx extension | **2 %** vs **11–12 %** for Wasm and Lua on the same task | [USENIX](https://www.usenix.org/conference/osdi25/presentation/zheng-yusheng), [talk](https://eunomia.dev/others/miscellaneous/osdi-talk/) |
| Cranelift codegen quality | "~2 % slower than V8 (TurboFan), ~14 % slower than WAVM (LLVM)"; LWN: "approximately twice as slow on some benchmarks" for the rustc backend | [README](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md), [LWN 2024-03-15](https://lwn.net/Articles/964735/) |

**The structural blocker for this engine, not a constant factor:** **Wasm SIMD is fixed 128-bit**
(phase 4); ≥256-bit "flexible vectors" is **phase 1**
([proposal](https://github.com/WebAssembly/flexible-vectors/blob/main/proposals/flexible-vectors/Overview.md)).
A wasm mod cannot use the AVX2 baseline this engine assumes. Further frictions: guest memory is a
separate linear memory (host-provided backing exists via `MemoryCreator`, but "any modification of
them outside of wasmtime invoked routines is unsafe",
[docs](https://docs.wasmtime.dev/api/wasmtime/trait.MemoryCreator.html)); `threads`/shared memory is
**tier 2** and unsupported by the pooling allocator
([proposal status](https://docs.wasmtime.dev/stability-wasm-proposals.html)).

**wasm2c/RLBox is the one sandbox design that recovers what the others lose**: compiling the sandbox
to C and building it with the host toolchain restores "profile-guided optimization, inlining across
sandbox boundaries"
([Mozilla, 2021-12-06](https://blog.mozilla.org/attack-and-defense/2021/12/06/webassembly-and-back-again-fine-grained-sandboxing-in-firefox-95/)).

**eBPF cannot express an ECS system**: "BPF programs can only call specific functions exposed as BPF
helpers or kfuncs", 512-byte stack, 1 M-instruction exploration limit
([kernel docs](https://www.kernel.org/doc/html/latest/bpf/bpf_design_QA.html)).

**`rustc_codegen_cranelift` is a debug-build tool**, not a shipping path: `std::arch` only partially
supported, unwinding unsupported on Windows (`-Cpanic=abort` default), JIT mode "highly
experimental… requires all dependencies to be available as dynamic library"
([repo](https://github.com/rust-lang/rustc_codegen_cranelift),
[usage.md](https://raw.githubusercontent.com/rust-lang/rustc_codegen_cranelift/master/docs/usage.md)).

**Trust, if no sandbox.** Fractureiser distributed malware through compromised mod accounts on
CurseForge/BukkitDev, Feb–Jun 2023 ([writeup](https://github.com/trigram-mrp/fractureiser)).
Windows Smart App Control admits only RSA-signed binaries from trusted providers
([MS](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control)).
**No native modding path surveyed has a sandbox.**

---

## 8. Unloading — the part nobody has solved

- **abi_stable**: "The library is leaked so that the root module loader can do anything incompatible
  with library unloading"; unloading was removed because it "was possibly unsound".
- **Bevy cannot unregister at all**: [bevy#17564](https://github.com/bevyengine/bevy/issues/17564)
  (alice-i-cecile, 2025-01-27, open) exists to "support removing archetypes and unregistering
  components" and names the blocker — bevy_ecs assumes "archetype and component ids are dense and
  strictly increasing".
- **flecs** says a component can be unregistered by deleting its entity — and **contradicts
  itself**: `ComponentTraits.md` lists the OnDelete cleanup actions as "`Remove` … (default) /
  `Delete` / `Panic`: throw a fatal error **(default for components)**". Treat the default as
  unsettled and read the code before relying on it.
- **Fyrox** serialises plugin entities to an in-memory blob before unload and restores after,
  walking the scene for objects whose assembly name matches; requires `Visit` on essentially all
  plugin state; warns that trait objects hold vtable pointers that "can be easily invalidated if the
  plugin is unloaded" and that some dangling objects "could be overlooked by this system, which
  could result in crash or memory corruption".
- **Unity Burst**: additional libraries unload only on Play-mode exit / player quit.
- **Rust-level**: UCG#526 (§1.5) is open; glibc `NODELETE` (§1.5) makes it a no-op in practice.
- **The Machinery's warning applies to all of them**: "hot-reload code paths aren't exercised in
  normal operation".

---

## 9. How optionality is achieved — and who fails

**Achieves it:**

- **Bevy `hotpatching` (0.17)** is the exemplar. `#[cfg(feature = "hotpatching")]` at the single call
  site in `FunctionSystem::run_unsafe`, falling back to `self.func.run(input, params)` — and the
  extra state (`current_ptr: subsecond::HotFnPtr`) is also cfg'd out, so the struct layout differs
  between the two builds. Bevy removes even the **trait method** under the cfg. The PR author:
  "Everything in second commit is feature gated, it has no impact when the feature is not enabled."
  ([bevy#19309](https://github.com/bevyengine/bevy/pull/19309), merged 2025-06-03;
  [`function_system.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ecs/src/system/function_system.rs)
  at commit `2d63624`, 2026-08-31.) In `multi_threaded.rs` the refresh is not even per-run — it is
  guarded by `if hotpatch_tick.is_newer_than(...)`.
- **hot-lib-reloader** ships the gating idiom for exports:
  `#[cfg_attr(feature = "reload", unsafe(no_mangle))]`, "To not pay a penalty for exposing functions
  using `#[unsafe(no_mangle)]` in release mode"
  ([repo](https://github.com/rksm/hot-lib-reloader-rs)).
- **Separate crate, not a feature of the kernel** — `bevy_dylib`, the former `bevy_dynamic_plugin`:
  nothing to compile in at all.
- **Unreal's `MODULE_API`** — empty in monolithic builds (§3.2).
- **flecs** — `FLECS_CUSTOM_BUILD` / `FLECS_NO_<addon>`; "Without any addons, Flecs is just a minimal
  ECS storage".
- **Fyrox's profiles** — `dev-hot-reload` / `release-hot-reload` are *different builds*, not a
  flagged one.
- **subsecond** — enabled only under `debug_assertions`, so it cannot ship mods by construction.

**Forces a dynamic path on everyone (violates the constraint):**

- **Fyrox's own plugin container**: `PluginContainer { Static(Box<dyn Plugin>), Dynamic(Box<dyn DynamicPlugin>) }`
  is **not** cfg-gated ([`plugin/mod.rs`](https://raw.githubusercontent.com/FyroxEngine/Fyrox/master/fyrox-impl/src/plugin/mod.rs)) —
  every game pays the enum match and the `dyn` hop whether or not it hot-reloads. Static plugins are
  `Box<dyn Plugin>` regardless.
- **HL / Source / The Machinery**: all engine-internal calls go through function-pointer structs or
  virtual interfaces.
- **UE Mass**: every fragment is a `UScriptStruct`, every bit index from a runtime `StructTracker` —
  no non-reflective build exists.
- **Unity**: `TypeManager.Initialize()` scans assemblies at startup for every build;
  `MaximumTypesCount` is a fixed 8192 for everyone.
- **Bevy's `Components::indices` `TypeId → ComponentId` hashmap** is consulted by *typed* queries at
  `init_state` — a dynamic registry on the static path, hoisted to init time. This engine's per-type
  `OnceLock` is strictly cheaper and must not be regressed into a map for mods' sake.

**The one mechanism that can silently break optionality is cargo feature unification.** "Cargo will
use the union of all features enabled on that dependency"; features must be additive; mutually
exclusive features are explicitly discouraged
([Cargo book](https://doc.rust-lang.org/cargo/reference/features.html)). A `modding` feature can be
switched on by any third-party crate in the graph, for a game that never wanted it. A separate crate
that nothing else depends on is immune by construction.

---

## 10. Failure history, in one list

1. **Bevy dynamic plugins** — deprecated 0.14 ([PR #13080](https://github.com/bevyengine/bevy/pull/13080)),
   removed 0.15 ([PR #14534](https://github.com/bevyengine/bevy/pull/14534)); reasons at
   [issue #11969](https://github.com/bevyengine/bevy/issues/11969) (BD103, 2024-02-19).
2. **flecs #685** (UE5, 2022) and **#1034** (Windows shared library, 2023) — per-DLL static ids.
3. **Unreal legacy Hot Reload** — leftover "HOTRELOADED" classes, corrupted Blueprint assets
   ([community wiki](https://unrealcommunity.wiki/live-compiling-in-unreal-projects-tp14jcgs)).
4. **SKSE** — "Every update breaks old SKSE mods because the addresses where game functions live
   change" ([CompaSSE](https://github.com/drLemis/CompaSSE)); one SKSE build per game version
   ([skse.silverlock.org](https://skse.silverlock.org/)).
5. **Harmony** — "Methods that are too small might get inlined and your patches will not run";
   cannot add fields or extend enums; patching one generic method patches all reference-type `T`
   ([edge cases](https://harmony.pardeike.net/articles/patching-edgecases.html)).
6. **BepInEx 6 IL2CPP** failed to start on Unity 2021.2+
   ([issue #474](https://github.com/BepInEx/BepInEx/issues/474) — known only from the title).
7. **Tremor's stable-ABI conversion** — **~36 % slowdown, reduced to ~30 %**, with the plugins *still
   statically linked*: the loss came from the `#[repr(C)]` interface and its FFI-safe types
   (`RHashMap` instead of `halfbrown`), not from dynamic loading
   ([nullderef, 2022-07-26](https://nullderef.com/blog/plugin-end/)). **This is the price tag on
   option (B) in the design space.**
8. **Bevy dylib on Linux** — non-optimised Bevy exports 300,000+ symbols vs 18,000 optimised;
   **291,185 relocations** at load, ~150 ms load time with default visibility vs ~5 ms with
   protected visibility
   ([Lattimore, 2024-08-27](https://davidlattimore.github.io/posts/2024/08/27/rust-dylib-rabbit-holes.html)).
9. **Windows export ceiling** — §2.1.
10. **The Machinery ended** (2022) and its licence change asked users to delete the source
    ([report](https://gameworldobserver.com/2022/08/01/our-machinery-terminates-machinery-game-engine-changes-eula)) —
    a reminder that a plugin ABI is a decade-long commitment.

**Measured call costs collected** (microbenchmarks, not engine workloads): direct call **0.82 ns**,
dlopen'd function **1.89 ns**, `abi_stable` trait object **3.23 ns**
([nullderef](https://nullderef.com/blog/plugin-abi-stable/)); godot-rust FFI 248–288 ns before
caching → 7–32 ns after, Windows, release, Godot 4.1.1 `template_release`, 2023-09-05
([godot-rust](https://godot-rust.github.io/dev/ffi-optimizations-benchmarking/)). A community Go
benchmark reports 23–44 ns/op for graphics.gd but its measurement semantics are unclear
([discussion](https://github.com/quaadgras/graphics.gd/discussions/277)).

---

## 11. Academic work

- **Jangda, Powers, Berger, Guha**, "Not So Fast: Analyzing the Performance of WebAssembly vs. Native
  Code", USENIX ATC 2019 — 45–55 % mean slowdown, 2.08–2.5× peak.
  [arXiv:1901.09056](https://arxiv.org/abs/1901.09056)
- **Sehr et al.**, "Adapting Software Fault Isolation to Contemporary CPU Architectures", USENIX
  Security 2010 — SFI <5 % ARM / 7 % x86-64.
- **Yedidia**, "Lightweight Fault Isolation", ASPLOS 2024 — ~7 % ARM64.
- **Narayan, Garfinkel et al.**, "Segue & ColorGuard", ASPLOS 2025 — 13.8 % SPECint speedup from
  segmentation.
- **Zheng et al.**, "Extending Applications Safely and Efficiently" (EIM/bpftime), OSDI 2025 — 2 % vs
  11–12 %.
- **Johnson et al.**, "ThinLTO: Scalable and Incremental LTO", CGO 2017 ([blog summary](https://blog.llvm.org/2016/06/thinlto-scalable-and-incremental-lto.html)).

**No peer-reviewed work on Rust dynamic loading / ABI stability, and none on native game-mod ABIs,
was found.** That area lives in RFCs, compiler PRs and engine source. The nearest normative
documents are [RFC 1510 (cdylib)](https://rust-lang.github.io/rfcs/1510-cdylib.html),
[RFC 2945 (C-unwind)](https://rust-lang.github.io/rfcs/2945-c-unwind-abi.html),
RFC 3435 / `#[export]` ([draft](https://github.com/m-ou-se/rfcs/blob/export/text/0000-export.md)),
[RFC 3470 (crABI, still open, last activity 2024-03)](https://github.com/rust-lang/rfcs/pull/3470),
and the [2025H1 "safe linking" project goal](https://goals.rust-lang.org/2025h1/safe-linking.html).
One framing piece worth reading as opinion, not evidence:
["We Need Type Information, Not Stable ABI"](https://blaz.is/blog/post/we-dont-need-a-stable-abi/)
(2023-01-15).

---

## 12. What boyko-engine has today

All paths relative to the repository root, read at commit
**`d552be05be4b4f063b6cb39ddfd63eb4688f83fd`** (branch `merge/ke16-into-ecsnative`).

### 12.1 Identity: every id is minted into a process-global static, in call order

The shape has two variants.

**Non-generic types** cache the id in a `static OnceLock` *inside the impl body*, minted from a
process-global counter on first call:

| Kind | Trait / accessor | Registry statics |
|---|---|---|
| Component | `crates/boyko_ecs/src/ecs/core/component/component.rs:37` (trait), `:43` (`component_id`) | `…/component_registry/mod.rs:208` (`LAYOUTS`), `:214` (`NEXT_ID`), `:920` (`register_new`) |
| Resource | `crates/boyko_ecs/src/ecs/core/resources/resource.rs:42`, `:48` | `…/resources/resource_registry.rs:123,127` |
| Event | `crates/boyko_ecs/src/ecs/core/events/event.rs:18`, `:30` | `…/events/event_registry.rs:87,93` |
| Bundle | — | `crates/boyko_macros/src/bundle.rs:327-355` → `bundle_type_registry` |

The derive emits it at `crates/boyko_macros/src/component.rs:371-372`:

```rust
fn component_id() -> boyko_ecs::ecs::identifiers::primitives::ComponentId {
    static ID: ::std::sync::OnceLock<…ComponentId> = …;
```

**Generic types cannot use that shape** — a `static` in a generic fn body is shared by all
instantiations ([rust#22991](https://github.com/rust-lang/rust/issues/22991),
[rfcs#2130](https://github.com/rust-lang/rfcs/pull/2130), both cited in the source at
`crates/boyko_ecs/src/ecs/core/resources/resource_type_registry.rs:19-32`). Those go through
`TypeIntern`, a lock-free open-addressed `TypeId → dense id` table
(`crates/boyko_utils/src/type_intern/mod.rs:1-38`): resources
(`…/resources/resource_type_registry.rs:94`, table `:70`), queries
(`(TypeId::of::<D>(), TypeId::of::<F>()) → QueryTypeId`,
`crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs:206`, counter `:98`), non-send
resources (`…/resources/nonsend_resources.rs:402`), tuple component sets
(`…/iters/component_set.rs:39`), asset backing layouts (`…/asset/backing.rs:87`, still a
`Mutex<HashMap>`).

**Stability, in the source's own words.** `…/component_registry/mod.rs:5-22`: ids are "stable for the
lifetime of the process — but **not** stable across processes or across runs of the same binary if
the order of first calls differs", with an explicit startup warm-up contract for anything ingesting
ids from outside. `events/event_registry.rs:3-22` repeats it for events;
`…/ecs_master/tag_api.rs:44` for dynamic tags: "The numeric id is first-call-order process-unstable;
the **name** is the stable key."

**No linker-collected registry anywhere.** `inventory` / `linkme` / `#[used]` / `link_section` /
`ctor` appear nowhere in the tree, and `linkme` was evaluated and **rejected** at
`docs/archive/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md:149` — "`linkme` requires `dylib`/`bin` target
hooks. Doesn't work on `cdylib`/wasm without custom support; ecosystem fragility too high." That
rejection was about bundle ids; it is directly load-bearing here, because a link-time-collected
registry is the usual way a plugin system self-registers.

### 12.2 Fixed budgets, and what exceeding one does

| Registry | Cap | Site |
|---|---|---|
| `MAX_COMPONENTS` (typed components **and** dynamic tags share it) | **512** | `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs:63` |
| `RESOURCE_SLOT_COUNT` | 256 | `…/resources/resource_registry.rs:51` |
| `NON_SEND_RESOURCE_SLOT_COUNT` | 256 | `…/resources/nonsend_resource_registry.rs:49` |
| `MAX_EVENTS` | 256 | `…/events/event_registry.rs:51` |
| `MAX_QUERY_TYPES` | 1024 (4096 under `big_query_table`) | `…/iters/query/query_type_registry.rs:85,90` |
| `MAX_BUNDLE_TYPES` | 1024 | `…/bundle/bundle_type_registry.rs:84` |
| `MAX_TRIGGERS` | 256 | `…/component/observers/trigger.rs:37` |
| `MAX_ARCHETYPES` | 1024 | `…/iters/archetype_bit_set.rs:7` |
| `MAX_SYSTEMS_PER_SCHEDULE` | 1024 | `…/schedule/schedule_builder.rs:71` |
| `MAX_EVENT_THREADS` | 65 (= `MAX_WORKERS` + 1, tied by a `const _: () = assert!`) | `crates/boyko_ecs/src/ecs/constants.rs:418` |

Exhaustion is a **panic**, not an error (`register_new`'s `assert!` at
`…/component_registry/mod.rs:922`; `QUERY_NEXT_ID` saturates then panics,
`query_type_registry.rs:133-135`). `ComponentId` is also a structural constant: `ComponentMask` is
`[BitSet<u64>; 8]` = 512 bits and every `Archetype` carries an inline `columns: [Column; MAX_COMPONENTS]`,
so raising the cap is not free — `Access` is pinned at 192 B by
`const _: () = assert!(core::mem::size_of::<Access>() == 192);`
(`crates/boyko_ecs/src/ecs/core/system/access.rs:61`, "3 cache lines, no padding", `:56`).

Approximate occupancy today (grep of `derive(Component)` / `derive(Resource)` in `src`, includes
doc-comment examples — an upper bound, not a census): components ≈126 of 512; **resources ≈145 of
256**, of which `boyko_render` alone ≈89. Resources are the tight budget, and generic resources
(`State<S>`, `Assets<T>`, `ActionState<A>`) mint from the same counter.

### 12.3 Stable identity already exists — and is the good news

`Component` carries `FORMAT_VERSION` (`…/component/component.rs:174`), `LAYOUT_FINGERPRINT` (`:181`,
"best-effort hash of (size, align, repr, per-field offsets, field_count)") and `stable_name()`
(`:195`, defaulting to `type_name`, overridable by `#[component(stable_name = "…")]`). The load-time
index is FNV-1a-64 of that name → candidate ids with a full-string confirm:
`…/component_registry/serialize.rs:363` (`STABLE_NAME_INDEX`), `:383` (`register_stable_name`),
`:410` (`resolve_stable_name`).

This is the same design Unity reached (§4), with the layout hash separated from the name key. **Save
compatibility is already name-keyed and version/fingerprint-checked, not id-keyed.** A mod's
components should carry an explicit `stable_name`, not the `type_name` default (§1.2).

### 12.4 What can be minted from *data* at runtime — exactly one thing

**Zero-sized dynamic tags, keyed by name.** `EcsMaster::try_register_tag` /
`register_tag` (`…/ecs_master/tag_api.rs:47,65`) → `try_register_tag_by_name`
(`…/component_registry/tags.rs:182`), interning into
`static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>>` (`tags.rs:155`), with
`ComponentLayout::new_dynamic_tag` (size 0, align 1, no drop, shared sentinel `TypeId`) at
`…/component_registry/mod.rs:173`.

There is **no runtime path to register a *data* component from a size/align/drop triple**.
`try_register_dynamic(layout) -> Option<ComponentId>` exists
(`…/component_registry/mod.rs:967`) and is used today by
`register_asset_layout<T>(drop_fn)` (`…/asset/backing.rs:115`) with a **caller-supplied drop glue** —
the exact shape a mod would need — but it is `pub(crate)`. **Correction (design space rev 5, pass 4
C1): the dispenser is `pub(crate)`, but its asset caller is not.** `register_asset_layout<T>` is
`pub` and re-exported at `…/asset/mod.rs:60`; `Assets::<T>::with_reserved` (`…/asset/assets.rs:231`,
`pub`, `#[inline]`, generic; `Default` is `with_reserved(0)`, `:1052`) calls it through
`AssetBacking::register_layout`, and `impl_asset_pod_backing!` is `#[macro_export]` and
`$crate`-qualified (`backing.rs:142-160`) so any crate can implement the trait for its own type. A
foreign crate can therefore mint a `ComponentId` today, generically, by constructing an asset store
— type-driven rather than data-driven, so the sentence above stands, but the *reachability* it
implies does not. The path returns `None` at the cap and the caller `.expect`s
(`backing.rs:131-132`); it does **not** carry `register_new`'s assert (`mod.rs:951-953`).
`register_layout<T>` (`…/component_registry/mod.rs:1031`) is `#[doc(hidden)]`, generic over a Rust
`T`, and sanctioned only for tests and reserved-band scratch columns.

`ComponentLayout` itself is at `…/component_registry/mod.rs:109` and stores `size, alignment,
drop_fn, type_name, type_id`.

### 12.5 Systems: virtual dispatch already exists, so a mod system is not a new cost class

- `System` is an **`unsafe trait`** (`…/system/system.rs:57`) with
  `unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>)` (`:95`).
- `ScheduleBuilder::add_system` (`…/schedule/schedule_builder.rs:177`) erases:
  `let boxed: Box<dyn System<Out = ()>> = Box::new(sys);` (`:183`), wrapped in `SystemBox`
  (`…/schedule/system_box.rs:83`, the field). `Schedule` owns `systems: Vec<SystemBox>` in
  topological order (`…/schedule/schedule.rs:122`). The module header states it:
  "The schedule executes systems via `Box<dyn System<Out = ()>>`"
  (`…/system/exclusive_function_system.rs:25`).
- **A mod system dropped into the same `Vec<SystemBox>` costs exactly what an engine system costs.**
  *(Design space rev 6, pass 5 C2: true of the **dispatch** — one vtable call per system run — and
  false of the **registration**: `add_system`'s `Box::new` at `:183` is monomorphised in the calling
  crate, so under a two-image option the box is mod-allocated and host-freed, the cross-image
  allocation item 8 of §15 warns about. The design's §7.9(c) moves the allocation host-side behind a
  fn-pointer seam, at one extra indirect call per mod-system run — the same per-system class.)*
- What is *not* erased is the body: `FunctionSystem<F, M>` caches
  `<F::Param as SystemParam>::State` at `initialize` and the param fetch is monomorphic; query
  iteration is generic code instantiated in the **caller's** crate (§2.3).
- Conflict analysis is bitmask-over-ids (`Access`, §12.2), so a mod system's parallelism is decided
  by the same `ComponentId`/`ResourceId` bits as everyone else's.
- Run conditions are `Box<dyn System<Out = bool>>` (`…/schedule/system_box.rs:41`) kept **outside**
  `SystemBox`, behind a `has_condition` bitset whose all-zero state is the documented 0%-gate
  (`…/schedule/schedule.rs:132-137,189`).

### 12.6 Schedules are frozen at `finish()`

- The closed set is `CoreSchedule::{Main, Fixed}` (`…/app/app.rs:64`), with the comment "New
  top-level slots are an engine change by design." **There is no Render schedule in the code**;
  rendering is imperative host code in `crates/boyko_app/src/runner.rs` (`frame_loop`). A
  `CoreSchedule::Render` exists only as design
  (`D:/claude/BoykoEngine/docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:157`).
- `ScheduleBuilder::build` (`…/schedule/schedule_builder.rs:324` / `try_build` `:350`) consumes the
  builder: initialize → Tarjan SCC acyclicity → Kahn topo-sort → conflict bitsets → `Schedule`.
  **After that the schedule is frozen**: `Schedule`'s only public mutator is `run`
  (`…/schedule/schedule.rs:286`); there is no `add_system` / `remove_system` / rebuild API.
- `Schedule::run` release-panics (`boyko-B9101`, `schedule.rs:43,286`) if handed a world whose
  `WorldId` differs from the one it was built on.
- **Config phase vs run phase is a hard wall.** Every config method calls `assert_config_phase`
  (`…/app/app.rs:253`) and post-`finish` panics `boyko-B1802` (many sites, `:272` onward).
  `finish()` (`:583`) resolves the event policy, inserts clocks, **builds both schedules**, sets
  `finished = true`, then drains the one-shot startup closures — so **a startup system cannot add
  systems**. `insert_resource` (`:261`) and `world_mut()` (`:570`) carry **no** config-phase guard.
- `Plugin` is `pub trait Plugin: 'static { fn build(&self, app: &mut App); }`
  (`…/app/plugin.rs:27`), **not sealed**, de-duplicated on `TypeId::of::<P>()` (`:26`, the doc
  comment), added at `…/app/app.rs:550`; a duplicate is a panic (`boyko-B1801`). The `Plugins` tuple
  trait *is* sealed; `Plugin` itself is the extension point.

Runtime-mutable extension points that survive `finish`: **observers**
(`…/ecs_master/observer_api.rs:182` `add_observer`, `:199` `remove_observer`), one-shot systems
(`…/ecs_master/system_api.rs`), dynamic tags, resources, entities. **Closed** after archetyping:
lifecycle hooks — `register_hooks_by_id` (`…/component_registry/mod.rs:887`) refuses with
`HooksError::AlreadyArchetyped` once the component has been in an archetype, and `ArchetypeFlags`
are OR-computed once at archetype mint.

### 12.7 Memory, allocator, threading

- `ComponentPool` storage is **not on the heap**: each pool owns a `VmReservation` —
  `VirtualAlloc(MEM_RESERVE)` / lazy `MEM_COMMIT` on Windows, `mmap`/`mprotect` on unix
  (`crates/boyko_ecs/src/ecs/memory/vm.rs:63` the `unsafe extern "system"` import block, `:109`
  `reserve`, `:199` `commit`). These are process-wide OS calls with no allocator identity, so
  **address stability and page ownership survive a second binary**: a pointer minted in the exe is
  valid in a DLL.
- **There is no `#[global_allocator]` in the engine.** The only ones are opt-in test/bench shims
  (`crates/boyko_app/src/profiling/alloc_shim.rs`, behind `profiling-alloc`, whose comment states
  the hazard: "A crate that declares a `#[global_allocator]` forces every binary linking it to use
  that one."). The owner's allocator ruling is at
  `D:/claude/BoykoEngine/docs/memory/ALLOCATOR-DESIGN-SPACE.md:63`, and `:482` (O6) already flags the
  modding-adjacent risk: making `VmReservation::reserve/commit/base` public would make them "a side
  allocator" plugins could use.
- **What *does* assume one binary** is `ComponentLayout::drop_fn` (`…/component_registry/mod.rs:109`
  the struct) and the parallel fn-ptr tables: `CLONE` (`…/component_registry/clone.rs:100`),
  `MAP_ENTITIES` (`clone.rs:109`), `SERIALIZE` (`serialize.rs:164`), `FLAGS_DIRECT`
  (`flags.rs:91`), `HOOKS` (`mod.rs:226`), `STORAGE_KIND` (`mod.rs:375`), `RESIDENCY_CLASS`
  (`mod.rs:503`), `EVER_ARCHETYPED` (`mod.rs:824`). **Every slot is write-once `OnceLock`/atomic,
  never cleared — there is no deregistration path for anything. Loading is expressible; unloading is
  not.**
- **Threads.** `boyko_threadpool` has three thread-locals (`crates/boyko_threadpool/src/tls.rs`):
  `ACTIVE_POOL` (`:169`), `LANE_DEPOSIT` (the lane-identity predicate, invariants D1–D6 at `:23-70`),
  and `IN_SYSTEM_RUN`. **A duplicated `ACTIVE_POOL` fails silently, not loudly**: invariant **PAR7**
  (`…/iters/query/par_iter.rs:28`) is "when no active pool is attached to the calling thread, …" fall
  back to sequential iteration — so a mod whose copy of the TLS key reads null runs its parallel
  iteration serially **with no diagnostic**. Lane identity is indexed and exactly sized
  (`MAX_EVENT_THREADS = 65`, `constants.rs:418`), so a mod that spawns its own threads is an
  unattached sender.

### 12.8 No loadable artifact, and no C seam

- **Every engine crate is an rlib.** There is not one `crate-type` key in the workspace (`grep -rn
  "crate-type" --include=Cargo.toml` over the repository → **zero hits**, verified at `d552be05`).
  A game is one binary with the whole engine statically linked and, since 2026-09-04, **fat LTO**
  (`Cargo.toml:112`). **There is no "the engine" as a loadable artifact today.**
- **There is no `extern "C"` seam anywhere in the kernel.** `grep -rn 'extern "C"' crates/boyko_ecs/src
  crates/boyko_log/src crates/boyko_diag/src` → **zero hits**, verified at `d552be05`. The only
  `extern "system"` is the OS import side of `VmReservation` (`…/memory/vm.rs:63`). The `extern "C" fn`
  pre-flush seam that `docs/LOGGING-SYSTEM-PLAN.md:1642` designs "so it crosses a dylib boundary with
  no vtable and no allocation" is **planned, not shipped**.
- **Raw dynamic loading is already in-house and dependency-free**:
  `crates/boyko_rhi_vulkan/src/device.rs:1660` does `LoadLibraryA` + `GetProcAddress` + `FreeLibrary`
  with `// SAFETY:` comments (the Vulkan loader). If a mod loader is needed, that is the precedent —
  it adds no ABI guarantee.
- The Vulkan context is a **process singleton** in one `static SINGLETON: AtomicPtr<VulkanContext>`
  (`crates/boyko_rhi_vulkan/src/device.rs:783`); a second boot is `SingletonAlreadyBooted`.

### 12.9 Content extension points are compile-time closed

- **Asset loaders**: `HasLoaders::LOADERS: &'static [LoaderEntry<Self>]`
  (`crates/boyko_ecs/src/ecs/core/asset/loader.rs:68`), a const table resolved by linear scan, which
  **explicitly replaced** a runtime `HashMap<String, DecodeFn>` + `dyn Any` registry (`:58`). A
  mod-provided loader for an engine asset type cannot be expressed by a const slice — that is a
  design fork, not an implementation detail. (Bevy's loaders are the opposite:
  `Arc<dyn ErasedAssetLoader>` in a `Vec` plus an extension `HashMap`.)
- **Shaders** ship as `include_bytes!`-embedded `.spv` statics, byte-gated by re-DXC tests. The RHI
  *does* expose runtime `create_shader_module(&[u32])` / `create_compute_pipeline`
  (`crates/boyko_rhi/src/device.rs:513,523`), so mod shaders are mechanically possible — but nothing
  loads SPIR-V from a file today, and the variant manifest assumes a closed set.
- **Aether is a proc-macro DSL, not a language runtime**: `crates/aether/src/lib.rs` is a shim over
  `aether_lang::expand_block`, emitting `#[derive(Component)]` structs, `pub fn` systems and
  `state_chart!` invocations. No interpreter, no bytecode, no VM. Aether-authored mod content still
  requires rustc. Gaia's pipeline is "own text → build-time bake with reflection → binary, **zero
  runtime reflection**" — a data-mod path means shipping the baker, not an interpreter.
- **Reflection lives only in `D:/wt/reflect`** (`crates/boyko_reflect`), not in this tree. Its gating
  discipline is the best in-tree precedent for the owner's constraint (§12.11).

### 12.10 Panics, isolation, error channel

- Worker task bodies are wrapped in `catch_unwind`
  (`crates/boyko_threadpool/src/task/scoped.rs`, `worker.rs`); the first payload is re-raised on the
  dispatcher in `Scope::Drop`, documented as TPN9/SCH11 at `…/schedule/schedule.rs:277-278`.
- **Nothing above that catches it**: `crates/boyko_app/src` has no `catch_unwind` outside a test. A
  panic unwinds out of `update_with_delta` → `frame_loop` → `run_windowed` → `App::run` → `main`,
  with no Vulkan teardown.
- The commonest mod-authored failure — naming a resource that is not inserted — is a **release panic
  at param fetch** (`…/system/params/diagnostics.rs`, `missing_resource_panic`). An owner-session
  record (2026-08-26, not re-verified) says that under the windowed runner this presents as a frozen
  window rather than a crash, with the panic text lost to stderr.
- **There is no per-system isolation, no sandbox, no watchdog.** A mod system is exactly as trusted
  as an engine system. The profile is `panic = unwind` (no `panic` key in any profile), so an unwind
  out of any future `extern "C"` seam aborts the process (§1.3).

### 12.11 The optionality precedents the tree already has

| Mechanism | How "off" costs nothing | Site |
|---|---|---|
| **Optional dependency behind a cargo feature** | the crate is absent from the resolved closure: "not compiled, no rlib, no symbols"; proven by `cargo tree` plus a 3-leg symbol census with a present control and a linked-unused discriminator | `D:/wt/reflect/crates/boyko_reflect/src/lib.rs:1-26`; gates at `D:/wt/reflect/docs/REFLECTION-PLAN-GATES.md:788-850` |
| **Compile-time const on the trait** (`HAS_HOOKS`, `STORAGE_IS_BITSET`, `STORAGE_IS_DENSE`, `RESIDENCY`, `CLONE_BEHAVIOR`, `SERIALIZABILITY`) | `if const { … }` const-folds the install and every downstream branch away; every discriminator defaults to the zero-cost arm | `crates/boyko_ecs/src/ecs/core/component/component.rs:51,65,79,174,181` |
| **Per-archetype flag bits OR-computed once at mint** | one `u16` load + `test/jz`, predicted not-taken when no archetype declares the feature | `…/component/hooks/archetype_flags.rs:1-12` |
| **All-zero bitset gates in the schedule** (`has_condition`, `may_defer`) | "THE 0%-GATE … all-zero when no `.run_if` anywhere" | `…/schedule/schedule.rs:132-137,189` |
| **Build-script profile axis** (`BOYKO_PROFILE`) | constants, not branches; one build script at the bottom of the graph | `crates/boyko_diag/build.rs` |
| **The `editor` / `hwrt` feature precedent** | "a `not(hwrt)` build is TEXTUALLY the pre-feature code"; no engine crate depends on `boyko_editor` | `docs/editor/EDITOR-BOUNDARY.md` §2.1 |

**⚠ The measured trap those gates exist for** (`D:/wt/reflect/docs/REFLECTION-ANALYSIS.md:1467-1494`,
measured on `x86_64-pc-windows-gnu` release): a whole-image **symbol census is undecidable at default
release** — the symbol reads 1 in both legs; `--gc-sections` has "no effect whatsoever"; only
`lto="fat", codegen-units=1` makes it 0. "A dependency rlib's plain functions are codegen'd and
carried into the image whether or not anything can reach them." **Generic functions are the decidable
case; plain ones are not.** Any "modding costs nothing" gate must account for this or it will read
green from the wrong mechanism.

**Two name-keyed dynamic registries were already written with mods in mind**: `boyko_log::target`
dynamic targets — "registering `\"mod:acme\"` twice returns the same `TargetId`, which is what lets
two independently-loaded mods name one category without coordinating"
(`crates/boyko_log/src/target.rs:759-763`) — and `boyko_diag`'s dynamic zone registry, whose module
header names "a mod's manifest" as its motivating case and allocates from `.bss` arenas
(`crates/boyko_diag/src/profiling_abi/dyn_registry.rs:1-45`). The profiler also already partitions
samples `Region::Engine` vs `Region::User`, with `User` documented as "games, plugins, **mods**,
tools, benches" (`crates/boyko_diag/src/sample.rs`).

### 12.12 The adjacent ruling already on the books

`docs/editor/EDITOR-BOUNDARY.md:410` — **"## 6. Code hot reload — refused, with the reason"**. Not
deferred; refused. The document names this engine's three aggravating factors verbatim:

> component ids and archetype signatures are minted into PROCESS-GLOBAL registries at first touch;
> storage is address-stable `VmReservation` memory whose provenance is baked into query state; and
> the hot path is full of fn-ptr tables (`CLONE`, `MAP_ENTITIES`, `SERIALIZE`, `HOOKS`) holding
> pointers INTO the code a dylib unload would invalidate

**That ruling is about *unloading*. Loading-and-never-unloading is not covered by it, and that
distinction is the one live opening.** The same document records the reachable substitute — data hot
reload, already shipping for `.ui` (`crates/boyko_ui/src/reload/`).

Mods were also ratified **out of scope** in `docs/OPEN-QUESTIONS.md` F7 (2026-08-30) — a prior
decision this campaign is reopening, not overriding silently.

### 12.13 Where a mod loader could sit today

```
main()
  ├ App::new()
  ├ app.add_plugins(EnginePlugins::window(...))     // installs the runner
  ├ app.add_plugins(ModLoaderPlugin)                // ← HERE: config phase
  │     └ for each mod dll: load, call its entry(&mut App)
  └ app.run()  →  runner: device boot → world residents → app.finish() → frame_loop
```

Anything later is refused (§12.6). A mod loaded mid-session could only add **resources, entities,
dynamic tags, observers and one-shot systems** — not scheduled systems, not states, not hooks on
already-archetyped components.

---

## 13. Measurements taken during this research

Owner's workstation, `rustc 1.98.1 (48a229cea 2026-09-01)`, `x86_64-pc-windows-gnu`. Micro-measurements,
not workload numbers; each timing loop ran under a second. **These are the only numbers here that
were produced for this question rather than cited.**

**M-1 — a `cdylib` mod gets its OWN copy of every process-global registry.** rlib `reg` with
`static NEXT_ID: AtomicUsize`; a `cdylib` linking it; an exe linking it; the exe `LoadLibraryA`s the
DLL and calls in:

```
host mint #1 = 0      host NEXT_ID addr = 0x7ff7adb81080
host mint #2 = 1
mod  mint #1 = 0      mod  NEXT_ID addr = 0x7ffefc0000a0   ← second registry
host mint #3 = 2
```

**This is the decisive result for the naive design.** With the cdylib route, `NEXT_ID` / `LAYOUTS` /
`TypeIntern` / `NEXT_WORLD_ID` and every TLS key are duplicated, and the failure mode is a **silently
wrong id**, not a link error: the mod's `Transform::component_id()` would index the host world's
`[Column; 512]` at the wrong slot, and the `WorldId` gate at `schedule.rs:286` could not catch it
because both counters start at 0.

**M-2 — a Rust `dylib` shares them.** Same sources, `reg` built `--crate-type=dylib -C prefer-dynamic`,
everything `prefer-dynamic`:

```
host mint #1 = 0      host NEXT_ID addr = 0x7fff411490a0
host mint #2 = 1
mod  mint #1 = 2      mod  NEXT_ID addr = 0x7fff411490a0   ← same address
host mint #3 = 3
```

Price observed in the same run: `host.exe` refused to start with
`error while loading shared libraries: std-3a7e791385001413.dll` until the toolchain's lib dir was on
`PATH` — std, core, alloc, compiler_builtins and libc must all ship and resolve as DLLs. A dylib built
*without* `-C prefer-dynamic` cannot be linked at all:
`error: cannot satisfy dependencies so 'std' only shows up once`.

**M-3 — the Rust-dylib route is mutually exclusive with the shipped release profile.** Both measured:

```
$ rustc --crate-type=dylib -C lto=fat eng.rs
error: lto cannot be used for `dylib` crate type without `-Zdylib-lto`

$ rustc -C prefer-dynamic -C lto=fat -L . app.rs
error: cannot prefer dynamic linking when performing LTO
  = note: only 'staticlib', 'bin', and 'cdylib' outputs are supported with LTO
```

The cost of giving that up is this repository's own measured matrix (§2.2): geomean **0.793**,
`query_ref_iter/10k` **0.375**, binary **8.76 → 3.46 MB** (`Cargo.toml:89-112`).

**M-4 — what a boundary call costs, and what it does not.** 200 M iterations, dependent chain,
`-C opt-level=3`, no LTO, two runs each:

| call | static rlib | Rust dylib |
|---|---|---|
| `#[inline(never)]` engine fn | 2.22 / 1.72 ns | 2.43 / 2.50 ns |
| `#[inline]` engine fn | 0.128 / 0.162 ns | 0.138 / 0.145 ns |

The structural point matters more than the ~0.3–0.7 ns delta: **inlinable and generic engine code is
instantiated in the caller's crate even across a dylib boundary** (§2.3). Only opaque, non-inlinable
entry points pay the indirection — and, without LTO, *everything* loses the cross-crate inlining
M-3 prices.

---

### 13.1 Rev 2 — the install-time-relink feasibility probes (2026-09-16)

Same machine and toolchain. These settle the two steps the design space's option C asserted without
establishing (`MODDING-DESIGN-SPACE.md` §3): **who constructs the link line**, and **who emits the
`add_plugin` call**. They are link-and-run experiments, not timings.

**M-5 — a Rust binary CAN be relinked without rustc, on windows-gnu, on stable.**

`rustc --print link-args` prints the complete link line on **stable 1.98.1** (no `-Z` flag). The
line's driver is `x86_64-w64-mingw32-gcc`; by default that resolves to whatever mingw gcc is on
`PATH`, but the toolchain ships **its own** at
`lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained/x86_64-w64-mingw32-gcc.exe` (beside `ld.exe`
and `dlltool.exe`), whose `GCC-WARNING.txt` states it "cannot be used for compiling C files - it is
only used as a linker" — exactly the use here.

Replaying the recorded line with **that** driver, with no rustc in the process:

```
host.rs   : extern "C" { fn mod_entry() -> u32; }   (a bin)
modx.rs   : #[unsafe(no_mangle)] pub extern "C" fn mod_entry() -> u32 { 42 }
            rustc --crate-type=lib --emit=obj -O modx.rs -o modx.o

$ <self-contained>/x86_64-w64-mingw32-gcc.exe <recorded args> -o host_relinked.exe
link exit: 0
$ ./host_relinked.exe
host saw mod_entry() = 42
```

Two traps, both of which a naive attempt hits:

- The line names a **`symbols.o` rustc generates into a temp directory at link time**. `-C save-temps`
  preserves it; without it the replay fails with `cannot find … symbols.o`.
- Replaying the **default** line with the bundled driver fails —
  `ld: cannot find crt2.o … cannot find -lkernel32 …` — because that line was written for the
  external gcc's own sysroot. Building with **`-C link-self-contained=y`** makes rustc emit a line
  that uses the toolchain's own `crt2.o` and `-L …/lib/self-contained`, and *that* line replays
  cleanly. **The self-contained form is the one to ship.**

**M-6 — a linker-collected plugin registry works on windows-gnu, and `--gc-sections` silently
destroys it.** PE grouped sections (`.boykom$a` sentinel · `.boykom$m` entries · `.boykom$z`
sentinel, walked between the sentinels at run time), mod objects added to the link line by the
rustc-free replay of M-5:

| link | `.boykom` size | host reports |
|---|---|---|
| 0 mod objects | 0x10 | `mods=0 sum=0` |
| 1 mod object, **`--gc-sections` on** (rustc's default here) | 0x10 | **`mods=0 sum=0`** |
| 2 mod objects, **`--gc-sections` on** | 0x10 | **`mods=0 sum=0`** |
| 2 mod objects, `--gc-sections` **removed** | 0x20 | `mods=2 sum=49` (42 + 7) ✅ |

**The link succeeds, the binary runs, and the mods do not exist.** No error, no warning. This is the
`linkme` + `--gc-sections` dead-strip hazard class that `REFLECTION-ANALYSIS.md:307-311` (restated
`:742`) and `crates/boyko_log/src/target.rs:978-982` both refuse — reproduced here in the exact
configuration option C would use. Binary size without `--gc-sections`: 4 959 645 B → 4 965 583 B
(**+0.12 %** on a hello-world; meaningless for the engine, which must be measured separately).

**M-7 — the registry is rescued WITHOUT dropping `--gc-sections`, by rooting the right symbol.**

| root passed to the linker | `.boykom` | host reports |
|---|---|---|
| `-Wl,--undefined=<mod entry FUNCTION>` | 0x10 | **`mods=0 sum=0`** — insufficient |
| `-Wl,--undefined=<mod registration STATIC>` | 0x18 | `mods=1 sum=100` ✅ |

So the workable convention is one
`#[used] #[unsafe(no_mangle)] #[unsafe(link_section = ".boykom$m")]` registration **static** per mod
with a conventional name, and the installer passes `-Wl,--undefined=` for each — a name it already
has from the mod manifest. Rooting the entry *function* does **not** save the registration data.
Because the installer counts the roots it passed, the host can assert at boot that the collected
count equals the manifest count, which converts the silent failure of M-6 into a loud one.

**M-8 — what the install step would actually have to ship, measured on this box.**

| item | size |
|---|---|
| `bin/self-contained/` (mingw gcc driver 2.84 MB, `ld.exe` 1.91 MB, `dlltool.exe` 1.33 MB, `libwinpthread-1.dll` 54 KB) | **6.0 MB** |
| `lib/self-contained/` (`crt2.o` + 44 mingw import/static libs) | **26.8 MB** |
| the 20 std rlibs the link line names | **15.1 MB** |
| **⇒ linker + runtime libraries, total** | **≈48 MB** (before any engine objects) |
| `rust-lld.exe` **alone**, for comparison | **211 MB** |
| full `stable-x86_64-pc-windows-gnu` toolchain on this box (`bin` 529 MB, `lib/rustlib` 1373 MB, docs 865 MB) | **2772 MB** |

Two consequences. First, the design space's "on Windows the relink must use `rust-lld`/`lld-link`"
was **wrong on this target in both directions**: the bundled mingw driver works (M-5) and `rust-lld`
is ~4× the size of the entire alternative. Second, option D's "302 MB class compiler" (the Arch
figure) is a **floor**, not a Windows estimate.

---

### 13.2 Rev 3 — what the installed toolchain contains, and what it does not (2026-09-16)

Same machine and toolchain. Not a timing: a **reading of the installed files**, taken because rev 2's
option C attached a performance claim to one link route and its receipts to another, and the routes
differ by exactly what is on disk.

**M-9 — there is no LLVM linker plugin in the windows-gnu toolchain, and the only plugin-capable
linker in it costs 211 MB.**

Linker-plugin LTO — the mechanism that would let a *linker* perform LTO across engine and mod objects
— requires, in the rustc book's own words, that *"a linker with the LLVM plugin must be used (e.g.
LLD)"*, and for any other linker *"the LLVM linker plugin has to be specified explicitly … the path
to the plugin is passed as an argument"*, the example naming `LLVMgold.so`
([rustc book](https://doc.rust-lang.org/rustc/linker-plugin-lto.html)). In
`C:\Users\flint\.rustup\toolchains\stable-x86_64-pc-windows-gnu`:

| item | size | note |
|---|---|---|
| `*gold*` anywhere in the toolchain | — | **absent** (the only matches are rustdoc pages for `GOLDEN_RATIO`) |
| `lib/rustlib/x86_64-pc-windows-gnu/bin/rust-lld.exe` | **211 187 378 B** | the only plugin-capable linker present |
| `…/bin/gcc-ld/{ld.lld, ld64.lld, lld-link, wasm-ld}.exe` | 857 088 B **each, all four byte-identical** (same MD5) | shims: their strings include `rust-lld`, `-flavor`, `error running rust-lld child process` — they drive `rust-lld`, they are not linkers |
| `…/bin/self-contained/` | 6.0 MB (M-8) | `x86_64-w64-mingw32-gcc.exe` 2 840 064 B, `ld.exe` 1 910 784 B, `dlltool.exe` 1 328 640 B, `libwinpthread-1.dll`, `GCC-WARNING.txt` — **no plugin** |

**Consequence for the design space.** M-5/M-6/M-7 measured a **plain** link with the bundled mingw
driver: no LTO happens in that step, in either direction. Linker-plugin LTO is a *different* route
that would add the 211 MB linker to the shipped set — ~4× the entire 48 MB alternative M-8 measured —
and ship whole-engine LLVM bitcode to every player. The same rustc page additionally records that
compiling **proc-macros** under `-Clinker-plugin-lto` errors on Windows-like targets
(*"not supported together with `-C prefer-dynamic` when targeting Windows-like targets"*), and this
workspace has two proc-macro crates (`crates/aether`, `crates/boyko_macros`) — unverified for
windows-**gnu**, and it is on the list below rather than asserted.

### 13.3 Rev 3 — the derive-shaped static under fat LTO, and route 3b's two link lines (2026-09-16)

Same machine. Taken during the design space's rev-4 pass, because pass 3 of its critique could not
establish "how rustc emits a cross-crate-inlined function-local `static` under fat LTO" and marked
the hazard PLAUSIBLE. Each leg is under two seconds. Sources in `D:/tmp/modprobe/`; tools from the
installed `llvm-tools` component (`lib/rustlib/x86_64-pc-windows-gnu/bin/llvm-nm.exe`,
`llvm-readobj.exe`).

**M-10 — the rig.** `eng.rs` (rlib) is the derive's exact shape:

```rust
pub static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
#[inline(never)] pub fn register_new() -> usize { NEXT_ID.fetch_add(1, Ordering::Relaxed) }
#[inline] pub fn component_id() -> usize {
    static ID: OnceLock<usize> = OnceLock::new();
    *ID.get_or_init(register_new)
}
```

`modx.rs` is a mod object (`--crate-type=lib --emit=obj -O --extern eng=libeng.rlib`) whose one
`#[unsafe(no_mangle)] extern "C" fn mod_entry()` returns `eng::component_id()`; `app_host.rs` is a
bin that calls `component_id()` twice, then `mod_entry()`, then prints `NEXT_ID`. `rustc 1.98.1`,
`x86_64-pc-windows-gnu`, `-C opt-level=3` throughout.

| leg | what | result |
|---|---|---|
| 1 | `llvm-nm --demangle libeng.rlib` | `T eng::register_new` · `B eng::NEXT_ID` · **`D eng::component_id::ID`** · `D __imp__RNvNvCs…3eng12component_id2ID` — the function-local cell is a **global** data symbol of the defining crate, with windows-gnu's import-cell alias: **one cell per image**, referenced by name from every crate that instantiates the `#[inline]` body |
| 2 | bin at `-C lto=fat -C codegen-units=1` | `t eng::register_new` · `b eng::NEXT_ID` · **`d eng::component_id::ID`** — all three **local**, although `ID` is reachable from `main` |
| 2b | leg 2 + `-C link-dead-code` (stable) | unchanged: `t` / `b` / `d` |
| 2c | leg 2 on `rustc 1.100.0-nightly (8925ea358 2026-08-20)` + `-Z export-executable-symbols` | unchanged: `t` / `b` / `d`; the exe gains **1** PE export entry (the flag exports *"only `#[no_mangle]`"* symbols by its own page, [unstable book](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/export-executable-symbols.html)) |
| 3 | bin with one `#[unsafe(no_mangle)] pub extern "C" fn boyko_probe_symbol()`, `lto=fat` | `T boyko_probe_symbol` — **global through fat LTO** — and `llvm-readobj --coff-exports` lists **nothing** (control: the plain bin also lists nothing; leg 2c's exe lists one). A `#[no_mangle]` item in a windows-gnu executable is a global symbol, **not a PE export** |
| 4 | `llvm-nm -u modx.o` | `_RNv…3eng12register_new` · **`__imp__RNvNv…3eng12component_id2ID`** · `<std::sys::sync::once::futex::Once>::call` · `core::option::unwrap_failed` · `core::panicking::panic_cannot_unwind` · `rust_eh_personality` — the mod's instantiated `component_id()` references the engine's cell **and `std`'s internals** by name |
| 5 | **3b, sub-case "rlibs not on the line"**: LTO'd host + `modx.o` | `ld` fails: **7 undefined references** — `__imp__…component_id2ID` ×2, `eng::register_new` ×2, `Once::call`, `unwrap_failed` ×2. **Loud** |
| 6a | **3b, sub-case "rlib re-added"**: + `libeng.rlib` on the line | the engine references now resolve **from the rlib** (a second copy of `NEXT_ID` and `ID` enters the image); the `std` references remain undefined. Fails |
| 6b | 6a + all **27** sysroot `std` rlibs in `--start-group … --end-group` | fails on **`__rustc::__rust_alloc` / `__rust_dealloc` / `__rust_no_alloc_shim_is_unstable_v2`** from the second `alloc` — the allocator shim rustc emits once per final artifact, internalised by the host's LTO. Satisfying it means a second allocator in the image. **Loud** |
| 7 | `rustc --print link-args` for the fat-LTO bin vs the non-LTO bin | **2** `.rlib` entries (a temp-directory `libstd-….rlib` copy and `libcompiler_builtins-….rlib`) vs **21** — the "20 std rlibs" of M-8 are the non-LTO line's |

**Reading.** Route 3b — mod objects compiled `-O` against the engine rlibs, plain-linked onto an engine
object set rustc has already fat-LTO'd — contradicts itself: compiling against the rlibs is what puts
the references to `ID`, `register_new` and `std`'s internals into the mod object (leg 4), and fat LTO
is what turns their definitions local (leg 2). A `#[no_mangle]` export set can pin *functions* (leg
3) but not a function-local static, and no stable or nightly flag changes the binding (2b, 2c). The
line an installer would build fails loudly (5); the line that would create the silent local+global
pair the critique feared fails loudly earlier, on the allocator shim (6b). None of this depends on
workspace size; the workspace-scale `MODDING-DESIGN-SPACE.md` M-C4 stays queued so the gate is run
rather than argued, with an expected result.

**M-11 — `-Z dylib-lto` on the M-2 rig.** M-3's first error names its own escape (*"lto cannot be used
for `dylib` crate type without `-Zdylib-lto`"*); the flag exists on nightly and is documented as
*"enables using LTO for the `dylib` crate type … currently only used for compiling `rustc` itself"*
([unstable book](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/dylib-lto.html)).
On `nightly-x86_64-pc-windows-gnu` (`1.100.0-nightly (8925ea358 2026-08-20)`):

```
rustc +nightly --crate-type=dylib -C prefer-dynamic -C opt-level=3 -C lto=fat -Z dylib-lto eng.rs -o eng.dll   → builds (50 547 B)
rustc +nightly -C prefer-dynamic -C opt-level=3 --extern eng=eng.dll -L . app.rs                                → links, runs: id #1 = 0 id #2 = 0  NEXT_ID addr = 0x7fff4a35a0a0
rustc +nightly -C prefer-dynamic -C opt-level=3 -C lto=fat -Z dylib-lto --extern eng=eng.dll -L . app.rs        → links, runs: same output, same address
rustc          --crate-type=dylib -C prefer-dynamic -C opt-level=3 -C lto=fat eng.rs                           → error: lto cannot be used for `dylib` crate type without `-Zdylib-lto`   (stable control)
```

M-3's second error (*"cannot prefer dynamic linking when performing LTO"*) does not fire on the LTO'd
bin under the flag, and the static is shared (same address — M-2's check; the std DLL directory
`lib/rustlib/x86_64-pc-windows-gnu/lib` had to be on `PATH` to run, exactly as M-2 found).
**Feasibility only**: the rig is two functions, so whether intra-dylib fat LTO recovers the
workspace's 0.793 is `MODDING-DESIGN-SPACE.md` M-F1 leg (d). The flag is nightly and its page says
what it is currently for.

---

## 14. What could not be established

- **Whether `TypeId` is identical between separately-*configured* builds of the same engine crate.**
  In M-1/M-2 both sides linked the *same* compiled artifact, so equality was true by construction;
  differing `-C metadata` / feature sets were not tested. This is the remaining correctness question
  for the shared-dylib route and is cheap to measure.
- **Whether this workspace even builds as `crate-type = ["dylib"]`.** Not attempted (a full workspace
  rebuild). `boyko_ecs`/`boyko_render` contain `const _: () = assert!` layout pins and const-generic
  `SpirvBlob<N>` statics; no reason to think they break, but unverified.
- **Whether a `bin` compiled with `lto = "fat"` links a `dylib` dependency at all.** The rustc check
  keys on `prefer_dynamic` and on the *emitted* crate type; no authoritative statement for
  "LTO'd executable + dylib dep" was found. A 10-minute experiment settles it.
- **Link-only (not clean-build) time for this engine under fat LTO.** No number exists anywhere.
  ⚠ Note that **the mechanics** of link-only are now established (M-5): it is the *duration* that is
  unmeasured, not the possibility. ⚠⚠ And rev 3 re-poses the question: the installer's link under the
  route option C ships runs **no LTO at all**, so the number that decides the install step is the
  wall-clock of a *plain* link over the engine's post-LTO object set, not a fat-LTO link
  (`MODDING-DESIGN-SPACE.md` M-C1).
- ~~**Whether an engine object set that rustc has ALREADY fat-LTO'd can be relinked against mod
  objects.**~~ — **closed at toy scale by M-10 (§13.3), and the answer is no, loudly.** Both ways it
  could fail were measured: the symbols a mod references (the per-type cell, the dispenser, `std`'s
  internals) are internalised at the engine's own LTO step, and re-adding the rlibs brings a second
  copy of every engine static in and then fails on the allocator shim. Still open: the
  workspace-scale run (`MODDING-DESIGN-SPACE.md` M-C4), queued to confirm a mechanism rather than to
  open a question.
- **Whether nightly `-Z dylib-lto` recovers the fat-LTO number for the modding configuration on the
  workspace.** Feasible on the M-2 rig (M-11, §13.3): the dylib LTO's, the bin links with and without
  its own LTO, the statics stay shared. The price on `query_ref_iter` is unmeasured
  (`MODDING-DESIGN-SPACE.md` M-F1 leg (d)).
- **Whether `-Clinker-plugin-lto` errors for this workspace's two proc-macro crates on windows-gnu.**
  The rustc book's statement is for Windows-like targets together with `-C prefer-dynamic`; unverified
  here (§13.2 said "on the list below" and, until this revision, the item was missing from the list).
  Moot for a vendor build with an explicit `--target`: cargo then withholds `RUSTFLAGS` from host
  artifacts — *"Things being built for the host, such as build scripts or proc macros, will not
  receive the args"* ([cargo config](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags)).
- **Whether an out-of-process mod (design space §5.1, option F) can be given the engine's columns at
  all** — `ComponentPool` storage is `VmReservation` over `VirtualAlloc`, not a file mapping — and what
  a per-system cross-process hand-off costs on Windows 11. Not measured; not yet queued.
- **Whether an UNREACHED `static` in a linked dependency rlib is carried into the image.** The
  decidability measurement everything here leans on (`D:/wt/reflect/docs/REFLECTION-ANALYSIS.md:1477-1494`)
  is of a plain *function*. M-1 measured a duplicated `static` that the mod **did** reach. The
  unreached case — which is what a load-time address cross-check would have to discriminate on — is
  measured nowhere. `MODDING-DESIGN-SPACE.md` M-A4a.
- **Whether declaring a lib as `crate-type = ["rlib", "dylib"]` silently drops LTO for the rlib
  leg in cargo 2026.** The reported behaviour is that it does, for *every* crate type
  ([rust#51009](https://github.com/rust-lang/rust/issues/51009), open; cargo has special-cased
  rlib+cdylib and staticlib, [cargo#8254](https://github.com/rust-lang/cargo/pull/8254), which does
  not cover rlib+dylib). This is the mechanism the design space's option A′ needs, and the failure
  would land on the **non-modding** build with no diagnostic. `MODDING-DESIGN-SPACE.md` M-F1 leg (c).
- **Whether the precompiled sysroot `std` participates in linker-plugin LTO (route 2).** The rustc
  book's page for the flag ([linker-plugin-lto](https://doc.rust-lang.org/rustc/linker-plugin-lto.html),
  re-read 2026-09-17) states the linker requirement, LLVM-version matching and the proc-macro /
  `prefer-dynamic` error, and contains no occurrence of "std", "sysroot" or "precompiled". M-10
  leg 7 shows a temp-directory `libstd` rlib on today's fat-LTO link line. If `std` reaches the
  plugin link as machine code, route 2's binary is less optimised into `std` than the shipped
  fat-LTO build. `MODDING-DESIGN-SPACE.md` M-C5, toy scale, the M-10 rig.
- **How far `NEXT_ID` sits from its eventual value at the config-phase mod-load slot.** Engine ids
  mint lazily on first touch (`…/component_registry/mod.rs:911-914`); the slot precedes `finish()`,
  which builds the schedules and so resolves every queried type; nothing has ever printed the
  counter at those three points. Any loader bound has to absorb that drift, and the physics band it
  would spend is census-sized against 142 (`boyko_physics/src/scratch_ids.rs:671-684`).
  `MODDING-DESIGN-SPACE.md` M-K3.
- **Any runtime unload behaviour on Windows** (`FreeLibrary` with live fn-ptrs) — not measured.
- **Real workload cost of the dylib boundary.** M-4 prices one indirect call, not a frame.
- **Any shipping game that relinks its binary with mod objects at install** — searched, nothing
  found. (M-5/M-6/M-7 establish that the technique works on this toolchain; they do not supply a
  precedent for shipping it.)
- **The cost to the ENGINE of carrying a linker-collected plugin registry.** M-6's +0.12 % is a
  hello-world figure. Whether the sentinel section, and any `--gc-sections` change, cost the real
  workspace anything on `query_ref_iter` / `swap_remove` / dispatch or on binary size is unmeasured
  (`MODDING-DESIGN-SPACE.md` M-C0b).
- ~~**Windows rustup toolchain on-disk size**~~ — **closed by M-8**: 2772 MB for
  `stable-x86_64-pc-windows-gnu` on the owner's box (`bin` 529 MB, `lib/rustlib` 1373 MB, docs
  865 MB); the shippable *link-only* subset is ≈48 MB. Still open: whether rustc exposes a ThinLTO
  cache across builds the way `lld --thinlto-cache-dir` does.
- **Whether GDExtension can be compiled out of Godot.**
- **Whether the 65535-export ceiling is reachable with this crate graph on windows-gnu.**
- **Whether a worker panic still hangs the windowed host** (needs the GPU leg).

---

## 15. Open questions

Ordered by how much of the design each one decides.

1. **Trust model.** Native mods are full-trust; no native path surveyed has a sandbox, and
   Fractureiser is the field's warning. Sandbox versus signing/allow-list is a **values** call that
   decides the whole technique set, and it is the owner's to make.
2. **Which boundary?** Native-Rust `dylib` (pay LTO, ship `std` DLLs, exact-build lock) versus
   `cdylib` + `#[repr(C)]` host table (keep LTO and the static build; mods get their own statics /
   `TypeId` / allocator universe) versus install-time relink versus source mods. **These are not
   reconcilable by a middle option** — but they *are* reconcilable by **two shipping
   configurations**, which is what `MODDING-DESIGN-SPACE.md` §1.5 (option A′) proposes: the
   non-modding build keeps today's static + fat-LTO shape, and only the modding build pays the
   dylib price. M-2's shared-static result is what makes that variant correct by construction.
   *(Design space rev 4: the relink's 3b form is red at toy scale — M-10 — so C's live forms are
   route 2, linker-plugin LTO at install, or 3a, which forfeits fat LTO as the dylib does; and M-11
   shows the dylib's LTO forfeit is stable's, not Rust's.)*
3. **Must mods add new component *types*** (needs cross-binary type identity) **or only data
   prototypes of engine types** (Factorio's model, which sidesteps the whole identity problem)?
4. **Does a mod get to mint `ComponentId`s, or only receive them?** Receiving them (host mints, mod
   caches into its own slots) is the only variant compatible with the current `OnceLock` derive.
   ⚠ **It does NOT remove the duplicated-statics problem** — that phrasing was too strong and is
   corrected in `MODDING-DESIGN-SPACE.md` §0.3. `ComponentId` is only one of the counters; the
   **query** type-id path mints from the mod's own `QUERY_NEXT_ID` inside generic code the mod
   instantiates itself, and the resulting id indexes the host's type-erased query-state cache — a
   silent type confusion no handshake can prevent. Receiving ids is necessary and not sufficient.
5. **Is unload in scope at all?** If yes, the fn-ptr tables and leaked `&'static str` names become
   live-pointer hazards and §12.12's refusal applies directly. If no, the design simplifies to
   load-only, matching Burst's `LoadAdditionalLibrary` and abi_stable.
6. **Is `modding` a cargo feature or a separate crate?** A feature is subject to unification by any
   third-party dependency; a separate crate plus a distinct binary target is immune by construction —
   at the cost of two shipping configurations.
   **Answered in `MODDING-DESIGN-SPACE.md` §7.10 (rev 4)**: a separate crate — and nothing
   configuration-scoped may live behind any switch on a kernel crate (no attribute, `cfg`, feature or
   per-configuration constant); scoping is by dependency edge only, gated by a source census over
   every kernel crate a non-modding game links (`boyko_ecs`, `boyko_utils`, `boyko_threadpool`,
   `boyko_log`, `boyko_diag`, `boyko_macros` — 0 hits in each at `d552be05`).
   **Correction (design space rev 5, pass 4 W1):** rev 4 also gated on `llvm-readobj --coff-exports`
   over the non-modding executable and cited M-10 leg 3 for it. Leg 3's receipt says the opposite
   — on stable the exe's export directory is empty *with* a `#[no_mangle]` item present, and only
   nightly's `-Z export-executable-symbols` (leg 2c) adds an entry — so that leg could not fail on
   the configuration it gated and is withdrawn for the executable. It stays informative for mod DLLs
   and any `cdylib`.
7. **What is the mod component budget**, and is `MAX_COMPONENTS` raised for everyone or partitioned
   (a reserved high band, as `boyko_physics/src/scratch_ids.rs` already does for scratch ids)?
   Raising it costs `[Column; MAX_COMPONENTS]` per archetype and breaks the 192 B `Access` assert.
   **Answered in `MODDING-DESIGN-SPACE.md` §7.3 (rev 4)**: neither — a loader-side quota over the
   fixed 512, minted through the existing `try_register_dynamic` CAS; the non-modding build reserves
   nothing, and a per-configuration constant is withdrawn because its only in-tree mechanism is a
   cargo feature on `boyko_ecs`. **Partly re-opened in rev 5 (pass 4, C2):** the quota's *bound*
   compared against `NEXT_ID` at load time, but engine ids mint lazily after the load slot, so the
   comparison bounded nothing and the counter could climb into the physics scratch band — whose
   128-slot margin is census-sized (`scratch_ids.rs:671-684`) — ending in a collision panic rather
   than the budgeted assert. "No constant, no reservation" stands; the bound's mechanism (an eager
   mint pass, a census-held ceiling, or a downward mod band with the collision check as backstop) is
   the architect's, and M-K3 measures the drift.
8. **Does a mod allocate at all?** If every mod allocation goes through a host arena call, the
   `#[global_allocator]` hazard disappears; if not, the engine's future in-house allocator makes
   cross-boundary `Box`/`Vec` UB.
   **Answered in `MODDING-DESIGN-SPACE.md` §1 (allocator row) and §7.9(c) (rev 6):** the rule is
   *the allocating image frees, through its own code, kept mapped for the allocation's lifetime*;
   the one path every mod would have crossed it on — `ScheduleBuilder::add_system`'s
   `Box<dyn System>` (`schedule_builder.rs:183`), owned by the host's `SystemBox` and freed by host
   drop glue — is closed by a host-side registration seam (orchestrator ruling R2), and the
   mod-owned query state handed over with its own `drop_fn` is the sanctioned third form. The
   hazard's timing is as this item says: at `d552be05` no shipped image installs a
   `#[global_allocator]` (grep: benches, test modules, and one `#[cfg(feature = "profiling-alloc")]`
   shim at `boyko_app/src/profiling/alloc_shim.rs:145`), so both images reach one process heap
   today; the adopted per-type migration (`docs/memory/ALLOCATOR-DESIGN-SPACE.md:59`, `:72`) is what
   turns the crossing into corruption.
9. **What is the panic contract?** `extern "C"` + catch-inside-the-mod + status code is the only
   combination that neither aborts nor invokes unspecified behaviour across two `std` copies.
10. **Asset loaders**: const table + a `cfg`'d runtime fallback consulted only on miss, or a design
    change? The miss path is cold, so a gated second scan may cost nothing measurable — one
    measurement answers it.
11. **Save-file policy for missing mods**: flecs-style silent drop, Unity-style hard error, or
    quarantine-and-preserve.
12. **What measurement decides mod-side monomorphisation versus an engine-side dynamic query?** No
    published number exists (§6); a local A/B of `Query<&C>` against an id-driven column walk over the
    same data is the only evidence that will exist.
13. **Is an engine rebuild per mod set acceptable** (source-level, Godot-module style)? If yes, most
    of the ABI problem disappears and the cost moves to install time.

---

## 16. Sources

External claims above are cited inline. The consolidated list, grouped:

**Rust language and toolchain** — [Reference: Linkage](https://doc.rust-lang.org/reference/linkage.html) ·
[Reference: Type layout](https://doc.rust-lang.org/reference/type-layout.html) ·
[Reference: Functions (extern/panic)](https://doc.rust-lang.org/reference/items/functions.html) ·
[`TypeId`](https://doc.rust-lang.org/std/any/struct.TypeId.html) ·
[`type_name`](https://doc.rust-lang.org/std/any/fn.type_name.html) ·
[`ptr::eq`](https://doc.rust-lang.org/std/ptr/fn.eq.html) ·
[`catch_unwind`](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html) ·
[`LocalKey`](https://doc.rust-lang.org/std/thread/struct.LocalKey.html) ·
[`std::alloc`](https://doc.rust-lang.org/std/alloc/index.html) ·
[`std::alloc::System`](https://doc.rust-lang.org/std/alloc/struct.System.html) ·
[`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html) ·
[E0514](https://doc.rust-lang.org/error_codes/E0514.html) ·
[codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html) ·
[PGO](https://doc.rust-lang.org/rustc/profile-guided-optimization.html) ·
[`dylib-lto`](https://doc.rust-lang.org/stable/unstable-book/compiler-flags/dylib-lto.html) ·
[`randomize-layout`](https://doc.rust-lang.org/unstable-book/compiler-flags/randomize-layout.html) ·
[`control-flow-guard`](https://doc.rust-lang.org/stable/unstable-book/compiler-flags/control-flow-guard.html) ·
[Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) ·
[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) ·
[cargo `Metadata`](https://doc.rust-lang.org/stable/nightly-rustc/cargo/core/compiler/struct.Metadata.html) ·
[rustc-dev-guide: libs and metadata](https://rustc-dev-guide.rust-lang.org/backend/libs-and-metadata.html) ·
1.81 release notes · 1.97 release notes · v0-mangling-on-nightly ·
[rust#22991](https://github.com/rust-lang/rust/issues/22991) ·
[rust#31854](https://github.com/rust-lang/rust/issues/31854) ·
[rust#59629](https://github.com/rust-lang/rust/issues/59629) ·
[rust#61553](https://github.com/rust-lang/rust/issues/61553) ·
[rust#84161](https://github.com/rust-lang/rust/issues/84161) ·
[rust#100781](https://github.com/rust-lang/rust/issues/100781) ·
[rust#116505](https://github.com/rust-lang/rust/pull/116505) ·
[rust#116558](https://github.com/rust-lang/rust/issues/116558) ·
[rust#117047](https://github.com/rust-lang/rust/issues/117047) ·
[rust#129080](https://github.com/rust-lang/rust/issues/129080) ·
[rust#134767 (`sdylib`)](https://github.com/rust-lang/rust/pull/134767) ·
[rust#160448](https://github.com/rust-lang/rust/pull/160448) ·
[cargo#4611](https://github.com/rust-lang/cargo/issues/4611) ·
[cargo#8716](https://github.com/rust-lang/cargo/issues/8716) ·
[cargo#14830](https://github.com/rust-lang/cargo/pull/14830) ·
[rfcs#2130](https://github.com/rust-lang/rfcs/pull/2130) ·
[RFC 1510](https://rust-lang.github.io/rfcs/1510-cdylib.html) ·
[RFC 2945](https://rust-lang.github.io/rfcs/2945-c-unwind-abi.html) ·
[RFC 3435 draft](https://github.com/m-ou-se/rfcs/blob/export/text/0000-export.md) ·
[RFC 3470 (crABI)](https://github.com/rust-lang/rfcs/pull/3470) ·
[UCG#526](https://github.com/rust-lang/unsafe-code-guidelines/issues/526) ·
[safe-linking goal](https://goals.rust-lang.org/2025h1/safe-linking.html) ·
[production-ready Cranelift goal](https://goals.rust-lang.org/2025h2/production-ready-cranelift.html) ·
[`def_id.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_span/src/def_id.rs) ·
[`v0.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_symbol_mangling/src/v0.rs) ·
[`ty/util.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_middle/src/ty/util.rs) ·
[`back/lto.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_codegen_ssa/src/back/lto.rs) ·
[`session/config.rs`](https://raw.githubusercontent.com/rust-lang/rust/master/compiler/rustc_session/src/config.rs)

**Bevy** — [#1110](https://github.com/bevyengine/bevy/issues/1110) ·
[#2016](https://github.com/bevyengine/bevy/pull/2016) ·
[#8390](https://github.com/bevyengine/bevy/discussions/8390) ·
[#9774](https://github.com/bevyengine/bevy/pull/9774) ·
[#11969](https://github.com/bevyengine/bevy/issues/11969) ·
[#13080](https://github.com/bevyengine/bevy/pull/13080) ·
[#14534](https://github.com/bevyengine/bevy/pull/14534) ·
[#14930](https://github.com/bevyengine/bevy/issues/14930) ·
[#15030](https://github.com/bevyengine/bevy/pull/15030) ·
[#16396](https://github.com/bevyengine/bevy/pull/16396) ·
[#17564](https://github.com/bevyengine/bevy/issues/17564) ·
[#18251](https://github.com/bevyengine/bevy/issues/18251) ·
[#19309](https://github.com/bevyengine/bevy/pull/19309) ·
[#25797](https://github.com/bevyengine/bevy/issues/25797) ·
[migration 0.14→0.15](https://bevy.org/learn/migration-guides/0-14-to-0-15/) ·
[`bevy_dylib`](https://docs.rs/bevy_dylib/latest/bevy_dylib/) ·
[`bevy_dynamic_plugin` 0.14.2](https://docs.rs/bevy_dynamic_plugin/0.14.2/bevy_dynamic_plugin/) ·
[`ComponentDescriptor` 0.19.1](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/component/struct.ComponentDescriptor.html) ·
[`TypePath`](https://docs.rs/bevy_reflect/latest/bevy_reflect/trait.TypePath.html) ·
`crates/bevy_ecs/src/{component/info.rs, component/register.rs, system/function_system.rs, system/system.rs, system/schedule_system.rs, schedule/executor/multi_threaded.rs}`,
`crates/bevy_asset/src/server/loaders.rs`, `examples/ecs/dynamic.rs`, `.cargo/config_fast_builds.toml` (main)

**Other engines** — [Quake 2 `game.h` / `sv_game.c`](https://raw.githubusercontent.com/id-Software/Quake-2/master/game/game.h) ·
[Quake 3 QVM](https://fabiensanglard.net/quake3/qvm.php) ·
[Half-Life `eiface.h`](https://raw.githubusercontent.com/ValveSoftware/halflife/master/engine/eiface.h) ·
[Source `interface.h`](https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/master/src/public/tier1/interface.h) ·
[Metamod-P](https://github.com/Bots-United/metamod-p) ·
[UBT target reference](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-build-tool-target-reference) ·
[UE target files 4.26](https://docs.unrealengine.com/4.26/en-US/ProductionPipelines/BuildTools/UnrealBuildTool/TargetFiles) ·
[UE binary versioning](https://dev.epicgames.com/documentation/en-us/unreal-engine/how-to-version-binaries-in-unreal-engine) ·
[`MODULE_API`](https://dev.epicgames.com/documentation/unreal-engine/module-api-specifiers-in-unreal-engine) ·
[UE Live Coding](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-live-coding-to-recompile-unreal-engine-applications-at-runtime) ·
[Live++ docs](https://liveplusplus.tech/docs/documentation.html) ·
[UE Hot Reload community wiki](https://unrealcommunity.wiki/live-compiling-in-unreal-projects-tp14jcgs) ·
[Satisfactory case study](https://buckminsterfullerene02.github.io/dev-guide/CaseStudies/Satisfactory.html) ·
[Satisfactory modding docs](https://docs.ficsit.app/satisfactory-modding/latest/Development/Cpp/index.html) ·
[UE module registration analysis](https://en.imzlp.com/posts/24007/) ·
[godot-cpp / GDExtension](https://docs.godotengine.org/en/stable/tutorials/scripting/cpp/about_godot_cpp.html) ·
[godot#76406](https://github.com/godotengine/godot/pull/76406) ·
[godot#91282](https://github.com/godotengine/godot/issues/91282) ·
[godot-rust FFI benchmarks](https://godot-rust.github.io/dev/ffi-optimizations-benchmarking/) ·
[godot-rust compatibility](https://godot-rust.github.io/book/toolchain/compatibility.html) ·
[Unity scripting restrictions](https://docs.unity3d.com/Manual/scripting-restrictions.html) ·
[IL2CPP](http://docs.unity3d.com/Manual/scripting-backends-il2cpp.html) ·
[Il2CppInterop](https://github.com/BepInEx/Il2CppInterop) ·
[Harmony](https://harmony.pardeike.net/articles/intro.html) ·
[Harmony edge cases](https://harmony.pardeike.net/articles/patching-edgecases.html) ·
[HarmonyX valid targets](https://github.com/BepInEx/HarmonyX/wiki/Valid-patch-targets) ·
[Burst modding support](https://docs.unity3d.com/Packages/com.unity.burst@1.8/manual/modding-support.html) ·
[`TypeIndex`](https://docs.unity3d.com/Packages/com.unity.entities@1.2/api/Unity.Entities.TypeIndex.html) ·
[`TypeHash.cs`](https://github.com/needle-mirror/com.unity.entities/blob/master/Unity.Entities/Types/TypeHash.cs) ·
[`TypeManager.cs`](https://github.com/needle-mirror/com.unity.entities/blob/master/Unity.Entities/Types/TypeManager.cs) ·
[Entities 0.16 changelog](https://docs.unity3d.com/Packages/com.unity.entities@0.16/changelog/CHANGELOG.html) ·
[SKSE](https://skse.silverlock.org/) ·
[CommonLibSSE query/load](https://github.com/Ryan-rsm-McKenzie/CommonLibSSE/wiki/Query-and-Load) ·
[CommonLibSSE-NG](https://github.com/CharmedBaryon/CommonLibSSE-NG/wiki/CMake-Integration) ·
[CompaSSE](https://github.com/drLemis/CompaSSE) ·
[Fabric Mixin](https://wiki.fabricmc.net/tutorial:mixin_introduction) ·
[Mixin environment](https://github.com/SpongePowered/Mixin/wiki/Introduction-to-Mixins---The-Mixin-Environment) ·
[Factorio data lifecycle](https://lua-api.factorio.com/latest/auxiliary/data-lifecycle.html) ·
[Factorio mod structure](https://lua-api.factorio.com/latest/auxiliary/mod-structure.html) ·
[Factorio forum, boskid](https://forums.factorio.com/viewtopic.php?t=96889) ·
[Factorio perf wiki](https://wiki.factorio.com/Tutorial:Diagnosing_performance_issues) ·
[Luau performance](https://luau.org/performance) · [Luau sandbox](https://luau.org/sandbox) ·
[Roblox native codegen](https://create.roblox.com/docs/luau/native-code-gen) ·
[Machinery: little machines](https://ruby0x1.github.io/machinery_blog_archive/post/little-machines-working-together-part-1/index.html) ·
[Machinery: DLL hot reloading](https://ruby0x1.github.io/machinery_blog_archive/post/dll-hot-reloading-in-theory-and-practice/index.html) ·
[Machinery: API versioning](https://ruby0x1.github.io/machinery_blog_archive/post/api-versioning/index.html) ·
[Machinery shutdown](https://gameworldobserver.com/2022/08/01/our-machinery-terminates-machinery-game-engine-changes-eula) ·
[flecs `flecs_cpp.c`](https://github.com/SanderMertens/flecs/blob/master/src/addons/flecs_cpp.c) ·
[flecs `component.hpp`](https://github.com/SanderMertens/flecs/blob/master/include/flecs/addons/cpp/component.hpp) ·
[flecs `module.h`](https://github.com/SanderMertens/flecs/blob/master/include/flecs/addons/module.h) ·
[flecs `EntitiesComponents.md`](https://github.com/SanderMertens/flecs/blob/master/docs/EntitiesComponents.md) ·
[flecs `ComponentTraits.md`](https://github.com/SanderMertens/flecs/blob/master/docs/ComponentTraits.md) ·
[flecs `json.h`](https://github.com/SanderMertens/flecs/blob/master/include/flecs/addons/json.h) ·
[flecs `flecs.h`](https://raw.githubusercontent.com/SanderMertens/flecs/master/include/flecs.h) ·
[flecs #685](https://github.com/SanderMertens/flecs/issues/685) ·
[flecs #1034](https://github.com/SanderMertens/flecs/issues/1034) ·
[EnTT core.md](https://github.com/skypjack/entt/blob/main/docs/md/core.md) ·
[EnTT entity.md](https://github.com/skypjack/entt/blob/main/docs/md/entity.md) ·
[EnTT sparse_set.hpp](https://github.com/skypjack/entt/blob/main/src/entt/entity/sparse_set.hpp) ·
[EnTT: push across boundaries](https://github.com/skypjack/entt/wiki/Push-EnTT-across-boundaries) ·
[UE Mass `TTypeBitSetBuilder`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/MassEntity/TTypeBitSetBuilder) ·
[MassSample](https://github.com/Megafunk/MassSample) ·
[Fyrox hot reloading](https://fyrox-book.github.io/beginning/hot_reloading.html) ·
[Fyrox `plugin/mod.rs`](https://raw.githubusercontent.com/FyroxEngine/Fyrox/master/fyrox-impl/src/plugin/mod.rs) ·
[Fyrox `plugin/dylib.rs`](https://raw.githubusercontent.com/FyroxEngine/Fyrox/master/fyrox-impl/src/plugin/dylib.rs) ·
[MSFS 2024 WebAssembly](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/WASM/WebAssembly.htm)

**Libraries and runtimes** — [abi_stable](https://docs.rs/abi_stable/latest/abi_stable/) ·
[stabby README](https://raw.githubusercontent.com/ZettaScaleLabs/stabby/main/stabby/README.md) ·
[hot-lib-reloader](https://github.com/rksm/hot-lib-reloader-rs) ·
[subsecond](https://docs.rs/subsecond/latest/subsecond/) ·
[linkme](https://docs.rs/linkme/latest/linkme/) ·
[libloading](https://docs.rs/libloading/latest/libloading/struct.Library.html) ·
[dlopen2](https://docs.rs/dlopen2/latest/dlopen2/) ·
[Wasmtime fast execution](https://docs.wasmtime.dev/examples-fast-execution.html) ·
[Wasmtime proposal status](https://docs.wasmtime.dev/stability-wasm-proposals.html) ·
[`MemoryCreator`](https://docs.wasmtime.dev/api/wasmtime/trait.MemoryCreator.html) ·
[wasmtime#14280](https://github.com/bytecodealliance/wasmtime/pull/14280) ·
[Wasmtime/Cranelift 2023](https://bytecodealliance.org/articles/wasmtime-and-cranelift-in-2023) ·
[Wasmtime 10 performance](https://bytecodealliance.org/articles/wasmtime-10-performance) ·
[Cranelift README](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md) ·
[`rustc_codegen_cranelift`](https://github.com/rust-lang/rustc_codegen_cranelift) ·
[flexible vectors](https://github.com/WebAssembly/flexible-vectors/blob/main/proposals/flexible-vectors/Overview.md) ·
[Zed extensions](https://zed.dev/blog/zed-decoded-extensions) ·
[Veloren plugins](https://book.veloren.net/contributors/modders/writing-a-plugin.html) ·
[eBPF design Q&A](https://www.kernel.org/doc/html/latest/bpf/bpf_design_QA.html)

**Measurements and analyses** — [Fedora: no-semantic-interposition](https://fedoraproject.org/wiki/Changes/PythonNoSemanticInterpositionSpeedup) ·
[Fedora: static libpython](https://fedoraproject.org/wiki/Changes/PythonStaticSpeedup) ·
[Lemire: cost of a function call](https://lemire.me/blog/2026/02/08/the-cost-of-a-function-call/) ·
[johnnysswlab: virtual functions](https://johnnysswlab.com/the-true-price-of-virtual-functions-in-c/) ·
[mold README (link times)](https://github.com/rui314/mold) ·
[ThinLTO blog](https://blog.llvm.org/2016/06/thinlto-scalable-and-incremental-lto.html) ·
[LLVM passes](https://llvm.org/docs/Passes.html) ·
[LLVM developer policy](https://llvm.org/docs/DeveloperPolicy.html) ·
[nullderef: plugins with abi_stable](https://nullderef.com/blog/plugin-abi-stable/) ·
[nullderef: the end](https://nullderef.com/blog/plugin-end/) ·
[Lattimore: Rust dylib rabbit holes](https://davidlattimore.github.io/posts/2024/08/27/rust-dylib-rabbit-holes.html) ·
[Kra.hn: dylibs for incremental builds](https://robert.kra.hn/posts/2022-09-09-speeding-up-incremental-rust-compilation-with-dylibs/) ·
[matklad: `#[inline]` in Rust](https://matklad.github.io/2021/07/09/inline-in-rust.html) ·
[MaskRay: PLT](https://maskray.me/blog/2021-09-19-all-about-procedure-linkage-table) ·
[MaskRay: TLS](https://maskray.me/blog/2021-02-14-all-about-thread-local-storage) ·
[MS: `dllimport`](https://learn.microsoft.com/en-us/cpp/build/importing-function-calls-using-declspec-dllimport) ·
[MS: DLL best practices](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-best-practices) ·
[MS: TLS](https://learn.microsoft.com/en-us/cpp/parallel/thread-local-storage-tls) ·
[MS: VS 2022 redistribution](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution) ·
[MS: Smart App Control code signing](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control) ·
[Arch `rust` package](https://archlinux.org/packages/extra/x86_64/rust/) ·
[00f.net: Wasm runtimes 2026](https://00f.net/2026/06/23/webassembly-runtimes-2026/) ·
[Jangda et al., ATC '19](https://www.usenix.org/conference/atc19/presentation/jangda) ·
[Mozilla: RLBox in Firefox 95](https://blog.mozilla.org/attack-and-defense/2021/12/06/webassembly-and-back-again-fine-grained-sandboxing-in-firefox-95/) ·
[bugzilla 1829765](https://bugzilla.mozilla.org/show_bug.cgi?id=1829765) ·
[SpiderMonkey: memory64](https://spidermonkey.dev/blog/2025/01/15/is-memory64-actually-worth-using.html) ·
[NaCl paper](https://cliffle.com/pub/native-client-paper/) ·
[LFI](https://www.scs.stanford.edu/~zyedidia/docs/papers/lfi.pdf) ·
[Segue & ColorGuard](https://vahldiek.github.io/publication/narayan-2022/) ·
[bpftime / EIM, OSDI '25](https://www.usenix.org/conference/osdi25/presentation/zheng-yusheng) ·
[LWN on cg_clif](https://lwn.net/Articles/964735/) ·
[nnethercote perf book](https://nnethercote.github.io/perf-book/build-configuration.html) ·
[Fractureiser](https://github.com/trigram-mrp/fractureiser) ·
[blaz.is: we don't need a stable ABI](https://blaz.is/blog/post/we-dont-need-a-stable-abi/)

**Repository** (`D:/wt/joltab` @ `d552be05`, read-only) — cited inline as `path:line` in §12 and §2.2.
Neighbouring worktrees: `D:/wt/reflect` (branch `feat/reflection`) for `boyko_reflect` and its gates;
`D:/wt/aether` (branch `feat/aether-v2`). Design documents under `D:/claude/BoykoEngine/docs`.
