//! Bundle archetype-id cache surface on [`EcsMaster`] (mechanical split;
//! Phase 8.5).
//!
//! `bundle_archetype_id_for` + its cold registration helper. Extracted verbatim
//! from `ecs_master.rs`.

use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::bundle::bundle_type_registry::{BundleTypeId, MAX_BUNDLE_TYPES};
use crate::ecs::core::component::component_registry::{self};
use crate::ecs::identifiers::primitives::{
    ArchetypeId, ComponentId,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── Phase 8.5: Bundle archetype-id cache (SBC4) ─────────────────────────
    //
    // Phase 8.5 step-scoped `dead_code` allow on the two helpers below: the
    // first production caller lands in Step 5 (`SpawnCommand::apply`
    // rewrite), routed through the derive-generated `B::cached_archetype_id`
    // (Step 4). Remove the `#[allow(dead_code)]` then.

    /// Resolves the [`ArchetypeId`] for Bundle `B` in this world, lazily
    /// caching on the first call. Subsequent calls hit the cache (~3 ns).
    ///
    /// # Hot path (plan §6.2)
    ///
    /// 1. `B::bundle_type_id()` — single Acquire load on the per-impl
    ///    `OnceLock<BundleStaticInfo>` (~2 ns).
    /// 2. `self.bundle_archetype_cache[id.0].get()` — Acquire load on a
    ///    stable address (~1 ns).
    /// 3. If `Some(arch)`: return.
    /// 4. If `None`: fall into the cold path —
    ///    `Self::cold_register_bundle_archetype` resolves via
    ///    [`Self::get_or_create_archetype`] and `OnceLock::set`s the slot
    ///    (~1 µs).
    ///
    /// # Why `&mut self`
    ///
    /// The cold path calls [`Self::get_or_create_archetype`], which requires
    /// `&mut self` (it may register a new archetype). The hot path could be
    /// `&self`-only, but keeping the unified `&mut self` signature lets the
    /// caller (always `SpawnCommand::apply` post-Step 5) match its own
    /// `&mut EcsMaster` receiver. A `&self`-only fast-path accessor would
    /// be a Phase 9 design item (DEFERRED).
    ///
    /// # Visibility
    ///
    /// `pub(crate)` — user code does not call this directly. The
    /// `#[derive(Bundle)]`-generated `cached_archetype_id` (Step 4) is the
    /// blessed entry point; `SpawnCommand::apply` (Step 5) is the only
    /// in-tree caller.
    ///
    /// **Visibility note**: `pub`, not `pub(crate)`. The `#[derive(Bundle)]`
    /// macro in `boyko_macros` emits user-crate code calling this method
    /// from inside the generated `impl Bundle for UserType` block (specifically
    /// from `cached_archetype_id`). Direct user code SHOULD NOT call this —
    /// it is a macro-only API. The blessed surface for user code is
    /// `Commands::spawn(bundle)` (Phase 8.5 Step 5). Same soft-seal pattern
    /// as Bevy's `World::register_bundle_info`.
    #[allow(dead_code)]
    #[inline]
    pub fn bundle_archetype_id_for<B: Bundle>(&mut self) -> ArchetypeId {
        let type_id = B::bundle_type_id();
        debug_assert!(
            type_id.0 < MAX_BUNDLE_TYPES,
            "BundleTypeId out of bounds — saturate-then-panic in register_new should have prevented this"
        );

        if let Some(arch) = self.bundle_archetype_cache()[type_id.0].get() {
            return *arch;
        }

        self.cold_register_bundle_archetype::<B>(type_id)
    }

    /// Cold-path slot installer for [`Self::bundle_archetype_id_for`].
    ///
    /// Computes the canonical component-id list for `B`, registers (or
    /// reuses) the matching archetype, and publishes the result into the
    /// per-world cache slot. Idempotent: if another caller raced ahead and
    /// already populated the slot with an identical id (canonical-sorted
    /// ids + idempotent [`Self::get_or_create_archetype`] = deterministic
    /// `ArchetypeId`), [`OnceLock::set`] returns `Err` which we ignore and
    /// read back the winner's value.
    #[allow(dead_code)]
    #[cold]
    #[inline(never)]
    fn cold_register_bundle_archetype<B: Bundle>(
        &mut self,
        type_id: BundleTypeId,
    ) -> ArchetypeId {
        let ids = B::component_ids();
        // Required components (Feature 1, D4): expand the declared bundle ids
        // with the transitive closure of every component's `#[require]`s, then
        // canonical-sort, so the cached archetype already hosts every required
        // column. For a require-free bundle `for_each_required_id_excluding`
        // runs zero inner iterations and the effective set == `ids` — the
        // 0%-gate. Cold path only (once per (B, world)); the warm path reads the
        // OnceLock slot below.
        let arch = if component_registry::any_requires(ids) {
            let mut effective: Vec<ComponentId> = ids.to_vec();
            component_registry::for_each_required_id_excluding(ids, |cid| {
                effective.push(cid);
            });
            effective.sort_unstable_by_key(|c| c.0);
            self.get_or_create_archetype(&effective)
        } else {
            self.get_or_create_archetype(ids)
        };
        // OnceLock::set may race with a concurrent setter (Phase 9). If our
        // set loses, the value already stored is identical because (a)
        // component_ids() returns the same canonical-sorted slice for `B`
        // process-wide, and (b) get_or_create_archetype is idempotent on the
        // same id set within a single world. The Err return carries the
        // rejected value; we drop it and read back the winner's value.
        let cache = self.bundle_archetype_cache();
        let _ = cache[type_id.0].set(arch);
        *cache[type_id.0]
            .get()
            .expect("invariant: OnceLock populated by self or racer in cold path")
    }

}
