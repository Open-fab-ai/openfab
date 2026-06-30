spec: task
name: "openfab-layered-qa"
tags: []
---

## Intent

OpenFab's verify stage runs layered QA beyond the bound BDD tests — coverage now, mutation/fuzz
later — selected by a QA tier. Results are recorded in the signed provenance and gate the build,
so "tests are the verifier" gains depth (PPT S11/S14 pillar 1). A missing tool is reported as
skipped, never as passed.

## Decisions

- QA tiers are additive: Fast (bound tests) < Full (+coverage) < Deep (+mutation) < Nightly (+fuzz).
- The coverage gate only applies at Full or higher; below the threshold fails like a failed test.
- An absent coverage tool yields a `skipped` outcome (honest), distinct from `passed`/`failed`.

## Boundaries

### Allowed Changes
- src/**

## Completion Criteria

Scenario: the QA tier is resolved from configuration
  Test:
    Filter: test_qa_tier_from_str
  Given a tier name like "full"
  When the tier is resolved
  Then it maps to the Full tier and "fast" is the default for unknown input

Scenario: coverage below the threshold fails the gate at Full tier
  Test:
    Filter: test_qa_coverage_gate
  Given a coverage percentage and a minimum threshold at Full tier
  When the QA gate is evaluated
  Then below the threshold does not pass and at/above it passes

Scenario: a missing coverage tool is skipped, not passed
  Test:
    Filter: test_qa_missing_tool_is_skipped
  Given no coverage tool is available
  When QA runs at Full tier
  Then the coverage outcome is skipped and is not counted as passed

## Out of Scope

- Mutation and fuzz execution (Deep/Nightly) — wired later; honest-skip for now.
