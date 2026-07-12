# Phase 1 Software Factory Plan

Status: planning draft
Date: 2026-07-09
Scope: OpenFab + agent-chat + Robrix + Palpo

This document defines the Phase 1 implementation plan for the internal software
factory stack. The goal is not yet a 10,000-person enterprise rollout. Phase 1
should produce a production-shaped, self-hosted system that a small internal
team can use repeatedly, with clear ownership boundaries and an upgrade path to
larger multi-user deployments.

## 1. Product Goal

Phase 1 should let a developer or project owner work from Robrix, create a
Matrix project room, invite software-factory agents, discuss requirements, turn
the confirmed requirements into machine-readable inputs, produce a spec, run
implementation and review agents through agent-chat, and optionally ask OpenFab
to certify the result.

The default path remains direct Robrix + agent-chat project delivery. OpenFab is
an optional trust, provenance, verification, and release-certification layer. It
must not block ordinary project work unless a project explicitly opts into an
OpenFab gate.

## 2. Phase 1 Non-Goals

- No Kubernetes worker pool yet.
- No 10,000-user high-availability deployment yet.
- No mandatory OpenFab sign-off for every commit or PR.
- No replacement of agent-chat with agentd in this phase.
- No ARC design/implement agent loop adoption in this phase.
- No public Matrix federation requirement in this phase.
- No multi-tenant billing or external customer isolation in this phase.

agentd remains the future native runtime direction. Phase 1 should reduce tmux
fragility and make agent lifecycle serviceable, but it does not need to complete
the agentd replacement.

## 3. Success Criteria

Phase 1 is done when the following end-to-end scenario works from a clean local
or small-server deployment:

1. Palpo starts as the Matrix homeserver with deterministic local configuration.
2. agent-chat starts backend, dashboard, Matrix bridge, and worker agents through
   supervised services instead of hand-managed tmux commands.
3. Robrix can log in to Palpo, create a project room, and invite an agent-chat
   coordinator or implementer.
4. The bridge bot can observe the room without the user waiting for manual bot
   joins.
5. A Robrix room command such as "create issue" reaches the relevant agent-chat
   agent.
6. The coordinator can conduct requirements discussion and, after human
   confirmation, produce:
   - `specs/<id>.requirements.md`
   - `requirements/requirements.yaml`
   - `specs/<id>.spec.md`
7. agent-chat can dispatch implementation and review work to the selected
   agents.
8. Code can still be committed or proposed by the Robrix + agent-chat workflow
   without mandatory OpenFab sign-off.
9. OpenFab can optionally import the result, verify/spec-check it, produce
   provenance, and publish a certification summary back to the room.
10. A doctor or health command can explain failures across Palpo, Robrix,
    agent-chat, and OpenFab without reading raw logs first.

## 4. System Architecture

```text
Robrix desktop UI
  |
  | Matrix login, rooms, commands, approvals
  v
Palpo Matrix homeserver
  |
  | Matrix room events, membership, appservice/bot accounts
  v
agent-chat Matrix bridge
  |
  | normalized commands and agent events
  v
agent-chat backend + agent runtime
  |
  | project tasks, implementation, review, repository operations
  v
project repository

OpenFab bridge/server is optional:

agent-chat / Robrix result
  |
  | import / certify / verify / provenance request
  v
OpenFab
  |
  | attestation, badge, policy result, room summary
  v
Robrix Matrix room
```

The Matrix room is the collaboration and command surface. agent-chat owns agent
execution and team workflow. OpenFab owns optional trust artifacts and release
certification. Robrix owns the user experience. Palpo owns Matrix identity,
rooms, persistence, and deployment configuration.

## 5. Ownership Boundaries

| Component | Owns | Must Not Own |
| --- | --- | --- |
| Robrix | Project room UX, Matrix login, room creation, agent invite flow, issue/spec/approval views, optional OpenFab actions | Agent execution, OpenFab policy decisions, Matrix server internals |
| Palpo | Matrix homeserver, room membership, appservice registration, bot accounts, persistence, backups, local/team deployment profile | Agent workflow logic, project build policy, Robrix UI state |
| agent-chat | Agent registry, task routing, Matrix bridge, backend API, dashboard, agent lifecycle, implementation/review workflow | OpenFab trust policy, Robrix UI, Matrix homeserver ownership |
| OpenFab | Spec/provenance/signing/conformance/policy, optional import/certify path, base-agnostic ports | Mandatory project workflow, direct Matrix or agent-chat core dependency |

OpenFab core remains base-agnostic. Matrix, agent-chat, and Robrix details must
stay in adapters, bridge code, or integration documentation rather than leaking
into OpenFab core.

## 6. Data Flow

### 6.1 Project Creation

1. User creates or selects a project in Robrix.
2. Robrix creates a Palpo Matrix room with project metadata.
3. Robrix stores the mapping between room, project, repository, and optional
   OpenFab workspace.
4. Robrix invites the selected `@ac_*` agent and best-effort invites the bridge
   observer bot for that room.
5. agent-chat bridge observes membership and room events, then binds the Matrix
   room to an internal project/team context.

### 6.2 Requirements to Spec

1. User discusses requirements with the coordinator in the Robrix room.
2. The coordinator writes a human-readable requirements draft:
   `specs/<id>.requirements.md`.
3. After explicit human confirmation, the coordinator writes the structured ARC
   compatible input: `requirements/requirements.yaml`.
4. The coordinator compiles or derives the implementation spec:
   `specs/<id>.spec.md`.
5. The spec becomes the contract for implementation and review agents.

The ordering matters. `requirements.yaml` should not appear as an authoritative
input until the conversation has converged and the user has confirmed the
requirements.

### 6.3 Implementation and Review

1. agent-chat dispatches the spec to implementer agents.
2. Implementers modify the project repository and report task results.
3. Review agents inspect the change, tests, and spec fit.
4. The workflow can commit, open a PR, or prepare a handoff according to the
   project policy.
5. OpenFab is not required for this step unless the project opted into an
   OpenFab gate.

### 6.4 Optional OpenFab Certification

1. User or project policy requests OpenFab certification.
2. OpenFab imports the spec, result, repository state, and provenance inputs.
3. OpenFab runs verification, policy, and attestation generation.
4. OpenFab publishes a certification summary and badge status back through the
   bridge.
5. If gate mode is `none`, failure is reported but does not block the Robrix +
   agent-chat workflow.

## 7. Required Project Changes

### 7.1 OpenFab

OpenFab should be refined as an optional certification backend for this workflow.

Required changes:

- Keep Robrix + agent-chat direct delivery as the default path.
- Keep OpenFab gate mode configurable, with `none` as the integration default.
- Expose an import/certify endpoint suitable for agent-chat or Robrix to call
  after implementation.
- Ensure provenance attribution uses repository truth, not only bridge-supplied
  claims.
- Provide stable task/build identity checks for room-originated build or certify
  requests.
- Provide a concise badge or certification result that Robrix can render.
- Add a doctor command or health endpoint that checks:
  - OpenFab server availability.
  - bridge URL reachability.
  - repo path validity.
  - policy file readability.
  - signing/provenance prerequisites.
- Keep all Matrix and agent-chat coupling outside OpenFab core.

OpenFab Phase 1 deliverables:

- `openfab serve` integration profile for local/team use.
- Optional certify/import API documented for agent-chat and Robrix.
- Console or API status for `gate=none`, `gate=enforce`, and failed
  certification.
- End-to-end fixture showing direct project delivery plus optional certification.

### 7.2 agent-chat

agent-chat should become serviceable and reliable without jumping directly to
Kubernetes.

Required changes:

- Move from manual tmux startup to supervised services:
  - local machine: launchd on macOS or systemd on Linux.
  - small team server: Docker Compose.
  - keep Kubernetes for Phase 2 worker pools.
- Define lifecycle operations for backend, dashboard, bridge, relay, and worker
  agents:
  - start.
  - stop.
  - restart.
  - status.
  - log tail.
  - health check.
- Make worker agent instances explicit runtime records instead of ad hoc shell
  sessions.
- Preserve enough runtime state to avoid duplicate room command replay after
  restart.
- Use SSE or equivalent event streaming for dashboard and bridge updates where
  polling causes stale or replayed events.
- Enforce Matrix trust defaults:
  - trusted inviter allowlist.
  - ignored sender list for appservice bots.
  - mention-only behavior for auto-started gateway agents unless configured
    otherwise.
- Ensure bridge bot room observation is automatic after a Robrix room invite.
- Expose an agent roster API that Robrix and the dashboard can show reliably.
- Add doctor output for:
  - backend reachability.
  - dashboard reachability.
  - bridge Matrix login.
  - bridge joined rooms.
  - worker agent status.
  - configured remote backend.

agent-chat Phase 1 deliverables:

- `compose.yml` or equivalent service profile for backend, dashboard, bridge,
  relay, and workers.
- launchd/systemd templates for local developer machines.
- `agentchat doctor` or documented health endpoint.
- Matrix bridge tests for room command routing, ignored senders, trusted
  inviters, and duplicate replay prevention.
- Dashboard agent list fixed and verified at `http://127.0.0.1:8084/`.

### 7.3 Robrix

Robrix should be the cockpit for project-room work, not just a generic Matrix
client.

Required changes:

- Add a project-room creation flow that captures:
  - project name.
  - repository path or forge URL.
  - selected coordinator/implementer/reviewer agents.
  - optional OpenFab workspace.
- When inviting an `@ac_*` agent, also best-effort invite the bridge observer
  bot so agent-chat can see room events.
- Show agent availability and room membership clearly:
  - invited.
  - joined.
  - reachable by bridge.
  - currently working.
  - failed/offline.
- Provide first-class room actions:
  - create issue.
  - confirm requirements.
  - generate spec.
  - dispatch implementation.
  - request review.
  - optional OpenFab certify.
- Render artifacts in the room UI:
  - requirements draft.
  - `requirements.yaml`.
  - spec.
  - plan.
  - implementation result.
  - review result.
  - OpenFab badge/certification summary.
- Avoid making OpenFab sign-off look mandatory in the default project flow.
- Provide clear failure states when Palpo, agent-chat, or OpenFab is not
  reachable.

Robrix Phase 1 deliverables:

- Project-room wizard.
- Agent invite and bridge-bot auto-observer integration.
- Agent roster/status view.
- Issue/spec/action affordances backed by Matrix events.
- Optional OpenFab certification action and badge display.
- Local demo profile pointing at the Palpo + agent-chat + OpenFab stack.

### 7.4 Palpo

Palpo is the Matrix foundation for Phase 1. In the current local workspace the
Palpo deployment appears through Robrix deployment files rather than a separate
Palpo repository, so this plan treats Palpo changes as deployment and
configuration work unless a separate Palpo source tree is provided.

Required changes:

- Provide a deterministic local/team deployment profile:
  - homeserver URL.
  - server name.
  - admin/bootstrap user.
  - appservice registration.
  - persistent data volume.
  - logging.
- Define bot/appservice accounts for:
  - agent-chat bridge observer.
  - gateway agents if needed.
  - Robrix development/test users.
- Document local networking assumptions:
  - host to container access.
  - Robrix desktop access.
  - bridge access to homeserver.
  - appservice callback URL.
- Provide backup and reset commands for development and small-team use.
- Define when federation is disabled, internal-only, or enabled.
- Expose health checks that Robrix and agent-chat doctor commands can consume.

Palpo Phase 1 deliverables:

- Compose profile or equivalent local deployment under the Robrix deployment
  area.
- Appservice registration templates for agent-chat bridge integration.
- Environment example for local and small-team server deployments.
- Health and troubleshooting runbook.

## 8. Cross-Project Contracts

### 8.1 Matrix Identity and Naming

Recommended Phase 1 conventions:

- Human users: normal Palpo Matrix users.
- agent-chat agents: `@ac_<role>:<server>`.
- bridge observer bot: stable dedicated Matrix user, for example
  `@agent-bridge:<server>`.
- appservice bots such as Octos must be listed in agent-chat ignored senders
  when they share rooms with agent-chat.

### 8.2 Room Metadata

Every software-factory project room should have a durable mapping:

- Matrix room id.
- Project id.
- Repository path or forge URL.
- Selected agent team.
- Optional OpenFab workspace/build id.
- Current workflow stage.

The mapping may initially live in agent-chat and Robrix local state, but the
contract must be explicit so it can move to a shared service later.

### 8.3 Command Vocabulary

Phase 1 should standardize these room-level commands/events:

- `create_issue`
- `requirements_draft`
- `requirements_confirmed`
- `requirements_yaml_ready`
- `spec_ready`
- `implementation_requested`
- `implementation_result`
- `review_requested`
- `review_result`
- `openfab_certify_requested`
- `openfab_certification_result`

The Matrix text UX can remain natural-language friendly, but the bridge should
normalize commands into structured events internally.

### 8.4 Artifact Paths

Recommended project artifact paths:

- `specs/<id>.requirements.md`
- `requirements/requirements.yaml`
- `specs/<id>.spec.md`
- `specs/<id>.plan.md`
- `specs/<id>.review.md`
- `artifacts/openfab/<id>/`

`requirements.yaml` is produced only after requirements confirmation.

### 8.5 OpenFab Gate Modes

Phase 1 should support these modes:

- `none`: report only. Default for Robrix + agent-chat integration.
- `warn`: publish failed certification but allow continuation.
- `enforce`: block release/merge actions that are explicitly routed through
  OpenFab.

Direct Robrix + agent-chat project delivery should not accidentally inherit
`enforce`.

## 9. Implementation Workstreams

### Workstream A: Palpo Deployment Foundation

Repositories:

- Robrix deployment area.
- Palpo source repository if provided later.

Tasks:

1. Normalize Palpo local compose configuration.
2. Add appservice registration template for agent-chat bridge.
3. Add bootstrap script for users and bot accounts.
4. Add health check and reset commands.
5. Document host/container networking for Robrix and bridge clients.

Verification:

- Fresh Palpo start succeeds.
- Robrix can log in.
- agent-chat bridge can log in.
- bridge observer bot can be invited to a room.
- appservice callbacks work if enabled.

### Workstream B: agent-chat Service Runtime

Repositories:

- agent-chat.

Tasks:

1. Add supervised local service definitions for backend, dashboard, bridge, and
   relay.
2. Add Docker Compose profile for small-team server deployment.
3. Represent worker agents as lifecycle-managed runtime records.
4. Add status and doctor commands for all services.
5. Persist processed Matrix event ids to prevent duplicate command replay.
6. Fix dashboard agent roster source of truth.
7. Add tests for Matrix routing, ignored senders, trusted inviters, and replay
   behavior.

Verification:

- Services survive restart.
- Dashboard shows the expected agent list.
- Room command reaches the correct agent once.
- Ignored appservice bot messages are not routed to worker agents.
- Trusted inviter enforcement works in both allowed and denied cases.

### Workstream C: Robrix Project Cockpit

Repositories:

- robrix2.

Tasks:

1. Add or refine project-room creation flow.
2. Bind room to repository and selected agent team.
3. Auto-invite bridge observer bot when inviting agent-chat agents.
4. Show agent membership and reachability state.
5. Add room actions for issue/spec/implementation/review/certify.
6. Render generated artifacts and OpenFab badge state.
7. Add user-visible failure states for missing Palpo, bridge, or OpenFab.

Verification:

- User can create a room, invite agents, and see agent status.
- `create_issue` from Robrix reaches agent-chat.
- Requirements confirmation can trigger structured artifact generation.
- Optional OpenFab certification result appears in the room.

### Workstream D: OpenFab Optional Certification

Repositories:

- openfab.

Tasks:

1. Keep `gate=none` as the default for Robrix + agent-chat integration.
2. Document and stabilize import/certify API between agent-chat/Robrix and
   OpenFab.
3. Verify provenance from repository state and generated artifacts.
4. Add room/build identity checks for certification requests.
5. Add badge/certification result format for Robrix.
6. Add doctor coverage for repo, policy, bridge, and signing prerequisites.
7. Add integration fixture for optional certification after direct agent-chat
   delivery.

Verification:

- Direct Robrix + agent-chat commit/PR path works without OpenFab.
- Optional OpenFab certification succeeds on a known-good fixture.
- Failed certification reports clearly without blocking when gate is `none`.
- OpenFab core remains free of Matrix and agent-chat types.

### Workstream E: End-to-End Demo and Runbook

Repositories:

- openfab.
- agent-chat.
- robrix2.
- Palpo deployment area.

Tasks:

1. Create one canonical demo environment file set.
2. Create a start/stop runbook.
3. Add an acceptance checklist.
4. Add troubleshooting entries for the common failures:
   - duplicate tmux/session names during transition.
   - dashboard cannot see agents.
   - bridge bot not in room.
   - `create_issue` not routed.
   - OpenFab doctor command missing or failing.
   - Palpo appservice registration mismatch.
5. Record known ports, service names, and log locations.

Verification:

- A fresh machine can follow the runbook and complete the Phase 1 success
  scenario.
- Each service has an explicit owner and log location.
- Failures can be diagnosed through doctor/health commands before raw log
  inspection.

## 10. Suggested Execution Order

1. Palpo local/team deployment baseline.
2. agent-chat supervised service profile and doctor.
3. agent-chat Matrix routing reliability:
   - trusted inviter.
   - ignored senders.
   - bridge bot observation.
   - duplicate replay prevention.
4. Robrix room creation, agent invite, and agent status UX.
5. Requirements confirmation to `requirements.yaml` and spec generation.
6. OpenFab optional certify/import API and badge result.
7. Cross-project end-to-end demo runbook.
8. Hardening pass:
   - logging.
   - health checks.
   - secrets handling.
   - restart behavior.

This order keeps the communication substrate stable before improving UX and
before adding optional certification.

## 11. Phase 1 Configuration Matrix

| Area | Example Setting | Owner |
| --- | --- | --- |
| Matrix homeserver URL | `MATRIX_HOMESERVER` | Palpo / agent-chat |
| Matrix server name | `MATRIX_SERVER_NAME` | Palpo |
| Bridge bot user | `MATRIX_BOT_USERNAME` | agent-chat / Palpo |
| Trusted inviters | `MATRIX_TRUSTED_INVITER_MXIDS` | agent-chat |
| Trust mode | `MATRIX_TRUST_MODE=enforce` | agent-chat |
| Ignored senders | `MATRIX_IGNORED_SENDER_MXIDS` | agent-chat |
| agent-chat backend URL | `AGENTCHAT_BACKEND_URL` | agent-chat / Robrix |
| agent-chat dashboard URL | local/team dashboard config | agent-chat |
| OpenFab URL | `OPENFAB_URL` or integration config | OpenFab / Robrix |
| OpenFab gate | `none`, `warn`, `enforce` | OpenFab |
| Project repo path | room/project metadata | Robrix / agent-chat |

Secrets must stay in environment files or secret stores and must not be
committed.

## 12. Acceptance Checklist

Use this checklist before declaring Phase 1 complete:

- [ ] Palpo starts from documented local/team profile.
- [ ] Robrix can log in to Palpo.
- [ ] agent-chat backend starts under supervision.
- [ ] agent-chat dashboard starts under supervision.
- [ ] agent-chat Matrix bridge starts under supervision.
- [ ] worker agents can be started, stopped, and inspected.
- [ ] Robrix can create a project room.
- [ ] Robrix can invite an `@ac_*` agent.
- [ ] Bridge observer bot joins or is invited automatically.
- [ ] `create_issue` reaches the intended agent.
- [ ] Coordinator can produce requirements draft.
- [ ] Human confirmation is required before `requirements.yaml`.
- [ ] `requirements/requirements.yaml` is generated.
- [ ] `specs/<id>.spec.md` is generated.
- [ ] Implementer receives the spec and produces a result.
- [ ] Reviewer can review the result.
- [ ] Direct commit or PR path works without OpenFab sign-off.
- [ ] Optional OpenFab certification works on the same result.
- [ ] Failed OpenFab certification does not block when gate is `none`.
- [ ] Robrix displays OpenFab certification status when available.
- [ ] Doctor/health output identifies missing services and misconfiguration.

## 13. Risks and Mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Bridge bot is not in the room | Commands are not observed | Robrix best-effort invites bridge bot whenever an agent-chat agent is invited; agent-chat doctor lists unobserved rooms |
| Duplicate Matrix event replay | Agents perform the same command twice | Persist processed event ids and test restart replay behavior |
| OpenFab appears mandatory | Users think direct delivery is blocked | Default gate `none`; Robrix labels certification as optional |
| tmux transition creates mixed runtime state | Services are hard to diagnose | Standardize supervised services and provide explicit stop/status commands |
| Appservice bots trigger agent-chat agents | Spurious commands or loops | Configure ignored senders and test shared-room behavior |
| Palpo deployment details drift | Robrix and bridge cannot connect reliably | Keep one canonical env/runbook and health check |
| Provenance uses bridge claims only | Certification can be misleading | OpenFab derives provenance from git/repository state where possible |
| Room/project mapping splits across tools | Commands target wrong repo or team | Define durable room metadata contract and display it in Robrix |

## 14. Deliverables by Repository

### OpenFab

- Optional certification API and runbook.
- Gate-mode documentation for Robrix + agent-chat.
- Badge/certification result format.
- Doctor coverage for integration prerequisites.
- End-to-end fixture for optional certification.

### agent-chat

- Supervised service profile.
- Worker lifecycle records and status.
- Reliable Matrix bridge routing.
- Dashboard agent roster fix.
- Doctor command.
- Routing and replay tests.

### Robrix

- Project-room wizard.
- Agent invite plus bridge observer invite.
- Agent status view.
- Workflow actions for issue/spec/implement/review/certify.
- Artifact and badge rendering.
- Local demo configuration.

### Palpo

- Deterministic local/team Matrix deployment profile.
- Appservice/bot registration templates.
- Bootstrap/reset/backup commands.
- Health endpoints or health scripts.
- Networking and troubleshooting documentation.

## 15. Source References

Primary local references:

- `docs/OpenFab_MVP_Design_and_PRD.md`
- `docs/robrix2-agentchat-integration.md`
- agent-chat `README.md`
- Robrix `docs/robrix-with-agentchat/`
- Robrix `palpo-and-octos-deploy/`

Relevant remembered decisions:

- OpenFab sign-off is optional for Robrix + agent-chat direct workflows.
- Robrix should best-effort invite the agent-chat bridge observer bot when
  inviting `@ac_*` agents.
- agent-chat should ignore external appservice bots such as Octos in shared
  rooms.
- ARC integration should be limited to requirements-to-spec input generation in
  this phase; agent-chat remains responsible for implementation/review
  convergence.
- agentd is the later native runtime direction, not a Phase 1 dependency.
