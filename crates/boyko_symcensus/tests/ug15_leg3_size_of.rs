//! UG-15 leg (3), `size_of` pins (03 §6): `EcsMaster`, `ComponentPool`, `Scope`.
//!
//! The pins are `ug15/pins/leg3.pins`. The recipe runs this test in `dev` AND in `--release`, and
//! both must equal the one pin: none of the three struct bodies carries a `cfg`-gated field (read
//! at B3), so a profile difference is a finding, not noise. Miri is N/A (`ComponentPool` is 144 B
//! there by design, `component_pool.rs`'s own assert).

use std::path::Path;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_threadpool::Scope;

fn measured() -> Vec<(&'static str, usize, usize)> {
    vec![
        ("EcsMaster", size_of::<EcsMaster>(), align_of::<EcsMaster>()),
        ("ComponentPool", size_of::<ComponentPool>(), align_of::<ComponentPool>()),
        ("Scope", size_of::<Scope<'static>>(), align_of::<Scope<'static>>()),
    ]
}

#[test]
fn leg3_sizes_equal_the_pins() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ug15/pins/leg3.pins");
    let text = std::fs::read_to_string(&path).expect("the leg-(3) pin file exists");
    let pinned: Vec<(String, usize, usize)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            let (Some(n), Some(s), Some(a)) = (it.next(), it.next(), it.next()) else { panic!("leg-(3) pin line not understood: {l}") };
            (n.to_owned(), s.parse().expect("size"), a.parse().expect("align"))
        })
        .collect();
    let got = measured();
    let profile = if cfg!(debug_assertions) { "debug-assertions on (dev)" } else { "debug-assertions off (release)" };
    println!("UG-15 leg (3) under {profile}: read {} type(s), {} pin(s)", got.len(), pinned.len());
    for (n, s, a) in &got {
        println!("  {n}\t{s}\t{a}");
    }
    assert_eq!(pinned.len(), got.len(), "leg (3): the pin file names {} types, the leg measures {}", pinned.len(), got.len());
    for ((n, s, a), (pn, ps, pa)) in got.iter().zip(&pinned) {
        assert_eq!((*n, *s, *a), (pn.as_str(), *ps, *pa), "leg (3) RED: {n} is ({s}, {a}), pinned ({ps}, {pa})");
    }
}
