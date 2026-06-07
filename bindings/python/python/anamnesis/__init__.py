"""Anamnesis — calibration & proper-scoring metrics, powered by a Rust core.

This is the Python face of the Anamnesis scoring engine. Every function delegates
to the *same compiled Rust code* that powers the ``ana`` CLI and its MCP server,
so the numbers can never drift between languages — there is one implementation,
cross-checked by the Rust unit tests, surfaced here.

What's here that ``sklearn``/``netcal`` don't bundle together:

* the **exact** Murphy decomposition ``brier = reliability - resolution +
  uncertainty`` (grouped by unique forecast value, not range-binned, so the
  identity is exact);
* the **Winkler interval score** + empirical **coverage** for numeric/credible
  intervals;
* a **Wilson** score interval and **empirical-Bayes shrinkage**, the small-sample
  tools you want when an eval has tens — not thousands — of datapoints.

No hard dependency on numpy: any iterable of numbers works (lists, tuples, numpy
arrays, pandas Series). ``outcomes`` may be ``0/1``, floats, or bools.

Quickstart
----------
>>> import anamnesis as ana
>>> probs    = [0.9, 0.8, 0.3, 0.6, 0.5]
>>> outcomes = [1,   1,   0,   1,   0]
>>> round(ana.brier(probs, outcomes), 4)
0.11
>>> d = ana.decompose(probs, outcomes)
>>> abs((d.reliability - d.resolution + d.uncertainty) - d.brier) < 1e-12
True
>>> ana.shrink_toward(1, 1, prior_mean=0.5, strength=4)   # one fluke ≠ certainty
0.6

For driving a *ledger* (logging predictions before the outcome and resolving them
later), use the ``ana`` CLI or its MCP server (``ana mcp``) — see the project
README. This package is the stateless math layer.
"""
from __future__ import annotations

from collections import namedtuple
from typing import List, Optional, Sequence, Tuple

from . import _core

__version__: str = _core.__version__

__all__ = [
    "Decomposition",
    "Overconfidence",
    "Bin",
    "brier",
    "brier_weighted",
    "log_score",
    "base_rate",
    "decompose",
    "skill_score",
    "auc",
    "overconfidence",
    "directional_bias",
    "calibration_curve",
    "winkler",
    "coverage",
    "conformal_width_factor",
    "wilson_interval",
    "shrink_toward",
    "calibration_eprocess",
    "eprocess_pvalue",
    "fit_recalibration",
    "Recalibration",
    "brier_ci_bootstrap",
    "ewma_brier",
    "distinct_forecasts",
    "RiskCoverage",
    "risk_coverage",
    "risk_coverage_curve",
    "dialectical_mean",
    "decide",
    "Decision",
    "report",
    "Calibration",
]

# ── structured results ───────────────────────────────────────────────────────
Decomposition = namedtuple("Decomposition", "reliability resolution uncertainty brier")
Overconfidence = namedtuple("Overconfidence", "mean_confidence accuracy gap")
Bin = namedtuple("Bin", "lo hi count mean_pred observed")


def _floats(xs: Sequence) -> List[float]:
    """Coerce any iterable of numbers/bools to a plain list[float]."""
    return [float(x) for x in xs]


# ── binary forecasts ─────────────────────────────────────────────────────────
def brier(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Mean Brier score (mean squared error). 0 is perfect; lower is better."""
    return _core.brier(_floats(probs), _floats(outcomes))


def brier_weighted(probs: Sequence, outcomes: Sequence, weights: Sequence) -> Optional[float]:
    """Stake-weighted Brier: each forecast scaled by how much it mattered. With
    equal weights this equals :func:`brier`. ``None`` for a length mismatch or no
    positive weight — surfaces whether you're miscalibrated on the calls that count."""
    return _core.brier_weighted(_floats(probs), _floats(outcomes), _floats(weights))


def log_score(probs: Sequence, outcomes: Sequence, eps: float = 1e-9) -> Optional[float]:
    """Mean log (Good) score. Punishes confident misses far harder than Brier."""
    return _core.log_score(_floats(probs), _floats(outcomes), eps)


def base_rate(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Fraction of events that actually happened — the climatology to beat."""
    return _core.base_rate(_floats(probs), _floats(outcomes))


def decompose(probs: Sequence, outcomes: Sequence) -> Optional[Decomposition]:
    """Murphy's exact reliability/resolution/uncertainty partition of Brier."""
    t = _core.decompose(_floats(probs), _floats(outcomes))
    return None if t is None else Decomposition(*t)


def skill_score(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Brier skill score ``1 - brier/uncertainty`` vs always predicting the base
    rate. >0 is skill; None when every outcome is identical."""
    return _core.skill_score(_floats(probs), _floats(outcomes))


def auc(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Discrimination (area under ROC). 0.5 = none, 1.0 = perfect separation.
    Independent of calibration. None when outcomes are all one class."""
    return _core.auc(_floats(probs), _floats(outcomes))


def overconfidence(probs: Sequence, outcomes: Sequence) -> Optional[Overconfidence]:
    """Lichtenstein–Fischhoff gap = mean confidence − accuracy. Positive ⇒
    overconfident (bolder than you were right)."""
    t = _core.overconfidence(_floats(probs), _floats(outcomes))
    return None if t is None else Overconfidence(*t)


def directional_bias(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Calibration-in-the-large: mean forecast − base rate."""
    return _core.directional_bias(_floats(probs), _floats(outcomes))


def calibration_curve(probs: Sequence, outcomes: Sequence, n_bins: int = 10) -> List[Bin]:
    """Reliability-diagram data: per equal-width bin, the mean predicted
    probability vs the observed frequency (``observed`` is None for empty bins)."""
    return [Bin(*row) for row in _core.calibration_curve(_floats(probs), _floats(outcomes), n_bins)]


# ── numeric / interval forecasts ─────────────────────────────────────────────
def winkler(low: float, high: float, level: float, value: float) -> float:
    """Winkler interval score for one credible interval ``[low, high]`` at
    ``level`` (e.g. 0.8) given the realized ``value``. Lower is better."""
    return _core.winkler(float(low), float(high), float(level), float(value))


def coverage(
    lows: Sequence, highs: Sequence, levels: Sequence, values: Sequence
) -> Optional[float]:
    """Fraction of intervals that actually contained their value — compare with
    the nominal level to see interval over/under-confidence."""
    return _core.coverage(_floats(lows), _floats(highs), _floats(levels), _floats(values))


def conformal_width_factor(
    lows: Sequence, highs: Sequence, levels: Sequence, values: Sequence
) -> Optional[float]:
    """Conformal width multiplier — multiply your interval half-widths by this to
    hit your nominal coverage (``>1`` widen, ``<1`` sharpen); the numeric analogue
    of the recalibration map. ``None`` below three usable intervals."""
    return _core.conformal_width_factor(
        _floats(lows), _floats(highs), _floats(levels), _floats(values)
    )


# ── small-sample uncertainty ─────────────────────────────────────────────────
def wilson_interval(
    successes: float, n: int, z: float = 1.959963984540054
) -> Optional[Tuple[float, float]]:
    """Wilson score interval for a binomial rate ``successes/n``. Stays in [0,1]
    and behaves at small n, unlike the normal (Wald) approximation. z≈1.96 = 95%."""
    return _core.wilson_interval(float(successes), int(n), float(z))


def shrink_toward(successes: float, n: int, prior_mean: float, strength: float) -> float:
    """Empirical-Bayes shrinkage of ``successes/n`` toward ``prior_mean``:
    ``(successes + strength·prior_mean)/(n + strength)``. Keeps one fluky result
    from dominating a small-n rate."""
    return _core.shrink_toward(float(successes), int(n), float(prior_mean), float(strength))


# ── anytime-valid calibration test & recalibration map ───────────────────────
def calibration_eprocess(probs: Sequence, outcomes: Sequence) -> Optional[float]:
    """Anytime-valid calibration **e-value** (betting / test-martingale). Evidence
    that you are *mis*calibrated, valid no matter how often you check it: ``≈1`` is
    none, ``≥20`` is significant at α=0.05. Pass samples chronologically (the order
    outcomes were learned)."""
    return _core.calibration_eprocess(_floats(probs), _floats(outcomes))


def eprocess_pvalue(e: float) -> float:
    """Anytime-valid p-value from an e-value, via the ``1/e`` calibrator."""
    return _core.eprocess_pvalue(float(e))


class Recalibration:
    """A learned correction ``p ↦ σ(a + b·logit p)``. ``a`` is log-odds bias;
    ``b`` is slope (``<1`` too extreme, ``>1`` too timid). Build it with
    :func:`fit_recalibration` and apply it with :meth:`apply`."""

    def __init__(self, a: float, b: float, n: int):
        self.a, self.b, self.n = float(a), float(b), int(n)

    def apply(self, p: float) -> float:
        """Correct a single stated probability through the map."""
        return _core.recalibration_apply(self.a, self.b, float(p))

    def __repr__(self) -> str:
        return f"Recalibration(a={self.a:.3f}, b={self.b:.3f}, n={self.n})"


def fit_recalibration(
    probs: Sequence, outcomes: Sequence, ridge: float = 1.5
) -> Optional["Recalibration"]:
    """Fit a ridge-shrunk logistic recalibration map from resolved calls. The
    ridge pulls it toward the identity at small n (you must *earn* a correction).
    Returns ``None`` for an empty record."""
    t = _core.fit_recalibration(_floats(probs), _floats(outcomes), float(ridge))
    return None if t is None else Recalibration(*t)


# ── small-sample / over-time bands ───────────────────────────────────────────
def brier_ci_bootstrap(
    probs: Sequence,
    outcomes: Sequence,
    level: float = 0.95,
    resamples: int = 2000,
    seed: int = 0xA11A5EEDC0FFEE00,
) -> Optional[Tuple[float, float]]:
    """Bootstrap percentile band on the Brier score — how far luck alone could
    move it. Deterministic given ``seed`` (reproducible). ``None`` for < 2 samples.
    A rough band; the rigorous calibration call is :func:`calibration_eprocess`."""
    return _core.brier_ci_bootstrap(
        _floats(probs), _floats(outcomes), float(level), int(resamples), int(seed)
    )


def ewma_brier(probs: Sequence, outcomes: Sequence, half_life: float = 5.0) -> Optional[float]:
    """Recency-weighted (EWMA) Brier — "how am I doing *lately*", read against the
    lifetime :func:`brier`. Pass samples chronologically. Descriptive trend, not a
    significance test. ``None`` for an empty record."""
    return _core.ewma_brier(_floats(probs), _floats(outcomes), float(half_life))


def distinct_forecasts(probs: Sequence) -> int:
    """Number of distinct forecast probabilities used — a coarse confidence
    vocabulary caps the resolution you can achieve."""
    return _core.distinct_forecasts(_floats(probs))


# ── selective prediction ─────────────────────────────────────────────────────
RiskCoverage = namedtuple("RiskCoverage", "risk_full risk_half aurcc")


def risk_coverage(probs: Sequence, outcomes: Sequence) -> Optional[RiskCoverage]:
    """Selective-prediction summary: directional error acting on every call
    (``risk_full``), on your most-confident half (``risk_half``), and the area
    under the risk–coverage curve (``aurcc``, lower ⇒ your confidence ranks your
    calls). ``None`` for an empty record."""
    t = _core.risk_coverage_summary(_floats(probs), _floats(outcomes))
    return None if t is None else RiskCoverage(*t)


def risk_coverage_curve(probs: Sequence, outcomes: Sequence) -> List[Tuple[float, float]]:
    """The full risk–coverage curve as ``(coverage, risk)`` points (coverage
    ascending) — the data behind a selective-prediction plot."""
    return _core.risk_coverage_curve(_floats(probs), _floats(outcomes))


# ── elicitation aid ──────────────────────────────────────────────────────────
def dialectical_mean(p1: float, p2: float) -> float:
    """Herzog–Hertwig "crowd within": average a first estimate with a deliberate
    "consider the opposite" second one. Recovers ~half the gain of a second person."""
    return _core.dialectical_mean(float(p1), float(p2))


# ── decision gate ────────────────────────────────────────────────────────────
Decision = namedtuple("Decision", "act adjusted_p proceed_threshold margin")


def decide(p: float, stake: float = 1.0, verify_cost: float = 0.2, recal=None) -> Decision:
    """Should you act on a stated probability ``p``? Corrects it through an optional
    earned :class:`Recalibration` ``recal`` (verbalized confidence is unreliable),
    then applies Chow's stake-aware threshold. Returns a :class:`Decision` whose
    ``act`` is ``"proceed"``, ``"verify"`` (check first), or ``"abstain"`` (replan).
    Raise ``stake`` for consequential or irreversible calls — the bar to proceed,
    ``1 − verify_cost/stake``, climbs with it. Pass ``recal`` only once a correction
    has been earned (real e-process evidence)."""
    a = recal.a if recal is not None else None
    b = recal.b if recal is not None else None
    t = _core.decide(float(p), float(stake), float(verify_cost), a, b)
    return Decision(*t)


# ── convenience ──────────────────────────────────────────────────────────────
def report(probs: Sequence, outcomes: Sequence) -> dict:
    """Compute every binary metric at once and return a plain dict — handy in a
    notebook. Mirrors the headline numbers of ``ana report``."""
    p, o = _floats(probs), _floats(outcomes)
    d = decompose(p, o)
    oc = overconfidence(p, o)
    n = len(p)
    br = base_rate(p, o)
    base_ci = None
    if br is not None and n:
        base_ci = wilson_interval(br * n, n)
    return {
        "n": n,
        "brier": brier(p, o),
        "log_score": log_score(p, o),
        "skill_score": skill_score(p, o),
        "auc": auc(p, o),
        "base_rate": br,
        "base_rate_ci95": base_ci,
        "directional_bias": directional_bias(p, o),
        "reliability": None if d is None else d.reliability,
        "resolution": None if d is None else d.resolution,
        "uncertainty": None if d is None else d.uncertainty,
        "mean_confidence": None if oc is None else oc.mean_confidence,
        "accuracy": None if oc is None else oc.accuracy,
        "confidence_gap": None if oc is None else oc.gap,
        "calibration_eprocess": calibration_eprocess(p, o),
        "brier_ci": brier_ci_bootstrap(p, o),
        "recent_brier": ewma_brier(p, o),
        "distinct_forecasts": distinct_forecasts(p),
        "selective": (lambda s: None if s is None else s._asdict())(risk_coverage(p, o)),
    }


class Calibration:
    """A tiny stateful convenience over a single set of (probs, outcomes).

    >>> c = Calibration([0.9, 0.2, 0.8], [1, 0, 1])
    >>> c.brier() is not None
    True
    >>> "overconfident" in repr(c) or "underconfident" in repr(c) or "calibrated" in repr(c)
    True
    """

    def __init__(self, probs: Sequence, outcomes: Sequence):
        self.probs = _floats(probs)
        self.outcomes = _floats(outcomes)

    def brier(self) -> Optional[float]:
        return brier(self.probs, self.outcomes)

    def auc(self) -> Optional[float]:
        return auc(self.probs, self.outcomes)

    def decompose(self) -> Optional[Decomposition]:
        return decompose(self.probs, self.outcomes)

    def overconfidence(self) -> Optional[Overconfidence]:
        return overconfidence(self.probs, self.outcomes)

    def summary(self) -> dict:
        return report(self.probs, self.outcomes)

    def __len__(self) -> int:
        return len(self.probs)

    def __repr__(self) -> str:
        oc = self.overconfidence()
        if oc is None:
            return "Calibration(empty)"
        gap = oc.gap
        word = "calibrated" if abs(gap) < 0.05 else ("overconfident" if gap > 0 else "underconfident")
        return (
            f"Calibration(n={len(self)}, brier={self.brier():.3f}, "
            f"{word} {gap * 100:+.0f}pts)"
        )
