# FSF-0 Acceptance Record

- Record version: 0 (evidence collection draft)
- Created: 2026-07-12
- Status: **NOT ACCEPTED**
- Accountable owner: AlexZ (sign-off review delegated to Claude Fable 5,
  2026-07-12; delegation does not waive any evidence requirement)
- Human sign-off: not recorded

This record implements the evidence shape required by the enterprise factory
roadmap. Unchecked operational instructions or a successful screenshot do not
change its status. FSF-1/AD-E0 promotion remains blocked until every mandatory
row has exact evidence and the accountable human signs this record.

## 1. Repository Revisions

| System | Inspected revision | Branch | Worktree state | Acceptance meaning |
| --- | --- | --- | --- | --- |
| OpenFab | `88eadb2` | `feat/phase1-2-agentchat-agentspec-console` | 17 changed/untracked entries | Governance baseline committed; not a clean release revision |
| agent-chat | `bf85365` | `feat/matrix-agent-capabilities` | 19 changed/untracked entries | B1 candidate passed scoped review; not a clean release revision |
| Robrix2 | `3d273cbc` | `main` | 9 changed/untracked entries | FSF-0A candidate passed; not a clean release revision |
| Palpo | `b5aaa17` | `main` | clean nested checkout | Exact Palpo source revision recorded for FSF-0A candidate |
| agentd candidate | `12e1b66` | `agentd/tr_01KWWTVEK1AC6C836SXSP7Y3Q3` | generated evidence remains unclassified | AD-E0 HOLD review committed; not part of FSF-0 runtime |

The accepted record must replace dirty-worktree counts with immutable repository
revisions or signed artifact digests for the exact deployed bytes.

## 2. Mandatory Exit Evidence

| FSF-0 requirement | Required evidence | Current evidence | Result |
| --- | --- | --- | --- |
| Clean install reaches room creation through delivery | install command/logs, room/project/run/artifact ids, exact revisions | July 6 checklist contains manual commands but no completed run record | MISSING |
| Restart causes zero replayed accepted commands | before/after processed-event ids, run ids, restart logs, duplicate count | checklist item 5.3 is unchecked | MISSING |
| Optional certification failure does not block direct delivery | `gate=none` run plus injected certification failure and delivered subject digest | no recorded failure-injection run | MISSING |
| Doctor exposes every missing dependency | doctor command output for each absent service/credential/configuration | checklist invokes doctor but has no captured matrix | MISSING |
| Supervised services recover deterministically | service definitions, startup order, restart results, health outputs | B1 candidate: 93/93 scoped tests, 5/5 exact selectors, image `sha256:715c5270...`, exact-byte digest `26032666...`, final independent review accept; full repository suite remains non-green and bytes are uncommitted | PARTIAL |
| Palpo local/team profile and recovery runbook | immutable configuration, backup/restore drill, RPO/RTO result | five isolated real E2E selectors passed; candidate evidence records Palpo revision and tested-byte digest, but no human sign-off or committed Robrix2 revision | PARTIAL |
| Trusted inviter and ignored sender enforcement | positive and negative Matrix event ids and results | checklist items are unchecked | MISSING |
| Durable processed-event ids | restart/replay event corpus and zero duplicate effects | no artifact digest recorded | MISSING |
| Human-confirmed requirements-to-ARC flow | requirement/spec/traceability digests and human confirmation event | no completed acceptance artifact | MISSING |
| Cross-project isolation | two-project command/result routing and doctor report | no recorded run | MISSING |

## 3. Required Commands and Artifacts

All six FSF-0 task contracts under `specs/phase1/` parse and lint at 100%.
FSF-0A now implements and passes its five opt-in real selectors; see
[`evidence/2026-07-12-fsf0-a-palpo-real-e2e.md`](evidence/2026-07-12-fsf0-a-palpo-real-e2e.md).
B1 now passes its five exact selectors and independent review; see
[`evidence/2026-07-12-fsf0-b1-agentchat-services.md`](evidence/2026-07-12-fsf0-b1-agentchat-services.md).
The other critical E2E filters remain absent outside their spec files, including
`test_runbook_e2e_walkthrough`, `test_drill_restart_zero_replay`,
`test_drill_cert_failure_nonblocking`, `test_drill_doctor_explains_faults`, and
`test_acceptance_record_fields_present`. FSF-0A and B1 implementation evidence
is partial; cross-system implementation and acceptance are not complete.

The accountable acceptance run must record exact commands and exit codes for:

1. clean install/build of Palpo, Robrix2, agent-chat, OpenFab, and bridge;
2. service startup and health/doctor checks;
3. room/project creation, requirement confirmation, ARC compile, implementation,
   direct delivery, and optional certification;
4. service restart followed by replay of the same Matrix events;
5. injected optional-certification failure under `gate=none`;
6. missing-dependency matrix for homeserver, bridge, backend, agent, forge,
   signer, and OpenFab;
7. backup/restore or recovery drill for durable Matrix/bridge state.

For each command, attach stdout/stderr digest, exit code, start/end timestamps,
repository revision, environment profile digest, and produced artifact ids.
Secrets must be redacted before digest publication.

## 4. Exceptions

No exceptions are approved. Any exception must name the affected gate, owner,
expiry date, compensating control, and explicit human approval.

## 5. Sign-Off

Sign-off review authority for this record was delegated by the factory
accountable owner (AlexZ) to Claude Fable 5 on 2026-07-12 (authorization
recorded in the openfab Claude session). The delegation does not waive any
evidence requirement: the status stays `NOT ACCEPTED` until every mandatory
row in section 2 has exact evidence, and the delegated review executes against
that evidence before any signature row is completed. Each completed row must
cite this delegation in `Signature/reference`.

| Role | Identity | Decision | Timestamp | Signature/reference |
| --- | --- | --- | --- | --- |
| Factory accountable owner | AlexZ (review delegated to Claude Fable 5) | not signed | not recorded | not recorded |
| OpenFab maintainer | not recorded | not signed | not recorded | not recorded |
| agent-chat/Matrix operator | not recorded | not signed | not recorded | not recorded |
| Robrix/Palpo operator | not recorded | not signed | not recorded | not recorded |

The only valid status transitions are from `NOT ACCEPTED` to either
`ACCEPTED WITH EXPLICIT EXCEPTIONS` or `ACCEPTED`. A transition requires all
evidence digests and the factory accountable owner's signature.
