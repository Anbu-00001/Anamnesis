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
    "wilson_interval",
    "shrink_toward",
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
