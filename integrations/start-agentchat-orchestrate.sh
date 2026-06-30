#!/usr/bin/env bash
# Bring up the agent-chat ORCHESTRATE demo: OpenFab delegates a build to a REAL
# Claude Code agent running under agent-chat (watchable live), then OpenFab
# independently verifies + signs + gates the result.
#
#   - agent-chat backend  :8090   (agent registry, messaging, task graphs)
#   - agent-chat dashboard:8084   <- WATCH THE AGENTS HERE
#   - agent-chat push-relay       (injects messages into agent tmux panes)
#   - OpenFab agent-chat adapter :8741 in mode=orchestrate
#
# Prereqs: tmux + the `claude` CLI (agents are Claude Code), Node 22.
# After this, in OpenFab (http://127.0.0.1:8787) pick base = agent-chat — each
# build launches a real agent in tmux session `openfab-builder`
# (tmux attach -t openfab-builder, or the dashboard) while OpenFab signs.
set -uo pipefail
AC="${AGENTCHAT_REPO:-$HOME/projects/agent-chat}"
OF="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kill_port(){ lsof -ti tcp:"$1" 2>/dev/null | xargs kill -9 2>/dev/null || true; }

command -v tmux >/dev/null || { echo "ERROR: tmux required (brew install tmux)"; exit 1; }
command -v claude >/dev/null || { echo "ERROR: the 'claude' CLI required (agents are Claude Code)"; exit 1; }

echo "== agent-chat backend + dashboard + push-relay (from $AC) =="
( cd "$AC"
  set -a; source ./.env 2>/dev/null
  # team-native mode runs the coder/reviewer team INSIDE agent-chat via its LLM client —
  # give the backend the Ollama config (POST /api/team-build reads OPENFAB_OLLAMA_*).
  source "$HOME/.config/openfab/cloud.env" 2>/dev/null; set +a
  mkdir -p logs
  kill_port 8090; nohup node backend-v2.js  >/tmp/agentchat-backend.log   2>&1 & sleep 2
  kill_port 8084; nohup node server.js       >/tmp/agentchat-dashboard.log 2>&1 & sleep 2
  nohup node push-relay.js >/tmp/agentchat-relay.log 2>&1 &
)
sleep 2
for p in 8090 8084; do echo "  :$p HTTP $(curl -s -o /dev/null -w '%{http_code}' --max-time 4 http://127.0.0.1:$p/ 2>/dev/null)"; done

# MODE: team-native = coder+reviewer team runs INSIDE agent-chat via its LLM client
#       (fast, ~10-15s); team = real tmux CLI agents (slow); orchestrate = one tmux agent.
MODE="${AGENTCHAT_NATIVE_MODE:-team-native}"
echo "== OpenFab agent-chat adapter :8741 in ${MODE} mode =="
source "$HOME/.config/openfab/cloud.env" 2>/dev/null || true
export PATH="$HOME/.local/bin:$PATH"   # so the agents' CLIs (claude/codex) are found
kill_port 8741
AGENTCHAT_NATIVE_MODE="$MODE" AGENTCHAT_REPO="$AC" AGENTCHAT_ORCH_WAIT_MS=180000 \
  nohup node "$OF/integrations/agent-chat/server.mjs" >/tmp/agentchat-adapter.log 2>&1 &
sleep 2; grep -iE 'listening|mode=' /tmp/agentchat-adapter.log | head -1
[ "$MODE" = team ] && echo "  team = claude coder + claude reviewer (critique→revise); ~6-9 min/build"

echo
echo "  ✅ Watch agents:  http://127.0.0.1:8084   (or: tmux attach -t openfab-builder)"
echo "  ✅ OpenFab:       http://127.0.0.1:8787   → pick base = agent-chat, then Fabricate"
echo "  (orchestrate is slow ~1-2 min/build — it's a real agent; set AGENTCHAT_NATIVE_MODE=llm for the fast path)"
