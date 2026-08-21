# `boyko_reflect` — the gates plan: absence, cost, CI

> **Status:** PLAN — a rung ladder a developer executes. Rev 1, 2026-08-21, branch `feat/reflection`.
> **Design input:** `docs/REFLECTION-ANALYSIS.md` (rev 2026-08-21). Read §2, §7, **B.6** and **B.9**
> before this file; nothing here re-argues them.
> **Siblings** — cross-referenced by filename, never duplicated:
> `docs/REFLECTION-PLAN-CORE.md` (registry, `TypeInfo`, the value taxonomy, the derive),
> `docs/REFLECTION-PLAN-ECS.md` (enumeration, the by-id structural seam, the refusal matrix),
> `docs/REFLECTION-PLAN-BOUNDARY.md` (`Sink`/`Source`, name-keyed roundtrip).
> `graphify` CLI is not installed on this machine; orientation was Grep/Read.

This is the load-bearing half of the campaign, and it is load-bearing for a reason that has nothing
to do with reflection: **the gate is the only thing standing between "the shipped game does not
contain the editor's reflection layer" and a sentence in a design document.** The analysis is
explicit that the mechanism is build hygiene plus a CI gate rather than a compiler property (§0/§2),
which means the gate *is* the mechanism. A gate that cannot fail leaves the claim with no mechanism
at all.

This campaign has paid for that lesson often enough that it is written into the ladder's shape:
every rung below carries a **RED MUTATION** — a specific edit that must make the rung's gate fail,
run and recorded before the rung is called done. The corpus of prior failures this rule comes from
is not abstract:

* twelve benches in a gate table, **none of which existed**, while eleven rungs reported "gated"
  (logging L10);
* a symbol census that read `1` in every cell and could not fail at all, until two REDs of the same
  shape were run against it (`crates/profile_fixture/tests/profile_axis_census.rs`, the `--gc-sections`
  leg that *changed nothing*);
* a bare `cargo check --all-targets` at this workspace root that compiled a `println!` in 0.2 s and
  reported success while every engine crate went unchecked (root `Cargo.toml`'s `default-members`
  comment records the measurement: 0 errors where `--workspace` found 4);
* a Miri job that is a **hand-listed allowlist**, so a new package is not covered until it is named
  (`.github/workflows/ci.yml:222-226`).

Each of those four lands directly on this plan, and each has a rung.

---

## 0. What this plan gates, stated so it can fail

The claim under gate, in the owner's words as §0 records them — *"on while developing, literally
absent in the shipped game"* — decomposes into **four independent properties**. They are independent
in the strong sense: each can be false while the other three are true, and no single instrument sees
more than two of them.

| # | Property | The instrument | Rung |
|---|---|---|---|
| **P-A** | No workspace manifest can *cause* `boyko_reflect` to enter a ship target's closure — no shared crate declares a `reflect` feature, no `default` list enables one, no dependency edge on `boyko_reflect` is non-optional | manifest census over `cargo metadata`'s **packages** half (invocation-independent) | **G1** |
| **P-B** | The *invocation that ships* resolves without it | `cargo tree -p <ship> -e features --edges normal,build` | **G2** |
| **P-C** | The linked artifact carries no symbol from the crate | `llvm-nm` census, fat-LTO-linked, three legs | **G3** |
| **P-D** | With the feature off, the derive emits **nothing** — no tokens, no codegen, no residue inside the shipping crates it expands into | compile refusal (`E0433`) + token census + codegen identity | **G6**, **G7** |

**Why P-D is separate from P-A/P-B/P-C and cannot be folded into any of them.** The reflection opt-in
is a *token* emitted by `boyko_macros` into whatever crate wrote `#[component(reflect)]`
(A.5, and `crates/boyko_macros/src/component.rs:348` — the `component_id()` install funnel, whose six
existing slots `#storage_install` … `#serialize_install` are exactly this shape). Tokens are not
dependency edges: `boyko_macros` does **not** depend on `boyko_ecs` and never will
(`crates/boyko_macros/Cargo.toml`), and `aether_lang` states the same rule verbatim in its own
manifest. So a botched `#[cfg]` on the reflect slot produces code **inside a shipping crate** with
**no manifest edge to see** and — if it compiles at all — no `boyko_reflect` symbol either. P-A and
P-B are structurally blind to it; P-C sees it only if the residue happens to name the crate.

---

## 1. What each half catches that the other cannot

The analysis presents `cargo tree` and the symbol check as two ways of saying the same thing (§2,
§7 Wave 0), and **B.6 already corrected half of that** — the `cargo tree` half is load-bearing, the
symbol check is corroboration. This plan states the rest of the correction: they have different
subjects, and each is *structurally* blind to a class the other catches.

| instrument | subject | catches | **structurally blind to** |
|---|---|---|---|
| **G1** manifest census | the manifest set, as written | a `reflect` feature on a crate every ship target depends on (the §2 marker-feature footgun, generalized); `boyko_reflect` declared non-optional anywhere; `reflect` in any `default`; a consumer that wrote `boyko_reflect = { optional = true }` and then forgot `dep:` | anything true of a *build* and false of the manifests: a `--features` on the command line, a `[patch]`, a `.cargo/config.toml` default, a `cargo tree` the operator never ran |
| **G2** `cargo tree` | one resolved invocation | the shipping invocation actually differing from the manifest reading; feature unification reaching the ship crate through a shared dependency | every member it is not run for; and **derive residue**, which has no edge |
| **G3** symbol census | the linked artifact | derive residue that names the crate; a `#[cfg]` that failed to strip; a `TypeInfo` that migrated into `boyko_ecs` under B.1 Horn 1 and now ships | **the dependency edge itself when it contributes no reachable symbol** — a build that compiles and links `boyko_reflect` and then strips it reads **0** here, while "literally absent from the shipped game" is false (it was resolved, compiled, and linked). And, without fat LTO, everything: MEASURED in-tree, `--gc-sections` had *no effect whatsoever* on this exact target |
| **G6/G7** compile + codegen proof | the token stream and the emitted code | a derive whose off-arm emits anything at all — the one class that lands *inside a shipping crate* | anything about the resolver or the artifact |

**The asymmetry that decides the ranking, and it is the opposite of the intuitive one.** Reflection
is the *favourable* case for a symbol census — with the feature off, `boyko_reflect` is not in the
resolved closure, so no rlib is built and there is nothing to carry into the image (B.6). That is
precisely why its zero is cheap and why it must not be read as the proof: **the zero is earned by
the resolver, and the census is reporting the resolver's work under its own name.** G3 therefore
carries a third leg (§G3, leg **L3**) whose entire purpose is to find out whether the census can see
a linked-but-unused crate *at all* on this target. If it cannot — which is what B.6's measurement
predicts — then that is recorded as the census's stated limit, not discovered later by someone who
trusted it.

---

## 2. Decisions

Numbered so a later reader cannot re-litigate them from scratch. Each carries its reason and the
alternatives rejected.

### D1 — The gate is four instruments over four properties, not one gate with two halves.

**Reason:** §1's blindness table. Any single instrument leaves a class unwatched, and three of the
four classes have in-tree precedent for going unnoticed.
**Rejected:** (a) symbol census alone — B.6 measured that its naive form cannot fail; (b) `cargo tree`
alone — blind to derive residue, which is the one class that lands *inside* a shipping crate;
(c) "the compiler will catch it" — true only for the `E0433` sub-case (G6a), not for a residue that
compiles.

### D2 — The ship target is `boyko_demo` + the root `boyko-engine` package. The **gated artifact** is a purpose-built fixture, not either of them.

**Reason.** §2's `game_app` / `editor_app` do not exist; the analysis says so itself. What exists:
`crates/boyko_demo` is the only game-shaped `[[bin]]` member (`crates/boyko_demo/Cargo.toml:7`), and the
workspace root is also a package (`boyko-engine`, `src/main.rs`). But `boyko_demo` is **excluded from
every CI leg** — `--exclude boyko_demo` appears on `.github/workflows/ci.yml:62, :87, :89, :129, :167,
:176, :191` — and it pulls eframe/egui/winit/wgpu. A gate whose artifact takes minutes to link is a
gate that gets moved to a nightly and then to nowhere.

**`boyko_app` is NOT a ship target and must not be named as one.** `docs/REFLECTION-PLAN-CORE.md`'s
C0 lists it beside `boyko_demo` and the root package; measured,
`crates/boyko_app/Cargo.toml:13` declares `[lib]` and **no `[[bin]]`**. It is the host *library*, not
an artifact anything links as a program, so it has no image to census and no closure to ship. It is a
legitimate subject for **P-A** (a manifest is a manifest) and for nothing else. **This decision is the
single owner of the ship-target list; CORE cites it rather than restating it.**

So the two roles split. **`boyko_demo` and `boyko-engine` are the ship targets for P-A/P-B** (G1/G2 are
cheap manifest/resolver reads and can name every member). **A fixture package is the artifact for
P-C/P-D** (G3/G6/G7), following the precedent verbatim: `crates/profile_fixture` builds its own two
legs rather than consuming other jobs' artifacts, *"explicitly so it is runnable locally — a gate
only CI can run is a gate whose RED nobody has seen"* (`.github/workflows/ci.yml:137-139`).

**Rejected:** (a) gating the demo binary directly — cost, plus it is excluded from CI, so the gate
would run in no leg that exists; (b) inventing the `game_app` the analysis names — it does not exist
and inventing a package to satisfy a gate is the gate deciding the architecture.

### D3 — ~~A `reflect` feature may only be declared by a **leaf**~~ → **REPLACED (2026-08-21, second pass): six clauses that survive unification, and the leaf rule is one of them, narrowed.**

> **Why this decision changed.** The leaf rule is unimplementable *and* it forbids the dogfood, and
> the two facts are the same fact. Every dogfood target the four plans name is defined in a shared
> engine crate — `Transform` / `Name` / `Visibility` (`boyko_scene`), `GpuTransform3D` /
> `EmitterActive` (`boyko_render`) — and the opt-in's `#[cfg(feature = "reflect")]` is evaluated in
> the **defining** crate. Measured: `boyko-scene` has **five** workspace dependents (`aether_tests`,
> `boyko_app`, `boyko_physics`, `boyko_render`, `boyko_ui`), so C1-as-written reds the moment
> `Transform` opts in. And the declaration cannot be dodged by omission: root `Cargo.toml:25-26`'s
> `unexpected_cfgs` check-cfg list **adds to** Cargo's per-manifest feature list, so a
> `#[cfg(feature = "reflect")]` in a crate with no such feature warns and the existing `-D warnings`
> gate reds it. Full analysis, the three candidates and the decision: **`REFLECTION-ANALYSIS.md`
> B.12**; the owner row is **B.13 #1**. This plan **proceeds on B.12's option (b)**.

The original reason for the leaf rule stands and is not withdrawn: `boyko_ecs`'s own manifest records
that while `profiling-analysis` was default-on, *"there was NO command line that could turn it off"* —
nine siblings depend on `boyko-ecs` without `default-features = false`, and unification put the
default straight back; `cargo tree --workspace -e features --no-default-features` still reported it
**ENABLED**. **But that is a measurement about a DEFAULT-ON feature**, and the same manifest states
the conclusion the tree actually drew: *"opt-in is the only shape in which the flag means what it
says. Enable it the same way `hwrt` and `bench-alloc` are enabled: `--features
boyko-ecs/profiling-analysis`."*

The tree then built the general case, three crates deep and all shared:
`boyko_rhi_vulkan` `hwrt = []` → `boyko_render` `hwrt = ["boyko_rhi_vulkan/hwrt"]` (`:31`) →
`boyko_app` `hwrt = ["boyko-render/hwrt"]` (`:48`). Default OFF at every level, in no ship build,
because **nothing enables it**. `boyko_render`'s `test-readback` is the same shape with a different
enabler (a self-referential dev-dependency, `crates/boyko_render/Cargo.toml:91`). The property that
defeats unification is *"nothing enables it"*, not *"nobody declares it"*.

The one thing wrong with `hwrt` is **F17** — `grep -c hwrt .github/workflows/ci.yml` = **0**, so every
`#[cfg(feature = "hwrt")]` body in the tree is compiled by nothing. That is a **coverage** defect, not
a **containment** defect, and clause C6 below exists so this design does not inherit it.

**The six clauses** (each is a clause of G1's census; all are decidable from `cargo metadata --no-deps`):

| # | clause | catches |
|---|---|---|
| **C1** | No member's `default` list transitively enables a `reflect` feature or names `boyko-reflect`. | the `profiling-analysis` failure verbatim |
| **C2** | Every dependency edge naming `boyko-reflect` has `optional == true`. | a non-optional edge, unconditionally in the closure |
| **C3** | A feature that pulls the crate is written exactly `["dep:boyko-reflect", …]`. | the bare `["boyko-reflect"]` form, which implicitly mints a second feature — how a consumer silently acquires an always-on optional dependency |
| **C4** | **No dependency edge of ANY kind lists `reflect` or `<pkg>/reflect` in its `features` array.** Enablement is by a `[features]` forward or a command line — never by an edge. | the one form nothing can switch off: the same manifest records that *"an explicit `features = [...]` survives `--no-default-features` by design"* |
| **C5** | **No ship-target member (`boyko_demo`, root `boyko-engine`) declares or forwards a `reflect` feature.** | the leaf rule, narrowed to where unification can actually reach a shipped artifact — this is what survives of the old C1 |
| **C6** | **Every command line in the repo that enables a `reflect` feature is a named row in `tests/reflect_ci_coverage.rs`.** | **F17**: a feature-gated body compiled by no leg. Measured, not feared |

**Consequence, and it is a real constraint on where tests live** — unchanged in force, changed in
shape: the trybuild corpus and the fixtures still live in **their own consumer packages**, because a
trybuild harness compiles its fixtures with its own package's features. But the reason is now
ergonomic rather than prohibitive, and it is joined by a harder one — **Miri cannot execute FFI**, so
the package the Miri row names must not reach `boyko_rhi_vulkan`. Hence **two** consumer packages
(D15). **Still rejected:** a `#[cfg(test)]`-only feature on `boyko_ecs` — Cargo features are not
`cfg(test)`-scoped and unify across the whole build; and a `reflect` feature on `boyko_ecs` at all,
which C5's spirit and §2's original finding both refuse (the kernel is on every ship path, so its
feature has no off-switch that a mistake cannot reach).

### D4 — `boyko_reflect` itself carries **no** `reflect` feature, and therefore has no self-hosted derived-type tests.

**Reason:** the `#[cfg(feature = "reflect")]` in the derive's output is a *consumer-side* construct
(A.5 — cfg in derive output is evaluated in the consumer crate). A self-hosting exception would test a
configuration no consumer ever has, and would put a `reflect` feature on the crate whose absence is
the whole claim.
**Consequence for B.9:** the Miri row that matters is the **fixture's**, not the crate's. `boyko_reflect`'s
own unit tests cannot construct a `#[component(reflect)]` component, so `-p boyko-reflect` alone under Miri
covers the arithmetic and none of the `unsafe`. Both packages go on the allowlist and the fixture goes on it
**with the feature on** (G4).

**This decision is the campaign's single owner of the Miri wording, and three sibling documents were
wrong about it.** `docs/REFLECTION-PLAN-CORE.md` §7 item 2 / C4 gate 4 / C11 gate 3,
`docs/REFLECTION-PLAN-ECS.md` §10 and `docs/REFLECTION-PLAN-BOUNDARY.md` B7 all inherited
`REFLECTION-ANALYSIS.md` B.9's closing line — *"the sweep must run with the feature ON, or it compiles
an empty crate and reports green"* — and applied it to `boyko_reflect` itself. Both halves are false:

* `cargo +nightly miri test -p boyko-reflect --features reflect` is a **hard cargo error**
  (*"none of the selected packages contains these features: reflect"*), because the crate has no such
  feature and this decision says it never will;
* with the feature "off" the crate is **not empty** — nothing in `crates/boyko_reflect/src/**` is
  `cfg`-gated, so its whole contents always compile.

**The line that lands, verbatim, in `.github/workflows/ci.yml` (G4):**

```
cargo +nightly miri test --all-targets \
  -p boyko-ecs -p boyko-utils -p boyko-threadpool -p boyko-serialize \
  -p boyko-math -p boyko_sdf_math -p boyko_image \
  -p boyko-reflect \
  -p reflect-fixture --features reflect-fixture/reflect
```

`reflect-dogfood` is **deliberately absent** — it reaches `boyko_render` → `boyko_rhi_vulkan`, and
Miri cannot execute FFI, which is why this sweep is hand-listed at all (F18).

**Two constraints on the fixture that the analysis does not state and that turn this row red for
reasons unrelated to reflection if they are missed** — both because the sweep runs `--all-targets`:

* **Miri cannot spawn processes.** `reflect_absence_census.rs` (G3) and `reflect_codegen_identity.rs`
  (G7a) both run `Command::new(env!("CARGO"))`. Each needs `#[cfg(not(miri))]`. The template they
  copy — `crates/profile_fixture/tests/profile_axis_census.rs` — carries **no such guard**, verified,
  and does not need one *because `profile_fixture` is not on the allowlist*. Copying it verbatim onto
  the allowlist imports the failure. The likeliest "fix" for it is dropping `-p reflect-fixture`,
  which silently reverts B.9 — which is why the guard is named here rather than discovered.
* **`reflect_optin_cost.rs` (G7b) needs the same guard**, for the same reason `--all-targets` reaches
  it: a criterion bench under Miri is a wall-clock instrument executing on an interpreter.

### D5 — Two needles, chosen by symbol **kind**, and the crate-name fragment is the primary.

B.6's method, cited rather than rediscovered: *ask what kind of symbol it names, not which subsystem
it belongs to.* A **generic** function exists only if a site instantiated it (decidable without LTO);
a **plain function in a dependency's rlib** is codegen'd regardless (not decidable without LTO).

* **Needle A — `boyko_reflect`**, the crate-name fragment. Both Rust mangling schemes encode the
  defining crate, so this counts *every* symbol the crate contributes, including instantiations that
  land in a downstream CGU. It is a count, not a boolean, and the count is the measurand.
* **Needle B — `install_type_info`**, a plain `pub fn` in the crate's rlib. Its only job is to be the
  **LTO-sensitivity probe**: it is the same kind of symbol as `mint_cold`
  (`crates/profile_fixture/tests/profile_axis_census.rs:90`), the one B.6 measured as undecidable
  without `lto = "fat"`.

  **Why *this* plain fn and not another, stated because the reason is an ordering constraint, not
  taste:** G0 lands `install_type_info` as a deliberately hollow `#[inline(never)] pub fn` so this
  census has a subject three rungs before any real registry exists, and
  `docs/REFLECTION-PLAN-CORE.md`'s **C2 replaces that body with the real one**. The needle is chosen
  because **the name survives the replacement** — C2 keeps the signature (`install_type_info(component_id:
  usize, info: &'static TypeInfo)`, its D6) and keeps it a plain `pub fn`, so the census's subject does
  not move under it. A needle named after anything G0 invents for its own convenience would go silently
  subject-less at C2, which is the failure mode D5 exists to avoid one paragraph above.

**Rejected:** a single hand-picked function name — it names one symbol and is silent about every
other, which is how a census acquires a subject that can vanish while the check stays green.

### D6 — Tool absence is a **RED**, never a SKIP.

Inherited verbatim: *"a gate that passes on every machine lacking its tool is a gate that passes"*
(`.github/workflows/ci.yml:141-143`; the panicking resolver at
`crates/profile_fixture/tests/profile_axis_census.rs:131`). `components: llvm-tools` on the new job,
and the `llvm-nm` / `llvm-size` resolution copied from
`crates/profile_fixture/tests/profile_axis_census.rs:165` — **copied, not shared**, for the reason that
file states: sharing it would mean a dev-dependency edge that unifies features into the very images
under census.

### D7 — Both CI legs are mandatory, and neither is redundant with the other.

* **Feature OFF is not "the existing jobs, for free."** Nothing in the workspace enables `reflect`, so
  with only OFF legs `boyko_reflect` is compiled by **nothing in CI** — it would be a package in the
  tree that never sees a compiler. That is the `default-members` vacuity in a new costume.
* **Feature ON is not sufficient either.** With only ON legs, the derive's `#[cfg]`-off arm is never
  compiled, so P-D goes unwatched — and the off arm is the one that ships.
* **The ON leg must be a two-cell matrix `{debug, release}`.** §5's release-editor gap is the reason:
  the legitimate `--release --features reflect` editor build compiles `debug_assert!` out, which is
  exactly where the kind-check's load-bearing `-> bool` return is the only remaining guard. A
  debug-only ON leg never exercises the configuration the gap is about. The existing `test` job is
  already `matrix: profile: [debug, release]` (`.github/workflows/ci.yml:78-79`); the ON job mirrors it.

Net: **four cells** — `{off, on} × {debug, release}` — of which two already exist.

### D8 — The hot-path cost proof is a **codegen-identity** proof, not a wall-clock A/B.

The analysis claims the compile-boundary argument is *stronger* than a runtime branch (§8, last
bullet). This decision is what turns that from an assertion into a measurement, and it also refuses
the measurement that would look like one.

**A feature-on-vs-off wall-clock A/B is cross-build by construction** — the feature decides what is
compiled — and this campaign has already measured what a cross-build absolute is worth on this box.
`crates/boyko_ecs/benches/gj1_flag_cost.rs:22-29` refused its own leg C for exactly this: *"the same
unchanged bench leg read **10.16 / 10.94 / 11.72 / 12.11 ns** across four sittings, a spread wider
than anything the comparison could have found. A number taken that way would be drift wearing a
verdict's name."*

And a timing null is the weaker claim anyway. "No measurable delta" is a statement about the
instrument's floor; "the symbol does not exist" is decidable. So the primary instrument is a
**comparison of emitted code**, and the timing half is demoted to what it can honestly carry (D11).

**Rejected:** (a) criterion feature-on vs feature-off — cross-build, refused above; (b) asserting the
0 % claim from the absence gate alone — that proves the crate is absent, not that the *derive* left
nothing behind in the crates it expanded into, which is a different question with a different subject.

### D9 — The measurand of the codegen-identity proof is the **sorted symbol-name multiset** plus the `.text` size — with raw-byte identity **measured, then adopted only if it holds**.

Raw binary identity is the tighter claim and the tempting one. It is also decided by things unrelated
to the subject: PE images carry a link timestamp, and a mismatch there would make the null
uncertifiable for a reason that has nothing to do with reflection. So the plan does not *predict*
which measurand is available — G7 **measures** it: build the same fixture twice, unchanged, and record
whether the bytes match. If they do, the measurand tightens to bytes and the tightening is recorded in
the ledger. If they do not, the symbol multiset + `.text` size stands, and the reason it stands is
recorded with the observed diff.

### D10 — The codegen-identity instrument needs **two** controls, and its null is a determinism null, not a resolution null.

`crates/boyko_log/benches/log_gate_cost.rs:42-46` records the trap in the general form: *"a zero
control whose expected value is exactly zero measures DRIFT, not RESOLUTION"* — P4-6's twin read 0 on
all ten passes, the rule silently became "is nonzero", and it reported a false RESOLVED.

Here the null's expected value **is** exactly zero (two builds of the same source must be identical),
so the null certifies **determinism** and nothing else. It cannot certify that the instrument would
*see* a difference. That needs a **positive control**: a third fixture identical to the first plus one
trivial `#[inline(never)]` function, whose symbol multiset must **differ**. If it does not, the
extraction is broken and every equality the run reports is an equality between two empty sets.

This is the same 2×2 discipline B.6 states for the census (*"a `shipping` binary with no emission
symbol is ambiguous on its own"*), applied to a different instrument.

### D11 — The timing half is measured **on the ON leg only, in one process**, and it measures the derive's per-component residue — never the crate's absence.

What is legitimately measurable in a single sitting: with the feature **on**, a component that opted
into `#[component(reflect)]` against a twin that did not, ABBA-counterbalanced, with the A-vs-A twin as the
sitting's floor. That is `gj1_flag_cost`'s shape, in-sitting, and it answers a real question — what the
opt-in costs a type that has it, including the first-touch `TYPE_INFO` registration §2 warns must not
be mistaken for a hot-path regression.

**What it may not be reported as:** a measurement of the OFF configuration. The OFF configuration's
cost is established by G7's codegen identity, not by this bench. Stated here because the analysis's
Wave 5 wording ("hot-loop 0%-gate, feature on vs off") invites exactly that misreading.

### D12 — One trybuild fixture directory, two harness legs: `compile_fail` with the feature on, `pass` with it off.

The same `.rs` fixtures serve both claims. Feature on, `#[component(reflect)]` applies and the derive must reject
→ `t.compile_fail()` against a blessed `.stderr`. Feature off, the attribute is stripped and the file
must **compile** → `t.pass()`. One directory, so the two legs cannot drift apart, and the off leg is a
genuine assertion rather than a comment.

**Rejected:** two fixture directories — drift, and the drift is invisible; an off-leg `t.pass()` only —
does not test the rejections; a compile_fail-only corpus — never compiles the arm that ships.

### D13 — The trybuild corpus's blessing is pinned to a compiler, and that pin already exists.

`tests/trybuild_corpus_compiler_witness.rs` freezes `BLESSED_RUSTC = "rustc 1.97.1 (8bab26f4f
2026-07-14)"` and counts the corpus (`>=`, so new fixtures never red it). Adding reflection fixtures
means blessing under that exact compiler. The recorded hazard is not hypothetical: a chocolatey
`rustc` 1.95.0 at `C:\ProgramData\chocolatey\bin` shadows `~/.cargo/bin` and **can bless `.stderr`
files that the mandated toolchain then rejects**, and `rust-toolchain.toml` does not stop it. Every
bless runs under `export PATH="$HOME/.cargo/bin:$PATH"`.

### D14 — Every gate carries a **RED ledger row**: the mutation, the observed red, the date. A gate with no run red is not landed.

The ledger is a table in this document (Appendix GB) plus a mechanical completeness check (G8). The
precedent for needing the mechanical half rather than the table alone: logging L10 found **twelve**
bench rows in a gate table where **not one bench existed**, and eleven rungs had reported themselves
gated against it.

### D15 — THREE new packages, not two: `boyko-reflect` (engine), `reflect-fixture` (user, FFI-free), `reflect-dogfood` (user, engine-types). Recorded as decisions because the census demands decisions, not omissions.

`tests/engine_packages_census.rs` asserts every member is in `boyko_diag::sample::ENGINE_PACKAGES`
(`crates/boyko_diag/src/sample.rs:150`) or in the test's own `USER_PACKAGES` exemption. Adding
members without rows reds it — which is a **free red on G0**, the first gate in this ladder that
fires without anyone building it.

* `boyko-reflect` → `ENGINE_PACKAGES`. It is engine code; a zone it ever declares is an engine zone.
* `reflect-fixture` → `USER_PACKAGES`, with `profile-fixture`'s recorded argument verbatim: *"the
  census argument needs the fixture to be a stand-in for a game's own crate, not for the engine."*
* `reflect-dogfood` → `USER_PACKAGES`, same argument.

**Why the third package exists, and why it is not a convenience.** Two constraints intersect and
neither can be relaxed:

| | `reflect-fixture` | `reflect-dogfood` |
|---|---|---|
| deps | `boyko-ecs`, `boyko-macros`, `boyko-reflect` — **and nothing else, ever** | the same **plus** `boyko-scene`, `boyko-render` |
| features | `reflect = ["dep:boyko-reflect"]` | `reflect = ["dep:boyko-reflect", "boyko-scene/reflect", "boyko-render/reflect"]` (a **leaf** umbrella, D3 C5) |
| on the Miri allowlist? | **yes**, with the feature on (D4) | **no** — `boyko_render` → `boyko_rhi_vulkan`, and Miri cannot execute FFI (F18) |
| carries | G3's census bins, G5's trybuild corpus, G6's twins, G7's images and bench; the engine's *shapes* reproduced locally (a `storage = "dense"` struct of `[f32; 4]`, a fieldless `#[repr(u8)]` enum, a tuple struct) | the acceptance test against the **real** `Transform` / `Name` / `Visibility` / `GpuTransform3D` / `EmitterActive` |
| its role | **the primary gated subject** | **the dogfood claim**, proved separately |

Three sibling documents say the Miri row and the real-engine dogfood are the same test
(`docs/REFLECTION-PLAN-CORE.md` C6 gate 1 — *"Both are real engine types, so this gate is a dogfood,
not a fixture exercise"* — C10 gate 1, `docs/REFLECTION-PLAN-ECS.md` EG1/EG3/EG8). They cannot be:
one row must be FFI-free and the other must reach `boyko_render`. Splitting them costs a package and
buys back two obligations that were otherwise mutually exclusive. **The fixture's local shapes are
therefore the primary subject, not a stand-in** — the phrasing in G4 point 3 is corrected accordingly.

**`reflect-dogfood` exists only if the owner takes B.13 #1.** If engine crates may not carry a
`reflect` feature, this package and its umbrella are deleted, the four gates that say "real engine
types" say "the engine's shapes", and nothing else moves (`REFLECTION-ANALYSIS.md` B.12,
"Reversibility").

**One exposure this creates, stated so it is never mistaken for a leak.** The invocation
`cargo test -p reflect-dogfood --features reflect-dogfood/reflect` turns `boyko-scene/reflect` on for
**every selected member**, `boyko_demo` included, because a bare root build selects every member
(F16). That is correct and expected — it is not the ship invocation. **G2's harness therefore asserts
that it was itself invoked with no `--features`**, and G3's legs keep per-leg `CARGO_TARGET_DIR`s, so
no artifact from the dogfood leg can be censused by mistake.

### D16 — The absence claim's **scope** is decided by `docs/REFLECTION-PLAN-CORE.md`'s resolution of the B.1 fork, and this plan is written to survive either horn.

If **Horn 2** (two tables) is taken, `TypeInfo` and every accessor live in `boyko_reflect`, needle A
covers the whole layer, and P-C is a clean "zero symbols from this crate".
If **Horn 1** (merge into `BIND_ACCESSORS`) is taken, the metadata type lives in `boyko_ecs` and
**ships** — `BIND_ACCESSORS` is already in the release binary unconditionally
(`crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs:277`) — so the absence claim
narrows from "no reflection metadata ships" to "no *reflection crate* ships", and G3 must say so in its
own message rather than let a reader infer the wider claim from a zero.

**This is an owner call that blocks G3's wording, not G3's construction.** G0–G2 and G4–G8 are
horn-independent.

---

## 3. Rung ladder

**Unconditional gate on every rung**, in addition to the rung's own:

```powershell
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace --all-targets --no-fail-fast
```

`--workspace` and `--no-fail-fast` are both load-bearing and both were added after a measurement
(CLAUDE.md, "Build commands"). Two further habits this ladder inherits and does not restate per rung:
clippy reporting *"Finished in 0.1–0.2 s"* right after an edit means stale fingerprints and the lints
did **not** re-run; and `cargo test -p X <filter>` fans over many binaries, so *"0 passed, N filtered
out"* masks the target you meant — select with `--lib` / `--test <name>` and grep `running [1-9]`.

Rungs are ordered so that **every gate is built and RED-proven before it has a real subject.** That is
deliberate and it is the opposite of the natural order. A gate written after the thing it gates is a
gate whose first run is green, and a first run that is green teaches nothing.

---

### G0 — the three packages, the ship target named, and the census rows they owe — **size S**

> **G0 is the first commit of the WHOLE campaign, not only of this ladder — and that is a change.**
> `docs/REFLECTION-PLAN-CORE.md`'s C0 also created `crates/boyko_reflect/` and
> `crates/reflect_fixture/`, with different contents, and its own gate 3 required a CI leg that only
> G4 can supply — while G4 sits behind G0–G3, which sit behind those same directories. **G0 owns the
> package creation; C0 is rewritten as a consume-and-extend rung that adds the red canary.** The
> campaign order is stated once, here, and cited by the siblings rather than restated:
>
> ```
> G0 → G1 → G2 → G3 → G4        (packages, manifest census, resolver, artifact census, CI legs)
>   └─ C0 (canary; consumes G4's leg) → C1 → C2 (replaces G0's stub) → … → C11
>        └─ G5, G6, G7  (need CORE's derive)      └─ EG0 → EG1 → EG2* → … → EG8
>             └─ G8 (ledger completeness, last)         └─ B0 → B1 → … → B7
> ```
> `*` EG2 is owner-gated (`REFLECTION-ANALYSIS.md` B.13 #2). G8 is deliberately last: it is the rung
> that watches the ladder.

**Lands.**

* `crates/boyko_reflect/` — package `boyko-reflect`, `[lints] workspace = true`, `edition = "2024"`,
  `[dependencies] boyko-ecs = { path = "../boyko_ecs" }`. Contents at this rung:
  `pub fn install_type_info(component_id: usize, info: &'static TypeInfo)` as a `#[inline(never)]`
  stub over an opaque `TypeInfo` placeholder, and nothing else. **Deliberately hollow** — it exists to
  be a census subject three rungs before a registry does. `docs/REFLECTION-PLAN-CORE.md`'s **C2
  replaces this body with the real one, keeping the name and the signature** — which is why D5's
  needle B is that name and not something invented here.
  **No `[features]` table, now or ever** (D4).
* `crates/reflect_fixture/` — package `reflect-fixture`, a **consumer**: deps `boyko-ecs`,
  `boyko-macros`, and `boyko-reflect = { path = "../boyko_reflect", optional = true }`; features
  `default = []`, `reflect = ["dep:boyko-reflect"]`. Its dependency table is the gate's argument in
  exactly the sense `crates/profile_fixture/Cargo.toml` states for itself — **a third production
  dependency added here does not weaken the gate's number, it destroys the gate's argument**, which
  is worse, because the number still looks right. It also has a second, harder reason now: this is
  the package the Miri row names, and Miri cannot execute FFI (D4/D15).

  **Four `[[bin]]`s and one `[[bench]]`, and the list is reconciled with every rung that consumes it**
  (G0 previously landed two of the four, so `reflect_never` and `reflect_off_twin_plus` were required
  by G3/G6c/G7a and created by nobody):

  | target | source | who needs it |
  |---|---|---|
  | `reflect_on` | ~~annotated (`#[component(reflect)]` on its components)~~ → **reflect-linked** (landed deviation below; the annotation arrives at CORE C7) | **G3 L2** — the present control |
  | `reflect_off_twin` | **the same source as `reflect_on`** | **G3 L1** (feature off — the ship cell); **G7a N1/N2** — the determinism null |
  | `reflect_never` | the same source **minus the `reflect` key** *(until C7: minus the linkage)* | **G3 L3** — the linked-unused leg (feature **on**); **G6c / G7a T** — the twin |
  | `reflect_off_twin_plus` | `reflect_off_twin` plus one `#[inline(never)] fn` | **G7a P** — the positive control |
  | `[[bench]] reflect_optin_cost` | — | **G7b** |

  > **Landed deviation (2026-08-21, recorded at execution; temporary until CORE C7).** The
  > `reflect_on` row's "annotated" cell could not be built as written: the `reflect` derive key
  > does not exist until CORE C7 lands it, and the `Component` derive **hard-errors on unknown
  > keys** — *"unknown #[component(...)] key; valid keys: on_add, …"*
  > (`crates/boyko_macros/src/component.rs:775`, verified at G0 by writing the annotation and
  > watching the derive refuse it). What landed instead: `reflect_on.rs` (and its copy
  > `reflect_off_twin_plus.rs`) carries a **direct `#[cfg(feature = "reflect")]`-gated
  > fn-pointer reference to `boyko_reflect::install_type_info`** (`reflect_linkage()`), which
  > puts needle A and needle B in the feature-ON image by the same mechanism the annotation
  > will; `reflect_never.rs` is the same shape **minus that linkage**. **CORE C7 swaps the
  > linkage for the real `#[component(reflect)]` annotation**, and the swap-over is named in
  > both file headers (`src/bin/reflect_on.rs`, `src/bin/reflect_never.rs`) so the deviation
  > cannot silently outlive the rung that retires it. G3 carries the same statement at its leg
  > table, because its "`#[component(reflect)]` in the source?" column is what this deviation
  > temporarily re-carries.

  **`autobins = false` / `autobenches = false` in `crates/reflect_fixture/Cargo.toml` is part
  of this rung's Lands, and it is load-bearing for the third RED, not tidiness — MEASURED at
  execution (2026-08-21):** with Cargo's default target auto-discovery on, deleting the
  `[[bin]] reflect_never` row left the gate **green**, because `src/bin` auto-discovery
  silently re-mints the target from the file path. With discovery off, the reconciled table
  above is the **single** source of truth for targets, and a deleted row is a missing target
  that gate 6's named-target invocation reds.

* `crates/reflect_dogfood/` — package `reflect-dogfood` (D15): the same three deps **plus**
  `boyko-scene` and `boyko-render` (plain, non-optional, **no `features` array on either edge** — D3
  C4), and a leaf umbrella `reflect = ["dep:boyko-reflect", "boyko-scene/reflect",
  "boyko-render/reflect"]`. Empty at this rung except its manifest; `docs/REFLECTION-PLAN-ECS.md`'s
  EG8 fills it. **Lands only if the owner takes B.13 #1** — otherwise this bullet and D15's third row
  are deleted together.
* **`crates/boyko_scene/Cargo.toml` and `crates/boyko_render/Cargo.toml` each gain a non-default
  `reflect = ["dep:boyko-reflect"]` feature plus an `optional = true` dependency edge on
  `boyko-reflect`** (the B.12 option-(b) shape, clean under D3 C1–C4). **Named in Lands at
  execution (2026-08-21): the original list omitted them, but this rung's own gate 2 —
  `cargo check -p reflect-dogfood --all-targets --features reflect` — cannot resolve without
  them**: the dogfood umbrella forwards `boyko-scene/reflect` and `boyko-render/reflect`, and a
  forward to a feature no manifest declares is a hard resolver error. So the edits are G0's by
  necessity, not C7's by convenience; only the *features and edges* land here — the gated
  `#[cfg(feature = "reflect")]` bodies arrive with CORE C7/C8. *(Consequence for CORE C0's
  second RED, recorded where it is caused: `boyko_scene` declares `reflect` from G0 onward, so
  a `#[cfg(feature = "reflect")]` canary in `boyko_scene` is a KNOWN cfg — the
  `unexpected_cfgs` red that mutation expects cannot fire unless the mutation also removes this
  feature. The re-specification belongs to CORE.)*
* All three members added to `[workspace] members` **and** `default-members` in the root `Cargo.toml`
  (omitting the second is the 2026-07 vacuity).
* The three census rows D15 requires.
* **The ship-target list is named here and nowhere else** (D2): `boyko_demo` and the root
  `boyko-engine`. **Not `boyko_app`** — it is `[lib]`-only (`crates/boyko_app/Cargo.toml:13`) and has
  no artifact to census.

**Naming.** Directories are `snake_case`, package names follow each crate's own manifest — this
workspace does not spell them uniformly, and `tests/engine_packages_census.rs` exists because rung 1
of the profiling campaign assumed it did. `boyko-reflect`, `reflect-fixture` and `reflect-dogfood` are
hyphenated, like their nearest neighbours `boyko-serialize` and `profile-fixture`.

**Gate.**

1. `cargo check -p boyko-reflect -p reflect-fixture -p reflect-dogfood --all-targets` — green.
2. `cargo check -p reflect-fixture --all-targets --features reflect` — green.
   `cargo check -p reflect-dogfood --all-targets --features reflect` — green (this is also the first
   compile that proves an engine crate's `reflect` feature resolves at all, D3).
3. `cargo test -p boyko-engine --test engine_packages_census` — green **only** with the D15 rows.
4. `cargo clippy -p boyko-reflect -p reflect-fixture -p reflect-dogfood --all-targets -- -D warnings`
   — the new members must be inside the workspace lint policy. Before the 2026-07 audit only
   `boyko_threadpool` opted in, so "workspace-wide" was not.
5. If any of the three needs a banned type (`HashMap`/`Mutex`/…), a row in
   `docs/HOT-PATH-EXCEPTIONS.md` with a frequency class from the closed vocabulary, or
   `scripts/check_hotpath_exceptions.py` reds. Blanket `#[allow]` on a `mod` is forbidden by that
   script, not by taste.
6. **The four bins and the bench exist and build**, each in both feature states where that is
   meaningful — because a target required by a later rung and created by no rung is this campaign's
   most-repeated defect (twelve benches in a gate table, none of which existed).

**RED MUTATION.** Delete the `reflect-fixture` row from `USER_PACKAGES` in
`tests/engine_packages_census.rs` ⇒ the census reds naming the package. **This red costs nothing to
run and is the first one in the ladder**: the gate already exists, so the rung's first act is to watch
an existing gate fail on the rung's own change.

**Second RED.** Add the members to `members` but not to `default-members` ⇒ a bare root
`cargo check --all-targets` stops covering them while still reporting success. Recorded because it is
the shape the 2026-07 audit found across the entire CI.

**Third RED.** Delete the `[[bin]] reflect_never` target ⇒ gate 6 reds naming it, and G3's L3 and
G7a's `T` lose their subject. Run it: it is the mutation that reproduces, on demand, the exact defect
this rung's target table was added to fix.

> **Amended at execution (2026-08-21): as first written this RED could not fire — MEASURED.**
> Deleting the `[[bin]] reflect_never` row left the gate **green**: Cargo's `src/bin`
> auto-discovery silently re-minted the target from the file path, so the row's deletion changed
> nothing the gate could see. The fix is `autobins = false` / `autobenches = false` in
> `crates/reflect_fixture/Cargo.toml` (now named in this rung's Lands, with the reason); with
> discovery off, the same mutation reds as specified. A RED that reads green is the exact defect
> class this ladder exists to catch — found in the ladder's own first rung, by running it.

---

### G1 — the manifest census: `reflect` features are leaves (P-A) — **size S**

> **Landed deviation (2026-08-21, recorded at execution).** Clause **C6** below asserts
> against `tests/reflect_ci_coverage.rs`'s named list — a file this plan lands at **G4** —
> and its subject is non-empty from G0 onward: this document's own gate lines and G4 job
> specifications are command lines that enable a `reflect` feature, and the non-vacuity
> clause demands the scan find ≥ 1. A C6 with no reference list reds G1 for a reason G4
> owns; a C6 that skipped a missing list would be a gate that cannot fail. So the **named
> list half** of `tests/reflect_ci_coverage.rs` (the `NAMED_ENABLING_SPECS` rows between
> `BEGIN/END REFLECT ENABLING SPECS` delimiters, plus a list-integrity test) lands **at
> G1 by necessity** — the same shape as G0's scene/render feature edits — and G4 adds the
> file's ci.yml-parsing half exactly as written. Rows are normalized `--features` SPECS
> (the token after `--features`, one comma-part at a time), because the found set includes
> prose-embedded command lines whose exact argv equality would be brittle; G1's census
> asserts found ⊆ named, and G4's half asserts the leg-shaped rows are real CI legs.

**Lands.** `tests/reflect_manifest_census.rs`, in the **root package** for the reason
`tests/internal_docs_anchors.rs` and `tests/engine_packages_census.rs` both state: `CARGO_MANIFEST_DIR`
**is** the repository root there, so no `../..` walking can point the scan at the wrong tree, and the
root package has effectively no dependencies, so the gate needs no engine build.

It reads `cargo metadata --format-version 1 --no-deps`'s **packages** half — the manifests as written,
which is invocation-independent — and asserts **D3's six clauses** over every workspace member. Each
entry in that half carries `kind` (null / `dev` / `build`), `optional`, `features` and
`uses_default_features` per dependency, plus the package's own `features` map, so every clause below
is decidable without resolving anything:

| clause | assertion |
|---|---|
| **C1 — never default** | no member's `default` feature list transitively enables a `reflect` feature, and no member's `default` names `boyko-reflect`. |
| **C2 — optional edges only** | every dependency edge naming `boyko-reflect` has `optional == true`. |
| **C3 — `dep:` discipline** | a feature that pulls the crate is defined as `["dep:boyko-reflect", …]` exactly — the bare `["boyko-reflect"]` form implicitly creates a *second*, differently-named feature and is the way a consumer silently gets an always-on optional dependency. |
| **C4 — no edge enables it** | no dependency edge of **any** kind lists `reflect` or `<pkg>/reflect` in its `features` array. An edge is the one enabler nothing can switch off: `crates/boyko_ecs/Cargo.toml` records that *"an explicit `features = [...]` survives `--no-default-features` by design"*. |
| **C5 — ship targets are clean** | no ship-target member (`boyko_demo`, root `boyko-engine`) declares **or forwards** a `reflect` feature. Forwarding is `reflect = ["<pkg>/reflect", …]` in the member's own `[features]` table. |
| **C6 — every enabling invocation is a leg** | every command line under `.github/`, `scripts/` and this document that enables a `reflect` feature appears in `tests/reflect_ci_coverage.rs`'s named list. |

**The failure message names the decision, not only the clause.** A C5 or C4 red prints the B.13 #1
row it enforces and the sentence *"engine crates MAY declare `reflect` (`REFLECTION-ANALYSIS.md`
B.12); what they may not do is let anything enable it — this is that rule, not the older leaf rule"*.
Without that, the next person to hit it "fixes" it by deleting the feature from `boyko_scene` and
silently deletes the dogfood.

**Non-vacuity (mandatory, not optional).** The scan must assert it found **≥ 1 member declaring
`reflect`**, **≥ 1 optional edge to `boyko-reflect`**, **≥ 1 leaf umbrella forwarding `<pkg>/reflect`**
(once B.13 #1 is taken), and **≥ 1 named enabling invocation**. Without it, deleting `reflect-fixture`
leaves a green gate certifying nothing — *"a check whose subject can vanish while the check stays
green is not a check"* (`tests/trybuild_corpus_compiler_witness.rs`, the corpus-non-emptiness test).

**Why this clause set and not "just run `cargo tree`".** C1/C4/C5 are the *only* instruments in this
plan that see the §2 footgun **before** it fires. `cargo tree` reports a closure; these report that no
closure *could* contain it. The measurement that makes them necessary is in this tree already: while
`profiling-analysis` was default-on, no command line could turn it off, because unification restored
it through nine sibling manifests (`crates/boyko_ecs/Cargo.toml`). A gate that only reads closures
would have reported that configuration green until someone tried the command line.

**RED MUTATION (five, all cheap).**

* **R1a** — add `default = ["reflect"]` to `crates/reflect_fixture/Cargo.toml` ⇒ **C1** reds. *This is
  the `profiling-analysis` failure, mechanically caught.*
* **R1b** — flip `reflect-fixture`'s `boyko-reflect` edge to non-optional ⇒ **C2** reds.
* **R1c** — write the feature as `reflect = ["boyko-reflect"]` ⇒ **C3** reds. Run it, because this is
  the mutation a human makes by accident.
* **R1d** — write `boyko-scene = { path = "../boyko_scene", features = ["reflect"] }` on
  `reflect-dogfood`'s edge instead of using its umbrella ⇒ **C4** reds. **This is the mutation that
  matters most**, because it is the *convenient* way to wire the dogfood and it is the one form
  nothing downstream can turn off.
* **R1e** — add `reflect = ["boyko-render/reflect"]` to `crates/boyko_demo/Cargo.toml` ⇒ **C5** reds
  naming the ship target. *This is the old C1's property, preserved exactly where it can fire.*

> **Measured at execution (2026-08-21): R1b and R1e as written red BEFORE the census.**
> Both mutations produce a manifest cargo refuses to load — R1b because a `dep:` reference
> to a non-optional dependency is a manifest parse error, R1e because `boyko-render` is not
> a dependency of `boyko_demo` — so every cargo command in the repository reds, the census
> included (it cannot even spawn `cargo metadata`). That is a red, but it is the build
> system's, not the clause's, and a clause whose only observed red is upstream of it is a
> clause nobody has seen fail. Each was therefore run a second time in the nearest form
> that keeps the manifest loadable: R1b with the `dep:` reference also removed
> (`reflect = []`) ⇒ **C2's own assertion** fires naming the edge; R1e as the declare-half
> `reflect = []` on `boyko_demo` ⇒ **C5's own assertion** fires naming the ship target
> (the forward-half cannot be built loadably on `boyko_demo` today: it has no dependency
> carrying a `reflect` feature, and a forward to a real dep's undeclared feature —
> `boyko-ecs/reflect` — reds the resolver instead, *"does not have that feature"*). Both
> observations are in the ledger. The as-written forms remain listed because their red is
> real and cheap; the supplementary forms are what prove the clauses can fire.

---

### G2 — the resolver gate on the named invocation (P-B) — **size S**

**Lands.** `tests/reflect_ship_closure.rs` (root package). For each ship target of D2 it runs

```
cargo tree -p <ship> -e features --edges normal,build --format "{p} {f}"
```

and asserts the output contains no `boyko-reflect` and no enabled `reflect` feature.

**Three things about that command line, each of which is a decision.**

1. **No `--workspace`.** The property is per-invocation. `cargo tree --workspace -e features
   --no-default-features` is measured in-tree to report a feature ENABLED that the flags asked to
   disable (`crates/boyko_ecs/Cargo.toml`), because a workspace invocation unifies. Asking the wrong
   question loudly is worse than not asking.
2. **`--edges normal,build`.** Dev-dependencies do not ship. Including them would red on a fixture's
   own dev edge and the gate would be relaxed to silence it — the standard way a gate becomes
   decorative.
3. **`-e features`**, not the default: the plain form shows packages, and the failure mode being
   watched is a *feature* that pulls a package in.

**The positive control runs every time, not once.** The same helper is pointed at
`-p reflect-fixture --features reflect` and must **find** `boyko-reflect`. Without it, a typo in the
needle, a changed `--format`, or a `cargo tree` that failed and returned empty output all read exactly
like a pass. This is B.6's *present control* discipline applied to the resolver instead of the linker.

**Fourth thing about the command line, added because D15 creates the exposure: the harness asserts
its own invocation carried no `--features`.** With engine crates permitted a `reflect` feature (D3),
there is now a legitimate workspace invocation that turns it on for *every selected member* —
`cargo test -p reflect-dogfood --features reflect-dogfood/reflect`, which reaches `boyko_demo` because
a bare root build selects every member (F16). That invocation is **correct** and is not a leak; what
would be a defect is reading a ship closure out of it. So the harness reads `CARGO_ENCODED_ARGS` (or,
portably, re-spawns its own `cargo tree` with an explicitly empty feature selection and
`--no-default-features` **absent**) and reds with *"the ship-closure gate was invoked under a feature
selection; the number below is not a ship closure"* rather than reporting a green. This is the
`g14b_the_shipping_build_still_runs_its_always_tier` clause — *"the build did not use the profile this
test asked for"* — applied to a feature selection instead of a profile.

> **Measured at execution (2026-08-21): the env form is unbuildable, so the portable form
> landed — and it closes the exposure by construction rather than by detection.**
> `CARGO_ENCODED_ARGS` is not among the variables cargo sets for a test process, and the
> full `CARGO*` environment of the harness binary is **byte-identical** under
> `cargo test -p boyko-engine -p reflect-dogfood --features reflect-dogfood/reflect`
> versus the plain invocation (empirically diffed: NO-DIFF). An outer feature selection is
> therefore unobservable from inside the harness, and a guard claiming to detect it would
> be a gate that cannot fail. What landed: the harness spawns its own `cargo tree` with an
> explicitly constructed argv, and a purity assertion refuses any feature-selecting token
> in **the argv that is spawned** (one construction, one assertion, one spawn) before a
> ship reading is taken. Consequence for the reading: the parent's `--features` cannot
> leak into a fresh `cargo tree` process, so the number this gate reports is **always** a
> ship closure — the outer invocation the ledger names runs green *and truthfully*. The
> guard's RED is the harness-side mutation (route a feature flag into the ship reading ⇒
> the purity assertion reds with this decision's sentence), and both observations are in
> the ledger. One more measured note: the needle match is on **parsed package names**,
> never raw substrings — this worktree's own path contains both `reflect` and
> `boyko_reflect`, so a substring needle would red clean trees and would hold the second
> RED green for the wrong reason.

**Cost, stated rather than hidden.** `cargo tree` resolves; it does not build. Three invocations,
seconds. There is no reason for this gate to live anywhere but the ordinary `cargo test --workspace`
sweep.

**RED MUTATION.** Add `boyko-reflect = { path = "../boyko_reflect", optional = true }` plus
`default = ["reflect"]` to `crates/boyko_demo/Cargo.toml` ⇒ the `boyko_demo` clause reds. Revert.
**Second RED:** break the needle (search for `boyko_reflect` with an underscore, which is the *lib*
name and not the *package* name `cargo tree` prints) ⇒ the positive control reds while the ship
clauses stay green — the exact shape of a gate that passes for the wrong reason, caught by its own
control.

---

### G3 — the artifact census: three legs, two needles, and the link configuration MEASURED (P-C) — **size M**

**Lands.** `crates/reflect_fixture/tests/reflect_absence_census.rs`, built on
`crates/profile_fixture/tests/profile_axis_census.rs` — **the template, cited and copied, not
rediscovered**. It builds its own legs (`Command::new(env!("CARGO"))` with `--config
profile.release.lto="fat"` and `--config profile.release.codegen-units=1`, a per-leg
`CARGO_TARGET_DIR` under the system temp dir, `RUSTFLAGS` removed because an inherited
`-C embed-bitcode=no` is incompatible with `-C lto`), so it is runnable locally.

> **Log-scraper caveat, permanent (G0, 2026-08-21).** The mandated shared-source twin —
> `[[bin]] reflect_off_twin` pointing at `src/bin/reflect_on.rs` — makes **every** build of
> `reflect-fixture` print Cargo's notice *"warning: `…\crates\reflect_fixture\Cargo.toml`: file
> `…\src\bin\reflect_on.rs` found to be present in multiple build targets: \* `bin` target
> `reflect_on` \* `bin` target `reflect_off_twin`"*. It is a **Cargo** notice, not a rustc
> lint: it cannot red `-D warnings`, and no flag silences it. This harness spawns its own
> `cargo` builds — its log scraping must **not** treat that line (or the bare presence of
> `warning:` in build output) as a failure; assert on the specific diagnostics each leg names.

**The three legs.** Two are the obvious ones; the third is the one that decides whether the other two
mean anything.

> **L3's fixture is corrected, 2026-08-21 (second pass) — as first written it could not be built, and
> what it *would* have measured is a false statement about the instrument.** The table gave L3 the
> `reflect_off_twin` source with the feature **on** and the opt-in "absent". Those two cells cannot
> both hold: `reflect_off_twin` **is** the annotated twin (G6c defines it against `reflect_never`, *"a
> byte-identical source minus every opt-in"*), so building it with the feature on makes the opt-in
> **present** and L3 collapses into L2 under a different name. It would then read > 0 for the trivial
> reason that the annotation is present, and the implementer would write down *"the census can only
> see is-the-crate-in-the-graph; L1's zero is earned by the resolver"* — **a wrong conclusion about
> the instrument, produced by the instrument, in the gate this plan's own preamble calls "the
> mechanism".** A wrong conclusion recorded as a measurement is worse than a missing leg.
>
> The leg L3 was written for needs a source with **no** opt-in built with the feature **on**: the
> crate is then resolved, compiled and linked (the feature adds the `dep:` edge regardless of whether
> any code names it) and **nothing calls it**. That fixture already exists in this plan — it is
> `reflect_never`, which G6c defines and G0 now builds. One word.
>
> *(A note on spelling, since it is what made the collision easy to miss: the opt-in is a **key inside
> an existing attribute** — `#[component(reflect)]`, `docs/REFLECTION-PLAN-CORE.md`'s D3 — not a
> free-standing `#[reflect]`. This plan said `#[reflect]` throughout; the tables below say what the
> source actually contains, and G7a's twin-identity check depends on the difference.)*

| leg | fixture bin | `reflect` feature | `#[component(reflect)]` in the source? | the cell's role |
|---|---|---|---|---|
| **L1** `off` | `reflect_off_twin` | off | **present**, and `#[cfg]`-stripped | **the ship cell** — needle A must read 0 |
| **L2** `on` | `reflect_on` | on | **present**, and live | **the present control** — needle A must read > 0, or L1's zero is indistinguishable from "no fixture" |
| **L3** `linked-unused` | **`reflect_never`** | **on** | **absent from the source** | **the instrument's discriminator** — the crate is resolved, compiled and linked, and nothing in the image names it |

> **Until CORE C7 lands the `reflect` key, the "`#[component(reflect)]` in the source?" column
> is carried by G0's landed deviation** (see G0's target-table note): "present" means
> `reflect_on.rs`'s direct `#[cfg(feature = "reflect")]`-gated fn-pointer reference to
> `install_type_info` (`reflect_linkage()`); "absent" means `reflect_never.rs` carries no such
> linkage. The legs and both needles are unchanged — the linkage puts the same symbols in the
> image the annotation will — and until C7 this rung's third RED reads *"delete
> `reflect_linkage()` from `reflect_on`"* rather than *"delete the `reflect` key"*. C7 swaps
> the linkage for the annotation; the swap-over is named in both bin headers.

**L3 is the contribution this rung makes over the analysis.** B.6 establishes that a plain function in
a dependency's rlib is carried into the image whether or not anything reaches it, and that
`--gc-sections` does nothing about it on this target. L3 asks that question *of reflection*:

* If **L3 reads > 0** under fat LTO, the census can only see "is the crate in the graph" — which is
  the resolver's question, already answered better by G1/G2 — and its L1 zero is being earned by the
  resolver. **Record that, in the gate's own failure message**, so no reader infers the wider claim.
* If **L3 reads 0** under fat LTO, the census genuinely distinguishes *reachable* from *linked*, and
  P-C is an independent property rather than a corroboration.

Either outcome is a result. Neither is predicted here.

**The link-configuration table is MEASURED at this rung and pasted back into this file**, exactly as
B.6's was. The implementer fills every cell by running it:

| link configuration | L1 needle A | L2 needle A | L3 needle A | L1 needle B | L3 needle B | decidable? |
|---|---|---|---|---|---|---|
| default release | 0 | 1 | 0 | 0 | 0 | yes — today; see the note |
| `-C link-arg=-Wl,--gc-sections` | 0 | 1 | 0 | 0 | 0 | yes — today; see the note |
| `lto = "fat"`, `codegen-units = 1` | 0 | 1 | 0 | 0 | 0 | yes |

*(Needle A = the crate-name fragment `boyko_reflect`; needle B = `install_type_info`, the plain-fn
LTO probe. D5.)*

> **MEASURED at execution (2026-08-21, this box, `x86_64-pc-windows-gnu`).** Two findings:
>
> * **L3 reads 0 under fat LTO** — the second branch of the interpretation above holds: the
>   census genuinely distinguishes *reachable* from *linked*, and P-C is an independent
>   property. (Verified: L2's single needle-A hit IS needle B's subject —
>   `_RNvCsd7WGKwjPoHP_13boyko_reflect17install_type_info`, the crate's only fn at G0; v0
>   mangling encodes the defining crate.)
> * **Unlike B.6's `mint_cold`, L3 reads 0 in the non-LTO rows too**, and the difference is
>   mechanical, not contradictory: a PE linker pulls an rlib's object only when some
>   undefined symbol resolves into it. `profile_fixture` CALLS into `boyko_diag`, so its
>   object is pulled and `mint_cold` rides along; `reflect_never` references NOTHING in
>   `boyko_reflect`, so its object is never pulled at all. The rule that survives both
>   measurements is *"one referenced symbol pulls the whole object"* — the moment CORE C2
>   gives the crate a real surface and anything references any of it, the non-LTO rows stop
>   being sharp per-symbol instruments. The gate leg therefore stays fat-LTO even though
>   today's table is flat, and the first RED's outcome (below) is expected to change at C2:
>   **re-run the calibration (`measure_link_configuration_table`, `--ignored`) at CORE C2
>   and re-paste this table.**

**Gate.**

1. L1 needle A **== 0**, L1 needle B **== 0**.
2. L2 needle A **> 0** — asserted with a message naming its role, following
   `crates/profile_fixture/tests/profile_axis_census.rs:292`'s wording: an absent control is
   **NOT RESOLVED (census inert)**, never a pass.
3. L3 recorded, and asserted against whichever value the measured table shows, with the gate's message
   carrying the interpretation the table forces.
4. `llvm-nm` absence ⇒ panic (D6).
5. Each fixture binary prints one line reporting its own configuration and the count it believes it
   has; the test asserts the line matches the leg it asked for. This is
   `g14b_the_shipping_build_still_runs_its_always_tier`'s clause — *"the build did not use the profile
   this test asked for … the half that only a harness spawning its own build can check."*

> **Landed notes (2026-08-21, recorded at execution).** (1) Gate 5 requires the fixture
> binaries to print a self-report line, and G0's bins printed nothing — the print
> (`bin=<CARGO_BIN_NAME> reflect_feature=<on/off> linkage=<present/absent/never>`) landed
> at this rung in `reflect_on.rs`, `reflect_never.rs` **and** `reflect_off_twin_plus.rs`
> (the plus-copy mirrors it so G7a's twin comparison stays "the marker and nothing
> else"). (2) The L3 non-collision scan keys on the linkage tokens `reflect_linkage` /
> `boyko_reflect::` and deliberately NOT on `feature = "reflect"` — the self-report line
> legitimately probes `cfg!(feature = "reflect")` in every bin, and a `cfg!` probe can
> put no symbol in an image (found by this rung's own first green run: the broader token
> set red the census on its own gate-5 print). (3) The bin→source mapping for the
> non-collision scan goes through the manifest's `[[bin]]` table (`autobins = false`
> makes it the single source of truth), so the fourth RED cannot be dodged by renaming.

**RED MUTATION.** Drop `--config profile.release.lto="fat"` from the harness's `build()` ⇒ re-read all
six cells and record what happens. B.6's second red, in its own words, is the one that matters more:
*"⇒ every cell reads 1 and the gate can no longer fail at all. That is the state this gate would have
shipped in."*
**Second RED.** Point L1's build at the `reflect_on` fixture ⇒ the ship cell fills. This is the red
that proves the needle names something in the *image*, not merely in the source.
**Third RED.** Delete the `reflect` key from `reflect_on`'s `#[component(…)]` ⇒ **L2** collapses to
zero ⇒ the present control reds. Run it: it is the mutation that turns the whole census inert while
leaving its headline assertion green.
**Fourth RED, and it is L3's own.** Point L3's build at `reflect_off_twin` instead of `reflect_never`
⇒ L3's cell becomes L2's cell, and the leg's non-collision assertion reds. That assertion is a literal
part of the gate: **L3 asserts that its source contains no `reflect` key** before it reports a number,
because a discriminator that silently became a duplicate of the present control is exactly how this
leg would report a false finding about the census's reach.

**What this gate cannot claim** — enumerated, because a gate that does not name its exclusions is the
defect this campaign keeps finding:

* Nothing about a member it does not build. It censuses a fixture (D2), not `boyko_demo`.
* Nothing about **build time or dependency surface**. A crate that is resolved, compiled and linked
  and then stripped reads 0 here and is not absent in the sense the owner means. That is G1/G2's
  question and this gate does not answer it.
* Nothing about metadata that lives in `boyko_ecs` under B.1 Horn 1 (D16).
* Nothing about a **profile a CI leg does not build** — `dev` vs `release` are different images.

---

### G4 — the CI matrix: four cells, the Miri allowlist, and a gate over the workflow file itself — **size M**

**Lands.** In `.github/workflows/ci.yml`:

1. **`reflect-on`** — a new job, `matrix: profile: [debug, release]` (D7):
   ```
   cargo test -p boyko-reflect -p reflect-fixture --all-targets --no-fail-fast \
     --features reflect-fixture/reflect
   ```
   Multi-package selection requires the `pkg/feature` spelling; the bare `--features reflect` is
   ambiguous across selected packages and is the form that silently selects nothing.
2. **`components: llvm-tools`** on a `reflect-census` job running
   `cargo test -p reflect-fixture --test reflect_absence_census`, mirroring `profile-census`
   (`.github/workflows/ci.yml:144-153`).
3. **Miri allowlist** — `.github/workflows/ci.yml:222-226` grows **two rows with different
   shapes**, and the difference is the whole point (D4):

   ```
   cargo +nightly miri test --all-targets \
     -p boyko-ecs -p boyko-utils -p boyko-threadpool -p boyko-serialize \
     -p boyko-math -p boyko_sdf_math -p boyko_image \
     -p boyko-reflect \
     -p reflect-fixture --features reflect-fixture/reflect
   ```

   * `-p boyko-reflect` **PLAIN — no `--features`.** The crate carries no `reflect` feature (D4) and
     never will, so `--features reflect` on it is a hard cargo error; and with the feature "off" it is
     **not empty** — nothing in its source is `cfg`-gated. This row covers the arithmetic, the
     registry and the `prim::` accessors over hand-built pointers.
   * `-p reflect-fixture --features reflect-fixture/reflect` — the **only** row that reaches any
     derive-generated `unsafe`, because only a consumer can carry a `#[component(reflect)]` type. The
     `pkg/feature` spelling is load-bearing: a bare `--features reflect` across a multi-package
     selection is ambiguous and is the form that silently selects nothing.

   **Two guards inside the fixture, or this row reds for reasons unrelated to reflection** (the sweep
   runs `--all-targets`): `reflect_absence_census.rs` (G3), `reflect_codegen_identity.rs` (G7a) and
   the `reflect_optin_cost` bench (G7b) all spawn processes or measure wall clock, and **Miri can do
   neither**. Each carries `#[cfg(not(miri))]`. The template they copy —
   `crates/profile_fixture/tests/profile_axis_census.rs` — carries **no such guard**, verified, and
   does not need one *because `profile_fixture` is not on the allowlist*; copying it verbatim onto the
   allowlist imports the failure, whose likeliest "fix" is dropping `-p reflect-fixture` and silently
   reverting B.9.

   **`reflect-dogfood` is deliberately NOT on the allowlist.** Miri cannot execute FFI — that is why
   the sweep is hand-listed at all (F18) — and the dogfood reaches `boyko_render` →
   `boyko_rhi_vulkan` (`crates/boyko_render/Cargo.toml`). So **the Miri row and the dogfood
   acceptance row are two tests in two packages** (D15), and the fixture's local shapes (a
   `#[component(storage = "dense")]` struct of `[f32; 4]` arrays, a fieldless `#[repr(u8)]` enum, a
   tuple struct) are **the primary subject, not a stand-in**. Whether `boyko-scene` alone
   (`Transform`, `Name`, `Visibility`) is Miri-executable is **decided by running it**, not by reading
   its manifest — it depends on `boyko-input` (`crates/boyko_scene/Cargo.toml`), and the OS input ring
   is FFI. If it turns out to be, a third Miri row may later name a scene-only dogfood; that is an
   outcome, not a plan.
4. **A `reflect-dogfood` job**, outside Miri:
   `cargo test -p reflect-dogfood --all-targets --no-fail-fast --features reflect-dogfood/reflect`.
   It is the only CI leg that compiles an engine crate's `reflect` feature, so it is the leg that
   makes D3's whole permission meaningful — and, per D15, it is also the one invocation in CI where
   `boyko-reflect` legitimately appears in `boyko_demo`'s closure. G2's harness asserts it is not
   itself running under that selection.
5. **`tests/reflect_ci_coverage.rs`** (root package) — parses `.github/workflows/ci.yml` and asserts:
   the `reflect-on` job exists and its command contains `reflect-fixture/reflect`; the
   `reflect-dogfood` job exists and its command contains `reflect-dogfood/reflect`; the Miri sweep
   line names `-p boyko-reflect` **without** a feature flag and `-p reflect-fixture` **with**
   `reflect-fixture/reflect`, and does **not** name `reflect-dogfood`; the census job requests
   `llvm-tools`. It is also **D3 C6's other half**: the set of enabling command lines it names must
   equal the set G1's manifest census found. **This is the gate against B.9's exact failure mode** —
   a mandatory Miri clause that is mandatory in prose and covered by nothing.

**Non-vacuity inside each leg, not only around it.** Every reflect test asserts
`cfg!(feature = "reflect")` at its head and reds with a message naming the leg if it is false;
`gj1_flag_cost`'s `flags_on()` is the precedent (*"leg A asked for the profiler and did not get it …
every number below would be the logger's alone under a joint name"*). Without this, dropping
`--features` from the job turns every reflect test into a no-op that reports green.

> **Landed shape (2026-08-21, recorded at execution): the assertion is armed by a leg
> marker, `BOYKO_REFLECT_LEG`.** A test that asserts `cfg!(feature = "reflect")`
> unconditionally reds every plain workspace sweep — feature-off is the SHIP
> configuration there and correct — and the reverse (detecting the job's feature
> selection from inside the process) is impossible: G2's measurement, an outer
> `--features` is unobservable from a test binary. So the job sets
> `BOYKO_REFLECT_LEG: reflect-on` beside its `--features`, and
> `crates/reflect_fixture/tests/reflect_leg_nonvacuity.rs` asserts the feature whenever
> the variable names a leg, redding WITH the leg's name. `tests/reflect_ci_coverage.rs`
> pins the flag and the variable together in the job, so the pair cannot drift. The
> first RED below was run in both layers: the workflow layer (the coverage test) and
> the leg simulation (`BOYKO_REFLECT_LEG=reflect-on cargo test -p boyko-reflect
> -p reflect-fixture --all-targets --no-fail-fast` without the flag).

**RED MUTATION.** Remove `--features reflect-fixture/reflect` from the `reflect-on` job ⇒ the per-test
non-vacuity assertion reds. **Second RED:** remove `-p reflect-fixture` from the Miri sweep ⇒
`tests/reflect_ci_coverage.rs` reds. **Third RED:** and the one to run because it is the tempting
convenience — add `reflect` to `reflect-fixture`'s `default` list so the job's `--features` becomes
unnecessary ⇒ **G1 C1** reds. **Fourth RED, and it is the one that proves the corrected Miri wording
rather than asserting it:** write the sweep as `-p boyko-reflect --features reflect` ⇒ the job fails
before compiling anything, with cargo's *"none of the selected packages contains these features"*.
Record the exact message in the ledger — it is the evidence that the sentence four sibling documents
inherited from B.9 was unrunnable. **Fifth RED:** delete `#[cfg(not(miri))]` from
`reflect_absence_census.rs` ⇒ the Miri leg reds on a process spawn, for a reason that has nothing to
do with reflection. Run it, so the guard's absence is a red somebody has *seen* rather than a comment
somebody wrote. Five gates, five independent mutations, five reds.

---

### G5 — the derive's refusals: one trybuild corpus, two legs — **size M** *(prerequisite: `docs/REFLECTION-PLAN-CORE.md`'s derive)*

**Lands.** `crates/reflect_fixture/tests/reflect_compile_fail.rs` +
`crates/reflect_fixture/tests/reflect_compile_fail/*.rs` + blessed `.stderr`, following
`crates/boyko_ecs/tests/compile_fail_zero_init.rs`'s shape exactly (including its `#[cfg(not(miri))]`
guard — trybuild spawns cargo, which Miri cannot).

```rust
#[cfg(all(feature = "reflect", not(miri)))]
#[test]
fn ui_rejections() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/reflect_compile_fail/*.rs");
}

#[cfg(all(not(feature = "reflect"), not(miri)))]
#[test]
fn ui_the_same_fixtures_compile_with_the_feature_off() {
    let t = trybuild::TestCases::new();
    t.pass("tests/reflect_compile_fail/*.rs");   // D12 — the SAME directory
}
```

**The corpus.** One fixture per refusal the design states, each named for the rule rather than the
symptom. The list is taken from §5, A.2/A.6, B.4 and B.5 and is not invented here:

| fixture | the rule | source |
|---|---|---|
| `generic_component_rejected` | a per-impl `static TYPE_INFO` collapses across monomorphizations | §5 |
| `repr_packed_rejected` | taking `&field` on a packed type is UB | §5 |
| `bitset_storage_rejected` | no per-row bytes; the presence view substitutes | §4 M6, B.4(1) |
| `fieldless_enum_without_repr_rejected` | a missing `repr` is a **compile error**, not a silent `Opaque` | A.9 (FIX Mi3) |
| `data_carrying_enum_rejected` | no Reference-guaranteed variant-field layout | A.1 v2 |
| `option_field_rejected` | niche optimization ⇒ no guaranteed discriminant location; **not "cheap enough"** | A.1 v2 |
| `vec_field_rejected` | `Opaque` in a serialized type is a hard error, not a silent omission | A.6 |
| `vec_field_skip_accepted` | …**and the refusal is opt-out-able** — `#[reflect(skip)]` compiles | A.6 correction |
| `union_rejected` | not a struct or enum | v1 scope |

Two of those rows are load-bearing in a way the others are not. `vec_field_skip_accepted` is a
`t.pass()` fixture **inside a compile_fail directory**, so it belongs in a sibling `pass` directory —
the point it makes is that A.6's *"refuses to serialize"* must be **documented, spanned and
opt-out-able**, not a surprise at first `#[component(reflect)]`. And `bitset_storage_rejected` has a twin
elsewhere: B.5 records that an Aether user writes `tag Foo(bitset);` and would otherwise *"get an
error about a derive they never typed"*, so the **spanned** version of this refusal is an
`aether_tests` trybuild fixture (`crates/aether_tests/Cargo.toml` already carries `trybuild = "1"`)
and belongs to `docs/REFLECTION-PLAN-CORE.md`'s Aether item. Named here so the two are not built
twice.

**Gate.**

1. Feature on: every fixture fails with its blessed `.stderr`.
2. Feature off: every fixture **compiles**.
3. `cargo test -p boyko-engine --test trybuild_corpus_compiler_witness` — green, i.e. the corpus was
   blessed under `rustc 1.97.1 (8bab26f4f 2026-07-14)` (D13).
4. Every diagnostic **names the rule and the way out**, not merely the failure. The precedent is
   `g16c`'s assertion that a refusal's stderr contains the rule it enforces: *"a build that fails for
   an unnamed reason teaches the operator nothing."*

**RED MUTATION.** Delete one refusal from the derive ⇒ its fixture compiles ⇒ trybuild reds with
*"expected compile failure"*. **Second RED, and it is the one that proves the corpus is
feature-sensitive:** run the `compile_fail` harness with the feature **off** ⇒ every fixture compiles
⇒ the harness reds on all nine at once. That red is what shows the corpus is testing the derive rather
than testing rustc. **Third RED:** re-bless one `.stderr` under a shadowing chocolatey `rustc` 1.95.0
⇒ `trybuild_corpus_compiler_witness` reds naming both compilers (D13).

---

### G6 — `cfg_attr`-off compiles to nothing: three levels, and only the third is about codegen — **size S**

**Lands.** Three assertions, deliberately at three different altitudes, because the analysis's claim
("zero tokens, zero `boyko_reflect::` paths, nothing to resolve" — §2) is three claims.

**G6a — the compile refusal, which is the mechanism, not a test.** `reflect_off_twin` is built with
`reflect` **off**, so `boyko_reflect` is not in its dependency graph at all. Any `boyko_reflect::…`
path the derive emits is a hard `E0432`/`E0433`. **The fixture compiling *is* the proof** — this is
the one place in the whole plan where the compiler is the gate, and it is the strongest of the three
because it cannot be satisfied by accident.

**G6b — the token census.** A `#[test]` in `reflect-fixture` compiled with the feature **off** asserts
that the expansion of `#[derive(Component)] #[component(reflect)] struct …` contains no
`boyko_reflect` token. In practice: `cargo rustc -p reflect-fixture --bin reflect_off_twin --
-Zunpretty=expanded` is **nightly**, so the stable form is a `trybuild`-adjacent snapshot — or,
cheaper and stable, an assertion that the symbol `<T as Reflect>` is not nameable, i.e. a
`compile_fail` fixture whose body says
`let _ = <MyComp as boyko_reflect::Reflect>::TYPE_INFO;`. **Which form lands is an implementation
choice with a stated criterion:** whichever one has a RED that a person can run on this toolchain
without nightly. Record the choice and the reason in the ledger.

> **This sub-rung is the single owner of the token-census question, and one sibling was restating it
> without its toolchain caveat.** `docs/REFLECTION-PLAN-CORE.md`'s C8 gate 3 said *"the expansion of a
> `#[component(reflect)]` type contains ZERO occurrences of the string `boyko_reflect`. Measured, not
> asserted — the rung records the count"*, and a count of an *expansion* has exactly one stable-Rust
> route: not `-Zunpretty=expanded`. C8 gate 3 now **defers to G6b** and inherits whichever form G6b's
> criterion selects, rather than specifying a measurement that cannot be taken on the mandated
> toolchain. C8's RED (the count going 0 → non-zero) survives the substitution: under the
> `compile_fail` form the same mutation makes the fixture *compile*, which is the same red wearing a
> different instrument.

**G6c — codegen identity against a never-annotated twin.** The measurand of D9, the machinery of G7.
`reflect_off_twin` (annotated, feature off) versus `reflect_never` (the same source *minus* the
`reflect` key) must emit the **same symbol multiset and the same `.text` size**. This is
the only one of the three that catches a residue which compiles *and* names nothing —
e.g. a `const IS_REFLECT: bool = false` plus an `if Self::IS_REFLECT { … }` in the `component_id()`
funnel that failed to const-fold, or a `#[used]` static that survives.

> **The twin needs a gate of its own, and without it this instrument reports a false positive on its
> headline claim.** "A byte-identical source minus the opt-in" is, as specified, **two hand-copied
> files with nothing checking the relationship**. Any unrelated drift between them — a field renamed
> in one, an `#[inline]` added, a stray `use` — makes G7a clause 3 fire **RESOLVED — RESIDUE** and
> blame the derive for an edit nobody made. That is the instrument accusing its subject of the
> instrument's own defect, on the one claim the campaign exists to establish.
>
> **The gate, and it is cheap.** The two bins share their component definitions through **one** file,
> and differ in exactly one token:
>
> ```rust
> // crates/reflect_fixture/src/twin_body.rs   — the ONE source
> macro_rules! twin_components { ($($extra:meta),*) => {
>     #[derive(::boyko_macros::Component, Default)] $(#[$extra])*
>     pub struct TwinPod { pub a: u32, pub b: f32 }
>     // … the dense/array, enum and tuple-struct shapes, all once
> }; }
> ```
> `reflect_off_twin.rs` writes `twin_components!(component(reflect));`; `reflect_never.rs` writes
> `twin_components!();`; `reflect_off_twin_plus.rs` writes the former plus its one
> `#[inline(never)] fn`. **There is then no second copy to drift.**
>
> **Fallback, with the same stated criterion G6b uses:** if routing the opt-in through a
> `macro_rules!` changes what the derive sees in a way that makes the comparison less faithful than a
> plain item (measure it — it is one `cargo expand`-free A/B of the two symbol multisets against
> hand-written twins), keep two files and add
> `crates/reflect_fixture/tests/twin_source_identity.rs`: read both, delete every occurrence of the
> `reflect` key, assert the remainders are **byte-identical**. Note the deletion is of a **key inside
> `#[component(…)]`**, not of a whole line — a line-wise filter would also delete `no_bundle`,
> `storage = "dense"` and every hook path that shares the attribute, and would pass while the twins
> genuinely differed.
>
> **RED for whichever form lands:** add one unrelated field to `reflect_never`'s copy (or to the
> shared body under a stray `cfg`) ⇒ the identity gate reds **before** G7a runs, naming the file and
> the token. Without this rung, that same edit makes G7a print `RESOLVED — RESIDUE` and name a symbol,
> which a reader would record as a finding about the derive.

**Gate.** All three, each with its own message. G6a's message must say that a compile failure here is
the *expected* result of a broken `#[cfg]` and points at the derive, not at the fixture.

**RED MUTATION.** Remove the `#[cfg(feature = "reflect")]` wrapper from the derive's reflect slot ⇒
**G6a** fails to compile with `E0433`, naming `boyko_reflect`. **Second RED:** make the reflect slot
emit `#[used] static _REFLECT_KEEPALIVE: u8 = 0;` **unconditionally** ⇒ G6a still compiles (the token
names nothing external) and **G6c** reds on the symbol multiset. Run both: they are the two halves of
"compiles to nothing", and only the second one is about codegen.

---

### G7 — the hot-path cost proof — **size M**

The rung that turns §8's *"the compile-boundary hot-path proof ('the symbol does not exist') — stronger
than the 14a/14b runtime-branch discipline"* from a claim into a measurement.

#### G7a — the codegen-identity instrument (primary)

**Lands.** `crates/reflect_fixture/tests/reflect_codegen_identity.rs`. It builds **four** images with
identical link configuration (fat LTO, `codegen-units = 1`, own `CARGO_TARGET_DIR`, `RUSTFLAGS`
cleared) and compares symbol multisets and `.text` sizes:

| image | source | feature | role |
|---|---|---|---|
| `N1` | `reflect_off_twin` | off | the **determinism null**, build 1 |
| `N2` | `reflect_off_twin` | off | the **determinism null**, build 2 — must equal `N1` |
| `T` | `reflect_never` | off | the **twin**: the same source minus the `reflect` key — **and G6c's identity gate is its precondition** |
| `P` | `reflect_off_twin_plus` | off | the **positive control**: `reflect_off_twin` plus one `#[inline(never)] fn` |

All four bins are created by **G0**, whose target table is the reconciled list (G0 previously landed
two of them and G3/G6c/G7a required four).

> **Log-scraper caveat, permanent (G0, 2026-08-21) — same as G3's.** The shared-source twin
> (`[[bin]] reflect_off_twin` → `src/bin/reflect_on.rs`) makes every `reflect-fixture` build
> print Cargo's *"file … found to be present in multiple build targets"* notice. It is a Cargo
> notice, not a rustc lint — it cannot red `-D warnings` — and this harness builds all four
> images itself, so its log scraping must not treat that line (or bare `warning:` presence) as
> a failure.

**The verdict vocabulary, and the order the clauses run in** — copied from `gj1_flag_cost`'s tail,
because the order is the whole discipline:

0. **G6c's twin-source identity gate is red** ⇒ **NOT MEASURABLE (twin)**: the two sources differ by
   something other than the opt-in, so clause 3 below would name the derive for an edit nobody made.
   Report the differing token. This clause runs **first**, before determinism, because a broken twin
   makes the *strongest-sounding* verdict the wrong one.
1. `N1 != N2` ⇒ **NOT MEASURABLE (instrument)**: the build is not deterministic under this
   configuration, so no equality below means anything. Report the diff. Do **not** report a verdict.
2. `P == N1` ⇒ **NOT RESOLVED (control inert)**: the instrument cannot see a one-function difference,
   so it could not have seen a reflection residue either. This is D10's positive control and it is the
   clause that stops "the sets are equal" from being an equality between two empty sets.
3. `T != N1` ⇒ **RESOLVED — RESIDUE**: the `#[component(reflect)]` opt-in changed the emitted code with the
   feature off. Print the symbol difference. This is a **failure of the central claim**, not of the
   instrument.
4. otherwise ⇒ **RESOLVED — the off-arm emits nothing**, with the measured `.text` size and symbol
   count printed so the number is a reading and not a boast.

**The measurand is decided by measurement (D9).** The same run records whether `N1` and `N2` are
byte-identical. If they are, a fifth clause tightens to raw bytes and the tightening goes in the
ledger with the date. If they are not, the reason is printed (on `x86_64-pc-windows-gnu` the first
suspect is the PE link timestamp) and the multiset measurand stands **with that reason recorded**, so
the next reader does not re-open it.

**Cost, stated rather than hidden.** Four LTO builds of a 4-crate graph plus four `llvm-nm` runs. The
comparable in-tree instrument reports *"~20 s on this box"*
(`crates/profile_fixture/tests/profile_axis_census.rs`, its Cost section). The reflect fixture's graph
is larger because it pulls `boyko-ecs`; the actual figure is **MEASURED at this rung and written into
the file's header**, because a cost nobody measured is how a gate ends up in a nightly.

#### G7b — the in-sitting timing half (secondary, ON leg only)

**Lands.** `crates/reflect_fixture/benches/reflect_optin_cost.rs`. **Feature ON**, one process, two
legs: a component carrying `#[component(reflect)]` against a twin that does not, through the same
spawn/query/`component_id()` path. ABBA-counterbalanced with the A-vs-A twin as the sitting's floor,
per `crates/boyko_ecs/benches/gj1_flag_cost.rs`; the spread floor is the **clock's resolution**, never
a fraction of the reading (`crates/boyko_log/benches/instrument.rs`'s rule).

**Two things it must separate, because §2 says they are separate:** the **first-touch** `TYPE_INFO`
registration is real cold cost and *"must not be mistaken for a hot-path regression"*; the
**steady-state** query/spawn inner loop is the subject. So the registration happens outside the timed
block, and a second, explicitly-labelled reading reports the first-touch cost on its own.

**It must compile with the feature OFF.** `.github/workflows/ci.yml:176` runs
`cargo bench --workspace --no-run`, so a bench that only compiles with a feature reds a job that has
nothing to do with reflection. Feature off, `main()` prints `NOT BUILT (feature off)` and returns —
and the `reflect-on` job asserts the *printed verdict token* is present, so the vacuous form cannot
be mistaken for a run.

**What G7b may not be reported as (D11):** a measurement of the OFF configuration. That is G7a's.

**RED MUTATION (G7a).** Build the four images with `codegen-units = 16` ⇒ clause 1 fires,
**NOT MEASURABLE**. **Second RED:** delete the extra function from the positive control ⇒ clause 2
fires, **NOT RESOLVED (control inert)** — the instrument reporting that it cannot see. **Third RED:**
make the derive's reflect slot emit one un-`cfg`'d item ⇒ clause 3 fires, **RESOLVED — RESIDUE**, and
prints which symbol. **Fourth RED, and it is the one that separates the instrument's defects from its
subject's:** add one unrelated field to the twin's side ⇒ **clause 0** fires, **NOT MEASURABLE
(twin)**, naming the token. Run it and record what clause 3 *would* have printed instead — that
sentence is the false finding this clause exists to prevent, and it is worth having in the ledger in
its own words.
**RED MUTATION (G7b).** Delete the runtime `component_id()` install gate so both legs run the same
code ⇒ the pair stops resolving. Note the shape of the trap `gj1_flag_cost` recorded the hard way: run
against a body with **two** gates, that red only half fires and the pair goes on resolving — so
G7b's body has exactly one subject, and the bench asserts it.

---

### G8 — the RED ledger and its completeness check — **size S**

**Lands.** Appendix GB below, plus `tests/reflect_red_ledger.rs` (root package):

* it extracts every rung id `G0`…`G8` from the `## 3. Rung ladder` headings of this file;
* it extracts every ledger row id from Appendix GB;
* it **normalizes a sub-rung suffix to its rung** before comparing — `G6c`, `G7a`, `G7b` all map to
  `G6`/`G7`. This is not a convenience: the ledger already carried `G7a`/`G7b` rows against a `### G7`
  heading, so a naïve set equality would have reported the ledger *incomplete in both directions* on
  its very first run, and the obvious "fix" is to relax the comparison — which turns an equality into
  a subset check wearing an equality's name, the exact defect the second RED below exists to catch.
  The normalization is stated here so it is a rule, not a patch;
* it asserts the two sets are equal **after normalization**, that no row has an empty *observed red*
  cell, and that every row carries a date.

**Why this is a rung and not a habit.** Logging L10 measured a twelve-row gate table in which **not
one** of the twelve benches existed, while eleven rungs reported themselves gated against it. A table
in a document is not a mechanism; a table plus a check that the table is complete is. The check is
twenty lines and it is the only thing in this ladder that watches the ladder.

**Gate.** `cargo test -p boyko-engine --test reflect_red_ledger`.

**RED MUTATION.** Delete one row from Appendix GB ⇒ the completeness test reds naming the rung.
**Second RED:** add a rung heading `G9` with no ledger row ⇒ reds in the other direction. Both, because
a set equality that is only ever tested in one direction is a subset check wearing an equality's name.

---

## 4. DEFERRED — explicitly, and to what

| item | deferred to | why, and what it costs to defer |
|---|---|---|
| Censusing the **real** `boyko_demo` artifact | a rung of its own, once a ship binary exists that CI builds | `boyko_demo` is `--exclude`d from every CI leg today (`.github/workflows/ci.yml:62` and five siblings) and pulls eframe/wgpu. Deferring costs: G3's artifact claim is about a fixture, and the fixture stands in for the demo by *argument*, not by identity. Stated in G3's "cannot claim". |
| Adding `REFLECTION-PLAN-*.md` to `GATED_DOCS` | ~~after the four plans stop moving~~ → **`docs/REFLECTION-PLAN-ECS.md`'s EG8 gate 6, which owns it** | Two plans held opposite decisions on one gate: ECS EG8 gate 6 registers all four documents in `GATED_DOCS` with a `("REFLECTION-PLAN-*.md", 0)` row in `OVER_WAIVED_MAX`, while this row deferred exactly that and called the deferral *"a choice, not an omission"*. **EG8 wins, and it wins on its own reasoning**: it is the last rung of the longest ladder, so by then all four documents exist — which answers this row's real objection, that the anchor gate's *path* check reds on a dead link to a sibling that has not landed. Verified: `tests/internal_docs_anchors.rs:231`'s `GATED_DOCS` holds four documents today and `PARTICLES-PLAN.md` is not among them, so this is a genuine addition and not a restatement. **Until EG8 lands, this file's anchors are hand-checked** (Appendix GC), and **EG8's checklist includes deleting that caveat in the same commit** — a document that says its anchors are hand-checked while a gate checks them is the doc-rot class this campaign has measured at 75 %. |
| The **bevy-shaped baseline** bench (`get_field` vs `HashMap<TypeId>` + `&dyn Reflect` + downcast) | `docs/REFLECTION-PLAN-CORE.md`, Wave 5 | It is a *beats-bevy* claim, not an *absence* claim. §3.3 is explicit that the feature-on/off delta *"does not prove beats-bevy"* and that both benches are required. Two claims, two owners. |
| A **cross-build wall-clock** feature-on/off A/B | **REFUSED, not deferred** | D8. The measured cross-build drift on this box (10.16 → 12.11 ns on an unchanged leg, `crates/boyko_ecs/benches/gj1_flag_cost.rs:22-29`) is wider than any effect the comparison could find. Recorded as refused so it is not re-proposed as "the obvious missing measurement". |
| `cargo-deny` / licence-surface assertions about the absent crate | out of scope | The claim is about this workspace's own graph. |
| A gate on the **editor** build's own hygiene (release + feature on) | `docs/REFLECTION-PLAN-ECS.md` | §5's release-editor gap is a *soundness* property of the setters, tested by the ECS plan's release-mode tests. G4 provides the leg it runs in (D7); it does not own the assertions. |
| Gating `has_component`'s missing `Bitset` branch | `boyko_ecs`, independently | B.4's incidental finding: a wrong answer rather than a refusal. Found via reflection, owned by the kernel. Already in `docs/OPEN-QUESTIONS.md`. |

---

## 5. Dependencies on the sibling plans

Stated as dependencies rather than assumptions, because each one can arrive with a different answer
than this plan expects.

| # | Depends on | Which sibling | Blocks | If it lands differently |
|---|---|---|---|---|
| **1** | **The B.1 fork — one table or two.** Horn 1 puts `TypeInfo` in `boyko_ecs`, which **ships** | `docs/REFLECTION-PLAN-CORE.md` (owner call, B.11 #1) | **G3's wording**, not its construction (D16) | Horn 1 ⇒ G3's message must narrow the claim from "no reflection metadata ships" to "no reflection *crate* ships", and needle A stops covering the metadata type |
| **2** | **The install mechanism** — the lazy `component_id()` funnel (A.5's recommendation) vs the explicit `register::<T>()` fallback | `docs/REFLECTION-PLAN-CORE.md` | **G6b, G6c, G7a** — all three compare *what the derive emitted*, and the explicit form emits an associated fn instead of a funnel slot | The explicit form makes G6c's twin comparison **easier** (an un-called associated fn is a cleaner symbol delta) and G6a's `E0433` mechanism **weaker** (the path appears in an `impl` the consumer may never name). Re-run G6a's red under whichever form lands |
| **3** | **The derive's refusal set** | `docs/REFLECTION-PLAN-CORE.md` | **G5** — one trybuild fixture per refusal; the corpus is a mirror of that set and reds when it drifts | Every added or removed refusal is a fixture added or removed, blessed under D13's compiler |
| **4** | **The Aether opt-in** (`reflect` key in `ComponentDef`) and its **spanned** refusal | `docs/REFLECTION-PLAN-CORE.md` (owner call, B.11 #3) | **G5's Aether twin** in `aether_tests` | If Aether components are reflectable **by default**, D3's leaf invariant reaches every crate containing an `aether!` block — G1 C1 must then treat "declares `reflect`" as a property Aether *imposes*, and the census message must say so |
| **5** | **The public by-id seam on `EcsMaster`** — now **four** items in **one** owner call: `add_component_by_id`, `remove_component_by_id`, `mark_component_changed`, and `EnableTagId::try_from_component_id` (the fourth was filed twice, as B.11 #2 and as the BOUNDARY plan's B-1, because it was reached from two directions) | `docs/REFLECTION-PLAN-ECS.md` EG2 (owner call, **B.13 #2**) | nothing in this ladder **directly** — and that is the point worth recording: it widens a **shipping** crate's public API for a dev-only feature, and **no gate here can see it**, because a public fn taking `Entity` + `ComponentId` names no reflection type and leaves no symbol with a reflection name | If it lands, the honest statement of the ship claim becomes "the reflection *crate* is absent; the seam it needed is not" — a sentence that belongs in the ECS plan and in `docs/REFLECTION-ANALYSIS.md` §4, not hidden in a gate's green |
| **8** | **May engine crates carry a `reflect` feature?** (`REFLECTION-ANALYSIS.md` B.12, owner sheet **B.13 #1**) | the owner; this plan **proceeds on yes** | **G0** (whether `reflect_dogfood` exists at all), **G1** (whether C5 has any subject beyond the ship targets), **G4** (whether the `reflect-dogfood` job exists) | If **no**: delete `crates/reflect_dogfood/`, its `USER_PACKAGES` row, its CI job, and G1's leaf-umbrella non-vacuity clause; four gates in the sibling plans stop saying "real engine types" and say "the engine's shapes". Nothing else in this ladder moves — D3's six clauses are correct either way, they simply have less to check |
| **9** | **CORE's C2 replaces G0's hollow `install_type_info`** — same name, same signature, real body | `docs/REFLECTION-PLAN-CORE.md` C2 (its D6) | **G3's needle B**, which is that name | If C2 renames it or makes it generic, needle B goes silently subject-less and the LTO-sensitivity probe stops probing. D5 records that the name's survival is *why* this needle was chosen; a rename is therefore a change to this plan, not only to CORE's |
| **10** | **`GATED_DOCS` registration for the four plan documents** | `docs/REFLECTION-PLAN-ECS.md` EG8 gate 6 | **Appendix GC's hand-checked caveat**, which EG8 deletes in the same commit | This plan previously deferred the registration and called the deferral a choice; EG8 registers it. One owner (EG8), one commit, and the caveat is not allowed to outlive the gate |
| **6** | **The v1 taxonomy** — specifically `ValueKind::Array` (B.8) | `docs/REFLECTION-PLAN-CORE.md` | **G4's Miri fixture shape** — the dense/array shapes are reproduced locally because the real ones live in `boyko_render`, which Miri cannot reach | If arrays slip to v2, the local dense fixture drops its array member and the Miri row shrinks accordingly |
| **7** | **`Sink`/`Source`'s one `dyn`** | `docs/REFLECTION-PLAN-BOUNDARY.md` | **G7a's twin comparison** — a `dyn` in the feature-on arm is fine; a vtable that survives into the feature-off image is a residue and clause 3 fires | — |

**Dependency 5 is the one to read twice.** It is the single place where this ladder is *structurally
blind*, it is blind for a reason no additional gate can fix, and the analysis already names it (§4:
*"This is the one place a dev-only feature widens a SHIPPING crate's public surface"*). Writing it
into the dependency table is the only mechanism available: a gate cannot watch it, so a reader must.

---

## Appendix GA — the cost-proof protocol, in one place

Written out because G7 is the rung most likely to be executed by someone who did not read §2, and
because every number below has a stated provenance rather than a target.

**Build configuration, identical for all four images.**

```
cargo build -p reflect-fixture --bin <bin> --release
  --config profile.release.lto="fat"
  --config profile.release.codegen-units=1
env: CARGO_TARGET_DIR = <temp>/boyko-reflect-codegen-<bin>   (per image — two legs sharing a
                                                              target dir rebuild each other, and a
                                                              nested cargo sharing the outer sweep's
                                                              dir is the linker `permission denied`
                                                              this campaign has already paid for)
env: RUSTFLAGS removed   (an inherited `-C embed-bitcode=no` is incompatible with `-C lto`)
```

**Extraction.** `llvm-nm <image>` → the name column, sorted, as a multiset. `llvm-size <image>` →
`.text`. Both tools resolved PATH-first then through the rustup toolchains' `rustlib` bins, and
**absent ⇒ panic** (D6).

**Clause order** — 1 through 4 of G7a, in that order, each returning before the next. The order is the
discipline: a verdict printed before its instrument was certified is the failure `gj1_flag_cost`'s
header describes as *"drift wearing a verdict's name."*

**Numbers this protocol produces, all MEASURED and none predicted here:**

| reading | where it is recorded |
|---|---|
| whether `N1` and `N2` are byte-identical | G7a clause 5 + Appendix GB |
| the symbol count and `.text` size of `N1` | G7a's pass message |
| the symbol delta of `P` − `N1` (the positive control's magnitude) | G7a clause 2's message |
| wall time of the four builds | the test file's own header |
| the six cells of G3's link-configuration table | §G3, pasted back into this file |
| G7b's per-component opt-in cost and its first-touch cost, separately | the bench's stdout |

---

## Appendix GB — the RED ledger

**Every row must be filled by running the mutation.** An empty *observed* cell means the rung is not
landed, and `tests/reflect_red_ledger.rs` (G8) enforces exactly that. Rows are `—` until run; nothing
in this table is a prediction.

| rung | mutation | expected red | observed | date |
|---|---|---|---|---|
| **G0** | delete `reflect-fixture`'s `USER_PACKAGES` row | `engine_packages_census` names the package | `every_workspace_member_is_classified_as_engine_or_user` FAILED (exit 101): *"workspace members [\"reflect-fixture\"] are in neither ENGINE_PACKAGES nor this gate's USER_PACKAGES"* *(run at the G1–G4 session — G0's session recorded its REDs in prose amendments but left these cells unfilled)* | 2026-08-21 |
| **G0** | members without `default-members` | bare root `cargo check` stops covering them, still green | observed from both sides: with the three reflect members trimmed from `default-members` and a deliberate `E0308` in `reflect_never.rs`, bare root `cargo check --all-targets` finished **exit 0** in 2 m 00 s — success reported over a member it never compiled; `default-members` restored, same command: **exit 101**, `error[E0308]: mismatched types` in `reflect_never` | 2026-08-21 |
| **G0** | delete the `[[bin]] reflect_never` target | gate 6 reds naming it; G3 L3 and G7a `T` lose their subject | gate 6's named-target invocation (`cargo check -p reflect-fixture --bin reflect_never`): **exit 101**, *"error: no bin target named `reflect_never` in `reflect-fixture` package"* — and auto-discovery did NOT re-mint it (`autobins = false` doing its load-bearing work, per this rung's amendment) | 2026-08-21 |
| **G1** | `default = ["reflect"]` on `reflect-fixture` | C1 reds | `c1_no_default_reaches_reflect` FAILED: *"reflect-fixture: `default` transitively enables `reflect-fixture/reflect`"* | 2026-08-21 |
| **G1** | `boyko-reflect` edge non-optional | C2 reds | as written, cargo refuses the manifest BEFORE the census (exit 101): *"feature `reflect` includes `dep:boyko-reflect`, but `boyko-reflect` is not an optional dependency"*; with the `dep:` reference also removed so the manifest loads, **C2's own red**: *"reflect-fixture -> boyko-reflect (normal edge) is NOT optional"* | 2026-08-21 |
| **G1** | `reflect = ["boyko-reflect"]` (no `dep:`) | C3 reds | `c3_reflect_is_pulled_only_through_dep_syntax` FAILED: *"feature `reflect` contains `boyko-reflect` -- the only permitted form is `dep:boyko-reflect`"* | 2026-08-21 |
| **G1** | `boyko-scene = { …, features = ["reflect"] }` on an edge | **C4** reds — the convenient wiring, and the one nothing can turn off | `c4_no_dependency_edge_enables_reflect` FAILED naming the edge and printing the B.12 rule: *"reflect-dogfood -> boyko-scene (normal edge) carries `features = [.. \"reflect\" ..]`"* | 2026-08-21 |
| **G1** | `reflect = ["boyko-render/reflect"]` on `boyko_demo` | **C5** reds naming the ship target | as written, cargo refuses the manifest BEFORE the census (`boyko-render` is not a dependency of `boyko_demo`, exit 101); the declare-half `reflect = []` reaches **C5's own red**: *"boyko_demo: declares a `reflect` feature"* (see the G1 measured note: the forward-half is unbuildable loadably on `boyko_demo` today) | 2026-08-21 |
| **G2** | `boyko_demo` gains `default = ["reflect"]` | ship clause reds | `ship_closures_contain_no_reflect` FAILED naming `boyko_demo` and printing the offending rows (*"boyko-reflect feature \"default\"" / "boyko-reflect v0.1.0"*). Note: the mutation needs the `reflect = ["dep:boyko-reflect"]` feature definition too, or the manifest does not load (`default` may only name an existing feature) | 2026-08-21 |
| **G2** | needle spelled `boyko_reflect` (lib name, not package) | positive control reds, ship clauses stay green | exactly that: `positive_control_finds_the_crate` FAILED (*"NOT RESOLVED (closure census inert)"*), `ship_closures_contain_no_reflect` ok — and this shape depends on parsed-name matching: a raw-substring needle would keep finding `boyko_reflect` in the worktree's own path | 2026-08-21 |
| **G2** | run the harness under `--features reflect-dogfood/reflect` | the invocation guard reds: *"not a ship closure"* | **two halves, per the G2 measured note.** Outer form (`cargo test -p boyko-engine -p reflect-dogfood --features reflect-dogfood/reflect --test reflect_ship_closure`): green, exit 0 — correctly, since the harness's own spawned `cargo tree` is feature-clean by construction and the reported number IS a ship closure (the env form that would have detected the outer selection is unbuildable: `CARGO_ENCODED_ARGS` does not exist, env NO-DIFF, measured). Portable-form RED (route `--features reflect-dogfood/reflect` into `ship_tree`'s argv): the purity guard reds with *"the ship-closure gate was invoked under a feature selection ([\"--features\"]); the number below is not a ship closure"* | 2026-08-21 |
| **G3** | drop `lto = "fat"` | **all six cells re-read and recorded** | re-read: **unchanged** (L1 0/0, L2 A=1, L3 0/0) and the gate **stayed green** — B.6's *"every cell reads 1"* does NOT reproduce on this subject today, because at G0 the crate has exactly one fn and L1/L3 reference none of it (the pulled-object rule, §G3's measured note); the LTO sensitivity is expected to appear when CORE C2 lands a real surface — re-run this RED and the calibration there | 2026-08-21 |
| **G3** | L1 built from `reflect_on` | ship cell fills | two layers, both observed: as a bare bin swap, **gate 5 reds first** (*"L1's artifact reports bin=reflect_on reflect_feature=on linkage=present -- the build did not use the leg this test asked for"*); with gate 5's L1 expectation aligned so the counts are reached, **the ship cell fills**: *"THE SHIP CELL IS NOT ZERO: needle A = 1, needle B = 1"* — the needle names image content, not source text | 2026-08-21 |
| **G3** | `reflect` key deleted from `reflect_on` | L2 present control reds (census inert) | run in the landed-deviation form (the call to `reflect_linkage()` deleted): *"NOT RESOLVED (census inert): the present control carries NO `boyko_reflect` symbol … (L2 A = 0)"* | 2026-08-21 |
| **G3** | L3 built from `reflect_off_twin` instead of `reflect_never` | L3's no-opt-in assertion reds — the discriminator caught becoming a duplicate of L2 | the non-collision assertion reds naming the resolved source (*"L3's fixture (…src/bin/reflect_on.rs) contains the reflect opt-in tokens [\"reflect_linkage\", \"boyko_reflect::\"]"*) — note the path: the bin→source mapping goes through the `[[bin]]` table, so the swapped bin name resolves to the twin's SOURCE and cannot dodge the scan | 2026-08-21 |
| **G4** | drop `--features reflect-fixture/reflect` | per-test non-vacuity assertion reds | both layers observed (one combined run for the workflow layer — the deleting sed also took the Miri row, so RED1+RED2 fired together, each from its own test with its own message). Leg simulation: `the_leg_that_names_itself_carries_the_feature` FAILED — *"leg `reflect-on` asked for the reflect feature and did not get it … every reflect test in this selection is currently a no-op wearing a green name"*. Workflow layer: `reflect_on_job_is_wired` FAILED — *"the `reflect-on` job lost `--features reflect-fixture/reflect`"* | 2026-08-21 |
| **G4** | drop `-p reflect-fixture` from the Miri sweep | `reflect_ci_coverage` reds | `miri_sweep_names_the_right_rows_in_the_right_shapes` FAILED: *"the Miri sweep lost `-p reflect-fixture --features reflect-fixture/reflect` -- the ONLY row that reaches derive-generated unsafe under Miri (B.9); dropping it is the silent revert this gate exists to catch"* | 2026-08-21 |
| **G4** | `reflect` added to `default` to avoid the flag | G1 C1 reds | `c1_no_default_reaches_reflect` FAILED: *"reflect-fixture: `default` transitively enables `reflect-fixture/reflect`"* | 2026-08-21 |
| **G4** | write the sweep as `-p boyko-reflect --features reflect` | cargo errors before compiling: *"none of the selected packages contains these features"* — **record the literal message** | recorded, two shapes on cargo 1.97.1. In the sweep's multi-package selection: **`error: none of the selected packages contains this feature: reflect`** (+ `help: packages with the missing feature: boyko-render, boyko-scene, reflect-fixture, reflect-dogfood`) — the plan's quoted plural is 1.97.1's singular. Single-package form: *"error: the package 'boyko-reflect' does not contain this feature: reflect"*. Caveat: on THIS box the literal `cargo +nightly miri test …` form dies EARLIER — the msvc-hosted nightly cannot build Miri's sysroot at all (no `link.exe`; a GNU coreutils `link` shadows it) — so the resolver message was recorded through plain cargo with the identical selection, which is the same resolver | 2026-08-21 |
| **G4** | delete `#[cfg(not(miri))]` from `reflect_absence_census.rs` | the Miri leg reds on a process spawn, for a non-reflection reason | observed under a GNU-hosted nightly miri (installed for this rung; the msvc-hosted one cannot build a sysroot on this box — no `link.exe`): the leg exits 1 with *"error: unsupported operation: `CreateFileW` not available when isolation is enabled"* — it reds ONE STEP BEFORE the predicted process spawn, at the census's own source-file read, which is the same class (host I/O the interpreter refuses, nothing to do with reflection). Guard restored ⇒ the leg is green again (the census compiles out, `reflect_leg_nonvacuity` runs) | 2026-08-21 |
| **G6c** | one unrelated field added to the twin's source | the twin-source identity gate reds **before** G7a runs, naming the token | — | — |
| **G5** | one refusal removed from the derive | trybuild: *expected compile failure* | — | — |
| **G5** | `compile_fail` harness run with feature **off** | all fixtures compile ⇒ reds on all at once | — | — |
| **G5** | re-bless under chocolatey `rustc` 1.95.0 | `trybuild_corpus_compiler_witness` names both compilers | — | — |
| **G6** | `#[cfg(feature = "reflect")]` removed from the reflect slot | `reflect_off_twin` fails with `E0433` | — | — |
| **G6** | un-`cfg`'d `#[used]` static in the reflect slot | G6a compiles, **G6c** reds on the multiset | — | — |
| **G7a** | one unrelated field on the twin's side | clause 0: NOT MEASURABLE (twin) — **and record what clause 3 would have printed** | — | — |
| **G7a** | `codegen-units = 16` | clause 1: NOT MEASURABLE (instrument) | — | — |
| **G7a** | positive control's extra fn deleted | clause 2: NOT RESOLVED (control inert) | — | — |
| **G7a** | one un-`cfg`'d item in the reflect slot | clause 3: RESOLVED — RESIDUE, symbol named | — | — |
| **G7b** | `component_id()` install gate deleted | the pair stops resolving | — | — |
| **G8** | one ledger row deleted | completeness test names the rung | — | — |
| **G8** | rung `G9` added with no row | completeness test reds in the other direction | — | — |

---

## Appendix GC — anchors used by this document

Verified against the tree at **2026-08-21**, branch `feat/reflection`. This file is **not yet** in
`tests/internal_docs_anchors.rs`'s `GATED_DOCS`, so these are hand-checked, not machine-checked —
recorded here so a future reader knows which kind of claim they are.

> ⏳ **This caveat has an expiry, and deleting it is a line item on another rung.**
> `docs/REFLECTION-PLAN-ECS.md`'s **EG8 gate 6** registers this file and its three siblings in
> `GATED_DOCS` with an `OVER_WAIVED_MAX` row of `0`. When EG8 lands, every anchor below becomes
> machine-checked on every `cargo test`, and **this paragraph must be deleted in the same commit** —
> a document that still claims its anchors are hand-checked after a gate started checking them is the
> doc-rot class, and it is the kind that makes a reader distrust the gate rather than the sentence.

| anchor | what it carries for this plan |
|---|---|
| `crates/profile_fixture/tests/profile_axis_census.rs` | the measured census template: the LTO finding, the present control, the RED-not-SKIP tool rule, the self-building legs |
| `crates/profile_fixture/tests/profile_axis_census.rs:90` | `ZONE_EMIT_SYMBOL = "mint_cold"` — the plain-fn symbol class D5 mirrors |
| `crates/profile_fixture/Cargo.toml` | *"Adding any second dependency here … destroys the gate's ARGUMENT"* — G0's fixture rule |
| `.github/workflows/ci.yml:62, :87, :89, :129, :167, :176, :191` | the `--exclude boyko_demo` legs — D2's reason |
| `.github/workflows/ci.yml:78-79` | the existing `[debug, release]` matrix the ON leg mirrors (D7) |
| `.github/workflows/ci.yml:144-153` | the `profile-census` job + `components: llvm-tools` (D6) |
| `.github/workflows/ci.yml:222-226` | the hand-listed Miri sweep — B.9's allowlist, G4's row |
| `crates/boyko_ecs/Cargo.toml` | the `profiling-analysis` measurement: unification defeated `--no-default-features` (D3, G2) |
| `crates/boyko_ecs/benches/gj1_flag_cost.rs` | the ABBA/twin/verdict idiom; the cross-build refusal at `:22-29` (D8) |
| `crates/boyko_log/benches/log_gate_cost.rs:42-46` | *"a zero control whose expected value is exactly zero measures DRIFT"* (D10) |
| `tests/engine_packages_census.rs` | the two census rows G0 owes (D15) |
| `tests/trybuild_corpus_compiler_witness.rs` | `BLESSED_RUSTC`, the corpus count, the chocolatey hazard (D13) |
| `tests/internal_docs_anchors.rs:231` | `GATED_DOCS` — why this file's anchors are hand-checked |
| `crates/boyko_ecs/tests/compile_fail_zero_init.rs` | the trybuild harness shape G5 copies |
| `crates/boyko_macros/src/component.rs:348` | the `component_id()` funnel and its six install slots — the emission site P-D watches |
| `crates/boyko_macros/Cargo.toml` | tokens-are-not-deps, stated in the manifest |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs:277` | `BIND_ACCESSORS` — the shipping table D16's Horn 1 would merge into |
| `crates/boyko_render/Cargo.toml` | `boyko_rhi_vulkan` edge ⇒ `GpuTransform3D` is Miri-unreachable (G4) |
| `crates/boyko_scene/Cargo.toml` | `boyko-input` edge ⇒ Miri-executability is a thing to run, not to read |
