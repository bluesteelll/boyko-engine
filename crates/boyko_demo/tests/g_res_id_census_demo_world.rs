//! **G-RES (plan gate UG-20, record-only) and the id census (AL:M-A13 / MD:M-K3) for
//! `boyko_demo`'s world** — rung B2's (D) row.
//!
//! # The demo has no `App`
//!
//! `boyko_demo` is an `EcsMaster` driven by a `SimRunner` inside an eframe app. `DemoApp::new`
//! needs eframe's creation context (a window and a wgpu device), so this test builds a replica of
//! its WORLD HALF — every statement from `let mut world = EcsMaster::with_capacity(..)` to
//! `let runner = SimRunner::new(..)`, in order, with `ParticleSpawner::resolve` inlined (it is
//! private) — and a **drift guard** holds the replica to the source: [`demo_world_half_is_the_replica`]
//! parses `src/app.rs` and requires that span of `DemoApp::new`, the body of
//! `ParticleSpawner::resolve` and the `WORLD_ENTITY_CAPACITY` initialiser to print exactly as
//! [`DEMO_WORLD_HALF`] records. A change to how the demo builds its world turns the guard RED
//! until the replica and the record are updated together.
//!
//! # Reading points (M-K3 mapped onto a world without `App`)
//!
//! * `config` — the world is built and the spawner resolved, `SimRunner::new` not yet called (the
//!   schedule is built inside it): the analog of the config-phase mod-load slot.
//! * `after_runner_new` — the analog of "immediately after `App::finish()`".
//! * `after_step_1` — the first step (its synthesized `on_enter(Particles)` spawns the cloud).
//! * `steady_64` — M-A13's steady state, after 64 steps.
//!
//! `G-RES app=demo point=after_boot` is read after `SimRunner::new` plus one step.
//!
//! The demo links no physics, so the pinned scan is expected to read 0 here — recorded, not
//! asserted, so a demo that later composes physics is a recorded change, not a red.
//!
//! The shared scans are `#[path]`-included from `boyko_app`'s `tests/b2_common/census_scan.rs`
//! (it depends on `boyko_ecs` and `boyko_macros` only).

#![cfg(not(target_arch = "wasm32"))]

#[path = "../../boyko_app/tests/b2_common/census_scan.rs"]
mod census_scan;

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;
use quote::ToTokens;

use boyko_demo::render::instance::GpuInstance;
use boyko_demo::sim::bundles::ParticleBundle;
use boyko_demo::sim::components::{ParticleTag, Position, Velocity};
use boyko_demo::sim::modes::PARTICLE_COUNT;
use boyko_demo::sim::resources::{InputState, SimParams};
use boyko_demo::sim::runner::SimRunner;

use census_scan::{g_res_line, id_cross_check, id_line};

/// One fixed step at the engine's 64 Hz, so each `step` runs exactly one substep.
const FIXED_DT: f32 = 1.0 / 64.0;
/// M-A13's steady state for the demo.
const STEADY_STEPS: usize = 64;

/// `DemoApp::new`'s world half, as `quote` prints it with whitespace dropped between punctuation
/// (`key_of`). Recorded on `u/b2` @ `c1e9f1db`.
const DEMO_WORLD_HALF: &[&str] = &[
    "let mut world=EcsMaster::with_capacity(WORLD_ENTITY_CAPACITY,2);",
    "world.insert_resource(InputState::default());",
    "world.insert_resource(SimParams::default());",
    "let spawner=ParticleSpawner::resolve(&mut world);",
    "#[cfg(not(target_arch=\"wasm32\"))]let pool=ThreadPoolBuilder::new().build();",
    "#[cfg(not(target_arch=\"wasm32\"))]let runner=SimRunner::new(Arc::clone(&pool),&mut world);",
    "#[cfg(target_arch=\"wasm32\")]let runner=SimRunner::new(&mut world);",
];
/// `ParticleSpawner::resolve`'s body.
const SPAWNER_RESOLVE: &str = "{Self{archetype:world.bundle_archetype_id_for::<ParticleBundle>(),\
pos_id:Position::component_id(),vel_id:Velocity::component_id(),gpu_id:GpuInstance::component_id(),\
tag_id:ParticleTag::component_id(),}}";
/// `WORLD_ENTITY_CAPACITY`'s initialiser.
const CAPACITY_INIT: &str = "crate::sim::modes::PARTICLE_COUNT";

/// Prints tokens compactly: a space only between two adjacent words (ident / literal).
fn key_of(ts: proc_macro2::TokenStream) -> String {
    fn push(ts: proc_macro2::TokenStream, out: &mut String, prev_word: &mut bool) {
        use proc_macro2::{Delimiter, TokenTree};
        for tt in ts {
            match tt {
                TokenTree::Ident(i) => {
                    if *prev_word {
                        out.push(' ');
                    }
                    out.push_str(&i.to_string());
                    *prev_word = true;
                }
                TokenTree::Literal(l) => {
                    if *prev_word {
                        out.push(' ');
                    }
                    out.push_str(&l.to_string());
                    *prev_word = true;
                }
                TokenTree::Punct(p) => {
                    out.push(p.as_char());
                    *prev_word = false;
                }
                TokenTree::Group(g) => {
                    let (o, c) = match g.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => ("", ""),
                    };
                    out.push_str(o);
                    *prev_word = false;
                    push(g.stream(), out, prev_word);
                    out.push_str(c);
                    *prev_word = false;
                }
            }
        }
    }
    let mut out = String::new();
    let mut prev_word = false;
    push(ts, &mut out, &mut prev_word);
    out
}

/// The drift guard: the demo's world half still prints as [`DEMO_WORLD_HALF`], its spawner still
/// resolves the same ids, and its capacity is still the particle count.
#[test]
fn demo_world_half_is_the_replica() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs");
    let src = std::fs::read_to_string(path).expect("read boyko_demo/src/app.rs");
    let file = syn::parse_file(&src).expect("parse boyko_demo/src/app.rs");
    let mut world_half: Option<Vec<String>> = None;
    let mut resolve: Option<String> = None;
    let mut capacity: Option<String> = None;
    for item in &file.items {
        match item {
            syn::Item::Const(c) if c.ident == "WORLD_ENTITY_CAPACITY" => {
                capacity = Some(key_of(c.expr.to_token_stream()));
            }
            syn::Item::Impl(imp) => {
                let syn::Type::Path(tp) = &*imp.self_ty else {
                    continue;
                };
                let Some(ty) = tp.path.segments.last().map(|s| s.ident.to_string()) else {
                    continue;
                };
                for ii in &imp.items {
                    let syn::ImplItem::Fn(f) = ii else { continue };
                    if ty == "ParticleSpawner" && f.sig.ident == "resolve" {
                        resolve = Some(key_of(f.block.to_token_stream()));
                    }
                    if ty == "DemoApp" && f.sig.ident == "new" {
                        let stmts: Vec<String> = f
                            .block
                            .stmts
                            .iter()
                            .map(|s| key_of(s.to_token_stream()))
                            .collect();
                        let start = stmts
                            .iter()
                            .position(|s| s.starts_with("let mut world=EcsMaster::"));
                        let end = stmts
                            .iter()
                            .rposition(|s| s.contains("let runner=SimRunner::new("));
                        if let (Some(a), Some(b)) = (start, end) {
                            world_half = Some(stmts[a..=b].to_vec());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let world_half = world_half.expect("DRIFT GUARD: `DemoApp::new`'s `let mut world = EcsMaster::..` .. `SimRunner::new` span not found");
    let expected: Vec<String> = DEMO_WORLD_HALF.iter().map(ToString::to_string).collect();
    assert_eq!(
        world_half, expected,
        "DRIFT GUARD: `DemoApp::new`'s world half changed — update the replica in \
         `demo_world_g_res_and_id_census` and DEMO_WORLD_HALF together"
    );
    assert_eq!(
        resolve.as_deref(),
        Some(SPAWNER_RESOLVE),
        "DRIFT GUARD: `ParticleSpawner::resolve` changed — update the replica and SPAWNER_RESOLVE together"
    );
    assert_eq!(
        capacity.as_deref(),
        Some(CAPACITY_INIT),
        "DRIFT GUARD: `WORLD_ENTITY_CAPACITY` changed — update the replica and CAPACITY_INIT together"
    );
}

/// The (D) row: the replica, read at four points.
#[test]
fn demo_world_g_res_and_id_census() {
    // `DemoApp::new`'s world half — see `DEMO_WORLD_HALF` and the guard above.
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 2);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());
    // `ParticleSpawner::resolve`, inlined (the type is private to the demo).
    let _archetype = world.bundle_archetype_id_for::<ParticleBundle>();
    let _ids = (
        Position::component_id(),
        Velocity::component_id(),
        GpuInstance::component_id(),
        ParticleTag::component_id(),
    );
    id_line("demo", "config", 0);
    let pool = ThreadPoolBuilder::new().build();
    let mut runner = SimRunner::new(Arc::clone(&pool), &mut world);
    id_line("demo", "after_runner_new", 1);

    let steps = runner.step(&mut world, FIXED_DT);
    assert!(steps > 0, "the runner must run at least one fixed step");
    id_line("demo", "after_step_1", 2);
    g_res_line("demo", "after_boot", &world);

    for _ in 1..STEADY_STEPS {
        runner.step(&mut world, FIXED_DT);
    }
    let steady = id_line("demo", "steady_64", 3);
    let minted = id_cross_check(&steady);
    println!(
        "ID-CENSUS app=demo cross-check: the consuming probe minted slot {minted} = the scanned NEXT_ID"
    );
}
