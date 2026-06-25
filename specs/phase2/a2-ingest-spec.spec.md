spec: task
name: "openfab-ingest-spec-and-requirements"
tags: []
---

## Intent

OpenFab can build from a spec authored elsewhere (the `wf_coordinator` requirements
conversation produces a `.spec.md`), and records the requirements document in the signed
provenance so the requirements→spec→code chain is fully traceable.

## Decisions

- Reuse `adapters::agent_spec::author_from_md` for ingesting a provided `.spec.md`.
- Spec source is selected by env: `OPENFAB_SPEC_FILE` (ingest a file) takes precedence over
  `OPENFAB_SPEC=agent-spec` (LLM-draft), which takes precedence over the native LLM author.
- The requirements doc lives at `<OPENFAB_SPEC_DIR>/<id>.requirements.md`; its SHA-256 goes
  into the `openfab/generation` predicate as `requirements_sha256` (signed, tamper-evident).

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not add external HTTP-client crates.

## Completion Criteria

Scenario: requirements doc hash is recorded in the signed provenance and tampering breaks it
  Test:
    Filter: test_requirements_sha256_recorded_and_tamper_breaks
  Given a generation attestation built with a requirements_sha256
  When the predicate's requirements_sha256 is altered after signing
  Then signature verification fails

Scenario: requirements hash helper reads the requirements doc file
  Test:
    Filter: test_requirements_sha256_helper_reads_file
  Given a requirements.md on disk for a spec id
  When the requirements hash helper runs
  Then it returns the sha256 of the file contents

Scenario: spec source selection prefers an explicit spec file
  Test:
    Filter: test_spec_source_prefers_file
  Given OPENFAB_SPEC_FILE is set
  When the spec source is resolved
  Then it selects the ingest-from-file source

## Out of Scope

- The dashboard rendering of documents (that is task A3).
