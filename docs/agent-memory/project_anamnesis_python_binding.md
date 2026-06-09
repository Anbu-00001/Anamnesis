---
name: project-anamnesis-python-binding
description: Anamnesis has a Python binding (PyO3/maturin); LangChain glue was deliberately rejected in favor of MCP.
metadata: 
  node_type: memory
  type: project
  originSessionId: 434d3d50-1c5a-4981-acec-430b13856106
---

Anamnesis (`/home/anbu/26_class/PlayGround/anamnesis`) gained a **Python binding**
on 2026-06-07: [bindings/python/](../../../../26_class/PlayGround/anamnesis/bindings/python)
is a PyO3 + maturin crate building one `abi3` wheel (CPython 3.8+) that exposes the
pure `scoring.rs` core as `import anamnesis`. It's a *standalone* crate (own empty
`[workspace]`, depends on the core lib by path) so the core's `cargo build/clippy/test`
never pulls in pyo3. Thin delegates only — one implementation, two languages, zero drift.
Build/verify: `cd bindings/python && maturin develop && pytest` (16 tests). Verified
green alongside the core's 30 unit + 1 integration test.

**Why:** the user asked me to "start the Python/LangChain binding" but to first decide
whether it was actually useful. I concluded a bespoke **LangChain binding is wasted
work** — the existing `ana mcp` MCP server is already consumed by LangChain/LangGraph
via `langchain-mcp-adapters` (`MultiServerMCPClient`, stdio) with no custom code. The
real gap was the *scoring core as a numpy-friendly Python lib* (no mainstream lib bundles
exact Murphy decomposition + Winkler + Wilson + EB shrinkage with the calibration-vs-
discrimination framing).

**How to apply:** don't add LangChain `Tool` classes — point users at the MCP server +
the LangGraph example at `bindings/python/examples/langgraph_mcp.py`. Extend the binding
by wrapping `anamnesis::scoring`, never by reimplementing math in Python. User handles all
git push (do not commit/push). See [[project-anamnesis-playground]] for the wider project.
