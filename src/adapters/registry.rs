//! Adapter registry — the single place that knows every base and forge, their live
//! connection status, and how to construct them. Used by both the CLI and the web API
//! so the swap surface is identical everywhere.

use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;

use crate::adapters::base_claude::ClaudeBase;
use crate::adapters::base_framework::{Framework, FrameworkBase};
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
    pub runtime: String,
    pub connected: bool,
    pub note: String,
}

/// A selectable forge + whether a *live* instance is configured (else local instance).
#[derive(Debug, Clone, Serialize)]
pub struct ForgeInfo {
    pub id: String,
    pub display: String,
    pub live: bool,
    pub note: String,
}

fn claude_present() -> bool {
    let bin = std::env::var("OPENFAB_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
    std::process::Command::new(&bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn list_bases() -> Vec<BaseInfo> {
    let mut v = vec![BaseInfo {
        id: "claude".into(),
        display: "Claude (local CLI)".into(),
        runtime: "native".into(),
        connected: claude_present(),
        note: if claude_present() {
            "live LLM via the claude CLI".into()
        } else {
            "claude CLI not found on PATH".into()
        },
    }];
    for fw in Framework::all() {
        let native = std::env::var(fw.native_env())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        v.push(BaseInfo {
            id: fw.id().to_string(),
            display: fw.display().to_string(),
            runtime: if native {
                "native".into()
            } else {
                "bridged".into()
            },
            connected: true,
            note: if native {
                if fw == Framework::AgentChat {
                    format!(
                        "native via matrix bridge ({}) — implementer agents in a Matrix room",
                        fw.native_env()
                    )
                } else {
                    format!("native runtime configured ({})", fw.native_env())
                }
            } else {
                format!(
                    "bridged via OpenFab LLM (set {} for native)",
                    fw.native_env()
                )
            },
        });
    }
    v
}

pub fn build_base(id: &str, policy: &Policy) -> Result<Box<dyn BasePort>> {
    match id {
        "claude" | "claude-cli" => Ok(Box::new(ClaudeBase::new(policy.clone()))),
        other => match Framework::from_id(other) {
            Some(fw) => Ok(Box::new(FrameworkBase::new(fw, policy.clone()))),
            None => bail!("unknown base '{other}' (use: claude | agentscope | hiclaw | agent-chat | openhands)"),
        },
    }
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
        // GitHub is live via the REST API (token + repo) OR the legacy `gh` CLI remote.
        "github" => {
            RestForge::is_configured("github") || std::env::var("OPENFAB_GITHUB_REMOTE").is_ok()
        }
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
                    if kind == "github" && RestForge::is_configured("github") {
                        "live — GitHub REST API (token)".into()
                    } else {
                        "live instance configured".into()
                    }
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
            if RestForge::is_configured("github") {
                // Token-based GitHub REST API (api.github.com) — no `gh` CLI dependency.
                Ok(Box::new(RestForge::from_env("github", repo.to_path_buf())?))
            } else if std::env::var("OPENFAB_GITHUB_REMOTE").is_ok() {
                // Legacy path: the `gh` CLI.
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
    fn lists_five_bases_and_four_forges() {
        assert_eq!(list_bases().len(), 5); // claude + 4 frameworks (no mock)
        assert_eq!(list_forges().len(), 4);
    }

    #[test]
    fn builds_every_base() {
        let p = Policy::default();
        for b in ["claude", "agentscope", "hiclaw", "agent-chat", "openhands"] {
            assert!(build_base(b, &p).is_ok(), "base {b} should build");
        }
        assert!(build_base("mock", &p).is_err()); // mock is gone
        assert!(build_base("nope", &p).is_err());
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
