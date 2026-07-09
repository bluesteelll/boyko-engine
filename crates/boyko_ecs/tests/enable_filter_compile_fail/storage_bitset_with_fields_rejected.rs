// EnableTag D5 (Step 10 (3)): a `#[component(storage = "bitset")]` on a struct
// WITH fields is a compile error. A bitset enable tag has no `ComponentPool`, so
// any field data would have nowhere to live. The derive fails loud, requiring a
// fieldless struct (e.g. `struct Stunned;`).

use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct Bad(u32);

fn main() {}
