# Self-hosting — OpenFab develops OpenFab (PRD §6)

> *"OpenFab is built to eventually build itself — like a self-hosting compiler. New
> capabilities are added by OpenFab, each with its own provenance."* — PRD §6

Yes — OpenFab reaches the stage where its **own development is done through OpenFab, and
every action is logged**. This page shows it running and explains the guardrails.

## Run it

```bash
demo/run_selfhost_demo.sh           # base = claude (the LLM); or pass a framework base
```

It clones OpenFab's own source into `demo/.work/selfhost`, then points `openfab` at that
clone to **implement a change to OpenFab itself**. The change is verified with the
project's **own checks** — `cargo build` + `cargo test` — in the sandbox, signed with
provenance, and the merge is **gated on N-of-M human sign-off**. Then browse it:

```bash
openfab serve --repo demo/.work/selfhost     # the self-development run, in the UI
```

## What you see (and what it proves)

```
import openfab source (seed)
   → feat(openfab-selfdev): OpenFab adds a capability to OpenFab    [Co-Authored-By: agent DID]
        acceptance in sandbox: cargo build ✓ · cargo test ✓        ← OpenFab's OWN checks
        signed in-toto/SLSA attestation (openfab/generation)
   → chore: sign-off by alice        [OpenFab-Signoff: did:key:…]
   → chore: sign-off by bob          [OpenFab-Signoff: did:key:…]
   → merge (OpenFab gate accepted)                                 ← N-of-M satisfied
```

Every step is a **signed git commit with provenance trailers** (`Spec`, `Co-Authored-By`
agent DID, `OpenFab-Base`, `OpenFab-Attestation`, `OpenFab-Acceptance`, `OpenFab-Signoff`)
— the same auditable trail as any other run, viewable in the UI's **Audit trail** tab or
with third-party tools (`git log`, a forge web UI, `jq` on the attestation).

The "feature" in the demo is a real integration test added to OpenFab's own suite that
exercises Core through the public library API (`use openfab::core::...`) — so it genuinely
tests the system, verified by `cargo test` (not a vacuous addition, R14). Evolving the
*trusted core* itself follows the identical gated path, just with deeper review.

## Where this sits (PRD §6 phases)

| Phase | What | Status here |
|---|---|---|
| 0 — stage-0 | Hand-build Core, ports, one base + forge, spec engine, provenance, CLI | ✅ done (this repo) |
| 1 — self-hosting | Point OpenFab at its own repo; implement the next feature *through* OpenFab, gated by humans | ✅ demonstrated (`run_selfhost_demo.sh`) |
| 2 — recursive growth | Every new capability specced + built by OpenFab, each with provenance | the path is open; same cycle |

## Guardrails (critical — PRD §6)

Self-generated changes must pass OpenFab's **own** trust model — this is non-negotiable:

1. **Machine acceptance** — the project's checks (`cargo build`/`test`/`clippy`) run in the
   sandbox; a failure keeps the gate blocked (no vacuous pass).
2. **Human sign-off / N-of-M** — maintainers approve OpenFab's changes to itself. **No
   self-merge**, even when OpenFab is building OpenFab.
3. **Signed provenance + reputation** — every self-change is attributed and auditable.

The single most sensitive component is the **trust gate / signing core** (the code that
decides whether to accept a change). It may be self-developed, but only under the strictest
human / N-of-M control — **always versioned, never hot-loaded, never self-approved**.
OpenFab cannot autonomously weaken its own gate (`policy.trust_gate_self_modifiable = false`).

## Payoff

OpenFab is its own first user: its provenance trail is the reference showcase, the
spec-cycle and trust model are stress-tested on a real, complex codebase (its own), and
every capability it grows is auditable and gated.
