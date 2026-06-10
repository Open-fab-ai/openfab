# OpenFab — Trustworthy Software, Fabricated from Natural Language

*An open, neutral fab for the age of AI-authored software.*
**open-fab.ai · Apache-2.0 · Governance: AOSF**

---

## The problem: AI writes the code, but the trust model broke

A fast-growing share of the world's software is now written by AI agents. That shift is
permanent — and it has quietly invalidated the assumptions the software supply chain was
built on. Our tools for trusting code assume a **human author**, working at **human speed**,
under **human review**. AI authorship breaks all three at once.

When an agent emits a change, the questions that decide whether you can *trust* it suddenly
have no answer:

- **Who wrote this?** Which model, which agent identity, from which exact instruction?
- **Who is accountable?** Did a responsible human actually approve it — or did it merge
  because the tests happened to pass?
- **Does it meet a contract,** or does it merely *look* right? "The tests are green" and
  "the diff reads plausibly" are not the same as "it does what was asked."
- **Can anyone else verify it** — independently, later, without trusting the tool that
  produced it?

At the same time, the rules are tightening from the other direction. The **EU Cyber
Resilience Act**, **SLSA**, executive orders on software supply-chain security, and
enterprise procurement all now demand **provenance, signed attestation, SBOMs, and a clear
chain of accountability** — exactly as authorship becomes opaque and moves at machine scale.

And the market is fragmenting the wrong way: AI coding is consolidating into **closed
platforms** that lock you to one vendor's agent and one vendor's forge, with **no neutral,
portable notion of "trustworthy AI-built software"** that survives moving between them.

> The result is a **trust gap**: more code, written faster, by less accountable authors,
> under stricter rules, with no portable way to prove any of it.

---

## The gap: the pieces exist — the integration doesn't

This is not for lack of primitives. The open ecosystem has produced remarkable building
blocks:

- **Provenance & attestation:** in-toto, SLSA
- **Signing & transparency:** Sigstore (cosign / fulcio / rekor)
- **Content provenance:** C2PA
- **Identity:** DID (`did:key`, `did:web`)
- **Bill of materials:** SPDX, CycloneDX, Syft
- **Policy:** OPA / Rego
- **Agent runtimes:** AgentScope, HiClaw, agent-chat, OpenHands
- **Forges:** GitHub, Forgejo, Gitea, GitCode

What's missing is the thing that turns these parts into a guarantee. **No one has integrated
them into a single, open, neutral "fab"** that takes natural language in and emits
trustworthy software out — where the provenance is **portable** (it travels across vendors
and forges instead of being trapped in one), **AI-aware** (it records machine authorship and
human accountability as first-class facts), and **end-to-end** (from the natural-language
intent all the way to a verifiable, reproducible release).

Existing AI coding tools optimize for **speed of generation**. Existing supply-chain tools
secure **human-authored builds**. Neither closes the loop for *AI authorship under human
accountability*. That loop is the gap OpenFab fills.

---

## OpenFab: natural language in, trustworthy software out

**OpenFab is a fab** — a software factory. You describe what you want in plain English;
OpenFab produces a working software product in which **every artifact carries a reproducible
build + signed provenance + AI/Human attribution**, on a **swappable agent base** and a
**swappable forge**, under **neutral governance**.

OpenFab does not reinvent the primitives above — it **composes** them. The novel, durable
thing is the **integrated whole** (the fab) and the asset it emits: not just the code, but
the **process + decision memory + signed provenance** behind it.

### The spec cycle — how a sentence becomes trustworthy software

```
natural language intent
   → Spec (the LLM authors a machine-checkable contract + acceptance criteria)
   → Generate (an agent base writes the code; authorship recorded by DID, model, prompt-hash)
   → Verify (acceptance criteria re-run in a policy-gated sandbox — no vacuous passes)
   → Sign (in-toto/SLSA attestation + SBOM, signed; per-file AI/Human attribution)
   → Gate (merge BLOCKED until N-of-M human maintainers sign off — never self-approved)
   → Provenance committed in-repo, portable and forge-neutral
   → Reproduce & verify — by anyone, anywhere, without OpenFab
```

Every step is a **signed git commit carrying provenance trailers**. The full trail is plain
git + JSON, committed in the repository, so a third party can audit and verify it with
standard tools on any forge — the tamper-evident, attributable evidence that CRA and SLSA
ask for.

### What makes it different

| Principle | What it means in OpenFab |
|---|---|
| **AI/Human attribution is first-class** | The `openfab/generation` in-toto predicate records the agent DID, model, prompt hash, parameters, and each changed file/line range tagged `author: ai \| human`. Human sign-offs are appended as `human` authorship with signatures. |
| **Human accountability, by design** | Machine acceptance passing is *not* enough to ship. A configurable **N-of-M human gate** (solo / team / crowd) blocks the merge. The most sensitive component — the trust gate itself — is versioned, never hot-loaded, **never self-approved.** |
| **Trust nothing, verify everything** | One click re-verifies signatures, confirms the committed source is **bit-identical** to the signed digests, and **re-runs every acceptance check**. Reproducibility is verification, not faith. |
| **Neutral & portable — no lock-in** | **Ports & adapters:** the agent **Base** and the git **Forge** are swappable behind thin interfaces. Provenance is forge-neutral JSON committed in-repo, so it travels with the code across vendors and forges. |
| **Sovereign by default** | A single static binary, offline-verifiable identity (`did:key`), no mandatory cloud service. Suitable for air-gapped and sovereign deployments. |
| **The fab builds the fab** | OpenFab develops OpenFab — each self-change verified by the project's own tests, signed, and gated by the same human trust model. Its own provenance trail is the reference showcase. |

---

## Who it's for

- **Enterprises shipping AI-written code** — provenance and signed attestation that satisfy
  CRA / SLSA / audit, without slowing teams to human-only review.
- **Regulators & auditors** — portable, third-party-verifiable evidence of *who built what,
  from what instruction, approved by whom.*
- **Open-source maintainers** — an N-of-M gate for untrusted or AI-generated contributions,
  and **reputation projected purely from signed attestations** — standing earned from
  verifiable work, not asserted in a side database.
- **Sovereign & regulated deployments** — air-gapped, single-binary, offline-verifiable.
- **The ecosystem** — a neutral standard *and* a reference implementation that any agent
  base or forge can plug into, instead of N incompatible closed platforms.

---

## Ecosystem & standards

OpenFab is deliberately a **thin, neutral integration layer** over mature open standards —
so adopting it is additive, not a migration:

**Implements / composes:** SLSA · in-toto · Sigstore · C2PA · DID · SPDX / Syft · OPA.
**Plugs into:** agent bases (AgentScope · HiClaw · agent-chat · OpenHands) and forges
(GitHub · Forgejo · Gitea · GitCode).

The seams are the invitation. The ecosystem grows by contributing **Base adapters** (new
agent runtimes), **Forge adapters** (new git hosts), **policies** (Rego trust rules), and
**conformance profiles** — each behind a stable interface, none requiring changes to the
trusted Core.

---

## Governance

OpenFab is stewarded by the **AOSF (aosf.ai)** as a **neutral foundation**, and licensed
**Apache-2.0**. Neutrality is a feature, not an afterthought: because the base and the forge
are swappable, **no single vendor's agent, forge, or license can capture the project** — the
guarantee OpenFab makes about trustworthy software is the same regardless of whose
infrastructure runs underneath it.

---

## Status & how to engage

A working **reference implementation (v0.2, in Rust)** runs today: the spec-cycle engine,
the `openfab/generation` predicate, `did:key` signing + verification, the N-of-M trust gate,
conformance, reputation, a full web UI, the swappable base/forge matrix, and one-click
reproduce/verify. Each lighter v0.1/v0.2 choice (did:key, gated-host sandbox, SPDX-lite)
names its production-grade swap (Sigstore, Podman/gVisor, Syft) — nothing is overstated.

- **Try it:** `git clone` → `demo/run_web_demo.sh` → open the web UI.
- **Read it:** [`README.md`](../README.md) · [`OpenFab_MVP_Design_and_PRD.md`](OpenFab_MVP_Design_and_PRD.md)
- **Build on it:** add a Base adapter, a Forge adapter, a policy, or a conformance profile.
- **Web:** [open-fab.ai](https://open-fab.ai) · **Code:** [github.com/open-fab-ai/openfab](https://github.com/open-fab-ai/openfab)

> Artifacts are cheap. The durable asset a fab produces is **trust** — process, decision
> memory, and signed provenance. OpenFab makes that asset open, neutral, and portable.
