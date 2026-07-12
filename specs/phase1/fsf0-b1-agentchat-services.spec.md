spec: task
name: "FSF-0 B1: agent-chat supervised service runtime"
tags: [phase1, fsf0, agent-chat]
estimate: 2d
---

## Intent

Replace manual tmux startup of agent-chat with supervised services so the
backend, dashboard, Matrix bridge, and relay survive restarts and report their
own health. Worker agents become registry records with explicit lifecycle
states instead of implicit tmux panes. Primary repository: agent-chat.

## Decisions

- Service set: `backend`, `dashboard`, `bridge`, `relay` defined in one
  supervisor profile (`services-local` for local, Docker Compose for team).
- Roster source of truth: the dashboard agent roster reads the agent registry;
  it does not scan tmux sessions.
- Commands: `status` prints per-service health; `doctor` explains a failing
  service with the failing dependency named.

## Boundaries

### Allowed Changes
- services/**
- src/**
- tests/**

### Forbidden
- Do not change the Matrix command vocabulary in this task.
- Do not remove the existing tmux path before the supervised path passes.

## Out of Scope

- Matrix routing correctness (workstream B2).
- agentd replacement work (FSF-1 and later).

## Completion Criteria

Scenario: supervised start brings up all four services
  Test:
    Package: agent-chat
    Filter: services_start_all_healthy
  Given the `services-local` profile on a clean checkout
  When the supervisor starts
  Then `status` reports `backend`, `dashboard`, `bridge`, and `relay` as healthy

Scenario: restart preserves registered agent records
  Test:
    Package: agent-chat
    Filter: restart_preserves_agent_registry
  Given three worker agents registered in the agent registry
  When the backend service is stopped and started again
  Then the registry still lists the same three agents

Scenario: dashboard roster equals the registry
  Test:
    Package: agent-chat
    Filter: dashboard_roster_matches_registry
  Given agents registered in the registry and one stale tmux session name
  When the dashboard roster endpoint is queried
  Then the roster equals the registry contents and does not include the stale tmux name

Scenario: doctor names a stopped bridge as the failing dependency
  Test:
    Package: agent-chat
    Filter: doctor_reports_stopped_bridge
  Given the bridge service is stopped
  When `doctor` runs
  Then it exits non-zero and its output names the bridge as the failing service

Scenario: status reports a crashed service instead of hanging
  Test:
    Package: agent-chat
    Filter: status_reports_crashed_service
  Given the relay process was killed
  When `status` runs
  Then it completes within 5 seconds and marks `relay` as not healthy
