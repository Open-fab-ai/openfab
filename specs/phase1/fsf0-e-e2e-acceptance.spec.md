spec: task
name: "FSF-0 E: end-to-end demo, runbook, and acceptance record"
tags: [phase1, fsf0, e2e]
depends: [fsf0-a-palpo-deploy, fsf0-b1-agentchat-services, fsf0-b2-matrix-routing, fsf0-c-robrix-cockpit, fsf0-d-openfab-certify]
estimate: 2d
---

## Intent

Prove FSF-0 as a whole: a fresh machine follows one runbook from clean install
to delivered work, the exit-gate drills pass, and the result is captured as a
versioned acceptance record — the artifact that unblocks FSF-1 promotion.

## Decisions

- Runbook: one canonical start/stop runbook plus environment file set covering
  Palpo, agent-chat services, Robrix, and optional OpenFab.
- Runbook checklist: `docs/ACCEPTANCE-CHECKLIST.md` remains the operator
  walkthrough and troubleshooting source; unchecked boxes are not acceptance
  evidence.
- Acceptance record:
  `docs/acceptance/2026-07-12-fsf-0-acceptance-record.md` is the versioned gate
  artifact. It captures immutable repository revisions, drill commands, exit
  codes, result/artifact digests, exceptions, accountable owner, and human
  sign-off.
- Fault drills: scripted drills cover service restart, certification failure,
  and dependency loss; each drill asserts through `doctor` output, not log
  reading.

## Boundaries

### Allowed Changes
- docs/**
- bridge/**
- tests/**

### Forbidden
- Do not mark any checklist item passed without a recorded command and result.
- Do not close this task while any critical scenario in workstreams A-D fails.

## Out of Scope

- Team-scale load or capacity targets (FSF-7 planning targets).
- PRD/ADR amendment work (FSF-1).

## Completion Criteria

Scenario: clean-machine runbook completes the Phase 1 success scenario (critical)
  Tags: critical
  Review: human
  Test:
    Package: agentchat-demo
    Filter: test_runbook_e2e_walkthrough
  Given a machine with none of the services installed
  When an operator follows the runbook end to end
  Then room creation through delivery succeeds and the operator records each command result and artifact digest in the versioned acceptance record

Scenario: restart drill shows zero replayed accepted commands (critical)
  Tags: critical
  Test:
    Package: agentchat-demo
    Filter: test_drill_restart_zero_replay
  Given a deployment with at least one accepted command in history
  When every service is restarted and Matrix history is re-synced
  Then the drill reports zero duplicate executions

Scenario: certification-failure drill leaves delivery unblocked (critical)
  Tags: critical
  Test:
    Package: agentchat-demo
    Filter: test_drill_cert_failure_nonblocking
  Given a run that will fail optional certification
  When the drill executes the delivery and certification steps
  Then delivery completes and the certification failure is reported in the room

Scenario: dependency-loss drill is fully explained by doctor
  Test:
    Package: agentchat-demo
    Filter: test_drill_doctor_explains_faults
  Given the drill stops Palpo, removes the signing key, and evicts the bridge bot one at a time
  When `doctor` runs after each fault injection
  Then each run exits non-zero and names the injected fault as the cause

Scenario: acceptance record is complete before sign-off
  Review: human
  Test:
    Package: agentchat-demo
    Filter: test_acceptance_record_fields_present
  Given all drills have run
  When the versioned acceptance record is reviewed
  Then it contains immutable repository revisions, drill commands, exit codes, result and artifact digests, exceptions, and accountable owner
  And the status cannot become accepted before the required human signature is recorded
