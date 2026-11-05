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

## Addendum 1, 2026-10-19: the log missed most runs

Written after two weeks of data, so it changes no rule above. It says which data the rules apply
to, and records what was found.

**Q3 has had its one confirmatory look.** The 30th exit-graded `kind:tests-pass` claim resolved
on 2026-10-08. The script was not run again until 2026-10-19, and that run is the confirmatory
look (rule 4): hand-graded minus exit-graded hit rate **−2%, 95% interval [−17%, +12%], includes
zero**. With 39 hand-graded and 431 exit-graded claims, no difference can be claimed. Every later
Q3 reading is exploratory.

**The log undercounted both sides of Q2.** Replaying the 1,190 real Bash commands from
2026-10-05 to 2026-10-19 that mention a test runner or `ana run`, taken from the session
transcripts, through `ana hook pre-tool`:

- `ana run` was recognised only when it began the last `&&` part. These shapes were never
  logged: `cd x; ana run …`, `nice -n 19 ana run …`, `ID=$(ana add …) && ana run "$ID" -- …`,
  `ana run … | tail && git add …`, `(ana run …)`, and multi-line scripts. That is 382 of the 471
  `ana run` calls in the transcripts.
- Bare `.venv/bin/python -m pytest` was not seen as a test run, because the grader's matcher
  knows only a bare `python`. Nor was a test run behind `nice`, `timeout` or `env`. So the
  denominator was short too, and the commonest bare run in the largest project was invisible.
- A subagent's call carries its parent's `transcript_path`, checked against a real 2.1.251
  payload (now `tests/fixtures/claude-code-2.1.251/pre_tool_subagent.json`). The model read
  from that transcript was the parent's for 18 of 33 subagent runs.

`src/pin.rs` now finds `ana run` and test runs in every segment, past assignments, wrappers and
subshell brackets. It skips here-document bodies, names a runner by its program's name, reads a
subagent's own transcript, and adds `subagent` to each log line. The grader (`hook::test_run`)
is unchanged.

**Rebuilt from the transcripts** (exploratory: reconstructed, not logged at the time). Each
command is credited to the model that wrote it, and every bare test run counts, including runs
that checked no change of the agent's own:

| | through `ana run` | the old log said |
|---|---|---|
| claude-sonnet-5-5 | 250 of 505 (50%) | 50 of 62 (81%) |
| claude-opus-5-5 | 166 of 219 (76%) | 31 of 36 (86%) |
| claude-haiku-5-5 | 20 of 20 | 3 of 3 |
| main agent / subagents | 312 of 565 (55%) / 125 of 180 (69%) | not recorded |

All of it comes from 7 sessions, and one project (showcase) holds 454 of the 745 runs at 48%.
A session-level rate from 7 sessions is an anecdote. Treat the table as a reason to keep
measuring, not as a compliance figure.

**What follows:**

1. Q2 is read from **2026-10-20**, the first full day of the fixed logger
   (`protocol_report.py --since 2026-10-20`). The script flags earlier events and gives them no
   compliance reading. The Q2 rules are unchanged: per model, and no percentage under 10 runs.
2. The report now prints the project mix and an exploratory exit-graded line without the
   largest project. Showcase holds 337 of the 431 exit-graded claims, so any pooled figure is
   mostly showcase's.
3. Not added: a control group. About 7 sessions ran tests in two weeks. Randomising the
   calibration line per session would give 3 or 4 sessions per arm, and no effect a few weeks
   could produce would be detectable. The tool's effect stays unmeasured, as stated above.
4. Not code: 130 claims more than a few days overdue are ungraded. They are priced into the
   evidence test (`src/evidence.rs`), and resolving them honestly is the only fix. Voiding them
   in bulk would push the late-void share past the line that withholds the verdict.

## Addendum 2, 2026-11-05: what a week should be able to say

Written before the data it describes, but **after seeing** the figures from 2026-10-20 to
2026-11-04: 896 runs from 6 sessions, one holding 70%; Opus 594 of 846 through `ana run`, Sonnet
16 of 48. Those figures are why these rules exist, and they are not evidence for them.

**What changed in what is recorded** (`"v": 3` on each log line):

- `repo`: the repository's folder name, through subfolders, a leading `cd` and linked worktrees.
  `project`, the folder a command ran in, stays, but it made a few codebases look like 23
  projects.
- `after_edit`: whether an edit was seen since the last test run, from the transcript. `null`
  when the last 256 KB does not settle it. The standing rule covers runs that check a change, and
  every bare run was counted against it. A script can edit without being seen, so `false` means
  none was seen.
- A claim logged inside Claude Code carries `origin` (`session`, `repo`, `client`), read from the
  environment Claude Code gives its child processes. Before this, a claim had no session id, so
  nothing could say how many came from one session. Older claims stay unstamped.
- More runs are recognised: `python -m unittest`, `python -B -m pytest`, `$P -m pytest`,
  `node --test`, `uv run`, `npm run test:*`, `ctest`, `eval`, shell keywords, `r=$(...)` and
  `bash -c "..."`.
- `ana add "the suite will run" ...` no longer counts as a pinned run.

Replaying the 11,923 real Bash commands from 2026-10-20 to 2026-11-04 through the old and new
hook: 18 false `ana_run` lines go, and 46 runs are gained (38 bare, 8 `ana_run`). The share through
`ana run` moves from 68% (614 of 898) to 65% (604 of 926), so the earlier window stays roughly
comparable. A sample of 160 replayed against the transcript as it stood at that moment: 91 of 139
runs after an edit went through `ana run` (65%). That is close to the overall rate, so no-edit runs
are not what holds the figure down.

**Rules for the reading on or after 2026-11-12** (`protocol_report.py --since 2026-11-05`):

1. Q2 is unchanged and stays the headline. `after_edit` is exploratory, and is labelled so.
2. **Sessions are the unit that limits what can be said.** Claims and runs from one session are
   not independent, and conventional intervals over-reject with 5 to 30 clusters, worse when their
   sizes are uneven (Cameron, Gelbach & Miller 2008; MacKinnon & Webb 2017). So the report prints
   no interval across sessions. It prints the effective number of sessions, the largest one's
   share, and the range when any one session is left out.
3. **No difference between models is claimed** unless each model has runs from at least 5 sessions
   and the two leave-one-out ranges do not overlap. Below that, the report's per-model figures
   describe these sessions and nothing else.
4. A week may simply not reach rule 3. That is a result, and the next release does not wait for it.

## Running it

```bash
python3 validation/protocol_report.py                 # everything
python3 validation/protocol_report.py --since 2026-10-20   # Q2: the fixed logger (addendum 1)
```

## Switching it off

`ANAMNESIS_PIN_NUDGE=off` (or unset) makes `ana hook pre-tool` do nothing, and no log is written.
To remove it completely, delete the `PreToolUse` entry that runs `pre-tool.sh` from `settings.json`.
