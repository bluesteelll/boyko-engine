//! Render rung GB — the chained per-pass scope: debug-utils labels, breadcrumb markers and NVIDIA
//! checkpoints, recorded from ONE scope per graph pass and per hand-recorded host step.
//!
//! # The chained scope
//!
//! Every graph pass of every declarator crosses exactly one of the three barrier funnels
//! (`record_graph_pass`, `record_forward_pass`, `record_vb_pass`), and each funnel calls
//! [`Renderer::crumb_enter_pass`] before it records the pass's barriers. Entering a scope CLOSES
//! the scope that is open and opens the new one, so a scope runs from its own funnel call to the
//! start of the next scope — barriers and body together — and no pass body needed an edit. The
//! three hand-recorded steps that are not graph passes (FXAA, SMAA, the swapchain blit) enter a
//! [`HostScope`], and [`Renderer::crumb_frame_close`] closes the last scope and the frame scope
//! immediately before `vkEndCommandBuffer`. Every site sits outside any dynamic-rendering instance
//! (pipeline barriers are not legal inside one, and the funnels record barriers), which the
//! frame-close host-read barrier requires.
//!
//! A consequence worth stating in every report: a scope absorbs any hand-recorded work between
//! its funnel call and the next scope (the swapchain transitions after the last pass, for one).
//!
//! # Breadcrumbs
//!
//! Per frame-in-flight slot the marker buffer holds one [`CrumbRegion`]: a frame begin/end pair
//! and a `[begin, end]` pair per RECORD POSITION (not per pass id — completion is a record-order
//! notion). A scope writes its begin at `TOP_OF_PIPE` and its end at `BOTTOM_OF_PIPE`, both with
//! the frame's serial `S` (a per-armed-frame counter that never takes `0`), so no per-frame reset
//! is needed: a stale value from an earlier frame is never `S`. For a scope `p`:
//! completed ⇔ `begin == end == S`; in flight ⇔ `begin == S ≠ end`; not started ⇔ `begin ≠ S`.
//! `begin(k+1)` may land before `end(k)` (TOP is not in BOTTOM's first scope), so a device-loss
//! report prints the in-flight SET, never one guessed culprit. Positions past [`CRUMB_SCOPE_CAP`]
//! get labels and checkpoints but no markers, and are counted in `overflow`.
//!
//! # Zero cost when disarmed
//!
//! Every site is `if self.crumbs_armed() { self.<cold call>(..) }`, and `crumbs_armed` is
//! `GpuCrashDiagLevel::COMPILED_IN && self.crumb_bits != 0`: a `const false` in a `shipping`
//! build (the site and its cold body are deleted), one byte load and one predicted branch in a
//! disarmed `dev` build, and no command recorded either way. `tests/gpu_crumbs_census.rs` (G1)
//! holds every site to that shape.
//!
//! # The GPU timestamp zones are separate
//!
//! The zones (`gpu_zone.rs`) bracket pass BODIES, deliberately excluding the graph's barriers, and
//! their stamp positions are pinned by the profiling witness; folding them into this scope would
//! change what every pinned number measures. They stay apart at this rung.

use core::cell::Cell;
use core::ptr::{self, NonNull};

use crate::device::{DeviceFns, VulkanContext};
use crate::ffi::*;
use crate::framegraph::{FrameGraph, PassId, ResId};
use crate::gpu_crash::{
    CheckpointFns, CrumbLedger, DebugUtilsCmdFns, GpuCrashDiagLevel, MarkerFns, piece,
};

use super::FRAMES_IN_FLIGHT;
use super::frame_driver::Renderer;

/// Record positions per frame that carry breadcrumbs. `FRAME_GRAPH_CAPACITY.max_pass` (96) is a
/// first-frame allocation hint, not a bound, so positions at or past this cap still get labels and
/// checkpoints, are counted in `overflow`, and are never used as an index.
pub(crate) const CRUMB_SCOPE_CAP: usize = 128;

/// Scope ids at or above this value are [`HostScope`]s; below it they are graph pass ids.
pub(crate) const HOST_SCOPE_BASE: u16 = 0xFFF0;

/// The longest label or object name GB hands the driver, including its NUL.
pub(crate) const LABEL_BYTES: usize = 64;

/// A hand-recorded step that is not a graph pass but still gets its own scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub(crate) enum HostScope {
    /// FXAA (`record_fxaa`), Deferred and VisibilityBuffer.
    Fxaa = 0,
    /// SMAA (`record_smaa`), Deferred and VisibilityBuffer.
    Smaa = 1,
    /// The hand-recorded swapchain blit and present transition, every declarator.
    PresentBlit = 2,
}

impl HostScope {
    /// This scope's id in the position → id table.
    #[inline]
    pub(crate) const fn id(self) -> u16 {
        HOST_SCOPE_BASE + self as u16
    }

    /// The label this scope records (the same shape as a pass name).
    #[inline]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Fxaa => "fxaa",
            Self::Smaa => "smaa",
            Self::PresentBlit => "present_blit",
        }
    }
}

/// The declarators by `ResolvedRenderPathGpu::path` discriminant.
pub(crate) const DECLARATOR_NAMES: [&str; 4] = ["deferred", "forward", "forward_plus", "vb"];

/// The frame scope's label per declarator.
const FRAME_LABELS: [&str; 4] = ["frame_deferred", "frame_forward", "frame_forward_plus", "frame_vb"];

/// One frame-in-flight slot's breadcrumbs in the marker buffer. Never formed as a Rust value over
/// mapped memory (the GPU may be writing it): only its offsets are used, and the host reads the
/// words one `read_volatile` at a time.
#[repr(C)]
pub(crate) struct CrumbRegion {
    /// The serial at `TOP_OF_PIPE`, written when the first scope opens the frame.
    pub(crate) frame_begin: u32,
    /// The serial at `BOTTOM_OF_PIPE`, written at frame close.
    pub(crate) frame_end: u32,
    _pad: [u32; 2],
    /// `[begin @ TOP_OF_PIPE, end @ BOTTOM_OF_PIPE]` per record position.
    pub(crate) scopes: [[u32; 2]; CRUMB_SCOPE_CAP],
}

/// [`CrumbRegion`]'s size in bytes.
pub(crate) const REGION_BYTES: usize = core::mem::size_of::<CrumbRegion>();
/// [`CrumbRegion`]'s size in `u32` words.
pub(crate) const REGION_WORDS: usize = REGION_BYTES / 4;
/// The marker buffer's size: room for every slot's region.
pub(crate) const CRUMB_BUFFER_BYTES: u64 = 4096;

const OFF_FRAME_BEGIN: u32 = core::mem::offset_of!(CrumbRegion, frame_begin) as u32;
const OFF_FRAME_END: u32 = core::mem::offset_of!(CrumbRegion, frame_end) as u32;
const OFF_SCOPES: u32 = core::mem::offset_of!(CrumbRegion, scopes) as u32;
const W_FRAME_BEGIN: usize = OFF_FRAME_BEGIN as usize / 4;
const W_FRAME_END: usize = OFF_FRAME_END as usize / 4;
const W_SCOPES: usize = OFF_SCOPES as usize / 4;

const _: () = assert!(REGION_BYTES == 16 + 8 * CRUMB_SCOPE_CAP);
const _: () = assert!(REGION_BYTES.is_multiple_of(4), "every marker offset is a multiple of 4 (VUID-vkCmdWriteBufferMarkerAMD-dstOffset-01801)");
const _: () = assert!(OFF_FRAME_BEGIN == 0 && OFF_FRAME_END == 4 && OFF_SCOPES == 16);
const _: () = assert!(REGION_BYTES * FRAMES_IN_FLIGHT <= CRUMB_BUFFER_BYTES as usize);
// The checkpoint payload is a 64-bit integer carried in a pointer-sized marker.
const _: () = assert!(usize::BITS == 64, "GB checkpoints carry a 64-bit payload in a pointer");

/// The byte offset of scope `pos`'s begin word inside a region.
#[inline]
pub(crate) const fn scope_begin_offset(pos: usize) -> u32 {
    OFF_SCOPES + (pos as u32) * 8
}

/// The byte offset of scope `pos`'s end word inside a region.
#[inline]
pub(crate) const fn scope_end_offset(pos: usize) -> u32 {
    OFF_SCOPES + (pos as u32) * 8 + 4
}

/// What one checkpoint marks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum CheckpointKind {
    /// A scope opened.
    Begin = 0,
    /// A scope closed.
    End = 1,
    /// The frame opened (its first scope).
    FrameBegin = 2,
    /// The frame closed.
    FrameEnd = 3,
}

/// Packs a checkpoint payload: `serial << 32 | kind << 16 | position`. Carried as a pointer-sized
/// integer the driver reports back and nobody dereferences.
#[inline]
pub(crate) const fn encode_checkpoint(serial: u32, kind: CheckpointKind, position: u16) -> u64 {
    ((serial as u64) << 32) | ((kind as u64) << 16) | position as u64
}

/// A scope's state in a region read back for serial `S`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScopeState {
    /// `begin == end == S`.
    Completed,
    /// `begin == S`, `end != S`.
    InFlight,
    /// `begin != S`.
    NotStarted,
}

/// Classifies record position `pos` of `region` (the slot's words) for `serial`. Pure.
#[inline]
pub(crate) fn scope_state(region: &[u32], serial: u32, pos: usize) -> ScopeState {
    let begin = region[W_SCOPES + 2 * pos];
    let end = region[W_SCOPES + 2 * pos + 1];
    if begin != serial {
        ScopeState::NotStarted
    } else if end == serial {
        ScopeState::Completed
    } else {
        ScopeState::InFlight
    }
}

/// `true` iff the frame of `serial` completed: its frame begin and end and every one of its first
/// `recorded` positions carry `serial`. Pure.
pub(crate) fn frame_complete(region: &[u32], serial: u32, recorded: usize) -> bool {
    region[W_FRAME_BEGIN] == serial
        && region[W_FRAME_END] == serial
        && (0..recorded.min(CRUMB_SCOPE_CAP)).all(|p| scope_state(region, serial, p) == ScopeState::Completed)
}

/// The next serial after `s`, skipping `0` on wrap (`0` is the buffer's never-written value).
#[inline]
pub(crate) const fn next_serial(s: u32) -> u32 {
    let n = s.wrapping_add(1);
    if n == 0 { 1 } else { n }
}

/// Copies `name` into a NUL-terminated label. Names are graph pass and resource names
/// (`^[a-z][a-z0-9_]{0,62}$`, G1), so a longer one is a declaration defect: refused in a debug
/// build, truncated in release.
pub(crate) fn label_cstr(name: &str) -> [u8; LABEL_BYTES] {
    debug_assert!(
        name.len() < LABEL_BYTES,
        "invariant: a GB label is at most {} bytes, got {} ({name})",
        LABEL_BYTES - 1,
        name.len()
    );
    let mut buf = [0u8; LABEL_BYTES];
    let n = name.len().min(LABEL_BYTES - 1);
    buf[..n].copy_from_slice(&name.as_bytes()[..n]);
    buf
}

/// The operations the chained scope emits, in order. The real sink records Vulkan commands; the
/// unit tests' sink logs them. Generic at the call site, so no virtual dispatch.
pub(crate) trait CrumbSink {
    /// Open a label region named `name`.
    fn label_begin(&mut self, name: &'static str);
    /// Close the innermost label region.
    fn label_end(&mut self);
    /// Write `value` to byte `offset` of this slot's region once execution reaches `stage`.
    fn marker(&mut self, stage: VkFlags, offset: u32, value: u32);
    /// Record a checkpoint carrying `payload`.
    fn checkpoint(&mut self, payload: u64);
    /// Make this slot's marker writes visible to a host read after the frame's fence wait.
    fn host_read_barrier(&mut self);
}

/// One frame-in-flight slot's host shadow: the record-position table and the counters of the
/// frame last recorded into the slot. Renderer-internal scratch of the `FrameGraph`-arena class:
/// allocated once at boot (armed runs only), read and written through `Cell` on the one recording
/// thread.
pub(crate) struct CrumbSlot {
    /// This frame's serial (`0` before the slot's first armed frame).
    pub(crate) serial: Cell<u32>,
    /// Positions entered this frame, overflow included.
    pub(crate) cursor: Cell<u16>,
    /// A scope is open (its end not yet recorded).
    pub(crate) open: Cell<bool>,
    /// The frame scope is open.
    pub(crate) frame_open: Cell<bool>,
    /// The frame was closed — the command buffer reached its end — and awaits verification.
    pub(crate) pending: Cell<bool>,
    /// Positions past [`CRUMB_SCOPE_CAP`] this frame.
    pub(crate) overflow: Cell<u16>,
    /// The declarator (`ResolvedRenderPathGpu::path`).
    pub(crate) declarator: Cell<u8>,
    /// Passes the frame graph declared this frame.
    pub(crate) pass_count: Cell<u16>,
    /// Graph-pass scopes entered this frame (host scopes excluded).
    pub(crate) passes_entered: Cell<u16>,
    /// Label regions opened / closed this frame.
    pub(crate) labels_begun: Cell<u16>,
    pub(crate) labels_ended: Cell<u16>,
    /// This frame's graph resources were named (labels piece).
    pub(crate) names_done: Cell<bool>,
    /// The serial last verified on this slot (`0`: none yet).
    pub(crate) last_verified: Cell<u32>,
    /// Position → pass id, or [`HOST_SCOPE_BASE`] + [`HostScope`].
    pub(crate) scope_ids: [Cell<u16>; CRUMB_SCOPE_CAP],
    /// Pass id → name, copied once per armed frame (the report runs after the graph was
    /// re-declared). Pass ids at or past the cap are not snapshot.
    pub(crate) names: [Cell<&'static str>; CRUMB_SCOPE_CAP],
    /// Resource index → the handle last named on this slot (labels piece).
    pub(crate) named: [Cell<u64>; CRUMB_SCOPE_CAP],
}

impl CrumbSlot {
    /// An unused slot.
    pub(crate) fn new() -> Self {
        Self {
            serial: Cell::new(0),
            cursor: Cell::new(0),
            open: Cell::new(false),
            frame_open: Cell::new(false),
            pending: Cell::new(false),
            overflow: Cell::new(0),
            declarator: Cell::new(0),
            pass_count: Cell::new(0),
            passes_entered: Cell::new(0),
            labels_begun: Cell::new(0),
            labels_ended: Cell::new(0),
            names_done: Cell::new(false),
            last_verified: Cell::new(0),
            scope_ids: core::array::from_fn(|_| Cell::new(0)),
            names: core::array::from_fn(|_| Cell::new("")),
            named: core::array::from_fn(|_| Cell::new(0)),
        }
    }

    /// Readies the slot for a new frame of `serial`, declared by `declarator` with `pass_count`
    /// passes. The name snapshot is the caller's.
    pub(crate) fn begin(&self, serial: u32, declarator: u8, pass_count: usize) {
        self.serial.set(serial);
        self.cursor.set(0);
        self.open.set(false);
        self.frame_open.set(false);
        self.pending.set(false);
        self.overflow.set(0);
        self.declarator.set(declarator);
        self.pass_count.set(u16::try_from(pass_count).unwrap_or(u16::MAX));
        self.passes_entered.set(0);
        self.labels_begun.set(0);
        self.labels_ended.set(0);
        self.names_done.set(false);
    }

    /// Positions that carry breadcrumbs this frame.
    #[inline]
    pub(crate) fn recorded(&self) -> usize {
        (self.cursor.get() as usize).min(CRUMB_SCOPE_CAP)
    }

    fn bump(c: &Cell<u16>) {
        c.set(c.get().saturating_add(1));
    }
}

/// The frame label for declarator `d`.
#[inline]
fn frame_label(d: u8) -> &'static str {
    FRAME_LABELS.get(d as usize).copied().unwrap_or("frame")
}

/// Opens the frame scope: its label, its begin marker and its checkpoint.
fn open_frame<S: CrumbSink>(slot: &CrumbSlot, bits: u8, sink: &mut S) {
    let serial = slot.serial.get();
    if bits & piece::LABELS != 0 {
        sink.label_begin(frame_label(slot.declarator.get()));
        CrumbSlot::bump(&slot.labels_begun);
    }
    if bits & piece::MARKERS != 0 {
        sink.marker(VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, OFF_FRAME_BEGIN, serial);
    }
    if bits & piece::CHECKPOINTS != 0 {
        sink.checkpoint(encode_checkpoint(serial, CheckpointKind::FrameBegin, 0));
    }
    slot.frame_open.set(true);
}

/// Closes the open scope (the one at position `cursor - 1`): its end marker, its checkpoint, its
/// label.
fn close_scope<S: CrumbSink>(slot: &CrumbSlot, bits: u8, sink: &mut S) {
    let serial = slot.serial.get();
    let pos = slot.cursor.get().saturating_sub(1);
    if bits & piece::MARKERS != 0 && (pos as usize) < CRUMB_SCOPE_CAP {
        sink.marker(VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, scope_end_offset(pos as usize), serial);
    }
    if bits & piece::CHECKPOINTS != 0 {
        sink.checkpoint(encode_checkpoint(serial, CheckpointKind::End, pos));
    }
    if bits & piece::LABELS != 0 {
        sink.label_end();
        CrumbSlot::bump(&slot.labels_ended);
    }
    slot.open.set(false);
}

/// Enters scope `id` named `name`: opens the frame if this is its first scope, closes the open
/// scope, and opens this one at the next record position.
pub(crate) fn enter_scope<S: CrumbSink>(slot: &CrumbSlot, bits: u8, sink: &mut S, id: u16, name: &'static str) {
    if !slot.frame_open.get() {
        open_frame(slot, bits, sink);
    }
    if slot.open.get() {
        close_scope(slot, bits, sink);
    }
    let serial = slot.serial.get();
    let pos = slot.cursor.get();
    if bits & piece::LABELS != 0 {
        sink.label_begin(name);
        CrumbSlot::bump(&slot.labels_begun);
    }
    if (pos as usize) < CRUMB_SCOPE_CAP {
        slot.scope_ids[pos as usize].set(id);
        if bits & piece::MARKERS != 0 {
            sink.marker(VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, scope_begin_offset(pos as usize), serial);
        }
    } else {
        CrumbSlot::bump(&slot.overflow);
    }
    if bits & piece::CHECKPOINTS != 0 {
        sink.checkpoint(encode_checkpoint(serial, CheckpointKind::Begin, pos));
    }
    if id < HOST_SCOPE_BASE {
        CrumbSlot::bump(&slot.passes_entered);
    }
    CrumbSlot::bump(&slot.cursor);
    slot.open.set(true);
}

/// Closes the frame: the open scope, then the frame scope, then (markers) the host-read barrier.
/// A frame that entered no scope records nothing.
pub(crate) fn close_frame<S: CrumbSink>(slot: &CrumbSlot, bits: u8, sink: &mut S) {
    if !slot.frame_open.get() {
        return;
    }
    if slot.open.get() {
        close_scope(slot, bits, sink);
    }
    let serial = slot.serial.get();
    if bits & piece::MARKERS != 0 {
        sink.marker(VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, OFF_FRAME_END, serial);
    }
    if bits & piece::CHECKPOINTS != 0 {
        sink.checkpoint(encode_checkpoint(serial, CheckpointKind::FrameEnd, slot.cursor.get()));
    }
    if bits & piece::LABELS != 0 {
        sink.label_end();
        CrumbSlot::bump(&slot.labels_ended);
    }
    if bits & piece::MARKERS != 0 {
        sink.host_read_barrier();
    }
    slot.frame_open.set(false);
    slot.pending.set(true);
}

/// The persistently mapped marker buffer: `HOST_VISIBLE | HOST_COHERENT`, preferring memory that
/// is not `DEVICE_LOCAL` (system memory survives a device reset better than a BAR mapping).
pub(crate) struct CrumbBuffer {
    pub(crate) buffer: VkBuffer,
    memory: VkDeviceMemory,
    /// The `vkMapMemory` pointer to byte 0. Every host read derives from it with an in-bounds
    /// offset; no reference is ever formed over the mapping, since the GPU may be writing it.
    mapped: NonNull<u8>,
}

impl CrumbBuffer {
    /// Creates, binds, maps and zeroes the marker buffer. `None` on any failure (the caller
    /// degrades the markers piece); every object created before the failure is destroyed.
    fn create(ctx: &VulkanContext, fns: &DeviceFns, device: VkDevice) -> Option<Self> {
        let ci = VkBufferCreateInfo {
            s_type: VkStructureType::BufferCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            size: CRUMB_BUFFER_BYTES,
            usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
        };
        let mut buffer = VkBuffer::NULL;
        // SAFETY: `device` is live; `ci` is a fully-initialised create-info naming no queue-family
        // array; `&mut buffer` is a valid out-pointer; NULL allocator selects the default.
        let raw = unsafe { (fns.create_buffer)(device, &ci, ptr::null(), &mut buffer) };
        if !VkResult::from_raw(raw).is_success() {
            return None;
        }
        let mut reqs = VkMemoryRequirements { size: 0, alignment: 0, memory_type_bits: 0 };
        // SAFETY: `buffer` was just created on `device`; `&mut reqs` is a valid out-pointer.
        unsafe { (fns.get_buffer_memory_requirements)(device, buffer, &mut reqs) };
        let Some(type_index) = crumb_memory_type(ctx.memory_properties(), reqs.memory_type_bits) else {
            // SAFETY: `buffer` was created above, is bound to nothing and used by nothing.
            unsafe { (fns.destroy_buffer)(device, buffer, ptr::null()) };
            return None;
        };
        let ai = VkMemoryAllocateInfo {
            s_type: VkStructureType::MemoryAllocateInfo,
            p_next: ptr::null(),
            allocation_size: reqs.size,
            memory_type_index: type_index,
        };
        let mut memory = VkDeviceMemory::NULL;
        // SAFETY: `device` is live; `ai` names a memory type the buffer accepts and the buffer's
        // own required size; `&mut memory` is a valid out-pointer.
        let raw = unsafe { (fns.allocate_memory)(device, &ai, ptr::null(), &mut memory) };
        if !VkResult::from_raw(raw).is_success() {
            // SAFETY: as above — the unbound buffer is destroyed once.
            unsafe { (fns.destroy_buffer)(device, buffer, ptr::null()) };
            return None;
        }
        let mut mapped_raw: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `memory` is a fresh allocation of a type that accepts the buffer, bound at
        // offset 0 (alignment 0 ≤ any requirement); it is HOST_VISIBLE, so mapping the whole
        // range is legal; `&mut mapped_raw` is a valid out-pointer.
        let ok = unsafe {
            VkResult::from_raw((fns.bind_buffer_memory)(device, buffer, memory, 0)).is_success()
                && VkResult::from_raw((fns.map_memory)(device, memory, 0, VK_WHOLE_SIZE, 0, &mut mapped_raw))
                    .is_success()
        };
        let mapped = NonNull::new(mapped_raw.cast::<u8>());
        let (true, Some(mapped)) = (ok, mapped) else {
            // SAFETY: nothing references `buffer` or `memory` yet; each is destroyed once, the
            // buffer before the memory it may be bound to.
            unsafe {
                (fns.destroy_buffer)(device, buffer, ptr::null());
                (fns.free_memory)(device, memory, ptr::null());
            }
            return None;
        };
        // SAFETY: `mapped` is the start of a live host mapping at least `reqs.size >=
        // CRUMB_BUFFER_BYTES` bytes long, and no submitted work references the buffer yet, so
        // zeroing it races nothing.
        unsafe { ptr::write_bytes(mapped.as_ptr(), 0, CRUMB_BUFFER_BYTES as usize) };
        Some(Self { buffer, memory, mapped })
    }

    /// Reads slot `fi`'s region, one volatile `u32` at a time, into `out`.
    ///
    /// # Safety
    ///
    /// The mapping is live (the buffer was not destroyed). The caller either waited the fence of
    /// the submission that last wrote the slot (whose frame close made the writes host-visible)
    /// or the device is lost, where the mapping stays valid with undefined contents.
    unsafe fn read_region(&self, fi: usize, out: &mut [u32; REGION_WORDS]) {
        debug_assert!(fi < FRAMES_IN_FLIGHT);
        let base = self.mapped.as_ptr().wrapping_add(fi * REGION_BYTES).cast::<u32>();
        for (w, slot) in out.iter_mut().enumerate() {
            // SAFETY: `fi * REGION_BYTES + 4 * w + 4 <= FRAMES_IN_FLIGHT * REGION_BYTES <=
            // CRUMB_BUFFER_BYTES` (the const assert above), so the word is inside the mapping; the
            // mapping starts at a `vkMapMemory` pointer aligned to `minMemoryMapAlignment` (>= 64)
            // and every offset is a multiple of 4; no reference to the memory exists, and the
            // read is volatile because the device writes it.
            *slot = unsafe { ptr::read_volatile(base.add(w)) };
        }
    }

    /// Destroys the buffer and frees its memory.
    ///
    /// # Safety
    ///
    /// The device is idle (no submission references the buffer), and this runs once.
    unsafe fn destroy(&self, fns: &DeviceFns, device: VkDevice) {
        // SAFETY: per the contract nothing in flight references the buffer; unmapping before the
        // free and destroying the buffer before its memory is the spec's teardown order.
        unsafe {
            (fns.unmap_memory)(device, self.memory);
            (fns.destroy_buffer)(device, self.buffer, ptr::null());
            (fns.free_memory)(device, self.memory, ptr::null());
        }
    }
}

/// The marker buffer's memory type: `HOST_VISIBLE | HOST_COHERENT`, preferring one that is not
/// `DEVICE_LOCAL`, among the types `type_bits` admits. Pure.
pub(crate) fn crumb_memory_type(props: &VkPhysicalDeviceMemoryProperties, type_bits: u32) -> Option<u32> {
    let required = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    let count = (props.memory_type_count as usize).min(VK_MAX_MEMORY_TYPES);
    let admits = |i: usize| type_bits & (1u32 << i) != 0 && props.memory_types[i].property_flags & required == required;
    (0..count)
        .find(|&i| admits(i) && props.memory_types[i].property_flags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT == 0)
        .or_else(|| (0..count).find(|&i| admits(i)))
        .map(|i| i as u32)
}

/// The renderer's GB state: the armed pieces, their entry points, the marker buffer, the serial
/// counter, the ledger and the two slot shadows. `Some` on the [`Renderer`] only in an armed run.
pub(crate) struct CrumbState {
    /// The armed per-frame pieces ([`piece::PER_FRAME`] bits).
    pub(crate) bits: u8,
    device: VkDevice,
    debug_utils: Option<DebugUtilsCmdFns>,
    markers: Option<MarkerFns>,
    checkpoints: Option<CheckpointFns>,
    buffer: Option<CrumbBuffer>,
    last_serial: Cell<u32>,
    pub(crate) ledger: Cell<CrumbLedger>,
    pub(crate) slots: [CrumbSlot; FRAMES_IN_FLIGHT],
}

impl CrumbState {
    /// Builds the renderer's GB state from what the context armed: `None` when no per-frame piece
    /// is armed (every unarmed run, every `shipping` build). A marker buffer that cannot be built
    /// drops the markers piece with `boyko-W2107`. Names the per-slot command buffers and the
    /// marker buffer when labels are armed. Cold: once per renderer.
    #[cold]
    #[inline(never)]
    pub(crate) fn new(
        ctx: &VulkanContext,
        fns: &DeviceFns,
        device: VkDevice,
        command_buffers: &[VkCommandBuffer; FRAMES_IN_FLIGHT],
    ) -> Option<Box<Self>> {
        let crash = ctx.gpu_crash();
        let mut bits = crash.pieces & piece::PER_FRAME;
        if bits == 0 {
            return None;
        }
        let buffer = if bits & piece::MARKERS != 0 {
            let b = CrumbBuffer::create(ctx, fns, device);
            if b.is_none() {
                bits &= !piece::MARKERS;
                crate::device::report_gpu_crash_piece_unavailable(
                    &crate::device::W2107_MARKERS_SITE,
                    "markers",
                    "the breadcrumb buffer could not be created",
                );
            }
            b
        } else {
            None
        };
        if bits == 0 {
            return None;
        }
        let state = Box::new(Self {
            bits,
            device,
            debug_utils: crash.debug_utils.filter(|_| bits & piece::LABELS != 0),
            markers: crash.markers.filter(|_| bits & piece::MARKERS != 0),
            checkpoints: crash.checkpoints.filter(|_| bits & piece::CHECKPOINTS != 0),
            buffer,
            last_serial: Cell::new(0),
            ledger: Cell::new(CrumbLedger::default()),
            slots: [CrumbSlot::new(), CrumbSlot::new()],
        });
        if let Some(b) = state.buffer.as_ref() {
            state.set_name(VK_OBJECT_TYPE_BUFFER, b.buffer.0, "gpu_crumbs");
        }
        const CMD_NAMES: [&str; FRAMES_IN_FLIGHT] = ["frame_cmd_0", "frame_cmd_1"];
        for (cmd, name) in command_buffers.iter().zip(CMD_NAMES) {
            state.set_name(VK_OBJECT_TYPE_COMMAND_BUFFER, cmd.0.addr() as u64, name);
        }
        Some(state)
    }

    /// Names one object (labels piece). A NULL handle is skipped
    /// (`VUID-VkDebugUtilsObjectNameInfoEXT-objectType-02590`).
    fn set_name(&self, object_type: i32, handle: u64, name: &str) {
        let Some(d) = self.debug_utils else {
            return;
        };
        if handle == 0 {
            return;
        }
        let buf = label_cstr(name);
        let info = VkDebugUtilsObjectNameInfoExt {
            s_type: VkStructureType::DebugUtilsObjectNameInfoExt,
            p_next: ptr::null(),
            object_type,
            object_handle: handle,
            p_object_name: buf.as_ptr().cast(),
        };
        // SAFETY: `device` is the live device the object was created on (the renderer's, which
        // owns every handle named here or borrows it from a target that outlives the frame);
        // `info` is fully initialised, its name a NUL-terminated buffer on this frame; the
        // labels piece enabled `VK_EXT_debug_utils`, so the entry point is valid. The result is
        // ignored: a name is advisory, and the call has no failure the frame must act on.
        let _ = unsafe { (d.set_object_name)(self.device, &info) };
    }

    /// Names this slot's graph resources from `graph.res_name`, re-naming only the handles that
    /// changed since the slot's last armed frame (a resize, a first frame). An object two
    /// resources resolve to keeps the first name. Resource indices at or past
    /// [`CRUMB_SCOPE_CAP`] are not named.
    fn name_graph_resources(
        &self,
        slot: &CrumbSlot,
        graph: &FrameGraph,
        images: &[VkImage],
        buffers: &[VkBuffer],
        image_count: usize,
    ) {
        let n = graph.res_count().min(CRUMB_SCOPE_CAP);
        for r in 0..n {
            let res = ResId(r as u16);
            let (object_type, handle) = if graph.res_is_image(res) {
                match images.get(r) {
                    Some(h) => (VK_OBJECT_TYPE_IMAGE, h.0),
                    None => continue,
                }
            } else {
                match r.checked_sub(image_count).and_then(|i| buffers.get(i)) {
                    Some(h) => (VK_OBJECT_TYPE_BUFFER, h.0),
                    None => continue,
                }
            };
            if handle == 0 || slot.named[r].get() == handle {
                continue;
            }
            slot.named[r].set(handle);
            if (0..r).any(|q| slot.named[q].get() == handle) {
                continue;
            }
            self.set_name(object_type, handle, graph.res_name(res));
        }
        slot.names_done.set(true);
    }

    /// The sink that records into `cmd` for slot `fi`.
    fn sink<'a>(&'a self, fns: &'a DeviceFns, cmd: VkCommandBuffer, fi: usize) -> VkCrumbSink<'a> {
        VkCrumbSink { st: self, fns, cmd, region_base: (fi * REGION_BYTES) as u32 }
    }

    /// Destroys the marker buffer.
    ///
    /// # Safety
    ///
    /// The device is idle, and this runs once (the renderer's `Drop`).
    pub(crate) unsafe fn destroy(&self, fns: &DeviceFns) {
        if let Some(b) = self.buffer.as_ref() {
            // SAFETY: forwarded from this fn's contract.
            unsafe { b.destroy(fns, self.device) };
        }
    }
}

/// The real [`CrumbSink`]: records into the open command buffer through the armed entry points.
struct VkCrumbSink<'a> {
    st: &'a CrumbState,
    fns: &'a DeviceFns,
    cmd: VkCommandBuffer,
    /// This slot's region offset inside the marker buffer.
    region_base: u32,
}

impl CrumbSink for VkCrumbSink<'_> {
    fn label_begin(&mut self, name: &'static str) {
        let Some(d) = self.st.debug_utils else {
            return;
        };
        let buf = label_cstr(name);
        let info = VkDebugUtilsLabelExt {
            s_type: VkStructureType::DebugUtilsLabelExt,
            p_next: ptr::null(),
            p_label_name: buf.as_ptr().cast(),
            color: [0.0; 4],
        };
        // SAFETY: `cmd` is recording (every GB site runs inside the recorder's begin/end bracket)
        // and outside any rendering instance; `info` and its NUL-terminated name live on this
        // frame for the call; the labels piece enabled `VK_EXT_debug_utils`.
        unsafe { (d.begin_label)(self.cmd, &info) };
    }

    fn label_end(&mut self) {
        let Some(d) = self.st.debug_utils else {
            return;
        };
        // SAFETY: `cmd` is recording; the chained scope ends exactly the regions it began in this
        // command buffer (`labels_begun == labels_ended` at frame close), so a region is open
        // (`VUID-vkCmdEndDebugUtilsLabelEXT-commandBuffer-01912`).
        unsafe { (d.end_label)(self.cmd) };
    }

    fn marker(&mut self, stage: VkFlags, offset: u32, value: u32) {
        let (Some(m), Some(b)) = (self.st.markers, self.st.buffer.as_ref()) else {
            return;
        };
        // SAFETY: `cmd` is recording; `b.buffer` is the live `TRANSFER_DST` marker buffer;
        // `region_base + offset + 4 <= FRAMES_IN_FLIGHT * REGION_BYTES <= CRUMB_BUFFER_BYTES`
        // and is a multiple of 4 (the const asserts); `stage` is one stage bit; the markers piece
        // enabled `VK_AMD_buffer_marker`.
        unsafe { (m.write_buffer_marker)(self.cmd, stage, b.buffer, u64::from(self.region_base + offset), value) };
    }

    fn checkpoint(&mut self, payload: u64) {
        let Some(c) = self.st.checkpoints else {
            return;
        };
        // SAFETY: `cmd` is recording; the marker is an opaque integer the driver stores and
        // reports back, never dereferences — built without provenance for that reason; the
        // checkpoints piece enabled `VK_NV_device_diagnostic_checkpoints`.
        unsafe { (c.set_checkpoint)(self.cmd, ptr::without_provenance(payload as usize)) };
    }

    fn host_read_barrier(&mut self) {
        let Some(b) = self.st.buffer.as_ref() else {
            return;
        };
        let barrier = VkBufferMemoryBarrier {
            s_type: VkStructureType::BufferMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
            dst_access_mask: VK_ACCESS_HOST_READ_BIT,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            buffer: b.buffer,
            offset: u64::from(self.region_base),
            size: REGION_BYTES as u64,
        };
        // SAFETY: `cmd` is recording outside any rendering instance (the frame close follows the
        // last `vkCmdEndRendering`); one buffer barrier over this slot's live region, from every
        // stage's transfer write (a marker write is a transfer write at TOP or BOTTOM) to the
        // host's read after the fence wait; `&barrier` lives for the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
                VK_PIPELINE_STAGE_HOST_BIT,
                0,
                0,
                ptr::null(),
                1,
                &barrier,
                0,
                ptr::null(),
            );
        }
    }
}

impl Renderer<'_> {
    /// `true` iff this run records GB scopes. The ONE branch every disarmed site pays; `false` at
    /// compile time in a `shipping` build.
    #[inline]
    pub(crate) fn crumbs_armed(&self) -> bool {
        GpuCrashDiagLevel::COMPILED_IN && self.crumb_bits != 0
    }

    /// Readies the current slot for this frame: a new serial, the declarator, the pass count and
    /// the pass-name snapshot. Called right after the frame graph is declared, before recording.
    #[cold]
    #[inline(never)]
    pub(crate) fn crumb_begin_frame(&self, declarator: u32) {
        let Some(st) = self.crumbs.as_deref() else {
            return;
        };
        let slot = &st.slots[self.frame_index];
        let serial = next_serial(st.last_serial.get());
        st.last_serial.set(serial);
        let pass_count = self.frame_graph.pass_count();
        slot.begin(serial, u8::try_from(declarator).unwrap_or(u8::MAX), pass_count);
        for p in 0..pass_count.min(CRUMB_SCOPE_CAP) {
            slot.names[p].set(self.frame_graph.pass_name(PassId(p as u16)));
        }
    }

    /// Enters graph pass `pass`'s scope (the three funnels). On the frame's first funnel call it
    /// also names the frame's graph resources from the funnel's sink arrays (labels piece).
    #[cold]
    #[inline(never)]
    pub(crate) fn crumb_enter_pass(
        &self,
        cmd: VkCommandBuffer,
        pass: PassId,
        images: &[VkImage],
        buffers: &[VkBuffer],
        image_count: usize,
    ) {
        let Some(st) = self.crumbs.as_deref() else {
            return;
        };
        debug_assert!(pass.0 < HOST_SCOPE_BASE, "invariant: a graph pass id is below the host-scope ids");
        let slot = &st.slots[self.frame_index];
        if st.bits & piece::LABELS != 0 && !slot.names_done.get() {
            st.name_graph_resources(slot, &self.frame_graph, images, buffers, image_count);
        }
        let mut sink = st.sink(self.fns, cmd, self.frame_index);
        enter_scope(slot, st.bits, &mut sink, pass.0, self.frame_graph.pass_name(pass));
    }

    /// Enters a hand-recorded host step's scope (FXAA, SMAA, the swapchain blit).
    #[cold]
    #[inline(never)]
    pub(crate) fn crumb_enter_host(&self, cmd: VkCommandBuffer, scope: HostScope) {
        let Some(st) = self.crumbs.as_deref() else {
            return;
        };
        let slot = &st.slots[self.frame_index];
        let mut sink = st.sink(self.fns, cmd, self.frame_index);
        enter_scope(slot, st.bits, &mut sink, scope.id(), scope.name());
    }

    /// Closes the open scope and the frame scope, and (markers) records the host-read barrier.
    /// Called immediately before `vkEndCommandBuffer`.
    #[cold]
    #[inline(never)]
    pub(crate) fn crumb_frame_close(&self, cmd: VkCommandBuffer) {
        let Some(st) = self.crumbs.as_deref() else {
            return;
        };
        let slot = &st.slots[self.frame_index];
        let mut sink = st.sink(self.fns, cmd, self.frame_index);
        close_frame(slot, st.bits, &mut sink);
    }

    /// Verifies the frame last recorded into slot `fi`, whose fence the caller just waited, and
    /// folds it into the ledger: scope parity (scopes entered == passes declared), and with
    /// markers, completion (every scope's begin and end and the frame end carry its serial).
    /// A debug build asserts both, so every armed debug frame is also a G2 check.
    #[cold]
    #[inline(never)]
    pub(crate) fn crumb_verify_slot(&self, fi: usize) {
        let Some(st) = self.crumbs.as_deref() else {
            return;
        };
        let slot = &st.slots[fi];
        if !slot.pending.get() {
            return;
        }
        slot.pending.set(false);
        let serial = slot.serial.get();
        let mut l = st.ledger.get();
        l.frames_checked += 1;
        l.scopes_checked += u64::from(slot.cursor.get());
        l.overflow += u64::from(slot.overflow.get());
        l.labels_begun += u64::from(slot.labels_begun.get());
        l.labels_ended += u64::from(slot.labels_ended.get());
        let parity = slot.passes_entered.get() == slot.pass_count.get();
        if !parity {
            l.parity_mismatch += 1;
        }
        let complete = match st.buffer.as_ref() {
            Some(b) => {
                let mut region = [0u32; REGION_WORDS];
                // SAFETY: the buffer is live (destroyed only in the renderer's `Drop`), and the
                // caller waited the fence of the submission that recorded this slot's frame,
                // whose frame close made its marker writes host-visible.
                unsafe { b.read_region(fi, &mut region) };
                frame_complete(&region, serial, slot.recorded())
            }
            None => true,
        };
        if !complete {
            l.frames_incomplete += 1;
        }
        let last = slot.last_verified.get();
        if last != 0 && (serial.wrapping_sub(last) as i32) <= 0 {
            l.serial_regressions += 1;
        }
        slot.last_verified.set(serial);
        if l.last_serial == 0 || (serial.wrapping_sub(l.last_serial) as i32) > 0 {
            l.last_serial = serial;
        }
        st.ledger.set(l);
        debug_assert!(
            parity,
            "invariant (GB G2): {} graph-pass scopes entered for {} declared passes on serial {serial} \
             ({} declarator) -- a declared pass never crossed a funnel, or one crossed twice",
            slot.passes_entered.get(),
            slot.pass_count.get(),
            DECLARATOR_NAMES.get(slot.declarator.get() as usize).copied().unwrap_or("?"),
        );
        debug_assert!(
            complete,
            "invariant (GB G2): serial {serial}'s breadcrumbs are incomplete after its fence wait"
        );
    }

    /// Render rung GB: the breadcrumb ledger — what the per-frame verification counted since boot.
    /// All zero in an unarmed run.
    #[inline]
    pub fn gpu_crumb_ledger(&self) -> CrumbLedger {
        match self.crumbs.as_deref() {
            Some(st) => st.ledger.get(),
            None => CrumbLedger::default(),
        }
    }

    /// Render rung GB: the armed per-frame pieces (`piece::LABELS | MARKERS | CHECKPOINTS` bits);
    /// `0` in an unarmed run.
    #[inline]
    pub fn gpu_crumb_pieces(&self) -> u8 {
        self.crumb_bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One logged sink operation.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Op {
        LabelBegin(&'static str),
        LabelEnd,
        Marker { stage: VkFlags, offset: u32, value: u32 },
        Checkpoint(u64),
        HostRead,
    }

    #[derive(Default)]
    struct LogSink(Vec<Op>);

    impl CrumbSink for LogSink {
        fn label_begin(&mut self, name: &'static str) {
            self.0.push(Op::LabelBegin(name));
        }
        fn label_end(&mut self) {
            self.0.push(Op::LabelEnd);
        }
        fn marker(&mut self, stage: VkFlags, offset: u32, value: u32) {
            self.0.push(Op::Marker { stage, offset, value });
        }
        fn checkpoint(&mut self, payload: u64) {
            self.0.push(Op::Checkpoint(payload));
        }
        fn host_read_barrier(&mut self) {
            self.0.push(Op::HostRead);
        }
    }

    const ALL: u8 = piece::PER_FRAME;
    const TOP: VkFlags = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
    const BOTTOM: VkFlags = VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT;

    /// Records one frame of `passes` graph passes then `hosts` host scopes with `bits`.
    fn frame(bits: u8, passes: usize, hosts: &[HostScope], serial: u32) -> (CrumbSlot, Vec<Op>) {
        let slot = CrumbSlot::new();
        slot.begin(serial, 0, passes);
        let mut sink = LogSink::default();
        for p in 0..passes {
            enter_scope(&slot, bits, &mut sink, p as u16, "pass");
        }
        for h in hosts {
            enter_scope(&slot, bits, &mut sink, h.id(), h.name());
        }
        close_frame(&slot, bits, &mut sink);
        (slot, sink.0)
    }

    /// The chained scope's whole op stream for a frame of N passes + 3 host scopes, every piece
    /// armed: frame begin, then `Begin(k)` closing `End(k-1)`, then the frame end — and every
    /// label region it opens, it closes.
    ///
    /// RED: drop the "close the open scope" step from `enter_scope` and `labels_ended` is 2
    /// against `labels_begun` N + 4.
    #[test]
    fn a_frame_is_one_chained_scope_per_pass_and_host_step() {
        let hosts = [HostScope::Fxaa, HostScope::Smaa, HostScope::PresentBlit];
        let n = 5;
        let s = 7;
        let (slot, ops) = frame(ALL, n, &hosts, s);
        let total = n + hosts.len();
        assert_eq!(slot.labels_begun.get() as usize, total + 1, "a label per scope + the frame's");
        assert_eq!(slot.labels_begun.get(), slot.labels_ended.get(), "every label region is closed");
        assert_eq!(slot.passes_entered.get() as usize, n, "host scopes are not graph passes");
        assert!(slot.pending.get() && !slot.open.get() && !slot.frame_open.get());

        let mut want = vec![
            Op::LabelBegin("frame_deferred"),
            Op::Marker { stage: TOP, offset: 0, value: s },
            Op::Checkpoint(encode_checkpoint(s, CheckpointKind::FrameBegin, 0)),
        ];
        for pos in 0..total {
            if pos > 0 {
                let prev = pos - 1;
                want.push(Op::Marker { stage: BOTTOM, offset: scope_end_offset(prev), value: s });
                want.push(Op::Checkpoint(encode_checkpoint(s, CheckpointKind::End, prev as u16)));
                want.push(Op::LabelEnd);
            }
            let name = if pos < n { "pass" } else { hosts[pos - n].name() };
            want.push(Op::LabelBegin(name));
            want.push(Op::Marker { stage: TOP, offset: scope_begin_offset(pos), value: s });
            want.push(Op::Checkpoint(encode_checkpoint(s, CheckpointKind::Begin, pos as u16)));
        }
        let last = total - 1;
        want.push(Op::Marker { stage: BOTTOM, offset: scope_end_offset(last), value: s });
        want.push(Op::Checkpoint(encode_checkpoint(s, CheckpointKind::End, last as u16)));
        want.push(Op::LabelEnd);
        want.push(Op::Marker { stage: BOTTOM, offset: 4, value: s });
        want.push(Op::Checkpoint(encode_checkpoint(s, CheckpointKind::FrameEnd, total as u16)));
        want.push(Op::LabelEnd);
        want.push(Op::HostRead);
        assert_eq!(ops, want);
        for (pos, h) in hosts.iter().enumerate() {
            assert_eq!(slot.scope_ids[n + pos].get(), h.id());
        }
    }

    /// Each piece records only its own ops; labels-only records no marker and no host barrier.
    #[test]
    fn each_piece_records_only_its_own_ops() {
        let (_, labels) = frame(piece::LABELS, 3, &[HostScope::PresentBlit], 1);
        assert!(labels.iter().all(|o| matches!(o, Op::LabelBegin(_) | Op::LabelEnd)), "{labels:?}");
        assert_eq!(labels.len(), 2 * (3 + 1 + 1));
        let (_, markers) = frame(piece::MARKERS, 3, &[], 1);
        assert!(markers.iter().all(|o| matches!(o, Op::Marker { .. } | Op::HostRead)), "{markers:?}");
        assert_eq!(markers.len(), 2 + 2 * 3 + 1);
        let (_, cps) = frame(piece::CHECKPOINTS, 3, &[], 1);
        assert!(cps.iter().all(|o| matches!(o, Op::Checkpoint(_))), "{cps:?}");
        assert_eq!(cps.len(), 2 + 2 * 3);
    }

    /// Nothing armed records nothing at all — the property the zero-cost claim leans on.
    #[test]
    fn no_piece_records_nothing() {
        let (slot, ops) = frame(0, 4, &[HostScope::Fxaa], 1);
        assert!(ops.is_empty(), "{ops:?}");
        assert_eq!(slot.cursor.get(), 5, "positions are still counted for the ledger");
    }

    /// A frame that entered no scope closes nothing (no unbalanced label end).
    #[test]
    fn an_empty_frame_records_nothing_at_close() {
        let (slot, ops) = frame(ALL, 0, &[], 1);
        assert!(ops.is_empty());
        assert!(!slot.pending.get(), "an empty frame is not verified");
    }

    /// Past the cap a position gets its label and checkpoints but no marker, and is counted.
    #[test]
    fn overflow_at_the_cap_keeps_labels_and_drops_markers() {
        let n = CRUMB_SCOPE_CAP + 3;
        let (slot, ops) = frame(ALL, n, &[], 9);
        assert_eq!(slot.overflow.get(), 3);
        assert_eq!(slot.recorded(), CRUMB_SCOPE_CAP);
        assert_eq!(slot.labels_begun.get(), slot.labels_ended.get());
        assert_eq!(slot.labels_begun.get() as usize, n + 1);
        let max_offset = ops
            .iter()
            .filter_map(|o| match o {
                Op::Marker { offset, .. } => Some(*offset),
                _ => None,
            })
            .max()
            .expect("markers were written");
        assert_eq!(max_offset, scope_end_offset(CRUMB_SCOPE_CAP - 1), "no marker past the region");
        assert!(ops.contains(&Op::Checkpoint(encode_checkpoint(9, CheckpointKind::Begin, (n - 1) as u16))));
    }

    /// The region layout the offsets and the classifier share.
    #[test]
    fn region_offsets_match_the_layout() {
        assert_eq!(REGION_BYTES, 1040);
        assert_eq!(scope_begin_offset(0), 16);
        assert_eq!(scope_end_offset(0), 20);
        assert_eq!(scope_end_offset(CRUMB_SCOPE_CAP - 1) as usize + 4, REGION_BYTES);
        assert!(REGION_BYTES * FRAMES_IN_FLIGHT <= CRUMB_BUFFER_BYTES as usize);
    }

    /// A region as the GPU would leave it: positions `< done` complete, `< begun` in flight.
    fn region(serial: u32, done: usize, begun: usize, frame_end: bool) -> [u32; REGION_WORDS] {
        let mut r = [0u32; REGION_WORDS];
        r[W_FRAME_BEGIN] = serial;
        if frame_end {
            r[W_FRAME_END] = serial;
        }
        for p in 0..begun {
            r[W_SCOPES + 2 * p] = serial;
            if p < done {
                r[W_SCOPES + 2 * p + 1] = serial;
            }
        }
        r
    }

    /// The classifier over synthetic regions: complete, an overlap, and stale values.
    ///
    /// RED: classify "started" as `begin != 0` instead of `begin == S` and the stale-`S-2`
    /// region reads as completed.
    #[test]
    fn the_classifier_reads_completion_overlap_and_staleness() {
        let s = 41;
        let all = region(s, 6, 6, true);
        assert!(frame_complete(&all, s, 6));
        assert!((0..6).all(|p| scope_state(&all, s, p) == ScopeState::Completed));

        // `k` complete, `k+1` and `k+2` begun (overlapping), the rest not started.
        let overlap = region(s, 3, 5, false);
        assert!(!frame_complete(&overlap, s, 6));
        assert_eq!(scope_state(&overlap, s, 2), ScopeState::Completed);
        assert_eq!(scope_state(&overlap, s, 3), ScopeState::InFlight);
        assert_eq!(scope_state(&overlap, s, 4), ScopeState::InFlight);
        assert_eq!(scope_state(&overlap, s, 5), ScopeState::NotStarted);

        // A slot whose words still hold frame S-2's values: nothing of S has started.
        let stale = region(s - 2, 6, 6, true);
        assert!(!frame_complete(&stale, s, 6));
        assert!((0..6).all(|p| scope_state(&stale, s, p) == ScopeState::NotStarted));

        // A missing frame end is incomplete even when every scope completed.
        assert!(!frame_complete(&region(s, 6, 6, false), s, 6));
    }

    #[test]
    fn the_serial_skips_zero_on_wrap() {
        assert_eq!(next_serial(0), 1);
        assert_eq!(next_serial(41), 42);
        assert_eq!(next_serial(u32::MAX), 1);
    }

    #[test]
    fn labels_are_copied_with_a_nul() {
        let b = label_cstr("ssao_atrous");
        assert_eq!(&b[..12], b"ssao_atrous\0");
        let max = "a".repeat(LABEL_BYTES - 1);
        let b = label_cstr(&max);
        assert_eq!(b[LABEL_BYTES - 1], 0);
        assert!(b[..LABEL_BYTES - 1].iter().all(|&c| c == b'a'));
    }

    /// A 64-byte name is a declaration defect: refused in a debug build …
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "a GB label is at most 63 bytes")]
    fn a_64_byte_label_is_refused_in_debug() {
        let _ = label_cstr(&"a".repeat(LABEL_BYTES));
    }

    /// … and truncated, still NUL-terminated, in release.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_64_byte_label_is_truncated_in_release() {
        let b = label_cstr(&"a".repeat(LABEL_BYTES));
        assert_eq!(b[LABEL_BYTES - 1], 0);
    }

    fn props(types: &[VkFlags]) -> VkPhysicalDeviceMemoryProperties {
        let mut p = VkPhysicalDeviceMemoryProperties {
            memory_type_count: types.len() as u32,
            memory_types: [VkMemoryType { property_flags: 0, heap_index: 0 }; VK_MAX_MEMORY_TYPES],
            memory_heap_count: 0,
            memory_heaps: [VkMemoryHeap { size: 0, flags: 0 }; VK_MAX_MEMORY_HEAPS],
        };
        for (i, &f) in types.iter().enumerate() {
            p.memory_types[i] = VkMemoryType { property_flags: f, heap_index: 0 };
        }
        p
    }

    /// The marker buffer prefers host-coherent system memory over a BAR (device-local) mapping,
    /// and falls back to the BAR when it is the only host-visible type.
    #[test]
    fn the_marker_memory_prefers_system_memory() {
        let dl = VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
        let hv = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT;
        let hc = VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        let p = props(&[dl, dl | hv | hc, hv | hc]);
        assert_eq!(crumb_memory_type(&p, u32::MAX), Some(2));
        assert_eq!(crumb_memory_type(&p, 0b011), Some(1), "only the BAR is admitted");
        assert_eq!(crumb_memory_type(&p, 0b001), None);
        assert_eq!(crumb_memory_type(&props(&[hv]), u32::MAX), None, "coherence is required");
    }
}
