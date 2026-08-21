// GATES G0: `reflect_never` — G3 L3, the LINKED-UNUSED leg (built feature-ON), and
// G6c / G7a's twin T. The same consumer shape as `reflect_on` MINUS the reflect linkage
// (~~once CORE C7 lands: minus the `reflect` key~~ → corrected 2026-08-21, CORE D26: at
// C7 `reflect_on` GAINS the `reflect` key while KEEPING the linkage, so this file is then
// minus both; at C8, which is where the derive's install slot puts G3's needle B back in
// the image, the linkage goes away and this file is minus the key alone). With the feature on, the optional
// `boyko_reflect` rlib is built and linked but reached by nothing — the leg that decides
// whether the census can tell "absent" from "carried along by the linker", which B.6
// measured is undecidable without `lto = "fat"` + `codegen-units = 1`
// (REFLECTION-ANALYSIS.md B.6: a plain fn in a dependency's rlib is codegen'd whether or
// not anything can reach it).

use boyko_macros::Component;

/// Same shape as `reflect_on`'s `FixturePod`, never annotated, never linked to reflect.
///
/// `Default` is derived here for the same reason it is derived there — so this stays
/// *the same source minus the opt-in* rather than differing in a second way. The opt-in
/// is the `#[component(reflect)]` key and the linkage fn, both of which this file lacks;
/// nothing else about the two shapes may diverge, or L3 stops discriminating what G3
/// says it discriminates.
#[derive(Component, Default)]
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
