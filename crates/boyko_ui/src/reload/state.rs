//! The hot-reload watch resource + the document-root list (P3 Decision 7 / 10).

use std::time::{Duration, Instant, SystemTime};

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::Resource;

use crate::text::report::UiParseReport;

/// The default poll interval: the watch system does at most one `metadata()`
/// syscall per this window, and a detected change settles for one further
/// window before being applied (Decision 7 / 8).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Inline capacity for the document-root list. Most `.ui` files have a single
/// root; this avoids a heap allocation for the common case (Decision 10).
const SMALL_ROOTS_INLINE: usize = 4;

/// A small inline-capacity list of root [`Entity`] ids — the scope anchor for a
/// document's reconcile (Decision 10). Spills to the heap past
/// [`SMALL_ROOTS_INLINE`]; spilling is a cold, rare case (a multi-root document).
///
/// `Entity` has no `Default`, so the inline buffer holds `Option<Entity>` and a
/// spill `Vec`. `as_slice` is built lazily into the spill `Vec` only when
/// spilled; the common 1-root path never touches the heap.
#[derive(Clone, Debug, Default)]
pub struct SmallRoots {
    inline: [Option<Entity>; SMALL_ROOTS_INLINE],
    len: usize,
    /// Populated only when the count exceeds the inline capacity.
    spill: Vec<Entity>,
}

impl SmallRoots {
    /// An empty root list.
    #[inline]
    pub fn new() -> Self {
        Self {
            inline: [None; SMALL_ROOTS_INLINE],
            len: 0,
            spill: Vec::new(),
        }
    }

    /// Appends a root id.
    #[inline]
    pub fn push(&mut self, e: Entity) {
        if self.spill.is_empty() && self.len < SMALL_ROOTS_INLINE {
            self.inline[self.len] = Some(e);
            self.len += 1;
        } else {
            // Spilled (or spilling now): migrate any inline entries on the first
            // spill so iteration is a single contiguous source.
            if self.spill.is_empty() {
                self.spill.extend(self.inline[..self.len].iter().flatten().copied());
            }
            self.spill.push(e);
        }
    }

    /// The roots as a slice. When unspilled, returns the inline prefix as a
    /// slice of `Entity` is not possible (`Option<Entity>`), so callers use
    /// [`SmallRoots::iter`] instead; this returns the spill slice (empty when
    /// unspilled). Prefer [`SmallRoots::to_vec`] / [`SmallRoots::iter`].
    #[inline]
    pub fn spilled_slice(&self) -> &[Entity] {
        &self.spill
    }

    /// Iterates the root ids in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        let (inline, spill): (&[Option<Entity>], &[Entity]) = if self.spill.is_empty() {
            (&self.inline[..self.len], &[])
        } else {
            (&[], &self.spill)
        };
        inline.iter().flatten().copied().chain(spill.iter().copied())
    }

    /// Collects the roots into a `Vec` (cold path: only the reconcile's scoped
    /// descent calls this, once per reload).
    #[inline]
    pub fn to_vec(&self) -> Vec<Entity> {
        self.iter().collect()
    }

    /// Number of roots.
    #[inline]
    pub fn len(&self) -> usize {
        if self.spill.is_empty() { self.len } else { self.spill.len() }
    }

    /// Whether the list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The hot-reload watch state (a normal `Resource`, Principle-0 storage).
///
/// Holds the watched path, the document's root scope, and the change-detection
/// state machine (mtime+size with a two-poll settle). `Send + Sync` (every field
/// is), so it is a valid `Resource`.
#[derive(Resource)]
pub struct UiHotReload {
    /// The `.ui` file watched for changes.
    pub path: &'static str,
    /// The document's root entity ids — the reconcile scope (Decision 10).
    pub(crate) doc_roots: SmallRoots,
    /// Last-applied modification time (`None` until the first successful load).
    pub(crate) last_mtime: Option<SystemTime>,
    /// Last-applied file size in bytes.
    pub(crate) last_size: u64,
    /// A detected-but-not-yet-settled `(mtime, size)` (Decision 8 settle buffer).
    pub(crate) pending: Option<(SystemTime, u64)>,
    /// When the watch system last polled (throttle anchor).
    pub(crate) last_poll: Instant,
    /// The minimum interval between polls.
    pub(crate) poll_interval: Duration,
    /// The most recent reconcile's lower-time recoverable report (errors +
    /// warnings discovered while re-parsing component bodies at patch/lowering
    /// time). Reachable observability — a host can inspect it after a reload — so
    /// lowering errors are never silently dropped (the lowering report must be
    /// reachable). Empty after a clean reload.
    pub last_report: UiParseReport,
}

impl UiHotReload {
    /// Constructs the watch state for `path` with the default poll interval.
    /// `last_poll` is seeded one interval in the past so the first scheduled run
    /// polls immediately.
    #[inline]
    pub fn new(path: &'static str) -> Self {
        Self::with_poll_interval(path, DEFAULT_POLL_INTERVAL)
    }

    /// Constructs the watch state with an explicit poll interval.
    pub fn with_poll_interval(path: &'static str, poll_interval: Duration) -> Self {
        Self {
            path,
            doc_roots: SmallRoots::new(),
            last_mtime: None,
            last_size: 0,
            pending: None,
            // Seed last_poll so the first run is not throttled.
            last_poll: Instant::now()
                .checked_sub(poll_interval)
                .unwrap_or_else(Instant::now),
            poll_interval,
            last_report: UiParseReport::default(),
        }
    }

    /// Records the document's root ids (called after the initial spawn).
    #[inline]
    pub(crate) fn set_doc_roots(&mut self, roots: SmallRoots) {
        self.doc_roots = roots;
    }

    /// Seeds the last-applied `(mtime, size)` from the file the initial load
    /// read, so the first poll does not re-load an unchanged file.
    #[inline]
    pub(crate) fn seed_signature(&mut self, mtime: Option<SystemTime>, size: u64) {
        self.last_mtime = mtime;
        self.last_size = size;
    }
}
