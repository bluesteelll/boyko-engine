//! Phase 4 Seam 4 — [`GpuAccessIntent`], the abstract per-system GPU
//! descriptor (D7 + IM-6 + Q3).
//!
//! `boyko_ecs` is GPU-capable but **graphics-pure**: this module names no
//! `Vk*`/`boyko_rhi` type. The conflict graph (unchanged) gives undirected
//! ordering edges; a GPU system additionally declares, via a
//! `GpuAccessIntent`, WHICH device columns it touches and HOW (stage +
//! read/write). A future `boyko_render` lowers `(edge, intent_src,
//! intent_dst)` → `vkCmdPipelineBarrier` — the lowering needs `Vk*` and stays
//! in `boyko_render`; the abstract intent CAN live in core.
//!
//! # 0%-gate
//!
//! `SystemMeta` carries the intent as `Option<Box<GpuAccessIntent>>` (8 B),
//! `None` for every CPU system → zero alloc, zero deref. The boxed-with-
//! fixed-inline-array shape (Q3) caps the per-system touch count at a small
//! `N` while keeping `SystemMeta` 8 B heavier in the tail padding only.

use crate::ecs::memory::device_column::DeviceColumnHandle;

/// Maximum number of device columns a single GPU system may declare it
/// touches (Q3 — fixed inline array; raise if a real GPU system needs more).
///
/// Eight covers the foundation's expected GPU systems (a compute pass over a
/// handful of SoA columns); a system that needs more should be split.
pub const MAX_GPU_TOUCHES: usize = 8;

/// Pipeline stage at which a GPU system accesses its device columns
/// (graphics-pure abstraction — `boyko_render` lowers it to a Vulkan stage
/// mask).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GpuStage {
    /// A compute-shader dispatch.
    Compute = 0,
    /// A transfer (copy / upload / download) operation.
    Transfer = 1,
    /// An indirect-draw / indirect-dispatch argument fetch.
    Indirect = 2,
}

/// How a GPU system accesses one device column (read vs write) — the abstract
/// twin of an `Access` read/write bit, scoped to a device column.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GpuAccess {
    /// The system reads the column.
    Read = 0,
    /// The system writes the column.
    Write = 1,
}

/// One declared (column, access) touch inside a [`GpuAccessIntent`].
///
/// `#[repr(C)]` + `Copy` POD so the inline `touches` array is a flat,
/// cache-friendly run of records (an 8-byte handle, a 1-byte access, and
/// padding). Carries only the opaque [`DeviceColumnHandle`] `u64` — never a
/// pointer, never a graphics type.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GpuTouch {
    /// The opaque device-column token this touch addresses.
    pub column: DeviceColumnHandle,
    /// Whether the touch reads or writes the column.
    pub access: GpuAccess,
}

/// Abstract per-system GPU access descriptor (D7).
///
/// Declared by a GPU-compute system; carried on `SystemMeta` as
/// `Option<Box<GpuAccessIntent>>` (`None` for every CPU system → 0%). The
/// conflict graph still owns ordering; this only records the stage + the
/// per-column accesses a future `boyko_render` needs to emit a precise
/// barrier. Core exposes honest abstract edges + intents; the `Vk*` lowering
/// (and the superset-correct over-synchronisation obligation) lives in
/// `boyko_render` (Phase 5).
///
/// # `touches` storage (Q3)
///
/// A fixed inline `[GpuTouch; MAX_GPU_TOUCHES]` plus a `len` — no heap
/// indirection beyond the single `Box` that wraps the whole intent on
/// `SystemMeta`. Caps the touch count at [`MAX_GPU_TOUCHES`]; a system that
/// needs more should be split (graphics-foundation scope).
#[derive(Clone, Debug)]
pub struct GpuAccessIntent {
    /// The pipeline stage at which the declared touches occur.
    pub stage: GpuStage,
    /// Inline touch records; only `touches[..len]` are valid.
    touches: [GpuTouch; MAX_GPU_TOUCHES],
    /// Number of valid entries in `touches` (`<= MAX_GPU_TOUCHES`).
    len: u8,
}

impl GpuAccessIntent {
    /// Constructs an intent at `stage` with no touches yet.
    #[inline]
    pub fn new(stage: GpuStage) -> Self {
        Self {
            stage,
            // A zeroed `DeviceColumnHandle(0)` + `GpuAccess::Read` filler; only
            // `touches[..len]` is ever read, so the padding entries are inert.
            touches: [GpuTouch {
                column: DeviceColumnHandle(0),
                access: GpuAccess::Read,
            }; MAX_GPU_TOUCHES],
            len: 0,
        }
    }

    /// Records that this system touches `column` with `access`.
    ///
    /// # Panics
    /// Panics (release-present) if the touch count would exceed
    /// [`MAX_GPU_TOUCHES`] — a setup-time programmer error (a GPU system
    /// declaring more device columns than the fixed array holds). Setup-only;
    /// not on any hot path.
    #[inline]
    pub fn push(&mut self, column: DeviceColumnHandle, access: GpuAccess) {
        let i = self.len as usize;
        assert!(
            i < MAX_GPU_TOUCHES,
            "GpuAccessIntent: too many touches (max {MAX_GPU_TOUCHES}); split the system"
        );
        self.touches[i] = GpuTouch { column, access };
        self.len += 1;
    }

    /// Returns the declared pipeline stage.
    #[inline]
    pub fn stage(&self) -> GpuStage {
        self.stage
    }

    /// Returns the declared touches (the valid `touches[..len]` prefix).
    #[inline]
    pub fn touches(&self) -> &[GpuTouch] {
        &self.touches[..self.len as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_discriminants_match_plan() {
        assert_eq!(GpuStage::Compute as u8, 0);
        assert_eq!(GpuStage::Transfer as u8, 1);
        assert_eq!(GpuStage::Indirect as u8, 2);
        assert_eq!(GpuAccess::Read as u8, 0);
        assert_eq!(GpuAccess::Write as u8, 1);
    }

    #[test]
    fn push_and_read_touches() {
        let mut intent = GpuAccessIntent::new(GpuStage::Compute);
        assert_eq!(intent.stage(), GpuStage::Compute);
        assert!(intent.touches().is_empty());

        intent.push(DeviceColumnHandle(7), GpuAccess::Write);
        intent.push(DeviceColumnHandle(9), GpuAccess::Read);

        let t = intent.touches();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].column, DeviceColumnHandle(7));
        assert_eq!(t[0].access, GpuAccess::Write);
        assert_eq!(t[1].column, DeviceColumnHandle(9));
        assert_eq!(t[1].access, GpuAccess::Read);
    }

    #[test]
    #[should_panic(expected = "too many touches")]
    fn push_beyond_capacity_panics() {
        let mut intent = GpuAccessIntent::new(GpuStage::Transfer);
        for _ in 0..(MAX_GPU_TOUCHES + 1) {
            intent.push(DeviceColumnHandle(1), GpuAccess::Read);
        }
    }
}
