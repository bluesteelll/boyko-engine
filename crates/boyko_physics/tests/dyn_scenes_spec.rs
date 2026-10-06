//! Dynamic parity scenes (lane DYN-SCENES), C1: the shared program specification's known answers
//! and invariants.
//!
//! The spec lives beside the parity runner (`benches/jolt_parity_pyramid/dyn_spec.rs`) and is
//! included here by path, so this test reads the very file the runner and the Rapier harness
//! compile. The four canonical-dump pins are the FNV-1a 64 values an independent Python derivation
//! prints (`docs/physics/perf-campaign/levers/scenes/gate/dyn_spec_ref.py`): two derivations agreed
//! before anything was pinned. A change to a program moves its pin, by design: re-derive it with
//! the Python script, never by copying this test's output.

#[path = "../benches/jolt_parity_pyramid/dyn_spec.rs"]
mod spec;

use spec::{Program, SplitMix64};

/// FNV-1a 64.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn dump(program: Program) -> String {
    let mut s = String::new();
    spec::canonical_dump(program, &mut s).expect("invariant: writing to a String cannot fail");
    s
}

#[test]
fn splitmix64_known_answers() {
    let mut r = SplitMix64::new(0);
    assert_eq!(
        [r.next_u64(), r.next_u64(), r.next_u64()],
        [0xe220_a839_7b1d_cdaf, 0x6e78_9e6a_a1b9_65f4, 0x06c4_5d18_8009_454f],
        "splitmix64 known answer (seed 0)"
    );
}

#[test]
fn below_rejects_exactly_the_biased_tail() {
    for n in [1u64, 2, 3, 641, 1089, 1240, 1 << 63, u64::MAX] {
        let t = 0u64.wrapping_sub(n) % n;
        assert_eq!(u128::from(t), (1u128 << 64) % u128::from(n), "below({n}): the rejection bound is 2^64 mod n");
        let mut r = SplitMix64::new(n);
        for _ in 0..64 {
            assert!(r.below(n) < n, "below({n}) out of range");
        }
    }
    let mut r = SplitMix64::new(7);
    for _ in 0..1000 {
        let x = r.range(-3, 3);
        assert!((-3..=3).contains(&x), "range(-3, 3) gave {x}");
    }
}

#[test]
fn kick_program_invariants() {
    let events = spec::kick_program();
    assert_eq!(events.len(), 24);
    let mut kicks = 0;
    for (e, ev) in events.iter().enumerate() {
        assert_eq!(ev.step, 200 + 25 * e as u32, "event {e}'s step");
        for w in ev.entries.windows(2) {
            assert!(w[0].index < w[1].index, "event {e}: indices not distinct and ascending");
        }
        for x in &ev.entries {
            assert!((x.index as usize) < spec::JT_BODIES, "event {e}: index {} is not a J-T box", x.index);
            let n2: i64 = x.k.iter().map(|&c| i64::from(c) * i64::from(c)).sum();
            assert!((64 * 64..=320 * 320).contains(&n2), "event {e}: |k|^2 = {n2} outside the shell");
            assert!(x.k.iter().all(|c| (-320..=320).contains(c)), "event {e}: a component outside [-320, 320]");
            for (dv, k) in x.dv().into_iter().zip(x.k) {
                assert_eq!(f64::from(dv) * 64.0, f64::from(k), "event {e}: dv is not k/64 exactly");
            }
            kicks += 1;
        }
    }
    assert_eq!(kicks, 24 * 62);
    // Both signs occur on every axis, so a sign error anywhere moves the pin.
    for a in 0..3 {
        let all = events.iter().flat_map(|e| e.entries.iter().map(move |x| x.k[a]));
        assert!(all.clone().any(|c| c < 0) && all.clone().any(|c| c > 0), "axis {a}: one sign only");
    }
}

#[test]
fn shoot_program_invariants() {
    let launches = spec::shoot_program();
    assert_eq!(launches.len(), 60);
    let mut prev: Option<[i64; 2]> = None;
    let mut negative = [false; 3];
    for (k, l) in launches.iter().enumerate() {
        assert_eq!(l.step, 200 + 10 * k as u32, "launch {k}'s step");
        assert_eq!(l.index as usize, 1240 + k, "launch {k}'s body index");
        let [a, b] = l.ring;
        let r2 = a * a + b * b;
        assert!((480 * 480..=544 * 544).contains(&r2), "launch {k}: ring radius^2 {r2}");
        if let Some([pa, pb]) = prev {
            let dot = a * pa + b * pb;
            let pr2 = pa * pa + pb * pb;
            assert!(dot <= 0 || 4 * dot * dot <= 3 * r2 * pr2, "launch {k}: less than 30 degrees from the previous one");
        }
        prev = Some(l.ring);
        let hl = i64::from(l.pos16[1]);
        assert!((640..=768).contains(&hl), "launch {k}: launch height {hl}");
        assert!((64..=448).contains(&l.target_h), "launch {k}: aim height {}", l.target_h);
        assert!((15..=40).contains(&l.speed), "launch {k}: speed {}", l.speed);
        assert_eq!([i64::from(l.pos16[0]), i64::from(l.pos16[2])], [a - 16, b - 16], "launch {k}: spawn point");
        let d = [-a, l.target_h - hl, -b];
        let dd = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as u64;
        let n = spec::isqrt(dd);
        assert!(n * n <= dd && dd < (n + 1) * (n + 1), "launch {k}: isqrt({dd}) = {n}");
        for i in 0..3 {
            let want = (l.speed * 256 * d[i]) / n as i64;
            assert_eq!(i64::from(l.vel256[i]), want, "launch {k}: velocity component {i}");
            negative[i] |= l.vel256[i] < 0;
        }
        // The speed is s within truncation: each component is cut toward zero by less than one
        // 1/256 m/s unit from `s·256·d/n`, whose norm is `s·256·√(d·d)/n` (≥ s·256, since n is the
        // floor of the root).
        let v = l.velocity();
        let v256 = l.vel256.iter().map(|&c| f64::from(c) * f64::from(c)).sum::<f64>().sqrt();
        let ideal = l.speed as f64 * 256.0 * (dd as f64).sqrt() / n as f64;
        let cut = ideal - v256;
        assert!(
            (-1e-9..=3f64.sqrt() + 1e-9).contains(&cut) && ideal >= l.speed as f64 * 256.0,
            "launch {k}: |v| = {} m/s, s = {}, untruncated {} m/s",
            v256 / 256.0,
            l.speed,
            ideal / 256.0
        );
        let p = l.position();
        for i in 0..3 {
            assert_eq!(f64::from(p[i]) * 16.0, f64::from(l.pos16[i]), "launch {k}: position is not pos16/16 exactly");
            assert_eq!(f64::from(v[i]) * 256.0, f64::from(l.vel256[i]), "launch {k}: velocity is not vel256/256 exactly");
        }
    }
    assert_eq!(negative, [true; 3], "every axis must draw a negative component, or truncation is not exercised");
}

#[test]
fn slide_gravity_bits() {
    let g = Program::Slide.gravity();
    assert_eq!(Program::Slide.gravity_bits(), [0x408c_63a9, 0xc10c_63a9, 0]);
    assert_eq!(g[1], -2.0 * g[0], "the slope is exactly 1/2");
    let norm = (f64::from(g[0]).powi(2) + f64::from(g[1]).powi(2)).sqrt();
    assert!((norm - 9.81).abs() < 1e-5, "|g| = {norm}");
    assert_eq!(Program::Kick.gravity_bits(), [0, (-9.81f32).to_bits(), 0]);
}

#[test]
fn dump_floats_are_exact_program_integers() {
    // Every program-derived float is an exact multiple of its unit: k/64, pos/16, vel/256.
    let parse = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).expect("hex"));
    for program in Program::ALL {
        let text = dump(program);
        for line in text.lines() {
            let t: Vec<&str> = line.split(' ').collect();
            match t[0] {
                "kick" => {
                    for h in &t[3..6] {
                        let x = f64::from(parse(h)) * 64.0;
                        assert!(x.fract() == 0.0 && x.abs() <= 320.0, "{line}");
                    }
                }
                "launch" => {
                    for h in &t[3..6] {
                        let x = f64::from(parse(h)) * 16.0;
                        assert!(x.fract() == 0.0 && x.abs() <= 768.0, "{line}");
                    }
                    for h in &t[10..13] {
                        let x = f64::from(parse(h)) * 256.0;
                        assert!(x.fract() == 0.0 && x.abs() <= 40.0 * 256.0, "{line}");
                    }
                }
                "body" => {
                    for h in &t[2..5] {
                        let x = f64::from(parse(h)) * 2.0;
                        assert!(x.fract() == 0.0, "{line}");
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
fn canonical_dump_pins() {
    // (program, FNV-1a 64 of the dump, bytes, lines), as `dyn_spec_ref.py` prints them.
    let pins = [
        (Program::None, 0x90a2_7955_d181_1c55_u64, 295_474, 1247),
        (Program::Kick, 0x9646_22eb_d490_2cd7, 355_172, 2735),
        (Program::Shoot, 0x7d80_4c68_b157_a6ac, 310_796, 1312),
        (Program::Slide, 0x3a62_4339_18d7_1628, 296_095, 1252),
    ];
    let mut failed = Vec::new();
    for (program, fnv, bytes, lines) in pins {
        let text = dump(program);
        let got = (fnv1a64(text.as_bytes()), text.len(), text.lines().count());
        if got != (fnv, bytes, lines) {
            failed.push(format!(
                "{}: dump {:#018x} / {} bytes / {} lines, pinned {fnv:#018x} / {bytes} / {lines}",
                program.name(),
                got.0,
                got.1,
                got.2
            ));
        }
    }
    assert!(failed.is_empty(), "canonical dump pins moved: {failed:#?}");
}

#[test]
fn programs_are_self_consistent() {
    for program in Program::ALL {
        let text = dump(program);
        let last = text.lines().last().expect("a dump has lines");
        assert_eq!(last, format!("end {} {}", program.events(), program.final_bodies()));
        let (a, b) = program.metric_window();
        assert!(a < b && b <= program.steps());
        assert_eq!(Program::parse(program.name()), Some(program));
        assert_eq!(program.statics()[0], spec::FLOOR, "the floor is always the first static");
    }
}
