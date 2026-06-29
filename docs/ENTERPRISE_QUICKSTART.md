# OpenFab Enterprise Quickstart

**For teams that already have an AI agent factory.** OpenFab is not a replacement
for your code-generation tooling — it is the **provenance + governance boundary**
you wrap around it. Your factory keeps producing code; OpenFab turns *how it was
produced* into a signed, portable, re-verifiable proof (an **AI-BOM**) and gates the
release behind machine acceptance + N-of-M human sign-off.

This guide shows the exact CLI flow to get that signed proof.

---

## TL;DR — the three commands that matter

```bash
# 1. Attest existing code your factory already produced → signed AI-BOM.
openfab attest --repo ./checkout --spec feature.spec.yaml

# 2. Notarize human approval (N-of-M gate).
openfab signoff --repo ./checkout --run <run-id> --as alice

# 3. Verify the proof anywhere — offline, no OpenFab server, from a bare clone.
openfab verify-file --repo ./checkout --att provenance/<spec>-v1.att.json
```

The output of step 1 — `provenance/<spec>-vN.att.json` — **is** the portable proof.
Commit it next to the code. Anyone can re-establish integrity, authenticity, and
conformance from that file alone.

---

## What the signed proof contains

The attestation is a DSSE-style signed envelope wrapping an
[in-toto Statement](https://in-toto.io/Statement/v1) carrying the
**`openfab/generation` predicate** (spec:
<https://open-fab.ai/attestation/generation/v0.1>):

| field | meaning |
|-------|---------|
| `generated[]` | per-file / per-range **human-vs-AI authorship** + content `sha256` |
| `agent` | the base + **model** that produced the code, and its `did:key` |
| `prompt_sha256` | fingerprint of the generation prompt (text is deliberately *not* stored) |
| `acceptance[]` | the **frozen acceptance contract** — the exact shell checks, embedded so anyone can re-run them |
| `acceptance_passed` | did the machine contract pass in the sandbox |
| `signoffs[]` | the recorded human sign-offs (the N-of-M gate) |
| `signatures[]` | ed25519 signatures (the fab key + each human approver) bound to the file digests |

Because the acceptance contract travels *inside* the signed attestation,
verification is **forge-agnostic and server-less**: `openfab verify-file` re-runs
the checks and re-verifies the signatures from any clone, with no OpenFab service.

---

## Two adoption paths

### Path A — route generation *through* OpenFab (recommended)

Wrap your factory as a **BasePort** adapter — a thin process OpenFab calls with the
spec and that returns a file manifest. OpenFab then independently runs acceptance,
signs, gates, and opens the PR. Your inner loop is untouched; OpenFab observes
authorship *as it happens*, which yields the strongest attestation.

```bash
openfab run --spec feature.spec.yaml --repo ./checkout \
            --base your-factory --forge local --gate team
```

A working reference adapter (~250 lines) lives at
`integrations/agent-chat/server.mjs`. A BasePort adapter only needs to:

1. accept a build request (`{ intent, target_dir, language, acceptance[] }`),
2. invoke your factory,
3. return `{ files: { "<path>": "<contents>", ... }, notes }`.

Everything else — sandboxed acceptance, signing, the trust gate, the PR — is OpenFab.

### Path B — attest code your factory *already* produced

If the code already exists on disk and you just want to stamp it, use `attest`.
It skips generation entirely: it reads the existing files under the spec's
`target_dir`, computes their digests, runs the acceptance contract in the sandbox,
and writes the signed attestation — same `openfab/generation` predicate, same
`verify-file` compatibility as Path A.

```bash
openfab attest --repo ./checkout --spec feature.spec.yaml --gate solo
```

**The files must be committed first.** Each run is branched from the repo's root
commit into a self-contained worktree, so `attest` snapshots the files up front and
re-commits them on the run branch; uncommitted/untracked files under `target_dir`
are not what gets attested. Commit your factory's output, then attest.

**Authorship is recorded as `ai`.** For attesting an AI factory's output that is the
honest claim; the attestation also records `base = attest` and a runtime of
`attested` (not `native`), so a verifier can see OpenFab notarized pre-existing files
rather than observing the generation itself.

Use Path B for retrofitting provenance onto an existing pipeline; use Path A when
you want OpenFab in the loop at generation time.

---

## The spec (the contract)

Both paths take a spec — natural-language intent plus machine-checkable acceptance:

```yaml
id: fee-rounding
version: 1
intent: "Bankers' rounding helper for fee calculation."
target_dir: app
acceptance:
  - id: a1-build
    check: "cargo build --quiet"
    must_pass: true
  - id: a2-rounds-half-even
    check: "python3 -c 'import app.fee as f; assert f.round_half_even(2.5)==2'"
    must_pass: true
```

The acceptance checks are arbitrary sandboxed shell — `cargo test`, `pytest`,
`grep` for required structure, anything that exits 0 on success. These exact
commands are what get embedded in the signed attestation.

---

## The trust gate

A release merges only when **both** hold:

- **machine acceptance** passed in the sandbox, and
- **N-of-M human sign-off** by distinct allowlisted maintainers.

```bash
openfab maintainer-add --repo ./checkout --name alice
openfab signoff        --repo ./checkout --run <run-id> --as alice
openfab signoff        --repo ./checkout --run <run-id> --as bob   # 2-of-2 → gate opens
```

Gate modes: `solo` (1-of-1, self-approve), `team` (2-of-2), `crowd` (2-of-3 for
untrusted contributions), `none` (provenance recorded, no human gate). A human
**cannot** sign past a failed acceptance check — the two conditions are independent.

---

## Verify — the part auditors care about

```bash
openfab verify-file --repo ./checkout --att provenance/fee-rounding-v1.att.json
```

This, from a bare clone with no OpenFab service:

1. recomputes each file `sha256` → **integrity**,
2. verifies the ed25519 signatures against their `did:key` → **authenticity**,
3. re-runs the embedded acceptance checks → **conformance**.

That is the difference between "trust us, the AI did it well" and a signed proof a
third party can re-check years later — the evidence regimes like the EU CRA
(vuln-reporting Sept 2026, full obligations Dec 2027, 10-year retention) increasingly
expect.

---

## When you do *not* need OpenFab

Be honest with yourself: if you ship only internal tooling with no audit,
supply-chain, compliance, or external-contribution pressure, OpenFab is overhead.
Its value is concentrated exactly where you must **prove** how software was produced —
at the release / contribution boundary — not in the inner loop where your factory
already shines.
