# OpenFab vs. existing tools — the honest delta

> The skeptic's companion to [`OpenFab_Overview.md`](OpenFab_Overview.md) (full
> problem/vision narrative) and [`VALUE_PROPS.md`](VALUE_PROPS.md) (demo
> evidence). This doc answers the three questions those don't tackle head-on:
> **what exactly can GitHub not do, and how is OpenFab consistent under
> scrutiny?** Written to survive a skeptical read — including an honest list of
> what OpenFab does *not* solve.

---

## 0. The honest one-liner

OpenFab does **not** reproduce AI-generated code from a prompt — that is
impossible, and OpenFab does not claim it. What it produces is **one signed,
portable attestation that binds four things together**:

```
   exact artifact bytes
 ⊗ how it was generated   (human vs AI, which model, which prompt)
 ⊗ an executed, machine-checkable acceptance contract
 ⊗ N-of-M human sign-off
```

The thing that is **reproducible is the *verification*** — anyone can re-run the
**frozen, signed acceptance contract** against the **signed artifact bytes** and
get the same pass/fail answer, forever, offline. **Not** the bytes, **not** the
generation, **not** the authoring of the contract. (See §5.0 — nothing involving
the LLM is reproducible, and OpenFab does not need it to be.)

---

## 1. Problem statement (recap)

> AI now writes a large and growing share of code, but there is no portable,
> tamper-evident, vendor-neutral way to know **how** a given artifact was
> generated, **whether** it provably meets its contract, and **who** actually
> approved it — and today's signals (GitHub identity, CI, branch protection) are
> siloed, platform-locked, and silent on AI provenance.

*(Full narrative in [`OpenFab_Overview.md`](OpenFab_Overview.md). Compact form
here so §4's mechanism mapping is self-contained.)* Broken into items:

| # | Problem | Who feels it |
|---|---|---|
| P1 | You can't tell if code was written by a human or an agent — and if an agent, which model and from what instruction. | Auditors, regulators (EU AI Act), enterprises, license/IP review |
| P2 | "Definition of done" is a vendor log (CI) that expires and isn't bound to the bytes — you can't prove offline that an artifact met its contract. | Anyone consuming software they didn't build |
| P3 | Approval gates are platform settings an admin can silently disable; they prove nothing to an outsider who distrusts the platform. | Downstream consumers, supply-chain security |
| P4 | All of the above is locked to one forge. Leave GitHub and your trust evidence dies. | Anyone wanting forge independence / sovereignty |

---

## 2. What GitHub already does (conceded honestly)

OpenFab is a **trust layer on top of** a forge, not a replacement. GitHub does
more than people think, and pretending otherwise would be dishonest:

- ✅ Committer identity (GPG / SSH / Sigstore signing)
- ✅ Public diff history = a real audit trail
- ✅ Required reviewers / branch protection
- ✅ CI that runs tests
- ✅ Build provenance — **GitHub Artifact Attestations** (SLSA via Sigstore)

The mistake is to stop there and conclude "so GitHub already does this." It
doesn't — the gaps are specific.

---

## 3. The genuine gaps (item by item)

| Capability | GitHub today | Gap OpenFab fills |
|---|---|---|
| Committer identity | ✅ signing | overlaps — OpenFab reuses the same open primitives (did:key / Sigstore) |
| Public diff = audit trail | ✅ | overlaps |
| Required reviewers | ✅ | **platform setting an admin can silently disable**; not cryptographically bound to the artifact; unverifiable by an outsider |
| CI runs tests | ✅ | results are **vendor logs that expire**, not a signed, portable part of provenance |
| Build provenance (SLSA) | ✅ Artifact Attestations | covers *"CI built X from source Y."* Says **nothing** about whether the *source* was human- or AI-written, which model, which prompt |
| **Generation provenance (AI BOM)** | ❌ no schema | **the core gap** — human/AI authorship per range, model, prompt hash, bound to the bytes |
| **Acceptance contract as a signed artifact** | ❌ | "this artifact provably passed *this* definition-of-done," checkable offline, forever |
| **Forge-independent cryptographic gate** | ❌ | "2 named humans signed *these exact bytes*," provable to a third party who trusts neither admin nor forge |
| **Portability across forges** | ❌ GitHub-locked | same attestation verifies on Gitea / Forgejo / GitCode / offline |

**Bottom line:** GitHub gives four *separate, platform-locked, partly-revocable*
signals. OpenFab gives **one signed, portable, offline-verifiable bundle** that
also includes the one signal GitHub has no schema for: **AI generation
provenance.**

---

## 4. How OpenFab addresses each problem (mechanism)

| Problem | Mechanism | Open standard |
|---|---|---|
| P1 generation provenance | `openfab/generation` predicate: agent base, model, `prompt_sha256`, per-file/range human-vs-agent authorship — embedded in the signed attestation | in-toto predicate |
| P2 contract not bound to bytes | The spec's **acceptance contract** (machine-checkable shell checks) is executed in a gated sandbox; pass/fail is signed into the attestation and re-runnable from scratch (`reproduce`) | in-toto / SLSA |
| P3 revocable, unprovable gate | **Trust gate**: N-of-M maintainer `did:key` signatures over the attestation + a policy allowlist — a cryptographic, offline-verifiable approval | DID / Sigstore |
| P4 forge lock-in | Forge-agnostic attestation + swappable `ForgePort` adapters (GitHub / Forgejo / Gitea / GitCode / local) | in-toto / SLSA / DID |

---

## 5.0 Two kinds of non-determinism (and why neither breaks the claim)

The word "reproducible" only ever refers to **checking**, never **generating**:

| | Non-deterministic? | Part of OpenFab's guarantee? |
|---|---|---|
| **Authoring** the contract (LLM writes the checks) | Yes — different model/run → different checks; contract quality depends on its author | No |
| **Generating** the code (LLM writes the source) | Yes — same prompt+model can differ run-to-run, even at temp 0 | No |
| **Checking** the frozen contract against the signed bytes | **No — pure shell, deterministic** | **Yes — this is the reproducible unit** |

Once the contract is authored and signed it is a static list of shell commands
(`test -f …`, `python3 … | grep -Fxq …`). Re-running it involves **no LLM** and
yields the same result every time. The LLM's role ends at signing time; after
that the contract is just code. **OpenFab never claims to reproduce an LLM** — it
claims you can re-verify a *fixed artifact* against a *fixed contract*, forever.

## 5. The three sharp objections, answered

### 5.1 "Signing is PR/release-level, prompts are dev-level — inconsistent?"

No — they are **evidence vs notarization**:

- **Dev level (draft runs)** = the *evidence*: each generation records spec,
  model, prompt hash, per-file authorship, acceptance results.
- **Release level (promote)** = the *notarization*: the signature is taken over
  an attestation that **embeds that evidence for the bytes that actually
  shipped.**

The prompt record is the *input* to the release signature, not a parallel track.
Discarded intermediate prompts are git history, not part of the signed bundle —
only the recipe that produced the final committed bytes is attested.

### 5.2 "Same prompt → different code on different LLMs. So what's the prompt record for?"

Correct — the prompt hash is **useless for reproducing the code**, and was never
for that. Its three real uses:

1. **Disclosure / attribution** — an AI Bill of Materials for licensing, IP,
   liability, and trust decisions (regulators increasingly require this).
2. **Tamper-evidence** — `prompt_sha256` pins the claim so nobody can later
   forge a different, innocent prompt.
3. **Re-verifiable conformance (NOT regeneration).** You cannot re-run the
   prompt and expect passing code — LLM output is non-deterministic even at
   temperature 0, so a regeneration may *fail* the contract. What you *can* do is
   re-run the **frozen, signed contract** against the **already-signed artifact
   bytes** and deterministically confirm it still conforms — forever, offline, by
   anyone.

OpenFab moves reproducibility from *"same bytes / same generation"* (impossible
with LLMs) to *"anyone can re-verify that **this exact artifact** meets **this
exact contract**"* (deterministic, offline). The reproducible unit is the
**verification of a frozen artifact against a frozen contract** — neither the
prompt nor any regeneration is part of the guarantee.

### 5.3 "What can GitHub not do?"

See §3. The irreducible answer: **generation provenance (AI BOM)**, plus binding
the **acceptance contract** and a **forge-independent cryptographic gate** into
**one portable attestation** that verifies without trusting the forge or CI
vendor.

---

## 6. A concrete example — the audit GitHub can't satisfy

**Scenario.** A bank ships a payments microservice. **18 months later**, a
regulator opens an investigation into a fraud bug in its fee-rounding logic. The
bank must answer, for that exact deployed artifact:

1. Was the rounding code written by a human or an AI agent? Which model? From what
   instruction?
2. Did it actually pass its correctness checks — or was it merged green-washed?
3. Did a *responsible human* approve **this exact build**?

Meanwhile: the original engineer has left, the CI logs have rotated away, and the
bank migrated off GitHub to a self-hosted Forgejo last year.

### With GitHub alone

| Question | What GitHub can produce 18 months later |
|---|---|
| Human or AI? which model/prompt? | **Nothing** — the commit has an author email; there is no schema for AI authorship, model, or prompt |
| Did it pass its checks? | **Gone** — Actions logs rotated; and they were GitHub-locked, not bound to the bytes |
| Did a human approve *this* build? | **Unprovable** — branch protection was a setting (possibly toggled); no artifact-bound proof, and it died in the migration |
| Verify any of it now, off-GitHub? | **No** — the evidence lived inside GitHub |

### With OpenFab

The artifact carries a portable in-toto/SLSA attestation. The auditor runs
`openfab verify <artifact>` **offline, on the Forgejo copy, with no GitHub
account**:

```
✔ generation: fee_round.py lines 12–47 = agent-authored
              (base=claude · model=claude-opus-4 · prompt sha256=ab39…c1)
              lines 1–11, 48–60 = human-authored
✔ contract:   a3-round-half-even  `python3 -c "...; assert round_half_even(2.5)==2"`
              → re-run NOW against the signed bytes → PASS  (deterministic, no LLM)
✔ sign-off:   2-of-2 maintainers signed THIS artifact hash
              did:key:z6Mk…A (Alice), did:key:z6Mk…B (Bob)
```

Every line verifies **without GitHub, without the CI vendor, without the original
team, on a different forge, 18 months later** — because the evidence travels
*with the artifact* as signed open-standard attestations, not inside a platform.

**The decisive line:** GitHub can't say *"this code was AI-written by model X from
prompt-hash Y, provably passed contract Z, and was signed off by humans A and
B"* — as **one portable, offline-verifiable fact**. That sentence is OpenFab's
entire reason to exist.

> Note what is *still* not claimed: OpenFab cannot regenerate that rounding
> function, nor prove the *original* CI run happened. It proves the **shipped
> bytes** carry their generation recipe, conform to a **frozen contract** when
> re-checked, and were **signed by named humans** — the three facts the audit
> actually needs.

## 7. Cross-forge & governance — precise status

"Cross-forge / forge-agnostic" has **two senses**. The first is the one that
matters.

### 7.1 Forge-agnostic **trust** (the verification side — the important one)

Because the OpenFab artifacts (attestation + SBOM + maintainer allowlist) are
**committed into git alongside the code**, a `git clone` from **any** forge — or a
tarball, or an air-gapped copy — carries everything needed to verify. The verifier
runs `openfab verify` / `reproduce` **locally, touching no forge API**:

| What you can re-confirm / 可复验 | From what (all in the clone) / 凭据来源 |
|---|---|
| **Integrity / 完整性** | per-file sha256 vs the attestation |
| **Authenticity / 真实性** | maintainers' `did:key` signatures over the artifact hash |
| **Provenance / 溯源** | generation predicate (human/AI, model, prompt-hash) |
| **Conformance / 达标** | re-run the frozen contract on the committed bytes |

**The trust root is the signatures + open-standard attestations, not the forge.**
The forge is just transport/hosting — GitHub or Gitea yields the same verification,
even offline. / 信任根是签名与开放标准凭证，不是平台；平台只是搬运。

The acceptance **check commands are embedded inside the signed attestation** (not
just the pass/fail verdict), so contract-replay needs **no local run-state**. One
command proves it from a bare clone:

```
openfab verify-file --repo . --att provenance/<spec>-vN.att.json
  → signatures valid · source bit-identical · 4/4 embedded checks re-passed
  → ✅ reproducible — verified offline, no .openfab/ run-state
```

### 7.2 Forge-**swappable** plumbing (the lesser sense)

Separately, OpenFab's `ForgePort` adapters let it *drive* GitHub / Forgejo / Gitea
/ GitCode / local-git (branch, commit, PR, merge) through the same Core pipeline.
That is "swappable forge" in the operational sense — useful, but secondary to 7.1.

### 7.3 Honest nuance / 诚实细节

The verifier needs the repo's **maintainer allowlist** (the did:keys allowed to
sign) — but that **also travels in git** (`.openfab/maintainers/`), so verification
stays fully self-contained. What you decide out-of-band is whether you *trust those
identities*: OpenFab proves *they signed*, not *that they are trustworthy people*.
/ OpenFab 证明"他们签了名"，不证明"这些人可信"。

**vs GitHub:** GitHub's Artifact Attestations live in **GitHub's** attestation
store — forge-locked; leave GitHub and they're gone. OpenFab's live in the git
tree → portable. / GitHub 的凭证锁在平台；OpenFab 的在 git 树里，带得走。

### 7.4 Governance

- **Governance: a vendor-neutral aspiration, not a code feature.** Someone must
  own the spec / trust-root so no single vendor (including OpenFab) controls it —
  the way SLSA sits under OpenSSF. Honest framing: *"OpenFab proposes its
  generation predicate as a vendor-neutral, community-governed open standard"* —
  it is **not** yet adopted by any standards body.

---

## 7.45 The simplest value — a normal AI commit, made checkable

Not spec-driven development. The clearest value needs **no spec and no workflow change.**

You build a payment-rounding function with Claude Code — straight to the LLM, normal
workflow. Ship it. Six months later, an audit asks three questions:

| Question | GitHub / Claude Code today | With OpenFab at the commit |
|---|---|---|
| Was this AI-written? which parts / model? | ❌ `git log` just says *you* | ✅ "lines 12–47 AI (claude-opus, prompt #ab39); 1–11 human" |
| Did it pass tests *when shipped*? | ❌ CI logs rotated — can't prove | ✅ re-run the signed checks now → still pass |
| Who approved *these exact bytes*? | ⚠️ a revocable branch setting | ✅ N maintainers' signatures over the artifact hash |

Same coding workflow; the commit just carries a **signed AI-BOM + test-pass proof**,
re-verifiable offline on any forge, years later.

**The principle:** *value = (how much someone must trust this code without re-reading it)
× (how long that trust must hold).* / 价值 = 别人需盲信你代码的程度 × 信任维持的时长。

- Solo throwaway script → ~0 value (be honest — don't use OpenFab).
- Teammate merges your AI PR · you `npm install` a vibe-coded package · an OSPO ingests an
  outside contribution · a CRA audit 10 years out → **exactly this, and it compounds.**

OpenFab doesn't ask you to code differently. You already trust AI-written code you can't
see into; OpenFab makes that trust **checkable instead of assumed.** / 把"默认信任"变成"可验证的信任"。

## 7.5 Where OpenFab fits — the boundary, not the inner loop

The most common objection: *"Nobody writes a spec to prompt Claude Code — the whole
cycle feels unrealistic."* Correct — so be precise about **where OpenFab runs.**

| Layer | Reality | OpenFab's role |
|---|---|---|
| **Inner loop / 内循环** | Claude Code / Cursor, many iterations, **no spec**, fast & messy | none — or **draft mode** (generate + check, *no* signing) just mirrors it |
| **Release / contribution boundary / 发布·贡献边界** | occasional — a PR, a merge, a published artifact | **here**: attest **once** — AI-BOM + signed conformance + gate |

OpenFab is **not** a coding tool and does **not** change how anyone codes. The right
mental model is **an AI-native CI + provenance gate on a PR** — nobody objects to CI
running at a merge. / OpenFab 是发布边界上的"CI+溯源闸门"，不碰你的开发内循环。

**Spec-anchored, not spec-first.** The acceptance contract is **auto-derived** from the
intent, or — realistically — **IS the repo's existing test suite.** A human never
hand-authors a heavyweight spec. The greenfield "LLM authors the spec" flow is a demo
convenience; in a real repo the contract = the checks you already have. Spec-driven
development is a valid and *resurgent* practice for AI (Kiro, spec-kit: "the spec is the
durable artifact, code is disposable output") — OpenFab leans on that, without asking
developers to write specs by hand.

### AI-BOM scope — a manifest, not a transcript

Like an SBOM (which lists components + versions + hashes, never full source), the AI-BOM
is a **manifest**:

- **In the signed BOM:** per-range human/AI authorship, model id, **prompt-hash**,
  spec-ref, acceptance result, signature.
- **Not in it (linked by content-hash, access-controlled):** full prompt text, full
  agent/LLM turn logs, discarded iterations — they carry secrets/PII, bloat the artifact,
  and don't aid reproduction. Iteration history is captured by **chaining to the parent
  attestation**, not by dumping every transcript.

## 7.6 The human gate is behavioral, not spec-reading

The naive version of spec-driven dev assumes a human **reads** the spec to gate it. That
breaks for autonomous AI: the spec is itself AI-generated (non-deterministic, possibly
incomplete/wrong), and at scale a long generated spec is as infeasible to read as the code.
In real agentic systems the human gates at the **behavioral** stage — they *view the
running product* and approve or say "tweak X." So the fix is to **separate who gates from
what's recorded.**

| | Who / what | When |
|---|---|---|
| **Human gate / 人工关卡** | a person **views the running build** → approves or "tweak X" | once, at the viewable stage |
| **Acceptance contract / 验收合同** | the **machine-checkable snapshot of what passed when they approved** | auto-captured *around* that approval — **never read by the human** |

The human never reads the spec; they approve **behavior.** OpenFab's job is to **notarize
that behavioral approval + the checks that held at that moment + how it was made (AI-BOM)**,
so the same trust is **re-verifiable later by machine** without dragging a human back.

**Why an AI-drafted contract still has value once anchored to approval:**
- **Determinism after freeze** — AI-drafted, but once frozen it re-checks identically
  forever (no LLM in the loop). Code correctness stays open-ended; the captured checks don't.
- **Cross-validation** — if the human approved the running product *and* the checks passed
  on it, the checks are at least consistent with approved behavior. A check the approved
  product would fail would have blocked approval. The contract is trusted not *because an
  AI wrote it*, but because it **agrees with what a human accepted.**

**Honest limit:** the contract only covers **what was exercised/approved.** An edge case
the human never triggered and the AI never checked is a blind spot, forever. OpenFab
attests *"conforms to **these** checks + a human approved **this** behavior + made **this**
way"* — **not** *"fully correct."* Same bound as any human-written test suite: presence of
checked behavior, never absence of all bugs.

**Design implications (this sharpens OpenFab):**
1. **Make the human gate behavioral** — "view the running build → approve/tweak" (the
   *Run the app* + draft→refine loop is exactly this). Don't make people read specs.
2. **Keep the gating contract small** — a handful of **must-pass invariants**, not a giant
   generated spec. A 500-line spec is a smell, not a gate.
3. **Notarize the approval** so it re-checks at machine scale without re-involving the human.

Net: **humans gate behavior once; OpenFab turns that one behavioral "yes" into durable,
re-verifiable, attributed evidence.** The spec/acceptance isn't a reading task — it's the
**memory** of the human's behavioral approval, kept in a form a machine can re-check forever.

## 8. What OpenFab does **not** solve (so nobody is misled)

- Bit-for-bit reproduction of LLM output — impossible; not attempted.
- Proof that the *original* generation/CI event actually happened the way claimed
  — OpenFab attests the **shipped bytes** carry a self-consistent recipe + frozen
  contract + human signatures; it does not (and cannot) replay history. The trust
  root is the maintainers' `did:key` signatures, not a recording of the build.
- Making a weak model write good code — the acceptance gate only *catches* bad
  output, it doesn't prevent it.
- Replacing GitHub — it's a trust layer on top of any forge, not a forge.

---

## 9. The workflow / 工作流 — each step, value, and pain it kills

`Intent → Spec+Contract → Generate → Verify → Sign → Gate → Portable Proof`

| # | Step / 步骤 | Function / 功能 | Pain it addresses / 解决的痛点 |
|---|---|---|---|
| 1 | **Intent / 意图输入** | Plain-English ask → structured task | No manual spec-writing / 免手写规约 |
| 2 | **Spec + Acceptance / 规约 + 验收契约** | LLM authors a versioned spec **and** machine-checkable checks | "Done" is vague; *tests-green ≠ meets-ask* / 验收标准模糊 |
| 3 | **Generate / 代码生成** | Dispatch to a **swappable base** (claude·codex·agentscope·agent-chat) | Agent/vendor lock-in / 智能体厂商锁定 |
| 4 | **Verify / 验收验证** | Re-run the **frozen contract** in a gated sandbox | CI logs expire & aren't bound to the bytes / CI不可信、会过期 |
| 5 | **Sign / 签名溯源** | in-toto/SLSA + **generation predicate** (human/AI per line, model, prompt-hash) + SBOM | GitHub can't say human-or-AI / which model / which prompt / 无AI溯源 |
| 6 | **Gate / 信任门** | **N-of-M** human sign-off via `did:key` | Branch protection is revocable & unprovable / 审批不可证明 |
| 7 | **Portable Proof / 可移植凭证** | Attestation verifies across **any forge, offline, forever** | Trust dies when you leave the platform / 平台锁定、离开即失效 |

**One line / 一句话:** GitHub records *who pushed*; OpenFab proves *how it was
made (AI/human + model + prompt), that it meets a frozen contract, and that humans
signed it* — as one portable, offline fact. / GitHub只记录"谁提交"；OpenFab证明
"如何生成 + 是否达标 + 谁签核"，且跨平台离线永久可验。

---

## 10. The sandbox & re-verification — how step 4 actually works

**Where / 沙箱在哪:** `src/adapters/sandbox.rs` → `exec_gated`. Each acceptance
check runs as `bash -c "<check>"` confined to the run's workdir, with:

- **Policy gate / 策略门** — `Policy::check_command` (allowlist/denylist) checked
  *before* execution.
- **Timeout + process-group kill / 超时强杀** — a check that spawns a server can't
  wedge the cycle; the whole group is hard-killed on expiry.
- **Honest runtime label / 诚实标注** — records `gated-host-subprocess` (v0.1);
  the production target is a Podman/gVisor container. It never claims isolation it
  didn't use (R14).

**Acceptance vs. Reproduce — same checks, different job:** the *same* sandbox and
the *same* frozen checks run in both; only the wrapper differs.

| | **Acceptance / 验收** (build time) | **Reproduce / 复验** (verify time) |
|---|---|---|
| When | Once, right after generation | Anytime later, by anyone, offline |
| Path | `base.run_sandboxed` → `exec_gated` | `ops::reproduce` → `exec_gated` |
| Purpose | **First-time gate** — decides whether to sign | **Independent re-verification** of the *signed* artifact |
| Extra | none — just runs the checks | ① fresh checkout of the attested branch ② **hash-match** every file vs recorded sha256 ③ **verify signatures** ④ then re-run the same checks |
| Verdict | `acceptance_passed` | `reproducible = signature_valid && source_identical && all_passed` |

So *"re-run the frozen contract"* **is** the acceptance step — but `reproduce`
wraps it with **source-hash + signature checks on a clean checkout**, proving
*"the exact signed bytes still pass the exact signed contract,"* not merely *"some
working tree passed."* That wrapper is what makes §6's 18-months-later offline
audit possible.

---

## 11. Iteration & recording / 迭代与记录 — the proof lives *in* the commit

**Each spec change is a new versioned run** (v1→v2→v3, each linked to its
`parent_run`). What fires depends on mode:

| Mode / 模式 | Chain / 链路 | Committed to git / 提交内容 |
|---|---|---|
| **Draft / 草稿** | author → generate → **acceptance check** → commit source | source + a trailer noting *NOT signed, un-attested* |
| **Release / 发布** | + **sign** (in-toto/SLSA) + SBOM + PR + **N-of-M gate** | source **and** attestation **and** SBOM — in **one atomic commit** |
| **Promote / 提升** | run the full Release ceremony on a passing draft once | as Release |

**Two distinct stores — don't confuse them:**

- **Local run-state** (`.openfab/runs/<run_id>/`: `spec.yaml`, `run.json`,
  `timeline.md`, `events.jsonl`, `status.json`) — OpenFab's working notes /
  replayable log. **Not committed to the artifact repo** (it's machine-local).
- **The committed artifact** (the Release commit below) — the **portable** part
  that travels with `git clone`. Everything a third party needs to verify is
  here, *not* in the run-state.

### The key point: provenance is committed *with* the code, not beside it

In **Release** mode OpenFab makes **one atomic commit**
([`spec_cycle.rs:451`](../src/spec_cycle.rs)) containing:

```
ONE commit:
  • generated source files
  • provenance/<spec>-vN.att.json   ← generation predicate (human/AI, model,
                                       prompt-hash) + the EMBEDDED acceptance
                                       contract (check commands + results) + signature
  • provenance/<spec>-vN.sbom.json  ← SBOM
  + git trailers: Spec, OpenFab-Base, OpenFab-Attestation:<sha>,
                  OpenFab-Acceptance:passed, Co-Authored-By: openfab-agent (did:key:…)
```

So the attestation, SBOM, acceptance verdict and signature are **part of the
git check-in itself** — inseparable from the code at that commit, and carried by
any `git clone`. / 凭证、SBOM、验收结论、签名 与代码同处一次提交，克隆即带走。

**Why this beats a GitHub PR / 优于 GitHub PR 之处:**

| | GitHub PR | OpenFab commit |
|---|---|---|
| The diff / 变更 | ✅ in the commit | ✅ in the commit |
| Acceptance result / 验收结论 | ⚠️ CI status in GitHub's DB, **not in git**, expires | ✅ in the committed attestation + git trailer |
| How generated (AI/model/prompt) / 生成方式 | ❌ nowhere | ✅ generation predicate, in the commit |
| Signature + SBOM / 签名+SBOM | ❌ outside git | ✅ in the commit, travels with `git clone` |

A GitHub PR records *what changed*; its CI/approval data lives **outside git** and
dies if you leave the platform. OpenFab folds the proof **into the git tree**. /
GitHub 把验收/审批放在数据库、不在 git；OpenFab 写进 git 树，离开平台也带得走。

**Caveat / 注意:** only the **final committed bytes** of a run are attested —
intermediate discarded generations within a run are not separately signed.
