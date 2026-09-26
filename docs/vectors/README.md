# Signed conformance vectors (spec rev 0.1.4)

Four complete signed attestations, one from each reference implementation, with and
without human sign-offs:

| File | Producer | Sign-offs |
|---|---|---|
| `rust-signed.json` | `src/` (Rust CLI) | none |
| `rust-signed-with-signoffs.json` | `src/` (Rust CLI) | alice, bob |
| `browser-signed.json` | `web/` (browser, WebCrypto) | none |
| `browser-signed-with-signoffs.json` | `web/` (browser, WebCrypto) | alice, bob |

Every file verifies under the rev 0.1.4 rules in **both** implementations — the
Rust test `published_vectors_verify` (`src/core/provenance.rs`) cross-verifies all
four in CI, including the browser-signed ones, so the byte agreement between
implementations is machine-checked on every commit, not observed once.

What the sign-off vectors exercise: the fab signature and `payload_sha256` cover the
statement **without** `signoffs`; alice's signature covers the statement with record
1 (her own); bob's covers records 1–2 (his own included); each record's `did` equals
its signature's `keyid`.

## Test keys — published on purpose

`TEST-KEYS.json` holds the **private** keys so anyone can reproduce or extend these
vectors: the browser JWKs, and the Rust identities' fixed seeds (`[1u8;32]` fab,
`[2u8;32]` alice, `[3u8;32]` bob — `Identity::from_seed`). They sign test vectors
only. Never use them for a real identity; nothing signed by them should ever be
trusted.

## Regenerating

- Rust: `cargo run --example gen_vectors` (deterministic keys; timestamps change).
- Browser: the vectors were produced by `web/fabcrypto.js` + the sign-off flow in
  `web/ops_browser.js` running in a real browser, using the JWKs in
  `TEST-KEYS.json`.
