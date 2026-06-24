//! Conformance — does an artifact meet the OpenFab profile? (PRD §5: "OpenFab profile /
//! conformance spec".) This is what `openfab verify` checks: the attestation is
//! well-formed, its signatures verify, it records the required generation metadata,
//! machine acceptance passed, and (if required) human sign-off is present.
//!
//! Conformance is deliberately separate from `trust`: `trust` decides *acceptance for
//! merge* (needs the runtime allowlists); `conformance` decides *is this a valid
//! OpenFab artifact* (self-contained, checkable by anyone from the file alone).

use serde::{Deserialize, Serialize};

use crate::core::provenance::{Attestation, PREDICATE_TYPE, STATEMENT_TYPE};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub conformant: bool,
    pub checks: Vec<CheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

impl ConformanceReport {
    fn push(&mut self, id: &str, passed: bool, detail: impl Into<String>) {
        if !passed {
            self.conformant = false;
        }
        self.checks.push(CheckResult {
            id: id.to_string(),
            passed,
            detail: detail.into(),
        });
    }
}

/// Check an attestation against the OpenFab v0.1 profile. `require_signoff` reflects
/// the spec's `human_signoff_required`.
pub fn check(att: &Attestation, require_signoff: bool) -> ConformanceReport {
    let mut r = ConformanceReport {
        conformant: true,
        checks: vec![],
    };
    let p = &att.statement;

    r.push(
        "C1.statement-type",
        p._type == STATEMENT_TYPE,
        format!("_type = {}", p._type),
    );
    r.push(
        "C2.predicate-type",
        p.predicate_type == PREDICATE_TYPE,
        format!("predicateType = {}", p.predicate_type),
    );
    r.push(
        "C3.subject-digest",
        p.subject.iter().all(|s| s.digest.sha256.len() >= 6),
        format!("{} subject(s) with sha256 digest", p.subject.len()),
    );
    let pred = &p.predicate;
    r.push(
        "C4.agent-did",
        pred.agent.did.starts_with("did:key:"),
        format!("agent DID = {}", pred.agent.did),
    );
    r.push(
        "C5.model-recorded",
        !pred.agent.model.is_empty(),
        format!("model = {}", pred.agent.model),
    );
    r.push(
        "C6.prompt-hash",
        pred.prompt_sha256.len() == 64,
        "prompt sha256 present".to_string(),
    );
    r.push(
        "C7.attribution",
        !pred.generated.is_empty() && pred.generated.iter().all(|g| !g.author.is_empty()),
        format!(
            "{} file/range(s) carry an ai/human author tag",
            pred.generated.len()
        ),
    );
    r.push(
        "C8.spec-ref",
        pred.spec_ref.contains("#v"),
        format!("spec_ref = {}", pred.spec_ref),
    );

    // Cryptographic verification.
    match att.verify_signatures() {
        Ok(v) => {
            r.push(
                "C9.fab-signature",
                true,
                format!("{} fab signature(s) verify", v.fab.len()),
            );
            if require_signoff {
                r.push(
                    "C10.human-signoff",
                    !v.humans.is_empty(),
                    format!("{} human sign-off signature(s) verify", v.humans.len()),
                );
            }
        }
        Err(e) => {
            r.push(
                "C9.fab-signature",
                false,
                format!("signature verification failed: {e}"),
            );
        }
    }

    r.push(
        "C11.machine-acceptance",
        pred.acceptance_passed,
        "machine acceptance recorded as passed".to_string(),
    );

    // C12 — agent-spec contract gate. Applicability is decided by whether the spec was
    // authored via agent-spec (the signed `spec_contract_sha256` is present), NOT by whether
    // verdicts happen to be present — otherwise stripping the verdicts would silently skip
    // the gate. When it applies, there must be ≥1 scenario and every one must be `pass`
    // (skip ≠ pass). N/A only for the native-spec path (no contract hash).
    if pred.spec_contract_sha256.is_some() {
        let non_pass: Vec<&str> = pred
            .agent_spec_verdicts
            .iter()
            .filter(|v| v.verdict != "pass")
            .map(|v| v.scenario.as_str())
            .collect();
        let ok = !pred.agent_spec_verdicts.is_empty() && non_pass.is_empty();
        r.push(
            "C12.agent-spec-scenarios",
            ok,
            if pred.agent_spec_verdicts.is_empty() {
                "spec has a signed contract hash but no scenario verdicts were recorded".into()
            } else if non_pass.is_empty() {
                format!(
                    "{} agent-spec scenario(s) all pass",
                    pred.agent_spec_verdicts.len()
                )
            } else {
                format!("non-passing scenario(s): {}", non_pass.join(", "))
            },
        );
    }

    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::identity::Identity;
    use crate::core::provenance::{GeneratedRange, GenerationInput, ScenarioVerdict};

    fn att(fab: &Identity) -> Attestation {
        att_with_verdicts(fab, vec![])
    }

    fn att_with_verdicts(fab: &Identity, verdicts: Vec<ScenarioVerdict>) -> Attestation {
        Attestation::build_and_sign(
            GenerationInput {
                spec_ref: "demo#v1".into(),
                app_name: "demo".into(),
                source_bundle_sha256: "abcabc".into(),
                agent_did: fab.did(),
                base_name: "mock".into(),
                model: "mock-1".into(),
                prompt: "p".into(),
                params: serde_json::json!({}),
                generated: vec![GeneratedRange {
                    path: "app/main.py".into(),
                    lines: "1-10".into(),
                    sha256: "deadbeef".into(),
                    author: "ai".into(),
                }],
                materials: vec![],
                acceptance_passed: true,
                spec_contract_sha256: if verdicts.is_empty() {
                    None
                } else {
                    Some("c".repeat(64))
                },
                agent_spec_verdicts: verdicts,
                run_log_ref: None,
                requirements_sha256: None,
            },
            fab,
        )
        .unwrap()
    }

    fn verdict(scenario: &str, verdict: &str) -> ScenarioVerdict {
        ScenarioVerdict {
            scenario: scenario.into(),
            verdict: verdict.into(),
        }
    }

    #[test]
    fn well_formed_attestation_is_conformant_without_signoff() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), false);
        assert!(r.conformant, "{:?}", r.checks);
    }

    #[test]
    fn missing_signoff_fails_when_required() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), true);
        assert!(!r.conformant);
        assert!(r
            .checks
            .iter()
            .any(|c| c.id == "C10.human-signoff" && !c.passed));
    }

    #[test]
    fn agent_spec_all_pass_is_conformant() {
        let fab = Identity::generate("fab").unwrap();
        let a = att_with_verdicts(
            &fab,
            vec![verdict("happy", "pass"), verdict("edge", "pass")],
        );
        let r = check(&a, false);
        assert!(r.conformant, "{:?}", r.checks);
        assert!(r
            .checks
            .iter()
            .any(|c| c.id == "C12.agent-spec-scenarios" && c.passed));
    }

    #[test]
    fn agent_spec_skip_breaks_conformance() {
        let fab = Identity::generate("fab").unwrap();
        let a = att_with_verdicts(
            &fab,
            vec![verdict("happy", "pass"), verdict("edge", "skip")],
        );
        let r = check(&a, false);
        assert!(!r.conformant);
        assert!(r
            .checks
            .iter()
            .any(|c| c.id == "C12.agent-spec-scenarios" && !c.passed));
    }

    #[test]
    fn no_verdicts_skips_c12() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), false);
        // native-spec path: C12 is not applicable and must not appear
        assert!(!r.checks.iter().any(|c| c.id == "C12.agent-spec-scenarios"));
    }

    #[test]
    fn agent_spec_contract_without_verdicts_fails_c12() {
        // Bypass attempt: a spec authored via agent-spec (contract hash present) but with the
        // verdicts stripped must FAIL C12, not skip it.
        let fab = Identity::generate("fab").unwrap();
        let att = Attestation::build_and_sign(
            GenerationInput {
                spec_ref: "demo#v1".into(),
                app_name: "demo".into(),
                source_bundle_sha256: "abcabc".into(),
                agent_did: fab.did(),
                base_name: "mock".into(),
                model: "mock-1".into(),
                prompt: "p".into(),
                params: serde_json::json!({}),
                generated: vec![GeneratedRange {
                    path: "app/main.py".into(),
                    lines: "1-10".into(),
                    sha256: "deadbeef".into(),
                    author: "ai".into(),
                }],
                materials: vec![],
                acceptance_passed: true,
                spec_contract_sha256: Some("c".repeat(64)), // contract present…
                agent_spec_verdicts: vec![],                // …but verdicts stripped
                run_log_ref: None,
                requirements_sha256: None,
            },
            &fab,
        )
        .unwrap();
        let r = check(&att, false);
        assert!(!r.conformant);
        assert!(r
            .checks
            .iter()
            .any(|c| c.id == "C12.agent-spec-scenarios" && !c.passed));
    }

    #[test]
    fn signoff_present_is_conformant() {
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let mut a = att(&fab);
        a.add_signoff(&alice).unwrap();
        let r = check(&a, true);
        assert!(r.conformant, "{:?}", r.checks);
    }
}
