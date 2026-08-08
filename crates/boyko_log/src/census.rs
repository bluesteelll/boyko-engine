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
use crate::target::{TargetId, engine_targets, target_control, target_stats};

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

/// Take one row per engine target.
///
/// **Every** engine target appears, not only the armed ones: a target a host forgot to arm is
/// exactly the case the census exists to make visible, and omitting it would let "no row" read as
/// "nothing to report".
pub fn rows() -> impl Iterator<Item = CensusRow> {
    engine_targets().map(|(id, name)| {
        let (delivered, dropped, sampled_out, _sync) = target_stats(id);
        let status = if dropped > 0 {
            // Lossy wins over everything else it could be: a count that is a lower bound must
            // never be presented as a total, whatever else is also true of it.
            LossStatus::UnprovenLossy
        } else if sampled_out > 0 {
            LossStatus::UnprovenSampled
        } else if delivered == 0 {
            LossStatus::Unproven
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

/// `true` when any target's counts are a lower bound rather than a total.
///
/// The single bit a UI must read before rendering any count as a total.
#[must_use]
pub fn lossy() -> bool {
    rows().any(|r| r.status != LossStatus::Measured && r.status != LossStatus::Unproven)
}

/// Print the census through the synchronous channel, one line per target.
///
/// Returns the number of lines written — `0` when no synchronous destination is configured, which
/// is not a failure: a host that asked for no console and no crash file asked for no census
/// either, and saying so through the return value beats writing it nowhere and reporting success.
pub fn print() -> u32 {
    let mut written = 0;
    for row in rows() {
        let mut buf = [0u8; 160];
        let n = render(&mut buf, &row);
        // SAFETY: `render` writes only ASCII copied from `&'static str`s and decimal digits.
        let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
        if crate::sync_out::write_oracle_line("LOG-CENSUS ", text).is_some() {
            written += 1;
        }
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
    #[test]
    fn the_census_covers_every_engine_target() {
        let n = rows().count();
        assert_eq!(n, engine_targets().count());
        assert!(n >= 26, "the engine table has at least 26 rows; saw {n}");
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
}
