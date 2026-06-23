//! C + D(par) — CODEGEN-time rejection verification for the relation QUERY DSL.
//!
//! # Why this is not a trybuild test
//!
//! The `par_iter`-over-`Related` and dense-inner rejections are inline
//! `const { assert!(..) }` blocks inside GENERIC functions
//! (`par_iter.rs` for-each, `Related::init_state`). A const block in a generic fn
//! is evaluated only at MONOMORPHISATION (codegen) — `cargo check` (the mode
//! trybuild runs) does NOT instantiate it, so trybuild reports these cases as
//! "compiled successfully" even though `cargo build` / `cargo test` rejects them.
//! (Unlike the change-detection guard, the relation DSL exposes no public
//! const-fn `assert_*` callable in a `const _: () = ...` ITEM context, which is
//! what makes a const reject check-time-catchable — see FINDING-CODEGEN-ONLY-REJECT.)
//!
//! This test therefore drives the REAL codegen path: it compiles each isolated
//! reject source as a `cargo build --example -p boyko-ecs` target and asserts the
//! build FAILS with the guard's panic message. The reference sources live in
//! `tests/codegen_reject_relations_query/` (NOT a trybuild glob).
//!
//! Gated `#[cfg(not(miri))]` (spawns a compiler) and behind the `cfg(reject_codegen)`
//! opt-in is unnecessary — the test is cheap (two `cargo build` checks against the
//! already-compiled `boyko-ecs` rlib).

#![cfg(not(miri))]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Copies `src_name` from `tests/codegen_reject_relations_query/` into an
/// `examples/<example>.rs`, runs `cargo build -p boyko-ecs --example <example>`,
/// removes the example, and returns `(success, combined_output)`.
fn build_example_from_reject_src(src_name: &str, example: &str) -> (bool, String) {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_dir
        .join("tests")
        .join("codegen_reject_relations_query")
        .join(src_name);
    let examples_dir = manifest_dir.join("examples");
    fs::create_dir_all(&examples_dir).expect("create examples dir");
    let dst = examples_dir.join(format!("{example}.rs"));
    fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {src:?} -> {dst:?}: {e}"));

    let out = Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "boyko-ecs",
            "--example",
            example,
        ])
        .current_dir(&manifest_dir)
        .output()
        .expect("spawn cargo build");

    // Best-effort cleanup of the throwaway example source.
    let _ = fs::remove_file(&dst);
    // Remove the examples dir if we created it and it is now empty.
    let _ = fs::remove_dir(&examples_dir);

    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), combined)
}

#[test]
fn par_iter_over_related_is_codegen_rejected() {
    let (success, output) =
        build_example_from_reject_src("par_iter_rejects_related.rs", "cf_par_iter_related");
    assert!(
        !success,
        "par_iter over a Related<R,&T> query MUST fail to build (codegen const reject); \
         output:\n{output}"
    );
    assert!(
        output.contains("not supported on `par_iter`"),
        "the build failure must be the Related-par_iter guard message; output:\n{output}"
    );
}

#[test]
fn dense_inner_in_related_is_codegen_rejected() {
    let (success, output) =
        build_example_from_reject_src("dense_inner_rejected.rs", "cf_dense_inner");
    assert!(
        !success,
        "Related<R, &DenseComp> MUST fail to build (codegen const reject); output:\n{output}"
    );
    assert!(
        output.contains("DENSE inner component is not supported"),
        "the build failure must be the dense-inner guard message; output:\n{output}"
    );
}
