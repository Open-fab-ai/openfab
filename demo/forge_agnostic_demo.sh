#!/usr/bin/env bash
# OpenFab demo — forge-agnostic + tamper-proof (demo items #3 + #4).
# Builds ONE signed artifact, pushes it to TWO forges, verifies from each clone,
# then tampers one byte to prove tamper-evidence. No docker / no network needed
# (two local bare repos stand in for "github" and "gitea" — git is forge-agnostic
# by construction, so this proves the real claim reliably for a live demo).
#
# For the talk you MAY swap the bare repos for a real GitHub repo (gh is authed)
# and a real Gitea; the verify step is identical. This script is the safe path.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
source "$HOME/.config/openfab/cloud.env" 2>/dev/null || true
export PATH="$HOME/.local/bin:$PATH"
OF="$PWD/target/release/openfab"; POL="$PWD/policy/trust.json"
BASE="${1:-codex}"   # codex = fast; or claude / agent-chat
INTENT="${2:-a python cli palindrome.py that prints yes if arg1 is a palindrome else no}"

TMP=$(mktemp -d)
echo "== 1. Build a signed RELEASE artifact (base=$BASE) =="
"$OF" build "$INTENT" --repo "$TMP" --base "$BASE" --forge local --gate none --policy "$POL" | tail -1
BR=$(git -C "$TMP" branch --list 'openfab/*' | grep -v draft | head -1 | tr -d ' *')
git -C "$TMP" checkout -q "$BR"
ATT=$(basename "$(ls "$TMP"/provenance/*.att.json | head -1)")

echo; echo "== 2. Push the SAME artifact to two forges (github + gitea) =="
GH=/tmp/forge-github.git; GT=/tmp/forge-gitea.git
rm -rf "$GH" "$GT"; git init -q --bare "$GH"; git init -q --bare "$GT"
git -C "$TMP" push -q "$GH" "$BR"; git -C "$TMP" push -q "$GT" "$BR"
echo "   pushed $BR"

echo; echo "== 3. Clone from EACH forge, verify locally (offline, no run-state) =="
CA=/tmp/from-github; CB=/tmp/from-gitea; rm -rf "$CA" "$CB"
git clone -q --branch "$BR" "$GH" "$CA"; git clone -q --branch "$BR" "$GT" "$CB"
echo "-- from GitHub --"; ( cd "$CA" && "$OF" verify-file --att "provenance/$ATT" --policy "$POL" )
echo "-- from Gitea --";  ( cd "$CB" && "$OF" verify-file --att "provenance/$ATT" --policy "$POL" )

echo; echo "== 4. Tamper the GitHub clone, re-verify (must FAIL) =="
echo "# evil" >> "$CA"/app/*.py
( cd "$CA" && "$OF" verify-file --att "provenance/$ATT" --policy "$POL" ) || echo "   ^ correctly rejected the tampered copy."
