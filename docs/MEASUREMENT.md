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

## Addendum 3, 2026-11-07: an experiment on the checkpoint, before any data

Written before a single draw was logged. Everything above asks whether the protocol runs and
is followed. None of it can say whether the tool makes anyone better, because nothing is ever
withheld: there is no comparison. This adds one, for the one part that can be withheld
without losing anything that matters.

**The design.** From 0.5.0, the mid-session checkpoint (`ana hook user-prompt`) is no longer
shown on every 7th prompt. Each prompt gets it with probability 1/7, drawn from a hash of the
session id and the prompt number, and every draw is logged to `checkpoints.jsonl` beside the
ledger: time, session, prompt number, the probability, whether it was drawn. Never the prompt.
A drawn prompt can still print nothing (an unreadable ledger, or nothing to say), so
each line also records `shown`. The test compares prompts by the draw, never by
`shown`: what was shown depends on the ledger, and only the draw is random (intent to
treat). The share drawn but not shown is reported beside it. This is a micro-randomized trial (Klasnja et al. 2015; Boruvka et al. 2018): within a session,
prompts that got the checkpoint and prompts that did not differ only by the draw. A claim
logged inside Claude Code carries `origin.session`, and is assigned to the latest prompt of
that session at or before its creation time.

**The question.** Do the calls made right after the standing calibration is shown score
better than calls made without it?

**Rules**, implemented by `validation/effect_report.py`, which does not get to pick them:

1. **Primary outcome:** the mean Brier score (first forecast) of the graded binary claims
   logged under a prompt, one value per prompt. Lower is better. Two-sided randomization test
   of the sharp null "the checkpoint changes nothing", at α = 0.05: the logged assignments are
   re-drawn from their logged probabilities, 10,000 times, outcomes held fixed.
2. **One confirmatory reading**, when 40 drawn prompts have a graded claim, or on 2026-12-19
   (six weeks), whichever comes first. Every reading before or after is exploratory, and is
   labelled so. No arm with fewer than 10 prompts gets a comparison.
3. **Secondary outcomes, all exploratory:** the confidence gap per prompt, the share of
   exit-graded `kind:tests-pass` claims that passed, claims logged per prompt (every prompt
   counts, zero included), and the share of test runs through `ana run` (protocol log v3).
4. **Nothing is dropped.** Prompts without a claim count where the outcome allows it (claims
   logged per prompt), and a claim graded late counts at the prompt it was logged under.
5. **The probability stays 1/7** until the confirmatory reading. A session that sets
   `ANAMNESIS_INTROSPECT_EVERY` logs its own probability, and the test uses what was logged.

**Why the test is valid, and where it is weak.**

- The p-value rests on the draw alone: no model of sessions, no assumption that they are
  alike or many, which is what limits every figure above (addendum 2, rule 2). An exact
  randomization test stays exact when an effect carries over to later prompts, as long as a
  draw cannot depend on what happens after it (Bojinov & Shephard 2019). Ours cannot: it is a
  hash of the session id and the prompt number.
- **Carryover dilutes.** A checkpoint at prompt 9 may still shape prompt 10, which is in the
  "not drawn" arm. That pulls the two arms together, so a real effect is under-read, never
  invented.
- **The primary outcome exists only where a claim was graded.** If the checkpoint changes
  whether a claim gets logged, the two arms' claims are no longer comparable sets. The test of
  the sharp null stays valid, since under it nothing changes, but the size of a difference
  then mixes "better calls" with "different calls". Claims logged per prompt is the check.
- **Power is likely low.** About one prompt in seven is drawn, and most prompts log no graded
  claim. A null at the confirmatory reading is the expected result unless the effect is large,
  and it is reported as a null, not as proof of none.

**What it can show:** whether seeing the standing calibration mid-session changes the calls
made right after it. **What it cannot:** the effect of the tool as a whole, which is always on,
or of the session-start line, which every session gets. Those stay unmeasured.

**Note, 2026-11-07, before the first such claim.** From today the agent's global
instructions ask it to log a `kind:external-fact` claim before it researches a risky point
on the web, and to resolve it from the page. These are graded binary claims, so by rule 4
they enter the primary outcome above like any other. They are not filtered out, and the
secondary outcomes are also reported without them, labelled exploratory.

**Note, 2026-11-07: two more instruments, neither part of this trial.** The person's bets on
the agent (`?70`, `bets.json`) are not claims in the agent ledger and enter no outcome here
or in addendum 4. Audit sampling (`ANAMNESIS_AUDIT_RATE`) refuses runs, so it stays off on
this machine until the confirmatory reading above. Any later reading of it gets its own
addendum, written first.

## Addendum 4, 2026-11-07: the whole tool, a week at a time, before any data

Written before the outcome log below had recorded a single run. Addendum 3 asks about one
part, the checkpoint. This asks about all of it, at the only scale one person's volume can
carry, and it does not start until addendum 3 has had its confirmatory reading.

**What is recorded now, from the release that adds it.** Every test run the post-tool hook
sees gets one line in `outcomes.jsonl` beside the ledger (`pin::log_outcome`, `v: 1`). It
holds the time, session, the session's current prompt number, project and repo, model,
whether a subagent ran it, `after_edit`, the runner label, `via` (`bare` or `ana_run`), the
exit status, and what the grader made of it (`pass`, `fail`, `untrusted`, `not_graded` or
`background`, by the same `grade_of` rules that grade claims). It never records the command,
its output or the prompt. Every file edit gets a line too (`pin::log_edit`, from a
`PostToolUse` matcher on `Edit|Write|MultiEdit|NotebookEdit`): time, session, prompt, project
and repo, never the file's name or its content. Both kinds carry the trial arm in force, or
`null` outside a trial. The log is written whether or not a claim is open and whether or not
the pin reminder is on, and it prints nothing.

**Built the same day, before any data:** `ana trial start` writes `trial.json` once, under the
ledger lock, and refuses a start that is not a future Monday on or after 2027-01-04
(`src/trial.rs`). `ana trial status` names the week but never the arm or the schedule. The
arm gate is `trial::speaks`: in an OFF week the session-start and checkpoint hooks print
nothing, the pin refusal does not fire, the post-tool hooks grade without telling the model,
and the MCP server sends empty instructions. A missing or unreadable `trial.json` reads as no
trial, so it can only leave the tool ON. `validation/crossover_report.py` implements the rules
below and carries a `--selftest`.

**Still to do before week 1, by the owner:** move the protocol text in the global agent
instructions into the session-start output, so that the arm controls it, and register the edit
matcher wherever the hooks are installed by hand rather than by the plugin. If either is not
done by 2027-01-04, the start moves to the first Monday after, and nothing else changes.

**The design.** Thirteen pairs of weeks, Monday to Sunday, from the first Monday after
addendum 3's confirmatory reading, and not before 2027-01-04. In each pair one week is ON and
one is OFF. The order is drawn once at enrolment from a seeded hash and written to
`trial.json` before the first week begins, and it is never shown to the model. In an OFF week
every hook that speaks to the model stays silent: the session-start line, the checkpoint (it
logs `arm: off` and draws nothing), the pin refusals, and the MCP server's instructions. The
protocol text in the owner's global agent instructions moves into the session-start output
first, so that the arm controls it. The grader, the ledger and the outcome log keep working in
both arms.

**The question.** Does Anamnesis, as a whole, change how often the agent hands over code that
fails its next test run?

**Rules**, implemented by `validation/crossover_report.py`, which does not get to pick them:

1. **Primary outcome:** a week's handoff failure rate. Take the prompt turns that edited a
   file. The rate is the share whose first graded test run after the last edit failed, where
   that run may come in the same turn or a later turn of the same session and repo. Lower is
   better. Turns with no later graded run are counted per arm and left out of the rate.
2. **Test:** the exact sign-flip randomization test over the 13 within-pair differences (OFF
   minus ON). It uses all 8,192 assignments, is two-sided at α = 0.05, and comes with its 95%
   interval.
3. **One confirmatory reading,** at the end of the thirteenth pair. The e-value may be read at
   any time and is labelled anytime-valid. It is the mean over λ ∈ {1, 2, 3, 4, 6, 8} of
   ∏ exp(λd − λ²d²/2), valid because cosh x ≤ exp(x²/2). The p-value is read once.
4. **Adherence:** a week is in its arm if at least 90% of its graded runs carry that arm. A
   week that is not stays in its assigned arm (intent to treat), and the count is reported.
5. **Secondary outcomes, all exploratory:**
   - the share of edit-turns ending green;
   - time to green;
   - test runs per edit-turn (whether OFF changes how often the agent tests);
   - the share of edit-turns with no later graded run, per arm;
   - `ana` calls in OFF weeks.
6. **This addendum and addendum 3 answer different questions,** each at α = 0.05. No result is
   reported as the two combined.

**Why the test is valid, and where it is weak.**

- The p-value rests on the draws that ordered each pair, not on any model of weeks, as in
  addendum 3 (Bojinov & Shephard 2019).
- **Power is low unless the effect is large.** A simulation written for this design, with 60
  edit-turns a week and a base rate of 0.75, gives two figures, which hold only under its own
  assumed distributions. Thirteen pairs detect a 10-point change with power 0.66 to 0.98, and
  a 5-point change with 0.21 to 0.48. A null is the expected reading for anything smaller, and
  it is reported as a null, not as proof of none.
- **The owner knows the arm.** The weeks are not blinded, because the session-start line is
  missing in OFF weeks. If the owner checks the agent's work more closely in one arm, that is
  part of the result.
- **Contamination.** The MCP tool list stays visible in OFF weeks, and the agent can still call
  `ana` from habit. Those calls are counted.
- **Drift.** Projects change from week to week. Pairing adjacent weeks removes slow drift, but
  not a change of project inside a pair.
- **Carryover.** If habits persist from an ON week into an OFF one, through memory or
  instructions the arm does not control, the contrast shrinks. The test then under-reads the
  effect and never invents one.

**What it can show:** whether the tool as a whole changes handoff failures for this owner by
about ten points or more over six months.

**What it cannot show:**
- a smaller effect;
- the effect on the owner as a forecaster;
- which part of the tool does the work;
- anything about other people's agents, which needs other people to run the same protocol and
  publish their e-values.

## Running it

```bash
python3 validation/protocol_report.py                 # everything
python3 validation/protocol_report.py --since 2026-10-20   # Q2: the fixed logger (addendum 1)
python3 validation/effect_report.py                   # the checkpoint experiment (addendum 3)
python3 validation/crossover_report.py                # the weekly trial (addendum 4)
```

## Switching it off

`ANAMNESIS_PIN_NUDGE=off` (or unset) makes `ana hook pre-tool` do nothing, and no log is written.
To remove it completely, delete the `PreToolUse` entry that runs `pre-tool.sh` from `settings.json`.
