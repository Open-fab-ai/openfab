# OpenFab — Roadmap / TODO

Post-v0.1 ideas surfaced during design review. Not yet built; captured so they aren't lost.

---

## 1. Serverless "concept demo" — browser-only, no server (GitHub Pages)

**Goal:** demo OpenFab anywhere with **no server component** — a static web app that only
needs an LLM-provider setting (OpenAI-compatible key/URL, incl. Ollama Cloud). Simulates a
multi-agent swarm and produces a real signed AI-BOM, all client-side.

**Feasibility (honest breakdown):**

| Capability | Browser-only? | How |
|---|---|---|
| Multi-agent **swarm simulation** | ✅ easy | call the LLM provider's `/v1/chat/completions` directly from JS; run N "agents" (planner / coder / reviewer) as separate prompted calls |
| **Spec + acceptance** authoring | ✅ | one LLM call from the browser |
| **in-toto / SLSA signing** (did:key, ed25519, sha256) | ✅ | WebCrypto (`crypto.subtle`) + a small ed25519 lib (noble-ed25519); generate did:key in-browser |
| **Tamper-evident verify** (hash + signature) | ✅ | recompute sha256 + verify ed25519 entirely client-side — the §6 "tamper a byte → ✗" beat works in-browser |
| **Run acceptance checks** (shell: python3/bash/grep) | ⚠️ **the hard part** | a browser has no shell. Options: (a) limit targets to JS/HTML and check in-browser; (b) **Pyodide/WASM** to run python checks (heavy but real); (c) clearly **label execution as "simulated"** — but that's vacuous (R14), so only as an explicit, labeled concept stub |
| **Forge push** | ◑ | can't `git push` from a static page, but the **GitHub API (octokit)** + a token can commit the artifact; or just offer a **download** of `att.json` + source |

**Recommended scope (strong + honest):** simulate the swarm (real LLM calls) + **real**
in-toto/SLSA signing + **real** in-browser tamper/signature verification. The *only* part
that can't be genuine without a runtime is **executing** the acceptance checks — either use
Pyodide for python targets, or restrict the concept demo to JS/web targets checkable in the
browser. **Never** fake a passing check (R14 — empty/simulated success is failure).

**Effort:** ~a few hours of frontend (no Rust). Reuses the existing UI/UX; swaps the
`/api/*` server calls for client-side LLM + WebCrypto. Good "demo it on a plane" artifact.

---

## 2. Lineage chaining in the attestation

Embed `parent_attestation_sha256` in the generation predicate so a release **cryptographically
links to the version it refined** (provable v1→v2→v3 chain), instead of lineage living only
in local run-state (`parent_run`). Lightweight; high value for audit.

## 3. "Use the repo's existing tests as the contract" mode

The realistic-adoption path (see VALUE_PROPOSITION §7.5): instead of OpenFab authoring a
spec, attach to an **existing repo + existing test suite**, run *those* as the acceptance
contract, record the AI-BOM for the diff, sign conformance. No spec authored by anyone.
This is the OSPO "gate inbound contributions" use case.

## 4. Behavioral approval as a first-class signed event

Today the human gate = N-of-M maintainer sign-off over the artifact hash. Add a signed
record of the **behavioral** approval (VALUE_PROPOSITION §7.6): "maintainer X viewed build
Y's running output and approved," so the human's behavioral "yes" is itself notarized.

## 5. OpenFab shows the live swarm

Stream agent-chat's live agent activity into OpenFab's own timeline (today you watch the
swarm on the agent-chat dashboard :8084; OpenFab shows a step timeline). Backend relay +
frontend stream.

## 6. AI-BOM split-hash

Hash the **intent** and the **acceptance contract** as separate predicate fields rather than
one blob that also contains output-contract boilerplate — cleaner disclosure, lets you prove
"same intent, different checks."
