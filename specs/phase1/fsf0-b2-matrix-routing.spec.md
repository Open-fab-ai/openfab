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
- SDK checkpointing: the Matrix sync token advances only after the bridge has
  awaited durable processing for every event in the sync response.
- Crash-window idempotency: authenticated Matrix bridge requests carry
  `source_event_id`; the backend reserves a durable receipt before dispatch,
  commits it only after the message and acceptance event are fsynced, and
  returns the original message for retries with the same event id.
- Receipt retention: Matrix receipts never share the bounded message retention
  lifecycle, so old event ids remain idempotent after message archival.
- Checkpoint order: the bridge records a processed event only after backend
  acceptance. A crash before that checkpoint retries the same `source_event_id`
  and therefore cannot create a second stored message or dispatch.
- Group routing: a room present in `roomGroupMap` routes as a group; with
  exactly one agent member it default-wakes `wf_coordinator`, never an agent DM.
- Authenticated default routing: the bridge marks its generated
  `wf_coordinator` fallback so the backend can deliver it even while Matrix and
  backend group membership are temporarily out of sync.
- Sender policy: `trusted inviters` gate room joins; `ignored senders` (bridge
  and appservice bots) are never routed to worker agents. Realtime and polled
  bot invites use the same trust-policy entry point.
- Fail-closed inputs: production Matrix ingestion requires a configured shared
  bridge secret and a non-empty Matrix event id.

## Boundaries

### Allowed Changes
- backend-v2.js
- bridge-matrix.js
- src/**
- tests/**
- .env.example
- services/README.md

### Forbidden
- Do not weaken or bypass the persisted event-id check in any dispatch path.
- Do not add a second dispatch entry point that skips sender policy.
- Do not accept `source_event_id` from unauthenticated or non-Matrix callers.

## Out of Scope

- Robrix UI behavior (workstream C).
- agentd Matrix gateway (FSF-4).
- Exactly-once semantics for bridge-local `!` administration commands. B2's
  factory command path is the authenticated Matrix-to-`/api/messages` worker
  dispatch path; the existing `!` command vocabulary remains unchanged.

## Completion Criteria

<!-- lint-ack: output-mode-coverage — the persisted event-id store is internal state, verified through the restart replay scenario; there is no user-facing file output mode in this task -->

Scenario: mapped room with one agent routes as group and wakes wf_coordinator
  Test:
    Filter: routes_mapped_room_as_group_wakes_coordinator
  Given a Matrix room bound in `roomGroupMap` with exactly one agent member
  When a room message arrives
  Then the dispatch payload targets the group and mentions `wf_coordinator`

Scenario: restart plus history replay creates zero duplicate dispatch (critical)
  Tags: critical
  Test:
    Filter: replay_after_restart_zero_duplicates
  Given a command event was accepted and dispatched before a restart
  When the bridge restarts and the homeserver replays the same event id
  Then no second dispatch occurs and the event is acknowledged as already processed

Scenario: crash after backend acceptance creates zero duplicate dispatch (critical)
  Tags: critical
  Test:
    Filter: accepted_before_checkpoint_replay_zero_duplicates
  Given a Matrix command was stored and dispatched by the backend
  And the bridge crashed before persisting its processed-event checkpoint
  When the restarted bridge retries the same `source_event_id`
  Then the backend returns the original message id
  And no second message or dispatch is created

Scenario: invite from an untrusted inviter is rejected
  Test:
    Filter: untrusted_inviter_denied
  Given an invite from a user not in `trusted inviters`
  When the invite is processed
  Then the join is rejected and the denial is logged with the sender id

Scenario: message from an ignored appservice sender is rejected for dispatch
  Test:
    Filter: ignored_sender_not_routed
  Given a message from a sender on the `ignored senders` list in a mapped room
  When the message is processed
  Then dispatch is rejected and no worker agent receives it

Scenario: create_issue command reaches exactly one agent exactly once
  Test:
    Filter: create_issue_routed_exactly_once
  Given a healthy mapped room with a coordinator agent
  When a user sends one `create issue` command
  Then exactly one dispatch is recorded for exactly one target agent

Scenario: corrupted processed-event journal fails closed
  Test:
    Filter: corrupt_processed_event_journal_fails_closed
  Level: integration
  Targets: data/matrix/processed-events.jsonl
  Given the processed-event journal contains malformed data before its final line
  When the Matrix bridge initializes
  Then initialization fails with the corrupted journal path
  And no Matrix event is dispatched

Scenario: Matrix SDK sync token waits for durable handler
  Test:
    Filter: sdk_sync_waits_for_durable_handler
  Given a Matrix sync response containing a worker command
  When the bridge handler is still awaiting backend acceptance
  Then the SDK event dispatch remains pending
  And the sync token is configured to persist only after processing

Scenario: backend failure keeps Matrix event replayable (critical)
  Tags: critical
  Test:
    Filter: backend_failure_keeps_sync_event_replayable
  Given backend acceptance fails after the bridge retry budget is exhausted
  When the Matrix message handler completes
  Then it rejects instead of converting the failure into a successful result
  And the SDK cannot persist the sync token for that response

Scenario: reserved backend dispatch resumes after restart (critical)
  Tags: critical
  Test:
    Filter: backend_reserved_dispatch_resumes_after_restart
  Given the backend persisted the Matrix message and receipt but crashed before acceptance
  When a new backend instance receives the same source event id
  Then it records one acceptance for the original message id
  And creates no second message

Scenario: backend Matrix receipt survives message retention
  Test:
    Filter: backend_receipt_survives_retention
  Given an accepted Matrix message has been archived by bounded message retention
  When the same source event id is replayed
  Then the backend returns the original message id without a second acceptance

Scenario: mapped-room coordinator reaches the real backend
  Test:
    Filter: mapped_room_coordinator_delivered_by_backend
  Given Matrix sees one implementer while the backend group temporarily lacks the coordinator
  When an unaddressed mapped-room command is routed
  Then the backend acceptance targets wf_coordinator without suppression

Scenario: allowlisted room does not bypass inviter policy
  Test:
    Filter: allowlisted_room_untrusted_inviter_denied
  Given an allowlisted room is invited by an untrusted sender
  When the invite is processed in enforce mode
  Then the bridge rejects the invite without joining

Scenario: Matrix event without event id is not dispatched
  Test:
    Filter: missing_event_id_not_dispatched
  Given a Matrix message has no event id
  When the bridge receives it
  Then no backend dispatch occurs
