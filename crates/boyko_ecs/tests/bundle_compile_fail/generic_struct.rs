// Phase 8.5 Step 9 — `#[derive(Bundle)]` on a generic struct must be
// rejected. Each `Bundle` impl owns one process-global
// `OnceLock<BundleStaticInfo>` slot; monomorphisation would create one
// slot per (G, T1, ..., Tn) tuple, defeating the cache and violating
// SBC2. The derive guards generics at macro time with a deterministic
// `compile_error!`.

use boyko_macros::Bundle;

#[derive(Bundle)]
struct G<T> {
    x: T,
}

fn main() {}
