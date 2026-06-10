//! Identity — `did:key` over ed25519 (PRD §5 reuse table: "did:key / did:web").
//!
//! Why self-contained ed25519 rather than the `didkit`/`ssi` crates: the dependency
//! budget (AGENTS.md) prefers the smallest design that satisfies the spec. A `did:key`
//! for ed25519 is just `did:key:z` + base58btc(0xed01 ‖ pubkey); signing is raw
//! ed25519. The *public key is embedded in the DID*, so verification needs no
//! keystore — exactly the portable, air-gapped property the PRD wants. Production
//! swap: Sigstore OIDC for human identities (recorded in `docs/OpenFab_MVP_Design_and_PRD.md` §5).
//!
//! Private seeds are persisted under `.openfab/` and are gitignored — never committed.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// The multicodec prefix for an ed25519 public key (varint 0xed 0x01).
const MULTICODEC_ED25519_PUB: [u8; 2] = [0xed, 0x01];

/// A signing identity: the fab itself, or a human maintainer. Holds the secret seed.
pub struct Identity {
    name: String,
    signing: SigningKey,
}

impl Identity {
    /// Create a fresh identity from the system CSPRNG.
    pub fn generate(name: &str) -> Result<Identity> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed)
            .map_err(|e| anyhow::anyhow!("CSPRNG seed for identity: {e}"))?;
        Ok(Identity {
            name: name.to_string(),
            signing: SigningKey::from_bytes(&seed),
        })
    }

    /// Load an identity's seed from disk, or create + persist one if absent.
    /// Seeds live under a gitignored directory so they never reach a commit.
    pub fn load_or_create(dir: &Path, name: &str) -> Result<Identity> {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
        let path = dir.join(format!("{name}.seed"));
        if path.exists() {
            let raw =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            if raw.len() != 32 {
                bail!(
                    "seed {} is corrupt ({} bytes, expected 32)",
                    path.display(),
                    raw.len()
                );
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&raw);
            Ok(Identity {
                name: name.to_string(),
                signing: SigningKey::from_bytes(&seed),
            })
        } else {
            let id = Identity::generate(name)?;
            std::fs::write(&path, id.signing.to_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            Ok(id)
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// This identity's `did:key`.
    pub fn did(&self) -> String {
        encode_did_key(&self.signing.verifying_key())
    }

    /// Sign a message; returns a base64 (standard) signature.
    pub fn sign_b64(&self, msg: &[u8]) -> String {
        use base64::Engine;
        let sig: Signature = self.signing.sign(msg);
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }
}

/// Encode an ed25519 verifying key as a `did:key`.
pub fn encode_did_key(vk: &VerifyingKey) -> String {
    let mut bytes = Vec::with_capacity(34);
    bytes.extend_from_slice(&MULTICODEC_ED25519_PUB);
    bytes.extend_from_slice(&vk.to_bytes());
    format!("did:key:z{}", bs58::encode(bytes).into_string())
}

/// Recover the ed25519 verifying key embedded in a `did:key`. This is what makes the
/// attestation self-verifying: no external keystore is consulted.
pub fn decode_did_key(did: &str) -> Result<VerifyingKey> {
    let rest = did
        .strip_prefix("did:key:z")
        .context("not a did:key:z multibase DID")?;
    let bytes = bs58::decode(rest)
        .into_vec()
        .context("did:key base58 decode")?;
    if bytes.len() != 34 || bytes[0..2] != MULTICODEC_ED25519_PUB {
        bail!("did:key is not an ed25519 key (bad multicodec/length)");
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&bytes[2..34]);
    VerifyingKey::from_bytes(&pk).context("invalid ed25519 public key in did:key")
}

/// Verify a base64 signature over `msg` against the public key inside `did`.
/// Returns Ok(()) only on a valid signature; any failure is an error with context.
pub fn verify_b64(did: &str, msg: &[u8], sig_b64: &str) -> Result<()> {
    use base64::Engine;
    let vk = decode_did_key(did)?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .context("signature is not valid base64")?;
    let sig = Signature::from_slice(&raw).context("signature is not 64 bytes")?;
    vk.verify(msg, &sig)
        .context("signature does not verify against did:key")?;
    Ok(())
}

/// A directory for fab/maintainer seeds, anchored at the repo's `.openfab/`.
pub fn identity_dir(repo: &Path, kind: &str) -> PathBuf {
    repo.join(".openfab").join(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_roundtrips_and_signs() {
        let id = Identity::generate("fab").unwrap();
        let did = id.did();
        assert!(did.starts_with("did:key:z"));
        let msg = b"openfab attestation payload";
        let sig = id.sign_b64(msg);
        verify_b64(&did, msg, &sig).expect("valid sig must verify");
    }

    #[test]
    fn tampered_message_fails() {
        let id = Identity::generate("fab").unwrap();
        let sig = id.sign_b64(b"original");
        assert!(verify_b64(&id.did(), b"tampered", &sig).is_err());
    }

    #[test]
    fn wrong_signer_fails() {
        let a = Identity::generate("a").unwrap();
        let b = Identity::generate("b").unwrap();
        let sig = a.sign_b64(b"m");
        assert!(verify_b64(&b.did(), b"m", &sig).is_err());
    }

    #[test]
    fn persisted_seed_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let d1 = Identity::load_or_create(tmp.path(), "fab").unwrap().did();
        let d2 = Identity::load_or_create(tmp.path(), "fab").unwrap().did();
        assert_eq!(d1, d2, "reloading the seed must yield the same DID");
    }
}
