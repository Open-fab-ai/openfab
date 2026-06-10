# Kickoff — prompting a coding agent to build OpenFab

Put `OpenFab_MVP_Design_and_PRD.md`, `AGENTS.md`, and `init.sh` in the repo root first. Then start the agent (Claude Code / Codex) in the repo and paste the **first-session prompt** below.

## First-session prompt (paste verbatim)

> You are building OpenFab in this repository, in Rust.
>
> 1. Read `OpenFab_MVP_Design_and_PRD.md` (the source of truth) and `AGENTS.md` (how to work here) in full before writing any code. Do not skim.
> 2. Run `./init.sh` and confirm the toolchain is ready.
> 3. We build strictly in the PRD **§7 build order**, one step per branch/PR. Do **build order step 1 now**: scaffold the Rust workspace — `Cargo.toml`, `rust-toolchain.toml`, the module layout from PRD §7 (`src/core/`, `src/ports/`, `src/adapters/`, `src/loop.rs`, `src/cli.rs`, `src/main.rs`), and define the `BasePort` and `ForgePort` **traits** plus the core types (`TaskCard`, `Spec`, `Attestation`, `RunResult`) as the PRD describes. Stub the adapters so it compiles. **No business logic yet — just a skeleton that builds.**
> 4. After the change, run the verify-on-edit checks from `AGENTS.md`: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`. All must pass; fix until green.
> 5. Honor the golden rules and safety boundaries in `AGENTS.md`: `core/` depends on no base/forge (only via the ports); reuse crates over new code; keep it simple; never commit secrets; **do not self-merge**.
> 6. When step 1 is green, summarize what you built, list open questions, write a short handoff note, and **STOP for my review**. Do not start step 2 without my go-ahead.

## Every later session (cadence)

> Read `AGENTS.md` and the relevant PRD section, run `./init.sh`, then take the **next** PRD §7 build-order step only. Write the trait/spec first, then the implementation, then tests. Keep the PR small. Run the three verify-on-edit checks until green. End by summarizing done / next / decisions, and stop for sign-off.

## Why this shape
- The agent gets its **boundaries and conventions** from `AGENTS.md`, its **spec** from the PRD, and a **deterministic env** from `init.sh` — that is the harness (§8 of the PRD).
- One step per session keeps changes small, reviewable, and signable, and keeps the agent inside its context window.
- The **human sign-off gate** at the end of each step is the same gate that later governs OpenFab building OpenFab (Phase 1+).
