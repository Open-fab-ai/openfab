#!/usr/bin/env bash
# OpenFab × agent-spec demo — the spec is an agent-spec Task Contract (.spec.md), and
# verification is delegated to `agent-spec lifecycle`. OpenFab still owns signing,
# provenance (in-toto/SLSA + openfab/generation), the conformance gate (incl. the new
# C12 agent-spec-scenarios gate), N-of-M human sign-off, and reproduction.
#
# Pipeline (Phase 0 — local, no Matrix):
#   NL intent → agent-spec drafts .spec.md → `agent-spec lint` gate → implement (base) →
#   `agent-spec lifecycle` verify → sign → conformance/N-of-M gate → commit (.spec.md +
#   provenance into repo) → reproduce (re-runs lifecycle).
#
# PREREQUISITES (manual — see the final checklist):
#   • agent-spec installed:   cargo install agent-spec   (or set OPENFAB_AGENT_SPEC_BIN)
#   • a working LLM backend for drafting the .spec.md AND for the implementer base:
#       - claude CLI authenticated (default), OR
#       - OPENFAB_LLM=dashscope with DASHSCOPE_API_KEY set
#
# Usage:  demo/run_agentspec_demo.sh [base]   (default base: claude)
set -euo pipefail

BASE="${1:-claude}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

OF="$ROOT/target/release/openfab"
POLICY="$ROOT/policy/trust.json"
WORK="$ROOT/demo/.work-agentspec"
REPO="$WORK/forge-local"

# Author specs via agent-spec; keep the .spec.md contracts under demo work dir.
export OPENFAB_SPEC="agent-spec"
export OPENFAB_SPEC_DIR="$WORK/specs"
# Quality gate threshold for `agent-spec lint` (default 0.7).
export OPENFAB_SPEC_MIN_SCORE="${OPENFAB_SPEC_MIN_SCORE:-0.7}"

banner() { printf '\n\033[1;36m========== %s ==========\033[0m\n' "$*"; }
note()   { printf '\033[0;33m» %s\033[0m\n' "$*"; }
run_capture() { local out; out="$("$@" 2>&1)"; echo "$out"; echo "$out" | sed -n 's/^Run id: //p' | tail -1; }

banner "0. PREFLIGHT"
command -v agent-spec >/dev/null || { echo "agent-spec not found — cargo install agent-spec"; exit 1; }
note "agent-spec: $(agent-spec --version)"
note "OPENFAB_SPEC=$OPENFAB_SPEC  OPENFAB_SPEC_DIR=$OPENFAB_SPEC_DIR  base=$BASE"

banner "1. BUILD OpenFab"
cargo build --release --quiet
rm -rf "$WORK"; mkdir -p "$WORK" "$OPENFAB_SPEC_DIR"

banner "2. MAINTAINERS (N-of-M trust gate)"
"$OF" maintainer-add --repo "$REPO" --name alice
"$OF" maintainer-add --repo "$REPO" --name bob

banner "3. BUILD FROM NL INTENT (agent-spec authors the .spec.md, then OpenFab builds)"
note "agent-spec drafts a Task Contract, lint-gates it, OpenFab maps it and dispatches the base"
RUN="$(run_capture "$OF" build \
  "Build a small command-line tool that adds two integers passed as arguments and prints their sum." \
  --repo "$REPO" --base "$BASE" --forge local --forge-name forge-local --gate team --policy "$POLICY" | tail -1)"
note "run id: $RUN"

banner "4. THE CONTRACT (.spec.md) authored by agent-spec"
ls -1 "$OPENFAB_SPEC_DIR"/*.spec.md && echo "---" && cat "$OPENFAB_SPEC_DIR"/*.spec.md

banner "5. N-of-M HUMAN SIGN-OFF (gate blocks merge until met)"
"$OF" signoff --repo "$REPO" --run "$RUN" --as alice --policy "$POLICY"
"$OF" signoff --repo "$REPO" --run "$RUN" --as bob --policy "$POLICY"

banner "6. VERIFY (conformance incl. C12 agent-spec-scenarios gate)"
"$OF" verify --repo "$REPO" --run "$RUN"

banner "7. AUDIT — the committed contract + provenance travel with the code"
note "repo holds: specs/<id>.spec.md (contract), provenance/*.att.json (signed), *.sbom.json"
git -C "$REPO" log --oneline -1 || true
ls -1 "$REPO/specs" "$REPO/provenance" 2>/dev/null || true

banner "DONE — NL intent → agent-spec contract → trustworthy, reproducible software"
