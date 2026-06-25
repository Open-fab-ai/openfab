#!/usr/bin/env bash
# Phase 1 helper — preflight the agent-chat backend and start the OpenFab↔agent-chat Bridge.
#
# This does NOT start Palpo or the agent-chat backend/agents — those live in robrix2's
# roadmap/agentchat-demo (start-demo.sh). Bring those up first, then run this to launch the
# Bridge and print the OpenFab env to use.
#
# Env (override as needed):
#   AGENTCHAT_URL        (default http://127.0.0.1:8090)
#   AGENTCHAT_API_TOKEN  operator Bearer token (required for live use)
#   BRIDGE_PORT          (default 8077)
#   BRIDGE_ASSIGNEE      (default wf_implementer)
#   OPENFAB_AGENTCHAT_ROOM  the Matrix room id (e.g. !demoboard:localhost)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AC="${AGENTCHAT_URL:-http://127.0.0.1:8090}"
PORT="${BRIDGE_PORT:-8077}"

note() { printf '\033[0;33m» %s\033[0m\n' "$*"; }
fail() { printf '\033[0;31m✗ %s\033[0m\n' "$*"; exit 1; }

command -v node >/dev/null || fail "node not found (need ≥18)"

note "checking agent-chat backend at $AC ..."
if curl -sS -m 3 "$AC/api/agents?view=names" >/dev/null 2>&1; then
  note "agent-chat backend reachable ✓"
else
  note "WARNING: agent-chat backend not reachable at $AC"
  note "  start it first (robrix2/roadmap/agentchat-demo/start-demo.sh), then re-run."
fi

[ -n "${AGENTCHAT_API_TOKEN:-}" ] || note "WARNING: AGENTCHAT_API_TOKEN is empty (operator calls may 401)"

cat <<EOF

Once the Bridge is up, point OpenFab at it:

  export OPENFAB_AGENTCHAT_URL=http://127.0.0.1:$PORT
  export OPENFAB_AGENTCHAT_ROOM='${OPENFAB_AGENTCHAT_ROOM:-!demoboard:localhost}'
  openfab build "<intent>" --base agent-chat --forge local --gate team --policy policy/trust.json

EOF

note "starting Bridge on :$PORT → $AC"
exec node "$ROOT/bridge/openfab-agentchat-bridge.mjs"
