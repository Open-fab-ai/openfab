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
pub const PREDICATE_TYPE: &str = "https://open-fab.ai/attestation/generation/v0.1";

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
    /// Canonical `AGENT_NAME:MODEL_VERSION` identifier — the Linux-kernel `Assisted-by:`
    /// convention (adopted by OpenSSF TIs), so the commit trailer and this predicate share
    /// one vocabulary and can be cross-checked. Optional + omitted when absent so v0.1
    /// attestations round-trip byte-identically (canonical-JSON signatures stay valid).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    /// Tools the agent used during generation (the kernel convention's `[tool1] [tool2]`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tools: Option<Vec<String>>,
}

/// The kernel-convention `Assisted-by:` identifier for an agent base + model.
pub fn assisted_by_id(base: &str, model: &str) -> String {
    format!("{base}:{model}")
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
    /// The statement EXACTLY as parsed from the received JSON (spec rev 0.1.4, F4).
    /// Verification preimages are built from this, never from the typed round-trip —
    /// a typed re-serialization silently drops unknown members, so a member added
    /// AFTER signing would vanish from the hashed bytes and the tampered file would
    /// still verify. `None` for attestations built in-process (nothing was parsed).
    #[serde(skip)]
    pub raw_statement: Option<serde_json::Value>,
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
                id: Some(assisted_by_id(&input.base_name, &input.model)),
                base: input.base_name,
                model: input.model,
                tools: None,
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
            raw_statement: None,
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
        // Spec rev 0.1.4 (F2/F3): the n-th sign-off signature covers the statement
        // with the first n records INCLUDING ITS OWN — so no record is ever outside
        // a signed preimage. (Before rev 0.1.4 the record was appended after signing,
        // leaving the last record covered by no signature.)
        self.statement.predicate.signoffs.push(SignoffRecord {
            did: signer.did(),
            name: signer.name().to_string(),
            timestamp: timeutil::iso_now(),
        });
        let canonical = canonical_json(&self.statement)?;
        let sig = signer.sign_b64(canonical.as_bytes());
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
        use crate::core::canonical::statement_preimage;
        // Preimages come from the RAW parsed statement when we have one (F4): the
        // received bytes are authoritative, and a typed round-trip must never be
        // able to drop what was — or was not — signed. In-process attestations
        // (raw_statement = None) fall back to the typed statement we just built.
        let raw_owned;
        let raw: &serde_json::Value = match &self.raw_statement {
            Some(r) => r,
            None => {
                raw_owned = serde_json::to_value(&self.statement)
                    .context("statement to value")?;
                &raw_owned
            }
        };
        // Fab signature + payload_sha256 cover the statement WITHOUT signoffs (F2).
        let build_payload = canonical_json(&statement_preimage(raw, None)?)?;
        if sha256_hex(build_payload.as_bytes()) != self.payload_sha256 {
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
                    // The n-th sign-off signature covers the statement with the
                    // first n records, ITS OWN INCLUDED (F2) — so every record is
                    // inside a signed preimage.
                    let nth = humans.len();
                    let rec = self
                        .statement
                        .predicate
                        .signoffs
                        .get(nth)
                        .with_context(|| format!("sign-off signature #{i} has no matching record (one signature per record, rev 0.1.4)"))?;
                    // A record is bound to its signer: record.did == signature.keyid (F3).
                    if rec.did != s.keyid {
                        bail!("signoffs[{nth}].did '{}' does not match its signature keyid '{}' (rev 0.1.4)", rec.did, s.keyid);
                    }
                    let payload = canonical_json(&statement_preimage(raw, Some(nth + 1))?)?;
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
        // Exactly one sign-off signature per record (F3): an appended record with no
        // signature would otherwise inflate any count derived from the records.
        if humans.len() != self.statement.predicate.signoffs.len() {
            bail!("{} sign-off record(s) but {} sign-off signature(s) — every record must be signed (rev 0.1.4)",
                self.statement.predicate.signoffs.len(), humans.len());
        }
        Ok(VerifiedSigners { fab, humans })
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize attestation")
    }

    pub fn from_json(s: &str) -> Result<Attestation> {
        // Duplicate member names are refused outright (rev 0.1.4 / I-JSON): with
        // last-wins parsing, a signature can cover one reading of a duplicated
        // member while another parser acts on the other.
        let raw = crate::core::canonical::parse_json_no_dups(s)?;
        let mut att: Attestation =
            serde_json::from_value(raw.clone()).context("parse attestation")?;
        att.raw_statement = raw.get("statement").cloned();
        Ok(att)
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
    // Spec rev 0.1.4 (F6): integers only, inside the I-JSON safe range. Floats and
    // larger integers hash differently across the reference implementations.
    crate::core::canonical::validate_value_domain(&v)?;
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
            // Spec rev 0.1.4 (F1): member names ordered by UTF-16 CODE UNITS, exactly
            // as RFC 8785 — NOT Unicode code points. The two differ for names mixing
            // non-BMP characters with U+E000..U+FFFF, and JS `sort()` is UTF-16 order,
            // so this keeps Rust and the browser signing identical bytes.
            keys.sort_by(|a, b| {
                a.encode_utf16()
                    .collect::<Vec<u16>>()
                    .cmp(&b.encode_utf16().collect::<Vec<u16>>())
            });
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

    /// F1 (rev 0.1.4): member names order by UTF-16 CODE UNITS (RFC 8785), which
    /// differs from code-point order for non-BMP vs U+E000..U+FFFF names — and is
    /// what JavaScript's sort() does, so both implementations sign the same bytes.
    #[test]
    fn canonical_key_order_is_utf16_code_units() {
        let v = serde_json::json!({"\u{1F600}": 1, "\u{FF61}": 2});
        // UTF-16: U+1F600 starts with surrogate 0xD83D < 0xFF61 → the emoji sorts first.
        assert_eq!(canonical_json(&v).unwrap(), "{\"\u{1F600}\":1,\"\u{FF61}\":2}");
    }

    /// F6 (rev 0.1.4): integers only, I-JSON safe range.
    #[test]
    fn canonical_value_domain_gate() {
        assert!(canonical_json(&serde_json::json!({"t": 0.5})).is_err());
        assert!(canonical_json(&serde_json::json!({"n": 9007199254740993i64})).is_err());
        assert!(canonical_json(&serde_json::json!({"n": 9007199254740991i64})).is_ok());
    }

    /// F3 (rev 0.1.4): every sign-off record is inside a signed preimage and bound
    /// to its signer.
    #[test]
    fn signoff_records_are_signed_and_bound() {
        let fab = Identity::generate("fab").unwrap();
        let alice = Identity::generate("alice").unwrap();
        let mut att = Attestation::build_and_sign(sample_input(&fab.did()), &fab).unwrap();
        att.add_signoff(&alice).unwrap();
        assert!(att.verify_signatures().is_ok());

        // Tamper the LAST record's name after signing (pre-0.1.4 this verified).
        let mut t = att.clone();
        t.statement.predicate.signoffs[0].name = "mallory".to_string();
        assert!(t.verify_signatures().is_err(), "tampered last record must fail");

        // Append a record with no signature (inflated the browser count pre-0.1.4).
        let mut t = att.clone();
        t.statement.predicate.signoffs.push(SignoffRecord {
            did: alice.did(),
            name: "alice-again".into(),
            timestamp: "2026-09-25T00:00:00Z".into(),
        });
        assert!(t.verify_signatures().is_err(), "unsigned record must fail");

        // Record naming a key that did not sign.
        let mut t = att.clone();
        t.statement.predicate.signoffs[0].did = "did:key:z6MkSomeoneElse".into();
        assert!(t.verify_signatures().is_err(), "did/keyid mismatch must fail");
    }

    /// F4 (rev 0.1.4): the RECEIVED statement is authoritative — an unknown member
    /// added after signing fails (the typed round-trip used to drop it silently),
    /// and a duplicate member name is refused at parse (I-JSON).
    #[test]
    fn raw_statement_is_authoritative() {
        let fab = Identity::generate("fab").unwrap();
        let att = Attestation::build_and_sign(sample_input(&fab.did()), &fab).unwrap();
        let json = att.to_json().unwrap();
        assert!(Attestation::from_json(&json).unwrap().verify_signatures().is_ok());

        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["statement"]["predicate"]["injected"] = serde_json::json!("after-signing");
        let tampered = serde_json::to_string(&v).unwrap();
        assert!(
            Attestation::from_json(&tampered).unwrap().verify_signatures().is_err(),
            "member added after signing must fail (was silently dropped pre-0.1.4)"
        );

        let dup = json.replacen("\"payload_type\":", "\"payload_type\": \"x\", \"payload_type\":", 1);
        assert!(Attestation::from_json(&dup).is_err(), "duplicate member must be refused");
    }

    /// Golden conformance vector (spec rev 0.1.3, "Envelope encoding"). The pinned
    /// sha256 was independently produced by the BROWSER implementation's canonicalJson
    /// over the same statement — this test proves the two implementations emit
    /// byte-identical canonical form, and pins the encoding against drift. If this
    /// test ever needs a new hash, that is a BREAKING change to signature
    /// verification and must be a new predicate version, not a rev.
    #[test]
    fn canonical_encoding_golden_vector() {
        let stmt = serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [{ "name": "golden-v1", "digest": { "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" } }],
            "predicateType": "https://open-fab.ai/attestation/generation/v0.1",
            "predicate": {
                "spec_ref": "golden#v1",
                "builder": { "id": "openfab/0.1", "base": "golden" },
                "agent": { "did": "did:key:z6MkGOLDEN", "base": "golden", "model": "test-model", "id": "golden:test-model" },
                "prompt_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "params": {},
                "generated": [{ "path": "app/中文 \"quoted\"\npath.js", "lines": "1-1", "sha256": "00", "author": "ai" }],
                "materials": [],
                "acceptance_passed": true,
                "acceptance": [{ "id": "a1", "check": "js:true", "must_pass": true, "passed": true }],
                "timestamp": "2026-09-22T00:00:00Z"
            }
        });
        let canon = canonical_json(&stmt).unwrap();
        assert_eq!(canon.len(), 756, "canonical byte length drifted");
        assert_eq!(
            sha256_hex(canon.as_bytes()),
            "7051cb7073a3bee0a038255fd59d4679c95443bd6a81bb92d0ff3e765713bacd",
            "canonical encoding drifted from the golden vector (browser-computed)"
        );
    }

    #[test]
    fn published_vectors_verify() {
        // Cross-implementation conformance (F5): the committed vectors from BOTH
        // reference implementations — with and without sign-offs — must verify
        // here. The browser vectors were signed by web/fabcrypto.js; verifying
        // them in Rust proves the two implementations agree on the canonical
        // bytes and the rev 0.1.4 signature-coverage rules, not just on one
        // pinned statement. Regenerate with `cargo run --example gen_vectors`
        // and docs/vectors/TEST-KEYS.json (published test keys).
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/vectors");
        for (file, signoffs) in [
            ("rust-signed.json", 0),
            ("rust-signed-with-signoffs.json", 2),
            ("browser-signed.json", 0),
            ("browser-signed-with-signoffs.json", 2),
        ] {
            let text = std::fs::read_to_string(format!("{dir}/{file}"))
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            let att = Attestation::from_json(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
            let v = att
                .verify_signatures()
                .unwrap_or_else(|e| panic!("{file}: verification failed: {e:#}"));
            assert_eq!(v.fab.len(), 1, "{file}: expected one fab signature");
            assert_eq!(v.humans.len(), signoffs, "{file}: sign-off count");
        }
    }
}
