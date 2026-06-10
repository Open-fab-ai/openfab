#!/usr/bin/env bash
# OpenFab end-to-end demo — natural language in, trustworthy software out.
#
# Showcases every value proposition in one run:
#   • NL → software            a spec's natural-language intent becomes a working app
#   • trustworthy              every artifact carries a signed in-toto/SLSA attestation
#   • AI/Human attribution     the openfab/generation predicate records who authored what
#   • reproducible (verify)    re-runnable acceptance + signature verification (openfab verify)
#   • human-in-the-loop        the trust gate BLOCKS merge until N-of-M maintainer sign-off
#   • neutral / cross-forge    identical flow on two independent forges; portable provenance
#   • swappable base           --base claude (LLM) or a framework base (agentscope, …)
#   • spec cycle / iteration   human feedback bumps the spec (v→v+1) and re-runs the cycle
#   • decision memory          a human-readable timeline/audit trail per run
#
# Every artifact (spec + code) comes from the LLM base. Uses a saved spec file so the
# SAME contract runs across two forges (cross-forge proof) — for NL → spec authoring,
# see `openfab build "<intent>"` or the web UI.
#
# Usage:  demo/run_demo.sh [claude|agentscope|hiclaw|agent-chat|openhands]   (default: claude)
set -euo pipefail

BASE="${1:-claude}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

OF="$ROOT/target/release/openfab"
POLICY="$ROOT/policy/trust.json"
SPEC="$ROOT/specs/demo-temp-converter.spec.yaml"
WORK="$ROOT/demo/.work"
F1="$WORK/github-local"     # forge #1 (stands in for GitHub)
F2="$WORK/forgejo-local"    # forge #2 (stands in for Forgejo) — proves cross-forge

banner() { printf '\n\033[1;36m========== %s ==========\033[0m\n' "$*"; }
note()   { printf '\033[0;33m» %s\033[0m\n' "$*"; }

# Capture a command's stdout, echo it, and extract the "Run id:" it prints.
run_capture() { local out; out="$("$@" 2>&1)"; echo "$out"; echo "$out" | sed -n 's/^Run id: //p' | tail -1; }

banner "0. BUILD OpenFab (Rust, single static binary)"
cargo build --release --quiet
note "binary: $OF"
rm -rf "$WORK"; mkdir -p "$WORK"

banner "1. NL → TRUSTWORTHY SOFTWARE  (base=$BASE, forge=github-local)"
note "Pre-approving maintainers (the human sign-off allowlist — N-of-M trust model)"
"$OF" maintainer-add --repo "$F1" --name alice
"$OF" maintainer-add --repo "$F1" --name bob
note "Running the spec cycle: the intent below becomes a signed, PR'd app"
grep -A6 '^intent:' "$SPEC" | sed 's/^/    /'
RUN1="$(run_capture "$OF" run --spec "$SPEC" --repo "$F1" --base "$BASE" --forge local --forge-name github-local --policy "$POLICY" | tail -1)"
note "run id = $RUN1  (gate is BLOCKED above — no merge without human sign-off)"

banner "2. HUMAN-IN-THE-LOOP  (N-of-M = 2-of-2 sign-off opens the gate)"
note "alice signs off (1-of-2 — still blocked):"
"$OF" signoff --repo "$F1" --run "$RUN1" --as alice --policy "$POLICY"
note "bob signs off (2-of-2 — gate opens, PR merges):"
"$OF" signoff --repo "$F1" --run "$RUN1" --as bob --policy "$POLICY"

banner "3. VERIFY  (anyone can re-verify from the committed provenance alone)"
"$OF" verify --repo "$F1" --run "$RUN1"

banner "4. CROSS-FORGE / NEUTRAL  (identical flow on a second, independent forge)"
"$OF" maintainer-add --repo "$F2" --name alice
"$OF" maintainer-add --repo "$F2" --name bob
RUN2="$(run_capture "$OF" run --spec "$SPEC" --repo "$F2" --base "$BASE" --forge local --forge-name forgejo-local --policy "$POLICY" | tail -1)"
"$OF" signoff --repo "$F2" --run "$RUN2" --as alice --policy "$POLICY" >/dev/null
"$OF" signoff --repo "$F2" --run "$RUN2" --as bob --policy "$POLICY"  >/dev/null
"$OF" verify --repo "$F2" --run "$RUN2"
note "Same Core, two forges, portable in-repo provenance — verified on both."

banner "5. SPEC CYCLE / ITERATION  (human feedback → spec v→v+1 → re-dispatch)"
note 'feedback: "also support Kelvin" adds a new acceptance check and re-runs the cycle'
RUN3="$(run_capture "$OF" feedback --repo "$F1" --run "$RUN1" \
  --note "also support Kelvin conversion (c2k)" \
  --add-check "id=a4-c2k,check=python3 app/convert.py 0 c2k | grep -q 273" \
  --base "$BASE" --policy "$POLICY" | tail -1)"
"$OF" signoff --repo "$F1" --run "$RUN3" --as alice --policy "$POLICY" >/dev/null
"$OF" signoff --repo "$F1" --run "$RUN3" --as bob --policy "$POLICY"  >/dev/null
"$OF" verify --repo "$F1" --run "$RUN3"

banner "6. REPUTATION  (projected purely from the signed attestations)"
"$OF" reputation --repo "$F1"
echo
"$OF" list --repo "$F1"

banner "7. THE DURABLE ASSETS  (process + decision memory + signed provenance)"
note "Signed in-toto/SLSA attestation with the openfab/generation predicate:"
sed -n '1,60p' "$F1/$(sed -n 's/.*"attestation_repo_path": "\(.*\)",/\1/p' "$F1/.openfab/runs/$RUN1/run.json" | head -1)" 2>/dev/null \
  || cat "$F1"/provenance/*.att.json | sed -n '1,60p'
echo
note "Human-readable decision log / audit trail (the 'Matrix room timeline' stand-in):"
cat "$F1/.openfab/runs/$RUN1/timeline.md"
echo
note "Git history on github-local (provenance committed in-repo, merge commit from the gate):"
git -C "$F1" --no-pager log --oneline --graph -n 8

banner "DONE — OpenFab built the app, proved it, signed it, and gated it. base=$BASE"
