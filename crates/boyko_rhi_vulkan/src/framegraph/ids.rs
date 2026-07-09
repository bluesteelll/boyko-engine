//! Stable `u16` newtype handles into the frame graph's SoA arenas.
//!
//! The graph is tiny (tens of resources, tens of passes), so `u16` indices are
//! ample and keep every arena row and barrier POD compact (I-cache + D-cache).
//! They are *arena-local* — valid only for the frame-graph instance that minted
//! them, invalidated by [`FrameGraph::reset`](super::graph::FrameGraph::reset).

/// A logical frame-graph resource (a transient/history image or a buffer),
/// indexing the resource SoA arena. NOT a raw `VkImage`/`VkBuffer` — the physical
/// handle is resolved at record time (Step 1c) so one derived plan is valid for
/// any per-frame physical slot (Step 1d FIF-slotting / history rotation).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ResId(pub u16);

/// A frame-graph pass (one GPU work item: a raster bracket or a compute
/// dispatch), indexing the pass SoA arena in declaration order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PassId(pub u16);

impl ResId {
    /// The arena row index as `usize`.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl PassId {
    /// The arena row index as `usize`.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
