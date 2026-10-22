//! BOOT-1 F-A runner gates: the windowed runner's persisted pipeline cache, end to end.
//!
//! Each parent test spawns this binary's own worker [`boot_cache_child`] — a room-class scene
//! through `EnginePlugins::window`, hidden, three frames — with `BOYKO_PIPELINE_CACHE_DIR` pointed
//! at a scratch directory under `CARGO_TARGET_TMPDIR`, so no run touches the per-user cache. The
//! worker prints one `BOOT1-STATS` line from `HostBootStats` and `HostTeardownStats`.
//!
//! Run: `cargo test -p boyko-app --test boot_pipeline_cache -- --ignored --test-threads=1`.

#![cfg(windows)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use boyko_app::prelude::*;
use boyko_rhi_vulkan::pipeline_cache::{CacheLoad, CacheSave};

/// The worker's arming variable: unset, the worker returns at once.
const CHILD_MARKER: &str = "BOYKO_BOOT1_CACHE_CHILD";
/// One worker's budget: a cold boot plus three frames and teardown.
const WORKER_DEADLINE: Duration = Duration::from_secs(180);

/// The room of `room_smoke.rs`, reduced: a floor, four cubes, a sun, a sky fill and a camera.
fn room(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }
    let sun = [-0.45, 0.82, 0.36];
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(sun[0], sun[1], sun[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(sun, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 320.0 / 240.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

#[test]
#[ignore = "gpu-windowed: worker spawned by the boot_pipeline_cache gates; returns at once unless BOYKO_BOOT1_CACHE_CHILD is set"]
fn boot_cache_child() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        eprintln!("SKIP boot_cache_child: {CHILD_MARKER} is unset (the boot_pipeline_cache gates spawn it)");
        return;
    }
    let mut app = App::new();
    app.add_startup_system(room);
    app.add_plugins(EnginePlugins::window("boyko boot1 cache child", 320, 240));
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    let _ = app.run();
    let boot = app.world().try_resource::<HostBootStats>().copied().unwrap_or_default();
    let teardown = app.world().try_resource::<HostTeardownStats>().copied().unwrap_or_default();
    let saved = match boot.saved {
        CacheSave::Saved { bytes, .. } => bytes,
        _ => 0,
    };
    // One word per value: the parent splits the line on whitespace.
    let load = match boot.cache_load {
        CacheLoad::Off => "Off",
        CacheLoad::Ephemeral => "Ephemeral",
        CacheLoad::Absent => "Absent",
        CacheLoad::Loaded { .. } => "Loaded",
        CacheLoad::Rejected(_) => "Rejected",
        CacheLoad::CrashGuard => "CrashGuard",
        CacheLoad::DriverRefused => "DriverRefused",
        CacheLoad::CreateFailed => "CreateFailed",
    };
    println!(
        "\nBOOT1-STATS sampled={} pipelines={} hits={} invalid={} saved={} load={load} created_total={} teardown_sampled={}",
        boot.sampled,
        boot.pipelines,
        boot.cache_hits,
        boot.feedback_invalid,
        saved,
        teardown.pipelines_created_total,
        teardown.sampled
    );
    let _ = std::io::stdout().flush();
}

/// One worker's `BOOT1-STATS` line, parsed.
#[derive(Debug, Default)]
struct Stats {
    sampled: bool,
    pipelines: u32,
    hits: u32,
    invalid: u32,
    saved: u64,
    load: String,
    created_total: u32,
    raw: String,
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .unwrap_or("")
}

fn parse(stdout: &str) -> Option<Stats> {
    let line = stdout.lines().find_map(|l| l.find("BOOT1-STATS ").map(|i| &l[i..]))?;
    Some(Stats {
        sampled: field(line, "sampled") == "true",
        pipelines: field(line, "pipelines").parse().ok()?,
        hits: field(line, "hits").parse().ok()?,
        invalid: field(line, "invalid").parse().ok()?,
        saved: field(line, "saved").parse().ok()?,
        load: field(line, "load").to_string(),
        created_total: field(line, "created_total").parse().ok()?,
        raw: line.to_string(),
    })
}

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("boot1_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("invariant: the target tmp dir accepts a subdirectory");
    d
}

/// Runs the worker from `exe` with the cache directory `cache_dir` (and `extra` env).
fn run_worker(exe: &Path, cache_dir: &Path, label: &str, extra: &[(&str, &str)]) -> Stats {
    let log_dir = cache_dir.parent().expect("invariant: the cache dir has a parent").join("logs");
    std::fs::create_dir_all(&log_dir).expect("invariant: the logs dir is creatable");
    let out_path = log_dir.join(format!("{label}.stdout.txt"));
    let err_path = log_dir.join(format!("{label}.stderr.txt"));
    let out = std::fs::File::create(&out_path).expect("invariant: the log is writable");
    let err = std::fs::File::create(&err_path).expect("invariant: the log is writable");
    let mut cmd = Command::new(exe);
    cmd.args(["boot_cache_child", "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("BOYKO_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env(CHILD_MARKER, "1")
        .env("BOYKO_PIPELINE_CACHE_DIR", cache_dir)
        .env("BOYKO_WIN_HIDDEN", "1")
        .env("BOYKO_WINDOW_FRAMES", "3")
        .env("BOYKO_DISABLE_VALIDATION", "1");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("invariant: the test binary can spawn a copy of itself");
    let deadline = Instant::now() + WORKER_DEADLINE;
    let status = loop {
        if let Some(s) = child.try_wait().expect("invariant: the child is waitable") {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{label}: the worker hung past {WORKER_DEADLINE:?} (stderr: {})", err_path.display());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    assert!(
        status.success(),
        "{label}: the worker exited {status} (stdout {}, stderr {})",
        out_path.display(),
        err_path.display()
    );
    let stats = parse(&stdout).unwrap_or_else(|| panic!("{label}: no BOOT1-STATS line in {}", out_path.display()));
    assert!(stats.sampled, "{label}: HostBootStats was never written: {}", stats.raw);
    eprintln!("{label}: {}", stats.raw);
    stats
}

fn cache_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "vkpc"))
}

#[test]
#[ignore = "gpu-windowed: spawns hidden windowed workers; run with --test-threads=1"]
fn second_boot_hits_every_pipeline() {
    let root = scratch("second_boot");
    let dir = root.join("pc");
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");

    let cold = run_worker(&exe, &dir, "child1_cold", &[]);
    assert_eq!(cold.load, "Absent", "child 1 starts with no file: {}", cold.raw);
    assert!(cold.pipelines >= 59, "a room boot builds the engine's 59-pipeline boot set: {}", cold.raw);
    assert!(cold.saved > 0, "child 1 saves the cache at boot end: {}", cold.raw);
    assert!(cache_file(&dir).is_some(), "child 1 left a .vkpc file in {}", dir.display());

    let warm = run_worker(&exe, &dir, "child2_warm", &[]);
    assert_eq!(warm.load, "Loaded", "child 2 loads the file: {}", warm.raw);
    assert_eq!(warm.pipelines, cold.pipelines, "the same scene builds the same set");
    assert_eq!(warm.hits, warm.pipelines, "child 2: every pipeline is a cache hit: {}", warm.raw);
    assert_eq!(warm.saved, 0, "child 2: a warm boot writes nothing: {}", warm.raw);

    // Check I2: a rebuilt / relocated exe keeps the cache warm — the key is never the exe.
    let copy_dir = root.join("copy");
    std::fs::create_dir_all(&copy_dir).expect("invariant: scratch is writable");
    let copy = copy_dir.join("boot_pipeline_cache_copy.exe");
    std::fs::copy(&exe, &copy).expect("invariant: the test exe is copyable");
    let moved = run_worker(&copy, &dir, "child2b_copied_exe", &[]);
    assert_eq!(moved.hits, moved.pipelines, "child 2b (another exe path): every pipeline hits: {}", moved.raw);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "gpu-windowed: spawns hidden windowed workers; run with --test-threads=1"]
fn off_switch_leaves_the_file_alone() {
    let root = scratch("off_switch");
    let dir = root.join("pc");
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let _ = run_worker(&exe, &dir, "seed", &[]);
    let file = cache_file(&dir).expect("the seed run wrote a cache file");
    let before = std::fs::read(&file).expect("invariant: readable");
    let mtime = std::fs::metadata(&file).and_then(|m| m.modified()).expect("invariant: mtime");

    let off = run_worker(&exe, &dir, "off", &[("BOYKO_PIPELINE_CACHE", "off")]);
    assert_eq!(off.load, "Off", "BOYKO_PIPELINE_CACHE=off creates no cache: {}", off.raw);
    assert_eq!(off.hits, 0, "no cache, no hit: {}", off.raw);
    let after = std::fs::read(&file).expect("invariant: readable");
    let mtime_after = std::fs::metadata(&file).and_then(|m| m.modified()).expect("invariant: mtime");
    assert!(before == after && mtime == mtime_after, "the off run touched the cache file");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "gpu-windowed: spawns hidden windowed workers; run with --test-threads=1"]
fn frame_loop_creates_no_pipeline() {
    let root = scratch("frame_loop");
    let dir = root.join("pc");
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let s = run_worker(&exe, &dir, "frames", &[]);
    assert!(s.pipelines > 0, "the boot created pipelines: {}", s.raw);
    assert_eq!(
        s.created_total, s.pipelines,
        "pipelines were created after boot (inside the frame loop or teardown): {}",
        s.raw
    );
    assert_eq!(s.invalid, 0, "every creation reported VALID feedback: {}", s.raw);
    let _ = std::fs::remove_dir_all(&root);
}
