spec: task
name: "openfab-github-rest-api-forge"
tags: []
---

## Intent

OpenFab can use the GitHub REST API (token-based, via api.github.com) to push and open pull
requests — without depending on the `gh` CLI — consistent with the Forgejo/Gitea/GitCode
token forges.

## Decisions

- Reuse the REST forge adapter for GitHub: a `github` kind whose API base is
  `https://api.github.com`, PR endpoint `/repos/<slug>/pulls`, and push host `github.com`.
- GitHub is configured by `OPENFAB_GITHUB_TOKEN` + `OPENFAB_GITHUB_REPO`; the API URL is
  fixed (no `OPENFAB_GITHUB_URL` required).

## Boundaries

### Allowed Changes
- src/**

### Forbidden
- Do not log or commit the token.

## Completion Criteria

Scenario: GitHub is configured from token and repo env
  Test:
    Filter: test_github_is_configured
  Given OPENFAB_GITHUB_TOKEN and OPENFAB_GITHUB_REPO are set
  When the github forge configuration is checked
  Then it reports configured

Scenario: the GitHub PR API url targets api.github.com
  Test:
    Filter: test_github_pr_api_url
  Given a repo slug "owner/repo"
  When the GitHub pull-request API url is built
  Then it is "https://api.github.com/repos/owner/repo/pulls"

Scenario: the GitHub push remote embeds the token for github.com
  Test:
    Filter: test_github_push_remote
  Given a token and slug "owner/repo"
  When the authenticated GitHub push remote is built
  Then it is "https://x-access-token:<token>@github.com/owner/repo.git"

## Out of Scope

- Live PR creation against a real GitHub repo (needs a token; manual checklist).
