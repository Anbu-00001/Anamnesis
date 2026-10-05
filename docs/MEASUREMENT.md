# Measuring the pin-first protocol

Written on 2026-10-05, **before any data exists**, so that the analysis cannot be chosen after the
result is known. `validation/protocol_report.py` implements the rules below; it does not get to
pick them.

## What this is for, and what it is not

The README says plainly that the tool's effect on an agent's performance is unmeasured. This does
not change that. It measures three narrower things, all of which a few weeks of ordinary use can
answer:

1. **Does the protocol run?** Do the hooks fire, do claims get pinned and graded by exit status,
   does anything loop or break.
2. **Is it followed, and by whom?** A global instruction was followed by one model (Opus) and
   ignored by another (Sonnet) in the same task. A reminder now fires before the first bare test
   run of a session. How often does a run go through `ana run`, per model, and does the one-time
   refusal change what happens next?
3. **Does self-grading flatter?** Predictions about test runs used to be graded by the agent
   itself. Now they are graded by the exit status. Do the two records differ?

It cannot say whether the tool makes anyone better at predicting or at decisions. There is no
control group.

## What is recorded

- **The ledger** (`~/.anamnesis/agent.json`): pinned claims carry the exact command (`check`),
  `kind:tests-pass`, `project:<folder>`, and a `model:<model>` tag the agent writes. Graded ones
  carry `resolved_by: "auto"`.
- **`protocol.jsonl`**, beside the ledger, written by `ana hook pre-tool` when
  `ANAMNESIS_PIN_NUDGE` is on: one line per test run seen, with the time, the session id, the
  project folder name, the model (read from the session transcript, the harness's own record), a runner kind (`cargo`, `pytest`,
  `node`, …) and one of `ana_run`, `denied_no_pin`, `denied_unrun_pin`, `allowed_after_nudge`.
  **Never a command, a path, an argument or any output.** The file is private (0600) and is not
  sent anywhere; nothing in this tool touches the network.

## Baseline, frozen 2026-10-05

Taken from the agent ledger before the reminder was switched on:

| | |
|---|---|
| claims | 700 |
| resolved, scored (first forecast) | 540, Brier 0.181, said 69%, came true 69% |
| `kind:tests-pass` | 39, **all graded by hand** |
| `kind:tests-pass` by hand | Brier 0.163, **said 61%, came true 74%** (an underconfident record) |
| graded by exit status | 0 |
| `model:` tags | 0 |

## Questions and rules

**Q1. Mechanics.** After one week: `protocol.jsonl` has events from several sessions and at least
two projects; at least 15 claims were graded by exit status; no hook error or runaway turn was
reported. If not, the finding is about the plumbing and the report says which part.

**Q2. Compliance.** Per model: the share of test runs that went through `ana run`, and, among
sessions refused once, the share that went on to use `ana run`. Reported as counts with the
denominator. No threshold is pre-set for "good"; what is pre-set is that a model with fewer than
10 runs gets no percentage.

**Q3. Does self-grading flatter?** For `kind:tests-pass` claims: the hit rate of hand-graded
claims minus the hit rate of exit-graded ones, with a 95% bootstrap interval.

Rules:

1. **One week is for mechanics.** Q1 and Q2 may be read at one week. Q3 may not.
2. **No comparison below 20 claims on each side.** Below that, any difference is noise, and the
   script prints no comparison rather than one that invites a conclusion.
3. **The interval, not the point.** A difference is reported as "excludes zero" or "includes
   zero", and a null is reported as a null.
4. **One confirmatory look at Q3**, when there are at least 30 exit-graded claims or after four
   weeks, whichever is first. Anything read earlier or later is exploratory and is labelled so.
5. **Nothing is dropped.** Claims logged after a refusal count. Pinned claims that were never run
   are counted as such, not removed.

## Confounds, stated now

- **Different tasks.** The hand-graded baseline is mostly work on this repository. The exit-graded
  claims will come from whatever projects get worked on. A difference may be the projects.
- **The agent chooses when to run tests and what to pin.** Pinning `true` settles a claim with
  `true`. A narrow command makes a prediction easy to get right.
- **The reminder changes behaviour.** Being made to write a probability before a run may change
  the probabilities. That is a real effect of the protocol, but it means Q3 compares two
  conditions that differ in more than the grader.
- **Model mix.** Sessions use more than one model. The `model:` tag is written by the agent from
  what it knows of itself, and `protocol.jsonl` records what the harness reports; they can
  disagree.
- **Small samples.** A few weeks of casual use may not reach the thresholds. That is a result.

## Running it

```bash
python3 validation/protocol_report.py                 # everything
python3 validation/protocol_report.py --since 2026-10-06
```

## Switching it off

`ANAMNESIS_PIN_NUDGE=off` (or unset) makes `ana hook pre-tool` do nothing, and no log is written.
To remove it completely, delete the `PreToolUse` entry that runs `pre-tool.sh` from `settings.json`.
