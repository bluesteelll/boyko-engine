// GATES G0: `reflect_never` — G3 L3, the LINKED-UNUSED leg (built feature-ON), and
// G6c / G7a's twin T. The same consumer shape as `reflect_on` MINUS the reflect linkage
// (once CORE C7 lands: minus the `reflect` key). With the feature on, the optional
// `boyko_reflect` rlib is built and linked but reached by nothing — the leg that decides
// whether the census can tell "absent" from "carried along by the linker", which B.6
// measured is undecidable without `lto = "fat"` + `codegen-units = 1`
// (REFLECTION-ANALYSIS.md B.6: a plain fn in a dependency's rlib is codegen'd whether or
// not anything can reach it).

use boyko_macros::Component;

/// Same shape as `reflect_on`'s `FixturePod`, never annotated, never linked to reflect.
#[derive(Component)]
#[repr(C)]
pub struct FixturePod {
    /// One POD field so the derive has a real (non-ZST) subject.
    pub value: f32,
}

fn main() {
    let pod = FixturePod { value: 1.0 };
    core::hint::black_box(pod.value);

    // GATES G3 gate 5: the artifact reports its own configuration. `linkage=never`
    // is this bin's identity — the census's L3 leg asserts it, so pointing L3 at the
    // wrong fixture is caught by the artifact's own mouth as well as by the source scan.
    println!(
        "bin={} reflect_feature={} linkage=never",
        env!("CARGO_BIN_NAME"),
        if cfg!(feature = "reflect") { "on" } else { "off" },
    );
}
