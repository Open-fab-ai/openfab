# OpenFab demo walkthrough

## Run it

```bash
./init.sh                  # verify toolchain (cargo required; cosign/syft/podman optional in v0.1)
demo/run_demo.sh           # base = claude (the LLM writes the app from the NL intent)
demo/run_demo.sh agentscope  # or any framework base — same Core pipeline, bridged via the LLM
```

`run_demo.sh` is idempotent: it rebuilds the release binary and recreates two scratch
forges under `demo/.work/` (gitignored). Every artifact (spec, acceptance, code) is
authored by the LLM base — there is no mock or template.

## What each section demonstrates

| # | Section | Proves |
|---|---|---|
| 0 | Build | single Rust binary |
| 1 | NL → trustworthy software | intent → app → sandbox acceptance → signed attestation → PR; **gate BLOCKED** |
| 2 | Human-in-the-loop | 1-of-2 sign-off still blocked; 2-of-2 opens the gate → PR merges |
| 3 | Verify | re-check signatures + acceptance + sign-off from committed provenance |
| 4 | Cross-forge | identical flow + verification on a second, independent forge |
| 5 | Spec cycle | human feedback "add Kelvin" → spec v→v+1 → re-dispatch → re-verify |
| 6 | Reputation | projected purely from the signed attestations |
| 7 | Durable assets | the attestation JSON, the decision-log timeline, the gated git history |

## Inspect the artifacts after a run

```bash
F=demo/.work/github-local

# the signed attestation (in-toto Statement + openfab/generation predicate)
jq . "$F"/provenance/*-v1.att.json

# AI/Human attribution + sign-offs
jq '.statement.predicate | {agent, generated, signoffs}' "$F"/provenance/*-v1.att.json

# the SBOM
jq . "$F"/provenance/*-v1.sbom.json

# the human-readable decision log
cat "$F"/.openfab/runs/*/timeline.md

# the gated git history (provenance committed in-repo, merge commits from the gate)
git -C "$F" log --oneline --graph

# re-verify any run yourself
./target/release/openfab list --repo "$F"
./target/release/openfab verify --repo "$F" --run <run-id-from-list>
```

## Knobs

- `OPENFAB_CLAUDE_MODEL` — pin the model the claude base uses (`--model`).
- `OPENFAB_CLAUDE_TIMEOUT_SECS` — per-dispatch timeout (default 300).
- `--policy policy/trust.json` — the trust parameters (N-of-M, allowlists, sandbox rules).
- A real GitHub forge: `--forge github` with `OPENFAB_GITHUB_REMOTE=<git url>` and an
  authenticated `gh` (intentionally off by default).

## Negative / honesty checks you can run

```bash
# Tamper with a signed artifact → verify must fail:
F=demo/.work/github-local
sed -i '' 's/"author": "ai"/"author": "human"/' "$F"/provenance/*-v1.att.json
./target/release/openfab verify --repo "$F" --run <run-id>   # → FAIL (signature mismatch)
git -C "$F" checkout -- provenance                            # restore
```
