//! **Production-reachability census — is this subsystem shipped by anything, or only by its own
//! tests?**
//!
//! # Why this file exists, measured twice, both times found by a human and late
//!
//! **Precedent 1 — `ProfilerPlugin`, and the comment it left behind.** From
//! `crates/boyko_app/src/plugins.rs` (the block above `app.add_plugin(ProfilerPlugin)`):
//! *"MEASURED after profiling rung 15, and it is the reason this line exists: `ProfilerPlugin` was
//! added NOWHERE outside tests. Fifteen rungs of profiler — the store, the fold, the GPU channel,
//! the retention tiers, the telemetry writer, the overlay — sat complete and unreachable from any
//! host, because the resource they all read was never inserted."* Fifteen rungs of green tests, and
//! the subsystem did not run in a single shipped frame.
//!
//! **Precedent 2 — `boyko_ui`.** A complete UI subsystem — layout solver, clipping, MSDF text, a
//! pointer model, 48 test files — that no shipped path has ever drawn. Its *only* reverse
//! dependency edge in the whole workspace is `crates/boyko_render/Cargo.toml`'s
//! `[dev-dependencies]`, annotated at the site "GUI P6b screenshot test ONLY". `boyko_app` — the
//! host layer, the crate a game links — names it in neither table.
//!
//! Both defects have the same shape and neither is hard to detect. Nothing was looking.
//!
//! # Why a per-crate test suite structurally CANNOT catch this
//!
//! This is the deep reason the 48 green `boyko_ui` test files prove nothing about reachability, and
//! it is worth stating plainly because it is the argument against "just add a test in the crate".
//!
//! `crates/boyko_ui/Cargo.toml` names no render crate and no host crate — it cannot, because
//! `boyko_render` dev-depends on `boyko_ui` and the reverse edge would be a cycle. A test *inside*
//! `boyko_ui` therefore has no way to spell "and then the host drew it": the types that would let
//! it say so are not in its dependency graph. Every test it can write is a test of its own
//! arithmetic. The same holds for every crate in this workspace: **a crate cannot test that
//! something outside it uses it, because the thing outside it is not in its graph.**
//!
//! Reachability is a property of (a) the manifest graph and (b) the call graph *across* crate
//! boundaries. The only place in this repository where both are visible at once is the workspace
//! root, which is why this census lives here and not in any crate. `CARGO_MANIFEST_DIR` **is** the
//! repository root for the `boyko-engine` package, so no `../..` walking can point the scan at the
//! wrong tree, and the package has zero dependencies, so the census needs no GPU, no `dxc`, no
//! golden corpus and no build of the engine — the `internal_docs_anchors.rs` /
//! `engine_packages_census.rs` rationale, verbatim.
//!
//! # What is checked — three classes
//!
//! **Class 1, the manifest class ([`every_workspace_member_is_shipped_or_recorded`]).** Every
//! workspace member is classified against the declared [`PRODUCT_ROOTS`]: `ProductRoot`,
//! `ProductionReachable` (reachable from a root through `[dependencies]` edges only),
//! `DevOnlyReferenced` (**a member shipped by nothing** — every reverse edge is a
//! `[dev-dependencies]` one), `ReferencedOnlyByUnreachable` (a normal reverse edge, but only from
//! members that are themselves unreachable), or `Unreferenced`. This is two lines of logic and its
//! value is already proven: `boyko_ui` fails it today and would have failed it the day that
//! dev-dependency line was written.
//!
//! **Class 2, the plugin class ([`every_production_plugin_has_a_production_registration`]).** Every
//! `impl Plugin for X` written in production source must have at least one production
//! `add_plugin`/`add_plugins` registration naming `X`. This is the class that would have caught
//! `ProfilerPlugin` fifteen rungs earlier.
//!
//! **Class 3, the host-seam class ([`every_declared_host_seam_is_named_outside_its_own_crate`]).**
//! A short, explicit [`HOST_SEAMS`] roster of entry points documented as "the host calls this" —
//! each must be named in production source of a crate *other* than the one defining it. A seam
//! whose only callers are inside its own crate is not a seam. `UiUploadSystem::
//! host_upload_frame_from_world` is documented at `crates/boyko_render/src/ui/upload.rs:180-182` as
//! "the single call the render host makes each frame" and has **zero callers anywhere, tests
//! included**.
//!
//! # Production vs test — the exclusion that is load-bearing
//!
//! **Production** is `crates/*/src/**` + `tools/*/src/**` + `src/**`, minus every `#[cfg(test)]`
//! region. **Test** is `tests/`, `benches/`, `examples/` (never walked at all) *and* those excluded
//! regions. The exclusion has three shapes in this tree and all three are handled:
//!
//! 1. `#[cfg(test)] mod tests { … }` — an inline block, removed by brace balance.
//! 2. `#[cfg(test)] mod tests;` — a whole **file** pulled in by a gated `mod` declaration. Eight
//!    files in this tree are test-only this way, including
//!    `crates/boyko_ecs/src/ecs/core/profiling/tests.rs`, which contains two
//!    `app.add_plugin(ProfilerPlugin)` calls. A census that walked `src/**` without resolving that
//!    declaration would score `ProfilerPlugin` as registered even with
//!    `plugins.rs:399` deleted — **a gate that cannot fail.**
//! 3. `#[cfg(test)] #[path = "resources_tests.rs"] mod tests;` — the same, with the file name
//!    overridden. Two sites in `boyko_physics`.
//!
//! A fourth trap is not a `cfg` at all: **doc comments**. `crates/boyko_ui/src/interaction/
//! plugin.rs:141` reads `/// app.add_plugins(UiBindingPlugin::default());`. A grep-shaped census
//! scores that as a registration and reports the dead UI as reachable. Comments and string literals
//! are blanked before any pattern is looked for, and
//! [`a_registration_written_in_a_doc_comment_is_not_a_registration`] pins it.
//!
//! Also load-bearing, and asserted rather than assumed: the `#[cfg(…)]` test-gate rule matches the
//! word `test` anywhere in the predicate, which would be **wrong** for `#[cfg(not(test))]` — code
//! present in production and absent in tests. No such site exists in this tree, and
//! [`the_cfg_test_rule_has_no_not_test_counterexample_in_the_tree`] fails the moment one is written.
//!
//! # What is NOT checked — enumerated, because a gate that hides its blind spots is the defect
//!
//! * **Semantics.** Nothing here says a reachable subsystem *works*, is ordered correctly, or is
//!   ever actually stepped. The MEASURED example is the particle subsystem, whose class-2 row this
//!   file carried until the plugin was wired: `boyko_render::upload_particle_emit_requests` is
//!   named from `boyko_app::runner`, so class 3 was satisfied and stayed satisfied the whole time
//!   the ECS half was missing — because "is this seam crossed in source" is not "does this seam
//!   ever run". It did not: every call site sat inside `host.gpu.particle_upload_slots(s).map(..)`,
//!   which was `None` in every production host. Reading a green run as "the engine is wired" is a
//!   mistake, and the direction of that mistake is toward green.
//! * **Module inclusion.** The walk takes every `.rs` file under a member's `src/`; it does NOT
//!   resolve the module tree from `lib.rs`, so an orphan file that no `mod` declaration includes —
//!   rustc never compiles it and nothing warns — is read as production. If such a file held a
//!   plugin's only registration, class 2 would pass on it. **Measured, not predicted:** the class-2
//!   file-module injection was first written with the child file at the wrong path
//!   (`src/revival.rs` for a `mod` declared in `src/plugins.rs`, where rustc looks in
//!   `src/plugins/`), the census counted its registration, and the run went green. The error
//!   direction is under-reporting, the same direction as the doc-comment trap. Resolving the module
//!   tree properly means handling `#[path]`, `cfg`-gated non-test modules and `mod` inside inline
//!   modules — more machinery and more false-positive surface than the hole justifies today.
//! * **Transitive plugin reachability.** Class 2 is flat: one production registration anywhere is
//!   enough. `InputPlugin` passes on the strength of `FlyCameraPlugin::build`, and
//!   `FlyCameraPlugin` is itself registered only by `boyko_app/examples/*`. Making this transitive
//!   needs each registration attributed to its enclosing `Plugin::build`, which is a real design
//!   with real false-positive risk; it was deliberately not built. The consequence is stated so it
//!   is not discovered as a surprise.
//! * **Intra-crate deadness.** `pack_ui_instance` has a same-crate caller
//!   (`UiUploadSystem::pack_sort_upload`) which is itself only reachable through the dead
//!   `host_upload_frame*`. Class 3's cross-crate rule catches it; a symbol whose whole dead chain
//!   is inside one crate is invisible to every class here.
//! * **Non-plugin, non-roster symbols.** Class 3 covers exactly the fourteen rows of
//!   [`HOST_SEAMS`]. There is no mechanical rule over "every `pub fn`" because there cannot be one
//!   that is a gate: a library crate's entire public API would be reported, and a census that
//!   reports four hundred things reports nothing.
//! * **Feature-gated edges.** A dependency behind a non-default feature is read as a normal edge.
//!   `boyko-app`'s `hwrt` forward is a feature on an edge that already exists, so nothing in this
//!   tree turns on the distinction today.
//! * **Anything outside `src`.** `build.rs`, `examples/`, `benches/` and `tests/` are never
//!   scanned; by construction they cannot make a symbol production-reachable, which is the point.
//!
//! # The exemptions, and why they are a list rather than a heuristic
//!
//! A roster with no exemptions gets deleted by the first person it inconveniences, so
//! [`PRODUCT_ROOTS`] and [`EXEMPT_PLUGINS`] are explicit, one reason per row, in the manner of
//! `engine_packages_census.rs`'s `USER_PACKAGES`. "Ends in `_demo`", "contains `bench`" or "is in
//! `tools/`" would be a rule that silently absorbs the next real defect. An exemption whose reason
//! is "it fails" is not a reason and does not belong on either list.
//!
//! **Measured defects are NOT exemptions.** They go in [`KNOWN_UNREACHABLE_MEMBERS`],
//! [`KNOWN_UNREGISTERED_PLUGINS`] and [`KNOWN_DEAD_SEAMS`], which are asserted for **exact
//! equality** — a new one reds the gate, and so does a *repaired* one. A `<=` would let the count
//! fall silently and could not tell "someone fixed it" from "the scanner stopped seeing it". The
//! baselines were taken at `59009f8a` and the failure messages say exactly what to edit.
//!
//! # For whoever wants to delete this test
//!
//! Two questions first. (1) Which *other* mechanism in this repository would have reported that
//! `boyko_ui`'s only reverse edge is a `[dev-dependencies]` line? The workspace builds green, every
//! crate's tests pass, clippy is clean at `-D warnings`, and `cargo check --workspace
//! --all-targets` compiles `boyko_ui` happily — because compiling a crate is not the same as
//! shipping it. (2) If the answer is "code review", note that both precedents *passed* review, for
//! fifteen rungs and for a whole campaign respectively. If a better mechanism exists, delete this
//! file and say which one it is in the commit message. Turning a class off because it went red is
//! the failure this file is written against.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung 1 — the manifest class: roots, exemptions, recorded defects
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A workspace member that IS a product, so "what depends on it?" is the wrong question.
///
/// Named fields rather than a tuple: two same-typed positional strings in one signature is exactly
/// the transposition `docs/RUST-ERGONOMICS.md` ERG-02 names.
struct ProductRoot {
    /// The `[package] name`, spelled as the manifest spells it (this workspace mixes `-` and `_`).
    package: &'static str,
    /// Why this member is a product and not a library awaiting a consumer. One line, at the site.
    reason: &'static str,
}

/// The declared products. **Reachability is measured FROM here**, so a wrong row here is a hole in
/// every class below — which is why each carries its argument rather than a category name.
///
/// ⚠️ Derived by reading the tree, not assumed. `CLAUDE.md` calls the root crate "library-shaped";
/// `src/main.rs` is confirmed to be a `println!` placeholder with an empty `[dependencies]` table,
/// so it roots nothing, and it is listed only because it is a member that must be classified.
const PRODUCT_ROOTS: [ProductRoot; 8] = [
    ProductRoot {
        package: "boyko-app",
        // The top of the engine stack: the OS loop, device boot, the windowed runner and
        // `EnginePlugins`. Nothing in-tree depends on it because there is no in-tree game — it is
        // the crate a game LINKS, and `crates/boyko_app/examples/*` is how it is driven today.
        // Roots the whole render/RHI/scene/input closure.
        reason: "the host layer — the crate a game links; the render stack hangs off it",
    },
    ProductRoot {
        package: "boyko_demo",
        // A `[[bin]]` game (also wasm32-targeted). Depends on the ECS core only, never on the host.
        reason: "the demo game binary",
    },
    ProductRoot {
        package: "boyko-engine",
        // The workspace-root package. Its `src/main.rs` is a placeholder `println!` with an empty
        // `[dependencies]` table, so it roots nothing; it also hosts the source-tree censuses,
        // this file among them.
        reason: "the workspace-root placeholder binary + the home of the source-tree censuses",
    },
    ProductRoot {
        package: "bench-bevy-vs-boyko",
        // A criterion harness. It reaches the engine through `[dev-dependencies]`, which is
        // correct for a bench crate and must not be read as the `boyko_ui` shape.
        reason: "the Bevy comparison bench harness — a measurement product, not a dependency",
    },
    ProductRoot {
        package: "aether-tests",
        // Boots real `App`s the way a game does; `engine_packages_census.rs` records the same
        // decision for the same crate ("a consumer of the engine, not part of it").
        reason: "the Aether integration-test crate — a consumer of the engine",
    },
    ProductRoot {
        package: "prof_decode",
        reason: "the offline profile-artifact decoder tool (`tools/prof_decode`, one `[[bin]]`)",
    },
    ProductRoot {
        package: "profile-fixture",
        // G14(a)'s subject: two binaries, one zone site each. The profiling gates BUILD these.
        reason: "profiling-gate fixture binaries, built and run by the profiling gates",
    },
    ProductRoot {
        package: "profile-fixture-log",
        reason: "G16(a)/(b)'s one-`debug!`-site fixture binary",
    },
];

/// How a member relates to the declared products. A closed set, so an exhaustive `match` is
/// possible and a sixth case cannot be forgotten (ERG-14).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Reach {
    /// Listed in [`PRODUCT_ROOTS`].
    ProductRoot,
    /// Reachable from some root through `[dependencies]` edges only.
    ProductionReachable,
    /// Not root-reachable; every reverse edge from another member is a dev/build one.
    /// **This is the `boyko_ui` class: shipped by nothing.**
    DevOnlyReferenced,
    /// Not root-reachable; has a normal reverse edge, but only from members that are themselves
    /// not root-reachable. A dead chain hanging off a dead member.
    ReferencedOnlyByUnreachable,
    /// Not root-reachable and named by no other member at all, in any table.
    Unreferenced,
}

/// A member that is neither a root nor production-reachable, recorded with the class it holds.
struct RecordedMember {
    /// The `[package] name`.
    package: &'static str,
    /// The class measured when the baseline was taken. Pinned, not just the name: a member moving
    /// from `DevOnlyReferenced` to `Unreferenced` (its last dev-dependency deleted) is a change
    /// worth a red.
    class: Reach,
    /// What was measured, and whether it looks like a defect or a decision.
    note: &'static str,
}

/// The measured baseline at `59009f8a`. **Defects awaiting a decision, NOT exemptions.**
///
/// Asserted for exact equality in both directions, so this list cannot rot: a new unreachable
/// member reds the gate, and so does wiring one of these up.
const KNOWN_UNREACHABLE_MEMBERS: [RecordedMember; 5] = [
    RecordedMember {
        package: "boyko-ui",
        class: Reach::DevOnlyReferenced,
        note: "PRECEDENT 2. Only reverse edge in the workspace is crates/boyko_render/Cargo.toml's \
               [dev-dependencies], annotated 'GUI P6b screenshot test ONLY'. boyko-app names it in \
               neither table. A complete UI subsystem no shipped path draws.",
    },
    RecordedMember {
        package: "boyko-physics",
        class: Reach::DevOnlyReferenced,
        note: "Same shape, found by this census: the ONLY reverse edge is boyko-render's \
               [dev-dependencies]. The in-house TGS-Soft solver is compiled by no shipped path. A \
               PhysicsPlugin host wiring is reported in flight on another branch; at this HEAD the \
               edge does not exist.",
    },
    RecordedMember {
        package: "boyko-serialize",
        class: Reach::Unreferenced,
        note: "Found by this census, and the strongest form: ZERO reverse edges of any kind. No \
               member names boyko-serialize in [dependencies] OR [dev-dependencies]. It is built \
               only because it is a workspace member.",
    },
    RecordedMember {
        package: "aether",
        class: Reach::DevOnlyReferenced,
        note: "The Aether proc-macro shim. Its only reverse edge is aether-tests' \
               [dev-dependencies]. Plausibly deliberate — a DSL is for consumer code, and no \
               in-tree consumer exists yet — but it is structurally the boyko_ui shape and is \
               recorded rather than exempted so the decision is taken, not assumed.",
    },
    RecordedMember {
        package: "aether-lang",
        class: Reach::ReferencedOnlyByUnreachable,
        note: "The transpiler half. Normal-depended-on by `aether` alone, which is itself \
               unreachable — so it is dead by transitivity rather than by its own edges. Resolves \
               the moment `aether` does.",
    },
];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung 2 — the symbol classes: plugin exemptions, recorded defects, the host-seam roster
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A plugin type that legitimately has no in-tree registration.
struct ExemptPlugin {
    /// The type name as `impl Plugin for …` spells it.
    ty: &'static str,
    /// Why nothing in the tree registers it, and why that is correct.
    reason: &'static str,
}

/// Plugins a consumer adds, so no in-tree registration is expected.
///
/// Three rows, each an argument. Note what is NOT here: "any plugin in `boyko_app`" would have
/// swallowed nothing today but is the heuristic that swallows tomorrow's defect.
const EXEMPT_PLUGINS: [ExemptPlugin; 3] = [
    ExemptPlugin {
        ty: "EnginePlugins",
        // The root plugin group. `App::add_plugins(EnginePlugins::window(…))` is written by the
        // consumer; being unregistered in-tree is what "root" means.
        reason: "the root plugin group — the consumer's first line, by construction unregistered",
    },
    ExemptPlugin {
        ty: "FlyCameraPlugin",
        // Its own doc: "Add it ALONGSIDE `EnginePlugins` for an interactive windowed scene".
        // `EnginePlugins` deliberately adds no `InputPlugin` (it is action-type-agnostic), so a
        // scene opts in. Four `boyko_app/examples/*` do exactly that.
        reason: "opt-in host plugin added by the scene; EnginePlugins is action-type-agnostic",
    },
    ExemptPlugin {
        ty: "TransformPlugin",
        // `crates/boyko_app/src/plugins.rs:428` states the decision: "CameraPlugin SUPERSEDES
        // TransformPlugin — adding both would" duplicate the propagation systems. It is the
        // alternative composition for a consumer who wants propagation without a camera.
        reason: "superseded in-tree by CameraPlugin (plugins.rs:428); the camera-free alternative",
    },
];

/// A plugin type with no production registration, recorded rather than exempted.
struct RecordedPlugin {
    /// The type name.
    ty: &'static str,
    /// What it registers and why its absence matters. The declaring `file:line` is not pinned
    /// here — line anchors rot, and the failure message prints the live one.
    note: &'static str,
}

/// The measured class-2 baseline at `59009f8a`, asserted for exact equality.
///
/// **One row has been struck since that measurement**, in the commit that removed the defect it
/// recorded: `ParticlePlugin`, now composed by `EnginePlugins::build`. The baseline is asserted in
/// BOTH directions, so a repair reds this class exactly as a new defect does — deliberately, so
/// that "improved" can never be confused with "the scanner stopped seeing it". Striking the row is
/// part of the same act as the wiring, never a reaction to the red.
const KNOWN_UNREGISTERED_PLUGINS: [RecordedPlugin; 6] = [
    RecordedPlugin {
        ty: "DdgiPlugin",
        note: "boyko_render — inserts DdgiConfig / ResolvedDdgi / DdgiUpdateConfig / DdgiCaps and \
               resolve_ddgi_grid_gated. boyko_app builds the DDGI atlas, UBOs and update pipeline \
               in gpu_scene/csm.rs, so the device side is wired and the ECS side is not.",
    },
    RecordedPlugin {
        ty: "UiPlugin",
        note: "boyko_ui — the layout/text/clipping subsystem's App entry point.",
    },
    RecordedPlugin {
        ty: "UiInteractionPlugin",
        note: "boyko_ui — the pointer model's App entry point.",
    },
    RecordedPlugin {
        ty: "UiBindingPlugin",
        note: "boyko_ui — its own doc example writes `app.add_plugins(UiBindingPlugin::default())` \
               in a `///` comment, which is the exact shape a grep-shaped census scores as a \
               registration.",
    },
    RecordedPlugin {
        ty: "UiWidgetsPlugin",
        note: "boyko_ui — the widget layer's App entry point.",
    },
    RecordedPlugin {
        ty: "ProfilingOverlayPlugin",
        note: "boyko_ui — the profiler's on-screen overlay: the last of the fifteen profiler rungs, \
               and it is dead for the SECOND time, now because the crate that hosts it is.",
    },
];

/// One entry point documented as "the host calls this", pinned to the file that defines it.
struct HostSeam {
    /// The function name, as `fn <name>` spells it.
    symbol: &'static str,
    /// Repo-relative path of the file that must define it. Pinned so a renamed or deleted seam
    /// reds with "the roster is stale" rather than silently reporting itself dead.
    defined_in: &'static str,
    /// What the seam is, and who is supposed to cross it.
    reason: &'static str,
}

/// The declared host seams. Fourteen rows — deliberately small, deliberately hand-maintained.
///
/// Six are healthy and six-plus-two are the UI chain, so the class proves BOTH directions on every
/// run: a class whose every row is in the defect baseline never demonstrates that it can go green,
/// and one with no red row never demonstrates that it can go red.
const HOST_SEAMS: [HostSeam; 14] = [
    // ── Healthy: `boyko_app` crosses these every frame or at boot ──────────────────────────────
    HostSeam {
        symbol: "upload_light_table",
        defined_in: "crates/boyko_render/src/upload.rs",
        reason: "per-frame light-table staging upload; crossed from boyko_app::runner",
    },
    HostSeam {
        symbol: "upload_sdf_edit_list",
        defined_in: "crates/boyko_render/src/upload.rs",
        reason: "one-shot boot-static SDF edit-list upload (host plan R7)",
    },
    HostSeam {
        symbol: "upload_particle_emit_requests",
        defined_in: "crates/boyko_render/src/upload.rs",
        reason: "per-frame particle emit staging. This row's own note used to say the host called \
                 it 'each frame' while ParticlePlugin was unregistered, so the GPU half read \
                 staging no plugin inserted -- OVERSTATED, and in the direction that makes the \
                 defect sound survivable. The call sits inside particle_upload_slots(s).map(..), \
                 which was None in every production host: the plugin was unregistered, so \
                 ParticleConfig was absent, so the bundle was never built, so this block never \
                 ran. What it cost was worse than a dead upload -- the moment an owner supplied \
                 ParticleConfig ALONE (the arming convention plugins.rs documents), the bundle WAS \
                 built and the block's first line panicked on the missing ParticleClock. Both are \
                 closed: the plugin is composed by EnginePlugins::build.",
    },
    HostSeam {
        symbol: "upload_mesh_assets",
        defined_in: "crates/boyko_render/src/gpu_upload.rs",
        reason: "mesh asset residency upload; crossed from boyko_app::runner",
    },
    HostSeam {
        symbol: "sync_instance_model_cols",
        defined_in: "crates/boyko_render/src/instance_model.rs",
        reason: "a pub SYSTEM, not a call — registered via `b.add_system(…)` in boyko_app's \
                 plugin composition. On the roster so the class covers the by-value shape too.",
    },
    HostSeam {
        symbol: "resolve_render_path",
        defined_in: "crates/boyko_render/src/render_path_config.rs",
        reason: "boot-time render-path resolve, called directly by boyko_app::runner",
    },
    // ── The UI chain: every row measured dead at 59009f8a ──────────────────────────────────────
    HostSeam {
        symbol: "host_upload_frame_from_world",
        defined_in: "crates/boyko_render/src/ui/upload.rs",
        reason: "documented at :180-182 as 'the single call the render host makes each frame'",
    },
    HostSeam {
        symbol: "host_upload_frame",
        defined_in: "crates/boyko_render/src/ui/upload.rs",
        reason: "the world-free sibling of the above: read slot -> fence -> pack_sort_upload",
    },
    HostSeam {
        symbol: "ui_setup",
        defined_in: "crates/boyko_render/src/gpu_column.rs",
        reason: "RhiContext UI capability: builds the pipeline + per-slot rings",
    },
    HostSeam {
        symbol: "ui_upload",
        defined_in: "crates/boyko_render/src/gpu_column.rs",
        reason: "RhiContext UI capability: memcpys the packed instances into the slot",
    },
    HostSeam {
        symbol: "ui_handles",
        defined_in: "crates/boyko_render/src/gpu_column.rs",
        reason: "RhiContext UI capability: the per-frame pipeline + bind-group pair",
    },
    HostSeam {
        symbol: "ui_pass",
        defined_in: "crates/boyko_render/src/gpu_column.rs",
        reason: "RhiContext UI capability: the pass the host records the UI draw into",
    },
    HostSeam {
        symbol: "pack_ui_instance",
        defined_in: "crates/boyko_render/src/ui/pack.rs",
        reason: "the CPU pack. It HAS a same-crate caller (pack_sort_upload) — which is itself \
                 only reachable through the dead host_upload_frame*, so the chain is dead inside \
                 one crate and only the cross-crate rule sees it.",
    },
    HostSeam {
        symbol: "record_ui_rects",
        defined_in: "crates/boyko_render/src/ui/draw.rs",
        reason: "the shared RhiApi-generic one-draw recorder (GUI P5a rung 5)",
    },
];

/// The measured class-3 baseline at `59009f8a`, asserted for exact equality.
const KNOWN_DEAD_SEAMS: [&str; 8] = [
    "host_upload_frame",
    "host_upload_frame_from_world",
    "pack_ui_instance",
    "record_ui_rects",
    "ui_handles",
    "ui_pass",
    "ui_setup",
    "ui_upload",
];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Non-vacuity floors
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Members the workspace must have. A parser that stopped parsing empties every set and reports a
/// triumphant green; the floor catches the dead scan, and the roster equality below catches the
/// scan that reads the wrong rows. Stood at 29 when the baseline was taken.
const MIN_WORKSPACE_MEMBERS: usize = 25;

/// Production `.rs` files the walker must find. Stood at 701 (709 under `src`, minus the 8 files
/// owned by a `#[cfg(test)]`-gated `mod` declaration).
const MIN_PRODUCTION_FILES: usize = 600;

/// `impl Plugin for …` sites the roster scan must recover. Stood at 27. A reformatted `impl`
/// header the line scan stops recognising would otherwise empty class 2 silently.
const MIN_PLUGINS: usize = 20;

/// `add_plugin`/`add_plugins` argument identifiers the registration scan must recover. Stood at 18.
const MIN_REGISTERED_IDENTS: usize = 12;

/// `#[cfg(test)]`-gated items the stripper must remove. Stood at 369. A stripper that matched
/// nothing would score every `#[cfg(test)] mod tests` registration as production.
const MIN_CFG_TEST_ITEMS_STRIPPED: usize = 250;

/// Files the gated-`mod` resolution must exclude. Stood at 8. Zero here means shape 2 of the
/// exclusion silently stopped working, which is precisely the "gate that cannot fail" state.
const MIN_TEST_ONLY_FILES: usize = 4;

/// A file that must be excluded as test-only, as a named positive control.
///
/// It is the `ProfilerPlugin` precedent's own file: it contains two `app.add_plugin(ProfilerPlugin)`
/// calls and is reached only through `#[cfg(test)] mod tests;` in its parent `mod.rs`. If it is
/// ever moved, re-point this constant in the same commit — it is a control, not a fact about the
/// profiler.
const TEST_ONLY_CONTROL_FILE: &str = "crates/boyko_ecs/src/ecs/core/profiling/tests.rs";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Source lexing — comments, strings and `#[cfg(test)]` regions
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The repository root. `CARGO_MANIFEST_DIR` IS the workspace root for the `boyko-engine` package.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Blanks `[from, to)` with spaces, preserving newlines so byte offsets AND line numbers survive.
fn blank(buf: &mut [u8], from: usize, to: usize) {
    let end = to.min(buf.len());
    for b in &mut buf[from..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Whether `b` may appear inside a Rust identifier.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 1-based line number of byte offset `at`.
fn line_of(src: &[u8], at: usize) -> usize {
    src[..at.min(src.len())].iter().filter(|b| **b == b'\n').count() + 1
}

/// Replaces every comment body and every string / char literal with spaces.
///
/// This is the pass that stops `/// app.add_plugins(UiBindingPlugin::default())` — a real line at
/// `crates/boyko_ui/src/interaction/plugin.rs:141` — from registering a plugin, and stops a `#` or
/// a brace inside a string literal from unbalancing the `#[cfg(test)]` stripper. Byte-level and
/// length-preserving, so every offset taken afterwards still indexes the original text.
fn blank_comments_and_strings(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    let n = src.len();
    let mut i = 0usize;
    while i < n {
        let c = src[i];
        if c == b'/' && i + 1 < n && src[i + 1] == b'/' {
            let end = src[i..].iter().position(|b| *b == b'\n').map_or(n, |p| i + p);
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if c == b'/' && i + 1 < n && src[i + 1] == b'*' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < n && depth > 0 {
                if src[j] == b'/' && j + 1 < n && src[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if src[j] == b'*' && j + 1 < n && src[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j.min(n));
            i = j;
            continue;
        }
        // Raw string: `r`, then zero or more `#`, then `"`. Terminated by `"` + the same `#` run.
        if c == b'r' && i + 1 < n && (src[i + 1] == b'#' || src[i + 1] == b'"') {
            let before_is_ident = i > 0 && is_ident_byte(src[i - 1]);
            let mut k = i + 1;
            while k < n && src[k] == b'#' {
                k += 1;
            }
            if !before_is_ident && k < n && src[k] == b'"' {
                let hashes = k - i - 1;
                let mut j = k + 1;
                let mut end = n;
                while j < n {
                    if src[j] == b'"' && src[j + 1..].iter().take(hashes).all(|b| *b == b'#') {
                        end = (j + 1 + hashes).min(n);
                        break;
                    }
                    j += 1;
                }
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        if c == b'"' {
            let mut j = i + 1;
            while j < n {
                if src[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if src[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            blank(&mut out, i, j.min(n));
            i = j;
            continue;
        }
        // A char literal, but NOT a lifetime (`'a`, `'static`): require a closing quote within 4
        // bytes, which every `'x'` / `'\n'` / `'\u{1}'`-free form in this tree satisfies.
        if c == b'\'' {
            let close = if i + 2 < n && src[i + 1] == b'\\' {
                src[i + 2..].iter().take(4).position(|b| *b == b'\'').map(|p| i + 3 + p)
            } else if i + 2 < n && src[i + 2] == b'\'' {
                Some(i + 3)
            } else {
                None
            };
            if let Some(end) = close {
                blank(&mut out, i, end.min(n));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// What one file's `#[cfg(test)]` strip removed.
struct CfgTestStrip {
    /// Number of gated items blanked.
    items: usize,
    /// Module names declared as `#[cfg(test)] mod NAME;` — each names a whole FILE that is
    /// test-only and must leave the production set.
    gated_file_mods: Vec<String>,
    /// Values of `#[path = "…"]` attributes inside a gated item, read from the RAW text because
    /// the blanking pass has already emptied the string.
    gated_path_overrides: Vec<String>,
}

/// Blanks every item annotated with a `cfg` predicate naming `test`, in place.
///
/// `blanked` must already have been through [`blank_comments_and_strings`] — brace balance is only
/// sound once a `{` inside a string cannot be seen — and `raw` must be the same file untouched, so
/// `#[path = "…"]` values are still readable at the same offsets.
///
/// The predicate rule is "the word `test` appears in the `#[cfg(…)]` predicate". Measured over this
/// tree that is exactly right for all five shapes present (`test`, `all(test, …)`,
/// `any(test, feature = "goldens")`, `any(test, feature = "test-readback")`,
/// `all(feature = "ke16-a5", test)`), and it would be WRONG for `not(test)`, which does not occur —
/// [`the_cfg_test_rule_has_no_not_test_counterexample_in_the_tree`] keeps it that way.
fn strip_cfg_test(blanked: &mut [u8], raw: &[u8]) -> CfgTestStrip {
    let n = blanked.len();
    let src = blanked.to_owned();
    let mut out = CfgTestStrip { items: 0, gated_file_mods: Vec::new(), gated_path_overrides: Vec::new() };
    let mut i = 0usize;
    while i + 6 < n {
        if &src[i..i + 6] != b"#[cfg(" {
            i += 1;
            continue;
        }
        // Balanced `(` … `)` of the predicate.
        let mut depth = 0usize;
        let mut j = i + 5;
        while j < n {
            match src[j] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if j >= n || !contains_word(&src[i..=j], b"test") {
            i += 1;
            continue;
        }
        let Some(rb) = src[j..].iter().position(|b| *b == b']').map(|p| j + p + 1) else {
            i += 1;
            continue;
        };
        // The item runs to its `{ … }` block or to its `;`, whichever comes first.
        let mut p = rb;
        while p < n && src[p] != b'{' && src[p] != b';' {
            p += 1;
        }
        if p >= n {
            i += 1;
            continue;
        }
        let end = if src[p] == b';' {
            p + 1
        } else {
            let mut d = 0usize;
            let mut q = p;
            while q < n {
                match src[q] {
                    b'{' => d += 1,
                    b'}' => {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                q += 1;
            }
            (q + 1).min(n)
        };
        let item = &src[rb..end];
        if src[p] == b';'
            && let Some(name) = declared_mod_name(item)
        {
            out.gated_file_mods.push(name);
            if let Some(path) = path_attribute(&raw[rb..end]) {
                out.gated_path_overrides.push(path);
            }
        }
        blank(blanked, i, end);
        out.items += 1;
        i = end;
    }
    out
}

/// Whether `hay` contains `word` delimited by non-identifier bytes.
fn contains_word(hay: &[u8], word: &[u8]) -> bool {
    find_word(hay, word, 0).is_some()
}

/// Offset of the next occurrence of `word` in `hay` at or after `from`, delimited by non-identifier
/// bytes on both sides.
fn find_word(hay: &[u8], word: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + word.len() <= hay.len() {
        if &hay[i..i + word.len()] == word
            && (i == 0 || !is_ident_byte(hay[i - 1]))
            && (i + word.len() == hay.len() || !is_ident_byte(hay[i + word.len()]))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `mod NAME;` written directly in `item` (not inside a nested block) — the file-module shape.
fn declared_mod_name(item: &[u8]) -> Option<String> {
    let at = find_word(item, b"mod", 0)?;
    if item[..at].contains(&b'{') {
        return None;
    }
    let mut i = at + 3;
    while i < item.len() && item[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    while i < item.len() && is_ident_byte(item[i]) {
        i += 1;
    }
    if i == start { None } else { String::from_utf8(item[start..i].to_vec()).ok() }
}

/// The value of a `#[path = "…"]` attribute in `raw_item`, if any.
fn path_attribute(raw_item: &[u8]) -> Option<String> {
    let at = find_word(raw_item, b"path", 0)?;
    let q1 = at + raw_item[at..].iter().position(|b| *b == b'"')?;
    let q2 = q1 + 1 + raw_item[q1 + 1..].iter().position(|b| *b == b'"')?;
    String::from_utf8(raw_item[q1 + 1..q2].to_vec()).ok()
}

/// Blanks every `use … ;` declaration.
///
/// An import is not a use of a symbol — `-D warnings` already fails an unused one — and counting a
/// `pub use` re-export as a reference would score every re-exported symbol as live. Applied only
/// to the class-3 reference scan; class 2 reads `add_plugin(…)` arguments, where a `use` cannot
/// appear.
fn blank_use_declarations(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    let mut i = 0usize;
    while let Some(at) = find_word(src, b"use", i) {
        // `.use` / `::use` cannot start a declaration; `use` is a keyword so no identifier ends here.
        let prev = src[..at].iter().rposition(|b| !b.is_ascii_whitespace());
        let is_decl = prev.is_none_or(|p| src[p] != b'.');
        if is_decl {
            let end = src[at..].iter().position(|b| *b == b';').map_or(src.len(), |p| at + p + 1);
            blank(&mut out, at, end);
            i = end;
        } else {
            i = at + 3;
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The source census
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One `.rs` file under a member's `src/`, as the census sees it.
#[derive(Clone)]
struct SourceFile {
    /// Repo-relative path with `/` separators.
    path: String,
    /// Owning member directory name (`boyko_render`, `prof_decode`, `.` for the root package).
    member_dir: String,
    /// Raw file text.
    text: String,
}

/// Everything the source pass recovered. Returned rather than asserted inside the scan so the
/// sensitivity controls can run the same code over mutated input.
struct SourceCensus {
    /// Files dropped because a `#[cfg(test)]`-gated `mod` declaration owns them.
    test_only_files: BTreeSet<String>,
    /// Files actually scanned as production.
    production_files: usize,
    /// `#[cfg(test)]`-gated items blanked, across all files.
    cfg_test_items: usize,
    /// Plugin type -> `path:line` of its `impl Plugin for` in production source.
    plugins: BTreeMap<String, String>,
    /// Capitalised identifier -> production `add_plugin(s)` sites naming it.
    registrations: BTreeMap<String, Vec<String>>,
    /// Seam symbol -> production files declaring `fn <symbol>`.
    seam_decls: BTreeMap<String, Vec<String>>,
    /// Seam symbol -> production sites naming it outside its defining member.
    seam_refs: BTreeMap<String, Vec<String>>,
}

/// Every `.rs` file under `src/`, `crates/*/src/` and `tools/*/src/`.
///
/// `tests/`, `benches/` and `examples/` are never walked: by construction nothing in them can make
/// a symbol production-reachable, and walking them is how a census reports a test fixture as a
/// shipped path.
fn live_sources() -> Vec<SourceFile> {
    let root = repo_root();
    let mut roots: Vec<(String, PathBuf)> = vec![(".".to_string(), root.join("src"))];
    for group in ["crates", "tools"] {
        let dir = root.join(group);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut members: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        members.sort();
        for m in members {
            let src = m.join("src");
            if src.is_dir() {
                let name = m.file_name().expect("invariant: a read_dir entry has a file name");
                roots.push((name.to_string_lossy().into_owned(), src));
            }
        }
    }
    let mut files = Vec::new();
    for (member_dir, dir) in roots {
        collect_rs(&dir, &root, &member_dir, &mut files);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Recursive `.rs` walk, pushing repo-relative paths.
fn collect_rs(dir: &Path, root: &Path, member_dir: &str, out: &mut Vec<SourceFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_rs(&p, root, member_dir, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            let rel = p
                .strip_prefix(root)
                .expect("invariant: the walk starts inside the repo root")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
            out.push(SourceFile { path: rel, member_dir: member_dir.to_string(), text });
        }
    }
}

/// Runs the whole source pass over supplied files, so a control can inject a defect without
/// touching anything on disk.
fn scan_sources(files: &[SourceFile]) -> SourceCensus {
    let present: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();

    // Pass 1 — blank comments/strings, strip `#[cfg(test)]`, and learn which FILES are test-only.
    let mut stripped: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    let mut test_only: BTreeSet<String> = BTreeSet::new();
    let mut cfg_test_items = 0usize;
    for f in files {
        let raw = f.text.as_bytes();
        let mut buf = blank_comments_and_strings(raw);
        let strip = strip_cfg_test(&mut buf, raw);
        cfg_test_items += strip.items;
        for cand in gated_module_files(&f.path, &strip) {
            if present.contains(cand.as_str()) {
                test_only.insert(cand);
            }
        }
        stripped.insert(f.path.as_str(), buf);
    }
    // A test-only module's own descendants are test-only too.
    loop {
        let grown: Vec<String> = test_only
            .iter()
            .flat_map(|f| {
                let base = f.strip_suffix("/mod.rs").map(str::to_string).unwrap_or_else(|| {
                    f.strip_suffix(".rs").expect("invariant: a walked file ends in .rs").to_string()
                });
                files
                    .iter()
                    .map(|g| g.path.clone())
                    .filter(move |p| p.starts_with(&format!("{base}/")))
            })
            .filter(|p| !test_only.contains(p))
            .collect();
        if grown.is_empty() {
            break;
        }
        test_only.extend(grown);
    }

    let production: Vec<&SourceFile> = files.iter().filter(|f| !test_only.contains(&f.path)).collect();

    // Pass 2 — the plugin roster and the registration set.
    let mut plugins: BTreeMap<String, String> = BTreeMap::new();
    let mut registrations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &production {
        let buf = &stripped[f.path.as_str()];
        for (name, off) in plugin_impls(buf) {
            plugins.entry(name).or_insert_with(|| format!("{}:{}", f.path, line_of(buf, off)));
        }
        for (name, off) in registered_idents(buf) {
            registrations
                .entry(name)
                .or_default()
                .push(format!("{}:{}", f.path, line_of(buf, off)));
        }
    }

    // Pass 3 — the host seams: where each is declared, and who names it from another member.
    let mut seam_decls: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut seam_refs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let no_uses: BTreeMap<&str, Vec<u8>> =
        production.iter().map(|f| (f.path.as_str(), blank_use_declarations(&stripped[f.path.as_str()]))).collect();
    for seam in HOST_SEAMS {
        let mut decl_members: BTreeSet<&str> = BTreeSet::new();
        for f in &production {
            let buf = &stripped[f.path.as_str()];
            if let Some(off) = fn_declaration(buf, seam.symbol.as_bytes()) {
                seam_decls
                    .entry(seam.symbol.to_string())
                    .or_default()
                    .push(format!("{}:{}", f.path, line_of(buf, off)));
                decl_members.insert(f.member_dir.as_str());
            }
        }
        for f in &production {
            if decl_members.contains(f.member_dir.as_str()) {
                continue;
            }
            let buf = &no_uses[f.path.as_str()];
            let mut at = 0usize;
            while let Some(off) = find_word(buf, seam.symbol.as_bytes(), at) {
                seam_refs
                    .entry(seam.symbol.to_string())
                    .or_default()
                    .push(format!("{}:{}", f.path, line_of(buf, off)));
                at = off + seam.symbol.len();
            }
        }
    }

    SourceCensus {
        test_only_files: test_only,
        production_files: production.len(),
        cfg_test_items,
        plugins,
        registrations,
        seam_decls,
        seam_refs,
    }
}

/// Candidate file paths for the modules a `#[cfg(test)]`-gated declaration owns.
///
/// Three resolutions, matching rustc's: an explicit `#[path = "…"]` relative to the declaring
/// file's directory; a sibling `NAME.rs` / `NAME/mod.rs` when the declaring file is
/// `lib.rs` / `main.rs` / `mod.rs`; and a child `<stem>/NAME.rs` / `<stem>/NAME/mod.rs` otherwise.
fn gated_module_files(declaring: &str, strip: &CfgTestStrip) -> Vec<String> {
    let dir = declaring.rsplit_once('/').map_or("", |(d, _)| d);
    let stem = declaring
        .rsplit_once('/')
        .map_or(declaring, |(_, f)| f)
        .strip_suffix(".rs")
        .unwrap_or("");
    let owns_siblings = matches!(stem, "lib" | "main" | "mod");
    let child_dir = if owns_siblings { dir.to_string() } else { format!("{dir}/{stem}") };
    let mut out = Vec::new();
    for p in &strip.gated_path_overrides {
        out.push(format!("{dir}/{p}"));
    }
    for name in &strip.gated_file_mods {
        out.push(format!("{child_dir}/{name}.rs"));
        out.push(format!("{child_dir}/{name}/mod.rs"));
    }
    out
}

/// Every `impl … Plugin for NAME` in a stripped buffer, with the offset of `NAME`.
///
/// Line-shaped on purpose: all 27 sites in this tree write the header on one line, including the
/// two generic ones (`impl<A: Actionlike> Plugin for InputPlugin<A>`), and [`MIN_PLUGINS`] reds if
/// that stops being true. A multi-line-tolerant scan would have to decide where an `impl` header
/// ends, which is more machinery than the shape justifies.
fn plugin_impls(buf: &[u8]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut line_start = 0usize;
    for line_end in buf
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .map(|(i, _)| i)
        .chain(std::iter::once(buf.len()))
    {
        let line = &buf[line_start..line_end];
        let trimmed_at = line.iter().position(|b| !b.is_ascii_whitespace());
        if let Some(t) = trimmed_at
            && line[t..].starts_with(b"impl")
            && let Some(at) = find_pattern(line, b"Plugin for ")
        {
            let mut i = at + b"Plugin for ".len();
            let start = i;
            while i < line.len() && is_ident_byte(line[i]) {
                i += 1;
            }
            if i > start
                && let Ok(name) = std::str::from_utf8(&line[start..i])
            {
                out.push((name.to_string(), line_start + start));
            }
        }
        line_start = line_end + 1;
    }
    out
}

/// Offset of `needle` in `hay`, plain substring.
fn find_pattern(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Every capitalised identifier appearing inside an `add_plugin(…)` / `add_plugins(…)` argument
/// list, with its offset.
///
/// Balanced-paren extraction over the blanked buffer, so nested tuples
/// (`add_plugins((A, B::<C>::new(), D))`) and path-qualified names (`boyko_render::AaPlugin`)
/// both resolve without a grammar. Lowercase idents are dropped: a path segment or a method name
/// is not a plugin.
fn registered_idents(buf: &[u8]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(at) = find_word_prefixed(buf, b"add_plugin", i) {
        let mut j = at + b"add_plugin".len();
        if j < buf.len() && buf[j] == b's' {
            j += 1;
        }
        while j < buf.len() && buf[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= buf.len() || buf[j] != b'(' {
            i = at + 1;
            continue;
        }
        let mut depth = 0usize;
        let mut k = j;
        while k < buf.len() {
            match buf[k] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let arg_end = k.min(buf.len());
        let mut p = j + 1;
        while p < arg_end {
            if is_ident_byte(buf[p]) && (p == 0 || !is_ident_byte(buf[p - 1])) {
                let start = p;
                while p < arg_end && is_ident_byte(buf[p]) {
                    p += 1;
                }
                if buf[start].is_ascii_uppercase()
                    && let Ok(name) = std::str::from_utf8(&buf[start..p])
                {
                    out.push((name.to_string(), start));
                }
            } else {
                p += 1;
            }
        }
        i = arg_end;
    }
    out
}

/// Offset of `stem` at or after `from` where the byte BEFORE it is not an identifier byte. Unlike
/// [`find_word`] the byte after may be one, so `add_plugin` also finds `add_plugins`.
fn find_word_prefixed(hay: &[u8], stem: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + stem.len() <= hay.len() {
        if &hay[i..i + stem.len()] == stem && (i == 0 || !is_ident_byte(hay[i - 1])) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Offset of `fn <name>` in a stripped buffer — the declaration, not a call.
fn fn_declaration(buf: &[u8], name: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    while let Some(at) = find_word(buf, b"fn", i) {
        let mut j = at + 2;
        while j < buf.len() && buf[j].is_ascii_whitespace() {
            j += 1;
        }
        if buf[j..].starts_with(name)
            && (j + name.len() == buf.len() || !is_ident_byte(buf[j + name.len()]))
        {
            return Some(j);
        }
        i = at + 2;
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The manifest census
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One workspace member's manifest.
#[derive(Clone)]
struct Manifest {
    /// Repo-relative directory, `.` for the root package.
    dir: String,
    /// The `[package] name`.
    package: String,
    /// Raw manifest text.
    text: String,
}

/// Which dependency table an edge came from. `Build` is folded with `Dev` for classification: a
/// build-script dependency is not shipped in the binary either.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DepKind {
    /// `[dependencies]` / `[target.*.dependencies]` — the shipped edge.
    Normal,
    /// `[dev-dependencies]` / `[build-dependencies]` and their target-qualified forms.
    NotShipped,
}

/// The member directories the root manifest's `[workspace] members` array names, plus `.`.
///
/// **The census walks exactly this set and no other**, which is the answer to "prove your member
/// set is the complete one": it is not derived from a directory listing (a crate absent from the
/// array is not a member, however many `Cargo.toml`s exist), and it is not derived from
/// `default-members` (the 2026-07-23 audit found that array hiding the whole CI). The count and
/// the names are printed by [`the_member_set_the_census_walks_is_the_workspace_member_set`].
fn workspace_member_dirs() -> Vec<String> {
    let root = repo_root();
    let text = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap_or_else(|e| panic!("cannot read the workspace manifest: {e}"));
    let bytes = text.as_bytes();
    let at = find_pattern(bytes, b"members = [")
        .expect("invariant: the workspace manifest declares a `members = [` array");
    let end = at + bytes[at..]
        .iter()
        .position(|b| *b == b']')
        .expect("invariant: the `members` array is closed");
    let mut dirs: Vec<String> = text[at..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    dirs.push(".".to_string());
    dirs
}

/// Reads every member manifest named by [`workspace_member_dirs`].
fn live_manifests() -> Vec<Manifest> {
    let root = repo_root();
    workspace_member_dirs()
        .into_iter()
        .map(|dir| {
            let path = root.join(&dir).join("Cargo.toml");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let package = package_name(&text)
                .unwrap_or_else(|| panic!("{} has no [package] name", path.display()));
            Manifest { dir, package, text }
        })
        .collect()
}

/// The `[package] name`, read the way Cargo reads it: the first `name =` row inside `[package]`,
/// ignoring the `name =` rows `[lib]` / `[[bin]]` / `[[bench]]` carry further down.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && line.starts_with("name") {
            return Some(line.split_once('=')?.1.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Every in-workspace edge a manifest declares, as `(target member dir, kind)`.
///
/// Keyed on the resolved `path = "…"`, not on the dependency's spelled name: this workspace renames
/// (`boyko-ecs` lives in `crates/boyko_ecs`) and a `package = "…"` key would need special-casing.
/// A path is only an edge when it appears in a dependency table — `[lib] path = "src/lib.rs"` and
/// the seven `[[bin]] path = …` rows must not become edges, which is why table tracking is not
/// optional.
fn manifest_edges(m: &Manifest, by_dir: &BTreeSet<String>) -> Vec<(String, DepKind)> {
    let mut edges = Vec::new();
    let mut kind: Option<DepKind> = None;
    for line in m.text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let table = t.trim_matches(['[', ']'].as_slice());
            kind = if table.starts_with("workspace") {
                // `[workspace.dependencies]` is a version-pin table, never an edge.
                None
            } else if table.ends_with("dev-dependencies") || table.ends_with("build-dependencies") {
                Some(DepKind::NotShipped)
            } else if table.ends_with("dependencies") {
                Some(DepKind::Normal)
            } else {
                None
            };
            continue;
        }
        let Some(k) = kind else { continue };
        let Some(rest) = t.split_once("path").map(|(_, r)| r) else { continue };
        let Some(open) = rest.find('"') else { continue };
        let Some(close) = rest[open + 1..].find('"') else { continue };
        let target = normalise_join(&m.dir, &rest[open + 1..open + 1 + close]);
        if target != m.dir && by_dir.contains(&target) {
            edges.push((target, k));
        }
    }
    edges
}

/// Joins a repo-relative directory with a possibly-`..` relative path and normalises it.
fn normalise_join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in base.split('/').chain(rel.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() { ".".to_string() } else { parts.join("/") }
}

/// Classifies every member against the declared roots.
fn classify_members(manifests: &[Manifest]) -> BTreeMap<String, Reach> {
    let by_dir: BTreeSet<String> = manifests.iter().map(|m| m.dir.clone()).collect();
    let mut normal: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut rev_normal: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut rev_not_shipped: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for m in manifests {
        for (target, kind) in manifest_edges(m, &by_dir) {
            match kind {
                DepKind::Normal => {
                    normal.entry(m.dir.as_str()).or_default().push(target.clone());
                    rev_normal.entry(target).or_default().push(m.dir.as_str());
                }
                DepKind::NotShipped => {
                    rev_not_shipped.entry(target).or_default().push(m.dir.as_str());
                }
            }
        }
    }

    let roots: BTreeSet<&str> = PRODUCT_ROOTS.iter().map(|r| r.package).collect();
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = manifests
        .iter()
        .filter(|m| roots.contains(m.package.as_str()))
        .map(|m| m.dir.clone())
        .collect();
    while let Some(dir) = queue.pop() {
        if !reachable.insert(dir.clone()) {
            continue;
        }
        for next in normal.get(dir.as_str()).into_iter().flatten() {
            queue.push(next.clone());
        }
    }

    manifests
        .iter()
        .map(|m| {
            let class = if roots.contains(m.package.as_str()) {
                Reach::ProductRoot
            } else if reachable.contains(&m.dir) {
                Reach::ProductionReachable
            } else if rev_normal.get(&m.dir).is_some_and(|v| !v.is_empty()) {
                Reach::ReferencedOnlyByUnreachable
            } else if rev_not_shipped.get(&m.dir).is_some_and(|v| !v.is_empty()) {
                Reach::DevOnlyReferenced
            } else {
                Reach::Unreferenced
            };
            (m.package.clone(), class)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung 1 — the gate
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Proves which member set the census walks, and that it is the workspace's own.
///
/// The two traps this workspace has already paid for: `cargo check --all-targets` at a virtual
/// manifest is vacuously green, and `default-members` once hid the whole CI. Neither can bite here
/// because the set comes from `[workspace] members` and is printed in full on every run.
#[test]
fn the_member_set_the_census_walks_is_the_workspace_member_set() {
    let manifests = live_manifests();
    println!("workspace members walked: {}", manifests.len());
    for m in &manifests {
        println!("  {:<24} {}", m.package, m.dir);
    }
    assert!(
        manifests.len() >= MIN_WORKSPACE_MEMBERS,
        "the census walked only {} members. Either the `[workspace] members` array shrank by more \
         than the floor allows, or the array parser stopped parsing — the second empties every \
         class below and reports green.",
        manifests.len()
    );
    let root = repo_root();
    for m in &manifests {
        assert!(
            root.join(&m.dir).join("Cargo.toml").is_file(),
            "`[workspace] members` names {} but {}/Cargo.toml does not exist",
            m.package,
            m.dir
        );
    }
    let names: BTreeSet<&str> = manifests.iter().map(|m| m.package.as_str()).collect();
    assert_eq!(
        names.len(),
        manifests.len(),
        "two workspace members share a [package] name; every classification below is keyed on it"
    );
    for r in PRODUCT_ROOTS {
        assert!(
            names.contains(r.package),
            "PRODUCT_ROOTS names `{}`, which no workspace member is called. Reachability is \
             measured FROM this roster, so a row naming nothing silently shrinks the reachable set \
             and manufactures unreachable members. Reason on the row: {}",
            r.package,
            r.reason
        );
    }
    for r in KNOWN_UNREACHABLE_MEMBERS {
        assert!(
            names.contains(r.package),
            "KNOWN_UNREACHABLE_MEMBERS names `{}`, which no workspace member is called — the \
             member was renamed or removed. Strike the row in the same commit.",
            r.package
        );
    }
}

/// **Class 1.** Every member is a declared product, production-reachable from one, or recorded.
#[test]
fn every_workspace_member_is_shipped_or_recorded() {
    let manifests = live_manifests();
    let classes = classify_members(&manifests);

    println!("── production reachability, by member ──");
    for (package, class) in &classes {
        println!("  {package:<24} {class:?}");
    }
    println!("── recorded unreachable members, and what was measured ──");
    for r in KNOWN_UNREACHABLE_MEMBERS {
        println!("  {:<24} {:?}
      {}", r.package, r.class, r.note);
    }

    let recorded: BTreeMap<&str, Reach> =
        KNOWN_UNREACHABLE_MEMBERS.iter().map(|r| (r.package, r.class)).collect();
    let actual: BTreeSet<String> = classes
        .iter()
        .filter(|(_, c)| !matches!(c, Reach::ProductRoot | Reach::ProductionReachable))
        .map(|(p, c)| format!("{p} = {c:?}"))
        .collect();
    let expected: BTreeSet<String> =
        recorded.iter().map(|(p, c)| format!("{p} = {c:?}")).collect();

    let unexpected: Vec<&String> = actual.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "NEW unreachable workspace member(s): {unexpected:?}\n\n\
         A member classified `DevOnlyReferenced` is shipped by NOTHING: every reverse edge to it \
         is a `[dev-dependencies]` one, so it is compiled only when something's tests are built. \
         `Unreferenced` is the stronger form — no member names it at all. This is the class that \
         let `boyko_ui` sit complete and undrawn for a whole campaign.\n\n\
         Either wire it into a product's `[dependencies]`, or add a row to \
         KNOWN_UNREACHABLE_MEMBERS with what was measured — or, if it is genuinely a product or a \
         tool, a row in PRODUCT_ROOTS with the argument for why. An entry whose reason is 'it \
         fails' is not a reason.\n\
         Full classification is printed above (`-- --nocapture`)."
    );
    let repaired: Vec<&String> = expected.difference(&actual).collect();
    assert!(
        repaired.is_empty(),
        "recorded unreachable member(s) {repaired:?} no longer hold the class this file records. \
         If they were wired up, delete the KNOWN_UNREACHABLE_MEMBERS row in the same commit; if \
         the class merely CHANGED, update it. The baseline is asserted for exact equality in both \
         directions on purpose — a `<=` could not tell 'repaired' from 'the scanner stopped \
         seeing it'."
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung 2 — the gates
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// **Class 2.** Every plugin declared in production source is registered from production source.
///
/// This is the `ProfilerPlugin` gate: fifteen rungs of a subsystem whose `Plugin::build` was never
/// called by any host, while its own tests called it twice.
#[test]
fn every_production_plugin_has_a_production_registration() {
    let census = scan_sources(&live_sources());
    let exempt: BTreeSet<&str> = EXEMPT_PLUGINS.iter().map(|p| p.ty).collect();
    let recorded: BTreeSet<&str> = KNOWN_UNREGISTERED_PLUGINS.iter().map(|p| p.ty).collect();

    println!("── plugin registration, by type ──");
    let mut unregistered: BTreeSet<String> = BTreeSet::new();
    for (ty, decl) in &census.plugins {
        let sites = census.registrations.get(ty).map(Vec::as_slice).unwrap_or_default();
        println!("  {:<24} {:<8} {decl}  {:?}", ty, if sites.is_empty() { "DEAD" } else { "ok" }, sites);
        if sites.is_empty() && !exempt.contains(ty.as_str()) {
            unregistered.insert(format!("{ty} ({decl})"));
        }
    }

    for p in EXEMPT_PLUGINS {
        assert!(
            census.plugins.contains_key(p.ty),
            "EXEMPT_PLUGINS names `{}`, which no production `impl Plugin for` declares — the type \
             was renamed or deleted, and the exemption is now silently covering nothing. Reason on \
             the row: {}",
            p.ty,
            p.reason
        );
    }
    for p in KNOWN_UNREGISTERED_PLUGINS {
        assert!(
            census.plugins.contains_key(p.ty),
            "KNOWN_UNREGISTERED_PLUGINS names `{}`, which no production `impl Plugin for` \
             declares. Strike the row in the same commit as the removal.",
            p.ty
        );
    }

    println!("── recorded unregistered plugins, and what was measured ──");
    for p in KNOWN_UNREGISTERED_PLUGINS {
        println!("  {:<24}
      {}", p.ty, p.note);
    }

    let expected: BTreeSet<String> = census
        .plugins
        .iter()
        .filter(|(ty, _)| recorded.contains(ty.as_str()))
        .map(|(ty, decl)| format!("{ty} ({decl})"))
        .collect();
    let unexpected: Vec<&String> = unregistered.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "plugin type(s) with NO production registration: {unexpected:?}\n\n\
         A `Plugin` nothing adds is a `build()` nothing calls: every resource it inserts is absent \
         and every system it registers never runs, while its own tests — which construct an `App` \
         and add it — pass. Registration means an `add_plugin`/`add_plugins` argument in \
         `crates/*/src` or `src`, OUTSIDE every `#[cfg(test)]` region and outside doc comments.\n\n\
         Add the registration, or record the row in KNOWN_UNREGISTERED_PLUGINS with what was \
         measured, or — if a consumer is meant to add it — EXEMPT_PLUGINS with the argument."
    );
    let repaired: Vec<&String> = expected.difference(&unregistered).collect();
    assert!(
        repaired.is_empty(),
        "recorded unregistered plugin(s) {repaired:?} now HAVE a production registration. Delete \
         the KNOWN_UNREGISTERED_PLUGINS row in the same commit as the wiring."
    );
}

/// **Class 3.** Every declared host seam is named from production source outside its own member.
#[test]
fn every_declared_host_seam_is_named_outside_its_own_crate() {
    let census = scan_sources(&live_sources());
    let recorded: BTreeSet<&str> = KNOWN_DEAD_SEAMS.iter().copied().collect();

    println!("── host seams ──");
    let mut dead: BTreeSet<String> = BTreeSet::new();
    for seam in HOST_SEAMS {
        let decls = census.seam_decls.get(seam.symbol).map(Vec::as_slice).unwrap_or_default();
        let refs = census.seam_refs.get(seam.symbol).map(Vec::as_slice).unwrap_or_default();
        println!(
            "  {:<32} {:<6} decl={decls:?} refs={:?}",
            seam.symbol,
            if refs.is_empty() { "DEAD" } else { "ok" },
            &refs[..refs.len().min(2)]
        );
        assert!(
            !decls.is_empty(),
            "HOST_SEAMS names `{}` but no production file declares `fn {}`. The roster is stale — \
             the seam was renamed or deleted. Fix the row rather than letting a vanished symbol \
             report itself dead. Reason on the row: {}",
            seam.symbol,
            seam.symbol,
            seam.reason
        );
        assert!(
            decls.iter().any(|d| d.starts_with(seam.defined_in)),
            "HOST_SEAMS pins `{}` to {} but it is declared at {decls:?}. Re-pin the row: the \
             cross-member reference rule EXCLUDES the defining member, so a wrong `defined_in` \
             excludes the wrong crate and can turn a dead seam green.",
            seam.symbol,
            seam.defined_in
        );
        if refs.is_empty() {
            dead.insert(seam.symbol.to_string());
        }
    }

    let expected: BTreeSet<String> = recorded.iter().map(|s| (*s).to_string()).collect();
    let unexpected: Vec<&String> = dead.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "host seam(s) named by NO production source outside their own crate: {unexpected:?}\n\n\
         A seam whose only callers live in the crate that defines it is not a seam — nothing \
         crosses it. Record it in KNOWN_DEAD_SEAMS, or remove the row if the symbol stopped being \
         a host entry point."
    );
    let repaired: Vec<&String> = expected.difference(&dead).collect();
    assert!(
        repaired.is_empty(),
        "recorded dead seam(s) {repaired:?} now have a cross-crate production reference. Delete \
         the KNOWN_DEAD_SEAMS row in the same commit as the wiring."
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Non-vacuity — the scanner is looking at something
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Floors under every quantity the classes above divide by.
///
/// With three baselines recorded and everything else green, these are what stand between a green
/// run and a scanner that has quietly stopped scanning. Each floor is a deliberate update, never a
/// reaction to a red run: lowering one is part of the same act as the removal that lowered it.
#[test]
fn the_scanner_is_not_vacuous() {
    let census = scan_sources(&live_sources());
    println!(
        "production files: {}, cfg(test) items stripped: {}, test-only files: {}, plugins: {}, \
         registered idents: {}",
        census.production_files,
        census.cfg_test_items,
        census.test_only_files.len(),
        census.plugins.len(),
        census.registrations.len()
    );
    println!("test-only files excluded: {:?}", census.test_only_files);
    assert!(
        census.production_files >= MIN_PRODUCTION_FILES,
        "the walk found {} production files (floor {MIN_PRODUCTION_FILES})",
        census.production_files
    );
    assert!(
        census.cfg_test_items >= MIN_CFG_TEST_ITEMS_STRIPPED,
        "the `#[cfg(test)]` stripper removed {} items (floor {MIN_CFG_TEST_ITEMS_STRIPPED}). A \
         stripper that matches nothing scores every `#[cfg(test)] mod tests` registration as \
         production — the exact shape that makes this census a gate that cannot fail.",
        census.cfg_test_items
    );
    assert!(
        census.test_only_files.len() >= MIN_TEST_ONLY_FILES,
        "only {} file(s) were excluded as owned by a `#[cfg(test)]`-gated `mod` declaration (floor \
         {MIN_TEST_ONLY_FILES}). Zero means shape 2 of the exclusion stopped working.",
        census.test_only_files.len()
    );
    assert!(
        census.test_only_files.contains(TEST_ONLY_CONTROL_FILE),
        "{TEST_ONLY_CONTROL_FILE} was NOT excluded as test-only. It is reached only through \
         `#[cfg(test)] mod tests;` in its parent and contains two `app.add_plugin(ProfilerPlugin)` \
         calls, so including it would score the precedent defect as repaired. If the file moved, \
         re-point TEST_ONLY_CONTROL_FILE — it is a control, not a fact about the profiler."
    );
    assert!(
        census.plugins.len() >= MIN_PLUGINS,
        "the roster scan recovered {} `impl Plugin for` sites (floor {MIN_PLUGINS}). A reformatted \
         or multi-line `impl` header would empty class 2 without any class reporting it.",
        census.plugins.len()
    );
    assert!(
        census.registrations.len() >= MIN_REGISTERED_IDENTS,
        "the registration scan recovered {} identifiers (floor {MIN_REGISTERED_IDENTS}). Too few \
         means every plugin looks dead, which reads as a flood of findings rather than as a broken \
         scan.",
        census.registrations.len()
    );
}

/// The `#[cfg(…)]` test-gate rule's precondition, asserted rather than assumed.
///
/// The rule blanks an item whose `cfg` predicate contains the word `test`. That is right for every
/// shape in this tree and WRONG for `not(test)` — code present in production and absent under
/// `cargo test` — which would be blanked and its contents scored unreachable. No such site exists;
/// this fails the moment one is written, which is a far better outcome than a silent false
/// positive in a class whose whole job is to accuse.
#[test]
fn the_cfg_test_rule_has_no_not_test_counterexample_in_the_tree() {
    let mut offenders = Vec::new();
    for f in live_sources() {
        let blanked = blank_comments_and_strings(f.text.as_bytes());
        let flat: Vec<u8> =
            blanked.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        if find_pattern(&flat, b"not(test)").is_some() {
            offenders.push(f.path.clone());
        }
    }
    assert!(
        offenders.is_empty(),
        "`#[cfg(not(test))]` (or `not(test)` inside a larger predicate) appears in {offenders:?}. \
         The stripper's word-match rule would blank that item as test-only and report everything it \
         registers as unreachable. Teach `strip_cfg_test` the polarity before landing the cfg."
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung 4 — sensitivity controls: each class is shown to FAIL on an injected defect
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Returns a copy of `files` with the single line containing `needle` in `path` removed.
///
/// Asserts the needle occurs exactly once, so a control that has drifted off its anchor fails
/// loudly instead of injecting nothing and passing.
fn with_line_removed(files: &[SourceFile], path: &str, needle: &str) -> Vec<SourceFile> {
    let mut out = files.to_vec();
    let f = out
        .iter_mut()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("control anchor {path} is not in the production walk"));
    let hits = f.text.matches(needle).count();
    assert_eq!(
        hits, 1,
        "control anchor `{needle}` occurs {hits} time(s) in {path}, expected exactly 1. Re-point \
         the control at a live single-occurrence line — a control that injects nothing passes."
    );
    f.text = f
        .text
        .lines()
        .filter(|l| !l.contains(needle))
        .collect::<Vec<_>>()
        .join("\n");
    out
}

/// Builds a `SourceFile` in the same member as an existing one.
fn synthetic(path: &str, member_dir: &str, text: &str) -> SourceFile {
    SourceFile { path: path.to_string(), member_dir: member_dir.to_string(), text: text.to_string() }
}

/// Returns `text` with `row` inserted as the first entry of the `[table]` it names.
///
/// ⚠️ The insertion point is the newline that ENDS the header line, found rather than computed.
/// This checkout's manifests are CRLF (`core.autocrlf`), so `at + "[dev-dependencies]\n".len()`
/// splices the row into the MIDDLE of the header line and the injected edge silently disappears.
/// Measured while writing this control: it reported `Unreferenced` and read as a classifier bug.
fn with_dep_row(text: &str, table: &str, row: &str) -> String {
    let at = text
        .find(table)
        .unwrap_or_else(|| panic!("control anchor: no `{table}` table in the manifest"));
    let eol = at
        + text[at..].find('\n').expect("invariant: a table header is followed by a newline")
        + 1;
    let mut out = text.to_string();
    out.insert_str(eol, row);
    out
}

/// **Control for class 1** — a member whose only reverse edge is a `[dev-dependencies]` one is
/// reported. This is the `boyko_ui` shape, injected: `ghost-subsystem` is complete, compiles, has
/// tests, and is shipped by nothing.
///
/// All three classes are exercised on the SAME synthetic member, so the control also shows the
/// classifier is reading the table rather than accusing whatever it does not recognise.
#[test]
fn a_member_referenced_only_as_a_dev_dependency_is_reported() {
    const GHOST_ROW: &str = "ghost-subsystem = { path = \"../ghost_subsystem\" }\n";
    let ghost = Manifest {
        dir: "crates/ghost_subsystem".to_string(),
        package: "ghost-subsystem".to_string(),
        text: "[package]\nname = \"ghost-subsystem\"\n\n[dependencies]\n".to_string(),
    };
    let base = live_manifests();
    let host_text = base
        .iter()
        .find(|m| m.package == "boyko-render")
        .expect("invariant: boyko-render is a workspace member")
        .text
        .clone();

    let injected = |table: &str| -> Vec<Manifest> {
        let mut out: Vec<Manifest> = base
            .iter()
            .cloned()
            .map(|mut m| {
                if m.package == "boyko-render" {
                    m.text = with_dep_row(&host_text, table, GHOST_ROW);
                }
                m
            })
            .collect();
        out.push(ghost.clone());
        out
    };

    let dev = classify_members(&injected("[dev-dependencies]"));
    assert_eq!(
        dev.get("ghost-subsystem"),
        Some(&Reach::DevOnlyReferenced),
        "a member reachable ONLY through a [dev-dependencies] edge was not classified \
         DevOnlyReferenced — class 1 is blind to the defect it was written for. Got {:?}",
        dev.get("ghost-subsystem")
    );

    let shipped = classify_members(&injected("[dependencies]"));
    assert_eq!(
        shipped.get("ghost-subsystem"),
        Some(&Reach::ProductionReachable),
        "the same member behind a [dependencies] edge was still reported unreachable — class 1 is \
         not distinguishing the two tables. Got {:?}",
        shipped.get("ghost-subsystem")
    );

    let mut alone = base.clone();
    alone.push(ghost);
    assert_eq!(
        classify_members(&alone).get("ghost-subsystem"),
        Some(&Reach::Unreferenced),
        "a member no manifest names at all was not classified Unreferenced — the `boyko-serialize` \
         shape must stay distinct from the dev-only one"
    );
}

/// **Control for class 2** — deleting a plugin's single production registration is reported.
///
/// `CsmPlugin` is the anchor: `boyko_app/src/plugins.rs` registers it exactly once, and its other
/// `app.add_plugin(CsmPlugin)` lives inside `csm_plugin.rs`'s `#[cfg(test)] mod`, which is
/// precisely the pair this control needs.
#[test]
fn deleting_a_plugins_only_production_registration_is_reported() {
    let live = live_sources();
    let before = scan_sources(&live);
    assert!(
        before.registrations.get("CsmPlugin").is_some_and(|v| !v.is_empty()),
        "control invariant broken: CsmPlugin has no production registration to delete. Re-point \
         this control at a plugin that does."
    );
    let injected =
        with_line_removed(&live, "crates/boyko_app/src/plugins.rs", "add_plugin(CsmPlugin)");
    let after = scan_sources(&injected);
    assert!(
        after.plugins.contains_key("CsmPlugin"),
        "the injection removed the plugin DECLARATION as well as the registration — the control is \
         not testing what it claims"
    );
    assert!(
        after.registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "class 2 still reports CsmPlugin registered after its ONLY production registration was \
         deleted. Its remaining `app.add_plugin(CsmPlugin)` is inside csm_plugin.rs's \
         `#[cfg(test)] mod`, so a census counting it is counting a test. Sites seen: {:?}",
        after.registrations.get("CsmPlugin")
    );
}

/// **Control for class 2, shape 1 — and the one that matters most.**
///
/// A registration written inside a `#[cfg(test)] mod tests { … }` block does NOT count. Without
/// this exclusion the census reports `boyko_ui` as reachable, because
/// `crates/boyko_ui/src/layout.rs:1819-1820` and `:1947-1948` DO register the layout systems —
/// inside a `#[cfg(test)] mod`. An earlier claim about this very defect was wrong for exactly that
/// reason.
#[test]
fn a_registration_inside_a_cfg_test_block_is_not_a_registration() {
    let live = live_sources();
    let mut injected =
        with_line_removed(&live, "crates/boyko_app/src/plugins.rs", "add_plugin(CsmPlugin)");
    injected.push(synthetic(
        "crates/boyko_render/src/csm_revival.rs",
        "boyko_render",
        "#[cfg(test)]\nmod tests {\n    fn revive(app: &mut App) {\n        \
         app.add_plugin(CsmPlugin);\n    }\n}\n",
    ));
    let after = scan_sources(&injected);
    assert!(
        after.registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "a registration inside a `#[cfg(test)] mod` was counted as production. This is THE \
         exclusion: with it broken, every crate whose tests build an App and add its plugin reads \
         as shipped, and the census can never fail. Sites seen: {:?}",
        after.registrations.get("CsmPlugin")
    );
    // The same line OUTSIDE the gate must count, or the exclusion is just deleting everything.
    let mut visible = injected.clone();
    visible.pop();
    visible.push(synthetic(
        "crates/boyko_render/src/csm_revival.rs",
        "boyko_render",
        "fn revive(app: &mut App) {\n    app.add_plugin(CsmPlugin);\n}\n",
    ));
    assert!(
        !scan_sources(&visible).registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "the SAME registration outside the `#[cfg(test)]` gate was also dropped — the stripper is \
         removing more than the gated item"
    );
}

/// **Control for class 2, shape 2** — a registration in a whole FILE owned by a
/// `#[cfg(test)] mod NAME;` declaration does not count.
///
/// This is the live `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` shape: the gate is in one
/// file and the registration is in another, so brace-balanced stripping alone cannot see it.
#[test]
fn a_registration_in_a_cfg_test_gated_file_module_is_not_a_registration() {
    let live = live_sources();
    let mut injected =
        with_line_removed(&live, "crates/boyko_app/src/plugins.rs", "add_plugin(CsmPlugin)");
    injected.push(synthetic(
        "crates/boyko_render/src/revival/mod.rs",
        "boyko_render",
        "#[cfg(test)]\nmod probe;\n",
    ));
    injected.push(synthetic(
        "crates/boyko_render/src/revival/probe.rs",
        "boyko_render",
        "fn revive(app: &mut App) {\n    app.add_plugin(CsmPlugin);\n}\n",
    ));
    let after = scan_sources(&injected);
    assert!(
        after.test_only_files.contains("crates/boyko_render/src/revival/probe.rs"),
        "the file owned by `#[cfg(test)] mod probe;` was not recognised as test-only. Excluded \
         set: {:?}",
        after.test_only_files
    );
    assert!(
        after.registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "a registration in a `#[cfg(test)] mod NAME;` FILE was counted as production. Sites seen: \
         {:?}",
        after.registrations.get("CsmPlugin")
    );
}

/// **Control for class 2, shape 3** — the `#[path = "…"]` override is followed.
///
/// `boyko_physics` writes `#[cfg(test)] #[path = "resources_tests.rs"] mod tests;` twice. The path
/// string is blanked by the comment/string pass, so the override has to be read from the raw text;
/// this control fails if that stops happening.
#[test]
fn a_cfg_test_module_with_a_path_override_is_followed() {
    let live = live_sources();
    let mut injected = live.clone();
    injected.push(synthetic(
        "crates/boyko_render/src/relocated.rs",
        "boyko_render",
        "#[cfg(test)]\n#[path = \"relocated_tests.rs\"]\nmod tests;\n",
    ));
    injected.push(synthetic(
        "crates/boyko_render/src/relocated_tests.rs",
        "boyko_render",
        "fn revive(app: &mut App) {\n    app.add_plugin(GhostPlugin);\n}\n",
    ));
    let after = scan_sources(&injected);
    assert!(
        after.test_only_files.contains("crates/boyko_render/src/relocated_tests.rs"),
        "a `#[path = \"…\"]`-relocated `#[cfg(test)] mod` did not exclude its file. Two live sites \
         in boyko_physics use this shape. Excluded set: {:?}",
        after.test_only_files
    );
    assert!(
        !after.registrations.contains_key("GhostPlugin"),
        "the relocated test file's registration was counted as production"
    );
}

/// **Control for the doc-comment trap** — a registration written in a `///` example is not a
/// registration.
///
/// Not hypothetical: `crates/boyko_ui/src/interaction/plugin.rs:141` reads
/// `/// app.add_plugins(UiBindingPlugin::default());`, and `UiBindingPlugin` is one of the plugins
/// this census reports dead. A grep-shaped census reports it alive.
#[test]
fn a_registration_written_in_a_doc_comment_is_not_a_registration() {
    let live = live_sources();
    let mut injected =
        with_line_removed(&live, "crates/boyko_app/src/plugins.rs", "add_plugin(CsmPlugin)");
    injected.push(synthetic(
        "crates/boyko_render/src/csm_doc.rs",
        "boyko_render",
        "/// Example:\n/// app.add_plugin(CsmPlugin);\n/* app.add_plugins((CsmPlugin,)); */\nfn \
         nothing() {}\n",
    ));
    assert!(
        scan_sources(&injected).registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "a plugin named only in a doc comment / block comment was counted as registered"
    );
    // A registration hidden in a string literal is the same trap wearing a different hat.
    let mut in_string = injected.clone();
    in_string.pop();
    in_string.push(synthetic(
        "crates/boyko_render/src/csm_doc.rs",
        "boyko_render",
        "fn nothing() -> &'static str {\n    \"app.add_plugin(CsmPlugin);\"\n}\n",
    ));
    assert!(
        scan_sources(&in_string).registrations.get("CsmPlugin").map(Vec::as_slice).unwrap_or_default().is_empty(),
        "a plugin named only inside a string literal was counted as registered"
    );
}

/// **Control for class 3** — deleting a seam's only cross-member reference is reported.
///
/// `sync_instance_model_cols` is the anchor because it exercises the by-VALUE shape: it is not
/// called, it is handed to `b.add_system(…)`. A class that only looked for `name(` would miss it,
/// and did in this file's first draft.
#[test]
fn deleting_a_host_seams_only_cross_crate_reference_is_reported() {
    let live = live_sources();
    let before = scan_sources(&live);
    assert!(
        before
            .seam_refs
            .get("sync_instance_model_cols")
            .is_some_and(|v| !v.is_empty()),
        "control invariant broken: sync_instance_model_cols has no cross-member reference to \
         delete. Re-point this control at a seam that does."
    );
    let injected = with_line_removed(
        &live,
        "crates/boyko_app/src/plugins.rs",
        "add_system(sync_instance_model_cols)",
    );
    let after = scan_sources(&injected);
    assert!(
        after
            .seam_refs
            .get("sync_instance_model_cols")
            .map(Vec::as_slice)
            .unwrap_or_default()
            .is_empty(),
        "class 3 still reports sync_instance_model_cols referenced after its only cross-member \
         registration was deleted. The surviving mentions are a `use` line and two doc comments — \
         a census counting either is counting an import and a sentence. Sites seen: {:?}",
        after.seam_refs.get("sync_instance_model_cols")
    );
}
