//! PyO3 bindings for the Anamnesis scoring core.
//!
//! Every function here is a *thin* delegate to `anamnesis::scoring` — the exact
//! same Rust code the `ana` CLI and the MCP server run. The numbers therefore
//! cannot drift between the Rust and Python faces of the project: there is one
//! implementation, cross-checked by the Rust unit tests, surfaced twice. The
//! Python side (see `python/anamnesis/__init__.py`) adds the ergonomics —
//! namedtuples, optional numpy input, a `report()` summary — on top of these
//! primitives, which deliberately take plain parallel sequences of floats and
//! return plain tuples so the binding stays dependency-light.

use anamnesis::scoring::{self, NumericSample, Sample};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Build binary [`Sample`]s from parallel probability/outcome sequences,
/// validating lengths and the `[0, 1]` probability domain. Any nonzero outcome
/// counts as "happened" (so `1`, `1.0`, and `True` all work from Python).
fn binary(probs: &[f64], outcomes: &[f64]) -> PyResult<Vec<Sample>> {
    if probs.len() != outcomes.len() {
        return Err(PyValueError::new_err(format!(
            "probs and outcomes must have equal length ({} vs {})",
            probs.len(),
            outcomes.len()
        )));
    }
    for &p in probs {
        if !(0.0..=1.0).contains(&p) || p.is_nan() {
            return Err(PyValueError::new_err(format!(
                "probability {p} is outside [0, 1]"
            )));
        }
    }
    Ok(probs
        .iter()
        .zip(outcomes)
        .map(|(&p, &o)| Sample::new(p, o != 0.0))
        .collect())
}

/// Mean Brier score (mean squared error of probabilistic forecasts). `None` for
/// an empty input.
#[pyfunction]
fn brier(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<f64>> {
    Ok(scoring::brier(&binary(&probs, &outcomes)?))
}

/// Mean logarithmic (Good) score. Probabilities are clamped into `[eps, 1-eps]`
/// so a single confident miss cannot return `+inf`.
#[pyfunction]
#[pyo3(signature = (probs, outcomes, eps=1e-9))]
fn log_score(probs: Vec<f64>, outcomes: Vec<f64>, eps: f64) -> PyResult<Option<f64>> {
    Ok(scoring::log_score(&binary(&probs, &outcomes)?, eps))
}

/// The base rate: fraction of events that actually happened.
#[pyfunction]
fn base_rate(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<f64>> {
    Ok(scoring::base_rate(&binary(&probs, &outcomes)?))
}

/// Murphy's exact decomposition as `(reliability, resolution, uncertainty,
/// brier)`. The identity `reliability - resolution + uncertainty == brier` holds
/// to floating-point precision.
#[pyfunction]
fn decompose(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<(f64, f64, f64, f64)>> {
    Ok(scoring::decompose(&binary(&probs, &outcomes)?)
        .map(|d| (d.reliability, d.resolution, d.uncertainty, d.brier)))
}

/// Brier skill score `1 - brier/uncertainty`. `None` when every outcome is the
/// same (uncertainty `0`).
#[pyfunction]
fn skill_score(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<f64>> {
    Ok(scoring::skill_score(&binary(&probs, &outcomes)?))
}

/// Discrimination as area under the ROC curve. `None` when outcomes are all the
/// same class (AUC undefined).
#[pyfunction]
fn auc(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<f64>> {
    Ok(scoring::auc(&binary(&probs, &outcomes)?))
}

/// Lichtenstein–Fischhoff over/under-confidence as `(mean_confidence, accuracy,
/// gap)`. Positive `gap` ⇒ overconfident.
#[pyfunction]
fn overconfidence(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<(f64, f64, f64)>> {
    Ok(scoring::overconfidence(&binary(&probs, &outcomes)?)
        .map(|o| (o.mean_confidence, o.accuracy, o.gap)))
}

/// Calibration-in-the-large: mean forecast minus base rate (directional bias).
#[pyfunction]
fn directional_bias(probs: Vec<f64>, outcomes: Vec<f64>) -> PyResult<Option<f64>> {
    Ok(scoring::directional_bias(&binary(&probs, &outcomes)?))
}

/// One reliability-diagram bin as returned to Python: `(lo, hi, count,
/// mean_pred, observed)`, where `observed` is `None` for an empty bin.
type BinRow = (f64, f64, usize, f64, Option<f64>);

/// Reliability-diagram data: per equal-width bin, `(lo, hi, count, mean_pred,
/// observed)`. `observed` is `None` for an empty bin (instead of NaN).
#[pyfunction]
#[pyo3(signature = (probs, outcomes, n_bins=10))]
fn calibration_curve(probs: Vec<f64>, outcomes: Vec<f64>, n_bins: usize) -> PyResult<Vec<BinRow>> {
    let s = binary(&probs, &outcomes)?;
    Ok(scoring::calibration_curve(&s, n_bins)
        .into_iter()
        .map(|b| {
            let observed = if b.observed.is_nan() {
                None
            } else {
                Some(b.observed)
            };
            (b.lo, b.hi, b.count, b.mean_pred, observed)
        })
        .collect())
}

/// Winkler interval score for one numeric forecast `[low, high]` stated at
/// `level` (e.g. `0.8`), given the realized `value`. Lower is better.
#[pyfunction]
fn winkler(low: f64, high: f64, level: f64, value: f64) -> f64 {
    scoring::winkler(&NumericSample {
        low,
        high,
        level,
        value,
    })
}

/// Empirical coverage: fraction of intervals that contained their value.
#[pyfunction]
fn coverage(
    lows: Vec<f64>,
    highs: Vec<f64>,
    levels: Vec<f64>,
    values: Vec<f64>,
) -> PyResult<Option<f64>> {
    let n = lows.len();
    if highs.len() != n || levels.len() != n || values.len() != n {
        return Err(PyValueError::new_err(format!(
            "lows, highs, levels, values must have equal length ({}, {}, {}, {})",
            lows.len(),
            highs.len(),
            levels.len(),
            values.len()
        )));
    }
    let samples: Vec<NumericSample> = (0..n)
        .map(|i| NumericSample {
            low: lows[i],
            high: highs[i],
            level: levels[i],
            value: values[i],
        })
        .collect();
    Ok(scoring::coverage(&samples))
}

/// Wilson score interval for a binomial rate `successes/n`. `z` defaults to the
/// 95% standard-normal quantile. `None` for `n == 0`.
#[pyfunction]
#[pyo3(signature = (successes, n, z=1.959963984540054))]
fn wilson_interval(successes: f64, n: usize, z: f64) -> Option<(f64, f64)> {
    scoring::wilson_interval(successes, n, z)
}

/// Empirical-Bayes shrinkage of `successes/n` toward `prior_mean` with the given
/// `strength` (pseudo-observations).
#[pyfunction]
fn shrink_toward(successes: f64, n: usize, prior_mean: f64, strength: f64) -> f64 {
    scoring::shrink_toward(successes, n, prior_mean, strength)
}

/// The compiled extension module `anamnesis._core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(brier, m)?)?;
    m.add_function(wrap_pyfunction!(log_score, m)?)?;
    m.add_function(wrap_pyfunction!(base_rate, m)?)?;
    m.add_function(wrap_pyfunction!(decompose, m)?)?;
    m.add_function(wrap_pyfunction!(skill_score, m)?)?;
    m.add_function(wrap_pyfunction!(auc, m)?)?;
    m.add_function(wrap_pyfunction!(overconfidence, m)?)?;
    m.add_function(wrap_pyfunction!(directional_bias, m)?)?;
    m.add_function(wrap_pyfunction!(calibration_curve, m)?)?;
    m.add_function(wrap_pyfunction!(winkler, m)?)?;
    m.add_function(wrap_pyfunction!(coverage, m)?)?;
    m.add_function(wrap_pyfunction!(wilson_interval, m)?)?;
    m.add_function(wrap_pyfunction!(shrink_toward, m)?)?;
    Ok(())
}
