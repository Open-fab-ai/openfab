# Phase 2 — Manual Verification Checklist

Everything implementable + headlessly verifiable is **done and green** (see status below).
The items here need a human at a live environment (Matrix/Robrix GUI, multi-agent runs) and
were deliberately deferred so they don't block development.

## Implemented & verified (no action needed)

- A2 ingest spec + requirements-in-provenance — `agent-spec lifecycle` 3/3 ✓
- A3 document bundle endpoints (`/api/runs/:id/docs`) — 2/2 ✓ + live console
- B1 Matrix mxid ↔ maintainer mapping (`/api/identity`, `resolve_signer`) — 3/3 ✓
- B2 Bridge approval relay (`POST /approve` + poller → OpenFab sign-off) — code + restart ✓
- B3 gate/provenance posted to room via `base.post()` (native agent-chat) ✓
- C1 stage pipeline (`/api/runs/:id/stages`) — 3/3 ✓ live
- C2 agent status (`/api/agents` proxied to Bridge) — 13 agents live ✓
- C3 tmux peek (`/api/agents/:name/peek`) — live ✓
- D1 board (`/api/board`) — live ✓
- D2 multi-spec graph (`/api/graph` via `agent-spec graph`) — live ✓
- Phase 2 console at **`/console`** (board · docs · stages · verify · agents · tmux · identity)
- Build: 79 unit tests, `clippy -D warnings` clean, `rustfmt` clean; 4/4 Phase-2 contracts
  pass `agent-spec lifecycle` (11 scenarios).

## Needs your hands (live)

- [ ] **Console visual pass**: open `http://127.0.0.1:8787/console` — confirm board lanes,
      click a run (stage pipeline + Docs viewer + Verify C1–C12 + Audit), Agents panel,
      click an agent → tmux peek, map an identity.
- [ ] **Requirements conversation (A1, live)**: launch the coordinator
      (`bin/agentchat up-v1 wf_coordinator claude …`; the `openfab-coordinator` skill is
      linked into `~/.claude/skills`). In Robrix, `/create-issue` and chat to clarify; it
      writes `specs/<id>.requirements.md` + `specs/<id>.spec.md` and posts "approve <id>".
- [ ] **Build from the conversation**: `OPENFAB_SPEC_FILE=specs/<id>.spec.md openfab build
      "<intent>" --base agent-chat …` (or Fabricate); confirm the requirements doc is
      committed and its hash appears in the attestation predicate (`requirements_sha256`).
- [ ] **Approval via Robrix (B2, live)**: map your Matrix user
      (`POST /api/identity {mxid:"@you:palpo", maintainer:"alice"}`), then in the room post
      `approve <run>`; the Bridge poller relays it to OpenFab sign-off. Confirm an **unmapped**
      user's `approve` is rejected (security).
- [ ] **Full Phase 2 e2e**: requirements chat → spec → implement (agent-chat) → verify →
      sign → Robrix `approve` (N-of-M) → merge, all visible in `/console`.

## Phase 2.1 enhancements (this round)

Implemented & verified:
- #1 **Project management on the dashboard** (create + select); console is select-only.
  88 tests; multi-project board/maintainer isolation verified.
- #2 **File upload** in the build input (`POST /api/upload`): a `.spec.md` builds directly
  (`build_with_spec_file`), a requirements doc is committed + hashed. `upload` lifecycle 3/3.
- #3 **Robrix room ↔ project binding + agent doc ingest**: `POST /api/rooms`, `POST /api/ingest`,
  `GET /api/incoming`; Bridge `POST /submit-doc`; coordinator skill submits. e2e verified
  (bind room → bridge submit → dashboard "Incoming from Robrix" shows it). room-binding 3/3.
- #4 **Console agent click → agent-chat Agent Monitor** (`:8084`) iframe + open-↗ link;
  `GET /api/config` provides the monitor URL; tmux-text fallback retained.

Self-hosting (dogfood OpenFab with OpenFab):
- Register a project pointing at an existing repo path (dashboard → Projects card). With the
  **"Isolate with a git worktree"** option (default on), OpenFab runs
  `git worktree add <projects_dir>/<name> -b openfab/<name>` so it works in a separate clean
  checkout — your live working tree is untouched. Verified: worktree created against the live
  OpenFab repo, main checkout stayed on `main`. `demo/.work/` is gitignored.
- Without the worktree option, OpenFab operates **in place** in the given repo (branches +
  commits there) — only do that on a clone/throwaway.

GitHub REST API forge (token-based, no `gh` CLI):
- Implemented: `RestForge` now covers GitHub via `api.github.com` (push with token URL, PR via
  `POST /repos/<slug>/pulls`). Gated by `OPENFAB_GITHUB_TOKEN` + `OPENFAB_GITHUB_REPO`; the
  forge dropdown shows "live — GitHub REST API (token)" and falls back to local honestly.
  93 tests; github-api lifecycle 3/3. Pure URL/remote/config builders are unit-tested.

Needs your hands (live):
- [ ] **GitHub live PR**: set `OPENFAB_GITHUB_TOKEN` (PAT with repo / Contents+PR write) +
      `OPENFAB_GITHUB_REPO=owner/repo`, Fabricate with forge=GitHub → confirm a real branch
      push + PR appears on GitHub, and the PR url is recorded in the run.
- [ ] **Self-host run**: register `selfdev` (OpenFab repo, worktree on); run a small spec
      against OpenFab itself (e.g. `specs/openfab-selfdev.spec.yaml`) → verify → sign → gate.
- [ ] **#3 live**: launch `wf_coordinator`, bind its room (`POST /api/rooms {room,project}`),
      chat requirements; confirm the coordinator's `/submit-doc` lands in the dashboard's
      "Incoming from Robrix" for the bound project.
- [ ] **#4 visual**: in `/console`, click an agent → the agent-chat Agent Monitor renders in
      the panel (and "open ↗" opens it full). Confirm the iframe loads (same host).
- [ ] **Per-project spec dir**: building an *incoming* spec uses `OPENFAB_SPEC_DIR` (global).
      For non-default projects, point the build at the project's `specs/` (follow-up; default
      project works as-is).

## Deferred by design

- [ ] **B4 — Robrix native approval UI (Makepad)**: approve/sign buttons + N-of-M progress
      bar as native widgets. Deferred per the demo's "robrix source unchanged" principle and
      because Makepad can't be built/verified in this environment. The additive patch is
      small and documented: add `/approve`, `/reject` to `WORKFLOW_SLASH_COMMANDS`
      (`robrix2/src/shared/mentionable_text_input.rs`, the `#[cfg(feature="agent_chat")]`
      block) — the slash-command + posted-text-card path (B2/B3) already delivers the
      function without touching robrix.

## Running services (started during development)

- Palpo (docker), agent-chat backend `:8090` + bridge-matrix + push-relay (pre-existing)
- OpenFab↔agent-chat Bridge `:8077` (`bridge/openfab-agentchat-bridge.mjs`, with approval relay)
- OpenFab dashboard/console `:8787` (agent-spec mode, agent-chat native)
- `wf_implementer` agent online (`openfab-implementer` skill)
