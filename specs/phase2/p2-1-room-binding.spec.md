spec: task
name: "openfab-room-project-binding"
tags: []
---

## Intent

A Robrix (Matrix) room is bound to an OpenFab project, so when the team's coordinator agent
finalizes a requirements/decision document it is ingested into the correct project — and the
user sees it on that project's dashboard without uploading anything.

## Decisions

- The binding is a registry of `{ room, project }` persisted under the projects dir; lookup
  is pure (a room maps to at most one project).

## Boundaries

### Allowed Changes
- src/**

## Completion Criteria

Scenario: a bound room resolves to its project
  Test:
    Filter: test_resolve_room_project_bound
  Given a binding of room "!demoboard:palpo" to project "alpha"
  When resolving the project for that room
  Then it returns "alpha"

Scenario: an unbound room resolves to no project
  Test:
    Filter: test_resolve_room_project_unbound
  Given a binding set without room "!ghost:palpo"
  When resolving the project for that room
  Then it returns none

## Out of Scope

- The Bridge poller that detects "doc ready" and the live coordinator submission.
