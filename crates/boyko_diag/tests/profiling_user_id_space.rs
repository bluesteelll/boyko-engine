//! **Profiling rung 10, `G11` (user half) — a game cannot starve the engine of zone ids.**
//!
//! # This file is a `User`-partition crate, and that is the gate
//!
//! An integration test is its own crate root, so the `profiling_partition!(User)` below is a real
//! crate-level partition declaration — not a simulation of one. Every `declare_zone!` in this file
//! therefore mints exactly the way a game's static zone mints, through the same
//! `crate::__BOYKO_ZONE_PARTITION` the macro reads at any other site.
//!
//! That is `G11`'s `[B3-fix]` in one line. Rev 3 of the corpus exercised this property through
//! `register_zone`, and then recommended `declare_zone!` as the game path — so the gate's input
//! class **excluded the defect it was written to catch**. The exhausting leg here is a static
//! `declare_zone!`, which is the path the plan recommends.
//!
//! # Why the exhaustion is driven through `mint_id_in` and not through 3072 macro invocations
//!
//! `declare_zone!` emits a `static` per call, so exhausting a 3072-slot budget by macro would mean
//! 3072 items in this file. What the gate needs is that a `User` crate's mint draws from the user
//! counter — and the macro's whole contribution to that is the `region` field it puts in the
//! descriptor. So this file does both: it proves the **macro's** minting lands in the user half
//! (with a real `declare_zone!`, below), and it drives the **exhaustion** through the counter that
//! macro feeds. Splitting it that way is what keeps the file readable without moving the claim.
//!
//! # ⚠️ Its own binary, deliberately
//!
//! `USER_ID_NEXT` and `ENGINE_ID_NEXT` are process-global and monotone. `crates/boyko_ecs/tests/
//! profiling_zone_registry.rs` exhausts the ENGINE counter in *its* process for the same reason,
//! and the two exhaustions cannot share one — the engine mint this file asserts still succeeds
//! would be refused by that file's leftovers. One process per exhaustion is not tidiness; it is the
//! only arrangement in which either claim is about what it says it is.

use boyko_diag::profiling_abi::dyn_registry::{RegisterError, ZoneSpec, register_zone};
use boyko_diag::profiling_abi::{
    ENGINE_ZONE_SLOTS, MAX_USER_BUDGET, ZONE_ID_EXHAUSTED, ZoneTier, mint_id, mint_id_in,
    minted_user_zones, minted_zones, zone_id,
};
use boyko_diag::sample::Region;

boyko_diag::profiling_partition!(User);

// A real static game zone, declared exactly as a game would declare one. Its id is what proves the
// macro routes by the DECLARING CRATE.
boyko_diag::declare_zone!(
    GAME_TICK,
    name = "game.tick",
    scope = 33,
    tier = ZoneTier::Always
);

/// **`G11`, the whole clause.** A game's static zone mints from the user half; exhausting the user
/// budget refuses the game and leaves the engine's next mint working.
///
/// One test, because the counters are process-global and monotone: split across two `#[test]`s,
/// `libtest`'s ordering would decide whether the engine mint happens before or after the
/// exhaustion, and only one of those orders tests anything.
#[test]
fn a_game_exhausting_its_budget_leaves_the_engine_minting() {
    // ---- 1. The recommended game path mints from the USER half. ----
    //
    // The RED for this clause is the one `[B3-fix]` names: key the partition on the MACRO rather
    // than on the declaring crate — i.e. make `mint_cold` call `mint_id()` instead of
    // `mint_id_in(handle.desc.region)` — and this id comes back below `ENGINE_ZONE_SLOTS`.
    let game_id = zone_id(&GAME_TICK);
    assert!(
        game_id as usize >= ENGINE_ZONE_SLOTS,
        "a static `declare_zone!` in a `profiling_partition!(User)` crate minted id {game_id}, \
         which is inside the ENGINE range [0, {ENGINE_ZONE_SLOTS}). The recommended game path is \
         taking engine ids -- which is the exact defect G11's input class used to exclude."
    );
    assert!(
        (game_id as usize) < ENGINE_ZONE_SLOTS + MAX_USER_BUDGET,
        "a user id must stay inside the user range"
    );
    assert_eq!(
        GAME_TICK.desc.region,
        Region::User,
        "the descriptor's region is what routes the mint; if this is Engine the partition never \
         reached the macro"
    );

    // ---- 2. An engine mint, taken BEFORE the exhaustion, for the comparison below. ----
    let engine_before = mint_id();
    assert!(
        (engine_before as usize) < ENGINE_ZONE_SLOTS,
        "an engine mint must come from the engine range"
    );

    // ---- 3. Exhaust the user budget. ----
    //
    // Through the same counter `declare_zone!` feeds, for the reason in the module docs. `+ 8`
    // rather than exactly the budget: the loop must run past the boundary so the refusal is
    // observed, not merely arrived at.
    let mut first_refusal = None;
    for i in 0..(MAX_USER_BUDGET + 8) {
        let id = mint_id_in(Region::User);
        if id == ZONE_ID_EXHAUSTED && first_refusal.is_none() {
            first_refusal = Some(i);
        }
    }
    let refused_at = first_refusal.expect(
        "the user budget must run out: MAX_USER_BUDGET mints plus eight more cannot all succeed, \
         and if they did the budget is not bounding anything",
    );
    assert!(
        refused_at <= MAX_USER_BUDGET,
        "the budget refused at {refused_at}, past MAX_USER_BUDGET = {MAX_USER_BUDGET}"
    );
    assert_eq!(
        minted_user_zones(),
        MAX_USER_BUDGET as u16,
        "the user occupancy must saturate at the budget, never report past it"
    );

    // Every further user mint keeps refusing — no wrap, no duplicate hand-out.
    for _ in 0..8 {
        assert_eq!(
            mint_id_in(Region::User),
            ZONE_ID_EXHAUSTED,
            "an exhausted user range must keep refusing rather than wrap into the engine's"
        );
    }
    // And so does the dynamic path, which draws from the same counter (D19).
    assert_eq!(
        register_zone(ZoneSpec { name: "too.late", scope: 40, tier: ZoneTier::Always }),
        Err(RegisterError::BudgetExhausted),
        "`register_zone` and `declare_zone!` share the user budget; one exhausting it must refuse \
         the other"
    );

    // ---- 4. THE CLAUSE: the engine still mints, and its ids are still engine ids. ----
    let engine_after = mint_id();
    assert_ne!(
        engine_after, ZONE_ID_EXHAUSTED,
        "the engine's mint was refused after a GAME exhausted its budget. The two counters are \
         supposed to be independent; if this fires they are sharing supply, and a plugin can \
         starve the engine of zones."
    );
    assert!(
        (engine_after as usize) < ENGINE_ZONE_SLOTS,
        "the engine's mint after user exhaustion returned {engine_after}, outside the engine range"
    );
    assert_eq!(
        engine_after,
        engine_before + 1,
        "the engine counter must be untouched by the user exhaustion -- it should have advanced by \
         exactly the one mint taken between the two reads"
    );
    assert!(
        minted_zones() < ENGINE_ZONE_SLOTS as u16,
        "the engine range must still have room; a user exhaustion that filled it would mean one \
         counter wearing two names"
    );
}
