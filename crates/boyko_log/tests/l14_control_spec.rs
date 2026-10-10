//! L14: `apply_control_spec` -- unnamed targets untouched, bad specs refused whole, one epoch bump.

use boyko_log::control::{ControlSource, ControlSpecError, apply_control_spec, load_control};
use boyko_log::target::{LogTarget, TargetControl, control_epoch, set_target_control, target_control};
use boyko_log::{Ecs, Level, Log, Render};

#[test]
fn a_spec_writes_what_it_names_and_leaves_everything_else_bit_identical() {
    let ecs = <Ecs as LogTarget>::ID;
    let render = <Render as LogTarget>::ID;
    set_target_control(ecs, TargetControl::new(Level::Off, 0, false));
    set_target_control(render, TargetControl::new(Level::Warn, 3, true));
    let render_before = target_control(render);

    assert_eq!(apply_control_spec("ecs=debug/6!").expect("a well-formed spec"), 1);
    let c = target_control(ecs);
    assert_eq!(c.level(), Level::Debug);
    assert_eq!(c.sample_shift(), 6, "the shift after `/` was dropped");
    assert!(c.sync_route(), "the trailing `!` did not set the synchronous bit");

    // A SPEC IS NOT A SNAPSHOT. `ecs=..` says something about `ecs` and NOTHING about the other
    // 255 -- a parser that reset them would make every console command a silent teardown of
    // whatever the operator configured before it.
    assert_eq!(
        target_control(render).raw(),
        render_before.raw(),
        "a spec that never named `render` changed it"
    );

    an_unknown_name_refuses_the_whole_spec_including_the_clauses_that_parsed();
    one_bump_per_spec_and_applying_it_twice_is_idempotent();
    a_control_file_is_one_clause_per_line_and_a_missing_one_is_not_an_error();
}

/// A control FILE: one clause per line, `#` comments, and the same parser as the console.
fn a_control_file_is_one_clause_per_line_and_a_missing_one_is_not_an_error() {
    let ecs = <Ecs as LogTarget>::ID;
    let render = <Render as LogTarget>::ID;

    // ABSENT IS NOT AN ERROR. "No control file" is the ordinary case; a host that must tell it
    // from an empty one can check the path itself.
    let missing = std::env::temp_dir().join("boyko_l14_no_such_control.txt");
    let _ = std::fs::remove_file(&missing);
    assert_eq!(load_control(ControlSource::File(Box::leak(
        missing.to_str().expect("a UTF-8 temp path").to_string().into_boxed_str()
    ))).expect("a missing file is not an error"), 0);
    assert_eq!(load_control(ControlSource::None).expect("None applies nothing"), 0);

    // ONE CLAUSE PER LINE, because that is how an operator writes a file. Requiring commas in it
    // would make the file format a worse version of the console's.
    let path = std::env::temp_dir().join("boyko_l14_control.txt");
    std::fs::write(&path, "# what I am chasing
ecs=trace/2

render=off
")
        .expect("the temp dir is writable");
    let leaked: &'static str =
        Box::leak(path.to_str().expect("a UTF-8 temp path").to_string().into_boxed_str());
    assert_eq!(load_control(ControlSource::File(leaked)).expect("a well-formed file"), 2);
    assert_eq!(target_control(ecs).level(), Level::Trace);
    assert_eq!(target_control(ecs).sample_shift(), 2);
    assert_eq!(target_control(render).level(), Level::Off);

    // A FILE THAT EXISTS AND DOES NOT PARSE IS REFUSED. That is an operator's typo, and silence
    // about it is the failure this subsystem exists to remove -- so the good line before it is
    // NOT applied either.
    set_target_control(ecs, TargetControl::new(Level::Info, 0, false));
    let before = target_control(ecs).raw();
    std::fs::write(&path, "ecs=trace
nosuchtarget=info
").expect("writable");
    assert_eq!(load_control(ControlSource::File(leaked)), Err(ControlSpecError::UnknownTarget));
    assert_eq!(target_control(ecs).raw(), before, "a refused control file applied its first line");

    let _ = std::fs::remove_file(&path);
}

// NOT `#[test]`s of their own: all three mutate the ONE process-wide `CONTROL` table and read the
// ONE epoch counter, and `cargo test` runs `#[test]` functions in parallel threads. Measured -- the
// epoch assertions saw three other bumps arrive mid-test. Sequenced from the first.
fn an_unknown_name_refuses_the_whole_spec_including_the_clauses_that_parsed() {
    let ecs = <Ecs as LogTarget>::ID;
    set_target_control(ecs, TargetControl::new(Level::Info, 0, false));
    let before = target_control(ecs);
    let epoch_before = control_epoch();

    // The good clause comes FIRST. A parser that applied as it went would have written it before
    // reaching the bad one, leaving a partially-applied configuration the operator did not ask for
    // and cannot see -- which is how people stop trusting a console.
    assert_eq!(
        apply_control_spec("ecs=trace, nosuchtarget=info"),
        Err(ControlSpecError::UnknownTarget)
    );
    assert_eq!(
        target_control(ecs).raw(),
        before.raw(),
        "a refused spec applied its first clause anyway"
    );
    assert_eq!(control_epoch(), epoch_before, "a refused spec bumped the epoch");

    for (spec, want) in [
        ("ecs", ControlSpecError::MissingEquals),
        ("ecs=verbose", ControlSpecError::UnknownLevel),
        ("ecs=info/99", ControlSpecError::BadShift),
        ("ecs=info/x", ControlSpecError::BadShift),
        ("ecs=info trailing", ControlSpecError::Trailing),
    ] {
        assert_eq!(apply_control_spec(spec), Err(want), "spec {spec:?}");
    }
}

fn one_bump_per_spec_and_applying_it_twice_is_idempotent() {
    let log = <Log as LogTarget>::ID;
    let e0 = control_epoch();
    // THREE clauses, ONE bump. A poller sampling between two clauses of one command would act on
    // half of it.
    assert_eq!(apply_control_spec("log=info, ecs=warn, render=off").expect("well-formed"), 3);
    assert_eq!(control_epoch(), e0 + 1, "a three-clause spec bumped the epoch more than once");

    let after_first = target_control(log).raw();
    assert_eq!(apply_control_spec("log=info, ecs=warn, render=off").expect("well-formed"), 3);
    assert_eq!(
        target_control(log).raw(),
        after_first,
        "a clause names an absolute state, so re-applying it must land in the same bits"
    );

    // Empty is `Ok(0)` and changes nothing: the natural way to type "no changes" is to type
    // nothing, and rejecting it would make an empty console line an error message.
    let e2 = control_epoch();
    assert_eq!(apply_control_spec("").expect("an empty spec is not an error"), 0);
    assert_eq!(apply_control_spec("   ,  ").expect("separators alone name nothing"), 0);
    assert_eq!(control_epoch(), e2, "a spec that named nothing bumped the epoch");
}
