//! The `LOG-CENSUS` — the report that makes a silence say which kind of silence it is.
//!
//! # The one thing this file exists to prevent
//!
//! A target that delivered nothing is **`Unproven`**, never *clean*. "No warnings from the
//! renderer" and "the renderer's warnings were switched off" produce the same empty log, and a
//! reader who cannot tell them apart will read the second as the first. Every status below names
//! one way a silence can be manufactured, so the report answers the question the empty log raises
//! instead of restating it.
//!
//! # There is no `vk-validation` row, and its absence is the design
//!
//! The Vulkan messenger is never edited by this campaign, so no record can ever reach a
//! `vk-validation` target. A row for it would read `records=0 status=Unproven` in every run
//! forever — whether the layer is on, off, dead, or printing nineteen messages at stderr. A status
//! that cannot change carries no information: it is a green-because-it-cannot-fail row wearing the
//! exact vocabulary invented to prevent them.

use boyko_diag::loss::LossStatus;

use crate::level::Level;
use crate::target::{TargetId, target_control, target_stats};

/// One target's census row.
#[derive(Clone, Copy)]
pub struct CensusRow {
    /// The target's id.
    pub id: TargetId,
    /// The target's printed name.
    pub name: &'static str,
    /// Its runtime ceiling at the moment the row was taken.
    pub level: Level,
    /// Records delivered to a sink.
    pub records: u64,
    /// Records the emission path refused.
    pub dropped: u64,
    /// What the counts may be read as.
    pub status: LossStatus,
}

impl CensusRow {
    /// The rendered status token — the string a support ticket quotes and a reader greps.
    ///
    /// Deliberately the v3 spelling rather than the type's `Debug`: these strings are the stable
    /// surface, and a `Debug` derive would silently rename them the day a variant is renamed.
    #[must_use]
    pub const fn status_str(&self) -> &'static str {
        match self.status {
            LossStatus::Measured => "MEASURED",
            LossStatus::Unproven => "UNPROVEN",
            LossStatus::UnprovenLossy => "UNPROVEN(lossy)",
            LossStatus::UnprovenSampled => "UNPROVEN(sampled)",
            LossStatus::UnprovenUnsunk => "UNPROVEN(unsunk)",
        }
    }
}

/// Take one row per target that exists — every engine row, then every REGISTERED dynamic one.
///
/// **Every** engine target appears, not only the armed ones: a target a host forgot to arm is
/// exactly the case the census exists to make visible, and omitting it would let "no row" read as
/// "nothing to report".
///
/// **Dynamic targets appear too, under their interned names** *(L10-B)*. Without this a mod's
/// records would arrive, be counted in `TARGET_STATS`, and be invisible in the one place a reader
/// looks to find out what was and was not measured — delivered to a row nobody prints, which is
/// this campaign's signature defect. Unregistered slots are absent rather than blank, which is
/// `targets()`'s rule and not a second one: a row for a target that does not exist is the vacuous
/// row this vocabulary was invented to prevent.
pub fn rows() -> impl Iterator<Item = CensusRow> {
    crate::target::targets().map(|(id, name)| {
        let (delivered, dropped, sampled_out, _sync) = target_stats(id);
        let status = if dropped > 0 {
            // Lossy wins over everything else it could be: a count that is a lower bound must
            // never be presented as a total, whatever else is also true of it.
            LossStatus::UnprovenLossy
        } else if sampled_out > 0 {
            LossStatus::UnprovenSampled
        } else if delivered == 0 {
            // AN ARMED TARGET THAT NO SINK ACCEPTS IS NOT "NOTHING HAPPENED" (disposition E20).
            // A game enables a category, sees an empty log, and concludes clean -- the vacuous
            // gate in a new costume. The two silences are indistinguishable in the file and are
            // told apart here, where the policy table can still be asked.
            //
            // Checked at the target's OWN ceiling, not at a fixed level: a target armed at `Warn`
            // whose sink floor is `Warn` is perfectly sunk, and testing it at `Trace` would report
            // every correctly-configured pair as unsunk.
            let ceiling = target_control(id).level();
            if ceiling != crate::Level::Off
                && !crate::sink::slot::any_sink_accepts(id, ceiling)
            {
                // NO REPORT HERE. `rows()` is a public ITERATOR: a host rendering a census overlay
                // walks it every frame, and a `warn!` on that path made `W0111` -- a row declaring
                // `Once` -- emit once per unsunk target per frame. The report moved to `print()`,
                // which runs at flush and at shutdown. Found by the `Once` register, which is the
                // first thing in this crate able to see a `Once` row firing more than once.
                LossStatus::UnprovenUnsunk
            } else {
                LossStatus::Unproven
            }
        } else {
            LossStatus::Measured
        };
        CensusRow {
            id,
            name,
            level: target_control(id).level(),
            records: delivered,
            dropped,
            status,
        }
    })
}

/// The `W0111` latch. **A named module-level `static`**, for the reason `OnceSite::reset` gives:
/// state a test cannot name is state a test cannot control.
static UNSUNK_REPORTED: crate::codes::OnceSite = crate::codes::OnceSite::new();

/// `boyko-W0111`: a target is armed and **no `Active` sink accepts it**.
///
/// `Once`, because the condition is a CONFIGURATION and not an event: every later row with the
/// same policy is also unsunk, and reporting each would turn one misconfiguration into a storm of
/// reports about it -- the argument `W0501` records for the query-type table and `W0114` for the
/// index budget.
///
/// It names the target, because "something was unsunk" leaves a reader diffing a census against a
/// sink policy by hand, which is the state a code exists to replace.
#[cold]
#[inline(never)]
fn report_unsunk(id: crate::TargetId, name: &str) {
    crate::warn!(
        crate::Log,
        crate::codes::W0111,
        "target {} is armed at {} but no active sink accepts it: its silence is not evidence",
        name,
        target_control(id).level().as_str()
    );
}

/// `true` when any target's counts are a lower bound rather than a total.
///
/// The single bit a UI must read before rendering any count as a total.
#[must_use]
pub fn lossy() -> bool {
    rows().any(|r| r.status != LossStatus::Measured && r.status != LossStatus::Unproven)
}

/// Emit the census through the RING, one record per target, plus the limiter line and one
/// `LOG-ONCE` row per fired site.
///
/// Ordinary records rather than the synchronous console channel (owner-directed 2026-08-20): the
/// summary lands in every sink the preset opened, so a `shipping` `.blog` and a `shipping-min`
/// text file carry their own loss report instead of leaving it on a console those presets turn
/// off. The caller owes a delivery pass afterwards — [`crate::lifecycle::shutdown`] runs its
/// final drain AFTER this, and closes the sinks after THAT.
///
/// Returns the number of rows OFFERED to the ring. Offered, not delivered: emission is subject to
/// the same four admission gates as any record, so an un-armed process offers rows that go
/// nowhere — the same silence, decided by the same policy, as every other record it never asked
/// to see.
pub fn print() -> u32 {
    let mut written = 0;
    for row in rows() {
        // THE UNSUNK REPORT LIVES HERE, and behind a latch, because the row declares `Once`.
        //
        // ONE latch for every target rather than one per target, which is what `W0111`'s own doc
        // comment argues for: the condition is a CONFIGURATION, so every later unsunk target is
        // the same misconfiguration, and reporting each would turn one mistake into a storm of
        // reports about it. That is the opposite of `W2102`'s three-site case, where the sites are
        // independent -- the granularity is a per-code judgement, not a rule.
        if row.status == LossStatus::UnprovenUnsunk && UNSUNK_REPORTED.claim() {
            report_unsunk(row.id, row.name);
        }
        let mut buf = [0u8; 160];
        let n = render(&mut buf, &row);
        // SAFETY: `render` writes only ASCII copied from `&'static str`s and decimal digits.
        let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
        // THROUGH THE RING, not the console channel (owner-directed 2026-08-20). The census used
        // to go through `sync_out::write_oracle_line` alone -- so under `shipping` and
        // `shipping-min`, whose console is OFF, the summary reached no destination at all and the
        // uploaded log of a released title could not say whether it lost anything. An ordinary
        // record lands in every sink the preset opened, console included, by the same slot policy
        // as everything else. The cost, stated: these rows are subject to the admission control
        // they report on -- acceptable at shutdown, where the lanes are quiet and the final drain
        // runs immediately after.
        crate::info!(crate::Log, "LOG-CENSUS {}", crate::dsp!(text, 160));
        written += 1;
    }
    let mut buf = [0u8; 96];
    let n = render_limiter(&mut buf);
    // SAFETY: `render_limiter` writes only ASCII literals and decimal digits.
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    crate::info!(crate::Log, "LOG-CENSUS {}", crate::dsp!(text, 96));
    written += 1;
    // ONE `LOG-ONCE` ROW PER FIRED SITE, under its own prefix.
    //
    // A separate prefix rather than more `LOG-CENSUS` lines, because these rows are per SITE while
    // every other census row is per TARGET, and a reader filtering on one prefix must not have to
    // know which shape each line is.
    for row in crate::once_sites::walk() {
        let mut buf = [0u8; 320];
        let n = render_once(&mut buf, &row);
        // SAFETY: `render_once` writes ASCII literals, decimal digits, and `LogSite::file`, which
        //   is a `&'static str` from `file!()`.
        let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
        crate::info!(crate::Log, "LOG-ONCE {}", crate::dsp!(text, 320));
        written += 1;
    }
    written
}

/// Render `target=N level=L records=R dropped=D status=S` into `buf`.
///
/// Hand-rolled rather than `core::fmt` for the same reason every other renderer in this crate is:
/// the census runs at flush and at shutdown, and shutdown can be reached from a panic hook, where
/// a formatter that allocates is a second failure on top of the first.
fn render(buf: &mut [u8], row: &CensusRow) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    let dec = |v: u64, n: &mut usize, put: &mut dyn FnMut(&[u8], &mut usize)| {
        let mut d = [0u8; 20];
        let mut v = v;
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        put(&d[i..], n);
    };
    put(b"target=", &mut n);
    put(row.name.as_bytes(), &mut n);
    put(b" level=", &mut n);
    put(row.level.as_str().as_bytes(), &mut n);
    put(b" records=", &mut n);
    dec(row.records, &mut n, &mut put);
    put(b" dropped=", &mut n);
    dec(row.dropped, &mut n, &mut put);
    put(b" status=", &mut n);
    put(row.status_str().as_bytes(), &mut n);
    n
}

/// Render `code=W2102 site=<file>:<line> fired=N suppressed=…` into `buf`.
///
/// # `fired` is the row that matters, and anything above `1` is a defect
///
/// A `Once` site with a latch fires exactly once per process. `fired=17` is a site whose registry
/// row promises `Once` and whose code delivers seventeen -- the condition that was previously
/// findable only by grepping identifier uses, which cannot tell an emitter from a doc link.
///
/// # The FULL path, not the basename
///
/// The corpus's worked example prints `site=device.rs:3100`. Two files in this workspace may share
/// a basename -- `mod.rs` is in twenty directories -- so a reader given the basename has to guess
/// which one, and the guess is silent. The path is what makes the row actionable.
fn render_once(buf: &mut [u8], row: &crate::once_sites::OnceRow) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    let dec = |v: u64, n: &mut usize, put: &mut dyn FnMut(&[u8], &mut usize)| {
        let mut d = [0u8; 20];
        let mut v = v;
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        put(&d[i..], n);
    };
    put(b"code=", &mut n);
    put(&[row.class], &mut n);
    dec(u64::from(row.code), &mut n, &mut put);
    put(b" site=", &mut n);
    put(row.file.as_bytes(), &mut n);
    put(b":", &mut n);
    dec(u64::from(row.line), &mut n, &mut put);
    put(b" fired=", &mut n);
    dec(u64::from(row.fired), &mut n, &mut put);
    // `Once` does not count its suppressions BY DESIGN -- counting them costs an RMW on a shared
    // line during the storm the policy exists to damp. Saying so is not the same as saying zero,
    // and the two must never render alike.
    if row.counted {
        // `OnceCounted` is specified to carry a real number and no counter is wired for it yet.
        // Named rather than printed as zero, for the same reason.
        put(b" suppressed=UNWIRED(OnceCounted)", &mut n);
    } else {
        put(b" suppressed=UNCOUNTED(by policy)", &mut n);
    }
    if row.fired > 1 {
        put(b"  <-- DECLARES Once AND HAS NO LATCH", &mut n);
    }
    n
}

/// Render `limiter suppressed=N unindexed=M` into `buf`.
///
/// # It is printed UNCONDITIONALLY, and a zero row is the point
///
/// A line that appeared only when a counter was non-zero would make "the limiter refused nothing"
/// and "this census does not report the limiter" the same output — the silence-is-not-evidence
/// defect `W0111` exists to refuse, one level up. A reader must be able to see the zero.
///
/// # What it closes
///
/// `03-CODES-REGISTRY.md` requires that `E0115` and the unindexed count are **both printed by the
/// census**, and until the rate gate was wired neither counter had a production reader at all:
/// `rate::suppressed` and `rate::unindexed` were `pub`, written by `admit`, and read by nothing.
/// The moment a policy actually suppresses, a log gets quieter — and a quietness nothing accounts
/// for is exactly what a diagnostics census is for.
fn render_limiter(buf: &mut [u8]) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    let dec = |v: u64, n: &mut usize, put: &mut dyn FnMut(&[u8], &mut usize)| {
        let mut d = [0u8; 20];
        let mut v = v;
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        put(&d[i..], n);
    };
    put(b"limiter suppressed=", &mut n);
    dec(crate::rate::suppressed(), &mut n, &mut put);
    put(b" unindexed=", &mut n);
    dec(crate::rate::unindexed(), &mut n, &mut put);
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{LogTarget, TargetControl, set_target_control};

    /// A target that delivered nothing is `UNPROVEN`, and arming it does not change that.
    ///
    /// The second half is the point: v2's census would have called an armed-but-silent target
    /// clean, which is the exact misreading the vocabulary exists to block. A level is a
    /// permission, not an observation.
    #[test]
    fn a_silent_target_is_unproven_whether_armed_or_not() {
        let id = <crate::Fontbake as LogTarget>::ID;
        let row = rows().find(|r| r.id == id).expect("every engine target has a row");
        assert_eq!(row.records, 0, "fixture: this target must be untouched by other tests");
        assert!(row.status == LossStatus::Unproven);
        assert_eq!(row.status_str(), "UNPROVEN");

        set_target_control(id, TargetControl::new(Level::Trace, 0, false));
        let row = rows().find(|r| r.id == id).expect("still present");
        assert!(row.status == LossStatus::Unproven, "arming a target proves nothing about it");
        set_target_control(id, TargetControl::OFF);
    }

    /// Every engine target gets a row — a missing row would read as "nothing to report".
    ///
    /// **The count is engine + REGISTERED DYNAMIC since L10-B**, and writing it that way is not
    /// pedantry. The previous form asserted `rows().count() == engine_targets().count()`, which
    /// after L10-B is true here only because this binary registers no dynamic target — an
    /// assertion that passes for a reason unrelated to what it claims, and one that would have
    /// gone red the first time a `#[cfg(test)]` neighbour registered one.
    #[test]
    fn the_census_covers_every_engine_target_and_every_registered_dynamic_one() {
        let n = rows().count();
        let engine = crate::target::engine_targets().count();
        assert_eq!(n, engine + crate::target::dyn_registered());
        assert!(n >= 26, "the engine table has at least 26 rows; saw {n}");

        // The claim the count alone does not make: each engine row is PRESENT, by id.
        for (id, name) in crate::target::engine_targets() {
            let row = rows().find(|r| r.id == id);
            assert!(row.is_some(), "engine target {name} has no census row");
        }
    }

    /// A row renders to the exact shape a reader greps for.
    #[test]
    fn a_row_renders_to_the_documented_shape() {
        let row = CensusRow {
            id: <crate::Ecs as LogTarget>::ID,
            name: "ecs",
            level: Level::Warn,
            records: 7,
            dropped: 0,
            status: LossStatus::Measured,
        };
        let mut buf = [0u8; 160];
        let n = render(&mut buf, &row);
        assert_eq!(
            core::str::from_utf8(&buf[..n]).expect("ascii"),
            "target=ecs level=warn records=7 dropped=0 status=MEASURED"
        );
    }

    /// The census reports what the limiter refused, and the number is the LIVE counter.
    ///
    /// Two renders around a known suppression, and the claim is a DELTA rather than an absolute:
    /// `SUPPRESSED` is process-global and the lib tests run on many threads, so an equality against
    /// a snapshot would be a race. A monotone counter makes `after - before >= 56` sound whatever
    /// else is running.
    ///
    /// It drives `rate::admit` directly rather than through a macro because the subject here is the
    /// CENSUS reading the counter -- the macro's own wiring is `tests/l8a_rate_policy_wired.rs`.
    #[test]
    fn the_census_prints_what_the_rate_limiter_refused() {
        fn suppressed_in_line() -> u64 {
            let mut buf = [0u8; 96];
            let n = render_limiter(&mut buf);
            let text = core::str::from_utf8(&buf[..n]).expect("ASCII only");
            let at = text.find("suppressed=").expect("the census line names the counter");
            let rest = &text[at + "suppressed=".len()..];
            let end = rest.find(' ').unwrap_or(rest.len());
            rest[..end].parse().expect("a decimal count")
        }

        let before = suppressed_in_line();
        // A slot no registry row owns, so this test's suppression is its own.
        const IDX: u32 = 502;
        for _ in 0..64 {
            let _ = crate::rate::admit(IDX, crate::codes::RatePolicy::EveryN(8), 0);
        }
        let after = suppressed_in_line();
        assert!(
            after - before >= 56,
            "the census must report the limiter's refusals: {before} -> {after}"
        );

        let mut buf = [0u8; 96];
        let n = render_limiter(&mut buf);
        let text = core::str::from_utf8(&buf[..n]).expect("ASCII only");
        assert!(text.contains("unindexed="), "the unindexed count is owed by the corpus too");

        // ── AND `print` MUST ACTUALLY EMIT IT ────────────────────────────────────────────────
        //
        // Everything above would pass on a `render_limiter` that no caller reaches -- which is the
        // dead-datum shape this whole line exists to remove, reproduced one level in. So the count
        // `print` returns is checked against the rows plus exactly one.
        //
        // The count is rows OFFERED to the ring, so it holds whether or not this process armed a
        // target -- which is the point: the census obeys the same admission gates as every other
        // record, and this test pins the offering, not the policy.
        let lines = print();
        assert_eq!(
            lines as usize,
            rows().count() + 1 + crate::once_sites::walk().count(),
            "the census must offer one row per target, the limiter line, and one LOG-ONCE row \
             per fired site"
        );
    }
}
