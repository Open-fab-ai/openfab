# ADR 0004 — Orchestrator ↔ agent (base) contract

**Status:** proposed · **Date:** 2026-07-01 · **Builds on:** ADR 0002, ADR 0003

## Context

ADR 0003 splits self-hosting into a stable control plane (tiny_http + `ops` layer + trust
gate) and a disposable data plane (worktree + preview). This ADR fixes the contract across
that boundary: how the **orchestrator** (`ops::start_run` on its background thread) drives
the **agent** — the `BasePort` implementation, a cross-process shell-out to the base CLI
(`claude`, codex, …) per ADR 0001.

Constraints inherited from prior ADRs, which this contract must respect rather than
reinvent:

- **Polling, not SSE** (ADR 0002 §3). tiny_http is blocking and there is no async runtime.
  Progress is an append-only `events.jsonl` + a `status.json`, read by `GET
  …/events?since=N`. This contract uses that same mechanism; it does **not** introduce SSE.
- **One orchestration path** (ADR 0002 §2, R3). CLI and web both call `ops`. The agent
  boundary is defined once, in `ops`, not duplicated per front-end.
- **Isolation** (ADR 0003). The agent writes only to its worktree; it cannot reach the
  remote.

## Decisions

1. **Start a run through `ops::start_run`.** Front-ends (CLI, `POST /api/runs`) pass intent
   + base + forge + a session worktree branch. `ops` allocates the worktree (ADR 0003 §2),
   invokes the `BasePort`, and returns a `runId`. Idempotency: a repeated client key
   returns the existing `runId` — a retried request never spawns a duplicate run.

2. **The agent emits an append-only event log; the orchestrator owns the file.** The base
   process reports progress as ordered events the orchestrator appends to `events.jsonl`
   with a monotonic `seq`; `status.json` holds the latest rollup. Event shape:

   ```jsonc
   { "seq": 12, "ts": "…", "type": "file.changed",  "data": { "path": "…", "op": "modify" } }
   { "seq": 13, "ts": "…", "type": "test.result",   "data": { "suite": "…", "passed": 42, "failed": 0 } }
   { "seq": 14, "ts": "…", "type": "spec.updated",   "data": { "from": 7, "to": 8 } }
   { "seq": 15, "ts": "…", "type": "build.ready",    "data": { "commit": "<sha-in-worktree>" } }
   { "seq": 16, "ts": "…", "type": "run.completed",  "data": { "status": "succeeded",
                                                               "diffRef": "<sha>", "specVersion": 8 } }
   ```

3. **Observe via the existing cursor, `GET …/events?since=N`.** The browser polls from its
   last-seen `seq`; reconnect is resumable by construction (this is ADR 0002's mechanism,
   not a new one). The live diff, test results, and spec evolution all render off this one
   stream.

4. **The agent proposes; it never publishes.** The `BasePort` process has no push
   credentials and no forge access. It writes only to its worktree and reports a `diffRef`
   (a commit on the session branch). Merge + attestation + push are **out of scope for this
   contract** — they belong to the trust-gate → attest path (`ops::signoff`), triggered by
   N-of-M human approval. Keeping publish out of the agent contract is what structurally
   prevents the agent from self-merging.

5. **Exactly one terminal event.** A run ends with a single `run.completed` (or
   `run.failed`). `status:"failed"` — including empty output or a timed-out base — is a
   failure, never a vacuous pass (R14). Cancellation (`ops::cancel`) is cooperative: the
   base checks the signal at safe points and still emits a terminal event.

## Consequences

- The web UI needs no new transport: the live diff/test/spec feed is the `?since=N` poll
  it already speaks (ADR 0002). No SSE, no async runtime added.
- The publish path has exactly one entrance (attest, behind the gate), so provenance and
  N-of-M sign-off cannot be bypassed from the agent side.
- The orchestrator must persist `events.jsonl` per run to keep polling resumable — already
  true today.
- Cost: cooperative cancellation requires the base wrapper to poll a signal and flush a
  terminal event; a hard kill must still leave a `run.failed` behind so the UI never hangs.

## Alternatives considered

- **SSE / WebSocket streaming with `Last-Event-ID`.** Rejected: contradicts ADR 0002 §3 —
  tiny_http is blocking and there is no async runtime. The `?since=N` poll already gives
  the same live feel and resumability with far less machinery.
- **Give the base push access and let it open the PR.** Rejected: collapses the ADR 0003
  plane split and lets the agent publish without the human gate — the exact failure the
  trust model exists to prevent.
- **A second orchestration path for the web agent, separate from `ops`.** Rejected: R3 /
  ADR 0002 §2 — one orchestration path; front-ends only format output.
