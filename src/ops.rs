//! Operations layer — the spec-cycle actions shared by the CLI and the web API, so the
//! two front-ends never duplicate the orchestration logic (R3). Each function returns
//! plain data; the CLI prints it, the server JSON-encodes it.

use std::path::Path;
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

/// Author a spec from a natural-language intent using the LLM (the Base). The user only
/// supplies the intent; the model derives the acceptance criteria, language, assumptions,
/// and open questions. The human then reviews/edits before building (the spec-time intent
/// check). Returns the drafted Spec plus (model, provider) for the record.
pub fn author_spec(intent: &str) -> Result<(Spec, String, String)> {
    if intent.trim().len() < 4 {
        bail!("describe what you want to build");
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
    let (spec, model, provider) = author_spec(intent)?;
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

    // Re-run the contract in the sandbox.
    let mut checks = vec![];
    let mut all_passed = true;
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
