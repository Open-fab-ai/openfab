# FSF-0B2 Matrix Routing Reliability Design

## Scope

B2 makes the factory command path from Matrix to Agent Chat exactly-once across
bridge restarts and the backend-acceptance crash window. It also consolidates
room invite trust checks, rejects ignored appservice senders before dispatch,
and preserves mapped-room group routing even when Matrix membership resembles a
DM. Bridge-local `!` administration commands keep their existing vocabulary and
are not part of the worker-dispatch idempotency contract.

The implementation works with the current uncommitted Matrix bridge changes.
It does not rewrite or revert the existing room backfill, outbound DM, avatar,
or polling behavior.

## Architecture

### Durable event journal

`src/matrix-event-store.mjs` owns
`<runtime>/data/matrix/processed-events.jsonl`. Each complete line records one
terminally accepted Matrix event id, backend message id, and timestamp. Appends
are followed by `fsync`. A truncated final line is discarded before the next
append because backend idempotency safely closes that crash window. Malformed
data before the final line fails bridge initialization and names the journal;
silently forgetting processed ids is forbidden.

The store never evicts event ids. Replay safety takes precedence over bounded
in-memory retention; compaction requires a later design with an equivalent
durability proof.

### Backend idempotency

The authenticated Matrix bridge adds `source_event_id` to `/api/messages`.
`backend-v2.js` accepts Matrix ingestion only when the shared bridge secret is
configured, authentication succeeds, and the event id is present. Before
dispatch it appends an fsynced reservation to
`<runtime>/data/matrix/source-events.jsonl`, including the immutable message
bytes and message id. It commits that receipt only after the message and a
stable-attempt `message.accepted` delivery event are durable. A retry resumes a
reserved receipt or returns a committed receipt's original message id with
`deduped: true`.

Receipts never follow bounded `messages.json` retention. Archiving an old
message therefore cannot release its idempotency key. The persisted inbox
message plus `message.accepted` event are the authoritative wake; SSE and push
notifications remain best-effort accelerators after that durable commit.

### SDK sync checkpoint

The bot uses a `MatrixClient` subclass whose sync dispatcher awaits bridge
handlers and sets `persistTokenAfterSync=true`. The SDK writes `next_batch`
only after all events in the response have completed. A backend or checkpoint
failure rejects processing, leaves the old token durable, and causes the
homeserver event to replay.

`submitHumanMessage` may send a Matrix delivery-failure notice, but it never
turns a terminal backend error into a resolved handler result. Missing message
ids are also treated as failed acceptance. This preserves the SDK rejection
signal that keeps the prior sync token durable.

### Inbound flow

For a worker-dispatch message, `MatrixBridge.onRoomMessage` performs:

1. Reject malformed content, bridge/agent/ignored senders, and untrusted rooms.
2. Reject the event without dispatch if `event_id` is absent.
3. Return immediately if the durable journal already contains `event_id`.
4. Claim the id in a per-process in-flight promise map. Concurrent delivery
   awaits the same attempt and shares its success or failure.
5. Resolve mapped-room routing before DM heuristics. With no explicit mention,
   a mapped room containing one Matrix agent wakes `wf_coordinator`.
   The bridge marks this generated fallback so the authenticated backend can
   deliver it during temporary Matrix/backend group-membership skew.
6. Submit `/api/messages` with `source_event_id`.
7. After backend acceptance, append the durable checkpoint with the returned
   message id. On delivery failure, do not checkpoint, so replay retries safely.
8. Release the in-flight claim in `finally`.

The existing recent-event map remains a bounded reply-event lookup cache, not
the source of truth for replay suppression.

### Invite policy

Realtime bot invites, polled bot invites, and agent invite polling apply the
same mandatory trusted-inviter check. Room allowlisting or prior managed-room
status cannot override an untrusted inviter in enforce mode. Ignored Matrix
senders are filtered before route resolution and backend calls.

## Failure Handling

- Existing but malformed event journal: bridge initialization fails closed.
- Complete blank journal record: bridge initialization fails closed.
- Truncated final journal write: truncate to the last complete newline; backend
  idempotency prevents duplicate dispatch if that event had already arrived.
- Existing journal permissions are repaired to `0600`; journal directories are
  repaired to `0700`; file and relevant directory metadata are fsynced.
- Backend timeout after acceptance: retry uses the same `source_event_id` and
  receives the original message.
- Backend restart after message persistence but before acceptance: the reserved
  receipt restores the original message id and appends one stable acceptance.
- Message retention archival: the independent receipt still deduplicates replay.
- Journal append failure after backend acceptance: surface failure and leave the
  event uncheckpointed; replay is still deduped by the backend.
- Missing/incorrect bridge credentials or missing Matrix event id: reject
  ingestion without message or dispatch side effects.

## Verification

The agent-spec selectors cover mapped-room routing, SDK checkpoint ordering,
bridge and backend restart windows, retention-safe receipts, authenticated
coordinator fallback, invite denial, ignored sender denial, required event ids,
single create-issue dispatch, and corrupted-journal fail-closed behavior.
Focused store and backend tests additionally cover tail truncation, blank
records, permission repair, exact id reuse, payload mismatch dedupe, and
ordinary non-Matrix message compatibility.

No test starts Claude or contacts a remote Matrix server. No commit is created
before operator testing.
