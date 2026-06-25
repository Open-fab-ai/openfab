# OpenFab — Complete End-to-End Test Plan (with Robrix)

A single, sequential, checkbox checklist to validate the whole stack: trust skeleton →
agent-chat implementer over Matrix → collaborative console → self-hosting → GitHub forge →
**Robrix GUI** (requirements chat + approval). Run top to bottom; each item has an action and
the expected result.

Legend: ⌨️ = run a command · 🖱️ = do it in a browser/GUI · ✅ = expected result.

Paths assumed:
- OpenFab: `~/Work/Projects/FW/openfab`
- agent-chat: `~/Work/Projects/consult/agent-chat`
- robrix2: `~/Work/Projects/FW/robius/robrix2`
- Palpo: docker (OrbStack), Matrix at `http://127.0.0.1:8128`, server name `127.0.0.1:8128`

---

## 0. Environment bring-up

- [ ] ⌨️ **Palpo up** (docker): `curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8128/_matrix/client/versions` ✅ `200`
- [ ] ⌨️ **agent-chat backend up** (:8090): `curl -s "http://127.0.0.1:8090/api/agents?view=names"` ✅ returns the agent name list (`wf_coordinator`, `wf_implementer`, …)
- [ ] ⌨️ **agent-chat dashboard/monitor up** (:8084): `curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8084/` ✅ `200` (the Agent Monitor)
- [ ] ⌨️ **LLM auth**: keep `~/.zshrc`'s `ANTHROPIC_API_KEY` commented; run OpenFab/agents with `env -u ANTHROPIC_API_KEY …`. Sanity: `env -u ANTHROPIC_API_KEY claude -p "say OK" --setting-sources project,local --output-format json` ✅ `is_error:false`
- [ ] ⌨️ **Build OpenFab**: `cd ~/Work/Projects/FW/openfab && cargo build --release` ✅ builds
- [ ] ⌨️ **Quality gate**: `cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check` ✅ 96 tests pass, clippy/fmt clean
- [ ] ⌨️ **Dogfood specs**: `for s in specs/phase2/*.spec.md; do agent-spec lifecycle "$s" --code . --format json | python3 -c "import sys,json;d=json.load(sys.stdin);print('$s', d['passed'])"; done` ✅ all `True`

Start the OpenFab-side services:

- [ ] ⌨️ **Bridge** (:8077):
  ```bash
  cd ~/Work/Projects/FW/openfab; AC=~/Work/Projects/consult/agent-chat
  API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')
  AGENTCHAT_URL=http://127.0.0.1:8090 AGENTCHAT_API_TOKEN="$API_TOKEN" AGENTCHAT_DIR="$AC" \
    BRIDGE_ASSIGNEE=wf_implementer BRIDGE_PORT=8077 OPENFAB_URL=http://127.0.0.1:8787 \
    nohup node bridge/openfab-agentchat-bridge.mjs > /tmp/of_bridge.log 2>&1 & disown
  ```
  ✅ `curl -s http://127.0.0.1:8077/healthz` → `{"ok":true}`; log shows `registered sender agent "openfab-bridge"`
- [ ] ⌨️ **OpenFab serve** (:8787, agent-spec + native agent-chat + projects + monitor):
  ```bash
  env -u ANTHROPIC_API_KEY OPENFAB_SPEC=agent-spec OPENFAB_SPEC_DIR="$PWD/demo/.work/web/specs" \
    OPENFAB_PROJECTS_DIR="$PWD/demo/.work/projects" OPENFAB_AGENTCHAT_MONITOR=http://127.0.0.1:8084 \
    OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 OPENFAB_AGENTCHAT_ROOM=openfab \
    nohup ./target/release/openfab serve --repo demo/.work/web --port 8787 --policy policy/trust.json \
    > /tmp/of_serve.log 2>&1 & disown
  ```
  ✅ `curl -s http://127.0.0.1:8787/api/forges` shows forges; `/api/bases` shows `agent-chat runtime: native`

---

## 1. Phase 0 — local trust skeleton (dashboard, no Matrix)

- [ ] 🖱️ Open `http://127.0.0.1:8787/` ✅ dashboard loads with the new **Project** selector + **Projects** card
- [ ] ⌨️ Add maintainers: `for m in alice bob; do curl -s -XPOST -H "Origin: http://127.0.0.1:8787" http://127.0.0.1:8787/api/maintainers -d "{\"name\":\"$m\"}"; done` ✅ each returns a `did:key`
- [ ] 🖱️ Intent: *"Build a Rust CLI that multiplies two integers and prints the product."* · base **claude** · forge **local** · gate **team** · **⚙ Fabricate**
- [ ] 🖱️ Live workflow advances: Spec → Generate → Verify → Sign → **Gate: BLOCKED** ✅ `agent-spec lifecycle` scenarios pass; attestation signed
- [ ] 🖱️ Sign off as **alice**, then **bob** ✅ gate **ACCEPTED → merged**
- [ ] 🖱️ **Verify** tab ✅ conformance **C1–C12 all PASS** (incl. C12 agent-spec scenarios)
- [ ] 🖱️ Reload the page ✅ the run is **restored** from "Recent runs" (no state lost)

---

## 2. Phase 1 — agent-chat implementer over Matrix (no Robrix GUI yet)

- [ ] ⌨️ Launch the implementer agent (its skill is `openfab-implementer`):
  ```bash
  AC=~/Work/Projects/consult/agent-chat
  ln -sfn ~/Work/Projects/FW/openfab/bridge/skills/openfab-implementer "$HOME/.claude/skills/openfab-implementer"
  cd "$AC"; set -a; source .env; set +a
  env -u TMUX -u ANTHROPIC_API_KEY "$AC/bin/agentchat" up-v1 wf_implementer claude \
    --project /tmp/openfab-impl-ws --project-mode symlink --allow-shared-workspace --fresh
  ```
  ✅ `curl -s http://127.0.0.1:8090/api/agents/wf_implementer | python3 -c "import sys,json;print(json.load(sys.stdin)['online'])"` → `True`; `tmux ls` shows `wf_implementer`
- [ ] 🖱️ On the dashboard, Fabricate again with **base = agent-chat**, gate **team**
- [ ] 🖱️ Watch: OpenFab dispatches into the room; the **implementer writes code+tests**; OpenFab harvests, integrity-checks, verifies (`lifecycle`), signs ✅ run reaches **blocked** (awaiting sign-off), `base_runtime=native`
- [ ] ⌨️ Confirm the implementer really ran (not bridged): `curl -s http://127.0.0.1:8787/api/runs | python3 -c "import sys,json;[print(r['run_id'],r.get('base_runtime')) for r in json.load(sys.stdin)[-1:]]"` ✅ `native`
- [ ] 🖱️ Sign off alice+bob → merged ✅ a real Matrix-agent-built, OpenFab-signed product

---

## 3. Phase 2 — console features

- [ ] 🖱️ Open `http://127.0.0.1:8787/console` ✅ same look as dashboard; header has **Project** selector + **Fabricate/Console** nav
- [ ] 🖱️ **Board** shows runs in lanes; a blocked run sits in **"Awaiting sign-off"** (not "Review")
- [ ] 🖱️ Click a run → **stage pipeline** lights up; **Docs** tab shows spec/README/code; **Verify** shows C1–C12; **Audit** shows provenance
- [ ] 🖱️ For a blocked run → the **in-console sign-off** panel appears; click "Sign off as alice"/"bob" ✅ gate flips to merged, board updates
- [ ] 🖱️ **Agents** panel lists agents (online dot); click `wf_implementer` → **Agent Monitor** (agent-chat :8084) renders in the tmux panel + "open ↗" link
- [ ] 🖱️ **Identity mapping**: map a Matrix user `POST /api/identity {mxid,maintainer}` (used in §5) ✅ stored

### Multi-project
- [ ] 🖱️ Dashboard → **Projects** card → New project `mobile` (no path) → Create ✅ appears in selector
- [ ] 🖱️ Switch project to `mobile` ✅ board/maintainers are **empty/isolated** (separate workspace)
- [ ] ⌨️ Security: `curl -s "http://127.0.0.1:8787/api/board?project=ghost"` ✅ `{"error":"unknown project 'ghost'"}`

### Document upload
- [ ] 🖱️ Dashboard → **📎 Upload requirements / spec doc** → pick a `.spec.md` ✅ "spec contract `<id>.spec.md`"; Fabricate builds it directly
- [ ] 🖱️ Upload a prose requirements `.md` ✅ "requirements `<id>.requirements.md` attached"; after a build its hash appears in the attestation (`requirements_sha256`)

---

## 4. Phase 2.1 — self-hosting (git worktree) + GitHub forge

### Self-hosting (dogfood OpenFab with OpenFab)
- [ ] 🖱️ Projects card → name `selfdev`, repo path `~/Work/Projects/FW/openfab`, **☑ Isolate with a git worktree** → Create
- [ ] ⌨️ `git -C ~/Work/Projects/FW/openfab worktree list` ✅ shows `…/demo/.work/projects/selfdev [openfab/selfdev]`; your **main checkout is untouched**
- [ ] 🖱️ Switch to `selfdev`; (optional) build a small spec against OpenFab itself → verify/sign/gate in the isolated worktree

### GitHub REST API forge (token, no `gh` CLI)
- [ ] ⌨️ `export OPENFAB_GITHUB_TOKEN=ghp_xxx OPENFAB_GITHUB_REPO=youruser/yourrepo` (PAT: repo scope, or fine-grained Contents+PR write), restart serve
- [ ] ⌨️ `curl -s http://127.0.0.1:8787/api/forges | python3 -c "import sys,json;print([f['note'] for f in json.load(sys.stdin) if f['id']=='github'][0])"` ✅ `live — GitHub REST API (token)`
- [ ] 🖱️ Fabricate with forge **GitHub** ✅ a real branch is pushed and a **PR appears on GitHub**; the run records the PR url
- [ ] ⌨️ Token-leak regression: trigger a push failure (e.g. wrong repo) and read `/api/runs/:id/events` ✅ the token is **redacted** (`***`), not present; `.git/config` of any clone has **no** token

---

## 5. Robrix GUI — requirements chat + Matrix approval (the full story)

### 5a. Bring up the agent team + coordinator
- [ ] ⌨️ Link the coordinator skill + launch it:
  ```bash
  AC=~/Work/Projects/consult/agent-chat
  ln -sfn ~/Work/Projects/FW/openfab/bridge/skills/openfab-coordinator "$HOME/.claude/skills/openfab-coordinator"
  cd "$AC"; set -a; source .env; set +a
  env -u TMUX -u ANTHROPIC_API_KEY "$AC/bin/agentchat" up-v1 wf_coordinator claude \
    --project /tmp/openfab-coord-ws --project-mode symlink --allow-shared-workspace --fresh
  ```
  ✅ `wf_coordinator` online (and `wf_implementer` from §2 still online)
- [ ] ⌨️ (If accounts not yet created) pre-create Matrix accounts: `cd ~/Work/Projects/FW/robius/robrix2/roadmap/agentchat-demo && node register-accounts.mjs` ✅ bot + agent accounts exist on Palpo

### 5b. Launch Robrix and log into Palpo
- [ ] ⌨️ Build/run Robrix with the workflow slash commands compiled in:
  ```bash
  cd ~/Work/Projects/FW/robius/robrix2
  cargo run --features agent_chat
  ```
  ✅ the Robrix window opens
- [ ] 🖱️ **Log in** to the Palpo homeserver (`http://127.0.0.1:8128`, server name `127.0.0.1:8128`) with your user account ✅ logged in, room list syncs
- [ ] 🖱️ **Settings → Preferences →** enable **"Agent-chat support (experimental)"** ✅ toggle on (runtime gate for the workflow slash commands)

### 5c. Bind the room to an OpenFab project
- [ ] 🖱️ Create/open the group room that contains the agents, e.g. send `!mkgroup demoboard wf_coordinator wf_implementer wf_reviewer wf_final_reviewer` (via the bot) ✅ room `demoboard` exists with the agents as members
- [ ] ⌨️ Find the room id (`!xxxx:127.0.0.1:8128`) and bind it to a project:
  ```bash
  curl -s -XPOST -H "Origin: http://127.0.0.1:8787" http://127.0.0.1:8787/api/rooms \
    -d '{"room":"!demoboard:127.0.0.1:8128","project":"default"}'
  ```
  ✅ `{"room":…,"project":"default"}`
- [ ] ⌨️ Map your Robrix Matrix user → a maintainer (only mapped users may sign):
  ```bash
  curl -s -XPOST -H "Origin: http://127.0.0.1:8787" http://127.0.0.1:8787/api/identity \
    -d '{"mxid":"@you:127.0.0.1:8128","maintainer":"alice"}'
  # repeat for a 2nd person → bob, to satisfy 2-of-2
  ```
  ✅ mapping stored

### 5d. Requirements conversation (human ↔ coordinator)
- [ ] 🖱️ In the room, the `/` menu shows the workflow commands (room has a `*_coordinator`, feature on). Send `/create-issue <title> | <description>` (or describe the need in plain text) ✅ coordinator replies
- [ ] 🖱️ Answer the coordinator's clarifying questions (goal, I/O, constraints, acceptance, out-of-scope) until it confirms ✅ multi-turn requirements chat visible in the room
- [ ] 🖱️ Coordinator writes `requirements.md` + `.spec.md` and posts **"Spec ready for approval: `<id>` — reply `approve <id>`"** ✅ message in room
- [ ] 🖱️ Open `http://127.0.0.1:8787/` → **📥 Incoming from Robrix** panel shows `<id>` (📄 = has requirements) ✅ the doc was ingested via the Bridge `submit-doc` → `/api/ingest` to the bound project (no upload needed)

### 5e. Build + approve from Robrix
- [ ] 🖱️ Click **Build** on the Incoming `<id>` (or Fabricate with base agent-chat) ✅ OpenFab builds from the ingested contract; implementer runs in the room; reaches **blocked**
- [ ] 🖱️ In the **Robrix room**, the mapped human posts `approve <run_id>` ✅ the Bridge approval poller relays it → OpenFab N-of-M sign-off **as the mapped maintainer**
- [ ] 🖱️ A second mapped human posts `approve <run_id>` ✅ gate **ACCEPTED → merged**; OpenFab posts the gate/provenance summary back into the room
- [ ] 🖱️ `/status` in the room ✅ shows the workflow state
- [ ] ⌨️ **Security regression — spoofing**: append a non-Matrix message claiming a maintainer's `sender_mxid` with body `approve <run>` (or use a non-mapped Matrix user) ✅ it is **NOT** honored (bridge requires `source==='matrix'`; OpenFab rejects unmapped mxids)

---

## 6. Security regression spot-checks

- [ ] ⌨️ **CSRF**: `curl -s -o /dev/null -w "%{http_code}\n" -XPOST -H "Origin: http://evil.com" http://127.0.0.1:8787/api/projects -d '{"name":"x"}'` ✅ `403`
- [ ] ⌨️ **Path traversal (ingest)**: `curl -s -XPOST -H "Origin: http://127.0.0.1:8787" http://127.0.0.1:8787/api/ingest -d '{"id":"../../tmp/pwn","spec_md":"x"}'` ✅ `{"error":"invalid id"}`, no file written
- [ ] ⌨️ **Unknown project**: `curl -s "http://127.0.0.1:8787/api/board?project=ghost"` ✅ error, not an arbitrary path
- [ ] 🖱️ **XSS**: create a project named `<img src=x onerror=alert(1)>` (via API) and view the Projects list ✅ it renders as **text**, no script runs
- [ ] ⌨️ **Bridge integrity**: (unit) `cargo test --lib bridge_client` ✅ a returned file with no hash, or a tampered file, fails `verify_integrity`

---

## 7. Reproduce / sovereignty

- [ ] 🖱️ For a merged run → **Reproduce & verify** ✅ signature valid + source bit-identical + acceptance re-passes (re-runs `agent-spec lifecycle`)
- [ ] ⌨️ Inspect the committed product repo (`demo/.work/web` or the project repo): `git log --oneline main` ✅ merge commit + `chore: record sign-off by …`; `specs/<id>.spec.md`, `specs/<id>.requirements.md`, `provenance/*.att.json` present

---

## 8. Cleanup / reset between runs

```bash
AC=~/Work/Projects/consult/agent-chat
API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')
# delete OpenFab agent-chat tasks
for t in $(curl -s http://127.0.0.1:8090/api/tasks | python3 -c "import sys,json;[print(x['id']) for x in json.load(sys.stdin) if 'openfab' in (x.get('labels') or [])]"); do
  curl -s -XDELETE "http://127.0.0.1:8090/api/tasks/$t" -H "Authorization: Bearer $API_TOKEN" >/dev/null; done
# remove a self-host worktree if created
git -C ~/Work/Projects/FW/openfab worktree remove --force demo/.work/projects/selfdev 2>/dev/null
git -C ~/Work/Projects/FW/openfab branch -D openfab/selfdev 2>/dev/null
# fresh default workspace
rm -rf ~/Work/Projects/FW/openfab/demo/.work/web && mkdir -p ~/Work/Projects/FW/openfab/demo/.work/web
# stop agents
tmux kill-session -t wf_coordinator 2>/dev/null; tmux kill-session -t wf_implementer 2>/dev/null
```

---

## Pass criteria (summary)

- §1 local trust chain green (C1–C12, 2-of-2, merged) ✅
- §2 agent-chat **native** implementer round-trip merged ✅
- §3 console board/stages/docs/agents/identity/multi-project work ✅
- §4 worktree self-host isolated + GitHub PR created (with token) ✅
- §5 **Robrix**: requirements chat → ingested doc on dashboard → `approve` in room → N-of-M merge ✅
- §6 security regressions all blocked (CSRF, traversal, XSS, spoofing, integrity) ✅
- §7 reproduce verifies signature + source + acceptance ✅
