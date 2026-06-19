//! SP3 soft-body SELF-COLLISION — same-body particle-vs-particle collision via a
//! per-body open-addressed spatial hash (counting-sort CSR).
//!
//! [`resolve_self_collision`] runs AFTER the volume-constraint sweep and BEFORE the
//! soft↔rigid coupling (so the corrected positions feed the coupled-velocity
//! computation for free — no separate velocity fold). For each particle it pushes
//! same-body neighbours within `2·radius` apart with a rigid (compliance `α = 0`)
//! PBD distance constraint, split by inverse mass, run for
//! [`PhysicsConfig::self_collision_iters`](crate::PhysicsConfig) Gauss-Seidel sweeps.
//!
//! # Scope (SP3 v1)
//!
//! SAME-BODY ONLY: it resolves particles WITHIN one [`SoftBody`]; there is no
//! inter-body self-collision. The pass is opt-in via `self_collision_iters > 0` and
//! requires `particle_radius > 0` — both gates early-return BEFORE any hashing, so a
//! world that does not opt in is byte-identical (the SP3 0%-gate).
//!
//! # Spatial hash (zero per-step alloc)
//!
//! A counting-sort compressed-sparse-row (CSR) table over a uniform grid of cell
//! size `2·radius`, rebuilt each substep into the body's preallocated
//! `sc_cell_start` / `sc_cell_items` / `sc_cursor` scratch columns (no allocation in
//! the step). The table size is `T = next_pow2(2n)`; the cell hash is Teschner et
//! al.'s `((ix·p1) ^ (iy·p2) ^ (iz·p3)) & (T − 1)` (wrapping integer multiplies,
//! cell coordinates from `floor(pos / cell)` as `i32`), kept in a pinned op form.
//!
//! # Determinism (INVIOLABLE — identical rules as SP1/SP2)
//!
//! EXACT `sqrt` + divide only — no `rsqrt`/`rcp`/`mul_add`/FMA, no
//! [`Vec3::normalize`] (the direction is an explicit `d * (1.0 / len)` past the
//! [`LEN_EPS`] guard). The candidate visit order NEVER depends on hash-bucket
//! traversal order in a way that changes float accumulation: particles are swept in
//! pinned index order `0..n`, and each particle's candidates are produced by a
//! deterministic CSR scan of its 27-cell neighbourhood with the de-dup rules below,
//! so the per-substep result is bit-stable.

use crate::math::Vec3;
use crate::soft::component::SoftBody;
use crate::soft::solver::{LEN_EPS, SoftCols};
use crate::solver::contact::is_dynamic_row;

/// Teschner et al. spatial-hash prime for the X cell coordinate.
const HASH_P1: i32 = 73_856_093;
/// Teschner et al. spatial-hash prime for the Y cell coordinate.
const HASH_P2: i32 = 19_349_663;
/// Teschner et al. spatial-hash prime for the Z cell coordinate.
const HASH_P3: i32 = 83_492_791;

/// Hashes integer cell coordinates into a bucket index in `[0, table)`.
///
/// `table` MUST be a power of two (the caller guarantees `next_pow2(2n)`), so the
/// `& (table − 1)` masks instead of taking a modulo. The multiplies are `wrapping`
/// (overflow is part of the hash, not UB) and kept in a pinned op order so the
/// bucket of a given cell is bit-stable across runs.
#[inline]
fn cell_hash(ix: i32, iy: i32, iz: i32, table: usize) -> usize {
    let h = (ix.wrapping_mul(HASH_P1)) ^ (iy.wrapping_mul(HASH_P2)) ^ (iz.wrapping_mul(HASH_P3));
    // `table` is a power of two ⇒ `& (table - 1)` is the bucket; cast through `u32`
    // so a negative `h` maps by its two's-complement bit pattern (deterministic),
    // never a sign-extended out-of-range `usize`.
    (h as u32 as usize) & (table - 1)
}

/// Floors a world coordinate to an integer cell index along one axis.
///
/// `inv_cell = 1.0 / cell` (`cell = 2·radius > 0`, the caller's precondition). EXACT
/// `mul` + `floor` only (the determinism boundary); the `as i32` truncates toward zero
/// AFTER the `floor`, so for the finite, in-range coordinates a stable soft body
/// produces it is the true floor.
///
/// SAFETY CONTRACT (no panic, no OOB): Rust's `f32 as i32` is a SATURATING cast — it
/// never panics and never produces an out-of-range value. `NaN → 0`, `+inf → i32::MAX`,
/// `-inf → i32::MIN`, and any magnitude beyond `i32`'s range clamps to the nearest
/// bound. So a sim that produces a non-finite or extreme position yields a well-defined
/// (if meaningless) cell index that hashes into a valid bucket — the pass degrades
/// gracefully rather than aborting. A debug guard at the per-particle read surfaces
/// such positions loudly without changing release behaviour. This is NOT enforced
/// here: callers must NOT rely on the input being finite for memory safety.
#[inline]
fn cell_coord(x: f32, inv_cell: f32) -> i32 {
    (x * inv_cell).floor() as i32
}

/// Runs the SP3 same-body self-collision pass on one soft body (`iters` GS sweeps).
///
/// Early-returns (a true no-op, no hashing) when `iters == 0`, `radius <= 0`, or the
/// body has fewer than two particles — these are the SP3 0%-gate guards. Otherwise it
/// rebuilds the spatial-hash CSR table ONCE and then runs `iters` Gauss-Seidel sweeps
/// of the push-to-`2·radius` constraint in pinned particle order.
///
/// The CSR table is built from the positions at pass entry and reused for ALL `iters`
/// sweeps. This is the standard PBD approximation, NOT exact bucketing validity: a
/// particle whose correction carries it across a cell boundary mid-pass is not
/// re-bucketed until the next substep's rebuild. The `cell <= L0` precondition keeps
/// the per-iteration drift below one cell, so a same-cell pair stays same-cell within
/// the pass; the staleness is bounded and self-corrects on the next substep.
///
/// `cell = 2·radius`. PRECONDITION (architect N): `cell <= L0`, the smallest
/// distance-constraint rest length — otherwise a neighbour one cell away can be a
/// genuine constraint partner the bucketing would miss. A violation emits a debug-only
/// warning (see `rest_len_warn`) and is release-safe regardless (a missed pair only
/// under-resolves).
pub fn resolve_self_collision(body: &mut SoftBody, iters: usize, radius: f32) {
    // SP3 0%-gate: no sweeps, or a degenerate radius (a `0` cell size would divide
    // by zero / form a degenerate grid). Return BEFORE any hashing.
    if iters == 0 || radius <= 0.0 {
        if radius <= 0.0 {
            radius_warn(radius);
        }
        return;
    }
    let n = body.particle_count();
    if n < 2 {
        // Fewer than two particles — no pair can collide.
        return;
    }

    let table = body.self_table_size();
    debug_assert!(
        table.is_power_of_two() && table >= 2,
        "invariant: self-collision table size must be a power of two >= 2"
    );
    debug_assert!(
        body.sc_cell_start.len() == table + 1
            && body.sc_cursor.len() == table + 1
            && body.sc_cell_items.len() == n,
        "invariant: self-collision CSR scratch must be sized to (table + 1) / n"
    );

    let cell = 2.0 * radius;
    let inv_cell = 1.0 / cell;
    // PRECONDITION (architect N): `cell <= L0`. A violation is release-safe (a missed
    // neighbour-cell pair only under-resolves), so warn loudly in debug WITHOUT
    // aborting an otherwise-valid sim — compiled out entirely in release.
    rest_len_warn(body, cell);

    build_hash(body, table, inv_cell);
    load_factor_warn(n, table);

    for _ in 0..iters {
        // The serial sweep VISITS each accepted pair by projecting it in place — the
        // SP3 leaf action. `project_self_pair` mutates `body` through the param the
        // visitor receives (not a capture), so it never conflicts with the `&mut body`
        // the sweep itself holds.
        sweep(body, table, inv_cell, |body, i, j| {
            project_self_pair(body, i, j, cell);
        });
    }
}

/// Rebuilds the spatial-hash CSR table into the body's preallocated scratch (zero
/// allocation): pass-1 count per bucket, exclusive prefix-sum into `sc_cell_start`,
/// pass-2 stable scatter of particle indices into `sc_cell_items`.
///
/// `pub(in crate::soft)` (SP4 W3-A): the colored self-collision pass builds the hash
/// ONCE per substep through this SINGLE definition (C4 — one hash, swept `iters`
/// times) before emitting/coloring/solving its pairs.
pub(in crate::soft) fn build_hash(body: &mut SoftBody, table: usize, inv_cell: f32) {
    let n = body.particle_count();

    // Pass 1 — count particles per bucket. `sc_cell_start` is reused as the per-
    // bucket counter (length `table + 1`); zero ALL `table + 1` slots, including the
    // trailing total slot `table`. The `sc_*` scratch PERSISTS on the body across
    // substeps/steps, so on rebuild #2+ the trailing slot still holds the previous
    // rebuild's total (== `n`); the exclusive prefix sum scans `0..=table` and would
    // fold that stale value into `acc`, breaking `debug_assert!(acc == n)`. Clearing
    // it here makes every rebuild start clean.
    for c in body.sc_cell_start[..=table].iter_mut() {
        *c = 0;
    }
    for i in 0..n {
        let b = particle_bucket(body, i, inv_cell, table);
        body.sc_cell_start[b] += 1;
    }

    // Exclusive prefix sum: `sc_cell_start[b]` becomes the start offset of bucket
    // `b`; the running sum lands in slot `table` (the trailing total == `n`).
    let mut acc: u32 = 0;
    for off in body.sc_cell_start.iter_mut() {
        let cnt = *off;
        *off = acc;
        acc += cnt;
    }
    debug_assert!(acc as usize == n, "invariant: CSR prefix-sum total must equal n");

    // Seed the scatter cursor from the offsets.
    body.sc_cursor.copy_from_slice(&body.sc_cell_start);

    // Pass 2 — stable scatter (ascending `i` ⇒ ascending order within each bucket,
    // so the candidate scan is deterministic).
    for i in 0..n {
        let b = particle_bucket(body, i, inv_cell, table);
        let slot = body.sc_cursor[b] as usize;
        body.sc_cell_items[slot] = i as u32;
        body.sc_cursor[b] += 1;
    }
}

/// One Gauss-Seidel self-collision sweep in pinned particle order `0..n`.
///
/// For each particle `i`, queries the 27 CELL COORDINATES of its 3×3×3
/// neighbourhood (the home cell `(0,0,0)` included — NOT special-cased), in pinned
/// `dz`→`dy`→`dx` nesting order. Each neighbour coordinate `(cx,cy,cz)` is hashed to
/// its bucket and that bucket's CSR slice is scanned ascending; a candidate `j` is
/// accepted only when `j > i` (the unordered-pair de-dup) AND `j`'s ACTUAL cell
/// coordinate equals `(cx,cy,cz)` — see [`resolve_pair_in_cell`].
///
/// The coordinate filter is what makes the pass correct AND deterministic under hash
/// collisions: every particle has exactly ONE true cell coordinate, so it is accepted
/// by exactly ONE of the 27 coordinate queries, regardless of how the 27 coordinates
/// alias onto buckets. There is no double-apply (a coordinate that hash-collides with
/// another simply scans the shared bucket again but filters to its own coordinate),
/// and no foreign particle that merely hash-collided into a scanned bucket is ever
/// paired. The visit order is a pure function of (`i` ascending, fixed offset order,
/// ascending CSR index) — independent of hash aliasing.
///
/// # Visitor (SP4 W3-A — the SINGLE traversal, two leaf actions)
///
/// `visit(body, i, j)` is the leaf action applied to each accepted pair in the
/// pinned SP3 order. The serial path passes a closure that PROJECTS the pair in
/// place ([`project_self_pair`]); the colored `soft::colored::step_body_colored`
/// passes a closure that EMITS `(i, j)` into an ordered pair list (greedy first-fit
/// is order-dependent, so it consumes pairs in exactly this emission order, C3a).
/// `body` is delivered as a PARAMETER (not a capture), so a mutating leaf action
/// never conflicts with the `&mut body` the traversal holds. `pub(in crate::soft)`
/// so the colored module reuses the SINGLE traversal — never a duplicate (the
/// C1/W3-A drift-avoidance).
pub(in crate::soft) fn sweep<F>(body: &mut SoftBody, table: usize, inv_cell: f32, mut visit: F)
where
    F: FnMut(&mut SoftBody, usize, usize),
{
    let n = body.particle_count();
    for i in 0..n {
        // Read `i`'s cell coordinates from its (live) position. NaN/inf would
        // saturating-cast to a finite cell index (no panic/OOB), but a stable sim
        // never produces them — catch it loudly in debug.
        debug_assert!(
            body.pos_x[i].is_finite() && body.pos_y[i].is_finite() && body.pos_z[i].is_finite(),
            "invariant: self-collision sweep read a non-finite particle position"
        );
        let ix = cell_coord(body.pos_x[i], inv_cell);
        let iy = cell_coord(body.pos_y[i], inv_cell);
        let iz = cell_coord(body.pos_z[i], inv_cell);

        // Query the home cell + the 26 neighbours BY CELL COORDINATE, pinned order.
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let cx = ix + dx;
                    let cy = iy + dy;
                    let cz = iz + dz;
                    let bucket = cell_hash(cx, cy, cz, table);
                    resolve_pair_in_cell(
                        body, i, bucket, cx, cy, cz, inv_cell, table, &mut visit,
                    );
                }
            }
        }
    }
}

/// Visits particle `i` against every candidate `j` in CSR `bucket` whose ACTUAL
/// cell coordinate equals the queried `(cx, cy, cz)`, applying the `visit` leaf
/// action to each accepted pair.
///
/// Two acceptance gates:
///  - `j > i` — the unordered-pair de-dup: it tests each unordered pair at most once
///    and skips the self-pair.
///  - `cell_coord(j) == (cx, cy, cz)` — the foreign-particle filter. A bucket can hold
///    particles from MULTIPLE distinct cells that hash-collide into it; only those
///    whose recomputed cell coordinate matches the coordinate this query is for are
///    true neighbours. Recomputing `j`'s coordinate with the SAME [`cell_coord`] +
///    `inv_cell` used at build time makes the comparison exact (no new float op that
///    could disagree). This guarantees `j` is accepted by exactly ONE of the 27
///    coordinate queries for a given `i` — no double-apply, fully hash-independent.
///
/// The CSR slice `sc_cell_items[start..end]` is scanned in ascending index order (the
/// stable scatter), so the accumulation order is deterministic. `visit(body, i, j)`
/// is the leaf action (project in place, or emit into the colored pair list) —
/// `pub(in crate::soft)` (SP4 W3-A) so the colored module shares this traversal.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(in crate::soft) fn resolve_pair_in_cell<F>(
    body: &mut SoftBody,
    i: usize,
    bucket: usize,
    cx: i32,
    cy: i32,
    cz: i32,
    inv_cell: f32,
    table: usize,
    visit: &mut F,
) where
    F: FnMut(&mut SoftBody, usize, usize),
{
    debug_assert!(bucket == cell_hash(cx, cy, cz, table), "invariant: bucket must hash from (cx,cy,cz)");
    let start = body.sc_cell_start[bucket] as usize;
    let end = body.sc_cell_start[bucket + 1] as usize;
    for s in start..end {
        let j = body.sc_cell_items[s] as usize;
        if j <= i {
            continue;
        }
        // Coordinate filter: reject foreign particles that only hash-collided into
        // this bucket. Recompute `j`'s cell coord with the identical build-time op.
        let jx = cell_coord(body.pos_x[j], inv_cell);
        let jy = cell_coord(body.pos_y[j], inv_cell);
        let jz = cell_coord(body.pos_z[j], inv_cell);
        if jx == cx && jy == cy && jz == cz {
            visit(body, i, j);
        }
    }
}

/// Projects one self-collision pair `(i, j)` — a rigid (`α = 0`) push apart to the
/// minimum separation `cell = 2·radius` (EXACT sqrt + divide only).
///
/// Mirrors [`project_distance`](crate::soft::solver) but ONE-SIDED (push apart only,
/// never pull together): the constraint is active only when the particles are CLOSER
/// than `cell`. Both-pinned pairs are skipped before the `sqrt`; a coincident pair
/// (`len < LEN_EPS`, zero gradient) is skipped (no usable push direction). The
/// correction is split by inverse mass (`s·wᵢ` / `−s·wⱼ`).
///
/// An isolated SHALLOW pair separates to exactly `2·radius` in a single iteration (the
/// full-strength rigid projection). Deeper or clustered overlaps converge over the
/// configured [`self_collision_iters`](crate::PhysicsConfig) Gauss-Seidel sweeps — the
/// `iters` count, NOT a per-constraint cap, is the deep-overlap limiter.
///
/// # Per-endpoint write guard (SP4 C1 — LOAD-BEARING)
///
/// Each endpoint's write is gated `if is_dynamic_row(w) { add }` through the SAME
/// [`is_dynamic_row`] predicate the SP4 self-collision coloring uses (the SHARED
/// kernel for serial + colored). Byte-preserving serially (a pinned endpoint's
/// skipped write was `+= ±0.0`); in the colored sweep it removes a same-color
/// pinned-write race. The LOAD-BEARING finiteness invariant is `s.is_finite()` past
/// the divide (see [`project_distance`](crate::soft::solver)); it MUST NOT be
/// `w >= 0.0`.
///
/// `pub(in crate::soft)` (SP4 W3-A): the SINGLE leaf definition the serial sweep
/// AND the colored color-by-color solve both call. A thin wrapper extracting
/// [`SoftCols`] from `body` and forwarding to the raw core [`project_self_pair_raw`]
/// (the math lives there ONCE; the serial path stays byte-identical to the raw
/// per-element form — `body.pos_x[i]` and `*pos_x.add(i)` name the same storage).
#[inline]
pub(in crate::soft) fn project_self_pair(body: &mut SoftBody, i: usize, j: usize, cell: f32) {
    let cols = SoftCols::from_body(body);
    // SAFETY: `cols` names `body`'s live column bases; `i`/`j` are valid particle
    //   rows (`< particle_count()`, produced by the sweep over hashed buckets). The
    //   serial caller holds the unique `&mut SoftBody`, so no aliasing.
    unsafe { project_self_pair_raw(cols, i, j, cell) };
}

/// The RAW per-element core of [`project_self_pair`] (SP4 W3-A soundness fix).
///
/// Identical one-sided rigid push-apart math, but every column access is a raw
/// `*base.add(p)` instead of a `Vec` index, so under Tree-Borrows it retags ONLY the
/// element it touches — never the whole buffer. This is what makes the
/// colored-parallel self-collision solve race-free: two workers writing the
/// C2-disjoint dynamic rows of one color form no overlapping element protectors. The
/// serial wrapper calls it through the unique `&mut SoftBody` (no aliasing); a
/// colored worker calls it through [`SoftCols`] built from the live body, writing
/// only its chunk's disjoint dynamic rows.
///
/// # Safety
/// `cols` must name a live `SoftBody`'s column bases; `i`/`j` must be valid particle
/// rows `< particle_count()`. In the parallel path, the caller must invoke this only
/// on pairs of ONE color whose DYNAMIC endpoints are pairwise disjoint across
/// concurrent workers (the C2 lemma); a SHARED PINNED row is read-only (the C1 guard
/// never writes it). On those conditions the per-element reads/writes touch only
/// provably-disjoint elements (writes) plus the read-only shared `inv_mass` column —
/// no UB.
#[inline]
pub(in crate::soft) unsafe fn project_self_pair_raw(cols: SoftCols, i: usize, j: usize, cell: f32) {
    // SAFETY (all the per-element accesses below): `i`/`j` are in range (the fn
    //   contract); each `*p.add(k)` retags only element `k`. Reads of `inv_mass` are
    //   read-only shared; the `pos_*` writes are gated to the pair's own dynamic rows
    //   (disjoint across workers — the contract).
    let wi = unsafe { *cols.inv_mass.add(i) };
    let wj = unsafe { *cols.inv_mass.add(j) };
    let wsum = wi + wj;
    if wsum == 0.0 {
        // Both endpoints pinned — skip BEFORE the sqrt.
        return;
    }
    let d = unsafe {
        Vec3::new(
            *cols.pos_x.add(i) - *cols.pos_x.add(j),
            *cols.pos_y.add(i) - *cols.pos_y.add(j),
            *cols.pos_z.add(i) - *cols.pos_z.add(j),
        )
    };
    // EXACT sqrt (the determinism boundary) — never `rsqrt`.
    let len = d.length_squared().sqrt();
    if len >= cell {
        // Not overlapping — one-sided constraint never pulls together.
        return;
    }
    if len < LEN_EPS {
        // Coincident particles — direction undefined, zero gradient ⇒ no push.
        return;
    }
    // DIVIDE then mul (explicit; NOT `rsqrt`, NOT `Vec3::normalize`).
    let nrm = d * (1.0 / len);
    // Constraint value `C = len - cell < 0` here (`len < cell`, the overlap branch).
    // EXACT `sub` only.
    let cc = len - cell;
    // `α = 0` (rigid) ⇒ the XPBD denominator is just `wsum`. `wsum > 0` here (the
    // both-pinned case returned).
    debug_assert!(wsum > 0.0, "invariant: self-collision denom must be > 0");
    let s = -cc / wsum;
    // SP4 C1: `s` finite is the LOAD-BEARING invariant that makes the pinned-endpoint
    // skip below bit-equal to `+= nrm*(s*±0.0)`. NOT `w >= 0.0` (see project_distance).
    debug_assert!(s.is_finite(), "invariant: self-collision Lagrange step must be finite");
    // Split by inverse mass; a pinned endpoint (w == 0) gets no move.
    let di = nrm * (s * wi);
    let dj = nrm * (-s * wj);
    // SP4 C1 per-endpoint write guard (shared by serial + colored): write only a
    // DYNAMIC endpoint, removing the value-benign pinned `+= ±0.0`. Byte-preserving
    // serially.
    if is_dynamic_row(wi) {
        unsafe {
            *cols.pos_x.add(i) += di.x;
            *cols.pos_y.add(i) += di.y;
            *cols.pos_z.add(i) += di.z;
        }
    }
    if is_dynamic_row(wj) {
        unsafe {
            *cols.pos_x.add(j) += dj.x;
            *cols.pos_y.add(j) += dj.y;
            *cols.pos_z.add(j) += dj.z;
        }
    }
}

/// Computes particle `i`'s bucket from its live position.
#[inline]
fn particle_bucket(body: &SoftBody, i: usize, inv_cell: f32, table: usize) -> usize {
    // A non-finite position saturating-casts to a finite (meaningless) cell index
    // rather than panicking (see `cell_coord`), but a stable sim never produces one —
    // catch it loudly in debug; no behaviour change in release.
    debug_assert!(
        body.pos_x[i].is_finite() && body.pos_y[i].is_finite() && body.pos_z[i].is_finite(),
        "invariant: self-collision build read a non-finite particle position"
    );
    let ix = cell_coord(body.pos_x[i], inv_cell);
    let iy = cell_coord(body.pos_y[i], inv_cell);
    let iz = cell_coord(body.pos_z[i], inv_cell);
    cell_hash(ix, iy, iz, table)
}

/// Debug-only warning when the `cell = 2·radius <= L0` precondition (architect N) is
/// violated — a neighbour one cell away could be a genuine distance-constraint partner
/// the bucketing misses. Release-safe (a missed pair only under-resolves), so this
/// only warns; it never aborts. No `log` crate dependency (architect SCOPE call);
/// compiled out in release.
#[cfg(debug_assertions)]
fn rest_len_warn(body: &SoftBody, cell: f32) {
    let l0 = body
        .c_rest
        .iter()
        .copied()
        .filter(|l| l.is_finite())
        .fold(None, |acc, l| Some(acc.map_or(l, |m: f32| if l < m { l } else { m })));
    if let Some(l0) = l0
        && cell > l0
    {
        rest_len_warn_cold(cell, l0);
    }
}

/// The cold print arm of [`rest_len_warn`] (kept off the hot inline path).
#[cfg(debug_assertions)]
#[cold]
fn rest_len_warn_cold(cell: f32, l0: f32) {
    eprintln!(
        "boyko_physics: SP3 self-collision precondition violated — cell size 2*radius \
         ({cell}) exceeds the smallest distance-constraint rest length L0 ({l0}); a \
         genuine one-cell-away constraint partner may be missed (under-resolved, not unsafe)"
    );
}

/// Release build: the precondition warning is compiled out.
#[cfg(not(debug_assertions))]
#[inline]
fn rest_len_warn(_body: &SoftBody, _cell: f32) {}

/// Debug-only warning when `radius <= 0` disables the self-collision pass (no `log`
/// crate dependency — architect SCOPE call). Compiled out in release.
#[cfg(debug_assertions)]
#[cold]
fn radius_warn(radius: f32) {
    eprintln!(
        "boyko_physics: SP3 self-collision skipped — particle_radius ({radius}) <= 0 \
         (a non-positive radius would form a degenerate cell size)"
    );
}

/// Release build: the radius warning is compiled out.
#[cfg(not(debug_assertions))]
#[inline]
fn radius_warn(_radius: f32) {}

/// Debug-only warning when the spatial-hash load factor is pathological — far more
/// particles than buckets means long bucket chains (quadratic candidate scans). No
/// `log` crate dependency (architect SCOPE call); compiled out in release.
#[cfg(debug_assertions)]
fn load_factor_warn(n: usize, table: usize) {
    // `table == next_pow2(2n)` ⇒ load factor ~0.5; this fires only if the invariant
    // is somehow violated (e.g. a hand-built body with mismatched scratch).
    if table != 0 && n > 4 * table {
        load_factor_warn_cold(n, table);
    }
}

/// The cold print arm of [`load_factor_warn`] (kept off the hot inline path).
#[cfg(debug_assertions)]
#[cold]
fn load_factor_warn_cold(n: usize, table: usize) {
    eprintln!(
        "boyko_physics: SP3 self-collision load factor pathological — {n} particles \
         in {table} buckets (expected ~0.5; bucket chains will be long)"
    );
}

/// Release build: the load-factor warning is compiled out.
#[cfg(not(debug_assertions))]
#[inline]
fn load_factor_warn(_n: usize, _table: usize) {}
