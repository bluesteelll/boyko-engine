//! Shared, backend-agnostic RHI error categories.
//!
//! [`RhiError`] carries only the *control-flow* categories an agnostic caller
//! (`boyko_render`, the Phase-4 core seam) must branch on — device loss,
//! out-of-memory, an unsupported seam, the two swapchain freshness signals, and
//! a catch-all opaque backend code. Backends keep their rich, command-name +
//! native-result diagnostic in their own per-backend `Error` enum and project
//! it down to this set with a hand-written `impl From<BackendError> for RhiError`
//! (per plan D4: one direction only, no blanket → no coherence collision).
//!
//! The trait bound is `Self::Error: From<RhiError>` (the reverse direction), so
//! a seam stub can write `Err(RhiError::Unsupported("...").into())`.

/// Backend-agnostic RHI error categories.
///
/// This is `Copy` because every variant is a discriminant plus at most a
/// `&'static str` — there is no owned allocation, no erasure, no `Box`. Cheap to
/// pass by value on the cold `Err` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhiError {
    /// The logical device was lost (driver crash, TDR, removal). Unrecoverable
    /// without a full device re-boot.
    DeviceLost,
    /// A device or host allocation failed.
    OutOfMemory,
    /// A trait method whose backend support has not landed yet (a declared seam,
    /// per plan D7). The `&'static str` names the method for diagnostics.
    Unsupported(&'static str),
    /// The swapchain no longer matches the surface and must be recreated
    /// (`VK_ERROR_OUT_OF_DATE_KHR`). Phase-2-3 on-screen seam.
    SurfaceOutOfDate,
    /// The swapchain still presents but no longer optimally matches the surface
    /// (`VK_SUBOPTIMAL_KHR`). Phase-2-3 on-screen seam.
    SuboptimalSurface,
    /// An opaque backend-specific failure that does not map to a finer category.
    /// The `&'static str` carries a short backend tag for diagnostics.
    BackendError(&'static str),
}

impl RhiError {
    /// Builds the `Unsupported` category for a declared-but-unimplemented seam.
    ///
    /// Marked `#[cold]` because every caller is a Phase-1 seam-stub default body
    /// on the `Err` path — keeping it out of the hot recording code's I-cache
    /// footprint (plan O4/W3(3): all `RhiError` construction on the error path is
    /// cold so the `?`-desugar conversion never inlines into a hot loop).
    #[cold]
    #[inline(never)]
    pub fn unsupported(method: &'static str) -> Self {
        RhiError::Unsupported(method)
    }
}

impl core::fmt::Display for RhiError {
    // The match arms are all cold (error reporting), so do not inline this into
    // any caller — it is reached only on a failure that already left the fast path.
    #[cold]
    #[inline(never)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RhiError::DeviceLost => f.write_str("device lost"),
            RhiError::OutOfMemory => f.write_str("out of memory"),
            RhiError::Unsupported(m) => write!(f, "unsupported RHI operation: {m}"),
            RhiError::SurfaceOutOfDate => f.write_str("surface out of date"),
            RhiError::SuboptimalSurface => f.write_str("suboptimal surface"),
            RhiError::BackendError(tag) => write!(f, "backend error: {tag}"),
        }
    }
}

impl core::error::Error for RhiError {}
