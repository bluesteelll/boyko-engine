# B3 probe build (i)–(v): answers, evidence, and what each decides

> Committed copy of the B3 stage-1 report. An evidence file named here is in this directory when it is
> a receipt (`probe-*.txt`, `textcheck-*.txt`, `rlibs.txt`, `leg7-A.txt`, `ug08.txt`, `ug09.txt`,
> `tools.txt`); logs, snapshots and the explore transcript (`ug08/*.log`, `leg7/*`, `explore/*`) stay in the
> B3 scratch directory of the session that took them.

Taken 2026-09-23 on the msvc gate host, worktree `D:/wt/lighttable`, branch `u/b3`. Build prefix on
every cargo command: `PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
CARGO_TARGET_DIR=D:/wt/_targets/vkval-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=5 TMP/TEMP=D:/wt/_targets/tmp`.
Every heavy step ran through `ug15` (`crates/boyko_symcensus/src/bin/ug15.rs`), invoked directly
after `cargo build -p boyko-symcensus --bin ug15`.

## Engine state the probes measured

No engine crate's source changed between the trunk `c1e9f1db` and any commit of this stage. The
stage's commits touch only `crates/boyko_symcensus/**`, the root `Cargo.toml` (member and profiles),
and the two root census tests. Each receipt's header names its HEAD:

| receipt | HEAD | note |
|---|---|---|
| `probe-i-ii.txt` | `6e9f2557` (commit A) | clean |
| `probe-iii.txt` | `6e9f2557` | 3 uncommitted paths, all in `crates/boyko_symcensus` (the `--pdb` fix, committed as `f574778f`) |
| `probe-v.txt` | `05cd7715` | on `u/b3-ctl-0` with the 2 planted files (`probe-v-branch.diff`) |
| `probe-iv.txt` | `c2756e82` | clean |
| `textcheck-*.txt`, `leg7/leg7-*-A.snap` | `05cd7715` | clean |
| `rlibs.txt` | `c2756e82` | clean |

## Tools (`ug15 tools`, every receipt header)

| tool | version | resolved from |
|---|---|---|
| rustc | 1.98.1 (48a229cea 2026-09-01), LLVM 22.1.8, host x86_64-pc-windows-msvc | `rustc -vV` |
| llvm-nm / llvm-objdump / llvm-readobj / llvm-size / llvm-ar | LLVM 22.1.8-rust-1.98.1-stable | rustup sysroot `…/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin` |
| llvm-symbolizer | LLVM 21.0.0git | MSVC Build Tools `…/MSVC/14.44.35207/bin/Hostx64/x64` (the only copy found) |
| link.exe | 14.44.35228.0 | `--print link-args` |

The instrument's absence reds were exercised by hand (`BOYKO_UG15_LLVM_BIN=Z:/nonexistent` → `RED [tool
absent]`; `BOYKO_UG15_SYMBOLIZER=Z:/none.exe` → `RED [tool absent]`; `CARGO_TARGET_DIR` unset → `RED
[target dir]`; `ug15 nm <msvc image>` → `RED [empty census] … no symbols`). They are recorded again as
controls T and E in T2.

## Link line (every release-family build)

`"/OPT:REF,ICF" "/DEBUG" "/PDBALTPATH:%_PDB%"`, plus the natvis files, on every subject (receipts'
`link args` lines). rustc passes `/OPT:REF,ICF` explicitly, so the `/DEBUG` default of `/OPT:NOREF,NOICF`
(MS Learn, `sources.txt` §4) does not apply. The final rustc line carries `-C strip=debuginfo` under
`release` on msvc (receipts' `final rustc` lines), which contradicts the plan's "disabled on msvc since
1.77.1" (03 §6, 00 §11); on msvc it does not remove `/DEBUG` from the link.

---

## (i) Anonymous-data forms — `probe-i-ii.txt`

**Built:** `release`, all five subjects (swap_remove, boyko_demo, clear, query_dsl, phase9_scheduler).

**Answers.**
- **Mangling:** v0 only. Defined raw names by prefix, swap_remove: `_R` 2720, `anon.` 8106, SEH (`$…`/`?…`)
  7169, other 362; no `_ZN`. `llvm-nm --demangle` prints v0 paths **without** crate disambiguators
  (`demangled sample` lines), so PC-2's `[hex]` stripping is a no-op on this host; it stays as a guard.
- **Reference forms in candidate bodies** (histogram over every located body, all subjects):
  `anon.<32 hex>.<n>` 267; defined mangled code 438; defined mangled data (`d` 53, `b` 27); `__imp_*` 89;
  content-named constants `__real@`/`__xmm@`/`__ymm@` 45; undefined C names `memcpy` 12, `memset` 13,
  `memmove` 1, `_tls_index` 6, `__chkstk` 2, `__floattidf` 4. **UNKNOWN forms: none.** No `.L*`,
  `__unnamed_*`, `alloc_*`, `str.*`, section-symbol or `switch.table.*` reference occurs in a candidate body.
- **Each `anon.*` is a local `r` symbol alone in its own `.rdata` section** (`llvm-readobj`, explore
  transcript). A panic `Location` is 24 bytes: an `IMAGE_REL_AMD64_ADDR64` to the path string at offset
  0, then `len: u64`, `line: u32`, `col: u32`; the body references it by `IMAGE_REL_AMD64_REL32`.
- **The swap_remove/10k body's `VmColumn::swap_remove` assert:** `EcsMaster::delete_entity` (H-2)
  references `anon.d942e55fefdd14610d579d30fde3626d.8` = `Location(crates\boyko_ecs\src\ecs\memory\vm_column.rs:281:9)`;
  line 281 col 9 is the `assert!(` inside `pub(crate) fn swap_remove` (checked by content on this tree).
  The body calls no `VmColumn…::swap_remove` symbol: the frame is inlined.
- Standard-library and registry `Location`s embed absolute machine paths
  (`C:\Users\flint\.rustup\…\library\core\src\time.rs`, `C:\Users\flint\.cargo\registry\…\criterion-0.5.1\…`).
  They are anonymous data like every other, so they never reach a normalised body.

**Decides.**
- `normalize::ANON_PREFIXES` = the one form seen (`anon.`) plus the 03 list (`.L*`, `__unnamed_*`,
  `alloc_*`, `str.*`, section symbols). Content-named constants stay names. SEH tables, funclets and
  switch tables are named through their owner.
- **Control (ix) runs as designed** on H-2's `delete_entity`, whose normalised body holds the
  `<anon-data>` that the vm_column.rs:281 assert owns (critique W2's fallback is not needed; its rule
  is recorded in the cut addendum anyway).
- **Control (x) runs two-sided as designed**: `drain_runaway_panic` is referenced from four located
  bodies (`code ref` lines): H-2's setup closure, H-2's `delete_entity`, and H-3's `Schedule::run` in both
  `clear` and `phase9_scheduler`.

## (ii) Symbol-size source — `probe-i-ii.txt` (`-- <id> <subject> | … | sec … sel=… raw=… aux=… | members=…` rows)

**Answers.**
- `llvm-nm -S` prints a zero size on every one of the 18357 (swap_remove) / 55177 (boyko_demo) /
  12183 (clear) / 18690 / 19449 defined lines: COFF symbol records carry no size.
- Every located candidate function sits at **offset 0 of its own COMDAT section** (`sel=NoDuplicates`,
  `IMAGE_SCN_LNK_COMDAT`), and the section's aux `Length` equals its `RawDataSize` in every row.
- Sections are **not** always single-symbol: MSVC-style SEH funclets `?dtor$<n>@?0?<fn>@4HA` live in the
  owner's section at non-zero offsets (e.g. `delete_entity`: section 2195 B, function 0..2080, two
  funclets at 2080 and 2144). `llvm-objdump --disassemble-symbols` stops at the next symbol.
- Local EH continuation labels `$ehgcr_<function ordinal>_<n>` also sit inside function sections
  (found in the (7)(b) multiset, probe (v)).

**Decides.**
- **Leg (2)'s size = the section's `RawDataSize`, and the pinned body = every symbol of that section**
  (the function, its funclets and labels, in offset order), so the disassembly ends at the section end.
  `ObjView::body` does exactly this.
- **Leg (7)(b)'s size** = the distance to the next symbol in the section, or to its end
  (`CoffTable::sizes`), which is the only definition that works for every symbol kind.
- Every candidate is a COMDAT, so `/OPT:REF` can see and remove each one (feeds (v)).

## (iii) Inline-frame resolution — `probe-iii.txt`

**Built:** swap_remove under `sensitivity-map` with `-C link-arg=/MAP:<target>/ug15/maps/probe-iii-swap_remove.map`.

**W3's precheck, from the object first:** of the five H-2 bodies, only `delete_entity` references a
`vm_column.rs` `Location` (`anon.af1772c5….8`), and none references a `VmColumn…::swap_remove` symbol. So
the frame is inlined in the chosen body before the symbolizer is read.

**Answer: PASS — route (a1).**
- The MSVC Build Tools symbolizer has **no `--pdb` option** (`unknown argument '--pdb=…'`; `--help` lists
  none). It finds the PDB through the image's CodeView record (`/PDBALTPATH:%_PDB%`, bare file name, PDB
  beside the image). `--relative-address` ("Interpret addresses as addresses relative to the image base",
  `--help`) takes RVAs.
- `/MAP` "Static symbols" names the local: `0001:000330a0 _RNv…EcsMaster13delete_entity 00000001400340a0 f
  swap_remove-….rcgu.o`; preferred load address `0x140000000`; body RVA `0x340a0`, length 2195.
- All 550 instructions in that range resolve; the outermost frame of each is `EcsMaster::delete_entity`.
  106 distinct frame functions over 44 files.
- RVA `0x34481` resolves the chain `VmColumn::swap_remove @ vm_column.rs:282` ←
  `Archetype::remove_entity @ archetype.rs:1383` ← `EcsMaster::delete_entity_core @ entity_api.rs:1051` ←
  `EcsMaster::delete_entity @ entity_api.rs:779`. So `line-tables-only` keeps inline-site records for a
  local function in the msvc PDB (00's open item, answered).

**Decides.**
- The file map (sensitivity map (a)) comes from the msvc image plus its `sensitivity-map` PDB. No
  windows-gnu build (`D:/wt/_targets/b3-gnu` was never created).
- **Map normalisation (for stage 2's `map a capture`):** frames name files in three spellings — the
  worktree's absolute path (`D:\wt\lighttable\crates\…`), the rustup source tree
  (`C:\Users\flint\.rustup\toolchains\…\lib\rustlib\src\rust\library\…`) and rustc's remapped std
  (`/rustc/48a229ce…/library\…`). A committed row must be repo-relative (`crates/…`) or `library/…`,
  with `/` separators, or the map would differ between worktrees.

## (iv) P6-1 reproducibility — `probe-iv.txt`

**Built:** (A) each of the five subjects twice under `release`, the second a forced final-unit rebuild;
(B) swap_remove, then the six census crates' `src/lib.rs` modification times bumped (front-end rebuild
of all six and every dependent, **no `cargo clean -p`, nothing deleted** — critique O7, cut D10), then
swap_remove again; (C) boyko_demo and clear twice each with `-C link-arg=/Brepro`.

**Answers.**
- **(A)** Object **byte-identical** for all five (e.g. swap_remove `c6fead8d…` twice, boyko_demo
  `3f91d946…`, clear `5c516d98…`); normalised candidate bodies and the (7)(b) multiset identical.
  Images differ only in the COFF `TimeDateStamp` and a handful of `.rdata` bytes at one place per
  image (the debug directory's timestamps and the CodeView record).
- **(B)** After the front-end rebuild of all six census crates: swap_remove's object is again
  `c6fead8d…`, byte-identical to (A).
- **(C)** `/Brepro`: boyko_demo's two images **byte-identical**; clear's are **not** (TimeDateStamp,
  four debug-directory timestamps, 17 B of the CodeView record, and 32 B at `.rdata +0x2bfec4`), while
  its object is byte-identical. Why `/Brepro` does not settle clear's image is not established here.
- Side evidence (`leg7/exp-clear-*.snap`): `bench-shipped` and `release` (the same configuration under
  two names) gave the same `-C metadata`, the same rlib hashes, and identical leg-(7) (a), (b) and (c)
  for clear; and clear's release object `5c516d98…` was byte-identical across builds with and without
  a `/MAP` link argument.

**Decides.**
- **`profile.release` is reproducible for the census.** Legs (2), (7b) and (7)(b) use it (legs 7/7b
  through `seam-census`, which is now configuration-identical to it). **No `[profile.ug15-det]`.** 03 §6
  (2) Profile keeps `profile.release`.
- Leg (7)(a)/(c) compare section sizes and export entries only, never image bytes, as the cut says;
  `/Brepro` identity is recorded, not required.
- Critique O9 is moot: probe (iv) passed, so commit A's leg-(7) snapshot is taken under `seam-census`.

## `seam-census` as the plan specifies it fails its own check — the profile was corrected

**Measured** (`leg7/superseded/`, `leg7/exp-clear-*.snap`, `textcheck-*.txt`):
- With `strip = "none"` (03 §6), `seam-census`'s `.text` was 1,957,315 B for clear against release's
  1,956,051 B, and 8,546,370 against 8,546,242 for boyko_demo; 633 multiset lines differed for clear.
- Cause: cargo hashes `strip` into every unit's `-C metadata` (`libboyko_ecs-816519045fb9c19b` under the
  old seam-census, `-2f70a22dd5086947` under release), which moves every crate's disambiguator, v0 names
  and CGU partition, and the code with them. The profile NAME is not hashed (bench-shipped above).
- `strip` acts at link time; the census reads the post-LTO object; and on msvc `release` still links
  with `/DEBUG`. The key bought nothing.

**Decision (mine, a mechanism fork):** `[profile.seam-census]` is `inherits = "release"` with no other
key (commit `05cd7715`). It exists for its artifact directory, which only `ug15` writes (critique W4).
After the change, `ug15 textcheck`: `.text` release = seam-census for both subjects (8,546,242 and
1,956,051). Full leg-(7) comparison of seam-census against release on one tree (`leg7-A.txt`, taken at
`c2756e82`): clear **identical** in (a), (b) and (c); boyko_demo identical except `.reloc` 26,504 vs
26,508 and one anonymous datum of 102 vs 98 bytes. That datum is a path: a third-party build script's
output (`…\seam-census\build\glutin_wgl_sys-<hash>\out/wgl_extra_bindings.rs`, found in the object) is
embedded as a panic `Location`, and `seam-census` is four characters longer than `release`. Any profile
with its own directory has it; leg (7) always compares seam-census with seam-census, so it cannot show
there. The objects also differ in bytes between profiles (the COFF `S_OBJNAME` record carries the
object's own path), never within one profile (probe (iv)).

## (v) Survival under fat LTO and `/OPT:REF` — `probe-v.txt`, `probe-v-branch.diff`, `leg7/probe-v-*`

**Built:** control branch `u/b3-ctl-0` cut from `05cd7715`, with three items planted in `boyko_ecs`
(uncommitted on the branch; the files were then restored from saved copies, `git status` clean,
`git switch u/b3`, `git branch -d u/b3-ctl-0` — no force needed): (a) `#[unsafe(no_mangle)] pub fn
ug15_probe_v_uncalled`, uncalled; (b) `pub static UG15_PROBE_V_TABLE: fn() -> u64 = ug15_probe_v_target`,
no attribute, no reader; (b′) `pub static UG15_PROBE_V_LIVE` read by `core::hint::black_box(&…)` at the
top of `EcsMaster::new`. Every body distinctive. boyko_demo and clear under `seam-census` with `/MAP`.
The parent is commit A's snapshot (`leg7/leg7-*-A.snap`).

**Answers.**

| item | post-LTO object | image (`/MAP` publics + statics) | exports |
|---|---|---|---|
| (a) uncalled `#[no_mangle]` | **present, global `T`**, 33 B, both subjects | **absent** (removed by `/OPT:REF`) | none |
| (b) static + pointee, no reader | **absent**, both subjects | absent | none |
| (b′) static read in `EcsMaster::new` | clear: `r boyko_ecs::UG15_PROBE_V_LIVE` 8 B and `t boyko_ecs::ug15_probe_v_live_target` 33 B; **boyko_demo: absent** | clear: both in "Static symbols"; demo: absent | none |

- boyko_demo never calls `EcsMaster::new`; it builds its world with `EcsMaster::with_capacity`
  (`leg7-boyko_demo-A.snap` has `with_capacity` and no `new`). So a read placed in `new` is unreachable in
  demo, and (b′) survives only in clear.
- The (b′) edit also moved unrelated bodies in clear: `EcsMaster::new` 576→592, `apply_via_raw_twin`
  316→313, a `QueryDataState::update` 512→528, two `heapsort` instantiations; `.text` +3,552 B. This is
  critique C1 measured: a section-size reading would go red from the carrier edit alone.
- boyko_demo's (7)(a) moved too (`.text` −16, `.reloc` +4) although nothing planted reached its image.
  Adding non-generic code to `boyko_ecs` perturbs fat-LTO codegen by itself.
- `ug15 leg7 compare A child --expect UG15_PROBE_V_LIVE` (and `--expect ug15_probe_v_live_target`):
  **PASS on clear, FAILED on boyko_demo**; `--expect UG15_PROBE_V_TABLE`: FAILED on both
  (`leg7/probe-v-*-expect-*.txt`).
- The first leg-(7)(b) diff was flooded by 66 renumbered `$ehgcr_<ordinal>_<n>` labels (demo);
  they now normalise to `$ehgcr` (`c2756e82`).

**Decides.**
- **Control (viii) as written cannot go red** (PC-13 confirmed): it takes form (b′). The pass condition is
  the named reading (critique C1): `ug15 leg7 compare <parent> <child> --expect <static> --expect
  <pointee>`, which passes only if both names appear among the diff's child-only (b) entries. The
  subject is **clear**, because boyko_demo does not reach `EcsMaster::new`.
- **Control (vii)** stays as written (a called `#[inline(never)]` fn survives `/OPT:REF`; `/OPT:REF` is on
  the link line).
- **UG-16 (B2):** an uncalled `#[no_mangle]` kernel fn is a global `T` in the post-LTO object but is
  removed by `/OPT:REF` from the msvc image and exported by nothing; so on msvc its image delta is not
  caused by the symbol itself, and leg (7)(b) (object) is what sees it.

## Leg (7b) member formats (PC-7) — `rlibs.txt`

boyko_demo under `seam-census`: five census-crate rlibs (`boyko_macros` is a proc-macro dll, no rlib).
Each has one `lib.rmeta`, one small COFF member (180–1212 B) and N bitcode members: boyko_ecs 16,
boyko_log 7, boyko_threadpool 5, boyko_diag 2, boyko_utils 1. **Decides:** (7b) reads bitcode with the
rustup `llvm-nm` (LLVM 22.1.8 = rustc's LLVM); a member of any other format is RED (`MemberFormat`).

## Candidate presence (feeds the frozen pin list at capture)

| candidate | where located (release) | absent in |
|---|---|---|
| P29-1..4 | boyko_demo, clear (identical bodies in both) | — |
| R-1 `register_new::<Transform>` | clear, **three local copies** (one per instantiating crate; demangled names identical; bodies identical) | — |
| R-2 `Transform::component_id` | clear (43 B) | — |
| D-2, D-3 | clear, boyko_demo | — |
| D-4 `resource_registry::register_new::<R>` | clear (`OcclusionForce`), boyko_demo (`SpatialGrid`) | — |
| D-5 `register_event_new::<E>` | — | clear, boyko_demo |
| R-3 `try_register_dynamic` | — | clear |
| R-4 `register_layout::<T>` | query_dsl (`query_dsl::Tag`) | swap_remove |
| S-1 `add_system::<F, M>` | — | clear |
| H-1 | query_dsl (5 bodies) | — |
| H-2 | swap_remove (5 bodies incl. `delete_entity`) | — |
| H-3 `Schedule::run` | clear, phase9_scheduler | boyko_demo |

Stage 2 must decide how a normalised name with several copies (R-1) is pinned; the copies differ only in
the instantiating crate, which the v0 demangler drops.
