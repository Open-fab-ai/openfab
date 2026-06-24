//! `forge_rest` — real adapter for token-based REST forges: **GitHub** (via api.github.com)
//! and the Gitea-lineage forges **Forgejo · Gitea · GitCode** (they share the same REST API
//! surface, so one adapter covers all four — R3). It drives `git` for clone/branch/commit/
//! push and the forge's REST API (via `curl`) for PR creation. Gated: selected only when its
//! env (`_TOKEN` + `_REPO`, plus `_URL` for the gitea family) is set, so the default offline
//! demo never touches a live forge. When not configured, the registry falls back to a
//! local-git instance that reports this forge's kind (honest "local instance" badge).

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

/// GitHub's REST API base — fixed (no per-instance URL, unlike the gitea family).
const GITHUB_API: &str = "https://api.github.com";

/// The pull-request API endpoint for a forge. GitHub: `api.github.com/repos/<slug>/pulls`;
/// gitea-lineage: `<base>/api/v1/repos/<slug>/pulls`. Pure (unit-tested).
pub fn pr_api_url(kind: &str, base_url: &str, slug: &str) -> String {
    if kind == "github" {
        format!("{GITHUB_API}/repos/{slug}/pulls")
    } else {
        format!("{base_url}/api/v1/repos/{slug}/pulls")
    }
}

/// The token-embedded push remote for a forge. GitHub uses `x-access-token@github.com`;
/// gitea-lineage uses `oauth2@<host>`. Pure (unit-tested). NEVER persist this to `.git/config`
/// or surface it in an error — use [`clean_remote`] for stored URLs and `redact` for errors.
pub fn authed_remote_for(kind: &str, base_url: &str, token: &str, slug: &str) -> String {
    if kind == "github" {
        format!("https://x-access-token:{token}@github.com/{slug}.git")
    } else {
        let host = base_url.replace("https://", "").replace("http://", "");
        format!("https://oauth2:{token}@{host}/{slug}.git")
    }
}

/// The credential-free remote URL (safe to store in `.git/config`). Pure (unit-tested).
pub fn clean_remote(kind: &str, base_url: &str, slug: &str) -> String {
    if kind == "github" {
        format!("https://github.com/{slug}.git")
    } else {
        let host = base_url.replace("https://", "").replace("http://", "");
        format!("https://{host}/{slug}.git")
    }
}

impl RestForge {
    /// Construct from env. GitHub: `OPENFAB_GITHUB_TOKEN` + `_REPO` (API url is fixed).
    /// Gitea family: `OPENFAB_<KIND>_URL` + `_TOKEN` + `_REPO`. Errors if not configured.
    pub fn from_env(kind: &str, workdir: PathBuf) -> Result<Self> {
        let up = kind.to_uppercase();
        let base_url = if kind == "github" {
            GITHUB_API.to_string()
        } else {
            env_req(&format!("OPENFAB_{up}_URL"), kind)?
        };
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
        is_configured_with(kind, |suffix| {
            std::env::var(format!("OPENFAB_{up}_{suffix}")).is_ok()
        })
    }

    /// Strip the secret token from any string before it reaches an error, log, or HTTP body.
    fn redact(&self, s: &str) -> String {
        if self.token.is_empty() {
            s.to_string()
        } else {
            s.replace(&self.token, "***")
        }
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .map_err(|e| anyhow::anyhow!("running git: {}", self.redact(&e.to_string())))?;
        if !out.status.success() {
            // NB: `args` (and stderr) may contain the token-embedded push URL — redact it so
            // it never leaks into the API 500 body or the on-disk run log.
            bail!(
                "git {} failed:\n{}",
                self.redact(&args.join(" ")),
                self.redact(String::from_utf8_lossy(&out.stderr).trim())
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn authed_remote(&self) -> String {
        // Embed the token in the push URL (read from env, never logged/committed).
        authed_remote_for(&self.kind, &self.base_url, &self.token, &self.repo_slug)
    }
}

/// Which env suffixes a forge kind requires, checked via `present`. GitHub needs TOKEN+REPO
/// (api.github.com is fixed); the gitea family also needs URL. Pure (unit-tested).
pub fn is_configured_with(kind: &str, present: impl Fn(&str) -> bool) -> bool {
    let keys: &[&str] = if kind == "github" {
        &["TOKEN", "REPO"]
    } else {
        &["URL", "TOKEN", "REPO"]
    };
    keys.iter().all(|s| present(s))
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
        // `git clone <url-with-token>` persists the credential into <dest>/.git/config. Rewrite
        // the origin to a credential-free URL so the token never sits in plaintext on disk.
        let clean = clean_remote(&self.kind, &self.base_url, &self.repo_slug);
        let _ = Command::new("git")
            .args(["remote", "set-url", "origin", &clean])
            .current_dir(dest)
            .status();
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
        // GitHub: api.github.com/repos/{slug}/pulls — gitea family: {base}/api/v1/...
        let api = pr_api_url(&self.kind, &self.base_url, &self.repo_slug);
        let payload = serde_json::json!({"title": title, "body": body, "head": head, "base": base})
            .to_string();
        let mut args: Vec<String> = vec![
            "-sS".into(),
            "-X".into(),
            "POST".into(),
            api.clone(),
            "-H".into(),
            format!("Authorization: token {}", self.token),
            "-H".into(),
            "Content-Type: application/json".into(),
        ];
        if self.kind == "github" {
            // GitHub requires a User-Agent and recommends the versioned Accept header.
            args.extend([
                "-H".into(),
                "Accept: application/vnd.github+json".into(),
                "-H".into(),
                "User-Agent: openfab".into(),
            ]);
        }
        args.extend(["-d".into(), payload]);
        let out = Command::new("curl")
            .args(&args)
            .output()
            .context("creating PR via REST")?;
        if !out.status.success() {
            bail!(
                "PR creation failed: {}",
                self.redact(String::from_utf8_lossy(&out.stderr).trim())
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

    #[test]
    fn test_github_pr_api_url() {
        assert_eq!(
            pr_api_url("github", GITHUB_API, "owner/repo"),
            "https://api.github.com/repos/owner/repo/pulls"
        );
        // gitea family keeps the /api/v1 prefix on its own base url
        assert_eq!(
            pr_api_url("gitea", "https://git.example.com", "o/r"),
            "https://git.example.com/api/v1/repos/o/r/pulls"
        );
    }

    #[test]
    fn test_github_push_remote() {
        assert_eq!(
            authed_remote_for("github", GITHUB_API, "TOK", "owner/repo"),
            "https://x-access-token:TOK@github.com/owner/repo.git"
        );
    }

    #[test]
    fn clean_remote_has_no_token() {
        // the URL stored in .git/config must never carry the secret
        let c = clean_remote("github", GITHUB_API, "owner/repo");
        assert_eq!(c, "https://github.com/owner/repo.git");
        assert!(!c.contains('@') && !c.contains("token"));
        assert_eq!(
            clean_remote("gitea", "https://git.example.com", "o/r"),
            "https://git.example.com/o/r.git"
        );
    }

    #[test]
    fn test_github_is_configured() {
        // GitHub: TOKEN + REPO are enough (no URL); missing either → not configured.
        let present = |k: &str| matches!(k, "TOKEN" | "REPO");
        assert!(is_configured_with("github", present));
        assert!(!is_configured_with("github", |k| k == "TOKEN")); // REPO missing
                                                                  // gitea family additionally requires URL
        assert!(!is_configured_with("gitea", present)); // URL missing
        assert!(is_configured_with("gitea", |k| matches!(
            k,
            "URL" | "TOKEN" | "REPO"
        )));
    }
}
