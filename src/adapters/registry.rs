//! Adapter registry — the single place that knows every base and forge, their live
//! connection status, and how to construct them. Used by both the CLI and the web API
//! so the swap surface is identical everywhere.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::adapters::base_claude::{ClaudeBase, CliKind};
use crate::adapters::base_framework::{endpoint_reachable, Framework, FrameworkBase};
use crate::adapters::forge_github::GitHubForge;
use crate::adapters::forge_local_git::LocalGitForge;
use crate::adapters::forge_rest::RestForge;
use crate::core::trust::Policy;
use crate::ports::base::BasePort;
use crate::ports::forge::ForgePort;

/// A selectable base + how it will execute here (honest runtime mode + connection note).
#[derive(Debug, Clone, Serialize)]
pub struct BaseInfo {
    pub id: String,
    pub display: String,
    /// "native" when reachable now, else "offline" (frameworks) / "native" (claude CLI).
    pub runtime: String,
    pub connected: bool,
    pub note: String,
    /// True when OpenFab can spin this base up itself (a bundled launcher exists).
    pub launchable: bool,
}

/// Live availability of a base, used for the run pre-flight and the UI.
#[derive(Debug, Clone, Serialize)]
pub struct BaseStatus {
    pub id: String,
    /// Reachable right now (claude CLI present / framework adapter answering).
    pub reachable: bool,
    /// A framework base (vs the always-native claude CLI). Only frameworks can bridge.
    pub is_framework: bool,
    /// Endpoint OpenFab would dispatch to (frameworks only).
    pub endpoint: Option<String>,
    /// OpenFab has a bundled launcher for this base.
    pub launchable: bool,
}

/// Outcome of an attempt to launch a base's native runtime.
#[derive(Debug, Clone, Serialize)]
pub struct LaunchOutcome {
    pub base: String,
    pub launched: bool,
    pub reachable: bool,
    pub detail: String,
}

/// A selectable forge + whether a *live* instance is configured (else local instance).
#[derive(Debug, Clone, Serialize)]
pub struct ForgeInfo {
    pub id: String,
    pub display: String,
    pub live: bool,
    pub note: String,
}

/// True when a local CLI responds to `--version` (used to mark claude/codex bases live).
fn cli_present(bin_env: &str, default_bin: &str) -> bool {
    let bin = std::env::var(bin_env).unwrap_or_else(|_| default_bin.to_string());
    std::process::Command::new(&bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn claude_present() -> bool {
    cli_present("OPENFAB_CLAUDE_BIN", "claude")
}

fn codex_present() -> bool {
    cli_present("OPENFAB_CODEX_BIN", "codex")
}

/// Relative path (from the OpenFab source root) of the launcher that brings a base's
/// native runtime up. `None` = no bundled launcher (start it manually / env-only).
fn launcher_script(id: &str) -> Option<&'static str> {
    match Framework::from_id(id) {
        Some(Framework::AgentChat) => Some("integrations/start-agentchat-orchestrate.sh"),
        // agentscope/hiclaw ship integration servers but no one-shot launcher yet.
        _ => None,
    }
}

/// OpenFab source root (where integrations/ lives): OPENFAB_HOME, else the process cwd.
fn openfab_home() -> std::path::PathBuf {
    std::env::var("OPENFAB_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| ".".into()))
}

/// Live availability of a base (claude CLI present, or framework adapter answering).
pub fn base_status(id: &str) -> BaseStatus {
    match id {
        "claude" | "claude-cli" => BaseStatus {
            id: "claude".into(),
            reachable: claude_present(),
            is_framework: false,
            endpoint: None,
            launchable: false,
        },
        "codex" | "codex-cli" => BaseStatus {
            id: "codex".into(),
            reachable: codex_present(),
            is_framework: false,
            endpoint: None,
            launchable: false,
        },
        other => {
            let endpoint = Framework::from_id(other).and_then(|fw| fw.resolve_endpoint());
            BaseStatus {
                id: other.to_string(),
                reachable: endpoint.as_deref().map(endpoint_reachable).unwrap_or(false),
                is_framework: Framework::from_id(other).is_some(),
                endpoint,
                launchable: launcher_script(other).is_some(),
            }
        }
    }
}

pub fn list_bases() -> Vec<BaseInfo> {
    let claude = claude_present();
    let codex = codex_present();
    let mut v = vec![
        BaseInfo {
            id: "claude".into(),
            display: "Claude (local CLI)".into(),
            runtime: "native".into(),
            connected: claude,
            note: if claude {
                "live LLM via the claude CLI".into()
            } else {
                "claude CLI not found on PATH".into()
            },
            launchable: false,
        },
        BaseInfo {
            id: "codex".into(),
            display: "Codex (local CLI)".into(),
            runtime: "native".into(),
            connected: codex,
            note: if codex {
                "live LLM via the codex CLI (codex exec) — typically faster".into()
            } else {
                "codex CLI not found / not logged in".into()
            },
            launchable: false,
        },
    ];
    for fw in Framework::all() {
        let st = base_status(fw.id());
        v.push(BaseInfo {
            id: fw.id().to_string(),
            display: fw.display().to_string(),
            runtime: if st.reachable {
                "native".into()
            } else {
                "offline".into()
            },
            connected: st.reachable,
            note: if st.reachable {
                format!("native runtime live at {}", st.endpoint.unwrap_or_default())
            } else if st.launchable {
                "native runtime not running — OpenFab can launch it".into()
            } else {
                format!(
                    "native runtime not running (start its adapter, or set {})",
                    fw.native_env()
                )
            },
            launchable: st.launchable,
        });
    }
    v
}

pub fn build_base(
    id: &str,
    policy: &Policy,
    allow_bridged: bool,
    base_model: Option<String>,
) -> Result<Box<dyn BasePort>> {
    match id {
        // The attest base must snapshot the existing files BEFORE the cycle branches/cleans
        // the worktree, so it is built via AttestBase::capture in ops::attest — not here.
        "attest" => bail!("the 'attest' base is constructed by `openfab attest` (it captures pre-existing files before branching) — it cannot be selected as a generic base"),
        "claude" | "claude-cli" => Ok(Box::new(ClaudeBase::new(policy.clone()))),
        "codex" | "codex-cli" => Ok(Box::new(ClaudeBase::with_kind(
            CliKind::Codex,
            policy.clone(),
        ))),
        other => match Framework::from_id(other) {
            Some(fw) => Ok(Box::new(FrameworkBase::with_bridge(
                fw,
                policy.clone(),
                allow_bridged,
                base_model,
            ))),
            None => bail!("unknown base '{other}' (use: claude | codex | agentscope | hiclaw | agent-chat | openhands)"),
        },
    }
}

/// Bring a base's native runtime up via its bundled launcher, then poll until the adapter
/// answers (or we give up). Used by `POST /api/base/{id}/launch` so the user can start the
/// real base from OpenFab instead of being silently bridged.
pub fn launch_base(id: &str) -> Result<LaunchOutcome> {
    let script = match launcher_script(id) {
        Some(s) => s,
        None => bail!("no bundled launcher for base '{id}' — start its adapter manually"),
    };
    let st = base_status(id);
    if st.reachable {
        return Ok(LaunchOutcome {
            base: id.to_string(),
            launched: false,
            reachable: true,
            detail: "already running".into(),
        });
    }
    let home = openfab_home();
    let path = home.join(script);
    if !path.exists() {
        bail!(
            "launcher not found: {} (set OPENFAB_HOME to the OpenFab source root)",
            path.display()
        );
    }
    // The launcher backgrounds its services and returns; capture its output for diagnostics.
    let out = std::process::Command::new("bash")
        .arg(&path)
        .current_dir(&home)
        .output()
        .with_context(|| format!("running launcher {}", path.display()))?;
    if !out.status.success() {
        bail!(
            "launcher failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // Poll the endpoint for up to ~40s (orchestrate adapters take a moment to bind).
    let endpoint = base_status(id).endpoint.unwrap_or_default();
    let mut reachable = false;
    for _ in 0..20 {
        if endpoint_reachable(&endpoint) {
            reachable = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Ok(LaunchOutcome {
        base: id.to_string(),
        launched: true,
        reachable,
        detail: if reachable {
            format!("native runtime live at {endpoint}")
        } else {
            "launched but the adapter did not answer in time — check its logs".into()
        },
    })
}

/// The four forge kinds in the demo matrix.
pub const FORGE_KINDS: [&str; 4] = ["github", "forgejo", "gitea", "gitcode"];

fn forge_display(kind: &str) -> &str {
    match kind {
        "github" => "GitHub",
        "forgejo" => "Forgejo",
        "gitea" => "Gitea",
        "gitcode" => "GitCode",
        _ => kind,
    }
}

/// True when a *live* remote instance is configured for this forge kind (so the demo's
/// local-instance fallback is NOT in use). Drives both the UI badge and whether sign-off
/// merges locally vs. defers to the remote forge's UI/API.
pub fn forge_live(kind: &str) -> bool {
    match kind {
        "github" => std::env::var("OPENFAB_GITHUB_REMOTE").is_ok(),
        "forgejo" | "gitea" | "gitcode" => RestForge::is_configured(kind),
        _ => false,
    }
}

/// A forge that is backed by a local git repo here (the offline demo path).
pub fn is_local_instance(kind: &str) -> bool {
    kind == "local" || !forge_live(kind)
}

pub fn list_forges() -> Vec<ForgeInfo> {
    FORGE_KINDS
        .iter()
        .map(|&kind| {
            let live = forge_live(kind);
            ForgeInfo {
                id: kind.to_string(),
                display: forge_display(kind).to_string(),
                live,
                note: if live {
                    "live instance configured".into()
                } else {
                    "local instance (portable provenance, offline)".into()
                },
            }
        })
        .collect()
}

/// Build a forge. If a *live* instance is configured for the kind, use the real adapter;
/// otherwise a local-git instance that reports this kind (honest "local instance").
pub fn build_forge(kind: &str, name: Option<String>, repo: &Path) -> Result<Box<dyn ForgePort>> {
    let display_name = name.unwrap_or_else(|| format!("{kind}-local"));
    match kind {
        "local" => Ok(Box::new(LocalGitForge::new(
            &display_name,
            repo.to_path_buf(),
        ))),
        "github" => {
            if forge_live("github") {
                Ok(Box::new(GitHubForge::from_env(repo.to_path_buf())?))
            } else {
                Ok(Box::new(LocalGitForge::with_kind(
                    "github",
                    &display_name,
                    repo.to_path_buf(),
                )))
            }
        }
        "forgejo" | "gitea" | "gitcode" => {
            if forge_live(kind) {
                Ok(Box::new(RestForge::from_env(kind, repo.to_path_buf())?))
            } else {
                Ok(Box::new(LocalGitForge::with_kind(
                    kind,
                    &display_name,
                    repo.to_path_buf(),
                )))
            }
        }
        other => bail!("unknown forge '{other}' (use: github | forgejo | gitea | gitcode | local)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_six_bases_and_four_forges() {
        assert_eq!(list_bases().len(), 6); // claude + codex + 4 frameworks (no mock)
        assert_eq!(list_forges().len(), 4);
    }

    #[test]
    fn builds_every_base() {
        let p = Policy::default();
        for b in [
            "claude",
            "codex",
            "agentscope",
            "hiclaw",
            "agent-chat",
            "openhands",
        ] {
            assert!(
                build_base(b, &p, false, None).is_ok(),
                "base {b} should build"
            );
        }
        assert!(build_base("mock", &p, false, None).is_err()); // mock is gone
        assert!(build_base("nope", &p, false, None).is_err());
    }

    #[test]
    fn base_status_reports_framework_and_launchability() {
        let agentchat = base_status("agent-chat");
        assert!(agentchat.is_framework);
        assert!(agentchat.launchable, "agent-chat has a bundled launcher");
        assert!(agentchat.endpoint.unwrap().contains("8741"));

        let claude = base_status("claude");
        assert!(!claude.is_framework);
        assert!(!claude.launchable);
        assert!(claude.endpoint.is_none());

        // OpenHands: a framework, but no bundled launcher.
        assert!(!base_status("openhands").launchable);
    }

    #[test]
    fn builds_every_forge_as_local_instance_offline() {
        let tmp = tempfile::tempdir().unwrap();
        for k in FORGE_KINDS {
            let f = build_forge(k, None, tmp.path()).unwrap();
            assert_eq!(f.kind(), k, "forge should report its kind");
        }
    }
}
