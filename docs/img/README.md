# Screenshots

Captured from the OpenFab web UI (`openfab serve` → http://127.0.0.1:8787) and referenced
from the top-level [`README.md`](../../README.md).

### Overview

| File | Shows |
|---|---|
| `openfab-ui.png` | the full web UI (hero shot) — NL in, live workflow, product + provenance |
| `demo-temperature-converter.png` | the generated Temperature Converter web app, running live |
| `maintainers-reputation.png` | the N-of-M maintainer allowlist + reputation projected from attestations |

### Live-workflow step panels (click a step in the stepper)

| File | Step | Value proposition it shows |
|---|---|---|
| `workflow-spec.png` | Spec | NL → versioned, machine-checkable **contract** (acceptance criteria) |
| `workflow-generate.png` | Generate | agent **DID** · model · prompt SHA-256 · per-file **ai/human attribution** (identity) |
| `workflow-verify.png` | Verify | acceptance criteria **re-run in the sandbox** (reproducibility) |
| `workflow-sign.png` | Sign | **signed provenance** — in-toto/SLSA, payload SHA-256, `did:key` signers |
| `workflow-gate.png` | Gate | **attestation & auditability** — C1–C11 conformance, the trust decision |

### Product-inspection tabs + the reproduce proof

| File | View | Value proposition it shows |
|---|---|---|
| `tab-provenance.png` | Provenance tab | the full signed `openfab/generation` attestation (provenance + attribution) |
| `tab-audit-trail.png` | Audit trail tab | **auditability** — signed git commit graph + provenance trailers (EU CRA / SLSA) |
| `tab-sbom.png` | SBOM tab | **supply chain** — SPDX bill of materials, files pinned by SHA-256 |
| `tab-decision-log.png` | Decision log tab | **decision memory** — the human-readable run timeline |
| `reproduce.png` | Reproduce & verify | **sovereign proof** — signature valid · source bit-identical · acceptance all pass |

To refresh: run a fabrication in the UI, click each step in the stepper / each product tab,
and screenshot the panel.
