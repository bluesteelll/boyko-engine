//! Runtime control from a string: `"net=debug/6!, ecs=off"` *(L14)*.
//!
//! One grammar, three sources. A console command, an env var and a control file all deliver the
//! same text to the same parser, because the alternative is three parsers that disagree about what
//! `debug/6!` means and a support answer that has to ask which one the reader used.
//!
//! ```text
//! spec  := clause ("," clause)*
//! clause := name "=" level ["/" shift] ["!"]
//! level := off | error | warn | info | debug | trace
//! shift := 0..=15      -- sample 1 in 2^shift
//! "!"                  -- synchronous: this target's records bypass the ring
//! ```
//!
//! # Three properties, each of which is a way to get this wrong
//!
//! - **Unnamed targets are left BIT-IDENTICAL.** A spec is not a snapshot. `"ecs=debug"` says
//!   something about `ecs` and nothing about the other 255, and a parser that reset them would
//!   turn every console command into a silent teardown of whatever the operator set before.
//! - **An unknown name is REFUSED, and the whole spec is refused with it.** Applying the clauses
//!   that parsed and dropping the one that did not gives an operator a partially-applied
//!   configuration they did not ask for and cannot see -- the failure mode that makes people stop
//!   trusting a console. Nothing is written until every clause is understood.
//! - **ONE epoch bump for the whole spec.** A poller that samples between two clauses of one
//!   command would act on half of it. The bump happens after the last write.

use crate::level::Level;
use crate::target::{TargetControl, TargetId, bump_control_epoch, find_target, set_target_control_quiet};

/// Why a control spec was refused. Nothing was applied.
///
/// A typed value and not a `String`: the caller renders it, and a caller that wants to log it has
/// its own target and code to do so. Building a message here would put a formatting allocation
/// inside a function a console calls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlSpecError {
    /// A clause had no `=`, e.g. `"ecs"`.
    MissingEquals,
    /// The name before `=` matches no engine, dynamic or downstream target.
    UnknownTarget,
    /// The word after `=` is not a level name.
    UnknownLevel,
    /// The digits after `/` are not a shift in `0..=15`.
    BadShift,
    /// Trailing characters after the optional `!`.
    Trailing,
}

/// Parse one clause into what it would write, without writing it.
fn parse_clause(clause: &str) -> Result<(TargetId, TargetControl), ControlSpecError> {
    let clause = clause.trim();
    let (name, rest) = clause.split_once('=').ok_or(ControlSpecError::MissingEquals)?;
    let id = find_target(name.trim()).ok_or(ControlSpecError::UnknownTarget)?;

    let mut rest = rest.trim();
    let sync = rest.ends_with('!');
    if sync {
        rest = &rest[..rest.len() - 1];
    }
    let (level_word, shift) = match rest.split_once('/') {
        Some((l, s)) => {
            let shift: u8 = s.trim().parse().map_err(|_| ControlSpecError::BadShift)?;
            if shift > 15 {
                return Err(ControlSpecError::BadShift);
            }
            (l.trim(), shift)
        }
        None => (rest.trim(), 0),
    };
    if level_word.contains(char::is_whitespace) {
        return Err(ControlSpecError::Trailing);
    }
    let level = match level_word {
        "off" => Level::Off,
        "error" => Level::Error,
        "warn" => Level::Warn,
        "info" => Level::Info,
        "debug" => Level::Debug,
        "trace" => Level::Trace,
        _ => return Err(ControlSpecError::UnknownLevel),
    };
    Ok((id, TargetControl::new(level, shift, sync)))
}

/// Apply a control spec. Returns how many targets were written.
///
/// **Parsed in full before anything is written.** An unknown name in the last clause refuses the
/// first one too, because a half-applied configuration is worse than a rejected one: the operator
/// can see a rejection.
///
/// Idempotent by construction -- a clause names an absolute state, not a delta -- so applying the
/// same spec twice leaves the table bit-identical and bumps the epoch twice. The second bump is
/// honest: a poller cannot tell "applied again" from "applied differently" without re-reading, and
/// re-reading is what the bump asks it to do.
///
/// An empty spec is `Ok(0)`: nothing named, nothing changed. It is not an error, because the
/// natural way to type "make no changes" into a console is to type nothing.
pub fn apply_control_spec(spec: &str) -> Result<u32, ControlSpecError> {
    // Bounded by the target space: a spec cannot name more targets than exist, and this array is
    // the reason nothing is written during parsing. 256 pairs of (id, control) on the stack, not a
    // `Vec`, because a console command must not allocate to be understood.
    let mut staged: [Option<(TargetId, TargetControl)>; crate::target::MAX_TARGETS] =
        [None; crate::target::MAX_TARGETS];
    let mut n = 0usize;

    for clause in spec.split(',') {
        if clause.trim().is_empty() {
            continue;
        }
        let parsed = parse_clause(clause)?;
        if n >= staged.len() {
            // Unreachable while clauses name distinct targets; a spec that repeats one name can
            // still overflow, and silently dropping the tail would apply a spec nobody wrote.
            return Err(ControlSpecError::Trailing);
        }
        staged[n] = Some(parsed);
        n += 1;
    }

    for entry in staged.iter().take(n) {
        let (id, control) = entry.expect("invariant: the first `n` entries were just written");
        set_target_control_quiet(id, control);
    }
    if n > 0 {
        // ONE bump, after the last write. A poller that sampled between two clauses of one command
        // would act on half of it -- which is why the writes above are the QUIET setter: the
        // ordinary `set_target_control` bumps per call, and three clauses would be three epochs.
        bump_control_epoch();
    }
    Ok(n as u32)
}
