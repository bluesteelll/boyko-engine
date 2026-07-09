//! P2 Test #5 — `compile_fail` acceptance tests for the `ui!` macro.
//!
//! Each `.rs` file in `tests/ui_macro_compile_fail/` must fail to compile with
//! the diagnostic recorded in its matching `.stderr` file. Regenerate the
//! `.stderr` files when adding/revising cases — and as the standard procedure on
//! toolchain bumps (snapshot compile-fail tests are toolchain-coupled) — via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ui --test ui_macro_compile_fail
//! ```
//!
//! Covered cases:
//!
//! Macro-time errors (span-precise `syn::Error`):
//! * `dup_name.rs`             — two `#foo` -> "duplicate ui name".
//! * `bare_name_item.rs`       — bare `#title` body item -> "a node reference …".
//! * `name_too_long.rs`        — 61-char `#name` -> "exceeds 60 bytes".
//! * `name_collides_commands.rs` — `#cmds` -> "collides with the commands binding".
//! * `children_not_last.rs`    — `children:` then a component -> "must be the last clause".
//! * `children_twice.rs`       — two `children:` clauses -> "must be the last
//!   clause" (the second `children:` is the trailing token after the first
//!   clause closes the body loop, so the not-last guard fires first; either
//!   diagnostic is correct — the snapshot records the actual one).
//! * `children_eq.rs`          — `children = [..]` -> "expected `:` after `children`".
//! * `inline_brace_node.rs`    — a `{..}` node among items -> "a child node must appear …".
//! * `bracket_array_item.rs`   — a bare `[..]` among items -> "a bare `[ ... ]` is not a component".
//! * `no_layout.rs`            — a node with no `UiLayout` -> "requires a `UiLayout`".
//! * `empty_node.rs`           — `{ }` -> "needs at least one component".
//!
//! Deferred-to-typecheck (span-forwarded to the user token):
//! * `bad_field.rs`            — `UiLayout { widht: .. }` -> E0560 at `widht`.
//! * `non_component.rs`        — a non-Component literal -> `Bundle` bound (E0277).
//! * `bad_field_in_bundle_slot.rs` — a wrong-type literal that lands in the
//!   `UiNodeBundle.layout` field -> a type mismatch pointing at the user's type.

#[cfg(not(miri))]
#[test]
fn ui_macro_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui_macro_compile_fail/*.rs");
}
