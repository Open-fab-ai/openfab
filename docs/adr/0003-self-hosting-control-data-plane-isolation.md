# ADR 0003 — Self-hosting: control/data-plane isolation

**Status:** proposed · **Date:** 2026-07-01 · **Builds on:** ADR 0002

## Context

We want OpenFab's own development loop to run *inside* OpenFab's web UI: a maintainer
types natural-language intent, the base agent changes OpenFab's own code, the change is
observed live, and on N-of-M sign-off it is merged, signed with provenance, and pushed.

Today (ADR 0002) a run executes on a **background thread inside the serving binary**, a
global lock serializes git-touching ops, and the browser polls `events.jsonl` /
`status.json`. That is safe when OpenFab operates on *some other* repo. It is unsafe the
moment the repo under edit **is OpenFab itself**: a self-edit that fails to compile, panics
on boot, or corrupts the working tree would take down the very binary — and therefore the
very UI — being used to review and approve it. The approval surface must not be able to
kill itself.

The central tension: the single-binary, one-command, air-gapped value-prop (ADR 0002)
vs. the need for a change under review to run somewhere it cannot harm the reviewer.

## Decisions

1. **Split the planes.** The **control plane** — the tiny_http server, the `ops` layer,
   the trust gate — always runs on the *released* OpenFab binary and never executes the
   code under edit. The **data plane** — the worktree being modified and the preview that
   boots it — runs as a *separate, disposable* process. A broken self-edit crashes only
   the data plane; the control plane stays up to reject it.

2. **Edit an isolated worktree, never the live tree.** Each self-hosting run gets its own
   `git worktree` on a session branch (`openfab/session-<id>`). The base agent writes only
   there. This reuses git's own isolation rather than copying the repo, and composes with
   the global-lock model (the live tree is never mutated by a run).

3. **Preview = a separate ephemeral process, not a hot-swap.** "Observe live" means the
   orchestrator boots the *changed* worktree as its own process behind a URL the UI
   iframes. The running server is never hot-patched. The preview process is torn down when
   the session ends; if it crashes, it is killed and the session retried.

4. **Merged self-changes are adopted on the next redeploy — discretely and versioned.**
   After sign-off, merge + attestation + push happen (ADR 0004 / attest path); the running
   control plane picks up the new code only via a subsequent redeploy. There is no live
   hot-swap of the serving binary. This keeps every self-improvement a discrete, attested,
   auditable version bump — consistent with the provenance moat.

## Consequences

- The UI is crash-immune to the change it is approving; the loop cannot brick its own
  approval surface. This is the whole point.
- Cost: self-hosting mode needs process/worktree isolation and a preview launcher — more
  moving parts than the in-process background thread. For operating on *other* repos, the
  simpler in-process model of ADR 0002 still applies; isolation is required specifically
  for the self-referential case.
- The one-command, single-binary story is preserved for the control plane; only the data
  plane is spawned out-of-process.
- A merged improvement is not live instantly — it waits for redeploy. Accepted: discrete
  versioning is better for provenance than continuous self-modification.

## Alternatives considered

- **Hot-patch the running binary (self-modify in place).** Rejected: chicken-and-egg — one
  bad edit takes down the UI mid-approval; there is no stable surface left to reject from.
- **Keep the ADR 0002 in-process background-thread model for self-hosting too.** Rejected:
  it works only because it operates on a *different* repo; pointed at OpenFab itself it
  couples run failure to server availability.
- **Full container per run instead of a git worktree.** Deferred, not rejected: a worktree
  is the lighter default and matches the sovereign/air-gapped ethos; a container is the
  production hardening swap when untrusted intent or stronger sandboxing is needed.
