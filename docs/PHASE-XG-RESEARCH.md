# Phase X.G — entities_inland Growth: Code Inventory

Input for the architect. Goal: delete the engine's LAST realloc-doubling — the
`EntityMaster::entities_inland: Vec<EntityInland>` growth memcpy, measured as the g7b worst
sub-batch (2.4–2.5 ms at ~580 k entities; attribution in
[PHASE-XF-RESULTS.md](PHASE-XF-RESULTS.md) §B6). Treatment: the X.F address-stable
reserve/commit pattern, applied to entity metadata.

## Design-deciding facts (verified file:line by project-analyst)

1. **`EntityInland`** (`entity_inland.rs:22-55`): `#[repr(C)] Copy { archetype_ptr: *mut
   Archetype @0, unit_index: u32 @8, generation: u32 @12 }` — 16 B / align 8, const-asserted.
   **`NULL` is ALL-ZERO bytes** (null ptr + 0 + 0) ⇒ **demand-zero pages are a free NULL-fill
   for the never-written tail** (today's `resize(.., NULL)` writes become a watermark bump).
   Liveness = `archetype_ptr.is_null()`; `unit_index`/`generation` unspecified when null; a
   fresh id expects generation 0 — a zero page behaves exactly like a fresh NULL slot.
   Recycled-dead slots in the WRITTEN range are `{null, 0, gen+1}` ≠ all-zero — only forbids
   decommit/re-zero of the live range, irrelevant to frontier growth. No UnsafeCell inside;
   the F4/A-2 UnsafeCell discipline lives on the POINTEE side (archetype slab) — X.G moves
   container, pointer VALUES are plain data.
2. **Zero interior pointers into the buffer, anywhere** (exhaustive: no `as_ptr`/
   `NonNull<EntityInland>`/stored `&EntityInland` in the workspace). Every consumer copies the
   16-B record per call or holds a statement-scoped `get`/`get_mut`. Cross-frame handles are
   `EntityId` INDICES (realloc-stable by construction). `len` is the only load-bearing scalar
   (bounds-check oracle for `get`, `capacity()`, `iter_entities`, `rewind_allocate`).
3. **The Phase-7 hot path** (`ecs_master.rs:1296` in `get_component_raw`):
   `entities_inland.get(id)?` = one len load + cmp + one 16-B indexed load (`base + id*16`,
   shl-4 folded into addressing). Benches pin it: `random_access.rs` `get_component_raw_hot`
   ≤16 ns (measured ~3 ns), `has_entity` ≤5 ns, `get_component_typed`, `set_component_raw`,
   `stale_generation`, `missing_component` (targets table `random_access.rs:6-16`). **A raw
   `(ptr, len)` store with a `Vec::get`-shaped bound check can be byte-identical; any
   chunked-page design adds a load and fails.**
4. **All writes/resizes are dispatcher-side** (owner direct API or apply window, `&mut self`):
   W2 `allocate_entity` resize (`entity_master.rs:122-123`); W3 `ensure_capacity`
   (`entity_master.rs:225-229`, sole production caller `spawn_batch_command.rs:324-326` with
   `end_id + MAX_BATCH_HINT(8192)`); W4 `register_batch` slice write; W5
   `register_entity_with_ptr` (+defensive resize); W6 `deallocate_entity` in-place null+gen
   bump; W7 `clear` (len→0, keeps allocation); W8/W9/W10 `create_entity_at*` /
   `SpawnAtCommand::apply` resizes; W11/W12 swap-fixups + migration repoints (indexed
   writes). **Workers reach exactly ONE EntityMaster field: the atomic `next_entity_id`**
   (Phase-11 `EntityCounter`, `entity_counter.rs:148-161`); reserved IDs get slots ONLY in
   the apply window. Send/Sync = SEND5 (`entity_master.rs:530-555`) — currently
   scheduling-enforced ("no worker can observe a mid-flight realloc"); address-stable growth
   makes it structural.
5. **Growth mechanics today**: `EntityMaster::new` ⇒ `Vec::new()` (len 0 cap 0 — pinned by
   `phase12_6_lazy_alloc.rs:62-71`); `EcsMaster::with_capacity(entity_cap, _)` ⇒
   `Vec::with_capacity` (capacity only, len 0; doc `ecs_master.rs:462-465`). Growth ONLY via
   `resize(.., NULL)` — zero `reserve()` calls. **The g7b chain is anchored at 9192** (first
   `ensure_capacity` request = 1000 + 8192): caps 9192→18384→…→294144→588288; crossings at
   sub-batches k=0,1,10,28,65,138,**285**,**580** — matches the measured twins exactly
   (#580 = 9.0 MiB memcpy + fresh-page faults). `INITIAL_ENTITY_CAPACITY = 1024`
   (`constants.rs:105`) is DEAD (zero uses).
6. **`free_entity_ids`**: LIFO recycle Vec, dispatcher-only, not indexed by position, no
   pointers — out of X.G's required scope (optional).
7. **Tests/benches**: `entity_master.rs` unit tests :557-937 (recycle/generation/batch/
   live_count/clear/rewind); `random_access.rs` lookup groups + `create_entity_10k` (≤5%
   gate) + `delete_entity_10k` + `iter_entities_*`; `phase12_6_lazy_alloc.rs` (cap-0
   contract — keep or formally amend); g7/g7b in `growth_crossing.rs` = the acceptance bench
   (argmax instrumentation + `XF_DUMP_PROFILE` kept from X.F); Miri churn suites: phase19/
   14a/14b/8cd; F4 witness `archetype_bundle.rs:1164` (pointee-side, must stay green).
8. **X.F machinery available for reuse** (`arena.rs`): per-OS reserve/commit/release arms
   (W-RES/W-CMT/U-RES/U-CMT/F-RES SAFETY patterns), `checked_align_up`, granule/slab
   constants, the isize::MAX guard, the Miri-fallback eager-alloc pattern, `grow_step`
   policy fn. Open choice: extract a shared `memory/vm.rs` primitive vs a self-contained
   store module (blast-radius trade-off — X.F gates would need re-running if arena.rs is
   refactored).
