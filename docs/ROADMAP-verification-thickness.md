# Roadmap — OpenFab verification thickness (PPT S11/S14 pillars)

Scope boundary (confirmed): **OpenFab = the factory backbone** (spec · verify · sign · gate ·
AI-BOM · orchestration). **matrix-Agent = the agent-chat execution layer**, *driven* by OpenFab
— its agent pooling / capability scheduling / 6-role richness is agent-chat's concern, not
OpenFab's. So this plan only thickens what OpenFab itself owns: **the verify stage**.

Against the PPT, OpenFab's main line (spec SSOT · tests-as-verifier · human gate · AI-BOM出证 ·
NL→artifact) is already satisfied. The two real gaps, both "verification thickness":

- **A. Layered QA + mutation/fuzz/coverage** (S11/S14 pillar 1) — verify runs the bound tests once.
- **B. Cross-model adversarial validation as a built-in verify stage** (S14 pillar 2) — only a
  single-reviewer caller-mode hook exists.

## Design principles
1. OpenFab only thickens **verify** — produce **signed evidence** + feed **conformance/gate**.
2. **Honest skip**: if a tool isn't installed, record `skipped` (≠ pass). Never fake green.
3. **spec-driven + TDD**, landed into the PR exactly like this session's work.
4. Everything new is **signed into the provenance predicate** and gate-checked, so it's auditable.

---

## Feature A — Layered QA (coverage / mutation / fuzz)

### A.1 QA tier model
`QaTier = Fast | Full | Deep | Nightly`, each additive:
- **Fast** = bound BDD tests (today's behaviour; default).
- **Full** = + coverage (threshold gate).
- **Deep** = + mutation (mutation-score gate).
- **Nightly** = + fuzz (time-budgeted).

Selection: `OPENFAB_QA=fast|full|deep|nightly` (default `fast`, backward-compatible) or a
`policy.qa.tier` field.

### A.2 Language-agnostic QA adapter — `src/adapters/qa.rs`
Detect the toolchain, run the tier's checks, parse results:
- Rust: coverage `cargo llvm-cov` / `cargo tarpaulin`; mutation `cargo-mutants`; fuzz `cargo-fuzz`.
- Python: `pytest --cov`; `mutmut`; `atheris`.
- Tool absent → `QaOutcome { tool, status: Skipped, reason }` (honest).
- `QaReport { tier, coverage_pct, mutation_score, fuzz_findings, outcomes[] }`.

### A.3 Wire into `spec_cycle`
After the agent-spec lifecycle verify, run the tier's QA → `QaReport`. Thresholds from policy
(`qa.min_coverage`, `qa.min_mutation`). On a Full+ tier, below-threshold blocks like a failed
acceptance (start warn-only behind a flag, then enforce).

### A.4 Signed evidence + conformance
- `GenerationInput` gains `qa_report` (signed, tamper-evident).
- New **C13 = QA tier satisfied** (coverage met when tier≥Full; mutation met when tier≥Deep).
- Surface QA results in the timeline + console.

### A.5 Contract + tests (TDD)
- `specs/phase3/qa-layered.spec.md`: tier selection, threshold gate, honest-skip, provenance.
- Pure unit tests: tier parse, threshold decision, `skipped ≠ pass`.

### Effort: medium. **First slice = Fast + Full (coverage gate) + C13 + honest-skip**; leave
Deep/Nightly (mutation/fuzz) as honest-skip stubs to wire later.

---

## Feature B — Cross-model adversarial validation (built-in verify stage)

### B.1 From single reviewer → a model-family panel
- Today: caller-mode → one `BRIDGE_REVIEWER`.
- Extend: `OPENFAB_REVIEW_PANEL=wf_reviewer:claude,wf_final_reviewer:codex` (role:model-family).
- OpenFab dispatches an adversarial review (prompt: refute / find bugs against the spec) to
  **each model family**, independently collecting verdicts. Two families ≠ shared blind spots.

### B.2 Cross-model merge decision
- Configurable: **any family finds a real bug → block** (adversarial-strict, default), or a
  scenario must pass a **majority** of families.
- Record **per-family verdicts** (Claude=…, Codex=…) in the signed provenance.

### B.3 Signed evidence + conformance
- `GenerationInput` gains `cross_model_verdicts: [{ model_family, scenario, verdict }]` (signed).
- New **C14 = cross-model adversarial passed** (no family reports a blocking bug).

### B.4 Reuse what exists
- Reuse the caller-mode `/review` endpoint + `review_and_wait` + reviewer skill.
- Change: `/review` accepts `reviewers[]` and fans out; `review_and_wait` collects N decision
  sets; OpenFab merges per B.2.

### B.5 Contract + tests (TDD)
- `specs/phase3/cross-model-verify.spec.md`: fan-out, merge rule, verdict recording, C14.
- Pure unit tests: merge decision (any-block / majority-pass), provenance recording.

### Effort: medium; half the plumbing exists (caller-mode). Mostly fan-out + merge + conformance.

---

## Suggested order
1. **Feature A, first slice** — Fast+Full (coverage gate) + C13 + honest-skip + spec/tests.
   Most universal, best ROI.
2. **Feature B** — model-family panel fan-out + merge + C14 (reuses caller-mode).
3. **Feature A, second slice** — Deep/Nightly (mutation/fuzz) wired to real tools.

Each step: spec-driven + TDD + all quality gates green + appended to the PR — same cadence as
this session.

## Explicitly NOT in scope (it's matrix-Agent / agent-chat, not OpenFab)
Agent pooling, capability-tier scheduling, the 6-role matrix (Architect/Coding/Testing/Review/
Integration/Documentation). OpenFab's responsibility there is only to *drive* the team — already
done (Bridge dispatch, workspace mode, caller-mode review). Enrich those inside agent-chat.
