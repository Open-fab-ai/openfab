//! `ForgePort` — the swappable git host (PRD §3, across axis). Provenance is
//! forge-neutral and portable: the attestation/SBOM are plain files committed
//! *in-repo*, so they travel with the code across GitHub / Forgejo / Gitea / GitCode.

use std::path::{Path, PathBuf};

use anyhow::Result;

pub type Sha = String;
pub type PrUrl = String;

/// Git commit trailers — the human-readable binding of a commit to its spec, agent,
/// and attestation (the `Co-Authored-By` / `Spec` / `Attestation` lines).
#[derive(Debug, Clone, Default)]
pub struct Trailers {
    pub entries: Vec<(String, String)>,
}

impl Trailers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: &str, val: &str) -> Self {
        self.entries.push((key.to_string(), val.to_string()));
        self
    }

    /// Render as the trailing block of a commit message.
    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The swappable forge. Mirrors PRD §3 (`clone`/`branch`/`commit`/`open_pr`/
/// `write_provenance`). The same Core code drives every forge through this trait, so
/// "same flow + verification works across forges" is a property of the seam, not of
/// any one adapter.
pub trait ForgePort {
    /// Display name (may be customized per instance, e.g. "forgejo-local").
    fn name(&self) -> &str;

    /// Adapter kind for reconstruction: "local" | "github" | "forgejo" | "gitea" | "gitcode".
    fn kind(&self) -> &str;

    /// Ensure a working repo exists at `dest` (clone for a remote forge; init for the
    /// local-git forge used in the demo).
    fn clone_repo(&self, dest: &Path) -> Result<()>;

    fn branch(&self, name: &str) -> Result<()>;

    /// Commit `paths` with a message and trailers; returns the commit SHA.
    fn commit(&self, paths: &[PathBuf], msg: &str, trailers: &Trailers) -> Result<Sha>;

    /// Open (or simulate, for local forges) a PR from `head` into `base`.
    fn open_pr(&self, title: &str, body: &str, head: &str, base: &str) -> Result<PrUrl>;

    /// Write a portable provenance file into the repo and return its path. Forge-neutral.
    fn write_provenance(&self, contents: &str, filename: &str) -> Result<PathBuf>;

    /// Absolute path of the working tree (so Core can place generated files).
    fn workdir(&self) -> &Path;
}
