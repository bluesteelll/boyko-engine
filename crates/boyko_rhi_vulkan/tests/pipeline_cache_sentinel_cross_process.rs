//! BOOT-1 crash-loop guard against a REAL second process: the sentinel's lock owner is another OS
//! process, which holds the guard, clears it, or is killed.
//!
//! The lib's sentinel tests (`pipeline_cache::tests`) model "another boot" with a second handle in
//! the same process. Here each parent test spawns this binary's own [`sentinel_holder_child`] as
//! the boot that holds the guard, so liveness is the OS's verdict about a separate process, and a
//! kill (`TerminateProcess` on Windows, `SIGKILL` on Unix) is what a crash is. No GPU: every file
//! lives in a scratch directory under the temp dir, never the per-user cache.
//!
//! Run: `cargo test -p boyko_rhi_vulkan --test pipeline_cache_sentinel_cross_process`. Every
//! scratch directory carries the test's name and the process id, so the tests may run in parallel.

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use boyko_rhi_vulkan::pipeline_cache::{
    CacheLoad, CacheSave, DeviceIdentity, PipelineCacheStore, VK_HEADER_SIZE, save_payload,
};

/// The holder child's arming variable: the cache directory it arms. Unset, the child returns at once.
const HOLDER_DIR: &str = "BOYKO_PC_SENTINEL_HOLDER_DIR";
/// What the armed child does next: `clear` (clear and exit) or `hold` (hold until killed).
const HOLDER_MODE: &str = "BOYKO_PC_SENTINEL_HOLDER_MODE";
/// The line the child prints once it holds the guard.
const ARMED_LINE: &str = "BOOT1-SENTINEL-ARMED";
/// How long the parent waits for the child's [`ARMED_LINE`].
const ARM_DEADLINE: Duration = Duration::from_secs(30);
/// How long a `hold` child sleeps if nothing kills it: far past any parent test.
const HOLD: Duration = Duration::from_secs(120);
/// How long the parent polls for a killed holder's lock to drop.
const UNLOCK_DEADLINE: Duration = Duration::from_secs(10);

fn id() -> DeviceIdentity {
    DeviceIdentity {
        vendor_id: 0x10de,
        device_id: 0x2504,
        driver_version: 0x8_1234,
        pipeline_cache_uuid: *b"0123456789abcdef",
    }
}

/// A driver payload: a `VkPipelineCacheHeaderVersionOne` for [`id`], then `body` bytes of a
/// deterministic pattern (the lib tests' `payload`).
fn payload(body: usize) -> Vec<u8> {
    let id = id();
    let mut p = Vec::with_capacity(VK_HEADER_SIZE + body);
    p.extend_from_slice(&(VK_HEADER_SIZE as u32).to_le_bytes());
    p.extend_from_slice(&1u32.to_le_bytes());
    p.extend_from_slice(&id.vendor_id.to_le_bytes());
    p.extend_from_slice(&id.device_id.to_le_bytes());
    p.extend_from_slice(&id.pipeline_cache_uuid);
    p.extend((0..body).map(|i| (i.wrapping_mul(31) ^ (i >> 3)) as u8));
    p
}

fn scratch_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("boyko_pc_xproc_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("invariant: the temp dir accepts a subdirectory");
    d
}

/// A store over `dir` holding a saved, valid cache file.
fn seeded_store(dir: &Path) -> PipelineCacheStore {
    let store = PipelineCacheStore::new(dir, id());
    let mut scratch = Vec::new();
    assert!(matches!(save_payload(&store, &payload(512), &mut scratch), CacheSave::Saved { .. }));
    store
}

/// The holder child, spawned by the tests below: it arms the sentinel of [`HOLDER_DIR`], prints
/// [`ARMED_LINE`], then clears and exits (`clear`) or holds the guard until the parent kills it
/// (`hold`). Run as a plain test (the variables unset), it returns at once.
#[test]
#[cfg_attr(miri, ignore = "miri-unsupported: the holder child of the cross-process sentinel tests locks a file")]
fn sentinel_holder_child() {
    let (Some(dir), Some(mode)) = (std::env::var_os(HOLDER_DIR), std::env::var_os(HOLDER_MODE)) else {
        return;
    };
    let store = PipelineCacheStore::new(Path::new(&dir), id());
    let guard = store.arm_sentinel().expect("the holder child: an unheld sentinel arms");
    println!("{ARMED_LINE}");
    if mode == "clear" {
        guard.clear();
        return;
    }
    // `hold`: the parent kills this process, so the guard is never cleared.
    std::thread::sleep(HOLD);
    drop(guard);
}

/// Spawns [`sentinel_holder_child`] over `dir` in `mode` and returns once it holds the guard.
fn spawn_holder(dir: &Path, mode: &str) -> Child {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut child = Command::new(exe)
        .args(["--exact", "sentinel_holder_child", "--nocapture", "--test-threads=1"])
        .env(HOLDER_DIR, dir)
        .env(HOLDER_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("invariant: the test binary can spawn a copy of itself");
    let out = child.stdout.take().expect("invariant: stdout is piped");
    let mut lines = BufReader::new(out).lines();
    let t0 = Instant::now();
    loop {
        let line = lines
            .next()
            .expect("the holder child ended before it armed")
            .expect("invariant: the child's stdout is readable");
        if line.contains(ARMED_LINE) {
            break;
        }
        assert!(t0.elapsed() < ARM_DEADLINE, "the holder child never armed");
    }
    // Keep draining the pipe so the child never blocks on a full one.
    std::thread::spawn(move || for _ in lines {});
    child
}

/// Review r1 W1 across processes: a live second process holds the guard, so a load here is a plain
/// warm load. Nothing is discarded, and this process cannot take the live guard.
#[test]
#[cfg_attr(miri, ignore = "miri-unsupported: spawns a child process and locks a file")]
fn a_live_holder_process_is_not_a_crash() {
    let dir = scratch_dir("live");
    let store = seeded_store(&dir);
    let mut child = spawn_holder(&dir, "hold");
    let (load, bytes) = store.load();
    let still_there = store.path().exists();
    let second = store.arm_sentinel().is_some();
    let _ = child.kill();
    let _ = child.wait();
    assert!(matches!(load, CacheLoad::Loaded { .. }), "a live owner in another process is not a crash: {load:?}");
    assert!(!bytes.is_empty(), "the whole file is handed over");
    assert!(still_there, "a live process's file is never discarded");
    assert!(!second, "a second process cannot take the live guard");
    let _ = fs::remove_dir_all(&dir);
}

/// The holder process is killed: the OS drops its lock, its body stays armed, and the next load
/// reads exactly one crash, then a plain cold boot.
#[test]
#[cfg_attr(miri, ignore = "miri-unsupported: spawns a child process and locks a file")]
fn a_killed_holder_process_is_one_crash() {
    let dir = scratch_dir("killed");
    let store = seeded_store(&dir);
    let mut child = spawn_holder(&dir, "hold");
    child.kill().expect("invariant: the live holder child can be killed");
    child.wait().expect("invariant: the killed child is reapable");
    // Windows releases a dead process's locks asynchronously (`LockFileEx` remarks): poll, bounded.
    // A load that races the release reads a live owner and changes nothing.
    let t0 = Instant::now();
    let mut seen = Vec::new();
    let crashed = loop {
        let (load, _) = store.load();
        seen.push(format!("{load:?}@{}ms", t0.elapsed().as_millis()));
        if load == CacheLoad::CrashGuard {
            break true;
        }
        if t0.elapsed() > UNLOCK_DEADLINE {
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    println!("BOOT1-SENTINEL-POLL {}", seen.join(" "));
    assert!(crashed, "a killed owner never read as a crash: {seen:?}");
    assert!(!store.path().exists(), "the crashed boot's blob is discarded");
    assert_eq!(store.load().0, CacheLoad::Absent, "the crash is consumed once");
    let _ = fs::remove_dir_all(&dir);
}

/// A holder process that clears its guard and exits leaves a warm file: no crash.
#[test]
#[cfg_attr(miri, ignore = "miri-unsupported: spawns a child process and locks a file")]
fn a_holder_process_that_clears_is_no_crash() {
    let dir = scratch_dir("cleared");
    let store = seeded_store(&dir);
    let mut child = spawn_holder(&dir, "clear");
    child.wait().expect("invariant: the clearing child exits");
    let (load, _) = store.load();
    assert!(matches!(load, CacheLoad::Loaded { .. }), "a cleared guard is not a crash: {load:?}");
    assert!(store.path().exists(), "a cleared guard leaves the file in place");
    let _ = fs::remove_dir_all(&dir);
}
