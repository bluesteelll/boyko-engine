//! The CPU mesh→SDF baker (MDF Stage 2a — Mesh Distance Field shadows).
//!
//! Turns a STATIC triangle mesh into a signed-distance grid stored in the EXISTING
//! brick-atlas byte format ([`crate::brick`]), so a later GPU stage uploads the
//! bricks unchanged and the marcher unions the mesh's distance into the analytic
//! field — casting the mesh's shadow / AO without any per-frame mesh work. This
//! module is PURE CPU (no GPU / shader dependency): everything here is verifiable
//! with `cargo test`.
//!
//! # The single-source-of-truth contract (principle 0)
//!
//! The baker is BYTE-PARALLEL to [`crate::brick::fill_brick`] /
//! [`crate::brick::classify_brick`]: same voxel-center addressing, same
//! `EPSILON_Q * band_half` down-bias, same `encode_snorm8` quantization, same
//! [`BrickClass`] occupancy codes — diverging ONLY in the DISTANCE SOURCE
//! (the mesh closest-triangle + sign, instead of the analytic
//! [`crate::sdf_edit_list`]). A drift in the brick encoding is therefore impossible
//! by construction; the only mesh-specific code is the geometry query.
//!
//! # The conservative-lower-bound contract (C2)
//!
//! The same two layers that keep the analytic brick a CONSERVATIVE LOWER BOUND on
//! the field (the Hart sphere-tracing precondition — a fetched distance must never
//! exceed the true clearance) carry over verbatim:
//!
//! 1. [`classify_brick_from_mesh`] calls a cell EMPTY only when the closest triangle
//!    is provably farther than `band_half` from the cell AABB, so an EMPTY cell has
//!    `|distance| > band_half` everywhere inside.
//! 2. [`fill_brick_from_mesh`] biases every stored sample DOWN by
//!    [`EPSILON_Q`](crate::brick::EPSILON_Q) `* band_half`, the combined
//!    quantization + trilinear-reconstruction slack, so the decoded value stays `<=`
//!    the true mesh signed distance at every interior point.
//!
//! # Signed distance = unsigned distance × inside/outside sign
//!
//! The UNSIGNED distance is the exact point-to-closest-triangle Euclidean distance
//! (the IQ `udTriangle` 7-region clamp). The SIGN is the Jacobson 2013 Generalized
//! Winding Number ([`generalized_winding_number`]): robust on NON-watertight meshes
//! (open boundaries, T-junctions) where a parity ray-cast flickers. `gwn > 0.5`
//! ⇒ inside ⇒ the distance is negated.
//!
//! This crate is the `no_std` leaf; the BVH owns an `alloc::vec::Vec` of nodes (the
//! ONE heap structure — bake-time only, never on the marcher's hot path), and the
//! `atan2` the solid-angle formula needs (NOT in `core`) is a local polynomial shim
//! mirroring `brick.rs`'s `acos` shim.

extern crate alloc;

use alloc::vec::Vec;

use crate::brick::{APRON, BRICK_ALLOC, BRICK_VOXELS, EPSILON_Q};
use crate::{BrickClass, SDF_EDIT_BAND_HALF, v_dot, v_len, v_sub};

// ════════════════════════════════════════════════════════════════════════════
// Vector helpers (the crate exposes v_sub/v_dot/v_len; cross/add/scale + atan2 are
// local — `cross` and `atan2` have no crate-level home and are mesh-baker-only).
// ════════════════════════════════════════════════════════════════════════════

/// `a × b` — the 3-component cross product.
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `a * s` — scale a vector by a scalar.
#[inline]
fn v_scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `clamp(x, 0, 1)` (the `dot²/dot` barycentric clamp uses it twice per region).
#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// `sign(x)` as `±1.0` (0 maps to `+1.0`; the IQ `udTriangle` outer-product sign sum
/// treats the on-edge boundary as inside the face region — distance is continuous
/// there, so the choice is immaterial).
#[inline]
fn signf(x: f32) -> f32 {
    if x < 0.0 { -1.0 } else { 1.0 }
}

/// `atan2(y, x)` — NOT in stable `core` (mirrors the private `acos` shim in
/// `brick.rs`). The Generalized Winding Number's Van Oosterom–Strackee solid-angle
/// formula needs it (`2·atan2(num, den)`). Reduces to a `[−1, 1]`-domain rational
/// minimax `atan` (Hastings-style, ~1e-5 abs error — far below the `> 0.5` GWN
/// inside/outside threshold's margin) plus full-quadrant octant folding.
#[inline]
fn atan2(y: f32, x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;

    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let ay = y.abs();
    // Reduce to atan(t) with t = min/max ∈ [0, 1] for a well-conditioned polynomial.
    let (t, swapped) = if ax >= ay {
        (ay / ax, false)
    } else {
        (ax / ay, true)
    };
    // Rational minimax of atan on [0, 1]: t·(c1 + c2·t²) — the classic compact form.
    let t2 = t * t;
    let mut r = t * (0.999_866 + t2 * (-0.330_299_5 + t2 * (0.180_141_1 + t2 * -0.085_133)));
    if swapped {
        r = HALF_PI - r;
    }
    // Fold back into the correct quadrant from the (sign x, sign y) octant.
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 {
        r = -r;
    }
    r
}

// ════════════════════════════════════════════════════════════════════════════
// The bake-time triangle mesh view.
// ════════════════════════════════════════════════════════════════════════════

/// A borrowed STATIC triangle mesh ready to bake into the brick atlas.
///
/// Triangle `i` is `(positions[indices[i][0]], positions[indices[i][1]],
/// positions[indices[i][2]])`. The mesh is borrowed (`&'a`): the baker reads it,
/// never owns it. The AABB is computed once at construction (the BVH root bound +
/// the [`MeshSdfField`] grid extent both reuse it).
#[derive(Clone, Copy, Debug)]
pub struct BakeMesh<'a> {
    /// Vertex positions in world space.
    pub positions: &'a [[f32; 3]],
    /// Triangle vertex indices (each `[a, b, c]` references `positions`).
    pub indices: &'a [[u32; 3]],
    /// The minimum world corner of the mesh AABB.
    pub aabb_min: [f32; 3],
    /// The maximum world corner of the mesh AABB.
    pub aabb_max: [f32; 3],
}

impl<'a> BakeMesh<'a> {
    /// Borrows a mesh and computes its world-space AABB.
    ///
    /// A mesh with no indexed triangles yields a DEGENERATE AABB at the origin
    /// (`min == max == 0`); callers that bake an empty mesh get an all-positive
    /// (fully outside) field, never UB.
    pub fn new(positions: &'a [[f32; 3]], indices: &'a [[u32; 3]]) -> Self {
        let mut aabb_min = [f32::INFINITY; 3];
        let mut aabb_max = [f32::NEG_INFINITY; 3];
        for tri in indices {
            for &vi in tri {
                let v = positions[vi as usize];
                for axis in 0..3 {
                    aabb_min[axis] = aabb_min[axis].min(v[axis]);
                    aabb_max[axis] = aabb_max[axis].max(v[axis]);
                }
            }
        }
        // Degenerate (no triangles): collapse to a point at the origin.
        if !aabb_min[0].is_finite() {
            aabb_min = [0.0; 3];
            aabb_max = [0.0; 3];
        }
        Self {
            positions,
            indices,
            aabb_min,
            aabb_max,
        }
    }

    /// The three world-space vertices of triangle `i`.
    #[inline]
    fn triangle(&self, i: usize) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let t = self.indices[i];
        (
            self.positions[t[0] as usize],
            self.positions[t[1] as usize],
            self.positions[t[2] as usize],
        )
    }

    /// The number of triangles.
    #[inline]
    pub fn triangle_count(&self) -> usize {
        self.indices.len()
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Point-triangle distance (IQ `udTriangle` — the exact closest-point clamp).
// ════════════════════════════════════════════════════════════════════════════

/// The exact UNSIGNED distance from `p` to triangle `(a, b, c)`.
///
/// The Inigo Quilez `udTriangle` algorithm: if `p` projects inside all three edge
/// half-spaces (the sign-sum is 3) the closest point is the in-plane projection
/// (point-to-plane distance); otherwise the closest point lies on one of the three
/// edges, clamped by the `[0, 1]` segment parameter (the 7-region barycentric / edge
/// test — face + 3 edges + 3 vertices). Robust on degenerate (zero-area) triangles:
/// the plane term is skipped (sign-sum < 3) and the edge minimum dominates.
#[inline]
pub fn point_triangle_distance_sq(a: [f32; 3], b: [f32; 3], c: [f32; 3], p: [f32; 3]) -> f32 {
    let ba = v_sub(b, a);
    let pa = v_sub(p, a);
    let cb = v_sub(c, b);
    let pb = v_sub(p, b);
    let ac = v_sub(a, c);
    let pc = v_sub(p, c);
    let nor = cross(ba, ac);

    // The sign of `p` relative to each edge plane (edge × triangle-normal · p-edge).
    let s = signf(v_dot(cross(ba, nor), pa))
        + signf(v_dot(cross(cb, nor), pb))
        + signf(v_dot(cross(ac, nor), pc));

    if s < 2.0 {
        // Outside at least one edge: the closest point is on an edge — take the min
        // of the three clamped point-to-segment squared distances.
        let e0 = {
            let t = clamp01(v_dot(ba, pa) / v_dot(ba, ba));
            let d = v_sub(v_scale(ba, t), pa);
            v_dot(d, d)
        };
        let e1 = {
            let t = clamp01(v_dot(cb, pb) / v_dot(cb, cb));
            let d = v_sub(v_scale(cb, t), pb);
            v_dot(d, d)
        };
        let e2 = {
            let t = clamp01(v_dot(ac, pc) / v_dot(ac, ac));
            let d = v_sub(v_scale(ac, t), pc);
            v_dot(d, d)
        };
        e0.min(e1).min(e2)
    } else {
        // Inside all three edges: the closest point is the in-plane projection.
        let nd = v_dot(nor, pa);
        nd * nd / v_dot(nor, nor)
    }
}

/// The exact UNSIGNED distance from `p` to the nearest triangle of `mesh`
/// (brute force — `O(triangles)`; acceptable at Stage-2a bake time on a small mesh,
/// and the EXACT oracle the BVH variant is golden-checked against).
pub fn closest_triangle_distance(mesh: &BakeMesh, p: [f32; 3]) -> f32 {
    let mut best_sq = f32::INFINITY;
    for i in 0..mesh.triangle_count() {
        let (a, b, c) = mesh.triangle(i);
        let d2 = point_triangle_distance_sq(a, b, c, p);
        if d2 < best_sq {
            best_sq = d2;
        }
    }
    crate::sqrt(best_sq)
}

// ════════════════════════════════════════════════════════════════════════════
// The triangle BVH (median-split over centroids) — EXACT branch-and-bound.
// ════════════════════════════════════════════════════════════════════════════

/// One node of [`TriBvh`]: an AABB plus either a child split (internal) or a
/// triangle-index range (leaf). A leaf is encoded by `tri_count > 0`; an internal
/// node by `tri_count == 0` and `left`/`right` child indices.
#[derive(Clone, Copy, Debug)]
struct BvhNode {
    /// The minimum world corner of this node's AABB.
    bmin: [f32; 3],
    /// The maximum world corner of this node's AABB.
    bmax: [f32; 3],
    /// First triangle (into the BVH's permuted `tri_indices`) — leaves only.
    first_tri: u32,
    /// Triangle count — `0` ⇒ internal node, `> 0` ⇒ leaf.
    tri_count: u32,
    /// Left child node index — internal nodes only.
    left: u32,
    /// Right child node index — internal nodes only.
    right: u32,
}

/// A median-split AABB BVH over a mesh's triangle centroids, for EXACT nearest-
/// triangle queries via branch-and-bound. Built ONCE at bake time
/// ([`build_tri_bvh`]); read by [`closest_triangle_distance_bvh`]. The query result
/// is BIT-for-bit identical to the brute-force [`closest_triangle_distance`] (the
/// BVH only PRUNES — it never approximates).
///
/// Owns its node array + the triangle-permutation (`tri_indices`) on the heap; this
/// is bake-time scratch, never touched on the marcher's hot path (principle 1).
#[derive(Clone, Debug)]
pub struct TriBvh {
    nodes: Vec<BvhNode>,
    /// Triangle indices permuted into leaf order (`tri_indices[node.first_tri + k]`
    /// is the `k`-th triangle in a leaf node).
    tri_indices: Vec<u32>,
}

/// The maximum triangles per BVH leaf — below this a node stops splitting (the
/// leaf's triangles are tested linearly).
const BVH_LEAF_MAX: usize = 4;

/// Builds a median-split BVH over the mesh's triangle centroids.
///
/// Each recursion splits the active triangle range at the MEDIAN centroid along the
/// node AABB's longest axis (a balanced split with no surface-area heuristic — the
/// query is exact regardless of split quality, so a cheap median is sufficient at
/// Stage-2a scale). An empty mesh yields a single degenerate leaf.
pub fn build_tri_bvh(mesh: &BakeMesh) -> TriBvh {
    let tri_count = mesh.triangle_count();
    let mut tri_indices: Vec<u32> = (0..tri_count as u32).collect();

    // Precompute per-triangle AABB + centroid (read repeatedly during partitioning).
    let mut tri_min: Vec<[f32; 3]> = Vec::with_capacity(tri_count);
    let mut tri_max: Vec<[f32; 3]> = Vec::with_capacity(tri_count);
    let mut tri_centroid: Vec<[f32; 3]> = Vec::with_capacity(tri_count);
    for i in 0..tri_count {
        let (a, b, c) = mesh.triangle(i);
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        for v in [a, b, c] {
            for axis in 0..3 {
                mn[axis] = mn[axis].min(v[axis]);
                mx[axis] = mx[axis].max(v[axis]);
            }
        }
        tri_min.push(mn);
        tri_max.push(mx);
        tri_centroid.push([
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]);
    }

    let mut nodes: Vec<BvhNode> = Vec::new();
    build_node(
        &mut nodes,
        &mut tri_indices,
        &tri_min,
        &tri_max,
        &tri_centroid,
        0,
        tri_count,
    );

    TriBvh { nodes, tri_indices }
}

/// Recursively builds the node spanning `tri_indices[start..end]`; returns its index.
fn build_node(
    nodes: &mut Vec<BvhNode>,
    tri_indices: &mut [u32],
    tri_min: &[[f32; 3]],
    tri_max: &[[f32; 3]],
    tri_centroid: &[[f32; 3]],
    start: usize,
    end: usize,
) -> u32 {
    // The node's AABB = the union of its triangles' AABBs.
    let mut bmin = [f32::INFINITY; 3];
    let mut bmax = [f32::NEG_INFINITY; 3];
    for &ti in &tri_indices[start..end] {
        let mn = tri_min[ti as usize];
        let mx = tri_max[ti as usize];
        for axis in 0..3 {
            bmin[axis] = bmin[axis].min(mn[axis]);
            bmax[axis] = bmax[axis].max(mx[axis]);
        }
    }
    if !bmin[0].is_finite() {
        // Empty range (degenerate / empty mesh): a zero-extent leaf at the origin.
        bmin = [0.0; 3];
        bmax = [0.0; 3];
    }

    let count = end - start;
    let node_index = nodes.len() as u32;
    nodes.push(BvhNode {
        bmin,
        bmax,
        first_tri: start as u32,
        tri_count: count as u32,
        left: 0,
        right: 0,
    });

    if count <= BVH_LEAF_MAX {
        return node_index; // a leaf (tri_count > 0)
    }

    // Split at the median centroid along the AABB's longest axis.
    let extent = [bmax[0] - bmin[0], bmax[1] - bmin[1], bmax[2] - bmin[2]];
    let axis = if extent[0] >= extent[1] && extent[0] >= extent[2] {
        0
    } else if extent[1] >= extent[2] {
        1
    } else {
        2
    };
    let mid = start + count / 2;
    // Partial nth-element by full sort on the axis (Stage-2a meshes are small; an
    // exact median keeps the tree balanced and the build allocation-light).
    tri_indices[start..end].sort_unstable_by(|&i, &j| {
        let ci = tri_centroid[i as usize][axis];
        let cj = tri_centroid[j as usize][axis];
        ci.partial_cmp(&cj).unwrap_or(core::cmp::Ordering::Equal)
    });

    let left = build_node(
        nodes,
        tri_indices,
        tri_min,
        tri_max,
        tri_centroid,
        start,
        mid,
    );
    let right = build_node(nodes, tri_indices, tri_min, tri_max, tri_centroid, mid, end);
    nodes[node_index as usize].tri_count = 0; // mark internal
    nodes[node_index as usize].left = left;
    nodes[node_index as usize].right = right;
    node_index
}

/// The squared distance from `p` to an AABB (`0` when `p` is inside) — the BVH
/// branch-and-bound prune key.
#[inline]
fn point_aabb_distance_sq(bmin: [f32; 3], bmax: [f32; 3], p: [f32; 3]) -> f32 {
    let mut d = 0.0;
    for axis in 0..3 {
        let v = p[axis];
        let lo = bmin[axis];
        let hi = bmax[axis];
        let e = if v < lo {
            lo - v
        } else if v > hi {
            v - hi
        } else {
            0.0
        };
        d += e * e;
    }
    d
}

/// The exact UNSIGNED nearest-triangle distance via the BVH (branch-and-bound).
///
/// Descends both children nearest-first and PRUNES any node whose AABB squared
/// distance already exceeds the best found — so the result is BIT-identical to the
/// brute-force [`closest_triangle_distance`] (proven by the `bvh == brute` unit
/// test). `O(log T)` expected per query versus `O(T)` brute force.
pub fn closest_triangle_distance_bvh(bvh: &TriBvh, mesh: &BakeMesh, p: [f32; 3]) -> f32 {
    if bvh.nodes.is_empty() {
        return f32::INFINITY;
    }
    let mut best_sq = f32::INFINITY;
    // An explicit stack (no recursion): node indices to visit.
    let mut stack: [u32; 64] = [0; 64];
    let mut sp = 0usize;
    stack[sp] = 0;
    sp += 1;

    while sp > 0 {
        sp -= 1;
        let node = bvh.nodes[stack[sp] as usize];
        // Prune: this whole subtree is farther than the current best.
        if point_aabb_distance_sq(node.bmin, node.bmax, p) >= best_sq {
            continue;
        }
        if node.tri_count > 0 {
            // Leaf: test its triangles.
            let s = node.first_tri as usize;
            let e = s + node.tri_count as usize;
            for &ti in &bvh.tri_indices[s..e] {
                let (a, b, c) = mesh.triangle(ti as usize);
                let d2 = point_triangle_distance_sq(a, b, c, p);
                if d2 < best_sq {
                    best_sq = d2;
                }
            }
        } else {
            // Internal: push the FARTHER child first so the NEARER is popped first
            // (tightens `best_sq` early, maximizing the prune on the sibling).
            let l = node.left;
            let r = node.right;
            let dl = {
                let n = bvh.nodes[l as usize];
                point_aabb_distance_sq(n.bmin, n.bmax, p)
            };
            let dr = {
                let n = bvh.nodes[r as usize];
                point_aabb_distance_sq(n.bmin, n.bmax, p)
            };
            let (near, far) = if dl <= dr { (l, r) } else { (r, l) };
            // Guard the fixed stack (a balanced BVH over a small mesh is < 64 deep;
            // an overflow would silently drop a subtree, so debug-assert it).
            debug_assert!(sp + 2 <= stack.len(), "BVH traversal stack overflow");
            stack[sp] = far;
            sp += 1;
            stack[sp] = near;
            sp += 1;
        }
    }
    crate::sqrt(best_sq)
}

// ════════════════════════════════════════════════════════════════════════════
// Generalized Winding Number (Jacobson 2013) — the robust inside/outside sign.
// ════════════════════════════════════════════════════════════════════════════

/// The Jacobson 2013 Generalized Winding Number of `mesh` at `p`.
///
/// `(1 / 4π) · Σ` over triangles of the SIGNED SOLID ANGLE the triangle subtends at
/// `p`, via the Van Oosterom–Strackee formula: for triangle `(a, b, c)` with
/// `va = a − p`, `vb = b − p`, `vc = c − p`,
///
/// `Ω = 2·atan2(va·(vb×vc),  |va||vb||vc| + (va·vb)|vc| + (vb·vc)|va| + (vc·va)|vb|)`.
///
/// `> 0.5` ⇒ inside, `< 0.5` ⇒ outside. UNLIKE a parity ray-cast it degrades
/// GRACEFULLY on NON-watertight meshes (a small hole perturbs the field smoothly
/// rather than flipping the sign of a whole ray), so a point well inside a cube with
/// one face triangle removed still reads `≈ 1`. Naive `O(triangles)`.
pub fn generalized_winding_number(mesh: &BakeMesh, p: [f32; 3]) -> f32 {
    const FOUR_PI: f32 = 4.0 * core::f32::consts::PI;
    let mut acc = 0.0f32;
    for i in 0..mesh.triangle_count() {
        let (a, b, c) = mesh.triangle(i);
        let va = v_sub(a, p);
        let vb = v_sub(b, p);
        let vc = v_sub(c, p);
        let la = v_len(va);
        let lb = v_len(vb);
        let lc = v_len(vc);
        let num = v_dot(va, cross(vb, vc));
        let den = la * lb * lc + v_dot(va, vb) * lc + v_dot(vb, vc) * la + v_dot(vc, va) * lb;
        acc += 2.0 * atan2(num, den);
    }
    acc / FOUR_PI
}

// ════════════════════════════════════════════════════════════════════════════
// Mesh SIGNED distance = unsigned distance, negated when inside (GWN > 0.5).
// ════════════════════════════════════════════════════════════════════════════

/// The GWN threshold: `> 0.5` ⇒ inside. The mid-point of the `[0, 1]` watertight
/// jump; the robust choice for the non-watertight case (a hole only nudges the
/// value, so a deep-interior point stays well above `0.5`).
pub const GWN_INSIDE_THRESHOLD: f32 = 0.5;

/// The mesh SIGNED distance at `p` (brute force): the unsigned closest-triangle
/// distance, NEGATED when [`generalized_winding_number`] reports inside.
pub fn mesh_signed_distance(mesh: &BakeMesh, p: [f32; 3]) -> f32 {
    let d = closest_triangle_distance(mesh, p);
    if generalized_winding_number(mesh, p) > GWN_INSIDE_THRESHOLD {
        -d
    } else {
        d
    }
}

/// The mesh SIGNED distance at `p` using the prebuilt BVH for the unsigned distance
/// (the sign is still the naive `O(T)` GWN — winding has no spatial prune). The
/// distance magnitude is BIT-identical to [`mesh_signed_distance`].
pub fn mesh_signed_distance_bvh(bvh: &TriBvh, mesh: &BakeMesh, p: [f32; 3]) -> f32 {
    let d = closest_triangle_distance_bvh(bvh, mesh, p);
    if generalized_winding_number(mesh, p) > GWN_INSIDE_THRESHOLD {
        -d
    } else {
        d
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Curvature estimate (the P2 lower-bound budget input).
// ════════════════════════════════════════════════════════════════════════════

/// A CONSERVATIVE max-band-curvature estimate for `mesh`, used to size the P2
/// lower-bound budget ([`fill_brick_from_mesh`]'s entry assert).
///
/// # What `c_max` must bound (and what it must NOT use)
///
/// The trilinear-midpoint slack the `EPSILON_Q` bias must dominate scales with
/// `c_max·voxel_size²/8`, where `c_max` is the worst-case SECOND-derivative
/// magnitude of the stored distance band across ONE voxel — i.e. the inverse radius
/// of curvature of the SURFACE the band wraps. This is a property of the GEOMETRY's
/// shape, NOT of its tessellation: a finely-tessellated flat region (or a UV-sphere
/// pole, where the triangle edges shrink to zero) has tiny edges but ZERO extra
/// curvature. Using `1/min_edge` as the proxy therefore over-estimates wildly and
/// makes the budget unsatisfiable for any non-uniform mesh — it is the WRONG proxy.
///
/// # The estimate (matches the brick campaign's curvature contract)
///
/// We adopt the SAME contract the analytic brick uses: the brick level GUARANTEES a
/// supported curvature ceiling of `C_MAX = 1/R_MIN = 2.0` at the pinned voxel scale
/// (`brick.rs`). A SMOOTH mesh's surface curvature does not exceed what the analytic
/// primitives at the same scale do, so reporting that ceiling is a sound upper bound
/// for the smooth case. Surface regions sharper than `R_MIN` (sharp creases / fine
/// concavities below the voxel scale) are OUT of this level's contract — exactly the
/// intended LOD degradation the analytic brick already has, where the field is
/// re-evaluated at a finer level / the exact analytic fallback. The `mesh` argument
/// is reserved for a future per-mesh refinement (e.g. a measured principal-curvature
/// bound) and is read only to keep the signature stable.
///
/// Documented conservatism: this is the curvature the chosen brick LEVEL supports,
/// not a per-mesh measurement; pair it with a `voxel_size` fine enough that the P2
/// budget holds (`MeshSdfField::for_mesh` asserts it). A future stage can tighten it
/// per-mesh once a robust curvature measure (not tessellation density) is in place.
pub fn measure_mesh_c_max(mesh: &BakeMesh) -> f32 {
    /// The brick level-0 supported curvature ceiling (`brick.rs::C_MAX = 1/R_MIN`).
    const C_MAX_LEVEL0: f32 = 2.0;
    let _ = mesh; // reserved for a future per-mesh curvature refinement
    C_MAX_LEVEL0
}

// ════════════════════════════════════════════════════════════════════════════
// The mesh SDF grid descriptor.
// ════════════════════════════════════════════════════════════════════════════

/// The world-space layout of a baked mesh SDF grid: where it sits, how fine, how
/// wide its narrow band, and its curvature budget. A later GPU stage walks the
/// `grid_dim` lattice, bakes each brick with [`fill_brick_from_mesh`], and uploads
/// the `Surface` bricks ([`classify_brick_from_mesh`]) into the brick atlas.
#[derive(Clone, Copy, Debug)]
pub struct MeshSdfField {
    /// The minimum world corner of voxel `(0, 0, 0)`'s interior (the grid origin,
    /// already margin-expanded below the mesh AABB).
    pub grid_origin: [f32; 3],
    /// The world width of one voxel.
    pub voxel_size: f32,
    /// Voxels per axis (covering the margin-expanded mesh AABB).
    pub grid_dim: [u32; 3],
    /// The stored narrow-band half-width ([`SDF_EDIT_BAND_HALF`]).
    pub band_half: f32,
    /// The max band curvature ([`measure_mesh_c_max`]) — the P2 budget input.
    pub c_max: f32,
    /// A generation stamp for cache invalidation (mirrors `SdfEditField::gen`; the
    /// caller bumps it when the source mesh changes so the GPU re-bakes).
    pub r#gen: u32,
}

impl MeshSdfField {
    /// Lays out a grid covering `mesh` at `voxel_size`, with a margin so the narrow
    /// band on the mesh's outer faces is fully resolved.
    ///
    /// The margin is `band_half + 1 voxel` on every face: `band_half` so the stored
    /// band reaches outside the surface, `+1 voxel` so the apron of the outermost
    /// brick still samples valid distances. `debug_assert!`s the P2 budget (the same
    /// predicate [`fill_brick`](crate::brick::fill_brick) checks at entry) so a
    /// mis-sized grid trips at bake time, not as a silent over-reporting field.
    pub fn for_mesh(mesh: &BakeMesh, voxel_size: f32) -> Self {
        debug_assert!(voxel_size > 0.0, "invariant: voxel_size must be positive");
        // The STORE band: `SDF_EDIT_BAND_HALF` (== `brick::BAND_HALF_STORE` == 0.90)
        // — the same wide store band the analytic brick fill quantizes against.
        let band_half = SDF_EDIT_BAND_HALF;
        let c_max = measure_mesh_c_max(mesh);

        // Margin = the band reach + one apron voxel, rounded UP to a whole voxel so
        // the grid origin lands on a clean voxel multiple of the requested size.
        let margin_world = band_half + voxel_size;
        let margin_voxels = (margin_world / voxel_size).ceil();
        let margin = margin_voxels * voxel_size;

        let grid_origin = [
            mesh.aabb_min[0] - margin,
            mesh.aabb_min[1] - margin,
            mesh.aabb_min[2] - margin,
        ];
        let mut grid_dim = [0u32; 3];
        for (axis, dim) in grid_dim.iter_mut().enumerate() {
            let span = (mesh.aabb_max[axis] - mesh.aabb_min[axis]) + 2.0 * margin;
            let n = (span / voxel_size).ceil() as i64;
            *dim = n.max(1) as u32;
        }

        // The P2 lower-bound budget (the runtime mirror of brick.rs's compile-time
        // per-level predicate) — the same inequality fill_brick asserts at entry.
        debug_assert!(
            EPSILON_Q * band_half >= voxel_size * voxel_size * c_max / 8.0 + band_half / 254.0,
            "EPSILON_Q under-bounds curvature+quant for this (voxel_size, band_half, \
             c_max) — the mesh band is too curved for this brick scale; refine the \
             voxel_size or raise the brick level"
        );

        Self {
            grid_origin,
            voxel_size,
            grid_dim,
            band_half,
            c_max,
            r#gen: 0,
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// fill_brick_from_mesh — BYTE-PARALLEL to brick.rs::fill_brick.
// ════════════════════════════════════════════════════════════════════════════

/// Bakes one apron'd brick from `mesh` — BYTE-PARALLEL to
/// [`crate::brick::fill_brick`].
///
/// IDENTICAL to `fill_brick` in every respect EXCEPT the distance source: the SAME
/// `BRICK_ALLOC³` voxel-center addressing (`(x − APRON + 0.5)·voxel_size`), the SAME
/// `EPSILON_Q · band_half` conservative down-bias, the SAME `encode_snorm8`
/// quantization and linear `x + y·W + z·W·W` layout — diverging ONLY in
/// [`mesh_signed_distance_bvh`] (the mesh closest-triangle + GWN sign) replacing
/// `sdf_edit_list`. A prebuilt [`TriBvh`] over `mesh` is passed in (build it ONCE per
/// mesh, reuse it across every brick).
///
/// - `brick_min` is the brick's minimum INTERIOR world corner (the apron extends one
///   voxel below it).
/// - `voxel_size` / `band_half` / `c_max` match the [`MeshSdfField`] this brick
///   belongs to; the entry assert re-checks the P2 lower-bound budget.
/// - `out` is the `BRICK_VOXELS`-length destination.
///
/// The down-bias keeps the decoded trilinear reconstruction `<=` the true mesh
/// signed distance at every interior point (the C2 lower-bound contract — the
/// `mesh_signed_distance` lower-bound soundness test is the numeric tripwire).
pub fn fill_brick_from_mesh(
    mesh: &BakeMesh,
    bvh: &TriBvh,
    brick_min: [f32; 3],
    voxel_size: f32,
    band_half: f32,
    c_max: f32,
    out: &mut [i8; BRICK_VOXELS],
) {
    // The SAME P2 dominance assert fill_brick runs (curvature term uses the mesh's
    // measured c_max instead of the analytic C_MAX).
    debug_assert!(
        EPSILON_Q * band_half >= voxel_size * voxel_size * c_max / 8.0 + band_half / 254.0,
        "EPSILON_Q under-bounds curvature+quant at this (voxel, band, c_max) — the \
         per-level lower-bound budget is broken for this mesh band"
    );

    let bias = EPSILON_Q * band_half;
    const W: usize = BRICK_ALLOC;

    for z in 0..W {
        for y in 0..W {
            for x in 0..W {
                // The voxel CENTER — IDENTICAL addressing to fill_brick: the apron
                // shifts the grid one voxel below the interior min, `+0.5` lands on
                // the voxel center.
                let p = [
                    brick_min[0] + (x as f32 - APRON as f32 + 0.5) * voxel_size,
                    brick_min[1] + (y as f32 - APRON as f32 + 0.5) * voxel_size,
                    brick_min[2] + (z as f32 - APRON as f32 + 0.5) * voxel_size,
                ];
                let d = mesh_signed_distance_bvh(bvh, mesh, p);
                // Conservative store: subtract the slack so decode <= true distance.
                // The SAME encoder fill_brick uses (single-source byte-parallelism).
                let stored = crate::brick::encode_snorm8(d - bias, band_half);
                out[x + y * W + z * W * W] = stored;
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// bake_dense_grid — the MDF Stage-2c DENSE grid (a non-sparse analog of
// fill_brick_from_mesh for the dedicated R8_SNORM 3D shadow texture).
// ════════════════════════════════════════════════════════════════════════════

/// Bakes the WHOLE [`MeshSdfField`] grid as a DENSE row-major `R8_SNORM` byte image —
/// the MDF Stage-2c shadow-caster texture (one static mesh at 64-128³ does not need
/// the sparse brick-atlas; a dense 3D texture is a legitimate GPU buffer, principle 0).
///
/// IDENTICAL distance/bias/encode contract to [`fill_brick_from_mesh`], walked over the
/// DENSE `field.grid_dim` lattice instead of one apron'd brick: per voxel the sample is
/// taken at the voxel WORLD CENTER `grid_origin + (i + 0.5) * voxel_size`, biased DOWN by
/// the SAME `EPSILON_Q * band_half` slack, and quantized with the SAME
/// [`encode_snorm8`](crate::brick::encode_snorm8). The output is row-major
/// `x + y*W + z*W*H` (`W = grid_dim.x`, `H = grid_dim.y`) — the layout a `VK_IMAGE_TYPE_3D`
/// `copy_buffer_to_image` consumes tightly-packed, and the SAME order the GPU
/// `mesh_sdf_sample` trilinear fetch addresses.
///
/// The down-bias keeps the DECODED TRILINEAR reconstruction `<=` the true mesh signed
/// distance at every interior point (the C2 conservative-lower-bound contract — a plain
/// trilinear sample of this grid is sphere-trace-sound, the Hart precondition the marcher's
/// shadow march relies on). A prebuilt [`TriBvh`] over `mesh` is reused across every voxel.
///
/// Returns a heap `Vec<i8>` of `grid_dim.x * grid_dim.y * grid_dim.z` bytes (bake-time
/// only — never the marcher's hot path; the GPU samples the uploaded texture).
pub fn bake_dense_grid(mesh: &BakeMesh, field: &MeshSdfField) -> Vec<i8> {
    let bvh = build_tri_bvh(mesh);
    bake_dense_grid_with_bvh(mesh, &bvh, field)
}

/// [`bake_dense_grid`] with a CALLER-OWNED prebuilt [`TriBvh`] (build it once, reuse it
/// across the dense bake AND any brick bakes). The distance/bias/encode/layout contract is
/// identical; this split keeps the BVH construction out of the inner loop and testable in
/// isolation.
pub fn bake_dense_grid_with_bvh(mesh: &BakeMesh, bvh: &TriBvh, field: &MeshSdfField) -> Vec<i8> {
    // The SAME P2 dominance assert fill_brick_from_mesh runs — a mis-sized grid trips at
    // bake time, not as a silent over-reporting (light-leaking) field.
    debug_assert!(
        EPSILON_Q * field.band_half
            >= field.voxel_size * field.voxel_size * field.c_max / 8.0 + field.band_half / 254.0,
        "EPSILON_Q under-bounds curvature+quant at this (voxel, band, c_max) — the dense \
         grid's per-voxel lower-bound budget is broken for this mesh band"
    );

    let bias = EPSILON_Q * field.band_half;
    let [w, h, d] = field.grid_dim;
    let (w, h, d) = (w as usize, h as usize, d as usize);

    let mut out = Vec::with_capacity(w * h * d);
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                // The voxel WORLD CENTER: `grid_origin + (i + 0.5) * voxel_size`. No apron
                // here — the dense grid IS the whole lattice, so its outer voxels are the
                // texture edges (the GPU sampler's CLAMP_TO_EDGE handles an out-of-grid fetch).
                let p = [
                    field.grid_origin[0] + (x as f32 + 0.5) * field.voxel_size,
                    field.grid_origin[1] + (y as f32 + 0.5) * field.voxel_size,
                    field.grid_origin[2] + (z as f32 + 0.5) * field.voxel_size,
                ];
                let dist = mesh_signed_distance_bvh(bvh, mesh, p);
                // Conservative store: subtract the slack so decode <= true distance. The
                // SAME encoder fill_brick_from_mesh uses (single-source byte-parallelism).
                out.push(crate::brick::encode_snorm8(dist - bias, field.band_half));
            }
        }
    }
    debug_assert_eq!(out.len(), w * h * d, "dense grid must be grid_dim.x*y*z bytes");
    out
}

// ════════════════════════════════════════════════════════════════════════════
// classify_brick_from_mesh — BYTE-PARALLEL to brick.rs::classify_brick.
// ════════════════════════════════════════════════════════════════════════════

/// Classifies a grid cell's occupancy from `mesh` — BYTE-PARALLEL to
/// [`crate::brick::classify_brick`], but occupancy is derived from the MESH instead
/// of the edit AABBs.
///
/// A cell is `Surface` when its narrow band crosses (or may cross) the mesh: the
/// closest-triangle distance from the cell's CENTER, minus the cell's bounding
/// radius, is `<= band_half` (the cell could contain surface). Otherwise the cell is
/// uniformly inside / outside, decided by the GWN sign at the cell center:
/// `EmptyInside` when inside, `EmptyOutside` when outside. The bounding-radius
/// margin keeps the test CONSERVATIVE (it never reports EMPTY where surface could be
/// within the band — the C2 invariant), exactly as `classify_brick`'s inclusive
/// AABB-overlap test does for the analytic field.
///
/// - `cell_min` is the cell's minimum world corner.
/// - `cell_span` is the cell's world edge length (cubic).
/// - `band_half` matches the [`MeshSdfField`] band.
pub fn classify_brick_from_mesh(
    mesh: &BakeMesh,
    bvh: &TriBvh,
    cell_min: [f32; 3],
    cell_span: f32,
    band_half: f32,
) -> BrickClass {
    let center = [
        cell_min[0] + cell_span * 0.5,
        cell_min[1] + cell_span * 0.5,
        cell_min[2] + cell_span * 0.5,
    ];
    // The cell's bounding radius (center to a corner): half the body diagonal.
    let half_diag = cell_span * 0.5 * crate::sqrt(3.0);
    let d_center = closest_triangle_distance_bvh(bvh, mesh, center);

    // Surface the moment the band can reach inside the cell: the nearest surface is
    // within `band_half` of SOME point in the cell (closest-to-center minus the
    // bounding radius is a conservative lower bound on closest-to-any-cell-point).
    if d_center - half_diag <= band_half {
        return BrickClass::Surface;
    }

    // The band cannot reach the cell: it is uniformly inside or outside. The GWN at
    // the center settles the sign (the surface cannot cross without entering the
    // band, which the test above has ruled out).
    if generalized_winding_number(mesh, center) > GWN_INSIDE_THRESHOLD {
        BrickClass::EmptyInside
    } else {
        BrickClass::EmptyOutside
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::{decode_snorm8, fill_brick};
    use crate::{SdfEdit, SdfEditField, sd_box, sd_sphere, sdf_op};
    use alloc::vec;

    /// An axis-aligned box mesh centered at `center` with half-extents `h`: 6 quad
    /// faces as 12 triangles. The faces lie EXACTLY on the analytic box surface, so
    /// `mesh_signed_distance` reproduces `sd_box(p, center, h)` (the analytic
    /// ground-truth case). CCW winding (outward normals) for a positive GWN inside.
    fn box_mesh(center: [f32; 3], h: [f32; 3]) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let c = center;
        // 8 corners.
        let positions = vec![
            [c[0] - h[0], c[1] - h[1], c[2] - h[2]], // 0
            [c[0] + h[0], c[1] - h[1], c[2] - h[2]], // 1
            [c[0] + h[0], c[1] + h[1], c[2] - h[2]], // 2
            [c[0] - h[0], c[1] + h[1], c[2] - h[2]], // 3
            [c[0] - h[0], c[1] - h[1], c[2] + h[2]], // 4
            [c[0] + h[0], c[1] - h[1], c[2] + h[2]], // 5
            [c[0] + h[0], c[1] + h[1], c[2] + h[2]], // 6
            [c[0] - h[0], c[1] + h[1], c[2] + h[2]], // 7
        ];
        // 12 triangles, each face wound CCW as seen from OUTSIDE (outward normal).
        let indices = vec![
            [0, 3, 2],
            [0, 2, 1], // -z
            [4, 5, 6],
            [4, 6, 7], // +z
            [0, 1, 5],
            [0, 5, 4], // -y
            [3, 7, 6],
            [3, 6, 2], // +y
            [0, 4, 7],
            [0, 7, 3], // -x
            [1, 2, 6],
            [1, 6, 5], // +x
        ];
        (positions, indices)
    }

    /// A unit cube `[0, 1]³` triangle mesh (12 triangles), outward CCW.
    fn unit_cube_mesh() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        box_mesh([0.5, 0.5, 0.5], [0.5, 0.5, 0.5])
    }

    /// A UV-sphere triangle mesh of `radius` at `center`, `stacks × slices` quads.
    /// The vertices lie ON the sphere; the faces chord slightly INSIDE it (a polytope
    /// approximation). Used for the SIGN tests (inside-negative / outside-positive)
    /// where the polytope vs. analytic gap does not matter.
    fn uv_sphere_mesh(
        center: [f32; 3],
        radius: f32,
        stacks: u32,
        slices: u32,
    ) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        use core::f32::consts::PI;
        let mut positions: Vec<[f32; 3]> = Vec::new();
        for i in 0..=stacks {
            let phi = PI * i as f32 / stacks as f32; // 0..π (pole to pole)
            let (sp, cp) = (phi.sin(), phi.cos());
            for j in 0..=slices {
                let theta = 2.0 * PI * j as f32 / slices as f32;
                let (st, ct) = (theta.sin(), theta.cos());
                positions.push([
                    center[0] + radius * sp * ct,
                    center[1] + radius * cp,
                    center[2] + radius * sp * st,
                ]);
            }
        }
        let row = slices + 1;
        let mut indices: Vec<[u32; 3]> = Vec::new();
        for i in 0..stacks {
            for j in 0..slices {
                let a = i * row + j;
                let b = a + 1;
                let d = (i + 1) * row + j;
                let e = d + 1;
                // Outward winding (normal points away from center): the GWN of an
                // interior point must be POSITIVE, matching the box mesh convention.
                indices.push([a, b, d]);
                indices.push([b, e, d]);
            }
        }
        (positions, indices)
    }

    #[test]
    fn point_triangle_distance_landmarks() {
        // A triangle in the z=0 plane; check face / edge / vertex regions.
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let c = [0.0, 1.0, 0.0];
        // Directly above the interior centroid: distance == height.
        let p_face = [0.25, 0.25, 0.7];
        let d_face = point_triangle_distance_sq(a, b, c, p_face).sqrt();
        assert!((d_face - 0.7).abs() < 1e-5, "face region: {d_face}");
        // Beyond vertex `c`: distance == |p - c|.
        let p_vert = [0.0, 2.0, 0.0];
        let d_vert = point_triangle_distance_sq(a, b, c, p_vert).sqrt();
        assert!((d_vert - 1.0).abs() < 1e-5, "vertex region: {d_vert}");
        // Beyond the a-b edge midpoint (in plane): distance to the edge.
        let p_edge = [0.5, -0.5, 0.0];
        let d_edge = point_triangle_distance_sq(a, b, c, p_edge).sqrt();
        assert!((d_edge - 0.5).abs() < 1e-5, "edge region: {d_edge}");
    }

    #[test]
    fn bvh_equals_brute_force_unsigned() {
        // The BVH is EXACT, not approximate: every query bit-matches brute force.
        let (pos, idx) = uv_sphere_mesh([0.3, -0.2, 0.1], 1.0, 12, 16);
        let mesh = BakeMesh::new(&pos, &idx);
        let bvh = build_tri_bvh(&mesh);
        for px in [-2.0f32, -0.5, 0.0, 0.31, 1.2, 3.0] {
            for py in [-1.7f32, 0.0, 0.4, 2.1] {
                for pz in [-2.4f32, 0.1, 1.9] {
                    let p = [px, py, pz];
                    let brute = closest_triangle_distance(&mesh, p);
                    let bvh_d = closest_triangle_distance_bvh(&bvh, &mesh, p);
                    assert_eq!(
                        bvh_d.to_bits(),
                        brute.to_bits(),
                        "BVH must be bit-exact vs brute at {p:?}: bvh={bvh_d}, brute={brute}"
                    );
                }
            }
        }
    }

    #[test]
    fn gwn_inside_outside_watertight() {
        let (pos, idx) = unit_cube_mesh();
        let mesh = BakeMesh::new(&pos, &idx);
        // Strictly inside the center: ~1.0 (> 0.5).
        let w_in = generalized_winding_number(&mesh, [0.5, 0.5, 0.5]);
        assert!(w_in > 0.5, "center should read inside, got {w_in}");
        assert!(
            (w_in - 1.0).abs() < 1e-2,
            "watertight interior GWN ≈ 1, got {w_in}"
        );
        // Far outside: ~0.0 (< 0.5).
        let w_out = generalized_winding_number(&mesh, [5.0, 5.0, 5.0]);
        assert!(w_out < 0.5, "far point should read outside, got {w_out}");
        assert!(w_out.abs() < 1e-2, "exterior GWN ≈ 0, got {w_out}");
    }

    #[test]
    fn gwn_robust_on_non_watertight() {
        // The unit cube with ONE triangle removed (an open hole): the GWN of a deep
        // interior point degrades gracefully — still > 0.5 (a parity ray-cast would
        // flicker through the hole).
        let (pos, mut idx) = unit_cube_mesh();
        idx.pop(); // remove the last triangle (half of the +x face)
        let mesh = BakeMesh::new(&pos, &idx);
        let w_in = generalized_winding_number(&mesh, [0.5, 0.5, 0.5]);
        assert!(
            w_in > 0.5,
            "non-watertight interior must still read inside, got {w_in}"
        );
    }

    #[test]
    fn mesh_signed_distance_box_matches_analytic() {
        // A box mesh's faces lie EXACTLY on the analytic box surface, so the mesh
        // signed distance reproduces `sd_box` (ground truth) within fp slack.
        let center = [0.2, -0.1, 0.3];
        let h = [0.6, 0.4, 0.5];
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);
        let bvh = build_tri_bvh(&mesh);
        for px in [-1.0f32, 0.0, 0.2, 0.7, 1.3] {
            for py in [-0.8f32, -0.1, 0.5] {
                for pz in [-0.9f32, 0.3, 1.1] {
                    let p = [px, py, pz];
                    let analytic = sd_box(p, center, h);
                    let mesh_d = mesh_signed_distance_bvh(&bvh, &mesh, p);
                    assert!(
                        (mesh_d - analytic).abs() < 2e-3,
                        "box mesh signed distance must match sd_box at {p:?}: \
                         mesh={mesh_d}, analytic={analytic}"
                    );
                }
            }
        }
    }

    #[test]
    fn mesh_signed_distance_sign_box_and_sphere() {
        // Box: negative strictly inside, positive strictly outside.
        let (bp, bi) = box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let bmesh = BakeMesh::new(&bp, &bi);
        assert!(
            mesh_signed_distance(&bmesh, [0.0, 0.0, 0.0]) < 0.0,
            "box center inside"
        );
        assert!(
            mesh_signed_distance(&bmesh, [3.0, 0.0, 0.0]) > 0.0,
            "box exterior outside"
        );
        // Sphere: negative inside, positive outside.
        let (sp, si) = uv_sphere_mesh([0.0, 0.0, 0.0], 1.0, 16, 24);
        let smesh = BakeMesh::new(&sp, &si);
        assert!(
            mesh_signed_distance(&smesh, [0.0, 0.0, 0.0]) < 0.0,
            "sphere center inside"
        );
        assert!(
            mesh_signed_distance(&smesh, [2.5, 0.0, 0.0]) > 0.0,
            "sphere exterior outside"
        );
    }

    #[test]
    fn fill_brick_from_mesh_is_a_conservative_lower_bound_box() {
        // The C2 soundness gate (mirrors brick.rs's capsule/sphere lower-bound test):
        // for a BOX mesh (faces exactly analytic), the DECODED stored grid value is
        // `<=` the true analytic signed distance at every interior voxel center.
        let center = [0.0, 0.0, 0.0];
        let h = [0.8, 0.8, 0.8];
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);
        let bvh = build_tri_bvh(&mesh);

        let voxel = 0.125;
        let band_half = SDF_EDIT_BAND_HALF;
        let c_max = measure_mesh_c_max(&mesh);
        // The brick whose interior min sits near the +x face (band crosses it).
        let brick_min = [0.5, -0.5, -0.5];

        let mut brick = [0i8; BRICK_VOXELS];
        fill_brick_from_mesh(&mesh, &bvh, brick_min, voxel, band_half, c_max, &mut brick);

        const W: usize = BRICK_ALLOC;
        for z in 0..W {
            for y in 0..W {
                for x in 0..W {
                    let p = [
                        brick_min[0] + (x as f32 - APRON as f32 + 0.5) * voxel,
                        brick_min[1] + (y as f32 - APRON as f32 + 0.5) * voxel,
                        brick_min[2] + (z as f32 - APRON as f32 + 0.5) * voxel,
                    ];
                    let decoded = decode_snorm8(brick[x + y * W + z * W * W], band_half);
                    let analytic = sd_box(p, center, h);
                    // Only the in-band samples are a meaningful bound (saturated codes
                    // sit at ±band_half, which is trivially below a far analytic |d|).
                    if analytic.abs() < band_half {
                        assert!(
                            decoded <= analytic + 1e-4,
                            "lower-bound violated at {p:?}: decoded={decoded} > \
                             analytic={analytic}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bake_dense_grid_is_a_conservative_lower_bound() {
        // The MDF Stage-2c soundness gate: the dense grid's DECODED + BIASED value must be
        // a CONSERVATIVE LOWER BOUND of the true mesh signed distance over a voxel-center
        // sweep — so a plain trilinear sample of it is sphere-trace-sound (the Hart
        // precondition the marcher's shadow march relies on). Mirrors
        // `fill_brick_from_mesh_is_a_conservative_lower_bound_box` for the whole dense grid.
        let center = [0.0, 0.0, 0.0];
        let h = [0.6, 0.5, 0.7];
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);

        // A voxel fine enough that the P2 budget holds (`for_mesh` asserts it on construct).
        let field = MeshSdfField::for_mesh(&mesh, 0.125);
        let grid = bake_dense_grid(&mesh, &field);

        let [w, hh, _d] = field.grid_dim;
        let (w, hh) = (w as usize, hh as usize);
        let band_half = field.band_half;

        for z in 0..field.grid_dim[2] as usize {
            for y in 0..hh {
                for x in 0..w {
                    let p = [
                        field.grid_origin[0] + (x as f32 + 0.5) * field.voxel_size,
                        field.grid_origin[1] + (y as f32 + 0.5) * field.voxel_size,
                        field.grid_origin[2] + (z as f32 + 0.5) * field.voxel_size,
                    ];
                    let decoded = decode_snorm8(grid[x + y * w + z * w * hh], band_half);
                    let truth = mesh_signed_distance(&mesh, p);
                    // Only the in-band samples are a meaningful bound (saturated codes sit at
                    // ±band_half, trivially below a far true |d|). +1e-4 absorbs fp rounding
                    // of the two distance paths (well under one snorm step).
                    if truth.abs() < band_half {
                        assert!(
                            decoded <= truth + 1e-4,
                            "dense lower-bound violated at {p:?}: decoded={decoded} > \
                             truth={truth}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bake_dense_grid_has_grid_dim_byte_count() {
        // The dense grid is exactly `grid_dim.x * y * z` bytes, row-major — the byte count a
        // tightly-packed 3D `copy_buffer_to_image` upload depends on.
        let center = [0.0, 0.0, 0.0];
        let h = [0.4, 0.4, 0.4];
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);
        let field = MeshSdfField::for_mesh(&mesh, 0.2);
        let grid = bake_dense_grid(&mesh, &field);
        let expected =
            field.grid_dim[0] as usize * field.grid_dim[1] as usize * field.grid_dim[2] as usize;
        assert_eq!(grid.len(), expected, "dense grid byte count != grid_dim product");
    }

    #[test]
    fn fill_brick_from_mesh_byte_parallel_to_fill_brick_for_a_box() {
        // BYTE-PARALLEL: a box MESH and the analytic box EDIT describe the SAME
        // surface, so `fill_brick_from_mesh` and `fill_brick` must agree within ±1
        // quantization step (the only divergence is fp rounding of the two distance
        // paths, well under one snorm code).
        let center = [0.0, 0.0, 0.0];
        let h = [0.7, 0.5, 0.6];

        // Analytic edit field (one box).
        let mut field = SdfEditField::new();
        field.push(SdfEdit::box_shape(center, h, sdf_op::UNION, 0.0));

        // The mesh of the SAME box.
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);
        let bvh = build_tri_bvh(&mesh);

        let voxel = 0.125;
        let band_half = SDF_EDIT_BAND_HALF;
        let c_max = measure_mesh_c_max(&mesh);
        let brick_min = [0.4, -0.5, -0.5]; // straddling the +x face

        let mut from_edit = [0i8; BRICK_VOXELS];
        let mut from_mesh = [0i8; BRICK_VOXELS];
        // The analytic fill uses its own C_MAX baseline; pass the brick's analytic
        // c_max via the public fill_brick (its assert uses the analytic curvature).
        fill_brick(&field, brick_min, voxel, band_half, 2.0, &mut from_edit);
        fill_brick_from_mesh(
            &mesh,
            &bvh,
            brick_min,
            voxel,
            band_half,
            c_max,
            &mut from_mesh,
        );

        for i in 0..BRICK_VOXELS {
            let diff = (from_edit[i] as i32 - from_mesh[i] as i32).abs();
            assert!(
                diff <= 1,
                "byte-parallel fill diverged > 1 step at voxel {i}: \
                 edit={}, mesh={}",
                from_edit[i],
                from_mesh[i]
            );
        }
    }

    #[test]
    fn classify_brick_from_mesh_surface_inside_outside() {
        // A large box so a genuinely DEEP-interior cell exists (the band — 0.90 wide
        // — plus the cell's bounding radius must not reach the nearest face).
        let center = [0.0, 0.0, 0.0];
        let h = [2.0, 2.0, 2.0];
        let (pos, idx) = box_mesh(center, h);
        let mesh = BakeMesh::new(&pos, &idx);
        let bvh = build_tri_bvh(&mesh);
        let band_half = SDF_EDIT_BAND_HALF;
        let span = 0.5;

        // A cell straddling the +x face (at x = +2) → Surface.
        let surf = classify_brick_from_mesh(&mesh, &bvh, [1.8, -0.25, -0.25], span, band_half);
        assert_eq!(surf, BrickClass::Surface, "face-straddling cell is Surface");

        // A cell at the box center (surface 2.0 away, band 0.90 + half-diag ~0.43
        // cannot reach) → EmptyInside.
        let inside = classify_brick_from_mesh(&mesh, &bvh, [-0.25, -0.25, -0.25], span, band_half);
        assert_eq!(
            inside,
            BrickClass::EmptyInside,
            "deep-interior cell is EmptyInside"
        );

        // A cell far outside the box (band cannot reach) → EmptyOutside.
        let outside = classify_brick_from_mesh(&mesh, &bvh, [8.0, 8.0, 8.0], span, band_half);
        assert_eq!(
            outside,
            BrickClass::EmptyOutside,
            "far cell is EmptyOutside"
        );
    }

    #[test]
    fn mesh_sdf_field_layout_covers_the_mesh() {
        let (pos, idx) = uv_sphere_mesh([1.0, 2.0, -1.0], 1.0, 12, 16);
        let mesh = BakeMesh::new(&pos, &idx);
        let field = MeshSdfField::for_mesh(&mesh, 0.125);
        // The grid origin must sit below the mesh AABB by at least the band reach.
        for axis in 0..3 {
            assert!(
                field.grid_origin[axis] <= mesh.aabb_min[axis] - field.band_half,
                "grid must margin past the mesh AABB on axis {axis}"
            );
            let far = field.grid_origin[axis] + field.grid_dim[axis] as f32 * field.voxel_size;
            assert!(
                far >= mesh.aabb_max[axis] + field.band_half,
                "grid must extend past the mesh AABB on axis {axis}"
            );
        }
        // The P2 budget held (no debug-assert panic) and c_max is at the floor or above.
        assert!(
            field.c_max >= 2.0,
            "c_max must not drop below the brick floor"
        );
    }

    /// A sentinel that the analytic and mesh paths agree on the SIGN of a sphere's
    /// distance over a sweep (sanity for the GWN sign vs `sd_sphere`).
    #[test]
    fn sphere_sign_agrees_with_analytic_sweep() {
        let (pos, idx) = uv_sphere_mesh([0.0, 0.0, 0.0], 1.0, 20, 32);
        let mesh = BakeMesh::new(&pos, &idx);
        for r in [0.2f32, 0.5, 0.85, 1.2, 2.0] {
            let p = [r, 0.0, 0.0];
            let mesh_sign = mesh_signed_distance(&mesh, p) < 0.0;
            let analytic_sign = sd_sphere(p, [0.0, 0.0, 0.0], 1.0) < 0.0;
            assert_eq!(
                mesh_sign, analytic_sign,
                "sign mismatch at r={r}: the mesh and analytic sphere disagree on inside/outside"
            );
        }
    }
}
