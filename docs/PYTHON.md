# Using the engine from Python

## Use the engine from Python

The scoring core ships as a Python package via a [PyO3](https://pyo3.rs) +
[maturin](https://www.maturin.rs) binding ([bindings/python/](bindings/python/)) —
one `abi3` wheel for CPython 3.8+. It calls the **same compiled Rust** as the CLI,
so the numbers never drift between languages; there is a single implementation,
cross-checked by the Rust tests.

```python
import anamnesis as ana
probs, outcomes = [0.9, 0.8, 0.3, 0.6, 0.5], [1, 1, 0, 1, 0]
ana.brier(probs, outcomes)                      # 0.11
d = ana.decompose(probs, outcomes)              # exact Murphy partition (namedtuple)
ana.shrink_toward(1, 1, prior_mean=0.5, strength=4)   # 0.6 — one fluke ≠ certainty
ana.report(probs, outcomes)                     # every metric as a dict
```

Lists, tuples, numpy arrays, or pandas Series all work (numpy is *not* a
dependency). This is the stateless math layer; to *keep a ledger* from Python, an
agent framework can drive the `ana mcp` server — LangChain/LangGraph adapt it with
[`langchain-mcp-adapters`](https://github.com/langchain-ai/langchain-mcp-adapters),
no bespoke binding required ([example](bindings/python/examples/langgraph_mcp.py)).

See [bindings/python/README.md](../bindings/python/README.md) for the full API.
