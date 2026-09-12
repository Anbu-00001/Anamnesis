---
name: The verdict looks wrong
about: The report says something about your calibration that does not match your record
labels: verdict
---

**What the report said**

The verdict line, and the numbers near it:

```

```

**What you expected instead, and why**

**An anonymized ledger, if you are willing**

This is the one thing that makes a verdict complaint actually diagnosable, and it
does not require publishing what you predicted:

```
ana export --anonymize --out anonymized.json
```

That removes every free-text field, replaces ids, rounds dates to the day and
keeps only whitelisted tag namespaces. The numbers are unchanged, which is what
the report is computed from. Open the file before attaching it.

**Version**

```
ana --version
```
