# OpenFab live-forge matrix (plain HTTP)

Stand up **genuine** live forges so OpenFab classifies them as live (not the offline
local-git fallback) and reaches their real REST APIs. Friction-free: plain HTTP, sqlite,
no Caddy, no TLS, no `/etc/hosts` edits, no `sudo`.

| Forge   | Host URL                | Runtime                                   |
|---------|-------------------------|-------------------------------------------|
| GitHub  | github.com              | real repo `csheargm/openfab-forge-demo`   |
| Gitea   | http://localhost:3000   | container `gitea/gitea:1.22` (this dir)    |
| Forgejo | http://localhost:3001   | container `codeberg.org/forgejo/forgejo:7.0` |

## The exact contract (from `src/adapters/`)

`forge_rest.rs` (Gitea/Forgejo/GitCode share one adapter) reads three env vars per kind.
`RestForge::is_configured(kind)` — and therefore `registry::forge_live(kind)` — is true
**iff all three are set** (it only checks env presence; the URL scheme is irrelevant to
classification):

```
OPENFAB_<KIND>_URL     base url   (adapter trims a trailing slash)
OPENFAB_<KIND>_TOKEN   access token; sent as `Authorization: token <token>`
OPENFAB_<KIND>_REPO    "owner/repo" slug
```
`<KIND>` is upper-cased: `OPENFAB_GITEA_*`, `OPENFAB_FORGEJO_*`.
PR endpoint the adapter calls: `POST <url>/api/v1/repos/<owner/repo>/pulls`.

`forge_github.rs` is live when `OPENFAB_GITHUB_REMOTE=<git url>` is set.

## Known limitation — git push over plain HTTP (honest)

`forge_rest.rs::authed_remote()` (lines 64–71) hardcodes an `https://oauth2:<token>@host`
push URL regardless of the `OPENFAB_<KIND>_URL` scheme. So with these plain-HTTP forges:

- **Classification + all REST calls work** (token auth, repo create, branch create, the
  `…/pulls` PR endpoint) — these honor the configured `http://` base url.
- **A `git push` through the adapter FAILS** with a TLS error, because it dials `https://`
  at a plain-HTTP port. Verified: `tlsv1 alert protocol version`.
- The forge itself **is** push-capable: pushing to `http://oauth2:<token>@localhost:3000/openfab/demo.git`
  succeeds and returns a PR-compare URL. Only the adapter's hardcoded scheme is the blocker.

The prior iteration of this dir worked around this with a Caddy local-TLS proxy. That was
removed per the friction-free requirement. The genuine fix is a one-liner in
`authed_remote()`: preserve the scheme from `base_url` instead of forcing `https://`.

## Quick start

```bash
docker compose up -d                         # boots gitea (:3000) + forgejo (:3001)
# wait for /api/v1/version on each, then provision admin/token/repo (see this session's log)
source forges/forges.env                     # exports OPENFAB_GITEA_*, OPENFAB_FORGEJO_*, OPENFAB_GITHUB_REMOTE

# OpenFab now reports them live:
./target/release/openfab serve --repo /tmp/r --port 7800 &
curl -s localhost:7800/api/forges | python3 -m json.tool   # gitea/forgejo/github -> "live": true
```

`forges.env` holds real (throwaway, local-only) tokens and is gitignored.

## Files

- `docker-compose.yml` — `gitea/gitea:1.22`, `codeberg.org/forgejo/forgejo:7.0`; sqlite;
  `INSTALL_LOCK=true` (no install wizard); registration disabled; ports
  `127.0.0.1:3000` (gitea), `127.0.0.1:3001` (forgejo); healthchecks on `/api/healthz`.
- `forges.env` — `OPENFAB_*` exports (gitignored, contains tokens).
- `.gitignore` — ignores `*.env` and `volumes/`.

## Teardown

```bash
cd forges
docker compose down        # stop + remove containers, KEEP data volumes
docker compose down -v     # also delete gitea-data + forgejo-data volumes (full reset)
```
The GitHub repo `csheargm/openfab-forge-demo` persists on github.com; delete with
`gh repo delete csheargm/openfab-forge-demo --yes` if you want it gone.

## Credentials (throwaway, local only)

`openfab` / `openfab-demo-1`, email `openfab@local`, admin. Tokens scoped
`write:repository,write:user`. Do not reuse anywhere real.

## Verified live (this machine, this session)

- `GET /api/forges` with no env  → gitea/forgejo/github `live: false` (control).
- `GET /api/forges` with `forges.env` sourced → gitea/forgejo/github `live: true`,
  gitcode `live: false` (never configured — correct).
- Gitea  `/api/v1/version` → `1.22.6`; Forgejo → `7.0.16+gitea-1.21.11`.
- Real authenticated writes on `openfab/demo`: token→admin user, `POST …/branches` HTTP 201,
  adapter PR endpoint `GET …/pulls` HTTP 200, on both forges.
- Push limitation reproduced: adapter https push fails (TLS), correct-scheme http push succeeds.
