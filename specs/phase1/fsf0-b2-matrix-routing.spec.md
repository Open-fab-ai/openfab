spec: task
name: "FSF-0 B2: Matrix routing reliability"
tags: [phase1, fsf0, agent-chat, matrix]
depends: [fsf0-a-palpo-deploy, fsf0-b1-agentchat-services]
estimate: 2d
---

## Intent

Make room-command routing exactly-once and abuse-resistant: a Robrix room
command reaches the right agent once, replayed Matrix history never re-executes
commands, and only trusted senders can wake agents. This closes the FSF-0 exit
gate "service restart causes zero replayed accepted commands". Primary
repository: agent-chat.

## Decisions

- Deduplication: processed Matrix event ids are persisted durably and survive
  process restart; an event id seen before is acknowledged without dispatch.
- Group routing: a room present in `roomGroupMap` routes as a group; with
  exactly one agent member it default-wakes `wf_coordinator`, never an agent DM.
- Sender policy: `trusted inviters` gate room joins; `ignored senders` (bridge
  and appservice bots) are never routed to worker agents.

## Boundaries

### Allowed Changes
- src/**
- tests/**

### Forbidden
- Do not weaken or bypass the persisted event-id check in any dispatch path.
- Do not add a second dispatch entry point that skips sender policy.

## Out of Scope

- Robrix UI behavior (workstream C).
- agentd Matrix gateway (FSF-4).

## Completion Criteria

<!-- lint-ack: output-mode-coverage — the persisted event-id store is internal state, verified through the restart replay scenario; there is no user-facing file output mode in this task -->

Scenario: mapped room with one agent routes as group and wakes wf_coordinator
  Test:
    Package: agent-chat
    Filter: routes_mapped_room_as_group_wakes_coordinator
  Given a Matrix room bound in `roomGroupMap` with exactly one agent member
  When a room message arrives
  Then the dispatch payload targets the group and mentions `wf_coordinator`

Scenario: restart plus history replay creates zero duplicate dispatch (critical)
  Tags: critical
  Test:
    Package: agent-chat
    Filter: replay_after_restart_zero_duplicates
  Given a command event was accepted and dispatched before a restart
  When the bridge restarts and the homeserver replays the same event id
  Then no second dispatch occurs and the event is acknowledged as already processed

Scenario: invite from an untrusted inviter is rejected
  Test:
    Package: agent-chat
    Filter: untrusted_inviter_denied
  Given an invite from a user not in `trusted inviters`
  When the invite is processed
  Then the join is rejected and the denial is logged with the sender id

Scenario: message from an ignored appservice sender is rejected for dispatch
  Test:
    Package: agent-chat
    Filter: ignored_sender_not_routed
  Given a message from a sender on the `ignored senders` list in a mapped room
  When the message is processed
  Then dispatch is rejected and no worker agent receives it

Scenario: create_issue command reaches exactly one agent exactly once
  Test:
    Package: agent-chat
    Filter: create_issue_routed_exactly_once
  Given a healthy mapped room with a coordinator agent
  When a user sends one `create issue` command
  Then exactly one dispatch is recorded for exactly one target agent
