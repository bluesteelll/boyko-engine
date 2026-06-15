// EnableTag D6 (Step 10 (2c)): a derived `#[component(storage = "bitset")]` tag
// is NOT a `Bundle`. The derive suppresses the single-component `Bundle`
// emission (`storage = "bitset"` implies `no_bundle`) because a bitset tag has
// no `ComponentPool` and must not be spawnable as a one-component bundle. So
// `Commands::spawn(Stunned)` / using `Stunned` where a `Bundle` is expected is a
// compile error: `Stunned: Bundle` is unsatisfied.

use boyko_ecs::ecs::core::bundle::bundle::Bundle;
use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct Stunned;

fn requires_bundle<B: Bundle>() {}

fn main() {
    requires_bundle::<Stunned>();
}
