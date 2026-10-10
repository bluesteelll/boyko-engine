// GATES G0: the `reflect_on` fixture bin — G3 L2's PRESENT CONTROL. Built with
// `--features reflect`, this image must contain `boyko_reflect` symbols (needle A: the
// crate-name fragment; needle B: `install_type_info` — GATES D5). Without a present
// control beside the absent cell, "no symbol" in the ship leg is indistinguishable from
// "no fixture" (REFLECTION-ANALYSIS.md B.6).
//
// `reflect_off_twin` is THIS SAME FILE built feature-off (an explicit `[[bin]]` sharing
// this path): G3 L1 — the ship cell — and G7a's determinism null N1/N2.
//
// THE PLAN DEVIATION IS RETIRED (CORE C8, 2026-08-21). GATES G0's target table says this
// bin is "annotated (`#[component(reflect)]` on its components)", and it now is, with
// nothing beside it. The history, kept because it is the argument for the shape below:
// the `reflect` derive key did not exist until CORE C7, so G0 landed a direct
// `#[cfg(feature = "reflect")]` fn-pointer reference (`reflect_linkage()`) to give G3's
// needle B — the literal name `install_type_info` — a subject. C7 ADDED the annotation
// BESIDE that reference rather than replacing it (CORE D26), because C7 emits no install
// call and the swap would have left L2, the present control, with no needle-B subject and
// no gate able to red. **C8 is the rung that emits the install**, so the reference is
// deleted here and needle B's subject is now the derive's own output.
//
// WHAT MAKES THAT TRUE IS THE `component_id()` TOUCH IN `main()`, AND ONLY THAT (CORE
// D27, measured). The install slot lives inside `component_id()`'s `get_or_init` closure,
// and an uncalled non-generic `#[inline]` method is dropped before the linker sees it —
// `llvm-nm` over this bin's three link configurations found ZERO occurrences of
// `component_id`, `register_new` and `install_hooks`, i.e. the six pre-existing slots
// were already absent and the seventh would have joined them. Deleting the linkage
// without the touch reds BOTH `l2_a > 0` and `l2_b > 0` in the census.
//
// `linkage=` in the self-report below now denotes the derive's emitted install rather
// than the retired fn; the value is `cfg!(feature = "reflect")` either way, which is the
// property the census's gate-5 clauses read.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_macros::Component;

/// The fixture's consumer component: a stand-in for a game's own type (the
/// `profile-fixture` census argument, verbatim).
///
/// **Annotated at CORE C7** (landed 2026-08-21) and **the sole reflect opt-in since CORE
/// C8**, which retired the linkage fn this annotation was landed beside (D26 — see the
/// header). `#[derive(Default)]` is load-bearing, not decoration: `#[component(reflect)]`
/// bakes `default_in_place` from `Default` and asserts the bound through
/// `boyko_reflect::ReflectDefault`, so the annotation alone reds this file on a type that
/// has none.
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

    // CORE C8 / D27 — THE FUNNEL TOUCH. Not decoration and not a smoke test: the reflect
    // install slot lives inside `component_id()`, so until something CALLS it the slot
    // reaches no image and G3's needle B has no subject. MEASURED before this line
    // existed: `component_id` occurred 0 times in every link configuration of this bin.
    //
    // Its RED is the deletion of this one statement. **It reds `l2_a > 0`, not `l2_b > 0`**
    // — corrected 2026-08-21 from the C8 landing's own run, which is the only place this
    // was measured rather than predicted. Deleting the touch drops the whole slot before
    // the linker sees it, so the pulled object goes with it and BOTH needles read 0; the
    // census asserts `l2_a > 0` ~35 lines ahead of `l2_b > 0`, so `l2_a` is the clause that
    // reports. The red is real and stronger than the earlier text claimed — it is simply
    // not `l2_b`'s.
    //
    // It also buys the property L1's zero did not have. With the funnel untouched, the
    // ship cell's zero was earned by the funnel's absence in BOTH images; with it touched,
    // the funnel is in both and only the emitted `#[cfg(feature = "reflect")]` separates
    // them — which is precisely what the ship-absence claim asserts.
    core::hint::black_box(<FixturePod as ComponentTrait>::component_id());

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
