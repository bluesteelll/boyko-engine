// EnableTag D5 (Step 10 (1) / W1-r6): an unknown `#[component(storage = "...")]`
// string is a compile error. The derive parses the value as a `LitStr` and
// rejects anything other than "bitset", naming the allowed value in the message
// so the user fixes the typo at the declaration.

use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "typo")]
struct Bad;

fn main() {}
