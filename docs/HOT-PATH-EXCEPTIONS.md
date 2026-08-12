# Hot-path type-ban exceptions

Every production use of a type banned by [`clippy.toml`](../clippy.toml)'s `disallowed-types`
(`HashMap` / `HashSet` / `Mutex` / `RwLock` / `Rc` / `RefCell`, plus the `parking_lot` and
`hashbrown` forward tripwires) is enumerated here, one row per `#[allow(clippy::disallowed_types)]`
site. [`scripts/check_hotpath_exceptions.py`](../scripts/check_hotpath_exceptions.py) fails CI when
the sources and this file disagree, so an exception cannot land without a written justification a
reviewer has to read.

## Why a registry and not a comment

The 2026-07 audit traced the callers of all 43 then-existing exceptions. Five carried a hand-written
comment claiming coldness — "Cold (registration-only … never on the trigger hot path)",
"the PER-FRAME system path never reaches it", "cold-path only (panics are rare)" — and **all five
were false in the same way**: the memo probe sat *after* the lock acquire, so the lock was taken on
every call rather than on the first. A prose comment nobody re-reads is not a control.

Those five are fixed, not documented:

| site | was | now |
| --- | --- | --- |
| `observers/trigger.rs` | `OnceLock<Mutex<HashMap<TypeId, TriggerId>>>` locked per relation link/unlink, *before* the observer 0%-gate | lock-free slot scan + an id-free `has_any_trigger_observer()` early-out |
| `iters/query/query_type_registry.rs` | global `Mutex` per `world.query::<D, F>()` (~50×/frame, every worker thread) | [`TypeIntern`](../crates/boyko_utils/src/type_intern/mod.rs) |
| `resources/resource_type_registry.rs` | global `Mutex` reached unconditionally from `boyko_app`'s `frame_loop` | `TypeIntern` |
| `resources/nonsend_resources.rs` | the memo *was* the locked map; `get_mut_ptr` is the runner's documented "every frame, cheap" leg | `TypeIntern`, leaked-`OnceLock` indirection deleted |
| `iters/component_set.rs` | `RwLock::read()` on every call before any memo hit | `OnceLock` slot array, matching the single-component leg beside it |
| `threadpool/scope.rs` | `Mutex::new` per scope + unconditional `lock()` in `Scope::drop` (a scope per system run) | `AtomicPtr` CAS-once slot; the no-panic path loads null |

## Frequency classes

The class column is a closed vocabulary of *provably cold* classes. There is deliberately **no
`per-frame` class**: a hot site cannot be registered, only fixed.

| class | meaning |
| --- | --- |
| `once-per-process` | one construction/read for the process lifetime |
| `once-per-type` | a [rust#22991] `TypeId` mint whose per-call fast path is lock-free |
| `load-time` | asset/scene/config load, outside the frame loop |
| `boot` | device/window/pool/registry construction before the first frame |
| `shutdown` | teardown after the last frame |
| `debug-only` | inside `#[cfg(debug_assertions)]` or a `debug_assert!` |
| `codegen-tool` | offline generator behind a default-off feature; never in a shipped binary |
| `test-harness` | production-resident, but every caller is a test/bench target (weakest class — see the script's note) |
| `alloc-guarded` | a `RefCell` (never a lock or map) on a `!Send + !Sync` owner, gating an operation that dwarfs it — a `vkCreateBuffer`, a heap allocation. The justification must name that operation |

[rust#22991]: https://github.com/rust-lang/rust/issues/22991

## What is not an exception, and the false positive that taught it

**An item carrying `#[cfg(test)]` is not registered here, because it does not exist in a shipping
build.** A `static` or `fn` the library never compiles cannot be on a hot path — there is nothing to
justify.

The scanner did not know that. `cfg_test_spans` recognises `#[cfg(test)] mod name { .. }` blocks and
nothing else, so a `#[cfg(test)]` on a single item was invisible and its `#[allow]` counted as a
production exception. **MEASURED 2026-08-12**: three such sites —
`boyko_ecs`'s `profiling::store::{TEST_SERIAL, test_serial}` and `boyko_log`'s
`drain_owner::TEST_SERIAL`, every one a test-harness serialization lock whose own doc-comment calls
itself "the sanctioned exception shape — a `#[cfg(test)]` fixture, never on any engine path" — held
this gate **RED**, exit 1.

**And the red was invisible for as long as it existed**, because the script prints its summary line
("… exception(s) across … file(s)") on *both* verdicts. A report that quoted that line read like a
pass. The lesson is the campaign's own, arriving this time in the certification rather than in the
engine: **a count is not a verdict, and the only thing that establishes a gate's colour is its exit
code.** `scripts/check_hotpath_exceptions.py::item_is_cfg_test` closes the classification hole; the
fix was shown red twice — once by stripping a `#[cfg(test)]` off one of the three (the lock becomes
real production state, gate red again) and once by planting a genuinely production
`#[allow(clippy::disallowed_types)]` in `lifecycle.rs` (caught, so the new rule does not over-reach).

Registered count after the fix: **32 across 11 files**.

## Exceptions

| file | symbol | type | class | why this is cold and why a boyko-native structure cannot serve |
| --- | --- | --- | --- | --- |
| `crates/boyko_ecs/src/ecs/core/asset/backing.rs` | `use std::collections::HashMap` | `HashMap` | boot | Import feeding `ASSET_LAYOUTS` only; every other map in the file is a `ComponentPool` column. |
| `crates/boyko_ecs/src/ecs/core/asset/backing.rs` | `use std::sync::{Mutex, OnceLock}` | `Mutex` | boot | Same static. `OnceLock` in the pair is not banned and carries the lazy construction. |
| `crates/boyko_ecs/src/ecs/core/asset/backing.rs` | `ASSET_LAYOUTS` | `OnceLock<Mutex<HashMap<TypeId, ComponentId>>>` | boot | rust#22991 forbids a per-`T` static inside the generic minter, so the layout table must be `TypeId`-keyed. Unlike the four registries this audit converted, there is no generic `id_for::<T>()` shim on a frame path — the only callers are asset-type registration during plugin/app build. |
| `crates/boyko_ecs/src/ecs/core/asset/backing.rs` | `register_asset_layout::<T>` | `Mutex` lock + `HashMap` entry | boot | The lock IS unconditional per call, which is why the caller trace matters rather than the code shape: every caller registers an asset type at setup. If an asset type is ever minted from a frame path, this becomes the sixth violation and must move to `TypeIntern`. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs` | `use std::cell::RefCell` | `RefCell` | once-per-type | Import for the `BUILDING` cycle-detection stack below; the file has no other interior-mutable cell. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs` | `BUILDING` | `RefCell<Vec<usize>>` | once-per-type | `build_required_plan` returns from the memoized `REQUIRES_ALL[id]` `OnceLock` at required.rs:283 BEFORE any `BuildingGuard` is pushed, so an expansion of an already-planned component never borrows the cell. This is the correct shape the five fixed sites lacked: the memo probe precedes the guarded structure. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs` | `use std::collections::HashMap` | `HashMap` | once-per-process | Import feeding `STABLE_NAME_INDEX` only; the file holds no other map, so the import's reachability is exactly that static's. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs` | `use std::sync::{Mutex, OnceLock}` | `Mutex` | once-per-process | Same `STABLE_NAME_INDEX` static; no other lock exists in the file, so nothing else can inherit this import. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs` | `STABLE_NAME_INDEX` / `stable_name_index()` | `OnceLock<Mutex<HashMap<u64, Vec<usize>>>>` | once-per-type | Write side is `register_stable_name::<C>()`, gated by the derive's per-type `OnceLock` so it runs once per component type. Read side is `resolve_stable_name`, which the loader calls once per file-local type into a dense `Vec<ResolvedType>`; row lookup then indexes that vector, never this map. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs` | `use std::collections::HashMap` | `HashMap` | boot | Import feeding `TAG_NAMES` only — verified by enumerating all five `Mutex` tokens in the file. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs` | `use std::sync::{Mutex, OnceLock}` | `Mutex` | boot | Same `TAG_NAMES` intern; a grep of the file yields exactly five `Mutex` tokens and all five sit on it. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs` | `TAG_NAMES` | `OnceLock<Mutex<HashMap<Box<str>, TagId>>>` | boot | Name→`TagId` intern for the string-keyed registration API. The per-frame tag path uses `TagId` (a dense index into a bitset), never a name, so the map has no frame-path reader. |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs` | `try_register_tag_by_name` / `tag_by_name` | `Mutex` lock + `HashMap` get/insert | boot | The two lock-taking entry points. Both reachable only from `App::add_plugin`-time registration; a workspace grep for `tag_by_name` finds no caller in any system body, serializer, or renderer. |
| `crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs` | `DenseStore::check_invariant` | `HashSet<u32>` | test-harness | Builds a reference model of the free-list to cross-check the dense column. Its only callers are the `tests/dense_d1_*.rs` suites. Weakest class in the table: it is a `pub fn` in the library, so a future production caller would make it hot without tripping this gate — prefer moving it behind `#[cfg(test)]` if the API surface ever allows. |
| `crates/boyko_rhi_vulkan/src/device.rs` | `use core::cell::{OnceCell, RefCell}` | `RefCell` | alloc-guarded | Import for the two sub-allocator cells below; `OnceCell` in the pair is not banned and carries the lazy creation. |
| `crates/boyko_rhi_vulkan/src/device.rs` | `VulkanContext::host_block` | `OnceCell<RefCell<HostVisibleBlock>>` | alloc-guarded | The shared host-visible block every `create_buffer(HostVisible)` sub-allocates from. The cell exists to hand `&mut` to the sub-allocator from an `&self` call on a `!Send + !Sync` context — it cannot contend, so Principle 4's lock-free rule is not in play. The guarded operation is a `vkCreateBuffer` plus suballocation; a non-atomic borrow-flag branch is not measurable beside it. |
| `crates/boyko_rhi_vulkan/src/device.rs` | `VulkanContext::device_block` | `OnceCell<RefCell<DeviceLocalBlock>>` | alloc-guarded | Same argument as `host_block`, for the VRAM block behind `create_buffer(DeviceLocal)`. Never mapped. |
| `crates/boyko_rhi_vulkan/src/device.rs` | `VulkanContext::host_block()` | `&RefCell<HostVisibleBlock>` | alloc-guarded | The lazy accessor: returns the cached cell, or creates the block once. Callers are `rhi_impl::device`'s buffer-creation verbs only — a workspace grep for `host_block()` finds four call sites, all inside `create_buffer`/`map` paths. |
| `crates/boyko_rhi_vulkan/src/device.rs` | `VulkanContext::device_block()` | `&RefCell<DeviceLocalBlock>` | alloc-guarded | Same shape and same four-call-site trace as `host_block()`. |
| `crates/boyko_scene/src/identity.rs` | `use std::collections::HashMap` | `HashMap` | once-per-process | Import for `InternerState.map`; nothing else in the file uses one. |
| `crates/boyko_scene/src/identity.rs` | `use std::sync::{Mutex, OnceLock}` | `Mutex` | once-per-process | Same interner; `Mutex` appears at exactly the import, the static's type, and the accessor's return type. |
| `crates/boyko_scene/src/identity.rs` | `InternerState.map` | `HashMap<&'static str, u32>` | once-per-process | Dedup table for string interning. Private struct, two fields, every use read. |
| `crates/boyko_scene/src/identity.rs` | `INTERNER` / `interner()` | `OnceLock<Mutex<InternerState>>` | once-per-process | `OnceLock::get_or_init` constructs the `Mutex` and the `HashMap` exactly once per process; the accessor itself is a lock-free load. |
| `crates/boyko_scene/src/identity.rs` | `intern()` / `resolve()` | `Mutex` lock | load-time | These do take the lock per call. Coldness is a property of the callers: names are interned when a scene/prefab is built and resolved for diagnostics, never per entity or per frame. A per-frame name lookup would make this hot — resolve to a `Name` once at load and carry the id. |
| `crates/boyko_threadpool/src/sync.rs` | `pub(crate) use std::sync::{Condvar, Mutex}` | `Mutex`, `Condvar` | boot | After the `ScopeShared` rework the shim's only consumer is `ThreadPoolBuilder::build`'s one-shot bootstrap handshake: a worker parks on the condvar until the pool publishes itself. That is a genuine blocking wait, not a memo lookup — the thing a mutex is actually for. |
| `crates/boyko_threadpool/src/thread_pool.rs` | `ThreadPool::join_handles` | `Mutex<Option<Vec<WorkerJoin>>>` | shutdown | The only two `lock()` sites in the crate are `ThreadPool::join` and `Drop for ThreadPool`; both do `.lock().take()` and hand the handles to `shutdown_and_join`. No dispatch path touches it. |
| `crates/boyko_threadpool/src/thread_pool.rs` | `ThreadPoolBuilder::build` bootstrap slot | `Arc<Mutex<Option<Arc<PoolInner>>>>` + `Condvar` | boot | Breaks a construction-order cycle: workers need the `Arc<PoolInner>` that cannot exist until their `Thread` handles do. `notify_all` fires exactly once, immediately after the single publish. |
| `crates/boyko_log/src/probe.rs` | `OBSERVE_LOCK` | `Mutex<()>` | test-harness | Serializes the tests that drive a `Once` site so a sibling cannot spend its latch inside an observer's window. Behind `feature = "test-probe"`, which every emitting crate enables in `[dev-dependencies]` ONLY, so no shipping binary links this module. It is listed rather than exempted because a FEATURE gate is not a `#[cfg(test)]` gate: the scanner is right that this code is structurally reachable if someone enables the feature in a normal dependency, and that is the thing the row exists to make a reviewer notice. |
| `crates/boyko_log/src/probe.rs` | `observe_lock()` | `MutexGuard<'static, ()>` | test-harness | The guard the row above hands out; same scope, same feature, same reason it is a row and not an exemption. |
| `crates/boyko_ui/src/reload/system.rs` | `use std::sync::{Arc, Mutex}` | `Mutex` | load-time | The hot-reload watcher's sinks; `boyko_ui`'s render and layout paths contain no lock. |
| `crates/boyko_ui/src/reload/system.rs` | `reconcile_in_world` sinks | `Arc<Mutex<DespawnPlan>>`, `Arc<Mutex<UiParseReport>>` | load-time | The enclosing system does run every frame, but all four acquires sit behind a three-gate chain (reload enabled → file mtime changed → parse succeeded), so they execute once per confirmed `.ui` edit. In steady state the system returns before constructing them. |
| `crates/boyko_ui/src/text/parser.rs` | `use std::collections::HashSet` | `HashSet` | load-time | Import for `parse_ui`'s duplicate-name check. |
| `crates/boyko_ui/src/text/parser.rs` | `parse_ui` → `seen_names` | `HashSet<String>` | load-time | A function-local scratch set, not a store: it lives for one parse of one `.ui` document and is dropped with the call. Parsing happens at load or on a confirmed hot-reload edit. |
| `crates/boyko_ui/src/text/parser.rs` | `split_name` → `seen_names` param | `&mut HashSet<String>` | load-time | Borrows `parse_ui`'s local; owns nothing. Same lifetime and same callers. |

## Blanket exemptions

A module-scoped `#![allow(clippy::disallowed_types)]` is normally a CI failure — it hides the ban
from a whole subtree. These three are permitted because the entire module is off the engine path,
and each is listed here so the decision is reviewed once instead of hidden in an attribute.

| file | scope | class | why the whole module is exempt |
| --- | --- | --- | --- |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | `module` | boot | Lowers the authored schedule into the flat id-indexed tables the executor runs on. It runs before the first frame and never again, and the whole file has that one temperature — ~30 scattered attributes would restate a single decision. |
| `crates/boyko_macros/src/ui.rs` | `module` | codegen-tool | Proc-macro expansion: the maps are the `ui!` parser's duplicate-`#name` validation tables, built and dropped inside rustc while compiling the invocation. Nothing here exists in the shipped binary. |
| `crates/boyko_shaderdsl/src/emit/mod.rs` | `module` | codegen-tool | The SSA recorder and HLSL printer behind `feature = "emit"`, which is off by default and declared only as a dev-dependency, so no shipped binary links this arena. The cell is touched in ~35 places restating one decision. |
