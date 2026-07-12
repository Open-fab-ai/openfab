# OpenFab × robrix2 + agent-chat — Integration Design

> Status: Phase 0 + Phase 1 (OpenFab side) implemented; Phase 1 env, Phase 2 (robrix2 GUI)
> and live runs pending manual setup. See the [final checklist](#10-final-checklist).

## 1. Goal

Integrate the **robrix2 + agent-chat workflow demo** into OpenFab so that:

- **OpenFab is an optional trustworthy backend/certification layer** for room-built work.
- **agent-spec** authors the spec (a `.spec.md` Task Contract) and provides verification +
  a contract gate (`spec + verify + gate`).
- The **agent-chat** multi-agent team (over Matrix, via robrix2) can complete and submit code
  directly; OpenFab import/signoff is opt-in.
- **robrix2** becomes the human cockpit.

## 2. Key insight — same pipeline, two layers

robrix2's demo (`issue → spec → plan → implement → review → final_review`) and OpenFab's
spec-cycle (`intent → spec → dispatch → verify → sign → gate → signoff`) are the **same
pipeline at different layers**. robrix2 provides multi-agent collaboration UX but no
signing/provenance/SLSA/reproduce; OpenFab provides that trust backbone when a project asks
for certification. They are complementary, but OpenFab sign-off is not a mandatory barrier
for ordinary Robrix/agent-chat completion. agent-spec is the formalized, CLI-verified version
of OpenFab's spec.

## 3. Ownership boundary (no double state machine)

| Stage | Owner | Mechanism |
|-------|-------|-----------|
| author spec | **agent-spec** (LLM-draft + human-accept) | `ops::author_spec` → `agent_spec::author_via_agent_spec` (draft `.spec.md` → `agent-spec lint` gate → `parse` → map to `Spec`) |
| implement | **agent-chat** room agent | `BasePort::dispatch` (agent-chat native) → Bridge → Matrix room |
| verify | **agent-spec** | `spec_cycle` verify delegates to `agent-spec lifecycle`; `--ai-mode caller` routes design-intent scenarios to the reviewer |
| sign | **OpenFab, optional** | in-toto/SLSA + `openfab/generation` predicate (+ spec-contract hash + verdicts) when imported |
| gate | **OpenFab + agent-spec, optional** | conformance C1–C12; human N-of-M only when `gate=solo/team/crowd` |
| commit/PR | **agent-chat/room workflow by default; OpenFab forge when opted in** | room-built work may commit directly; imported OpenFab runs add trailers + `.spec.md` + provenance |
| reproduce | **OpenFab, optional** | re-run `agent-spec lifecycle` + verify signature + source hash |

## 4. Target topology

```
 robrix2 GUI (human cockpit)
   │  /create-issue · approve · /status        (Matrix plain text)
   ▼
 Palpo Matrix room  ◄─────────── optional OpenFab verification/provenance summary
   ▲                                   │
   │ implementer agent reads task,     │
   │ posts task_result (files+prompt)  │
 ┌─┴───────────────────┐               │
 │ Bridge sidecar      │◄── blocking HTTP ── OpenFab spec_cycle (optional)
 │ bridge/*.mjs        │   POST /tasks         author → dispatch → verify
 │ wraps agent-chat    │   GET  /tasks/:id     → SIGN → optional GATE → reproduce
 │ backend :8090       │   POST /post
 └─────────────────────┘
```

The **Bridge** absorbs the async↔blocking impedance (OpenFab has no tokio / is a single
binary; Matrix/agent-chat is async). Both robrix2 and agent-chat source stay unchanged.

## 5. agent-spec ↔ OpenFab mapping (implemented)

`agent-spec parse --format json` AST → `core::spec::Spec` (`adapters::agent_spec::parse_contract`):

| agent-spec | OpenFab `Spec` |
|------------|----------------|
| `meta.name` | `id` (slugged) |
| `intent.content` | `intent` (falls back to NL ask) |
| `acceptance_criteria.scenarios[]` | `acceptance[]` (id = scenario name, check = `agent-spec test: pkg::filter`) |
| `decisions` / `boundaries` | kept on `AgentSpecContract`, folded into `assumptions` (constrain the implementer) |
| `out_of_scope` | `assumptions` |

`agent-spec lifecycle --format json` `verification.results[]` → `AcceptanceOutcome[]`
(`outcomes_from_lifecycle`): `pass` → passed; `skip`/`fail`/`uncertain` → not passed
(**skip ≠ pass**). Per-scenario verdicts are recorded in the signed predicate.

**Gotcha (verified):** `agent-spec init` defaults to `inherits: project`, which makes
`contract`/`lifecycle` fail to resolve. OpenFab emits standalone `.spec.md` (draft prompt
says so; `extract_spec_md` strips any `inherits:` line defensively).

## 6. Provenance changes (implemented)

`openfab/generation` predicate gains (signed, tamper-checked):
- `spec_contract_sha256` — SHA-256 of the `.spec.md` (the contract is signed evidence)
- `agent_spec_verdicts[]` — `{scenario, verdict}` per BDD scenario
- `run_log_ref` — optional run-log reference

Conformance adds **C12.agent-spec-scenarios**: when verdicts are present, every scenario
must be `pass`. The commit gains an `OpenFab-Spec-Contract: <sha256>` trailer and the
`.spec.md` is committed into the repo's `specs/` (portable, travels with the code).

## 7. Bridge contract (implemented)

OpenFab side (`adapters::bridge_client`), Bridge side (`bridge/openfab-agentchat-bridge.mjs`):

| Method | Path | Body | Response |
|--------|------|------|----------|
| POST | `/tasks` | `{spec_ref,intent,target_dir,language,acceptance,assumptions,context,room}` | `{task_id}` |
| GET | `/tasks/:id` | — | `{status, files:{path:content}, file_hashes:{path:sha256}, model, prompt, error?}` |
| POST | `/post` | `{room,msg}` | `{ok}` |

**Trust (Phase 1-Trust):** OpenFab can only sign bytes it can hash. The Bridge returns
bit-identical full file contents + the prompt; `BridgeResult::verify_integrity` cross-checks
the content against the Bridge's claimed per-file hashes before signing. The implementer
agent must follow the result contract in `bridge/README.md` (posts `task_result` with
`files`, `prompt`, `model`).

## 8. Configuration (env)

| Var | Default | Meaning |
|-----|---------|---------|
| `OPENFAB_SPEC` | (unset) | `agent-spec` → author + verify via agent-spec |
| `OPENFAB_SPEC_DIR` | `specs` | where `.spec.md` contracts are written |
| `OPENFAB_SPEC_MIN_SCORE` | `0.7` | `agent-spec lint` quality threshold |
| `OPENFAB_AGENT_SPEC_BIN` | `agent-spec` | agent-spec binary |
| `OPENFAB_AGENTCHAT_URL` | (unset) | Bridge URL → agent-chat base runs **native** |
| `OPENFAB_AGENTCHAT_ROOM` | `openfab` | Matrix room id for the implementer |
| `OPENFAB_BRIDGE_POLL_SECS` / `OPENFAB_BRIDGE_TIMEOUT_SECS` | `5` / `1800` | poll cadence / timeout |

## 9. robrix2 cockpit (Phase 2 — minimal, additive)

robrix2 changes are minimal and additive (the GUI just sends slash commands as plain text;
OpenFab/Bridge/skill-side logic is used only when the room opts into certification):

- `/create-issue` already triggers the agent flow; with the Bridge, the room can optionally
  ask OpenFab to drive or certify a build.
- `approve` → maps to OpenFab `ops::signoff` (N-of-M) only for runs created with a human gate.
- OpenFab posts the verification/provenance/`agent-spec explain` summary back into the room via
  `BasePort::post` → Bridge `/post`, so it renders in robrix2's existing timeline.
- Optional: add `/verify`, `/provenance`, `/explain` to `WORKFLOW_SLASH_COMMANDS`
  (`robrix2/src/shared/mentionable_text_input.rs`, the `#[cfg(feature = "agent_chat")]`
  block). Purely additive constants; no logic change.

## 10. Final checklist

Items that need manual/live verification (could not be run in the dev sandbox):

- [ ] **agent-spec installed**: `cargo install agent-spec` (have: 0.3.0).
- [ ] **LLM key for drafting/implementing**: authenticate the `claude` CLI, or set
      `OPENFAB_LLM=dashscope` + `DASHSCOPE_API_KEY`. (Dev sandbox claude returned 401.)
- [ ] **Phase 0 end-to-end**: `bash demo/run_agentspec_demo.sh` — NL → `.spec.md` → lint →
      implement → lifecycle verify → sign → gate → reproduce.
- [ ] **Phase 1 env**: start Palpo (docker) + agent-chat backend/bridge/relay; register
      agents; link the `issue-workflow` skill (with the `task_result` contract for
      `wf_implementer`).
- [ ] **Start the Bridge**: `node bridge/openfab-agentchat-bridge.mjs`, then
      `OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 OPENFAB_AGENTCHAT_ROOM=<room> openfab build … --base agent-chat`.
- [ ] **Phase 1 end-to-end**: confirm the implementer's files come back, integrity passes,
      OpenFab signs + records conformance; human sign-off only when gate mode requests it.
- [ ] **Phase 2 robrix2**: add the optional slash commands; confirm gated runs can still
      use `approve` → signoff and that verification/provenance summaries appear in the room.
- [ ] **Phase 3 reviewer caller**: run `agent-spec lifecycle --ai-mode caller`; route
      `pending-ai-requests.json` to `wf_reviewer`; merge with `agent-spec resolve-ai`.

## 11. What is implemented (with tests)

- `adapters::agent_spec` — author (`author_via_agent_spec`/`author_from_md`), `parse_contract`,
  `lint_gate`, `verify_via_lifecycle`/`lifecycle_run`, `outcomes_from_lifecycle`,
  `verdicts_from_lifecycle`, `contract_sha256`, `lifecycle_ai_pending`, repo/spec paths.
- `adapters::bridge_client` — `build_task_payload`, `BridgeResult::{parse,verify_integrity,
  into_manifest}`, `post_task`/`get_task`/`post_message`/`dispatch_and_wait`.
- `adapters::base_framework` — agent-chat native dispatch + `post` via the Bridge.
- `core::provenance` / `core::conformance` — agent-spec evidence fields + C12 gate.
- `ops`/`spec_cycle` — `OPENFAB_SPEC=agent-spec` branch, verify delegation, `.spec.md`
  committed into the repo, reproduce re-runs lifecycle.
- `bridge/openfab-agentchat-bridge.mjs` — the sidecar.
- `demo/run_agentspec_demo.sh` — Phase 0 demo.

All covered by unit tests (pure logic) + an `#[ignore]` live smoke
(`tests/agent_spec_authoring_smoke.rs`). `cargo test` is hermetic; `clippy -D warnings` and
`rustfmt` are clean.
