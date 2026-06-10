//! `forge_github` — GitHub adapter (real, but gated).
//!
//! Demonstrates that adding a forge is a single-file change against `ForgePort`. It
//! drives `git` + the `gh` CLI. It is intentionally NOT used by the default demo:
//! cross-forge portability is proven offline with two local forges (so the overnight
//! build never creates real PRs on anyone's account). Select it explicitly with
//! `--forge github` and `OPENFAB_GITHUB_REMOTE=<git url>`; it errors clearly if the
//! environment isn't configured, rather than silently degrading.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::ports::forge::{ForgePort, PrUrl, Sha, Trailers};

pub struct GitHubForge {
    workdir: PathBuf,
    remote: String,
}

impl GitHubForge {
    /// Construct from env. Requires `OPENFAB_GITHUB_REMOTE` and an authenticated `gh`.
    pub fn from_env(workdir: PathBuf) -> Result<Self> {
        let remote = std::env::var("OPENFAB_GITHUB_REMOTE").map_err(|_| {
            anyhow::anyhow!(
                "GitHub forge selected but OPENFAB_GITHUB_REMOTE is not set (git URL of the repo)"
            )
        })?;
        if Command::new("gh").arg("--version").output().is_err() {
            bail!("GitHub forge needs the `gh` CLI on PATH");
        }
        Ok(GitHubForge { workdir, remote })
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
}

impl ForgePort for GitHubForge {
    fn name(&self) -> &str {
        "github"
    }

    fn kind(&self) -> &str {
        "github"
    }

    fn clone_repo(&self, dest: &Path) -> Result<()> {
        if dest.join(".git").exists() {
            return Ok(());
        }
        let status = Command::new("git")
            .args(["clone", &self.remote, dest.to_str().context("dest path")?])
            .status()
            .context("git clone")?;
        if !status.success() {
            bail!("git clone of {} failed", self.remote);
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
        self.git(&["push", "-u", "origin", head])?;
        let out = Command::new("gh")
            .args([
                "pr", "create", "--title", title, "--body", body, "--head", head, "--base", base,
            ])
            .current_dir(&self.workdir)
            .output()
            .context("gh pr create")?;
        if !out.status.success() {
            bail!(
                "gh pr create failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
