// GATES G0: `reflect_never` — G3 L3, the LINKED-UNUSED leg (built feature-ON), and
// G6c / G7a's twin T. The same consumer shape as `reflect_on` MINUS the derive's reflect
// key — and since CORE C8 (2026-08-21) that is the ONLY difference, which is the state
// G0's target table always described. The two corrections that got it here, kept because
// each was a measurement: ~~once CORE C7 lands: minus the key~~ → CORE D26, at C7
// `reflect_on` GAINS the key while KEEPING G0's temporary linkage fn, so this file was
// then minus both; ~~at C8 the linkage goes away because the derive's install slot puts
// G3's needle B back in the image~~ → CORE D27, that is true only because C8 also made
// `main()` CALL `component_id()`. The slot lives inside it, and an uncalled `#[inline]`
// method is dropped before the linker sees it. With the feature on, the optional
// `boyko_reflect` rlib is built and linked but reached by nothing — the leg that decides
// whether the census can tell "absent" from "carried along by the linker", which B.6
// measured is undecidable without `lto = "fat"` + `codegen-units = 1`
// (REFLECTION-ANALYSIS.md B.6: a plain fn in a dependency's rlib is codegen'd whether or
// not anything can reach it).

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_macros::Component;

/// Same shape as `reflect_on`'s `FixturePod`, never annotated, never linked to reflect.
///
/// `Default` is derived here for the same reason it is derived there — so this stays
/// *the same source minus the opt-in* rather than differing in a second way. The opt-in
/// is the derive's reflect key, which this file lacks and that one lacks nothing else
/// besides; nothing else about the two shapes may diverge, or L3 stops discriminating
/// what G3 says it discriminates.
///
/// **NOTHING IN THIS FILE MAY SPELL THE OPT-IN IN CODE.** The census's L3 non-collision
/// clause scans this file for every entry of `OPT_IN_TOKENS`, and a match is read as *"the
/// discriminator has collapsed into the present control"*.
///
/// ~~…IN PROSE OR IN CODE (CORE C8).~~ → **the prose half was a rule, and is now a
/// mechanism (C8 follow-up).** As first landed the clause scanned raw source text, so
/// writing the attribute out here as an *example* matched exactly like real code — observed
/// while landing C8, when the first draft of *this very paragraph* reddened the census. The
/// rule fixed this file and left the instrument alone, and the **L2** half of the same
/// `contains` list then failed in the silent direction: `reflect_on.rs`'s header spells the
/// attribute twice, and deleting its real annotation left that clause green. Both halves now
/// scan `code_only()`'s output, so comments are invisible to either direction. Quoting the
/// key here would no longer red the census — but naming it rather than spelling it is still
/// the clearer sentence, so this file keeps doing that.
#[derive(Component, Default)]
#[repr(C)]
pub struct FixturePod {
    /// One POD field so the derive has a real (non-ZST) subject.
    pub value: f32,
}

fn main() {
    let pod = FixturePod { value: 1.0 };
    core::hint::black_box(pod.value);

    // CORE C8 / D27 — the funnel touch, mirrored from `reflect_on.rs` because "nothing
    // else about the two shapes may diverge". Here it touches the SAME six pre-existing
    // install slots and no seventh, since this type carries no reflect key — which is
    // what makes L3's zeros a statement about the annotation rather than about the funnel
    // being absent from this image too.
    core::hint::black_box(<FixturePod as ComponentTrait>::component_id());

    // GATES G3 gate 5: the artifact reports its own configuration. `linkage=never`
    // is this bin's identity — the census's L3 leg asserts it, so pointing L3 at the
    // wrong fixture is caught by the artifact's own mouth as well as by the source scan.
    println!(
        "bin={} reflect_feature={} linkage=never",
        env!("CARGO_BIN_NAME"),
        if cfg!(feature = "reflect") { "on" } else { "off" },
    );
}
