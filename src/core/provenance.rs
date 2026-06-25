//! Provenance — the moat's signature artifact (PRD §5, "genuinely new code" #2).
//!
//! Every product the fab makes carries a **signed in-toto Statement** whose predicate
//! is OpenFab's custom `openfab/generation` predicate: it records the agent **DID**,
//! the **model**, the **prompt hash**, generation **params**, and the changed
//! **file/line ranges** with an **ai/human author tag** — which is what enables
//! AI-vs-Human attribution and spec-as-contract.
//!
//! Format: an in-toto Statement v1, signed DSSE-style (ed25519 over the canonical
//! JSON of the statement). Production swaps (PRD §5): cosign/fulcio/rekor for the
//! transparency log; slsa-verifier for SLSA verification. The signature scheme here
//! is verifiable offline with nothing but the embedded `did:key`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::core::identity::{self, Identity};
use crate::core::sha256_hex;
use crate::core::timeutil;

pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
pub const PREDICATE_TYPE: &str = "https://openfab.ai/attestation/generation/v0.1";

/// in-toto subject: the thing the attestation is about (here: the generated app's
/// frozen source bundle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    pub name: String,
    pub digest: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Digest {
    pub sha256: String,
}

/// Who built it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Builder {
    pub id: String,   // "openfab/0.1"
    pub base: String, // the base name, e.g. "claude-cli"
}

/// The agent that authored the code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub did: String,
    pub base: String,
    pub model: String,
}

/// One file (or line range) and who authored it — the attribution unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedRange {
    pub path: String,
    /// e.g. "1-42" — the line range authored.
    pub lines: String,
    pub sha256: String,
    /// "ai" or "human".
    pub author: String,
}

/// A material/context input that fed the generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// One acceptance check, embedded in the signed predicate so the **frozen contract**
/// travels with the artifact (any clone, any forge, offline) — not just the pass/fail
/// verdict. This is what makes `reproduce` forge-agnostic: the verifier re-runs these
/// exact commands, no local run-state needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCheck {
    pub id: String,
    /// The shell command (exit 0 = pass).
    pub check: String,
    pub must_pass: bool,
    /// The result recorded at build time (the verifier re-derives this independently).
    pub passed: bool,
}

/// Recorded human sign-off (folded into the predicate at acceptance time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignoffRecord {
    pub did: String,
    pub name: String,
    pub timestamp: String,
}

/// The `openfab/generation` predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenfabGeneration {
    pub spec_ref: String,
    pub builder: Builder,
    pub agent: Agent,
    pub prompt_sha256: String,
    pub params: serde_json::Value,
    pub generated: Vec<GeneratedRange>,
    pub materials: Vec<Material>,
    pub acceptance_passed: bool,
    /// The frozen acceptance contract (the actual check commands), embedded so the
    /// artifact is self-verifying off any forge. Empty on pre-v0.2 attestations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<AcceptanceCheck>,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signoffs: Vec<SignoffRecord>,
}

/// An in-toto Statement v1 with the OpenFab generation predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statement {
    #[serde(rename = "_type")]
    pub _type: String,
    pub subject: Vec<Subject>,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub predicate: OpenfabGeneration,
}

/// A signature over the statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttSignature {
    pub keyid: String, // did:key
    pub sig: String,   // base64 ed25519
    pub algo: String,  // "ed25519"
    pub role: String,  // "fab" | "human-signoff"
}

/// The signed attestation envelope (DSSE-style). `payload_sha256` is the digest of the
/// canonical JSON of `statement` — exactly the bytes the signatures cover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    pub payload_type: String,
    pub payload_sha256: String,
    pub statement: Statement,
    pub signatures: Vec<AttSignature>,
}

/// Inputs to build a generation attestation (neutral data, so `core` stays
/// independent of `ports`: the loop maps a `RunResult` into this).
pub struct GenerationInput {
    pub spec_ref: String,
    pub app_name: String,
    pub source_bundle_sha256: String,
    pub agent_did: String,
    pub base_name: String,
    pub model: String,
    pub prompt: String,
    pub params: serde_json::Value,
    pub generated: Vec<GeneratedRange>,
    pub materials: Vec<Material>,
    pub acceptance_passed: bool,
    pub acceptance: Vec<AcceptanceCheck>,
}

impl Attestation {
    /// Build and sign a generation attestation with the fab identity.
    pub fn build_and_sign(input: GenerationInput, fab: &Identity) -> Result<Attestation> {
        let predicate = OpenfabGeneration {
            spec_ref: input.spec_ref,
            builder: Builder {
                id: "openfab/0.1".to_string(),
                base: input.base_name.clone(),
            },
            agent: Agent {
                did: input.agent_did,
                base: input.base_name,
                model: input.model,
            },
            prompt_sha256: sha256_hex(input.prompt.as_bytes()),
            params: input.params,
            generated: input.generated,
            materials: input.materials,
            acceptance_passed: input.acceptance_passed,
            acceptance: input.acceptance,
            timestamp: timeutil::iso_now(),
            signoffs: vec![],
        };
        let statement = Statement {
            _type: STATEMENT_TYPE.to_string(),
            subject: vec![Subject {
                name: input.app_name,
                digest: Digest {
                    sha256: input.source_bundle_sha256,
                },
            }],
            predicate_type: PREDICATE_TYPE.to_string(),
            predicate,
        };
        let canonical = canonical_json(&statement)?;
        let sig = fab.sign_b64(canonical.as_bytes());
        Ok(Attestation {
            payload_type: "application/vnd.in-toto+json".to_string(),
            payload_sha256: sha256_hex(canonical.as_bytes()),
            statement,
            signatures: vec![AttSignature {
                keyid: fab.did(),
                sig,
                algo: "ed25519".to_string(),
                role: "fab".to_string(),
            }],
        })
    }

    /// Append a human sign-off signature and record it in the predicate. The signed
    /// bytes are the *same* original statement payload (the sign-off endorses exactly
    /// what the fab produced), then we re-pin the payload digest after recording.
    pub fn add_signoff(&mut self, signer: &Identity) -> Result<()> {
        // The human signs the original payload digest binding (what they reviewed).
        let canonical = canonical_json(&self.statement)?;
        let sig = signer.sign_b64(canonical.as_bytes());
        self.statement.predicate.signoffs.push(SignoffRecord {
            did: signer.did(),
            name: signer.name().to_string(),
            timestamp: timeutil::iso_now(),
        });
        self.signatures.push(AttSignature {
            keyid: signer.did(),
            sig,
            algo: "ed25519".to_string(),
            role: "human-signoff".to_string(),
        });
        Ok(())
    }

    /// Verify the fab signature (and any human sign-offs) against the embedded DIDs.
    /// Returns the list of DIDs whose signatures verified. The fab signature covers
    /// the canonical statement *without* the sign-off records (the state at build);
    /// each human sign-off covers the statement state at the moment they signed.
    pub fn verify_signatures(&self) -> Result<VerifiedSigners> {
        // Reconstruct the fab-time statement: the predicate had no signoffs yet.
        let mut at_build = self.statement.clone();
        at_build.predicate.signoffs.clear();
        let build_payload = canonical_json(&at_build)?;

        if sha256_hex(build_payload.as_bytes()) != self.payload_sha256 {
            // payload_sha256 must match the fab-time payload (tamper check).
            bail!("attestation payload digest does not match the fab-time statement (tampered?)");
        }

        let mut fab = vec![];
        let mut humans = vec![];
        for (i, s) in self.signatures.iter().enumerate() {
            match s.role.as_str() {
                "fab" => {
                    identity::verify_b64(&s.keyid, build_payload.as_bytes(), &s.sig)
                        .with_context(|| format!("fab signature #{i} failed to verify"))?;
                    fab.push(s.keyid.clone());
                }
                "human-signoff" => {
                    // Reconstruct the statement state when this signer signed: the
                    // predicate held the sign-offs recorded *before* this one.
                    let nth = humans.len();
                    let mut at_sign = self.statement.clone();
                    at_sign.predicate.signoffs.truncate(nth);
                    let payload = canonical_json(&at_sign)?;
                    identity::verify_b64(&s.keyid, payload.as_bytes(), &s.sig)
                        .with_context(|| format!("human sign-off #{i} failed to verify"))?;
                    humans.push(s.keyid.clone());
                }
                other => bail!("unknown signature role '{other}'"),
            }
        }
        if fab.is_empty() {
            bail!("attestation has no valid fab signature");
        }
        Ok(VerifiedSigners { fab, humans })
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize attestation")
    }

    pub fn from_json(s: &str) -> Result<Attestation> {
        serde_json::from_str(s).context("parse attestation")
    }
}

/// The signers whose signatures verified.
#[derive(Debug, Clone)]
pub struct VerifiedSigners {
    pub fab: Vec<String>,
    pub humans: Vec<String>,
}

/// Deterministic canonical JSON: object keys sorted recursively, compact separators.
/// This is what we sign, so signer and verifier always agree on the bytes.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let v = serde_json::to_value(value).context("to canonical value")?;
    let mut out = String::new();
    write_canonical(&v, &mut out);
    Ok(out)
}

fn write_canonical(v: &serde_json::Value, out: &mut String) {
    use serde_json::Value::*;
    match v {
        Null => out.push_str("null"),
        Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Number(n) => out.push_str(&n.to_string()),
        String(s) => out.push_str(&serde_json::to_string(s).unwrap()),
        Array(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(e, out);
            }
            out.push(']');
        }
        Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(did: &str) -> GenerationInput {
        GenerationInput {
            spec_ref: "demo#v1".to_string(),
            app_name: "demo-app".to_string(),
            source_bundle_sha256: "abc123".to_string(),
            agent_did: did.to_string(),
            base_name: "mock".to_string(),
            model: "mock-1".to_string(),
            prompt: "build a thing".to_string(),
            params: serde_json::json!({"temperature": 0}),
            generated: vec![GeneratedRange {
                path: "app/main.py".to_string(),
                lines: "1-10".to_string(),
                sha256: "deadbeef".to_string(),
                author: "ai".to_string(),
            }],
            materials: vec![],
            acceptance_passed: true,
            acceptance: vec![],
        }
    }

    #[test]
    fn build_sign_and_verify() {
        let fab = Identity::generate("fab").unwrap();
        let att = Attestation::build_and_sign(sample_input(&fab.did()), &fab).unwrap();
        let v = att.verify_signatures().unwrap();
        assert_eq!(v.fab.len(), 1);
        assert!(v.humans.is_empty());
        assert_eq!(att.statement.predicate_type, PREDICATE_TYPE);
    }

    #[test]
    fn signoff_then_verify_two() {
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let mut att = Attestation::build_and_sign(sample_input(&fab.did()), &fab).unwrap();
        att.add_signoff(&alice).unwrap();
        att.add_signoff(&bob).unwrap();
        let v = att.verify_signatures().unwrap();
        assert_eq!(v.fab.len(), 1);
        assert_eq!(v.humans.len(), 2);
        assert_eq!(att.statement.predicate.signoffs.len(), 2);
    }

    #[test]
    fn tampering_with_code_breaks_verification() {
        let fab = Identity::generate("fab").unwrap();
        let mut att = Attestation::build_and_sign(sample_input(&fab.did()), &fab).unwrap();
        // An attacker swaps the generated file digest after signing.
        att.statement.predicate.generated[0].sha256 = "0000".to_string();
        assert!(att.verify_signatures().is_err());
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = serde_json::json!({"b": 1, "a": {"d": 2, "c": 3}});
        assert_eq!(canonical_json(&v).unwrap(), r#"{"a":{"c":3,"d":2},"b":1}"#);
    }
}
