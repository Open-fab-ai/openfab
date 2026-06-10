#!/usr/bin/env bash
# init.sh — deterministic OpenFab dev-session setup. Idempotent. Run at the start of every session.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

# Make a rustup-installed toolchain visible even if the interactive shell profile
# hasn't picked it up yet (common right after `rustup` install).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export PATH="$HOME/.cargo/bin:$PATH"

echo "== OpenFab session init =="

# Hard requirements for v0.1 (Core + CLI + demo).
MISS=0
need() { command -v "$1" >/dev/null 2>&1 || { echo "  MISSING: $1 — $2"; MISS=1; }; }
need cargo          "install Rust via https://rustup.rs"
need git            "install git"
if [ "$MISS" = 1 ]; then echo "Install the required tools above, then re-run ./init.sh"; exit 1; fi

# Optional in v0.1 — only needed for the PRODUCTION swaps (see README / ADR 0001).
# v0.1 uses did:key signing, an SPDX-lite SBOM, and a policy-gated host sandbox instead,
# so their absence is a warning, not a failure.
opt() { command -v "$1" >/dev/null 2>&1 || echo "  (optional) $1 not found — $2"; }
opt cosign         "production signing/transparency (Sigstore); v0.1 uses did:key/ed25519"
opt syft           "production SBOM (SPDX/CycloneDX); v0.1 emits SPDX-lite directly"
opt slsa-verifier  "production SLSA verification; v0.1 verifies via openfab verify"
command -v podman >/dev/null 2>&1 || command -v docker >/dev/null 2>&1 \
  || echo "  (optional) no podman/docker — v0.1 falls back to a policy-gated host sandbox"

# ensure rust components
rustup component add rustfmt clippy >/dev/null 2>&1 || true

echo "-- toolchain --"
cargo --version
cosign version 2>/dev/null | head -1 || true
syft version 2>/dev/null | head -1 || true

if [ -f Cargo.toml ]; then
  echo "-- fetch + checks --"
  cargo fetch -q || true
  cargo fmt --all --check || echo "  (formatting drift — run: cargo fmt --all)"
  cargo clippy --all-targets --all-features -q -- -D warnings || echo "  (clippy warnings — fix before done)"
  cargo check -q
  echo "== ready. Next: pick the next PRD §7 build-order step. =="
else
  echo "-- no Cargo.toml yet --"
  echo "== ready. Repo not scaffolded. Next: do PRD §7 build-order step 1 (scaffold the Rust workspace). =="
fi
