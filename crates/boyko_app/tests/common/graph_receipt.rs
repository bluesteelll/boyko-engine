//! OBS (render-wave plan H0 Scope 5) — the `BOYKO_GRAPH_RECEIPT` reader, the class-table census
//! over a receipt, the burst check, and the max-config worker scene the OBS device gates boot.
//!
//! Pulled into each OBS binary with `#[path = "common/graph_receipt.rs"] mod graph_receipt;`
//! (`common/mod.rs` is not involved). Stdlib only on the reading side.
//!
//! # Independence
//!
//! [`class_table_census`] re-derives each resource's class from the receipt's RAW `seeded` bit and
//! seed fields — it never reads the `class` label production wrote, and never calls
//! `SeedClass::derive` — and checks it against [`SEED_CLASS_TABLE`] as data. A receipt writer that
//! emitted a wrong seed, or a declarator that declared a row bare, is red here even when the (P)
//! assert in the same build is compiled out (`--release`).

#![allow(dead_code)] // each OBS binary uses a subset of this module

use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{
    AaConfig, AaMode, DdgiConfig, EmitterActive, GeometryLegs, HzbConfig, HzbMode, Material,
    PARTICLE_BLEND_ADDITIVE, PARTICLE_SHAPE_CONE, ParticleConfig, ParticleEffect,
    ParticleEffectHandle, ParticleEffectsExt, ParticleEmitter, ParticleMode, RenderPath,
    RenderPathConfig, ResolvedRenderPath, SsaoConfig, SsaoQuality,
};
use boyko_rhi_vulkan::present::RECEIPT_CONFIG_KEYS;
use boyko_rhi_vulkan::present::seed_class::{
    DECL_DEFERRED, DECL_FORWARD, DECL_VB, SEED_CLASS_TABLE, SeedKind, declarator_name,
};

// ===============================================================================================
// The reader
// ===============================================================================================

/// One table of the receipt: its header (`""` for the root, `config` / `counts` for the single
/// tables, `resource` / `pass` / `access` / `barrier` / `sink` / `frame` for the arrays) and its
/// `key = value` lines in order, values verbatim (strings keep no quotes).
#[derive(Clone, Debug)]
pub struct Table {
    pub header: String,
    pub fields: Vec<(String, String)>,
}

impl Table {
    /// The raw value of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The value of `key`, which must exist.
    pub fn str(&self, key: &str) -> &str {
        self.get(key).unwrap_or_else(|| panic!("receipt table [{}] has no `{key}`", self.header))
    }

    /// `key` as an integer.
    pub fn u64(&self, key: &str) -> u64 {
        let v = self.str(key);
        v.parse().unwrap_or_else(|e| panic!("receipt [{}] `{key} = {v}` is not an integer ({e})", self.header))
    }

    /// `key` as a signed integer (image layouts).
    pub fn i64(&self, key: &str) -> i64 {
        let v = self.str(key);
        v.parse().unwrap_or_else(|e| panic!("receipt [{}] `{key} = {v}` is not an integer ({e})", self.header))
    }

    /// `key` as a boolean.
    pub fn bool(&self, key: &str) -> bool {
        match self.str(key) {
            "true" => true,
            "false" => false,
            v => panic!("receipt [{}] `{key} = {v}` is not a boolean", self.header),
        }
    }
}

/// A parsed receipt.
#[derive(Clone, Debug)]
pub struct Receipt {
    pub tables: Vec<Table>,
}

/// One `[[resource]]` row.
#[derive(Clone, Debug)]
pub struct ResourceRow {
    pub index: u64,
    pub name: String,
    pub is_image: bool,
    pub mips: u64,
    pub seeded: bool,
    pub seed_layout: i64,
    pub seed_flush_stages: u64,
    pub seed_flush_access: u64,
    pub seed_visible_stages: u64,
    pub seed_visible_access: u64,
}

impl ResourceRow {
    /// Every stage the seed orders the next frame's first access after.
    pub fn seed_stages(&self) -> u64 {
        self.seed_visible_stages | self.seed_flush_stages
    }

    /// The class, re-derived here from the raw bit and the seed: a seed is stateful iff any of its
    /// fields differs from the undefined seed (`VK_IMAGE_LAYOUT_UNDEFINED` = 0, no stage, no access).
    pub fn derived_class(&self) -> &'static str {
        let stateful = self.seed_layout != 0
            || self.seed_flush_stages != 0
            || self.seed_flush_access != 0
            || self.seed_visible_stages != 0
            || self.seed_visible_access != 0;
        match (self.seeded, stateful) {
            (true, true) => "CrossFrame",
            (false, true) => "FrameLocal",
            (false, false) => "Bare",
            (true, false) => "ProvenanceOnly",
        }
    }
}

/// One `[[sink]]` row.
#[derive(Clone, Debug)]
pub struct SinkRow {
    pub declarator: String,
    pub is_image: bool,
    pub slot: u64,
    pub sink_name: String,
    pub fif: String,
    pub res: u64,
    pub graph_name: String,
}

/// One `[[frame]]` record.
#[derive(Clone, Debug)]
pub struct FrameRow {
    pub ordinal: u64,
    pub epoch: u64,
    pub pass_count: u64,
    pub passes: Vec<String>,
    pub config: Vec<(String, u64)>,
}

impl Receipt {
    /// Parses the receipt's flat TOML subset: `# comments`, `[table]` / `[[table]]` headers, and
    /// `key = value` lines whose value is an integer, a boolean or a `"string"`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut tables = vec![Table { header: String::new(), fields: Vec::new() }];
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(h) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
                tables.push(Table { header: h.to_owned(), fields: Vec::new() });
                continue;
            }
            if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                tables.push(Table { header: h.to_owned(), fields: Vec::new() });
                continue;
            }
            let (k, v) = line
                .split_once(" = ")
                .ok_or_else(|| format!("line {}: not `key = value`: {raw}", n + 1))?;
            let v = match v.strip_prefix('"') {
                Some(s) => s
                    .strip_suffix('"')
                    .ok_or_else(|| format!("line {}: unterminated string: {raw}", n + 1))?,
                None => v,
            };
            tables.last_mut().expect("invariant: the root table exists").fields.push((k.to_owned(), v.to_owned()));
        }
        Ok(Self { tables })
    }

    /// Reads and parses `path` — a missing file is RED with a message naming the usual causes.
    pub fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!(
                "no receipt at {} ({e}) — the worker never reached its receipt frame (a failed boot, \
                 a (P) panic on an earlier frame, a hung loop, or no device); RED, never a skip",
                path.display()
            )
        });
        Self::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The root table (the header keys).
    pub fn root(&self) -> &Table {
        &self.tables[0]
    }

    /// The single table `[name]`.
    pub fn single(&self, name: &str) -> &Table {
        let mut it = self.tables.iter().filter(|t| t.header == name);
        let t = it.next().unwrap_or_else(|| panic!("the receipt has no [{name}] table"));
        assert!(it.next().is_none(), "the receipt has two [{name}] tables");
        t
    }

    /// Every table of the array `[[name]]`, in file order.
    pub fn array<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Table> + 'a {
        self.tables.iter().filter(move |t| t.header == name)
    }

    /// `[config]`'s value of `key`.
    pub fn config(&self, key: &str) -> u64 {
        self.single("config").u64(key)
    }

    /// The `[[resource]]` rows.
    pub fn resources(&self) -> Vec<ResourceRow> {
        self.array("resource")
            .map(|t| ResourceRow {
                index: t.u64("index"),
                name: t.str("name").to_owned(),
                is_image: match t.str("kind") {
                    "image" => true,
                    "buffer" => false,
                    k => panic!("[[resource]] kind `{k}` is neither image nor buffer"),
                },
                mips: t.u64("mips"),
                seeded: t.bool("seeded"),
                seed_layout: t.i64("seed_layout"),
                seed_flush_stages: t.u64("seed_flush_stages"),
                seed_flush_access: t.u64("seed_flush_access"),
                seed_visible_stages: t.u64("seed_visible_stages"),
                seed_visible_access: t.u64("seed_visible_access"),
            })
            .collect()
    }

    /// The declared pass names, in order.
    pub fn pass_names(&self) -> Vec<String> {
        self.array("pass").map(|t| t.str("name").to_owned()).collect()
    }

    /// The `[[sink]]` rows, or the reason there are none: a release build writes `sink_rows =
    /// false` (the slot metadata exists only in debug builds), which is stated, never read as an
    /// empty — clean — table.
    pub fn sinks(&self) -> Result<Vec<SinkRow>, String> {
        let root = self.root();
        if !root.bool("sink_rows") {
            return Err(format!(
                "this receipt carries NO sink rows (build = {}): the sink fills' name/Fif columns \
                 exist only in debug builds",
                root.str("build")
            ));
        }
        Ok(self
            .array("sink")
            .map(|t| SinkRow {
                declarator: t.str("declarator").to_owned(),
                is_image: t.str("kind") == "image",
                slot: t.u64("slot"),
                sink_name: t.str("sink_name").to_owned(),
                fif: t.str("fif").to_owned(),
                res: t.u64("res"),
                graph_name: t.str("graph_name").to_owned(),
            })
            .collect())
    }

    /// The `[[frame]]` records.
    pub fn frames(&self) -> Vec<FrameRow> {
        self.array("frame")
            .map(|t| {
                let passes = t.str("passes");
                FrameRow {
                    ordinal: t.u64("ordinal"),
                    epoch: t.u64("epoch"),
                    pass_count: t.u64("pass_count"),
                    passes: if passes.is_empty() { Vec::new() } else { passes.split(',').map(str::to_owned).collect() },
                    config: t
                        .fields
                        .iter()
                        .filter_map(|(k, v)| {
                            k.strip_prefix("cfg_").map(|k| {
                                (k.to_owned(), v.parse().unwrap_or_else(|e| panic!("[[frame]] `cfg_{k} = {v}` ({e})")))
                            })
                        })
                        .collect(),
                }
            })
            .collect()
    }
}

// ===============================================================================================
// The checks
// ===============================================================================================

/// The declarator bit a receipt's `declarator` word names.
pub fn declarator_bit(word: &str) -> u8 {
    [DECL_DEFERRED, DECL_FORWARD, DECL_VB]
        .into_iter()
        .find(|d| declarator_name(*d) == word)
        .unwrap_or_else(|| panic!("receipt declarator `{word}` is not a declarator name"))
}

/// One class-table finding over a receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusViolation {
    pub name: String,
    pub declarator: &'static str,
    /// `WrongClass`, `WrongKind`, `SeedWithoutStages`, `SeedMissingMinStages`,
    /// `ReaderSeedOnWriterRow`, or `Absent` (an `expect_present` row the receipt does not declare).
    pub rule: &'static str,
    pub derived: &'static str,
    pub row_class: &'static str,
    pub owner: &'static str,
}

impl core::fmt::Display for CensusViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} on {}: {} (receipt derives {}, the row is {}, owner {})",
            self.name, self.declarator, self.rule, self.derived, self.row_class, self.owner
        )
    }
}

/// The (R) class-table census (OPEN-ITEMS 1): every receipt resource whose name is a row of
/// [`SEED_CLASS_TABLE`] for `declarator` must derive the row's class and kind, carry a seed with
/// stages, name the row's minimum stages, and be a writer seed where the row says so; every
/// `expect_present` row of the declarator must be declared.
pub fn class_table_census(r: &Receipt, declarator: u8) -> Vec<CensusViolation> {
    let decl = declarator_name(declarator);
    let resources = r.resources();
    let mut out = Vec::new();
    for row in SEED_CLASS_TABLE.iter().filter(|row| row.declarators & declarator != 0) {
        let declared: Vec<&ResourceRow> = resources.iter().filter(|res| res.name == row.name).collect();
        let base = |rule, derived| CensusViolation {
            name: row.name.to_owned(),
            declarator: decl,
            rule,
            derived,
            row_class: row.class.name(),
            owner: row.owner,
        };
        if declared.is_empty() {
            if row.expect_present {
                out.push(base("Absent", "-"));
            }
            continue;
        }
        for res in declared {
            let derived = res.derived_class();
            let stages = res.seed_stages();
            let kind = if res.is_image { SeedKind::Image } else { SeedKind::Buffer };
            let rule = if derived != row.class.name() {
                "WrongClass"
            } else if kind != row.kind {
                "WrongKind"
            } else if stages == 0 {
                "SeedWithoutStages"
            } else if u64::from(row.seed_stages_min) & !stages != 0 {
                "SeedMissingMinStages"
            } else if row.writer_seed && res.seed_flush_stages == 0 {
                "ReaderSeedOnWriterRow"
            } else {
                continue;
            };
            out.push(base(rule, derived));
        }
    }
    out
}

/// The burst check (H0 test 1's burst case): exactly `n` records over consecutive presented
/// frames, `submission_epoch` strictly increasing, the whole `[config]` in each, and the declared
/// pass names of each.
pub fn burst_check(r: &Receipt, n: usize) -> Result<(), String> {
    let frames = r.frames();
    if frames.len() != n {
        return Err(format!("the burst asked for {n} [[frame]] records, the receipt holds {}", frames.len()));
    }
    for w in frames.windows(2) {
        if w[1].epoch <= w[0].epoch {
            return Err(format!(
                "submission_epoch not strictly increasing: record ordinal {} has epoch {}, the next \
                 (ordinal {}) has {} — a per-frame record that is not per frame",
                w[0].ordinal, w[0].epoch, w[1].ordinal, w[1].epoch
            ));
        }
        if w[1].ordinal != w[0].ordinal + 1 {
            return Err(format!(
                "the records are not consecutive presented frames: ordinal {} then {}",
                w[0].ordinal, w[1].ordinal
            ));
        }
    }
    for f in &frames {
        for key in RECEIPT_CONFIG_KEYS {
            if !f.config.iter().any(|(k, _)| k == key) {
                return Err(format!("record ordinal {} lacks cfg_{key}", f.ordinal));
            }
        }
        if f.pass_count == 0 || f.passes.len() as u64 != f.pass_count {
            return Err(format!(
                "record ordinal {} declares pass_count {} but names {} passes",
                f.ordinal,
                f.pass_count,
                f.passes.len()
            ));
        }
    }
    Ok(())
}

/// Every `[[sink]]` row whose slot resolves a declared `ResId` names it the way the graph does,
/// and every declared resource some pass touches has a slot (H0 Scope 3, (R) form).
pub fn sink_check(r: &Receipt) -> Result<usize, String> {
    let sinks = r.sinks()?;
    let mut bad = Vec::new();
    for s in &sinks {
        if !s.graph_name.is_empty() && s.sink_name != s.graph_name {
            bad.push(format!(
                "{} {} slot {} is named `{}` but resolves ResId {} = `{}`",
                s.declarator,
                if s.is_image { "image" } else { "buffer" },
                s.slot,
                s.sink_name,
                s.res,
                s.graph_name
            ));
        }
    }
    let touched: std::collections::BTreeSet<u64> = r.array("access").map(|t| t.u64("res")).collect();
    for res in touched {
        if !sinks.iter().any(|s| s.res == res && !s.graph_name.is_empty()) {
            let name = r.resources().into_iter().find(|x| x.index == res).map_or(String::new(), |x| x.name);
            bad.push(format!("declared resource {res} (`{name}`) is touched but has no sink slot"));
        }
    }
    if bad.is_empty() { Ok(sinks.len()) } else { Err(bad.join("\n")) }
}

/// `[counts]` against the named capacity: `(resources, res_state_total, passes, accesses)` each
/// within `(max_res, max_res, max_pass, max_acc)`.
pub fn counts_check(r: &Receipt) -> Result<String, String> {
    let c = r.single("counts");
    let (res, states, passes, acc) = (c.u64("resources"), c.u64("res_state_total"), c.u64("passes"), c.u64("accesses"));
    let (max_res, max_pass, max_acc) = (c.u64("max_res"), c.u64("max_pass"), c.u64("max_acc"));
    let line = format!(
        "[counts] resources {res} / {max_res}, res_state_total {states} / {max_res}, passes {passes} / \
         {max_pass}, accesses {acc} / {max_acc}"
    );
    let mut bad = Vec::new();
    if res > max_res {
        bad.push(format!("[counts] resources {res} > {max_res}"));
    }
    if states > max_res {
        bad.push(format!("[counts] res_state_total {states} > {max_res}"));
    }
    if passes > max_pass {
        bad.push(format!("[counts] passes {passes} > {max_pass}"));
    }
    if acc > max_acc {
        bad.push(format!("[counts] accesses {acc} > {max_acc}"));
    }
    if bad.is_empty() {
        Ok(line)
    } else {
        Err(format!("{} — a frame past FRAME_GRAPH_CAPACITY regrows an arena (cold) every boot; {line}", bad.join("; ")))
    }
}

// ===============================================================================================
// The max-config worker scene
// ===============================================================================================

/// The env knob selecting the worker's render path: `deferred`, `forward`, `forwardplus`, `vb`.
pub const ENV_PATH: &str = "BOYKO_OBS_PATH";
/// `0` disarms the interpolated body (default armed).
pub const ENV_INTERP: &str = "BOYKO_OBS_INTERP";
/// `0` disarms the particle subsystem (default armed).
pub const ENV_PARTICLES: &str = "BOYKO_OBS_PARTICLES";
/// `0` stops the per-frame material edit, so no frame past 0 uploads (default: edit every frame).
pub const ENV_UPLOAD: &str = "BOYKO_OBS_UPLOAD";
/// `0` disarms TAA where the path would run it (default armed) — a jitter-free frame.
pub const ENV_TAA: &str = "BOYKO_OBS_TAA";
/// `0` disarms DDGI (default armed). A gate comparing two RUNS texel for texel turns it off:
/// measured on the Deferred OBS scene at frame 30, two runs with DDGI armed differed in 821 texels
/// by one LSB, and with it off they agreed exactly in 3 of 3 runs (the probe update is a
/// round-robin accumulator).
pub const ENV_DDGI: &str = "BOYKO_OBS_DDGI";

/// The window edge every OBS worker renders at.
pub const OBS_WIN: u32 = 256;

/// What one OBS worker boot arms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObsArming {
    pub path: RenderPath,
    pub interp: bool,
    pub particles: bool,
    pub upload: bool,
    pub taa: bool,
    pub ddgi: bool,
}

impl ObsArming {
    /// Reads the worker's arming from the environment the driver set.
    pub fn from_env() -> Self {
        let flag = |k: &str| std::env::var(k).map_or(true, |v| v.trim() != "0");
        let path = match std::env::var(ENV_PATH).as_deref().map(str::trim) {
            Ok("forward") => RenderPath::Forward,
            Ok("forwardplus") => RenderPath::ForwardPlus,
            Ok("vb") => RenderPath::VisibilityBuffer,
            Ok("deferred") | Err(_) => RenderPath::Deferred,
            Ok(other) => panic!("{ENV_PATH}={other} is not deferred|forward|forwardplus|vb"),
        };
        Self {
            path,
            interp: flag(ENV_INTERP),
            particles: flag(ENV_PARTICLES),
            upload: flag(ENV_UPLOAD),
            taa: flag(ENV_TAA),
            ddgi: flag(ENV_DDGI),
        }
    }

    /// The env word of `path`.
    pub fn path_word(path: RenderPath) -> &'static str {
        match path {
            RenderPath::Forward => "forward",
            RenderPath::ForwardPlus => "forwardplus",
            RenderPath::VisibilityBuffer => "vb",
            _ => "deferred",
        }
    }

    /// The geometry legs the path runs at max config: both legs where the path marches SDF;
    /// ForwardPlus is pinned mesh-only (`forwardplus_mesh`).
    pub fn legs(self) -> GeometryLegs {
        match self.path {
            RenderPath::ForwardPlus => GeometryLegs::Mesh,
            _ => GeometryLegs::Both,
        }
    }

    /// The declarator bit this path dispatches to.
    pub fn declarator(self) -> u8 {
        match self.path {
            RenderPath::Forward | RenderPath::ForwardPlus => DECL_FORWARD,
            RenderPath::VisibilityBuffer => DECL_VB,
            _ => DECL_DEFERRED,
        }
    }
}

/// The material the per-frame edit targets (minted at startup).
#[derive(Resource, Default)]
pub struct EditedMaterial(Option<Handle<Material>>);

/// The arming, as a resource the startup system reads.
#[derive(Resource)]
pub struct ObsArmingRes(pub ObsArming);

const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

#[allow(clippy::needless_pass_by_value)]
fn obs_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut effects: ResMut<Assets<ParticleEffect>>,
    dev: NonSendRes<GpuDevice>,
    arming: Res<ObsArmingRes>,
    mut edited: ResMut<EditedMaterial>,
) {
    let arming = arming.0;
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);
    let glow = materials.add(Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.9, 0.5, [0.0, 1.5, 0.0], 0));
    edited.0 = Some(glow);
    let row = u16::try_from(glow.index()).expect("invariant: the OBS scene mints < 65536 rows");

    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY)).insert(ShadowCaster);
    let cube_at = Transform::from_translation(Vec3::new(-0.9, 0.5, 0.0));
    let body = commands
        .spawn(MeshBundle { material: MaterialHandle(row), ..MeshBundle::new(cube, cube_at) })
        .insert(ShadowCaster)
        .id();
    if arming.interp {
        commands.entity(body).insert(InterpPair { pair: GpuTransform3D::from_transform(&cube_at) });
    }
    if arming.legs() == GeometryLegs::Both {
        commands.spawn(SdfPrimitive(SdfEdit::sphere([0.9, 0.6, 0.0], 0.6, sdf_op::UNION, 0.0).with_material(row)));
    }

    let lamp = Vec3::new(0.0, 2.2, 1.5);
    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(lamp),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([lamp.x, lamp.y, lamp.z], [1.0, 0.8, 0.6], 120.0, 6.0),
        })
        .insert(CastsPunctualShadow);

    if arming.particles {
        let effect = effects.register_effect(ParticleEffect {
            gravity: [0.0; 3],
            drag: 0.0,
            rot_speed: 0.0,
            lifetime_min: 1.0,
            lifetime_max: 1.0,
            speed_min: 1.0,
            speed_max: 1.0,
            size_base: 0.05,
            cone_cos: 0.766_044_4,
            collision_radius: 0.0,
            restitution: 0.0,
            friction: 0.0,
            color_keys: [0xFFFF_FFFF; 4],
            color_times: [0.0, 0.333_333_34, 0.666_666_7, 1.0],
            size_keys: [1.0; 4],
            tex_index: 0,
            blend_class: PARTICLE_BLEND_ADDITIVE,
            flags: 0,
            emitter_shape: PARTICLE_SHAPE_CONE,
        });
        commands
            .spawn(ParticleEmitter { rate: 0.0, accumulator: 0.0, burst: 4, speed_scale: 1.0 })
            .insert(Transform::from_translation(Vec3::new(0.0, 0.3, 0.5)))
            .insert(ParticleEffectHandle(effect.index()))
            .enable::<EmitterActive>();
    }

    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.6, 5.0), Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// The interpolation pair as a one-field bundle (a dense component's self-`Bundle` is suppressed;
/// `interp_smoke.rs`'s shape).
#[derive(boyko_macros::Bundle)]
struct InterpPair {
    pair: GpuTransform3D,
}

/// Re-arms one particle burst per frame (spawning is burst-driven, independent of `dt`).
#[allow(clippy::needless_pass_by_value)]
fn obs_arm_burst(mut emitters: Query<&mut ParticleEmitter>) {
    for e in emitters.iter_mut() {
        e.burst = 4;
    }
}

/// A DM1 Tier-1 edit on every frame: the emissive alternates, so every frame stages an upload and
/// declares `material_table` (the resource is declared on upload frames only).
#[allow(clippy::needless_pass_by_value)]
fn obs_edit_material(stats: Res<HostFrameStats>, edited: Res<EditedMaterial>, mut materials: ResMut<Assets<Material>>) {
    let Some(h) = edited.0 else {
        return;
    };
    let mat = materials.get_mut(h).expect("invariant: the edited material is live");
    let g = if stats.frames.is_multiple_of(2) { 1.5 } else { 1.25 };
    mat.gpu.emissive = [0.0, g, 0.0, 0.0];
}

/// Builds the max-config OBS app for `arming`: mesh (+ SDF where the path marches it), CSM and a
/// punctual atlas shadow, TAA and SSAO where the path runs them, DDGI on both-leg Deferred / VB,
/// the HZB pyramid and the froxel light cull on VB (the clustered cull arms on VB only; measured:
/// `light_cull = 0` on the other paths with it requested), particles, the interpolated body, and a
/// material edited every frame.
pub fn build_obs_app(title: &'static str, arming: ObsArming) -> App {
    build_obs_app_with(EnginePlugins::window(title, OBS_WIN, OBS_WIN), arming)
}

/// [`build_obs_app`] over caller-built plugins — render wave PX1a's receipt drivers boot SSAA,
/// which only `EnginePlugins::with_ssaa_scale` arms (the plugins must be `OBS_WIN` square).
pub fn build_obs_app_with(plugins: EnginePlugins, arming: ObsArming) -> App {
    let mut app = App::new();
    app.add_plugins(plugins);
    app.insert_resource(ObsArmingRes(arming));
    app.insert_resource(EditedMaterial::default());
    app.add_startup_system(obs_setup);
    // Configuration AFTER `add_plugins`, which seeds every default.
    app.insert_resource(RenderPathConfig { path: arming.path, legs: arming.legs() });
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    match arming.path {
        RenderPath::Deferred => {
            if arming.taa {
                app.insert_resource(AaConfig { mode: AaMode::Taa });
            }
            app.insert_resource(SsaoConfig { quality: SsaoQuality::High, ..Default::default() });
            if arming.ddgi {
                app.insert_resource(DdgiConfig { ddgi_indirect: true, ..Default::default() });
            }
        }
        RenderPath::VisibilityBuffer => {
            if arming.taa {
                app.insert_resource(AaConfig { mode: AaMode::Taa });
            }
            app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
            if arming.ddgi {
                app.insert_resource(DdgiConfig { ddgi_indirect: true, ..Default::default() });
            }
            app.insert_resource(HzbConfig { mode: HzbMode::Build });
            app.insert_resource(boyko_render::LightingConfig { clusters_enabled: true, ..boyko_render::LightingConfig::default() });
            app.insert_resource(boyko_render::ClusterConfig::default());
        }
        _ => {}
    }
    if arming.particles {
        app.insert_resource(ParticleConfig { mode: ParticleMode::GpuUnlit, capacity: 4096, ..ParticleConfig::default() });
        app.add_systems(obs_arm_burst);
    }
    if arming.upload {
        app.add_systems(obs_edit_material);
    }
    app
}

/// Asserts the boot resolved the asked path — a degraded boot proves nothing about that path.
pub fn assert_obs_resolved(app: &App, arming: ObsArming) {
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran (no windowed device?) — RED, never a skip");
    let resolved = *app.world().resource::<ResolvedRenderPath>();
    assert_eq!(resolved.path, arming.path, "PREMISE: asked for {:?}, the boot resolved {:?}", arming.path, resolved.path);
}

// ===============================================================================================
// The driver side
// ===============================================================================================

/// Every dump instrument the OBS drivers clear from a child's environment before setting their
/// own: an inherited one would hold the loop open, or exclude the instrument under test.
pub const FOREIGN_INSTRUMENTS: [&str; 10] = [
    "BOYKO_HOST_DUMP",
    "BOYKO_VG_CENSUS",
    "BOYKO_VB_PROBE",
    "BOYKO_HZB_DUMP",
    "BOYKO_VB_CULL_READBACK",
    "BOYKO_PARTICLE_READBACK_FRAME",
    "BOYKO_SHADOW_POISON_PROBE",
    "BOYKO_GRAPH_RECEIPT",
    "BOYKO_IMAGE_DUMP",
    "BOYKO_VG_OCC_FORCE",
];

/// A fresh output path under the temp dir (a stale file is removed so it cannot be read as this
/// run's evidence).
pub fn fresh_out(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("boyko_obs");
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let p = dir.join(name);
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(&p);
    } else {
        let _ = std::fs::remove_file(&p);
    }
    p
}

/// Spawns this test binary's `worker` (by name, `--exact`) with `env` over a cleaned instrument
/// environment, and asserts it exited 0. `validation_on` arms the layer for the child
/// (`BOYKO_ENABLE_VALIDATION`, the parent's `BOYKO_DISABLE_VALIDATION` removed — critique O7 — and
/// the layer's message limit off, the `boot_validation_clean` recipe); otherwise it is disabled.
pub fn run_worker(worker: &str, env: &[(&str, String)], validation_on: bool) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"]);
    for k in FOREIGN_INSTRUMENTS {
        cmd.env_remove(k);
    }
    if validation_on {
        cmd.env_remove("BOYKO_DISABLE_VALIDATION")
            .env("BOYKO_ENABLE_VALIDATION", "1")
            .env("VK_KHRONOS_VALIDATION_ENABLE_MESSAGE_LIMIT", "false");
    } else {
        cmd.env("BOYKO_DISABLE_VALIDATION", "1");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "the OBS worker `{worker}` ({env:?}) exited {status}");
}
