# HANDOFF — OpenFab v0.2 (web UI + full base/forge matrix)

**For review.** v0.1 was the CLI engine + provenance/trust moat. v0.2 adds the three
things requested: a **web UI**, the **whole base/forge matrix** (5 bases, 4 forges), and
a **full visual UX** (NL → live workflow → approve/refine → product + provenance →
reproduce). Every artifact (spec, acceptance, code) is authored by the LLM base — the
v0.1 `mock` base was removed. Everything below is real and runs.

## Do this first (≈2 minutes)

```bash
cd <this repo>
./init.sh                  # toolchain check (Rust was installed via rustup)
demo/run_web_demo.sh       # builds + serves the UI → open http://127.0.0.1:8787
```

In the browser: intent → pick base (×5) + forge (×4) → **Fabricate** → watch the live
workflow → gate **BLOCKS** → **Run the app** → sign off as alice & bob → merges → inspect
software + provenance → **Reproduce & verify** → **Refine** to iterate. Walkthrough:
[`docs/WEB.md`](WEB.md). The scriptable CLI demo still works: `demo/run_demo.sh`
(claude) or `demo/run_demo.sh <framework-base>`.

## Done (v0.2 additions on top of v0.1)

- **Web UI + JSON API** — `openfab serve` (tiny_http). The whole SPA is `include_str!`'d
  into the binary (single static binary, sovereign). Live workflow via event-stream
  polling; background run threads; per-repo serialization.
- **5 swappable bases** — `claude` (native LLM) and `agentscope · hiclaw · agent-chat ·
  openhands` via one parameterized adapter (`base_framework`). Each runs **native** if its
  `OPENFAB_*_URL` is set, else **bridged** through `llm_backend` (claude CLI, or
  Qwen/DashScope via `OPENFAB_LLM=dashscope`). The run badges its runtime honestly (R14)
  and provenance records `base` + `runtime`. There is no mock — every artifact comes from
  a real LLM.
- **4-forge matrix** — `github · forgejo · gitea · gitcode`. Real adapters
  (`forge_github` via `gh`, `forge_rest` for the Gitea-lineage three) when creds are set,
  else an offline **local instance** that still proves portable in-repo provenance.
- **One-click reproduce/verify** — re-verifies the signature, confirms the committed
  source is **bit-identical** to the signed digests, and **re-runs every acceptance check**
  in the sandbox → a single REPRODUCIBLE verdict (the sovereign/air-gapped proof).
- **Clickable workflow steps + Audit-trail view** — each step (Spec/Generate/Verify/Sign/
  Gate) opens its artifact; an Audit tab renders the live git commit graph with provenance
  trailers + the third-party verify commands (`git`/`jq`/`cosign`/`slsa-verifier`).
- **Self-hosting (PRD §6)** — `demo/run_selfhost_demo.sh`: OpenFab clones its own source
  and implements a change to *itself*, verified by `cargo build`/`test` in the sandbox,
  signed, and gated on N-of-M sign-off. Enabled by splitting the crate into a `lib` +
  thin `bin` so self-development exercises the real API. See `docs/SELF_HOSTING.md`.
- **Shared `ops` layer** — CLI and API call the same `start_run`/`signoff`/`verify`/
  `feedback`/`reproduce`/`artifacts` functions, so there's one orchestration path (R3).
- **Quality gate green:** `cargo fmt --all --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo test` (**41/41**). UI verified in a real browser
  (live workflow, run-the-app, sign-off→merge, provenance, reproduce all confirmed).

## Honesty model for the matrix (decided with you, R14)

The four framework bases and three non-GitHub forges are **real adapters**, but standing
up their external servers wasn't in scope, so by default they run in clearly-badged
fallback modes: framework bases → **bridged** (the task runs via OpenFab's LLM backend,
labelled bridged in the UI *and* in the provenance `runtime` field); forges → **local
instance** (a real local git repo, labelled as such). Connect a native runtime / live
forge via its env vars and the same run flips to `native` / `live` with no code change.
Nothing claims to be what it isn't.

## Key decisions (full rationale in [ADR 0001](adr/0001-mvp-architecture-decisions.md))

Both versions implement the moat with the **smallest dependency set that satisfies the
spec** and shell out to `git`/the base/`curl`, naming each lighter choice's production
swap: did:key/ed25519 (→ Sigstore), in-process policy over `trust.json` (→ OPA/regorus),
policy-gated host sandbox (→ Podman/gVisor), SPDX-lite SBOM (→ Syft), local forge
instances (→ live forges), acceptance-re-run reproducibility (→ Nix bit-identical builds).
The Core, predicate, spec-cycle, and trust model don't change when swapping these in.

## Open questions / decisions for you

1. **Stand up a native base?** e.g. run OpenHands (Docker) or a Matrix server for HiClaw
   so one base is genuinely `native` end-to-end. The adapter seam is ready.
2. **Production swaps priority** — Sigstore (cosign/rekor), Podman sandbox, or a live
   forge first?
3. **Demo app** — the temperature converter keeps the *fab* the star; want a meatier
   target (small REST API) for the showcase?
4. **N-of-M default** is 2-of-2 (`policy/trust.json`) — confirm the real policy.

## Known gaps / next steps (do NOT bundle with a feature — R8)

- **File-size budget (R4).** Over 300 lines (incl. doc-comments + `#[cfg(test)]`):
  `spec_cycle.rs` (514), `cli.rs` (387), `provenance.rs` (376), `ops.rs` (373),
  `trust.rs` (362), `server.rs` (346), `llm_backend.rs` (343), `base_framework.rs` (316),
  `runstate.rs` (302). Split each in its own refactor session before extending it.
- The native framework dispatch (`base_framework::dispatch_native`) and the live
  `forge_rest` PR path are implemented but **untested against real servers** (no instances
  in this env). Verify when a native runtime / live forge is connected.
- `BasePort::events()` (live inbound human-feedback stream, Matrix when base=HiClaw) is
  not a trait method — feedback enters via the API/CLI. Add when wiring a live HiClaw.
- No Rekor transparency log / Nix bit-identical build yet (later phases).
- The web UI is ~614 lines of vanilla JS/CSS/HTML embedded via `include_str!`; no test
  harness for it yet (verified manually in-browser). A Playwright smoke test is the
  natural follow-up.

## R13 — fresh-session review before merge

This was one building session. Per R13, a second session with no history should review
the diff against R1–R14 before this is considered "accepted":

```
git -C <repo> add -A && git -C <repo> diff --staged > /tmp/openfab.diff
# new session: "Review this diff against the project engineering standards (R1–R14).
#               Flag duplication, silent error-swallowing, dishonest baselines, file-budget."
```

The same human sign-off gate this code implements is the gate that should govern merging
this code. No self-merge.
