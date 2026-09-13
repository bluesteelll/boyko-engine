//! Shared harness for the two teardown-probe device tests, `vb_teardown_destroys_boot_resources.rs`
//! and `forward_teardown_destroys_forward_sets.rs`.
//!
//! Not a test target of its own: a `mod.rs` in a subdirectory of `tests/` is not auto-discovered
//! (the `tests/common/mod.rs` precedent), so each binary declares `mod teardown_probe;`.
//!
//! # One run
//!
//! [`run`] boots one window on the configured render path, runs [`BUDGET`] Main runs, and lets the
//! windowed runner tear down as in production. The runner samples [`HostTeardownStats`] after every
//! teardown destroy and before `VulkanContext`'s `Drop` clears the memory pools and destroys the
//! device. The harness reads that record once `app.run()` has returned.
//!
//! At Main run [`DRIVE_FRAME`], the `drive` system:
//! 1. keys the geometry table's `bounds_buffer` and `meta_buffer` into watch slots 0 and 1 when
//!    [`ProbeCfg::watch_table`] is set;
//! 2. creates a 64-byte host-visible **buffer control**, keys it into the next watch slot, and queues
//!    it on `RetiredGpuBuffers` with `retire_frame = u64::MAX`. The per-frame drain keeps an entry
//!    whose `retire_frame` is above the epoch, so only teardown's force-drain destroys it;
//! 3. records whether each watched key is live in-frame;
//! 4. runs the **counter control**: it creates and destroys one bind group, reading
//!    `VulkanContext::persistent_descriptor_pools` before, between and after.
//!
//! `drive` also records the persistent-descriptor-pool count at every Main run
//! (`pools_per_frame`), so a render-target rebuild during the run shows as a step in that trace
//! instead of only as a different final count.
//!
//! # Controls
//!
//! [`assert_preconditions_and_controls`] asserts, after the preconditions:
//! - **C1**: every watched key read live in-frame, so the probe can see a live allocation;
//! - **C2**: the counter read `p0, p0 + 1, p0` around the control group, so it moves on create and
//!   on destroy;
//! - **C3**: the buffer control, which teardown's force-drain destroyed, reads dead at the sample,
//!   so the sample sits after the destroys (measured);
//! - **C4**: `GpuDevice` was already gone from the World at the sample (declared).
//!
//! # The summary
//!
//! [`run`] builds one string from every measured value right after `app.run()` returns and prints
//! it once. Every SKIP and assertion message, here and in both binaries, ends with it, so a red
//! shows the whole record and not only the value that failed.
//!
//! # Environment
//!
//! Every `BOYKO_*` variable outside [`ENV_ALLOWED`] fails a precondition before `App::new()`, so a
//! stray knob costs no device boot. The list denies by default because the predicted counts assume
//! no arming, frame-loop or dump knob, and a deny-by-default list needs no per-knob trace.

use boyko_app::HOST_TEARDOWN_WATCH_SLOTS;
use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{
    AaConfig, AaMode, GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
    ResolvedRenderPath, RetiredGpuBuffers,
};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc, BufferUsage,
    DescriptorKind, MemoryLocation, RhiDevice, ShaderStage,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::present::FRAMES_IN_FLIGHT;

/// The only `BOYKO_*` variables a probe run tolerates: the two validation flags, window visibility
/// and logging. Their values are printed in the summary.
pub const ENV_ALLOWED: [&str; 4] =
    ["BOYKO_DISABLE_VALIDATION", "BOYKO_ENABLE_VALIDATION", "BOYKO_WIN_HIDDEN", "BOYKO_LOG"];

/// Main runs in one probe, as an array length.
const BUDGET_FRAMES: usize = 12;
/// Main runs in one probe; the budget system requests exit on the last one.
pub const BUDGET: u32 = BUDGET_FRAMES as u32;
/// The Main run at which `drive` keys the watches and runs both controls.
pub const DRIVE_FRAME: u32 = 4;
/// Size of each control buffer.
const CONTROL_BUFFER_BYTES: u64 = 64;

/// Main runs left before the budget system requests exit.
#[derive(Resource)]
pub struct FrameBudget(pub u32);

/// What one probe boots and watches. Inserted as a Resource so the systems can read it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ProbeCfg {
    /// Window title and message prefix.
    pub label: &'static str,
    /// The render path requested through `RenderPathConfig`.
    pub path: RenderPath,
    /// The geometry legs requested through `RenderPathConfig`.
    pub legs: GeometryLegs,
    /// The anti-aliasing mode requested through `AaConfig`.
    pub aa: AaMode,
    /// Whether to watch the geometry table's `bounds_buffer` and `meta_buffer` (slots 0 and 1).
    pub watch_table: bool,
}

/// What the probe's systems recorded during the run. A plain Resource, so it survives teardown.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ProbeLedger {
    /// `drive` ran at least once.
    pub ran: bool,
    /// The startup system found what it needs to measure: on a VisibilityBuffer boot, a built
    /// geometry table; on any other path, always `true`.
    pub armed: bool,
    /// Main runs `drive` has counted.
    pub frame: u32,
    /// The Main run at which the watches were keyed and the controls ran.
    pub drive_frame: Option<u32>,
    /// The watch slot holding the buffer control.
    pub control_slot: usize,
    /// Per watch slot: whether the watched buffer is host-visible (`mapped.is_some()`), i.e.
    /// whether its key names a host-pool sub-allocation.
    pub watch_host_visible: [Option<bool>; HOST_TEARDOWN_WATCH_SLOTS],
    /// Per watch slot: `host_allocation_is_live` for its key at [`DRIVE_FRAME`].
    pub in_frame_live: [Option<bool>; HOST_TEARDOWN_WATCH_SLOTS],
    /// The counter control: persistent descriptor pools before, after creating, and after
    /// destroying one bind group.
    pub pool_ctl: Option<(u32, u32, u32)>,
    /// Persistent descriptor pools at the start of each Main run's `drive`.
    pub pools_per_frame: [Option<u32>; BUDGET_FRAMES],
}

/// One run's full record, read after `app.run()` returned.
pub struct Probe {
    /// The configuration the run booted with.
    pub cfg: ProbeCfg,
    /// `AppExit.0` as the runner returned it.
    pub exit: bool,
    /// `FrameBudget` after the run: [`BUDGET`] means the frame loop never ran.
    pub budget_left: u32,
    /// The systems' record.
    pub ledger: ProbeLedger,
    /// The runner's post-teardown sample.
    pub stats: HostTeardownStats,
    /// The render path the boot resolved, if the Resource is present.
    pub resolved: Option<ResolvedRenderPath>,
    /// Every value above in one line; appended to every SKIP and assertion message.
    pub summary: String,
}

/// Fails a precondition, before any device boot, if a `BOYKO_*` variable outside [`ENV_ALLOWED`]
/// is set. Names are compared ASCII-case-insensitively, as Windows reads environment names.
pub fn check_env(label: &str) {
    let stray: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| {
            let upper = name.to_ascii_uppercase();
            upper.starts_with("BOYKO_") && !ENV_ALLOWED.contains(&upper.as_str())
        })
        .collect();
    assert!(
        stray.is_empty(),
        "[teardown-probe {label}] precondition: unset {stray:?} — this probe's predicted counts assume no \
         arming/loop/dump knob"
    );
}

/// Boots one windowed run for `cfg`, lets it tear down, and returns the record with its summary.
pub fn run(cfg: ProbeCfg) -> Probe {
    check_env(cfg.label);

    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(cfg);
    app.insert_resource(ProbeLedger::default());
    app.insert_resource(HostTeardownStats::default());
    app.add_systems(exit_after_budget);
    app.add_systems(drive);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window(cfg.label, 320, 240));
    // Inserted AFTER `add_plugins`, which installs the Deferred and `AaMode::Off` defaults — the
    // ordering `vb_geometry_slot_retire_churn.rs` and `vb_mesh_tex.rs` use.
    app.insert_resource(RenderPathConfig { path: cfg.path, legs: cfg.legs });
    app.insert_resource(AaConfig { mode: cfg.aa });

    let exit = app.run().0;

    let budget_left = app.world().resource::<FrameBudget>().0;
    let ledger = *app.world().resource::<ProbeLedger>();
    let stats = *app.world().resource::<HostTeardownStats>();
    let resolved = app.world().try_resource::<ResolvedRenderPath>().copied();
    let summary = summarize(&cfg, exit, budget_left, &ledger, &stats, resolved.as_ref());
    eprintln!("{summary}");
    Probe { cfg, exit, budget_left, ledger, stats, resolved, summary }
}

/// Every measured value of one run, in one line.
fn summarize(
    cfg: &ProbeCfg,
    exit: bool,
    budget_left: u32,
    ledger: &ProbeLedger,
    stats: &HostTeardownStats,
    resolved: Option<&ResolvedRenderPath>,
) -> String {
    let (resolved_path, vb_geometry_table) = match resolved {
        Some(r) => (format!("{:?}", r.path), r.vb_geometry_table.to_string()),
        None => ("<absent>".to_string(), "<absent>".to_string()),
    };
    let env: Vec<String> = ENV_ALLOWED
        .iter()
        .map(|name| match std::env::var_os(name) {
            Some(value) => format!("{name}={}", value.to_string_lossy()),
            None => format!("{name}=<unset>"),
        })
        .collect();
    format!(
        "[teardown-probe {label}] requested=({path:?}, {legs:?}, {aa:?}) watch_table={watch_table} exit={exit} \
         budget_left={budget_left}/{BUDGET} ran={ran} armed={armed} drive_frame={drive_frame:?} \
         resolved.path={resolved_path} vb_geometry_table={vb_geometry_table} FRAMES_IN_FLIGHT={FRAMES_IN_FLIGHT} \
         control_slot={control_slot} watch_host_visible={watch_host_visible:?} in_frame_live={in_frame_live:?} \
         pool_ctl={pool_ctl:?} pools_per_frame={pools_per_frame:?} stats={stats:?} env=[{env}]",
        label = cfg.label,
        path = cfg.path,
        legs = cfg.legs,
        aa = cfg.aa,
        watch_table = cfg.watch_table,
        ran = ledger.ran,
        armed = ledger.armed,
        drive_frame = ledger.drive_frame,
        control_slot = ledger.control_slot,
        watch_host_visible = ledger.watch_host_visible,
        in_frame_live = ledger.in_frame_live,
        pool_ctl = ledger.pool_ctl,
        pools_per_frame = ledger.pools_per_frame,
        env = env.join(" "),
    )
}

/// Returns `false` after printing SKIP when nothing could be measured; otherwise asserts every
/// precondition and the controls C1–C4 and returns `true`. Every message ends with the summary.
pub fn assert_preconditions_and_controls(p: &Probe) -> bool {
    let label = p.cfg.label;
    let s = &p.summary;
    let is_vb = matches!(p.cfg.path, RenderPath::VisibilityBuffer);

    // SKIP: a skip is not a pass, and says so.
    if p.budget_left == BUDGET {
        eprintln!(
            "SKIP [teardown-probe {label}]: windowed boot unavailable — the frame loop never ran, nothing was \
             measured (a skip, not a pass)\n{s}"
        );
        return false;
    }
    if is_vb && !p.ledger.armed {
        eprintln!(
            "SKIP [teardown-probe {label}]: the VisibilityBuffer boot built no MeshGeometryTable on this device \
             (resolve degraded) — nothing was measured (a skip, not a pass)\n{s}"
        );
        return false;
    }

    // Preconditions.
    assert!(p.exit, "precondition: the windowed runner returns AppExit(true)\n{s}");
    assert_eq!(p.budget_left, 0, "precondition: the frame loop ran the full {BUDGET}-frame budget\n{s}");
    assert!(p.ledger.ran, "precondition: the drive system ran — else every ledger field is a Default\n{s}");
    assert_eq!(
        p.ledger.drive_frame,
        Some(DRIVE_FRAME),
        "precondition: the watches were keyed and the controls ran at Main run {DRIVE_FRAME}\n{s}"
    );
    let Some(resolved) = p.resolved else {
        panic!("precondition: ResolvedRenderPath is in the World after the run\n{s}");
    };
    assert_eq!(
        resolved.path, p.cfg.path,
        "precondition: the boot resolved the requested render path (no degrade)\n{s}"
    );
    if is_vb {
        assert!(
            resolved.vb_geometry_table,
            "precondition: the VisibilityBuffer boot resolved with the geometry table armed\n{s}"
        );
    }
    assert!(
        p.stats.sampled,
        "precondition: the runner's post-teardown sample ran (HostTeardownStats::sampled)\n{s}"
    );
    let used = p.ledger.control_slot + 1;
    for (slot, w) in p.stats.watch.iter().enumerate().take(used) {
        assert!(w.key.is_some(), "precondition: watch slot {slot} carries a key\n{s}");
        assert_eq!(
            p.ledger.watch_host_visible[slot],
            Some(true),
            "precondition: the buffer watched in slot {slot} is host-visible, so its key names a host-pool \
             sub-allocation\n{s}"
        );
    }
    for (i, a) in p.stats.watch.iter().enumerate().take(used) {
        for (j, b) in p.stats.watch.iter().enumerate().take(used).skip(i + 1) {
            assert_ne!(a.key, b.key, "precondition: watch keys {i} and {j} are distinct\n{s}");
        }
    }

    // C1: the probe sees an allocation that is certainly alive.
    for (slot, live) in p.ledger.in_frame_live.iter().enumerate().take(used) {
        assert_eq!(
            *live,
            Some(true),
            "control C1: watch slot {slot} did not read live in-frame at Main run {DRIVE_FRAME} — the probe \
             cannot see an allocation that is certainly alive, so a dead reading at teardown proves nothing\n{s}"
        );
    }
    // C2: the counter moves on create and on destroy.
    let Some((p0, p1, p2)) = p.ledger.pool_ctl else {
        panic!("control C2: the counter control never ran\n{s}");
    };
    assert!(
        p0.checked_add(1) == Some(p1) && p2 == p0,
        "control C2: the persistent-descriptor-pool counter read ({p0}, {p1}, {p2}) around one bind group's \
         create and destroy, not (p0, p0 + 1, p0) — a counter that does not track pools proves nothing about \
         the pools teardown left\n{s}"
    );
    // C3: measured sample position.
    assert!(
        !p.stats.watch[p.ledger.control_slot].live_at_teardown,
        "control C3 (measured sample position): the buffer control, destroyed by teardown's force-drain of \
         RetiredGpuBuffers, still reads live at the sample — the sample ran before that destroy, or a later \
         allocation re-used its key, so a live reading of any other watch proves nothing\n{s}"
    );
    // C4: declared sample position.
    assert!(
        !p.stats.gpu_device_present_at_sample,
        "control C4 (declared sample position): GpuDevice was still in the World at the sample, so \
         record_teardown_stats ran before teardown's last step\n{s}"
    );
    true
}

fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// Spawns one cube (VB-aware on a VisibilityBuffer boot, so it claims a geometry-table slot) and a
/// minimal view.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    cfg: Res<ProbeCfg>,
    mut ledger: ResMut<ProbeLedger>,
) {
    let handle = if matches!(cfg.path, RenderPath::VisibilityBuffer) {
        ledger.armed = geo.0.is_some();
        let Some(table) = geo.0.as_mut() else {
            return;
        };
        meshes.cube_vb(dev.get(), 1.0, table)
    } else {
        ledger.armed = true;
        meshes.cube(dev.get(), 1.0)
    };
    commands.spawn(MeshBundle::new(handle, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))));
    spawn_minimal_view(&mut commands);
}

/// Counts Main runs, traces the pool count, and at [`DRIVE_FRAME`] keys the watches and runs both
/// controls. Its NonSend parameters keep it on the runner thread.
fn drive(
    geo: NonSendRes<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    mut retired: NonSendResMut<RetiredGpuBuffers>,
    cfg: Res<ProbeCfg>,
    mut ledger: ResMut<ProbeLedger>,
    mut stats: ResMut<HostTeardownStats>,
) {
    let ctx = dev.get();
    ledger.ran = true;
    let frame = ledger.frame;
    ledger.frame += 1;
    if let Some(entry) = ledger.pools_per_frame.get_mut(frame as usize) {
        *entry = Some(ctx.persistent_descriptor_pools());
    }
    if frame != DRIVE_FRAME || !ledger.armed {
        return;
    }
    ledger.drive_frame = Some(frame);

    let control_slot = if cfg.watch_table {
        let table = geo
            .0
            .as_ref()
            .expect("invariant: `armed` on a VisibilityBuffer boot means the geometry table was built");
        watch(&mut stats, &mut ledger, 0, table.bounds_buffer());
        watch(&mut stats, &mut ledger, 1, table.meta_buffer());
        2
    } else {
        0
    };
    ledger.control_slot = control_slot;

    // Buffer control: alive for the rest of the run, destroyed only by teardown's force-drain.
    let control = ctx
        .create_buffer(&control_buffer_desc())
        .expect("fixture: a 64-byte host-visible control buffer allocates");
    watch(&mut stats, &mut ledger, control_slot, &control);
    retired.push(control, u64::MAX);

    for (slot, w) in stats.watch.iter().enumerate() {
        if let Some((block, offset)) = w.key {
            ledger.in_frame_live[slot] = Some(ctx.host_allocation_is_live(block, offset));
        }
    }

    ledger.pool_ctl = Some(pool_counter_control(ctx));
}

/// Keys `buf` into watch slot `slot` and records whether it is host-visible.
fn watch(stats: &mut HostTeardownStats, ledger: &mut ProbeLedger, slot: usize, buf: &BoundBuffer) {
    stats.watch[slot].key = Some((buf.block, buf.offset));
    ledger.watch_host_visible[slot] = Some(buf.mapped.is_some());
}

fn control_buffer_desc() -> BufferDesc {
    BufferDesc {
        size: CONTROL_BUFFER_BYTES,
        usage: BufferUsage::STORAGE,
        location: MemoryLocation::HostVisibleCoherent,
    }
}

/// Control C2: one bind group created and destroyed between reads of the persistent-descriptor-pool
/// count. Returns `(before, after create, after destroy)`. Nothing it creates is ever bound or
/// submitted.
fn pool_counter_control(ctx: &VulkanContext) -> (u32, u32, u32) {
    let before = ctx.persistent_descriptor_pools();
    let layout = ctx
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::StorageBuffer,
                stage: ShaderStage::COMPUTE,
            }],
        })
        .expect("fixture: a one-binding storage-buffer layout creates");
    let buffer = ctx
        .create_buffer(&control_buffer_desc())
        .expect("fixture: a 64-byte host-visible buffer allocates");
    let group = ctx
        .create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[BindGroupEntry::StorageBuffer { buffer: &buffer }],
        })
        .expect("fixture: a one-entry storage-buffer bind group creates");
    let after_create = ctx.persistent_descriptor_pools();
    // SAFETY: `group` was created just above on this `ctx`; it was never bound to a command buffer
    // nor named by any submission, and the by-value move destroys it exactly once.
    unsafe { ctx.destroy_bind_group(group) };
    let after_destroy = ctx.persistent_descriptor_pools();
    // SAFETY: both were created just above on this `ctx`. The only set allocated against `layout`
    // and naming `buffer` died with its pool in the destroy above; neither was used by any
    // submission, and each by-value move destroys it exactly once.
    unsafe {
        ctx.destroy_bind_group_layout(layout);
        ctx.destroy_buffer(buffer);
    }
    (before, after_create, after_destroy)
}

/// A sun, a sky light and a camera — the shape of `vb_geometry_slot_retire_churn.rs`'s view.
fn spawn_minimal_view(commands: &mut Commands) {
    const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
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
