# AGENTS.md — working in the OpenFab repo

OpenFab is an open-source software **fab**: natural language in, trustworthy software out. It composes mature OSS + open standards (SLSA / in-toto / Sigstore / C2PA / DID) into a **reproducible, auditable, cross-forge** fab running on a **swappable agent base**. **Read `docs/OpenFab_MVP_Design_and_PRD.md` first — it is the source of truth.** This file is how to *work* in the repo.

## Golden rules
- **Base-agnostic.** Core never calls a specific base (AgentScope / HiClaw / agent-chat / OpenHands) directly — only through `ports::BasePort`. Same for git hosts via `ports::ForgePort`. No Matrix/AgentScope types leak into `core/`.
- **Core is the moat, and base-independent.** `core/` must not import or assume any base or forge.
- **Reuse the primitives; build the whole.** Prefer a well-maintained crate or CLI over new code. The novelty is the integrated fab, not the building blocks.
- **Simple · modular · plug-and-play.** Smallest design that satisfies the spec. Define the trait/interface before the implementation.
- **Everything traceable.** Every change is driven by a spec and ends in a signed attestation. No silent scope creep.

## Stack
Rust (stable, edition 2021). Core + CLI in Rust; shell out to language-agnostic CLIs (`cosign`, `syft`, `slsa-verifier`) and to the agent base. Native crates of choice: `sigstore` (signing/verify), `c2pa` (AI-content provenance), `didkit`/`ssi` (did:key), `regorus` (Rego policy, in-process), `git2` (git), `reqwest` + `serde`/`serde_yaml` (forge REST + spec), `clap` (CLI), `tokio` (async). Pick the simplest that works; justify any new dependency.

## Build / test / verify — run after EVERY change (verify-on-edit)
```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
All three must be green before a change is "done". CI runs the same. If a check fails, fix it — never weaken the check, the signing, or the trust gate to make it pass.

## Layout (see PRD §7)
`src/core/` (spec, provenance, identity, trust, reputation, conformance) · `src/ports/` (base, forge) · `src/adapters/` (base_*, forge_*) · `src/loop.rs` (spec-cycle) · `src/cli.rs` + `src/main.rs`. Specs in `specs/`, JSON Schemas + the `openfab/generation` in-toto predicate in `schemas/`, Rego in `policy/`.

## Work cadence
- The PRD **§7 build order is the feature list** — do one step per branch/PR, in order.
- Start each task by writing/adjusting the **trait or the spec**, then the impl, then tests.
- Keep PRs small and reviewable; write a one-paragraph handoff note (done / next / decisions) at the end of each session so the next session has context.

## Safety / boundaries (non-negotiable)
- Run generated or untrusted code only via the sandbox (`run_sandboxed`) — never on the host.
- Never commit secrets, keys, or tokens. Read them from the environment.
- Do not add a dependency without a one-line justification in the PR.
- **No self-merge.** A change is "accepted" only after human sign-off (N-of-M per `policy/trust.rego`). This holds even when OpenFab is building OpenFab.
- When a build-order step is green, summarize, list open questions, and **stop for human review** — do not start the next step without a go-ahead.
