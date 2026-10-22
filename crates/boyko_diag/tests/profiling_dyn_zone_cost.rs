//! **Profiling rung 10, `G17` — the dynamic path's cost, and the substitution this box forced.**
//!
//! # What `G17` asks for, and why it is not what runs here
//!
//! The corpus states `G17` as five absolute thresholds in one sitting: *"static-armed ≤ 12 ns,
//! dyn-armed ≤ 14 ns, static-disarmed ≤ 2 ns, dyn-disarmed ≤ 3 ns, script ≤ 18 ns"*, with the RED
//! being *"implement `zone_dyn!` with a `REGISTRY[id]` dereference to recover the scope bit ⇒ the
//! dyn-armed leg exceeds 14 ns"*.
//!
//! **An absolute nanosecond threshold on this box measures the box.** This campaign has already
//! paid to learn that: `docs/PROFILING-FLOOR.md`'s protocol found a **6.5 %** cross-process floor
//! on the artifact channel, with individual repetitions spanning 4.7 % to 14.3 % — on a *GPU* pass
//! costing microseconds. A 14 ns budget is two nanoseconds of headroom over 12; a scheduler slice,
//! a turbo transition or another test binary sharing a core moves it by more than that. A gate
//! that red-lights on a background process is not a gate on this code.
//!
//! # What runs instead: the same RED, measured as an A/B in one sitting
//!
//! The RED `G17` names is a **structural** change — recovering the arm bit from `REGISTRY[id]`
//! instead of carrying it on the handle. So this file implements *both* variants and compares them
//! **against each other, interleaved, in one process**:
//!
//! * `dyn_armed_shipped` — what `zone_dyn!` expands to: one `Acquire` load of the arm mask and a
//!   bit test.
//! * `dyn_armed_registry` — the RED: an `Acquire` load of `REGISTRY[id]`, a null check, a
//!   dereference to reach `scope`, then the same mask test.
//!
//! Both legs run the same loop, the same number of times, alternating, on the same thread. A
//! machine that is slow today is slow for both, so the RATIO is stable where the absolutes are not.
//! **The absolute nanosecond figures are PRINTED** — so the corpus's numbers can be compared by a
//! human — and asserted only as a sanity floor that no plausible box violates.
//!
//! # What this gate CANNOT claim
//!
//! Everything `G17`'s own limits column says, and one more. It cannot claim the dynamic path is
//! fast *in a game*: the handle sits in a register here, and a real one is a cold load out of an
//! ECS component. It measures the path's **floor**. And it cannot claim the shipped path meets any
//! particular nanosecond figure on any other machine — it claims the shipped path is not slower
//! than the variant the corpus asks it to beat.
//!
//! # Where the PROPERTY is gated now, and why the A/B left ordinary CI (ci-fix-2, F7)
//!
//! CI run 39663719484 read the A/B RED in its `force_alloc_panic` release leg and green in the
//! plain release leg of the same run: shipped 0.46 vs registry-deref 0.41 ns/iter, ratio 0.88.
//! At that size both legs run at about 1.3 cycles per iteration, so the dependent L1 hit the RED
//! adds is hidden by an out-of-order core, and the sign of a sub-nanosecond difference is decided
//! by the shared runner (the sibling timing test ran concurrently in the same binary). A timing
//! assertion that a background process can flip is not a gate on this code.
//!
//! The property the A/B stood for — **the handle carries `arm_bit`, and the emission path never
//! dereferences the registry** — is gated deterministically by two tests that can fail:
//!
//! * [`g17_the_emission_gate_reads_the_handle_not_the_registry`] (behaviour): a FORGED handle
//!   whose id has no descriptor but whose bit names an armed scope must open, and a registered
//!   zone whose bit names a never-armed scope must not. "Registry instead of the bit" reds the
//!   first half, "registry OR bit" the second, "registry AND bit" the first.
//! * [`g17_no_emission_item_names_the_registry`] (structure): the bodies of every emission item
//!   name none of `REGISTRY`, `zone_desc`, `DYN_DESCS`, which also catches a dereference that
//!   changes only the cost, which no behavioural test can see.
//!
//! The A/B itself is `workstation`-ignored and runs by name on the dev host, quiet, in release
//! (its ignore reason carries the command). Its result is a reading, not a gate.
//! [`a_disarmed_dynamic_zone_costs_less_than_an_armed_one`] stays in ordinary CI: its legs differ by
//! an order of magnitude (two clock reads and a push against one load), and the gate's
//! `zone_dyn_open == 0` on a disarmed handle now pins its property deterministically too.

use std::hint::black_box;

use boyko_diag::profiling_abi::dyn_registry::{
    DynZoneHandle, ZoneSpec, register_zone, zone_dyn_open,
};
use boyko_diag::profiling_abi::{
    USER_SCOPE_BASE, ZONE_ID_SPACE, ZoneTier, arm_scope, disarm_scope, zone_desc,
};

boyko_diag::profiling_partition!(User);

/// Iterations per leg. Large enough that one scheduler slice is a small fraction of the total,
/// small enough that the whole file is well under a second.
const ITERS: u32 = 200_000;
/// Alternating rounds. Interleaving is the point: a one-shot A-then-B would attribute a thermal
/// or frequency drift entirely to B.
const ROUNDS: u32 = 8;

/// The SHIPPED gate: the arm bit is on the handle, so this is one atomic load and a mask test.
#[inline(never)]
fn dyn_armed_shipped(h: DynZoneHandle) -> u64 {
    let mut hits = 0u64;
    for _ in 0..ITERS {
        if black_box(h).armed() {
            hits += 1;
        }
    }
    hits
}

/// **The RED variant**: recover the scope from the registry on every emission.
///
/// This is what `zone_dyn!` would have to do if `DynZoneHandle` carried only an id — a dependent
/// load into a table that is 56 KiB wide, followed by a pointer chase to reach one `u32`.
#[inline(never)]
fn dyn_armed_registry(h: DynZoneHandle) -> u64 {
    let mut hits = 0u64;
    for _ in 0..ITERS {
        let id = black_box(h).id;
        // The dereference the shipped path does not do.
        if let Some(desc) = zone_desc(id)
            && boyko_diag::profiling_abi::scope_armed(desc.scope)
        {
            hits += 1;
        }
    }
    hits
}

/// Which build profile produced the printed figures.
///
/// Load-bearing on the print, not decoration. `cargo test` builds `dev` unless told otherwise, and
/// a debug-profile nanosecond figure compared against the corpus's thresholds — which are release
/// figures — is a comparison between two different things. Naming the profile in the output is what
/// stops the next reader making it.
const PROFILE: &str = if cfg!(debug_assertions) { "debug" } else { "release" };

/// Nanoseconds per iteration for one leg, using the engine's own clock.
fn ns_per_iter(total_ticks: u64) -> f64 {
    let per_iter = total_ticks as f64 / f64::from(ITERS);
    per_iter / boyko_diag::clock::ticks_per_ns()
}

/// **`G17`, as an A/B.** The shipped dynamic gate is not slower than the registry-dereferencing
/// variant the corpus names as its RED.
///
/// Both legs are also asserted to have done the same WORK (the same hit count), which is what stops
/// a compiler that optimised one loop away from producing a flattering ratio.
///
/// A reading on the dev host, not a CI gate: see the module doc's F7 section. The property it stood
/// for is gated by [`g17_the_emission_gate_reads_the_handle_not_the_registry`] and
/// [`g17_no_emission_item_names_the_registry`].
#[test]
#[ignore = "workstation: G17's timing A/B (the shipped arm_bit gate against a registry dereference) is a sub-nanosecond comparison a shared runner decides: CI run 39663719484 read 0.46 vs 0.41 ns/iter (ratio 0.88) in its force_alloc_panic leg and green in the plain release leg of the same run. The property is gated by g17_the_emission_gate_reads_the_handle_not_the_registry and g17_no_emission_item_names_the_registry. Run it on the dev host, quiet, in release: cargo test --release -p boyko-diag --test profiling_dyn_zone_cost -- --ignored --exact the_dynamic_gate_beats_the_registry_dereference_it_replaced"]
fn the_dynamic_gate_beats_the_registry_dereference_it_replaced() {
    boyko_diag::clock::calibrate();

    let h = register_zone(ZoneSpec {
        name: "g17.dyn_cost",
        scope: USER_SCOPE_BASE + 7,
        tier: ZoneTier::Always,
    })
    .expect("a fresh registration succeeds");
    arm_scope(USER_SCOPE_BASE + 7);

    // Warm both paths before either is timed: the first pass faults the registry page in, and
    // charging that to whichever leg happens to run first is exactly the artefact interleaving
    // exists to remove.
    black_box(dyn_armed_shipped(h));
    black_box(dyn_armed_registry(h));

    let (mut shipped_ticks, mut registry_ticks) = (0u64, 0u64);
    let (mut shipped_hits, mut registry_hits) = (0u64, 0u64);
    for _ in 0..ROUNDS {
        let t0 = boyko_diag::clock::ticks();
        shipped_hits += dyn_armed_shipped(h);
        let t1 = boyko_diag::clock::ticks();
        registry_hits += dyn_armed_registry(h);
        let t2 = boyko_diag::clock::ticks();
        shipped_ticks += t1.wrapping_sub(t0);
        registry_ticks += t2.wrapping_sub(t1);
    }

    let shipped_ns = ns_per_iter(shipped_ticks / u64::from(ROUNDS));
    let registry_ns = ns_per_iter(registry_ticks / u64::from(ROUNDS));
    println!(
        "G17 [{PROFILE}] dyn zone gate, {ROUNDS} interleaved rounds x {ITERS} iters: \
         shipped {shipped_ns:.2} ns/iter, registry-deref {registry_ns:.2} ns/iter, \
         ratio {:.2}x",
        registry_ns / shipped_ns.max(f64::MIN_POSITIVE)
    );

    assert_eq!(
        shipped_hits, registry_hits,
        "the two legs must do the same work, or the comparison is between different loops"
    );
    assert!(
        shipped_hits > 0,
        "an armed zone must report armed; a zero hit count means both loops measured nothing"
    );
    assert!(
        shipped_ticks <= registry_ticks,
        "the shipped dynamic gate ({shipped_ns:.2} ns/iter) is SLOWER than the registry \
         dereference it exists to avoid ({registry_ns:.2} ns/iter). Carrying `arm_bit` on the \
         handle is the whole reason the handle is 16 bytes instead of 8; if this fires, either the \
         bit stopped being carried or the emission path grew a dereference."
    );

    disarm_scope(USER_SCOPE_BASE + 7);
}

/// The DISARMED leg: a zone whose scope is off costs one load and a test, and pushes nothing.
///
/// Asserted as a ratio against the armed leg rather than against a nanosecond figure, for the
/// module doc's reason. The claim is the one that matters to a shipped title: turning a scope off
/// makes its zones cheaper, and measurably so.
#[test]
fn a_disarmed_dynamic_zone_costs_less_than_an_armed_one() {
    boyko_diag::clock::calibrate();

    let h = register_zone(ZoneSpec {
        name: "g17.dyn_disarmed",
        scope: USER_SCOPE_BASE + 8,
        tier: ZoneTier::Always,
    })
    .expect("registered");

    let mut armed_ticks = 0u64;
    let mut disarmed_ticks = 0u64;
    // Warm.
    arm_scope(USER_SCOPE_BASE + 8);
    black_box(open_close_loop(h));
    disarm_scope(USER_SCOPE_BASE + 8);
    black_box(open_close_loop(h));

    for _ in 0..ROUNDS {
        arm_scope(USER_SCOPE_BASE + 8);
        let t0 = boyko_diag::clock::ticks();
        black_box(open_close_loop(h));
        let t1 = boyko_diag::clock::ticks();
        disarm_scope(USER_SCOPE_BASE + 8);
        let t2 = boyko_diag::clock::ticks();
        black_box(open_close_loop(h));
        let t3 = boyko_diag::clock::ticks();
        armed_ticks += t1.wrapping_sub(t0);
        disarmed_ticks += t3.wrapping_sub(t2);
    }

    println!(
        "G17 [{PROFILE}] dyn open/close, {ROUNDS} rounds x {ITERS} iters: armed {:.2} ns/iter, \
         disarmed {:.2} ns/iter",
        ns_per_iter(armed_ticks / u64::from(ROUNDS)),
        ns_per_iter(disarmed_ticks / u64::from(ROUNDS)),
    );

    assert!(
        disarmed_ticks < armed_ticks,
        "a disarmed scope must be cheaper than an armed one: disarmed {disarmed_ticks} ticks vs \
         armed {armed_ticks}. If these are equal the gate is not being taken, which would mean the \
         arm mask is not consulted before the clock is read."
    );
}

/// The open/close pair `zone_dyn!` expands to, as a loop.
#[inline(never)]
fn open_close_loop(h: DynZoneHandle) -> u64 {
    let mut opened = 0u64;
    for _ in 0..ITERS {
        let g = boyko_diag::zone_dyn!(black_box(h));
        if g.is_some() {
            opened += 1;
        }
        drop(g);
    }
    opened
}

/// **`G17`'s property, behaviourally:** the emission gate is the handle's `arm_bit` against the arm
/// mask, and the registry is not consulted at all.
///
/// Two handles make every RED shape visible:
///
/// * FORGED — an id with no descriptor (the top of the id space, which no registration in this
///   binary reaches) and the bit of an ARMED scope. The shipped gate opens it. A gate that recovers
///   the scope from the registry (alone, or AND-ed with the bit) finds no descriptor and does not.
/// * CONVERSE — a registered zone on the armed scope, its bit swapped for a never-armed scope. The
///   shipped gate refuses it. A gate that OR-s the registry's answer in opens it.
///
/// Scopes `USER_SCOPE_BASE + 9` and `+ 10` are this test's alone; the timing tests use `+ 7` and
/// `+ 8`, and the arm mask is process-wide.
#[test]
fn g17_the_emission_gate_reads_the_handle_not_the_registry() {
    const ARMED: u32 = USER_SCOPE_BASE + 9;
    const NEVER_ARMED: u32 = USER_SCOPE_BASE + 10;
    arm_scope(ARMED);

    let forged = DynZoneHandle {
        id: (ZONE_ID_SPACE - 1) as u16,
        arm_bit: 1u64 << ARMED,
    };
    assert!(
        zone_desc(forged.id).is_none(),
        "fixture: id {} has a descriptor, so the forged handle cannot tell the bit from the \
         registry",
        forged.id
    );
    assert!(
        forged.armed(),
        "a handle whose bit names an armed scope must read armed with no descriptor behind its id: \
         the gate consulted the registry instead of (or as well as) the handle's arm_bit"
    );
    assert_ne!(
        zone_dyn_open(forged),
        0,
        "zone_dyn_open refused a handle whose arm_bit is armed: the open path consults the registry"
    );
    let guard = boyko_diag::zone_dyn!(forged);
    assert!(
        guard.is_some(),
        "zone_dyn! refused a handle whose arm_bit is armed: the macro's gate consults the registry"
    );
    // Never closed: a forged id must not push a sample.
    core::mem::forget(guard);

    let registered = register_zone(ZoneSpec {
        name: "g17.gate_reads_the_handle",
        scope: ARMED,
        tier: ZoneTier::Always,
    })
    .expect("a fresh registration succeeds");
    let desc = zone_desc(registered.id).expect("fixture: a registered zone has a descriptor");
    assert_eq!(
        desc.scope, ARMED,
        "fixture: the descriptor names the armed scope"
    );
    assert!(
        registered.armed(),
        "fixture: the registered handle carries the armed scope's bit"
    );
    let converse = DynZoneHandle {
        arm_bit: 1u64 << NEVER_ARMED,
        ..registered
    };
    assert!(
        !converse.armed(),
        "a handle whose bit names a never-armed scope read armed because its descriptor's scope is \
         armed: the gate OR-s the registry into the handle's bit"
    );
    assert_eq!(
        zone_dyn_open(converse),
        0,
        "zone_dyn_open opened a handle whose arm_bit is not armed: the mask must gate the clock read"
    );
    assert!(
        boyko_diag::zone_dyn!(converse).is_none(),
        "zone_dyn! opened a handle whose arm_bit is not armed"
    );

    disarm_scope(ARMED);
}

/// The emission items whose bodies must not name the registry: `(file, the item's unique header)`.
/// `dyn_registry.rs` holds the dynamic path; `sample::push` and `arm_mask_bits` are what it calls.
const EMISSION_ITEMS: [(&str, &str); 9] = [
    ("dyn_registry.rs", "pub fn armed("),
    ("dyn_registry.rs", "pub fn zone_dyn_open("),
    ("dyn_registry.rs", "pub fn zone_dyn_close("),
    ("dyn_registry.rs", "pub fn open(handle: DynZoneHandle)"),
    ("dyn_registry.rs", "impl Drop for DynZoneGuard"),
    ("dyn_registry.rs", "macro_rules! zone_dyn {"),
    ("dyn_registry.rs", "pub fn emit_dyn("),
    ("sample.rs", "pub fn push("),
    ("profiling_abi.rs", "pub fn arm_mask_bits("),
];

/// What an emission body must not name: the registry, its accessor, the descriptor arena.
const REGISTRY_TOKENS: [&str; 3] = ["REGISTRY", "zone_desc", "DYN_DESCS"];

const SOURCES: [(&str, &str); 3] = [
    (
        "dyn_registry.rs",
        include_str!("../src/profiling_abi/dyn_registry.rs"),
    ),
    ("sample.rs", include_str!("../src/sample.rs")),
    ("profiling_abi.rs", include_str!("../src/profiling_abi.rs")),
];

/// The brace-balanced body of the one item in `src` whose header is `header`, with `//` comments
/// and string literals skipped. `Err` names why it could not be read.
fn item_body(src: &str, header: &str) -> Result<String, String> {
    let mut found = src.match_indices(header);
    let (at, _) = found
        .next()
        .ok_or_else(|| format!("item `{header}` not found"))?;
    if found.next().is_some() {
        return Err(format!("item header `{header}` is not unique"));
    }
    let bytes = src.as_bytes();
    let mut i = at;
    let mut depth = 0usize;
    let mut body = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| format!("unbalanced braces after `{header}`"))?;
            if depth == 0 {
                return Ok(body);
            }
        }
        if depth > 0 {
            body.push(char::from(c));
        }
        i += 1;
    }
    Err(format!("item `{header}` has no closing brace"))
}

fn source(file: &str) -> &'static str {
    SOURCES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, text)| *text)
        .expect("invariant: every listed file is in SOURCES")
}

/// **`G17`'s property, structurally:** no emission item names the registry. This is what catches a
/// dereference that changes the cost but not the behaviour (a `zone_desc` call whose result is
/// discarded, say, in `zone_dyn_close`), which
/// [`g17_the_emission_gate_reads_the_handle_not_the_registry`] cannot see.
///
/// Anti-vacuity: every listed item must be found exactly once, and the positive control —
/// `register_zone`, the one function that PUBLISHES into the registry — must name it, or the
/// scanner is reading nothing.
#[test]
fn g17_no_emission_item_names_the_registry() {
    let control = item_body(source("dyn_registry.rs"), "pub fn register_zone(")
        .expect("the positive control is readable");
    assert!(
        control.contains("REGISTRY"),
        "positive control: register_zone's body must name REGISTRY (it publishes there); the \
         scanner is not reading the source it claims to"
    );

    let mut errors = Vec::new();
    for (file, header) in EMISSION_ITEMS {
        match item_body(source(file), header) {
            Err(why) => errors.push(format!(
                "{file}: {why} -- the census lost an emission item; update EMISSION_ITEMS"
            )),
            Ok(body) => {
                for token in REGISTRY_TOKENS {
                    if body.contains(token) {
                        errors.push(format!(
                            "{file}: `{header}` names `{token}`: the emission path dereferences \
                             the registry, which is G17's RED (carry the bit on the handle instead)"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        errors.is_empty(),
        "G17 structural census:\n  {}",
        errors.join("\n  ")
    );
}
