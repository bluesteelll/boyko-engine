// Phase 8.5 Step 9 — `#[derive(Bundle)]` on a unit struct must be rejected
// with "Bundle requires at least one field". A bundle with no fields can
// neither carry any Component data nor populate an archetype slot, so the
// derive intercepts it at macro time before the rest of the impl runs.

use boyko_macros::Bundle;

#[derive(Bundle)]
struct Marker;

fn main() {}
