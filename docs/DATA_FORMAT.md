# Data format

One JSON file. Greppable, diffable, git-friendly, and intelligible without this
program — a record of your own judgement should never be trapped in a format only
one tool can read.

## Data format

One human-readable JSON file. Greppable, diffable, git-friendly, and intelligible without this program — because a record of your own mind should never be trapped in a format only one tool can read.

```json
{
  "claims": [
    {
      "id": "3ef7f5",
      "statement": "Bitcoin closes above $200k at some point in 2026",
      "created_at": "2026-01-04T10:00:00Z",
      "resolve_by": "2026-12-31",
      "tags": ["markets", "crypto"],
      "forecasts": [
        { "at": "2026-01-04T10:00:00Z", "prob": 0.35, "because": "halving tailwind, macro headwind" },
        { "at": "2026-06-01T09:00:00Z", "prob": 0.20, "because": "rally fizzled" }
      ],
      "resolution": { "at": "2026-12-31T12:00:00Z", "outcome": "no", "note": "anchored on the bull case too long" }
    }
  ]
}
```

A claim is a **palimpsest**: every revision is *appended*, never overwritten. Writes are atomic (temp file + rename), so a crash mid-save never corrupts the record.

## Fields added since 0.3.0

All of them are `#[serde(default)]` and are not written when absent, so a ledger
written by an older version stays byte-identical through a load/save round trip.
That is guarded by `a_v0_3_0_ledger_still_loads_and_reports` in
[`tests/hn_scenarios.rs`](../tests/hn_scenarios.rs), against a real 0.3.0 file in
[`tests/fixtures/`](../tests/fixtures/).

| field | meaning |
|---|---|
| `void` | `{at, reason}` — the question was annulled. Kept in history, excluded from every score. |
| `amendments` | `[{at, old_statement, new_statement, old_tags, new_tags}]` — corrections to wording or tags, pre-resolution only. |
| `resolution.resolved_by` | `self` (default, not written), `auto` (graded from an observed fact, e.g. a test run's exit status) or `human`. |
| `stake` | how much the call matters; `1.0` by default and not written when default. |
| `horizon_days` | how many days after creation the claim becomes answerable when it has no `resolve_by`. Set at creation from the `kind:` tag (default 7; `tests-pass`/`bug-hypothesis` 1; `estimate`/`approach`/`compat` 3). Stored rather than computed at read time so the evidence order is auditable from the file and cannot shift when a default changes. Absent ⇒ the current default. |

## What is immutable

Probabilities and timestamps. `ana amend` can fix a typo in a statement or correct
its tags before the claim resolves; it cannot touch a forecast or a date. Forecasts
are appended, never overwritten — the claim is a palimpsest, and the whole point of
the instrument is that the record of what you believed cannot be quietly revised
once you know how it turned out.

## The threat model, stated plainly

This guards against **hindsight bias** — your own memory rewriting how sure you
were. It does **not** guard against someone editing their own JSON file. Nothing
here is cryptographically signed, and it is not trying to be: the ledger is yours,
on your disk, and if you want to lie to yourself you can always open it in an
editor. What the tool prevents is lying to yourself *by accident*, which is the
thing that actually happens.

The one part that does not rest on your word is `resolved_by: auto` — claims graded
from a command's exit status rather than from anyone's account of what happened.
The report shows what fraction of your record those make up.
