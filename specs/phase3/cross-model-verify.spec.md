spec: task
name: "openfab-cross-model-verify"
tags: []
---

## Intent

OpenFab verify can run a cross-model adversarial panel: the same change is reviewed by agents
from different model families (e.g. Claude and Codex), and a blocking bug found by ANY family
blocks the gate. Per-family verdicts are signed into the provenance and gated by conformance
(PPT S14 pillar 2 — "two model families don't share blind spots").

## Decisions

- Adversarial-strict merge: the build is blocked if any family returns a non-pass verdict for any
  scenario; it passes only when every family passes every scenario.
- Verdicts are recorded per `(model_family, scenario)` as signed evidence.

## Boundaries

### Allowed Changes
- src/**

## Completion Criteria

Scenario: any model family finding a blocking bug blocks the merge
  Test:
    Filter: test_cross_model_any_block
  Given verdicts from two model families where one returns a non-pass
  When the cross-model decision is computed
  Then the result is blocked

Scenario: all families passing clears the cross-model gate
  Test:
    Filter: test_cross_model_all_pass
  Given verdicts from two model families that all pass
  When the cross-model decision is computed
  Then the result is not blocked

Scenario: per-family verdicts serialize as signed evidence
  Test:
    Filter: test_cross_model_verdicts_json
  Given a set of per-family verdicts
  When they are serialized for the provenance predicate
  Then each entry carries model_family, scenario and verdict

## Out of Scope

- The live panel fan-out over the Bridge (needs running reviewer agents of two families).
