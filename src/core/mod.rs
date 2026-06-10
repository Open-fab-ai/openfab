//! OpenFab Core — the moat. Base- and forge-independent.
//!
//! Golden rule (AGENTS.md): nothing in `core/` may import `adapters/` or assume any
//! specific base (AgentScope/HiClaw/claude) or forge (GitHub/Forgejo). Core depends
//! only on `ports` *types* (TaskCard etc. actually live here in `spec`, and ports
//! depend on core — never the reverse).

pub mod conformance;
pub mod identity;
pub mod provenance;
pub mod reputation;
pub mod sbom;
pub mod spec;
pub mod timeutil;
pub mod trust;

/// A small helper used across core: sha256 hex of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}
