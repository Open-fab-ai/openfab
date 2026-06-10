# ADR 0001 — OpenFab v0.1 MVP architecture decisions

**Status:** accepted (Phase-0 hand-built MVP) · **Date:** 2026-06-09

## Context

The PRD ([`OpenFab_MVP_Design_and_PRD.md`](../../OpenFab_MVP_Design_and_PRD.md)) asks
for a Rust fab that turns a spec into a signed, attributed, gated software product, on a
swappable base and forge. This build was done in one autonomous overnight session with a
hard requirement: a **runnable end-to-end demo** the next morning, on a local machine
with no cloud accounts wired up. The environment had: no Rust (installed via rustup), the
`claude` CLI available, **no** cosign/syft/slsa-verifier, docker present but daemon off.

The central tension: maximum fidelity to the PRD's production stack vs. a demo that
**actually runs** unattended. The decisions below optimize for *the integrated whole
working end-to-end and being honest about it*, with every lighter choice naming its
production swap.

## Decisions

1. **Language = Rust, per the PRD** (not a quicker Python/Node pivot). The Core types and
   ports (`BasePort`/`ForgePort`) are the moat and are defined in Rust as specified.

2. **Smallest dependency set that satisfies the spec** (AGENTS.md). We use pure-Rust,
   fast-compiling crates only (serde, clap, ed25519-dalek, sha2, bs58, …) and **shell out**
   to `git` and the agent base — both legitimate per the PRD (the Base Port is a
   cross-process boundary; the PRD endorses shelling to CLIs). We deliberately avoid the
   heavy crates (`sigstore`, `c2pa`, `didkit`, `regorus`, `git2`, `reqwest`, `tokio`) for
   v0.1 to guarantee an unattended compile, and document each as a production swap.

3. **Identity = did:key + ed25519, verifiable offline.** This is the PRD's identity choice
   and is self-contained: the public key is embedded in the DID, so attestations verify
   with no keystore and no transparency-log round-trip. **Swap:** Sigstore
   (cosign/fulcio/rekor) for the public transparency log.

4. **Trust policy = `policy/trust.json` read by an in-process evaluator**, with
   `policy/trust.rego` shipped as the illustrative production (OPA/`regorus`) form. The
   literal parameter values live in exactly one place — the JSON — so there is no
   duplication (R3). **Swap:** `regorus` evaluating the `.rego`.

5. **Sandbox = policy-gated host subprocess** confined to the task workdir (docker daemon
   was off). The gate refuses anything off the allowlist / on the denylist *before*
   execution. The runtime label is recorded truthfully so provenance never overstates
   isolation. **Swap:** Podman / gVisor.

6. **Two bases ship: `claude` (real LLM) and `mock` (deterministic).** The mock is a
   genuine alternate base that writes working source from a built-in library — it proves
   base-agnosticism and gives CI/air-gapped runs a network-free path. It is **not** a
   rubber-stamp: no built-in solution ⇒ honest failure, never a vacuous pass (R14).
   > **Superseded in v0.2 (see ADR 0002).** The `mock` base was **removed**: every
   > artifact (spec, acceptance, and code) must come from a real LLM, so a built-in
   > solution library could never silently strengthen the demo. Base-agnosticism is now
   > proven by `claude` + the four framework bases (`base_framework`), and the offline
   > path is the `bridged` runtime rather than a deterministic mock.

7. **Cross-forge proven with two local-git forges; GitHub adapter real but gated.**
   Portability is a property of the `ForgePort` seam + in-repo portable provenance, which
   two independent local forges demonstrate without touching anyone's account overnight.
   `forge_github.rs` is real and selectable with explicit env. **Swap:** live GitHub +
   Forgejo + Gitea.

8. **`loop.rs` → `spec_cycle.rs`.** `loop` is a Rust keyword and cannot name a module; the
   file is `spec_cycle.rs`, documented at the top.

## Consequences

- The demo runs end-to-end (~1 min with the `claude` base, network-dependent) on a bare
  laptop, exercising the real moat: spec cycle, `openfab/generation` predicate, signing,
  N-of-M gate, conformance, reputation, cross-forge, iteration.
- Nothing is faked or overstated; every reduction names its production-grade swap (see the
  table in the README).
- Migrating to the production stack is adapter/dependency work behind stable interfaces —
  the Core, the predicate, the spec-cycle, and the trust model do not change.

## Follow-ups (tracked in HANDOFF.md)

- Split `spec_cycle.rs` (~330 lines, over the 300-line budget) in its own refactor session
  (R4/R8).
- Add a `BasePort::events()` live-feedback stream for the HiClaw/Matrix adapter.
- Wire Sigstore + Syft + Podman behind the existing seams.
