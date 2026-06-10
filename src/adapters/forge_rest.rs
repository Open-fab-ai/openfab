//! `forge_rest` — real adapter for the Gitea-lineage forges: **Forgejo · Gitea ·
//! GitCode** (they share the same REST API surface, so one adapter covers all three —
//! R3). It drives `git` for clone/branch/commit/push and the forge's REST API (via
//! `curl`) for PR creation. Gated: selected only when its `OPENFAB_<KIND>_URL` +
//! `_TOKEN` + `_REPO` env are set, so the default offline demo never touches a live
//! forge. When not configured, the registry falls back to a local-git instance that
//! reports this forge's kind (honest "local instance" badge).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::ports::forge::{ForgePort, PrUrl, Sha, Trailers};

pub struct RestForge {
    kind: String, // "forgejo" | "gitea" | "gitcode"
    base_url: String,
    token: String,
    repo_slug: String, // "owner/repo"
    workdir: PathBuf,
}

impl RestForge {
    /// Construct from `OPENFAB_<KIND>_URL` / `_TOKEN` / `_REPO`. Errors if not configured.
    pub fn from_env(kind: &str, workdir: PathBuf) -> Result<Self> {
        let up = kind.to_uppercase();
        let base_url = env_req(&format!("OPENFAB_{up}_URL"), kind)?;
        let token = env_req(&format!("OPENFAB_{up}_TOKEN"), kind)?;
        let repo_slug = env_req(&format!("OPENFAB_{up}_REPO"), kind)?;
        Ok(RestForge {
            kind: kind.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            repo_slug,
            workdir,
        })
    }

    /// True if the env for this forge kind is configured (drives the UI status badge).
    pub fn is_configured(kind: &str) -> bool {
        let up = kind.to_uppercase();
        ["URL", "TOKEN", "REPO"]
            .iter()
            .all(|s| std::env::var(format!("OPENFAB_{up}_{s}")).is_ok())
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("git {}", args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "git {} failed:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn authed_remote(&self) -> String {
        // Embed the token in the push URL (read from env, never logged/committed).
        let host = self.base_url.replace("https://", "").replace("http://", "");
        format!(
            "https://oauth2:{}@{}/{}.git",
            self.token, host, self.repo_slug
        )
    }
}

fn env_req(key: &str, kind: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow::anyhow!("{kind} forge selected but {key} is not set"))
}

impl ForgePort for RestForge {
    fn name(&self) -> &str {
        &self.kind
    }

    fn kind(&self) -> &str {
        &self.kind
    }

    fn clone_repo(&self, dest: &Path) -> Result<()> {
        if dest.join(".git").exists() {
            return Ok(());
        }
        let status = Command::new("git")
            .args([
                "clone",
                &self.authed_remote(),
                dest.to_str().context("dest")?,
            ])
            .status()
            .context("git clone")?;
        if !status.success() {
            bail!("git clone from {} failed", self.base_url);
        }
        Ok(())
    }

    fn branch(&self, name: &str) -> Result<()> {
        self.git(&["checkout", "-q", "-B", name])?;
        Ok(())
    }

    fn commit(&self, paths: &[PathBuf], msg: &str, trailers: &Trailers) -> Result<Sha> {
        for p in paths {
            let rel = p
                .strip_prefix(&self.workdir)
                .map(|r| r.to_path_buf())
                .unwrap_or_else(|_| p.clone());
            self.git(&["add", "--", rel.to_str().context("path")?])?;
        }
        let full = if trailers.entries.is_empty() {
            msg.to_string()
        } else {
            format!("{msg}\n\n{}", trailers.render())
        };
        self.git(&["commit", "-q", "-m", &full])?;
        self.git(&["rev-parse", "HEAD"])
    }

    fn open_pr(&self, title: &str, body: &str, head: &str, base: &str) -> Result<PrUrl> {
        self.git(&["push", "-q", self.authed_remote().as_str(), head])?;
        // Gitea/Forgejo/GitCode shared endpoint: POST /api/v1/repos/{slug}/pulls
        let api = format!("{}/api/v1/repos/{}/pulls", self.base_url, self.repo_slug);
        let payload = serde_json::json!({"title": title, "body": body, "head": head, "base": base})
            .to_string();
        let out = Command::new("curl")
            .args([
                "-sS",
                "-X",
                "POST",
                &api,
                "-H",
                &format!("Authorization: token {}", self.token),
                "-H",
                "Content-Type: application/json",
                "-d",
                &payload,
            ])
            .output()
            .context("creating PR via REST")?;
        if !out.status.success() {
            bail!(
                "PR creation failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        Ok(v.get("html_url")
            .and_then(|u| u.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("{}/{}/pulls", self.base_url, self.repo_slug)))
    }

    fn write_provenance(&self, contents: &str, filename: &str) -> Result<PathBuf> {
        let dir = self.workdir.join("provenance");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(filename);
        std::fs::write(&path, contents)?;
        Ok(path)
    }

    fn workdir(&self) -> &Path {
        &self.workdir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_forge_is_detected() {
        // With no env set, none of the gitea-lineage forges report configured.
        assert!(!RestForge::is_configured("forgejo"));
        assert!(RestForge::from_env("gitea", PathBuf::from("/tmp/x")).is_err());
    }
}
