# OpenFab — Full End-to-End Demo Walkthrough

A copy-paste checklist to run the whole pipeline: **natural-language requirements →
agent-spec contract → implementation by a Matrix agent → agent-spec verification →
in-toto/SLSA signing → N-of-M human sign-off (via Robrix or dashboard) → merge**, all
visible in the Phase 2 console.

There are two tracks:
- **Track A — Dashboard e2e** (no Robrix GUI needed). Fastest proof of the whole chain.
- **Track B — Full Robrix e2e** (requirements chat + approval in Matrix). The complete story.

---

## 0. Preflight — confirm services are up

```bash
# Palpo (Matrix server)
curl -s -o /dev/null -w "Palpo        %{http_code}\n" http://127.0.0.1:8128/_matrix/client/versions
# agent-chat backend
curl -s -o /dev/null -w "agent-chat   %{http_code}\n" "http://127.0.0.1:8090/api/agents?view=names"
# OpenFab↔agent-chat Bridge
curl -s http://127.0.0.1:8077/healthz ; echo "  (bridge)"
# OpenFab dashboard/console (agent-chat must show runtime=native)
curl -s http://127.0.0.1:8787/api/bases | python3 -c "import sys,json;print('agent-chat runtime:',[b['runtime'] for b in json.load(sys.stdin) if b['id']=='agent-chat'][0])"
# implementer agent online?
curl -s http://127.0.0.1:8090/api/agents | python3 -c "import sys,json;d=json.load(sys.stdin);ag=d if isinstance(d,list) else d.get('agents',[]);print('online:',[a['name'] for a in ag if a.get('online') or a.get('agentOnline')])"
```

Expect: Palpo 200, agent-chat 200, bridge `{"ok":true}`, **agent-chat runtime: native**,
online includes `wf_implementer`.

### If something is down — restart commands

```bash
cd ~/Work/Projects/FW/openfab
AC=~/Work/Projects/consult/agent-chat
API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')

# Bridge (:8077) — with approval relay → OpenFab
lsof -ti tcp:8077 | xargs kill 2>/dev/null
AGENTCHAT_URL=http://127.0.0.1:8090 AGENTCHAT_API_TOKEN="$API_TOKEN" AGENTCHAT_DIR="$AC" \
  BRIDGE_ASSIGNEE=wf_implementer BRIDGE_PORT=8077 OPENFAB_URL=http://127.0.0.1:8787 \
  nohup node bridge/openfab-agentchat-bridge.mjs > /tmp/of_bridge.log 2>&1 & disown

# Dashboard/console (:8787) — agent-spec mode + agent-chat native
lsof -ti tcp:8787 | xargs kill 2>/dev/null
env -u ANTHROPIC_API_KEY OPENFAB_SPEC=agent-spec OPENFAB_SPEC_DIR="$PWD/demo/.work/web/specs" \
  OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 OPENFAB_AGENTCHAT_ROOM=openfab \
  nohup ./target/release/openfab serve --repo demo/.work/web --port 8787 --policy policy/trust.json \
  > /tmp/of_serve.log 2>&1 & disown

# wf_implementer agent (if not online) — link skill + launch
ln -sfn "$PWD/bridge/skills/openfab-implementer" "$HOME/.claude/skills/openfab-implementer"
cd "$AC"; set -a; source .env; set +a
env -u TMUX -u ANTHROPIC_API_KEY "$AC/bin/agentchat" up-v1 wf_implementer claude \
  --project /tmp/openfab-impl-ws --project-mode symlink --allow-shared-workspace --fresh
cd ~/Work/Projects/FW/openfab
```

> Note: OpenFab spawns `claude -p` with `--setting-sources project,local` and the demo
> uses `env -u ANTHROPIC_API_KEY` so the CLI uses your subscription login (an invalid
> `ANTHROPIC_API_KEY` would 401). Keep `~/.zshrc`'s `ANTHROPIC_API_KEY` line commented.

---

## Track A — Dashboard e2e (fastest, no Robrix)

1. **Open the console:** http://127.0.0.1:8787/console  (classic dashboard: http://127.0.0.1:8787/)

2. **Ensure maintainers exist** (the trust gate needs 2 for "team"). On the classic
   dashboard add `alice` and `bob`, or:
   ```bash
   curl -s -XPOST http://127.0.0.1:8787/api/maintainers -d '{"name":"alice"}'
   curl -s -XPOST http://127.0.0.1:8787/api/maintainers -d '{"name":"bob"}'
   ```

3. **Fabricate.** On the classic dashboard (`/`):
   - Intent: e.g. *"Build a Rust CLI that multiplies two integers and prints the product."*
   - Base: **agent-chat**  ·  Forge: **local**  ·  Gate: **team**
   - Click **⚙ Fabricate**.

4. **Watch the pipeline.** In `/console`, the run appears on the board (lane *implementing*).
   Click it to see the **stage pipeline** advance:
   `spec → implement → verify → sign → gate → merge`.
   - `spec`: agent-spec drafts the `.spec.md`, lint-gated.
   - `implement`: dispatched to **wf_implementer** in Matrix (watch tmux: click the agent in
     the Agents panel, or `tmux attach -t wf_implementer`). It writes code + bound tests and
     replies with a `task_result`.
   - `verify`: OpenFab runs `agent-spec lifecycle` against the returned files.
   - `sign`: in-toto/SLSA attestation. `gate`: blocked, awaiting N-of-M.

5. **Sign off (2-of-2).** On the classic dashboard click **Sign off** as `alice`, then `bob`
   (or via API — get the run id from the board):
   ```bash
   RUN=<run_id>
   curl -s -XPOST http://127.0.0.1:8787/api/runs/$RUN/signoff -d '{"as":"alice"}'
   curl -s -XPOST http://127.0.0.1:8787/api/runs/$RUN/signoff -d '{"as":"bob"}'
   ```
   The gate flips to **ACCEPTED → merged**; the board card moves to lane *merged*.

6. **Inspect in `/console`.** Click the run:
   - **Docs** tab: the spec contract, README, and generated code (committed, travels with it).
   - **Verify** tab: conformance **C1–C12 all PASS** (incl. C12 agent-spec scenarios).
   - **Audit** tab: the signed provenance + commit graph.

✅ That is the full trust chain: NL → contract → Matrix-agent implementation → machine
verification → signature → human N-of-M → merge.

---

## Track B — Full Robrix e2e (requirements chat + Matrix approval)

Adds the two human-collaboration ends: **requirements conversation** and **approval in a
Matrix room**.

### B1. Launch the coordinator agent (requirements conversation)

```bash
AC=~/Work/Projects/consult/agent-chat
ln -sfn ~/Work/Projects/FW/openfab/bridge/skills/openfab-coordinator "$HOME/.claude/skills/openfab-coordinator"
cd "$AC"; set -a; source .env; set +a
env -u TMUX -u ANTHROPIC_API_KEY "$AC/bin/agentchat" up-v1 wf_coordinator claude \
  --project /tmp/openfab-coord-ws --project-mode symlink --allow-shared-workspace --fresh
```

### B2. Map your Matrix identity to a maintainer (security)

Only a **mapped** Matrix user can sign. Log into Robrix with your Palpo account, note your
mxid (form `@<you>:127.0.0.1:8128`), then map it to a maintainer:

```bash
curl -s -XPOST http://127.0.0.1:8787/api/identity \
  -d '{"mxid":"@you:127.0.0.1:8128","maintainer":"alice"}'
# (do the same for a second person → bob, to satisfy 2-of-2)
```
Verify the security property — an **unmapped** mxid must be rejected:
```bash
curl -s -XPOST http://127.0.0.1:8787/api/runs/SOME_RUN/signoff -d '{"mxid":"@stranger:127.0.0.1:8128"}'
# → error: "matrix user ... is not mapped to any maintainer — cannot sign"
```

### B3. In Robrix: chat requirements → spec

1. Open Robrix, join/create the group room that contains `wf_coordinator` and
   `wf_implementer` (e.g. `!mkgroup demoboard wf_coordinator wf_implementer`).
2. Send `/create-issue <title> | <description>` or just describe what you want.
3. The coordinator **asks clarifying questions** (goal, I/O, constraints, acceptance,
   out-of-scope). Answer until it confirms.
4. It writes `specs/<id>.requirements.md` + `specs/<id>.spec.md` and posts
   **"Spec ready for approval: `<id>` — reply `approve <id>`"**.

### B4. Build from the conversation

```bash
ID=<id-from-coordinator>
env -u ANTHROPIC_API_KEY OPENFAB_SPEC_FILE="$PWD/demo/.work/web/specs/$ID.spec.md" \
  OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 OPENFAB_AGENTCHAT_ROOM=openfab \
  ./target/release/openfab build "<the intent>" --repo demo/.work/web \
  --base agent-chat --forge local --gate team --policy policy/trust.json
```
(Or trigger it from the dashboard — the requirements doc is committed and its hash appears
in the attestation as `requirements_sha256`.)

### B5. Approve in Robrix

In the room, the mapped human posts:  `approve <run_id>`
The Bridge's approval poller relays it to OpenFab as an N-of-M sign-off **as the mapped
maintainer**. A second mapped human posts `approve <run_id>` → gate ACCEPTED → merged.
OpenFab posts the gate/provenance summary back into the room.

✅ Full story: requirements chat (human↔coordinator) → spec → implement (Matrix agent) →
verify → sign → approve in Matrix (human↔gate, N-of-M) → merge — visible in `/console`.

---

## What to point at when demoing

| Claim | Where to show it |
|-------|------------------|
| NL → software | the intent box / coordinator chat → working code in Docs tab |
| Trustworthy | Verify tab: C1–C12 PASS; signed in-toto/SLSA attestation |
| AI/Human attribution | provenance predicate `generated[]` (ai) + `signoffs[]` (human) |
| Requirements traceable | `requirements_sha256` in the attestation; committed `requirements.md` |
| Real multi-agent | Agents panel + tmux peek of `wf_implementer` doing the work |
| Human-in-the-loop | gate BLOCKED until 2-of-2; approve in Robrix or dashboard |
| Identity safety | unmapped mxid `approve` is refused |
| Neutral / portable | provenance + spec committed into the repo; `local` forge pseudo-PR |
| Reproducible | `ops::reproduce` re-runs `agent-spec lifecycle` + re-checks signature + source hash |

---

## Cleanup / reset between demos

```bash
# delete OpenFab agent-chat tasks (so the board/queue is clean)
AC=~/Work/Projects/consult/agent-chat
API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')
for t in $(curl -s http://127.0.0.1:8090/api/tasks | python3 -c "import sys,json;[print(x['id']) for x in json.load(sys.stdin) if 'openfab' in (x.get('labels') or [])]"); do
  curl -s -XDELETE "http://127.0.0.1:8090/api/tasks/$t" -H "Authorization: Bearer $API_TOKEN" >/dev/null
done
# fresh OpenFab workspace (wipes prior runs/merges)
rm -rf demo/.work/web && mkdir -p demo/.work/web
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Fabricate stuck at *implement* | is `wf_implementer` online? `tmux ls`; check `/tmp/of_bridge.log`; nudge: `bash ~/Work/Projects/FW/robius/robrix2/roadmap/agentchat-demo/nudge.sh wf_implementer` |
| `claude … 401 Invalid API key` | `~/.zshrc` exports a stale `ANTHROPIC_API_KEY`; keep it commented; services started with `env -u ANTHROPIC_API_KEY` |
| agent-chat base shows `bridged` not `native` | serve wasn't started with `OPENFAB_AGENTCHAT_URL`; restart per §0 |
| `bridge file integrity check failed` | implementer didn't return bit-identical full files; see `bridge/README.md` result contract |
| Agents panel empty in console | Bridge down or serve missing `OPENFAB_AGENTCHAT_URL`; restart per §0 |
| sign-off "not a registered maintainer" | add the maintainer first (`/api/maintainers`) and, for Robrix, map the mxid (`/api/identity`) |
