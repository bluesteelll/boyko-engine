# Physics performance campaign — orchestrator rulings (2026-09-19)

Authoritative text: `01-PLAN-REV1.md` + this file. The review (`02-REVIEW-OF-REV1.md`) found 1 Blocking and
4 Important remarks, each with an unambiguous "what is needed"; they are adopted here rather than in a second
loop.

## Owner decisions already taken (chat, 2026-09-18)

- "simd_solve включай": **D1 (the colored solver as the default path) and `simd_solve = true`** are decided.
  A separate lane (`perf/physics-colored-simd-default`) carries them.
- "широкая фаза, узкие цвета, patch friction и в целом все другие методики — максимум производительности":
  the value-changing levers **D2 (friction per patch), D3 (sleeping on by default), D4 (contact reuse)** are
  approved in principle. Each still lands only with its gates, its A/B, and its moved pins re-measured by
  their own rules; none is loosened to pass.

## Rulings on the review

- **B1 (the frozen-island warm-start test counted manifolds):** ADOPT. The test measures warm-start HITS per
  point (a hit counter, or the count of points whose seeded impulse is non-zero after `build_columns`); a
  twin pile that never slept is the control and must show hits == points at the same step (proves the
  assertion can pass); expected hits == 0 before the fix; the velocity check compares against the twin.
  If the test is red, the fix is designed in this lane (web research included: Box2D v3 keeps a sleeping
  island's contacts and impulses in its sleeping solver set) and lands as its own commit; it changes values
  on wake, so the sleeping-on suites' moved pins are listed and re-measured.
- **W1 (profile):** the headline parity numbers use our SHIPPED profile: `[profile.parity]` inherits
  `release` (fat LTO, default codegen units). A single-codegen-unit profile may exist separately for lever
  A/Bs; every number names its profile.
- **W2 (L6 fork always true):** the fork compares a FIXED per-wave dispatch cost (ω₁ from J-P1, or a
  microbenched spawn/join at zero work) against L(8), with the imbalance share (max chunk − mean chunk)
  reported separately.
- **W3 (levers gated on their own span):** every bit-identical lever (L2–L7) is gated end to end on median
  T(W), W ∈ {1, 2, 4, 8, 16}, on J and R, against the A/A0 band, with I(W) reported before and after.
- **W4 (instrument vacuity):** per-run anti-vacuity (each zone's count per step equals its structural
  expectation, else the row is void); `u < 0` joins closure; the validation tests get their own
  `harness = false` binary or the kernel's `exclusive()` discipline; the count test uses a fixture with at
  least one wide color; armed samples > 0 and disarmed samples = 0 are asserted.
- **Optional, all adopted:** O1 (no zone inside a worker's chunk task), O2 (the residue r in the identity,
  bounded), O3 (L5 uses per-chunk buffers concatenated in pair order; subtract the serial pre-read, the
  serial axis commit and the compaction; the `box_box` thread_local counters), O4 (H4 is a no-op; keep
  `-q=Discrete`), O5 (thread `-allow_sleep` into `StartTest`; compare tails after all asleep; record awake
  counts), O6 (one disarmed `shipping` row), O7 (L1's gate needs a pre-L1 binary; the B1 fix moves
  sleeping-on pins; the bench is already `harness = false`), O8 (label §0.2's "would widen the gap" a
  prediction).
- **Open questions:** (1) the runner builds exactly one world per process, or arms only the measured world;
  (2) record whether the dispatcher executes scope tasks at W=16, and state the thread counts on both sides.
