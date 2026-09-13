# STATE.md — what the repo actually contains, 2026-09-12

Established by running the commands in `README_AND_LAUNCH_TASK.md` §1 rather than
trusting that document's picture. Where the two disagree, this file records the
repo's answer. Regenerate by re-running §1.

## Verified against the code

| Question | Answer |
|---|---|
| README length | **286 lines** (target ~120) |
| README headings | 15, including a **duplicated `## Usage`** at lines 95 and 97 |
| `TODO(human)` markers | 6, at lines 3, 7, 9, 73, 77, 279 |
| Cut claims from the old plan | all gone (`first quantified`, `every other agent-memory`, `research is blunt`, `genuinely running on me`) |
| Feynman quote | still present, line 246 — inside the generated report block |
| `docs/agent-memory/` | deleted from the tree; still in history at `e07c740`, `70e4150` |
| Emoji in README | 1, the `⚠` the binary itself prints inside `<!-- BEGIN:report -->` |
| Raw HTML in README | 0 |
| Mermaid in README | 0 |
| Star-begging / tracking | 0 |
| Badges | none yet |
| Generated-output markers | `<!-- BEGIN:report -->` / `<!-- END:report -->` present, lines 148/248 |
| Sensitive paths (`/home/`, `PlayGround`, session ids) | none tracked |
| `ana import` | **exists** — resolves open question 21 |
| Mermaid already in `docs/` | **2**, in `docs/AGENTS.md:44` and `docs/DESIGN.md:31` |
| `accTitle` / `accDescr` on those | **0** |
| `docs/demo.tape`, recording | absent |
| `docs/assets/` | `badge.svg`, `card-dark.png`, `card-light.png` |
| `CONTRIBUTING.md`, `CITATION.cff`, `.github/ISSUE_TEMPLATE/` | absent |
| `CHANGELOG.md`, `LICENSE` | present |
| Scripts | `regen-examples.sh`, `check-versions.sh`, `check-test-count.sh`, `check-banned-phrases.sh` |
| Tests | 120 Rust (86 unit, 8 CLI, 26 scenarios), 36 Python |

## Discrepancies with the task document

1. **The task says three diagrams are to be built. Two already exist**, and both
   violate the rules it sets out: emoji inside nodes (`📝 ✏️ ✅ 🪞 🎯 ✋ 🔍`), HTML
   tags (`<b>`, `<i>`), and Greek/mathematical characters in labels (`σ`, `τ`,
   `≥`, `½`, `p̂`). Neither has `accTitle`/`accDescr`. So the work is *rewrite two
   and add one*, not *add three*, and the cap of three in the repo is already the
   binding constraint.
2. **`docs/DESIGN.md` has a duplicated heading** (`## Choices` immediately
   followed by `## Design choices`), the same defect as the README's double
   `## Usage`.
3. **`ana import` exists**, so open question 21 is closed: a visitor with an
   existing history has a path in.
4. **PredictionBook must not go in the comparison table.** It went read-only on
   2024-01-10 and is now retired, with the site itself pointing at Fatebook. The
   current table correctly omits it; leave it omitted.

## Blocking facts found while surveying

These are not in the task document and matter more than anything in it.

1. **Both package manifests declare a name that is taken on both registries.**
   `Cargo.toml` and `bindings/python/pyproject.toml` both say `name = "anamnesis"`.
   Checked 2026-09-12 against the registry APIs:

   | name | crates.io | PyPI |
   |---|---|---|
   | `anamnesis` | **200 — taken** | **200 — taken** |
   | `anamnesis-calibration` | 404 — free | 404 — free |
   | `ana` | 200 — taken | — |

   A publish of the current manifests cannot succeed. This is owned by the human
   (item 5) but the manifests are currently pointing at a wall, not merely
   undecided.

2. **The PyPI-facing description is stale and now wrong.**
   `bindings/python/pyproject.toml` advertises "exact Murphy decomposition". 0.4.0
   replaced that with CORP precisely because the exact-value grouping reported
   0.073 of calibration error where the truth was 0.000. The one-line description
   on the package page would contradict the release it ships with.

3. **Two READMEs are published, not one.** `Cargo.toml` has
   `readme = "README.md"` (root) and `bindings/python/pyproject.toml` has
   `readme = "README.md"` resolving to `bindings/python/README.md`. The
   no-Mermaid rule therefore binds on **both** files, not just the root.

4. **`ana` on `PATH` is version 0.3.0** (`~/.local/bin/ana`), while the repo
   builds 0.4.0. Every session hook and every `ana report` run outside
   `./target/release/` has been reporting 0.3.0 wording — including the phrase
   "well calibrated in aggregate", which 0.4.0 deliberately removed. Nothing in
   the repo is wrong; the installed binary is stale.

## Verified external claims

| Claim | Status |
|---|---|
| crates.io does not render Mermaid | **confirmed** — rust-lang/crates.io discussion #5724 is an open *request* for it |
| PyPI does not render Mermaid | **confirmed in substance** — rendering needs JavaScript, which PyPI's description pipeline does not execute |
| PredictionBook status | **retired**; read-only from 2024-01-10, site now points at Fatebook |
| `anamnesis-calibration` free on both registries | **confirmed** 2026-09-12 |

## Tooling available for the recording

`asciinema` and `agg` are installed; **`vhs` is not**. §5's second option is the
one that can actually run here.


---

## What this pass changed (2026-09-12)

Everything below was verified by running it, not by reading it.

| Item | State |
|---|---|
| README rewritten to the §3 structure | done — 182 lines, 5 `TODO(human)` markers left |
| Example output in the README | generated by a new `report_head` block in `regen-examples.sh`, cut at the decomposition identity rather than a line count; CI already fails when stale |
| Demo recording | done — `docs/assets/demo.gif`, 67 KiB, 9.1 s, 100x46, regenerated by `scripts/regen-demo.sh` from `docs/demo-session.sh` and the fictional `docs/demo-ledger.csv` |
| Diagrams | three, all with `accTitle`/`accDescr`, prose first, implementing function named: evidence sequence and `decide` in METHODS, agent integration in AGENTS |
| Diagram verification | parsed with mermaid 11.17.2 **and** rendered to SVG with `mermaid-cli` + Chrome; 0 error boxes, labels confirmed present in the output |
| `CONTRIBUTING.md`, `CITATION.cff`, issue templates | done; CFF validated as YAML with all required keys |
| Install line | **ran it.** `cargo install --git ... --locked --root <tmp>` exits 0 and produces `ana 0.4.0` |
| Link check | every relative link in the README and in the five edited docs resolves |

### Defects found while doing it

1. **The prebuilt install path 404s today.** `releases/latest/download/sha256.sum`
   returns 404 because no release exists yet. The installer fails closed with a
   build-from-source message, so it degrades safely, but the README line is live
   only after the tag. This is exactly what item 9's smoke test is for.
2. **The Stop hook described behaviour that no longer exists** — it said the
   evidence test "stops at the first" overdue claim, which gap-filling removed.
   Fixed.
3. **`docs/METHODS.md` said "well calibrated" never appears below 50 graded
   calls.** It never appears at all now. Fixed.
4. **Duplicate headings in four files**: `README.md` (`## Usage` twice, and "Ids
   can be abbreviated" twice), `docs/DESIGN.md` (two H1s, `## Limitations`
   twice), `docs/DATA_FORMAT.md` (`## Data format` twice), and `docs/METHODS.md`
   had two `### 3d` sections with `3c` after one of them. All fixed.
5. **The report printed the same table twice.** With a bare-tag ledger the
   grouping falls back to `topic`, which lists exactly the rows "By domain"
   already showed. Visible in the demo recording as "By domain" followed by "By
   topic" with identical numbers. The tag table is now suppressed when the
   breakdown is keyed on bare tags, which also cut the report from 98 lines to 86.
6. **The PyPI description advertised "exact Murphy decomposition"** — the thing
   0.4.0 replaced. Corrected.
7. **The first frame of the recording was an empty terminal**, because the
   session opened with a sleep. The first frame is the poster, so the sleep is
   gone.

### Also prepared

- `docs/assets/social-preview.png` — 1280x640, cropped from the top of
  `card-dark.png` so the aspect is preserved rather than squashed. It shows the
  tool's own verdict card ("Overconfident — you oversell yourself"). Whether that
  is the right first impression is your call; the alternative is the light card.
- `docs/launch/GOOD_FIRST_ISSUES.md` — four drafts, each checked against the code
  rather than invented: a `--since` window on `report`, three unwrapped functions
  in the Python binding, JSON input for `ana import`, and the demo's missing
  `kind:` tags. Each names the trap a first attempt would otherwise hit.
- `README_AND_LAUNCH_TASK.md` is in `.git/info/exclude`, like `LAUNCH_PLAN.md`.
  If you want either tracked, remove the line.

### Still blocking, and owned by the human

- Package names: the manifests declare `anamnesis`, taken on both registries.
- The five `TODO(human)` markers in the README, and the author block in
  `CITATION.cff`.
- The tag, the release, and the three-runner install smoke test — items 7 to 10.
- The git-history question for `docs/agent-memory/`.

---

## Pass 3 (2026-09-13)

### What the hooks on this machine were actually running

| Fact | Found |
|---|---|
| `ana` on `PATH` | `~/.local/bin/ana`, **0.3.0** |
| `~/.anamnesis/bin/ana` | **0.3.0** |
| `~/.cargo/bin/ana` | absent — and `~/.local/bin` precedes `~/.cargo/bin` on `PATH`, so `cargo install --path . --force` would have installed a binary nothing runs |
| Registered hooks | SessionStart → `~/.anamnesis/session-calibration.sh`, a local jq script never in the repo; UserPromptSubmit → `~/.anamnesis/hooks/user-prompt.sh`, the 0.3.0 jq script. **PostToolUse and Stop were never registered**, so auto-resolution has never run here |
| The jq scripts against 0.4.0's JSON | every field they read is still present, so they keep printing "well-calibrated overall" while 0.4.0's own verdict on the same ledger is `no_evidence_of_miscalibration` |
| Did 0.3.0 ship that wording? | yes — `plugin/hooks/user-prompt.sh` and `session-start.sh` at `70e4150` both contain it |

The consequence: "upgrade the binary, then check the greeting" would have confirmed
the wrong thing. The greeting's wording came from the script, not the engine.

**Done** — everything replaced was first backed up to
`~/.anamnesis/backup-0.3.0-2026-09-13/`:

- `cargo install --path . --locked --force --root ~/.local`, which targets the
  directory `PATH` actually resolves; and the same binary vendored to
  `~/.anamnesis/bin/ana`.
- The 0.4.0 hook launchers copied into `~/.anamnesis/hooks/`.
- The registered SessionStart script replaced with a launcher, so
  `~/.claude/settings.json` did not need editing. Its checksum was compared before
  and after: unchanged.

Each hook was then run exactly as registered, against a copy of the real ledger.
All four first lines read `(ana 0.4.0)`; none contains stale wording; post-tool
graded its claim from the exit status; the MCP launcher reports
`serverInfo.version` 0.4.0. The next session's real greeting:

```
⟢ Anamnesis (ana 0.4.0) — your standing calibration
no miscalibration found across 236 graded calls — which is not proof you are calibrated, only that nothing shows otherwise
  110 ungraded call(s) are costing you evidence, oldest [bd85bc] — resolve or void them
```

**Revert:** copy `local-bin-ana` back to `~/.local/bin/ana`, `anamnesis-bin-ana` to
`~/.anamnesis/bin/ana`, and `session-calibration.sh` and `hooks/` back to
`~/.anamnesis/`.

**Not done, and yours:** registering PostToolUse and Stop changes behaviour in
every project on the machine. `bash plugin/install.sh --yes` would do it, but here
it would also register a *second* SessionStart and UserPromptSubmit hook, because
the existing entries use a different script path and a literal `$HOME` that the
installer's duplicate check does not match. Remove those two entries first, or add
the other two by hand.

### Code changed

1. **Duplicate breakdown, fixed at selection rather than in the renderer.**
   Reproduced first: `kind:alpha` plus a bare `alpha` on every claim printed
   "By domain" and "By kind" with identical rows, which the name-based render fix
   had missed. The selector now materialises each candidate grouping once and
   collapses candidates that induce the same partition before choosing; the table,
   `K`, the alarm threshold and the hook read that one result.
2. **Engine version on the first line of every hook.**
3. **Installer:** reuses a `PATH` engine only when its version matches the plugin's.
4. **Hook launcher and MCP launcher:** run the newest engine available, not the
   first on `PATH`. The MCP launcher's comment said outright that it resolved
   "the same way the hooks do — PATH first".
5. **Banned-phrase guard** extended to `plugin/`; verified to fail on a phrase
   planted in a shipped hook.

Three scenarios added — one partition is one breakdown with one K; every hook
names its engine; an upgrade never runs a stale engine from `PATH`, driven through
the real installer, the installed hook and the MCP launcher. 123 Rust tests, 36
Python.

### README at phone width

Rendered through GitHub's own Markdown API in `markdown` mode, then screenshotted
under true mobile emulation at 390x844. Both matter. The first attempt used `gfm`
mode, which is the *comment* renderer and turns every source newline into `<br>`;
and it used a desktop Chrome window, which cannot be made 390px wide and clipped
the page. Together they made the README look broken when it was not.

| At 390x844 (fold at 844px) | as rewritten | Demo moved below Try it | checksum note folded into the install block |
|---|---|---|---|
| Install heading | 1179 | 539 | 539 |
| `cargo install` block | 1233 | 593 | 593 |
| Try it heading | 1501 | 811 | 747 |
| `ana demo` block | not measured | 866 | **802** |
| Demo heading | 539 | 1810 | 1746 |

The first-screen test from the task's §7 failed as written: at phone width the
demo recording — 385px tall, mostly an empty terminal — sat between the problem
statement and the install line. Moving Demo below Try it is a deliberate
departure from the section order in §3.1, taken because §7 is the acceptance test.
There is 42px of headroom above the fold; the one-sentence description still to be
written sits above all of it, and a two-line sentence would use most of that.

### Known edge, not fixed

A real `topic:` tag namespace collides with the bare-tag pseudo-namespace that is
also called `topic`: the selector maps both to one name and groups only the bare
tags, so a ledger tagged `topic:markets` never gets a breakdown by those tags.
Fixing it means choosing a display name for bare tags that cannot collide with a
real prefix, which is a naming decision rather than a bug fix.

---

## Pass 4 (2026-09-13): the TODO(human) sections, written at the author's request

The README's one-sentence description, the framing around the comparison table,
and "How this was built" are written, and the description is mirrored into
`FACTS.md` as the GitHub repository description (119 chars). The `CITATION.cff`
author note is resolved, crediting `Anbu` (`Anbu-00001`). No `TODO(human)` marker
remains in the README, `CITATION.cff` or `FACTS.md`.

"How this was built" states plainly that Claude wrote most of the code and docs.
Two things in it are the author's to confirm or correct, because only the author
knows them: the proportion of the work that was Claude's, and who performed the
pre-launch review, which the section deliberately leaves unattributed.

The Fatebook facts in the comparison framing were re-checked against its
repository: MIT-licensed, with Slack and Chrome integrations.

HN's own rules still apply to what gets posted there: the title, the submission
text and every comment are the author's to write.

---

## Pass 5 (2026-09-13): pre-submission sweep

Run as a stranger would run it, against a fresh clone and clean environments.

| Check | Result |
|---|---|
| Fresh clone, `cargo build --release --locked` | builds |
| Fresh clone, `cargo test --all` | 123 passed (125 after this pass) |
| Fresh clone, `./validation/repro.sh`, `validation/sims.py` | green |
| Fresh clone, `validation/ratio.py` | **failed** — needs the Python binding, which the README did not say. Fixed: README note, and the script now explains what it needs |
| Wheel from a fresh clone in a clean venv | 36 passed |
| `ana mcp` from the clean build | `initialize` and `tools/list` answer; the tool list matches the README exactly |
| Other operating systems | CI on the pushed commit passes on Linux, macOS, Windows and the MSRV job |
| CI badge as GitHub serves it | "passing" |
| `ana report` with no ledger, an empty ledger, one open prediction | friendly message, exit 0 |
| Truncated JSON, missing fields, wrong types, `outcome: "maybe"` | precise error, exit 1, file untouched |
| **0-byte ledger** | **"EOF while parsing a value"**. Fixed: reads as empty, unless a backup sits beside it, in which case it is refused so the next save cannot overwrite the only good copy |
| **`prob: 1.7` in a hand-edited ledger** | **scored silently, verdict `OVERCONFIDENT`**. Fixed: refused at load, naming the claim. Both real ledgers on this machine (436 and 8 claims) were checked against the new rule first; neither contains anything it rejects |
| README example commands, verbatim | run; `ana add` accepts a past `--by`, so the hard-coded dates will not break |
| Every link and image in every doc | **four broken**, all in `docs/AGENTS.md` and `docs/PYTHON.md`, written relative to the repo root instead of `docs/` (including the langgraph example). Fixed; 79 of 79 now resolve |
| External references | the five that return 403 to scripts all exist: four DOIs resolve through Crossref, and the Good Judgment page is bot-blocked, not missing |
| `.github/workflows/*.yml` | actionlint clean |
| Release workflow | asset names and `sha256.sum` format match `install-ana.sh` — confirmed by running the workflow's own package and publish commands locally, then the installer and the smoke check against the result. But its smoke job ran `cargo install --path .` and never touched the published binaries. Rewritten, and `-rc` tags are now marked pre-release so they cannot become "latest" |
| History of `docs/agent-memory/` | exposes the local username in two paths; no token-shaped string anywhere in any commit. No rewrite needed |
| GitHub description | was a different, longer text; now the README sentence. Topics `decision-making`, `claude-code`, `e-values` added |

Still manual, because GitHub has no API for it: upload
`docs/assets/social-preview.png` under Settings → General → Social preview.
