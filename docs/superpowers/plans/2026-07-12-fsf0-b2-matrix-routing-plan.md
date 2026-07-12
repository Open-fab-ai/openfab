# FSF-0B2 Matrix Routing Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make authenticated Matrix factory commands exactly-once across bridge restarts and backend/checkpoint crashes while enforcing one trusted routing path.

**Architecture:** Persist terminal Matrix event checkpoints in an fsync-backed JSONL journal and persist `source_event_id` on backend messages as the authoritative idempotency key. The bridge uses one in-flight claim and one invite-policy entry point; mapped-room routing wins over DM heuristics.

**Tech Stack:** Node.js 22 ESM, synchronous filesystem durability primitives, Express, Vitest, Supertest, agent-spec.

## Global Constraints

- Allowed implementation files: `backend-v2.js`, `bridge-matrix.js`, `src/**`, and `tests/**`.
- Do not change Matrix `!` command vocabulary.
- Do not add a dispatch path that bypasses sender policy or durable event checks.
- Accept `source_event_id` only for authenticated Matrix bridge requests.
- Work with existing uncommitted bridge changes; do not revert them.
- Do not use Claude or a remote Matrix server in tests.
- Do not commit before operator testing.

---

### Task 1: Durable Processed-Event Journal

**Files:**
- Create: `src/matrix-event-store.mjs`
- Create: `tests/matrix-event-store.test.js`

**Interfaces:**
- Produces: `MatrixEventStore({ journalPath })` with `has(eventId)`, `get(eventId)`, and `recordProcessed({ eventId, messageId })`.
- Persists: newline-delimited records `{ eventId, messageId, processedAt }`.

- [x] **Step 1: Write failing journal tests**

Cover a missing journal, append plus reload, duplicate id stability, truncated
final-line recovery, malformed middle-line fail closed with the path in the
error, and mode `0600` for a newly created journal.

- [x] **Step 2: Verify RED**

Run:

```bash
npx vitest run tests/matrix-event-store.test.js --no-file-parallelism --maxWorkers=1
```

Expected: module-not-found failure for `src/matrix-event-store.mjs`.

- [x] **Step 3: Implement the journal**

Load only complete newline-terminated JSON records. Truncate an incomplete final
fragment, reject malformed complete records, validate non-empty event ids and
message ids, append one record, call `fsyncSync`, then update the in-memory map.

- [x] **Step 4: Verify GREEN**

Expected: all journal tests pass and no temporary file remains.

### Task 2: Backend Matrix Idempotency

**Files:**
- Modify: `backend-v2.js`
- Modify: `tests/api-messages.test.js`

**Interfaces:**
- Consumes request field: `source_event_id`.
- Persists message field: `sourceEventId`.
- Duplicate response: existing response shape plus `deduped: true` and the
  original `id`; no new message or dispatch event.

- [x] **Step 1: Write failing API tests**

Use an isolated backend runtime. POST the same authenticated Matrix payload
twice and assert one stored message, one SSE/dispatch effect, equal ids, and
`deduped: true` on retry. Add negative cases for unauthenticated bridge secret
and `source=api` so those callers cannot reserve Matrix idempotency keys.

- [x] **Step 2: Verify RED**

Run the exact new filters in `tests/api-messages.test.js`; expect two different
message ids before implementation.

- [x] **Step 3: Implement minimal backend support**

Normalize `source_event_id` to at most 255 characters only when the bridge is
authenticated and `sourceType === 'matrix'`. After request validation and before
id reservation, return the existing Matrix message when the id matches. Store
`sourceEventId` on new messages and expose it through message serialization.

- [x] **Step 4: Verify GREEN and compatibility**

Run all `tests/api-messages.test.js` tests; ordinary API messages must retain
their existing behavior.

### Task 3: Bridge Durable Replay Integration

**Files:**
- Modify: `bridge-matrix.js`
- Create: `tests/fsf0-b2-matrix-routing.test.js`
- Modify: `tests/bridge-matrix.test.js`

**Interfaces:**
- `MatrixBridge` defaults to a journal at
  `<runtime>/data/matrix/processed-events.jsonl` and accepts an injected store
  for focused tests.
- Worker-dispatch payloads include `source_event_id: event.event_id`.

- [x] **Step 1: Write restart and concurrent-delivery tests**

Write `replay_after_restart_zero_duplicates` using two bridge instances backed
by the same real journal. Add a concurrent duplicate test that blocks the first
backend call and invokes the same event twice; assert one backend call.

- [x] **Step 2: Verify RED**

Expect the second bridge instance and concurrent call to dispatch again.

- [x] **Step 3: Integrate journal and in-flight claims**

Remove the eager in-memory `rememberMatrixEvent` call at ingress. After policy
checks, consult the journal, claim the id, add `source_event_id` to group/DM
payloads, checkpoint only a successful result containing `id`, and release the
claim in `finally`. Keep the recent map only for reply-id resolution.

- [x] **Step 4: Verify GREEN**

Run bridge and B2 selector tests; restart and concurrent duplicates produce one
backend call and one persisted checkpoint.

### Task 4: Close the Backend-Acceptance Crash Window

**Files:**
- Modify: `tests/fsf0-b2-matrix-routing.test.js`

**Interfaces:**
- Selector: `accepted_before_checkpoint_replay_zero_duplicates`.

- [x] **Step 1: Write the failing crash-window integration test**

Use the actual isolated backend. Submit a Matrix message with an event id while
injecting a journal append failure, create a restarted bridge with a healthy
journal, and replay the event. Assert one backend message, equal message ids,
and no second dispatch effect.

- [x] **Step 2: Verify RED against either missing idempotency or missing retry**

The test must fail if `source_event_id` is removed from the bridge payload or if
backend dedupe is disabled.

- [x] **Step 3: Make only integration corrections required by the test**

Do not weaken either store or backend tests. Preserve the same idempotency key
through timeout retry and replay.

- [x] **Step 4: Verify GREEN**

Run the crash-window selector three times to detect state or timing flakiness.

### Task 5: Unified Trust and Group Routing

**Files:**
- Modify: `bridge-matrix.js`
- Modify: `tests/bridge-matrix.test.js`
- Modify: `tests/fsf0-b2-matrix-routing.test.js`

**Interfaces:**
- Produces: `MatrixBridge.handleBotInvite(roomId, inviteEvent)` used by realtime
  and poll paths.
- Group default recipient: `wf_coordinator` for the specified mapped-room case.

- [x] **Step 1: Write the four policy/routing selectors**

Add `routes_mapped_room_as_group_wakes_coordinator`,
`untrusted_inviter_denied`, `ignored_sender_not_routed`, and
`create_issue_routed_exactly_once`. The mapped room must contain one non-
coordinator Matrix agent so the test distinguishes group routing from agent DM.

- [x] **Step 2: Verify RED for current helper behavior and split invite paths**

Expect the mapped-room test to target the sole agent and the invite tests to
show separate handling before integration.

- [x] **Step 3: Implement the shared policy path**

Mapped room routing precedes DM detection. With no explicit mention in the
specified case, use `wf_coordinator`. Move realtime and polled bot invite logic
through `handleBotInvite`; ignored senders return before route or backend work.

- [x] **Step 4: Verify GREEN**

All seven exact B2 selectors pass with no remote Matrix access.

### Task 6: Contract Gate and Candidate Evidence

**Files:**
- Modify: `docs/acceptance/2026-07-12-fsf-0-acceptance-record.md`
- Create: `docs/acceptance/evidence/2026-07-12-fsf0-b2-matrix-routing.md`

**Interfaces:**
- Produces an exact-byte manifest digest, test commands, revisions, limitations,
  and independent review verdict without changing overall NOT ACCEPTED status.

- [ ] **Step 1: Run focused and exact verification**

Run journal, backend message, bridge, and selector tests together; run every B2
selector three times; run syntax and `git diff --check`.

- [ ] **Step 2: Run agent-spec lifecycle**

Use explicit B2 changed paths because the worktree contains unrelated B1 and
operator changes:

```bash
agent-spec lifecycle specs/phase1/fsf0-b2-matrix-routing.spec.md \
  --code /Users/zhangalex/Work/Projects/consult/agent-chat \
  --change backend-v2.js --change bridge-matrix.js \
  --change src/matrix-event-store.mjs \
  --change tests/api-messages.test.js \
  --change tests/bridge-matrix.test.js \
  --change tests/matrix-event-store.test.js \
  --change tests/fsf0-b2-matrix-routing.test.js \
  --format json
```

- [ ] **Step 3: Independent read-only review**

Require no Critical/High/Medium findings for durability, idempotency,
authentication, trust policy, and test strength. Reproduce and fix findings by
TDD before evidence generation.

- [ ] **Step 4: Write PARTIAL candidate evidence**

Record dirty-worktree status, exact bytes, full-suite disclosure, and missing
real Palpo/Robrix E2E. Do not sign, accept, commit, or mark FSF-0 complete.

- [ ] **Step 5: Present operator test commands**

Report the local B1 service commands plus B2 selector command. Leave all bytes
uncommitted until the operator completes testing.
