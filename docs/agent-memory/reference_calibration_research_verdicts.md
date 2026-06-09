---
name: calibration-research-verdicts
description: External 2023-2026 evidence on agent/LLM calibration that should drive Anamnesis design decisions.
metadata: 
  node_type: memory
  type: reference
  originSessionId: 434d3d50-1c5a-4981-acec-430b13856106
---

Deep literature review (2026-06-07) for Anamnesis. Key external verdicts, with arXiv ids:

- **Generalized Correctness Models — arXiv 2509.24988.** LLMs have *little self-knowledge*: a model predicting its own correctness does no better than an unrelated model. Reliable confidence is a **model-agnostic skill learned by encoding correctness HISTORY, not introspection.** → near-literal scientific justification for a recorded-ledger tool; lean on history + mechanical correction, not "introspect harder."
- **Reflexion — arXiv 2303.11366.** In-context *recorded* feedback stored as episodic memory improves agents with NO weight updates: +11% HumanEval, +20% HotPotQA, +22% AlfWorld. → the mechanism Anamnesis hooks rely on is evidence-backed (provided feedback = track record, not a nudge to introspect).
- **OpenAI Sept 2025 — "Why Language Models Hallucinate" (Kalai, Nachum, Vempala & Zhang, arXiv 2509.04664).** Training + evaluation procedures reward confident guessing over admitting uncertainty → models are *structurally* trained toward overconfidence. An external calibration instrument is the counter-pressure.
- **Code domain:** token-probability confidence is *better-calibrated than verbalized* confidence → the verbalized numbers I log are the weakest signal class (argues for a mechanical recalibration layer). Multicalibration for Code LLMs: arXiv 2512.08810.
- **Anytime-valid e-processes — Henzi–Ziegel arXiv 2103.08402; Ramdas game-theoretic stats.** Fixed-n calibration tests (Spiegelhalter Z) are INVALID under per-session peeking (false-positive 0.05→0.15). e-process = running product, valid under optional stopping, reject when e ≥ 1/α. Supersedes Spiegelhalter as the decision tool.
- **Dialectical bootstrapping / "crowd within" — Herzog–Hertwig 2009.** Averaging a deliberate "consider-the-opposite" 2nd estimate ≈ half the gain of a second person. Debiasing: consider **2** counter-reasons, not 10 (CHAMPS KNOW training +6–11%, Tetlock/Good Judgment).
- **Small-n stats:** analytic Brier SEs underestimate uncertainty at small n → bootstrap CI. Conformal prediction + Small-Sample Beta Correction (arXiv 2509.15349) gives distribution-free guaranteed interval coverage. Multicalibration (Hébert-Johnson 2018) formalizes per-`kind:` calibration.

**Phase-2 fresh pass (2026-06-08), post-Tier-3 — the frontier has MOVED off scoring math:**
- **The 2026 consensus finding (Zylos 2026-04; "Beyond Accuracy" arXiv 2504.02902; UQ-in-Agents survey arXiv 2602.05073):** agents can *verbalize* uncertainty accurately yet **fail to ACT on it** — taking irreversible actions while stating they're unsure. Self-improvement loops *raise* ECE (overconfidence compounds). → the open problem isn't measuring calibration (Anamnesis does that); it's **coupling calibrated confidence to action.**
- **Confidence-gating / uncertainty-aware deferral — ReDAct arXiv 2604.07036.** The named fix: estimate uncertainty, compare to a *calibrated* threshold τ, then proceed / clarify / defer-to-stronger-model / abstain. Confidence-gating hits precision 0.95 at ~70% display rate. The threshold must be calibrated — which is exactly Anamnesis's output (recalibration map + e-process evidence + stake weights + the **risk–coverage curve I already ship**, whose risk-vs-coverage axes ARE error-vs-display-rate). So the gate is a small natural addition on the existing substrate, not new scoring.
- **Decision-theoretic calibration (arXiv 2408.02841; truthfulness 2503.02384):** every proper scoring rule = a weighted sum of decision losses over thresholds; calibration's value is the cost of decisions made with it. Generalizes stake-weighting; subsumed by an explicit action-gate.
- **Online/Adaptive Conformal Inference (Gibbs–Candès 2021; retrospective-adjustment arXiv 2511.04275):** adapts interval width online to drift. DEFERRED — tuned for streaming scale; at an agent's n the static pooled `conformal_width_factor` is more stable and ACI's learning-rate is a false-alarm knob (same reason CUSUM was rejected).
- **VERDICT → BUILT (2026-06-08):** the **decision/action-gate** shipped. `scoring::decide(p, recal, stake, verify_cost) -> Decision{act: Proceed|Verify|Abstain, adjusted_p, proceed_threshold, margin}`: recalibrate the stated p (evidence-gated), then **Chow's reject rule** `τ = 1 − verify_cost/stake` (proceed iff `p̂ ≥ τ`; the bar climbs with stakes — the fix for irreversible-action-while-uncertain); below even odds after correction ⇒ abstain (parameter-free split of Chow's reject region). Surfaced as CLI `ana decide --prob --stake`, MCP `decide` tool (6 tools now), Python `ana.decide(...)`. The evidence gate is `report::earned_recalibration` (single source of truth, shared by report/recalibrate/decide). 49 lib + 7 e2e + 27 py tests. This is the literature's #1 agent open-problem and the "inputs/workflow dominate downstream math" thesis made operational.

See [[anamnesis-research-roadmap]] for how these map to build tiers. Related: [[project-anamnesis-python-binding]].
