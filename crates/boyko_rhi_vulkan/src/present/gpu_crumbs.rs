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

    /// Every host scope, in id order.
    pub(crate) const ALL: [Self; 3] = [Self::Fxaa, Self::Smaa, Self::PresentBlit];

    /// The host scope `id` names, if it names one (the device-loss report).
    pub(crate) fn from_id(id: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
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

/// Unpacks [`encode_checkpoint`]; `None` for a payload GB never wrote (the device-loss report).
pub(crate) fn decode_checkpoint(payload: u64) -> Option<(u32, CheckpointKind, u16)> {
    let kind = match (payload >> 16) & 0xFFFF {
        0 => CheckpointKind::Begin,
        1 => CheckpointKind::End,
        2 => CheckpointKind::FrameBegin,
        3 => CheckpointKind::FrameEnd,
        _ => return None,
    };
    Some(((payload >> 32) as u32, kind, (payload & 0xFFFF) as u16))
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
    /// The mapping is live (the buffer was not destroyed), and the caller either waited the fence
    /// of the submission that last wrote the slot or saw the device lost. On a live device the
    /// waited frame's close made the writes host-visible; on a lost device (where the wait may
    /// still return `VK_SUCCESS`) the mapping stays valid with undefined contents.
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
/// counter, the ledger, the two slot shadows and the loss suspect. `Some` on the [`Renderer`] only
/// in an armed run.
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
    /// The frame kept for the device-loss report when its breadcrumbs were incomplete after a
    /// successful fence wait ([`verify_frame`]).
    suspect: LossSuspect,
}

/// The loss suspect: the first frame, since the device last proved alive, whose breadcrumbs were
/// incomplete after its fence wait returned `VK_SUCCESS`.
///
/// The spec lets `vkWaitForFences` return `VK_SUCCESS` on a lost device ("Lost Device"), so such a
/// frame is a suspected loss, not yet a G2 break. Its slot's shadow is reused two frames later, so
/// the frame is copied out here, and the device-loss report still names its in-flight set when the
/// loss only surfaces at a later submit (GB review r1 W1). A later frame that completes proves the
/// device alive: it clears the suspect, and the incomplete frame becomes a G2 break.
pub(crate) struct LossSuspect {
    /// The suspect's serial, `0` when none is held (a serial never takes `0`): the one word every
    /// verified frame reads.
    serial: Cell<u32>,
    /// The suspect as it stood when it was verified: its shadow and its region as read then.
    /// Copied only on the cold paths (a new suspect, the report).
    frame: Cell<Option<SlotSnapshot>>,
}

impl LossSuspect {
    /// No suspect held.
    pub(crate) fn new() -> Self {
        Self { serial: Cell::new(0), frame: Cell::new(None) }
    }

    /// The suspect held, if any.
    fn get(&self) -> Option<SlotSnapshot> {
        if self.serial.get() == 0 { None } else { self.frame.get() }
    }
}

/// What [`verify_frame`] found for one frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FrameVerdict {
    /// Graph-pass scopes entered == passes the graph declared. Host-side bookkeeping, so a lost
    /// device cannot falsify it.
    pub(crate) parity: bool,
    /// Every breadcrumb of the frame carries its serial (always `true` without the markers piece).
    pub(crate) complete: bool,
    /// `Some(s)`: this frame completed while the suspect of serial `s` was held. The device ran
    /// past `s`, so `s` was incomplete on a LIVE device: a G2 break, not a loss.
    pub(crate) incomplete_while_alive: Option<u32>,
}

/// Verifies the frame last recorded into `slot` (slot index `fi`) after its fence wait returned
/// `VK_SUCCESS`, and folds it into `ledger`. `region` is the slot's words as read back (`None`: the
/// markers piece is not armed). The device-free half of [`Renderer::crumb_verify_slot`]:
/// - the ledger counts the frame, its scopes, its labels and its parity;
/// - an incomplete frame is counted in `frames_incomplete` and becomes the loss suspect when none
///   is held (the first one is nearest the loss, so a later one never replaces it);
/// - a complete frame newer than a held suspect clears the suspect and names it in the verdict.
pub(crate) fn verify_frame(
    slot: &CrumbSlot,
    fi: usize,
    region: Option<&[u32; REGION_WORDS]>,
    ledger: &Cell<CrumbLedger>,
    suspect: &LossSuspect,
) -> FrameVerdict {
    let serial = slot.serial.get();
    let mut l = ledger.get();
    l.frames_checked += 1;
    l.scopes_checked += u64::from(slot.cursor.get());
    l.overflow += u64::from(slot.overflow.get());
    l.labels_begun += u64::from(slot.labels_begun.get());
    l.labels_ended += u64::from(slot.labels_ended.get());
    let parity = slot.passes_entered.get() == slot.pass_count.get();
    if !parity {
        l.parity_mismatch += 1;
    }
    let complete = region.is_none_or(|r| frame_complete(r, serial, slot.recorded()));
    let held = suspect.serial.get();
    let mut incomplete_while_alive = None;
    if !complete {
        l.frames_incomplete += 1;
        if held == 0 {
            suspect.frame.set(Some(SlotSnapshot::of(slot, fi, region.copied())));
            suspect.serial.set(serial);
        }
    } else if held != 0 && (serial.wrapping_sub(held) as i32) > 0 {
        suspect.serial.set(0);
        suspect.frame.set(None);
        incomplete_while_alive = Some(held);
    }
    let last = slot.last_verified.get();
    if last != 0 && (serial.wrapping_sub(last) as i32) <= 0 {
        l.serial_regressions += 1;
    }
    slot.last_verified.set(serial);
    if l.last_serial == 0 || (serial.wrapping_sub(l.last_serial) as i32) > 0 {
        l.last_serial = serial;
    }
    ledger.set(l);
    FrameVerdict { parity, complete, incomplete_while_alive }
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
            suspect: LossSuspect::new(),
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

    /// Verifies the frame last recorded into slot `fi`, whose fence wait the caller just saw
    /// return `VK_SUCCESS`, and folds it into the ledger ([`verify_frame`]): scope parity (scopes
    /// entered == passes declared), and with markers, completion (every scope's begin and end and
    /// the frame end carry its serial).
    ///
    /// A debug build asserts parity on every frame. Completion is asserted one frame late: the
    /// spec lets the wait return `VK_SUCCESS` on a lost device, so an incomplete frame is first
    /// kept as the loss suspect, and it is a G2 break only once a later frame completes and so
    /// proves the device alive. Every armed debug frame stays a G2 check, and a real loss still
    /// reaches the `boyko-E3013` report instead of a panic (GB review r1 W1).
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
        let region = st.buffer.as_ref().map(|b| {
            let mut words = [0u32; REGION_WORDS];
            // SAFETY: the buffer is live (destroyed only in the renderer's `Drop`), and the caller
            // waited the fence of the submission that recorded this slot's frame. On a live device
            // that frame's close made its marker writes host-visible. On a lost device the wait may
            // still return `VK_SUCCESS` (spec, "Lost Device"); the mapping then stays valid with
            // undefined contents, which is why `verify_frame` keeps an incomplete frame as a suspect
            // rather than trusting it. Either way the reads are volatile, in bounds, and form no
            // reference over the mapping.
            unsafe { b.read_region(fi, &mut words) };
            words
        });
        let v = verify_frame(slot, fi, region.as_ref(), &st.ledger, &st.suspect);
        debug_assert!(
            v.parity,
            "invariant (GB G2): {} graph-pass scopes entered for {} declared passes on serial {} \
             ({} declarator) -- a declared pass never crossed a funnel, or one crossed twice",
            slot.passes_entered.get(),
            slot.pass_count.get(),
            slot.serial.get(),
            DECLARATOR_NAMES.get(slot.declarator.get() as usize).copied().unwrap_or("?"),
        );
        debug_assert!(
            v.incomplete_while_alive.is_none(),
            "invariant (GB G2): serial {}'s breadcrumbs were incomplete after its fence wait, and \
             serial {} then completed, so the device was alive -- a scope's marker never landed",
            v.incomplete_while_alive.unwrap_or(0),
            slot.serial.get(),
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

// ---------------------------------------------------------------------------------------------
// The device-loss report (render rung GB, D8 / D9).
// ---------------------------------------------------------------------------------------------

/// Checkpoint records read back per loss; NVIDIA reports one per pipeline stage, far below this.
const CHECKPOINT_READ_CAP: usize = 16;
/// Address and vendor records read back per fault.
const FAULT_RECORD_CAP: usize = 16;
/// Address lines / vendor-record lines the report prints (each record line is bounded).
const FAULT_ADDRESS_LINES: usize = 4;
const FAULT_VENDOR_LINES: usize = 3;
/// In-flight scopes one slot line lists by name.
const IN_FLIGHT_NAMED: usize = 4;
/// One report line's bound, in bytes.
pub(crate) const REPORT_LINE_BYTES: usize = 256;
/// The loss suspect's slot-line tag: the line keeps the `slot … passes N` head every slot line
/// has (G3 parses it), and says why this frame is listed although its slot moved on.
const SUSPECT_TAG: &str = " (suspect: incomplete after its fence wait)";

/// One slot's frame, copied out of the shadow and the marker region at report time.
#[derive(Clone, Copy)]
struct SlotSnapshot {
    fi: usize,
    serial: u32,
    declarator: u8,
    pass_count: u16,
    cursor: u16,
    overflow: u16,
    /// `Some` when the markers piece is armed (the region read through the mapping).
    region: Option<[u32; REGION_WORDS]>,
    scope_ids: [u16; CRUMB_SCOPE_CAP],
    names: [&'static str; CRUMB_SCOPE_CAP],
}

impl SlotSnapshot {
    /// Copies `slot`'s shadow (slot index `fi`) beside `region`, its words as read back.
    fn of(slot: &CrumbSlot, fi: usize, region: Option<[u32; REGION_WORDS]>) -> Self {
        Self {
            fi,
            serial: slot.serial.get(),
            declarator: slot.declarator.get(),
            pass_count: slot.pass_count.get(),
            cursor: slot.cursor.get(),
            overflow: slot.overflow.get(),
            region,
            scope_ids: core::array::from_fn(|i| slot.scope_ids[i].get()),
            names: core::array::from_fn(|i| slot.names[i].get()),
        }
    }

    fn recorded(&self) -> usize {
        (self.cursor as usize).min(CRUMB_SCOPE_CAP)
    }

    /// The name of record position `pos`, with its `#ordinal` among same-named passes of the frame
    /// when the name repeats (`ssao_atrous#2` is the third `ssao_atrous`).
    fn write_scope<W: core::fmt::Write>(&self, out: &mut W, pos: usize) -> core::fmt::Result {
        let id = self.scope_ids[pos];
        if let Some(h) = HostScope::from_id(id) {
            return write!(out, "#{pos} {}", h.name());
        }
        let p = id as usize;
        let Some(&name) = self.names.get(p) else {
            return write!(out, "#{pos} pass{id}");
        };
        let declared = (self.pass_count as usize).min(CRUMB_SCOPE_CAP);
        let total = self.names[..declared].iter().filter(|n| **n == name).count();
        if total > 1 {
            let ordinal = self.names[..p.min(declared)].iter().filter(|n| **n == name).count();
            write!(out, "#{pos} {name}#{ordinal}")
        } else {
            write!(out, "#{pos} {name}")
        }
    }
}

/// The checkpoint half of the report.
// One value per report, built once on the terminal path: the 272-byte variant costs nothing, and
// boxing it would add an allocation for no benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy)]
enum CheckpointRead {
    /// The level did not ask for checkpoints.
    NotRequested,
    /// The level asked, the device lacks them (`boyko-W2107` said so at boot).
    Unavailable,
    /// `(stage, payload)` per record, `total` as the driver counted.
    Read { total: u32, count: usize, entries: [(VkFlags, u64); CHECKPOINT_READ_CAP] },
}

/// What the vendor binary capture did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VendorBinaryStatus {
    /// The fault piece is not armed, or the device does not support `deviceFaultVendorBinary`.
    Unsupported,
    /// The driver reported a zero-sized binary.
    Empty,
    /// The host could not allocate the reported size (`try_reserve_exact` failed).
    AllocationFailed,
    /// Captured; the bytes are [`DeviceLostReport::vendor_binary`].
    Captured,
}

/// The fault half of the report.
enum FaultRead {
    NotRequested,
    Unavailable,
    QueryFailed(VkResult),
    Read(Box<FaultRecords>),
}

/// `vkGetDeviceFaultInfoEXT`'s output, bounded.
struct FaultRecords {
    description: [core::ffi::c_char; VK_MAX_DESCRIPTION_SIZE],
    address_total: u32,
    addresses: [VkDeviceFaultAddressInfoExt; FAULT_RECORD_CAP],
    address_count: usize,
    vendor_total: u32,
    vendors: [VkDeviceFaultVendorInfoExt; FAULT_RECORD_CAP],
    vendor_count: usize,
    binary_size: u64,
}

/// Render rung GB: everything the renderer can say after `VK_ERROR_DEVICE_LOST` — both slots'
/// breadcrumbs with their pass names, the NVIDIA checkpoints and the `VK_EXT_device_fault` report
/// with its vendor binary. Built by [`Renderer::device_lost_report`]; rendered into bounded lines
/// by [`Self::write_lines`]. Marker contents after a loss are undefined by the spec, so every
/// line is a hint, and the report says so.
pub struct DeviceLostReport {
    level: GpuCrashDiagLevel,
    pieces: u8,
    frame_pieces: u8,
    slots: [Option<SlotSnapshot>; FRAMES_IN_FLIGHT],
    /// The loss suspect held at the loss ([`LossSuspect`]): a frame already seen incomplete after
    /// a successful fence wait, whose slot may since have been reused.
    suspect: Option<SlotSnapshot>,
    checkpoints: CheckpointRead,
    fault: FaultRead,
    vendor_binary: Vec<u8>,
    vendor_status: VendorBinaryStatus,
}

/// The `VkPipelineStageFlagBits` name a checkpoint reports, or `None` for a combination.
fn stage_name(stage: VkFlags) -> Option<&'static str> {
    Some(match stage {
        0x0000_0001 => "TOP_OF_PIPE",
        0x0000_0002 => "DRAW_INDIRECT",
        0x0000_0004 => "VERTEX_INPUT",
        0x0000_0008 => "VERTEX_SHADER",
        0x0000_0080 => "FRAGMENT_SHADER",
        0x0000_0100 => "EARLY_FRAGMENT_TESTS",
        0x0000_0200 => "LATE_FRAGMENT_TESTS",
        0x0000_0400 => "COLOR_ATTACHMENT_OUTPUT",
        0x0000_0800 => "COMPUTE_SHADER",
        0x0000_1000 => "TRANSFER",
        0x0000_2000 => "BOTTOM_OF_PIPE",
        0x0000_4000 => "HOST",
        0x0001_0000 => "ALL_COMMANDS",
        _ => return None,
    })
}

/// The `VkDeviceFaultAddressTypeEXT` name.
fn address_type_name(t: i32) -> &'static str {
    match t {
        0 => "none",
        1 => "read-invalid",
        2 => "write-invalid",
        3 => "execute-invalid",
        4 => "ip-unknown",
        5 => "ip-invalid",
        6 => "ip-fault",
        _ => "unknown-type",
    }
}

/// The NUL-terminated `c_char` array as text, lossy, without allocating.
fn c_text(chars: &[core::ffi::c_char]) -> &str {
    // SAFETY: `c_char` is `i8` or `u8`, both one byte with alignment 1, so reinterpreting the
    // slice's bytes is sound; the borrow keeps them alive.
    let bytes = unsafe { core::slice::from_raw_parts(chars.as_ptr().cast::<u8>(), chars.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    match core::str::from_utf8(&bytes[..end]) {
        Ok(s) => s,
        Err(e) => core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    }
}

impl DeviceLostReport {
    /// The vendor binary the fault query returned (empty unless [`Self::vendor_status`] is
    /// `Captured`). The host crate writes it to a file; its first 56 bytes are a
    /// `VkDeviceFaultVendorBinaryHeaderVersionOneEXT`, decoded by
    /// [`VendorBinaryHeader::decode`](crate::gpu_crash::VendorBinaryHeader::decode).
    pub fn vendor_binary(&self) -> &[u8] {
        &self.vendor_binary
    }

    /// What the vendor binary capture did.
    pub fn vendor_status(&self) -> VendorBinaryStatus {
        self.vendor_status
    }

    /// Renders the report, one bounded line (≤ 256 bytes, at most 16 lines) per `emit` call.
    ///
    /// `site` and `result` say where the loss surfaced; `binary_outcome` is the host's account of
    /// the vendor binary (the file path it wrote, or why it did not) and is printed only when a
    /// binary was captured.
    pub fn write_lines(&self, site: &str, result: VkResult, binary_outcome: &str, mut emit: impl FnMut(&str)) {
        use core::fmt::Write as _;
        let mut line = boyko_log::DspBuf::<REPORT_LINE_BYTES>::new();
        macro_rules! out {
            ($($arg:tt)*) => {{
                line.clear();
                let _ = write!(&mut line, $($arg)*);
                emit(line.as_str());
            }};
        }

        let mut pieces = boyko_log::DspBuf::<64>::new();
        let _ = crate::gpu_crash::write_pieces(&mut pieces, self.pieces);
        out!(
            "device lost at {site} ({result:?}); GPU crash report: level={} pieces={} profile={}",
            self.level.name(),
            pieces.as_str(),
            boyko_diag::profile::PROFILE_NAME
        );

        let markers = self.frame_pieces & piece::MARKERS != 0;
        let mut order: [Option<&SlotSnapshot>; FRAMES_IN_FLIGHT] = [None; FRAMES_IN_FLIGHT];
        for (o, s) in order.iter_mut().zip(self.slots.iter()) {
            *o = s.as_ref();
        }
        // Newest serial first (serials never take 0 and wrap after 2^32 armed frames). Unstable
        // and in place: two entries with distinct serials (or `None`s, which print nothing) have
        // no order to keep, and the stable sort's scratch is a heap site (UG-02).
        order.sort_unstable_by(|a, b| b.map(|s| s.serial).cmp(&a.map(|s| s.serial)));
        let mut any_in_flight = false;
        let mut any_armed = false;
        if markers {
            // The suspect first: the host already saw that frame incomplete after a successful
            // fence wait, so the loss most likely began there (GB review r1 W1). A slot that
            // still holds the suspect's serial prints instead: its words were read later.
            let suspect =
                self.suspect.as_ref().filter(|x| self.slots.iter().flatten().all(|s| s.serial != x.serial));
            if let Some(snap) = suspect {
                any_armed = true;
                any_in_flight |= self.write_slot(snap, SUSPECT_TAG, &mut line, &mut emit);
            }
            for snap in order.iter().flatten() {
                any_armed = true;
                any_in_flight |= self.write_slot(snap, "", &mut line, &mut emit);
            }
            for (fi, s) in self.slots.iter().enumerate() {
                if s.is_none() {
                    out!("slot {fi}: no armed frame recorded");
                }
            }
            if any_armed && !any_in_flight && suspect.is_none() {
                out!(
                    "no GB scope was in flight in either slot: the loss is likely outside the frame command \
                     buffer (an upload or readback submission on the same queue)"
                );
            }
        } else {
            out!("breadcrumbs not armed (BOYKO_GPU_CRASH_DIAG=markers arms them)");
        }

        match &self.checkpoints {
            CheckpointRead::NotRequested => {
                out!("checkpoints not armed (BOYKO_GPU_CRASH_DIAG=checkpoints arms them; NVIDIA only)")
            }
            CheckpointRead::Unavailable => out!("checkpoints unavailable on this device"),
            CheckpointRead::Read { total, count, entries } => {
                line.clear();
                let _ = write!(&mut line, "checkpoints ({total}):");
                for (i, &(stage, payload)) in entries[..*count].iter().enumerate() {
                    let _ = if i > 0 { line.write_str(";") } else { Ok(()) };
                    let _ = match stage_name(stage) {
                        Some(n) => write!(&mut line, " {n} ->"),
                        None => write!(&mut line, " 0x{stage:x} ->"),
                    };
                    match decode_checkpoint(payload) {
                        Some((serial, kind, pos)) => {
                            let k = match kind {
                                CheckpointKind::Begin => "begin",
                                CheckpointKind::End => "end",
                                CheckpointKind::FrameBegin => "frame-begin",
                                CheckpointKind::FrameEnd => "frame-end",
                            };
                            let _ = write!(&mut line, " serial {serial} {k}");
                            if matches!(kind, CheckpointKind::Begin | CheckpointKind::End) {
                                let snap = self.slots.iter().flatten().find(|s| s.serial == serial);
                                let _ = match snap {
                                    Some(s) if (pos as usize) < s.recorded() => {
                                        let _ = line.write_str(" ");
                                        s.write_scope(&mut line, pos as usize)
                                    }
                                    _ => write!(&mut line, " #{pos}"),
                                };
                            }
                        }
                        None => {
                            let _ = write!(&mut line, " 0x{payload:x}");
                        }
                    }
                }
                emit(line.as_str());
            }
        }

        match &self.fault {
            FaultRead::NotRequested => out!("device fault: not armed (BOYKO_GPU_CRASH_DIAG=fault arms it)"),
            FaultRead::Unavailable => out!("device fault: VK_EXT_device_fault is unavailable on this device"),
            FaultRead::QueryFailed(r) => out!("device fault: vkGetDeviceFaultInfoEXT returned {r:?}"),
            FaultRead::Read(f) => {
                out!(
                    "device fault: \"{}\"; addresses {}; vendor records {}; vendor binary {} B",
                    c_text(&f.description),
                    f.address_total,
                    f.vendor_total,
                    f.binary_size
                );
                for (i, a) in f.addresses[..f.address_count.min(FAULT_ADDRESS_LINES)].iter().enumerate() {
                    let mask = a.address_precision.max(1).wrapping_sub(1);
                    let (lo, hi) = (a.reported_address & !mask, a.reported_address | mask);
                    let more = if i + 1 == FAULT_ADDRESS_LINES && f.address_count > FAULT_ADDRESS_LINES {
                        f.address_count - FAULT_ADDRESS_LINES
                    } else {
                        0
                    };
                    out!(
                        "fault address {i}: {} 0x{:x} [0x{lo:x}..0x{hi:x}]{}",
                        address_type_name(a.address_type),
                        a.reported_address,
                        if more > 0 { " (more follow)" } else { "" }
                    );
                }
                for (i, v) in f.vendors[..f.vendor_count.min(FAULT_VENDOR_LINES)].iter().enumerate() {
                    out!(
                        "fault vendor record {i}: code 0x{:x} data 0x{:x} \"{}\"",
                        v.vendor_fault_code,
                        v.vendor_fault_data,
                        c_text(&v.description)
                    );
                }
            }
        }

        match self.vendor_status {
            VendorBinaryStatus::Captured => match crate::gpu_crash::VendorBinaryHeader::decode(&self.vendor_binary) {
                Some(h) => {
                    line.clear();
                    let _ = write!(
                        &mut line,
                        "vendor binary: {} B, vendor 0x{:x} device 0x{:x} driver 0x{:x} uuid ",
                        self.vendor_binary.len(),
                        h.vendor_id,
                        h.device_id,
                        h.driver_version
                    );
                    for b in h.pipeline_cache_uuid {
                        let _ = write!(&mut line, "{b:02x}");
                    }
                    let _ = write!(&mut line, " -> {binary_outcome}");
                    emit(line.as_str());
                }
                None => out!(
                    "vendor binary: {} B, no version-one header -> {binary_outcome}",
                    self.vendor_binary.len()
                ),
            },
            VendorBinaryStatus::Unsupported => out!("vendor binary: not captured: unsupported"),
            VendorBinaryStatus::Empty => out!("vendor binary: not captured: the driver reported none"),
            VendorBinaryStatus::AllocationFailed => out!("vendor binary: not captured: the host allocation failed"),
        }
        out!(
            "note: scopes overlap where no barrier orders them, so the in-flight set is a SET, not a culprit; \
             marker contents after a device loss are undefined by the spec (read as a hint)"
        );
    }

    /// One slot's breadcrumb line, `tag` after its header (empty for a live slot). Returns whether
    /// any scope of it was in flight.
    fn write_slot(
        &self,
        s: &SlotSnapshot,
        tag: &str,
        line: &mut boyko_log::DspBuf<REPORT_LINE_BYTES>,
        emit: &mut impl FnMut(&str),
    ) -> bool {
        use core::fmt::Write as _;
        line.clear();
        let decl = DECLARATOR_NAMES.get(s.declarator as usize).copied().unwrap_or("?");
        let _ = write!(
            line,
            "slot {} serial {} declarator {decl} passes {}{tag}: ",
            s.fi, s.serial, s.pass_count
        );
        let Some(region) = s.region.as_ref() else {
            let _ = line.write_str("no breadcrumbs");
            emit(line.as_str());
            return false;
        };
        let (frame_begin, frame_end) = (region[W_FRAME_BEGIN], region[W_FRAME_END]);
        let recorded = s.recorded();
        if frame_end == s.serial && frame_complete(region, s.serial, recorded) {
            let _ = write!(line, "frame complete ({} scopes)", s.cursor);
            emit(line.as_str());
            return false;
        }
        if frame_begin != s.serial {
            let _ = write!(line, "frame not started ({} scopes recorded)", s.cursor);
            emit(line.as_str());
            return false;
        }
        let state = |p: usize| scope_state(region, s.serial, p);
        match (0..recorded).rev().find(|&p| state(p) == ScopeState::Completed) {
            Some(p) => {
                let _ = line.write_str("last completed ");
                let _ = s.write_scope(line, p);
            }
            None => {
                let _ = line.write_str("none completed");
            }
        }
        let in_flight = (0..recorded).filter(|&p| state(p) == ScopeState::InFlight).count();
        let _ = line.write_str("; in flight:");
        for (i, p) in (0..recorded).filter(|&p| state(p) == ScopeState::InFlight).take(IN_FLIGHT_NAMED).enumerate() {
            let _ = line.write_str(if i == 0 { " " } else { ", " });
            let _ = s.write_scope(line, p);
        }
        if in_flight == 0 {
            let _ = line.write_str(" none");
        } else if in_flight > IN_FLIGHT_NAMED {
            let _ = write!(line, " (+{} more)", in_flight - IN_FLIGHT_NAMED);
        }
        let not_started = (0..recorded).filter(|&p| state(p) == ScopeState::NotStarted).count();
        let _ = write!(line, "; not started: {not_started}");
        if let Some(p) = (0..recorded).find(|&p| state(p) == ScopeState::NotStarted) {
            let _ = line.write_str(" (first ");
            let _ = s.write_scope(line, p);
            let _ = line.write_str(")");
        }
        if s.overflow > 0 {
            let _ = write!(line, "; {} scopes past the breadcrumb cap", s.overflow);
        }
        emit(line.as_str());
        in_flight > 0
    }
}

impl Renderer<'_> {
    /// Render rung GB: reads everything the GPU crash diagnostics can say about a device loss —
    /// both slots' breadcrumbs (raw volatile reads of the mapping) with their pass names, the
    /// NVIDIA checkpoints (`vkGetQueueCheckpointDataNV`, sync1) and the `VK_EXT_device_fault`
    /// report with its vendor binary. Cold: the terminal path.
    ///
    /// # Safety
    ///
    /// The device reported `VK_ERROR_DEVICE_LOST` (both queries are legal only on a lost device:
    /// `VUID-vkGetQueueCheckpointDataNV-queue-02025`, `VUID-vkGetDeviceFaultInfoEXT-device-07336`),
    /// and `ctx` is the context this renderer was created on.
    #[cold]
    #[inline(never)]
    pub unsafe fn device_lost_report(&self, ctx: &VulkanContext) -> DeviceLostReport {
        let crash = ctx.gpu_crash();
        let st = self.crumbs.as_deref();
        let mut slots: [Option<SlotSnapshot>; FRAMES_IN_FLIGHT] = [None; FRAMES_IN_FLIGHT];
        if let Some(st) = st {
            for (fi, out) in slots.iter_mut().enumerate() {
                let slot = &st.slots[fi];
                if slot.serial.get() == 0 {
                    continue;
                }
                let region = st.buffer.as_ref().map(|b| {
                    let mut words = [0u32; REGION_WORDS];
                    // SAFETY: the buffer is live (destroyed only in the renderer's `Drop`); on a
                    // lost device the mapping stays valid with undefined contents, which the
                    // report states; the reads are volatile and form no reference.
                    unsafe { b.read_region(fi, &mut words) };
                    words
                });
                *out = Some(SlotSnapshot::of(slot, fi, region));
            }
        }
        let suspect = st.and_then(|s| s.suspect.get());

        let wants = crash.level.requested_pieces();
        let checkpoints = match st.and_then(|s| s.checkpoints) {
            Some(c) => {
                let mut total: u32 = 0;
                // SAFETY: the device is lost (this fn's contract), the one state the query is
                // legal in; `self.queue` is the renderer's live queue; a NULL array is the count
                // query; `&mut total` is a valid out-pointer.
                unsafe { (c.get_queue_checkpoint_data)(self.queue, &mut total, ptr::null_mut()) };
                let mut data: [VkCheckpointDataNv; CHECKPOINT_READ_CAP] = core::array::from_fn(|_| VkCheckpointDataNv {
                    s_type: VkStructureType::CheckpointDataNv,
                    p_next: ptr::null_mut(),
                    stage: 0,
                    p_checkpoint_marker: ptr::null_mut(),
                });
                let mut count = total.min(CHECKPOINT_READ_CAP as u32);
                // SAFETY: as above; `data` holds `count <= CHECKPOINT_READ_CAP` initialised
                // output structs (each with its `s_type`), and `count` tells the driver so.
                unsafe { (c.get_queue_checkpoint_data)(self.queue, &mut count, data.as_mut_ptr()) };
                let count = (count as usize).min(CHECKPOINT_READ_CAP);
                let mut entries = [(0u32, 0u64); CHECKPOINT_READ_CAP];
                for (e, d) in entries.iter_mut().zip(&data[..count]) {
                    *e = (d.stage, d.p_checkpoint_marker.addr() as u64);
                }
                CheckpointRead::Read { total, count, entries }
            }
            None if wants & piece::CHECKPOINTS != 0 => CheckpointRead::Unavailable,
            None => CheckpointRead::NotRequested,
        };

        let mut vendor_binary = Vec::new();
        let mut vendor_status = VendorBinaryStatus::Unsupported;
        let fault = match crash.fault {
            Some(f) => {
                let mut counts = VkDeviceFaultCountsExt {
                    s_type: VkStructureType::DeviceFaultCountsExt,
                    p_next: ptr::null_mut(),
                    address_info_count: 0,
                    vendor_info_count: 0,
                    vendor_binary_size: 0,
                };
                // SAFETY: the device is lost (contract); `ctx.device()` is the live device the
                // fault piece was enabled on; `counts` is an initialised in/out struct and a NULL
                // `p_fault_info` is the count query.
                let r = VkResult::from_raw(unsafe { (f.get_device_fault_info)(ctx.device(), &mut counts, ptr::null_mut()) });
                if !r.is_success() && r != VkResult::INCOMPLETE {
                    FaultRead::QueryFailed(r)
                } else {
                    let mut rec = Box::new(FaultRecords {
                        description: [0; VK_MAX_DESCRIPTION_SIZE],
                        address_total: counts.address_info_count,
                        addresses: [VkDeviceFaultAddressInfoExt { address_type: 0, reported_address: 0, address_precision: 0 };
                            FAULT_RECORD_CAP],
                        address_count: 0,
                        vendor_total: counts.vendor_info_count,
                        vendors: [VkDeviceFaultVendorInfoExt {
                            description: [0; VK_MAX_DESCRIPTION_SIZE],
                            vendor_fault_code: 0,
                            vendor_fault_data: 0,
                        }; FAULT_RECORD_CAP],
                        vendor_count: 0,
                        binary_size: counts.vendor_binary_size,
                    });
                    let binary_len = usize::try_from(counts.vendor_binary_size).unwrap_or(usize::MAX);
                    if f.vendor_binary {
                        vendor_status = if binary_len == 0 {
                            VendorBinaryStatus::Empty
                        } else if vendor_binary.try_reserve_exact(binary_len).is_ok() {
                            vendor_binary.resize(binary_len, 0);
                            VendorBinaryStatus::Captured
                        } else {
                            VendorBinaryStatus::AllocationFailed
                        };
                    }
                    counts.address_info_count = counts.address_info_count.min(FAULT_RECORD_CAP as u32);
                    counts.vendor_info_count = counts.vendor_info_count.min(FAULT_RECORD_CAP as u32);
                    counts.vendor_binary_size =
                        if vendor_status == VendorBinaryStatus::Captured { vendor_binary.len() as u64 } else { 0 };
                    let mut info = VkDeviceFaultInfoExt {
                        s_type: VkStructureType::DeviceFaultInfoExt,
                        p_next: ptr::null_mut(),
                        description: [0; VK_MAX_DESCRIPTION_SIZE],
                        p_address_infos: rec.addresses.as_mut_ptr(),
                        p_vendor_infos: rec.vendors.as_mut_ptr(),
                        p_vendor_binary_data: if counts.vendor_binary_size > 0 {
                            vendor_binary.as_mut_ptr().cast()
                        } else {
                            ptr::null_mut()
                        },
                    };
                    // SAFETY: the device is lost (contract); `counts` now states the capacities of
                    // the arrays `info` points at — `rec.addresses` / `rec.vendors` hold
                    // `FAULT_RECORD_CAP` entries each and `vendor_binary` holds exactly
                    // `vendor_binary_size` initialised bytes (or the pointer is NULL with a size of
                    // 0); every pointee outlives the call and nothing else references it.
                    let r = VkResult::from_raw(unsafe { (f.get_device_fault_info)(ctx.device(), &mut counts, &mut info) });
                    if !r.is_success() && r != VkResult::INCOMPLETE {
                        vendor_binary.clear();
                        vendor_status = if f.vendor_binary { VendorBinaryStatus::Empty } else { vendor_status };
                        FaultRead::QueryFailed(r)
                    } else {
                        rec.address_count = (counts.address_info_count as usize).min(FAULT_RECORD_CAP);
                        rec.vendor_count = (counts.vendor_info_count as usize).min(FAULT_RECORD_CAP);
                        rec.description = info.description;
                        if vendor_status == VendorBinaryStatus::Captured {
                            vendor_binary.truncate((counts.vendor_binary_size as usize).min(vendor_binary.len()));
                        }
                        FaultRead::Read(rec)
                    }
                }
            }
            None if wants & piece::FAULT != 0 => FaultRead::Unavailable,
            None => FaultRead::NotRequested,
        };

        DeviceLostReport {
            level: crash.level,
            pieces: crash.pieces,
            frame_pieces: self.crumb_bits,
            slots,
            suspect,
            checkpoints,
            fault,
            vendor_binary,
            vendor_status,
        }
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

    #[test]
    fn checkpoint_payloads_round_trip() {
        for kind in [CheckpointKind::Begin, CheckpointKind::End, CheckpointKind::FrameBegin, CheckpointKind::FrameEnd] {
            for (serial, pos) in [(1u32, 0u16), (u32::MAX, u16::MAX), (0x1234_5678, 77)] {
                assert_eq!(decode_checkpoint(encode_checkpoint(serial, kind, pos)), Some((serial, kind, pos)));
            }
        }
        assert_eq!(decode_checkpoint(0x0000_0001_0009_0000), None, "an unknown kind is not GB's");
        for h in HostScope::ALL {
            assert_eq!(HostScope::from_id(h.id()), Some(h));
        }
        assert_eq!(HostScope::from_id(7), None);
    }

    const NAMES: [&str; 6] = ["gbuffer", "ssao", "ssao_atrous", "ssao_atrous", "ssao_atrous", "resolve"];

    /// A slot whose frame `serial` recorded the six passes of [`NAMES`] then the present blit.
    fn snapshot(fi: usize, serial: u32, region: [u32; REGION_WORDS]) -> SlotSnapshot {
        let mut scope_ids = [0u16; CRUMB_SCOPE_CAP];
        let mut names = [""; CRUMB_SCOPE_CAP];
        for (p, n) in NAMES.iter().enumerate() {
            scope_ids[p] = p as u16;
            names[p] = n;
        }
        scope_ids[NAMES.len()] = HostScope::PresentBlit.id();
        SlotSnapshot {
            fi,
            serial,
            declarator: 0,
            pass_count: NAMES.len() as u16,
            cursor: NAMES.len() as u16 + 1,
            overflow: 0,
            region: Some(region),
            scope_ids,
            names,
        }
    }

    fn vendor_fixture() -> Vec<u8> {
        let mut b = vec![0u8; crate::gpu_crash::VendorBinaryHeader::SIZE + 8];
        for (off, v) in [(0usize, 56u32), (4, 1), (8, 0x10de), (12, 0x2c02), (16, 0x9988_7766), (52, 0x0040_4155)] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn lines_of(r: &DeviceLostReport, outcome: &str) -> Vec<String> {
        let mut lines = Vec::new();
        r.write_lines("render", VkResult::ERROR_DEVICE_LOST, outcome, |l| lines.push(l.to_owned()));
        lines
    }

    /// The device-loss report over a synthetic loss: both slots, newest serial first, the
    /// in-flight SET with ordinals for the repeated à-trous passes, the checkpoints resolved to
    /// pass names, the fault section and the vendor-binary header — every line bounded.
    ///
    /// RED: drop the newest-first sort in `write_lines` and slot 0 (serial 8) prints first.
    #[test]
    fn the_report_names_the_in_flight_set_newest_slot_first() {
        let (old, new) = (8u32, 9u32);
        // Slot 1 holds the newer frame: 3 scopes completed, 2 overlapping in flight, 2 not started.
        let mut lost = region(new, 3, 5, false);
        lost[W_SCOPES + 2 * 5] = old; // a stale begin from the frame before is "not started"
        let mut fault = Box::new(FaultRecords {
            description: [0; VK_MAX_DESCRIPTION_SIZE],
            address_total: 1,
            addresses: [VkDeviceFaultAddressInfoExt { address_type: 2, reported_address: 0x1234_5678, address_precision: 4096 };
                FAULT_RECORD_CAP],
            address_count: 1,
            vendor_total: 1,
            vendors: [VkDeviceFaultVendorInfoExt { description: [0; VK_MAX_DESCRIPTION_SIZE], vendor_fault_code: 0xC0, vendor_fault_data: 0xDA7A };
                FAULT_RECORD_CAP],
            vendor_count: 1,
            binary_size: 64,
        });
        for (i, b) in b"MMU fault".iter().enumerate() {
            fault.description[i] = *b as core::ffi::c_char;
            fault.vendors[0].description[i] = *b as core::ffi::c_char;
        }
        let mut entries = [(0u32, 0u64); CHECKPOINT_READ_CAP];
        entries[0] = (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, encode_checkpoint(new, CheckpointKind::Begin, 4));
        entries[1] = (VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, encode_checkpoint(new, CheckpointKind::End, 2));
        let report = DeviceLostReport {
            level: GpuCrashDiagLevel::Checkpoints,
            pieces: piece::FAULT | piece::PER_FRAME,
            frame_pieces: piece::PER_FRAME,
            slots: [Some(snapshot(0, old, region(old, 7, 7, true))), Some(snapshot(1, new, lost))],
            suspect: None,
            checkpoints: CheckpointRead::Read { total: 2, count: 2, entries },
            fault: FaultRead::Read(fault),
            vendor_binary: vendor_fixture(),
            vendor_status: VendorBinaryStatus::Captured,
        };
        let lines = lines_of(&report, "D:/out/boyko-gpu-fault.bin");
        let text = lines.join("\n");
        assert!(lines.len() <= 16, "{} lines:\n{text}", lines.len());
        assert!(lines.iter().all(|l| l.len() <= REPORT_LINE_BYTES), "{text}");
        assert!(
            lines[0].starts_with("device lost at render (VK_ERROR_DEVICE_LOST); GPU crash report: level=checkpoints \
                                  pieces=fault,labels,markers,checkpoints profile="),
            "{text}"
        );
        assert!(
            lines[1].starts_with("slot 1 serial 9 declarator deferred passes 6: "),
            "the newest serial prints first:\n{text}"
        );
        assert!(lines[1].contains("last completed #2 ssao_atrous#0"), "{text}");
        assert!(lines[1].contains("in flight: #3 ssao_atrous#1, #4 ssao_atrous#2"), "{text}");
        assert!(lines[1].contains("not started: 2 (first #5 resolve)"), "{text}");
        assert!(lines[2].starts_with("slot 0 serial 8 declarator deferred passes 6: frame complete (7 scopes)"), "{text}");
        assert!(!text.contains("no GB scope was in flight"), "{text}");
        assert!(text.contains("checkpoints (2): TOP_OF_PIPE -> serial 9 begin #4 ssao_atrous#2; BOTTOM_OF_PIPE -> serial 9 end #2 ssao_atrous#0"), "{text}");
        assert!(text.contains("device fault: \"MMU fault\"; addresses 1; vendor records 1; vendor binary 64 B"), "{text}");
        assert!(text.contains("fault address 0: write-invalid 0x12345678 [0x12345000..0x12345fff]"), "{text}");
        assert!(text.contains("fault vendor record 0: code 0xc0 data 0xda7a \"MMU fault\""), "{text}");
        assert!(text.contains("vendor binary: 64 B, vendor 0x10de device 0x2c02 driver 0x99887766 uuid 00000000000000000000000000000000 -> D:/out/boyko-gpu-fault.bin"), "{text}");
        assert!(lines.last().is_some_and(|l| l.starts_with("note: scopes overlap")), "{text}");
    }

    /// A level without breadcrumbs, checkpoints or the fault query says so, piece by piece.
    #[test]
    fn the_report_names_every_missing_piece() {
        let report = DeviceLostReport {
            level: GpuCrashDiagLevel::Labels,
            pieces: piece::LABELS,
            frame_pieces: piece::LABELS,
            slots: [None, None],
            suspect: None,
            checkpoints: CheckpointRead::NotRequested,
            fault: FaultRead::NotRequested,
            vendor_binary: Vec::new(),
            vendor_status: VendorBinaryStatus::Unsupported,
        };
        let text = lines_of(&report, "").join("\n");
        assert!(text.contains("breadcrumbs not armed (BOYKO_GPU_CRASH_DIAG=markers arms them)"), "{text}");
        assert!(text.contains("checkpoints not armed"), "{text}");
        assert!(text.contains("device fault: not armed"), "{text}");
        assert!(text.contains("vendor binary: not captured: unsupported"), "{text}");
    }

    /// Every slot complete: the loss came from outside the frame (critique open question 2).
    #[test]
    fn a_loss_with_nothing_in_flight_points_outside_the_frame() {
        let report = DeviceLostReport {
            level: GpuCrashDiagLevel::Markers,
            pieces: piece::LABELS | piece::MARKERS,
            frame_pieces: piece::LABELS | piece::MARKERS,
            slots: [Some(snapshot(0, 4, region(4, 7, 7, true))), Some(snapshot(1, 5, region(5, 7, 7, true)))],
            suspect: None,
            checkpoints: CheckpointRead::NotRequested,
            fault: FaultRead::Unavailable,
            vendor_binary: Vec::new(),
            vendor_status: VendorBinaryStatus::Unsupported,
        };
        let text = lines_of(&report, "").join("\n");
        assert!(text.contains("no GB scope was in flight in either slot"), "{text}");
        assert!(text.contains("device fault: VK_EXT_device_fault is unavailable"), "{text}");
    }

    /// A frame whose fence wait returned `VK_SUCCESS` with its breadcrumbs incomplete — the spec
    /// allows that on a lost device — is counted and kept as the loss suspect, not asserted, and
    /// the copy survives the slot's reuse. A second incomplete frame keeps the FIRST suspect, the
    /// one nearest the loss (GB review r1 W1).
    ///
    /// RED: drop the suspect store in `verify_frame` and `suspect.get()` is `None`.
    #[test]
    fn an_incomplete_frame_after_a_successful_wait_is_kept_as_the_loss_suspect() {
        let ledger = Cell::new(CrumbLedger::default());
        let suspect = LossSuspect::new();
        let (a, _) = frame(ALL, 6, &[HostScope::PresentBlit], 7);
        let hung = region(7, 2, 4, false);
        let v = verify_frame(&a, 0, Some(&hung), &ledger, &suspect);
        assert_eq!(v, FrameVerdict { parity: true, complete: false, incomplete_while_alive: None });
        assert_eq!(ledger.get().frames_incomplete, 1);
        let kept = suspect.get().expect("the incomplete frame is kept as the suspect");
        assert_eq!((kept.fi, kept.serial, kept.cursor, kept.pass_count), (0, 7, 7, 6));
        assert!(kept.region == Some(hung), "the suspect keeps the words read at its verification");

        // The slot's shadow is reused two frames later; the suspect is a copy.
        a.begin(9, 0, 6);
        assert_eq!(suspect.get().map(|s| s.serial), Some(7));

        // The device stays lost: the next frame is incomplete too, and the first suspect stays.
        let (b, _) = frame(ALL, 6, &[HostScope::PresentBlit], 8);
        let v = verify_frame(&b, 1, Some(&region(6, 7, 7, true)), &ledger, &suspect);
        assert!(!v.complete && v.incomplete_while_alive.is_none(), "{v:?}");
        assert_eq!(suspect.get().map(|s| (s.fi, s.serial)), Some((0, 7)));
        assert_eq!(ledger.get().frames_incomplete, 2);
        assert_eq!(ledger.get().frames_checked, 2);
    }

    /// A frame that completes after the suspect proves the device alive: the suspect is cleared
    /// and named in the verdict — the G2 break `crumb_verify_slot` asserts in a debug build. A
    /// complete frame with no suspect held names nothing.
    ///
    /// RED: drop the clearing branch in `verify_frame` and the verdict carries `None`.
    #[test]
    fn a_later_complete_frame_proves_the_device_alive_and_clears_the_suspect() {
        let ledger = Cell::new(CrumbLedger::default());
        let suspect = LossSuspect::new();
        let (a, _) = frame(ALL, 6, &[], 7);
        assert!(!verify_frame(&a, 0, Some(&region(7, 2, 4, false)), &ledger, &suspect).complete);
        let (b, _) = frame(ALL, 6, &[], 8);
        let v = verify_frame(&b, 1, Some(&region(8, 6, 6, true)), &ledger, &suspect);
        assert_eq!(v, FrameVerdict { parity: true, complete: true, incomplete_while_alive: Some(7) });
        assert!(suspect.get().is_none(), "a live device clears the suspect");
        assert_eq!(ledger.get().frames_incomplete, 1, "the incomplete frame stays counted for G2");

        let (c, _) = frame(ALL, 6, &[], 9);
        let v = verify_frame(&c, 0, Some(&region(9, 6, 6, true)), &ledger, &suspect);
        assert_eq!(v.incomplete_while_alive, None);

        // Without markers nothing is ever incomplete, so nothing is ever suspected.
        let (d, _) = frame(piece::LABELS, 6, &[], 10);
        let v = verify_frame(&d, 1, None, &ledger, &suspect);
        assert!(v.complete && suspect.get().is_none(), "{v:?}");
    }

    /// The loss surfaces at the submit of frame S+2, after the wait for S returned `VK_SUCCESS`
    /// with S's breadcrumbs incomplete: slot 0 now holds S+2 and slot 1 holds S+1, neither
    /// started. The report still names S's in-flight set through the suspect, first, and does
    /// not send the reader outside the frame. Without the suspect the same slots print the
    /// "outside the frame" hint — the W1 failure. A slot that still holds the suspect's serial
    /// prints in its place, once.
    ///
    /// RED: skip the suspect in `write_lines` and the hint prints with no line naming serial 7.
    #[test]
    fn a_loss_after_a_successful_wait_still_names_the_suspect() {
        let (s, s1, s2) = (7u32, 8u32, 9u32);
        let hung = region(s, 3, 5, false);
        let report = |slots: [Option<SlotSnapshot>; FRAMES_IN_FLIGHT], suspect: Option<SlotSnapshot>| DeviceLostReport {
            level: GpuCrashDiagLevel::Markers,
            pieces: piece::FAULT | piece::LABELS | piece::MARKERS,
            frame_pieces: piece::LABELS | piece::MARKERS,
            slots,
            suspect,
            checkpoints: CheckpointRead::NotRequested,
            fault: FaultRead::Unavailable,
            vendor_binary: Vec::new(),
            vendor_status: VendorBinaryStatus::Unsupported,
        };
        // Slot 0's words still hold S's breadcrumbs; slot 1's hold S-1's (S+1 never began).
        let reused = [Some(snapshot(0, s2, hung)), Some(snapshot(1, s1, region(s - 1, 7, 7, true)))];

        let lines = lines_of(&report(reused, Some(snapshot(0, s, hung))), "");
        let text = lines.join("
");
        assert!(lines.len() <= 16 && lines.iter().all(|l| l.len() <= REPORT_LINE_BYTES), "{text}");
        assert!(
            lines[1].starts_with("slot 0 serial 7 declarator deferred passes 6 (suspect: incomplete after its fence wait): "),
            "the suspect prints first:
{text}"
        );
        assert!(lines[1].contains("in flight: #3 ssao_atrous#1, #4 ssao_atrous#2"), "{text}");
        assert!(lines[2].starts_with("slot 0 serial 9 declarator deferred passes 6: frame not started"), "{text}");
        assert!(lines[3].starts_with("slot 1 serial 8 declarator deferred passes 6: frame not started"), "{text}");
        assert!(!text.contains("no GB scope was in flight"), "{text}");

        let without = lines_of(&report(reused, None), "").join("
");
        assert!(without.contains("no GB scope was in flight in either slot"), "{without}");
        assert!(!without.contains("serial 7"), "{without}");

        // The loss surfaced before slot 0 was reused: the live slot prints S, the suspect does not.
        let held = [Some(snapshot(0, s, hung)), Some(snapshot(1, s1, region(s - 1, 7, 7, true)))];
        let text = lines_of(&report(held, Some(snapshot(0, s, hung))), "").join("
");
        assert_eq!(text.matches("serial 7 ").count(), 1, "{text}");
        assert!(!text.contains(SUSPECT_TAG), "{text}");
    }
}
