// The symmetric direction: `error!` handed a `WarnCode`.
//
// Both directions are covered because a one-sided gate proves only that SOME mismatch is refused,
// and the failure this guards against is a site reaching for the wrong constant in either
// direction. Expected diagnostic: `WarnCode` where `ErrorCode` was expected.

use boyko_log::Ecs;
use boyko_log::codes;

fn main() {
    boyko_log::error!(Ecs, codes::W1501, "an error site carrying a W-class code");
}
