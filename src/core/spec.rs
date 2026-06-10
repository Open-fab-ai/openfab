//! Spec engine — parse, validate, version, and compile specs into task-cards.
//!
//! The spec is the *contract* (PRD §4): natural-language `intent` plus
//! machine-checkable `acceptance`. NL is ambiguous, so a spec also records
//! `assumptions` and surfaces `open_questions` back to the human. Development is a
//! cycle, not a line — every loop bumps `version` and accrues decision memory.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// One machine-checkable acceptance criterion. `check` is any executable command;
/// exit 0 = pass (PRD §4). These run in the sandbox, never on the host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Acceptance {
    pub id: String,
    pub check: String,
    #[serde(default = "default_true")]
    pub must_pass: bool,
}

fn default_true() -> bool {
    true
}

/// A versioned spec. Mirrors the PRD §4 YAML format, plus a few fields the fab
/// needs to actually build the app (`target_dir`, `language`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub id: String,
    #[serde(default = "default_version")]
    pub version: u32,
    /// The natural-language ask. This is the "NL in" of the fab.
    pub intent: String,
    #[serde(default)]
    pub context: Vec<String>,
    pub acceptance: Vec<Acceptance>,
    /// Filled in when the NL was ambiguous (the spec records what it assumed).
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Surfaced to the human via the base's comms channel.
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default = "default_true")]
    pub human_signoff_required: bool,
    /// Where, inside the forge repo, the generated app lives (relative path).
    #[serde(default = "default_target_dir")]
    pub target_dir: String,
    /// Primary language hint for the coding agent (e.g. "python", "rust", "node").
    #[serde(default)]
    pub language: Option<String>,
}

fn default_version() -> u32 {
    1
}
fn default_target_dir() -> String {
    "app".to_string()
}

impl Spec {
    /// Parse a spec from YAML text and validate it (the in-process equivalent of
    /// JSON-Schema validation against `schemas/spec.schema.json`).
    pub fn from_yaml(text: &str) -> Result<Spec> {
        let spec: Spec =
            serde_yaml::from_str(text).context("spec is not valid YAML / wrong shape")?;
        spec.validate()?;
        Ok(spec)
    }

    /// Load a spec from a file path.
    pub fn from_path(path: &std::path::Path) -> Result<Spec> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading spec {}", path.display()))?;
        Spec::from_yaml(&text)
    }

    /// Structural validation. Keeps the contract honest: no anonymous criteria, no
    /// empty intent, unique acceptance ids, at least one criterion.
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("spec.id must not be empty");
        }
        if self.intent.trim().is_empty() {
            bail!("spec.intent (the natural-language ask) must not be empty");
        }
        if self.version == 0 {
            bail!("spec.version must be >= 1");
        }
        if self.acceptance.is_empty() {
            bail!("spec must declare at least one acceptance criterion (the contract)");
        }
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.acceptance {
            if a.id.trim().is_empty() {
                bail!("every acceptance criterion needs an id");
            }
            if a.check.trim().is_empty() {
                bail!("acceptance '{}' has an empty check command", a.id);
            }
            if !seen.insert(&a.id) {
                bail!("duplicate acceptance id '{}'", a.id);
            }
        }
        Ok(())
    }

    /// `id#vN` — the stable reference used in provenance and trailers.
    pub fn spec_ref(&self) -> String {
        format!("{}#v{}", self.id, self.version)
    }

    /// Compile a spec into the task-card(s) dispatched to the base. v0.1 emits a
    /// single card per spec; the seam is here to fan out later.
    pub fn compile(&self, workdir: PathBuf) -> Vec<TaskCard> {
        vec![TaskCard {
            id: format!("{}-task1", self.id),
            spec_id: self.id.clone(),
            spec_version: self.version,
            intent: self.intent.clone(),
            context: self.context.clone(),
            assumptions: self.assumptions.clone(),
            acceptance: self.acceptance.clone(),
            target_dir: self.target_dir.clone(),
            language: self.language.clone(),
            workdir,
        }]
    }

    /// Produce the next spec version, folding human feedback **into the intent** (so the
    /// agent acts on it — not buried as an assumption) and optionally adding a new
    /// acceptance criterion. This is the "Spec v -> v+1" step, meant for *tweaking* the
    /// current product. (A wholly different app is a fresh spec — its acceptance contract
    /// differs too — not a refine.)
    pub fn next_version(&self, feedback: &str, new_acceptance: Option<Acceptance>) -> Spec {
        let mut next = self.clone();
        next.version += 1;
        next.intent = format!(
            "{}\n\nFollow-up change requested by the human (v{}): {}",
            self.intent.trim(),
            next.version,
            feedback.trim()
        );
        next.assumptions
            .push(format!("v{}: human feedback: {}", next.version, feedback));
        if let Some(a) = new_acceptance {
            next.acceptance.push(a);
        }
        next.open_questions.clear();
        next
    }
}

/// A unit of work dispatched to the base (PRD `TaskCard`). It carries everything the
/// agent needs and nothing about which base will run it.
#[derive(Debug, Clone)]
pub struct TaskCard {
    pub id: String,
    pub spec_id: String,
    pub spec_version: u32,
    pub intent: String,
    pub context: Vec<String>,
    pub assumptions: Vec<String>,
    pub acceptance: Vec<Acceptance>,
    pub target_dir: String,
    pub language: Option<String>,
    /// Absolute path of the repo working tree the agent edits.
    pub workdir: PathBuf,
}

impl TaskCard {
    pub fn spec_ref(&self) -> String {
        format!("{}#v{}", self.spec_id, self.spec_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id: demo-x
version: 1
intent: "Build a thing"
acceptance:
  - id: a1
    check: "true"
    must_pass: true
"#;

    #[test]
    fn parses_and_validates() {
        let s = Spec::from_yaml(SAMPLE).unwrap();
        assert_eq!(s.id, "demo-x");
        assert_eq!(s.spec_ref(), "demo-x#v1");
        assert_eq!(s.acceptance.len(), 1);
        assert!(s.human_signoff_required); // defaults on
    }

    #[test]
    fn rejects_empty_intent() {
        let bad = "id: x\nintent: ''\nacceptance: [{id: a1, check: 'true'}]\n";
        assert!(Spec::from_yaml(bad).is_err());
    }

    #[test]
    fn rejects_duplicate_acceptance_ids() {
        let bad = r#"
id: x
intent: "y"
acceptance:
  - {id: a1, check: "true"}
  - {id: a1, check: "false"}
"#;
        assert!(Spec::from_yaml(bad).is_err());
    }

    #[test]
    fn compiles_one_card() {
        let s = Spec::from_yaml(SAMPLE).unwrap();
        let cards = s.compile(PathBuf::from("/tmp/x"));
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].spec_ref(), "demo-x#v1");
    }

    #[test]
    fn next_version_bumps_and_records() {
        let s = Spec::from_yaml(SAMPLE).unwrap();
        let n = s.next_version("add CSV export", None);
        assert_eq!(n.version, 2);
        assert!(n.assumptions[0].contains("human feedback"));
    }
}
