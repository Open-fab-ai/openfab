# OpenFab value propositions — what the demo proves, and how to see it yourself

**Visual demo:** `demo/run_web_demo.sh` → http://127.0.0.1:8787 — every proposition below
is a click in the UI ([`docs/WEB.md`](WEB.md)). **Scriptable demo:** `demo/run_demo.sh`
(base = `claude` by default; pass a framework base, e.g. `demo/run_demo.sh agentscope`).
All drive the *same* Core pipeline via one shared `ops` layer. Every artifact — the spec,
the acceptance criteria, and the code — is authored by the Base (LLM); there is no mock
and no template. Below, each proposition is tied to the concrete evidence it produces.

The demo builds a real product from one NL ask:

> *"Build a small command-line temperature converter … it must also support a
> `--selftest` flag that checks all conversions and exits non-zero if any result is
> wrong."* — `specs/demo-temp-converter.spec.yaml`

---

### 1. Natural language in, software out
The spec's `intent:` is plain English; the Base (LLM) also *authors* that spec — including
its acceptance criteria — from the user's bare intent. The LLM then turns it into a working
app under `app/`. **Evidence:** the generated file exists and runs; re-running the same
intent produces freshly-generated source (proof it is genuine LLM generation, not a
hardcoded answer or a template). There is no mock and no built-in solution library — every
base is a real LLM (claude natively, or a framework base bridged through the LLM backend).

```bash
cat demo/.work/github-local/app/convert.py
```

### 2. Trustworthy — signed provenance on every artifact
Each product carries a signed in-toto/SLSA attestation, committed in-repo.
**Evidence:** `provenance/<spec>-vN.att.json` — an in-toto Statement, signed
(ed25519 over canonical JSON) by the fab's did:key, plus a `payload_sha256` tamper-pin.

```bash
jq '.statement.predicateType, .signatures[].role' demo/.work/github-local/provenance/*.att.json
```

### 3. AI-vs-Human attribution (the `openfab/generation` predicate)
The custom in-toto predicate records the **agent DID, base, model, prompt hash,
params, and each changed file/line range with an `author: ai|human` tag**. Human
sign-offs are appended as `human` authorship + signatures.
**Evidence:**

```bash
jq '.statement.predicate | {agent, prompt_sha256, generated, signoffs}' \
   demo/.work/github-local/provenance/*.att.json
```

### 4. Reproducible — verification, not generation
`openfab verify` re-checks the contract from the committed files alone: signatures
verify, the recorded acceptance passed, attribution is present, sign-off is present.
Tampering with any signed byte fails verification (unit-tested:
`tampering_with_code_breaks_verification`).
**Evidence:** `openfab verify --repo … --run …` prints 11 conformance checks → PASS.
(Bit-identical *builds* via Nix are the v0.2 step; v0.1 reproducibility = re-runnable
acceptance + signature verification.)

### 5. Human-in-the-loop — the trust gate blocks merge
Machine acceptance passing is **not** enough to merge. The gate requires **N-of-M
distinct, pre-approved maintainer sign-offs** (default 2-of-2). One maintainer signing
twice does *not* satisfy it (unit-tested). The single most sensitive component — the
gate — is versioned, never hot-loaded, never self-approved (PRD §6).
**Evidence:** in the demo, after 1-of-2 the gate is **BLOCKED**; after 2-of-2 it
**ACCEPTS** and the PR merges. See the timeline `🛡️` lines.

### 6. Neutral / cross-forge — portable provenance
The same Core runs against two independent forges (`github-local`, `forgejo-local`)
with identical behavior; the attestation/SBOM are plain JSON committed in-repo, so they
travel with the code. The GitHub adapter (`forge_github.rs`) is real but gated off by
default so the overnight build never touches a real account.
**Evidence:** section 4 of the demo verifies the same product on both forges.

### 7. Swappable base — base-agnostic Core
`--base claude` and the four framework bases (`agentscope`/`hiclaw`/`agent-chat`/
`openhands`) exercise the *exact same* provenance / signing / trust / verify path — they
share one parameterized adapter (`base_framework`), so adding a base is metadata, not a
new pipeline.
**Evidence:** run the demo with each base; the only differences are the agent log line,
the recorded `base`/`runtime`, and the generated source — the trust machinery is identical.

### 8. Spec cycle — iterative, human feedback drives v→v+1
`openfab feedback` folds a human NL note into the spec, bumps the version, optionally
adds a new acceptance criterion, and re-runs the whole cycle — a fresh attestation,
fresh sign-off, fresh PR.
**Evidence:** section 5 adds *"also support Kelvin"* → spec v2 with a new `c2k` check →
re-dispatched, re-verified, re-gated.

### 9. Decision memory + reputation — the durable asset
Every run persists a human-readable timeline (the Matrix-room-timeline stand-in) and
its attestation. Reputation is a pure projection over those signed attestations — no
separate trust DB.
**Evidence:** `.openfab/runs/<id>/timeline.md`, and `openfab reputation --repo …`
(authored / accepted / acceptance-rate / sign-offs, per DID).

---

### Honest controls (R14)
- Every artifact comes from a real LLM — there is **no mock**, no template, and no
  built-in solution library that could rubber-stamp a pass. If the base can't produce a
  working solution, acceptance fails and the gate stays blocked.
- A failed acceptance check keeps the gate **blocked** (`acceptance_passed: false`);
  empty/failed steps never count as success (no vacuous pass).
- A `bridged` base is labelled bridged in the UI **and** in the provenance `runtime`
  field — it never pretends an external framework server is running.
- The sandbox label is recorded truthfully (`gated-host-subprocess`) — OpenFab never
  claims container isolation it didn't use.
