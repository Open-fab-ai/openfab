//! The spec cycle (PRD §4) — the orchestration glue, the first of the three pieces of
//! "genuinely new code" (PRD §5). One pass of:
//!
//!   NL intent → compile task-cards → dispatch to base → dual verify (machine
//!   acceptance in sandbox) → build + sign provenance (in-toto/SLSA + openfab/generation
//!   predicate) + SBOM → commit portable provenance to the forge → open PR → trust gate
//!   (blocks merge until N-of-M human sign-off).
//!
//! Human feedback re-enters as a new NL note that bumps the spec (v → v+1) and re-runs
//! the cycle — see `cli::cmd_feedback`. Core drives this entirely through the ports, so
//! it never names a concrete base or forge.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::core::identity::Identity;
use crate::core::provenance::{Attestation, GeneratedRange, GenerationInput, Material};
use crate::core::sbom::Sbom;
use crate::core::spec::Spec;
use crate::core::trust::{self, Policy, TrustInput};
use crate::core::{sha256_hex, timeutil};
use crate::ports::base::{BasePort, ChangedFile};
use crate::ports::forge::{ForgePort, Trailers};
use crate::runstate::{self, AcceptanceOutcome, RunRecord};

pub struct CycleConfig<'a> {
    pub spec: &'a Spec,
    pub base: &'a dyn BasePort,
    pub forge: &'a dyn ForgePort,
    pub fab: &'a Identity,
    pub policy: &'a Policy,
    pub parent_run: Option<String>,
    /// Pre-generated run id (the web API reserves it before spawning the run thread so
    /// it can return immediately and let the UI poll). `None` → generate one.
    pub run_id: Option<String>,
    /// Human-approval gate mode (recorded so sign-off/verify reconstruct the same gate).
    pub gate_mode: String,
    /// "provider · model" when the spec's acceptance was LLM-authored (timeline note).
    pub authored_by: Option<String>,
}

/// Live decision log: printed, posted to the base's comms channel, streamed to disk as
/// events (for the web UI), and persisted as a markdown timeline.
struct Timeline {
    repo: PathBuf,
    run_id: String,
    seq: u64,
    lines: Vec<String>,
}

impl Timeline {
    fn new(repo: PathBuf, run_id: String) -> Self {
        Timeline {
            repo,
            run_id,
            seq: 0,
            lines: vec![],
        }
    }

    fn step(&mut self, base: &dyn BasePort, icon: &str, msg: &str) {
        self.seq += 1;
        let ts = timeutil::iso_now();
        let line = format!("[{ts}] {icon} {msg}");
        println!("{line}");
        let _ = base.post("openfab", msg);
        runstate::append_event(
            &self.repo,
            &self.run_id,
            &runstate::Event {
                seq: self.seq,
                ts,
                icon: icon.to_string(),
                msg: msg.to_string(),
            },
        );
        self.lines.push(line);
    }

    fn render(&self, spec_ref: &str) -> String {
        format!(
            "# OpenFab decision log — {spec_ref}\n\n{}\n",
            self.lines
                .iter()
                .map(|l| format!("- {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

/// Run one full spec cycle and persist the result. Returns the resulting RunRecord.
/// Gate modes that require human sign-off return blocked until N-of-M is satisfied; `none`
/// can be accepted immediately after machine verification and signing.
pub fn run_cycle(cfg: CycleConfig) -> Result<RunRecord> {
    let spec = cfg.spec;
    let base = cfg.base;
    let forge = cfg.forge;
    let repo = forge.workdir().to_path_buf();
    let run_id = cfg
        .run_id
        .clone()
        .unwrap_or_else(|| format!("{}-v{}-{}", spec.id, spec.version, timeutil::unix_now()));
    let mut tl = Timeline::new(repo.clone(), run_id.clone());
    set_status(
        &repo,
        &run_id,
        &spec.spec_ref(),
        "running",
        "starting",
        None,
    );

    tl.step(
        base,
        "📥",
        &format!("NL intent received → \"{}\"", truncate(&spec.intent, 100)),
    );
    if let Some(by) = &cfg.authored_by {
        tl.step(
            base,
            "🧾",
            &format!(
                "spec authored by the LLM ({by}) → {} acceptance criteria (the contract)",
                spec.acceptance.len()
            ),
        );
    }

    // Capability negotiation (PRD §3): inspect the base and have OpenFab fill any gap.
    let caps = base.capabilities();
    tl.step(
        base,
        "🔌",
        &format!(
            "base '{}' capabilities: orchestrate={} comms={} memory={} sandbox={}{}",
            base.name(),
            caps.orchestrate,
            caps.comms,
            caps.memory,
            caps.sandbox,
            if caps.sandbox {
                ""
            } else {
                " → OpenFab supplies its own sandbox"
            }
        ),
    );
    // Decision memory: has this exact spec ref been processed before?
    if let Ok(Some(prev)) = base.memory_get(&format!("seen:{}", spec.spec_ref())) {
        tl.step(
            base,
            "🧠",
            &format!(
                "decision memory: spec seen before ({} bytes of prior state)",
                prev.len()
            ),
        );
    }
    if !spec.assumptions.is_empty() {
        tl.step(
            base,
            "📝",
            &format!("recorded assumptions: {}", spec.assumptions.join("; ")),
        );
    }
    if !spec.open_questions.is_empty() {
        tl.step(
            base,
            "❓",
            &format!(
                "open questions surfaced to human: {}",
                spec.open_questions.join("; ")
            ),
        );
    }

    // 1. Compile → task cards.
    let cards = spec.compile(repo.clone());
    tl.step(
        base,
        "🧩",
        &format!(
            "spec {} compiled into {} task-card(s)",
            spec.spec_ref(),
            cards.len()
        ),
    );

    // 2. Branch on the forge.
    let branch = format!("openfab/{}-v{}", spec.id, spec.version);
    forge
        .branch(&branch)
        .with_context(|| format!("creating branch {branch}"))?;
    tl.step(
        base,
        "🌿",
        &format!("forge '{}' → branch {branch}", forge.name()),
    );

    // 3. Dispatch to the base; collect what the agent authored.
    let mut changed_files = vec![];
    let mut model = String::new();
    let mut prompt = String::new();
    for card in &cards {
        let handle = base
            .dispatch(card)
            .with_context(|| format!("dispatch {}", card.id))?;
        let result = base.result(&handle)?;
        debug_assert_eq!(
            result.task_id, card.id,
            "result must correspond to the dispatched card"
        );
        tl.step(
            base,
            if result.success { "🤖" } else { "⚠️" },
            &format!("base '{}' ({}) → {}", base.name(), result.model, result.log),
        );
        model = result.model;
        prompt = result.prompt;
        changed_files.extend(result.changed_files);
    }

    // 4. Dual verification — machine acceptance in the sandbox.
    set_status(
        &repo,
        &run_id,
        &spec.spec_ref(),
        "running",
        "verifying acceptance",
        None,
    );
    tl.step(
        base,
        "🧪",
        &format!(
            "sandbox = {}; running {} acceptance check(s)",
            crate::adapters::sandbox::runtime_label(),
            spec.acceptance.len()
        ),
    );
    let mut outcomes = vec![];
    let mut agent_spec_verdicts: Vec<crate::core::provenance::ScenarioVerdict> = vec![];
    let mut spec_contract_sha256: Option<String> = None;
    if crate::adapters::agent_spec::enabled() {
        // Verification is delegated to `agent-spec lifecycle` (the contract's BDD scenarios
        // bound to real tests). With OPENFAB_REVIEW=caller, AI-pending scenarios (design intent
        // / quality) are additionally routed to the reviewer agent, whose verdict is merged in.
        let (outs, verdicts) = if crate::adapters::agent_spec::review_caller_enabled() {
            let bridge = std::env::var("OPENFAB_AGENTCHAT_URL").unwrap_or_default();
            let room = std::env::var("OPENFAB_AGENTCHAT_ROOM").unwrap_or_else(|_| "openfab".into());
            let paths: Vec<String> = changed_files.iter().map(|f| f.path.clone()).collect();
            tl.step(
                base,
                "🔎",
                "review: routing AI-pending scenarios to the reviewer (caller mode)",
            );
            crate::adapters::agent_spec::verify_with_review(spec, &repo, &bridge, &room, &paths)?
        } else {
            crate::adapters::agent_spec::verify_via_lifecycle(spec, &repo)?
        };
        outcomes = outs;
        agent_spec_verdicts = verdicts;
        spec_contract_sha256 = crate::adapters::agent_spec::contract_sha256(spec);
        for o in &outcomes {
            tl.step(
                base,
                if o.passed { "✅" } else { "❌" },
                &format!(
                    "scenario [{}] {} → {}",
                    o.id,
                    o.check,
                    if o.passed { "pass" } else { "FAIL" }
                ),
            );
        }
    } else {
        for a in &spec.acceptance {
            let cmd = vec!["bash".to_string(), "-c".to_string(), a.check.clone()];
            let exec = base.run_sandboxed(&cmd, &repo)?;
            let passed = exec.passed();
            tl.step(
                base,
                if passed { "✅" } else { "❌" },
                &format!(
                    "acceptance [{}] `{}` → {}",
                    a.id,
                    a.check,
                    if passed { "pass" } else { "FAIL" }
                ),
            );
            if !passed {
                let detail = first_nonempty(&exec.stdout, &exec.stderr);
                if !detail.is_empty() {
                    tl.step(
                        base,
                        "  ›",
                        &format!("sandbox output: {}", truncate(&detail, 160)),
                    );
                }
            }
            outcomes.push(AcceptanceOutcome {
                id: a.id.clone(),
                check: a.check.clone(),
                passed,
                exit_code: exec.exit_code,
            });
        }
    }
    // Requirements doc (Phase 2): if the spec was distilled from a requirements conversation,
    // record its hash in the signed provenance (requirements → spec → code traceability).
    let requirements_sha256 = crate::adapters::agent_spec::requirements_sha256(&spec.id);

    // Spec-drift guard: a required scenario with no verification outcome means the verified
    // contract differs from the built one (don't fail silently — name the cause).
    let drift = crate::adapters::agent_spec::unverified_scenarios(spec, &outcomes);
    if !drift.is_empty() {
        tl.step(
            base,
            "⚠️",
            &format!(
                "spec drift — {} required scenario(s) had NO verification result; the verified \
                 contract differs from the built one (is OPENFAB_SPEC_DIR inside the repo, so a \
                 branch op reset the authored spec?): [{}]",
                drift.len(),
                drift.join(", ")
            ),
        );
    }

    // Trust gate hardening (two holes that let dishonest builds pass acceptance):
    //  ① empty implementation — the base produced no files, so the bound tests "pass" against
    //     the pre-existing tree (a no-op build that reports success). Must fail.
    //  ② out-of-scope edits — the base touched files the spec's Allowed Changes forbid (e.g.
    //     deleting the data model, or rewriting a "frozen" test to make an unchanged-check pass).
    //     The Forbidden boundary is otherwise only advisory prompt text; enforce it on the diff.
    // Use git's OWN view of the worktree (not the base's self-reported `changed_files`): a no-op
    // build "writes" files whose content already matches HEAD → net-zero diff → git shows
    // nothing, even though the base claims it produced files. This is what let v3/v4/v5 pass.
    let real_changed = git_worktree_changes(&repo);
    let empty_impl = real_changed.is_empty();
    if empty_impl {
        tl.step(
            base,
            "⛔",
            "no changes produced — the implementation made no net change to the code (git diff is \
             empty); acceptance cannot be credited (it would only re-test pre-existing code).",
        );
    }
    let boundary_violations = crate::adapters::agent_spec::boundary_violations(
        &real_changed,
        &spec.target_dir,
        &spec.assumptions,
    );
    if !boundary_violations.is_empty() {
        tl.step(
            base,
            "⛔",
            &format!(
                "boundary violation — {} file(s) edited outside the spec's Allowed Changes (the \
                 Forbidden boundary was crossed): [{}]",
                boundary_violations.len(),
                boundary_violations.join(", ")
            ),
        );
    }

    let acceptance_passed = !empty_impl
        && boundary_violations.is_empty()
        && spec.acceptance.iter().filter(|a| a.must_pass).all(|a| {
            outcomes
                .iter()
                .find(|o| o.id == a.id)
                .map(|o| o.passed)
                .unwrap_or(false)
        });

    // 4b. Layered QA (PPT S11/S14 pillar 1): beyond the bound tests, run the configured tier's
    // checks (coverage now; mutation/fuzz honest-skip). A QA failure blocks like a failed test;
    // the report is signed into the provenance and gated by conformance C13.
    let qa_tier = crate::adapters::qa::QaTier::from_env();
    let qa_min_cov = std::env::var("OPENFAB_QA_MIN_COVERAGE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let qa_min_mut = std::env::var("OPENFAB_QA_MIN_MUTATION")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    // Coverage/mutation run where the code lives (target_dir), not the repo root.
    let qa_code_dir = crate::adapters::agent_spec::acceptance_code_dir(&repo, spec);
    let qa = crate::adapters::qa::run(&qa_code_dir, qa_tier, qa_min_cov, qa_min_mut);
    let qa_passed = qa.passed();
    let qa_report_json = if matches!(qa_tier, crate::adapters::qa::QaTier::Fast) {
        None
    } else {
        for o in &qa.outcomes {
            let icon = match o.status {
                crate::adapters::qa::QaStatus::Passed => "✅",
                crate::adapters::qa::QaStatus::Failed => "❌",
                crate::adapters::qa::QaStatus::Skipped => "⏭️",
            };
            tl.step(
                base,
                icon,
                &format!("qa[{:?}] {} — {}", qa_tier, o.check, o.detail),
            );
        }
        serde_json::to_value(&qa).ok()
    };
    // QA folds into machine acceptance: the build only passes verify if both hold.
    let acceptance_passed = acceptance_passed && qa_passed;

    // 5. Build + sign provenance (in-toto/SLSA + openfab/generation predicate).
    let attributed_files = git_worktree_changed_file_records(&repo, &real_changed)?;
    let generated: Vec<GeneratedRange> = attributed_files
        .iter()
        .map(|f| GeneratedRange {
            path: f.path.clone(),
            lines: format!("1-{}", f.lines),
            sha256: f.sha256.clone(),
            author: "ai".to_string(), // sign-off adds the human author tag later
        })
        .collect();
    let mut bundle = attributed_files
        .iter()
        .map(|f| format!("{}:{}", f.path, f.sha256))
        .collect::<Vec<_>>();
    bundle.sort();
    let source_bundle_sha256 = sha256_hex(bundle.join("\n").as_bytes());
    let materials = spec
        .context
        .iter()
        .map(|c| Material {
            uri: c.clone(),
            sha256: None,
        })
        .collect();

    let att = Attestation::build_and_sign(
        GenerationInput {
            spec_ref: spec.spec_ref(),
            app_name: format!("{}-{}", spec.id, spec.target_dir),
            source_bundle_sha256,
            agent_did: cfg.fab.did(),
            base_name: base.name().to_string(),
            model: model.clone(),
            prompt,
            params: serde_json::json!({ "base": base.name(), "model": model }),
            generated,
            materials,
            acceptance_passed,
            spec_contract_sha256,
            agent_spec_verdicts,
            run_log_ref: None,
            requirements_sha256,
            qa_report: qa_report_json,
            cross_model_verdicts: None,
        },
        cfg.fab,
    )?;
    tl.step(
        base,
        "🔏",
        &format!(
            "signed in-toto/SLSA attestation (predicate openfab/generation); fab DID {}; payload sha256 {}",
            short(&cfg.fab.did()),
            &att.payload_sha256[..16]
        ),
    );

    // 6. SBOM.
    let sbom = Sbom::build(
        &format!("{}-v{}", spec.id, spec.version),
        &attributed_files
            .iter()
            .map(|f| (f.path.clone(), f.sha256.clone()))
            .collect::<Vec<_>>(),
    );

    // 7. Write portable provenance into the repo and commit everything.
    let att_name = format!("{}-v{}.att.json", spec.id, spec.version);
    let sbom_name = format!("{}-v{}.sbom.json", spec.id, spec.version);
    let att_path = forge.write_provenance(&att.to_json()?, &att_name)?;
    let sbom_path = forge.write_provenance(&sbom.to_json()?, &sbom_name)?;
    tl.step(
        base,
        "📦",
        &format!("wrote portable provenance: provenance/{att_name} + SBOM"),
    );

    let mut commit_paths: Vec<PathBuf> = real_changed.iter().map(|p| repo.join(p)).collect();
    commit_paths.push(att_path.clone());
    commit_paths.push(sbom_path.clone());

    // When authored via agent-spec, commit the `.spec.md` contract into the repo so it
    // travels with the code (portable, reproducible).
    if crate::adapters::agent_spec::enabled() {
        if let Ok(md) = std::fs::read(crate::adapters::agent_spec::spec_md_path(&spec.id)) {
            let dest = crate::adapters::agent_spec::repo_spec_md_path(&repo, &spec.id);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, md).context("committing .spec.md into repo")?;
            tl.step(
                base,
                "📄",
                &format!("committed contract specs/{}.spec.md", spec.id),
            );
            commit_paths.push(dest);
        }
    }

    // Commit the requirements document (Phase 2) into the repo so requirements travel with
    // the code, matching the hash recorded in the attestation.
    {
        let spec_dir = std::env::var("OPENFAB_SPEC_DIR").unwrap_or_else(|_| "specs".to_string());
        let req_src = crate::adapters::agent_spec::requirements_md_path_in(
            std::path::Path::new(&spec_dir),
            &spec.id,
        );
        if let Ok(req) = std::fs::read(&req_src) {
            let dest = repo
                .join("specs")
                .join(format!("{}.requirements.md", spec.id));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, req).context("committing requirements.md into repo")?;
            tl.step(
                base,
                "📝",
                &format!("committed requirements specs/{}.requirements.md", spec.id),
            );
            commit_paths.push(dest);
        }
    }

    let mut trailers = Trailers::new()
        .with("Spec", &spec.spec_ref())
        .with(
            "Co-Authored-By",
            &format!("openfab-agent ({}) <agent@open-fab.ai>", cfg.fab.did()),
        )
        .with("OpenFab-Base", base.name())
        .with("OpenFab-Attestation", &att.payload_sha256)
        .with(
            "OpenFab-Acceptance",
            if acceptance_passed {
                "passed"
            } else {
                "failed"
            },
        );
    if let Some(h) = &att.statement.predicate.spec_contract_sha256 {
        trailers = trailers.with("OpenFab-Spec-Contract", h);
    }
    let commit_msg = format!("feat({}): {}", spec.id, truncate(&spec.intent, 60));
    let sha = forge.commit(&commit_paths, &commit_msg, &trailers)?;
    tl.step(
        base,
        "📌",
        &format!(
            "committed {} on {branch} with provenance trailers",
            &sha[..sha.len().min(10)]
        ),
    );

    // 8. Open PR.
    let pr_body = format!(
        "Implements `{}`.\n\nMachine acceptance: {}\nProvenance: `provenance/{}`\n\n_Merge is blocked until N-of-M human sign-off (OpenFab trust gate)._",
        spec.spec_ref(),
        if acceptance_passed { "PASSED" } else { "FAILED" },
        att_name
    );
    let pr_url = forge.open_pr(&format!("OpenFab: {}", spec.id), &pr_body, &branch, "main")?;
    tl.step(base, "🔀", &format!("opened PR {pr_url}"));

    // 9. Trust gate (pre-sign-off: expected to block on the human gate).
    set_status(
        &repo,
        &run_id,
        &spec.spec_ref(),
        "running",
        "trust gate",
        None,
    );
    let fab_allow = runstate::fab_allowlist(&repo)?;
    let maint = runstate::maintainer_dids(&repo)?;
    let decision = trust::evaluate(
        cfg.policy,
        &TrustInput {
            att: &att,
            fab_allowlist: &fab_allow,
            maintainer_allowlist: &maint,
            base_name: base.name(),
            acceptance_passed,
        },
    );
    for s in &decision.satisfied {
        tl.step(base, "  ✓", s);
    }
    for b in &decision.blocking {
        tl.step(base, "  ⛔", b);
    }
    tl.step(
        base,
        "🛡️",
        &format!(
            "trust gate: {} — {}",
            if decision.accepted {
                "ACCEPTED"
            } else {
                "BLOCKED"
            },
            if decision.accepted {
                "ready to merge"
            } else {
                "awaiting human sign-off"
            }
        ),
    );
    // Make dashboard→Robrix approval smooth: when the run blocks at the gate, tell the room the
    // exact run id + how to approve from chat (reaches the room when base = agent-chat). The user
    // need not copy the id off the dashboard.
    if !decision.accepted {
        tl.step(
            base,
            "🔔",
            &format!(
                "Run `{}` is awaiting sign-off. Reply `approve {}` here to release it, or sign in the OpenFab dashboard.",
                run_id, run_id
            ),
        );
    }

    // 10. Persist run state + decision.
    let final_status = if !acceptance_passed {
        "failed"
    } else if decision.accepted {
        "accepted"
    } else {
        "blocked"
    };
    let rec = RunRecord {
        run_id: run_id.clone(),
        spec_ref: spec.spec_ref(),
        base_name: base.name().to_string(),
        forge_kind: forge.kind().to_string(),
        forge_name: forge.name().to_string(),
        base_runtime: base.runtime_mode().to_string(),
        status: final_status.to_string(),
        gate_mode: cfg.gate_mode.clone(),
        branch,
        pr_url,
        attestation_repo_path: format!("provenance/{att_name}"),
        sbom_repo_path: format!("provenance/{sbom_name}"),
        acceptance: outcomes,
        acceptance_passed,
        accepted: decision.accepted,
        merged: false,
        parent_run: cfg.parent_run.clone(),
        created: timeutil::iso_now(),
    };
    let spec_yaml = serde_yaml::to_string(spec).context("serialize spec")?;
    runstate::save_run(&repo, &rec, &spec_yaml, &tl.render(&spec.spec_ref()))?;
    std::fs::write(
        runstate::run_dir(&repo, &run_id).join("decision.json"),
        serde_json::to_string_pretty(&decision)?,
    )?;
    set_status(&repo, &run_id, &spec.spec_ref(), final_status, "done", None);
    // Record decision memory for future cycles (PRD §3 memory port).
    let _ = base.memory_put(
        &format!("seen:{}", spec.spec_ref()),
        att.payload_sha256.as_bytes(),
    );

    println!("\nRun id: {run_id}");
    println!("Next: openfab signoff --repo <repo> --run {run_id} --as <maintainer>");
    Ok(rec)
}

fn set_status(
    repo: &Path,
    run_id: &str,
    spec_ref: &str,
    status: &str,
    step: &str,
    error: Option<String>,
) {
    runstate::write_status(
        repo,
        &runstate::StatusFile {
            run_id: run_id.to_string(),
            spec_ref: spec_ref.to_string(),
            status: status.to_string(),
            step: step.to_string(),
            updated: timeutil::iso_now(),
            error,
        },
    );
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

/// The implementation's real changed paths from git (`git status --porcelain -uall`) — the
/// ground truth for "did the implementation actually change anything, and within bounds?", used
/// by the trust-gate hardening instead of the base's self-reported file list. On any git error,
/// returns empty (which the caller treats as a no-op → fail-closed, never silently pass).
fn git_worktree_changes(repo: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            crate::adapters::agent_spec::parse_git_status_paths(&String::from_utf8_lossy(&o.stdout))
        }
        _ => vec![],
    }
}

/// File records for attestation/SBOM, derived from git's real changed paths and the bytes
/// currently on disk. Deleted paths stay in the commit path list, but are not valid
/// `generated` entries because there is no file content to hash.
fn git_worktree_changed_file_records(repo: &Path, paths: &[String]) -> Result<Vec<ChangedFile>> {
    let mut records = Vec::new();
    for rel in paths {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute()
            || rel_path
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            anyhow::bail!("git reported unsafe changed path: {rel}");
        }
        let abs = repo.join(rel_path);
        match std::fs::read(&abs) {
            Ok(bytes) => records.push(ChangedFile {
                path: rel.clone(),
                lines: line_count(&bytes),
                sha256: sha256_hex(&bytes),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Deletions are real implementation changes and must be committed, but there is
                // no generated file range to sign for a path that no longer exists.
            }
            Err(e) => return Err(e).with_context(|| format!("reading changed file {rel}")),
        }
    }
    records.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(records)
}

fn line_count(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        0
    } else {
        let newlines = bytes.iter().filter(|b| **b == b'\n').count();
        if bytes.ends_with(b"\n") {
            newlines
        } else {
            newlines + 1
        }
    }
}

fn short(did: &str) -> String {
    if did.len() > 20 {
        format!("{}…{}", &did[..14], &did[did.len() - 4..])
    } else {
        did.to_string()
    }
}

fn first_nonempty(a: &str, b: &str) -> String {
    let a = a.trim();
    if !a.is_empty() {
        a.to_string()
    } else {
        b.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::process::Command;

    use super::*;
    use crate::adapters::forge_local_git::LocalGitForge;
    use crate::core::provenance::Attestation;
    use crate::core::spec::Acceptance;
    use crate::ports::base::{Capabilities, ChangedFile, ExecResult, RunHandle, RunResult};

    struct MismatchedBase {
        result: RefCell<Option<RunResult>>,
    }

    impl MismatchedBase {
        fn new() -> Self {
            Self {
                result: RefCell::new(None),
            }
        }
    }

    impl BasePort for MismatchedBase {
        fn name(&self) -> &str {
            "mock"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                orchestrate: true,
                comms: true,
                memory: true,
                sandbox: false,
            }
        }

        fn dispatch(&self, task: &crate::core::spec::TaskCard) -> Result<RunHandle> {
            let real = task.workdir.join("app/real.txt");
            std::fs::create_dir_all(real.parent().unwrap())?;
            std::fs::write(&real, "real bytes\n")?;
            *self.result.borrow_mut() = Some(RunResult {
                task_id: task.id.clone(),
                changed_files: vec![ChangedFile {
                    path: "app/claimed.txt".into(),
                    lines: 1,
                    sha256: crate::core::sha256_hex(b"claimed bytes\n"),
                }],
                model: "fake-model".into(),
                prompt: "write real.txt".into(),
                log: "wrote real.txt but claimed another path".into(),
                success: true,
            });
            Ok(RunHandle {
                id: task.id.clone(),
            })
        }

        fn result(&self, _h: &RunHandle) -> Result<RunResult> {
            self.result
                .borrow_mut()
                .take()
                .context("missing fake run result")
        }

        fn post(&self, _channel: &str, _msg: &str) -> Result<()> {
            Ok(())
        }

        fn memory_get(&self, _key: &str) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn memory_put(&self, _key: &str, _val: &[u8]) -> Result<()> {
            Ok(())
        }

        fn run_sandboxed(&self, cmd: &[String], workdir: &Path) -> Result<ExecResult> {
            let out = Command::new(&cmd[0])
                .args(&cmd[1..])
                .current_dir(workdir)
                .output()?;
            Ok(ExecResult {
                exit_code: out.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            })
        }
    }

    #[test]
    fn provenance_uses_git_truth_not_base_reported_changed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let forge = LocalGitForge::new("local-test", repo.clone());
        forge.clone_repo(&repo).unwrap();

        let fab = Identity::generate("fab").unwrap();
        runstate::ensure_fab_allowlisted(&repo, &fab.did()).unwrap();

        let spec = Spec {
            id: "git-truth".into(),
            version: 1,
            intent: "write the real file".into(),
            context: vec![],
            acceptance: vec![Acceptance {
                id: "real-file-exists".into(),
                check: "test -f app/real.txt".into(),
                must_pass: true,
            }],
            assumptions: vec![],
            open_questions: vec![],
            human_signoff_required: false,
            target_dir: "app".into(),
            language: None,
        };
        let base = MismatchedBase::new();
        let policy = Policy::default().for_gate_mode("none");

        let rec = run_cycle(CycleConfig {
            spec: &spec,
            base: &base,
            forge: &forge,
            fab: &fab,
            policy: &policy,
            parent_run: None,
            run_id: Some("git-truth-run".into()),
            gate_mode: "none".into(),
            authored_by: None,
        })
        .unwrap();

        let att_text = std::fs::read_to_string(rec.attestation_path(&repo)).unwrap();
        let att: Attestation = serde_json::from_str(&att_text).unwrap();
        let generated = &att.statement.predicate.generated;

        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].path, "app/real.txt");
        assert_eq!(generated[0].lines, "1-1");
        assert_eq!(
            generated[0].sha256,
            crate::core::sha256_hex(b"real bytes\n")
        );
        assert!(!repo.join("app/claimed.txt").exists());
    }
}
