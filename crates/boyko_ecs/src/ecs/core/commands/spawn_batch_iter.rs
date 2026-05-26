//! Phase 12.5 Opt-A2 — `SpawnBatchIter<'a, 's, B>` user-facing iter over
//! reserved entity IDs.
//!
//! See `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` §5.2 / W5 for the
//! design (the `I` type parameter was dropped per W5 — the iter walks a
//! plain `Range<usize>` and yields `Entity`, leaving the bundle iterator
//! state inside the enqueued `SpawnBatchCommand`).
//!
//! # Drop semantics (SBO8b — I-N2)
//!
//! Dropping the iter WITHOUT consuming it has **no semantic effect on
//! the spawn**: the underlying `SpawnBatchCommand` is already enqueued
//! and runs in full on the next apply. Entity IDs remain reserved
//! (counter advanced); not consuming just discards the user-visible ID
//! list.

#![allow(dead_code)]

use core::marker::PhantomData;
use core::ops::Range;

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::system::params::commands::Commands;
use crate::ecs::identifiers::primitives::EntityId;

/// Phase 12.5 Opt-A2 (§5.2 / W5): iterator over reserved entity IDs
/// returned by [`Commands::spawn_batch`].
///
/// **W5 RESOLUTION**: dropped the dead `I` type parameter. The iter
/// walks `range: Range<usize>` and yields `Entity` — the bundle iterator
/// type is irrelevant to the user-facing iter, and leaking it would
/// force every caller signature to name an unnameable type. Retaining
/// `B` for ergonomic discoverability (`SpawnBatchIter<'_, '_, EnemyBundle>`
/// reads as "iter of entity IDs spawned from EnemyBundle") and to lock
/// in the spawn-type at the type-system level.
///
/// # `!Send + !Sync`
///
/// The iter carries a `PhantomData<&'a mut Commands<'s>>` which is
/// `!Sync` (the underlying `Commands<'s>::queue` is `&'s mut CommandQueue`).
/// Workers cannot share an iter across threads — by design.
pub struct SpawnBatchIter<'a, 's, B> {
    range: Range<usize>,
    _phantom: PhantomData<(&'a mut Commands<'s>, B)>,
}

impl<'a, 's, B> SpawnBatchIter<'a, 's, B> {
    /// `pub(crate)` — produced exclusively by
    /// [`Commands::spawn_batch`](Commands::spawn_batch).
    #[inline]
    pub(crate) fn new(range: Range<usize>) -> Self {
        Self {
            range,
            _phantom: PhantomData,
        }
    }
}

impl<'a, 's, B> Iterator for SpawnBatchIter<'a, 's, B> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        self.range.next().map(|id| Entity::new(EntityId(id), 0))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, 's, B> ExactSizeIterator for SpawnBatchIter<'a, 's, B> {
    #[inline]
    fn len(&self) -> usize {
        self.range.len()
    }
}
