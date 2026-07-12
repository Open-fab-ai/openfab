# Enterprise Software Factory Roadmap

- Status: proposed enterprise architecture; PRD/ADR ratification required
- Date: 2026-07-11
- Scope: Specify + ARC + Robrix + Palpo/Matrix + agent-chat transition + agentd + OpenFab

This document is the cross-system roadmap for a software factory that starts on
one developer machine, supports small teams, and can grow to an enterprise with
10,000 developers. It defines ownership, protocol boundaries, phase dependencies,
and acceptance gates. Repository-specific task identifiers belong in each
repository's implementation roadmap, not here.

The roadmap makes four architecture decisions explicit:

1. Specify is required in enterprise mode as the `ProjectAuthorityPort`
   implementation. Specify-web UX, implementation language, storage technology,
   and deployment topology are deferred decisions.
2. agentd owns execution control and workers. It replaces agent-chat by fixing
   the operational problems exposed by agent-chat rather than cloning its tmux
   implementation shape.
3. OpenFab retains fab/spec-cycle orchestration when it is invoked and owns
   independent verification, provenance, certification, release-policy results,
   and the governed Skill Hub. It does not own durable execution-control state.
4. ARC compiles confirmed requirements into a spec. It does not own project
   lifecycle and does not run implementation agents.

## 0. Current PRD Compatibility Gate

`docs/OpenFab_MVP_Design_and_PRD.md` remains the source of truth until amended.
It currently defines OpenFab as the spec-cycle orchestrator that dispatches work
through `BasePort`. The enterprise target preserves that public contract:

- an OpenFab-governed run is still started and orchestrated by OpenFab;
- agentd becomes the durable execution implementation reached through a
  `BasePort` adapter rather than a second OpenFab runtime state machine;
- OpenFab continues to validate and pin the execution spec it consumes, while
  Specify is the enterprise authority for the product requirement/spec lifecycle;
- a project may use Robrix + agentd without OpenFab when policy allows direct
  delivery, but that path does not silently gain OpenFab certification.

The proposed PRD amendment and ADR are recorded in
`docs/OpenFab_MVP_Design_and_PRD.md` and
`docs/adr/0003-agentd-execution-and-openfab-certification-boundary.md`. They do
not become approved merely because the files exist.

FSF-1 cannot exit until the PRD and relevant ADRs ratify this decomposition. Until
then, this document is a proposed enterprise evolution, not a claim about current
OpenFab behavior.

## 1. North Star

The product is a fiduciary-grade software delivery system:

```text
human intent
  -> approved requirement version
  -> compiled and reviewed spec version
  -> immutable project execution snapshot
  -> policy-constrained agent execution
  -> tests, review, and artifact evidence
  -> optional or policy-required independent certification
  -> accountable software delivery
```

Trust does not come from a model assertion. It comes from authoritative inputs,
versioned policy, isolated execution, independent verification, accountable
human decisions, and an evidence chain that can be reconstructed after every
service involved in the run has restarted.

## 2. Authoritative Roles

Each durable state class has one authoritative owner. Other systems may retain
immutable references, projections, or bounded caches, but cannot overwrite the
authoritative record.

| Role | Authoritative state | Must not own |
| --- | --- | --- |
| `SpecifyProjectAuthority` | Organization, team, project, repository binding, requirement/spec lifecycle, spec freeze, product workflow, project RBAC/quota intent, certification-policy intent, project-to-Matrix-room binding | Worker registry, execution leases, runtime sessions, checkpoints, transcripts, certification signatures |
| ARC | Deterministic transformation from approved requirements input to spec and traceability output | Project authority, spec approval/freeze, implementation, review convergence, runtime execution |
| `MatrixRobrixTransport` | Matrix identities, room protocol, human interaction, notifications, and Robrix UI state | Canonical project bindings, execution history, transcripts, artifacts, certification decisions |
| `AgentdMatrixGateway` | Matrix sync cursor, processed-event inbox, command normalization, command/run deduplication, and semantic execution summaries | Project authority, Matrix homeserver identity, run/task execution state, raw transcripts |
| `AgentdControlPlane` | Durable execution identity, agent/worker registry, dispatch, queue, leases, runtime-session records, checkpoints, execution artifact index, execution audit, measured quota usage | Project/spec authorship or freeze, product workflow authority, Matrix identity authority, certification signatures |
| `AgentdWorker` | Live process/PTY, bounded output ring, worktree, cache, and unacknowledged artifact spool on one host | Durable projects, specs, runs, leases, acknowledged artifacts, policy authorship |
| OpenFab fab + `OpenFabCertificationAuthority` | Invoked spec-cycle orchestration through `BasePort`/`ForgePort`; verification requests/results, signatures, provenance attestations, certification state, conformance results, and Skill Hub package trust state | Durable execution queues, runtime sessions, task leases, product workflow, or authoritative commit/PR state — an invoked spec cycle may still create commits/PRs through `ForgePort` without becoming their durable owner |
| Palpo / Matrix | Homeserver identity, room and event transport, media transport, appservice substrate | Project semantics, execution scheduling, certification policy |
| Robrix | Human cockpit over Matrix and factory APIs | Hidden execution state machines or duplicated project/certification stores |
| agent-chat | Transitional Phase FSF-0 runtime and Matrix workflow bridge | Long-term enterprise source of truth |

### 2.1 Deployment Modes

Standalone mode explicitly selects `LocalProjectAuthority`. It uses the same
stable identifiers and snapshot semantics as enterprise mode but does not imply
that a local cache is Specify.

Enterprise mode configures `SpecifyProjectAuthority`. When that authority is
unavailable, agentd fails closed for new project-context resolution and does not
silently fall back to local authority. Changing authority mode requires an
explicit import/rebind operation.

OpenFab remains optional unless the project policy snapshot requires a
certification result. Absence of OpenFab under `gate=none` does not transfer
certification authority to agentd and does not block direct delivery.

## 3. End-to-End Workflow

### 3.1 Requirements and Spec

1. A human discusses requirements in Robrix.
2. Specify records the requirement draft and its project context.
3. Explicit human confirmation creates an approved requirement version and a
   materialized `requirements/requirements.yaml` artifact.
4. ARC compiles that approved version into:
   - `specs/<id>.requirements.md`.
   - `specs/<id>.spec.md`.
   - `specs/<id>.arc.traceability.json`.
   - `specs/<id>.arc.compilation.json`, binding the requirements schema/version,
     normalized input digest, ARC build/image digest, compiler configuration,
     output digests, and reproducibility result.
5. Specify records the compiled spec as a draft, manages human review, and
   freezes an immutable spec version.
6. Git may contain the materialized requirement/spec files, but Specify owns
   their lifecycle and authoritative version references. The authority snapshot
   binds the Git commit/blob hashes to the approved versions.

ARC output is derived evidence. ARC does not call agentd as an implementation
loop, and OpenFab does not create a second requirements or execution state
machine.

### 3.2 Execution

1. Robrix sends an authorized command through Matrix transport and selects an
   allowed delivery profile: OpenFab-governed or direct agentd delivery.
2. `AgentdMatrixGateway` authenticates the enterprise principal, resolves the
   Specify-owned room/project binding, and creates a canonical `command_id`.
3. For an OpenFab-governed profile, OpenFab runs the spec cycle and dispatches
   through its agentd `BasePort` adapter. For a direct profile, the gateway calls
   agentd without claiming an OpenFab certification result.
4. agentd requests a versioned `ProjectExecutionSnapshot` through
   `ProjectAuthorityPort`.
5. The snapshot pins project, repository/base commit, room binding, frozen spec,
   RBAC policy, quota policy, model restrictions, and certification policy.
6. `AgentdMatrixGateway` and `AgentdControlPlane` atomically accept the
   `command_id`, create or return one run, and advance delivery state through a
   transactional inbox/outbox boundary.
7. `AgentdControlPlane` creates fenced task leases.
8. An authenticated worker pulls a lease, creates an isolated worktree, and
   runs agents inside the configured execution sandbox.
9. The worker submits explicit outcomes and content-addressed artifacts. stdout
   is runtime data, not an accepted workflow result.
10. The control plane records execution evidence and sends idempotent semantic
   summaries back to Specify and Matrix.

### 3.3 Verification and Delivery

Delivery and certification are separate decisions:

| State | Meaning |
| --- | --- |
| `produced` | agentd produced an artifact, but verification is incomplete |
| `delivered` | source/PR was delivered according to project delivery policy |
| `machine_attested` | machine checks and provenance recording passed; no human release approval is implied |
| `human_certified` | the configured human/N-of-M certification policy passed |
| `released` | the forge/release policy accepted the certified or explicitly allowed artifact |
| `revoked` | a prior attestation or release is no longer trusted |

`gate=none` permits direct delivery and optional machine attestation. It must not
be presented as human-certified. OpenFab's own repository and other protected
projects may prohibit `gate=none` through project policy and forge branch
protection.

When certification is requested:

1. agentd submits an immutable artifact/evidence reference and project policy
   reference through `CertificationPort`.
2. OpenFab verifies the signed evidence envelope, repository truth, and trusted
   builder identity.
3. Policy decides which acceptance checks must be independently re-run in an
   OpenFab-controlled sandbox.
4. OpenFab records the result, signature, attestation, policy version, and
   subject digests.
5. Specify and agentd store immutable references to the result. Neither may
   rewrite OpenFab certification state.

For policy-required certification, a forge admission/status-check adapter is the
release enforcer. It atomically checks the current Specify policy version, exact
subject digest, and valid OpenFab result before merge/release. Specify authors the
policy and OpenFab owns the result; neither needs to own the commit or pull
request.

## 4. Cross-System Contracts

### 4.1 Project Execution Snapshot

The snapshot is immutable and versioned. At minimum it carries:

- authority key and authority revision.
- organization/team/project identifiers.
- target repository and base commit.
- Matrix room binding and allowed command classes.
- approved requirement references.
- frozen spec version reference.
- product workflow reference.
- RBAC, quota, model, delivery, and certification policy versions.
- data classification, allowed worker trust domains/regions, execution image and
  cache-isolation policy.
- issue time, expiry time, revocation epoch, offline-recovery policy, and
  canonical content hash.

A configured enterprise authority error fails closed. A cached snapshot can be
used only when its signed policy explicitly allows pinned offline recovery and
its validity window has not expired.

Revocation is checked at dispatch, lease renewal, artifact acceptance, delivery,
and release. The policy defines whether an emergency revocation cancels, drains,
or quarantines already-running work; it never authorizes a new run from an
expired or revoked snapshot.

### 4.2 Execution Evidence Envelope

Every accepted run publishes a versioned envelope containing:

- project snapshot reference and content hash.
- run, task-attempt, agent, worker, runtime, and model identities.
- lease fencing epoch and execution sandbox profile.
- requirement, ARC compiler/configuration, spec, prompt, plan, and skill-package
  hashes.
- base commit, produced commit/diff, artifact digests, and object-store refs.
- test commands, exit status, logs, coverage where applicable, and timestamps.
- review identities, verdicts, and human-decision references.
- policy decisions, measured usage, retries, and recovery events.

Canonical serialization and hash algorithms are schema-versioned. A consumer
rejects unknown required fields, mismatched authority, expired policy, stale
fencing tokens, and late results from superseded attempts.

### 4.3 Event Contract

Cross-system events carry `event_id`, `command_id` when command-derived,
`schema_version`, `occurred_at`,
`producer`, `tenant/project`, `correlation_id`, `causation_id`, authority owner,
and payload digest. Delivery is at-least-once; consumers provide durable inbox,
idempotency, and ordered projections where ordering is required.

`command_id` is unique within the authoritative room/project binding. Its inbox
record, resulting `run_id`, and outbox event are committed atomically in agentd;
replay returns the existing result and cannot create a second run. Cursor advance
is acknowledged only after that durable commit.

The command inbox/deduplication ledger is `AgentdMatrixGateway`-owned
transport-delivery state. The atomic commit above is possible because the gateway
and `AgentdControlPlane` share one durable execution store and one local
transaction boundary; that shared boundary is a contract requirement of this
roadmap, not an implementation preference, and deployments that split the two
stores require a new ADR rather than a distributed-transaction workaround.

Matrix event IDs may be retained as evidence references, but a Matrix event
alone is not an immutable approval. Approval envelopes bind the human identity,
authority, policy version, decision time, and exact requirement/spec digest.

### 4.4 Skill Package Contract

The Skill Hub uses immutable, content-addressed releases:

```text
skill-package.zip
  SKILL.md
  manifest.json
  dependency-lock.json
  scripts/
  references/
  assets/
```

The manifest defines owner, version, supported runtimes, entrypoints,
permissions, dependencies, and visibility. Publication adds builder identity,
hash, signature, SBOM, scan results, review policy, and audit history.

Lifecycle states are `draft`, `in_review`, `approved`, `signed`, `yanked`,
`revoked`, and `deprecated`. Published bytes are never physically deleted while
run evidence references them. Yank/revoke controls future installation while
preserving historical verification.

OpenFab owns package trust state and catalog APIs. Runtime-specific installation
is performed by an agentd/base adapter under project policy. OpenFab core must
not import Matrix, Robrix, or agentd runtime types.

## 5. Factory Phase Roadmap

Phase names use the `FSF-` prefix to avoid collision with OpenFab and agentd
repository milestones.

Every exit gate produces a versioned acceptance record containing repository
revisions, test/load/failure-injection commands, results, artifact digests,
exceptions, accountable owner, and required human sign-off. A prose assertion or
dashboard screenshot alone is not phase-completion evidence.

### FSF-0: Reliable Transitional Factory

Purpose: make the existing Robrix + Palpo + agent-chat + optional OpenFab path
repeatable for local and small-team use.

Deliverables:

- supervised services for backend, dashboard, bridge, and worker agents.
- deterministic Palpo local/team profile and recovery runbook.
- trusted inviters, ignored appservice senders, observer reconciliation, and
  durable Matrix processed-event IDs.
- human-confirmed requirements-to-ARC flow.
- direct delivery with optional OpenFab `gate=none` attestation.
- cross-project doctor and acceptance checklist.

Exit gate:

- room creation through delivery succeeds after a clean install.
- service restart causes zero replayed accepted commands.
- failed optional certification does not block direct delivery.
- every missing dependency is surfaced by doctor/health output.

### FSF-1: Authority and Artifact Contracts

Depends on: FSF-0 for promotion and deployment. Contract reconciliation may run
in parallel, but FSF-1 cannot exit before the FSF-0 acceptance record exists.

Purpose: establish single ownership before adding enterprise APIs.

Deliverables:

- `ProjectAuthorityPort` with Specify and explicit local implementations.
- approved PRD/ADR amendment preserving OpenFab's `BasePort` spec-cycle contract
  while moving durable execution state to agentd.
- state-ownership matrix and negative boundaries.
- versioned project snapshot, approval envelope, event, execution evidence, and
  certification schemas.
- explicit authority import/rebind and fail-closed behavior.
- ARC traceability bound to approved requirement and frozen spec versions.
- one canonical agentd task lineage; duplicate worktree-only P-number series are
  reconciled before any candidate is integrated.

Exit gate:

- the same project cannot have two active authorities.
- authority restart does not lose project/repository/room/spec bindings.
- stale, expired, wrong-project, or wrong-authority snapshots are rejected.
- every cross-system mutation routes to one authoritative owner.

### FSF-2: Secure Execution Control Plane

Depends on: FSF-1.

Purpose: establish secure multi-tenant execution before fleet scale.

Deliverables:

- enterprise identities for user, service, agent, worker, run, task attempt,
  runtime session, artifact, and policy version.
- OIDC/enterprise-principal mapping and service mTLS.
- Matrix user/device/appservice mapping to enterprise principals, including
  deprovisioning, device revocation, and homeserver trust policy.
- scoped credentials and secret broker; no long-lived repository/model secrets
  in worker configuration.
- tenant/project authorization on every API and storage object.
- durable project snapshot refs, runs, tasks, artifacts, audit, and usage.
- mandatory `ExecutionSandbox`: ephemeral workspace, resource limits,
  default-deny egress profile, syscall/process isolation, and cleanup.
- snapshot revocation and placement enforcement for data classification, worker
  trust domain/region, signed image, dedicated pool, and cache isolation.
- short-lived attempt capabilities bound to the current fencing epoch for forge,
  artifact, secret-broker, and high-risk tool side effects.

Exit gate:

- cross-tenant reads/writes are denied in API and object-store tests.
- generated code cannot access host credentials or another tenant workspace.
- shared caches, model caches, network egress, and worker reuse pass cross-tenant
  isolation tests.
- control-plane restart preserves accepted run/task/artifact state.
- audit reconstructs requester, policy, worker, model, tool use, and artifacts.

### FSF-3: Durable Scheduler and Worker Fleet

Depends on: FSF-2.

Purpose: support replaceable workers without weakening task ownership.

Deliverables:

- lease state machine before worker acquisition APIs.
- monotonic fencing token, attempt ID, CAS ownership, TTL, renewal, explicit
  release, cancellation, retry, dead-letter state, and stale-lease reaper.
- authenticated worker registration, heartbeat, capability/capacity, drain,
  pull acquisition, artifact upload acknowledgement, and offline recovery.
- durable outbox/inbox and backpressure for exhausted policy or capacity.
- epoch-aware forge/secret/artifact/tool admission so stale workers cannot cause
  irreversible external side effects.

Exit gate:

- a reassigned task rejects all late output from the old fencing epoch.
- worker/control-plane restart loses no acknowledged task or artifact state.
- duplicate acquire/release/upload requests are idempotent.
- failure-injection covers expiry/reassignment and partial artifact upload.

### FSF-4: Matrix/Robrix Cutover to agentd

Depends on: FSF-3.

Purpose: remove agent-chat from project command routing while retaining a safe
rollback path.

Deliverables:

- Specify-owned room binding and project ACL snapshots.
- `AgentdMatrixGateway`-owned durable event cursor and processed-event store.
- transactional `command_id` inbox/run/outbox acceptance with a unique
  room/project deduplication key.
- trusted command normalization, attachment ingest, and semantic summaries.
- Robrix project, run, approval, artifact, evidence, and failure views.
- migration stages: observe, shadow-read-only, canary, per-project cutover,
  drain, retire.
- side-effect suppression in shadow mode and measurable rollback triggers.
- forward-only rollback: agentd drains/cancels accepted runs under current
  fencing while only not-yet-accepted commands may return to the legacy route.

Exit gate:

- Robrix creates/binds a project through Specify and dispatches through agentd
  without agent-chat.
- every accepted Matrix sender resolves to a live enterprise principal and
  current project authorization.
- historical Matrix events never create duplicate execution after restart.
- canary rollback never rewinds a cursor or transfers an active lease; it
  preserves project, run, task, deduplication, and audit ownership.
- Matrix contains commands/summaries, not raw runtime transcripts.

This is the control-plane/workflow cutover, not the final runtime cutover.

### FSF-5: OpenFab Evidence and Skill Supply Chain

Depends on: FSF-2; integrates with FSF-3 and FSF-4.

Purpose: make execution evidence independently verifiable and internal skills
governed software-factory assets.

Deliverables:

- signed evidence-envelope import and independent verification policy.
- forge admission/status-check enforcement for policy-required certification.
- clear delivery, machine-attestation, human-certification, release, and
  revocation states.
- production swaps for signing/transparency, SBOM, policy evaluation, sandboxed
  verification, and reproducibility, each tracked as an explicit OpenFab
  repository milestone.
- Skill Hub upload/search/detail/version APIs and UI.
- safe archive extraction, immutable versions, dependency locks, SBOM/scan,
  threshold review, signature verification, yank/revoke, and install policy.
- evidence-chain UI in OpenFab and projection in Robrix.

Exit gate:

- a compromised/untrusted worker assertion cannot become certified without the
  checks required by certification policy.
- every certification resolves to immutable source, spec, evidence, skill, and
  policy digests.
- revoked packages cannot be newly installed but historical runs remain
  verifiable.
- OpenFab core remains base- and forge-agnostic.

### FSF-6: Native Runtime and Final agent-chat Cutover

Depends on: FSF-3, FSF-4, and the FSF-5 evidence/state protocol. OpenFab service
use may remain disabled for a `gate=none` project, but the delivery/certification
state contract must already be stable.

Purpose: remove tmux and agent-chat production runtime assumptions.

Deliverables:

- native runtime contract and process/PTY host.
- native Claude/Codex session reference capture and recovery.
- runtime event stream, snapshot, wait, interrupt, and shutdown APIs.
- explicit `runtime_gone` and resumability semantics.
- runtime isolation tied to FSF-2 sandbox profiles.
- shadow comparison, state import, service install, rollback, and legacy removal.

Exit gate:

- agentd runs and recovers supported agents without tmux.
- dashboard, Robrix, Matrix, and agentctl read the same durable runtime state.
- production pilot passes rollback and worker-loss drills.
- pilot sign-off authorizes removal, and the phase is complete only after
  agent-chat/tmux production configuration, startup entrypoints, runtime
  dependencies, and operator procedures are removed.
- any retained legacy support is an explicitly scoped offline import tool, not a
  production execution path.

### FSF-7: Kubernetes and Multi-Region Scale

Depends on: FSF-3, FSF-5, and FSF-6.

Purpose: scale established semantics; Kubernetes does not substitute for missing
identity, lease, artifact, or recovery contracts.

Deliverables:

- Palpo HA topology, SSO/SCIM scale automation, appservice scaling, media policy, backup/restore,
  federation boundary, and disaster-recovery profile.
- highly available Specify Project Authority and agentd control plane.
- Kubernetes worker deployment, signed images, per-zone pools, rollout audit,
  and queue/policy-driven autoscaling.
- multi-region artifact replication, tenant keys, retention, legal hold, and
  transcript secret-redaction policy.
- Robrix enterprise project discovery, delegated administration, approval queue,
  and evidence search.

Exit gate:

- losing one worker or one control-plane instance loses no accepted state.
- zone failure meets the declared RPO/RTO and does not violate fencing.
- operators can explain every running, queued, blocked, denied, and retried task.
- capacity and cost pressure are visible by organization, team, and project.

## 6. Initial Capacity and Reliability Targets

These are planning acceptance targets, not current capability claims. Changes
require a capacity ADR with measured evidence.

| Profile | Registered principals | Concurrent runs | Registered workers | Control-plane availability | RPO / RTO |
| --- | ---: | ---: | ---: | ---: | ---: |
| Team pilot | 100 | 25 | 50 | 99.5% monthly | 15 min / 60 min |
| Enterprise pre-production | 10,000 | 500 | 2,000 | 99.9% monthly | 5 min / 30 min |

Additional pre-production gates:

- publish a fixed load model covering tenants, projects, rooms, Matrix event
  rate, queue backlog, artifact/log bandwidth, certification throughput,
  test duration, failure injection, and noisy-neighbor distribution.
- scheduler lease acquisition p95 below 2 seconds under the target profile.
- authorized Matrix command acknowledgement p95 below 3 seconds, excluding
  homeserver federation delay outside the trust boundary.
- zero duplicate accepted executions in restart/replay suites.
- zero accepted stale-fencing results in race/failure-injection suites.
- at least 99.9% of certification-eligible runs have complete required evidence.
- 100% of cross-tenant isolation tests deny unauthorized access.

## 7. Product and Operational Metrics

Metrics require an owner, data source, definition, and reporting window:

- idea to approved requirements.
- approved requirements to frozen spec.
- frozen spec to PR/delivery.
- review rework count and human decision latency.
- queue latency, lease retry rate, dead-letter rate, and worker utilization.
- run success, recovery time, and runtime-gone rate.
- evidence completeness and independent verification failure rate.
- machine-attestation, human-certification, release, and revocation counts.
- Skill Hub reuse, rejection, yank/revoke, and vulnerable dependency counts.
- incident MTTR and RPO/RTO drill results.
- model/tool cost by organization, team, project, and accepted artifact.

## 8. Immediate Execution Order

1. Finish FSF-0 reliability and acceptance.
2. In parallel, review the committed agentd P263-P271 candidate lineage at
   `3c27424`; adopt it as FSF-1/FSF-2 input only after FSF-0 passes, PRD/ADR
   ratification completes, and it is reconciled with current agentd `main`.
3. Define the execution sandbox, service identity, and tenant authorization
   contracts before expanding the worker fleet.
4. Define durable lease/fencing semantics before worker acquisition.
5. Execute Matrix/Robrix cutover through staged migration.
6. Build OpenFab evidence import and Skill Hub supply-chain controls.
7. Complete native runtime and final agent-chat/tmux removal.
8. Scale Palpo, Specify, agentd, OpenFab, and Robrix only after the preceding
   semantics pass their phase gates.

### 8.1 Factory-to-agentd Mapping

| Factory phase | agentd phase | Start/exit rule |
| --- | --- | --- |
| FSF-0 | Transitional agent-chat baseline | No new AD-E runtime phase; acceptance blocks FSF-1 promotion |
| FSF-1 | AD-E0 | Candidate review may run in parallel; integration requires PRD/ADR and FSF-0 gates |
| FSF-2 | AD-E1 | Starts after AD-E0 ownership/lineage ratification |
| FSF-3 | AD-E2 | Starts after AD-E1 security gate |
| FSF-4 | AD-E3 | Starts after durable scheduler/worker gate |
| FSF-5 | AD-E4 plus OpenFab work | Evidence contract starts after security; certification integration consumes scheduler/gateway outputs |
| FSF-6 | AD-E5 then AD-E6 | Final removal requires gateway, evidence-state, and native-runtime gates |
| FSF-7 | AD-E7 | Starts only after legacy production removal |

## 9. Deferred Decisions

The following are intentionally not decided by this roadmap:

- Specify-web information architecture, UI, implementation language, framework,
  database, and deployment topology.
- concrete object store, SQL engine, event broker, or Kubernetes distribution.
- public Matrix federation policy beyond the requirement for an explicit
  enterprise trust boundary.
- commercial packaging, billing, public marketplace, and public hosted service.

## 10. Source References

Local sources:

- `docs/OpenFab_MVP_Design_and_PRD.md`.
- `docs/PHASE1-SOFTWARE-FACTORY-PLAN.md`.
- `docs/robrix2-agentchat-integration.md`.
- agentd `docs/specs/2026-05-29-agentd-specify-boundary.md`.
- agentd `docs/specs/2026-07-10-enterprise-execution-ownership-boundary.md`.
- agentd candidate branch `agentd/tr_01KWWTVEK1AC6C836SXSP7Y3Q3`, including
  canonical `docs/plans/2026-07-09-agentd-native-runtime-roadmap.md` at
  `b6cfa03` and the AD-E1 minimum security design at `3c27424`; neither commit is
  integrated into agentd `main`.
- `docs/acceptance/2026-07-12-fsf-0-acceptance-record.md`, currently
  `NOT ACCEPTED` until exact evidence and human signatures are recorded.

Durable decision evidence:

- mempal `drawer_openfab_review_81b7262efa4c`: Specify is required as the
  enterprise Project Authority; Specify-web details are deferred.
- mempal `drawer_openfab_review_58b83145`: ARC is a requirements-to-spec adapter,
  not an execution engine.
- mempal `drawer_openfab_review_ececfe5b`: direct delivery keeps optional
  OpenFab sign-off with `gate=none` as the integration default.

External product reference:

- Anthropic / Claude, "Working at the frontier: How Thomson Reuters builds AI
  for high-stakes professional work", 2026-07-08.
