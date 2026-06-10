#!/usr/bin/env bash
# OpenFab visual demo — builds the single binary and launches the web UI.
#
# Open the printed URL, then drive the whole flow in the browser:
#   1. describe what to build (NL)  →  pick a base (×5) and a forge (×4)
#   2. watch the live workflow stream  →  the trust gate BLOCKS on the human sign-off
#   3. sign off as the maintainers  →  the gate opens and the PR merges
#   4. inspect the software + signed provenance  →  "Reproduce & verify" for sovereign proof
#   5. refine with feedback (spec v→v+1) and re-fabricate
#
# Usage:  demo/run_web_demo.sh [port]
set -euo pipefail
PORT="${1:-8787}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

echo "== building OpenFab (release) =="
cargo build --release --quiet

# Optional: use Qwen/DashScope for the bridged bases instead of the claude CLI.
#   export OPENFAB_LLM=dashscope DASHSCOPE_API_KEY=sk-...
# Optional: connect a base's native runtime (then its badge flips to "native"):
#   export OPENFAB_OPENHANDS_URL=http://localhost:3000/dispatch
# Optional: a live forge (badge flips to "live"):
#   export OPENFAB_GITHUB_REMOTE=git@github.com:you/repo.git   # needs gh auth

WORK="$ROOT/demo/.work/web"
echo "== launching web UI on http://127.0.0.1:$PORT =="
echo "   workspace: $WORK"
exec ./target/release/openfab serve --repo "$WORK" --port "$PORT" --policy "$ROOT/policy/trust.json"
