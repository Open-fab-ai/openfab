//! Operations layer — the spec-cycle actions shared by the CLI and the web API, so the
//! two front-ends never duplicate the orchestration logic (R3). Each function returns
//! plain data; the CLI prints it, the server JSON-encodes it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::adapters::registry;
use crate::adapters::sandbox;
use crate::core::identity::Identity;
use crate::core::provenance::Attestation;
use crate::core::spec::{Acceptance, Spec};
use crate::core::trust::{self, Policy, TrustInput};
use crate::core::{conformance, sha256_hex};
use crate::ports::forge::Trailers;
use crate::runstate::{self, RunRecord};
use crate::spec_cycle::{self, CycleConfig};

/// Inputs to start a run (from the CLI or the web form).
pub struct RunRequest {
    pub spec: Spec,
    pub base: String,
    pub forge_kind: String,
    pub forge_name: Option<String>,
    pub parent_run: Option<String>,
    pub run_id: Option<String>,
    /// Human-approval gate mode: "solo" | "team" | "crowd" | "none".
    pub gate_mode: String,
    /// "provider · model" when the spec was LLM-authored (shown in the timeline).
    pub authored_by: Option<String>,
}

/// Where the spec to build comes from (Phase 2 spec-driven ingest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecSource {
    /// Ingest a pre-authored `.spec.md` (e.g. from the wf_coordinator conversation).
    File(std::path::PathBuf),
    /// Draft a `.spec.md` via agent-spec + the LLM.
    AgentSpecDraft,
    /// Native LLM author (OpenFab's built-in JSON spec author).
    NativeLlm,
}

/// Pure source resolution (testable without env): explicit file > agent-spec draft > native.
pub fn resolve_spec_source(spec_file: Option<&str>, agent_spec_enabled: bool) -> SpecSource {
    match spec_file {
        Some(f) if !f.trim().is_empty() => SpecSource::File(std::path::PathBuf::from(f)),
        _ if agent_spec_enabled => SpecSource::AgentSpecDraft,
        _ => SpecSource::NativeLlm,
    }
}

/// Resolve the spec source from the environment.
pub fn spec_source() -> SpecSource {
    resolve_spec_source(
        std::env::var("OPENFAB_SPEC_FILE").ok().as_deref(),
        crate::adapters::agent_spec::enabled(),
    )
}

pub fn author_spec(intent: &str) -> Result<(Spec, String, String)> {
    author_spec_with_file(intent, None)
}

/// Author the spec, optionally ingesting an explicit `.spec.md` file (e.g. one the user
/// uploaded in the dashboard) which takes precedence over the env-selected source.
pub fn author_spec_with_file(
    intent: &str,
    spec_file: Option<&Path>,
) -> Result<(Spec, String, String)> {
    if intent.trim().len() < 4 {
        bail!("describe what you want to build");
    }
    // Spec source (Phase 2): an explicit `.spec.md` file (an uploaded contract, or the
    // wf_coordinator requirements conversation) takes precedence over the agent-spec LLM
    // draft, which takes precedence over the native LLM author.
    let spec_dir = std::env::var("OPENFAB_SPEC_DIR").unwrap_or_else(|_| "specs".to_string());
    let source = match spec_file {
        Some(p) => SpecSource::File(p.to_path_buf()),
        None => spec_source(),
    };
    match source {
        SpecSource::File(path) => {
            let md = std::fs::read_to_string(&path)
                .with_context(|| format!("reading OPENFAB_SPEC_FILE {}", path.display()))?;
            let authored = crate::adapters::agent_spec::author_from_md(
                &md,
                intent,
                Path::new(&spec_dir),
                "ingested".to_string(),
                "coordinator".to_string(),
            )?;
            let spec = authored.contract.folded_spec();
            spec.validate().context("the ingested spec was invalid")?;
            return Ok((spec, authored.model, authored.provider));
        }
        SpecSource::AgentSpecDraft => {
            let authored =
                crate::adapters::agent_spec::author_via_agent_spec(intent, Path::new(&spec_dir))?;
            let spec = authored.contract.folded_spec();
            spec.validate()
                .context("the agent-spec-authored spec was invalid")?;
            return Ok((spec, authored.model, authored.provider));
        }
        SpecSource::NativeLlm => {}
    }
    let (a, model, provider) = crate::adapters::llm_backend::author_spec(intent)?;
    let spec = Spec {
        id: slug(intent),
        version: 1,
        intent: intent.trim().to_string(),
        context: vec![],
        acceptance: a
            .acceptance
            .into_iter()
            .map(|c| Acceptance {
                id: c.id,
                check: c.check,
                must_pass: true,
            })
            .collect(),
        assumptions: a.assumptions,
        open_questions: a.open_questions,
        human_signoff_required: true,
        target_dir: a.target_dir,
        language: Some(a.language),
    };
    spec.validate()
        .context("the LLM-authored spec was invalid")?;
    Ok((spec, model, provider))
}

/// One-shot: author a spec from NL intent (LLM) and build it. `run_id` is pre-reserved by
/// the caller so the web API can return immediately while the LLM authors in the background.
#[allow(clippy::too_many_arguments)]
pub fn build(
    repo: &Path,
    intent: &str,
    run_id: String,
    base: &str,
    forge_kind: &str,
    forge_name: Option<String>,
    gate_mode: &str,
    policy: &Policy,
) -> Result<RunRecord> {
    build_with_spec_file(
        repo, intent, run_id, base, forge_kind, forge_name, gate_mode, policy, None,
    )
}

/// `build`, optionally ingesting an explicit uploaded `.spec.md` (Phase 2.1 #2).
#[allow(clippy::too_many_arguments)]
pub fn build_with_spec_file(
    repo: &Path,
    intent: &str,
    run_id: String,
    base: &str,
    forge_kind: &str,
    forge_name: Option<String>,
    gate_mode: &str,
    policy: &Policy,
    spec_file: Option<&Path>,
) -> Result<RunRecord> {
    let (spec, model, provider) = author_spec_with_file(intent, spec_file)?;
    start_run(
        repo,
        RunRequest {
            spec,
            base: base.to_string(),
            forge_kind: forge_kind.to_string(),
            forge_name,
            parent_run: None,
            run_id: Some(run_id),
            gate_mode: gate_mode.to_string(),
            authored_by: Some(format!("{provider} · {model}")),
        },
        policy,
    )
}

/// Refine: fold the human's feedback into the intent, **re-author the spec (fresh
/// acceptance criteria)** so the new requirement is actually captured + tested, and
/// rebuild as v→v+1. This is why a refine genuinely changes the software (issue: a refine
/// that kept the old acceptance just rebuilt to the same contract).
pub fn refine(
    repo: &Path,
    prior_run: &str,
    note: &str,
    run_id: String,
    base: &str,
    policy: &Policy,
) -> Result<RunRecord> {
    let prior = runstate::load_run(repo, prior_run)?;
    let prior_spec = Spec::from_yaml(&runstate::load_run_spec_yaml(repo, prior_run)?)?;
    let combined = format!(
        "{}\n\nRevision requested by the human: {}",
        prior_spec.intent.trim(),
        note.trim()
    );
    let (mut spec, model, provider) = author_spec(&combined)?;
    spec.id = prior_spec.id.clone();
    spec.version = prior_spec.version + 1;
    spec.intent = combined;
    start_run(
        repo,
        RunRequest {
            spec,
            base: base.to_string(),
            forge_kind: effective_forge_kind(&prior),
            forge_name: Some(prior.forge_name.clone()),
            parent_run: Some(prior.run_id.clone()),
            run_id: Some(run_id),
            gate_mode: prior.gate_mode.clone(),
            authored_by: Some(format!(
                "{provider} · {model} (re-authored from your feedback)"
            )),
        },
        policy,
    )
}

/// One stage of the spec-cycle pipeline for the dashboard process-detail view (C1).
#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    pub key: String,
    pub label: String,
    /// "done" | "active" | "pending" | "failed".
    pub state: String,
}

/// Ordered pipeline stages and a substring marker that, if present in any event message,
/// means the stage was reached.
const STAGE_MARKERS: &[(&str, &str, &str)] = &[
    ("spec", "Spec", "compiled into"),
    ("implement", "Implement", "base '"),
    ("verify", "Verify", "acceptance check"),
    ("sign", "Sign", "signed in-toto"),
    ("gate", "Gate", "trust gate"),
];

/// Derive the stage pipeline from a run's events + status (pure; the UI renders it).
pub fn derive_stages(event_msgs: &[String], status: &str) -> Vec<Stage> {
    let merged = status == "merged";
    let failed = status == "failed";
    let mut stages: Vec<Stage> = STAGE_MARKERS
        .iter()
        .map(|(key, label, marker)| {
            let done = merged || event_msgs.iter().any(|m| m.contains(marker));
            Stage {
                key: key.to_string(),
                label: label.to_string(),
                state: if done { "done" } else { "pending" }.to_string(),
            }
        })
        .collect();
    stages.push(Stage {
        key: "merge".into(),
        label: "Merge".into(),
        state: if merged { "done" } else { "pending" }.into(),
    });
    // The first pending stage becomes "active" (or "failed" if the run failed there).
    if let Some(s) = stages.iter_mut().find(|s| s.state == "pending") {
        s.state = if failed { "failed" } else { "active" }.into();
    }
    stages
}

/// Project a run's status/flags onto a kanban board lane (D1). `blocked` means the work is
/// done (implemented, verified, signed) and only the human N-of-M release sign-off remains —
/// so the lane is "sign-off", not "review" (the code review/verification already happened).
pub fn board_lane(status: &str, accepted: bool, merged: bool) -> &'static str {
    match status {
        "merged" => "merged",
        _ if merged => "merged",
        "accepted" => "accepted",
        _ if accepted => "accepted",
        "blocked" => "sign-off",
        "failed" => "failed",
        "rejected" => "rejected",
        "running" => "implementing",
        _ => "implementing",
    }
}

/// Render the multi-spec dependency graph (DOT) via `agent-spec graph` (D2).
pub fn spec_graph(spec_dir: &Path) -> Result<String> {
    let bin = std::env::var("OPENFAB_AGENT_SPEC_BIN").unwrap_or_else(|_| "agent-spec".to_string());
    let out = Command::new(&bin)
        .args(["graph", "--spec-dir", &spec_dir.to_string_lossy()])
        .output()
        .with_context(|| format!("running `{bin} graph` — is agent-spec installed?"))?;
    if !out.status.success() {
        bail!(
            "agent-spec graph failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// An uploaded document is either an agent-spec contract or a requirements doc (Phase 2.1).
pub fn classify_upload(name: &str, body: &str) -> &'static str {
    let trimmed = body.trim_start();
    if name.to_lowercase().ends_with(".spec.md") || trimmed.starts_with("spec:") {
        "spec"
    } else {
        "requirements"
    }
}

/// Destination filename for an uploaded doc in the project's spec dir: `<slug>.spec.md` or
/// `<slug>.requirements.md`. The slug is filesystem-safe (no separators/traversal).
pub fn upload_dest_name(name: &str, kind: &str) -> String {
    let base = name
        .trim_end_matches(".spec.md")
        .trim_end_matches(".requirements.md");
    let id = slug(base);
    if kind == "spec" {
        format!("{id}.spec.md")
    } else {
        format!("{id}.requirements.md")
    }
}

/// Save an uploaded document into the project's spec dir; returns (id, kind, dest path).
pub fn save_upload(spec_dir: &Path, name: &str, body: &str) -> Result<(String, String, PathBuf)> {
    let kind = classify_upload(name, body).to_string();
    let fname = upload_dest_name(name, &kind);
    let id = fname
        .trim_end_matches(".spec.md")
        .trim_end_matches(".requirements.md")
        .to_string();
    std::fs::create_dir_all(spec_dir)
        .with_context(|| format!("creating spec dir {}", spec_dir.display()))?;
    let dest = spec_dir.join(&fname);
    std::fs::write(&dest, body).with_context(|| format!("writing {}", dest.display()))?;
    Ok((id, kind, dest))
}

/// Stage pipeline for one run (reads its events + status).
pub fn stages(repo: &Path, run: &str) -> Result<Vec<Stage>> {
    let rec = runstate::load_run(repo, run)?;
    let msgs: Vec<String> = runstate::read_events(repo, run, 0)
        .into_iter()
        .map(|e| e.msg)
        .collect();
    Ok(derive_stages(&msgs, &rec.status))
}

/// One board card: a run with its derived lane + key links (D1).
#[derive(Debug, Clone, Serialize)]
pub struct BoardItem {
    pub run_id: String,
    pub spec_ref: String,
    pub lane: String,
    pub base_name: String,
    pub pr_url: String,
    pub created: String,
}

/// Project all runs onto the kanban board lanes.
pub fn board(repo: &Path) -> Result<Vec<BoardItem>> {
    Ok(runstate::list_runs(repo)?
        .into_iter()
        .map(|r| BoardItem {
            lane: board_lane(&r.status, r.accepted, r.merged).to_string(),
            run_id: r.run_id,
            spec_ref: r.spec_ref,
            base_name: r.base_name,
            pr_url: r.pr_url,
            created: r.created,
        })
        .collect())
}

/// A run document for the dashboard (Phase 2 A3 document engineering).
#[derive(Debug, Clone, Serialize)]
pub struct Doc {
    pub name: String,
    pub kind: String,
    pub content: String,
}

/// Classify a committed file path into a document kind for the dashboard.
pub fn classify_doc_kind(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.ends_with(".requirements.md") {
        "requirements"
    } else if n.ends_with(".spec.md") || n.ends_with(".spec.yaml") {
        "spec"
    } else if n.ends_with(".design.md") {
        "design"
    } else if n.ends_with("readme.md") {
        "readme"
    } else if n.starts_with("provenance/") {
        "provenance"
    } else {
        "code"
    }
}

/// The document bundle for a run: requirements + spec contract + design + code + readme,
/// classified and read from the committed repo (so docs travel with the product).
pub fn docs(repo: &Path, run: &str) -> Result<Vec<Doc>> {
    let rec = runstate::load_run(repo, run)?;
    let spec = Spec::from_yaml(&runstate::load_run_spec_yaml(repo, run)?)?;
    let id = &spec.id;
    let mut out = Vec::new();
    let mut add = |rel: &str| {
        if let Ok(content) = std::fs::read_to_string(repo.join(rel)) {
            out.push(Doc {
                name: rel.to_string(),
                kind: classify_doc_kind(rel).to_string(),
                content,
            });
        }
    };
    add(&format!("specs/{id}.requirements.md"));
    add(&format!("specs/{id}.spec.md"));
    add(&format!("specs/{id}.design.md"));
    add("README.md");
    // generated code files (from the signed attestation's per-file attribution).
    if let Ok(text) = std::fs::read_to_string(rec.attestation_path(repo)) {
        if let Ok(att) = Attestation::from_json(&text) {
            for g in &att.statement.predicate.generated {
                add(&g.path);
            }
        }
    }
    Ok(out)
}

/// Reserve a run id for a fresh NL build (no LLM call needed — derived from the intent).
pub fn reserve_intent_run_id(intent: &str) -> String {
    format!("{}-v1-{}", slug(intent), crate::core::timeutil::unix_now())
}

/// Reserve a run id for a refine of `prior_run` (next version of the same spec).
pub fn reserve_refine_run_id(repo: &Path, prior_run: &str) -> Result<String> {
    let prior_spec = Spec::from_yaml(&runstate::load_run_spec_yaml(repo, prior_run)?)?;
    Ok(format!(
        "{}-v{}-{}",
        prior_spec.id,
        prior_spec.version + 1,
        crate::core::timeutil::unix_now()
    ))
}

/// A short, file-system-safe id derived from the intent.
fn slug(intent: &str) -> String {
    let mut s: String = intent
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let short: String = s.chars().take(40).collect();
    let short = short.trim_matches('-').to_string();
    if short.is_empty() {
        "app".to_string()
    } else {
        short
    }
}

/// Reserve a run id before launching (so the web API can return immediately and poll).
pub fn reserve_run_id(spec: &Spec) -> String {
    format!(
        "{}-v{}-{}",
        spec.id,
        spec.version,
        crate::core::timeutil::unix_now()
    )
}

/// Run one full spec cycle (blocking). The server calls this on a background thread.
pub fn start_run(repo: &Path, req: RunRequest, policy: &Policy) -> Result<RunRecord> {
    let fab = Identity::load_or_create(&runstate::fab_identity_dir(repo), "fab")?;
    runstate::ensure_fab_allowlisted(repo, &fab.did())?;

    let forge = registry::build_forge(&req.forge_kind, req.forge_name.clone(), repo)?;
    forge.clone_repo(repo)?;
    let base = registry::build_base(&req.base, policy)?;

    // The human-approval gate is a policy choice per run (solo / team / crowd / none).
    let gate_policy = policy.for_gate_mode(&req.gate_mode);
    std::fs::create_dir_all(runstate::openfab_dir(repo))?;
    std::fs::write(
        runstate::openfab_dir(repo).join("policy.effective.json"),
        gate_policy.to_json()?,
    )?;

    spec_cycle::run_cycle(CycleConfig {
        spec: &req.spec,
        base: base.as_ref(),
        forge: forge.as_ref(),
        fab: &fab,
        policy: &gate_policy,
        parent_run: req.parent_run,
        run_id: req.run_id,
        gate_mode: req.gate_mode.clone(),
        authored_by: req.authored_by.clone(),
    })
}

/// The forge adapter kind for a run, tolerant of records that predate `forge_kind`.
pub fn effective_forge_kind(rec: &RunRecord) -> String {
    if !rec.forge_kind.is_empty() {
        rec.forge_kind.clone()
    } else if rec.forge_name == "github" {
        "github".to_string()
    } else {
        "local".to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SignoffOutcome {
    pub signer_name: String,
    pub signer_did: String,
    pub accepted: bool,
    pub merged: bool,
    pub status: String,
    pub satisfied: Vec<String>,
    pub blocking: Vec<String>,
}

/// A maintainer signs off; on N-of-M the gate opens and (for a local forge) the PR merges.
/// Sign off as the maintainer mapped to a Matrix user id (Phase 2: Robrix approval relay).
/// Rejects any mxid not mapped to exactly one allowlisted maintainer (see `resolve_signer`).
pub fn signoff_by_mxid(
    repo: &Path,
    run: &str,
    mxid: &str,
    policy: &Policy,
) -> Result<SignoffOutcome> {
    let maintainers = runstate::load_maintainers(repo)?;
    let name = runstate::resolve_signer(mxid, &maintainers)?.name.clone();
    signoff(repo, run, &name, policy)
}

pub fn signoff(repo: &Path, run: &str, as_name: &str, policy: &Policy) -> Result<SignoffOutcome> {
    let mut rec = runstate::load_run(repo, run)?;
    let maint_dids = runstate::maintainer_dids(repo)?;
    let signer = runstate::load_maintainer_identity(repo, as_name)?;
    if !maint_dids.iter().any(|d| d == &signer.did()) {
        bail!("'{as_name}' is not a registered maintainer — register with maintainer-add first");
    }

    let att_abs = rec.attestation_path(repo);
    let mut att = Attestation::from_json(&std::fs::read_to_string(&att_abs)?)?;
    att.add_signoff(&signer)?;
    std::fs::write(&att_abs, att.to_json()?)?;

    let kind = effective_forge_kind(&rec);
    let forge = registry::build_forge(&kind, Some(rec.forge_name.clone()), repo)?;
    forge.branch(&rec.branch)?;
    forge.commit(
        std::slice::from_ref(&att_abs),
        &format!("chore: record sign-off by {as_name}"),
        &Trailers::new().with("OpenFab-Signoff", &signer.did()),
    )?;

    let gate_policy = policy.for_gate_mode(&rec.gate_mode);
    let decision = trust::evaluate(
        &gate_policy,
        &TrustInput {
            att: &att,
            fab_allowlist: &runstate::fab_allowlist(repo)?,
            maintainer_allowlist: &maint_dids,
            base_name: &rec.base_name,
            acceptance_passed: rec.acceptance_passed,
        },
    );

    rec.accepted = decision.accepted;
    if decision.accepted && !rec.merged {
        // Local-instance forges (the offline demo path) merge here; a *live* remote
        // forge defers the merge to its own UI/API after the gate opens.
        if registry::is_local_instance(&kind) {
            local_merge(repo, &rec.branch)?;
            rec.merged = true;
            rec.status = "merged".to_string();
        } else {
            rec.status = "accepted".to_string();
        }
    } else if !decision.accepted {
        rec.status = "blocked".to_string();
    }

    let spec_yaml = runstate::load_run_spec_yaml(repo, run)?;
    let timeline = std::fs::read_to_string(runstate::run_dir(repo, run).join("timeline.md"))
        .unwrap_or_default();
    runstate::save_run(repo, &rec, &spec_yaml, &timeline)?;
    runstate::write_status(
        repo,
        &runstate::StatusFile {
            run_id: rec.run_id.clone(),
            spec_ref: rec.spec_ref.clone(),
            status: rec.status.clone(),
            step: "signoff".into(),
            updated: crate::core::timeutil::iso_now(),
            error: None,
        },
    );

    Ok(SignoffOutcome {
        signer_name: as_name.to_string(),
        signer_did: signer.did(),
        accepted: decision.accepted,
        merged: rec.merged,
        status: rec.status,
        satisfied: decision.satisfied,
        blocking: decision.blocking,
    })
}

/// Reject a run: the human declines to approve it (the PR is not merged). The branch +
/// its provenance stay in git as a record; the human can then refine (v→v+1) instead.
pub fn reject(repo: &Path, run: &str) -> Result<RunRecord> {
    let mut rec = runstate::load_run(repo, run)?;
    if rec.merged {
        bail!("run already merged — cannot reject");
    }
    rec.accepted = false;
    rec.status = "rejected".to_string();
    let spec_yaml = runstate::load_run_spec_yaml(repo, run)?;
    let timeline = std::fs::read_to_string(runstate::run_dir(repo, run).join("timeline.md"))
        .unwrap_or_default();
    runstate::save_run(repo, &rec, &spec_yaml, &timeline)?;
    runstate::write_status(
        repo,
        &runstate::StatusFile {
            run_id: rec.run_id.clone(),
            spec_ref: rec.spec_ref.clone(),
            status: "rejected".into(),
            step: "rejected".into(),
            updated: crate::core::timeutil::iso_now(),
            error: None,
        },
    );
    Ok(rec)
}

/// Merge a branch into main on the local-git forge (the demo's "merge the PR").
fn local_merge(repo: &Path, branch: &str) -> Result<()> {
    let git = |args: &[&str]| -> Result<()> {
        let out = Command::new("git").args(args).current_dir(repo).output()?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    };
    git(&["checkout", "-q", "main"])?;
    git(&[
        "merge",
        "--no-ff",
        "-q",
        branch,
        "-m",
        &format!("merge {branch} (OpenFab gate accepted)"),
    ])?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutcome {
    pub spec_ref: String,
    pub conformant: bool,
    pub accepted: bool,
    pub merged: bool,
    pub checks: Vec<conformance::CheckResult>,
}

/// Verify an artifact against the OpenFab profile (signatures + acceptance + sign-off).
pub fn verify(repo: &Path, run: &str) -> Result<VerifyOutcome> {
    let rec = runstate::load_run(repo, run)?;
    let spec = Spec::from_yaml(&runstate::load_run_spec_yaml(repo, run)?)?;
    let att = Attestation::from_json(&std::fs::read_to_string(rec.attestation_path(repo))?)
        .context("loading committed attestation")?;
    let report = conformance::check(&att, spec.human_signoff_required);
    let decision = trust::evaluate(
        &Policy::default().for_gate_mode(&rec.gate_mode),
        &TrustInput {
            att: &att,
            fab_allowlist: &runstate::fab_allowlist(repo)?,
            maintainer_allowlist: &runstate::maintainer_dids(repo)?,
            base_name: &rec.base_name,
            acceptance_passed: rec.acceptance_passed,
        },
    );
    Ok(VerifyOutcome {
        spec_ref: rec.spec_ref,
        conformant: report.conformant,
        accepted: decision.accepted,
        merged: rec.merged,
        checks: report.checks,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecOutput {
    pub cmd: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run an ad-hoc command against a run's generated software, in the policy-gated sandbox
/// (same allow/deny rules as acceptance). Lets a human *try* the product before refining.
pub fn exec_in_run(repo: &Path, run: &str, cmd: &str, policy: &Policy) -> Result<ExecOutput> {
    if cmd.trim().is_empty() {
        bail!("empty command");
    }
    let rec = runstate::load_run(repo, run)?;
    // Ensure the run's source is in the working tree.
    let forge = registry::build_forge(
        &effective_forge_kind(&rec),
        Some(rec.forge_name.clone()),
        repo,
    )?;
    forge.branch(&rec.branch)?;
    let command = vec!["bash".to_string(), "-c".to_string(), cmd.to_string()];
    let exec = sandbox::exec_gated_timeout(policy, &command, repo, sandbox::TRY_TIMEOUT_SECS)?;
    Ok(ExecOutput {
        cmd: cmd.to_string(),
        exit_code: exec.exit_code,
        stdout: exec.stdout,
        stderr: exec.stderr,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ReproduceCheck {
    pub id: String,
    pub check: String,
    pub passed: bool,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReproduceOutcome {
    pub run_id: String,
    pub signature_valid: bool,
    pub source_identical: bool,
    pub all_acceptance_passed: bool,
    pub reproducible: bool,
    pub checks: Vec<ReproduceCheck>,
    pub files_checked: usize,
}

/// Independently reproduce a product: re-verify its signature, confirm the committed
/// source matches the signed digests bit-for-bit, and re-run every acceptance check in
/// the sandbox. This is the sovereign/air-gapped proof — "don't trust, verify".
pub fn reproduce(repo: &Path, run: &str, policy: &Policy) -> Result<ReproduceOutcome> {
    let rec = runstate::load_run(repo, run)?;
    let spec = Spec::from_yaml(&runstate::load_run_spec_yaml(repo, run)?)?;
    let att = Attestation::from_json(&std::fs::read_to_string(rec.attestation_path(repo))?)?;
    let signature_valid = att.verify_signatures().is_ok();

    // Check out the run's source so the working tree holds exactly what was attested.
    let forge = registry::build_forge(
        &effective_forge_kind(&rec),
        Some(rec.forge_name.clone()),
        repo,
    )?;
    forge.branch(&rec.branch)?;

    // Each generated file must hash-match its recorded digest (bit-identical source).
    let mut source_identical = true;
    let mut files_checked = 0;
    for g in &att.statement.predicate.generated {
        files_checked += 1;
        match std::fs::read(repo.join(&g.path)) {
            Ok(bytes) if sha256_hex(&bytes) == g.sha256 => {}
            _ => source_identical = false,
        }
    }

    // Re-run the contract. If the spec was authored via agent-spec (the attestation records
    // a contract hash), re-verify by re-running `agent-spec lifecycle` against the committed
    // `.spec.md`; otherwise re-run the native sandbox acceptance commands.
    let mut checks = vec![];
    let mut all_passed = true;
    if att.statement.predicate.spec_contract_sha256.is_some() {
        let spec_md = crate::adapters::agent_spec::repo_spec_md_path(repo, &spec.id);
        let (outcomes, _verdicts) = crate::adapters::agent_spec::lifecycle_run(&spec_md, repo)?;
        for o in &outcomes {
            if !o.passed {
                all_passed = false;
            }
            checks.push(ReproduceCheck {
                id: o.id.clone(),
                check: o.check.clone(),
                passed: o.passed,
                exit_code: o.exit_code,
            });
        }
    } else {
        for a in &spec.acceptance {
            let cmd = vec!["bash".to_string(), "-c".to_string(), a.check.clone()];
            let exec = sandbox::exec_gated(policy, &cmd, repo)?;
            if a.must_pass && !exec.passed() {
                all_passed = false;
            }
            checks.push(ReproduceCheck {
                id: a.id.clone(),
                check: a.check.clone(),
                passed: exec.passed(),
                exit_code: exec.exit_code,
            });
        }
    }

    Ok(ReproduceOutcome {
        run_id: rec.run_id,
        signature_valid,
        source_identical,
        all_acceptance_passed: all_passed,
        reproducible: signature_valid && source_identical && all_passed,
        checks,
        files_checked,
    })
}

/// List committed artifact contents for a run (attestation, SBOM, generated files, log).
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactBundle {
    pub run: RunRecord,
    pub spec: serde_json::Value,
    pub attestation: serde_json::Value,
    pub sbom: serde_json::Value,
    pub files: Vec<ArtifactFile>,
    pub timeline: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactFile {
    pub path: String,
    pub contents: String,
    pub sha256: String,
    pub author: String,
}

pub fn artifacts(repo: &Path, run: &str) -> Result<ArtifactBundle> {
    let rec = runstate::load_run(repo, run)?;
    let att_text = std::fs::read_to_string(rec.attestation_path(repo))?;
    let attestation: serde_json::Value = serde_json::from_str(&att_text)?;
    let sbom: serde_json::Value = std::fs::read_to_string(repo.join(&rec.sbom_repo_path))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    let att = Attestation::from_json(&att_text)?;
    let mut files = vec![];
    for g in &att.statement.predicate.generated {
        let contents = std::fs::read_to_string(repo.join(&g.path)).unwrap_or_default();
        files.push(ArtifactFile {
            path: g.path.clone(),
            contents,
            sha256: g.sha256.clone(),
            author: g.author.clone(),
        });
    }
    let timeline = std::fs::read_to_string(runstate::run_dir(repo, run).join("timeline.md"))
        .unwrap_or_default();
    let spec: serde_json::Value = serde_yaml::from_str(&runstate::load_run_spec_yaml(repo, run)?)
        .unwrap_or(serde_json::Value::Null);
    Ok(ArtifactBundle {
        run: rec,
        spec,
        attestation,
        sbom,
        files,
        timeline,
    })
}

// --- audit trail (the portable, third-party-verifiable git history) ---

#[derive(Debug, Clone, Serialize)]
pub struct AuditCommit {
    pub short: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub refs: String,
    pub trailers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThirdPartyCmd {
    pub tool: String,
    pub purpose: String,
    pub cmd: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditTrail {
    pub run_id: String,
    pub forge_kind: String,
    pub branch: String,
    pub merged: bool,
    pub graph_ascii: String,
    pub commits: Vec<AuditCommit>,
    pub third_party: Vec<ThirdPartyCmd>,
    pub compliance_note: String,
}

fn git_out(repo: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// The auditable git history for a run: the commit graph + each commit's provenance
/// trailers, plus the exact commands a third party uses to inspect/verify it *without*
/// OpenFab. This is the "trustworthy · reproducible · auditable" evidence, in standard,
/// portable formats (git + in-toto/SLSA JSON + SPDX) — viewable on any forge or git tool.
pub fn audit(repo: &Path, run: &str) -> Result<AuditTrail> {
    let rec = runstate::load_run(repo, run)?;
    let graph_ascii = git_out(
        repo,
        &["log", "--graph", "--oneline", "--decorate", "-n", "24"],
    );

    // One log call: metadata + trailers per commit, delimited by control chars.
    let fmt = "%x1eCOMMIT%x1f%h%x1f%s%x1f%an%x1f%ad%x1f%D%x1fTRAILERS%x1f%(trailers:only,unfold,separator=%x1f)";
    let raw = git_out(
        repo,
        &[
            "log",
            &rec.branch,
            "-n",
            "40",
            "--date=short",
            &format!("--pretty=format:{fmt}"),
        ],
    );
    let mut commits = vec![];
    for chunk in raw.split('\u{1e}') {
        let chunk = chunk.trim_start_matches("COMMIT\u{1f}");
        if chunk.trim().is_empty() {
            continue;
        }
        // Fields: [%h, %s, %an, %ad, %D, "TRAILERS", <trailer>, <trailer>, …]
        let parts: Vec<&str> = chunk.split('\u{1f}').collect();
        if parts.len() < 6 {
            continue;
        }
        let trailers = parts
            .iter()
            .skip(6)
            .filter_map(|t| {
                t.split_once(": ")
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        commits.push(AuditCommit {
            short: parts[0].to_string(),
            subject: parts[1].to_string(),
            author: parts[2].to_string(),
            date: parts[3].to_string(),
            refs: parts[4].trim().to_string(),
            trailers,
        });
    }

    let repo_disp = repo.display();
    let att = &rec.attestation_repo_path;
    let sbom = &rec.sbom_repo_path;
    let third_party = vec![
        ThirdPartyCmd {
            tool: "git · gitk · VS Code Git Graph · GitHub/Gitea/Forgejo web UI".into(),
            purpose: "view the signed commit graph + provenance trailers (AI authorship, sign-offs, attestation hash)".into(),
            cmd: format!("git -C {repo_disp} log --graph --decorate --format=full"),
        },
        ThirdPartyCmd {
            tool: "jq · any JSON tool · in-toto / SLSA verifiers".into(),
            purpose: "read the in-toto/SLSA attestation (predicate openfab/generation)".into(),
            cmd: format!("jq . {repo_disp}/{att}"),
        },
        ThirdPartyCmd {
            tool: "SPDX tools · syft".into(),
            purpose: "inspect the SBOM (SPDX 2.3)".into(),
            cmd: format!("jq .files {repo_disp}/{sbom}"),
        },
        ThirdPartyCmd {
            tool: "cosign (Sigstore) — production verify path".into(),
            purpose: "verify the signature against a transparency log (v0.2 swap for did:key)".into(),
            cmd: format!("cosign verify-blob --bundle <bundle> {repo_disp}/{att}"),
        },
        ThirdPartyCmd {
            tool: "slsa-verifier — production verify path".into(),
            purpose: "verify SLSA provenance for the built artifact".into(),
            cmd: "slsa-verifier verify-artifact <artifact> --provenance-path <att>".into(),
        },
    ];

    Ok(AuditTrail {
        run_id: rec.run_id,
        forge_kind: rec.forge_kind,
        branch: rec.branch,
        merged: rec.merged,
        graph_ascii,
        commits,
        third_party,
        compliance_note: "Every action — the AI's authorship and each human sign-off — is a \
            signed git commit carrying in-toto/SLSA provenance trailers, and the merge is gated \
            on N-of-M human approval. The trail is portable (plain git + JSON, committed in-repo) \
            and verifiable by third-party tools on any forge — the kind of tamper-evident, \
            attributable provenance the EU Cyber Resilience Act (CRA) and SLSA expect."
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_stages_marks_done_from_events() {
        let msgs = vec![
            "spec demo#v1 compiled into 1 task-card(s)".to_string(),
            "base 'claude' (m) → implemented".to_string(),
            "sandbox = x; running 2 acceptance check(s)".to_string(),
            "signed in-toto/SLSA attestation".to_string(),
        ];
        let stages = derive_stages(&msgs, "blocked");
        let by = |k: &str| stages.iter().find(|s| s.key == k).unwrap().state.clone();
        assert_eq!(by("spec"), "done");
        assert_eq!(by("implement"), "done");
        assert_eq!(by("verify"), "done");
        assert_eq!(by("sign"), "done");
        assert_eq!(by("gate"), "active");
        assert_eq!(by("merge"), "pending");
    }

    #[test]
    fn test_derive_stages_merged_completes() {
        let stages = derive_stages(&[], "merged");
        assert!(stages.iter().all(|s| s.state == "done"));
    }

    #[test]
    fn test_classify_upload_spec() {
        assert_eq!(classify_upload("x.spec.md", "spec: task\nname: y"), "spec");
        assert_eq!(classify_upload("anything.txt", "spec:\n..."), "spec");
    }

    #[test]
    fn test_classify_upload_requirements() {
        assert_eq!(
            classify_upload("notes.md", "# Requirements\n\nWe need a CLI…"),
            "requirements"
        );
        assert_eq!(classify_upload("doc", "just prose"), "requirements");
    }

    #[test]
    fn test_upload_dest_name() {
        assert_eq!(
            upload_dest_name("My Spec.spec.md", "spec"),
            "my-spec.spec.md"
        );
        assert_eq!(
            upload_dest_name("Payment Flow", "requirements"),
            "payment-flow.requirements.md"
        );
        // traversal/separators are slugged away
        assert!(!upload_dest_name("../../etc/passwd", "requirements").contains('/'));
    }

    #[test]
    fn test_board_lane_from_status() {
        assert_eq!(board_lane("running", false, false), "implementing");
        // blocked = work done, only the human release sign-off remains.
        assert_eq!(board_lane("blocked", false, false), "sign-off");
        assert_eq!(board_lane("merged", true, true), "merged");
    }

    #[test]
    fn test_classify_doc_kind_spec_and_requirements() {
        assert_eq!(classify_doc_kind("specs/x.spec.md"), "spec");
        assert_eq!(classify_doc_kind("specs/x.requirements.md"), "requirements");
    }

    #[test]
    fn test_classify_doc_kind_code() {
        assert_eq!(classify_doc_kind("src/main.rs"), "code");
        assert_eq!(classify_doc_kind("Cargo.toml"), "code");
        assert_eq!(classify_doc_kind("README.md"), "readme");
    }

    #[test]
    fn test_spec_source_prefers_file() {
        assert_eq!(
            resolve_spec_source(Some("specs/x.spec.md"), true),
            SpecSource::File("specs/x.spec.md".into())
        );
        // empty file value is ignored
        assert_eq!(
            resolve_spec_source(Some("  "), true),
            SpecSource::AgentSpecDraft
        );
        assert_eq!(resolve_spec_source(None, true), SpecSource::AgentSpecDraft);
        assert_eq!(resolve_spec_source(None, false), SpecSource::NativeLlm);
    }
}
