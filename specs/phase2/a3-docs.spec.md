spec: task
name: "openfab-run-document-bundle"
tags: []
---

## Intent

A run's documents (requirements, spec contract, design, code, README) are exposed as a
structured bundle so the dashboard can render them — replacing the single input box with
real document engineering.

## Decisions

- A document has a `name`, a `kind` (requirements | spec | design | code | readme | other),
  and `content`; the bundle is derived by classifying the committed repo files for a run.

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not read files outside the run's repo.

## Completion Criteria

Scenario: classifies a spec contract document by its filename
  Test:
    Filter: test_classify_doc_kind_spec_and_requirements
  Given file names like "specs/x.spec.md" and "specs/x.requirements.md"
  When the document kind is classified
  Then they are labelled spec and requirements respectively

Scenario: source files are classified as code
  Test:
    Filter: test_classify_doc_kind_code
  Given a file name like "src/main.rs"
  When the document kind is classified
  Then it is labelled code

## Out of Scope

- The web SPA rendering (that is the front-end portion of A3).
