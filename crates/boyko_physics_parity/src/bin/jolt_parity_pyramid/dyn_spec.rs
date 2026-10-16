//! Dynamic parity scenes (lane DYN-SCENES): the shared PROGRAM SPECIFICATION.
//!
//! One file, read three ways: the parity runner (`benches/jolt_parity_pyramid.rs`, through
//! `#[path]`), its known-answer test (`tests/dyn_scenes_spec.rs`, through `#[path]`) and the Rapier
//! harness's `dyn/` arms (`D:/tmp/rapier-parity/dyn/src/spec.rs`, a byte copy, checked by `cmp`).
//! Jolt's patch carries an independent C++ derivation and `docs/physics/perf-campaign/levers/
//! scenes/gate/dyn_spec_ref.py` an independent Python one; the canonical scene dump below is the
//! text all of them must produce byte for byte.
//!
//! Pure: no engine type and no allocation. Every program value is integer arithmetic;
//! the only floats are exact conversions `int × 2^-k` (positions in 1/16 m, velocities in 1/256 m/s,
//! velocity changes in 1/64 m/s), so no compiler's FMA contraction or rounding mode can change a
//! bit (Jolt's build passes `-ffp-contract=on -mfma`).
//!
//! # The scenes (`docs/physics/perf-campaign/levers/scenes/01-DESIGN.md`)
//!
//! | program | steps | metric window | statics | events |
//! |---|---|---|---|---|
//! | `none` (S-LAND) | 500 | [0, 100) | the J-T floor | none: J-T itself |
//! | `kick` (S-KICK) | 800 | [200, 800) | the J-T floor | 24 kicks of 62 boxes, steps 200 + 25k |
//! | `shoot` (S-SHOOT) | 800 | [200, 800) | floor + 4 walls + ceiling | 60 J-T boxes launched, steps 200 + 10k |
//! | `slide` (S-SLIDE) | 500 | [0, 500) | floor + 4 walls (+x at x = 31) + ceiling | none; gravity tilted, tan θ = 1/2 |
//!
//! An event "at step t" is applied after the state after t steps is read and before the step of
//! loop index t runs, outside the timed pair. A kick is a velocity change `v ← v + Δv` (an f32
//! add per component), never an impulse, so it is the same in every engine whatever its density.
//!
//! The arena's statics are large on purpose: floors and walls are what games have, and "large
//! statics" is a declared part of what S-SHOOT and S-SLIDE measure (ruling 22).
// Shared by `#[path]` into a bench, a test and the Rapier harness: each reads a different subset,
// so an item one of them leaves unused is not dead.
#![allow(dead_code)]

use core::fmt::{self, Write};

// ── Identity ─────────────────────────────────────────────────────────────────────────────────────

/// The canonical scene dump's first line.
pub const SCHEMA: &str = "boyko-dyn-scene v1";

// ── The J-T scene (Jolt `PyramidScene.h`, every harness's existing J-T spawn) ────────────────────

/// Dynamic boxes in the J-T pyramid.
pub const JT_BODIES: usize = 1240;
/// `cPyramidHeight`.
pub const JT_HEIGHT: i32 = 15;
/// `cBoxSize`.
pub const JT_BOX_SIZE: f32 = 2.0;
/// Every box's half-extent on every axis.
pub const JT_HALF: f32 = 1.0;
/// `cBoxSeparation`, the J-T layer gap.
pub const JT_GAP: f32 = 0.5;
/// Friction on every body and static.
pub const FRICTION: f32 = 0.2;
/// Restitution on every body and static.
pub const RESTITUTION: f32 = 0.0;
/// `cDeltaTime`: bits `0x3c888889`.
pub const DT: f32 = 1.0 / 60.0;
/// J-T gravity along y.
pub const GRAVITY_Y: f32 = -9.81;
/// S-SLIDE's gravity, `9.81 · (sin θ, −cos θ, 0)` with tan θ = 1/2, rounded once to f32: x is
/// `0x408c63a9` (4.3871655) and y exactly −2·x (`0xc10c63a9`), so the slope is exactly 1/2 in f32.
pub const SLIDE_GRAVITY_BITS: [u32; 3] = [0x408c_63a9, 0xc10c_63a9, 0];

// ── The programs ─────────────────────────────────────────────────────────────────────────────────

/// S-KICK's PRNG seed.
pub const KICK_SEED: u64 = 0x6b69_636b_0000_0001;
/// S-SHOOT's PRNG seed.
pub const SHOOT_SEED: u64 = 0x7368_6f6f_0000_0001;
/// The untimed settle prefix of S-KICK and S-SHOOT: J-T's own first steps.
pub const SETTLE_STEPS: u32 = 200;
/// S-KICK's events.
pub const KICK_EVENTS: usize = 24;
/// Steps between two kick events.
pub const KICK_CADENCE: u32 = 25;
/// Boxes one kick event changes (5 % of J-T).
pub const KICK_BOXES: usize = 62;
/// A Δv component's bound, in 1/64 m/s.
pub const KICK_K_MAX: i64 = 320;
/// The Δv shell's lower bound on |k|² (|Δv| ≥ 1 m/s).
pub const KICK_NORM2_MIN: i64 = 64 * 64;
/// The Δv shell's upper bound on |k|² (|Δv| ≤ 5 m/s).
pub const KICK_NORM2_MAX: i64 = KICK_K_MAX * KICK_K_MAX;
/// Δv's unit: 1/64 m/s.
pub const KICK_UNIT: f32 = 1.0 / 64.0;
/// S-SHOOT's launches.
pub const SHOOT_LAUNCHES: usize = 60;
/// Steps between two launches.
pub const SHOOT_CADENCE: u32 = 10;
/// The launch ring's radii, in 1/16 m (30 m and 34 m from the pile axis).
pub const RING_MIN: i64 = 480;
/// See [`RING_MIN`].
pub const RING_MAX: i64 = 544;
/// Launch heights, in 1/16 m (40 m to 48 m).
pub const LAUNCH_H: (i64, i64) = (640, 768);
/// Aim heights on the pile axis, in 1/16 m (4 m to 28 m).
pub const TARGET_H: (i64, i64) = (64, 448);
/// Launch speeds, in m/s.
pub const SPEED: (i64, i64) = (15, 40);
/// The pile axis x = z = −1 m, in 1/16 m.
pub const AXIS16: i64 = -16;
/// Positions' unit: 1/16 m.
pub const POS_UNIT: f32 = 1.0 / 16.0;
/// Velocities' unit: 1/256 m/s.
pub const VEL_UNIT: f32 = 1.0 / 256.0;

// ── The sanity bar (the scorer `gate/dyn_sanity.py` is the authority; these are its constants) ───

/// B3: the speed no body may exceed. The highest legitimate one is a 40 m/s shot from 48 m falling
/// to the floor, about 50 m/s.
pub const BAR_MAX_SPEED: f64 = 70.0;
/// B4: energy created from nothing, per unit J-T box mass, above the run's running minimum of
/// `e_total − e_inj`: `½ · (1 m/s)² · 1240`, every J-T box gaining 1 m/s. A design constant, fixed
/// before any engine ran against it (PC-DYN-8).
pub const BAR_ENERGY_EPS: f64 = 620.0;
/// B5: a launch's spawn point must be at least this far from every dynamic body's centre (two
/// J-T boxes' bounding spheres, 2·√3 m), so a spawn can never overlap a body.
pub const BAR_CLEARANCE: f64 = 3.464_101_615_137_754_4;
/// The sanity CSV's header (one row per state; row s = the state after s steps).
pub const SANITY_HEADER: &str = "step,e_lin,e_rot,e_pot,e_total,e_inj,max_speed,min_x,max_x,min_y,\
max_y,min_z,max_z,cx,cy,cz,nonfinite,bodies,clearance,engine_err";

/// A dynamic program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Program {
    /// S-LAND: J-T itself.
    None,
    /// S-KICK.
    Kick,
    /// S-SHOOT.
    Shoot,
    /// S-SLIDE.
    Slide,
}

/// One static box: integer half-extents and centre, in metres, identity rotation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StaticBox {
    /// Half-extents.
    pub half: [i32; 3],
    /// Centre.
    pub center: [i32; 3],
}

/// The J-T floor.
pub const FLOOR: StaticBox = StaticBox { half: [50, 1, 50], center: [0, -1, 0] };
/// The J-T statics: the floor alone.
const JT_STATICS: [StaticBox; 1] = [FLOOR];
/// S-SHOOT's arena: the floor, walls whose inner faces stand at x, z = ±50 (flush with the floor's
/// edge) and a ceiling whose face is at y = 61. Creation order: floor, +x, −x, +z, −z, ceiling.
const SHOOT_STATICS: [StaticBox; 6] = [
    FLOOR,
    StaticBox { half: [1, 31, 52], center: [51, 30, 0] },
    StaticBox { half: [1, 31, 52], center: [-51, 30, 0] },
    StaticBox { half: [52, 31, 1], center: [0, 30, 51] },
    StaticBox { half: [52, 31, 1], center: [0, 30, -51] },
    StaticBox { half: [52, 1, 52], center: [0, 62, 0] },
];
/// S-SLIDE's arena: S-SHOOT's with the +x (downhill) wall moved in to x = 31, its face at x = 30.
const SLIDE_STATICS: [StaticBox; 6] = [
    FLOOR,
    StaticBox { half: [1, 31, 52], center: [31, 30, 0] },
    StaticBox { half: [1, 31, 52], center: [-51, 30, 0] },
    StaticBox { half: [52, 31, 1], center: [0, 30, 51] },
    StaticBox { half: [52, 31, 1], center: [0, 30, -51] },
    StaticBox { half: [52, 1, 52], center: [0, 62, 0] },
];

impl Program {
    /// Every program, in the record's order.
    pub const ALL: [Self; 4] = [Self::None, Self::Kick, Self::Shoot, Self::Slide];

    /// By name, as `--dyn` / `-dyn=` spell it (`none` is J-T and has no flag value of its own).
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "kick" => Some(Self::Kick),
            "shoot" => Some(Self::Shoot),
            "slide" => Some(Self::Slide),
            _ => None,
        }
    }

    /// The program's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Kick => "kick",
            Self::Shoot => "shoot",
            Self::Slide => "slide",
        }
    }

    /// The program's length in steps.
    pub const fn steps(self) -> usize {
        match self {
            Self::None | Self::Slide => 500,
            Self::Kick | Self::Shoot => 800,
        }
    }

    /// The scene's metric window `[a, b)`.
    pub const fn metric_window(self) -> (usize, usize) {
        match self {
            Self::None => (0, 100),
            Self::Kick | Self::Shoot => (200, 800),
            Self::Slide => (0, 500),
        }
    }

    /// The statics, in creation order (floor first; created after the J-T boxes).
    pub const fn statics(self) -> &'static [StaticBox] {
        match self {
            Self::None | Self::Kick => &JT_STATICS,
            Self::Shoot => &SHOOT_STATICS,
            Self::Slide => &SLIDE_STATICS,
        }
    }

    /// Gravity, as f32 bits.
    pub const fn gravity_bits(self) -> [u32; 3] {
        match self {
            Self::Slide => SLIDE_GRAVITY_BITS,
            _ => [0, GRAVITY_Y.to_bits(), 0],
        }
    }

    /// Gravity.
    pub const fn gravity(self) -> [f32; 3] {
        let b = self.gravity_bits();
        [f32::from_bits(b[0]), f32::from_bits(b[1]), f32::from_bits(b[2])]
    }

    /// Events in the program (kick events or launches).
    pub const fn events(self) -> usize {
        match self {
            Self::None | Self::Slide => 0,
            Self::Kick => KICK_EVENTS,
            Self::Shoot => SHOOT_LAUNCHES,
        }
    }

    /// Dynamic bodies at the end of the program.
    pub const fn final_bodies(self) -> usize {
        match self {
            Self::Shoot => JT_BODIES + SHOOT_LAUNCHES,
            _ => JT_BODIES,
        }
    }

    /// The scene's bounds on a body centre (bar B2): `(min, max)` per axis.
    pub const fn bounds(self) -> ([f64; 3], [f64; 3]) {
        match self {
            Self::None | Self::Kick => ([-50.0, 0.0, -50.0], [50.0, f64::INFINITY, 50.0]),
            Self::Shoot => ([-50.0, 0.0, -50.0], [50.0, 61.0, 50.0]),
            Self::Slide => ([-50.0, 0.0, -50.0], [30.0, 61.0, 50.0]),
        }
    }
}

// ── The generator ────────────────────────────────────────────────────────────────────────────────

/// SplitMix64 (Steele, Lea, Flood 2014): seed-0 outputs `0xe220a8397b1dcdaf`, `0x6e789e6aa1b965f4`,
/// `0x06c45d188009454f`.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator at `seed`.
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next output.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, n)`, unbiased: outputs below `2^64 mod n` are rejected.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0, "invariant: below(0) has no value");
        let t = 0u64.wrapping_sub(n) % n;
        loop {
            let z = self.next_u64();
            if z >= t {
                return z % n;
            }
        }
    }

    /// Uniform in `[lo, hi]`, inclusive.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi, "invariant: an empty range has no value");
        lo + self.below((hi - lo + 1) as u64) as i64
    }
}

/// One box's velocity change.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct KickEntry {
    /// The J-T box, in spawn order.
    pub index: u32,
    /// Δv in 1/64 m/s.
    pub k: [i32; 3],
}

impl KickEntry {
    /// Δv in m/s (exact).
    pub fn dv(&self) -> [f32; 3] {
        [self.k[0] as f32 * KICK_UNIT, self.k[1] as f32 * KICK_UNIT, self.k[2] as f32 * KICK_UNIT]
    }
}

/// One kick event: [`KICK_BOXES`] distinct boxes, ascending by index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KickEvent {
    /// The step the event is applied before.
    pub step: u32,
    /// The kicked boxes, ascending by index.
    pub entries: [KickEntry; KICK_BOXES],
}

/// S-KICK's program: event e at step `200 + 25e`. Per event, 62 distinct indices are drawn first
/// (`below(1240)`, duplicates rejected, in draw order); then, for each index in draw order, a Δv
/// `k = range(−320, 320)³` redrawn until `64² ≤ |k|² ≤ 320²`; the event is applied in ascending
/// index order.
pub fn kick_program() -> [KickEvent; KICK_EVENTS] {
    let mut rng = SplitMix64::new(KICK_SEED);
    let empty = KickEvent { step: 0, entries: [KickEntry::default(); KICK_BOXES] };
    let mut events = [empty; KICK_EVENTS];
    for (e, event) in events.iter_mut().enumerate() {
        let mut indices = [0u32; KICK_BOXES];
        let mut n = 0;
        while n < KICK_BOXES {
            let c = rng.below(JT_BODIES as u64) as u32;
            if !indices[..n].contains(&c) {
                indices[n] = c;
                n += 1;
            }
        }
        for (slot, &index) in indices.iter().enumerate() {
            let k = loop {
                let k = [
                    rng.range(-KICK_K_MAX, KICK_K_MAX),
                    rng.range(-KICK_K_MAX, KICK_K_MAX),
                    rng.range(-KICK_K_MAX, KICK_K_MAX),
                ];
                let n2 = k[0] * k[0] + k[1] * k[1] + k[2] * k[2];
                if (KICK_NORM2_MIN..=KICK_NORM2_MAX).contains(&n2) {
                    break k;
                }
            };
            event.entries[slot] = KickEntry { index, k: [k[0] as i32, k[1] as i32, k[2] as i32] };
        }
        event.entries.sort_unstable_by_key(|x| x.index);
        event.step = SETTLE_STEPS + KICK_CADENCE * e as u32;
    }
    events
}

/// One projectile launch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Launch {
    /// The step the launch is applied before.
    pub step: u32,
    /// The projectile's body index, `1240 + k`.
    pub index: u32,
    /// The ring offset from the pile axis, in 1/16 m.
    pub ring: [i64; 2],
    /// The aim height on the pile axis, in 1/16 m.
    pub target_h: i64,
    /// The speed, in m/s.
    pub speed: i64,
    /// The spawn position, in 1/16 m.
    pub pos16: [i32; 3],
    /// The launch velocity, in 1/256 m/s.
    pub vel256: [i32; 3],
}

impl Launch {
    /// The spawn position in metres (exact).
    pub fn position(&self) -> [f32; 3] {
        [self.pos16[0] as f32 * POS_UNIT, self.pos16[1] as f32 * POS_UNIT, self.pos16[2] as f32 * POS_UNIT]
    }

    /// The launch velocity in m/s (exact).
    pub fn velocity(&self) -> [f32; 3] {
        [
            self.vel256[0] as f32 * VEL_UNIT,
            self.vel256[1] as f32 * VEL_UNIT,
            self.vel256[2] as f32 * VEL_UNIT,
        ]
    }
}

/// `⌊√x⌋`, exact.
pub fn isqrt(x: u64) -> u64 {
    x.isqrt()
}

/// S-SHOOT's program: launch k at step `200 + 10k`. Per launch, in this draw order: the ring
/// offset `(a, b) = range(−544, 544)²` redrawn until `480² ≤ a² + b² ≤ 544²` and, after the first,
/// until it is at least 30° from the previous launch (`dot ≤ 0` or `4·dot² ≤ 3·r²·r²_prev`); the
/// launch height `range(640, 768)`; the aim height `range(64, 448)`; the speed `range(15, 40)`.
/// The spawn point is `(a − 16, hL, b − 16)` in 1/16 m, the aim direction `d = (−a, hT − hL, −b)`,
/// and the velocity `trunc(s · 256 · dᵢ / ⌊√(d·d)⌋)` in 1/256 m/s (division toward zero).
pub fn shoot_program() -> [Launch; SHOOT_LAUNCHES] {
    let mut rng = SplitMix64::new(SHOOT_SEED);
    let mut launches = [Launch::default(); SHOOT_LAUNCHES];
    let mut prev: Option<(i64, i64)> = None;
    for (k, launch) in launches.iter_mut().enumerate() {
        let (a, b) = loop {
            let a = rng.range(-RING_MAX, RING_MAX);
            let b = rng.range(-RING_MAX, RING_MAX);
            let r2 = a * a + b * b;
            if !(RING_MIN * RING_MIN..=RING_MAX * RING_MAX).contains(&r2) {
                continue;
            }
            if let Some((pa, pb)) = prev {
                let dot = a * pa + b * pb;
                let pr2 = pa * pa + pb * pb;
                if !(dot <= 0 || 4 * dot * dot <= 3 * r2 * pr2) {
                    continue;
                }
            }
            break (a, b);
        };
        let hl = rng.range(LAUNCH_H.0, LAUNCH_H.1);
        let ht = rng.range(TARGET_H.0, TARGET_H.1);
        let s = rng.range(SPEED.0, SPEED.1);
        let d = [-a, ht - hl, -b];
        let n = isqrt((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as u64) as i64;
        let v = d.map(|di| (s * 256 * di) / n);
        *launch = Launch {
            step: SETTLE_STEPS + SHOOT_CADENCE * k as u32,
            index: (JT_BODIES + k) as u32,
            ring: [a, b],
            target_h: ht,
            speed: s,
            pos16: [(a + AXIS16) as i32, hl as i32, (b + AXIS16) as i32],
            vel256: [v[0] as i32, v[1] as i32, v[2] as i32],
        };
        prev = Some((a, b));
    }
    launches
}

/// The J-T box `n`'s spawn position, from the placement loop (`spawn_scene`'s expression, operand
/// for operand, so the f32 bits are every harness's).
pub fn jt_position(n: usize) -> [f32; 3] {
    let mut seen = 0usize;
    for i in 0..JT_HEIGHT {
        let lo = i / 2;
        let hi = JT_HEIGHT - (i + 1) / 2;
        let per_layer = ((hi - lo) * (hi - lo)) as usize;
        if n >= seen + per_layer {
            seen += per_layer;
            continue;
        }
        let r = (n - seen) as i32;
        let side = hi - lo;
        let (j, k) = (lo + r / side, lo + r % side);
        let odd = if i & 1 != 0 { 0.5 * JT_BOX_SIZE } else { 0.0 };
        return [
            -(JT_HEIGHT as f32) + JT_BOX_SIZE * j as f32 + odd,
            1.0 + (JT_BOX_SIZE + JT_GAP) * i as f32,
            -(JT_HEIGHT as f32) + JT_BOX_SIZE * k as f32 + odd,
        ];
    }
    [f32::NAN; 3]
}

// ── The canonical scene dump ─────────────────────────────────────────────────────────────────────

/// One f32 as 8 lowercase hex digits of its bits.
struct Hex(f32);

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0.to_bits())
    }
}

/// A dynamic body as a harness READ IT BACK from its engine after creating it (the dump is what the
/// engine holds, not what the harness meant to pass).
#[derive(Clone, Copy, Debug)]
pub struct BodyLine {
    /// The body's index in spawn order.
    pub index: u32,
    /// Position.
    pub p: [f32; 3],
    /// Rotation (x, y, z, w).
    pub q: [f32; 4],
    /// Linear velocity.
    pub v: [f32; 3],
    /// Angular velocity.
    pub w: [f32; 3],
    /// Box half-extents.
    pub half: [f32; 3],
    /// Friction.
    pub friction: f32,
    /// Restitution.
    pub restitution: f32,
    /// Convex radius (Jolt's `BoxShape`; 0 where the engine has none).
    pub radius: f32,
    /// Inverse mass over the first J-T box's.
    pub inv_mass_ratio: f32,
    /// Local inverse inertia diagonal over the first J-T box's.
    pub inv_inertia_ratio: [f32; 3],
    /// Linear damping.
    pub linear_damping: f32,
    /// Angular damping.
    pub angular_damping: f32,
    /// The body may sleep.
    pub can_sleep: bool,
    /// Continuous collision detection is on for it.
    pub ccd: bool,
}

impl BodyLine {
    /// The canonical J-T box `index` at `p` with velocity `v`, as every harness must read it back.
    pub fn jt_box(index: u32, p: [f32; 3], v: [f32; 3]) -> Self {
        Self {
            index,
            p,
            q: [0.0, 0.0, 0.0, 1.0],
            v,
            w: [0.0; 3],
            half: [JT_HALF; 3],
            friction: FRICTION,
            restitution: RESTITUTION,
            radius: 0.0,
            inv_mass_ratio: 1.0,
            inv_inertia_ratio: [1.0; 3],
            linear_damping: 0.0,
            angular_damping: 0.0,
            can_sleep: false,
            ccd: false,
        }
    }

    /// The fields after the tag: `<i> <p> <q> <v> <w> <half> <friction> <restitution> <radius>
    /// <inv_mass_ratio> <inv_inertia_ratio> <linear_damping> <angular_damping> <can_sleep> <ccd>`.
    fn write_fields(&self, out: &mut impl Write) -> fmt::Result {
        write!(out, "{}", self.index)?;
        for x in self.p.iter().chain(&self.q).chain(&self.v).chain(&self.w).chain(&self.half) {
            write!(out, " {}", Hex(*x))?;
        }
        write!(
            out,
            " {} {} {} {}",
            Hex(self.friction),
            Hex(self.restitution),
            Hex(self.radius),
            Hex(self.inv_mass_ratio)
        )?;
        for x in &self.inv_inertia_ratio {
            write!(out, " {}", Hex(*x))?;
        }
        write!(
            out,
            " {} {} {} {}",
            Hex(self.linear_damping),
            Hex(self.angular_damping),
            u8::from(self.can_sleep),
            u8::from(self.ccd)
        )
    }

    /// `body <fields>`: a body of the spawn, before the first step.
    pub fn write_body(&self, out: &mut impl Write) -> fmt::Result {
        out.write_str("body ")?;
        self.write_fields(out)?;
        out.write_char('\n')
    }

    /// `launch <step> <fields>`: a projectile, read back after its mid-run creation.
    pub fn write_launch(&self, step: u32, out: &mut impl Write) -> fmt::Result {
        write!(out, "launch {step} ")?;
        self.write_fields(out)?;
        out.write_char('\n')
    }
}

/// A static as a harness read it back.
#[derive(Clone, Copy, Debug)]
pub struct StaticLine {
    /// Half-extents.
    pub half: [f32; 3],
    /// Position.
    pub p: [f32; 3],
    /// Rotation (x, y, z, w).
    pub q: [f32; 4],
    /// Friction.
    pub friction: f32,
    /// Restitution.
    pub restitution: f32,
    /// Convex radius.
    pub radius: f32,
}

impl StaticLine {
    /// The canonical line of `s`.
    pub fn of(s: &StaticBox) -> Self {
        Self {
            half: s.half.map(|h| h as f32),
            p: s.center.map(|c| c as f32),
            q: [0.0, 0.0, 0.0, 1.0],
            friction: FRICTION,
            restitution: RESTITUTION,
            radius: 0.0,
        }
    }

    /// `static <half> <p> <q> <friction> <restitution> <radius>`.
    pub fn write(&self, out: &mut impl Write) -> fmt::Result {
        out.write_str("static")?;
        for x in self.half.iter().chain(&self.p).chain(&self.q) {
            write!(out, " {}", Hex(*x))?;
        }
        writeln!(out, " {} {} {}", Hex(self.friction), Hex(self.restitution), Hex(self.radius))
    }
}

/// The header: schema, program, steps, dt and gravity as the engine holds them.
pub fn write_header(program: Program, dt: f32, gravity: [f32; 3], out: &mut impl Write) -> fmt::Result {
    writeln!(out, "{SCHEMA}")?;
    writeln!(out, "program {}", program.name())?;
    writeln!(out, "steps {}", program.steps())?;
    writeln!(out, "dt {}", Hex(dt))?;
    writeln!(out, "gravity {} {} {}", Hex(gravity[0]), Hex(gravity[1]), Hex(gravity[2]))
}

/// `kick <step> <i> <dv>`: a velocity change the harness applied and verified by reading the
/// velocity back (`v_after == v_before + Δv`, bit for bit). A harness whose read-back disagrees
/// writes [`write_kick_mismatch`] instead, so the dump cannot match.
pub fn write_kick(step: u32, index: u32, dv: [f32; 3], out: &mut impl Write) -> fmt::Result {
    writeln!(out, "kick {step} {index} {} {} {}", Hex(dv[0]), Hex(dv[1]), Hex(dv[2]))
}

/// The line a harness writes when a kick's read-back velocity is not `v_before + Δv`.
pub fn write_kick_mismatch(step: u32, index: u32, out: &mut impl Write) -> fmt::Result {
    writeln!(out, "kick {step} {index} READBACK-MISMATCH")
}

/// `end <events applied> <dynamic bodies at the end>`.
pub fn write_end(events: usize, bodies: usize, out: &mut impl Write) -> fmt::Result {
    writeln!(out, "end {events} {bodies}")
}

/// The canonical dump of `program`: what every harness's `--scene-dump` must equal byte for byte.
pub fn canonical_dump(program: Program, out: &mut impl Write) -> fmt::Result {
    write_header(program, DT, program.gravity(), out)?;
    for s in program.statics() {
        StaticLine::of(s).write(out)?;
    }
    for n in 0..JT_BODIES {
        BodyLine::jt_box(n as u32, jt_position(n), [0.0; 3]).write_body(out)?;
    }
    match program {
        Program::None | Program::Slide => {}
        Program::Kick => {
            for event in kick_program() {
                for e in &event.entries {
                    write_kick(event.step, e.index, e.dv(), out)?;
                }
            }
        }
        Program::Shoot => {
            for l in shoot_program() {
                BodyLine::jt_box(l.index, l.position(), l.velocity()).write_launch(l.step, out)?;
            }
        }
    }
    write_end(program.events(), program.final_bodies(), out)
}

// ── The sanity readout ───────────────────────────────────────────────────────────────────────────

/// One body's mechanical energy per unit J-T box mass: `½|v|² + ⅓|ω|² − g·p` (a cube of side 2 has
/// `I = ⅔ m` about every axis, so `½ I |ω|² = ⅓ m |ω|²`).
pub fn energy(p: [f32; 3], v: [f32; 3], w: [f32; 3], g: [f32; 3]) -> f64 {
    let d = |a: [f32; 3], b: [f32; 3]| {
        f64::from(a[0]) * f64::from(b[0]) + f64::from(a[1]) * f64::from(b[1]) + f64::from(a[2]) * f64::from(b[2])
    };
    0.5 * d(v, v) + d(w, w) / 3.0 - d(g, p)
}

/// One row of the sanity CSV ([`SANITY_HEADER`]): the state after `step` steps, accumulated body by
/// body.
#[derive(Clone, Copy, Debug)]
pub struct SanityRow {
    step: usize,
    g: [f32; 3],
    e_lin: f64,
    e_rot: f64,
    e_pot: f64,
    e_inj: f64,
    max_speed: f64,
    min: [f64; 3],
    max: [f64; 3],
    sum: [f64; 3],
    nonfinite: u32,
    bodies: u32,
    clearance: Option<f64>,
    engine_err: u32,
}

impl SanityRow {
    /// An empty row for the state after `step` steps, under gravity `g`, with `e_inj` the energy
    /// the events applied before step `step` injected (each event's injection is the energy of the
    /// bodies it changed just after it minus just before it).
    pub fn new(step: usize, g: [f32; 3], e_inj: f64) -> Self {
        Self {
            step,
            g,
            e_lin: 0.0,
            e_rot: 0.0,
            e_pot: 0.0,
            e_inj,
            max_speed: 0.0,
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
            sum: [0.0; 3],
            nonfinite: 0,
            bodies: 0,
            clearance: None,
            engine_err: 0,
        }
    }

    /// Adds one dynamic body's state.
    pub fn add_body(&mut self, p: [f32; 3], q: [f32; 4], v: [f32; 3], w: [f32; 3]) {
        self.bodies += 1;
        if !(p.iter().chain(&q).chain(&v).chain(&w).all(|x| x.is_finite())) {
            self.nonfinite += 1;
        }
        let d = |a: [f32; 3], b: [f32; 3]| {
            f64::from(a[0]) * f64::from(b[0]) + f64::from(a[1]) * f64::from(b[1]) + f64::from(a[2]) * f64::from(b[2])
        };
        let v2 = d(v, v);
        self.e_lin += 0.5 * v2;
        self.e_rot += d(w, w) / 3.0;
        self.e_pot -= d(self.g, p);
        // NaN never compares greater, so a non-finite speed is counted by `nonfinite` instead.
        if v2.sqrt() > self.max_speed {
            self.max_speed = v2.sqrt();
        }
        for (a, &pa) in p.iter().enumerate() {
            let x = f64::from(pa);
            self.min[a] = self.min[a].min(x);
            self.max[a] = self.max[a].max(x);
            self.sum[a] += x;
        }
    }

    /// Records the clearance of the launch applied after this state (bar B5).
    pub fn set_clearance(&mut self, clearance: f64) {
        self.clearance = Some(clearance);
    }

    /// Records the engine's own error flags for the step (Jolt's `EPhysicsUpdateError`; 0 where the
    /// engine reports none).
    pub fn set_engine_err(&mut self, err: u32) {
        self.engine_err = err;
    }

    /// `e_lin + e_rot + e_pot`.
    pub fn e_total(&self) -> f64 {
        self.e_lin + self.e_rot + self.e_pot
    }

    /// The largest speed of the row.
    pub fn max_speed(&self) -> f64 {
        self.max_speed
    }

    /// Bodies with a non-finite state value.
    pub fn nonfinite(&self) -> u32 {
        self.nonfinite
    }

    /// The lowest body centre.
    pub fn min_y(&self) -> f64 {
        self.min[1]
    }

    /// Bodies whose centre is outside `program`'s bounds (bar B2).
    pub fn escaped(&self, program: Program) -> bool {
        let (lo, hi) = program.bounds();
        (0..3).any(|a| self.min[a] < lo[a] || self.max[a] > hi[a])
    }

    /// The row as one CSV line ([`SANITY_HEADER`]'s columns), newline included.
    pub fn write_csv(&self, out: &mut impl Write) -> fmt::Result {
        let n = f64::from(self.bodies.max(1));
        write!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},",
            self.step,
            self.e_lin,
            self.e_rot,
            self.e_pot,
            self.e_total(),
            self.e_inj,
            self.max_speed,
            self.min[0],
            self.max[0],
            self.min[1],
            self.max[1],
            self.min[2],
            self.max[2],
            self.sum[0] / n,
            self.sum[1] / n,
            self.sum[2] / n,
            self.nonfinite,
            self.bodies,
        )?;
        if let Some(c) = self.clearance {
            write!(out, "{c}")?;
        }
        writeln!(out, ",{}", self.engine_err)
    }
}

/// The distance from `p` to the nearest of `centres`: a launch's clearance (bar B5).
pub fn clearance(p: [f32; 3], centres: impl Iterator<Item = [f32; 3]>) -> f64 {
    centres
        .map(|c| {
            let d = [0, 1, 2].map(|a| f64::from(c[a]) - f64::from(p[a]));
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .fold(f64::INFINITY, f64::min)
}
