VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=1

# Architecture review: L11, per-contact solve setup

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. One Important remark. Its fix is local and does not reopen the design.

I checked the plan's cited facts against `D:/wt/joltab` and they hold. The body-body stream is sorted, and the grid dedups pairs (`resources.rs:1208`, `:1580`). There is one SDF manifold per row (`systems.rs:606-634`). Frozen status is decided per island (`colored.rs:1863-1871`). `vn_initial` has only the restitution reader (`:3146-3149`). `t2 = n.cross(t1)` (`contact.rs:67`), and `cross8` repeats it op for op (`simd.rs:557-559`). Cohorts are 8-group windows counted from each colour's first group on all three paths (`colored.rs:2756-2769`, `:2880-2897`, `:3008-3017`). Every manifold gets a colour (`resources.rs:2964`). Hand-built streams are unsorted (`colored_tests.rs:796-810` emits `(a, floor)` before `(a, a+1)`).

I found no determinism hole on production streams, which are strictly sorted.

## 🟡 Important

### W1. The data model cannot express the non-strict lookup that D2 and Lemma W need
**Where:**
- D2: "scans equal-key records by descending `mi`";
- the `Plan` struct: `src: u32`;
- P-c: "fid scan of `recs_r[src]`, last match";
- D3: "copy per point, by fid, from `recs_r[src]`".

**Problem:** Today the table value for `(pair, f)` is the **last insert of that point key**. The store inserts in canonical order (`colored.rs:3238-3245`), and an insert overwrites an equal key (`warm_start.rs:355-359`). Suppose two manifolds m1 < m2 have the same pair, and only m1 has fid f. Today's lookup returns m1's entry. D2 describes the right scan: walk the equal-key run by descending `mi`. But `Plan` carries one `src`, and both the fill and the carry scan only that one record. So they miss f.

**Consequence:** Take a duplicate-pair stream where the later manifold has fewer points, e.g. `box_manifold(lo, hi, …, 4)` followed by `box_manifold(lo, hi, …, 2)`. The fids are `0..n` (`colored_tests.rs:74-84`), and `random_scene` can emit exactly this (`:404-431`). Fids 2 and 3 then seed 0 where today they seed a value, and `point_hits` / `carry_hits` differ. G2 and S-e would go red at C1, and C1 cannot go green without changing the design mid-implementation. There is a worse outcome if G2's generator is "fixed" instead: `solve_colored` is public, so any caller that passes a duplicate-pair stream would then diverge silently from today.

**Confidence:** CONFIRMED, from the plan text and the code lines cited.

**What is needed:**
- `plan` must name the whole equal-key run (for example a cold-index position plus the run end), not one record.
- The P-c seed and the D3 carry must both go through the single lookup routine D2 specifies.
- Add a named mutation, "fill/carry scan only the last equal-key record", recorded red in G2.

## 🟢 Optional

- **O1. The SIMD warm-apply mask (D7 says "with movable masks").**
  - A padding rank of a present lane has λ = 0 but real `n` and `t1`. The impulse therefore has ±0 components, and applying it turns a velocity or angular component that is exactly −0.0 into +0.0.
  - The scalar walk never visits padding ranks, so the "bit-identical" claim for C3 fails for signed zeros.
  - The mask must be `active ∧ movable`, as in the kernel (`colored.rs:2481-2482`). Add a mutation ("movable-only mask") and a scene seeded with a −0.0 component. CONFIRMED from the text; the consequence is limited to signed-zero bits.
- **O2. Absent-lane body reads.** D6 says "a foreign read is impossible". That holds for cohort blocks, but not for bodies. Padding lanes carry body id 0, which is a real row, and another chunk of the same colour may be writing it. Today the gather skips lanes ≥ `nlanes` (`colored.rs:2364-2397`). State the invariant "lanes ≥ nlanes read no body row" for the kernel and for any future gather over `head.body_a[8]`. A value-identical race is invisible to the {1,N} bit oracles; that is this codebase's own SP4/C1 lesson. Give it a Miri multi-thread case that can fail. PLAUSIBLE.
- **O3. The D9 predicate for NaN.** Today's consumer skips on `restitution <= 0.0` (`:3146`). "Compute vn0 iff restitution > 0" differs from that for a NaN coefficient. Use the exact complement, `!(e <= 0.0)`. CONFIRMED; it affects NaN materials only.
- **O4. The port inventory.** The plan says `:264` and `:443` "stay", but neither compiles after C1/C2:
  - both call `cols.canonical()` (`colored_tests.rs:348`, `:494-501`), which C1 deletes;
  - both call the per-slot `cols.body_a(s)` (`:336-340`, `:477-480`), which C2 removes.

  List them, and list the retired canonical-permutation assertion as deleted rather than loosened. CONFIRMED.
- **O5. The L10 mapping row "kept_warm → `recs_r[src[mi]]` at move-in".** A moved-in island's manifolds are withheld from the step-t stream (L10 A2.4/A3), so they have no current `mi` and no `plan.src`. Their source is `recs_r` at the t−1 stream index, i.e. `graph(t−1).island(a)`. That is simpler still, but the row as written names the wrong index space. The reorder also changes L10's closed `KeptWarm` / `restore_warm` types, so it needs a ruling. CONFIRMED against `L10-sleeping/04-DESIGN-REV2.md:330-335`.

## Positive (keep)
- Lemma W is derived from the real insert order: solved manifolds first, then frozen, with per-point last-write-wins (`:3238-3256`, `:3282-3319`). Both kinds of duplicate (fids and keys) are taken seriously.
- The merge-join uses L10's own D6 ordinal. Sortedness is a fast path, not a premise, which is correct given `colored_tests.rs:789-815`.
- The dispatch structure is untouched: cohorts, point quotas and `group_start`. So the census and scope pins genuinely stay.
- D8 is right to reject both fusing the warm apply into the first sweep and once-per-step masses. Inertia is refreshed every substep (`:3597-3600`).
- Retiring `:966` is right: it passes when its `D:/tmp` file is absent.
- The "cannot claim" section is honest: 1.68–1.90× v5.6.0 per contact remains, and it is collision detection.
- No reuse criterion is introduced. Records are rebuilt every step from solved values and fid-matched as today, so there is no drift and no missed feature (no L9 risk).

**Topics absent, with no consequence:**
- prefetch: colour-order reads ascend within a colour;
- non-temporal stores: every write is re-read in the same step;
- loom: no new atomics protocol;
- PGO;
- FMA: the census covers it.

## Open questions
1. **Where the setup-gain split comes from.** The split (hash probes ≈ 15–20 ns per point, push overhead ≈ 42 ns per point) comes from an uncommitted witness; grep finds nothing in `docs/`. The headline "1.03–1.17× v5.3.0" depends on it. Add a C1-only armed A/B, or C0 sub-spans, so the C1 and C2 shares are measured before the C2 rewrite.
2. **The D7 budget may be exceeded.** The existing kernel costs about 522 ns per cohort per sweep (3.576 ms / 12 sweeps / ~571 cohorts). By my arithmetic, gathering and scattering 16 bodies is plausibly 100–160 ns of that. Over 2,284 cohort passes that alone is 0.23–0.37 ms, at or above D7's whole 0.15–0.30 ms budget. The −0.19 ms gate may therefore fail. Measure the kernel's gather/scatter share before C3. PLAUSIBLE.
3. For the orchestrator: the lane reorder (L11 C1–C2 before L10 C3a) changes the ruled order "L5 → broadphase → L10" and reopens L10's types.
4. For the orchestrator: whether to commission L12. It is plausibly worth more per contact than C3.

Files:
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`
- `D:/wt/joltab/crates/boyko_physics/src/resources.rs`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`