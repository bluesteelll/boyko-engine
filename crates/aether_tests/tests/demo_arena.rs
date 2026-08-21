//! A complete test scene written in Aether — every v1 construct in one block.
//!
//! Read it as a specimen of the language. It is also a GATE: building the plugin type-checks every
//! emitted item against the real engine (section 8's R4 anti-drift rule), and the world is never
//! updated, so no device is touched.

use aether::aether;
use boyko_ecs::App;
use boyko_math::Vec3;
use boyko_render::{SdfEdit, sdf_op};
use boyko_scene::Transform;

aether! {
    aether v1;

    plugin Arena;

    // ── Data ────────────────────────────────────────────────────────────────────

    component Health {
        hp: f32,
        max: f32,
    }

    component Velocity {
        v: Vec3,
    }

    // A ZST tag: a real component with real archetype membership.
    tag Enemy;

    // A bitset tag: O(1) toggle, no archetype migration, no per-row bytes.
    tag Stunned(bitset);

    bundle Pawn {
        health: Health,
        vel: Velocity,
    }

    // The participant slot is typed by the components the victim must carry.
    event Damage {
        victim: entity(Health),
        amount: f32,
    }

    event WaveCleared {
        wave: u32,
    }

    // ── Materials ───────────────────────────────────────────────────────────────

    material arena_gold  { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material arena_chalk { base: (0.86, 0.86, 0.88), roughness: 0.85 }
    material arena_lamp  { base: (0.02, 0.02, 0.02), emissive: (6.0, 4.4, 2.0) }

    // ── The world ───────────────────────────────────────────────────────────────

    scene arena {
        // `material:` mints the asset row ONCE per scene and shares the handle, so the two
        // chalk props resolve to the same row. `casts_shadow` attaches ShadowCaster.
        entity at (2.0, 0.5, -1.0)  { material: arena_gold, casts_shadow };
        entity at (-2.0, 0.5, -1.0) { material: arena_chalk };
        entity at (4.0, 0.5, -1.0)  { material: arena_chalk };
        entity at (0.0, 2.4, -3.0)  { material: arena_lamp };

        // `children:` lowers to add_child; the reverse `Children` collection is maintained by
        // the kernel's own component hooks.
        entity at (0.0, 0.0, 0.0) {
            children: [
                entity at (-0.5, 1.0, 0.0) { },
                entity at ( 0.5, 1.0, 0.0) { }
            ]
        };

        sun   { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
        sky   { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
        point { pos: (-1.8, 2.2, 2.4), color: (0.5, 0.7, 1.0), power: 240.0, range: 9.0 }

        // Analytic geometry, straight into the SDF field.
        sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);
    }

    // ── Behaviour ───────────────────────────────────────────────────────────────

    system integrate(q: query<(&mut Transform, &Velocity)>, time: res<Clock>) on update {
        for (t, vel) in &mut q {
            t.translation = t.translation + vel.v * time.dt;
        }
    }

    // Filters compose: only ENEMIES that are not stunned are chased.
    system chase(
        q: query<&mut Velocity, with Enemy, without Stunned>,
        tally: mut res<Tally>,
    ) on update after integrate {
        for vel in &mut q {
            vel.v = vel.v * 0.98;
            tally.chased += 1;
        }
    }

    // `events<E>` reads the kernel's real event lanes; `emit<E>` writes them.
    system apply_damage(
        dmg: events<Damage>,
        hurt: query<&mut Health>,
        out: emit<WaveCleared>,
        tally: mut res<Tally>,
    ) on update {
        let mut fatal = 0u32;
        for d in dmg.read() {
            tally.damage_applied += 1;
            for h in &mut hurt {
                h.hp -= d.parameters.amount;
                if h.hp <= 0.0 {
                    fatal += 1;
                }
            }
        }
        if fatal > 0 {
            let _ = out.send(WaveCleared {
                participants: WaveClearedParticipants {},
                parameters: WaveClearedParameters { wave: fatal },
            });
        }
    }

    // `when` gates a system on an ordinary fn — fully visible to rust-analyzer.
    system audit(tally: mut res<Tally>) on update when auditing {
        tally.audits += 1;
    }

    // ── Flow ────────────────────────────────────────────────────────────────────

    machine GameFlow {
        initial Boot;

        state Boot {
            on WaveCleared => Playing;
        }

        state Playing {
            initial Fighting;

            enter (tally: mut res<Tally>) {
                tally.entered_playing += 1;
            }

            state Fighting {
                on WaveCleared => Playing.Resting;
            }

            state Resting {
                on WaveCleared => Playing.Fighting;
            }
        }
    }
}

/// The `when` gate's condition — an ordinary fn, not a captured expression.
fn auditing() -> bool {
    true
}

#[derive(boyko_macros::Resource)]
struct Clock {
    dt: f32,
}

#[derive(boyko_macros::Resource, Default)]
struct Tally {
    chased: u32,
    damage_applied: u32,
    audits: u32,
    entered_playing: u32,
}

#[test]
fn the_arena_block_builds_against_the_real_engine() {
    let mut app = App::new();
    app.insert_resource(Clock { dt: 1.0 / 60.0 });
    app.insert_resource(Tally::default());
    app.add_plugins(Arena);
    let _ = &app;
}
