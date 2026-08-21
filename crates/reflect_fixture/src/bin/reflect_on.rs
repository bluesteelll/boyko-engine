// GATES G0: the `reflect_on` fixture bin — G3 L2's PRESENT CONTROL. Built with
// `--features reflect`, this image must contain `boyko_reflect` symbols (needle A: the
// crate-name fragment; needle B: `install_type_info` — GATES D5). Without a present
// control beside the absent cell, "no symbol" in the ship leg is indistinguishable from
// "no fixture" (REFLECTION-ANALYSIS.md B.6).
//
// `reflect_off_twin` is THIS SAME FILE built feature-off (an explicit `[[bin]]` sharing
// this path): G3 L1 — the ship cell — and G7a's determinism null N1/N2.
//
// PLAN DEVIATION, deliberate and temporary. GATES G0's target table says this bin is
// "annotated (`#[component(reflect)]` on its components)" — but that derive key DOES NOT
// EXIST until REFLECTION-PLAN-CORE.md C7 lands it; today the Component derive hard-errors
// on unknown keys ("unknown #[component(...)] key", boyko_macros/src/component.rs:775).
// Until C7, the reflect linkage is the direct `#[cfg(feature = "reflect")]` reference
// below; ~~C7 swaps it for the annotation~~ → corrected 2026-08-21 (CORE D26): C7 ADDS
// the annotation BESIDE this reference, and C8 removes the reference. The swap cannot
// happen at C7 because G3's needle B is the literal name `install_type_info` and C7
// emits NO install call — swapping there would leave L2, the present control, with no
// needle-B subject, and no gate would red. C8 deletes the reference below and updates
// `OPT_IN_TOKENS` in tests/reflect_absence_census.rs in the same change.
// Implementer note for C7: adding the annotation reds D20's `ReflectDefault` witness on
// `FixturePod`, which has no `Default` — `#[derive(Default)]` goes on it (and on
// `reflect_never.rs`'s twin shape, or L3 stops being the same source minus the opt-in).

use boyko_macros::Component;

/// The fixture's consumer component: a stand-in for a game's own type (the
/// `profile-fixture` census argument, verbatim).
///
/// **Annotated at CORE C7** (landed 2026-08-21), BESIDE the linkage below rather than
/// instead of it (D26 — see the header comment). `#[derive(Default)]` is load-bearing,
/// not decoration: `#[component(reflect)]` bakes `default_in_place` from `Default` and
/// asserts the bound through `boyko_reflect::ReflectDefault`, so the annotation alone
/// reds this file on a type that has none.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
pub struct FixturePod {
    /// One POD field so the derive has a real (non-ZST) subject.
    pub value: f32,
}

fn main() {
    // Touch the component so the derive's output is genuinely compiled in BOTH feature
    // states and the `boyko-ecs`/`boyko-macros` edges are used, not decorative.
    let pod = FixturePod { value: 1.0 };
    core::hint::black_box(pod.value);

    #[cfg(feature = "reflect")]
    reflect_linkage();

    // GATES G3 gate 5: the artifact reports its own configuration, so the census can
    // assert "the build used the leg this test asked for" — the half only a harness
    // spawning its own build can check (`g14b`'s clause). `CARGO_BIN_NAME` distinguishes
    // the two `[[bin]]` targets sharing this file.
    println!(
        "bin={} reflect_feature={} linkage={}",
        env!("CARGO_BIN_NAME"),
        if cfg!(feature = "reflect") { "on" } else { "off" },
        if cfg!(feature = "reflect") { "present" } else { "absent" },
    );
}

/// Feature-ON linkage: force `boyko_reflect`'s symbols into this image so the G3 census
/// needles have a subject (GATES D5). Taking the fn's address is enough for the symbol
/// to exist; `TypeInfo` is opaque at G0, so a real call cannot be written yet.
#[cfg(feature = "reflect")]
#[inline(never)]
fn reflect_linkage() {
    let install: fn(usize, &'static boyko_reflect::TypeInfo) = boyko_reflect::install_type_info;
    core::hint::black_box(install);
}
