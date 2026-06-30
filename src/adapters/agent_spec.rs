//! agent-spec adapter — author the spec as an agent-spec Task Contract (`.spec.md`)
//! instead of OpenFab's built-in LLM-authored JSON, and delegate verification to
//! `agent-spec lifecycle`. Selected by `OPENFAB_SPEC=agent-spec`.
//!
//! The `.spec.md` is the source of truth (committed into the repo's `specs/`); OpenFab's
//! `core::spec::Spec` is a *derived view* parsed from `agent-spec parse --format json`.
//! agent-spec's richer fields (Decisions, Boundaries) are kept on `AgentSpecContract` so
//! the dispatch prompt can triple-constrain the implementer and provenance can record them.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::core::provenance::ScenarioVerdict;
use crate::core::sha256_hex;
use crate::core::spec::{Acceptance, Spec};
use crate::runstate::AcceptanceOutcome;

/// A spec authored via agent-spec: the derived OpenFab `Spec` plus the agent-spec-only
/// fields (Decisions / Boundaries) that OpenFab's native spec format doesn't carry.
#[derive(Debug, Clone)]
pub struct AgentSpecContract {
    pub spec: Spec,
    /// "things already decided" — constrain *how* the implementer works.
    pub decisions: Vec<String>,
    /// Boundaries → Allowed Changes (file/module globs the agent may touch).
    pub allow: Vec<String>,
    /// Boundaries → Forbidden (things the agent must not do).
    pub deny: Vec<String>,
}

/// Map the AST from `agent-spec parse --format json` into an [`AgentSpecContract`].
/// `fallback_intent` is the original NL ask, used when the contract's Intent is empty.
pub fn parse_contract(ast: &serde_json::Value, fallback_intent: &str) -> Result<AgentSpecContract> {
    let name = ast
        .pointer("/meta/name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id = slug(&name);

    let sections = ast
        .get("sections")
        .and_then(|v| v.as_array())
        .context("agent-spec AST has no `sections` array")?;
    let section = |kind: &str| {
        sections
            .iter()
            .find(|s| s.get("kind").and_then(|k| k.as_str()) == Some(kind))
    };
    let str_items = |kind: &str| -> Vec<String> {
        section(kind)
            .and_then(|s| s.get("items"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };

    let intent = section("intent")
        .and_then(|s| s.get("content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_intent)
        .to_string();

    let decisions = str_items("decisions");

    let (mut allow, mut deny) = (Vec::new(), Vec::new());
    if let Some(items) = section("boundaries")
        .and_then(|s| s.get("items"))
        .and_then(|v| v.as_array())
    {
        for it in items {
            let text = it
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                continue;
            }
            match it.get("category").and_then(|v| v.as_str()) {
                Some("deny") => deny.push(text),
                _ => allow.push(text),
            }
        }
    }

    let mut acceptance = Vec::new();
    if let Some(scenarios) = section("acceptance_criteria")
        .and_then(|s| s.get("scenarios"))
        .and_then(|v| v.as_array())
    {
        for sc in scenarios {
            let aid = sc
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if aid.is_empty() {
                continue;
            }
            let pkg = sc
                .pointer("/test_selector/package")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let filter = sc
                .pointer("/test_selector/filter")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Execution is delegated to `agent-spec lifecycle`; the literal check records the
            // bound test selector for the audit trail (Filter-only, or package::filter).
            let check = if pkg.is_empty() {
                format!("agent-spec test: {filter}")
            } else {
                format!("agent-spec test: {pkg}::{filter}")
            };
            // Every scenario must pass for acceptance; the `critical` tag is surfaced to the
            // conformance gate separately (it sets agent-spec's gate_blocked).
            acceptance.push(Acceptance {
                id: aid,
                check,
                must_pass: true,
            });
        }
    }

    let assumptions: Vec<String> = str_items("out_of_scope")
        .into_iter()
        .map(|s| format!("out of scope: {s}"))
        .collect();

    // Where generated code lives: agent-spec `lifecycle --code <repo>` runs the bound tests
    // from the repo root, so a project whose Boundaries reference root-level layout
    // (`src/**`, `tests/**`, `Cargo.toml`, `pyproject.toml`, …) must be generated at the
    // root, not nested under `app/`. Otherwise default to `app/`.
    let root_layout = allow.iter().chain(deny.iter()).any(|p| {
        let p = p.trim_start_matches("./");
        p.starts_with("src/")
            || p.starts_with("src ")
            || p == "src"
            || p.starts_with("tests/")
            || p.starts_with("Cargo.toml")
            || p.starts_with("pyproject.toml")
            || p.starts_with("go.mod")
            || p.starts_with("package.json")
    });
    let target_dir = if root_layout {
        ".".to_string()
    } else {
        "app".to_string()
    };

    let spec = Spec {
        id,
        version: 1,
        intent,
        context: vec![],
        acceptance,
        assumptions,
        open_questions: vec![],
        human_signoff_required: true,
        target_dir,
        language: None,
    };
    spec.validate()
        .context("agent-spec-derived spec was invalid")?;

    Ok(AgentSpecContract {
        spec,
        decisions,
        allow,
        deny,
    })
}

/// Outcome of an `agent-spec lint --format json` quality gate.
#[derive(Debug, Clone)]
pub struct LintReport {
    pub overall: f64,
    pub errors: usize,
    pub warnings: usize,
    pub messages: Vec<String>,
}

/// Gate a drafted `.spec.md` on its agent-spec lint report: the overall quality score must
/// meet `min_score` and there must be no error-severity diagnostics. Returns the report on
/// success; bails with the diagnostics otherwise. This is the contract quality gate (the
/// "spec review" before the agent ever sees it).
pub fn lint_gate(lint_json: &serde_json::Value, min_score: f64) -> Result<LintReport> {
    let overall = lint_json
        .pointer("/quality_score/overall")
        .and_then(|v| v.as_f64())
        .context("agent-spec lint json has no quality_score.overall")?;

    let mut errors = 0;
    let mut warnings = 0;
    let mut messages = Vec::new();
    if let Some(diags) = lint_json.get("diagnostics").and_then(|v| v.as_array()) {
        for d in diags {
            let severity = d.get("severity").and_then(|v| v.as_str()).unwrap_or("");
            let message = d
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match severity {
                "error" => errors += 1,
                "warning" => warnings += 1,
                _ => {}
            }
            messages.push(format!("[{severity}] {message}"));
        }
    }

    let report = LintReport {
        overall,
        errors,
        warnings,
        messages,
    };
    if report.errors > 0 {
        bail!(
            "agent-spec contract has {} error-severity issue(s); fix before building:\n{}",
            report.errors,
            report.messages.join("\n")
        );
    }
    if overall < min_score {
        bail!(
            "agent-spec contract quality {:.2} is below the {:.2} threshold:\n{}",
            overall,
            min_score,
            report.messages.join("\n")
        );
    }
    Ok(report)
}

impl AgentSpecContract {
    /// The derived `Spec` with the agent-spec-only constraints (Decisions, Boundaries)
    /// folded into `assumptions`, so OpenFab's dispatch prompt surfaces them to the
    /// implementer (the agent is then constrained by *how*, *what to touch*, and *done*).
    pub fn folded_spec(&self) -> Spec {
        let mut spec = self.spec.clone();
        let mut folded = Vec::new();
        for d in &self.decisions {
            folded.push(format!("decision: {d}"));
        }
        for a in &self.allow {
            folded.push(format!("may modify: {a}"));
        }
        for d in &self.deny {
            folded.push(format!("must not: {d}"));
        }
        // keep the contract's own assumptions (e.g. out-of-scope) after the constraints
        folded.append(&mut spec.assumptions);
        spec.assumptions = folded;
        spec
    }
}

/// Build the LLM prompt that drafts an agent-spec Task Contract (`.spec.md`) from an NL
/// intent. The draft is then lint-gated and parsed — the human does Contract Acceptance.
pub fn draft_prompt(intent: &str) -> String {
    format!(
        r#"You are OpenFab's SPEC AUTHOR. Turn the user's natural-language request into an
agent-spec **Task Contract** in `.spec.md` format. Respond with ONLY the file content
(no prose, no code fences).

EXACT FORMAT — start with this frontmatter (DO NOT emit an `inherits:` line; the contract
must be standalone):

spec: task
name: "<short-kebab-name>"
tags: []
---

## Intent

<one or two sentences: the goal and context>

## Decisions

- <technical choices already decided — language, libraries, approach>

## Boundaries

### Allowed Changes
- <file/dir globs the agent may modify, e.g. app/**>

### Forbidden
- <things the agent must not do>

## Completion Criteria

Scenario: <happy-path behavior>
  Test:
    Filter: <a concrete test function name, e.g. test_happy_path>
  Given <precondition>
  When <action>
  Then <observable result>

Scenario: <an error / edge path>
  Test:
    Filter: <test function name, e.g. test_error_path>
  Given <precondition>
  When <invalid action>
  Then <error result>

## Out of Scope

- <things explicitly not in scope>

RULES:
- At least 2 scenarios; exception/error scenarios are as important as the happy path.
- EVERY scenario MUST have a `Test:` block with a concrete `Filter:` test name — the
  implementer will write a test of exactly that name (verification binds to it).
- Use ONLY `Filter:` in the Test block — do NOT add a `Package:` line. Verification runs
  the test by name (e.g. `cargo test <Filter>`, `pytest -k <Filter>`); a `Package:` value
  that isn't a real build-package name makes the run fail.
- Keep it standalone: no `inherits:` line.

USER REQUEST:
{intent}"#
    )
}

/// Extract the `.spec.md` body from an LLM reply: strip code fences / surrounding prose,
/// start at the `spec:` frontmatter, and remove any `inherits:` line (OpenFab task specs
/// must be standalone or agent-spec's contract/lifecycle fails to resolve inheritance).
pub fn extract_spec_md(llm_text: &str) -> String {
    let mut body = llm_text.trim().to_string();

    // 1. start at the `spec:` frontmatter, dropping any surrounding prose / opening fence.
    if let Some(pos) = body.find("spec:") {
        body = body[pos..].to_string();
    }

    // 2. cut at the first closing code fence (and anything after it).
    if let Some(idx) = body.find("```") {
        body.truncate(idx);
    }

    // 3. drop any `inherits:` line (task specs must be standalone).
    let cleaned: Vec<&str> = body
        .lines()
        .filter(|l| !l.trim_start().starts_with("inherits:"))
        .collect();

    cleaned.join("\n").trim().to_string()
}

/// Map an `agent-spec lifecycle --format json` report into OpenFab `AcceptanceOutcome`s.
/// One outcome per verified scenario: `pass` → passed (exit 0); `fail`/`skip`/`uncertain`
/// → not passed (skip ≠ pass, per agent-spec). Returns (outcomes, acceptance_passed) where
/// acceptance_passed honors the report's top-level `passed` when present.
pub fn outcomes_from_lifecycle(json: &serde_json::Value) -> Result<(Vec<AcceptanceOutcome>, bool)> {
    let results = json
        .pointer("/verification/results")
        .and_then(|v| v.as_array())
        .context("agent-spec lifecycle json has no verification.results")?;

    let mut outcomes = Vec::new();
    for r in results {
        let id = r
            .get("scenario_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let verdict = r.get("verdict").and_then(|v| v.as_str()).unwrap_or("skip");
        let passed = verdict == "pass";
        outcomes.push(AcceptanceOutcome {
            id,
            check: format!("agent-spec lifecycle [{verdict}]"),
            passed,
            exit_code: if passed { 0 } else { 1 },
        });
    }

    let acceptance_passed = match json.get("passed").and_then(|v| v.as_bool()) {
        Some(p) => p,
        None => !outcomes.is_empty() && outcomes.iter().all(|o| o.passed),
    };
    Ok((outcomes, acceptance_passed))
}

/// One AI-pending scenario the reviewer must decide (from caller-mode's pending requests).
#[derive(Debug, Clone)]
pub struct ReviewItem {
    pub scenario_name: String,
    pub intent: String,
}

/// Parse the caller-mode `pending-ai-requests.json` (an array of AiRequest objects) into the
/// review items OpenFab sends to the reviewer agent.
pub fn parse_ai_requests(json: &serde_json::Value) -> Vec<ReviewItem> {
    let arr = json.as_array().cloned().unwrap_or_default();
    arr.iter()
        .filter_map(|r| {
            let scenario_name = r
                .get("scenario_name")
                .or_else(|| r.get("scenario"))
                .and_then(|v| v.as_str())?
                .to_string();
            let intent = r
                .get("intent")
                .or_else(|| r.get("contract_intent"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ReviewItem {
                scenario_name,
                intent,
            })
        })
        .collect()
}

/// A reviewer agent's decision on one AI-pending scenario.
#[derive(Debug, Clone)]
pub struct ReviewDecision {
    pub scenario_name: String,
    pub verdict: String, // "pass" | "fail"
    pub confidence: f64,
    pub reasoning: String,
    pub model: String,
}

/// Serialize reviewer decisions into the `agent-spec resolve-ai --decisions` JSON format.
pub fn decisions_to_json(decisions: &[ReviewDecision]) -> serde_json::Value {
    serde_json::Value::Array(
        decisions
            .iter()
            .map(|d| {
                serde_json::json!({
                    "scenario_name": d.scenario_name,
                    "model": if d.model.is_empty() { "openfab-reviewer" } else { d.model.as_str() },
                    "confidence": d.confidence,
                    "verdict": d.verdict,
                    "reasoning": d.reasoning,
                })
            })
            .collect(),
    )
}

/// Parse reviewer decisions from a `review_result` payload's `decisions` array (from the
/// Bridge), tolerant of missing fields.
pub fn parse_review_decisions(json: &serde_json::Value) -> Vec<ReviewDecision> {
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    let scenario_name =
                        d.get("scenario_name").and_then(|v| v.as_str())?.to_string();
                    Some(ReviewDecision {
                        scenario_name,
                        verdict: d
                            .get("verdict")
                            .and_then(|v| v.as_str())
                            .unwrap_or("fail")
                            .to_string(),
                        confidence: d.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        reasoning: d
                            .get("reasoning")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        model: d
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One model family's verdict on one scenario (cross-model adversarial panel, PPT S14 pillar 2).
#[derive(Debug, Clone)]
pub struct CrossModelVerdict {
    pub model_family: String,
    pub scenario: String,
    pub verdict: String, // "pass" | "fail"
}

/// Adversarial-strict merge: blocked if ANY family returns a non-pass verdict for ANY scenario
/// (two model families don't share blind spots — one's bug is caught by the other). An empty set
/// is not blocking (nothing to object).
pub fn cross_model_blocked(verdicts: &[CrossModelVerdict]) -> bool {
    verdicts.iter().any(|v| v.verdict != "pass")
}

/// Serialize per-family verdicts for the signed provenance predicate.
pub fn cross_model_verdicts_json(verdicts: &[CrossModelVerdict]) -> serde_json::Value {
    serde_json::Value::Array(
        verdicts
            .iter()
            .map(|v| {
                serde_json::json!({
                    "model_family": v.model_family,
                    "scenario": v.scenario,
                    "verdict": v.verdict,
                })
            })
            .collect(),
    )
}

/// Extract per-scenario verdicts from an `agent-spec lifecycle --format json` report, for
/// recording in the signed provenance predicate.
pub fn verdicts_from_lifecycle(json: &serde_json::Value) -> Vec<ScenarioVerdict> {
    json.pointer("/verification/results")
        .and_then(|v| v.as_array())
        .map(|results| {
            results
                .iter()
                .filter_map(|r| {
                    let scenario = r.get("scenario_name").and_then(|v| v.as_str())?.to_string();
                    let verdict = r
                        .get("verdict")
                        .and_then(|v| v.as_str())
                        .unwrap_or("skip")
                        .to_string();
                    Some(ScenarioVerdict { scenario, verdict })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// When `agent-spec lifecycle --ai-mode caller` leaves scenarios for AI review, it sets
/// `ai_pending` and writes an `ai_requests_file`. Returns that path so the caller can route
/// the requests to a reviewer agent (robrix2's `wf_reviewer` via `--ai-mode caller`), whose
/// decisions are merged back with `agent-spec resolve-ai`. `None` when nothing is pending.
pub fn lifecycle_ai_pending(json: &serde_json::Value) -> Option<String> {
    let pending = json
        .get("ai_pending")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !pending {
        return None;
    }
    Some(
        json.get("ai_requests_file")
            .and_then(|v| v.as_str())
            .unwrap_or(".agent-spec/pending-ai-requests.json")
            .to_string(),
    )
}

/// Absolute path of the authored `.spec.md` for a spec (`<OPENFAB_SPEC_DIR>/<id>.spec.md`).
pub fn spec_md_path(spec_id: &str) -> PathBuf {
    let spec_dir = std::env::var("OPENFAB_SPEC_DIR").unwrap_or_else(|_| "specs".to_string());
    let p = Path::new(&spec_dir).join(format!("{spec_id}.spec.md"));
    p.canonicalize().unwrap_or(p)
}

/// SHA-256 of the authored `.spec.md` Task Contract (the contract is signed evidence).
/// `None` if the file can't be read.
pub fn contract_sha256(spec: &Spec) -> Option<String> {
    std::fs::read(spec_md_path(&spec.id))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
}

fn spec_dir_env() -> String {
    std::env::var("OPENFAB_SPEC_DIR").unwrap_or_else(|_| "specs".to_string())
}

/// In-spec-dir path of the requirements document a spec was distilled from (Phase 2).
pub fn requirements_md_path_in(dir: &Path, spec_id: &str) -> PathBuf {
    dir.join(format!("{spec_id}.requirements.md"))
}

/// SHA-256 of the requirements document in `dir` for `spec_id`, if present.
pub fn requirements_sha256_in(dir: &Path, spec_id: &str) -> Option<String> {
    std::fs::read(requirements_md_path_in(dir, spec_id))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
}

/// SHA-256 of the requirements document for a spec (under `OPENFAB_SPEC_DIR`), if present.
pub fn requirements_sha256(spec_id: &str) -> Option<String> {
    requirements_sha256_in(Path::new(&spec_dir_env()), spec_id)
}

/// The result of authoring a spec via agent-spec.
pub struct Authored {
    pub contract: AgentSpecContract,
    pub model: String,
    pub provider: String,
    /// Where the `.spec.md` was persisted (the source of truth).
    pub spec_md_path: PathBuf,
}

/// Run an `agent-spec` subcommand and parse its `--format json` stdout. Does NOT fail on a
/// non-zero exit (lint exits non-zero when issues exist); the caller's gate decides.
fn run_agent_spec_json(args: &[&str]) -> Result<serde_json::Value> {
    let bin = std::env::var("OPENFAB_AGENT_SPEC_BIN").unwrap_or_else(|_| "agent-spec".to_string());
    let out = Command::new(&bin).args(args).output().with_context(|| {
        format!(
            "running `{bin} {}` — is agent-spec installed?",
            args.join(" ")
        )
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).with_context(|| {
        format!(
            "`agent-spec {}` did not emit JSON.\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })
}

/// Author a spec as an agent-spec Task Contract: draft `.spec.md` from the NL intent (LLM),
/// quality-gate it (`agent-spec lint`), then parse it (`agent-spec parse`) into an
/// [`AgentSpecContract`]. The `.spec.md` is persisted in `spec_dir` as the source of truth.
pub fn author_via_agent_spec(intent: &str, spec_dir: &Path) -> Result<Authored> {
    let prompt = draft_prompt(intent);
    let (text, model, provider) = crate::adapters::llm_backend::complete(&prompt)
        .context("LLM failed to draft a .spec.md")?;
    let md = extract_spec_md(&text);
    if !md.contains("spec:") {
        bail!("LLM reply did not contain a .spec.md (no `spec:` frontmatter):\n{text}");
    }
    author_from_md(&md, intent, spec_dir, model, provider)
}

/// The deterministic half of authoring (no LLM): take a `.spec.md` body, gate it with
/// `agent-spec lint`, parse it with `agent-spec parse`, and persist it under its canonical
/// id. `model`/`provider` label who drafted the `.spec.md`.
pub fn author_from_md(
    md: &str,
    intent: &str,
    spec_dir: &Path,
    model: String,
    provider: String,
) -> Result<Authored> {
    std::fs::create_dir_all(spec_dir)
        .with_context(|| format!("creating spec dir {}", spec_dir.display()))?;
    let draft = spec_dir.join(".openfab-draft.spec.md");
    std::fs::write(&draft, md).with_context(|| format!("writing {}", draft.display()))?;

    let draft_str = draft.to_string_lossy().to_string();

    // Contract quality gate (spec review before the agent ever sees it).
    let min_score = std::env::var("OPENFAB_SPEC_MIN_SCORE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.7);
    let lint_json = run_agent_spec_json(&["lint", &draft_str, "--format", "json"])?;
    if let Err(e) = lint_gate(&lint_json, min_score) {
        let _ = std::fs::remove_file(&draft);
        return Err(e);
    }

    // Parse the contract into OpenFab's derived Spec.
    let ast = run_agent_spec_json(&["parse", &draft_str, "--format", "json"])?;
    let contract = parse_contract(&ast, intent)?;

    // Persist under the canonical id and drop the draft.
    let final_path = spec_dir.join(format!("{}.spec.md", contract.spec.id));
    std::fs::rename(&draft, &final_path)
        .with_context(|| format!("persisting spec to {}", final_path.display()))?;

    Ok(Authored {
        contract,
        model,
        provider,
        spec_md_path: final_path,
    })
}

/// Verify a spec by delegating to `agent-spec lifecycle` against the generated repo. The
/// `.spec.md` is located at `<OPENFAB_SPEC_DIR>/<spec.id>.spec.md` (the source of truth).
/// Returns per-scenario [`AcceptanceOutcome`]s; the caller computes acceptance_passed.
pub fn verify_via_lifecycle(
    spec: &Spec,
    repo: &Path,
) -> Result<(Vec<AcceptanceOutcome>, Vec<ScenarioVerdict>)> {
    lifecycle_run(&spec_md_path(&spec.id), repo)
}

/// Run `agent-spec lifecycle <spec_md> --code <repo>` and return the per-scenario
/// outcomes + verdicts. Used by both verify (authored copy) and reproduce (committed copy).
pub fn lifecycle_run(
    spec_md: &Path,
    repo: &Path,
) -> Result<(Vec<AcceptanceOutcome>, Vec<ScenarioVerdict>)> {
    let path_str = spec_md.to_string_lossy().to_string();
    let repo_str = repo.to_string_lossy().to_string();
    let json = run_agent_spec_json(&[
        "lifecycle",
        &path_str,
        "--code",
        &repo_str,
        "--format",
        "json",
    ])?;
    let (outcomes, _passed) = outcomes_from_lifecycle(&json)?;
    let verdicts = verdicts_from_lifecycle(&json);
    Ok((outcomes, verdicts))
}

/// Run `agent-spec lifecycle <spec> --code <repo> --ai-mode caller` and return the report.
/// In caller mode, scenarios whose bound test couldn't verify mechanically are surfaced as
/// AI-pending (see `lifecycle_ai_pending`) for a reviewer agent to decide.
pub fn lifecycle_caller_run(spec_md: &Path, repo: &Path) -> Result<serde_json::Value> {
    let path_str = spec_md.to_string_lossy().to_string();
    let repo_str = repo.to_string_lossy().to_string();
    run_agent_spec_json(&[
        "lifecycle",
        &path_str,
        "--code",
        &repo_str,
        "--ai-mode",
        "caller",
        "--format",
        "json",
    ])
}

/// Merge a reviewer's decisions back via `agent-spec resolve-ai` and return the final report.
/// Writes the decisions JSON to a temp file alongside the spec.
pub fn resolve_ai_run(
    spec_md: &Path,
    repo: &Path,
    decisions: &[ReviewDecision],
) -> Result<serde_json::Value> {
    let dec_path = repo.join(".openfab-ai-decisions.json");
    std::fs::write(
        &dec_path,
        serde_json::to_string_pretty(&decisions_to_json(decisions))?,
    )
    .context("writing ai decisions")?;
    let path_str = spec_md.to_string_lossy().to_string();
    let repo_str = repo.to_string_lossy().to_string();
    let dec_str = dec_path.to_string_lossy().to_string();
    let out = run_agent_spec_json(&[
        "resolve-ai",
        &path_str,
        "--code",
        &repo_str,
        "--decisions",
        &dec_str,
        "--format",
        "json",
    ]);
    let _ = std::fs::remove_file(&dec_path);
    out
}

/// Whether OpenFab should route AI-pending scenarios to a reviewer (agent-spec caller mode):
/// `OPENFAB_REVIEW=caller`.
pub fn review_caller_enabled() -> bool {
    std::env::var("OPENFAB_REVIEW").as_deref() == Ok("caller")
}

/// Verify in caller mode: run agent-spec lifecycle with `--ai-mode caller`; for any AI-pending
/// scenario (design intent / quality a bound test can't verify), send the code to the reviewer
/// agent via the Bridge, merge its decisions with `resolve-ai`, and map the final report. This
/// is the layer where the reviewer's *code* judgment feeds OpenFab's gate — distinct from the
/// contract+sign-off layer. Falls back to mechanical lifecycle when no Bridge is configured.
pub fn verify_with_review(
    spec: &Spec,
    repo: &Path,
    bridge_url: &str,
    room: &str,
    changed_paths: &[String],
) -> Result<(Vec<AcceptanceOutcome>, Vec<ScenarioVerdict>)> {
    let spec_md = spec_md_path(&spec.id);
    let caller = lifecycle_caller_run(&spec_md, repo)?;

    let report = match lifecycle_ai_pending(&caller) {
        Some(req_file) if !bridge_url.is_empty() => {
            // Read the AI-pending requests and the implemented code, ask the reviewer to decide.
            let requests: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&req_file).unwrap_or_default())
                    .unwrap_or(serde_json::json!([]));
            let mut files = std::collections::BTreeMap::new();
            for p in changed_paths {
                if let Ok(content) = std::fs::read_to_string(repo.join(p)) {
                    files.insert(p.clone(), content);
                }
            }
            let decisions_json = crate::adapters::bridge_client::review_and_wait(
                bridge_url,
                &spec.spec_ref(),
                &requests,
                &files,
                room,
            )?;
            let decisions = parse_review_decisions(&decisions_json);
            resolve_ai_run(&spec_md, repo, &decisions)?
        }
        // No AI-pending scenarios (or no Bridge): the caller-mode report is already final.
        _ => caller,
    };
    let (outcomes, _passed) = outcomes_from_lifecycle(&report)?;
    let verdicts = verdicts_from_lifecycle(&report);
    Ok((outcomes, verdicts))
}

/// In-repo, portable location of the committed `.spec.md` contract (travels with the code).
pub fn repo_spec_md_path(repo: &Path, spec_id: &str) -> PathBuf {
    repo.join("specs").join(format!("{spec_id}.spec.md"))
}

/// Whether spec authoring/verification should go through agent-spec (`OPENFAB_SPEC=agent-spec`).
pub fn enabled() -> bool {
    std::env::var("OPENFAB_SPEC").as_deref() == Ok("agent-spec")
}

/// Slugify a name into a stable, filename-safe spec id.
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "spec".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `agent-spec parse --format json` payload (captured from agent-spec 0.3.0)
    /// for a standalone task spec with intent, decisions, boundaries, one scenario, and
    /// an out-of-scope section.
    fn sample_ast() -> serde_json::Value {
        serde_json::json!({
            "meta": { "level": "task", "name": "demo-temp-converter", "inherits": null, "lang": ["zh","en"], "tags": [] },
            "sections": [
                { "kind": "intent", "content": "CLI temperature converter: `convert.py N c2f|f2c` prints the converted value." },
                { "kind": "decisions", "items": ["Python 3 standard library only"] },
                { "kind": "boundaries", "items": [
                    { "text": "app/**", "category": "allow" },
                    { "text": "Do not add third-party dependencies", "category": "deny" }
                ]},
                { "kind": "acceptance_criteria", "scenarios": [
                    { "name": "c2f converts correctly",
                      "steps": [],
                      "test_selector": { "filter": "test_c2f", "package": "app" },
                      "tags": [] }
                ]},
                { "kind": "out_of_scope", "items": ["GUI"] }
            ]
        })
    }

    #[test]
    fn maps_intent_id_and_acceptance() {
        let c = parse_contract(&sample_ast(), "build a converter").unwrap();
        assert_eq!(c.spec.id, "demo-temp-converter");
        assert_eq!(
            c.spec.intent,
            "CLI temperature converter: `convert.py N c2f|f2c` prints the converted value."
        );
        assert_eq!(c.spec.acceptance.len(), 1);
        assert_eq!(c.spec.acceptance[0].id, "c2f converts correctly");
        // the check carries the agent-spec test selector (package::filter)
        assert!(c.spec.acceptance[0].check.contains("test_c2f"));
        assert!(c.spec.acceptance[0].check.contains("app"));
    }

    #[test]
    fn keeps_decisions_and_boundaries() {
        let c = parse_contract(&sample_ast(), "x").unwrap();
        assert_eq!(c.decisions, vec!["Python 3 standard library only"]);
        assert_eq!(c.allow, vec!["app/**"]);
        assert_eq!(c.deny, vec!["Do not add third-party dependencies"]);
    }

    #[test]
    fn out_of_scope_becomes_assumptions() {
        let c = parse_contract(&sample_ast(), "x").unwrap();
        assert!(c.spec.assumptions.iter().any(|a| a.contains("GUI")));
    }

    #[test]
    fn falls_back_to_intent_when_contract_intent_missing() {
        let ast = serde_json::json!({
            "meta": { "name": "no-intent-spec" },
            "sections": [
                { "kind": "acceptance_criteria", "scenarios": [
                    { "name": "s1", "steps": [], "test_selector": { "filter": "t", "package": "p" }, "tags": [] }
                ]}
            ]
        });
        let c = parse_contract(&ast, "the original NL ask").unwrap();
        assert_eq!(c.spec.intent, "the original NL ask");
    }

    #[test]
    fn test_requirements_sha256_helper_reads_file() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "# Requirements\n\n- add two integers\n";
        std::fs::write(tmp.path().join("demo.requirements.md"), body).unwrap();
        let got = requirements_sha256_in(tmp.path(), "demo");
        assert_eq!(got, Some(crate::core::sha256_hex(body.as_bytes())));
        // absent file → None
        assert_eq!(requirements_sha256_in(tmp.path(), "missing"), None);
    }

    #[test]
    fn slug_normalizes() {
        assert_eq!(slug("Demo Temp Converter!"), "demo-temp-converter");
        assert_eq!(slug("  already-slug  "), "already-slug");
    }

    fn lint_json(overall: f64, diagnostics: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "spec_name": "demo",
            "diagnostics": diagnostics,
            "quality_score": { "determinism": 1.0, "testability": 1.0, "coverage": 1.0, "overall": overall }
        })
    }

    #[test]
    fn lint_gate_passes_clean_high_score() {
        let j = lint_json(
            1.0,
            serde_json::json!([
                { "rule": "decision-coverage", "severity": "warning", "message": "minor" }
            ]),
        );
        let r = lint_gate(&j, 0.7).unwrap();
        assert_eq!(r.overall, 1.0);
        assert_eq!(r.warnings, 1);
        assert_eq!(r.errors, 0);
    }

    #[test]
    fn lint_gate_fails_below_min_score() {
        let j = lint_json(0.5, serde_json::json!([]));
        assert!(lint_gate(&j, 0.7).is_err());
    }

    #[test]
    fn lint_gate_fails_on_error_severity_even_if_score_high() {
        let j = lint_json(
            1.0,
            serde_json::json!([
                { "rule": "error-path", "severity": "error", "message": "no error scenario" }
            ]),
        );
        assert!(lint_gate(&j, 0.7).is_err());
    }

    #[test]
    fn draft_prompt_demands_standalone_task_contract() {
        let p = draft_prompt("build a temperature converter CLI");
        assert!(
            p.contains("spec: task"),
            "must show the .spec.md frontmatter"
        );
        assert!(p.contains("Scenario:"), "must require BDD scenarios");
        assert!(p.contains("Test:"), "must require bound test selectors");
        assert!(
            p.to_lowercase().contains("inherits"),
            "must instruct NOT to emit an inherits line"
        );
        assert!(
            p.contains("build a temperature converter CLI"),
            "must embed the intent"
        );
    }

    #[test]
    fn extract_strips_fences_and_inherits() {
        let raw = "Sure, here it is:\n```markdown\nspec: task\nname: \"x\"\ninherits: project\n---\n\n## Intent\n\nhi\n```\nDone.";
        let md = extract_spec_md(raw);
        assert!(
            md.starts_with("spec: task"),
            "starts at frontmatter, got: {md:?}"
        );
        assert!(md.contains("## Intent"));
        assert!(
            !md.contains("inherits"),
            "the inherits line must be removed"
        );
        assert!(!md.contains("```"), "fences must be stripped");
    }

    #[test]
    fn extract_finds_frontmatter_after_prose() {
        let raw = "Here you go:\nspec: task\nname: \"y\"\n---\n## Intent\nhello\n";
        let md = extract_spec_md(raw);
        assert!(md.starts_with("spec: task"));
    }

    #[test]
    fn lifecycle_maps_verdicts_skip_is_not_pass() {
        let json = serde_json::json!({
            "passed": false,
            "verification": { "results": [
                { "scenario_name": "c2f converts correctly", "verdict": "pass" },
                { "scenario_name": "rejects unknown unit", "verdict": "skip" }
            ]}
        });
        let (outcomes, passed) = outcomes_from_lifecycle(&json).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].id, "c2f converts correctly");
        assert!(outcomes[0].passed);
        assert_eq!(outcomes[0].exit_code, 0);
        assert!(!outcomes[1].passed, "skip must not count as pass");
        assert_eq!(outcomes[1].exit_code, 1);
        assert!(!passed);
    }

    #[test]
    fn lifecycle_all_pass_is_accepted() {
        let json = serde_json::json!({
            "passed": true,
            "verification": { "results": [
                { "scenario_name": "s1", "verdict": "pass" },
                { "scenario_name": "s2", "verdict": "pass" }
            ]}
        });
        let (outcomes, passed) = outcomes_from_lifecycle(&json).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(passed);
        assert!(outcomes.iter().all(|o| o.passed));
    }

    #[test]
    fn test_parse_ai_requests() {
        let json = serde_json::json!([
            { "scenario_name": "code is clean", "intent": "well-structured and idiomatic", "code_paths": ["src/main.rs"] },
            { "scenario_name": "handles edge", "intent": "covers the empty case" }
        ]);
        let items = parse_ai_requests(&json);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].scenario_name, "code is clean");
        assert_eq!(items[0].intent, "well-structured and idiomatic");
        assert_eq!(items[1].scenario_name, "handles edge");
    }

    #[test]
    fn test_cross_model_any_block() {
        let v = vec![
            CrossModelVerdict {
                model_family: "claude".into(),
                scenario: "s1".into(),
                verdict: "pass".into(),
            },
            CrossModelVerdict {
                model_family: "codex".into(),
                scenario: "s1".into(),
                verdict: "fail".into(),
            },
        ];
        assert!(cross_model_blocked(&v)); // codex objected → blocked
    }

    #[test]
    fn test_cross_model_all_pass() {
        let v = vec![
            CrossModelVerdict {
                model_family: "claude".into(),
                scenario: "s1".into(),
                verdict: "pass".into(),
            },
            CrossModelVerdict {
                model_family: "codex".into(),
                scenario: "s1".into(),
                verdict: "pass".into(),
            },
        ];
        assert!(!cross_model_blocked(&v));
        assert!(!cross_model_blocked(&[])); // nothing to object
    }

    #[test]
    fn test_cross_model_verdicts_json() {
        let v = vec![CrossModelVerdict {
            model_family: "claude".into(),
            scenario: "adds correctly".into(),
            verdict: "pass".into(),
        }];
        let j = cross_model_verdicts_json(&v);
        let arr = j.as_array().unwrap();
        assert_eq!(arr[0]["model_family"], "claude");
        assert_eq!(arr[0]["scenario"], "adds correctly");
        assert_eq!(arr[0]["verdict"], "pass");
    }

    #[test]
    fn test_decisions_to_json() {
        let decs = vec![
            ReviewDecision {
                scenario_name: "code is clean".into(),
                verdict: "pass".into(),
                confidence: 0.9,
                reasoning: "idiomatic".into(),
                model: "claude".into(),
            },
            ReviewDecision {
                scenario_name: "handles edge".into(),
                verdict: "fail".into(),
                confidence: 0.7,
                reasoning: "no empty check".into(),
                model: "".into(),
            },
        ];
        let v = decisions_to_json(&decs);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["scenario_name"], "code is clean");
        assert_eq!(arr[0]["verdict"], "pass");
        assert_eq!(arr[0]["model"], "claude");
        assert!(arr[0]["reasoning"].is_string() && arr[0]["confidence"].is_number());
        // empty model is defaulted (never serialized blank)
        assert_eq!(arr[1]["model"], "openfab-reviewer");
        // round-trips through the reviewer parser
        let back = parse_review_decisions(&v);
        assert_eq!(back.len(), 2);
        assert_eq!(back[1].verdict, "fail");
    }

    #[test]
    fn test_caller_outcomes_block_on_fail() {
        // a resolve-ai merged report with one fail → acceptance not passed (skip ≠ pass already
        // covered; here the AI verdict itself is fail).
        let resolved = serde_json::json!({
            "passed": false,
            "verification": { "results": [
                { "scenario_name": "adds correctly", "verdict": "pass" },
                { "scenario_name": "code is clean", "verdict": "fail" }
            ]}
        });
        let (outcomes, passed) = outcomes_from_lifecycle(&resolved).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(!passed);
        assert!(outcomes
            .iter()
            .any(|o| o.id == "code is clean" && !o.passed));
    }

    #[test]
    fn ai_pending_detected_for_reviewer_routing() {
        let none = serde_json::json!({ "passed": true });
        assert!(lifecycle_ai_pending(&none).is_none());

        let pending = serde_json::json!({
            "ai_pending": true,
            "ai_requests_file": ".agent-spec/pending-ai-requests.json"
        });
        assert_eq!(
            lifecycle_ai_pending(&pending).as_deref(),
            Some(".agent-spec/pending-ai-requests.json")
        );
    }

    #[test]
    fn verdicts_extracted_for_provenance() {
        let json = serde_json::json!({
            "verification": { "results": [
                { "scenario_name": "happy", "verdict": "pass" },
                { "scenario_name": "edge", "verdict": "skip" }
            ]}
        });
        let v = verdicts_from_lifecycle(&json);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].scenario, "happy");
        assert_eq!(v[0].verdict, "pass");
        assert_eq!(v[1].verdict, "skip");
    }

    #[test]
    fn lifecycle_fail_verdict_blocks() {
        let json = serde_json::json!({
            "verification": { "results": [
                { "scenario_name": "s1", "verdict": "fail" }
            ]}
        });
        // no top-level `passed` → computed from outcomes
        let (outcomes, passed) = outcomes_from_lifecycle(&json).unwrap();
        assert!(!outcomes[0].passed);
        assert!(!passed);
    }

    #[test]
    fn folded_spec_carries_decisions_and_boundaries() {
        let c = parse_contract(&sample_ast(), "x").unwrap();
        let spec = c.folded_spec();
        let joined = spec.assumptions.join(" | ");
        assert!(
            joined.contains("Python 3 standard library only"),
            "decision folded in"
        );
        assert!(joined.contains("app/**"), "allowed boundary folded in");
        assert!(
            joined.contains("Do not add third-party dependencies"),
            "forbidden folded in"
        );
        // original out-of-scope assumption preserved
        assert!(joined.contains("GUI"));
    }
}
