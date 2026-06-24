//! `boyko_sdf_math` — the analytic SDF edit-list field math + std430 data model,
//! extracted as a `#![no_std]` leaf so it can be the SINGLE source of truth shared
//! by two consumers that must NOT depend on each other:
//!
//! - `boyko_rhi_vulkan` — the GPU golden mirror of `shaders/sdf_editlist.hlsl`
//!   (rung 8/9/10/11 diff the GPU readback against the field folded here).
//! - `boyko_physics` (W5) — the CPU SDF-collision queries evaluate the SAME
//!   analytic field for narrowphase, with ZERO readback and ZERO graphics deps.
//!
//! Keeping the field math in a graphics-free leaf makes the dependency graph
//! acyclic (`boyko_physics → boyko_sdf_math`, NOT `boyko_physics →
//! boyko_rhi_vulkan`) and guarantees the CPU physics evaluator and the GPU golden
//! fold the bit-identical arithmetic.
//!
//! # The scene model (SDF doc §2)
//!
//! The scene is an ORDERED list of [`SdfEdit`]s: each a primitive (SPHERE or BOX)
//! combined into the accumulated field by a boolean op (union / subtraction /
//! intersection), optionally smoothed (polynomial smooth-min/-max when
//! `smoothness > 0`). [`sdf_edit_list`] folds the list per evaluation point; the
//! gradient ([`sdf_edit_list_normal`]) is a central difference of that fold. This
//! is the analytic base — NO grid cache / brick atlas (deferred).
//!
//! # `no_std`
//!
//! The crate uses ONLY `core` f32 ops (`abs`/`max`/`min`/`clamp`) + fixed
//! `[f32; N]` arrays — no `Vec`, no allocation, ZERO third-party deps. The one
//! exception is `sqrt` ([`v_len`]): IEEE `sqrt` is NOT in stable `core` (it lives
//! in `std`, or behind the nightly `core_intrinsics` feature). To keep the crate
//! compiling on the pinned stable toolchain WITHOUT a `libm`-style dependency,
//! the `sqrt` source is feature-gated (see the private `sqrt` shim in `lib.rs`):
//!
//! - default (stable): links `std` SOLELY for `f32::sqrt` (the crate is otherwise
//!   `core`-only — no `Vec`, no allocation, no graphics).
//! - `nightly` feature: strictly `#![no_std]`, using `core::intrinsics::sqrtf32`.
//!
//! Both lower to the SAME hardware `sqrtss`, so the result is bit-identical in
//! either mode. The rest of the math is a verbatim cut from
//! `boyko_rhi_vulkan::compute`: the float op order is byte-for-byte identical, so
//! the committed GPU goldens are unaffected (a reordered FMA could push a golden
//! past its `±2/255` tolerance — this must NOT be "cleaned up").

// Strictly `#![no_std]` only when the `sqrt` intrinsic is available (the `nightly`
// feature); otherwise `std` is linked solely for `f32::sqrt` (see the `sqrt` shim).
#![cfg_attr(feature = "nightly", no_std)]
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![cfg_attr(feature = "nightly", allow(internal_features))]

/// IEEE-correct `f32` square root, the ONE op the field math needs that stable
/// `core` does not provide. Lowers to the hardware `sqrtss` in both modes, so the
/// result is bit-identical: the `nightly` feature uses `core::intrinsics::sqrtf32`
/// (strict `no_std`); the default build uses `std`'s `f32::sqrt` (links `std`
/// for this op only).
#[inline]
fn sqrt(x: f32) -> f32 {
    // `core::intrinsics::sqrtf32` is a safe intrinsic (a pure, total function over
    // all `f32` bit patterns) and lowers to the same hardware `sqrtss` as `std`'s
    // `f32::sqrt` — no `unsafe`, ZERO-new-unsafe mandate upheld.
    #[cfg(feature = "nightly")]
    {
        core::intrinsics::sqrtf32(x)
    }
    #[cfg(not(feature = "nightly"))]
    {
        x.sqrt()
    }
}

pub mod brick;

/// SDF primitive kind discriminant. Matches the shader's `KIND_*` constants.
pub mod sdf_kind {
    /// A sphere primitive — `params.x` is the radius.
    pub const SPHERE: u32 = 0;
    /// An axis-aligned box primitive — `params.xyz` are the half-extents.
    pub const BOX: u32 = 1;
}

/// SDF boolean-op discriminant. Matches the shader's `OP_*` constants.
pub mod sdf_op {
    /// Union — `min(acc, d)` (or smooth-min when `smoothness > 0`).
    pub const UNION: u32 = 0;
    /// Subtraction — `max(acc, -d)` (or smooth-max when `smoothness > 0`).
    pub const SUBTRACT: u32 = 1;
    /// Intersection — `max(acc, d)` (or smooth-max when `smoothness > 0`).
    pub const INTERSECT: u32 = 2;
}

/// SDF image width (pixels) — matches the shader's `IMG_W`.
pub const SDF_IMG_W: u32 = 64;
/// SDF image height (pixels) — matches the shader's `IMG_H`.
pub const SDF_IMG_H: u32 = 64;

/// Central-difference step for the SDF gradient (the surface normal). Mirrors the
/// shader's `GRAD_H`; shared so the CPU evaluator and the GPU golden use the same
/// epsilon.
pub const SDF_GRAD_H: f32 = 0.0005;

/// Fixed capacity of the edit-list (the §S2 ceiling, scaled for the basic slice).
/// Matches the shader's `MAX_SDF_EDITS`.
pub const MAX_SDF_EDITS: usize = 16;

/// One SDF edit: a primitive + a uniform transform (center) + size (params) + a
/// boolean op + an optional smoothness factor.
///
/// `#[repr(C, align(16))]` so the Rust layout is byte-identical to the std430
/// structured-buffer element `shaders/sdf_editlist.hlsl` reads (the const-asserts
/// below pin offsets/size/align). `center`/`params` are `[f32; 4]` (the std430
/// `float4`) rather than `[f32; 3]` so the following `float4` starts at offset 16
/// without std430 inserting padding the Rust side would have to mirror — the two
/// layouts are then trivially identical.
///
/// Layout (mirrored in the shader):
/// - offset  0: `center` `[f32; 4]` — xyz = center/position, w unused
/// - offset 16: `params` `[f32; 4]` — xyz = radius / half-extents, w unused
/// - offset 32: `kind` `u32` — [`sdf_kind`]
/// - offset 36: `op` `u32` — [`sdf_op`]
/// - offset 40: `smoothness` `f32` — 0 = hard op; > 0 = smooth-min/-max blend k
/// - offset 44: `_pad` `u32` — keeps the size a 16-byte multiple
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct SdfEdit {
    /// xyz = primitive center/position; w unused.
    pub center: [f32; 4],
    /// xyz = radius (sphere) / half-extents (box); w unused.
    pub params: [f32; 4],
    /// Primitive kind ([`sdf_kind`]).
    pub kind: u32,
    /// Boolean op ([`sdf_op`]).
    pub op: u32,
    /// Smooth-blend radius (0 = hard op).
    pub smoothness: f32,
    /// Padding to a 16-byte multiple (mirrors the shader's `_pad` word).
    pub _pad: u32,
}

impl SdfEdit {
    /// A sphere edit at `center` with `radius`, combined by `op` with `smoothness`.
    #[inline]
    pub fn sphere(center: [f32; 3], radius: f32, op: u32, smoothness: f32) -> Self {
        Self {
            center: [center[0], center[1], center[2], 0.0],
            params: [radius, 0.0, 0.0, 0.0],
            kind: sdf_kind::SPHERE,
            op,
            smoothness,
            _pad: 0,
        }
    }

    /// A box edit at `center` with `half_extents`, combined by `op` with `smoothness`.
    #[inline]
    pub fn box_shape(center: [f32; 3], half_extents: [f32; 3], op: u32, smoothness: f32) -> Self {
        Self {
            center: [center[0], center[1], center[2], 0.0],
            params: [half_extents[0], half_extents[1], half_extents[2], 0.0],
            kind: sdf_kind::BOX,
            op,
            smoothness,
            _pad: 0,
        }
    }

    /// Packs a 16-bit PBR material id into the `center.w` FREE LANE (Render PBR MVP-2,
    /// Decision 4 — NO stride change). The field eval provably SKIPS `center.w` (the
    /// shader's `load_edit` reads only `center.xyz`), so this is determinism-NEUTRAL: the
    /// distance/depth golden + the `cpu_gpu_sdf_agreement` field stay byte-exact. The
    /// marcher reads the id back via `asuint(Buf[base + 3])` in a path the field never
    /// touches, and ATTRIBUTES the nearest-surface edit's material to the hit pixel.
    ///
    /// `material_id` is a 16-bit table index (the `R16`-width G-buffer carrier); the bits
    /// are stored verbatim as `f32::from_bits(id as u32)` (never interpreted as a float
    /// arithmetically — `center.w` is unread by every distance function).
    #[inline]
    pub fn with_material(mut self, material_id: u16) -> Self {
        self.center[3] = f32::from_bits(material_id as u32);
        self
    }
}

// ---- std430 / repr(C) layout contract (the §3.8 compile-time fingerprint) ----
//
// A mismatch between this Rust struct and the std430 element the shader reads is
// silent GPU corruption that NEITHER the validation layer NOR a golden diff would
// localize (the buffer is the right size; the bytes are read at a shifted offset).
// These const-asserts make any drift a BUILD ERROR. They mirror the shader's
// documented offsets exactly.
const _: () = assert!(
    core::mem::size_of::<SdfEdit>() == 48,
    "SdfEdit must be 48 bytes (std430 element the shader reads)"
);
const _: () = assert!(
    core::mem::align_of::<SdfEdit>() == 16,
    "SdfEdit must be 16-byte aligned (std430 struct alignment)"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, center) == 0,
    "SdfEdit::center must be at offset 0"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, params) == 16,
    "SdfEdit::params must be at offset 16"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, kind) == 32,
    "SdfEdit::kind must be at offset 32"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, op) == 36,
    "SdfEdit::op must be at offset 36"
);
const _: () = assert!(
    core::mem::offset_of!(SdfEdit, smoothness) == 40,
    "SdfEdit::smoothness must be at offset 40"
);

/// `size_of::<SdfEdit>() / 4` — the number of `u32` words one packed edit
/// occupies. Matches the shader's `SDF_EDIT_WORDS`.
pub const SDF_EDIT_WORDS: usize = core::mem::size_of::<SdfEdit>() / 4;

/// Word offset of the edit array (word 0 is `edit_count`, padded to 16 bytes so
/// the array starts 16-byte aligned). Matches the shader's `HEADER_BASE`.
pub const HEADER_BASE_WORDS: usize = 4;

// The shader hardcodes `SDF_EDIT_WORDS = 12u`; pin it so a layout change that
// desyncs the host encoder from the shader is a build error.
const _: () = assert!(SDF_EDIT_WORDS == 12, "SDF_EDIT_WORDS must equal the shader's 12u");

// ---- The unified edit-authority payload + per-edit AABB (brick campaign W0) ----

/// A conservative axis-aligned bound for one [`SdfEdit`]'s region of influence.
///
/// `min`/`max` are world-space corners. The bound is CONSERVATIVE: it covers the
/// primitive's extent expanded by the narrow-band half-width AND the edit's
/// smooth-blend radius, so an edit's analytic influence on the field can never
/// reach outside its own AABB (see [`edit_aabb`]). The brick classifier
/// ([`crate::brick::classify_brick`]) calls a brick EMPTY only when NO edit AABB
/// overlaps it — this conservatism is what makes that skip sound.
///
/// `#[repr(C)]` POD: two contiguous `[f32; 3]`, 24 bytes, 4-byte aligned. It is a
/// COLD trailing member of [`SdfEditField`]; it is NEVER interleaved into the hot
/// `[SdfEdit; MAX_SDF_EDITS]` the physics AVX2 kernel streams.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfEditAabb {
    /// The minimum (most-negative) world-space corner.
    pub min: [f32; 3],
    /// The maximum (most-positive) world-space corner.
    pub max: [f32; 3],
}

/// The unified SDF edit-authority payload: the ONE owner of the scene edit list
/// (principle 0). Every consumer — the physics narrowphase, the GPU encoder, and
/// the brick reference ([`crate::brick`]) — reads its edits from here.
///
/// # Layout contract (the W4 0%-regression keystone — INVIOLABLE)
///
/// `edits` is FIRST (offset 0) and BYTE-IDENTICAL to the standalone
/// `[SdfEdit; MAX_SDF_EDITS]` the physics scalar/AVX2 kernels stream today: the
/// hot array is unchanged, no new field is interleaved before or within it. The
/// COLD members (`count`/`gen`/`_pad`/`aabbs`) trail AFTER the hot array, so the
/// kernel's `&edits[..count]` slice is over the exact same bytes as before and the
/// generated code is asm-identical.
///
/// - `edits`   — the hot, kernel-streamed `[SdfEdit; 16]` (offset 0).
/// - `count`   — number of live edits (`<= MAX_SDF_EDITS`).
/// - `gen`     — a monotonically-bumped generation stamp for cache invalidation
///   (the brick atlas re-bakes when `gen` changes; M1 wiring).
/// - `_pad`    — keeps `aabbs` 16-byte aligned and the header a round size.
/// - `aabbs`   — the per-edit conservative bounds (`aabbs[i]` bounds `edits[i]`),
///   recomputed on every [`push`](SdfEditField::push). COLD: read only by the
///   brick classifier, never by the field-fold hot path.
/// - `prev_aabb` — the per-edit PREVIOUS bound: `prev_aabb[i]` is what `aabbs[i]`
///   was before the most recent mutation of edit `i` ([`set_edit`](Self::set_edit)
///   / [`move_edit`](Self::move_edit)). The M3 dirty set is the union over edits
///   where `aabbs[i] != prev_aabb[i]` of `aabbs[i] ∪ prev_aabb[i]` (the swept
///   old+new region). A fresh [`push`](Self::push) seeds `prev_aabb[i]` to a
///   DEGENERATE point at the edit center (inside `aabbs[i]`), so the new edit dirties
///   over its NEW AABB only — no ghost, yet it DOES bake incrementally.
///   [`clear_dirty`](Self::clear_dirty) re-snapshots it after a bake.
///   COLD: read only by the M3 dirty-set classifier, never by the field-fold hot
///   path.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SdfEditField {
    /// The hot edit list (only `edits[..count]` are live). MUST stay first +
    /// byte-identical to the physics kernel's `[SdfEdit; MAX_SDF_EDITS]`.
    pub edits: [SdfEdit; MAX_SDF_EDITS],
    /// Number of live edits in `edits` (`<= MAX_SDF_EDITS`).
    pub count: u32,
    /// Generation stamp; bumped whenever the edit list changes (cache key).
    ///
    /// Named `gen` (a Rust 2024 reserved keyword, escaped as `r#gen`) — the cold
    /// brick-cache invalidation generation, NOT a `Generator`.
    pub r#gen: u32,
    /// Padding so `aabbs` stays 16-byte aligned (the cold header is a round size).
    pub _pad: [u32; 2],
    /// Per-edit conservative bounds — `aabbs[i]` bounds `edits[i]`. COLD.
    pub aabbs: [SdfEditAabb; MAX_SDF_EDITS],
    /// Per-edit PREVIOUS bounds — `prev_aabb[i]` is `aabbs[i]`'s value before the
    /// last mutation of edit `i` (the M3 union-dirty rule's old-location source).
    /// COLD: read only by the M3 dirty-set classifier, never by the hot fold.
    pub prev_aabb: [SdfEditAabb; MAX_SDF_EDITS],
}

/// Brick occupancy class — the result of [`crate::brick::classify_brick`].
///
/// `#[repr(u8)]` so it round-trips a single byte into the eventual GPU brick-meta
/// column. `EmptyOutside`/`EmptyInside` bricks need NO voxel data (the marcher
/// skips/fills them analytically); only `Surface` bricks allocate a voxel block.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrickClass {
    /// Provably outside every solid: the analytic field is `> band_half` over the
    /// whole brick (no edit AABB overlaps it and the center samples positive).
    EmptyOutside = 0,
    /// Provably inside a solid: the analytic field is `< -band_half` over the whole
    /// brick (no edit AABB overlaps it and the center samples negative).
    EmptyInside = 1,
    /// The narrow band crosses (or may cross) this brick: it needs voxel data.
    Surface = 2,
}

/// The STORED narrow-band half-width (world units): the per-edit AABB skin in
/// [`edit_aabb`] AND the brick atlas's `R8_SNORM` snorm scale
/// ([`crate::brick::BAND_HALF_STORE`]) both use this value.
///
/// An edit influences the COMPOSED field out to roughly its smooth-blend radius
/// plus the band the brick atlas stores; expanding the AABB by `band_half`
/// guarantees the bound contains every point where this edit can move the field
/// inside `[-band_half, +band_half]`. Tied to the brick STORE scale so the
/// classifier's non-overlap test is conservative against the SAME band the fill
/// quantizes.
///
/// This is the wide STORE band, deliberately DISTINCT from the marcher's narrower
/// USABLE trust band ([`crate::brick::USABLE_BAND`] ≈ 0.4418) — the M0
/// conservative-lower-bound fix stores a wider band so a trusted point's
/// bracketing corners never saturate. See `brick.rs` for the trust-region
/// contract. Growing this value only GROWS the per-edit AABBs, so the EMPTY
/// classifier stays conservative (no EMPTY regression). Callers that bake at a
/// different band pass their own value to [`edit_aabb`].
pub const SDF_EDIT_BAND_HALF: f32 = 0.90;

/// Computes a CONSERVATIVE world-space AABB for one [`SdfEdit`] (brick campaign).
///
/// The bound is the primitive's raw extent (sphere: `center ± radius`; box:
/// `center ± half_extents`) EXPANDED on every face by `band_half + smoothness`:
///
/// - `band_half` covers the narrow band the brick atlas stores — a point that far
///   from the surface still has `|field| <= band_half`, so it must be inside the
///   bound for the classifier's non-overlap skip to stay sound.
/// - `smoothness` covers the smooth-blend reach: a `smin`/`smax` join pulls the
///   surface up to `k` away from the hard intersection, so the edit's influence
///   extends that far past its hard extent.
///
/// A SUBTRACT edit still expands its carver's influence bound (a subtraction can
/// CREATE surface wherever the carver reaches), so the op does not shrink the
/// AABB — the carver region is exactly where the field can change, hence where a
/// brick must NOT be declared empty.
#[inline]
pub fn edit_aabb(e: &SdfEdit, band_half: f32) -> SdfEditAabb {
    let c = [e.center[0], e.center[1], e.center[2]];
    // The raw primitive half-extent: the sphere's radius on every axis, or the
    // box's per-axis half-extents.
    let half = if e.kind == sdf_kind::BOX {
        [e.params[0], e.params[1], e.params[2]]
    } else {
        [e.params[0], e.params[0], e.params[0]]
    };
    // The conservative skin: the stored band plus the smooth-blend reach. A
    // SUBTRACT carver expands the SAME way — its reach is where it can carve.
    let skin = band_half + e.smoothness.max(0.0);
    SdfEditAabb {
        min: [c[0] - half[0] - skin, c[1] - half[1] - skin, c[2] - half[2] - skin],
        max: [c[0] + half[0] + skin, c[1] + half[1] + skin, c[2] + half[2] + skin],
    }
}

impl SdfEditField {
    /// The empty authority payload — no edits (`count == 0`, `gen == 0`).
    ///
    /// Every slot is seeded with an inert zero-radius union sphere at the origin
    /// (never read past `count`); the matching `aabbs` slots are degenerate points.
    #[inline]
    pub fn new() -> Self {
        let placeholder = SdfEdit::sphere([0.0, 0.0, 0.0], 0.0, sdf_op::UNION, 0.0);
        let placeholder_aabb = SdfEditAabb { min: [0.0; 3], max: [0.0; 3] };
        Self {
            edits: [placeholder; MAX_SDF_EDITS],
            count: 0,
            r#gen: 0,
            _pad: [0; 2],
            aabbs: [placeholder_aabb; MAX_SDF_EDITS],
            prev_aabb: [placeholder_aabb; MAX_SDF_EDITS],
        }
    }

    /// Appends one edit, recomputing its conservative `aabbs[i]` ([`edit_aabb`]
    /// at [`SDF_EDIT_BAND_HALF`]). Returns `false` (and ignores the edit) once the
    /// list is full ([`MAX_SDF_EDITS`]), matching the shader's edit-count clamp.
    ///
    /// A fresh push's PREVIOUS region is EMPTY (the slot was unused), so the new edit
    /// must be DIRTY over its NEW AABB only. `prev_aabb[i]` is seeded to a DEGENERATE
    /// point at the edit's center: it lies inside the new AABB, so the union-dirty
    /// rule (`aabbs[i] ∪ prev_aabb[i]`) collapses to `aabbs[i]` (the new region, no
    /// ghost at a non-existent old location), while the point differs from the
    /// extent-bearing `aabbs[i]`, so [`edit_is_dirty`](Self::edit_is_dirty) is TRUE
    /// and the pushed edit's tile bakes incrementally. The next
    /// [`clear_dirty`](Self::clear_dirty) re-snapshots `prev_aabb := aabbs`, so the
    /// push-dirty holds only until the first bake consumes it.
    ///
    /// Does NOT bump `gen` — the caller stamps a coherent batch with
    /// [`bump_gen`](Self::bump_gen) once after a run of pushes.
    #[inline]
    pub fn push(&mut self, e: SdfEdit) -> bool {
        let i = self.count as usize;
        if i >= MAX_SDF_EDITS {
            return false;
        }
        let aabb = edit_aabb(&e, SDF_EDIT_BAND_HALF);
        self.edits[i] = e;
        self.aabbs[i] = aabb;
        // A new edit's prior region is EMPTY: seed prev to a degenerate POINT at the
        // edit center. It is contained in `aabb` (band_half + smoothness >= band_half
        // > 0 skin makes `aabb` strictly enclose the center), so the swept union
        // `aabb ∪ prev` equals `aabb` — the new region only, no spurious dirty area.
        // The point also differs from the extent-bearing `aabb`, so `edit_is_dirty(i)`
        // is true and the pushed edit's tile bakes incrementally.
        let center = [e.center[0], e.center[1], e.center[2]];
        self.prev_aabb[i] = SdfEditAabb { min: center, max: center };
        self.count += 1;
        true
    }

    /// Replaces a LIVE edit in place (the dynamic-edit path), recording its OLD
    /// `aabbs[i]` into `prev_aabb[i]` BEFORE overwriting so the M3 union-dirty rule
    /// can sweep both the old and the new region. `i` MUST be `< count` (a debug
    /// assert traps an out-of-range index — overwriting a dead slot is a caller bug).
    ///
    /// Does NOT bump `gen` — the caller stamps a coherent batch with
    /// [`bump_gen`](Self::bump_gen) once after a run of edits. The brick atlas
    /// re-bakes ONLY the dirtied cells on the next `gen` change
    /// ([`crate::brick::dirty_world_aabb`]).
    #[inline]
    pub fn set_edit(&mut self, i: usize, e: SdfEdit) {
        debug_assert!(
            (i as u32) < self.count,
            "set_edit index must address a live edit (< count)"
        );
        // Record the OLD bound before overwriting — the union-dirty rule needs the
        // pre-mutation AABB to clear the ghost at the edit's previous location.
        self.prev_aabb[i] = self.aabbs[i];
        self.edits[i] = e;
        self.aabbs[i] = edit_aabb(&e, SDF_EDIT_BAND_HALF);
    }

    /// Moves a LIVE edit's center to `center` (the common dynamic case — a
    /// translating brush), keeping its primitive/op/smoothness. A thin
    /// convenience over [`set_edit`](Self::set_edit) that preserves the old AABB in
    /// `prev_aabb[i]` (so the swept old→new region dirties, leaving no ghost).
    #[inline]
    pub fn move_edit(&mut self, i: usize, center: [f32; 3]) {
        debug_assert!(
            (i as u32) < self.count,
            "move_edit index must address a live edit (< count)"
        );
        let mut e = self.edits[i];
        e.center = [center[0], center[1], center[2], e.center[3]];
        self.set_edit(i, e);
    }

    /// Re-snapshots `prev_aabb := aabbs` (clears the dirty set): after a bake has
    /// consumed the dirty region, the previous-AABB ledger is brought current so the
    /// NEXT mutation diffs against the freshly-baked state, not the stale pre-bake
    /// one. The render path calls this right after a `rebake_dirty`.
    #[inline]
    pub fn clear_dirty(&mut self) {
        self.prev_aabb = self.aabbs;
    }

    /// Whether edit `i` is dirty since the last [`clear_dirty`](Self::clear_dirty)
    /// (`aabbs[i] != prev_aabb[i]`). A live-index helper for the M3 dirty-set fold.
    #[inline]
    pub fn edit_is_dirty(&self, i: usize) -> bool {
        self.aabbs[i] != self.prev_aabb[i]
    }

    /// Bumps the generation stamp (wrapping) — the cache key the brick atlas keys
    /// its re-bake on. Call once after a coherent batch of [`push`](Self::push)es.
    #[inline]
    pub fn bump_gen(&mut self) {
        self.r#gen = self.r#gen.wrapping_add(1);
    }

    /// The live edit slice (`edits[..count]`) — exactly what the field math folds
    /// and the physics kernel streams (byte-identical to the legacy hot array).
    #[inline]
    pub fn edits(&self) -> &[SdfEdit] {
        &self.edits[..self.count as usize]
    }
}

impl Default for SdfEditField {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// `edits` MUST be first (offset 0) so the kernel's `&edits[..count]` slice is over
// the exact same bytes as the standalone `[SdfEdit; 16]` — the W4 hot-path
// byte-identity contract. A drift here is silent physics/golden corruption.
const _: () = assert!(
    core::mem::offset_of!(SdfEditField, edits) == 0,
    "SdfEditField::edits must be at offset 0 (byte-identical to the physics hot array)"
);
const _: () = assert!(
    core::mem::size_of::<[SdfEdit; MAX_SDF_EDITS]>()
        == MAX_SDF_EDITS * core::mem::size_of::<SdfEdit>(),
    "the hot edit array must be a dense [SdfEdit; 16] with no interleaved padding"
);

// ---- The edit-list field math (single source of truth, mirrors the shader) ----

const SDF_FAR: f32 = 1.0e9;

/// `a - b` — component-wise vector subtraction (mirrors the shader's `-`).
/// Exposed because the rung-8 single-sphere golden helpers in
/// `boyko_rhi_vulkan::compute` reuse it.
#[inline]
pub fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `length(a)` — the Euclidean norm (mirrors the shader's `length`). Exposed
/// because the rung-8 single-sphere golden helpers in
/// `boyko_rhi_vulkan::compute` reuse it.
#[inline]
pub fn v_len(a: [f32; 3]) -> f32 {
    sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
}

/// `dot(a, b)` — the 3-component dot product (mirrors the shader's `dot`).
/// Exposed because the golden lighting in `boyko_rhi_vulkan::compute` reuses it.
#[inline]
pub fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `a / length(a)` — the unit vector (mirrors the shader's `normalize`). Exposed
/// because the golden lighting + gradient normalization in
/// `boyko_rhi_vulkan::compute` reuse it.
///
/// # Degenerate (zero-length / non-finite) input
///
/// When `length(a)` is exactly zero — or non-finite — this returns
/// `[0.0, 0.0, 0.0]` instead of the `0.0 / 0.0 == NaN` the raw division would
/// produce, mirroring [`boyko_physics::math::Vec3::normalize`]'s zero-guard. The
/// only such input the field math feeds here is a central-difference gradient
/// ([`sdf_edit_list_normal`]) at a FIELD CRITICAL POINT (e.g. a query point
/// coincident with a primitive center under deep penetration, or a
/// subtract/smooth-blend interior saddle): the difference is `[0, 0, 0]`, so a
/// degenerate gradient now arrives at the physics narrowphase as `Vec3::ZERO`
/// — a usable sentinel its seam-skip test recognizes — rather than as a `NaN`
/// normal that would poison the solver.
///
/// # Golden-neutrality
///
/// The guard intercepts ONLY the exactly-zero / non-finite-length path; for every
/// non-degenerate input the arithmetic is byte-identical to the raw
/// `[a0/len, a1/len, a2/len]`. The committed rung-8/9/10/11 GPU goldens evaluate
/// this normal only at SURFACE hit points where `|grad| ≈ 1` (never at a
/// zero-gradient critical point), and the GPU shader's HLSL `normalize(0)` is
/// undefined there anyway — so the goldens never sample the guarded path and this
/// change is golden-neutral.
#[inline]
pub fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len = v_len(a);
    // Degenerate gradient (a field critical point): a zero or non-finite length
    // would make the division `NaN`; return ZERO so the physics seam-skip fires.
    // Non-degenerate inputs take the byte-identical division (golden-neutral).
    if len <= f32::MIN_POSITIVE || !len.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

#[inline]
fn v_abs(a: [f32; 3]) -> [f32; 3] {
    [a[0].abs(), a[1].abs(), a[2].abs()]
}

#[inline]
fn v_max0(a: [f32; 3]) -> [f32; 3] {
    [a[0].max(0.0), a[1].max(0.0), a[2].max(0.0)]
}

/// `length(p - c) - r` — the analytic sphere distance (mirrors `sd_sphere`).
#[inline]
pub fn sd_sphere(p: [f32; 3], c: [f32; 3], r: f32) -> f32 {
    v_len(v_sub(p, c)) - r
}

/// The exact IQ box distance for an AABB centered at `c` with half-extents `h`
/// (mirrors the shader's `sd_box`).
#[inline]
pub fn sd_box(p: [f32; 3], c: [f32; 3], h: [f32; 3]) -> f32 {
    let q = v_sub(v_abs(v_sub(p, c)), h);
    let outside = v_len(v_max0(q));
    let inside = q[0].max(q[1].max(q[2])).min(0.0);
    outside + inside
}

/// One edit's primitive distance at `p` (mirrors the shader's `edit_distance`).
#[inline]
pub fn edit_distance(e: &SdfEdit, p: [f32; 3]) -> f32 {
    let center = [e.center[0], e.center[1], e.center[2]];
    if e.kind == sdf_kind::BOX {
        sd_box(p, center, [e.params[0], e.params[1], e.params[2]])
    } else {
        sd_sphere(p, center, e.params[0])
    }
}

/// Polynomial smooth-min (IQ `smin`), mirroring the shader's `smin`.
#[inline]
pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    let hh = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    // lerp(b, a, hh) = b + (a - b) * hh
    (b + (a - b) * hh) - k * hh * (1.0 - hh)
}

/// Polynomial smooth-max (the De Morgan dual of [`smin`]), mirroring `smax`.
#[inline]
pub fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// Combines the accumulated distance `acc` with one edit's distance `d` under
/// `op` (hard when `k <= 0`, smooth when `k > 0`), mirroring the shader's
/// `combine`.
#[inline]
pub fn combine(acc: f32, d: f32, op: u32, k: f32) -> f32 {
    match op {
        x if x == sdf_op::SUBTRACT => {
            if k > 0.0 {
                smax(acc, -d, k)
            } else {
                acc.max(-d)
            }
        }
        x if x == sdf_op::INTERSECT => {
            if k > 0.0 {
                smax(acc, d, k)
            } else {
                acc.max(d)
            }
        }
        // UNION (and any unknown discriminant falls back to union, matching the
        // shader's `else` branch).
        _ => {
            if k > 0.0 {
                smin(acc, d, k)
            } else {
                acc.min(d)
            }
        }
    }
}

/// Evaluates the ordered edit-list field at `p` (the CSG result), folding the
/// edits in order exactly as the shader's `sdf` does. The first edit seeds the
/// accumulator hard; each later edit combines under its own op.
///
/// This is the single source of truth a future CPU physics evaluator reuses;
/// `edits.len()` is clamped to [`MAX_SDF_EDITS`] to match the shader's `min`.
pub fn sdf_edit_list(edits: &[SdfEdit], p: [f32; 3]) -> f32 {
    let n = edits.len().min(MAX_SDF_EDITS);
    let mut acc = SDF_FAR;
    for (i, e) in edits.iter().take(n).enumerate() {
        let d = edit_distance(e, p);
        if i == 0 {
            acc = d;
        } else {
            acc = combine(acc, d, e.op, e.smoothness);
        }
    }
    acc
}

/// Surface normal via central differences of [`sdf_edit_list`] (the gradient of
/// the WHOLE edit-list field), mirroring the shader's `sdf_normal`.
#[inline]
pub fn sdf_edit_list_normal(edits: &[SdfEdit], p: [f32; 3]) -> [f32; 3] {
    let h = SDF_GRAD_H;
    let n = [
        sdf_edit_list(edits, [p[0] + h, p[1], p[2]]) - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]),
        sdf_edit_list(edits, [p[0], p[1] + h, p[2]]) - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]),
        sdf_edit_list(edits, [p[0], p[1], p[2] + h]) - sdf_edit_list(edits, [p[0], p[1], p[2] - h]),
    ];
    v_normalize(n)
}

// The unit tests link `std` for the test harness; they run under the default
// (non-`nightly`) profile, where `std` is already linked for `f32::sqrt`.
#[cfg(test)]
mod tests {
    use super::*;

    /// The load-bearing C1 guard: a zero-length gradient (a field critical point)
    /// must normalize to ZERO, NOT to `[NaN, NaN, NaN]` — otherwise the physics
    /// seam-skip (`length_squared() < eps²`, which is `false` for NaN) never fires.
    #[test]
    fn v_normalize_zero_is_zero_not_nan() {
        let r = v_normalize([0.0, 0.0, 0.0]);
        assert_eq!(r, [0.0, 0.0, 0.0]);
        assert!(r.iter().all(|c| c.is_finite()));
    }

    /// A non-finite-length input (defensive) also collapses to ZERO rather than
    /// propagating NaN/Inf.
    #[test]
    fn v_normalize_non_finite_is_zero() {
        assert_eq!(v_normalize([f32::INFINITY, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(v_normalize([f32::NAN, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    }

    /// Golden-neutrality: for a NON-degenerate input the guarded `v_normalize`
    /// must return BIT-IDENTICAL bytes to the raw `[a0/len, a1/len, a2/len]` the
    /// committed GPU goldens were produced with (the guard must not perturb the
    /// arithmetic of the surface-hit path).
    #[test]
    fn v_normalize_nonzero_byte_identical_to_raw() {
        for a in [
            [1.0_f32, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [3.0, -4.0, 12.0],
            [0.0005, -0.0005, 0.0005],
            [-1.25, 2.5, -3.75],
        ] {
            let len = v_len(a);
            let raw = [a[0] / len, a[1] / len, a[2] / len];
            let guarded = v_normalize(a);
            // Compare raw bits: the guard must change nothing on this path.
            assert_eq!(guarded[0].to_bits(), raw[0].to_bits());
            assert_eq!(guarded[1].to_bits(), raw[1].to_bits());
            assert_eq!(guarded[2].to_bits(), raw[2].to_bits());
        }
    }

    /// At a field critical point — a query point coincident with a primitive
    /// center under deep penetration — the central-difference gradient is
    /// symmetric and folds to `[0, 0, 0]`, so the normal arrives as ZERO (the
    /// sentinel the physics seam-skip recognizes), never as NaN.
    #[test]
    fn sdf_edit_list_normal_at_sphere_center_is_zero() {
        let edits = [SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)];
        let n = sdf_edit_list_normal(&edits, [0.0, 0.0, 0.0]);
        assert_eq!(n, [0.0, 0.0, 0.0]);
        assert!(n.iter().all(|c| c.is_finite()));
    }
}
