//! A deadline for every `dxc` process a re-DXC gate waits on, and a private, self-deleting scratch
//! path for every file such a gate hands to `dxc`.
//!
//! **Why this exists (observed 2026-09-17).** Inside `cargo test --workspace --all-targets`, one
//! `dxc.exe` spawned by `vb_lit_producer_spv_sync` (`vb_shade_split.comp.hlsl -D TEXTURED=1
//! -D HWRT=1`) ran a single thread at full CPU for 33.5 minutes and never exited. The identical
//! command run by hand finished in about a second with byte-identical output, and the same test
//! binary rerun alone passed in 4 s, so the compiler hung once rather than the shader being slow.
//! `Command::status()` and `Command::output()` wait forever, so that one process stalled the whole
//! workspace run until it was killed by hand.
//!
//! **Scope.** Several documents in this tree note that it has no kill-after-timeout pattern, so a
//! hang cannot be shown as a red (`crates/boyko_log/src/sync_out.rs`,
//! `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs`'s `GPU_ZONE_QUERY_FLAGS`,
//! `tests/gpu_blocking_reader_census.rs`). Those notes are about work inside the process, which
//! still cannot be killed. A CHILD process can be, and this module is the one place that does it,
//! for the `dxc` processes of the re-DXC gates. Their `spirv-dis` and `git` spawns are not covered.
//!
//! **What it provides.**
//! - [`BoundedRun`] adds `status_within` and `output_within` to [`Command`]. Each spawns the child
//!   with stdin closed, polls [`Child::try_wait`] until the deadline and, on expiry, kills the
//!   child, reaps it and PANICS with a message that begins [`HUNG_PREFIX`]. That message is
//!   distinct from a compile failure (the callers' `dxc failed …` assertions) and from a stale
//!   artifact (their `… is NOT the re-DXC …` assertions). A spawn error is still returned as `Err`,
//!   exactly as `status()` and `output()` return it, so each caller's
//!   `.expect("invariant: dxc was located and must run")` keeps its meaning.
//! - [`scratch_path`] returns a [`Scratch`]: a temp-directory path unique per process AND per
//!   call, whose file is deleted when the value drops, including while a panic unwinds. The gates
//!   used to name their temp files per variant only, so two worktrees running the suite at once
//!   (a normal state of this machine) wrote the same `%TEMP%` path; unique names alone would
//!   instead leave every failed run's files behind, which the drop-time delete prevents.
//!
//! **How it is included.** Cargo compiles each `tests/*.rs` as its own crate and does not treat
//! `tests/<dir>/mod.rs` as a test target, so each re-DXC file pulls this in with
//! `mod child_guard;`. The declaration sits at the END of each file, so that no line an internal
//! document cites moves. Every item here is used by every including file except
//! [`BoundedRun::status_within`], whose one-item `dead_code` allowance says why; there is no
//! module-wide allowance. `tests/child_guard_controls.rs` holds the negative controls.

use std::ffi::OsStr;
use std::io::{self, Read};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// The deadline every re-DXC gate passes. A standalone compile of the heaviest variant takes about
/// one second on the owner's workstation (2026-09-17), so two minutes leaves two orders of
/// magnitude for a loaded machine and still ends a hang long before anyone would notice it.
pub const DXC_DEADLINE: Duration = Duration::from_secs(120);

/// The first words of the panic a hung child produces, so a reader (and the controls) can tell a
/// hang from a failed compile or a stale artifact at a glance.
pub const HUNG_PREFIX: &str = "CHILD PROCESS HUNG";

/// How often a waiting test polls its child: short against [`DXC_DEADLINE`], long against a
/// scheduler tick.
const POLL: Duration = Duration::from_millis(20);

/// How long to wait for a finished or killed child's captured pipes to reach end-of-file. A
/// grandchild that inherited a pipe can hold it open after the child itself is gone.
const DRAIN: Duration = Duration::from_secs(10);

/// How much of a hung child's stderr the panic message quotes.
const STDERR_TAIL: usize = 2000;

/// `Command::status()` and `Command::output()` with a deadline. See the module doc.
pub trait BoundedRun {
    /// [`Command::status`] under `deadline`. Stdout and stderr are inherited, as `status()` does;
    /// stdin is closed. Panics, after killing and reaping the child, if it outlives `deadline`.
    // Each including file is its own crate, and two of them (`cluster_cull_hier_dis_gate`,
    // `vb_geo_preprocess_sync`) capture the output of every dxc they run, so in those two crates
    // this method is never called and rustc reports it unused. The allowance covers this one item.
    #[allow(dead_code)]
    fn status_within(&mut self, deadline: Duration) -> io::Result<ExitStatus>;

    /// [`Command::output`] under `deadline`. Stdout and stderr are captured, stdin is closed.
    /// Panics, after killing and reaping the child, if it outlives `deadline`.
    fn output_within(&mut self, deadline: Duration) -> io::Result<Output>;
}

impl BoundedRun for Command {
    fn status_within(&mut self, deadline: Duration) -> io::Result<ExitStatus> {
        let mut child = self.stdin(Stdio::null()).spawn()?;
        match wait_or_kill(&mut child, deadline)? {
            Waited::Exited(status) => Ok(status),
            Waited::Killed(hung) => panic!("{}", hung.message(self, deadline, None)),
        }
    }

    fn output_within(&mut self, deadline: Duration) -> io::Result<Output> {
        let mut child = self
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        match wait_or_kill(&mut child, deadline)? {
            Waited::Exited(status) => {
                let what = describe(self);
                Ok(Output {
                    status,
                    stdout: collect(&stdout, &what, "stdout"),
                    stderr: collect(&stderr, &what, "stderr"),
                })
            }
            Waited::Killed(hung) => {
                let tail = stderr.recv_timeout(DRAIN).ok();
                panic!("{}", hung.message(self, deadline, tail.as_deref()))
            }
        }
    }
}

/// A temp-directory path that no other process, and no other call in this process, is given.
/// `name` is kept as the suffix, so its extension (`.spv`, `.hlsl`, `.p.hlsl`) is preserved.
pub fn scratch_path(name: &str) -> Scratch {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    Scratch(std::env::temp_dir().join(format!("boyko-{}-{n}-{name}", std::process::id())))
}

/// A scratch file path whose file is removed when this value drops, on success and while a panic
/// unwinds alike. A process that is killed outright (no unwinding) still leaves its files behind.
/// Dereferences to [`Path`] and converts to the `AsRef` forms `Command::arg` and `std::fs` take.
#[derive(Debug, PartialEq, Eq)]
pub struct Scratch(PathBuf);

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for Scratch {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Absent is the normal case (the caller tidied up, or dxc never wrote the file).
        let _ = std::fs::remove_file(&self.0);
    }
}

/// How a bounded wait ended.
enum Waited {
    Exited(ExitStatus),
    Killed(Hung),
}

/// A child that outlived its deadline.
struct Hung {
    pid: u32,
    elapsed: Duration,
    /// The child's status after the kill, or the kill's own error (the child was then NOT reaped).
    reaped: io::Result<ExitStatus>,
}

impl Hung {
    fn message(&self, cmd: &Command, deadline: Duration, stderr: Option<&[u8]>) -> String {
        let tail = match stderr {
            Some(bytes) if !bytes.is_empty() => {
                let start = bytes.len().saturating_sub(STDERR_TAIL);
                format!(" Its stderr ended with: {}", String::from_utf8_lossy(&bytes[start..]))
            }
            Some(_) => " It wrote nothing to stderr.".to_string(),
            None => String::new(),
        };
        format!(
            "{HUNG_PREFIX}: `{}` was still running after {:.3} s (deadline {} s), so the watchdog \
             killed it (pid {}; kill, then reap: {:?}). The process hung: this is neither a failed \
             compile nor a stale artifact, so rerun the test binary.{tail}",
            describe(cmd),
            self.elapsed.as_secs_f64(),
            deadline.as_secs_f64(),
            self.pid,
            self.reaped,
        )
    }
}

/// Polls `child` until it exits or `deadline` passes; on expiry kills and reaps it.
fn wait_or_kill(child: &mut Child, deadline: Duration) -> io::Result<Waited> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Waited::Exited(status));
        }
        if start.elapsed() >= deadline {
            // `Child::kill` succeeds for a child that has already exited, so an error means the
            // kill itself failed. Waiting would then block on the very process the deadline gave
            // up on, so the error is reported in place of a reaped status.
            let reaped = child.kill().and_then(|()| child.wait());
            return Ok(Waited::Killed(Hung { pid: child.id(), elapsed: start.elapsed(), reaped }));
        }
        thread::sleep(POLL);
    }
}

/// Reads a captured pipe to end-of-file on its own thread, so a child that fills one pipe while
/// the test waits on the other cannot deadlock (the reason `Command::output` drains both at once).
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            // A read error leaves what was read so far, which is all a caller could use anyway.
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Receives a drained pipe of a child that has exited. A pipe still open after [`DRAIN`] means a
/// grandchild holds it; the capture would be incomplete, so that is a failure rather than a quiet
/// partial answer.
fn collect(rx: &Receiver<Vec<u8>>, what: &str, stream: &str) -> Vec<u8> {
    rx.recv_timeout(DRAIN).unwrap_or_else(|_| {
        panic!(
            "{HUNG_PREFIX}: `{what}` exited, but its {stream} was still open {} s later (a \
             grandchild process holds the pipe), so its captured output is incomplete.",
            DRAIN.as_secs()
        )
    })
}

/// The command as a reader would type it, with its working directory when one is set.
fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    if let Some(dir) = cmd.get_current_dir() {
        s.push_str(&format!(" (in {})", dir.display()));
    }
    s
}
