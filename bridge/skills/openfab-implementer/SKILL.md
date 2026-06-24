---
name: openfab-implementer
description: For agent-chat agents whose whoami name contains `implementer`. Handles OpenFab tasks dispatched via the OpenFab↔agent-chat Bridge — implement the spec, then reply with a `task_result` message carrying full file contents so OpenFab can hash, sign, and gate exactly those bytes.
---

# OpenFab implementer (agent-chat side of the Bridge)

You are an agent-chat **implementer**. In addition to the normal `issue-workflow` behavior,
you handle **OpenFab tasks**. OpenFab (the software fab) drives the workflow and owns
verification/signing/gating; you only do the *implement* segment, then hand back the exact
bytes you produced.

## When this applies

On `[NOTIFICATION]` → `check_inbox()`. A message is an **OpenFab task** when:
- `from == "openfab-bridge"`, AND
- `schema.kind == "task_request"` (its `schema.payload.task_id` is the agent-chat task id).

The message `full` text contains: `INTENT`, `LANGUAGE`, `TARGET DIR`, `CONSTRAINTS /
DECISIONS`, and `BOUND TEST SCENARIOS` (named tests you must make pass).

If the message is NOT an OpenFab task, fall back to your normal `issue-workflow` role.

## What to do

1. **Implement** the spec in the shared workspace: write the program AND a test for every
   bound scenario, using EXACTLY the given test names (e.g. `test_adds_two_integers`).
   Emit a complete, buildable project at the repo root (e.g. `Cargo.toml` + `src/` +
   `tests/` for Rust; module + `test_*.py` for Python). Standard library only unless the
   constraints say otherwise.
2. **Verify locally** if you can (`cargo test`, `pytest`) so the bound tests pass.
3. **Reply with a `task_result`** addressed back to the bridge. The `files` map MUST contain
   the FULL contents of every file you produced (OpenFab signs exactly these bytes — do not
   send diffs or summaries):

```
send_message(
  to="openfab-bridge",
  type="reply",
  summary="implemented <spec_ref>",
  full="<one-line note>",
  schema={
    "kind": "task_result",
    "version": 1,
    "payload": {
      "task_id": "<the task_id from the request payload>",
      "status": "completed",
      "model": "<your model, e.g. claude>",
      "prompt": "<the OpenFab instruction text you worked from>",
      "files": {
        "src/main.rs": "<full file contents>",
        "Cargo.toml": "<full file contents>",
        "tests/cli.rs": "<full file contents>"
      }
    }
  }
)
```

## Rules (load-bearing for trust)

- **Bit-identical bytes**: the `files` contents must be exactly what you wrote. OpenFab
  hashes and signs them; any mismatch fails the Bridge integrity check.
- **Bound test names must match** the scenarios' filters, or OpenFab's `agent-spec
  lifecycle` verification will report the scenario as non-passing (skip ≠ pass) and the
  trust gate will block.
- **One reply per task.** If you must iterate, send a new `task_result` with the same
  `task_id`.
- Keep files small and self-contained; no network or external services in the generated
  code unless the spec asks for it.
