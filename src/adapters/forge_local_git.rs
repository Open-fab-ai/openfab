//! `forge_local_git` — a local git repository acting as a forge.
//!
//! It implements the full `ForgePort` against a real on-disk git repo by shelling to
//! `git` (the dependency budget prefers this over linking `git2`/libgit2). The demo
//! spins up *two* of these under different names ("github-local", "forgejo-local") to
//! prove the cross-forge claim: the SAME Core flow runs against both, and the portable
//! provenance committed in-repo verifies identically on either. A PR is "opened" by
//! recording PR metadata and leaving the change on a branch (never auto-merged — the
//! trust gate blocks merge until N-of-M human sign-off).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::ports::forge::{ForgePort, PrUrl, Sha, Trailers};

pub struct LocalGitForge {
    kind: String,
    name: String,
    workdir: PathBuf,
}

impl LocalGitForge {
    pub fn new(name: &str, workdir: PathBuf) -> Self {
        LocalGitForge::with_kind("local", name, workdir)
    }

    /// A local-git instance that *reports* a specific forge kind (github/forgejo/gitea/
    /// gitcode) — used by the demo to exercise the full cross-forge matrix offline. The
    /// mechanism is honestly local git; the UI labels it "<kind> (local instance)".
    pub fn with_kind(kind: &str, name: &str, workdir: PathBuf) -> Self {
        LocalGitForge {
            kind: kind.to_string(),
            name: name.to_string(),
            workdir,
        }
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;
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

impl ForgePort for LocalGitForge {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &str {
        &self.kind
    }

    fn clone_repo(&self, dest: &Path) -> Result<()> {
        std::fs::create_dir_all(dest).with_context(|| format!("mkdir {}", dest.display()))?;
        if dest.join(".git").exists() {
            return Ok(());
        }
        // Initialize a fresh repo with a base commit so branches/PRs have a target.
        self.git(&["init", "-q", "-b", "main"])?;
        self.git(&["config", "user.name", "OpenFab Forge"])?;
        self.git(&["config", "user.email", "forge@open-fab.ai"])?;
        let readme = dest.join("README.md");
        std::fs::write(
            &readme,
            format!(
                "# {} (OpenFab local forge)\n\nProvenance lives in `provenance/`.\n",
                self.name
            ),
        )?;
        self.git(&["add", "README.md"])?;
        self.git(&["commit", "-q", "-m", "chore: initialize repository"])?;
        Ok(())
    }

    fn branch(&self, name: &str) -> Result<()> {
        // Create or switch to the branch.
        if self
            .git(&["rev-parse", "--verify", "--quiet", name])
            .is_ok()
        {
            self.git(&["checkout", "-q", name])?;
        } else {
            // Start every NEW branch from the repo's pristine ROOT commit (just the README),
            // never from the current branch. Otherwise `checkout -b` forks whatever run was
            // last checked out, so each app inherits the previous app's files — the cause of
            // a "calculator" build carrying a prior run's leftovers and a wrong launch
            // entrypoint. Branching from root makes every app/run self-contained.
            let root = self.git(&["rev-list", "--max-parents=0", "HEAD"])?;
            let root = root.lines().last().unwrap_or("main").trim().to_string();
            self.git(&["checkout", "-q", "-b", name, &root])?;
            // Drop any untracked files left in the worktree by a previous run so the new
            // app's generation + acceptance + launch see only its own files.
            let _ = self.git(&["clean", "-qfd"]);
        }
        Ok(())
    }

    fn commit(&self, paths: &[PathBuf], msg: &str, trailers: &Trailers) -> Result<Sha> {
        for p in paths {
            // git add expects paths relative to the repo; accept absolute too.
            let rel = p
                .strip_prefix(&self.workdir)
                .map(|r| r.to_path_buf())
                .unwrap_or_else(|_| p.clone());
            self.git(&["add", "--", rel.to_str().context("non-utf8 path")?])?;
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
        // Record PR metadata in-repo (portable; no forge account needed for the demo).
        let prs_dir = self.workdir.join(".openfab").join("prs");
        std::fs::create_dir_all(&prs_dir)?;
        // Branch names contain '/', which would create nested dirs — flatten for the filename.
        let pr_path = prs_dir.join(format!("{}.md", head.replace('/', "_")));
        std::fs::write(
            &pr_path,
            format!(
                "# PR: {title}\n\n- forge: {forge}\n- head: {head}\n- base: {base}\n- status: OPEN (awaiting N-of-M human sign-off)\n\n{body}\n",
                forge = self.name
            ),
        )?;
        Ok(format!("local-git://{}/{}/pull/{}", self.name, base, head))
    }

    fn write_provenance(&self, contents: &str, filename: &str) -> Result<PathBuf> {
        let dir = self.workdir.join("provenance");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(filename);
        std::fs::write(&path, contents)
            .with_context(|| format!("writing provenance {}", path.display()))?;
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
    fn init_branch_commit_pr_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let forge = LocalGitForge::new("github-local", repo.clone());
        forge.clone_repo(&repo).unwrap();
        forge.branch("openfab/demo").unwrap();
        let prov = forge.write_provenance("{\"ok\":true}", "att.json").unwrap();
        let sha = forge
            .commit(
                &[prov],
                "feat: add thing",
                &Trailers::new().with("Spec", "demo#v1"),
            )
            .unwrap();
        assert_eq!(sha.len(), 40, "git sha should be 40 hex chars");
        let url = forge
            .open_pr("Add thing", "body", "openfab/demo", "main")
            .unwrap();
        assert!(url.contains("github-local"));
        assert!(repo.join(".openfab/prs/openfab_demo.md").exists());
    }
}
