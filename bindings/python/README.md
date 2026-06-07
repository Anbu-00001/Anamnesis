# anamnesis (Python)

The calibration & proper-scoring core of [Anamnesis](https://github.com/Anbu-00001/Anamnesis),
exposed to Python through a small [PyO3](https://pyo3.rs) + [maturin](https://www.maturin.rs)
binding. Every function calls the **same compiled Rust code** as the `ana` CLI and
its MCP server — so the numbers never drift between languages. There is one
implementation, cross-checked by the Rust unit tests, surfaced here.

## Why this exists

`sklearn` gives you `brier_score_loss`; `netcal` gives you calibration error. None
of the mainstream Python libraries bundle, in one place and with the
calibration-vs-discrimination framing:

- the **exact** Murphy decomposition `brier = reliability − resolution + uncertainty`
  (grouped by *unique forecast value*, not range-binned — so the identity is exact, not approximate);
- the **Winkler interval score** + empirical **coverage** for numeric/credible intervals;
- a **Wilson** score interval and **empirical-Bayes shrinkage** — the small-sample
  tools you actually want when an eval has *tens*, not thousands, of datapoints.

## Install

```bash
pip install anamnesis            # wheel bundles the Rust core; no toolchain needed
```

From source (needs a Rust toolchain):

```bash
pip install maturin
cd bindings/python
maturin develop --release        # builds + installs into the active venv
```

## Use

```python
import anamnesis as ana

probs    = [0.9, 0.8, 0.3, 0.6, 0.5]
outcomes = [1,   1,   0,   1,   0]      # 0/1, floats, or bools all work

ana.brier(probs, outcomes)              # 0.11
ana.auc(probs, outcomes)                # discrimination, blind to calibration

d = ana.decompose(probs, outcomes)
assert abs((d.reliability - d.resolution + d.uncertainty) - d.brier) < 1e-12

ana.overconfidence(probs, outcomes).gap # +ve ⇒ overconfident
ana.report(probs, outcomes)             # every metric as a dict (notebook-friendly)

# small-sample honesty
ana.wilson_interval(8, 10)              # 95% CI that stays in [0, 1]
ana.shrink_toward(1, 1, prior_mean=0.5, strength=4)   # 0.6 — one fluke ≠ certainty

# numeric / credible intervals
ana.winkler(low=0, high=10, level=0.8, value=5)       # interval score
ana.coverage([0,0,0], [10,10,10], [0.8]*3, [5,50,5])  # 2/3
```

Any iterable of numbers works — lists, tuples, numpy arrays, pandas Series. numpy
is **not** a dependency.

## The stateless math layer vs. the ledger

This package is the **stateless** scoring engine. To *log* predictions before the
outcome and *resolve* them later (the discipline that makes calibration real), use
the `ana` CLI or its MCP server:

```bash
ana predict "tests pass first try" --prob 0.7 --tag who:me
ana resolve <id> yes
ana report
```

### LangChain / LangGraph

There is no separate LangChain binding to install — the `ana mcp` server is an MCP
server, and [`langchain-mcp-adapters`](https://github.com/langchain-ai/langchain-mcp-adapters)
adapts it into LangGraph tools automatically. See
[`examples/langgraph_mcp.py`](examples/langgraph_mcp.py).

## Develop / test

```bash
cd bindings/python
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest numpy
maturin develop
pytest -q
```

## License

MIT, same as the parent project.
