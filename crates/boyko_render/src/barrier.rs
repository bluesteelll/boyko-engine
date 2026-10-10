//! Barrier lowering (Phase 5 Wave D) — turn the schedule's abstract
//! producer→GPU-consumer edges + their [`GpuAccessIntent`]s into concrete
//! [`PlannedBarrier`]s that a [`GpuSystem`](crate::GpuSystem) REPLAYS as a
//! `vkCmdPipelineBarrier` on its device column before the compute dispatch.
//!
//! # Why a separate lowering pass (D6 / MF-6 / MF-7)
//!
//! `boyko_ecs` owns the ordering (the directed conflict-graph edge) and the
//! abstract intent (stage + read/write per device column); it names no `Vk*`
//! type. [`Schedule::gpu_barrier_inputs`](boyko_ecs::ecs::core::schedule::schedule::Schedule::gpu_barrier_inputs)
//! exposes those edges as a [`GpuBarrierEdge`] POD. This module is the ONLY
//! place that maps abstract `(GpuStage, GpuAccess)` → the `boyko_rhi`
//! [`BarrierStage`] / [`BarrierAccess`] masks, at schedule-BUILD time (cold),
//! into a per-consumer [`PlannedBarrier`] plan. The frame path then just walks
//! the plan and records the barrier — no per-frame graph walk.
//!
//! # The durable key (MF-7)
//!
//! A [`PlannedBarrier`] is keyed by the stable `(ArchetypeId, ComponentId)`
//! pair, NEVER a raw [`DeviceColumnHandle`](boyko_ecs::ecs::memory::device_column::DeviceColumnHandle) `u64`: a grow rotates the handle but
//! the pair is stable, so the [`GpuSystem`](crate::GpuSystem) resolves the pair
//! to the CURRENT device buffer each frame (one cold lookup, same indirect path
//! as its `target_key`). The build-time `u32` consumer index from
//! [`GpuBarrierEdge`] is TRANSIENT (O2) — it never leaves this pass; only the
//! durable key is persisted into the plan.
//!
//! # Superset-widen (D6)
//!
//! When the producer→consumer mapping is ambiguous (a producer with no declared
//! intent, or a multi-touch consumer), the lowering WIDENS rather than narrows:
//! the `src`/`dst` masks become the OR of the candidate stage/access bits. A
//! missing barrier is a sync-validation hazard (caught in Wave E); an over-wide
//! barrier is always sound (it only over-synchronises). The widen uses ONLY the
//! `boyko_rhi` constants that EXIST (`COMPUTE_SHADER | TRANSFER` for stages,
//! `SHADER_READ | SHADER_WRITE | TRANSFER_READ | TRANSFER_WRITE` for access) —
//! there is no `ALL_COMMANDS` / `MEMORY_*` in the foundation enum set (MF-8).

use boyko_ecs::ecs::core::schedule::schedule::GpuBarrierEdge;
use boyko_ecs::ecs::core::system::gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage};
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

use boyko_rhi::enums::{BarrierAccess, BarrierStage};

/// A planned `vkCmdPipelineBarrier` for one device column, lowered at schedule-
/// build time (cold) from a directed conflict edge + the two systems' intents
/// (Wave D / D6).
///
/// `#[repr(C)]` POD so a `Box<[PlannedBarrier]>` is a flat, cache-friendly run
/// the [`GpuSystem`](crate::GpuSystem) walks once before its dispatch. The
/// `key` is the DURABLE `(ArchetypeId, ComponentId)` pair (MF-7), resolved to
/// the current device buffer at replay time — NEVER a cached raw
/// [`DeviceColumnHandle`](boyko_ecs::ecs::memory::device_column::DeviceColumnHandle)
/// `u64` (a grow rotates the handle, the pair is stable).
///
/// Field order groups the two `BarrierStage`s, then the durable key, then the
/// two `BarrierAccess`es — each family adjacent for predictable reads when the
/// replay builds the `BarrierDesc` + `BufferBarrier`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedBarrier {
    /// Pipeline stage(s) that must complete before the barrier (the producer's
    /// stage — e.g. a prior `COMPUTE_SHADER` write).
    pub src_stage: BarrierStage,
    /// Pipeline stage(s) that wait on the barrier (the consumer's stage — e.g.
    /// this dispatch's `COMPUTE_SHADER` read/write).
    pub dst_stage: BarrierStage,
    /// The DURABLE `(archetype, component)` key of the column this barrier
    /// transitions (MF-7). Resolved to the current device buffer per frame.
    pub key: (ArchetypeId, ComponentId),
    /// Access scope before the barrier (the producer's access — e.g. a prior
    /// `SHADER_WRITE`).
    pub src_access: BarrierAccess,
    /// Access scope after the barrier (the consumer's access — e.g. this
    /// dispatch's `SHADER_READ | SHADER_WRITE`).
    pub dst_access: BarrierAccess,
}

impl PlannedBarrier {
    /// Returns the durable `(archetype, component)` key this barrier transitions
    /// (MF-7). The [`GpuSystem`](crate::GpuSystem) resolves it to the current
    /// device buffer at replay.
    #[inline]
    pub fn key(&self) -> (ArchetypeId, ComponentId) {
        self.key
    }
}

// ⚠️ `wide_stage()` — the `COMPUTE_SHADER | TRANSFER` stage superset — was DELETED at
// virtual-geometry rung R1, not merely left unused. Its only caller was `stage_of`'s `Indirect`
// arm, which existed because `BarrierStage::DRAW_INDIRECT` did not; R1 adds the constant and the
// arm becomes exact, so the helper has no reader. A widening helper kept "in case" is a thing an
// implementer greps for and reaches for, which is the defect class this campaign has repeatedly
// found in its own texts. `wide_access()` below survives because it still has one honest caller:
// Vulkan defines no indirect WRITE access, so that arm genuinely cannot be narrowed.

/// The widened superset of every buffer access the foundation's GPU systems can
/// perform (D6): `SHADER_READ | SHADER_WRITE | TRANSFER_READ | TRANSFER_WRITE`.
/// Used when the producer/consumer access is ambiguous (MF-8: no `MEMORY_*`).
#[inline]
fn wide_access() -> BarrierAccess {
    BarrierAccess::SHADER_READ
        | BarrierAccess::SHADER_WRITE
        | BarrierAccess::TRANSFER_READ
        | BarrierAccess::TRANSFER_WRITE
}

/// Maps an abstract [`GpuStage`] to its concrete `boyko_rhi` [`BarrierStage`].
///
/// Cold (build time).
///
/// ⚠️ **Virtual-geometry rung R1 NARROWED `Indirect`, and the widening it replaces was never a
/// design choice.** This arm returned the whole [`wide_stage`] superset *because
/// `BarrierStage::DRAW_INDIRECT` did not exist* — MF-8 forbids naming a constant the foundation does
/// not define, so widening was the only sound response available. R1 adds the constant, and the arm
/// becomes the one-line mapping it always should have been. Over-synchronisation was sound but not
/// free: it made every indirect-argument barrier wait on and block the entire compute+transfer
/// stage range, which is exactly the cost a GPU-decided cut exists to avoid.
#[inline]
fn stage_of(stage: GpuStage) -> BarrierStage {
    match stage {
        GpuStage::Compute => BarrierStage::COMPUTE_SHADER,
        GpuStage::Transfer => BarrierStage::TRANSFER,
        GpuStage::Indirect => BarrierStage::DRAW_INDIRECT,
    }
}

/// Maps an abstract `(GpuStage, GpuAccess)` to its concrete `boyko_rhi`
/// [`BarrierAccess`], on the side of the barrier (`src` = producer,
/// `dst` = consumer).
///
/// Cold (build time). The access family is chosen by the stage: a `Transfer`
/// stage uses the `TRANSFER_*` access bits, a `Compute` stage the `SHADER_*`
/// bits.
///
/// ⚠️ **Rung R1 narrowed `(Indirect, Read)` and deliberately LEFT `(Indirect, Write)` widened.**
/// The asymmetry is Vulkan's, not this function's: `VK_ACCESS_INDIRECT_COMMAND_READ_BIT` has **no
/// write counterpart**, because an indirect-argument buffer is *written* by a compute shader or a
/// transfer and only *read* by the `DRAW_INDIRECT` stage. So an `(Indirect, Write)` declaration is
/// incoherent — it names a stage that cannot write — and the sound response is still to widen rather
/// than to invent a bit or to silently substitute `SHADER_WRITE`, which would under-synchronise if
/// the producer was in fact a transfer.
#[inline]
fn access_of(stage: GpuStage, access: GpuAccess) -> BarrierAccess {
    match (stage, access) {
        (GpuStage::Compute, GpuAccess::Read) => BarrierAccess::SHADER_READ,
        (GpuStage::Compute, GpuAccess::Write) => BarrierAccess::SHADER_WRITE,
        (GpuStage::Transfer, GpuAccess::Read) => BarrierAccess::TRANSFER_READ,
        (GpuStage::Transfer, GpuAccess::Write) => BarrierAccess::TRANSFER_WRITE,
        (GpuStage::Indirect, GpuAccess::Read) => BarrierAccess::INDIRECT_COMMAND_READ,
        // See the note above: Vulkan has no indirect WRITE access, so this arm stays widened.
        (GpuStage::Indirect, GpuAccess::Write) => wide_access(),
    }
}

/// Folds an intent's declared touches into one widened `(stage, access)` mask
/// pair for the whole column-touch set (D6).
///
/// A GPU system may touch several columns at several accesses; for the
/// foundation's single-column barrier the precise per-column split is not
/// available from the edge alone (the durable barrier key comes from the
/// consumer's `target_key`, not the intent), so the lowering WIDENS to the OR of
/// every touch's mapped stage/access. An empty intent (a CPU producer touching
/// no device column) yields the bare stage mask with EMPTY access — the
/// barrier's `src` then names only the stage to wait on (the producer wrote
/// nothing GPU-visible to scope), which is still a sound dependency.
#[inline]
fn fold_intent(intent: &GpuAccessIntent) -> (BarrierStage, BarrierAccess) {
    let stage = stage_of(intent.stage());
    let mut access = BarrierAccess::NONE;
    for touch in intent.touches() {
        access = access | access_of(intent.stage(), touch.access);
    }
    (stage, access)
}

/// Lowers the schedule's GPU barrier-input edges into a per-consumer
/// [`PlannedBarrier`] plan (Phase 5 Wave D, schedule-build time, COLD).
///
/// For every [`GpuBarrierEdge`] whose consumer resolves to a durable
/// `(archetype, component)` key via `consumer_key`, emits ONE [`PlannedBarrier`]
/// keyed by that pair: the producer's intent maps to the barrier's `src`
/// (stage + access), the consumer's to its `dst` (D6 mapping). Edges whose
/// consumer has no resolvable key (a GPU system that touches no device column)
/// are skipped — there is nothing to synchronise.
///
/// The returned `Vec<(u32, Box<[PlannedBarrier]>)>` carries the TRANSIENT
/// consumer `SystemIndex.0` projection (O2): the build pass uses it to assign
/// each plan to its `GpuSystem`, then discards it — only the durable `key`
/// inside each [`PlannedBarrier`] is persisted. Multiple producer edges into the
/// same consumer accumulate into that consumer's single `Box<[PlannedBarrier]>`
/// (one entry per producer edge), so a GPU system fed by several producers
/// replays one barrier per upstream dependency.
///
/// # Parameters
/// - `edges`: the schedule's directed producer→GPU-consumer inputs (from
///   [`Schedule::gpu_barrier_inputs`](boyko_ecs::ecs::core::schedule::schedule::Schedule::gpu_barrier_inputs)).
/// - `consumer_key`: resolves a consumer's transient `u32` index to its durable
///   `(archetype, component)` target key — the build helper supplies each
///   `GpuSystem`'s `target_key` (a column-touching consumer always has one).
///
/// # Superset-widen (D6)
/// An ambiguous mapping widens to the OR of the existing stage/access bits
/// rather than omitting a barrier; over-synchronisation is sound, a missed
/// barrier trips sync-validation (Wave E).
pub fn lower_barriers(
    edges: impl Iterator<Item = GpuBarrierEdge>,
    mut consumer_key: impl FnMut(u32) -> Option<(ArchetypeId, ComponentId)>,
) -> Vec<(u32, Box<[PlannedBarrier]>)> {
    // Accumulate per-consumer plans, preserving first-seen consumer order. The
    // GPU-resident set is tiny in the stable-residency foundation (Regime-C), so
    // a linear find over the in-progress consumers is cheaper than a HashMap and
    // keeps the cold build pass allocation-light.
    let mut out: Vec<(u32, Vec<PlannedBarrier>)> = Vec::new();

    for edge in edges {
        let Some(key) = consumer_key(edge.consumer) else {
            // The consumer touches no resolvable device column — nothing to
            // synchronise on. (A GPU system with a real target always resolves.)
            continue;
        };

        let (src_stage, src_access) = fold_intent(&edge.producer_intent);
        let (dst_stage, dst_access) = fold_intent(&edge.consumer_intent);

        let barrier = PlannedBarrier {
            src_stage,
            dst_stage,
            key,
            src_access,
            dst_access,
        };

        match out.iter_mut().find(|(c, _)| *c == edge.consumer) {
            Some((_, plan)) => plan.push(barrier),
            None => out.push((edge.consumer, vec![barrier])),
        }
    }

    out.into_iter()
        .map(|(consumer, plan)| (consumer, plan.into_boxed_slice()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_ecs::ecs::memory::device_column::DeviceColumnHandle;

    /// A synthetic `GpuBarrierEdge` builder so the pure-logic tests need no
    /// `Schedule` (the lowering consumes only the public POD).
    fn edge(
        producer: u32,
        consumer: u32,
        producer_intent: GpuAccessIntent,
        consumer_intent: GpuAccessIntent,
    ) -> GpuBarrierEdge {
        GpuBarrierEdge {
            producer,
            consumer,
            producer_intent,
            consumer_intent,
        }
    }

    /// A `Compute`-stage intent that writes one synthetic device column.
    fn compute_write(handle: u64) -> GpuAccessIntent {
        let mut i = GpuAccessIntent::new(GpuStage::Compute);
        i.push(DeviceColumnHandle(handle), GpuAccess::Write);
        i
    }

    /// A `Compute`-stage intent that reads one synthetic device column.
    fn compute_read(handle: u64) -> GpuAccessIntent {
        let mut i = GpuAccessIntent::new(GpuStage::Compute);
        i.push(DeviceColumnHandle(handle), GpuAccess::Read);
        i
    }

    /// An empty `Compute`-stage intent (a CPU producer touching no device column).
    fn empty_compute() -> GpuAccessIntent {
        GpuAccessIntent::new(GpuStage::Compute)
    }

    const KEY: (ArchetypeId, ComponentId) = (ArchetypeId(3), ComponentId(7));

    /// Resolver: every consumer maps to the same durable key.
    fn key_for_all(_consumer: u32) -> Option<(ArchetypeId, ComponentId)> {
        Some(KEY)
    }

    #[test]
    fn directed_producer_consumer_yields_one_barrier_on_the_right_key() {
        // CPU producer (empty intent) → GPU consumer that writes the column.
        let edges = [edge(0, 1, empty_compute(), compute_write(42))];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        assert_eq!(plans.len(), 1, "one consumer => one plan entry");
        let (consumer, plan) = &plans[0];
        assert_eq!(*consumer, 1, "the transient consumer index is preserved");
        assert_eq!(plan.len(), 1, "one producer edge => one barrier");

        let b = plan[0];
        assert_eq!(b.key, KEY, "barrier keyed by the durable (archetype, component)");
        // Producer: empty intent → COMPUTE stage, NO access scope.
        assert_eq!(b.src_stage, BarrierStage::COMPUTE_SHADER);
        assert_eq!(b.src_access, BarrierAccess::NONE);
        // Consumer: COMPUTE write → COMPUTE stage, SHADER_WRITE.
        assert_eq!(b.dst_stage, BarrierStage::COMPUTE_SHADER);
        assert_eq!(b.dst_access, BarrierAccess::SHADER_WRITE);
    }

    #[test]
    fn gpu_to_gpu_edge_yields_read_after_write_barrier() {
        // GPU producer writes the column → GPU consumer reads it (RAW). The
        // barrier's src = the producer's COMPUTE write, dst = the consumer's
        // COMPUTE read — the load-bearing read-after-write ordering.
        let edges = [edge(0, 1, compute_write(9), compute_read(9))];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        let (_, plan) = &plans[0];
        let b = plan[0];
        assert_eq!(b.src_stage, BarrierStage::COMPUTE_SHADER);
        assert_eq!(b.src_access, BarrierAccess::SHADER_WRITE, "src = prior write");
        assert_eq!(b.dst_stage, BarrierStage::COMPUTE_SHADER);
        assert_eq!(b.dst_access, BarrierAccess::SHADER_READ, "dst = this read");
    }

    #[test]
    fn gpu_to_gpu_write_after_write_barrier() {
        // GPU producer writes → GPU consumer writes the same column (WAW).
        let edges = [edge(0, 1, compute_write(9), compute_write(9))];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        let b = plans[0].1[0];
        assert_eq!(b.src_access, BarrierAccess::SHADER_WRITE);
        assert_eq!(b.dst_access, BarrierAccess::SHADER_WRITE);
    }

    #[test]
    fn multi_touch_consumer_widens_to_or_of_access_bits() {
        // A consumer that both reads AND writes its column (two touches) widens
        // the dst access to the OR of the mapped bits — SHADER_READ | SHADER_WRITE.
        let mut consumer = GpuAccessIntent::new(GpuStage::Compute);
        consumer.push(DeviceColumnHandle(1), GpuAccess::Read);
        consumer.push(DeviceColumnHandle(2), GpuAccess::Write);

        let edges = [edge(0, 1, compute_write(1), consumer)];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        let b = plans[0].1[0];
        assert_eq!(
            b.dst_access,
            BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
            "multi-touch read+write widens to the OR of the existing access bits"
        );
        assert_eq!(b.dst_stage, BarrierStage::COMPUTE_SHADER);
    }

    /// **Virtual-geometry rung R1 replaced `indirect_stage_widens_to_full_superset`, and the
    /// replacement is a pair rather than an inversion.**
    ///
    /// The retired test froze `Indirect` widening BOTH stage and access to the whole
    /// `COMPUTE_SHADER | TRANSFER` × `SHADER|TRANSFER read/write` superset. That behaviour was
    /// sound and was never a design choice: `BarrierStage::DRAW_INDIRECT` did not exist, and MF-8
    /// forbids naming a constant the foundation does not define, so widening was the only response
    /// available. R1 adds the constant; the read arm becomes exact.
    ///
    /// What the pair asserts that a single inverted test would not: the **asymmetry survives**. The
    /// write arm still widens, because Vulkan has no indirect-write access bit at all — an
    /// indirect-argument buffer is written by compute or transfer and only read by `DRAW_INDIRECT`.
    /// Substituting `SHADER_WRITE` there would UNDER-synchronise whenever the real producer was a
    /// transfer, which is the one direction a barrier may never be wrong in.
    #[test]
    fn indirect_read_is_exact_and_indirect_write_still_widens() {
        // READ: the arm R1 narrowed. Exactly the indirect fetch stage and its one access bit.
        let mut producer = GpuAccessIntent::new(GpuStage::Indirect);
        producer.push(DeviceColumnHandle(5), GpuAccess::Read);
        let edges = [edge(0, 1, producer, compute_read(5))];
        let b = lower_barriers(edges.into_iter(), key_for_all)[0].1[0];
        assert_eq!(
            b.src_stage,
            BarrierStage::DRAW_INDIRECT,
            "an Indirect READ is exactly the DRAW_INDIRECT stage -- no superset"
        );
        assert_eq!(
            b.src_access,
            BarrierAccess::INDIRECT_COMMAND_READ,
            "an Indirect READ is exactly INDIRECT_COMMAND_READ -- no superset"
        );
        // ...and the narrowing is real, not a relabelling: the old superset is strictly wider.
        assert_ne!(
            b.src_stage,
            BarrierStage::COMPUTE_SHADER | BarrierStage::TRANSFER,
            "the pre-R1 stage superset must no longer be produced"
        );

        // WRITE: the arm R1 deliberately LEFT widened, because Vulkan offers nothing narrower.
        let mut writer = GpuAccessIntent::new(GpuStage::Indirect);
        writer.push(DeviceColumnHandle(6), GpuAccess::Write);
        let edges = [edge(0, 1, writer, compute_read(6))];
        let w = lower_barriers(edges.into_iter(), key_for_all)[0].1[0];
        assert_eq!(
            w.src_access,
            BarrierAccess::SHADER_READ
                | BarrierAccess::SHADER_WRITE
                | BarrierAccess::TRANSFER_READ
                | BarrierAccess::TRANSFER_WRITE,
            "an Indirect WRITE is incoherent in Vulkan's model, so it still widens rather than \
             guessing between SHADER_WRITE and TRANSFER_WRITE"
        );
    }

    #[test]
    fn transfer_stage_maps_to_transfer_access_family() {
        // A `Transfer`-stage producer (e.g. a staging upload) maps to the
        // TRANSFER stage + TRANSFER_WRITE access — the correct family for the
        // upload→compute hand-off.
        let mut producer = GpuAccessIntent::new(GpuStage::Transfer);
        producer.push(DeviceColumnHandle(8), GpuAccess::Write);

        let edges = [edge(0, 1, producer, compute_read(8))];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        let b = plans[0].1[0];
        assert_eq!(b.src_stage, BarrierStage::TRANSFER);
        assert_eq!(b.src_access, BarrierAccess::TRANSFER_WRITE);
    }

    #[test]
    fn consumer_with_no_resolvable_key_is_skipped() {
        // A consumer the resolver cannot key (touches no device column) yields no
        // barrier — there is nothing to synchronise.
        let edges = [edge(0, 1, empty_compute(), empty_compute())];
        let plans = lower_barriers(edges.into_iter(), |_| None);
        assert!(plans.is_empty(), "an unkeyable consumer produces no plan");
    }

    #[test]
    fn multiple_producers_accumulate_into_one_consumer_plan() {
        // Two producers feeding the SAME consumer accumulate into one plan with
        // two barriers (one per upstream dependency) — the property that every
        // covered consumer gets >= 1 barrier per producer edge.
        let edges = [
            edge(0, 2, compute_write(1), compute_read(1)),
            edge(1, 2, compute_write(1), compute_read(1)),
        ];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        assert_eq!(plans.len(), 1, "both edges target the same consumer => one plan");
        assert_eq!(plans[0].0, 2);
        assert_eq!(plans[0].1.len(), 2, "one barrier per producer edge");
    }

    #[test]
    fn every_keyed_consumer_gets_at_least_one_barrier() {
        // The covering property: every GPU consumer with a resolvable column key
        // receives >= 1 barrier (never a missed barrier). Three distinct
        // consumers, each with one producer.
        let edges = [
            edge(0, 1, empty_compute(), compute_write(1)),
            edge(0, 2, empty_compute(), compute_read(2)),
            edge(0, 3, compute_write(3), compute_write(3)),
        ];
        let plans = lower_barriers(edges.into_iter(), key_for_all);

        assert_eq!(plans.len(), 3, "three distinct consumers => three plans");
        for (consumer, plan) in &plans {
            assert!(
                !plan.is_empty(),
                "consumer {consumer} must get at least one covering barrier"
            );
        }
    }
}
