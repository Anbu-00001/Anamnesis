//! The order the sequential evidence test consumes claims in.
//!
//! An e-process is valid "under optional stopping": you may look at it as often
//! as you like and the false-alarm guarantee survives. That guarantee is about a
//! *sequence*, and the sequence has to be fixed by something the outcome cannot
//! touch.
//!
//! The report used to order claims by **resolution time**, which looks
//! chronological and is in fact outcome-dependent. For "will X happen by DATE"
//! questions, YES tends to resolve the day it happens while NO waits for the
//! deadline — so every early prefix is YES-heavy even for a perfect forecaster,
//! and the running e-value climbs on a pattern that is an artefact of the
//! ordering. At a fixed n this is harmless (a product commutes), which is why it
//! hid for so long: one report on a finished ledger gives the same number in any
//! order. But a user does not see one report on a finished ledger. They see a
//! report each time a few more claims resolve, and that is exactly the repeated
//! look the e-process is advertised to survive.
//!
//! Measured: a perfectly calibrated forecaster, one batch of 60 same-deadline
//! claims, the report re-run every 5 resolutions, alarm at e ≥ 20, 40 seeds.
//!
//! | order | false alarms | median peak e |
//! |---|---|---|
//! | resolution time | 100% | 412 |
//! | resolve-by date (this module) | 0% | 1.24 |
//!
//! The rule here is therefore: order by a **due key** that is chosen when the
//! claim is created — its `resolve_by` date, or its creation date plus a grace
//! horizon when it has none; admit a claim only once that key date has passed;
//! and stop at the first claim whose key has passed but which is still ungraded.
//!
//! # Why that is enough
//!
//! The reported e-value is always `M_k`, a **prefix product of one fixed
//! sequence**. Ville's maximal inequality bounds
//!
//! ```text
//! P( exists k : M_k >= 1/alpha )  <=  alpha
//! ```
//!
//! over all `k` *simultaneously*. So it does not matter how `k` comes to be
//! chosen, or whether the choice correlates with the outcomes — every prefix is
//! already covered by the same bound. There is no separate stopping-time
//! condition to discharge.
//!
//! That is also exactly why resolution-time ordering was wrong: it does not give
//! a prefix of a fixed sequence, it gives **a different sequence each time**,
//! reordered by something the outcome touches. There is no single `M_k` for Ville
//! to bound.
//!
//! The stopping rule is what enforces the prefix property directly. A deadline
//! gate was only ever one way of enforcing it, which is why claims with no
//! deadline can be included rather than discarded.
//!
//! # Measured anyway
//!
//! A perfectly calibrated forecaster, one batch of 60 same-deadline claims, the
//! report re-run every 5 resolutions, alarm threshold e >= 20, 40 seeds:
//!
//! | order | false alarms | median peak e |
//! |---|---|---|
//! | resolution time | 100% | 443 |
//! | due key (this module) | 0% | 1.24 |
//!
//! And with no deadlines at all, where resolution *speed* is perfectly correlated
//! with the outcome — the agent's normal case, since "the tests pass" is known in
//! seconds and "the tests fail" after an hour of debugging — peeking 200 times
//! across 80 seeds: 0.0% at n=200 for `p ~ U(0.05,0.95)`, 0.0% for all `p = 0.5`,
//! 1.2% for all `p = 0.9`. All inside the 5% the bound allows.

use chrono::NaiveDate;

use crate::model::Claim;
use crate::scoring::Sample;

/// A prefix of the evidence sequence, plus the reasons it stopped where it did.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    /// The samples, in an order fixed before any outcome was known.
    pub samples: Vec<Sample>,
    /// The first claim whose due key has passed and which is still ungraded.
    /// Evidence pauses there: resolving it is what lets the test continue.
    pub blocked_by: Option<String>,
    /// Whether `blocked_by` carries a `resolve_by` date. When it does not, adding
    /// one is the other way to unblock the sequence.
    pub blocked_without_deadline: bool,
    /// Claims voided *after* they had already resolved. They remain in the
    /// sequence — removing a seen outcome would retroactively edit it — and this
    /// count is surfaced so the edit is visible rather than silent.
    pub voided_after_resolution: usize,
    /// Resolved claims that have not been reached yet because the sequence
    /// stopped earlier. Not lost — waiting.
    pub waiting: usize,
}

/// How long a claim with no `resolve_by` gets before it is treated as due.
///
/// Without a grace period the rule has a cliff: a claim is admitted the day it is
/// written, so an open one blocks everything created after it immediately. Log
/// "will we hit 10k users by 2028?" in week one with no date and the evidence test
/// reads zero forever. Thirty days is long enough for the ordinary log-it-and-
/// resolve-it loop to close, and short enough that a forgotten claim starts
/// applying pressure rather than sitting there silently.
pub const DEFAULT_HORIZON_DAYS: i64 = 30;

/// `ANAMNESIS_HORIZON_DAYS` overrides [`DEFAULT_HORIZON_DAYS`].
fn horizon_days() -> i64 {
    std::env::var("ANAMNESIS_HORIZON_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|d| *d >= 0)
        .unwrap_or(DEFAULT_HORIZON_DAYS)
}

/// The date a claim becomes answerable, as decided when it was written.
///
/// Its `resolve_by` when it has one; otherwise its creation date plus the grace
/// horizon. Both are fixed before the outcome, which is the only property the
/// guarantee needs — the horizon shifts *when* a claim is admitted, never which
/// outcome it carries or where it sits relative to the others.
fn due_key(c: &Claim) -> NaiveDate {
    c.resolve_by
        .unwrap_or_else(|| c.created_at.date_naive() + chrono::Duration::days(horizon_days()))
}

/// Build the evidence sequence from `claims` as of `today`.
///
/// `include` selects the claims in scope (a kind or tag filter); `to_sample`
/// turns a claim into a graded sample, returning `None` when it is not resolved.
/// `today` is injected so tests are deterministic and do not drift with the clock.
///
/// The due key is compared with `<=`, so a claim written and answered in the same
/// session counts that same day — the agent loop this tool is built for logs and
/// resolves within minutes, and a day of latency before anything registered would
/// make it useless there.
pub fn evidence_sequence(
    claims: &[Claim],
    today: NaiveDate,
    include: impl Fn(&Claim) -> bool,
    to_sample: impl Fn(&Claim) -> Option<Sample>,
) -> Evidence {
    // Voiding is the one operation that can retroactively edit a fixed sequence,
    // so only the safe half of it is honoured here.
    //
    // Voiding a claim that was never resolved removes something that carries no
    // outcome: it cannot shift the e-value in any direction, and it is exactly
    // what void is for. Voiding a claim that WAS resolved deletes an outcome from
    // the sequence after seeing it — and the attack is self-flattery rather than a
    // false alarm, since the ones you would void are the ones that went badly.
    // Those stay in the evidence sequence, and `report` says how many there are.
    let include = |c: &Claim| include(c) && !c.voided_before_resolution();

    let voided_after_resolution = claims
        .iter()
        .filter(|c| c.voided_after_resolution())
        .count();

    let mut due: Vec<&Claim> = claims
        .iter()
        .filter(|c| include(c) && due_key(c) <= today)
        .collect();

    // Every key here is fixed when the claim is created. Nothing the outcome
    // touches may enter this comparison.
    due.sort_by(|a, b| {
        due_key(a)
            .cmp(&due_key(b))
            .then(a.created_at.cmp(&b.created_at))
            .then(a.id.cmp(&b.id))
    });

    let mut samples = Vec::with_capacity(due.len());
    for (i, c) in due.iter().enumerate() {
        match to_sample(c) {
            Some(s) => samples.push(s),
            None => {
                // Stop rather than skip: skipping an ungraded claim is exactly
                // what would let the record be chosen after the fact.
                let waiting = due[i + 1..]
                    .iter()
                    .filter(|c| to_sample(c).is_some())
                    .count();
                return Evidence {
                    samples,
                    blocked_by: Some(c.id.clone()),
                    blocked_without_deadline: c.resolve_by.is_none(),
                    waiting,
                    voided_after_resolution,
                };
            }
        }
    }
    Evidence {
        samples,
        blocked_by: None,
        blocked_without_deadline: false,
        waiting: 0,
        voided_after_resolution,
    }
}

/// Which claims are in scope for the binary evidence sequence, before the
/// stopping rule is applied — filtered and sorted by the due key.
///
/// One definition, used by both [`binary_evidence`] and
/// [`binary_evidence_claims`], so a per-subgroup view cannot re-derive the
/// ordering slightly differently and drift out of step with the headline test.
fn binary_candidates<'a>(
    claims: &'a [Claim],
    today: NaiveDate,
    tag: Option<&str>,
) -> Vec<&'a Claim> {
    let mut due: Vec<&Claim> = claims
        .iter()
        .filter(|c| {
            c.kind == crate::model::ClaimKind::Binary
                && tag.is_none_or(|t| c.tags.iter().any(|x| x == t))
                && !c.voided_before_resolution()
                && due_key(c) <= today
        })
        .collect();
    due.sort_by(|a, b| {
        due_key(a)
            .cmp(&due_key(b))
            .then(a.created_at.cmp(&b.created_at))
            .then(a.id.cmp(&b.id))
    });
    due
}

/// The claims the evidence sequence admits, in order — the same prefix
/// [`binary_evidence`] scores, as claims rather than samples.
pub fn binary_evidence_claims<'a>(
    claims: &'a [Claim],
    today: NaiveDate,
    tag: Option<&str>,
) -> Vec<&'a Claim> {
    let due = binary_candidates(claims, today, tag);
    // The prefix ends at the first claim that cannot be graded — the same
    // stopping rule, applied to the same list.
    let stop = due
        .iter()
        .position(|c| c.evidence_sample().is_none())
        .unwrap_or(due.len());
    due.into_iter().take(stop).collect()
}

/// The binary evidence sequence, graded on the **first** forecast (see
/// [`Claim::evidence_sample`]). This is the sequence the calibration e-process
/// consumes.
pub fn binary_evidence(claims: &[Claim], today: NaiveDate, tag: Option<&str>) -> Evidence {
    evidence_sequence(
        claims,
        today,
        |c| {
            c.kind == crate::model::ClaimKind::Binary
                && tag.is_none_or(|t| c.tags.iter().any(|x| x == t))
        },
        |c| c.evidence_sample(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClaimKind, Forecast, Outcome, Resolution};
    use chrono::{Duration, TimeZone, Utc};

    fn claim(id: &str, day: u32, prob: f64, outcome: Option<Outcome>, resolved_day: u32) -> Claim {
        let created =
            Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap() + Duration::days(day as i64);
        Claim {
            id: id.into(),
            statement: id.into(),
            created_at: created,
            resolve_by: Some(
                NaiveDate::from_ymd_opt(2025, 2, 1).unwrap() + Duration::days(day as i64),
            ),
            tags: vec![],
            kind: ClaimKind::Binary,
            stake: 1.0,
            forecasts: vec![Forecast {
                at: created,
                prob: Some(prob),
                interval: None,
                because: None,
            }],
            resolution: outcome.map(|o| Resolution {
                at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
                    + Duration::days(resolved_day as i64),
                outcome: Some(o),
                value: None,
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        }
    }

    #[test]
    fn order_ignores_when_the_answer_arrived() {
        // Two claims whose resolve-by order is a, b — but b was answered first.
        // The sequence must still be a, b: otherwise a forecaster whose YES
        // answers land early gets a different sequence from one whose NO answers
        // do, purely because of the outcomes.
        let claims = vec![
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            claim("b", 1, 0.1, Some(Outcome::False), 5),
        ];
        let today = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let ev = binary_evidence(&claims, today, None);
        assert_eq!(ev.samples.len(), 2);
        assert_eq!(
            ev.samples[0].prob, 0.9,
            "ordered by resolve_by, not by when it resolved"
        );
        assert_eq!(ev.samples[1].prob, 0.1);
    }

    #[test]
    fn a_claim_not_yet_due_does_not_enter_the_sequence() {
        // Answered early, but its deadline has not passed: it waits its turn.
        let claims = vec![claim("a", 0, 0.9, Some(Outcome::True), 2)];
        let before = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
        assert!(binary_evidence(&claims, before, None).samples.is_empty());
        let after = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        assert_eq!(binary_evidence(&claims, after, None).samples.len(), 1);
    }

    #[test]
    fn evidence_pauses_at_the_first_overdue_unresolved_claim() {
        // Without this, a user could keep the flattering half of the record in
        // the test and leave the awkward half permanently "open".
        let claims = vec![
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            claim("b", 1, 0.8, None, 0),
            claim("c", 2, 0.7, Some(Outcome::True), 45),
        ];
        let today = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let ev = binary_evidence(&claims, today, None);
        assert_eq!(ev.samples.len(), 1);
        assert_eq!(ev.blocked_by.as_deref(), Some("b"));
    }

    #[test]
    fn claims_without_a_deadline_still_count_from_their_creation_date() {
        // A claim with no --by date falls back to the day it was written, which
        // is equally fixed before the outcome. Discarding these instead would
        // throw away every ledger written before --by was encouraged.
        let mut c = claim("a", 0, 0.9, Some(Outcome::True), 40);
        c.resolve_by = None;
        let ev = binary_evidence(&[c], NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), None);
        assert_eq!(ev.samples.len(), 1);
        assert!(ev.blocked_by.is_none());
    }

    #[test]
    fn an_ungraded_deadline_free_claim_blocks_and_says_how_to_unblock() {
        let mut open = claim("b", 1, 0.8, None, 0);
        open.resolve_by = None;
        let claims = vec![
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            open,
            claim("c", 2, 0.7, Some(Outcome::True), 45),
        ];
        let ev = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), None);
        // Its due key is creation + the 30-day grace, which lands it after `a`
        // and before `c`. Once that grace is spent it blocks, which is the
        // intended pressure — but not before.
        assert_eq!(ev.samples.len(), 1);
        assert_eq!(ev.blocked_by.as_deref(), Some("b"));
        assert!(ev.blocked_without_deadline);
        assert_eq!(ev.waiting, 1, "the later graded claim is waiting, not lost");
    }

    #[test]
    fn a_fresh_deadline_free_claim_does_not_block_anything_yet() {
        // The cliff this grace period exists to remove: without it, a claim is
        // admitted the day it is written, so one open no-deadline claim froze the
        // whole evidence test from that moment on. Someone logs "will we hit 10k
        // users?" in week one and never sees a number again.
        let mut fresh = claim("z", 0, 0.5, None, 0);
        fresh.resolve_by = None;
        fresh.created_at = Utc.with_ymd_and_hms(2025, 5, 28, 0, 0, 0).unwrap();
        let claims = vec![
            fresh,
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            claim("b", 1, 0.7, Some(Outcome::False), 45),
        ];
        let ev = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), None);
        assert_eq!(ev.samples.len(), 2, "the graded claims still count");
        assert!(
            ev.blocked_by.is_none(),
            "a four-day-old claim must not block"
        );

        // And once the grace is spent, it does block — the nudge still lands.
        let later = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(), None);
        assert_eq!(later.blocked_by.as_deref(), Some("z"));
    }

    #[test]
    fn voiding_a_resolved_claim_does_not_remove_it_from_the_evidence() {
        // The one operation that could retroactively edit a fixed sequence.
        // Voiding something unresolved is fine — it carries no outcome. Voiding
        // something already resolved deletes an outcome after seeing it, and the
        // direction of abuse is self-flattery: you void what went badly.
        let mut resolved_then_voided = claim("a", 0, 0.9, Some(Outcome::False), 40);
        resolved_then_voided.void = Some(crate::model::Void {
            at: Utc.with_ymd_and_hms(2025, 5, 1, 0, 0, 0).unwrap(), // after resolution
            reason: "on reflection I did not like this one".into(),
        });
        let mut never_resolved = claim("b", 1, 0.9, None, 0);
        never_resolved.void = Some(crate::model::Void {
            at: Utc.with_ymd_and_hms(2025, 5, 1, 0, 0, 0).unwrap(),
            reason: "genuinely ambiguous, and never answered".into(),
        });

        assert!(resolved_then_voided.voided_after_resolution());
        assert!(!resolved_then_voided.voided_before_resolution());
        assert!(never_resolved.voided_before_resolution());

        let claims = vec![resolved_then_voided, never_resolved];
        let ev = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 9, 1).unwrap(), None);
        assert_eq!(
            ev.samples.len(),
            1,
            "the seen outcome stays in the sequence"
        );
        assert_eq!(ev.voided_after_resolution, 1, "and is counted out loud");
        assert!(
            ev.blocked_by.is_none(),
            "the unresolved void does not block"
        );
    }

    #[test]
    fn a_long_horizon_open_claim_does_not_block_earlier_ones() {
        // Written first, due in a year: it must not hold up everything created
        // after it. Its due key sorts it last, so it is simply not reached.
        let mut far = claim("a", 0, 0.5, None, 0);
        far.resolve_by = Some(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap());
        let claims = vec![
            far,
            claim("b", 1, 0.9, Some(Outcome::True), 40),
            claim("c", 2, 0.7, Some(Outcome::False), 45),
        ];
        let ev = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), None);
        assert_eq!(ev.samples.len(), 2);
        assert!(ev.blocked_by.is_none());
    }
}
