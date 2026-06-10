//! Trust gate — the single most sensitive component (PRD §6). It decides whether a
//! change is accepted. The same gate that guards untrusted crowd contributions guards
//! OpenFab's self-modification, so it is conservative by construction:
//!
//!   accept ⇔ valid fab signature ∧ fab DID allowlisted ∧ base allowlisted
//!            ∧ machine acceptance passed ∧ N-of-M human sign-off (distinct maintainers)
//!
//! Production swap (PRD §5): OPA/Rego via `regorus`. v0.1 reads the rule *parameters*
//! from `policy/trust.json` (the single machine-read source — `policy/trust.rego` is
//! the illustrative production policy and intentionally does NOT re-encode the literal
//! values, per R3). The gate also vets every sandbox command (allow/deny lists).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::core::provenance::Attestation;

/// N-of-M sign-off threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NofM {
    pub n: usize,
    pub m: usize,
}

/// Sandbox allow/deny rules — vetted before any command runs in `run_sandboxed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub allow_command_prefixes: Vec<String>,
    pub deny_substrings: Vec<String>,
}

/// The trust policy (parameters only; the maintainer/fab DID allowlists are runtime
/// data supplied separately, never hardcoded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub require_human_signoff: bool,
    pub n_of_m: NofM,
    pub allowed_bases: Vec<String>,
    pub sandbox: SandboxPolicy,
    /// The trust gate itself may be self-developed but is ALWAYS versioned, never
    /// hot-loaded, never self-approved (PRD §6). v0.1: false (no hot-load plane).
    pub trust_gate_self_modifiable: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            require_human_signoff: true,
            n_of_m: NofM { n: 2, m: 2 },
            allowed_bases: vec![
                "claude-cli".into(),
                "mock".into(),
                "agentscope".into(),
                "hiclaw".into(),
                "agent-chat".into(),
                "openhands".into(),
            ],
            sandbox: SandboxPolicy {
                allow_command_prefixes: vec![
                    "python3".into(),
                    "python".into(),
                    "node".into(),
                    "pytest".into(),
                    "cargo".into(),
                    "bash".into(),
                    "sh".into(),
                    "/bin/sh".into(),
                    "true".into(),
                    "false".into(),
                    "test".into(),
                    "grep".into(),
                ],
                deny_substrings: vec![
                    "rm -rf /".into(),
                    ":(){".into(),
                    "curl ".into(),
                    "wget ".into(),
                    "sudo ".into(),
                    " nc ".into(),
                    "/etc/passwd".into(),
                ],
            },
            trust_gate_self_modifiable: false,
        }
    }
}

impl Policy {
    pub fn from_json(s: &str) -> Result<Policy> {
        serde_json::from_str(s).context("parse trust policy json")
    }

    pub fn from_path(path: &Path) -> Result<Policy> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading policy {}", path.display()))?;
        Policy::from_json(&text)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize policy")
    }

    /// Derive a policy for a human-approval *gate mode*. The gate is a policy choice, not
    /// a fixed rule (real workflows differ): a solo developer self-approves the release; a
    /// team or an open/crowd project requires N-of-M distinct maintainers; `none` ships
    /// without a human gate (provenance still recorded). Everything else (sandbox,
    /// allowlists) is inherited unchanged.
    pub fn for_gate_mode(&self, mode: &str) -> Policy {
        let mut p = self.clone();
        match mode {
            // Solo: one approval — you accepting your own release. Still signed + logged.
            "solo" => {
                p.require_human_signoff = true;
                p.n_of_m = NofM { n: 1, m: 1 };
            }
            // Team: two distinct reviewers before it lands in the shared trusted repo.
            "team" => {
                p.require_human_signoff = true;
                p.n_of_m = NofM { n: 2, m: 2 };
            }
            // Crowd / untrusted agents: the gate IS the trust mechanism — N-of-M from the
            // pre-approved maintainer set guards contributions you don't otherwise trust.
            "crowd" => {
                p.require_human_signoff = true;
                p.n_of_m = NofM { n: 2, m: 3 };
            }
            // No human gate (provenance + machine acceptance still apply).
            "none" => {
                p.require_human_signoff = false;
            }
            _ => {}
        }
        p
    }

    /// Vet a sandbox command against the allow/deny lists. Deny wins. Returns an error
    /// (never silently swallows — R5) describing why a command is refused.
    pub fn check_command(&self, cmd: &[String]) -> Result<()> {
        let joined = cmd.join(" ");
        for deny in &self.sandbox.deny_substrings {
            if joined.contains(deny.as_str()) {
                anyhow::bail!("sandbox: command denied by policy (matched '{deny}'): {joined}");
            }
        }
        let prog = cmd.first().map(String::as_str).unwrap_or("");
        let prog_base = Path::new(prog)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(prog);
        let allowed = self
            .sandbox
            .allow_command_prefixes
            .iter()
            .any(|p| prog == p || prog_base == p || joined.starts_with(p.as_str()));
        if !allowed {
            anyhow::bail!("sandbox: command '{prog}' is not on the allowlist: {joined}");
        }
        Ok(())
    }
}

/// Inputs to a trust decision (all neutral data; `core` stays port-independent).
pub struct TrustInput<'a> {
    pub att: &'a Attestation,
    pub fab_allowlist: &'a [String],
    pub maintainer_allowlist: &'a [String],
    pub base_name: &'a str,
    pub acceptance_passed: bool,
}

/// The gate's verdict, with human-readable reasons for the decision log / audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub accepted: bool,
    pub satisfied: Vec<String>,
    pub blocking: Vec<String>,
}

/// Evaluate the trust gate. `accepted` is true only when every required condition holds.
pub fn evaluate(policy: &Policy, input: &TrustInput) -> Decision {
    let mut satisfied = vec![];
    let mut blocking = vec![];

    // 1. Cryptographic integrity + signer recovery.
    let verified = match input.att.verify_signatures() {
        Ok(v) => {
            satisfied.push(format!(
                "fab signature valid ({}); {} human sign-off signature(s) valid",
                short(&v.fab.first().cloned().unwrap_or_default()),
                v.humans.len()
            ));
            Some(v)
        }
        Err(e) => {
            blocking.push(format!("signature verification failed: {e}"));
            None
        }
    };

    // 2. Fab DID allowlisted.
    if let Some(v) = &verified {
        let fab_did = v.fab.first().cloned().unwrap_or_default();
        if input.fab_allowlist.iter().any(|d| d == &fab_did) {
            satisfied.push(format!("fab identity {} is allowlisted", short(&fab_did)));
        } else {
            blocking.push(format!(
                "fab identity {} is NOT allowlisted",
                short(&fab_did)
            ));
        }
    }

    // 3. Base allowlisted.
    if policy.allowed_bases.iter().any(|b| b == input.base_name) {
        satisfied.push(format!("base '{}' is allowlisted", input.base_name));
    } else {
        blocking.push(format!("base '{}' is NOT allowlisted", input.base_name));
    }

    // 4. Machine acceptance passed.
    if input.acceptance_passed {
        satisfied.push("machine acceptance checks passed in sandbox".to_string());
    } else {
        blocking.push("machine acceptance checks did NOT pass".to_string());
    }

    // 5. N-of-M human sign-off by DISTINCT allowlisted maintainers.
    if policy.require_human_signoff {
        let distinct: std::collections::BTreeSet<&String> = verified
            .as_ref()
            .map(|v| {
                v.humans
                    .iter()
                    .filter(|did| input.maintainer_allowlist.iter().any(|m| m == *did))
                    .collect()
            })
            .unwrap_or_default();
        let count = distinct.len();
        if count >= policy.n_of_m.n {
            satisfied.push(format!(
                "human sign-off satisfied: {}-of-{} ({} distinct allowlisted maintainers)",
                policy.n_of_m.n, policy.n_of_m.m, count
            ));
        } else {
            blocking.push(format!(
                "human sign-off NOT satisfied: need {} distinct allowlisted maintainers, have {}",
                policy.n_of_m.n, count
            ));
        }
    } else {
        satisfied.push("human sign-off not required by policy".to_string());
    }

    Decision {
        accepted: blocking.is_empty(),
        satisfied,
        blocking,
    }
}

/// Short form of a DID for human-readable logs.
fn short(did: &str) -> String {
    if did.len() > 20 {
        format!("{}…{}", &did[..14], &did[did.len() - 4..])
    } else {
        did.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::identity::Identity;
    use crate::core::provenance::{Attestation, GeneratedRange, GenerationInput};

    fn att_for(fab: &Identity) -> Attestation {
        Attestation::build_and_sign(
            GenerationInput {
                spec_ref: "demo#v1".into(),
                app_name: "demo".into(),
                source_bundle_sha256: "abc".into(),
                agent_did: fab.did(),
                base_name: "mock".into(),
                model: "mock-1".into(),
                prompt: "p".into(),
                params: serde_json::json!({}),
                generated: vec![GeneratedRange {
                    path: "a".into(),
                    lines: "1-1".into(),
                    sha256: "x".into(),
                    author: "ai".into(),
                }],
                materials: vec![],
                acceptance_passed: true,
            },
            fab,
        )
        .unwrap()
    }

    #[test]
    fn blocks_without_signoff() {
        let policy = Policy::default(); // 2-of-2 required
        let fab = Identity::generate("fab").unwrap();
        let att = att_for(&fab);
        let d = evaluate(
            &policy,
            &TrustInput {
                att: &att,
                fab_allowlist: &[fab.did()],
                maintainer_allowlist: &[],
                base_name: "mock",
                acceptance_passed: true,
            },
        );
        assert!(!d.accepted);
        assert!(d.blocking.iter().any(|b| b.contains("sign-off")));
    }

    #[test]
    fn accepts_with_two_distinct_maintainers() {
        let policy = Policy::default();
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let mut att = att_for(&fab);
        att.add_signoff(&alice).unwrap();
        att.add_signoff(&bob).unwrap();
        let d = evaluate(
            &policy,
            &TrustInput {
                att: &att,
                fab_allowlist: &[fab.did()],
                maintainer_allowlist: &[alice.did(), bob.did()],
                base_name: "mock",
                acceptance_passed: true,
            },
        );
        assert!(d.accepted, "blocking: {:?}", d.blocking);
    }

    #[test]
    fn same_maintainer_signing_twice_does_not_satisfy_2of2() {
        let policy = Policy::default();
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let mut att = att_for(&fab);
        att.add_signoff(&alice).unwrap();
        att.add_signoff(&alice).unwrap();
        let d = evaluate(
            &policy,
            &TrustInput {
                att: &att,
                fab_allowlist: &[fab.did()],
                maintainer_allowlist: &[alice.did()],
                base_name: "mock",
                acceptance_passed: true,
            },
        );
        assert!(!d.accepted, "one maintainer cannot satisfy 2-of-2");
    }

    #[test]
    fn unallowlisted_fab_is_blocked() {
        let policy = Policy::default();
        let fab = Identity::generate("fab").unwrap();
        let att = att_for(&fab);
        let d = evaluate(
            &policy,
            &TrustInput {
                att: &att,
                fab_allowlist: &[], // not allowlisted
                maintainer_allowlist: &[],
                base_name: "mock",
                acceptance_passed: true,
            },
        );
        assert!(!d.accepted);
        assert!(d.blocking.iter().any(|b| b.contains("allowlisted")));
    }

    #[test]
    fn sandbox_denies_dangerous_and_unlisted_commands() {
        let policy = Policy::default();
        assert!(policy
            .check_command(&["python3".into(), "app.py".into()])
            .is_ok());
        assert!(policy
            .check_command(&["bash".into(), "-c".into(), "rm -rf /".into()])
            .is_err());
        assert!(policy
            .check_command(&["telnet".into(), "evil".into()])
            .is_err());
    }
}
