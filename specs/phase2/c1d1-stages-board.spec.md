spec: task
name: "openfab-stage-pipeline-and-board"
tags: []
---

## Intent

The dashboard needs a process-detail view (the stage pipeline a run moved through) and a
project-management board (runs grouped by lifecycle status), both derived from existing run
state — no new persistence.

## Decisions

- Stage pipeline is derived from a run's timeline events by marker substrings, plus the run
  status; stages: spec → implement → verify → sign → gate → merge.
- Board lane is derived purely from a run's status/flags.

## Boundaries

### Allowed Changes
- src/**

## Completion Criteria

Scenario: stages are marked done from the run's events
  Test:
    Filter: test_derive_stages_marks_done_from_events
  Given events whose messages contain the spec/implement/verify/sign markers
  When the stage pipeline is derived
  Then those stages are marked done and the next is active

Scenario: a merged run shows the whole pipeline complete
  Test:
    Filter: test_derive_stages_merged_completes
  Given a run whose status is merged
  When the stage pipeline is derived
  Then the gate and merge stages are done

Scenario: board lane is derived from run status
  Test:
    Filter: test_board_lane_from_status
  Given runs with statuses running, blocked, and merged
  When the board lane is computed
  Then they map to implementing, review, and merged lanes respectively

## Out of Scope

- The web SPA rendering of the pipeline and board.
