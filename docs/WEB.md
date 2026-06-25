# OpenFab web UI

The visual, end-to-end demo. One binary serves the UI *and* the API (the whole SPA is
`include_str!`'d into the binary — nothing to install, matching the sovereign posture).

## Launch

```bash
demo/run_web_demo.sh            # builds + serves on http://127.0.0.1:8787
# or:
openfab serve --repo demo/.work/web --port 8787 --policy policy/trust.json
```

## The flow (what to click)

1. **Describe what to build.** One natural-language intent box (prefilled with a sample).
   You supply *only* the intent — the Base (LLM) authors the spec **and** its
   machine-checkable acceptance criteria from it; review them in the **Spec** step of the
   live workflow.
2. **Pick a base and a forge.** Six bases, four forges. Each shows an honest badge:
   - base runtime: `native` (claude, or a framework with its `OPENFAB_*_URL` set) or
     `bridged` (a framework whose native runtime isn't connected — the task runs via
     OpenFab's LLM backend and the provenance says so).
   - forge: `live` (creds configured) or `local instance` (offline, portable provenance).
3. **Fabricate.** One click. The **live workflow** streams the decision log as it happens; the
   stepper moves Spec → Generate → Verify → Sign → Gate. **Click any step** to inspect
   exactly what it produced: **Spec** shows the machine-checkable spec compiled from your
   NL; **Generate** the agent/model/prompt-hash + files; **Verify** each acceptance
   criterion (a1, a2, …) and its result; **Sign** the signatures; **Gate** the trust
   decision. It ends **BLOCKED** — machine acceptance passing is not enough to merge.
4. **Human approval — intent, not crypto.** Pick an **Approval policy** in section 1:
   **Solo** (you approve your own release — one click), **Team** (2 distinct maintainers),
   **Crowd** (untrusted contributors/agents — 2 maintainers gate the merge), or **None**
   (provenance only). The approval panel asks the human the *only* question they can
   actually answer — "does the software do what you asked?" — after you've **run it** in
   "Try the software". The machine conformance checks (C1–C11) are automatic and tucked
   away; you never hand-verify a DID. If it's not right, **Request changes** (refine) or
   **Reject** — the flow is never a dead end.
5. **Try the software.** Run the generated app right there in the policy-gated sandbox —
   click a preset command (built from the acceptance checks + the generated file) or type
   your own, and see stdout/stderr + exit code. This is how you *see what it does*, not
   just read the code.
6. **The product & its provenance.** Tabs for the generated **Software**, the signed
   **Provenance** (agent DID · model · prompt hash · per-file **ai/human** attribution ·
   signatures · sign-offs), the **Audit trail**, the **SBOM**, and the **Decision log**.
   The **Audit trail** tab shows the live **git commit graph** — every action (the AI's
   authorship and each human sign-off) is a signed commit carrying provenance trailers
   (`Spec`, `Co-Authored-By` agent DID, `OpenFab-Base`, `OpenFab-Attestation`,
   `OpenFab-Acceptance`, `OpenFab-Signoff`) — and a **"verify independently"** panel with
   the exact `git` / `jq` / `cosign` / `slsa-verifier` commands a third party runs to
   inspect/verify the same artifacts **without OpenFab** (the EU-CRA / SLSA audit story).
7. **Reproduce & verify** (the sovereign proof). Re-verifies the signature, confirms the
   committed source is **bit-identical** to the signed digests, and **re-runs every
   acceptance check** in the sandbox — "trust nothing, verify everything".
8. **Refine.** Tried it and it's not right? Type a feedback note (and optionally a new
   acceptance check) → the spec bumps v→v+1 → the cycle re-runs. The reputation panel
   updates from the attestations.

## Switches (env)

| Want | Set |
|---|---|
| Qwen/DashScope instead of claude for bridged bases | `OPENFAB_LLM=dashscope` + `DASHSCOPE_API_KEY=…` |
| A base's **native** runtime (badge → native) | `OPENFAB_AGENTSCOPE_URL` / `OPENFAB_HICLAW_URL` / `OPENFAB_AGENTCHAT_URL` / `OPENFAB_OPENHANDS_URL` |
| A **live** GitHub forge | `OPENFAB_GITHUB_REMOTE=<git url>` (+ authenticated `gh`) |
| A **live** Forgejo/Gitea/GitCode | `OPENFAB_<KIND>_URL`, `OPENFAB_<KIND>_TOKEN`, `OPENFAB_<KIND>_REPO` |
| Pin the claude model | `OPENFAB_CLAUDE_MODEL=…` |

## API (same `ops` layer as the CLI)

```
GET  /api/bases | /api/forges | /api/maintainers | /api/reputation | /api/runs
POST /api/maintainers {name}
POST /api/author {intent}                           -> the LLM-authored spec (preview)
POST /api/run {intent | spec, base, forge, gate}    -> {run_id}   (runs on a background thread)
GET  /api/runs/{id}                                 -> record/status
GET  /api/runs/{id}/events?since=N                  -> live timeline
POST /api/runs/{id}/signoff {as}
POST /api/runs/{id}/feedback {note, add_check?}     -> {run_id}
GET  /api/runs/{id}/verify | /artifacts | /audit
POST /api/runs/{id}/reproduce
POST /api/runs/{id}/exec {cmd}                      -> run the product in the sandbox
```

## Independent verification (no OpenFab required)

Everything OpenFab produces is in **standard, portable formats committed in the repo**, so
the audit trail is readable and verifiable by third-party tools — the proof it isn't
locked into OpenFab and travels across forges:

| What | Tool(s) | Command |
|---|---|---|
| Signed commit graph + provenance trailers | `git`, `gitk`, VS Code Git Graph/GitLens, **GitHub/Gitea/Forgejo web UI** | `git -C <repo> log --graph --decorate --format=full` |
| in-toto/SLSA attestation | `jq` / any JSON tool, in-toto / SLSA verifiers | `jq . <repo>/provenance/<spec>.att.json` |
| SBOM (SPDX 2.3) | SPDX tools, `syft` | `jq .files <repo>/provenance/<spec>.sbom.json` |
| Signature (production path) | **cosign** (Sigstore) | `cosign verify-blob …` (v0.2 swap for did:key) |
| SLSA provenance (production path) | **slsa-verifier** | `slsa-verifier verify-artifact …` |

Push a run to a *live* forge (set its env) and the same commit graph + trailers render
natively in the GitHub/Gitea/Forgejo web UI — cross-forge, OpenFab-independent.

## Honesty notes (R14)

- A `bridged` base is labelled bridged everywhere (badge + provenance `runtime`); it does
  not pretend an external framework server is running. Connect a real one via its
  `OPENFAB_*_URL` and the same run goes `native`.
- A `local instance` forge is labelled as such; it proves the `ForgePort` seam + portable
  in-repo provenance, not a live remote.
- Reproduce reports each sub-check independently (signature / source-identical /
  acceptance); a single failure flips the verdict to **NOT REPRODUCIBLE**.
