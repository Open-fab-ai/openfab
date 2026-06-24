spec: task
name: "openfab-project-git-worktree"
tags: []
---

## Intent

When registering a project that points at an existing git repo, OpenFab can create an
isolated git worktree (instead of operating on the user's live working tree), so self-hosting
— developing OpenFab with OpenFab — never disturbs the user's checkout.

## Decisions

- The worktree path is `<projects_dir>/<name>` and the new branch is `openfab/<name>`.
- The git command is constructed purely and unit-tested; the actual `git worktree add` is a
  thin shell-out reusing that command.

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not create a worktree at a path outside the projects dir.

## Completion Criteria

Scenario: the worktree git command is constructed for a source repo and destination
  Test:
    Filter: test_worktree_add_command
  Given a source repo, a destination path and a branch name
  When the worktree add command args are built
  Then they are ["-C", src, "worktree", "add", dest, "-b", branch]

Scenario: the worktree path and branch are derived from the project name
  Test:
    Filter: test_worktree_paths
  Given a projects dir and a project name
  When the worktree path and branch are derived
  Then the path is "<projects_dir>/<name>" and the branch is "openfab/<name>"

## Out of Scope

- The dashboard checkbox wiring.
