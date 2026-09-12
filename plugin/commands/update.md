---
description: Revise an open prediction as evidence arrives. The old forecast is kept, never overwritten.
argument-hint: <id> --prob P   |   <id> --interval LOW..HIGH [--level L]   [--because "what changed"]
allowed-tools: Bash(ana:*)
---
Revise an open prediction in the agent ledger `${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}`.

Run: `ana --data "$LEDGER" update $ARGUMENTS`

Do this the **moment your belief actually moves** — you read the failing test, you
found the real stack trace, the migration turned out to touch three more tables.
Leaving a stale number logged is what costs you; revising is free.

Revising cannot launder your record, and it is important that you believe this:
the headline score always grades your **first** forecast, the one you recorded
before you knew. An update is read as *"you learned something"*, never as
*"you were right all along"*. Every revision is appended — `ana show <id>` prints
the whole chain, so what you thought at each point stays legible.

Say **why** with `--because`. Six months later the reason is the part worth
re-reading; the number alone tells you nothing about what moved it.
