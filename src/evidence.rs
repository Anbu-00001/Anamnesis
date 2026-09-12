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
//! claim is created — its `resolve_by` date, or its creation date plus a horizon
//! stored on the claim when it has none; admit a claim once that key date has
//! passed; and **price** a claim that is due but still ungraded at the worst
//! factor it could possibly have contributed, rather than stopping there.
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
//! Gap-pricing extends that argument by one sentence. The two factors an ungraded
//! claim could contribute average to exactly `1` under the null, so their minimum
//! is `≤ 1` and is `≤` the true factor whichever outcome it would have had. The
//! gap-filled wealth is therefore pointwise `≤` the fully-graded martingale at
//! every `n`, and is itself a non-negative **supermartingale** starting at `1` —
//! which Ville also covers. Across repeated views, whatever is ungraded at view
//! time `t` still satisfies `W'(t) ≤ M_n` for one fixed process. See
//! [`crate::scoring::calibration_log_eprocess_seq`].
//!
//! # Why it no longer stops
//!
//! Stopping was a correct answer to the wrong question. Any prefix rule yields
//! about `(1−g)/g` usable claims for an ungraded rate `g`, so a ledger that is
//! 35% ungraded gets a **two-claim** sequence however much it holds — measured on
//! a real 426-claim ledger, 23 of 309 graded calls counted. Partitioning does not
//! rescue it: `K` partitions give `K` sequences that are each just as short, and
//! the `K`-fold mixture penalty cancels the gain. Monthly partitions on that same
//! ledger would have given ~7 usable claims, worse than the 23.
//!
//! Pricing the gap instead keeps every graded call in the test (236 of 309 on
//! that ledger) and turns discipline from a wall into a cost that is reported.
//! An undisciplined user's e-value drifts down, so gaps can *hide* miscalibration
//! — they could already do that by freezing the test under the old rule, and
//! neither direction can manufacture a false alarm.
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
use crate::scoring::{Sample, Step};

/// The evidence sequence, and what it cost to build.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    /// The graded samples, in an order fixed before any outcome was known. Used
    /// by everything that needs outcomes (CORP, the recalibration fit).
    pub samples: Vec<Sample>,
    /// The full due sequence including gaps — what the e-process consumes. An
    /// ungraded claim is priced at the worst factor it could have contributed
    /// (see [`crate::scoring::calibration_log_eprocess_seq`]) instead of halting
    /// the sequence.
    pub steps: Vec<Step>,
    /// The oldest due-but-ungraded claim: the one to grade first. It no longer
    /// blocks anything — it is named because clearing it is what buys wealth back.
    pub oldest_gap: Option<String>,
    /// Whether `oldest_gap` carries a `resolve_by` date. When it does not, giving
    /// it one is the other way to move it.
    pub oldest_gap_without_deadline: bool,
    /// Claims voided *after* they had already resolved. They remain in the
    /// sequence — removing a seen outcome would retroactively edit it — and this
    /// count is surfaced so the edit is visible rather than silent.
    pub voided_after_resolution: usize,
    /// Every due-but-ungraded claim in scope. Each one is priced into the
    /// e-value, so this is a cost, not a queue.
    pub ungraded_due: usize,
}

/// How long a claim with no `resolve_by` gets before it is treated as due.
///
/// Without a grace period the rule has a cliff: a claim is admitted the day it is
/// written, so an open one starts costing wealth immediately.
///
/// This was **30 days** while an ungraded claim halted the sequence, because
/// admitting one early was catastrophic — it froze the test. Now that a gap is
/// priced rather than fatal, admitting one early costs a little wealth and
/// nothing more, so the horizon can be short enough to be useful. Measured on a
/// real agent ledger of 426 claims, resolution latency was **median 6.6 minutes,
/// p90 5 hours, p99 3.3 days**; a 30-day horizon held 130 already-resolved claims
/// out of the sequence for nothing.
pub const DEFAULT_HORIZON_DAYS: i64 = 7;

/// The horizon for a claim being created now, from its `kind:` tag.
///
/// "When is an answer fair to expect" depends on the sort of call: a
/// `kind:tests-pass` claim is answered by the next command, a `kind:approach`
/// claim by the end of the task. Claims with no `kind:` tag get the default —
/// which is most of them in practice (363 of 422 in the ledger measured above),
/// so this refines the default rather than replacing it.
///
/// The result is stored on the claim, so it is fixed before the outcome and
/// visible in the file. Computing it at read time from a global meant the
/// evidence order could shift under an existing ledger when a default changed.
pub fn horizon_for(tags: &[String]) -> i64 {
    tags.iter()
        .find_map(|t| t.strip_prefix("kind:"))
        .map(|kind| match kind {
            "tests-pass" | "bug-hypothesis" | "measurement" => 1,
            "estimate" | "approach" | "compat" | "refactor" => 3,
            _ => horizon_days(),
        })
        .unwrap_or_else(horizon_days)
}

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
/// Its `resolve_by` when it has one; otherwise its creation date plus the horizon
/// **stored on the claim**, falling back to the current default for ledgers
/// written before horizons were recorded. All of these are fixed before the
/// outcome, which is the only property the guarantee needs.
fn due_key(c: &Claim) -> NaiveDate {
    c.resolve_by.unwrap_or_else(|| {
        let days = c.horizon_days.unwrap_or_else(horizon_days);
        c.created_at.date_naive() + chrono::Duration::days(days)
    })
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
    let mut steps = Vec::with_capacity(due.len());
    let mut oldest_gap = None;
    let mut oldest_gap_without_deadline = false;
    let mut ungraded_due = 0;

    for c in &due {
        match to_sample(c) {
            Some(s) => {
                steps.push(Step::graded(s.prob, s.outcome));
                samples.push(s);
            }
            None => {
                // Do not stop, and do not skip. Skipping an ungraded claim is what
                // would let the record be chosen after the fact; stopping hands a
                // 35%-ungraded ledger a two-claim sequence forever. Price it
                // instead, at the worst factor it could have contributed.
                if let Some(p) = c.first_prob() {
                    steps.push(Step::gap(p));
                }
                if oldest_gap.is_none() {
                    oldest_gap = Some(c.id.clone());
                    oldest_gap_without_deadline = c.resolve_by.is_none();
                }
                ungraded_due += 1;
            }
        }
    }

    Evidence {
        samples,
        steps,
        oldest_gap,
        oldest_gap_without_deadline,
        voided_after_resolution,
        ungraded_due,
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

/// The graded claims the evidence sequence scores, in order — the same ones
/// [`binary_evidence`] turns into samples, as claims rather than samples.
pub fn binary_evidence_claims<'a>(
    claims: &'a [Claim],
    today: NaiveDate,
    tag: Option<&str>,
) -> Vec<&'a Claim> {
    // Every due claim is in the sequence now: graded ones by their outcome,
    // ungraded ones priced at their worst case. Nothing is truncated away.
    binary_candidates(claims, today, tag)
        .into_iter()
        .filter(|c| c.evidence_sample().is_some())
        .collect()
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
            horizon_days: None,
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
    fn an_overdue_unresolved_claim_is_priced_in_not_skipped() {
        // Without this, a user could keep the flattering half of the record in
        // the test and leave the awkward half permanently "open". Skipping is
        // what would allow that; halting prevented it at the cost of the test.
        // Pricing the gap at its worst case prevents it AND keeps the test.
        let claims = vec![
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            claim("b", 1, 0.8, None, 0),
            claim("c", 2, 0.7, Some(Outcome::True), 45),
        ];
        let today = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let ev = binary_evidence(&claims, today, None);
        assert_eq!(ev.samples.len(), 2, "both graded claims count now");
        assert_eq!(ev.steps.len(), 3, "and the gap is in the sequence");
        assert_eq!(ev.steps[1].outcome, None, "as a gap, in its fixed position");
        assert_eq!(ev.oldest_gap.as_deref(), Some("b"));
        assert_eq!(ev.ungraded_due, 1);

        // The gap costs wealth: the same record with `b` graded scores higher.
        let mut graded = claims.clone();
        graded[1] = claim("b", 1, 0.8, Some(Outcome::True), 41);
        let with_gap = crate::scoring::calibration_log_eprocess_seq(&ev.steps).unwrap();
        let full = crate::scoring::calibration_log_eprocess_seq(
            &binary_evidence(&graded, today, None).steps,
        )
        .unwrap();
        assert!(with_gap < full, "an ungraded claim must cost, not be free");
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
        assert!(ev.oldest_gap.is_none());
    }

    #[test]
    fn an_ungraded_deadline_free_claim_is_named_and_says_how_to_clear_it() {
        let mut open = claim("b", 1, 0.8, None, 0);
        open.resolve_by = None;
        let claims = vec![
            claim("a", 0, 0.9, Some(Outcome::True), 40),
            open,
            claim("c", 2, 0.7, Some(Outcome::True), 45),
        ];
        let ev = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), None);
        // Its due key is creation + the grace horizon, which lands it after `a`
        // and before `c`. Once that grace is spent it starts costing wealth,
        // which is the intended pressure — but not before.
        assert_eq!(ev.samples.len(), 2, "the later graded claim still counts");
        assert_eq!(ev.oldest_gap.as_deref(), Some("b"));
        assert!(ev.oldest_gap_without_deadline);
        assert_eq!(ev.ungraded_due, 1);
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
            ev.oldest_gap.is_none(),
            "a four-day-old claim is not yet overdue"
        );

        // And once the grace is spent it is priced in — the nudge still lands.
        let later = binary_evidence(&claims, NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(), None);
        assert_eq!(later.oldest_gap.as_deref(), Some("z"));
        assert_eq!(later.ungraded_due, 1);
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
            ev.oldest_gap.is_none(),
            "the unresolved void leaves no gap to price — it is gone, not pending"
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
        assert!(ev.oldest_gap.is_none(), "not due yet, so not a gap yet");
    }

    #[test]
    fn the_horizon_is_per_kind_and_stored_on_the_claim() {
        // "When is an answer fair to expect" depends on the sort of call.
        assert_eq!(horizon_for(&["kind:tests-pass".into()]), 1);
        assert_eq!(horizon_for(&["kind:approach".into()]), 3);
        assert_eq!(horizon_for(&["who:claude".into()]), DEFAULT_HORIZON_DAYS);
        assert_eq!(horizon_for(&[]), DEFAULT_HORIZON_DAYS);

        // A stored horizon wins over the global default, which is the point of
        // storing it: an existing ledger's evidence order cannot shift when the
        // default changes under it.
        let mut fast = claim("f", 0, 0.9, None, 0);
        fast.resolve_by = None;
        fast.horizon_days = Some(1);
        let mut slow = claim("s", 0, 0.9, None, 0);
        slow.resolve_by = None;
        slow.horizon_days = Some(365);

        // Two days after creation: the 1-day claim is due (and priced), the
        // 365-day one is not yet due at all.
        let today = NaiveDate::from_ymd_opt(2025, 1, 3).unwrap();
        let ev = binary_evidence(&[fast, slow], today, None);
        assert_eq!(ev.ungraded_due, 1, "only the short-horizon claim is due");
        assert_eq!(ev.oldest_gap.as_deref(), Some("f"));
    }

    #[test]
    fn a_ledger_written_before_horizons_still_loads_and_orders() {
        // Backward compatibility: no stored horizon falls back to the current
        // default, so old files keep working rather than dropping out.
        let mut old = claim("legacy", 0, 0.9, Some(Outcome::True), 1);
        old.resolve_by = None;
        old.horizon_days = None;
        let before = binary_evidence(
            &[old.clone()],
            NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
            None,
        );
        assert!(before.samples.is_empty(), "not due one day in");
        let after = binary_evidence(&[old], NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(), None);
        assert_eq!(
            after.samples.len(),
            1,
            "due once the default horizon passes"
        );
    }

    #[test]
    fn a_backlog_shrinks_the_evidence_it_can_never_inflate_it() {
        // The validity property, at the module level: whatever the ungraded
        // claims would have been, pricing them cannot push the e-value up. An
        // undisciplined user can therefore hide miscalibration, exactly as they
        // could by freezing the test under the old rule — but cannot invent it.
        let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let mut all_graded = Vec::new();
        for i in 0..40u32 {
            let o = if i % 3 == 0 {
                Outcome::False
            } else {
                Outcome::True
            };
            all_graded.push(claim(&format!("c{i:02}"), i, 0.8, Some(o), 40 + i));
        }
        let full = crate::scoring::calibration_log_eprocess_seq(
            &binary_evidence(&all_graded, today, None).steps,
        )
        .unwrap();

        // Now leave every fourth one ungraded.
        let mut with_gaps = all_graded.clone();
        for (i, c) in with_gaps.iter_mut().enumerate() {
            if i % 4 == 3 {
                c.resolution = None;
            }
        }
        let ev = binary_evidence(&with_gaps, today, None);
        let gapped = crate::scoring::calibration_log_eprocess_seq(&ev.steps).unwrap();
        assert_eq!(ev.ungraded_due, 10);
        assert!(
            gapped <= full + 1e-12,
            "gaps must not raise the e-value: {gapped} vs {full}"
        );
    }
}
