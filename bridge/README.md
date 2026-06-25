# OpenFab ↔ agent-chat Bridge (Phase 1)

A zero-dependency Node sidecar that lets OpenFab (blocking HTTP, single binary, no async
runtime) drive the agent-chat multi-agent team running over Matrix. **OpenFab stays the
source of truth and drives the workflow**; the agent-chat implementer agent in a Matrix
room only does the *implement* segment. OpenFab then verifies (agent-spec lifecycle),
signs (in-toto/SLSA), gates (conformance + N-of-M), and reproduces.

```
OpenFab ──blocking HTTP──▶ Bridge ──HTTP──▶ agent-chat backend (:8090) ──▶ Matrix room (Palpo)
 (bridge_client.rs)        (this dir)        /api/tasks /api/messages /api/dm/{op}/history
```

## OpenFab-facing API (consumed by `src/adapters/bridge_client.rs`)

| Method | Path | Body | Response |
|--------|------|------|----------|
| POST | `/tasks` | `{spec_ref,intent,target_dir,language,acceptance,assumptions,context,room}` | `{task_id}` |
| GET | `/tasks/:id` | — | `{status:"running\|done\|failed", files:{path:content}, file_hashes:{path:sha256}, model, prompt, error?}` |
| POST | `/post` | `{room,msg}` | `{ok:true}` |
| GET | `/healthz` | — | `{ok:true}` |

## Agent-side RESULT CONTRACT (load-bearing for trust)

OpenFab can only sign bytes it can hash. The implementer agent MUST return **bit-identical
full file contents** plus the prompt it worked from, by posting a message:

```json
{ "schema": { "kind": "task_result", "version": 1, "payload": {
    "task_id": "<agent-chat task id>",
    "status": "completed",
    "model": "<model id>",
    "prompt": "<the prompt the agent worked from>",
    "files": { "app/add.py": "<full file content>", "app/test_add.py": "<full content>" }
}}}
```

The Bridge computes `file_hashes` from the returned content; `bridge_client.rs`
(`BridgeResult::verify_integrity`) re-checks them before OpenFab signs. The implementer
must also write tests matching the contract's `Test:` selectors so `agent-spec lifecycle`
(OpenFab's verify step) passes.

> This contract is implemented in the shared `issue-workflow` skill branch for the
> `wf_implementer` role (see the robrix2 agent-chat demo). Wiring that skill branch is a
> manual step in the final checklist.

## Config (env)

| Var | Default | Meaning |
|-----|---------|---------|
| `BRIDGE_PORT` | `8077` | OpenFab-facing port (set `OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077`) |
| `AGENTCHAT_URL` | `http://127.0.0.1:8090` | agent-chat backend |
| `AGENTCHAT_API_TOKEN` | — | operator Bearer token |
| `BRIDGE_ASSIGNEE` | `wf_implementer` | implementer agent |
| `BRIDGE_OPERATOR` | `operator` | DM history owner to harvest results from |

## Run

```bash
node bridge/openfab-agentchat-bridge.mjs
# then point OpenFab at it:
export OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077
export OPENFAB_AGENTCHAT_ROOM='!demoboard:localhost'   # the Matrix room id
openfab build "…" --base agent-chat --forge local …
```

## Status

Code complete + syntax-checked. **Live end-to-end requires a running agent-chat backend +
Palpo + the implementer skill branch** — see the final checklist (Phase 1-Env).
