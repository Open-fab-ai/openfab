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

/// Which verification mode produced a report's verdict. Spec rev 0.1.2: a verifier
/// MUST record which mode produced its verdict, and MUST NOT report an artifact as
/// conformant-by-execution unless the checks actually executed and passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerifyMode {
    /// Signatures + digests + attribution only; `acceptance_passed` is read as the
    /// producer's self-report, never as a machine result.
    AttestOnly,
    /// This verifier re-executed the embedded acceptance contract itself.
    ChecksExecuted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub conformant: bool,
    /// The mode that produced this verdict (rev 0.1.2 MUST).
    pub mode: VerifyMode,
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
/// the spec's `human_signoff_required`. `reexecuted_acceptance` is `Some(all_passed)`
/// ONLY when the caller re-executed the embedded acceptance contract itself; `None`
/// means attest-only, where the producer's `acceptance_passed` bit is reported as a
/// claim, never as a machine result (issue 44).
pub fn check(
    att: &Attestation,
    require_signoff: bool,
    reexecuted_acceptance: Option<bool>,
) -> ConformanceReport {
    let mut r = ConformanceReport {
        conformant: true,
        mode: match reexecuted_acceptance {
            Some(_) => VerifyMode::ChecksExecuted,
            None => VerifyMode::AttestOnly,
        },
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

    // C11 is a property of the BYTES: the producer signed the claim. It is never a
    // machine result — from the file alone all it settles is that the producer
    // signed the word true (issue 44; spec rev 0.1.2 self-report MUST).
    r.push(
        "C11.acceptance-claimed",
        pred.acceptance_passed,
        format!(
            "producer asserts acceptance_passed = {}, not re-executed by this verifier",
            pred.acceptance_passed
        ),
    );
    // C12 is reachable ONLY when this verifier executed the contract itself —
    // never from the file alone (spec rev 0.1.2 verdict-mode MUST).
    if let Some(reexecuted) = reexecuted_acceptance {
        r.push(
            "C12.acceptance-re-executed",
            reexecuted,
            format!("this verifier re-executed the embedded contract: all_passed = {reexecuted}"),
        );
    }

    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::identity::Identity;
    use crate::core::provenance::{GeneratedRange, GenerationInput};

    fn att(fab: &Identity) -> Attestation {
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
                acceptance: vec![],
            },
            fab,
        )
        .unwrap()
    }

    #[test]
    fn well_formed_attestation_is_conformant_without_signoff() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), false, None);
        assert!(r.conformant, "{:?}", r.checks);
    }

    #[test]
    fn missing_signoff_fails_when_required() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), true, None);
        assert!(!r.conformant);
        assert!(r
            .checks
            .iter()
            .any(|c| c.id == "C10.human-signoff" && !c.passed));
    }

    #[test]
    fn attest_only_reports_claim_not_machine_fact() {
        // Issue 44: without execution, C11 is the producer's claim, C12 unreachable,
        // and the report says which mode produced the verdict.
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), false, None);
        assert_eq!(r.mode, VerifyMode::AttestOnly);
        let c11 = r.checks.iter().find(|c| c.id == "C11.acceptance-claimed").unwrap();
        assert!(c11.detail.contains("not re-executed by this verifier"));
        assert!(!r.checks.iter().any(|c| c.id.starts_with("C12")));
    }

    #[test]
    fn executed_mode_adds_reexecuted_check() {
        let fab = Identity::generate("fab").unwrap();
        let r = check(&att(&fab), false, Some(true));
        assert_eq!(r.mode, VerifyMode::ChecksExecuted);
        assert!(r.checks.iter().any(|c| c.id == "C12.acceptance-re-executed" && c.passed));
        let failed = check(&att(&fab), false, Some(false));
        assert!(!failed.conformant, "re-executed failure must fail conformance");
    }

    #[test]
    fn signoff_present_is_conformant() {
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let mut a = att(&fab);
        a.add_signoff(&alice).unwrap();
        let r = check(&a, true, None);
        assert!(r.conformant, "{:?}", r.checks);
    }
}
