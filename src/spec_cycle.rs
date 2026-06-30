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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::core::identity::Identity;
use crate::core::provenance::{Attestation, GeneratedRange, GenerationInput, Material};
use crate::core::sbom::Sbom;
use crate::core::spec::Spec;
use crate::core::trust::{self, Policy, TrustInput};
use crate::core::{sha256_hex, timeutil};
use crate::ports::base::BasePort;
use crate::ports::forge::{ForgePort, Trailers};
use crate::runstate::{self, AcceptanceOutcome, RunRecord};

/// Status string for a draft (un-attested) run — single source of truth (R3).
pub const DRAFT_STATUS: &str = "draft";

/// How much of the trust ceremony a run performs (PRD roadmap: fast loop vs checkpoint).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// Fast inner loop: generate code + run acceptance, commit source to a draft branch,
    /// then STOP. No signature, SBOM, PR, or trust gate — the run is explicitly
    /// **un-attested**. This is what a developer iterates with; nothing heavy fires per edit.
    Draft,
    /// Full ceremony: sign in-toto/SLSA + SBOM + PR + N-of-M trust gate. The default, so
    /// every legacy path and persisted record keeps its existing trustworthy behaviour.
    #[default]
    Release,
}

impl RunMode {
    pub fn is_draft(&self) -> bool {
        matches!(self, RunMode::Draft)
    }
}

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
    /// Draft (fast, un-attested) vs Release (full ceremony). Defaults to Release.
    pub mode: RunMode,
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

/// Run one full spec cycle and persist the result. Returns the resulting RunRecord
/// (in "open / awaiting sign-off" state — `accepted` is false until N-of-M sign-off).
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

    // 2. Branch on the forge. Drafts go on their own `openfab/draft/<run>` branch so a
    //    fast iteration never disturbs the release branch or its provenance.
    let branch = if cfg.mode.is_draft() {
        format!("openfab/draft/{run_id}")
    } else {
        format!("openfab/{}-v{}", spec.id, spec.version)
    };
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

    // Persist the generation prompt as LOCAL run-state (not the signed BOM — the BOM keeps
    // only its sha256 by design) so the UI can show the author the exact prompt on demand.
    let _ = std::fs::write(
        runstate::run_dir(&repo, &run_id).join("prompt.txt"),
        &prompt,
    );

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
    let acceptance_passed = spec.acceptance.iter().filter(|a| a.must_pass).all(|a| {
        outcomes
            .iter()
            .find(|o| o.id == a.id)
            .map(|o| o.passed)
            .unwrap_or(false)
    });

    // 4b. DRAFT MODE — the fast inner loop stops here. We commit the *source* to the draft
    //     branch so it can be inspected and later promoted, but we run NO signature, SBOM,
    //     PR, or trust gate. The run is explicitly UN-ATTESTED (status = "draft"); an
    //     explicit `openfab promote` checkpoint runs the full ceremony once, on the
    //     accepted state. This is what keeps iteration from kicking off heavy work per edit.
    if cfg.mode.is_draft() {
        let commit_paths: Vec<PathBuf> = changed_files.iter().map(|f| repo.join(&f.path)).collect();
        let trailers = Trailers::new()
            .with("Spec", &spec.spec_ref())
            .with("OpenFab-Base", base.name())
            .with("OpenFab-Mode", DRAFT_STATUS)
            .with(
                "OpenFab-Acceptance",
                if acceptance_passed {
                    "passed"
                } else {
                    "failed"
                },
            );
        let commit_msg = format!(
            "draft({}): {} [UN-ATTESTED]",
            spec.id,
            truncate(&spec.intent, 50)
        );
        let sha = if commit_paths.is_empty() {
            String::new()
        } else {
            forge.commit(&commit_paths, &commit_msg, &trailers)?
        };
        tl.step(
            base,
            "⚡",
            &format!(
                "DRAFT — acceptance {}; source committed{} · NOT signed, NOT gated (un-attested). Run `openfab promote` for a signed release.",
                if acceptance_passed { "PASSED" } else { "FAILED" },
                if sha.is_empty() {
                    String::new()
                } else {
                    format!(" as {}", &sha[..sha.len().min(10)])
                }
            ),
        );
        let rec = RunRecord {
            run_id: run_id.clone(),
            spec_ref: spec.spec_ref(),
            base_name: base.name().to_string(),
            forge_kind: forge.kind().to_string(),
            forge_name: forge.name().to_string(),
            base_runtime: base.runtime_mode().to_string(),
            status: DRAFT_STATUS.to_string(),
            gate_mode: cfg.gate_mode.clone(),
            branch,
            pr_url: String::new(),
            attestation_repo_path: String::new(),
            sbom_repo_path: String::new(),
            acceptance: outcomes,
            acceptance_passed,
            accepted: false,
            merged: false,
            parent_run: cfg.parent_run.clone(),
            created: timeutil::iso_now(),
        };
        let spec_yaml = serde_yaml::to_string(spec).context("serialize spec")?;
        runstate::save_run(&repo, &rec, &spec_yaml, &tl.render(&spec.spec_ref()))?;
        set_status(
            &repo,
            &run_id,
            &spec.spec_ref(),
            DRAFT_STATUS,
            "draft · un-attested",
            None,
        );
        println!("\nDraft run id: {run_id}  (un-attested)");
        println!(
            "Next: openfab promote --repo <repo> --run {run_id}   # full ceremony → signed release"
        );
        return Ok(rec);
    }

    // 5. Build + sign provenance (in-toto/SLSA + openfab/generation predicate).
    let generated: Vec<GeneratedRange> = changed_files
        .iter()
        .map(|f| GeneratedRange {
            path: f.path.clone(),
            lines: format!("1-{}", f.lines),
            sha256: f.sha256.clone(),
            author: "ai".to_string(), // sign-off adds the human author tag later
        })
        .collect();
    let mut bundle = changed_files
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

    // Embed the frozen acceptance contract (the actual check commands + result) into the
    // signed predicate, so `reproduce` works from any clone off any forge — no local
    // run-state needed. This is what makes contract-replay genuinely forge-agnostic.
    let acceptance_checks = spec
        .acceptance
        .iter()
        .map(|a| crate::core::provenance::AcceptanceCheck {
            id: a.id.clone(),
            check: a.check.clone(),
            must_pass: a.must_pass,
            passed: outcomes
                .iter()
                .find(|o| o.id == a.id)
                .is_some_and(|o| o.passed),
        })
        .collect::<Vec<_>>();
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
            acceptance: acceptance_checks,
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
        &changed_files
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

    let mut commit_paths: Vec<PathBuf> = changed_files.iter().map(|f| repo.join(&f.path)).collect();
    commit_paths.push(att_path.clone());
    commit_paths.push(sbom_path.clone());
    let trailers = Trailers::new()
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
    use super::*;

    #[test]
    fn runmode_defaults_to_release() {
        // Legacy paths and any RunMode::default() must keep full-ceremony behaviour.
        assert_eq!(RunMode::default(), RunMode::Release);
        assert!(!RunMode::default().is_draft());
        assert!(RunMode::Draft.is_draft());
    }

    #[test]
    fn runmode_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&RunMode::Draft).unwrap(), "\"draft\"");
        assert_eq!(
            serde_json::to_string(&RunMode::Release).unwrap(),
            "\"release\""
        );
        assert!(serde_json::from_str::<RunMode>("\"draft\"")
            .unwrap()
            .is_draft());
        assert!(!serde_json::from_str::<RunMode>("\"release\"")
            .unwrap()
            .is_draft());
    }
}
