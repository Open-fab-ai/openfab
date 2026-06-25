spec: task
name: "openfab-multi-project-registry"
tags: []
---

## Intent

OpenFab manages multiple projects (each its own repo/workspace with its own runs and
maintainers), selected per request, while keeping the existing single-workspace behavior as
the default project so nothing breaks.

## Decisions

- A project is `{ name, repo }`; the registry is a JSON list persisted under the projects
  dir. Resolution is pure: a request's project name maps to its repo, or the default repo.
- Project names are restricted (alphanumeric, `-`, `_`) to prevent path traversal.

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not let a project name contain path separators or `..`.

## Completion Criteria

Scenario: an unset or default project resolves to the default repo
  Test:
    Filter: test_resolve_project_repo_default
  Given a registry and no project name
  When the project repo is resolved
  Then it returns the default repo

Scenario: a registered project resolves to its own repo
  Test:
    Filter: test_resolve_project_repo_registered
  Given a registry containing project "alpha" at its repo
  When resolving project "alpha"
  Then it returns alpha's repo

Scenario: an unknown project is rejected
  Test:
    Filter: test_resolve_project_repo_unknown
  Given a registry without project "ghost"
  When resolving project "ghost"
  Then resolution fails

Scenario: an unsafe project name is rejected
  Test:
    Filter: test_valid_project_name_rejects_traversal
  Given project names containing "/" or ".."
  When the name is validated
  Then validation fails

## Out of Scope

- The console project-switcher UI.
