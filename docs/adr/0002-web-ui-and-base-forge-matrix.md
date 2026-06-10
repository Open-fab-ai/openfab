# ADR 0002 — Web UI + the full base/forge matrix (v0.2)

**Status:** accepted · **Date:** 2026-06-09 · **Supersedes parts of:** ADR 0001 scope

## Context

After v0.1 (CLI engine + provenance/trust moat), the ask was: a presentable **web UI**,
the **whole architecture** implemented (BasePort across all 4 reference bases, ForgePort
across all 4 forges) so a base/forge can be swapped to *demonstrate* the claims, and a
**full visual UX** (NL → live workflow → approve/refine → product + provenance →
reproduce/verify for sovereignty).

## Decisions

1. **Web server inside the Rust binary (tiny_http), UI embedded via `include_str!`.** No
   async runtime, no JS build step, no separate frontend service — one static binary
   serves both API and SPA. This matches OpenFab's single-binary / air-gapped / sovereign
   value-prop and keeps the demo a one-command launch. Vanilla JS/CSS (no framework) for
   the same reason. Trade-off: hand-rolled UI vs. a component library — accepted for
   zero-install.

2. **Shared `ops` layer.** Both the CLI and the API call `ops::{start_run, signoff,
   verify, feedback, reproduce, artifacts}`. There is exactly one orchestration code path
   (R3); the front-ends only format output.

3. **Live progress by event-stream polling, not SSE.** The cycle appends timeline events
   to `events.jsonl` and a `status.json`; the browser polls `…/events?since=N`. Simpler
   and more robust than SSE under tiny_http's blocking model, with the same live feel.
   Runs execute on a background thread; a global lock serializes git-touching ops.

4. **All 5 bases + 4 forges are real adapters; honest native/bridged + live/local badges
   (R14).** There is no `mock` base — every artifact must come from a real LLM. Truly
   running HiClaw/AgentScope/agent-chat/OpenHands needs their external servers; running
   live GitHub/Forgejo/Gitea/GitCode needs accounts. Rather than fake them or omit them,
   each adapter:
   - **base:** dispatches to its native runtime if `OPENFAB_<NAME>_URL` is set (real
     `native`); otherwise runs the task through OpenFab's LLM backend (`bridged`). The run
     records its true `base` + `runtime` in provenance and the UI badges it.
   - **forge:** uses its real REST/`gh` path if creds are set (`live`); otherwise a local
     git instance reporting that forge's kind (`local instance`) — which still proves the
     `ForgePort` seam + portable in-repo provenance.
   The four framework bases share **one** parameterized adapter (`base_framework`) since
   they share the dispatch contract — only metadata differs (R3). Likewise the three
   Gitea-lineage forges share `forge_rest`.

5. **LLM backend is pluggable too.** `claude` CLI by default; Qwen/DashScope via
   `OPENFAB_LLM=dashscope` (+ `DASHSCOPE_API_KEY`), reached by shelling to `curl` (no
   HTTP-client crate added).

6. **Reproduce = the sovereignty proof.** `ops::reproduce` re-verifies the signature,
   hashes each committed file against its signed digest (bit-identical source), and
   re-runs every acceptance check in the sandbox. Any single failure → NOT REPRODUCIBLE.

## Consequences

- One can demo "swap the base" and "swap the forge" across the full matrix today, offline,
  with every run producing valid, honestly-attributed provenance — and connect a real
  native runtime / live forge later with no code change.
- The honest fallback badges mean the demo is safe to present: nothing overstates what ran.
- Only one new dependency (`tiny_http`); the moat (Core, predicate, spec-cycle, trust)
  is unchanged — the UI and adapters sit entirely behind the existing seams.

## Follow-ups

- Native framework dispatch + live `forge_rest` PR path are implemented but untested
  against real servers (none in this env) — verify when one is connected.
- Split the files now over the 300-line budget (R4) in their own sessions (see HANDOFF).
- Playwright smoke test for the UI.
