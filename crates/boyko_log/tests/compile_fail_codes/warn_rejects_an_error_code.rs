// L10-B follow-up — a `warn!` handed an `E`-class code must NOT compile.
//
// This is the case `codes.rs`'s own test claimed was already impossible. It was not: the macros
// took `$code:expr` into a `u16` field while the CLASS byte came from the macro NAME, so
// `warn!(T, codes::E2103.number(), ..)` compiled and printed a `W`-class line carrying `2103` — one
// `explain(b'W', 2103)` cannot resolve, past every registry check, because all of them key on the
// IDENTIFIER in source rather than on what the sink prints.
//
// The macros now take the typed newtype and call `number()` themselves, which is what makes the
// pairing the compiler's job. Expected diagnostic: `ErrorCode` where `WarnCode` was expected, at
// this call site.

use boyko_log::Ecs;
use boyko_log::codes;

fn main() {
    boyko_log::warn!(Ecs, codes::E2103, "a warn site carrying an E-class code");
}
