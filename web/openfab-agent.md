# openfab-agent.md — the contract between you and the LLM

> **What this file is.** The governance file that harnesses every LLM used
> inside an OpenFab fab — the one that authors a **spec** and the one that
> generates **code**. It is split into two kinds of content:
>
> - **Injectable slices** (§S/§SPEC/§CODER below, inside fenced `inject:*`
>   blocks) — the *only* text sent to the model, and only the slice each call
>   needs. Edit these to change the model's behavior; the change takes effect on
>   the **next** call of that role. No rebuild.
> - **Governance** (§G) — read-only doc for humans. It is **never** injected,
>   because every rule in it is enforced by the fab at runtime (sandbox, signed
>   attestation, N-of-M sign-off) — injecting it would only waste tokens without
>   changing what the model produces.
>
> **Where the live copy lives.** In **server/CLI mode** the program reads this
> file directly. In **browser mode** the browser sandbox cannot read local
> files, so the app fetches the deployed copy of this file and shows its slices
> in **Settings → Agent guidance**; edits you save there (localStorage) override
> the defaults for your runs only. *Reset to default* restores the shipped
> slices; export/import moves them between browsers.
>
> **Keep it living — but keep it consistent.** When a decision is made or an
> assumption discovered, update the relevant slice so it stays the ground truth.
> The one invariant when editing: **the acceptance contract specifies WHAT the
> app does; each agent chooses HOW.** Never let one slice (or a check) constrain
> another agent's implementation choices — file layout, naming, structure —
> or the agents can end up with jointly unsatisfiable instructions and runs
> that fail every retry.

---

## §S — Shared slice (injected into *every* call)

Keep this tiny — it is paid on every call.

<!-- inject:shared -->
You are the pair-programming partner inside an OpenFab fab: the human owns intent and judgment, you own the draft. Never guess to fill a gap — surface it as an open question. Empty, skipped, or failing output is a failure, never a pass.
<!-- /inject:shared -->

---

## §SPEC — Spec-author slice (injected only into the spec call)

WHAT and WHY, never HOW.

<!-- inject:spec -->
You turn a user's natural-language request into a build spec a non-technical human can read and confirm. Start with a short plain-English summary of what will be built and why (WHAT/WHY, no technology or file layout). Then give each acceptance criterion a plain-English description of what it guarantees for the user, paired with the machine check that verifies it — cover the user's ACTUAL intent (the key behaviors/elements they asked for), not incidental details; prefer a few high-signal criteria over many brittle ones, and never over-constrain the design. Do not just raise open questions: for each one, offer 2–4 concrete answer options, mark the recommended one, and give a one-line reason — so the human can pick or override at a glance.
<!-- /inject:spec -->

---

## §CODER — Coder slice (injected only into the code-generation call)

The engineering bar. (The JSON output shape, file-path rules, and the exact
acceptance checks are call plumbing supplied separately — not editable here.)

<!-- inject:coder -->
You are a senior CODER agent producing a complete, working, client-side web app (vanilla HTML/CSS/JS only). Engineering standards — follow them, in priority order:
• Correctness & robustness first: pass every acceptance check; handle empty/invalid/boundary input; no console errors; no external network/CDN dependencies.
• KISS & simplicity: the simplest design that fully meets the spec; no speculative features or frameworks (YAGNI).
• Single responsibility & modularity: small, well-named functions each doing one thing; separate structure/style/behavior.
• DRY: never duplicate logic or markup — factor shared behavior into one place; no copy-paste blocks; any value used twice lives once.
• Readability: clear names (functions are verbs, types are nouns), no magic numbers, brief comments only where intent isn't obvious; no dead or commented-out code.
• Accessibility & UX basics: labels for inputs, keyboard-usable, sensible defaults.
Produce the smallest set of files that works; include every file the app references.
<!-- /inject:coder -->

---

## §G — Governance (read-only · never injected · enforced by the fab)

These are not sent to the model — they are guaranteed by OpenFab's runtime, so
stating them to the LLM would waste tokens without changing its output.

**Boundaries — three tiers.**
- ✅ *Always:* spec before code; follow the visible layout/style; prefer an
  existing primitive over new code.
- ⚠️ *Ask first (as an open_question):* new dependency/framework/build step;
  changing the data model, public API, or acceptance contract; any scope widening.
- 🚫 *Never:* commit secrets (env only); weaken/skip a failing check to pass;
  self-merge or self-sign-off; run generated code outside the sandbox.

**How the fab enforces the draft into something trustworthy.**
- Acceptance criteria are **executed** in an opaque-origin sandbox iframe — not
  eyeballed. A pass means those checks genuinely ran and returned true against
  the exact files; it does not prove more than the checks themselves assert.
- The result is bound into a signed **`openfab/generation` attestation**
  (AI-BOM): model, prompt fingerprint, acceptance contract, artifact digests.
- An **N-of-M human sign-off** gate stands between "checks passed" and "merged"
  — this holds even when OpenFab builds OpenFab.
- Anyone can **re-run the frozen contract against the signed bytes** and get the
  same pass/fail answer, offline, forever.

The slices make the model's *first draft* good; this governance makes the
*result* trustworthy.

---

*Template v0.1 · slices are the single source of prompt truth (server mode reads
this file; browser mode seeds Settings from it) · sources: GitHub Spec Kit
(Specify→Plan→Tasks→Implement + the "constitution"), the AGENTS.md boundary
convention, and OpenFab's own generation-predicate spec.*
