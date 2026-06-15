//! EnableTag positive-term archetype-cull DIAGNOSTIC (task #5) -- profile-first
//! probe against the CURRENT (NO-OP cull) build. This bench does NOT implement
//! the cull; it measures whether the cull would buy a measurable win.
//!
//! # The question
//!
//! A query `Query<&EtPos, Enabled<EtFlag>>` currently visits EVERY archetype
//! that contains `EtPos`, including archetypes that have NO enable-column for
//! `EtFlag` (all rows disabled). For such a no-column archetype,
//! `Enabled::filter_fetch` short-circuits on a NULL column pointer
//! (`fetch.col.is_null() => return false`) per row. The proposed cull would skip
//! these archetypes entirely. OPEN QUESTION: is visiting these no-column
//! archetypes actually MEASURABLE, or does LLVM collapse the perfectly-predicted
//! `is_null() => continue` inner loop so the win is below this box's noise?
//!
//! # Bench ids / role
//!
//! | Bench id               | World (M, R, K) | Role                              |
//! |------------------------|-----------------|-----------------------------------|
//! | `cull_full`            | (64, 256, 4)    | A/B: 60 no-column archetypes      |
//! | `cull_equiv`           | ( 4, 256, 4)    | A/B: only the 4 with-column ones  |
//! | `cull_R16`             | (64,  16, 4)    | R-sweep: rows per no-col archetype|
//! | `cull_R256`            | (64, 256, 4)    | R-sweep (== cull_full world)      |
//! | `cull_R1024`           | (64,1024, 4)    | R-sweep                           |
//! | `cull_M8`              | ( 8,  64, 4)    | M-sweep: archetype count          |
//! | `cull_M64`             | (64,  64, 4)    | M-sweep                           |
//! | `cull_M256`            | (256, 64, 4)    | M-sweep                           |
//! | `cull_all_have_column` | (64, 256, 64)   | control: every archetype has a col|
//!
//! `cull_full` and `cull_R256` share the (64,256,4) world (built once, timed
//! under both ids so the orchestrator sees the A/B and the R-sweep midpoint from
//! one fixture). Each world is built ONCE in setup; only the query+iter is timed.
//!
//! # Distinct archetypes
//!
//! 256 marker ZSTs (`Marker0..Marker255`) + 256 bundles (`BundleN { pos: EtPos,
//! m: MarkerN }`). Spawning into the first M bundles yields exactly M distinct
//! `{EtPos, MarkerI}` archetypes (the markers give distinct signatures). All M
//! contain `EtPos`, so `Query<&EtPos, Enabled<EtFlag>>` matches all M; only the
//! K*R rows in the first K archetypes are enabled (those K get an EtFlag column),
//! the other M-K archetypes are visited with a NULL EtFlag column.

#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::iters::query::Enabled;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::{Bundle, Component};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::hint::black_box as hint_black_box;

// -- Real data component every archetype contains and every query reads -------

/// The data component every bundle / archetype contains and every query reads.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct EtPos {
    x: u64,
    y: u64,
}

/// The fieldless (ZST) bitset enable tag the diagnostic toggles. Filtered out of
/// every archetype signature; enabling it on a row allocates an `EnableColumn`
/// for that row's archetype. Archetypes that never see an `enable::<EtFlag>` have
/// a NULL EtFlag column -- the no-column case the cull targets.
#[derive(Component)]
#[component(storage = "bitset")]
struct EtFlag;

/// `spawn_batch` is chunked at 5_000 (<= MAX_BATCH_HINT). `u32`, not `u64`:
/// `spawn_batch` requires an `ExactSizeIterator` and `Range<u64>` is not one.
const CHUNK: u32 = 5_000;

#[inline]
fn pos(i: u64) -> EtPos {
    EtPos {
        x: i,
        y: i.wrapping_mul(3),
    }
}

// -- 256 distinct marker ZSTs + 256 single-marker bundles -------------------
//
// Each `MarkerN` is a distinct type, so `{EtPos, MarkerI}` is a distinct
// archetype signature. Explicit (no `paste`/`seq-macro`) to stay dev-dep-free.

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker0;

#[derive(Bundle)]
struct Bundle0 {
    pos: EtPos,
    m: Marker0,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker1;

#[derive(Bundle)]
struct Bundle1 {
    pos: EtPos,
    m: Marker1,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker2;

#[derive(Bundle)]
struct Bundle2 {
    pos: EtPos,
    m: Marker2,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker3;

#[derive(Bundle)]
struct Bundle3 {
    pos: EtPos,
    m: Marker3,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker4;

#[derive(Bundle)]
struct Bundle4 {
    pos: EtPos,
    m: Marker4,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker5;

#[derive(Bundle)]
struct Bundle5 {
    pos: EtPos,
    m: Marker5,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker6;

#[derive(Bundle)]
struct Bundle6 {
    pos: EtPos,
    m: Marker6,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker7;

#[derive(Bundle)]
struct Bundle7 {
    pos: EtPos,
    m: Marker7,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker8;

#[derive(Bundle)]
struct Bundle8 {
    pos: EtPos,
    m: Marker8,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker9;

#[derive(Bundle)]
struct Bundle9 {
    pos: EtPos,
    m: Marker9,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker10;

#[derive(Bundle)]
struct Bundle10 {
    pos: EtPos,
    m: Marker10,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker11;

#[derive(Bundle)]
struct Bundle11 {
    pos: EtPos,
    m: Marker11,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker12;

#[derive(Bundle)]
struct Bundle12 {
    pos: EtPos,
    m: Marker12,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker13;

#[derive(Bundle)]
struct Bundle13 {
    pos: EtPos,
    m: Marker13,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker14;

#[derive(Bundle)]
struct Bundle14 {
    pos: EtPos,
    m: Marker14,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker15;

#[derive(Bundle)]
struct Bundle15 {
    pos: EtPos,
    m: Marker15,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker16;

#[derive(Bundle)]
struct Bundle16 {
    pos: EtPos,
    m: Marker16,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker17;

#[derive(Bundle)]
struct Bundle17 {
    pos: EtPos,
    m: Marker17,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker18;

#[derive(Bundle)]
struct Bundle18 {
    pos: EtPos,
    m: Marker18,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker19;

#[derive(Bundle)]
struct Bundle19 {
    pos: EtPos,
    m: Marker19,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker20;

#[derive(Bundle)]
struct Bundle20 {
    pos: EtPos,
    m: Marker20,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker21;

#[derive(Bundle)]
struct Bundle21 {
    pos: EtPos,
    m: Marker21,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker22;

#[derive(Bundle)]
struct Bundle22 {
    pos: EtPos,
    m: Marker22,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker23;

#[derive(Bundle)]
struct Bundle23 {
    pos: EtPos,
    m: Marker23,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker24;

#[derive(Bundle)]
struct Bundle24 {
    pos: EtPos,
    m: Marker24,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker25;

#[derive(Bundle)]
struct Bundle25 {
    pos: EtPos,
    m: Marker25,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker26;

#[derive(Bundle)]
struct Bundle26 {
    pos: EtPos,
    m: Marker26,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker27;

#[derive(Bundle)]
struct Bundle27 {
    pos: EtPos,
    m: Marker27,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker28;

#[derive(Bundle)]
struct Bundle28 {
    pos: EtPos,
    m: Marker28,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker29;

#[derive(Bundle)]
struct Bundle29 {
    pos: EtPos,
    m: Marker29,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker30;

#[derive(Bundle)]
struct Bundle30 {
    pos: EtPos,
    m: Marker30,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker31;

#[derive(Bundle)]
struct Bundle31 {
    pos: EtPos,
    m: Marker31,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker32;

#[derive(Bundle)]
struct Bundle32 {
    pos: EtPos,
    m: Marker32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker33;

#[derive(Bundle)]
struct Bundle33 {
    pos: EtPos,
    m: Marker33,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker34;

#[derive(Bundle)]
struct Bundle34 {
    pos: EtPos,
    m: Marker34,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker35;

#[derive(Bundle)]
struct Bundle35 {
    pos: EtPos,
    m: Marker35,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker36;

#[derive(Bundle)]
struct Bundle36 {
    pos: EtPos,
    m: Marker36,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker37;

#[derive(Bundle)]
struct Bundle37 {
    pos: EtPos,
    m: Marker37,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker38;

#[derive(Bundle)]
struct Bundle38 {
    pos: EtPos,
    m: Marker38,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker39;

#[derive(Bundle)]
struct Bundle39 {
    pos: EtPos,
    m: Marker39,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker40;

#[derive(Bundle)]
struct Bundle40 {
    pos: EtPos,
    m: Marker40,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker41;

#[derive(Bundle)]
struct Bundle41 {
    pos: EtPos,
    m: Marker41,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker42;

#[derive(Bundle)]
struct Bundle42 {
    pos: EtPos,
    m: Marker42,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker43;

#[derive(Bundle)]
struct Bundle43 {
    pos: EtPos,
    m: Marker43,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker44;

#[derive(Bundle)]
struct Bundle44 {
    pos: EtPos,
    m: Marker44,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker45;

#[derive(Bundle)]
struct Bundle45 {
    pos: EtPos,
    m: Marker45,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker46;

#[derive(Bundle)]
struct Bundle46 {
    pos: EtPos,
    m: Marker46,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker47;

#[derive(Bundle)]
struct Bundle47 {
    pos: EtPos,
    m: Marker47,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker48;

#[derive(Bundle)]
struct Bundle48 {
    pos: EtPos,
    m: Marker48,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker49;

#[derive(Bundle)]
struct Bundle49 {
    pos: EtPos,
    m: Marker49,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker50;

#[derive(Bundle)]
struct Bundle50 {
    pos: EtPos,
    m: Marker50,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker51;

#[derive(Bundle)]
struct Bundle51 {
    pos: EtPos,
    m: Marker51,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker52;

#[derive(Bundle)]
struct Bundle52 {
    pos: EtPos,
    m: Marker52,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker53;

#[derive(Bundle)]
struct Bundle53 {
    pos: EtPos,
    m: Marker53,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker54;

#[derive(Bundle)]
struct Bundle54 {
    pos: EtPos,
    m: Marker54,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker55;

#[derive(Bundle)]
struct Bundle55 {
    pos: EtPos,
    m: Marker55,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker56;

#[derive(Bundle)]
struct Bundle56 {
    pos: EtPos,
    m: Marker56,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker57;

#[derive(Bundle)]
struct Bundle57 {
    pos: EtPos,
    m: Marker57,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker58;

#[derive(Bundle)]
struct Bundle58 {
    pos: EtPos,
    m: Marker58,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker59;

#[derive(Bundle)]
struct Bundle59 {
    pos: EtPos,
    m: Marker59,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker60;

#[derive(Bundle)]
struct Bundle60 {
    pos: EtPos,
    m: Marker60,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker61;

#[derive(Bundle)]
struct Bundle61 {
    pos: EtPos,
    m: Marker61,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker62;

#[derive(Bundle)]
struct Bundle62 {
    pos: EtPos,
    m: Marker62,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker63;

#[derive(Bundle)]
struct Bundle63 {
    pos: EtPos,
    m: Marker63,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker64;

#[derive(Bundle)]
struct Bundle64 {
    pos: EtPos,
    m: Marker64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker65;

#[derive(Bundle)]
struct Bundle65 {
    pos: EtPos,
    m: Marker65,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker66;

#[derive(Bundle)]
struct Bundle66 {
    pos: EtPos,
    m: Marker66,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker67;

#[derive(Bundle)]
struct Bundle67 {
    pos: EtPos,
    m: Marker67,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker68;

#[derive(Bundle)]
struct Bundle68 {
    pos: EtPos,
    m: Marker68,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker69;

#[derive(Bundle)]
struct Bundle69 {
    pos: EtPos,
    m: Marker69,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker70;

#[derive(Bundle)]
struct Bundle70 {
    pos: EtPos,
    m: Marker70,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker71;

#[derive(Bundle)]
struct Bundle71 {
    pos: EtPos,
    m: Marker71,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker72;

#[derive(Bundle)]
struct Bundle72 {
    pos: EtPos,
    m: Marker72,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker73;

#[derive(Bundle)]
struct Bundle73 {
    pos: EtPos,
    m: Marker73,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker74;

#[derive(Bundle)]
struct Bundle74 {
    pos: EtPos,
    m: Marker74,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker75;

#[derive(Bundle)]
struct Bundle75 {
    pos: EtPos,
    m: Marker75,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker76;

#[derive(Bundle)]
struct Bundle76 {
    pos: EtPos,
    m: Marker76,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker77;

#[derive(Bundle)]
struct Bundle77 {
    pos: EtPos,
    m: Marker77,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker78;

#[derive(Bundle)]
struct Bundle78 {
    pos: EtPos,
    m: Marker78,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker79;

#[derive(Bundle)]
struct Bundle79 {
    pos: EtPos,
    m: Marker79,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker80;

#[derive(Bundle)]
struct Bundle80 {
    pos: EtPos,
    m: Marker80,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker81;

#[derive(Bundle)]
struct Bundle81 {
    pos: EtPos,
    m: Marker81,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker82;

#[derive(Bundle)]
struct Bundle82 {
    pos: EtPos,
    m: Marker82,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker83;

#[derive(Bundle)]
struct Bundle83 {
    pos: EtPos,
    m: Marker83,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker84;

#[derive(Bundle)]
struct Bundle84 {
    pos: EtPos,
    m: Marker84,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker85;

#[derive(Bundle)]
struct Bundle85 {
    pos: EtPos,
    m: Marker85,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker86;

#[derive(Bundle)]
struct Bundle86 {
    pos: EtPos,
    m: Marker86,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker87;

#[derive(Bundle)]
struct Bundle87 {
    pos: EtPos,
    m: Marker87,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker88;

#[derive(Bundle)]
struct Bundle88 {
    pos: EtPos,
    m: Marker88,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker89;

#[derive(Bundle)]
struct Bundle89 {
    pos: EtPos,
    m: Marker89,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker90;

#[derive(Bundle)]
struct Bundle90 {
    pos: EtPos,
    m: Marker90,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker91;

#[derive(Bundle)]
struct Bundle91 {
    pos: EtPos,
    m: Marker91,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker92;

#[derive(Bundle)]
struct Bundle92 {
    pos: EtPos,
    m: Marker92,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker93;

#[derive(Bundle)]
struct Bundle93 {
    pos: EtPos,
    m: Marker93,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker94;

#[derive(Bundle)]
struct Bundle94 {
    pos: EtPos,
    m: Marker94,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker95;

#[derive(Bundle)]
struct Bundle95 {
    pos: EtPos,
    m: Marker95,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker96;

#[derive(Bundle)]
struct Bundle96 {
    pos: EtPos,
    m: Marker96,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker97;

#[derive(Bundle)]
struct Bundle97 {
    pos: EtPos,
    m: Marker97,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker98;

#[derive(Bundle)]
struct Bundle98 {
    pos: EtPos,
    m: Marker98,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker99;

#[derive(Bundle)]
struct Bundle99 {
    pos: EtPos,
    m: Marker99,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker100;

#[derive(Bundle)]
struct Bundle100 {
    pos: EtPos,
    m: Marker100,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker101;

#[derive(Bundle)]
struct Bundle101 {
    pos: EtPos,
    m: Marker101,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker102;

#[derive(Bundle)]
struct Bundle102 {
    pos: EtPos,
    m: Marker102,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker103;

#[derive(Bundle)]
struct Bundle103 {
    pos: EtPos,
    m: Marker103,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker104;

#[derive(Bundle)]
struct Bundle104 {
    pos: EtPos,
    m: Marker104,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker105;

#[derive(Bundle)]
struct Bundle105 {
    pos: EtPos,
    m: Marker105,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker106;

#[derive(Bundle)]
struct Bundle106 {
    pos: EtPos,
    m: Marker106,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker107;

#[derive(Bundle)]
struct Bundle107 {
    pos: EtPos,
    m: Marker107,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker108;

#[derive(Bundle)]
struct Bundle108 {
    pos: EtPos,
    m: Marker108,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker109;

#[derive(Bundle)]
struct Bundle109 {
    pos: EtPos,
    m: Marker109,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker110;

#[derive(Bundle)]
struct Bundle110 {
    pos: EtPos,
    m: Marker110,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker111;

#[derive(Bundle)]
struct Bundle111 {
    pos: EtPos,
    m: Marker111,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker112;

#[derive(Bundle)]
struct Bundle112 {
    pos: EtPos,
    m: Marker112,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker113;

#[derive(Bundle)]
struct Bundle113 {
    pos: EtPos,
    m: Marker113,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker114;

#[derive(Bundle)]
struct Bundle114 {
    pos: EtPos,
    m: Marker114,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker115;

#[derive(Bundle)]
struct Bundle115 {
    pos: EtPos,
    m: Marker115,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker116;

#[derive(Bundle)]
struct Bundle116 {
    pos: EtPos,
    m: Marker116,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker117;

#[derive(Bundle)]
struct Bundle117 {
    pos: EtPos,
    m: Marker117,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker118;

#[derive(Bundle)]
struct Bundle118 {
    pos: EtPos,
    m: Marker118,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker119;

#[derive(Bundle)]
struct Bundle119 {
    pos: EtPos,
    m: Marker119,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker120;

#[derive(Bundle)]
struct Bundle120 {
    pos: EtPos,
    m: Marker120,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker121;

#[derive(Bundle)]
struct Bundle121 {
    pos: EtPos,
    m: Marker121,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker122;

#[derive(Bundle)]
struct Bundle122 {
    pos: EtPos,
    m: Marker122,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker123;

#[derive(Bundle)]
struct Bundle123 {
    pos: EtPos,
    m: Marker123,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker124;

#[derive(Bundle)]
struct Bundle124 {
    pos: EtPos,
    m: Marker124,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker125;

#[derive(Bundle)]
struct Bundle125 {
    pos: EtPos,
    m: Marker125,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker126;

#[derive(Bundle)]
struct Bundle126 {
    pos: EtPos,
    m: Marker126,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker127;

#[derive(Bundle)]
struct Bundle127 {
    pos: EtPos,
    m: Marker127,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker128;

#[derive(Bundle)]
struct Bundle128 {
    pos: EtPos,
    m: Marker128,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker129;

#[derive(Bundle)]
struct Bundle129 {
    pos: EtPos,
    m: Marker129,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker130;

#[derive(Bundle)]
struct Bundle130 {
    pos: EtPos,
    m: Marker130,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker131;

#[derive(Bundle)]
struct Bundle131 {
    pos: EtPos,
    m: Marker131,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker132;

#[derive(Bundle)]
struct Bundle132 {
    pos: EtPos,
    m: Marker132,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker133;

#[derive(Bundle)]
struct Bundle133 {
    pos: EtPos,
    m: Marker133,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker134;

#[derive(Bundle)]
struct Bundle134 {
    pos: EtPos,
    m: Marker134,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker135;

#[derive(Bundle)]
struct Bundle135 {
    pos: EtPos,
    m: Marker135,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker136;

#[derive(Bundle)]
struct Bundle136 {
    pos: EtPos,
    m: Marker136,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker137;

#[derive(Bundle)]
struct Bundle137 {
    pos: EtPos,
    m: Marker137,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker138;

#[derive(Bundle)]
struct Bundle138 {
    pos: EtPos,
    m: Marker138,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker139;

#[derive(Bundle)]
struct Bundle139 {
    pos: EtPos,
    m: Marker139,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker140;

#[derive(Bundle)]
struct Bundle140 {
    pos: EtPos,
    m: Marker140,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker141;

#[derive(Bundle)]
struct Bundle141 {
    pos: EtPos,
    m: Marker141,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker142;

#[derive(Bundle)]
struct Bundle142 {
    pos: EtPos,
    m: Marker142,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker143;

#[derive(Bundle)]
struct Bundle143 {
    pos: EtPos,
    m: Marker143,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker144;

#[derive(Bundle)]
struct Bundle144 {
    pos: EtPos,
    m: Marker144,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker145;

#[derive(Bundle)]
struct Bundle145 {
    pos: EtPos,
    m: Marker145,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker146;

#[derive(Bundle)]
struct Bundle146 {
    pos: EtPos,
    m: Marker146,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker147;

#[derive(Bundle)]
struct Bundle147 {
    pos: EtPos,
    m: Marker147,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker148;

#[derive(Bundle)]
struct Bundle148 {
    pos: EtPos,
    m: Marker148,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker149;

#[derive(Bundle)]
struct Bundle149 {
    pos: EtPos,
    m: Marker149,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker150;

#[derive(Bundle)]
struct Bundle150 {
    pos: EtPos,
    m: Marker150,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker151;

#[derive(Bundle)]
struct Bundle151 {
    pos: EtPos,
    m: Marker151,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker152;

#[derive(Bundle)]
struct Bundle152 {
    pos: EtPos,
    m: Marker152,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker153;

#[derive(Bundle)]
struct Bundle153 {
    pos: EtPos,
    m: Marker153,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker154;

#[derive(Bundle)]
struct Bundle154 {
    pos: EtPos,
    m: Marker154,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker155;

#[derive(Bundle)]
struct Bundle155 {
    pos: EtPos,
    m: Marker155,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker156;

#[derive(Bundle)]
struct Bundle156 {
    pos: EtPos,
    m: Marker156,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker157;

#[derive(Bundle)]
struct Bundle157 {
    pos: EtPos,
    m: Marker157,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker158;

#[derive(Bundle)]
struct Bundle158 {
    pos: EtPos,
    m: Marker158,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker159;

#[derive(Bundle)]
struct Bundle159 {
    pos: EtPos,
    m: Marker159,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker160;

#[derive(Bundle)]
struct Bundle160 {
    pos: EtPos,
    m: Marker160,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker161;

#[derive(Bundle)]
struct Bundle161 {
    pos: EtPos,
    m: Marker161,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker162;

#[derive(Bundle)]
struct Bundle162 {
    pos: EtPos,
    m: Marker162,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker163;

#[derive(Bundle)]
struct Bundle163 {
    pos: EtPos,
    m: Marker163,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker164;

#[derive(Bundle)]
struct Bundle164 {
    pos: EtPos,
    m: Marker164,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker165;

#[derive(Bundle)]
struct Bundle165 {
    pos: EtPos,
    m: Marker165,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker166;

#[derive(Bundle)]
struct Bundle166 {
    pos: EtPos,
    m: Marker166,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker167;

#[derive(Bundle)]
struct Bundle167 {
    pos: EtPos,
    m: Marker167,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker168;

#[derive(Bundle)]
struct Bundle168 {
    pos: EtPos,
    m: Marker168,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker169;

#[derive(Bundle)]
struct Bundle169 {
    pos: EtPos,
    m: Marker169,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker170;

#[derive(Bundle)]
struct Bundle170 {
    pos: EtPos,
    m: Marker170,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker171;

#[derive(Bundle)]
struct Bundle171 {
    pos: EtPos,
    m: Marker171,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker172;

#[derive(Bundle)]
struct Bundle172 {
    pos: EtPos,
    m: Marker172,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker173;

#[derive(Bundle)]
struct Bundle173 {
    pos: EtPos,
    m: Marker173,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker174;

#[derive(Bundle)]
struct Bundle174 {
    pos: EtPos,
    m: Marker174,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker175;

#[derive(Bundle)]
struct Bundle175 {
    pos: EtPos,
    m: Marker175,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker176;

#[derive(Bundle)]
struct Bundle176 {
    pos: EtPos,
    m: Marker176,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker177;

#[derive(Bundle)]
struct Bundle177 {
    pos: EtPos,
    m: Marker177,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker178;

#[derive(Bundle)]
struct Bundle178 {
    pos: EtPos,
    m: Marker178,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker179;

#[derive(Bundle)]
struct Bundle179 {
    pos: EtPos,
    m: Marker179,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker180;

#[derive(Bundle)]
struct Bundle180 {
    pos: EtPos,
    m: Marker180,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker181;

#[derive(Bundle)]
struct Bundle181 {
    pos: EtPos,
    m: Marker181,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker182;

#[derive(Bundle)]
struct Bundle182 {
    pos: EtPos,
    m: Marker182,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker183;

#[derive(Bundle)]
struct Bundle183 {
    pos: EtPos,
    m: Marker183,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker184;

#[derive(Bundle)]
struct Bundle184 {
    pos: EtPos,
    m: Marker184,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker185;

#[derive(Bundle)]
struct Bundle185 {
    pos: EtPos,
    m: Marker185,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker186;

#[derive(Bundle)]
struct Bundle186 {
    pos: EtPos,
    m: Marker186,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker187;

#[derive(Bundle)]
struct Bundle187 {
    pos: EtPos,
    m: Marker187,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker188;

#[derive(Bundle)]
struct Bundle188 {
    pos: EtPos,
    m: Marker188,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker189;

#[derive(Bundle)]
struct Bundle189 {
    pos: EtPos,
    m: Marker189,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker190;

#[derive(Bundle)]
struct Bundle190 {
    pos: EtPos,
    m: Marker190,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker191;

#[derive(Bundle)]
struct Bundle191 {
    pos: EtPos,
    m: Marker191,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker192;

#[derive(Bundle)]
struct Bundle192 {
    pos: EtPos,
    m: Marker192,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker193;

#[derive(Bundle)]
struct Bundle193 {
    pos: EtPos,
    m: Marker193,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker194;

#[derive(Bundle)]
struct Bundle194 {
    pos: EtPos,
    m: Marker194,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker195;

#[derive(Bundle)]
struct Bundle195 {
    pos: EtPos,
    m: Marker195,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker196;

#[derive(Bundle)]
struct Bundle196 {
    pos: EtPos,
    m: Marker196,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker197;

#[derive(Bundle)]
struct Bundle197 {
    pos: EtPos,
    m: Marker197,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker198;

#[derive(Bundle)]
struct Bundle198 {
    pos: EtPos,
    m: Marker198,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker199;

#[derive(Bundle)]
struct Bundle199 {
    pos: EtPos,
    m: Marker199,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker200;

#[derive(Bundle)]
struct Bundle200 {
    pos: EtPos,
    m: Marker200,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker201;

#[derive(Bundle)]
struct Bundle201 {
    pos: EtPos,
    m: Marker201,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker202;

#[derive(Bundle)]
struct Bundle202 {
    pos: EtPos,
    m: Marker202,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker203;

#[derive(Bundle)]
struct Bundle203 {
    pos: EtPos,
    m: Marker203,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker204;

#[derive(Bundle)]
struct Bundle204 {
    pos: EtPos,
    m: Marker204,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker205;

#[derive(Bundle)]
struct Bundle205 {
    pos: EtPos,
    m: Marker205,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker206;

#[derive(Bundle)]
struct Bundle206 {
    pos: EtPos,
    m: Marker206,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker207;

#[derive(Bundle)]
struct Bundle207 {
    pos: EtPos,
    m: Marker207,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker208;

#[derive(Bundle)]
struct Bundle208 {
    pos: EtPos,
    m: Marker208,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker209;

#[derive(Bundle)]
struct Bundle209 {
    pos: EtPos,
    m: Marker209,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker210;

#[derive(Bundle)]
struct Bundle210 {
    pos: EtPos,
    m: Marker210,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker211;

#[derive(Bundle)]
struct Bundle211 {
    pos: EtPos,
    m: Marker211,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker212;

#[derive(Bundle)]
struct Bundle212 {
    pos: EtPos,
    m: Marker212,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker213;

#[derive(Bundle)]
struct Bundle213 {
    pos: EtPos,
    m: Marker213,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker214;

#[derive(Bundle)]
struct Bundle214 {
    pos: EtPos,
    m: Marker214,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker215;

#[derive(Bundle)]
struct Bundle215 {
    pos: EtPos,
    m: Marker215,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker216;

#[derive(Bundle)]
struct Bundle216 {
    pos: EtPos,
    m: Marker216,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker217;

#[derive(Bundle)]
struct Bundle217 {
    pos: EtPos,
    m: Marker217,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker218;

#[derive(Bundle)]
struct Bundle218 {
    pos: EtPos,
    m: Marker218,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker219;

#[derive(Bundle)]
struct Bundle219 {
    pos: EtPos,
    m: Marker219,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker220;

#[derive(Bundle)]
struct Bundle220 {
    pos: EtPos,
    m: Marker220,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker221;

#[derive(Bundle)]
struct Bundle221 {
    pos: EtPos,
    m: Marker221,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker222;

#[derive(Bundle)]
struct Bundle222 {
    pos: EtPos,
    m: Marker222,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker223;

#[derive(Bundle)]
struct Bundle223 {
    pos: EtPos,
    m: Marker223,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker224;

#[derive(Bundle)]
struct Bundle224 {
    pos: EtPos,
    m: Marker224,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker225;

#[derive(Bundle)]
struct Bundle225 {
    pos: EtPos,
    m: Marker225,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker226;

#[derive(Bundle)]
struct Bundle226 {
    pos: EtPos,
    m: Marker226,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker227;

#[derive(Bundle)]
struct Bundle227 {
    pos: EtPos,
    m: Marker227,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker228;

#[derive(Bundle)]
struct Bundle228 {
    pos: EtPos,
    m: Marker228,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker229;

#[derive(Bundle)]
struct Bundle229 {
    pos: EtPos,
    m: Marker229,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker230;

#[derive(Bundle)]
struct Bundle230 {
    pos: EtPos,
    m: Marker230,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker231;

#[derive(Bundle)]
struct Bundle231 {
    pos: EtPos,
    m: Marker231,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker232;

#[derive(Bundle)]
struct Bundle232 {
    pos: EtPos,
    m: Marker232,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker233;

#[derive(Bundle)]
struct Bundle233 {
    pos: EtPos,
    m: Marker233,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker234;

#[derive(Bundle)]
struct Bundle234 {
    pos: EtPos,
    m: Marker234,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker235;

#[derive(Bundle)]
struct Bundle235 {
    pos: EtPos,
    m: Marker235,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker236;

#[derive(Bundle)]
struct Bundle236 {
    pos: EtPos,
    m: Marker236,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker237;

#[derive(Bundle)]
struct Bundle237 {
    pos: EtPos,
    m: Marker237,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker238;

#[derive(Bundle)]
struct Bundle238 {
    pos: EtPos,
    m: Marker238,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker239;

#[derive(Bundle)]
struct Bundle239 {
    pos: EtPos,
    m: Marker239,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker240;

#[derive(Bundle)]
struct Bundle240 {
    pos: EtPos,
    m: Marker240,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker241;

#[derive(Bundle)]
struct Bundle241 {
    pos: EtPos,
    m: Marker241,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker242;

#[derive(Bundle)]
struct Bundle242 {
    pos: EtPos,
    m: Marker242,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker243;

#[derive(Bundle)]
struct Bundle243 {
    pos: EtPos,
    m: Marker243,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker244;

#[derive(Bundle)]
struct Bundle244 {
    pos: EtPos,
    m: Marker244,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker245;

#[derive(Bundle)]
struct Bundle245 {
    pos: EtPos,
    m: Marker245,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker246;

#[derive(Bundle)]
struct Bundle246 {
    pos: EtPos,
    m: Marker246,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker247;

#[derive(Bundle)]
struct Bundle247 {
    pos: EtPos,
    m: Marker247,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker248;

#[derive(Bundle)]
struct Bundle248 {
    pos: EtPos,
    m: Marker248,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker249;

#[derive(Bundle)]
struct Bundle249 {
    pos: EtPos,
    m: Marker249,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker250;

#[derive(Bundle)]
struct Bundle250 {
    pos: EtPos,
    m: Marker250,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker251;

#[derive(Bundle)]
struct Bundle251 {
    pos: EtPos,
    m: Marker251,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker252;

#[derive(Bundle)]
struct Bundle252 {
    pos: EtPos,
    m: Marker252,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker253;

#[derive(Bundle)]
struct Bundle253 {
    pos: EtPos,
    m: Marker253,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker254;

#[derive(Bundle)]
struct Bundle254 {
    pos: EtPos,
    m: Marker254,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Marker255;

#[derive(Bundle)]
struct Bundle255 {
    pos: EtPos,
    m: Marker255,
}

/// Spawns `count` `EtPos`+`MarkerIdx` rows into bundle `idx` via `spawn_batch`
/// (5k-chunked) and returns the handles. The match dispatches to the distinct
/// `BundleIdx` type so a distinct `{EtPos, MarkerIdx}` archetype is created.
fn spawn_into_archetype(ecs: &mut EcsMaster, idx: usize, count: usize) -> Vec<Entity> {
    macro_rules! spawn_bundle {
        ($Bundle:ident) => {{
            let mut entities: Vec<Entity> = Vec::with_capacity(count);
            let full_chunks = count / CHUNK as usize;
            let remainder = (count % CHUNK as usize) as u32;
            for chunk in 0..full_chunks as u64 {
                let base = chunk * u64::from(CHUNK);
                entities.extend(
                    ecs.spawn_batch((0..CHUNK).map(move |i| $Bundle {
                        pos: pos(base + u64::from(i)),
                        m: Default::default(),
                    }))
                    .expect("5000 <= MAX_BATCH_HINT"),
                );
            }
            if remainder > 0 {
                let base = full_chunks as u64 * u64::from(CHUNK);
                entities.extend(
                    ecs.spawn_batch((0..remainder).map(move |i| $Bundle {
                        pos: pos(base + u64::from(i)),
                        m: Default::default(),
                    }))
                    .expect("remainder < MAX_BATCH_HINT"),
                );
            }
            entities
        }};
    }
    match idx {
        0 => spawn_bundle!(Bundle0),
        1 => spawn_bundle!(Bundle1),
        2 => spawn_bundle!(Bundle2),
        3 => spawn_bundle!(Bundle3),
        4 => spawn_bundle!(Bundle4),
        5 => spawn_bundle!(Bundle5),
        6 => spawn_bundle!(Bundle6),
        7 => spawn_bundle!(Bundle7),
        8 => spawn_bundle!(Bundle8),
        9 => spawn_bundle!(Bundle9),
        10 => spawn_bundle!(Bundle10),
        11 => spawn_bundle!(Bundle11),
        12 => spawn_bundle!(Bundle12),
        13 => spawn_bundle!(Bundle13),
        14 => spawn_bundle!(Bundle14),
        15 => spawn_bundle!(Bundle15),
        16 => spawn_bundle!(Bundle16),
        17 => spawn_bundle!(Bundle17),
        18 => spawn_bundle!(Bundle18),
        19 => spawn_bundle!(Bundle19),
        20 => spawn_bundle!(Bundle20),
        21 => spawn_bundle!(Bundle21),
        22 => spawn_bundle!(Bundle22),
        23 => spawn_bundle!(Bundle23),
        24 => spawn_bundle!(Bundle24),
        25 => spawn_bundle!(Bundle25),
        26 => spawn_bundle!(Bundle26),
        27 => spawn_bundle!(Bundle27),
        28 => spawn_bundle!(Bundle28),
        29 => spawn_bundle!(Bundle29),
        30 => spawn_bundle!(Bundle30),
        31 => spawn_bundle!(Bundle31),
        32 => spawn_bundle!(Bundle32),
        33 => spawn_bundle!(Bundle33),
        34 => spawn_bundle!(Bundle34),
        35 => spawn_bundle!(Bundle35),
        36 => spawn_bundle!(Bundle36),
        37 => spawn_bundle!(Bundle37),
        38 => spawn_bundle!(Bundle38),
        39 => spawn_bundle!(Bundle39),
        40 => spawn_bundle!(Bundle40),
        41 => spawn_bundle!(Bundle41),
        42 => spawn_bundle!(Bundle42),
        43 => spawn_bundle!(Bundle43),
        44 => spawn_bundle!(Bundle44),
        45 => spawn_bundle!(Bundle45),
        46 => spawn_bundle!(Bundle46),
        47 => spawn_bundle!(Bundle47),
        48 => spawn_bundle!(Bundle48),
        49 => spawn_bundle!(Bundle49),
        50 => spawn_bundle!(Bundle50),
        51 => spawn_bundle!(Bundle51),
        52 => spawn_bundle!(Bundle52),
        53 => spawn_bundle!(Bundle53),
        54 => spawn_bundle!(Bundle54),
        55 => spawn_bundle!(Bundle55),
        56 => spawn_bundle!(Bundle56),
        57 => spawn_bundle!(Bundle57),
        58 => spawn_bundle!(Bundle58),
        59 => spawn_bundle!(Bundle59),
        60 => spawn_bundle!(Bundle60),
        61 => spawn_bundle!(Bundle61),
        62 => spawn_bundle!(Bundle62),
        63 => spawn_bundle!(Bundle63),
        64 => spawn_bundle!(Bundle64),
        65 => spawn_bundle!(Bundle65),
        66 => spawn_bundle!(Bundle66),
        67 => spawn_bundle!(Bundle67),
        68 => spawn_bundle!(Bundle68),
        69 => spawn_bundle!(Bundle69),
        70 => spawn_bundle!(Bundle70),
        71 => spawn_bundle!(Bundle71),
        72 => spawn_bundle!(Bundle72),
        73 => spawn_bundle!(Bundle73),
        74 => spawn_bundle!(Bundle74),
        75 => spawn_bundle!(Bundle75),
        76 => spawn_bundle!(Bundle76),
        77 => spawn_bundle!(Bundle77),
        78 => spawn_bundle!(Bundle78),
        79 => spawn_bundle!(Bundle79),
        80 => spawn_bundle!(Bundle80),
        81 => spawn_bundle!(Bundle81),
        82 => spawn_bundle!(Bundle82),
        83 => spawn_bundle!(Bundle83),
        84 => spawn_bundle!(Bundle84),
        85 => spawn_bundle!(Bundle85),
        86 => spawn_bundle!(Bundle86),
        87 => spawn_bundle!(Bundle87),
        88 => spawn_bundle!(Bundle88),
        89 => spawn_bundle!(Bundle89),
        90 => spawn_bundle!(Bundle90),
        91 => spawn_bundle!(Bundle91),
        92 => spawn_bundle!(Bundle92),
        93 => spawn_bundle!(Bundle93),
        94 => spawn_bundle!(Bundle94),
        95 => spawn_bundle!(Bundle95),
        96 => spawn_bundle!(Bundle96),
        97 => spawn_bundle!(Bundle97),
        98 => spawn_bundle!(Bundle98),
        99 => spawn_bundle!(Bundle99),
        100 => spawn_bundle!(Bundle100),
        101 => spawn_bundle!(Bundle101),
        102 => spawn_bundle!(Bundle102),
        103 => spawn_bundle!(Bundle103),
        104 => spawn_bundle!(Bundle104),
        105 => spawn_bundle!(Bundle105),
        106 => spawn_bundle!(Bundle106),
        107 => spawn_bundle!(Bundle107),
        108 => spawn_bundle!(Bundle108),
        109 => spawn_bundle!(Bundle109),
        110 => spawn_bundle!(Bundle110),
        111 => spawn_bundle!(Bundle111),
        112 => spawn_bundle!(Bundle112),
        113 => spawn_bundle!(Bundle113),
        114 => spawn_bundle!(Bundle114),
        115 => spawn_bundle!(Bundle115),
        116 => spawn_bundle!(Bundle116),
        117 => spawn_bundle!(Bundle117),
        118 => spawn_bundle!(Bundle118),
        119 => spawn_bundle!(Bundle119),
        120 => spawn_bundle!(Bundle120),
        121 => spawn_bundle!(Bundle121),
        122 => spawn_bundle!(Bundle122),
        123 => spawn_bundle!(Bundle123),
        124 => spawn_bundle!(Bundle124),
        125 => spawn_bundle!(Bundle125),
        126 => spawn_bundle!(Bundle126),
        127 => spawn_bundle!(Bundle127),
        128 => spawn_bundle!(Bundle128),
        129 => spawn_bundle!(Bundle129),
        130 => spawn_bundle!(Bundle130),
        131 => spawn_bundle!(Bundle131),
        132 => spawn_bundle!(Bundle132),
        133 => spawn_bundle!(Bundle133),
        134 => spawn_bundle!(Bundle134),
        135 => spawn_bundle!(Bundle135),
        136 => spawn_bundle!(Bundle136),
        137 => spawn_bundle!(Bundle137),
        138 => spawn_bundle!(Bundle138),
        139 => spawn_bundle!(Bundle139),
        140 => spawn_bundle!(Bundle140),
        141 => spawn_bundle!(Bundle141),
        142 => spawn_bundle!(Bundle142),
        143 => spawn_bundle!(Bundle143),
        144 => spawn_bundle!(Bundle144),
        145 => spawn_bundle!(Bundle145),
        146 => spawn_bundle!(Bundle146),
        147 => spawn_bundle!(Bundle147),
        148 => spawn_bundle!(Bundle148),
        149 => spawn_bundle!(Bundle149),
        150 => spawn_bundle!(Bundle150),
        151 => spawn_bundle!(Bundle151),
        152 => spawn_bundle!(Bundle152),
        153 => spawn_bundle!(Bundle153),
        154 => spawn_bundle!(Bundle154),
        155 => spawn_bundle!(Bundle155),
        156 => spawn_bundle!(Bundle156),
        157 => spawn_bundle!(Bundle157),
        158 => spawn_bundle!(Bundle158),
        159 => spawn_bundle!(Bundle159),
        160 => spawn_bundle!(Bundle160),
        161 => spawn_bundle!(Bundle161),
        162 => spawn_bundle!(Bundle162),
        163 => spawn_bundle!(Bundle163),
        164 => spawn_bundle!(Bundle164),
        165 => spawn_bundle!(Bundle165),
        166 => spawn_bundle!(Bundle166),
        167 => spawn_bundle!(Bundle167),
        168 => spawn_bundle!(Bundle168),
        169 => spawn_bundle!(Bundle169),
        170 => spawn_bundle!(Bundle170),
        171 => spawn_bundle!(Bundle171),
        172 => spawn_bundle!(Bundle172),
        173 => spawn_bundle!(Bundle173),
        174 => spawn_bundle!(Bundle174),
        175 => spawn_bundle!(Bundle175),
        176 => spawn_bundle!(Bundle176),
        177 => spawn_bundle!(Bundle177),
        178 => spawn_bundle!(Bundle178),
        179 => spawn_bundle!(Bundle179),
        180 => spawn_bundle!(Bundle180),
        181 => spawn_bundle!(Bundle181),
        182 => spawn_bundle!(Bundle182),
        183 => spawn_bundle!(Bundle183),
        184 => spawn_bundle!(Bundle184),
        185 => spawn_bundle!(Bundle185),
        186 => spawn_bundle!(Bundle186),
        187 => spawn_bundle!(Bundle187),
        188 => spawn_bundle!(Bundle188),
        189 => spawn_bundle!(Bundle189),
        190 => spawn_bundle!(Bundle190),
        191 => spawn_bundle!(Bundle191),
        192 => spawn_bundle!(Bundle192),
        193 => spawn_bundle!(Bundle193),
        194 => spawn_bundle!(Bundle194),
        195 => spawn_bundle!(Bundle195),
        196 => spawn_bundle!(Bundle196),
        197 => spawn_bundle!(Bundle197),
        198 => spawn_bundle!(Bundle198),
        199 => spawn_bundle!(Bundle199),
        200 => spawn_bundle!(Bundle200),
        201 => spawn_bundle!(Bundle201),
        202 => spawn_bundle!(Bundle202),
        203 => spawn_bundle!(Bundle203),
        204 => spawn_bundle!(Bundle204),
        205 => spawn_bundle!(Bundle205),
        206 => spawn_bundle!(Bundle206),
        207 => spawn_bundle!(Bundle207),
        208 => spawn_bundle!(Bundle208),
        209 => spawn_bundle!(Bundle209),
        210 => spawn_bundle!(Bundle210),
        211 => spawn_bundle!(Bundle211),
        212 => spawn_bundle!(Bundle212),
        213 => spawn_bundle!(Bundle213),
        214 => spawn_bundle!(Bundle214),
        215 => spawn_bundle!(Bundle215),
        216 => spawn_bundle!(Bundle216),
        217 => spawn_bundle!(Bundle217),
        218 => spawn_bundle!(Bundle218),
        219 => spawn_bundle!(Bundle219),
        220 => spawn_bundle!(Bundle220),
        221 => spawn_bundle!(Bundle221),
        222 => spawn_bundle!(Bundle222),
        223 => spawn_bundle!(Bundle223),
        224 => spawn_bundle!(Bundle224),
        225 => spawn_bundle!(Bundle225),
        226 => spawn_bundle!(Bundle226),
        227 => spawn_bundle!(Bundle227),
        228 => spawn_bundle!(Bundle228),
        229 => spawn_bundle!(Bundle229),
        230 => spawn_bundle!(Bundle230),
        231 => spawn_bundle!(Bundle231),
        232 => spawn_bundle!(Bundle232),
        233 => spawn_bundle!(Bundle233),
        234 => spawn_bundle!(Bundle234),
        235 => spawn_bundle!(Bundle235),
        236 => spawn_bundle!(Bundle236),
        237 => spawn_bundle!(Bundle237),
        238 => spawn_bundle!(Bundle238),
        239 => spawn_bundle!(Bundle239),
        240 => spawn_bundle!(Bundle240),
        241 => spawn_bundle!(Bundle241),
        242 => spawn_bundle!(Bundle242),
        243 => spawn_bundle!(Bundle243),
        244 => spawn_bundle!(Bundle244),
        245 => spawn_bundle!(Bundle245),
        246 => spawn_bundle!(Bundle246),
        247 => spawn_bundle!(Bundle247),
        248 => spawn_bundle!(Bundle248),
        249 => spawn_bundle!(Bundle249),
        250 => spawn_bundle!(Bundle250),
        251 => spawn_bundle!(Bundle251),
        252 => spawn_bundle!(Bundle252),
        253 => spawn_bundle!(Bundle253),
        254 => spawn_bundle!(Bundle254),
        255 => spawn_bundle!(Bundle255),
        other => panic!("marker index {other} out of range (0..256)"),
    }
}

// -- World builder -----------------------------------------------------------

/// Builds a fresh `EcsMaster`, spawns `r` rows into each of the first `m` bundle
/// archetypes (`{EtPos, Marker0}` .. `{EtPos, Marker(m-1)}`), and
/// `enable::<EtFlag>` on all rows of the first `k` of those `m` archetypes.
///
/// Result: `k` archetypes own an `EtFlag` enable-column (k*r enabled rows);
/// `m - k` archetypes have a NULL `EtFlag` column (visited, never yielded). All
/// `m` match `Query<&EtPos, Enabled<EtFlag>>`.
fn build_world(m: usize, r: usize, k: usize) -> EcsMaster {
    assert!(k <= m, "k (with-column) must not exceed m (total archetypes)");
    assert!(m <= 256, "only 256 marker bundles are defined");
    let mut ecs = EcsMaster::new();
    for idx in 0..m {
        let entities = spawn_into_archetype(&mut ecs, idx, r);
        if idx < k {
            for &e in &entities {
                ecs.enable::<EtFlag>(e);
            }
        }
    }
    ecs
}

/// The timed kernel: `Query<&EtPos, Enabled<EtFlag>>` build + full iter, summing
/// `p.x` with a per-element `black_box` so the inner loop cannot be elided.
#[inline]
fn time_query(ecs: &mut EcsMaster) -> u64 {
    let view = ecs.query::<&EtPos, Enabled<EtFlag>>();
    let mut sum = 0u64;
    for p in view.iter() {
        sum = sum.wrapping_add(hint_black_box(p.x));
    }
    sum
}

// ============================================================================
// (A) DECISIVE same-binary A/B  +  (B) R-sweep midpoint (shared (64,256,4) world)
// ============================================================================

fn bench_full_and_r256(c: &mut Criterion) {
    // (64, 256, 4): 4 with-column archetypes (4*256 enabled rows), 60 no-column.
    let mut ecs = build_world(64, 256, 4);
    c.bench_function("cull_full", |b| {
        b.iter(|| black_box(time_query(&mut ecs)));
    });
    // Same world, same id-role as the R-sweep midpoint (R=256). Re-timed so the
    // orchestrator gets `cull_R256` from the identical fixture.
    c.bench_function("cull_R256", |b| {
        b.iter(|| black_box(time_query(&mut ecs)));
    });
}

fn bench_equiv(c: &mut Criterion) {
    // (4, 256, 4): only the 4 with-column archetypes exist -- exactly what the
    // cull would make `cull_full` behave like.
    let mut ecs = build_world(4, 256, 4);
    c.bench_function("cull_equiv", |b| {
        b.iter(|| black_box(time_query(&mut ecs)));
    });
}

// ============================================================================
// (B) R-scaling on no-column archetypes (M=64, K=4): does the null loop scale
//     with rows, or is it collapsed? R in {16, 256, 1024}; R256 above.
// ============================================================================

fn bench_r_sweep(c: &mut Criterion) {
    let mut ecs16 = build_world(64, 16, 4);
    c.bench_function("cull_R16", |b| {
        b.iter(|| black_box(time_query(&mut ecs16)));
    });
    let mut ecs1024 = build_world(64, 1024, 4);
    c.bench_function("cull_R1024", |b| {
        b.iter(|| black_box(time_query(&mut ecs1024)));
    });
}

// ============================================================================
// (C) M-scaling (R=64, K=4): does per-archetype SETUP cost matter even if the
//     row loop is collapsed? M in {8, 64, 256}.
// ============================================================================

fn bench_m_sweep(c: &mut Criterion) {
    let mut ecs8 = build_world(8, 64, 4);
    c.bench_function("cull_M8", |b| {
        b.iter(|| black_box(time_query(&mut ecs8)));
    });
    let mut ecs64 = build_world(64, 64, 4);
    c.bench_function("cull_M64", |b| {
        b.iter(|| black_box(time_query(&mut ecs64)));
    });
    let mut ecs256 = build_world(256, 64, 4);
    c.bench_function("cull_M256", |b| {
        b.iter(|| black_box(time_query(&mut ecs256)));
    });
}

// ============================================================================
// (D) Control: every archetype has a column -- the cull would help nothing.
// ============================================================================

fn bench_control(c: &mut Criterion) {
    // (64, 256, 64): all 64 archetypes own an EtFlag column ("no free lunch").
    let mut ecs = build_world(64, 256, 64);
    c.bench_function("cull_all_have_column", |b| {
        b.iter(|| black_box(time_query(&mut ecs)));
    });
}

criterion_group!(
    cull_diagnostic,
    bench_full_and_r256,
    bench_equiv,
    bench_r_sweep,
    bench_m_sweep,
    bench_control,
);
criterion_main!(cull_diagnostic);
