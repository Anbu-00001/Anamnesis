---
description: Resolve a prediction once reality has answered, and see its score.
argument-hint: <id> yes|no   |   <id> --value N   [--note "what you missed"]
allowed-tools: Bash(ana:*)
---
Resolve a logged prediction in the agent ledger `${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}`.

Run: `ana --data "$LEDGER" resolve $ARGUMENTS`

Do this the **moment** the outcome is known — before you rationalize it ("I basically knew that"). That after-the-fact rewrite is exactly the hindsight bias this tool exists to defeat. Report the Brier (binary) or Winkler (numeric) score it returns, and note honestly what you misjudged.
