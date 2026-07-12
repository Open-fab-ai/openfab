# ADR 0003: Agentd Execution and OpenFab Certification Boundary

- Status: Proposed (maintainer design approval recorded 2026-07-12; acceptance
  remains gated on the requirements below)
- Date: 2026-07-12
- Decision owners: OpenFab and agentd maintainers
- Factory phases: FSF-1 / AD-E0

## Approvals

| Role | Decision | Date | Provenance |
| --- | --- | --- | --- |
| OpenFab maintainer | Design approved | 2026-07-12 | Decision delegated by AlexZ to Claude Fable 5 (openfab Claude session, 2026-07-12); reviewed against the factory role matrix, `ForgePort` clarification, and `command_id` idempotency semantics — no conflicts found |
| agentd maintainer | Design approved | 2026-07-12 | Same delegated review; consistent with P264 ownership boundary and the canonical AD-E roadmap at `b6cfa03` |

Maintainer approval satisfies only the first two Acceptance Requirements.
Status becomes `Accepted` when the remaining mechanical requirements pass:
idempotent dispatch/result contract tests, per-system outage failure tests, and
the complete human-signed FSF-0 acceptance record.

## Context

OpenFab currently orchestrates an invoked spec cycle through `BasePort` and
creates provenance, verification, and trust-gate results. The enterprise
software-factory roadmap assigns durable execution control to agentd and
independent certification to OpenFab. Without an explicit amendment, both
systems could appear to own runs, retries, delivery, or artifacts.

The agentd P264-P271 candidate lineage defines `SpecifyProjectAuthority`,
`AgentdControlPlane`, `AgentdWorker`, immutable execution artifacts/audit,
durable leases/fencing, and external certification references. Those commits
remain feature-branch candidates and are not integrated into agentd `main`.

## Decision

1. Preserve `BasePort` as the OpenFab execution seam. Add an agentd adapter
   rather than importing agentd runtime types into OpenFab Core.
2. For an OpenFab-governed run, OpenFab owns the spec-cycle state, verification
   request/result, provenance, certification state, trust-policy result, and
   human gate.
3. Agentd owns durable execution run/task identity, queueing, dispatch, leases,
   fencing, runtime-session/checkpoint state, artifact index, execution audit,
   and measured usage.
4. The agentd adapter maps one OpenFab dispatch to one immutable
   `ProjectExecutionSnapshot` and one durable agentd run reference. Idempotent
   dispatch returns the prior run rather than creating a second execution.
5. The adapter maps agentd completion to immutable artifact/evidence references.
   OpenFab may independently verify those subjects and records its own result;
   it does not mutate agentd execution history.
6. `ForgePort` remains the OpenFab forge seam. An invoked OpenFab spec cycle may
   create commits or pull requests through `ForgePort`, but authoritative
   delivery state and exact source subject must be bound to immutable evidence.
7. Direct Robrix-to-agentd delivery is a distinct project-policy profile. It
   bypasses OpenFab orchestration and gains no implicit OpenFab certification.
8. Specify is the enterprise Project Authority. OpenFab validates the pinned
   requirement/spec references it consumes but does not become the project/spec
   lifecycle authority.

## Required Protocol Properties

- The adapter records OpenFab cycle id, agentd run id, authority snapshot ref,
  exact source/base commit, and idempotency key.
- Cross-system delivery is at-least-once and consumers maintain durable inbox
  deduplication.
- OpenFab verification addresses immutable artifact/evidence digests, never a
  mutable branch name or worker-local path.
- Required certification can block release under project policy; optional
  `gate=none` certification failure does not block direct delivery.
- Cancellation, retry, and recovery preserve one authoritative owner for every
  state transition. OpenFab does not issue or renew agentd task leases.
- Neither service silently falls back to another authority when Specify,
  agentd, or OpenFab is unavailable.

## Negative Boundaries

- OpenFab Core must not persist or schedule agentd queues, leases, runtime
  sessions, worker heartbeats, checkpoints, or transcripts.
- Agentd must not author OpenFab certification verdicts, signatures,
  attestations, trust-policy results, or human approvals.
- `AgentdWorker` self-report cannot satisfy independent OpenFab verification.
- Matrix room history, tmux targets, process ids, paths, and provider session
  refs are not cross-system durable identities.
- ARC compiles approved requirements; it is not an OpenFab `BasePort` runtime
  and does not execute implementation agents.

## Consequences

- OpenFab needs an agentd `BasePort` adapter and versioned run/evidence mapping,
  but its core spec-cycle interfaces remain stable.
- Agentd needs an evidence export/certification-reference transport, but does
  not embed OpenFab's verifier or trust policy.
- Operational views may join immutable references from both systems, but cannot
  write through those projections.
- FSF-1/AD-E0 cannot exit until this ADR, the PRD amendment, and agentd's
  ownership contracts are approved and verified together.

## Alternatives Rejected

- Making OpenFab the durable execution scheduler duplicates agentd and violates
  the factory ownership matrix.
- Making agentd the certification authority lets the executor certify itself.
- Replacing `BasePort` with agentd-specific core types makes OpenFab no longer
  base-agnostic.
- Treating direct delivery and OpenFab-governed delivery as one implicit mode
  obscures certification and release semantics.

## Acceptance Requirements

Before changing this ADR to `Accepted`:

- OpenFab maintainers approve the PRD amendment and this ADR.
- Agentd maintainers approve P264 ownership and the reconciled
  Specify/`ProjectAuthorityPort` adapter relationship.
- Contract tests prove idempotent dispatch/result mapping without duplicated
  run or lease ownership.
- Failure tests cover Specify, agentd, OpenFab, and forge outages independently.
- The FSF-0 acceptance record is complete and human-signed.
