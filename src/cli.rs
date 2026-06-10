//! CLI — wires the spec cycle to the operator (PRD §7, build-order step 7).
//!
//! Commands:
//!   run            run one spec cycle (NL → signed PR, blocked on sign-off)
//!   feedback       fold a human NL note into the spec (v→v+1) and re-run the cycle
//!   maintainer-add register a pre-approved human signer (the trust allowlist)
//!   signoff        a maintainer signs off; on N-of-M the gate opens and the PR merges
//!   verify         check an artifact against the OpenFab profile (signatures + acceptance)
//!   reputation     project reputation from the signed attestations
//!   list           show runs in a repo

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::core::provenance::Attestation;
use crate::core::reputation;
use crate::core::spec::Spec;
use crate::core::trust::Policy;
use crate::ops;
use crate::runstate;

#[derive(Parser)]
#[command(
    name = "openfab",
    version,
    about = "OpenFab — natural language in, trustworthy software out"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one spec cycle: dispatch to the base, verify, sign, commit, open a PR.
    Run {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long, default_value = "claude")]
        base: String,
        #[arg(long, default_value = "local")]
        forge: String,
        /// Name for a local forge (lets the demo run two "forges": github-local, forgejo-local).
        #[arg(long)]
        forge_name: Option<String>,
        /// Human-approval gate: solo (self-approve) | team (2-of-2) | crowd | none.
        #[arg(long, default_value = "team")]
        gate: String,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Natural language in → the LLM authors the spec (incl. acceptance) → build it.
    Build {
        /// What to build, in plain English.
        intent: String,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long, default_value = "claude")]
        base: String,
        #[arg(long, default_value = "local")]
        forge: String,
        #[arg(long)]
        forge_name: Option<String>,
        #[arg(long, default_value = "solo")]
        gate: String,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Fold human feedback into the spec (v→v+1) and re-run the cycle.
    Feedback {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        run: String,
        #[arg(long)]
        note: String,
        /// Optional new acceptance criterion: "id=<id>,check=<shell>".
        #[arg(long)]
        add_check: Option<String>,
        #[arg(long, default_value = "claude")]
        base: String,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Register a pre-approved maintainer (adds them to the sign-off allowlist).
    MaintainerAdd {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Sign off a run as a maintainer. On N-of-M the gate opens and the PR merges.
    Signoff {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        run: String,
        #[arg(long = "as")]
        as_name: String,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Verify an artifact against the OpenFab profile (signatures + acceptance + sign-off).
    Verify {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        run: String,
    },
    /// Project reputation from the signed attestations in a repo.
    Reputation {
        #[arg(long)]
        repo: PathBuf,
    },
    /// List runs in a repo.
    List {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Launch the OpenFab web UI + API (the end-to-end visual demo).
    Serve {
        /// Repo/workspace root the UI operates on (forges live under it).
        #[arg(long, default_value = "demo/.work/web")]
        repo: PathBuf,
        #[arg(long, default_value_t = 8787)]
        port: u16,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            spec,
            repo,
            base,
            forge,
            forge_name,
            gate,
            policy,
        } => cmd_run(
            &spec,
            &repo,
            &base,
            &forge,
            forge_name,
            &gate,
            policy.as_deref(),
            None,
        ),
        Cmd::Build {
            intent,
            repo,
            base,
            forge,
            forge_name,
            gate,
            policy,
        } => cmd_build(
            &intent,
            &repo,
            &base,
            &forge,
            forge_name,
            &gate,
            policy.as_deref(),
        ),
        Cmd::Feedback {
            repo,
            run,
            note,
            add_check,
            base,
            policy,
        } => cmd_feedback(
            &repo,
            &run,
            &note,
            add_check.as_deref(),
            &base,
            policy.as_deref(),
        ),
        Cmd::MaintainerAdd { repo, name } => cmd_maintainer_add(&repo, &name),
        Cmd::Signoff {
            repo,
            run,
            as_name,
            policy,
        } => cmd_signoff(&repo, &run, &as_name, policy.as_deref()),
        Cmd::Verify { repo, run } => cmd_verify(&repo, &run),
        Cmd::Reputation { repo } => cmd_reputation(&repo),
        Cmd::List { repo } => cmd_list(&repo),
        Cmd::Serve { repo, port, policy } => {
            let repo = abs(&repo)?;
            std::fs::create_dir_all(&repo)?;
            crate::server::serve(repo, port, load_policy(policy.as_deref())?)
        }
    }
}

fn load_policy(path: Option<&Path>) -> Result<Policy> {
    match path {
        Some(p) => Policy::from_path(p),
        None => Ok(Policy::default()),
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    spec_path: &Path,
    repo: &Path,
    base_name: &str,
    forge_kind: &str,
    forge_name: Option<String>,
    gate: &str,
    policy_path: Option<&Path>,
    parent_run: Option<String>,
) -> Result<()> {
    let repo = abs(repo)?;
    let policy = load_policy(policy_path)?;
    let spec = Spec::from_path(spec_path)?;

    println!(
        "== OpenFab run: spec={} base={} forge={} gate={} ==",
        spec.spec_ref(),
        base_name,
        forge_kind,
        gate
    );
    let rec = ops::start_run(
        &repo,
        ops::RunRequest {
            spec,
            base: base_name.to_string(),
            forge_kind: forge_kind.to_string(),
            forge_name,
            parent_run,
            run_id: None,
            gate_mode: gate.to_string(),
            authored_by: None,
        },
        &policy,
    )?;
    if !rec.acceptance_passed {
        eprintln!("\nNote: machine acceptance did not pass — the gate will stay blocked. (Honest failure, not a vacuous pass.)");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_build(
    intent: &str,
    repo: &Path,
    base_name: &str,
    forge_kind: &str,
    forge_name: Option<String>,
    gate: &str,
    policy_path: Option<&Path>,
) -> Result<()> {
    let repo = abs(repo)?;
    let policy = load_policy(policy_path)?;
    println!("== OpenFab build: the LLM authors the spec from your intent, then builds ==");
    let run_id = ops::reserve_intent_run_id(intent);
    ops::build(
        &repo, intent, run_id, base_name, forge_kind, forge_name, gate, &policy,
    )?;
    Ok(())
}

fn cmd_feedback(
    repo: &Path,
    run: &str,
    note: &str,
    _add_check: Option<&str>,
    base_name: &str,
    policy_path: Option<&Path>,
) -> Result<()> {
    let repo = abs(repo)?;
    let policy = load_policy(policy_path)?;
    println!("== OpenFab refine: re-authoring the spec from your feedback → v+1 ==");
    let run_id = ops::reserve_refine_run_id(&repo, run)?;
    ops::refine(&repo, run, note, run_id, base_name, &policy)?;
    Ok(())
}

fn cmd_maintainer_add(repo: &Path, name: &str) -> Result<()> {
    let repo = abs(repo)?;
    let (did, new) = runstate::add_maintainer(&repo, name)?;
    println!(
        "maintainer '{name}' {} — {}",
        if new {
            "registered"
        } else {
            "already registered"
        },
        did
    );
    Ok(())
}

fn cmd_signoff(repo: &Path, run: &str, as_name: &str, policy_path: Option<&Path>) -> Result<()> {
    let repo = abs(repo)?;
    let policy = load_policy(policy_path)?;
    let out = ops::signoff(&repo, run, as_name, &policy)?;
    println!("✍️  {as_name} ({}) signed off", short(&out.signer_did));
    for s in &out.satisfied {
        println!("  ✓ {s}");
    }
    for b in &out.blocking {
        println!("  ⛔ {b}");
    }
    if out.merged {
        println!("🛡️  gate ACCEPTED → merged into main ✅");
    } else if out.accepted {
        println!("🛡️  gate ACCEPTED — merge the PR on the forge.");
    } else {
        println!("🛡️  gate still BLOCKED — needs more sign-off.");
    }
    Ok(())
}

fn cmd_verify(repo: &Path, run: &str) -> Result<()> {
    let repo = abs(repo)?;
    let out = ops::verify(&repo, run)?;
    println!("== openfab verify: {} ==", out.spec_ref);
    for c in &out.checks {
        println!(
            "  [{}] {} — {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.id,
            c.detail
        );
    }
    println!(
        "\nConformance: {}    Trust gate: {}    Merged: {}",
        yn(out.conformant),
        yn(out.accepted),
        yn(out.merged)
    );
    if !out.conformant {
        bail!("artifact is NOT conformant to the OpenFab profile");
    }
    println!(
        "✅ verify passed: signatures valid, attribution recorded, acceptance + sign-off present."
    );
    Ok(())
}

fn cmd_reputation(repo: &Path) -> Result<()> {
    let repo = abs(repo)?;
    let mut atts = vec![];
    for rec in runstate::list_runs(&repo)? {
        if let Ok(text) = std::fs::read_to_string(rec.attestation_path(&repo)) {
            if let Ok(att) = Attestation::from_json(&text) {
                atts.push(att);
            }
        }
    }
    let table = reputation::compute(&atts);
    println!(
        "== OpenFab reputation (projected from {} attestation(s)) ==",
        atts.len()
    );
    println!(
        "{:<34} {:>8} {:>9} {:>7} {:>9}",
        "DID", "authored", "accepted", "rate", "signoffs"
    );
    for (did, stat) in &table {
        println!(
            "{:<34} {:>8} {:>9} {:>6.0}% {:>9}",
            short(did),
            stat.authored,
            stat.accepted,
            stat.acceptance_rate() * 100.0,
            stat.signoffs_given
        );
    }
    Ok(())
}

fn cmd_list(repo: &Path) -> Result<()> {
    let repo = abs(repo)?;
    let runs = runstate::list_runs(&repo)?;
    println!("== OpenFab runs in {} ==", repo.display());
    for r in &runs {
        println!(
            "{:<40} spec={:<22} base={:<10} accepted={} merged={}",
            r.run_id,
            r.spec_ref,
            r.base_name,
            yn(r.accepted),
            yn(r.merged)
        );
    }
    if runs.is_empty() {
        println!("(none)");
    }
    Ok(())
}

fn abs(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}

fn short(did: &str) -> String {
    if did.len() > 24 {
        format!("{}…{}", &did[..16], &did[did.len() - 4..])
    } else {
        did.to_string()
    }
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}
