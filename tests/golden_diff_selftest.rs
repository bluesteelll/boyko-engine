//! Render wave PX0 (`px0/cut.md` PX0-7) — **`tools/golden_diff.py`'s self-tests inside
//! `cargo test --workspace`**, so the comparator every later HDR gate reads is gated by the same run
//! that gates the engine, and `.github/workflows/ci.yml` needs no edit.
//!
//! The interpreter is found by [`tool_runner::find_python`] (W6: a Store alias stub never counts);
//! no qualifying interpreter is a RED naming every candidate, never a skip. The run must report
//! every test of `tools/tests/test_golden_diff.py` by count, so an empty discovery is red too.

#[path = "tool_runner/mod.rs"]
mod tool_runner;

/// The self-tests the Python file defines (counted from its `def test_` lines, so a deleted test
/// shrinks the expectation and a discovery that ran fewer is red).
fn defined_tests() -> usize {
    let src = std::fs::read_to_string(tool_runner::repo_root().join("tools/tests/test_golden_diff.py"))
        .expect("tools/tests/test_golden_diff.py is readable");
    src.lines().filter(|l| l.trim_start().starts_with("def test_")).count()
}

#[test]
fn golden_diff_self_tests_pass() {
    let py = tool_runner::find_python().unwrap_or_else(|why| panic!("{why}"));
    let out = py
        .command()
        .args(["-B", "-m", "unittest", "discover", "-s", "tools/tests", "-p", "test_*.py"])
        .current_dir(tool_runner::repo_root())
        .output()
        .expect("the interpreter spawns");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let ran: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("Ran ")?.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    let want = defined_tests();
    assert!(want >= 18, "the self-test file defines {want} tests; the PX0-7 set is at least 18");
    assert!(
        out.status.success() && ran == want,
        "golden_diff.py self-tests ({}): exit {:?}, ran {ran} of {want}\n{text}",
        py.version,
        out.status.code()
    );
}

/// The discovery rule itself: a version answer is parsed, a stub's answer is not a version, and an
/// old interpreter is refused.
#[test]
fn the_discovery_rule_refuses_stubs_and_old_interpreters() {
    assert_eq!(tool_runner::python_minor("Python 3.14.5"), Some(14));
    assert_eq!(tool_runner::python_minor("Python 3.11"), Some(11));
    assert_eq!(tool_runner::python_minor("Python was not found; run without arguments to install"), None);
    assert_eq!(tool_runner::python_minor("Python 2.7.18"), None);
    assert!(tool_runner::python_minor("Python 3.10.2").is_some_and(|m| m < tool_runner::MIN_PYTHON_MINOR));
    assert_eq!(tool_runner::python_candidates()[0].0, if cfg!(windows) { "python" } else { "python3" });
}
