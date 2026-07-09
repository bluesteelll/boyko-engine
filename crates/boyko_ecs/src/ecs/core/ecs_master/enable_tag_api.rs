//! EnableTag toggle surface on [`EcsMaster`] (Decision D3 / Step 5).
//!
//! Mirrors the Phase-22 [`tag_api`] precedent (a separate `impl EcsMaster`
//! block + one `mod` line) but for the **bitset** storage backend: toggling a
//! flag is a single per-row atomic read-modify-write at `(archetype, row)` —
//! **O(1), no archetype migration, no structural-generation bump, no hook /
//! observer fire, no deferred drain** (flecs `CanToggle` semantics).
//!
//! Two halves:
//!
//! * **Registration (D5)** — `register_enable_tag` / `try_register_enable_tag` /
//!   `enable_tag_by_name` delegate to the global enable-tag intern in
//!   `component_registry.rs`, which additionally classifies the minted id as
//!   [`StorageKind::Bitset`] so it is filtered out of every archetype signature
//!   (Step 4).
//! * **Toggle / probe (D3)** — `enable` / `disable` / `is_enabled` (typed) +
//!   their `_id` (dynamic [`EnableTagId`]) variants. All take `&mut self` for
//!   the mutators / `&self` for the probe; dead / stale entities are silent
//!   no-ops (matching the deferred-command contract — a despawn may race a
//!   toggle within a frame).
//!
//! # `&mut self` exclusivity (the v1 soundness ground)
//!
//! The mutators take `&mut EcsMaster`. That exclusivity is what makes the
//! `Relaxed` atomics on the enable bit / `enable_generation` sound in v1: no
//! worker thread can be live during a toggle (Decision D8 / Multithreading
//! model). Do NOT relax the receiver to `&self` — that is the deferred D7
//! worker-marking seam, which must add real `Acquire`/`Release` + a loom proof.
//!
//! [`tag_api`]: crate::ecs::core::ecs_master::tag_api
//! [`StorageKind::Bitset`]: crate::ecs::core::component::component_registry::StorageKind

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, EnableTagId};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::ComponentId;

impl EcsMaster {
    // ── Registration (D5) ────────────────────────────────────────────────────

    /// Mints (or resolves) the enable tag named `name`, classifying its id as
    /// [`StorageKind::Bitset`] (Decision D5). Panicking sugar over
    /// [`try_register_enable_tag`](Self::try_register_enable_tag); mirrors
    /// [`register_tag`](Self::register_tag).
    ///
    /// The numeric id is first-call-order process-unstable; the **name** is the
    /// stable key. Registration is cold (lock + hash map) — mint once at setup
    /// and keep the [`EnableTagId`].
    ///
    /// # Panics
    ///
    /// If the shared `MAX_COMPONENTS` (512) ComponentId budget — shared with
    /// every typed component and dynamic tag — is exhausted and `name` was
    /// never minted.
    ///
    /// [`StorageKind::Bitset`]: crate::ecs::core::component::component_registry::StorageKind
    #[cold]
    pub fn register_enable_tag(&mut self, name: &str) -> EnableTagId {
        match component_registry::try_register_enable_tag_by_name(name) {
            Some(tag) => tag,
            None => register_enable_tag_exhausted_panic(name),
        }
    }

    /// Fallible enable-tag mint (Decision D5). Idempotent per name; returns
    /// `None` only when the shared `MAX_COMPONENTS` budget is exhausted and
    /// `name` was never minted. Mirrors
    /// [`try_register_tag`](Self::try_register_tag).
    #[cold]
    pub fn try_register_enable_tag(&mut self, name: &str) -> Option<EnableTagId> {
        component_registry::try_register_enable_tag_by_name(name)
    }

    // ── Typed toggle / probe (D3) ────────────────────────────────────────────

    /// Enables the flag `T` on `entity` (Decision D3). O(1) warm: no migration,
    /// no structural-generation bump, no hook / observer fire, no deferred
    /// drain. Dead / stale entities are a silent no-op.
    ///
    /// On the FIRST toggle of `T` into the entity's archetype this allocates the
    /// archetype's `EnableColumn` (and, lazily, the touched 512 B page) and
    /// records it in the per-world presence oracle + bumps the world's
    /// `enable_generation` exactly once (Decision D1 inv 5 / O2).
    #[inline]
    pub fn enable<T: Component>(&mut self, entity: Entity) {
        self.set_enable_bit(entity, T::component_id(), true);
    }

    /// Disables the flag `T` on `entity` (Decision D3). Same O(1) cost profile
    /// as [`enable`](Self::enable). Clearing a never-set flag is a no-op that
    /// never allocates a column or page.
    #[inline]
    pub fn disable<T: Component>(&mut self, entity: Entity) {
        self.set_enable_bit(entity, T::component_id(), false);
    }

    /// Returns `true` iff `entity` is live and the flag `T` is currently set
    /// (Decision D3). O(1): inland load → null/gen check → column lookup
    /// (≤4 scan) → paged bit test. ≤ 5 ns. `false` for dead / stale entities
    /// and for never-toggled flags.
    #[inline]
    pub fn is_enabled<T: Component>(&self, entity: Entity) -> bool {
        self.test_enable_bit(entity, T::component_id())
    }

    // ── Dynamic toggle / probe (D3) ──────────────────────────────────────────

    /// Enables the dynamic enable tag `tag` on `entity` (Decision D3). The
    /// dynamic twin of [`enable`](Self::enable).
    #[inline]
    pub fn enable_id(&mut self, entity: Entity, tag: EnableTagId) {
        self.set_enable_bit(entity, tag.component_id(), true);
    }

    /// Disables the dynamic enable tag `tag` on `entity` (Decision D3).
    #[inline]
    pub fn disable_id(&mut self, entity: Entity, tag: EnableTagId) {
        self.set_enable_bit(entity, tag.component_id(), false);
    }

    /// Returns `true` iff `entity` is live and the dynamic enable tag `tag` is
    /// currently set (Decision D3).
    #[inline]
    pub fn is_enabled_id(&self, entity: Entity, tag: EnableTagId) -> bool {
        self.test_enable_bit(entity, tag.component_id())
    }

    // ── Internal toggle / probe core ─────────────────────────────────────────

    /// Resolves `entity`'s live inland by value, or `None` for a dead / stale /
    /// never-registered handle.
    #[inline]
    fn live_inland(&self, entity: Entity) -> Option<EntityInland> {
        let slot = self.entity_master.entities_inland.get(entity.id().0)?;
        let inland: EntityInland = *slot;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        Some(inland)
    }

    /// Sets (or clears) the enable bit for `tag` on `entity` at its current row
    /// (Decision D3 toggle algorithm). `&mut self`-exclusive: the only writer in
    /// v1, which is what makes the `Relaxed` enable-bit / `enable_generation`
    /// stores sound (Decision D8).
    fn set_enable_bit(&mut self, entity: Entity, tag: ComponentId, value: bool) {
        debug_assert_eq!(
            component_registry::storage_kind(tag.0),
            component_registry::StorageKind::Bitset,
            "set_enable_bit: id {} is not a bitset enable tag",
            tag.0,
        );

        // Step 1: resolve the live inland by value (dead / stale ⇒ no-op).
        let Some(inland) = self.live_inland(entity) else {
            return;
        };
        // Step 2: current post-swap row, never cached (Decision D3).
        let row = inland.unit_index() as usize;
        let arch_ptr = inland.archetype_ptr();

        // Step 3: allocate (if needed) and flip the bit through the stable,
        // interior-mutable (`SharedReadWrite`, F4-rooted) slab pointer. The
        // `&mut Archetype` reborrow is confined to THIS block and dropped before
        // step 4 touches `self.archetype_master` — otherwise the reborrow
        // (which aliases into the archetype-master's slab) would overlap the
        // `&mut self.archetype_master` of `note_enable_column_alloc`.
        let (archetype_id, newly_allocated) = {
            // SAFETY (U1, U2, F1): `arch_ptr` is stable, interior-mutable
            //   (`SharedReadWrite`, F4-rooted) slab provenance — non-null +
            //   generation-matched above ⇒ the slot is live and not aliased by
            //   any other live borrow (`&mut self` exclusivity; the inland was
            //   copied out, so `entity_master` is no longer borrowed). The
            //   `&mut Archetype` is the sanctioned write surface (mirrors
            //   `EcsMaster::create_entity`'s reborrow). It is dropped at the end
            //   of this block, before `self.archetype_master` is touched.
            let archetype = unsafe { &mut *arch_ptr };
            let archetype_id = archetype.id();
            // `set_enable_bit` returns `true` only on the FIRST column for the
            // tag; a clear never allocates and returns `false`.
            let newly = archetype.set_enable_bit(tag, row, value);
            (archetype_id, newly)
        };

        // Step 4: one-time bookkeeping on a genuinely new column (Decision D1
        // inv 5 / O2). No `&mut Archetype` is live here. The presence-bit set
        // and the `enable_generation` bump are paired atomically inside
        // `note_enable_column_alloc` (per-world; keyed by this world's
        // `ArchetypeId`), so they can never desync.
        if newly_allocated {
            self.archetype_master_mut()
                .note_enable_column_alloc(tag, archetype_id);
        }
    }

    /// Tests the enable bit for `tag` on `entity` (Decision D3 `is_enabled`).
    /// `&self`: read-only. `false` for dead / stale entities, never-toggled
    /// flags (no column), and never-touched pages.
    fn test_enable_bit(&self, entity: Entity, tag: ComponentId) -> bool {
        let Some(inland) = self.live_inland(entity) else {
            return false;
        };
        let row = inland.unit_index() as usize;
        let arch_ptr = inland.archetype_ptr();
        // SAFETY (U1, U2, F1): stable interior-mutable slab provenance; non-null
        //   + generation-matched ⇒ live; a SHARED reborrow only (`&Archetype`)
        //   reading an `AtomicU64` enable bit — TB-clean, no `&mut` taken.
        let archetype = unsafe { &*arch_ptr };
        match archetype.enable_store.column(tag) {
            Some(col) => col.test(row),
            None => false,
        }
    }
}

/// Cold panic site for [`EcsMaster::register_enable_tag`] at budget exhaustion.
#[cold]
#[inline(never)]
fn register_enable_tag_exhausted_panic(name: &str) -> ! {
    panic!(
        "register_enable_tag(\"{name}\"): the shared component-id budget is exhausted — \
         enable tags share the {} -slot ComponentId space with typed components and \
         dynamic tags. Use try_register_enable_tag for a fallible mint.",
        component_registry::MAX_COMPONENTS
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry::{self, StorageKind};
    use crate::ecs::identifiers::primitives::ComponentId;

    // Reserved free block 323-324 (grep-verified empty in [320,340); disjoint
    // from all other test fixed-id ranges). The prior 510-511 collided with
    // resource_registry's `CompThenRes` at 510, and 456-457 with the registry
    // TEST_BASE block's u128 fixture — both in the shared lib-test process.
    const TABLE_POS: ComponentId = ComponentId(323);
    const TAG_STUNNED: ComponentId = ComponentId(324);

    #[repr(C)]
    struct Pos {
        x: f32,
        y: f32,
    }
    impl Component for Pos {
        fn component_id() -> ComponentId {
            TABLE_POS
        }
    }

    /// Enable-tag flag type for the typed `enable::<Stunned>` path. Its id is
    /// classified `StorageKind::Bitset` in `register_components`, mirroring what
    /// `#[component(storage = "bitset")]` (Wave 5) emits.
    #[repr(C)]
    struct Stunned;
    impl Component for Stunned {
        fn component_id() -> ComponentId {
            TAG_STUNNED
        }
    }

    fn register_components() {
        component_registry::register_layout::<Pos>(TABLE_POS.0);
        component_registry::register_layout::<Stunned>(TAG_STUNNED.0);
        component_registry::set_storage_kind(TAG_STUNNED.0, StorageKind::Bitset);
    }

    /// Spawns one `Pos` entity at the origin into `ecs`'s `[Pos]` archetype.
    fn spawn_pos(ecs: &mut EcsMaster, archetype_id: crate::ecs::identifiers::primitives::ArchetypeId) -> Entity {
        let p = Pos { x: 0.0, y: 0.0 };
        // SAFETY (test): `p` outlives the borrow; byte view of a `#[repr(C)]`.
        let bytes = unsafe {
            core::slice::from_raw_parts(&p as *const _ as *const u8, core::mem::size_of::<Pos>())
        };
        ecs.create_entity(archetype_id, &[(TABLE_POS, bytes)])
            .expect("spawn must succeed")
    }

    #[test]
    fn enable_then_is_enabled_typed() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[TABLE_POS]);
        let e = spawn_pos(&mut ecs, arch);

        assert!(!ecs.is_enabled::<Stunned>(e), "fresh entity starts disabled");
        ecs.enable::<Stunned>(e);
        assert!(ecs.is_enabled::<Stunned>(e), "enable must set the bit");
        ecs.disable::<Stunned>(e);
        assert!(!ecs.is_enabled::<Stunned>(e), "disable must clear the bit");
    }

    #[test]
    fn enable_id_dynamic_round_trip() {
        register_components();
        let mut ecs = EcsMaster::new();
        let tag = ecs.register_enable_tag("enable_api_dyn_round_trip");
        let arch = ecs.create_archetype(&[TABLE_POS]);
        let e = spawn_pos(&mut ecs, arch);

        assert!(!ecs.is_enabled_id(e, tag));
        ecs.enable_id(e, tag);
        assert!(ecs.is_enabled_id(e, tag));
        ecs.disable_id(e, tag);
        assert!(!ecs.is_enabled_id(e, tag));
    }

    /// Toggle is O(1) structurally: no archetype-count change, no STRUCTURAL
    /// generation bump — only `enable_generation` moves, and only on the first
    /// toggle of a tag into the archetype (Decision D1 inv 5 / O2).
    #[test]
    fn toggle_no_structural_change_enable_generation_bumps_once() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[TABLE_POS]);
        let e1 = spawn_pos(&mut ecs, arch);
        let e2 = spawn_pos(&mut ecs, arch);

        let arch_count_before = ecs.archetype_count();
        let struct_before = ecs.archetype_master().structural_generation();
        let enable_before = ecs.archetype_master().enable_generation();

        // First toggle into this archetype: enable_generation bumps once; the
        // column is allocated.
        ecs.enable::<Stunned>(e1);
        assert_eq!(ecs.archetype_count(), arch_count_before, "no new archetype");
        assert_eq!(
            ecs.archetype_master().structural_generation(),
            struct_before,
            "toggle must NOT bump structural_generation"
        );
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            enable_before + 1,
            "first column alloc bumps enable_generation exactly once"
        );

        // Second toggle (same tag, same archetype, different row) reuses the
        // column: enable_generation does NOT move.
        ecs.enable::<Stunned>(e2);
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            enable_before + 1,
            "reusing an existing column must NOT bump enable_generation again"
        );
        // And toggling e1 off/on again also does not bump.
        ecs.disable::<Stunned>(e1);
        ecs.enable::<Stunned>(e1);
        assert_eq!(ecs.archetype_master().enable_generation(), enable_before + 1);
    }

    /// `is_enabled` resolves the row via the InlandStore — a swap-remove that
    /// moves an enabled entity into a new row must keep `is_enabled` correct for
    /// that entity (O1 row resolution through `unit_index`).
    #[test]
    fn is_enabled_follows_entity_across_swap_remove() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[TABLE_POS]);
        let e0 = spawn_pos(&mut ecs, arch); // row 0
        let e1 = spawn_pos(&mut ecs, arch); // row 1
        let e2 = spawn_pos(&mut ecs, arch); // row 2 (last)

        ecs.enable::<Stunned>(e2); // tag the last entity
        assert!(ecs.is_enabled::<Stunned>(e2));
        assert!(!ecs.is_enabled::<Stunned>(e0));

        // Delete e0 → e2 swaps into row 0. Its enable bit must travel with it.
        assert!(ecs.delete_entity(e0));
        assert!(
            ecs.is_enabled::<Stunned>(e2),
            "the swapped entity's enable bit must follow it to the new row"
        );
        assert!(!ecs.is_enabled::<Stunned>(e1), "untagged entity stays disabled");
    }

    /// Page-boundary toggle: rows 4095 (page 0) and 4096 (page 1) are in
    /// different `EnablePage`s. Toggling across the boundary must be independent.
    #[test]
    fn toggle_across_page_boundary_4095_4096() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[TABLE_POS]);

        // Spawn 4097 entities; capture the ones at rows 4095 and 4096.
        let mut at_4095 = None;
        let mut at_4096 = None;
        for row in 0..4097usize {
            let e = spawn_pos(&mut ecs, arch);
            if row == 4095 {
                at_4095 = Some(e);
            } else if row == 4096 {
                at_4096 = Some(e);
            }
        }
        let e_4095 = at_4095.expect("row 4095 entity");
        let e_4096 = at_4096.expect("row 4096 entity");

        ecs.enable::<Stunned>(e_4095);
        assert!(ecs.is_enabled::<Stunned>(e_4095), "page-0 toggle set");
        assert!(!ecs.is_enabled::<Stunned>(e_4096), "page-1 row must be independent");

        ecs.enable::<Stunned>(e_4096);
        assert!(ecs.is_enabled::<Stunned>(e_4096), "page-1 toggle set");
        assert!(ecs.is_enabled::<Stunned>(e_4095), "page-0 toggle undisturbed");

        ecs.disable::<Stunned>(e_4095);
        assert!(!ecs.is_enabled::<Stunned>(e_4095));
        assert!(ecs.is_enabled::<Stunned>(e_4096), "clearing page 0 must not touch page 1");
    }

    /// Dead / stale entities are silent no-ops for enable / disable / is_enabled.
    #[test]
    fn dead_entity_toggle_is_safe_no_op() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[TABLE_POS]);
        let e = spawn_pos(&mut ecs, arch);
        assert!(ecs.delete_entity(e), "delete must succeed");

        // The handle is now stale.
        assert!(!ecs.is_enabled::<Stunned>(e), "is_enabled on a dead entity is false");
        // These must not panic and must not allocate a column / bump generation.
        let enable_before = ecs.archetype_master().enable_generation();
        ecs.enable::<Stunned>(e);
        ecs.disable::<Stunned>(e);
        assert!(!ecs.is_enabled::<Stunned>(e));
        assert_eq!(
            ecs.archetype_master().enable_generation(),
            enable_before,
            "a dead-entity toggle must not allocate a column / bump enable_generation"
        );
    }

    /// `register_enable_tag` classifies the minted id as bitset and is
    /// idempotent per name.
    #[test]
    fn register_enable_tag_classifies_and_interns() {
        let mut ecs = EcsMaster::new();
        let a = ecs.register_enable_tag("enable_api_register_classify");
        let b = ecs.register_enable_tag("enable_api_register_classify");
        assert_eq!(a, b, "same name must return the same EnableTagId");
        assert_eq!(
            component_registry::storage_kind(a.component_id().0),
            StorageKind::Bitset,
            "register_enable_tag must classify the id as Bitset"
        );
        // The fallible twin agrees.
        let c = ecs
            .try_register_enable_tag("enable_api_register_classify")
            .expect("interned name is always a success");
        assert_eq!(a, c);
    }
}
