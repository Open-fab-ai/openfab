#!/usr/bin/env bash
# Launch the full OpenFab multi-base / multi-forge demo on Ollama Cloud models.
#   - agent-chat  native  (Ollama Cloud qwen3-coder:480b)   :8741
#   - agentscope  native  (Ollama Cloud gpt-oss:120b, ReAct):8731
#   - claude      native  (local claude CLI)
#   - forges: gitea + forgejo (Docker) + github (csheargm) live
#   - OpenFab web UI                                          :8787  → http://127.0.0.1:8787
#
# Secrets live OUTSIDE the repo: ~/.config/openfab/cloud.env (Ollama key) and
# forges/forges.env (forge tokens, gitignored). Edit those, not this script.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

set -a
source "$HOME/.config/openfab/cloud.env"          # OLLAMA cloud key + OPENFAB_OLLAMA_* + OPENFAB_LLM=ollama
source "$ROOT/forges/forges.env"                  # OPENFAB_GITEA/FORGEJO/GITHUB_*
export OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8741/dispatch
export OPENFAB_AGENTSCOPE_URL=http://127.0.0.1:8731/dispatch
set +a

VENV="$ROOT/../agentscope/.venv/bin"
kill_port(){ lsof -ti tcp:"$1" 2>/dev/null | xargs kill -9 2>/dev/null || true; }

echo "== docker forges (gitea:3000 / forgejo:3001) =="
docker start openfab-gitea openfab-forgejo >/dev/null 2>&1 || echo "  (run forges/setup_forges.sh if these don't exist)"

echo "== agent-chat native adapter :8741 (qwen3-coder:480b) =="
kill_port 8741
AGENTCHAT_LLM_ENDPOINT="$AGENTCHAT_LLM_ENDPOINT" AGENTCHAT_LLM_KEY="$AGENTCHAT_LLM_KEY" AGENTCHAT_LLM_MODEL="$AGENTCHAT_LLM_MODEL" \
  nohup node "$ROOT/integrations/agent-chat/server.mjs" >/tmp/agentchat-adapter.log 2>&1 &

echo "== agentscope native adapter :8731 (gpt-oss:120b ReAct) =="
kill_port 8731
OLLAMA_HOST="$OLLAMA_HOST" OLLAMA_API_KEY="$OLLAMA_API_KEY" OPENFAB_AGENTSCOPE_MODEL="$OPENFAB_AGENTSCOPE_MODEL" \
  nohup "$VENV/python" "$ROOT/integrations/agentscope/server.py" >/tmp/agentscope-adapter.log 2>&1 &

sleep 4
echo "== OpenFab web UI :8787 =="
kill_port 8787
nohup ./target/release/openfab serve --repo "$ROOT/demo/.work/web" --port 8787 \
  --policy "$ROOT/policy/trust.json" >/tmp/openfab-server.log 2>&1 &
sleep 2

echo
echo "  ✅ OpenFab → http://127.0.0.1:8787"
curl -s http://127.0.0.1:8787/api/bases  | python3 -c "import sys,json;print('  bases :',', '.join(f\"{b['id']}({b['runtime']})\" for b in json.load(sys.stdin)))" 2>/dev/null
curl -s http://127.0.0.1:8787/api/forges | python3 -c "import sys,json;print('  forges:',', '.join(f\"{f['id']}({'live' if f['live'] else 'local'})\" for f in json.load(sys.stdin)))" 2>/dev/null
