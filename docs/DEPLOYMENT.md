# OpenFab — Deployment on a Fresh Machine

How to stand up OpenFab (and the optional agent-chat / Palpo / Robrix stack) on another
machine, and run the [E2E test plan](E2E-TESTPLAN.md). Read this first — the E2E plan and
RUNBOOK assume things are already installed and use this author's local paths.

## TL;DR — what you're deploying

OpenFab is **one self-contained Rust binary** (no DB, no async runtime; it shells out to
`git`, `curl`, `claude`, `agent-spec`). Deploying **OpenFab alone is trivial** (Phase 0).
The hard part is the *optional* multi-agent stack, which lives in **separate repos that are
NOT vendored here**:

| Component | Repo / source | Needed for |
|-----------|---------------|------------|
| **OpenFab** | this repo | everything (Phase 0 standalone works on its own) |
| **agent-chat** | separate repo (`…/consult/agent-chat`) | Phase 1 — the Matrix agent team |
| **Palpo** | `palpo-and-octos-deploy/` in the **robrix2** repo (docker) | Phase 1/5 — the Matrix server |
| **robrix2** | separate repo (`…/robius/robrix2`) | Phase 5 — the human GUI |

Pick a tier:

- **Tier 1 — OpenFab only** → Phases 0, plus 2/2.1 dashboard/console, multi-project, upload,
  worktree self-host, GitHub forge. No Matrix. Easiest.
- **Tier 2 — + agent-chat + Palpo + Bridge** → Phase 1 (Matrix implementer) and the
  Robrix-less parts of Phase 5 (room binding, ingest, approval via API).
- **Tier 3 — + robrix2** → the full Phase 5 GUI story.

---

## 0. Prerequisites

### Tier 1 (OpenFab)
- **Rust** stable, **edition 2021, rustc ≥ 1.74** (`rust-toolchain.toml` pins stable +
  rustfmt + clippy). Install via rustup.
- **git**, **curl** (used by the forge adapters and bridge client).
- **agent-spec** CLI: `cargo install agent-spec` (this build targets **0.3.0**).
- An **LLM backend** for authoring/implementing:
  - `claude` CLI authenticated (subscription/OAuth login), **or**
  - `OPENFAB_LLM=dashscope` + `DASHSCOPE_API_KEY`.
- (Optional) a **GitHub PAT** for the GitHub forge (repo scope, or fine-grained Contents+PR
  write).

### Tier 2 (+ agent-chat / Palpo / Bridge)
- **Node ≥ 18** (the Bridge sidecar + the agent-chat backend).
- **Docker / OrbStack** (Palpo runs in containers).
- The **agent-chat** repo, installed with its own `.env` (Matrix creds, `API_TOKEN`,
  `MATRIX_AGENT_PASSWORD_SECRET`, …) and `node` deps.
- **tmux** (agent sessions run in tmux panes).

### Tier 3 (+ robrix2)
- The **robrix2** repo and Makepad build deps (see robrix2's README; it uses a custom Makepad
  fork). Build with `cargo run --features agent_chat` to compile in the workflow slash commands.

### Platform note
Developed/tested on **macOS (darwin)** with OrbStack + tmux. Linux should work for OpenFab and
the Node/agent-chat pieces; adapt Palpo's docker compose and the agent-launch/tmux specifics.
Windows: use WSL2.

---

## 1. Deploy OpenFab (Tier 1)

```bash
git clone <this-repo> openfab && cd openfab
cargo install agent-spec        # 0.3.0
cargo build --release           # → target/release/openfab
# sanity
cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check
```

Run the dashboard/console:

```bash
# pick writable dirs on THIS machine — nothing is hardcoded
export OPENFAB_SPEC=agent-spec
export OPENFAB_SPEC_DIR="$PWD/work/specs"
export OPENFAB_PROJECTS_DIR="$PWD/work/projects"
# use your real login (a stale ANTHROPIC_API_KEY in the shell shadows the claude OAuth login):
env -u ANTHROPIC_API_KEY ./target/release/openfab serve \
  --repo "$PWD/work/web" --port 8787 --policy policy/trust.json
```

Open `http://127.0.0.1:8787/` (and `/console`). Phase 0 + Phase 2/2.1 dashboard features work
now. **You're done for Tier 1.**

> The CLI works too: `openfab build "<intent>" --repo <dir> --base claude --forge local --gate team --policy policy/trust.json`.

---

## 2. Add agent-chat + Palpo + Bridge (Tier 2)

1. **Palpo (Matrix server)** — deploy from the robrix2 repo's `palpo-and-octos-deploy/`
   (docker). Verify: `curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8128/_matrix/client/versions` → `200`.
2. **agent-chat backend** — install the agent-chat repo (`install-full.sh`), fill its `.env`
   (Matrix homeserver `http://127.0.0.1:8128`, bot creds, `API_TOKEN`,
   `MATRIX_AGENT_PASSWORD_SECRET`, …), start its services (backend :8090, dashboard/monitor
   :8084, matrix bridge, push-relay). Verify: `curl -s "http://127.0.0.1:8090/api/agents?view=names"`.
   - The robrix2 repo's `roadmap/agentchat-demo/` has `start-demo.sh`, `register-accounts.mjs`,
     `link-skill.sh`, `nudge.sh` — adapt their path variables to **this machine**.
3. **OpenFab↔agent-chat Bridge** (in this repo). **Set `AGENTCHAT_DIR`** to the agent-chat repo
   on this machine — there is **no portable default** (the bridge warns at startup and disables
   result harvesting if it's unset):
   ```bash
   AC=/abs/path/to/agent-chat
   API_TOKEN=$(grep -E '^API_TOKEN=' "$AC/.env" | head -1 | cut -d= -f2- | tr -d '"')
   AGENTCHAT_URL=http://127.0.0.1:8090 AGENTCHAT_API_TOKEN="$API_TOKEN" AGENTCHAT_DIR="$AC" \
     BRIDGE_ASSIGNEE=wf_implementer BRIDGE_PORT=8077 OPENFAB_URL=http://127.0.0.1:8787 \
     node bridge/openfab-agentchat-bridge.mjs
   ```
   Verify: `curl -s http://127.0.0.1:8077/healthz` → `{"ok":true}`.
4. **Restart OpenFab serve in native agent-chat mode** (point it at the Bridge):
   ```bash
   env -u ANTHROPIC_API_KEY OPENFAB_SPEC=agent-spec OPENFAB_SPEC_DIR="$PWD/work/specs" \
     OPENFAB_PROJECTS_DIR="$PWD/work/projects" \
     OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 OPENFAB_AGENTCHAT_ROOM=openfab \
     OPENFAB_AGENTCHAT_MONITOR=http://127.0.0.1:8084 \
     ./target/release/openfab serve --repo "$PWD/work/web" --port 8787 --policy policy/trust.json
   ```
   `/api/bases` now shows `agent-chat` as **native via matrix bridge**.
5. **Launch the implementer agent** with the OpenFab skill (link it into the agent's Claude
   skills dir, then `agentchat up-v1 wf_implementer claude …`) — see [E2E §2](E2E-TESTPLAN.md).

Now Phase 1 works. For room binding / ingest / approval-by-API see [E2E §5c–5e](E2E-TESTPLAN.md).

---

## 3. Add Robrix GUI (Tier 3)

Build/run robrix2 with the workflow commands compiled in, log into Palpo, enable the
Settings toggle, bind the room, chat requirements, approve — full steps in
[E2E §5](E2E-TESTPLAN.md). robrix2 is a separate repo with its own (Makepad) build.

---

## 4. What to watch out for (the real gotchas)

1. **Hardcoded local paths in the docs/scripts.** `E2E-TESTPLAN.md`, `RUNBOOK.md`, and the
   robrix2 `roadmap/agentchat-demo/*` scripts use this author's paths
   (`/Users/zhangalex/…`, `~/Work/Projects/consult/agent-chat`, …). **Rewrite every path for
   your machine.** In *shipped OpenFab code* the only machine-specific knob is the Bridge's
   `AGENTCHAT_DIR` (now required, no default).
2. **Separate repos, separate installs.** agent-chat, Palpo, and robrix2 are **not** in this
   repo. You need to obtain and install each. OpenFab itself is self-contained.
3. **Secrets don't transfer.** Regenerate on the new machine: the agent-chat `.env`
   (`API_TOKEN`, `MATRIX_AGENT_PASSWORD_SECRET`, bot password, Matrix reg token), any GitHub
   PAT, and your LLM auth. Don't copy tokens between machines.
4. **LLM auth pitfall.** A stale/invalid `ANTHROPIC_API_KEY` exported in your shell **shadows**
   the `claude` CLI's OAuth login → 401. Either unset it (`env -u ANTHROPIC_API_KEY …`, as in
   all commands here) or set a valid key. OpenFab runs `claude -p … --setting-sources
   project,local` so global `~/.claude` hooks don't hijack the call.
5. **Ports.** Defaults: Palpo `8128`, agent-chat backend `8090`, agent-chat monitor `8084`,
   Bridge `8077`, OpenFab `8787`. All bind `127.0.0.1`. If you remote-access the box, tunnel
   over SSH — **do not expose these** (the API has a CSRF Origin guard but no authentication;
   it's a localhost dev tool).
6. **Self-hosting writes to git.** A project pointed at an existing repo makes OpenFab branch
   and commit there. Use the **"Isolate with a git worktree"** option (or a clone) — never your
   live working tree.
7. **GitHub forge needs a token.** Set `OPENFAB_GITHUB_TOKEN` + `OPENFAB_GITHUB_REPO`
   (`owner/repo`). Without them the GitHub forge honestly falls back to a local instance.
8. **Persistent state lives under your chosen dirs.** Runs/maintainers/identity are in each
   repo's `.openfab/`; the project + room registries are under `OPENFAB_PROJECTS_DIR`. Back up
   or wipe those between environments. `.openfab/identity/` and `…/maintainers/` (signing seeds)
   are gitignored — keep them secret.
9. **agent-chat agents are live Claude sessions.** They consume your LLM quota and run in tmux;
   they must be launched per machine and kept online for Phase 1/5.

---

## 5. Minimal smoke after deploy

```bash
# Tier 1
curl -s -o /dev/null -w "console %{http_code}\n" http://127.0.0.1:8787/console
curl -s http://127.0.0.1:8787/api/forges | python3 -m json.tool | head
# Tier 2
curl -s http://127.0.0.1:8077/healthz; echo
curl -s http://127.0.0.1:8787/api/bases | python3 -c "import sys,json;print('agent-chat:',[b['runtime'] for b in json.load(sys.stdin) if b['id']=='agent-chat'][0])"
```

Then follow [E2E-TESTPLAN.md](E2E-TESTPLAN.md) top to bottom. Operational env-var reference and
troubleshooting are in [RUNBOOK.md](RUNBOOK.md).
