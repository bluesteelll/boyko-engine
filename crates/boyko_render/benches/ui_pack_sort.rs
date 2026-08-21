//! UI-ADVANCED S2 measurement §10.2 (`docs/UI-PLAN-SPRITES.md` §5): wall-clock
//! pack+sort at N ∈ {256, 2048} over ONE deterministic scene, run on BOTH sides of
//! the D1 widening (the 64 B build before commit B, the 80 B build after), criterion
//! median of each build.
//!
//! The ring-traffic half of §10.2 is arithmetic and is stated as such in the landing
//! report (N × stride, touched twice by the sort gather); THIS file is the
//! non-arithmetic half — whether the pack loop + the `(stack, append)` key sort +
//! the gather permutation (which copies every record twice: once into `pack`, once
//! through the sort gather) actually notice 16 more bytes per record.
//!
//! Scene: N nodes, every 4th clipped, half with a corner radius, stacks striped over
//! 16 values (so the sort does real work), scale 1.0. No GPU, no world — the same
//! `pack_ui_instance` + `UiRenderScratch::sort_by_stack` core the upload seam drives.
//!
//! Run: `cargo bench -p boyko-render --bench ui_pack_sort`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use boyko_render::{pack_ui_instance, PackInput, UiRenderScratch};

/// The deterministic §10.2 scene: identical on both builds by construction (pure
/// arithmetic on the index, no RNG, no time).
fn scene(n: usize) -> Vec<(PackInput, u32)> {
    (0..n)
        .map(|i| {
            let f = i as f32;
            let clipped = i % 4 == 0;
            let rounded = i % 2 == 0;
            let input = PackInput {
                rect: [f % 640.0, (f * 0.618) % 480.0, 24.0, 16.0],
                color: 0xFF00_00FF ^ (i as u32).wrapping_mul(0x9E37_79B1),
                border_color: 0,
                corner_radius: if rounded { [4.0; 4] } else { [0.0; 4] },
                border_width: [0.0; 4],
                clip: if clipped { Some([8.0, 8.0, 320.0, 240.0]) } else { None },
                text_uv: None,
                image: None,
            };
            // 16 stack strata, deliberately NOT in append order, so the sort permutes.
            let stack = ((i * 7) % 16) as u32;
            (input, stack)
        })
        .collect()
}

fn bench_pack_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("ui_pack_sort");
    for &n in &[256usize, 2048] {
        let nodes = scene(n);
        let mut scratch = UiRenderScratch::default();
        let mut gather = Vec::new();
        group.bench_with_input(BenchmarkId::new("pack_sort", n), &nodes, |b, nodes| {
            b.iter(|| {
                // The steady-state frame: clear + extend into the reused scratch
                // (capacity persists after the first iteration), then the in-place
                // total-order z-sort — the exact A1 step-2/step-3 core.
                scratch.pack.clear();
                scratch.keys.clear();
                for (append, (input, stack)) in nodes.iter().enumerate() {
                    scratch.pack.push(pack_ui_instance(input, 1.0));
                    scratch.keys.push((*stack, append as u32));
                }
                scratch.sort_by_stack(&mut gather);
                core::hint::black_box(scratch.pack.len())
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_pack_sort);
criterion_main!(benches);
