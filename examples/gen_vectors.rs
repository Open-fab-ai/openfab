//! Generate the PUBLISHED conformance vectors under docs/vectors/ (spec rev 0.1.4, F5):
//! real signed attestations, with and without sign-offs, under published test seeds.
//! Run: cargo run --example gen_vectors
use openfab::core::identity::Identity;
use openfab::core::provenance::{AcceptanceCheck, Attestation, GeneratedRange, GenerationInput};

// PUBLISHED TEST SEEDS — committed on purpose so anyone can reproduce/extend the
// vectors. Never use these for a real identity.
const FAB_SEED: [u8; 32] = [1u8; 32];
const ALICE_SEED: [u8; 32] = [2u8; 32];
const BOB_SEED: [u8; 32] = [3u8; 32];

fn input(did: String) -> GenerationInput {
    GenerationInput {
        spec_ref: "vector#v1".into(),
        app_name: "vector-app".into(),
        source_bundle_sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into(),
        agent_did: did,
        base_name: "vector-base".into(),
        model: "vector-model".into(),
        prompt: "published conformance vector".into(),
        params: serde_json::json!({}),
        generated: vec![GeneratedRange {
            path: "app/index.html".into(),
            lines: "1-1".into(),
            // sha256 of the literal file content "hello\n"
            sha256: "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03".into(),
            author: "ai".into(),
        }],
        materials: vec![],
        acceptance_passed: true,
        acceptance: vec![AcceptanceCheck {
            id: "a1".into(),
            check: "test -f app/index.html".into(),
            must_pass: true,
            passed: true,
        }],
    }
}

fn main() -> anyhow::Result<()> {
    let fab = Identity::from_seed("fab", FAB_SEED);
    let alice = Identity::from_seed("alice", ALICE_SEED);
    let bob = Identity::from_seed("bob", BOB_SEED);

    let att = Attestation::build_and_sign(input(fab.did()), &fab)?;
    std::fs::write("docs/vectors/rust-signed.json", att.to_json()?)?;

    let mut with = Attestation::build_and_sign(input(fab.did()), &fab)?;
    with.add_signoff(&alice)?;
    with.add_signoff(&bob)?;
    std::fs::write("docs/vectors/rust-signed-with-signoffs.json", with.to_json()?)?;
    println!("wrote docs/vectors/rust-signed{{,-with-signoffs}}.json (fab {})", fab.did());
    Ok(())
}
