//! Reputation — derived purely from signed attestations (PRD §5: "derived from rekor /
//! signed attestations"). No separate trust DB; reputation is a *projection* over the
//! provenance trail, so it can be recomputed by anyone from the committed evidence.
//!
//! v0.1 keeps it to the basic attestation-derived stat the PRD scopes (Non-goals:
//! "reputation beyond a basic attestation-derived stat").

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::provenance::Attestation;

/// Per-identity reputation, recomputable from attestations alone.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentStat {
    pub did: String,
    /// Attestations where this DID is the fab/agent author.
    pub authored: u32,
    /// Of those, how many passed machine acceptance.
    pub accepted: u32,
    /// Distinct sign-offs this identity has given as a maintainer.
    pub signoffs_given: u32,
}

impl AgentStat {
    pub fn acceptance_rate(&self) -> f64 {
        if self.authored == 0 {
            0.0
        } else {
            self.accepted as f64 / self.authored as f64
        }
    }
}

/// Fold a set of attestations into a reputation table keyed by DID.
pub fn compute(attestations: &[Attestation]) -> BTreeMap<String, AgentStat> {
    let mut table: BTreeMap<String, AgentStat> = BTreeMap::new();
    for att in attestations {
        let agent_did = att.statement.predicate.agent.did.clone();
        let entry = table.entry(agent_did.clone()).or_default();
        entry.did = agent_did;
        entry.authored += 1;
        if att.statement.predicate.acceptance_passed {
            entry.accepted += 1;
        }
        for so in &att.statement.predicate.signoffs {
            let e = table.entry(so.did.clone()).or_default();
            e.did = so.did.clone();
            e.signoffs_given += 1;
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::identity::Identity;
    use crate::core::provenance::{GeneratedRange, GenerationInput};

    fn att(fab: &Identity, passed: bool) -> Attestation {
        Attestation::build_and_sign(
            GenerationInput {
                spec_ref: "d#v1".into(),
                app_name: "d".into(),
                source_bundle_sha256: "a".into(),
                agent_did: fab.did(),
                base_name: "mock".into(),
                model: "m".into(),
                prompt: "p".into(),
                params: serde_json::json!({}),
                generated: vec![GeneratedRange {
                    path: "a".into(),
                    lines: "1".into(),
                    sha256: "x".into(),
                    author: "ai".into(),
                }],
                materials: vec![],
                acceptance_passed: passed,
                acceptance: vec![],
            },
            fab,
        )
        .unwrap()
    }

    #[test]
    fn computes_acceptance_rate_and_signoffs() {
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let mut a1 = att(&fab, true);
        a1.add_signoff(&alice).unwrap();
        let a2 = att(&fab, false);
        let table = compute(&[a1, a2]);
        let fab_stat = &table[&fab.did()];
        assert_eq!(fab_stat.authored, 2);
        assert_eq!(fab_stat.accepted, 1);
        assert!((fab_stat.acceptance_rate() - 0.5).abs() < 1e-9);
        assert_eq!(table[&alice.did()].signoffs_given, 1);
    }
}
