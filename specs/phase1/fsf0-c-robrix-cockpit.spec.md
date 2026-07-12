spec: task
name: "FSF-0 C: Robrix project cockpit"
tags: [phase1, fsf0, robrix]
depends: [fsf0-b2-matrix-routing]
estimate: 3d
---

## Intent

Give the human a project cockpit in Robrix: create a project room bound to a
repository and agent team, see agent reachability, trigger workflow actions,
and see generated artifacts and OpenFab badge state — with explicit failure
states instead of silent dead ends. Primary repository: robrix2.

## Decisions

- Room binding: project-room creation writes repository and agent-team metadata
  into room state (`roomGroupMap` binding), not into local-only config.
- Observer invite: inviting any agent-chat agent auto-invites the bridge
  observer bot in the same flow.
- Failure states: missing Palpo, missing bridge, and missing OpenFab each render
  a distinct visible error state; OpenFab absence never blocks room actions.

## Boundaries

### Allowed Changes
- src/**
- roadmap/agentchat-demo/**

### Forbidden
- Do not store execution history or transcripts inside Robrix state.
- Do not make OpenFab certification a prerequisite for any room action.

## Out of Scope

- New Matrix routing semantics (workstream B2 owns routing).
- Enterprise project discovery and delegated administration (FSF-7).

## Completion Criteria

Scenario: project room creation binds repository and team metadata
  Test:
    Package: robrix2
    Filter: room_creation_writes_binding_metadata
  Given a user creates a project room for repository "demo/site"
  When room creation completes
  Then the room state contains the repository binding and the selected agent team

Scenario: inviting an agent auto-invites the observer bot
  Test:
    Package: robrix2
    Filter: agent_invite_auto_invites_observer
  Given a project room without the bridge observer bot
  When the user invites `wf_coordinator`
  Then the observer bot invite is sent in the same flow without a separate user step

Scenario: bridge outage renders an unreachable agent state
  Test:
    Package: robrix2
    Filter: bridge_down_shows_unreachable
  Given a bound room whose bridge is stopped
  When the room member panel renders
  Then the agent is shown as unreachable rather than as a normal member

Scenario: missing OpenFab shows a failure state but room actions stay usable
  Test:
    Package: robrix2
    Filter: openfab_absent_actions_still_usable
  Given OpenFab is not running
  When the user opens the certify action
  Then a visible OpenFab-unavailable state renders and issue/spec/implement actions remain enabled

Scenario: certification badge renders from a published result
  Test:
    Package: robrix2
    Filter: badge_renders_from_result
  Given a room with a published OpenFab certification result for a build
  When the room artifact panel renders
  Then the badge shows the verdict and links the signed result reference
