//! The hand-written [`GpuSystem`] — a `boyko_ecs` [`System`] that dispatches the
//! `gpu_integrate` compute shader ON the GPU-resident ECS column (Phase 5 Wave C,
//! MF-5: the deep GpuSystem mechanism).
//!
//! # Why hand-written (MF-5)
//!
//! The `NonSendResMut<RhiContext>` `SystemParam` route is structurally wrong for a
//! GPU system: `NonSendResMut::init_access` calls `mark_universal()`, which forces
//! the system to `SystemKind::CpuExclusive` + universal access — contradicting the
//! GpuSystem's contract of EMPTY access + the `is_gpu` marker. So `GpuSystem` is a
//! HAND-WRITTEN `impl System` that:
//!
//! 1. declares EMPTY component/resource [`Access`] (no conflict-graph access
//!    edges) + carries a `GpuAccessIntent` on its [`SystemMeta`];
//! 2. is registered as `SystemKind::GpuCompute` via `SystemConfig::gpu()`
//!    (build-time) → `runs_on_dispatcher()` → EXC2-solo at `running == 0`;
//! 3. reaches the `!Send` [`RhiContext`] by projecting it from the world's NonSend
//!    slab through the dispatcher-only [`DispatcherToken`] inside
//!    [`run_dispatcher`](System::run_dispatcher) — replicating
//!    `NonSendResMut::get_param`'s projection WITHOUT `mark_universal` (the load-
//!    bearing MF-5 mechanism), via the Phase 5 Option C capability instead of a
//!    raw cell accessor. SOUND: GpuCompute ⇒ dispatcher-solo at `running == 0` ⇒
//!    the single-thread-touch discipline the `!Send` `RhiContext` needs holds,
//!    identical to NonSend (see `ecs_master.rs` SEND10 + `system_kind.rs`), without
//!    the param's universal-access side effect.
//!
//! # Why a token, not a raw cell accessor (Phase 5 Option C)
//!
//! Wave C reached the `!Send` `RhiContext` through a PUBLIC
//! `UnsafeEcsCell::nonsend_resource_mut`. That accessor was reachable on the
//! concurrent WORKER path (C1) and its `'w` return lifetime allowed two live
//! `&mut R` aliases (M1) — both real UB. The accessor was DELETED. A `GpuSystem`
//! now reaches the context ONLY through a [`DispatcherToken`], which the
//! scheduler mints solely on the dispatcher-solo path (and `run_system_once`):
//! a worker never sees one, and the token's `&mut self` projection lifetime
//! makes a second `&mut R` un-aliasable. The capability replaces the contract.
//!
//! # Per-frame indirection (MF-7)
//!
//! `GpuSystem` stores a `target_key: (ArchetypeId, ComponentId)`, NEVER a raw
//! `DeviceColumnHandle` `u64`. Each dispatch resolves the current device column
//! through `GpuColumnManager::resolve(archetype, component)` (one cold lookup), so a
//! grow that rotates the handle is transparent — there is no stale `u64` to cache.

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::dispatcher_token::DispatcherToken;
use boyko_ecs::ecs::core::system::gpu_intent::GpuAccessIntent;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

use boyko_rhi::ComputePipelineHandle;

use crate::gpu_column::RhiContext;

/// The committed `gpu_integrate` SPIR-V (`Data[i] = Data[i] + 100`), embedded at
/// compile time. The byte length must be a 4-byte multiple (a valid `.spv`); the
/// `SpirvBlob` wrapper guarantees 4-byte alignment so it can be re-viewed as the
/// `&[u32]` word stream `RhiDevice::create_shader_module` requires.
static GPU_INTEGRATE_SPV: SpirvBlob<968> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gpu_integrate.comp.spv"
)));

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
///
/// A bare `include_bytes!` is only `align(1)`; SPIR-V requires its `u32` word
/// stream to be 4-byte aligned. Mirrors the `boyko_rhi_vulkan::compute::SpirvBlob`
/// trick that aligns the Slice-0 compute shaders.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream after a magic-number check.
    #[inline]
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        const { assert!(N >= 4, "SPIR-V blob must hold at least the magic word") };
        // Confirm the embedded blob starts with the SPIR-V magic `0x07230203`
        // (little-endian: bytes `03 02 23 07`) — a misplaced / non-SPIR-V file fails
        // loud HERE rather than passing a corrupt / wrong-endian word stream to
        // `vkCreateShaderModule`. RELEASE-PRESENT (a plain `assert!`, not a
        // `debug_assert!`): a committed blob is validated once per process at
        // pipeline build, never on a hot loop, so the check costs nothing
        // measurable but cannot be elided in release — a bad blob can never reach
        // the driver unflagged. (A `const` magic check is not feasible: `N` is
        // generic, so `self.0[..4]` is a runtime read of the embedded bytes.)
        assert_eq!(
            [self.0[0], self.0[1], self.0[2], self.0[3]],
            [0x03, 0x02, 0x23, 0x07],
            "SPIR-V blob does not start with the magic number 0x07230203 \
             (corrupt, wrong-endian, or not a .spv file)"
        );
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid
        // `*const u32`; `N` is a 4-byte multiple (const-asserted above), so the blob
        // is exactly `N / 4` whole `u32` words; the `&self` borrow keeps the backing
        // `'static` blob alive for the slice's lifetime. Any byte pattern is a valid
        // `u32`, so no uninitialized/invalid read occurs.
        unsafe { core::slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed `gpu_integrate` compute SPIR-V as a `u32` word stream, ready for
/// [`RhiContext::create_compute_pipeline`].
///
/// The shader (`shaders/gpu_integrate.hlsl`, compiled offline to
/// `gpu_integrate.comp.spv`) is `RWStructuredBuffer<uint> Data : register(u0)` (set
/// 0, binding 0, STORAGE at COMPUTE) + a `uint count` push constant,
/// `[numthreads(64,1,1)]`, `Data[i] = Data[i] + 100` with an `if (i >= count)
/// return;` bounds check — so dispatch `ceil(count / 64)` groups (MF-5 shader
/// contract).
#[inline]
pub fn gpu_integrate_spirv() -> &'static [u32] {
    GPU_INTEGRATE_SPV.as_words()
}

/// A planned `vkCmdPipelineBarrier` for one device column, lowered at schedule-
/// build time from a directed conflict edge + the two systems' intents (Wave D).
///
/// `#[repr(C)]` POD so a `Box<[PlannedBarrier]>` is a flat, cache-friendly run.
/// For Wave C this slice is EMPTY (no barrier replay yet — Wave D fills it). The
/// `buffer` key is the durable `(archetype, component)`-resolved handle at replay
/// time, never a cached raw `u64` (MF-7); Wave D resolves it indirectly each frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedBarrier {
    /// The archetype of the column this barrier transitions (durable key, part 1).
    pub archetype: ArchetypeId,
    /// The component of the column this barrier transitions (durable key, part 2).
    pub component: ComponentId,
}

/// A hand-written `boyko_ecs` [`System`] that records + submits the `gpu_integrate`
/// compute dispatch on a GPU-resident ECS column (Phase 5 Wave C / MF-5).
///
/// Declares EMPTY [`Access`] and resolves to `SystemKind::GpuCompute` (set on its
/// `SystemConfig` via `.gpu()` at schedule-build time, OR inferred from its
/// `is_gpu()` trait override — Phase 5 Option C), so the scheduler dispatches it
/// solo on the dispatcher thread inside the apply window (`running == 0`). Its
/// [`run_dispatcher`](System::run_dispatcher) projects the `!Send` [`RhiContext`]
/// from the world's NonSend slab through the dispatcher-only [`DispatcherToken`],
/// resolves the target column indirectly by `(archetype, component)` (MF-7), and
/// dispatches the compute pass (bind → push `count` → `dispatch(ceil(len/64))` →
/// submit → fence).
pub struct GpuSystem {
    /// The compute pipeline handle (built once at setup from `gpu_integrate.spv`,
    /// owned by the manager's registry). Resolved per frame against the registry.
    pipeline: ComputePipelineHandle,

    /// The DURABLE key of the device column this system dispatches over (MF-7).
    /// Resolved indirectly each frame; NEVER a cached raw `DeviceColumnHandle`.
    target_key: (ArchetypeId, ComponentId),

    /// The abstract GPU access descriptor (stage + per-column touches). Stored on
    /// the [`SystemMeta`] at construction so Wave D's barrier lowering can read it
    /// off the schedule.
    intent: GpuAccessIntent,

    /// The lowered per-system barrier plan, replayed on the frame path before the
    /// dispatch (Wave D). EMPTY for Wave C — no barrier is replayed yet.
    barriers: Box<[PlannedBarrier]>,

    /// Per-system metadata (name, access, tick snapshots, the GPU intent). The
    /// `Access` stays EMPTY; `gpu_intent` is set from `intent` at construction.
    meta: SystemMeta,
}

impl GpuSystem {
    /// Constructs a `GpuSystem` over the device column keyed by `target_key`,
    /// dispatching the compute `pipeline` with the declared `intent` and `barriers`
    /// (MF-5 Key-types shape).
    ///
    /// The `pipeline` is created once at setup via
    /// [`RhiContext::create_compute_pipeline`] from the committed
    /// `gpu_integrate.comp.spv`. `target_key` is the durable `(archetype,
    /// component)` pair resolved indirectly each frame (MF-7). `intent` is stamped
    /// onto the system's [`SystemMeta`] so Wave D can lower barriers; `barriers` is
    /// the replay plan (EMPTY for Wave C).
    ///
    /// The system declares EMPTY [`Access`] — it touches no CPU column — and is
    /// expected to be registered with `SystemConfig::gpu()` so the scheduler
    /// resolves it to `SystemKind::GpuCompute` (dispatcher-solo).
    pub fn new(
        pipeline: ComputePipelineHandle,
        target_key: (ArchetypeId, ComponentId),
        intent: GpuAccessIntent,
        barriers: Box<[PlannedBarrier]>,
    ) -> Self {
        // `Tick::new(1)` is the construction-time sentinel (the dispatcher
        // overwrites the snapshot via `set_change_ticks` before the first run); the
        // GpuSystem itself never consumes change-detection ticks (empty access).
        let mut meta = SystemMeta::new(std::any::type_name::<GpuSystem>(), Tick::new(1));
        meta.set_gpu_intent(intent.clone());
        Self {
            pipeline,
            target_key,
            intent,
            barriers,
            meta,
        }
    }

    /// Returns the durable `(archetype, component)` target key (MF-7). The system
    /// resolves the current device handle from this each frame — it never caches
    /// the raw `u64`.
    #[inline]
    pub fn target_key(&self) -> (ArchetypeId, ComponentId) {
        self.target_key
    }

    /// Returns the declared abstract GPU access intent (stage + per-column touches).
    #[inline]
    pub fn intent(&self) -> &GpuAccessIntent {
        &self.intent
    }

    /// Returns the lowered per-system barrier plan (EMPTY for Wave C).
    #[inline]
    pub fn barriers(&self) -> &[PlannedBarrier] {
        &self.barriers
    }
}

// SAFETY (S1' + MF-5 / Option C): `run_dispatcher` reaches the world ONLY to
//   project the `!Send` `RhiContext` from the NonSend slab (through the
//   dispatcher-only `DispatcherToken`) and record/submit GPU work; it performs
//   NO CPU component access (the declared `Access` is empty). The aliasing
//   contract S1' is upheld by the scheduler: a `GpuCompute` system runs
//   dispatcher-solo at `running == 0` (no other system body is in flight on the
//   same world), and the token is mintable only in that context — the
//   `DispatcherToken::nonsend_resource_mut` projection's own SAFETY block
//   documents the single-thread-touch invariant for the `!Send` payload.
//   `run_unsafe` is an unreachable-by-design no-op (a worker has no token, so the
//   `!Send` `RhiContext` is structurally unreachable there).
unsafe impl System for GpuSystem {
    type Out = ();

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name()
    }

    /// EMPTY component/resource access — the GpuSystem touches no CPU column, so it
    /// adds NO conflict-graph access edges (MF-5). Ordering against a CPU producer
    /// is an explicit `.after(producer)` directed edge, not an access conflict.
    #[inline]
    fn access(&self) -> &Access {
        self.meta.access()
    }

    /// Phase 5 Option C — the defense-in-depth GPU marker. The schedule builder
    /// ORs this with the explicit `SystemConfig::gpu()` descriptor flag, so a
    /// `GpuSystem` resolves `SystemKind::GpuCompute` (dispatcher-solo) even if the
    /// caller forgot the explicit opt-in.
    #[inline]
    fn is_gpu(&self) -> bool {
        true
    }

    /// No two-phase init: the access surface is already EMPTY by construction and
    /// the GPU intent is stamped at `new`. Idempotent no-op.
    fn initialize(&mut self, _world: &mut EcsMaster) {}

    /// The worker path. A `GpuSystem` is `SystemKind::GpuCompute`, dispatched
    /// SOLO on the dispatcher via [`run_dispatcher`](System::run_dispatcher) — it
    /// must NEVER run on a worker. It has no [`DispatcherToken`], so the `!Send`
    /// `RhiContext` is STRUCTURALLY UNREACHABLE here (the Option-C C1 kill). A
    /// loud debug panic flags a scheduler bug; a benign release no-op.
    ///
    /// # Safety
    ///
    /// **S1** — Vacuous: this body touches no world state.
    unsafe fn run_unsafe(&mut self, _cell: UnsafeEcsCell<'_>) -> Self::Out {
        debug_assert!(
            false,
            "GpuSystem ran on a worker via run_unsafe; it must be \
             SystemKind::GpuCompute and dispatched solo via run_dispatcher \
             (register it with SystemConfig::gpu() or rely on is_gpu())"
        );
    }

    /// Records + submits the `gpu_integrate` dispatch on the GPU-resident column
    /// (MF-5 step b/c/d), reaching the `!Send` `RhiContext` through the
    /// dispatcher-only [`DispatcherToken`] (Phase 5 Option C).
    ///
    /// # Safety
    ///
    /// **S1'** — The caller must guarantee `running == 0` on the dispatcher (no
    /// worker live). The scheduler upholds this by dispatching a `GpuCompute`
    /// system solo on the dispatcher at `running == 0`; the [`DispatcherToken`]
    /// is mintable only in that context, so receiving one IS the witness.
    unsafe fn run_dispatcher(&mut self, mut token: DispatcherToken<'_>) -> Self::Out {
        // (a) Project the `!Send` RhiContext from the world's NonSend slab — the
        // MF-5 mechanism, now via the Option-C `DispatcherToken`: it replicates
        // `NonSendResMut::get_param`'s projection WITHOUT declaring universal
        // access, and is reachable ONLY on the dispatcher-solo path (no worker
        // holds a token). A GpuSystem with no registered RhiContext is a setup bug.
        //
        // SAFETY (MF-5, Option C): the token witnesses `running == 0` (it is
        //   mintable only on the dispatcher-solo path). At that point no worker
        //   holds an aliasing cell, so reaching the `!Send` `RhiContext`
        //   single-threaded on its owning thread is the same
        //   external-synchronisation discipline `NonSendResMut` relies on
        //   (ecs_master.rs SEND10) — without the param's universal-access
        //   promotion. The projection's `&mut self` lifetime makes the `&mut
        //   RhiContext` the unique live borrow (M1).
        let ctx = match token.nonsend_resource_mut::<RhiContext>() {
            Some(ctx) => ctx,
            None => {
                debug_assert!(
                    false,
                    "GpuSystem::run_dispatcher: no RhiContext NonSend resource registered \
                     (insert it via EcsMaster::insert_non_send_resource before dispatch)"
                );
                return;
            }
        };

        // (b)+(c)+(d) Resolve the target column INDIRECTLY by (archetype, component)
        // (MF-7), bind the pipeline + the device buffer as storage binding 0, push
        // the live row count as `count`, dispatch `ceil(len/64)` groups, submit +
        // fence. Wave C uses a straightforward submit+wait; deferred-wait overlap is
        // a Phase-6 refinement (the manager owns the seam).
        let (archetype, component) = self.target_key;
        match ctx.dispatch_compute(self.pipeline, archetype, component) {
            Ok(true) => {}
            Ok(false) => {
                // The column did not resolve — a stale key or a not-yet-created
                // column. Loud in debug (a GpuSystem should always have a live
                // target by the time it dispatches); a benign skip in release.
                debug_assert!(
                    false,
                    "GpuSystem::run_dispatcher: target column ({archetype:?}, {component:?}) \
                     did not resolve — was create_column called for this key?"
                );
            }
            Err(e) => gpu_dispatch_failed(&e),
        }
    }

    /// No deferred mutations — the GpuSystem flushes nothing back into the world
    /// (its effects live entirely in VRAM). No-op `apply` (MF-5).
    #[inline]
    fn apply(&mut self, _world: &mut EcsMaster) {}

    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    #[inline]
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.set_change_ticks(last_run, this_run);
    }

    #[inline]
    fn check_change_tick(&mut self, current: Tick) {
        self.meta.clamp_change_ticks(current);
    }
}

/// Cold diagnostic for a GPU dispatch RHI failure inside `run_unsafe`.
///
/// `run_unsafe` has no `Result` channel (the `System` trait body returns `()`), so
/// an RHI failure during the GPU dispatch is reported here. Pulled out `#[cold]
/// #[inline(never)]` so the error path never bloats the hot `run_unsafe` body's
/// I-cache. In debug it panics (the validation oracle would already have flagged a
/// device fault); in release it logs loudly without aborting the frame.
#[cold]
#[inline(never)]
fn gpu_dispatch_failed(error: &crate::error::GpuColumnError) {
    debug_assert!(false, "GpuSystem::run_unsafe: GPU dispatch failed: {error}");
    eprintln!("boyko_render: GpuSystem dispatch failed: {error}");
}
