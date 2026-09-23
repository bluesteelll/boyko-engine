//! Fixtures for leg (2)/(7)'s normalisation (03 §6 "Normalisation", PC-2, PC-22): each unstable
//! spelling the probe build measured must normalise away, and each stable one must survive.

use std::collections::BTreeMap;

use boyko_symcensus::normalize::{self, ANON, EH_LABEL, RenameList};

#[test]
fn names_lose_their_unstable_parts_and_keep_their_stable_ones() {
    let cases = [
        // v0 crate disambiguators (PC-2), only after an identifier and before `]::`.
        ("boyko_ecs[4a1b2c3d]::ecs::f", "boyko_ecs::ecs::f"),
        ("<[f64]>::len", "<[f64]>::len"),
        // legacy hash and `.llvm.N`.
        ("boyko_ecs::ecs::f::h0123456789abcdef", "boyko_ecs::ecs::f"),
        ("boyko_ecs::ecs::f.llvm.1234567", "boyko_ecs::ecs::f"),
        // LLVM's local uniquifier, as the demangler prints it (PC-22).
        ("<boyko_ecs::X>::new (.4715)", "<boyko_ecs::X>::new"),
        ("<boyko_ecs::X>::new (.12) (.3)", "<boyko_ecs::X>::new"),
        // A name that merely ends in digits is not a uniquifier.
        ("boyko_ecs::f32_to_u8", "boyko_ecs::f32_to_u8"),
    ];
    for (raw, want) in cases {
        assert_eq!(normalize::norm_name(raw), want, "norm_name({raw:?})");
    }
}

#[test]
fn anonymous_data_folds_and_content_named_constants_do_not() {
    let map: BTreeMap<String, String> = BTreeMap::new();
    let none = RenameList::default();
    for raw in ["anon.d942e55fefdd14610d579d30fde3626d.8", ".rdata", ".rdata$r", "__unnamed_3", "alloc_7f", "str.0"] {
        assert!(normalize::is_anon(raw), "{raw} is anonymous");
        assert_eq!(normalize::norm_target(&map, raw, &none), ANON);
    }
    for raw in ["__real@3f800000", "__xmm@00000000000000000000000000000001", "memcpy", "__imp_GetTickCount"] {
        assert!(!normalize::is_anon(raw), "{raw} is named");
        assert_eq!(normalize::norm_target(&map, raw, &none), raw);
    }
}

#[test]
fn owned_compiler_data_is_named_after_its_owner() {
    let mut map = BTreeMap::new();
    map.insert("_RNvXYZ".to_owned(), "<boyko_ecs::X>::run".to_owned());
    assert_eq!(normalize::demangle(&map, "$ehgcr_117_4"), EH_LABEL);
    assert_eq!(normalize::demangle(&map, "$ehgcr_2_0"), EH_LABEL, "the function ordinal moves with unrelated code (PC-22)");
    assert_eq!(normalize::demangle(&map, "$cppxdata$_RNvXYZ"), "$cppxdata$<boyko_ecs::X>::run");
    assert_eq!(normalize::demangle(&map, "$handlerMap$0$_RNvXYZ"), "$handlerMap$0$<boyko_ecs::X>::run");
    assert_eq!(normalize::demangle(&map, "?dtor$14@?0?_RNvXYZ@4HA"), "?dtor$14@?0?<boyko_ecs::X>::run@4HA");
    assert_eq!(normalize::demangle(&map, "switch.table._RNvXYZ.3.rel"), "switch.table(<boyko_ecs::X>::run)");
    assert_eq!(normalize::demangle(&map, "$cppxdata$_RNvXYZ.4715"), "$cppxdata$<boyko_ecs::X>::run");
}

#[test]
fn a_rename_list_maps_bounded_occurrences_only() {
    let r = RenameList::parse("# control (x)\na::ecs_master::drain_runaway_panic -> a::drain_runaway::drain_runaway_panic\n")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.apply("a::ecs_master::drain_runaway_panic"), "a::drain_runaway::drain_runaway_panic");
    assert_eq!(r.apply("call a::ecs_master::drain_runaway_panic ; REL32"), "call a::drain_runaway::drain_runaway_panic ; REL32");
    assert_eq!(r.apply("a::ecs_master::drain_runaway_panic_twice"), "a::ecs_master::drain_runaway_panic_twice", "not glued on the right");
    assert_eq!(r.apply("xa::ecs_master::drain_runaway_panic"), "xa::ecs_master::drain_runaway_panic", "not glued on the left");
    assert!(RenameList::parse("no arrow here").is_err());
    assert!(RenameList::parse("a ->").is_err());
    assert_eq!(RenameList::parse("x → y").unwrap_or_else(|e| panic!("{e}")).0.len(), 1);
}
