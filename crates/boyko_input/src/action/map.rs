//! [`InputMap<A>`] — `#[repr(u8)]` binding records in a flat arena,
//! `match`-dispatched (plan §6, Decision 6).
//!
//! A `match` over a `#[repr(u8)]` enum monomorphizes to a jump table — no
//! vtable, no box-per-binding (the leafwing anti-pattern). One flat `bindings`
//! arena plus per-action `(start, len)` ranges gives dense, cache-sequential
//! iteration during [`process_actions`](super::process::process_actions), with
//! zero per-action allocation.
//!
//! # I4 (this round)
//! `InputMap<A>` hand-implements `Resource` via the `TypeId` registry (plan
//! §7.1, C1) and gains [`InputMap::clone_arena`] so [`InputPlugin`] can insert a
//! copy of its default map. The contexts / priority-stack model (plan §6 V3)
//! lands in I5+. This module ships the single-context flat arena + the builder.
//!
//! [`InputPlugin`]: crate::plugin::InputPlugin

use core::marker::PhantomData;

use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::identifiers::primitives::ResourceId;

use crate::action::actionlike::Actionlike;
use crate::action::resource_id::id_for;
use crate::constants::MAX_CHORD_KEYS;
use crate::raw::keycode::{KeyCode, MouseButton};

/// A single button-like input source (a key or a mouse button), used as a
/// composite-axis leg.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputRef {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// How a digital (key-driven) 2D composite resolves its magnitude.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AxisMode {
    /// Diagonals are normalized to unit length (WASD default — no speed boost
    /// on diagonals).
    DigitalNormalized,
    /// Raw axis sum, unnormalized (diagonals are longer).
    DigitalRaw,
}

/// One binding for an action. `#[repr(u8)]` so dispatch is a jump table.
///
/// `Stick` is the reserved gamepad seam (plan §13): parsed and round-tripped by
/// the `.keys` format (I5), ignored at runtime in v1. It carries no fields yet
/// to keep the v1 surface minimal.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BindSpec {
    /// A single physical key (Button).
    Key(KeyCode),
    /// A single mouse button (Button).
    Mouse(MouseButton),
    /// A chord — all `keys[..len]` must be held together (e.g. `Ctrl+S`).
    /// `len <= MAX_CHORD_KEYS` (V9).
    Chord { keys: [KeyCode; MAX_CHORD_KEYS], len: u8 },
    /// A signed 1D axis from two opposing button legs, with a deadzone `dz`.
    Axis1 { neg: InputRef, pos: InputRef, dz: f32 },
    /// A 2D axis from four button legs (WASD), deadzone `dz`, resolution
    /// `mode`.
    Axis2 {
        up: InputRef,
        down: InputRef,
        left: InputRef,
        right: InputRef,
        dz: f32,
        mode: AxisMode,
    },
    /// Reserved gamepad-stick seam — ignored at runtime in v1 (plan §13).
    Stick,
    /// An explicit unbind (the action has this slot intentionally empty).
    None,
}

impl BindSpec {
    /// Builds a chord from a slice of keys (`len <= MAX_CHORD_KEYS`). Extra keys
    /// are truncated in release; a debug assert flags an over-long chord.
    pub fn chord(keys: &[KeyCode]) -> Self {
        debug_assert!(
            keys.len() <= MAX_CHORD_KEYS,
            "chord exceeds MAX_CHORD_KEYS ({MAX_CHORD_KEYS})"
        );
        let mut arr = [KeyCode::Unidentified(0); MAX_CHORD_KEYS];
        let len = keys.len().min(MAX_CHORD_KEYS);
        arr[..len].copy_from_slice(&keys[..len]);
        BindSpec::Chord {
            keys: arr,
            len: len as u8,
        }
    }

    /// The number of physical keys this binding requires simultaneously — the
    /// clash-resolution length metric (plan Decision 8). A bare key/mouse is 1,
    /// a chord is its `len`, composites and the reserved/none variants are 0
    /// (they do not participate in subset-clash suppression).
    #[inline]
    pub fn clash_len(&self) -> u8 {
        match self {
            BindSpec::Key(_) | BindSpec::Mouse(_) => 1,
            BindSpec::Chord { len, .. } => *len,
            BindSpec::Axis1 { .. }
            | BindSpec::Axis2 { .. }
            | BindSpec::Stick
            | BindSpec::None => 0,
        }
    }
}

/// Per-context clash-resolution strategy (plan §6 Decision 8 / V7).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClashStrategy {
    /// A binding whose key-set is a strict subset of another active binding's
    /// key-set is suppressed (`Ctrl+S` suppresses bare `S`).
    #[default]
    PrioritizeLongest,
    /// Every active binding fires; no suppression.
    AllowAll,
}

/// The action → bindings map (plan §6, Decision 6).
///
/// `bindings` is a flat arena allocated once at build; `ranges[A::index(a)]` is
/// the `(start, len)` slice of `bindings` for action `a`. Iteration during
/// processing is cache-sequential over the arena.
pub struct InputMap<A: Actionlike> {
    /// Flat arena of all bindings across all actions.
    bindings: Box<[BindSpec]>,
    /// `(start, len)` into `bindings`, indexed by `A::index`; `len == A::COUNT`.
    ranges: Box<[(u32, u32)]>,
    /// Clash-resolution strategy for this map.
    clash: ClashStrategy,
    // `fn() -> A`, not `A`: the marker owns no `A`, so `InputMap<A>` is
    // unconditionally `Send + Sync` (required by `Resource`) regardless of
    // whether `A: Send + Sync`.
    _pd: PhantomData<fn() -> A>,
}

impl<A: Actionlike> InputMap<A> {
    /// Starts building a map (cold path).
    #[inline]
    pub fn builder() -> InputMapBuilder<A> {
        InputMapBuilder::new()
    }

    /// The clash strategy.
    #[inline]
    pub fn clash(&self) -> ClashStrategy {
        self.clash
    }

    /// The bindings slice for action `a` (cache-sequential).
    #[inline]
    pub fn bindings_for(&self, a: A) -> &[BindSpec] {
        let (start, len) = self.ranges[a.index()];
        let start = start as usize;
        let len = len as usize;
        debug_assert!(
            start + len <= self.bindings.len(),
            "binding range out of arena bounds"
        );
        &self.bindings[start..start + len]
    }

    /// The bindings slice for a dense action index (used by the processor's
    /// index-driven loop, avoids reconstructing `A`).
    #[inline]
    pub fn bindings_at(&self, index: usize) -> &[BindSpec] {
        let (start, len) = self.ranges[index];
        let start = start as usize;
        let len = len as usize;
        &self.bindings[start..start + len]
    }

    /// Number of actions (== `A::COUNT`, the `ranges` length).
    #[inline]
    pub fn action_count(&self) -> usize {
        self.ranges.len()
    }

    /// The whole flat arena (for round-trip / inspection).
    #[inline]
    pub fn all_bindings(&self) -> &[BindSpec] {
        &self.bindings
    }

    /// Deep-copies the map's flat arena + ranges into a new owned `InputMap`
    /// (cold path). Used by [`InputPlugin::build`] to insert a copy of its
    /// default map without consuming the plugin's own template. `BindSpec` is
    /// `Copy`, so the copy is a flat `memcpy` of two boxed slices.
    ///
    /// [`InputPlugin::build`]: crate::plugin::InputPlugin
    pub fn clone_arena(&self) -> Self {
        Self {
            bindings: self.bindings.clone(),
            ranges: self.ranges.clone(),
            clash: self.clash,
            _pd: PhantomData,
        }
    }
}

// NOT `#[derive(Resource)]`: the generic-body `static` would collapse every `A`
// onto one id (rust#22991). Mint through the `TypeId`-keyed registry (plan §7.1).
impl<A: Actionlike> Resource for InputMap<A> {
    #[inline]
    fn resource_id() -> ResourceId {
        id_for::<Self>()
    }
}

/// Builds an [`InputMap`] from per-action binding lists (cold path).
///
/// Bindings accumulate in a per-action `Vec` during the cold build, then
/// [`InputMapBuilder::build`] flattens them into the single contiguous arena +
/// ranges. No allocation happens after `build`.
pub struct InputMapBuilder<A: Actionlike> {
    /// Per-action binding lists, indexed by `A::index`.
    per_action: Vec<Vec<BindSpec>>,
    clash: ClashStrategy,
    _pd: PhantomData<A>,
}

impl<A: Actionlike> InputMapBuilder<A> {
    /// A builder with every action initially unbound.
    pub fn new() -> Self {
        let mut per_action = Vec::with_capacity(A::COUNT);
        per_action.resize_with(A::COUNT, Vec::new);
        Self {
            per_action,
            clash: ClashStrategy::default(),
            _pd: PhantomData,
        }
    }

    /// Adds a binding for `action`.
    #[inline]
    pub fn bind(mut self, action: A, spec: BindSpec) -> Self {
        self.per_action[action.index()].push(spec);
        self
    }

    /// Preset: binds `action` to a WASD-style normalized 2D axis.
    #[inline]
    pub fn wasd(self, action: A) -> Self {
        self.bind(
            action,
            BindSpec::Axis2 {
                up: InputRef::Key(KeyCode::KeyW),
                down: InputRef::Key(KeyCode::KeyS),
                left: InputRef::Key(KeyCode::KeyA),
                right: InputRef::Key(KeyCode::KeyD),
                dz: 0.0,
                mode: AxisMode::DigitalNormalized,
            },
        )
    }

    /// Sets the clash strategy.
    #[inline]
    pub fn clash(mut self, s: ClashStrategy) -> Self {
        self.clash = s;
        self
    }

    /// Flattens into the final [`InputMap`] (one arena alloc + one ranges
    /// alloc). After this, no further allocation occurs on the process path.
    pub fn build(self) -> InputMap<A> {
        let total: usize = self.per_action.iter().map(Vec::len).sum();
        let mut bindings: Vec<BindSpec> = Vec::with_capacity(total);
        let mut ranges: Vec<(u32, u32)> = Vec::with_capacity(self.per_action.len());

        for list in &self.per_action {
            let start = bindings.len() as u32;
            bindings.extend_from_slice(list);
            let len = list.len() as u32;
            ranges.push((start, len));
        }

        InputMap {
            bindings: bindings.into_boxed_slice(),
            ranges: ranges.into_boxed_slice(),
            clash: self.clash,
            _pd: PhantomData,
        }
    }
}

impl<A: Actionlike> Default for InputMapBuilder<A> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
