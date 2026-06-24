# OpenFab Phase 2 Roadmap — Collaborative Software-Factory Console

Builds on the Phase 0/1 integration (see `robrix2-agentchat-integration.md`). Turns the
single-input-box "fabricate" into a collaborative console: requirements conversation,
document engineering, human review/approval/sign-off via Robrix (Matrix), and observability
(process / agent / tmux).

## Decisions (2026-06)

- **Two front-ends, clear split**:
  - **Robrix** (Matrix-native): requirements chat (with `wf_coordinator`), approve / sign
    (= N-of-M), agent status, tmux monitoring, real-time human↔agent collaboration.
  - **OpenFab dashboard** (web): document engineering (requirements / spec / design / code),
    process-detail (stage pipeline), project management (runs/issues board).
- **Requirements conversation**: chat with `wf_coordinator` in Robrix → produces
  `requirements.md` + `.spec.md`, human approves → OpenFab ingests and builds.
- **Human sign-off**: Matrix mxid ↔ OpenFab maintainer DID mapping; `approve`/`sign` in a
  Robrix room = `ops::signoff` via the Bridge.
- **Scope**: all of it, dependency-ordered.

## Architecture (two front-ends + the Bridge as connective tissue)

```
  Human ── Robrix (Matrix)                         OpenFab Dashboard (web)
        · requirements chat (wf_coordinator)       · requirements/spec/design/code viewers
        · approve / sign (= N-of-M)                · process detail (stage pipeline)
        · agent status / tmux monitor              · project management (runs/issues board)
              │ Matrix msgs                              ▲ JSON API
              ▼                                          │
   agent-chat(:8090) ──┐                          OpenFab serve(:8787)
   coordinator/         │   OpenFab↔agent-chat     author_from_md / signoff / build /
   implementer/...      └── Bridge(:8077) ───────── verify / events / docs
                            + identity map + approval relay
```

## Phases (dependency-ordered)

### Phase A — Requirements conversation + document engineering (input end, foundation)
| # | Task | Where | Deps |
|---|------|-------|------|
| A1 | `wf_coordinator` skill: multi-turn requirements chat → emit `requirements.md` + `.spec.md`, post for approval | agent-chat skill + robrix | — |
| A2 | OpenFab **ingest** a pre-authored `.spec.md` + requirements doc (reuse `author_from_md`; Bridge `POST /spec`; commit `requirements.md`; hash it into the attestation) | Bridge + ops/core | A1 |
| A3 | Dashboard **document viewers**: rich `.spec.md` (BDD/Decisions/Boundaries) + requirements + code + design; `/api/runs/{id}/contract`, `/docs` | dashboard + server | A2 |

### Phase B — Robrix approval / sign-off → OpenFab trust gate (trust end)
| # | Task | Where | Deps |
|---|------|-------|------|
| B1 | **Identity mapping**: Matrix mxid ↔ maintainer DID registry (`/api/identity`); only mapped + authenticated mxids may sign | OpenFab + Bridge | A2 |
| B2 | Bridge **approval relay**: room `approve`/`reject`/`sign <run>` → `ops::signoff` as mapped maintainer (incl. reject path) | Bridge | B1 |
| B3 | OpenFab posts **gate/provenance/N-of-M state** back into the room; Robrix uses slash commands + text cards first | Bridge + robrix (light) | B2 |
| B4 | (optional, heavy) Robrix **native approval UI**: approve/sign buttons + N-of-M progress (Makepad) | robrix2 GUI | B3 |

### Phase C — Observability (process / agent / tmux)
| # | Task | Where | Deps |
|---|------|-------|------|
| C1 | Dashboard **stage-pipeline view**: requirements→spec→implement→verify→sign→gate (from events) | dashboard | A3 |
| C2 | **Agent status panel**: surface agent-chat `/api/agents` (online/idle/blocked/model) | Bridge + both | — |
| C3 | **tmux monitoring**: live agent session peek (`cowork-tmux-peek` / capture-pane) | Bridge + front-end | C2 |

### Phase D — Project management
| # | Task | Where | Deps |
|---|------|-------|------|
| D1 | **Board**: issues/runs (needs→spec→implementing→review→blocked→merged), link run↔spec↔PR↔provenance | dashboard | A3,C1 |
| D2 | **Multi-spec/project orchestration**: reuse `agent-spec graph` dependency DAG + critical path | dashboard + agent-spec | D1 |

## Sequence & first milestone

**A → B → C∥D.** First milestone (A2+A3 core, A1 skill): chat requirements with the
coordinator → `requirements.md` + `.spec.md` → approve → OpenFab ingests + fabricates →
dashboard shows requirements/spec/code. Replaces the tiny input box with conversational
requirements + document engineering.

## Risks

1. **Makepad GUI is the heaviest** (B4, native panels): slow to iterate, hard to verify
   here, and the demo principle is "robrix source unchanged." Mitigation: slash commands +
   posted text cards first; native widgets later.
2. **Sign-off trust assumption** (B1): "Matrix user approved → sign with their maintainer
   key" relies on Matrix auth + mapping. Only mapped + authenticated mxids may sign; a bare
   `approve` from an unmapped room member must NOT sign. Document the threat model.
3. **Requirements in the trust chain** (A2): hash `requirements.md` into the attestation
   (like `spec_contract_sha256`) so requirements→spec→code is fully traceable.

## Build conventions (this phase is spec-driven)

- OpenFab-core features are authored as agent-spec `.spec.md` contracts under `specs/phase2/`
  and verified with `agent-spec lifecycle` (Filter-only test selectors bound to `#[test]`s).
- Bridge (Node) and web are specified as design contracts + implemented with targeted tests
  (`node --check`, API smoke); `cargo`/lifecycle does not cover them.
- Robrix native GUI and live multi-agent runs are deferred to the manual checklist.
