spec: task
name: "FSF-0 D: OpenFab optional certification"
tags: [phase1, fsf0, openfab]
depends: [fsf0-b2-matrix-routing]
estimate: 2d
---

## Intent

Keep OpenFab an optional trust layer for the Robrix + agent-chat path: direct
delivery works without it, `gate=none` stays the integration default, and when
certification is requested it verifies repository truth and reports clearly on
failure without blocking delivery. This closes the FSF-0 exit gate "failed
optional certification does not block direct delivery".

## Decisions

- Gate default: `gate=none` remains the default for room-driven integration;
  enabling a stricter gate is explicit per-project policy.
- Identity check: a certification request must carry room and build identity
  that match the imported run; mismatches are rejected before verification.
- Provenance source: generated-file attribution comes from git/worktree truth,
  not base self-reported `changed_files`.
- Badge format: the published result exposes `spec_id`, `verdict`, and a signed
  result reference consumable by Robrix.
- Doctor coverage: `doctor` checks repo access, policy, bridge reachability, and
  signing prerequisites.

## Boundaries

### Allowed Changes
- src/**
- bridge/**
- web/**
- tests/**

### Forbidden
- Do not import Matrix, Robrix, or agent-chat types into OpenFab core.
- Do not make certification mandatory for any default-path delivery.

## Out of Scope

- Skill Hub, SBOM, transparency log, sandboxed re-verification (FSF-5).
- agentd `BasePort` adapter (FSF-1 and later).

## Completion Criteria

Scenario: optional certification succeeds on a known-good fixture
  Test:
    Filter: test_optional_certify_known_good_fixture
  Given an imported run from the integration fixture with matching room and build identity
  When certification runs under `gate=none`
  Then the result verdict is pass and a signed result reference is published

Scenario: failed certification under gate=none does not block delivery (critical)
  Tags: critical
  Test:
    Filter: test_failed_certify_gate_none_nonblocking
  Given an imported run whose verification fails
  When certification completes under `gate=none`
  Then the failure is reported with a reason and the delivery status remains delivered

Scenario: mismatched room or build identity is rejected before verification
  Test:
    Filter: test_certify_rejects_identity_mismatch
  Given a certification request whose room id does not match the imported run
  When the request is submitted
  Then it is rejected with an identity-mismatch error and no verification starts

Scenario: provenance uses git truth over self-report
  Test:
    Filter: test_provenance_uses_git_truth_not_base_reported_changed_files
  Given a base that writes one file while claiming a different file
  When the run completes
  Then the attestation's generated entries reflect the file actually written

Scenario: badge payload carries spec_id, verdict, and signed reference
  Test:
    Filter: test_badge_payload_fields
  Given a completed certification result
  When the badge endpoint is queried
  Then the payload contains `spec_id`, `verdict`, and the signed result reference

Scenario: doctor names a missing signing key
  Test:
    Filter: test_doctor_reports_missing_signing_key
  Given an environment without a signing key configured
  When `doctor` runs
  Then it exits non-zero and names the signing prerequisite as missing
