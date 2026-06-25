# OpenFab — Full-Stack Deployment Runbook (AI-executable)

> Hand this file to an AI assistant on a fresh machine and say "deploy OpenFab + the Robrix
> stack following this runbook." It is the **session-hardened** guide: every pitfall below was
> hit and solved during real end-to-end testing. Read [DEPLOYMENT.md](DEPLOYMENT.md) for the
> lighter tiered version; this one is the complete operational truth.

Paths shown as `<...>` are per-machine — the AI must fill them in. Examples use this author's
layout; **do not copy paths verbatim.**

---

## 0. What you're deploying — FOUR repos, not one

OpenFab is one Rust binary + a Node bridge sidecar, but the full Robrix demo needs **four
separate git repos** (only the first is "this" repo):

| # | Component | Repo / where | Provides | Ports |
|---|-----------|--------------|----------|-------|
| 1 | **OpenFab** | this repo | the fab binary (`openfab serve`) + the Bridge sidecar (`bridge/openfab-agentchat-bridge.mjs`) | serve **8787**, bridge **8077** |
| 2 | **agent-chat** | separate repo, e.g. `<...>/consult/agent-chat` | the agent team backend + monitor + **`bridge-matrix.js`** (Matrix⇄agent-chat sync) | backend **8090**, monitor **8084** |
| 3 | **Palpo** | `palpo-and-octos-deploy/` **inside the robrix2 repo** (docker compose) | the Matrix homeserver | **8128** |
| 4 | **robrix2** | separate repo, e.g. `<...>/robius/robrix2` | the human GUI (Makepad) | desktop app |

Process inventory when fully up (8 long-running things):
- Palpo (docker), agent-chat backend, agent-chat monitor, **bridge-matrix** (node),
  **OpenFab↔agent-chat bridge** (node :8077), **openfab serve** (:8787),
- the **agent sessions** in tmux: `wf_coordinator`, `wf_implementer`, `wf_reviewer`,
  `wf_final_reviewer` (live Claude/Codex sessions — they burn LLM quota).

You can stop at any tier:
- **Tier 1** = OpenFab only (build via local `claude`, dashboard/console, gate). No Matrix.
- **Tier 2** = + agent-chat + Palpo + both bridges + agents (the Matrix team).
- **Tier 3** = + robrix2 GUI (chat requirements / approve from a real client).

---

## 1. Prerequisites

- **Rust** stable, edition 2021, rustc ≥ 1.74 (`rust-toolchain.toml` pins it). `cargo install agent-spec` (this build targets **0.3.0**).
- **git**, **curl**, **python3** (smoke scripts), **tmux**.
- **Node ≥ 18** (both bridges + agent-chat).
- **Docker / OrbStack** (Palpo).
- An **LLM** for the agents + authoring: `claude` CLI logged in (subscription/OAuth), and/or `DASHSCOPE_API_KEY`. The agents may also use Codex (`wf_final_reviewer`).
- Developed on **macOS (darwin)**. Linux: adapt tmux/docker; Windows: WSL2.

---

## 2. Bring-up order (do NOT reorder — each depends on the previous)

### 2.1 OpenFab build + Tier-1 sanity
```bash
cd <openfab>
cargo install agent-spec
cargo build --release           # → target/release/openfab
cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check
```

### 2.2 Palpo (Matrix server) — docker
```bash
cd <robrix2>/palpo-and-octos-deploy
docker compose up -d
# verify
curl -s -o /dev/null -w "palpo %{http_code}\n" http://127.0.0.1:8128/_matrix/client/versions   # → 200
```
Palpo's `palpo.toml` has `allow_registration = true`. **The registration token lives in
agent-chat's `.env` as `MATRIX_REG_TOKEN`** (see pitfall #6) — needed only to register *new*
Matrix accounts.

### 2.3 agent-chat backend + monitor
```bash
cd <agent-chat>
./install-full.sh          # first time
# fill <agent-chat>/.env: MATRIX_HOMESERVER=http://127.0.0.1:8128, API_TOKEN, MATRIX_REG_TOKEN,
#   MATRIX_AGENT_PASSWORD_SECRET, MATRIX_BOT_USERNAME (e.g. agent-bridge), MATRIX_BOT_PASSWORD, ...
# start the backend (:8090) + monitor (:8084) per agent-chat's own start script
curl -s "http://127.0.0.1:8090/api/agents?view=names"     # backend up
```

### 2.4 bridge-matrix (Matrix ⇄ agent-chat sync) — **the inbound lifeline**
This is what carries a human's room message (e.g. `approve <run>`) INTO agent-chat. If it's
down, nothing the human types in Robrix reaches the system (pitfall #5).
```bash
cd <agent-chat>
# robust .env load (zsh `source` silently drops some vars) + the skip patch (pitfall #6):
python3 - <<'PY'
import subprocess, os
env=dict(os.environ)
for line in open('.env'):
    s=line.strip()
    if not s or s.startswith('#') or '=' not in s: continue
    k,v=s.split('=',1); v=v.strip().strip('"').strip("'"); env[k.strip()]=v
env.setdefault('MATRIX_BRIDGE_SKIP_AGENTS','openfab-bridge')   # don't puppet the service relay
subprocess.Popen(['node','bridge-matrix.js'], env=env,
                 stdout=open('/tmp/bridge-matrix.log','w'), stderr=subprocess.STDOUT)
print("bridge-matrix launched")
PY
sleep 6; grep -iE "syncing|skipping|error|failed" /tmp/bridge-matrix.log
```

### 2.5 OpenFab↔agent-chat Bridge (this repo) — :8077
```bash
cd <openfab>
AC=<agent-chat>
API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')
MX_HS=$(grep -E '^MATRIX_HOMESERVER=' "$AC/.env" | cut -d= -f2- | tr -d '"')
MX_USER=$(grep -E '^MATRIX_BOT_USERNAME=' "$AC/.env" | cut -d= -f2- | tr -d '"')
MX_PASS=$(grep -E '^MATRIX_BOT_PASSWORD=' "$AC/.env" | cut -d= -f2- | tr -d '"')
AGENTCHAT_URL=http://127.0.0.1:8090 AGENTCHAT_API_TOKEN="$API_TOKEN" AGENTCHAT_DIR="$AC" \
  BRIDGE_ASSIGNEE=wf_implementer BRIDGE_REVIEWER=wf_reviewer BRIDGE_PORT=8077 \
  OPENFAB_URL=http://127.0.0.1:8787 BRIDGE_COORDINATOR_WS=<coordinator-workspace> \
  MATRIX_HOMESERVER="$MX_HS" MATRIX_BOT_USERNAME="$MX_USER" MATRIX_BOT_PASSWORD="$MX_PASS" \
  nohup node bridge/openfab-agentchat-bridge.mjs > /tmp/of_bridge.log 2>&1 &
curl -s http://127.0.0.1:8077/healthz      # → {"ok":true}
```
- `AGENTCHAT_DIR` is **required** (no hardcoded default) — the bridge reads `data/messages.json` there.
- `MATRIX_*` let the bridge post notifications **straight into the Matrix room via the bot** (pitfall #4).
- `BRIDGE_COORDINATOR_WS` = the coordinator agent's workspace; the bridge auto-harvests specs it writes there into the bound project.

### 2.6 OpenFab serve — :8787
```bash
cd <openfab>
env -u ANTHROPIC_API_KEY \                            # pitfall #1
  OPENFAB_SPEC=agent-spec OPENFAB_SPEC_DIR="$PWD/work/specs" OPENFAB_PROJECTS_DIR="$PWD/work/projects" \
  OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 \        # the BRIDGE, not agent-chat directly
  OPENFAB_AGENTCHAT_ROOM=<bound-project-or-room> OPENFAB_AGENTCHAT_MONITOR=http://127.0.0.1:8084 \
  OPENFAB_AGENTCHAT_WORKSPACE=shared \                 # pitfall #8 — refactor real repos in place
  nohup ./target/release/openfab serve --repo "$PWD/work/web" --port 8787 --policy policy/trust.json > /tmp/of_serve.log 2>&1 &
curl -s -o /dev/null -w "console %{http_code}\n" http://127.0.0.1:8787/console   # → 200
```

### 2.7 Launch the agent team (tmux Claude/Codex sessions)
```bash
# link the OpenFab skills so the agents adopt the OpenFab roles:
for s in openfab-coordinator openfab-implementer openfab-reviewer; do
  ln -sfn "<openfab>/bridge/skills/$s" "$HOME/.claude/skills/$s"
done
ln -sfn "<openfab>/bridge/skills/openfab-reviewer" "$HOME/.codex/skills/openfab-reviewer"  # codex final-reviewer
# launch — --allow-shared-workspace is REQUIRED so workspace-mode agents can edit the repo path:
cd <agent-chat>; set -a; source .env; set +a
for a in wf_coordinator wf_implementer wf_reviewer; do
  env -u TMUX -u ANTHROPIC_API_KEY ./bin/agentchat up-v1 $a claude --project /tmp/of-$a --project-mode symlink --allow-shared-workspace --fresh
done
env -u TMUX -u ANTHROPIC_API_KEY ./bin/agentchat up-v1 wf_final_reviewer codex --project /tmp/of-final --project-mode symlink --allow-shared-workspace --fresh
curl -s http://127.0.0.1:8090/api/agents | python3 -c "import sys,json;d=json.load(sys.stdin);print([a['name'] for a in (d if isinstance(d,list) else d['agents']) if a.get('online')])"
```

### 2.8 Robrix GUI (Tier 3)
```bash
cd <robrix2>
cargo run --features agent_chat        # the agent_chat feature gates the workflow slash-commands
```
Then in Robrix: log into Palpo (`http://127.0.0.1:8128`), Settings → enable **"agent-chat support"**.

### 2.9 Wire the project/room/identity (per-deployment data)
```bash
OF=http://127.0.0.1:8787 ; H="-H Origin:$OF -H Content-Type:application/json"
# a) register the project (worktree:true if pointing at an existing repo to self-host):
curl -s $H -XPOST $OF/api/projects -d '{"name":"openfab","repo":"<repo-path>","worktree":true}'
# b) maintainers + identity map for that project (only a mapped Matrix user can sign off):
curl -s $H -XPOST "$OF/api/maintainers?project=openfab" -d '{"name":"alice"}'
curl -s $H -XPOST "$OF/api/identity?project=openfab" -d '{"mxid":"@you:127.0.0.1:8128","maintainer":"alice"}'
# c) bind the room → project: in the Robrix room (must contain the bot + a *_coordinator), send the
#    PLAIN message:  bind openfab     (the bridge relays it; no curl, no need to know the room id)
```

---

## 3. The pitfalls (every one of these bit us — check them FIRST when something fails)

1. **Stale `ANTHROPIC_API_KEY` shadows the `claude` OAuth login → 401.** A key exported in the
   shell (e.g. in `.zshrc`) overrides the CLI's subscription login. **Always launch OpenFab and
   the agents with `env -u ANTHROPIC_API_KEY`** (or remove the export).

2. **The agent-chat message body limit (~100KB) — don't ship code over the bridge.** Mounting a
   repo's files into the dispatch message 413s. Use **shared-workspace mode**
   (`OPENFAB_AGENTCHAT_WORKSPACE=shared`): OpenFab sends the repo *path*; the agent reads the
   whole repo for context and edits in place; OpenFab reads the changed bytes off disk to sign.
   (Boundary-scoped file mount is only a remote-agent fallback.)

3. **Refactoring an existing repo as "greenfield" overwrites it.** Without workspace mode the
   agent can't see the real code, so it emits a fresh project whose `Cargo.toml` drops the real
   deps → won't compile → gate (correctly) blocks. Always self-host with a **git worktree**
   (`worktree:true`) so a bad build is contained on a branch, never the live tree.

4. **OpenFab→Robrix notifications must go via the Matrix bot, not the agent-chat group.** The
   service agent `openfab-bridge` has no Matrix puppet, so the agent-chat→Matrix path can't
   relay its messages. The bridge instead logs in as the **bot** (`MATRIX_BOT_*`, a real room
   member) and posts `m.room.message` straight to the bound room. → set `MATRIX_*` on the bridge.

5. **If `bridge-matrix` is down, NOTHING a human types in Robrix reaches OpenFab.** Inbound
   (room → agent-chat) is entirely on bridge-matrix. Symptom: you `approve <run>` and the
   dashboard stays blocked, with no `approve` message appearing in agent-chat's
   `data/messages.json`. Fix: (re)start bridge-matrix (§2.4).

6. **bridge-matrix crashes at startup needing `MATRIX_REG_TOKEN`** — because it tries to give
   *every* agent-chat agent a Matrix puppet, and the service agent `openfab-bridge` has none
   (and the reg token in `.env` is often empty after the initial setup). Fix (already patched in
   this stack): bridge-matrix **skips** `MATRIX_BRIDGE_SKIP_AGENTS` (default `openfab-bridge`)
   and treats a single agent's account failure as non-fatal. The pre-existing `wf_*` agents log
   in fine from their cached tokens in `data/matrix/bridge-state.json`. **The exact one-time
   edit to apply to agent-chat is in [agent-chat-patch.md](agent-chat-patch.md).**

7. **A manually-created Robrix room won't route** — it's not a registered agent-chat group and
   has no bot. Use a room that contains **`@agent-bridge` (the bot) + the `@ac_wf_*` agents**;
   that room maps to an agent-chat group (e.g. `octos-public`) which bridge-matrix forwards.

8. **Approvals: identity is enforced only on the relay path; the CLI is not.** Send a **plain**
   `approve <run>` (or even `@coordinator approve <run>` — the bridge now strips the mention) in
   the bound room: the bridge maps your Matrix id → your maintainer and signs **as you**.
   **Agents must never run `openfab signoff --as <name>`** — that CLI has *no* identity check
   and forges a human sign-off. The coordinator skill forbids it; see the open gap in §5.

9. **Project scoping is everywhere.** Runs/maintainers/identity/board are per-project. The
   dashboard remembers the selected project across refresh (localStorage); the bridge scopes
   sign-off/bind to the room's bound project. If data "disappears", you're on the wrong project.

10. **Link the skills before launching agents**, into `~/.claude/skills` (and `~/.codex/skills`
    for the Codex final-reviewer): `openfab-coordinator`, `openfab-implementer`,
    `openfab-reviewer`. After editing a skill, **restart the agent** to reload it.

---

## 4. Smoke test the whole loop

```bash
# Tier 1/2
curl -s http://127.0.0.1:8128/_matrix/client/versions >/dev/null && echo palpo-ok
curl -s http://127.0.0.1:8090/api/agents >/dev/null && echo agentchat-ok
ps aux | grep -q "[b]ridge-matrix.js" && echo bridge-matrix-ok
curl -s http://127.0.0.1:8077/healthz; echo
curl -s -o /dev/null -w "openfab %{http_code}\n" http://127.0.0.1:8787/console
curl -s http://127.0.0.1:8787/api/bases | python3 -c "import sys,json;print('agent-chat:',[b['runtime'] for b in json.load(sys.stdin) if b['id']=='agent-chat'][0])"
```
**Full e2e** (Robrix → OpenFab → gate → merge → room receipt): follow
[E2E-TESTPLAN.md](E2E-TESTPLAN.md). The happy path: in the room, `bind openfab` → `/create-issue`
or describe a small change → coordinator drafts spec → dashboard (openfab project) Build (base
`agent-chat`, gate `solo`) → run reaches **awaiting sign-off** and posts a 🔔 to the room →
reply `approve <run>` → gate opens, merges, room gets `✅ gate opened` → the LIVE panel flips to
merged **without a manual refresh**.

---

## 5. Sign-off authentication (the gate's teeth)

Name-based sign-off (CLI `--as`, API `{as}`) is **gated by a per-maintainer credential** so an
agent can't forge a human sign-off:

- **Matrix relay (recommended, no credential):** in the bound room send `approve <run>` — the
  bridge maps your verified Matrix id → your maintainer and signs as you. This is the path to use.
- **CLI / UI with a credential:** set one once, then present it.
  ```bash
  openfab maintainer-cred --repo <repo> --name alice --token "<passphrase>"   # stores only its hash
  openfab signoff --repo <repo> --run <id> --as alice --cred "<passphrase>"   # or OPENFAB_SIGNOFF_TOKEN
  ```
  The dashboard/console sign-off buttons now prompt for this passphrase.
- **Trusted single-operator override:** `OPENFAB_ALLOW_UNVERIFIED_SIGNOFF=1` re-allows
  credential-less name sign-off. **Set it only on the serve/CLI of a machine where no autonomous
  agent can reach the API or run the CLI** — otherwise the forge hole reopens. Default is OFF
  (name sign-off without a credential is refused with guidance).

Agents are independently forbidden (coordinator skill) from running `openfab signoff`.

---

## 6. Secrets & portability checklist (regenerate per machine — never copy)

- agent-chat `.env`: `API_TOKEN`, `MATRIX_REG_TOKEN`, `MATRIX_AGENT_PASSWORD_SECRET`, bot
  username/password, homeserver URL.
- Palpo: its own config + the registration token must match agent-chat's `MATRIX_REG_TOKEN`.
- OpenFab: `OPENFAB_GITHUB_TOKEN`+`OPENFAB_GITHUB_REPO` for the GitHub forge (optional); LLM auth.
- Signing seeds in each repo's `.openfab/identity/` and `.openfab/maintainers/` are gitignored —
  keep secret; they are what make sign-offs cryptographically yours.
- Ports all bind `127.0.0.1`. Remote access → SSH tunnel; **never expose** (the APIs have a CSRF
  Origin guard but no authentication — localhost dev tools).
