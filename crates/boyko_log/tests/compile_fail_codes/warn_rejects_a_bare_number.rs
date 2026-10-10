// A bare `u16` is refused — and THIS is the case that matters.
//
// `.number()` on a real code is what every production site wrote, and it is what made the class /
// number pairing decorative: the newtype was unwrapped at the macro boundary and the class came
// from the macro's name. Refusing the unwrapped form is what makes the pairing hold; refusing only
// the wrong-class newtype would leave the four-character bypass open.
//
// Expected diagnostic: an integer where `WarnCode` was expected.

use boyko_log::Ecs;

fn main() {
    boyko_log::warn!(Ecs, 2103u16, "a warn site carrying a bare number");
}
