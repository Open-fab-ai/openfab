spec: task
name: "openfab-upload-requirements-or-spec"
tags: []
---

## Intent

A user can upload a pre-written document in the dashboard input — either an agent-spec
`.spec.md` contract (used directly) or a requirements/decision document (stored as the run's
requirements doc) — so they do not have to retype it.

## Decisions

- Classification is by content/filename: a body whose frontmatter starts with `spec:` (or a
  `.spec.md` name) is a spec contract; anything else is a requirements document.
- An uploaded doc is persisted into the project's spec dir as `<id>.spec.md` or
  `<id>.requirements.md`; the id is slugged from the provided name.

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not write outside the project's spec dir.

## Completion Criteria

Scenario: a .spec.md upload is classified as a spec contract
  Test:
    Filter: test_classify_upload_spec
  Given an uploaded body beginning with "spec: task"
  When the upload kind is classified
  Then it is classified as spec

Scenario: a prose document upload is classified as requirements
  Test:
    Filter: test_classify_upload_requirements
  Given an uploaded body of plain requirements prose
  When the upload kind is classified
  Then it is classified as requirements

Scenario: the destination filename is derived safely from the name and kind
  Test:
    Filter: test_upload_dest_name
  Given a display name and a classified kind
  When the destination filename is computed
  Then it is "<slug>.spec.md" or "<slug>.requirements.md" with a safe slug

## Out of Scope

- The multipart/file picker UI wiring (front-end).
