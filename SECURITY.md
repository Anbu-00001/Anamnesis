# Security policy

## Reporting a vulnerability

Please report it privately, not in a public issue. On GitHub: **Security → Report a
vulnerability** on this repository
([direct link](https://github.com/Anbu-00001/Anamnesis/security/advisories/new)).

This is a one-person project run in spare hours, so these are intentions, not
guarantees: I aim to acknowledge a report within about a week, and to say what I can
reproduce and when a fix might land. Reports are credited unless you ask otherwise.

## Supported versions

Only the latest release gets fixes.

## What counts

- **Ledger text that steers an agent.** The hooks and the MCP server hand stored text
  (claim statements, tags, ids) to an agent. Anything that lets that text escape its
  quoting, forge the engine's header, or reach a model in volume is in scope. See
  "How ledger text is handled" below.
- **The installer and release binaries.** Bypassing the checksum check in
  `plugin/install-ana.sh`, or a release binary that is not what the workflow built.
- **Crashes and data loss.** A ledger file, a CSV import, a hook payload or an MCP
  request that makes `ana` panic, hang, or corrupt or truncate a ledger.

## What does not count

- **Someone editing your own ledger.** The threat model is hindsight bias, not
  tampering: nothing here stops a person with write access to the file from changing
  it. See [docs/DATA_FORMAT.md](docs/DATA_FORMAT.md).
- **A verdict you think is wrong.** That is a bug, but a public one: please use the
  "verdict looks wrong" issue template.

## What to know before you trust it

- **Hooks and the MCP server run with your privileges.** A Claude Code plugin's hooks
  run outside the sandbox. Read `plugin/hooks/` and `plugin/mcp-server.sh` before you
  install, as you would any plugin.
- **Stored text is shown to the agent as bounded, labelled data.** Each claim goes out
  as one line of at most 120 characters, with markup neutralised and the engine's
  header mark replaced, under a label saying it is data and not instructions. That
  removes the cheap attacks. It cannot stop a short, plain sentence from being
  persuasive, so do not import a ledger, or accept predictions from a client, you do
  not trust.
- **The checksum protects against corruption, not a compromised release.**
  `install-ana.sh` verifies the binary against `sha256.sum` from the same release. It
  fails closed if that file is missing or does not match. Release attestation is not
  set up yet, so there is no independent proof a binary came from the workflow.
- **The ledger is not private by default.** It is a plain JSON file written with your
  default file permissions, and it holds your claims and your reasoning. Do not keep
  it inside a repository you publish.
- **`ana run` runs only what you give it.** The command pinned to a claim is stored text
  that is compared and never executed, so importing a ledger cannot run anything. The
  command that runs is the one typed after `--`, directly with no shell, and only if it
  matches the pin. Do not give it a command you would not run yourself.
- **No network, no `unsafe`.** The binary has no networking dependency (its
  dependencies are `clap`, `serde`, `serde_json` and `chrono`) and the source contains
  no `unsafe` code. There is no telemetry and no account.
