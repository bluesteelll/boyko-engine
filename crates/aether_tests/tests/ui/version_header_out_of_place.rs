//! §6.3 puts the version header FIRST, and the position matters for a reason a message has to
//! carry: a header below a construct means the constructs above it were already parsed against
//! whatever version the block defaulted to. Reported as an unknown construct (`aether` is not one
//! of the nine), the reader is told the keyword does not exist — which is false and unactionable.
use aether::aether;

aether! {
    component Health { hp: f32 }

    aether v1;
}

fn main() {}
