//! Miri-curated subset for P3 (`.ui`): the parse logic + the `UiName` diff-key
//! unsafe, exercised WITHOUT the engine's `Commands`/`CommandQueue` apply path.
//!
//! Why this split: the lowering / reconcile *apply* runs through
//! `boyko_ecs::CommandQueue`, whose `CursorSync` drop uses a raw-pointer cursor
//! pattern that Miri's *Stacked* Borrows over-approximates as UB (a PRE-EXISTING
//! engine characteristic, UNRELATED to P3 — the violation Miri reports is in
//! `boyko_ecs/.../command_queue.rs`, not any `boyko_ui` file; the engine is
//! validated under Tree Borrows per the project's soundness history). To keep a
//! meaningful, fast, genuinely-clean Miri signal for the P3-OWNED code, this file
//! drives only the code P3 itself authored:
//!   * `parse_ui` — the one-pass indentation parser (the `UiName::CAP` bound check
//!     before constructing a `UiNameStr`, the arena-flat node `Vec`s, the recovery
//!     branches);
//!   * `UiName::as_str` (the `from_utf8_unchecked` unsafe) + the hand-written
//!     `Ord` (the diff key, P3 Decision 9) — `Ord`-consistent-with-`Eq`.
//!
//! Run: `cargo +nightly miri test -p boyko-ui --test p3_miri`.

use boyko_ui::components::UiName;
use boyko_ui::text::parse_ui;

#[test]
fn miri_parse_small_tree() {
    let src = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(120), height: Px(24) }
    UiSpacing { padding_left: Px(8) }
    #child  UiLayout { layout_type: Row, width: Px(40), height: Px(40) }
        StackIndex(3)
";
    let tree = parse_ui(src);
    assert!(tree.report.is_clean(), "small tree parses clean: {:?}", tree.report.errors);
    assert_eq!(tree.roots.len(), 1, "one root");
    let root = &tree.nodes[tree.roots[0]];
    assert_eq!(root.name.as_ref().unwrap().text, "root");
    assert_eq!(root.children.len(), 1, "root has one child");
    let child = &tree.nodes[root.children[0]];
    assert_eq!(child.name.as_ref().unwrap().text, "child");
    // The child's StackIndex tuple component was captured.
    assert!(child.components.iter().any(|c| c.name == "StackIndex"), "StackIndex captured");
}

#[test]
fn miri_parse_malformed_recovers() {
    // Exercise the recovery branches (bad version, over-CAP name, dup name, bad
    // indent) — pure parse, no commands. Keep the input small for Miri.
    let long = "z".repeat(80);
    let src = format!(
        "version=abc\n#{long}  UiLayout {{ layout_type: Column }}\n#dup  UiLayout {{ layout_type: Column }}\n#dup  UiLayout {{ layout_type: Column }}\n   UiSpacing {{ }}\n"
    );
    let tree = parse_ui(&src);
    assert!(!tree.report.is_clean(), "the malformed corpus records errors");
    // Never panics; every root index is in-bounds.
    for &r in &tree.roots {
        assert!(r < tree.nodes.len(), "root index in bounds");
    }
}

#[test]
fn miri_parse_anonymous_and_comments() {
    // Comment stripping (the quote-aware two-byte `//` rule) + an anonymous node.
    let src = "\
// header comment
version=1
#root  UiLayout { layout_type: Column }   // trailing
    #named  UiLayout { layout_type: Row }
    UiLayout { layout_type: Row }   // anonymous sibling child
";
    let tree = parse_ui(src);
    assert!(tree.report.is_clean(), "comments + anon parse clean: {:?}", tree.report.errors);
    let root = &tree.nodes[tree.roots[0]];
    assert_eq!(root.children.len(), 2, "root has the named + anonymous child");
}

#[test]
fn miri_uiname_as_str_unsafe() {
    // The `UiName` unsafe (`from_utf8_unchecked` over the inline buffer prefix).
    let a = UiName::new("a");
    let empty = UiName::new("");
    assert_eq!(a.as_str(), "a", "as_str reads the inline buffer (from_utf8_unchecked)");
    assert_eq!(empty.as_str(), "", "empty name as_str");
    assert!(empty.is_empty(), "empty name is_empty");
    // Multibyte UTF-8 round-trips through the inline buffer (the unsafe slice must
    // land on a char boundary == len).
    let m = UiName::new("héllo");
    assert_eq!(m.as_str(), "héllo", "multibyte name round-trips through the unsafe");
    assert_eq!(m.len(), "héllo".len(), "len counts bytes, not chars");
    // At capacity.
    let cap = "x".repeat(UiName::CAP);
    let c = UiName::new(&cap);
    assert_eq!(c.as_str(), cap, "name at CAP round-trips");
}

#[test]
fn miri_uiname_ord_consistent_with_eq() {
    // The hand-written `Ord` (P3 Decision 9): compare the meaningful prefix,
    // tie-break on len. Must be consistent with the derived `Eq`.
    let a = UiName::new("a");
    let ab = UiName::new("ab");
    let b = UiName::new("b");
    assert!(a < ab, "a < ab (prefix then len)");
    assert!(ab < b, "ab < b");
    assert!(a < b, "a < b");
    assert_eq!(a.cmp(&UiName::new("a")), core::cmp::Ordering::Equal, "equal names compare Equal");
    assert_eq!(a, UiName::new("a"), "equal names are Eq");
    // Sorting a small set is a total order (no panic, deterministic).
    let mut v = [b, a, ab];
    v.sort();
    assert_eq!(v[0].as_str(), "a");
    assert_eq!(v[1].as_str(), "ab");
    assert_eq!(v[2].as_str(), "b");
}
