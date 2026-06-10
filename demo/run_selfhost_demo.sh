#!/usr/bin/env bash
# OpenFab self-hosting demo (PRD §6) — OpenFab develops OpenFab, fully logged & gated.
#
# It clones OpenFab's own source into a workspace, then points `openfab` at that clone to
# implement a change to OpenFab itself. The change is verified with the project's OWN
# checks (cargo build + cargo test) in the sandbox, signed with provenance, and the merge
# is gated on N-of-M human sign-off — the same trust model that guards any contribution.
# Every action lands in git with provenance trailers.
#
# The LLM (base) writes the change to OpenFab; cargo verifies it in the sandbox.
# Usage:  demo/run_selfhost_demo.sh [claude|agentscope|…]   (default: claude)
set -euo pipefail
BASE="${1:-claude}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

OF="$ROOT/target/release/openfab"
POL="$ROOT/policy/trust.json"
SPEC="$ROOT/specs/openfab-selfdev.spec.yaml"
SELF="$ROOT/demo/.work/selfhost"

banner() { printf '\n\033[1;35m========== %s ==========\033[0m\n' "$*"; }

banner "0. BUILD OpenFab + clone its own source into a workspace"
cargo build --release --quiet
rm -rf "$SELF"; mkdir -p "$SELF"
rsync -a --exclude target --exclude .git --exclude 'demo/.work' --exclude '.openfab' ./ "$SELF/"
( cd "$SELF" && git init -q -b main && git add -A && git commit -q -m "import openfab source (self-host seed)" )
echo "  clone: $SELF ($(git -C "$SELF" rev-list --count HEAD) commit, $(find "$SELF/src" -name '*.rs' | wc -l | tr -d ' ') rust files)"

banner "1. OpenFab implements a change to OpenFab (base=$BASE)"
echo "  intent:"; grep -A6 '^intent:' "$SPEC" | sed 's/^/    /'
"$OF" maintainer-add --repo "$SELF" --name alice >/dev/null
"$OF" maintainer-add --repo "$SELF" --name bob >/dev/null
OUT="$("$OF" run --spec "$SPEC" --repo "$SELF" --base "$BASE" --forge local --forge-name openfab-selfhost --policy "$POL" 2>&1)"
echo "$OUT" | grep -E '🤖|🧪|✅|❌|🔏|🛡️|📌|Run id' || true
RID="$(echo "$OUT" | sed -n 's/^Run id: //p' | tail -1)"
echo "  (note: acceptance ran OpenFab's OWN checks — cargo build + cargo test — in the sandbox)"

banner "2. Gate on N-of-M human sign-off (same gate as any contribution)"
"$OF" signoff --repo "$SELF" --run "$RID" --as alice --policy "$POL" | grep -E '✍|🛡️|⛔' || true
"$OF" signoff --repo "$SELF" --run "$RID" --as bob   --policy "$POL" | grep -E '✍|🛡️|⛔' || true

banner "3. Verify + the auditable git history (self-development, fully logged)"
"$OF" verify --repo "$SELF" --run "$RID"
echo; echo "  what OpenFab added to itself (the feat commit):"
FEAT="$(git -C "$SELF" log --all --grep 'feat(openfab-selfdev)' --format=%H -n1)"
git -C "$SELF" show --stat --format='' "$FEAT" | grep -E '\|' | sed 's/^/    /' || true
echo "  provenance trailers on that commit:"
git -C "$SELF" show -s --format='%(trailers:only,unfold)' "$FEAT" | sed 's/^/    /'
echo; echo "  git graph:"; git -C "$SELF" --no-pager log --oneline --graph -n 6 | sed 's/^/    /'

banner "DONE — OpenFab built OpenFab, verified with cargo, signed, and gated. base=$BASE"
echo "Browse the self-development run + its provenance/audit in the UI:"
echo "  $OF serve --repo $SELF"
