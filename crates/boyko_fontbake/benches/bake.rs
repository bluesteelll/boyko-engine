//! Load-time bake microbenchmarks (criterion).
//!
//! All of this runs at setup/build time, NOT on the engine render hot path, so
//! the numbers are INFORMATIONAL (the plan's "MSDF bake time per glyph"
//! metric) — they bound load latency, not frame time. Measured paths:
//!
//! - per-glyph MSDF field generation (the per-texel distance + sign + error
//!   passes), single-threaded scalar reference path;
//! - whole-atlas bake over an ASCII set;
//! - `.bfont` write + read (serialization round-trip).
//!
//! The fixture is the checked-in libre `Ubuntu-Light.ttf`. If it is absent the
//! benches no-op (criterion still runs, with a tiny synthetic input where
//! possible) so `cargo bench` never hard-fails on a missing asset.

use std::hint::black_box;
use std::path::PathBuf;

use boyko_fontbake::extract::extract_codepoint;
use boyko_fontbake::msdf::generate_glyph_field;
use boyko_fontbake::{TtfFace, bake_font, read_bfont, write_bfont};
use criterion::{Criterion, criterion_group, criterion_main};

fn load_face() -> Option<TtfFace> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("Ubuntu-Light.ttf");
    let bytes = std::fs::read(path).ok()?;
    TtfFace::from_bytes(&bytes)
}

fn bench_glyph_field(c: &mut Criterion) {
    let Some(face) = load_face() else {
        eprintln!("bench_glyph_field skipped: fixture missing");
        return;
    };
    let mut group = c.benchmark_group("msdf_glyph_field");
    // Representative glyphs of different complexity:
    //  '.' simple (1 contour), 'o' two smooth rings, 'A' lines + counter,
    //  '8' three contours (the heaviest).
    for &cp in &['.', 'o', 'A', '8'] {
        let g = extract_codepoint(&face, cp);
        group.bench_function(format!("generate '{cp}'"), |b| {
            b.iter(|| {
                let f = generate_glyph_field(black_box(&g.outline), None);
                black_box(f);
            });
        });
    }
    group.finish();
}

fn bench_extract(c: &mut Criterion) {
    let Some(face) = load_face() else {
        return;
    };
    c.bench_function("extract_codepoint 'A'", |b| {
        b.iter(|| black_box(extract_codepoint(black_box(&face), black_box('A'))));
    });
}

fn bench_bake_atlas(c: &mut Criterion) {
    let Some(face) = load_face() else {
        return;
    };
    let ascii: Vec<char> = (b'!'..=b'~').map(|b| b as char).collect();
    let mut group = c.benchmark_group("bake_atlas");
    group.sample_size(10); // a full ASCII bake is heavy; fewer samples
    group.bench_function("ASCII printable (94 glyphs)", |b| {
        b.iter(|| {
            let baked = bake_font(black_box(&face), black_box(&ascii), None);
            black_box(baked);
        });
    });
    group.finish();
}

fn bench_bfont_serialization(c: &mut Criterion) {
    let Some(face) = load_face() else {
        return;
    };
    let baked = bake_font(&face, &"Ao.8eOi ".chars().collect::<Vec<_>>(), None);
    let bytes = write_bfont(&baked);

    let mut group = c.benchmark_group("bfont");
    group.bench_function("write", |b| {
        b.iter(|| black_box(write_bfont(black_box(&baked))));
    });
    group.bench_function("read", |b| {
        b.iter(|| black_box(read_bfont(black_box(&bytes))));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_glyph_field,
    bench_extract,
    bench_bake_atlas,
    bench_bfont_serialization
);
criterion_main!(benches);
