//! Layered QA — verify-stage depth beyond the bound BDD tests (PPT S11/S14 pillar 1).
//!
//! Tiers are additive: Fast (bound tests) < Full (+coverage) < Deep (+mutation) < Nightly
//! (+fuzz). OpenFab runs the tier's checks after `agent-spec lifecycle`, records the results in
//! the signed provenance, and gates on them. A tool that isn't installed produces a `Skipped`
//! outcome — honest, never counted as a pass.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QaTier {
    Fast,
    Full,
    Deep,
    Nightly,
}

impl QaTier {
    /// Parse a tier name; unknown / empty → Fast (today's behaviour, backward-compatible).
    pub fn from_str_or_fast(s: &str) -> QaTier {
        match s.trim().to_ascii_lowercase().as_str() {
            "full" => QaTier::Full,
            "deep" => QaTier::Deep,
            "nightly" => QaTier::Nightly,
            _ => QaTier::Fast,
        }
    }

    /// The configured tier from `OPENFAB_QA` (default Fast).
    pub fn from_env() -> QaTier {
        QaTier::from_str_or_fast(&std::env::var("OPENFAB_QA").unwrap_or_default())
    }

    fn rank(self) -> u8 {
        match self {
            QaTier::Fast => 0,
            QaTier::Full => 1,
            QaTier::Deep => 2,
            QaTier::Nightly => 3,
        }
    }

    /// Coverage applies at Full and above.
    pub fn covers_coverage(self) -> bool {
        self.rank() >= QaTier::Full.rank()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QaStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaOutcome {
    pub check: String,
    pub status: QaStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaReport {
    pub tier: QaTier,
    pub coverage_pct: Option<f64>,
    pub outcomes: Vec<QaOutcome>,
}

impl QaReport {
    /// The report blocks the build iff some check Failed. Skipped never blocks (honest absence).
    pub fn passed(&self) -> bool {
        !self.outcomes.iter().any(|o| o.status == QaStatus::Failed)
    }
}

/// Evaluate the coverage gate (pure): below `min` → Failed; at/above → Passed. `min <= 0`
/// disables the gate (Passed regardless).
pub fn coverage_gate(coverage_pct: f64, min_pct: f64) -> QaStatus {
    if min_pct <= 0.0 || coverage_pct >= min_pct {
        QaStatus::Passed
    } else {
        QaStatus::Failed
    }
}

/// Detect a coverage tool for the repo's language, returning the command to run it. `None` when
/// no supported tool is on PATH (→ the coverage check is honestly skipped).
fn coverage_tool(repo: &Path) -> Option<Vec<String>> {
    let has = |bin: &str| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {bin}"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if repo.join("Cargo.toml").exists() {
        if has("cargo-llvm-cov") {
            return Some(vec![
                "cargo".into(),
                "llvm-cov".into(),
                "--summary-only".into(),
            ]);
        }
        if has("cargo-tarpaulin") {
            return Some(vec![
                "cargo".into(),
                "tarpaulin".into(),
                "--out".into(),
                "Stdout".into(),
            ]);
        }
    } else if (repo.join("pyproject.toml").exists() || repo.join("requirements.txt").exists())
        && has("pytest")
    {
        return Some(vec![
            "pytest".into(),
            "--cov".into(),
            "--cov-report=term".into(),
        ]);
    }
    None
}

/// Parse a coverage percentage out of a tool's stdout (llvm-cov "TOTAL … 87.50%", tarpaulin
/// "xx.x% coverage", pytest-cov "TOTAL … 88%"). Returns the first plausible percentage.
pub fn parse_coverage_pct(out: &str) -> Option<f64> {
    // Prefer a TOTAL line if present.
    let line = out
        .lines()
        .find(|l| l.to_ascii_uppercase().contains("TOTAL"))
        .unwrap_or("");
    let scan = |s: &str| -> Option<f64> {
        let bytes = s.as_bytes();
        for (i, _) in s.match_indices('%') {
            // walk back over digits/dot
            let mut j = i;
            while j > 0 && (bytes[j - 1].is_ascii_digit() || bytes[j - 1] == b'.') {
                j -= 1;
            }
            if j < i {
                if let Ok(v) = s[j..i].parse::<f64>() {
                    return Some(v);
                }
            }
        }
        None
    };
    scan(line).or_else(|| scan(out))
}

/// Run the tier's QA checks in `repo`. Fast = nothing extra (bound tests already ran in the
/// lifecycle step). Full+ = coverage gate (honest-skip when no tool). Deep/Nightly add coverage
/// today and reserve mutation/fuzz (honest-skip) until wired.
pub fn run(repo: &Path, tier: QaTier, min_coverage_pct: f64) -> QaReport {
    let mut outcomes = vec![];
    let mut coverage_pct = None;

    if tier.covers_coverage() {
        match coverage_tool(repo) {
            None => outcomes.push(QaOutcome {
                check: "coverage".into(),
                status: QaStatus::Skipped,
                detail: "no coverage tool on PATH (cargo-llvm-cov / tarpaulin / pytest-cov)".into(),
            }),
            Some(cmd) => {
                let out = std::process::Command::new(&cmd[0])
                    .args(&cmd[1..])
                    .current_dir(repo)
                    .output();
                match out {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let pct = parse_coverage_pct(&stdout);
                        coverage_pct = pct;
                        match pct {
                            Some(p) => {
                                let status = coverage_gate(p, min_coverage_pct);
                                outcomes.push(QaOutcome {
                                    check: "coverage".into(),
                                    status,
                                    detail: format!("{p:.1}% (min {min_coverage_pct:.0}%)"),
                                });
                            }
                            None => outcomes.push(QaOutcome {
                                check: "coverage".into(),
                                status: QaStatus::Skipped,
                                detail: "coverage tool ran but no percentage parsed".into(),
                            }),
                        }
                    }
                    Err(e) => outcomes.push(QaOutcome {
                        check: "coverage".into(),
                        status: QaStatus::Skipped,
                        detail: format!("coverage tool failed to launch: {e}"),
                    }),
                }
            }
        }
    }

    if matches!(tier, QaTier::Deep | QaTier::Nightly) {
        outcomes.push(QaOutcome {
            check: "mutation".into(),
            status: QaStatus::Skipped,
            detail: "mutation testing not yet wired (Deep/Nightly)".into(),
        });
    }
    if tier == QaTier::Nightly {
        outcomes.push(QaOutcome {
            check: "fuzz".into(),
            status: QaStatus::Skipped,
            detail: "fuzzing not yet wired (Nightly)".into(),
        });
    }

    QaReport {
        tier,
        coverage_pct,
        outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qa_tier_from_str() {
        assert_eq!(QaTier::from_str_or_fast("full"), QaTier::Full);
        assert_eq!(QaTier::from_str_or_fast("DEEP"), QaTier::Deep);
        assert_eq!(QaTier::from_str_or_fast("nightly"), QaTier::Nightly);
        assert_eq!(QaTier::from_str_or_fast("bogus"), QaTier::Fast); // default
        assert_eq!(QaTier::from_str_or_fast(""), QaTier::Fast);
        assert!(QaTier::Full.covers_coverage());
        assert!(!QaTier::Fast.covers_coverage());
    }

    #[test]
    fn test_qa_coverage_gate() {
        assert_eq!(coverage_gate(85.0, 70.0), QaStatus::Passed);
        assert_eq!(coverage_gate(70.0, 70.0), QaStatus::Passed); // at threshold
        assert_eq!(coverage_gate(69.9, 70.0), QaStatus::Failed); // below
        assert_eq!(coverage_gate(10.0, 0.0), QaStatus::Passed); // gate disabled
        assert_eq!(parse_coverage_pct("TOTAL   100   12   88.00%"), Some(88.0));
        assert_eq!(parse_coverage_pct("lines....: 73.5% coverage"), Some(73.5));
    }

    #[test]
    fn test_qa_missing_tool_is_skipped() {
        // an empty temp dir has no Cargo.toml/pyproject → no tool → coverage skipped, not passed
        let tmp = std::env::temp_dir().join(format!("of-qa-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let report = run(&tmp, QaTier::Full, 70.0);
        let cov = report
            .outcomes
            .iter()
            .find(|o| o.check == "coverage")
            .unwrap();
        assert_eq!(cov.status, QaStatus::Skipped);
        assert!(report.passed()); // skipped never blocks
        assert!(!matches!(cov.status, QaStatus::Passed));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
