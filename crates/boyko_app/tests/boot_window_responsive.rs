//! BOOT-1 C1: the boot window, observed from OUTSIDE the process — never white, never hung, and a
//! close during boot hides the window at once and exits clean.
//!
//! # How it observes
//!
//! Each parent test spawns this binary's own worker [`boot_window_child`] (a room-class scene
//! through `EnginePlugins::window`, `BOYKO_PIPELINE_CACHE=off` so the boot is the cold, multi-second
//! one) and watches its window with calls that send it NO message: `EnumWindows`,
//! `InternalGetWindowText`, `GetClassNameW`, `IsWindowVisible`, `IsHungAppWindow`,
//! `GetForegroundWindow`, `GetClientRect` / `ClientToScreen` and a screen-DC `BitBlt` of the client
//! area, every [`SAMPLE_PERIOD`]. The only messages are the explicit probes: a second thread's
//! `SendMessageTimeoutW(WM_NULL)` round trips (the "is it answering" probe) and, in the close test,
//! one `PostMessageW(WM_CLOSE)`. This is the research recipe of `observe.py` (boot-latency
//! research, 2026-10-20) moved into a gate.
//!
//! A pixel sample is only trusted while the window is the foreground window: a background window
//! may be occluded. Each test therefore needs the window to come up in front, which it does when the
//! test runs from an interactive console. `BOYKO_WIN_HIDDEN` would make every sample blind, so the
//! parent refuses to run under it — a red, never a skip.
//!
//! # Running
//!
//! `cargo test -p boyko-app --test boot_window_responsive -- --ignored --test-threads=1`, with a
//! visible desktop. The worker is ignored too and returns at once unless the parent armed
//! it, so a plain `--ignored` sweep runs it as a no-op.

#![cfg(windows)]

use core::ffi::c_void;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_app::prelude::*;

/// The worker's arming variable: unset, the worker returns at once.
const CHILD_MARKER: &str = "BOYKO_BOOT1_OBSERVE_CHILD";
/// The variable carrying the worker's window title (unique per parent process).
const TITLE_VAR: &str = "BOYKO_BOOT1_TITLE";
/// The observer's sampling period.
const SAMPLE_PERIOD: Duration = Duration::from_millis(20);
/// The whole run's budget: boot + 30 frames + teardown.
const RUN_DEADLINE: Duration = Duration::from_secs(120);
/// The `WM_NULL` round trip's timeout.
const PROBE_TIMEOUT_MS: u32 = 4_500;
/// A round trip at or above this is a "not answering" red (the OS ghosts at 5 s).
const PROBE_RED_MS: u64 = 4_000;
/// The close test: the window must be invisible or destroyed this soon after `WM_CLOSE`.
const CLOSE_HIDE_BUDGET: Duration = Duration::from_millis(1_500);
/// The close test: the child must exit this soon after `WM_CLOSE`.
const CLOSE_EXIT_BUDGET: Duration = Duration::from_secs(60);
/// The pixel grid sampled inside the client area, per axis.
const GRID: usize = 5;
/// Consecutive rendered samples that mark the first rendered frame (100 ms at the sample period).
const RENDERED_RUN: usize = 5;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Win32 FFI, hand-declared (no Cargo change; the test's own `extern` blocks).
// ─────────────────────────────────────────────────────────────────────────────────────────────

type Hwnd = *mut c_void;
type Hdc = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

const _: () = assert!(core::mem::size_of::<BitmapInfoHeader>() == 40);

type EnumProc = unsafe extern "system" fn(Hwnd, isize) -> i32;

// SAFETY: signatures match the Win64 user32 ABI (HWND / HDC are opaque pointers, BOOL is i32,
// LPWSTR is *mut u16, WPARAM/LPARAM/LRESULT are pointer-width).
#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(proc_: EnumProc, lparam: isize) -> i32;
    fn InternalGetWindowText(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn GetClassNameW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn IsWindow(hwnd: Hwnd) -> i32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn IsHungAppWindow(hwnd: Hwnd) -> i32;
    fn GetForegroundWindow() -> Hwnd;
    fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
    fn SendMessageTimeoutW(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> isize;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
}

// SAFETY: signatures match the Win64 gdi32 ABI (HDC / HBITMAP / HGDIOBJ are opaque pointers).
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn DeleteDC(hdc: Hdc) -> i32;
    fn CreateDIBSection(
        hdc: Hdc,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> *mut c_void;
    fn SelectObject(hdc: Hdc, obj: *mut c_void) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn BitBlt(dst: Hdc, x: i32, y: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32, rop: u32) -> i32;
}

const WM_NULL: u32 = 0x0000;
const WM_CLOSE: u32 = 0x0010;
const SMTO_NORMAL: u32 = 0x0000;
const SRCCOPY: u32 = 0x00CC_0020;
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The worker: a room-class scene through the shipped windowed runner.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The room of `room_smoke.rs`, reduced to what makes the frame visibly lit: a floor, four cubes,
/// a sun, a sky fill and a camera.
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
            aspect: 640.0 / 480.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

#[test]
#[ignore = "gpu-windowed: worker spawned by the boot_window_responsive observer tests; returns at once unless BOYKO_BOOT1_OBSERVE_CHILD is set"]
fn boot_window_child() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        eprintln!("SKIP boot_window_child: {CHILD_MARKER} is unset (the observer tests spawn it)");
        return;
    }
    let title: &'static str = Box::leak(
        std::env::var(TITLE_VAR)
            .expect("invariant: the observer passes the window title")
            .into_boxed_str(),
    );
    let mut app = App::new();
    app.add_startup_system(room);
    app.add_plugins(EnginePlugins::window(title, 640, 480));
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    let exit = app.run();
    // `try_resource`: a close during boot unwinds before the runner inserts `HostFrameStats`.
    let frames = app.world().try_resource::<HostFrameStats>().map_or(0, |s| s.frames);
    println!("BOOT1-CHILD exit={} frames={frames}", exit.0);
    let _ = std::io::stdout().flush();
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The observer.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One observer sample.
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    /// Seconds since the child was spawned.
    t: f64,
    visible: bool,
    hung: bool,
    ghost: bool,
    foreground: bool,
    /// Whether the pixel grid was read (visible, non-empty client area, blit succeeded).
    pixels: bool,
    /// Grid points that are exactly (255, 255, 255).
    pure_white: usize,
    /// Grid points with every channel >= 235.
    white: usize,
    /// Grid points with every channel <= 20.
    black: usize,
    /// Mean Rec. 601 luminance over the grid, 0..255.
    luma: f32,
}

impl Sample {
    /// The cut's pure-white sample: >= 95 % of the grid exactly (255, 255, 255).
    fn is_pure_white(&self) -> bool {
        self.pixels && self.pure_white * 100 >= GRID * GRID * 95
    }

    /// The first rendered frame: neither mostly white nor mostly black, and lit.
    fn is_rendered(&self) -> bool {
        self.pixels
            && self.visible
            && !self.ghost
            && self.white * 2 < GRID * GRID
            && self.black * 2 < GRID * GRID
            && self.luma > 40.0
    }

    fn trusted(&self) -> bool {
        self.visible && self.foreground && self.pixels
    }
}

/// What one observed run produced.
struct Run {
    samples: Vec<Sample>,
    /// The `WM_NULL` probe: max round trip (ms), count, timeouts, and every slow one `(t, ms)`.
    probe_max_ms: u64,
    probe_count: u64,
    probe_timeouts: u64,
    probe_slow: Vec<(f64, u64)>,
    /// `Some(t)` when the close test posted `WM_CLOSE` at `t`.
    close_posted_at: Option<f64>,
    /// The first sample (seconds) at which the window was invisible or gone after the close.
    hidden_at: Option<f64>,
    exit_code: Option<i32>,
    exited_at: Option<f64>,
    stdout: String,
    stderr_path: PathBuf,
}

impl Run {
    fn first_visible(&self) -> Option<f64> {
        self.samples.iter().find(|s| s.visible).map(|s| s.t)
    }

    /// The first rendered frame: the start of the first run of [`RENDERED_RUN`] consecutive
    /// rendered samples. One sample is not enough: the first sample after `ShowWindow` can catch
    /// whatever was behind the window before DWM composited it.
    fn first_rendered(&self) -> Option<f64> {
        self.samples
            .windows(RENDERED_RUN)
            .find(|w| w.iter().all(Sample::is_rendered))
            .map(|w| w[0].t)
    }

    /// The trusted samples between the first visible one and the first rendered one.
    fn boot_samples(&self) -> impl Iterator<Item = &Sample> {
        let start = self.first_visible().unwrap_or(f64::MAX);
        let end = self.first_rendered().unwrap_or(f64::MAX);
        self.samples.iter().filter(move |s| s.t >= start && s.t < end && s.trusted())
    }

    /// The worker's `BOOT1-CHILD` line (libtest prints `test <name> ... ` before it on one line).
    fn child_line(&self) -> Option<&str> {
        self.stdout.lines().find_map(|l| l.find("BOOT1-CHILD ").map(|i| &l[i..]))
    }

    fn child_frames(&self) -> Option<u64> {
        let v = self.child_line()?.split("frames=").nth(1)?;
        v.split_whitespace().next()?.parse().ok()
    }

    fn timeline(&self) -> String {
        let mut out = String::new();
        let mut last: Option<(bool, bool, bool, bool, &'static str)> = None;
        for s in &self.samples {
            let class = if !s.pixels {
                "-"
            } else if s.is_pure_white() {
                "WHITE"
            } else if s.is_rendered() {
                "rendered"
            } else if s.black * 2 >= GRID * GRID {
                "dark"
            } else {
                "other"
            };
            let key = (s.visible, s.hung, s.ghost, s.foreground, class);
            if last != Some(key) {
                out.push_str(&format!(
                    "  t={:7.3}s visible={} hung={} ghost={} fg={} grid={class} (pure_white {} white {} black {} luma {:.0})\n",
                    s.t, s.visible as u8, s.hung as u8, s.ghost as u8, s.foreground as u8,
                    s.pure_white, s.white, s.black, s.luma
                ));
                last = Some(key);
            }
        }
        out.push_str(&format!(
            "  WM_NULL: {} round trips, max {} ms, {} timeouts; slow (>1 s): {:?}\n",
            self.probe_count, self.probe_max_ms, self.probe_timeouts, self.probe_slow
        ));
        out.push_str(&format!(
            "  close posted at {:?}, hidden at {:?}; child exit {:?} at {:?}; stdout BOOT1 line: {:?}\n",
            self.close_posted_at,
            self.hidden_at,
            self.exit_code,
            self.exited_at,
            self.child_line()
        ));
        out
    }
}

/// Collects the observed window and its ghost from one `EnumWindows` pass.
struct FindState {
    title: Vec<u16>,
    win: usize,
    ghost: usize,
}

/// `EnumWindows` callback: matches the title prefix with the message-free `InternalGetWindowText`.
unsafe extern "system" fn find_cb(hwnd: Hwnd, lparam: isize) -> i32 {
    // SAFETY: `lparam` is the `&mut FindState` `find` passes, alive for the whole `EnumWindows`
    // call; the callback runs synchronously on the calling thread, so no other borrow aliases it.
    let st = unsafe { &mut *(lparam as *mut FindState) };
    let mut text = [0u16; 512];
    // SAFETY: `hwnd` is the OS-supplied top-level window; `text` is a 512-element buffer and the
    // count passed is its length.
    let n = unsafe { InternalGetWindowText(hwnd, text.as_mut_ptr(), text.len() as i32) };
    let n = n.max(0) as usize;
    if n >= st.title.len() && text[..st.title.len()] == st.title[..] {
        let mut class = [0u16; 64];
        // SAFETY: as above, a 64-element buffer with its length.
        let c = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let ghost: Vec<u16> = "Ghost".encode_utf16().collect();
        if c.max(0) as usize == ghost.len() && class[..ghost.len()] == ghost[..] {
            if st.ghost == 0 {
                st.ghost = hwnd as usize;
            }
        } else if st.win == 0 {
            st.win = hwnd as usize;
        }
    }
    1
}

fn find(title: &str) -> (Hwnd, Hwnd) {
    let mut st = FindState { title: title.encode_utf16().collect(), win: 0, ghost: 0 };
    // SAFETY: `find_cb` matches `EnumProc`; the state pointer outlives the synchronous call.
    unsafe { EnumWindows(find_cb, &mut st as *mut FindState as isize) };
    (st.win as Hwnd, st.ghost as Hwnd)
}

/// A screen-DC capture of one window's client area into a top-down 32-bit DIB.
struct Capture {
    screen: Hdc,
    mem: Hdc,
    bmp: *mut c_void,
    bits: *mut c_void,
    w: i32,
    h: i32,
}

impl Capture {
    fn new(w: i32, h: i32) -> Option<Self> {
        // SAFETY: `GetDC(null)` returns the screen DC (released in `Drop`).
        let screen = unsafe { GetDC(core::ptr::null_mut()) };
        // SAFETY: `screen` is a live DC.
        let mem = unsafe { CreateCompatibleDC(screen) };
        let info = BitmapInfo {
            header: BitmapInfoHeader {
                size: 40,
                width: w,
                height: -h,
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        let mut bits: *mut c_void = core::ptr::null_mut();
        // SAFETY: `info` is a fully initialised top-down 32-bpp `BITMAPINFO`; `bits` is a valid
        // out-pointer; no file mapping.
        let bmp = unsafe { CreateDIBSection(mem, &info, 0, &mut bits, core::ptr::null_mut(), 0) };
        if screen.is_null() || mem.is_null() || bmp.is_null() || bits.is_null() {
            return None;
        }
        // SAFETY: `mem` and `bmp` are live and compatible; the previous object is the stock 1x1
        // bitmap, which `DeleteDC` reclaims.
        unsafe { SelectObject(mem, bmp) };
        Some(Self { screen, mem, bmp, bits, w, h })
    }

    /// Blits the screen rectangle at `(x, y)` and reads the grid.
    fn grid(&self, x: i32, y: i32, s: &mut Sample) {
        // SAFETY: both DCs are live; the blit stays inside the `w x h` DIB.
        let ok = unsafe { BitBlt(self.mem, 0, 0, self.w, self.h, self.screen, x, y, SRCCOPY) };
        if ok == 0 {
            return;
        }
        // SAFETY: `bits` points at the DIB's `w * h` BGRA texels, alive while `bmp` is selected;
        // the blit above completed synchronously.
        let px = unsafe {
            core::slice::from_raw_parts(self.bits as *const u8, (self.w * self.h * 4) as usize)
        };
        let mut luma = 0.0f32;
        for gy in 0..GRID {
            for gx in 0..GRID {
                let px_x = (self.w as usize) * (gx + 1) / (GRID + 1);
                let px_y = (self.h as usize) * (gy + 1) / (GRID + 1);
                let i = (px_y * self.w as usize + px_x) * 4;
                let (b, g, r) = (px[i], px[i + 1], px[i + 2]);
                if r == 255 && g == 255 && b == 255 {
                    s.pure_white += 1;
                }
                if r >= 235 && g >= 235 && b >= 235 {
                    s.white += 1;
                }
                if r <= 20 && g <= 20 && b <= 20 {
                    s.black += 1;
                }
                luma += 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            }
        }
        s.luma = luma / (GRID * GRID) as f32;
        s.pixels = true;
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: each object was created in `new` and is released exactly once, the bitmap after
        // the DC that selected it.
        unsafe {
            DeleteDC(self.mem);
            DeleteObject(self.bmp);
            ReleaseDC(core::ptr::null_mut(), self.screen);
        }
    }
}

/// Samples the observed window once.
fn sample(t: f64, hwnd: Hwnd, ghost: Hwnd, capture: &mut Option<Capture>) -> Sample {
    let mut s = Sample { t, ghost: !ghost.is_null(), ..Sample::default() };
    if hwnd.is_null() {
        return s;
    }
    // SAFETY: message-free queries on a window handle (a stale handle yields FALSE).
    unsafe {
        s.visible = IsWindowVisible(hwnd) != 0;
        s.hung = IsHungAppWindow(hwnd) != 0;
        let fg = GetForegroundWindow();
        s.foreground = fg == hwnd || (!ghost.is_null() && fg == ghost);
    }
    if !s.visible {
        return s;
    }
    let mut rc = Rect::default();
    let mut origin = Point::default();
    // SAFETY: valid out-pointers; both calls read the window's stored geometry (no message).
    let ok = unsafe { GetClientRect(hwnd, &mut rc) != 0 && ClientToScreen(hwnd, &mut origin) != 0 };
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    if !ok || w < 8 || h < 8 {
        return s;
    }
    if capture.as_ref().is_none_or(|c| c.w != w || c.h != h) {
        *capture = Capture::new(w, h);
    }
    if let Some(c) = capture.as_ref() {
        c.grid(origin.x, origin.y, &mut s);
    }
    s
}

/// Which observer test drives the run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Observe,
    CloseAtFirstVisible,
}

fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("boyko_boot1_observe");
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// Spawns the worker and observes its window until it exits (or the deadline).
fn observe(name: &str, mode: Mode) -> Run {
    assert!(
        std::env::var_os("BOYKO_WIN_HIDDEN").is_none(),
        "BOYKO_WIN_HIDDEN is set: the worker's window would never be shown and every sample would \
         be blind. Unset it; this observer needs a visible desktop."
    );
    // SAFETY: a process-wide DPI mode switch for THIS (parent) process only, so client rectangles
    // and the screen capture share physical pixels; a second call fails harmlessly.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let title = format!("boyko boot1 observe {} {name}", std::process::id());
    let dir = out_dir();
    let stdout_path = dir.join(format!("{name}.stdout.txt"));
    let stderr_path = dir.join(format!("{name}.stderr.txt"));
    let stdout = File::create(&stdout_path).expect("invariant: the out dir is writable");
    let stderr = File::create(&stderr_path).expect("invariant: the out dir is writable");

    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args(["boot_window_child", "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    // Every inherited engine knob is scrubbed except the cache directory override, which a lane
    // may point at its scratch; the worker's own knobs are set explicitly.
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("BOYKO_") && k != "BOYKO_PIPELINE_CACHE_DIR" {
            cmd.env_remove(&key);
        }
    }
    cmd.env(CHILD_MARKER, "1")
        .env(TITLE_VAR, &title)
        .env("BOYKO_PIPELINE_CACHE", "off")
        .env("BOYKO_WINDOW_FRAMES", "30")
        .env("BOYKO_DISABLE_VALIDATION", "1");

    let t0 = Instant::now();
    let mut child = cmd.spawn().expect("invariant: the test binary can spawn itself");

    // The `WM_NULL` probe thread: round trips against the window once it is found.
    let target = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let probe_max = Arc::new(AtomicU64::new(0));
    let probe_count = Arc::new(AtomicU64::new(0));
    let probe_timeouts = Arc::new(AtomicU64::new(0));
    let prober = {
        let (target, stop) = (Arc::clone(&target), Arc::clone(&stop));
        let (max, count, timeouts) =
            (Arc::clone(&probe_max), Arc::clone(&probe_count), Arc::clone(&probe_timeouts));
        std::thread::spawn(move || {
            let mut slow = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let hwnd = target.load(Ordering::Relaxed) as Hwnd;
                // SAFETY: a message-free validity check on a handle value.
                if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
                    std::thread::sleep(SAMPLE_PERIOD);
                    continue;
                }
                let start = Instant::now();
                let mut result = 0usize;
                // SAFETY: `WM_NULL` carries no pointers; `result` is a valid out-pointer.
                let ok = unsafe {
                    SendMessageTimeoutW(hwnd, WM_NULL, 0, 0, SMTO_NORMAL, PROBE_TIMEOUT_MS, &mut result)
                };
                let ms = start.elapsed().as_millis() as u64;
                // SAFETY: as above.
                let gone = unsafe { IsWindow(hwnd) } == 0;
                if ok == 0 && !gone {
                    timeouts.fetch_add(1, Ordering::Relaxed);
                }
                if !gone {
                    count.fetch_add(1, Ordering::Relaxed);
                    max.fetch_max(ms, Ordering::Relaxed);
                    if ms > 1_000 {
                        slow.push((t0.elapsed().as_secs_f64(), ms));
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            slow
        })
    };

    let mut samples = Vec::with_capacity(8_192);
    let mut capture: Option<Capture> = None;
    let mut close_posted_at = None;
    let mut hidden_at = None;
    let mut exit_code = None;
    let mut exited_at = None;
    loop {
        let t = t0.elapsed().as_secs_f64();
        let (hwnd, ghost) = find(&title);
        if !hwnd.is_null() {
            target.store(hwnd as usize, Ordering::Relaxed);
        }
        let s = sample(t, hwnd, ghost, &mut capture);
        if mode == Mode::CloseAtFirstVisible {
            if close_posted_at.is_none() && s.visible {
                // SAFETY: a posted message with no pointer payload.
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
                close_posted_at = Some(t);
            } else if close_posted_at.is_some() && hidden_at.is_none() && !s.visible {
                hidden_at = Some(t);
            }
        }
        samples.push(s);
        if let Ok(Some(status)) = child.try_wait() {
            exit_code = status.code();
            exited_at = Some(t0.elapsed().as_secs_f64());
            if close_posted_at.is_some() && hidden_at.is_none() {
                hidden_at = exited_at;
            }
            break;
        }
        if t0.elapsed() > RUN_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(SAMPLE_PERIOD);
    }
    stop.store(true, Ordering::Relaxed);
    let probe_slow = prober.join().expect("invariant: the probe thread does not panic");
    drop(capture);

    let run = Run {
        samples,
        probe_max_ms: probe_max.load(Ordering::Relaxed),
        probe_count: probe_count.load(Ordering::Relaxed),
        probe_timeouts: probe_timeouts.load(Ordering::Relaxed),
        probe_slow,
        close_posted_at,
        hidden_at,
        exit_code,
        exited_at,
        stdout: std::fs::read_to_string(&stdout_path).unwrap_or_default(),
        stderr_path,
    };
    write_timeline(&dir, name, &run);
    run
}

fn write_timeline(dir: &Path, name: &str, run: &Run) {
    let text = run.timeline();
    eprint!("{name} timeline:\n{text}");
    let _ = std::fs::write(dir.join(format!("{name}.timeline.txt")), text);
}

/// The observer itself must have seen the boot: found the window, saw it visible in front, and saw
/// the worker exit 0. Anything else is a harness (or scene) red, never a product verdict.
fn assert_observed(run: &Run, name: &str) {
    assert!(
        run.first_visible().is_some(),
        "{name}: OBSERVER BLIND - the worker's window was never found visible. Worker stderr: {}\n{}",
        run.stderr_path.display(),
        run.timeline()
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "{name}: the worker did not exit 0 (the scene, not the observer). Worker stderr: {}\n{}",
        run.stderr_path.display(),
        run.timeline()
    );
}

#[test]
#[ignore = "gpu-windowed: spawns a visible windowed worker and observes it from outside; needs an interactive desktop, BOYKO_WIN_HIDDEN unset, --test-threads=1"]
fn boot_client_area_never_white() {
    let run = observe("boot_client_area_never_white", Mode::Observe);
    assert_observed(&run, "boot_client_area_never_white");
    let first_rendered = run.first_rendered();
    assert!(
        first_rendered.is_some(),
        "OBSERVER BLIND: no sample ever showed a rendered frame (white < 50 %, black < 50 %, luma > 40)\n{}",
        run.timeline()
    );
    let trusted = run.boot_samples().count();
    assert!(
        trusted >= 3,
        "OBSERVER BLIND: only {trusted} visible + foreground samples between the first visible one \
         and the first rendered one (need >= 3). Keep the desktop free while it runs.\n{}",
        run.timeline()
    );
    let white: Vec<f64> = run.boot_samples().filter(|s| s.is_pure_white()).map(|s| s.t).collect();
    assert!(
        white.is_empty(),
        "WHITE WINDOW: {} of {trusted} boot samples had >= 95 % of the client grid at (255,255,255); \
         first at t={:.3} s (visible at {:.3} s, first rendered at {:.3} s)\n{}",
        white.len(),
        white[0],
        run.first_visible().unwrap_or(0.0),
        first_rendered.unwrap_or(0.0),
        run.timeline()
    );
}

#[test]
#[ignore = "gpu-windowed: spawns a visible windowed worker and observes it from outside; needs an interactive desktop, BOYKO_WIN_HIDDEN unset, --test-threads=1"]
fn boot_window_never_hung() {
    let run = observe("boot_window_never_hung", Mode::Observe);
    assert_observed(&run, "boot_window_never_hung");
    assert!(run.probe_count > 0, "OBSERVER BLIND: no WM_NULL round trip completed\n{}", run.timeline());
    let hung = run.samples.iter().filter(|s| s.hung).count();
    let ghosts = run.samples.iter().filter(|s| s.ghost).count();
    let first_hung = run.samples.iter().find(|s| s.hung).map(|s| s.t);
    assert!(
        hung == 0 && ghosts == 0 && run.probe_timeouts == 0 && run.probe_max_ms < PROBE_RED_MS,
        "HUNG WINDOW: {hung} IsHungAppWindow samples (first at {first_hung:?} s), {ghosts} ghost \
         samples, {} WM_NULL timeouts, max round trip {} ms (red at {PROBE_RED_MS} ms)\n{}",
        run.probe_timeouts,
        run.probe_max_ms,
        run.timeline()
    );
}

#[test]
#[ignore = "gpu-windowed: spawns a visible windowed worker and observes it from outside; needs an interactive desktop, BOYKO_WIN_HIDDEN unset, --test-threads=1"]
fn close_during_boot_hides_at_once_and_exits_clean() {
    let run = observe("close_during_boot", Mode::CloseAtFirstVisible);
    let posted = run.close_posted_at.unwrap_or_else(|| {
        panic!("OBSERVER BLIND: the window was never seen visible, so no WM_CLOSE was posted\n{}", run.timeline())
    });
    let hidden = run.hidden_at.expect("invariant: the loop records the hide or the exit");
    let hide = Duration::from_secs_f64((hidden - posted).max(0.0));
    assert!(
        hide <= CLOSE_HIDE_BUDGET,
        "CLOSE NOT HONOURED: WM_CLOSE posted at {posted:.3} s, the window was still visible until \
         {hidden:.3} s ({} ms > {} ms)\n{}",
        hide.as_millis(),
        CLOSE_HIDE_BUDGET.as_millis(),
        run.timeline()
    );
    let exited = run.exited_at.unwrap_or(f64::MAX);
    assert!(
        exited - posted <= CLOSE_EXIT_BUDGET.as_secs_f64() && run.exit_code == Some(0),
        "CLOSE DID NOT EXIT CLEAN: exit {:?} at {:?} s after WM_CLOSE at {posted:.3} s (budget {} s). \
         Worker stderr: {}\n{}",
        run.exit_code,
        run.exited_at,
        CLOSE_EXIT_BUDGET.as_secs(),
        run.stderr_path.display(),
        run.timeline()
    );
    assert_eq!(
        run.child_frames(),
        Some(0),
        "a close during boot must reach no frame (the worker's BOOT1-CHILD line)\n{}",
        run.timeline()
    );
}
