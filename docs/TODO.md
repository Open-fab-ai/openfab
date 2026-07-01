# OpenFab — TODO

The single, consolidated backlog. Supersedes the old `ROADMAP.md` and the "Known gaps"
list in `HANDOFF.md`. Ordered: **Priorities** (what's next), then **Backlog**,
**Engineering debt**, and **Process / docs**.

Rule of the house: a TODO is not a license to bundle. Refactors and new features get
their own session (R8); tests land with code (R6); nothing merges without a fresh-session
review (R13).

---

## Priorities

### P1 — OpenFab Web: the browser fab (serverless, as a GitHub Page)

Naming note: this is deliberately NOT called a "demo" — the first milestone is
demo-grade in scope, but the same artifact evolves into the real browser client
(forge push, then a swarm participant). Name: **OpenFab Web**; deploy target:
`app.open-fab.ai` (a Pages CNAME; no separate repo — the app is `web/` in this repo).

**Goal:** a browser-only OpenFab that runs as a static GitHub Page — no server. The user
configures an LLM provider on the page (OpenAI-compatible key/URL, incl. Ollama Cloud),
and OpenFab generates a small app that runs **entirely in the browser**, then produces a
**real signed AI-BOM** for it. The point is to make **attestation** tangible to anyone,
anywhere, with nothing to install.

**Why browser-only matters:** the whole trust story (sign → tamper → verify) can be shown
client-side, and it travels (a link, a conference kiosk, a plane).

**What can be genuine in the browser (honest breakdown, R14 — never fake a check):**

| Capability | Browser-only? | How |
|---|---|---|
| Swarm simulation (planner/coder/reviewer) | ✅ | direct `/v1/chat/completions` calls from JS |
| Spec + acceptance authoring | ✅ | one LLM call |
| in-toto/SLSA signing (did:key, ed25519, sha256) | ✅ | WebCrypto + noble-ed25519; did:key in-browser |
| Tamper-evident verify (hash + signature) | ✅ | recompute sha256 + verify ed25519 client-side |
| **Run acceptance checks** | ⚠️ hard | no shell in a browser. Restrict targets to **JS/HTML checkable in-browser**, or use **Pyodide/WASM** for python. **Never** label a simulated check as "passed." |
| Forge push | ◑ | GitHub API + token to commit the `att.json`, or just offer a **download** |

**Recommended first scope:** restrict generated apps to **browser-runnable targets**
(JS/HTML), so both the *run* and the *acceptance check* are genuine in-browser; real
did:key signing; real in-browser tamper/verify. ~a few hours of frontend, no Rust —
reuse the existing UI and swap `/api/*` calls for client-side LLM + WebCrypto.

**Architecture decision (maximize code sharing — one SPA, two backends behind a port):**
- The browser app is NOT a separate codebase or repo. `web/` in this repo stays the
  single UI source (same look & feel by construction); the GitHub Page is only a
  **deploy target** (an Action publishes `web/` + the browser adapter to Pages) while
  the Rust binary keeps `include_str!`-ing the same files. One change → both reflect it.
- The seam already exists: all ~36 server calls funnel through the single `api()` in
  `web/app.js`. Extract it into an **ops port** with two adapters — `ops_server.js`
  (today's `fetch /api/*`) and `ops_browser.js` (client-side LLM + WebCrypto signing +
  in-browser acceptance for web targets + forge-API push).
- **Mode selection:** probe `/api/ping` at boot → server mode; else browser mode; plus a
  manual switch in the Settings drawer. A header **mode badge** ("server fab" /
  "browser fab") reuses the native/bridged honesty-badge pattern.
- **Capability matrix, not if-scattering:** adapters declare capabilities
  (`shellSandbox`, `forgePush`, `launchApp`, …); features unsupported in a mode render
  honestly disabled with a tooltip — never silently stubbed (R14).
- **Where it lives:** `web/` in this repo (`index.html` + `app.js` + `style.css`), baked
  into the Rust binary via `include_str!` (server mode) AND published as-is to Pages
  (browser mode). One folder, two delivery paths.
- **STANDING RULE — every `web/` change must keep BOTH modes working.** Server mode
  (local OpenFab binary) and browser mode (Pages) ship from the same files; a change that
  breaks either is a regression. Dual-mode behavior is a review criterion for all future
  web UI work.
- **Base selection in browser mode:** the base dropdown is hidden — a static page cannot
  reach local CLIs (claude/codex) or a local agent-chat server. Browser mode has one
  built-in base, the **browser swarm** (planner/coder/reviewer as prompted roles over
  direct LLM calls in the tab). A CORS-enabled *remote* base may return later as an option.
- **LLM provider config (Settings card; mandatory in browser mode, no env vars there):**
  preset providers — **Ollama (CLOUD mode: ollama.com API + key — local Ollama is not
  reachable from a hosted page without user config)**, OpenRouter, OpenAI, Anthropic,
  NVIDIA, DashScope/Qwen, custom OpenAI-compatible URL — plus API key + model, stored in
  localStorage. Mark which providers are browser/CORS-friendly rather than pretending all
  work; state plainly that the key never leaves the browser except to the provider.
- **Artifact exits in browser mode (ForgePort re-embodied as forge REST APIs):**
  (1) **Download** — zip of code + `att.json` + SBOM, zero auth, always works;
  (2) **GitHub** — REST API with a fine-grained PAT, code + attestation in ONE commit,
  optional PR; (3) **Gitea/Forgejo** — token API when the instance enables CORS. The
  local-git forge does not exist in a browser.
- **Sequencing (R8/R4):** `app.js` is already over the 300-line budget, so this lands as
  (1) a pure refactor session extracting the ops port (zero behavior change), then
  (2) a feature session adding `ops_browser.js` + the Pages deploy workflow.

**Forge push from the browser (genuine, serverless):** GitHub's REST API supports
commits/branches/PRs from browser JS; auth via a fine-grained PAT the user pastes
(full OAuth needs a token-exchange micro-service — defer). Gitea/Forgejo work the same
with CORS enabled. Push the code and the `att.json` **in the same commit** so artifact
and attestation are born bound together.

**Later evolution:** grow it into a **web agent swarm that resolves a GitHub issue** —
point the page at an issue URL, the in-browser swarm proposes a fix, and OpenFab attaches
a signed AI-BOM to the resulting change. For `web-target` tasks **the browser is the
complete fab** — generate → run → verify → sign → deliver, all client-side — so anyone
with an LLM API key can participate. Peers are untrusted by design: a requester
**re-verifies** any contribution locally (re-run checks, re-hash, check signatures)
before it counts, each peer signs with its own did:key, and reputation-from-attestations
provides the sybil resistance ("untrusted compute, verified results"). Coordination
transport (WebRTC signaling, a relay, or forge-issues-as-message-bus) is the open
question; prototype with two cooperating tabs on one machine first.

### P2 — OpenFab develops itself (interactive self-improvement)

**Goal / open design question — needs to be thought through before building.** Today
`demo/run_selfhost_demo.sh` proves the *mechanism*: OpenFab clones its own source, an
`attest`/base run implements a change to itself, verified by `cargo build`/`cargo test`
in the sandbox, signed, and gated on N-of-M sign-off. What's missing is the **interactive
loop**: a person types a feature request in natural language, points at an OpenFab
install/checkout, and OpenFab adds the feature to *itself* through that interface.

**Questions to resolve first (don't build until these are answered):**
- **Interface:** is it the existing web textbox + a "target = this repo" selector, or a
  dedicated self-dev mode? How does the user point at the install folder safely?
- **Isolation:** self-dev must run in a clone/worktree (never mutate the running install
  in place); how does the improved build get proposed back (PR), reviewed, and only then
  swapped in?
- **The loop:** study **Claude Code agent loops / self-improving loops** as prior art —
  plan → edit → run tests → observe → revise, bounded by a human gate. OpenFab's version
  must keep the trust ceremony (acceptance + N-of-M sign-off) as the loop's exit gate, so
  self-improvement can never merge an unverified change. Explicitly avoid unattended
  self-rewrite (PRD §6: humans stay in the loop).
- **Honest guardrail (R14):** the loop's "it passed" must be a real `cargo test` in the
  sandbox, never a model's say-so.

**Next step:** write a short design note (interface + isolation + loop + gate) before any
code. This is a design task, not an implementation task, yet.

---

## Backlog (from the former ROADMAP)

- **Lineage chaining.** Embed `parent_attestation_sha256` in the generation predicate so a
  release cryptographically links to the version it refined (provable v1→v2→v3), instead
  of lineage living only in local run-state (`parent_run`). Lightweight; high audit value.
- **"Use the repo's existing tests as the contract" mode.** Attach to an existing repo +
  test suite and run *those* as the acceptance contract (no authored spec). The OSPO
  gate-inbound-contributions case. **Partially delivered** by `openfab attest` (signs +
  gates existing files against a spec's checks); the remaining piece is auto-adopting a
  repo's own test suite as the contract.
- **Behavioral approval as a first-class signed event.** Add a signed record that
  "maintainer X viewed build Y's running output and approved," so the human's behavioral
  "yes" is itself notarized (today the gate signs over the artifact hash only).
- **OpenFab shows the live swarm.** Stream agent-chat's live agent activity into OpenFab's
  own timeline (today you watch the swarm on the agent-chat dashboard :8084).
- **AI-BOM split-hash.** Hash the intent and the acceptance contract as separate predicate
  fields rather than one blob, so you can prove "same intent, different checks."

## Engineering debt

- **File-size budget (R4, >300 lines).** `spec_cycle.rs`, `cli.rs`, `provenance.rs`,
  `ops.rs`, `trust.rs`, `server.rs`, `llm_backend.rs`, `base_framework.rs`, `runstate.rs`.
  `cli.rs` and `ops.rs` grew further with `attest`. Split each in its own refactor session
  (R8) before extending it.
- **`attest` follow-ups (from the R13 review).** (a) registry guard test:
  `assert!(build_base("attest").is_err())`; (b) a failed-acceptance-stays-blocked test.
  Also, `attest` records `author: ai` (no per-line human/AI mix, no claimed-vs-observed
  distinction) and requires committed files — revisit if enterprises ask.
- **Native base / live forge — untested against real servers.**
  `base_framework::dispatch_native` and the `forge_rest` PR path are implemented but not
  exercised against real instances. Verify when one is connected.
- **`BasePort::events()`** (live inbound human-feedback stream, Matrix when base=HiClaw) is
  not a trait method yet — feedback enters via the API/CLI. Add when wiring a live HiClaw.
- **No Rekor transparency log / Nix bit-identical build yet** (later-phase production swaps).
- **Web UI has no test harness** (~600+ lines vanilla JS via `include_str!`, verified
  manually). A Playwright smoke test is the natural follow-up.

## Process / docs

- **README** — updated to cover `attest` + the enterprise quickstart (done).
- **`open_questions` are surfaced but not enforced.** The spec-author flags ambiguities,
  but they live only in the decision log — surface them in the Spec step and/or make an
  unanswered open question visible at approval time.
- **Review debt.** The `attest` slice got a fresh R13 review; the larger UI/base/mode work
  merged to `main` via PR #2 without one. Consider a post-hoc review sweep.
- **agent-chat dashboard is fragile** — backgrounded node gets reaped; no durable
  auto-restart. A launchd LaunchAgent (or the start script) is the fix before live demos.
- **OSSF community** — an AI-authorship predicate issue is drafted for the OpenSSF
  community (feedback, not adoption). Coordinate with any TAC-member referral so it isn't
  double-posted.
