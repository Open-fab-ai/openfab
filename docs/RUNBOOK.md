# OpenFab × agent-spec × agent-chat — Operations Runbook

Operational guide for running the integration. Design rationale lives in
[robrix2-agentchat-integration.md](robrix2-agentchat-integration.md).

## 0. Prerequisites

| Tool | Install | Used for |
|------|---------|----------|
| Rust (2021, ≥1.74) | rustup | build OpenFab |
| agent-spec 0.3.0 | `cargo install agent-spec` | spec authoring + verify + gate |
| node ≥ 18 | — | the Bridge sidecar |
| an LLM backend | claude CLI (authenticated) **or** `OPENFAB_LLM=dashscope` + `DASHSCOPE_API_KEY` | draft `.spec.md`, implement |
| (Phase 1) agent-chat + Palpo | see robrix2 demo | the implementer agent team over Matrix |

## 1. Environment variables

| Var | Default | Meaning |
|-----|---------|---------|
| `OPENFAB_SPEC` | unset | set to `agent-spec` to author + verify via agent-spec |
| `OPENFAB_SPEC_DIR` | `specs` | where `.spec.md` contracts are written |
| `OPENFAB_SPEC_MIN_SCORE` | `0.7` | `agent-spec lint` quality gate threshold |
| `OPENFAB_AGENT_SPEC_BIN` | `agent-spec` | agent-spec binary path |
| `OPENFAB_LLM` | `claude` | `claude` (CLI) or `dashscope` |
| `OPENFAB_AGENTCHAT_URL` | unset | Bridge URL → agent-chat base runs **native** |
| `OPENFAB_AGENTCHAT_ROOM` | `openfab` | Matrix room id for the implementer |
| `OPENFAB_BRIDGE_POLL_SECS` | `5` | Bridge poll cadence |
| `OPENFAB_BRIDGE_TIMEOUT_SECS` | `1800` | Bridge task timeout |
| `OPENFAB_GITHUB_TOKEN` + `OPENFAB_GITHUB_REPO` | unset | GitHub REST API forge (api.github.com, no `gh` CLI). REPO is `owner/repo`. |
| `OPENFAB_GITHUB_REMOTE` | unset | legacy GitHub forge via the `gh` CLI (used only if no token) |
| `OPENFAB_<FORGEJO\|GITEA\|GITCODE>_URL/_TOKEN/_REPO` | unset | gitea-family REST forges |

### GitHub (REST API, token-based)

```bash
export OPENFAB_GITHUB_TOKEN=ghp_xxx          # a PAT with repo scope (or fine-grained: Contents+PR write)
export OPENFAB_GITHUB_REPO=youruser/yourrepo # owner/repo
# start serve (or run the CLI) — the forge dropdown shows GitHub as "live — GitHub REST API (token)"
```
OpenFab pushes via `https://x-access-token:<token>@github.com/<repo>.git` and opens a real PR
via `POST https://api.github.com/repos/<repo>/pulls`. No `gh` CLI needed. The token is read
from env, never logged or committed.

## 2. Phase 0 — local trustworthy skeleton (no Matrix)

```bash
cargo build --release
bash demo/run_agentspec_demo.sh           # NL → .spec.md → lint → implement → lifecycle
                                          # verify → sign → conformance/N-of-M gate → reproduce
```

Or by hand:

```bash
export OPENFAB_SPEC=agent-spec OPENFAB_SPEC_DIR=/tmp/of-specs
openfab build "build a CLI that adds two integers" \
  --repo /tmp/of-repo --base claude --forge local --gate team --policy policy/trust.json
openfab signoff --repo /tmp/of-repo --run <RUN> --as alice --policy policy/trust.json
openfab signoff --repo /tmp/of-repo --run <RUN> --as bob   --policy policy/trust.json
openfab verify  --repo /tmp/of-repo --run <RUN>            # conformance incl. C12
```

## 3. Phase 1 — agent-chat implementer over Matrix

1. Bring up Palpo + agent-chat backend/bridge/relay and register agents (see the robrix2
   `roadmap/agentchat-demo` scripts). Ensure the `wf_implementer` skill branch posts a
   `task_result` message per `bridge/README.md`.
2. Start the OpenFab↔agent-chat Bridge:
   ```bash
   AGENTCHAT_URL=http://127.0.0.1:8090 AGENTCHAT_API_TOKEN=<token> \
   BRIDGE_ASSIGNEE=wf_implementer BRIDGE_OPERATOR=operator \
   node bridge/openfab-agentchat-bridge.mjs        # listens on :8077
   ```
3. Point OpenFab at the Bridge and run with the agent-chat base:
   ```bash
   export OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077
   export OPENFAB_AGENTCHAT_ROOM='!demoboard:localhost'
   openfab build "…" --base agent-chat --forge local --gate team --policy policy/trust.json
   ```
   OpenFab dispatches into the room, polls for the implementer's files, **verifies their
   integrity**, then verifies/signs/gates exactly as in Phase 0.

## 4. Reproduce / verify a run

```bash
openfab verify    --repo <repo> --run <RUN>     # signatures + conformance (C1–C12)
openfab reproduce --repo <repo> --run <RUN>     # re-runs agent-spec lifecycle + re-checks
                                                # signature + bit-identical source
```

## 5. Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `inheritance error: spec 'project' not found` | a `.spec.md` has `inherits: project` | OpenFab strips it; if hand-written, remove the `inherits:` line |
| `claude exited / Invalid API key (401)` | claude CLI not authenticated | authenticate claude, or `OPENFAB_LLM=dashscope` + `DASHSCOPE_API_KEY` |
| `agent-spec … did not emit JSON` | agent-spec missing / wrong version | `cargo install agent-spec` (need 0.3.0); set `OPENFAB_AGENT_SPEC_BIN` |
| contract quality below threshold | lint score < `OPENFAB_SPEC_MIN_SCORE` | improve the contract, or lower the threshold |
| C12 fails with `skip` scenarios | a `Test:` selector matched no test | the implementer must add the bound test (skip ≠ pass) |
| `bridge file integrity check failed` | files mutated in transit / wrong contract | the implementer must return bit-identical full content (see `bridge/README.md`) |
| `bridge task … timed out` | implementer idle / not done | nudge the agent; raise `OPENFAB_BRIDGE_TIMEOUT_SECS` |

## 6. CI

`.github/workflows/ci.yml` runs `rustfmt --check`, `clippy -D warnings`, and
`cargo test --all-features`. A second non-blocking job lints any agent-spec `.spec.md`
contracts and runs `agent-spec guard` (dogfooding; informational until OpenFab's own specs
migrate from `.spec.yaml` to bound agent-spec contracts).
