spec: task
name: "openfab-reviewer-caller-verdicts"
tags: []
---

## Intent

OpenFab can route agent-spec's AI-pending scenarios (design intent / quality that mechanical
tests can't cover) to a reviewer agent and merge the reviewer's decisions back, so the
reviewer's code-review verdict feeds the trust gate — a layer distinct from contract+sign-off.

## Decisions

- The reviewer returns `{scenario_name, verdict, confidence, reasoning}`; OpenFab serializes
  these into the agent-spec `resolve-ai` decisions JSON (`model` defaulted when absent).
- AI-pending scenarios are taken from the caller-mode report's `ai_requests_file`.

## Boundaries

### Allowed Changes
- src/**

## Completion Criteria

Scenario: reviewer decisions serialize to the resolve-ai format
  Test:
    Filter: test_decisions_to_json
  Given a list of reviewer decisions
  When they are serialized for resolve-ai
  Then each entry has scenario_name, verdict, confidence, reasoning and a model

Scenario: the pending AI requests are parsed into review items
  Test:
    Filter: test_parse_ai_requests
  Given a pending-ai-requests JSON array
  When it is parsed
  Then each request yields its scenario_name and intent

Scenario: a non-pass reviewer decision blocks acceptance
  Test:
    Filter: test_caller_outcomes_block_on_fail
  Given a resolve-ai report where one scenario verdict is fail
  When outcomes are mapped
  Then acceptance is not passed

## Out of Scope

- The live reviewer round-trip over the Bridge (needs a running reviewer agent).
