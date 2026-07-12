# FSF-0 Acceptance Record

- Record version: 0 (evidence collection draft)
- Created: 2026-07-12
- Status: **NOT ACCEPTED**
- Accountable owner: not recorded
- Human sign-off: not recorded

This record implements the evidence shape required by the enterprise factory
roadmap. Unchecked operational instructions or a successful screenshot do not
change its status. FSF-1/AD-E0 promotion remains blocked until every mandatory
row has exact evidence and the accountable human signs this record.

## 1. Repository Revisions

| System | Inspected revision | Branch | Worktree state | Acceptance meaning |
| --- | --- | --- | --- | --- |
| OpenFab | `f6b31d0` | `feat/phase1-2-agentchat-agentspec-console` | 17 changed/untracked entries | Not a clean release revision |
| agent-chat | `bf85365` | `feat/matrix-agent-capabilities` | 8 changed/untracked entries | Not a clean release revision |
| Robrix2 | `3d273cbc` | `main` | 5 changed/untracked entries | Not a clean release revision |
| Palpo | not located in inspected project roots | not recorded | unavailable | Mandatory evidence missing |
| agentd candidate | `3c27424` | `agentd/tr_01KWWTVEK1AC6C836SXSP7Y3Q3` | generated evidence remains unclassified | Candidate only; not part of FSF-0 runtime |

The accepted record must replace dirty-worktree counts with immutable repository
revisions or signed artifact digests for the exact deployed bytes.

## 2. Mandatory Exit Evidence

| FSF-0 requirement | Required evidence | Current evidence | Result |
| --- | --- | --- | --- |
| Clean install reaches room creation through delivery | install command/logs, room/project/run/artifact ids, exact revisions | July 6 checklist contains manual commands but no completed run record | MISSING |
| Restart causes zero replayed accepted commands | before/after processed-event ids, run ids, restart logs, duplicate count | checklist item 5.3 is unchecked | MISSING |
| Optional certification failure does not block direct delivery | `gate=none` run plus injected certification failure and delivered subject digest | no recorded failure-injection run | MISSING |
| Doctor exposes every missing dependency | doctor command output for each absent service/credential/configuration | checklist invokes doctor but has no captured matrix | MISSING |
| Supervised services recover deterministically | service definitions, startup order, restart results, health outputs | manual tmux startup only | MISSING |
| Palpo local/team profile and recovery runbook | immutable configuration, backup/restore drill, RPO/RTO result | Palpo checkout/revision not located | MISSING |
| Trusted inviter and ignored sender enforcement | positive and negative Matrix event ids and results | checklist items are unchecked | MISSING |
| Durable processed-event ids | restart/replay event corpus and zero duplicate effects | no artifact digest recorded | MISSING |
| Human-confirmed requirements-to-ARC flow | requirement/spec/traceability digests and human confirmation event | no completed acceptance artifact | MISSING |
| Cross-project isolation | two-project command/result routing and doctor report | no recorded run | MISSING |

## 3. Required Commands and Artifacts

All six FSF-0 task contracts under `specs/phase1/` parse and lint at 100%.
However, searches of the inspected OpenFab, agent-chat, and Robrix2 worktrees
found none of the critical E2E test filters outside those spec files, including
`test_runbook_e2e_walkthrough`, `test_drill_restart_zero_replay`,
`test_drill_cert_failure_nonblocking`, `test_drill_doctor_explains_faults`, and
`test_acceptance_record_fields_present`. Contract quality is therefore proven;
implementation and acceptance are not.

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

| Role | Identity | Decision | Timestamp | Signature/reference |
| --- | --- | --- | --- | --- |
| Factory accountable owner | not recorded | not signed | not recorded | not recorded |
| OpenFab maintainer | not recorded | not signed | not recorded | not recorded |
| agent-chat/Matrix operator | not recorded | not signed | not recorded | not recorded |
| Robrix/Palpo operator | not recorded | not signed | not recorded | not recorded |

The only valid status transitions are from `NOT ACCEPTED` to either
`ACCEPTED WITH EXPLICIT EXCEPTIONS` or `ACCEPTED`. A transition requires all
evidence digests and the factory accountable owner's signature.
