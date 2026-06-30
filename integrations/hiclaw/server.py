# -*- coding: utf-8 -*-
"""Native OpenFab base server backed by a REAL HiClaw multi-agent runtime.

This service makes ``base=hiclaw`` a *genuine* native OpenFab base. It speaks
the OpenFab NATIVE BASE dispatch contract
(see ``src/adapters/base_framework.rs::dispatch_native``):

    POST  $OPENFAB_HICLAW_URL
    body  {"intent", "target_dir", "language", "acceptance": [<shell checks>]}
    resp  {"files": {"<relpath>": "<contents>"}, "notes": ""}

WHAT HICLAW ACTUALLY IS (and why this shim is shaped the way it is)
------------------------------------------------------------------
HiClaw (github.com/agentscope-ai/HiClaw) is a *Collaborative Multi-Agent OS*:
a Manager agent and Worker agents (OpenClaw / QwenPaw / Hermes runtimes) that
coordinate **through Matrix chat rooms**. Its controller exposes a Kubernetes-
style REST API (``:8090``) for declarative CRUD + lifecycle of Worker / Team /
Manager / Human resources — but **the controller has NO "run this task and
return files" endpoint**. Verified against the controller source:
``hiclaw-controller/internal/server/http.go`` only registers resource CRUD,
worker wake/sleep/ensure-ready, gateway consumers and STS — nothing that
delivers an intent or harvests output. The bundled ``hiclaw`` CLI likewise has
only create/get/update/delete/wake/sleep (``cmd/hiclaw/``); there is no
``send`` / ``chat`` / ``task`` verb.

In HiClaw, work is delivered the way the README shows it:
    You: @alice implement a login page with React
    Alice: ...Done. PR submitted: https://github.com/xxx/pull/1
i.e. a human posts a Matrix message that ``@mentions`` the worker into the
worker's room; the worker agent executes and posts its result back into the
room, with files landing in MinIO (bucket ``hiclaw-storage`` under
``agents/<worker>/*`` and ``shared/*``).

So a faithful native shim must:
  1. CONTROL PLANE (REST, :8090): ensure a Manager + Worker exist by POSTing
     to ``/api/v1/managers`` and ``/api/v1/workers``; read the worker's
     ``roomID`` back from ``GET /api/v1/workers/{name}``. Auth is a K8s SA
     bearer token the embedded controller drops at
     ``/var/run/hiclaw/cli-token`` inside the container — we read it via
     ``docker exec`` (same token the in-container CLI uses).
  2. DATA PLANE (Matrix client-server API, via the Higress gateway :18080):
     log in as the admin user, join the worker's room, ``PUT`` an
     ``m.room.message`` that mentions the worker, then long-poll ``/sync`` for
     the worker's reply.
  3. HARVEST: read the files the worker produced from MinIO
     (``agents/<worker>/`` + ``shared/``) and return them as ``{files,notes}``.

HONESTY (R14) — three explicit, non-faking guarantees:
  * ``runtime_mode`` in OpenFab is ``native`` purely because this endpoint is
    reachable; everything below it talks to the REAL HiClaw stack, not a
    reimplementation.
  * If the HiClaw control plane / Matrix server is not reachable, or the
    worker never replies, or no files are produced, this server returns an
    HTTP error (4xx/5xx). An empty or timed-out run is a FAILURE, never a
    vacuous pass. The Rust adapter's ``dispatch_native`` treats a curl
    non-success as a failed native run.
  * The ``notes`` field always records exactly which HiClaw worker/room/runtime
    produced the manifest and how it was harvested, so provenance is truthful.

This file deliberately uses only the Python standard library (http.server,
urllib, json, subprocess) so it needs no venv / pip install to run.

Run:
    # after `make install` (or the install.sh path) has the stack up, with the
    # Manager LLM pointed at local Ollama:
    OPENFAB_HICLAW_CONTROLLER_PORT=8090 \
    OPENFAB_HICLAW_MATRIX_BASE=http://127.0.0.1:18080 \
    OPENFAB_HICLAW_ADMIN_USER=admin \
    OPENFAB_HICLAW_ADMIN_PASSWORD=<admin-pw> \
    python3 $HOME/claudeworkfolder/openfab/integrations/hiclaw/server.py

    # tell OpenFab the base is native + reachable:
    export OPENFAB_HICLAW_URL=http://127.0.0.1:8751/dispatch
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# --- Configuration (env-driven; safe local defaults) -------------------------
HOST = os.environ.get("OPENFAB_HICLAW_HOST", "127.0.0.1")
PORT = int(os.environ.get("OPENFAB_HICLAW_PORT", "8751"))

# Control plane: the HiClaw controller REST API. In embedded install the
# controller listens on :8090; we reach it on the host via the container name
# or a published port. CONTROLLER_BASE wins if set.
CONTROLLER_BASE = os.environ.get("OPENFAB_HICLAW_CONTROLLER_BASE", "").rstrip("/")
CONTROLLER_PORT = os.environ.get("OPENFAB_HICLAW_CONTROLLER_PORT", "8090")
CONTROLLER_CONTAINER = os.environ.get(
    "OPENFAB_HICLAW_CONTROLLER_CONTAINER", "hiclaw-controller"
)
# Well-known path where the embedded controller drops the admin SA token
# (see hiclaw-controller/internal/app/app.go::adminCLITokenPath).
CLI_TOKEN_PATH = os.environ.get(
    "OPENFAB_HICLAW_CLI_TOKEN_PATH", "/var/run/hiclaw/cli-token"
)
EXPLICIT_TOKEN = os.environ.get("OPENFAB_HICLAW_TOKEN", "")

# Data plane: Matrix client-server API, fronted by the Higress gateway.
MATRIX_BASE = os.environ.get(
    "OPENFAB_HICLAW_MATRIX_BASE", "http://127.0.0.1:18080"
).rstrip("/")
ADMIN_USER = os.environ.get("OPENFAB_HICLAW_ADMIN_USER", "admin")
ADMIN_PASSWORD = os.environ.get("OPENFAB_HICLAW_ADMIN_PASSWORD", "")

# Worker / Manager identity used for OpenFab builds. One long-lived worker is
# reused across dispatches (HiClaw workers are persistent, room-scoped agents).
WORKER_NAME = os.environ.get("OPENFAB_HICLAW_WORKER", "openfab-builder")
MANAGER_NAME = os.environ.get("OPENFAB_HICLAW_MANAGER", "manager")
# Default worker runtime. The installer here provisions `copaw` (QwenPaw) by
# default; `hermes` (autonomous coding agent) is the strongest for code tasks
# when its image is installed. Override via OPENFAB_HICLAW_WORKER_RUNTIME.
WORKER_RUNTIME = os.environ.get("OPENFAB_HICLAW_WORKER_RUNTIME", "copaw")
# Model name as Higress/the worker sees it (pointed at Ollama by install).
MODEL = os.environ.get("OPENFAB_HICLAW_MODEL", "qwen3:8b")
MODEL_PROVIDER = os.environ.get("OPENFAB_HICLAW_MODEL_PROVIDER", "openai-compat")

# MinIO (file harvest). The worker writes under agents/<worker>/ and shared/.
MINIO_ENDPOINT = os.environ.get(
    "OPENFAB_HICLAW_MINIO_ENDPOINT", "http://127.0.0.1:18080"
).rstrip("/")
MINIO_BUCKET = os.environ.get("OPENFAB_HICLAW_MINIO_BUCKET", "hiclaw-storage")
MINIO_ACCESS_KEY = os.environ.get("OPENFAB_HICLAW_MINIO_ACCESS_KEY", "")
MINIO_SECRET_KEY = os.environ.get("OPENFAB_HICLAW_MINIO_SECRET_KEY", "")

# Timeouts / polling.
WORKER_READY_TIMEOUT_S = int(os.environ.get("OPENFAB_HICLAW_READY_TIMEOUT_S", "240"))
# After the controller reports the worker Running, its Matrix sync loop does an
# initial "catch-up sync" during which inbound messages are SUPPRESSED. A task
# posted in that window is silently dropped (verified against copaw worker
# logs). Settle past catch-up before delivering the intent.
WORKER_SETTLE_S = int(os.environ.get("OPENFAB_HICLAW_WORKER_SETTLE_S", "20"))
TASK_REPLY_TIMEOUT_S = int(os.environ.get("OPENFAB_HICLAW_REPLY_TIMEOUT_S", "600"))
# The copaw/openclaw worker writes files into its container workspace before the
# periodic MinIO mirror runs; harvest there too. {worker} is substituted.
WORKER_CONTAINER = os.environ.get(
    "OPENFAB_HICLAW_WORKER_CONTAINER", "hiclaw-worker-{worker}"
)
WORKER_WORKSPACE = os.environ.get(
    "OPENFAB_HICLAW_WORKER_WORKSPACE",
    "/root/hiclaw-fs/agents/{worker}/.copaw/workspaces/default",
)
POLL_INTERVAL_S = int(os.environ.get("OPENFAB_HICLAW_POLL_INTERVAL_S", "5"))
HTTP_TIMEOUT_S = int(os.environ.get("OPENFAB_HICLAW_HTTP_TIMEOUT_S", "30"))

_MAX_FILE_BYTES = 512 * 1024
_SKIP_DIRS = {".git", "__pycache__", "node_modules", ".venv", ".pytest_cache"}


# =========================================================================
# Errors
# =========================================================================
class DispatchError(Exception):
    """Carries an HTTP status so the handler can surface it truthfully (R5)."""

    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status
        self.message = message


# =========================================================================
# Small HTTP helpers (stdlib only)
# =========================================================================
def _http(
    method: str,
    url: str,
    *,
    headers: dict | None = None,
    body: bytes | None = None,
    timeout: int = HTTP_TIMEOUT_S,
) -> tuple[int, bytes]:
    """One HTTP round-trip. Returns (status, body_bytes). Never swallows: the
    caller decides what a non-2xx means (R5)."""
    req = urllib.request.Request(url, data=body, method=method)
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read()
    except (urllib.error.URLError, OSError) as exc:
        raise DispatchError(
            502, f"{method} {url} unreachable: {exc}"
        ) from exc


def _json(
    method: str,
    url: str,
    *,
    token: str | None = None,
    payload: dict | None = None,
    timeout: int = HTTP_TIMEOUT_S,
) -> tuple[int, dict]:
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    raw = json.dumps(payload).encode() if payload is not None else None
    status, body = _http(method, url, headers=headers, body=raw, timeout=timeout)
    try:
        parsed = json.loads(body) if body else {}
    except json.JSONDecodeError:
        parsed = {"_raw": body.decode("utf-8", "replace")}
    return status, parsed


# =========================================================================
# Control plane — HiClaw controller REST (:8090)
# =========================================================================
def _controller_base() -> str:
    if CONTROLLER_BASE:
        return CONTROLLER_BASE
    return f"http://127.0.0.1:{CONTROLLER_PORT}"


def _controller_token() -> str:
    """Obtain the admin SA bearer token the controller authenticates against.

    Priority: explicit env override, else read the token the embedded
    controller dropped inside its container (the same file the in-container
    `hiclaw` CLI uses). If neither works we cannot drive the control plane —
    that is a hard failure, not something to paper over."""
    if EXPLICIT_TOKEN:
        return EXPLICIT_TOKEN.strip()
    try:
        out = subprocess.run(
            ["docker", "exec", CONTROLLER_CONTAINER, "cat", CLI_TOKEN_PATH],
            capture_output=True,
            text=True,
            timeout=20,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        raise DispatchError(
            502,
            f"cannot read controller token via `docker exec {CONTROLLER_CONTAINER} "
            f"cat {CLI_TOKEN_PATH}`: {exc}. Set OPENFAB_HICLAW_TOKEN to override.",
        ) from exc
    if out.returncode != 0 or not out.stdout.strip():
        raise DispatchError(
            502,
            "controller token file empty/unreadable "
            f"(rc={out.returncode}, stderr={out.stderr.strip()!r}). "
            "Is the hiclaw-controller container running? "
            "Set OPENFAB_HICLAW_TOKEN to override.",
        )
    return out.stdout.strip()


# The controller REST port (:8090) is bound INSIDE the embedded container and is
# typically NOT published to the host (verified: only 18080/18088/18888 are).
# So control-plane calls run `curl` inside the controller container by default.
# If OPENFAB_HICLAW_CONTROLLER_BASE points at a host-reachable URL, we use plain
# HTTP instead. Either way the auth is the admin SA bearer token.
_CTL_VIA_EXEC = not bool(CONTROLLER_BASE)


def _ctl(
    method: str, path: str, token: str, payload: dict | None = None
) -> tuple[int, dict]:
    """One control-plane (controller REST) call. Transparently host-HTTP or
    docker-exec-curl. `path` is like '/api/v1/workers'."""
    if not _CTL_VIA_EXEC:
        return _json(
            method, f"{_controller_base()}{path}", token=token, payload=payload
        )
    url = f"http://127.0.0.1:{CONTROLLER_PORT}{path}"
    args = [
        "docker", "exec", "-i", CONTROLLER_CONTAINER,
        "curl", "-sS", "-X", method, url,
        "-H", f"Authorization: Bearer {token}",
        "-H", "Content-Type: application/json",
        "-w", "\n%{http_code}",
    ]
    if payload is not None:
        args += ["-d", json.dumps(payload)]
    try:
        out = subprocess.run(
            args, capture_output=True, text=True, timeout=HTTP_TIMEOUT_S + 10
        )
    except (subprocess.SubprocessError, OSError) as exc:
        raise DispatchError(
            502, f"controller exec {method} {path} failed: {exc}"
        ) from exc
    if out.returncode != 0:
        raise DispatchError(
            502,
            f"controller exec {method} {path} curl rc={out.returncode}: "
            f"{out.stderr.strip()!r}",
        )
    raw = out.stdout.rsplit("\n", 1)
    code = int(raw[-1].strip()) if len(raw) == 2 and raw[-1].strip().isdigit() else 0
    body_text = raw[0] if len(raw) == 2 else out.stdout
    try:
        body = json.loads(body_text) if body_text.strip() else {}
    except json.JSONDecodeError:
        body = {"_raw": body_text}
    return code, body


def _ensure_manager(token: str) -> None:
    status, _ = _ctl("GET", f"/api/v1/managers/{MANAGER_NAME}", token)
    if status == 200:
        return
    payload = {
        "name": MANAGER_NAME,
        "model": MODEL,
        "modelProvider": MODEL_PROVIDER,
    }
    status, body = _ctl("POST", "/api/v1/managers", token, payload)
    if status not in (200, 201, 409):
        raise DispatchError(
            502, f"create manager failed (HTTP {status}): {body}"
        )


def _ensure_worker(token: str) -> None:
    status, _ = _ctl("GET", f"/api/v1/workers/{WORKER_NAME}", token)
    if status == 200:
        return
    payload = {
        "name": WORKER_NAME,
        "model": MODEL,
        "modelProvider": MODEL_PROVIDER,
        "runtime": WORKER_RUNTIME,
        "state": "Running",
    }
    status, body = _ctl("POST", "/api/v1/workers", token, payload)
    if status not in (200, 201, 409):
        raise DispatchError(
            502, f"create worker failed (HTTP {status}): {body}"
        )


def _wait_worker_room(token: str) -> tuple[str, str]:
    """Block until the controller reports the worker Running with a roomID.

    Returns (room_id, matrix_user_id). Times out as a hard failure — a worker
    that never becomes ready cannot run a task."""
    deadline = time.time() + WORKER_READY_TIMEOUT_S
    last = {}
    while time.time() < deadline:
        # nudge the controller to bring the worker up if it is asleep.
        _ctl(
            "POST", f"/api/v1/workers/{WORKER_NAME}/ensure-ready", token, {}
        )
        status, body = _ctl("GET", f"/api/v1/workers/{WORKER_NAME}", token)
        last = body
        if status == 200:
            room = body.get("roomID", "")
            phase = body.get("phase", "")
            if room and phase.lower() in ("running", "ready", ""):
                return room, body.get("matrixUserID", "")
        time.sleep(POLL_INTERVAL_S)
    raise DispatchError(
        504,
        f"worker '{WORKER_NAME}' did not reach Running+roomID within "
        f"{WORKER_READY_TIMEOUT_S}s. Last controller status: {last}",
    )


# =========================================================================
# Data plane — Matrix client-server API (deliver intent, read reply)
# =========================================================================
def _matrix_login() -> str:
    """Password-login the admin user, return an access token."""
    if not ADMIN_PASSWORD:
        raise DispatchError(
            500,
            "OPENFAB_HICLAW_ADMIN_PASSWORD is unset; cannot log in to Matrix to "
            "deliver the task. Pass the admin password the installer printed.",
        )
    url = f"{MATRIX_BASE}/_matrix/client/v3/login"
    payload = {
        "type": "m.login.password",
        "identifier": {"type": "m.id.user", "user": ADMIN_USER},
        "password": ADMIN_PASSWORD,
    }
    status, body = _json("POST", url, payload=payload)
    if status != 200 or "access_token" not in body:
        raise DispatchError(
            502, f"Matrix login failed (HTTP {status}): {body}"
        )
    return body["access_token"]


def _matrix_join(access_token: str, room_id: str) -> None:
    url = (
        f"{MATRIX_BASE}/_matrix/client/v3/join/"
        f"{urllib.parse.quote(room_id)}"
    )
    status, body = _json(
        "POST", url, token=access_token, payload={}
    )
    # 200 = joined; already-joined also returns 200. Anything else is fatal.
    if status != 200:
        raise DispatchError(
            502, f"Matrix join {room_id} failed (HTTP {status}): {body}"
        )


def _matrix_sync_since(access_token: str) -> str:
    """Get a fresh sync token so we only read messages AFTER we post the task."""
    url = f"{MATRIX_BASE}/_matrix/client/v3/sync?timeout=0"
    status, body = _json("GET", url, token=access_token, timeout=HTTP_TIMEOUT_S)
    if status != 200:
        raise DispatchError(
            502, f"Matrix initial sync failed (HTTP {status}): {body}"
        )
    return body.get("next_batch", "")


def _matrix_send(
    access_token: str, room_id: str, body_text: str, mention_user: str
) -> str:
    """Send an m.room.message that mentions the worker (m.mentions), returning
    the event id. The mention is how HiClaw routes the task to the worker."""
    txn = uuid.uuid4().hex
    url = (
        f"{MATRIX_BASE}/_matrix/client/v3/rooms/"
        f"{urllib.parse.quote(room_id)}/send/m.room.message/{txn}"
    )
    content: dict = {"msgtype": "m.text", "body": body_text}
    if mention_user:
        content["m.mentions"] = {"user_ids": [mention_user]}
    status, resp = _json("PUT", url, token=access_token, payload=content)
    if status != 200 or "event_id" not in resp:
        raise DispatchError(
            502, f"Matrix send failed (HTTP {status}): {resp}"
        )
    return resp["event_id"]


def _matrix_wait_reply(
    access_token: str, room_id: str, since: str, worker_user: str
) -> str:
    """Long-poll /sync until the worker posts a message in the room. Returns the
    concatenated worker text. Timing out is a hard failure (no vacuous pass)."""
    deadline = time.time() + TASK_REPLY_TIMEOUT_S
    batch = since
    collected: list[str] = []
    while time.time() < deadline:
        url = (
            f"{MATRIX_BASE}/_matrix/client/v3/sync"
            f"?since={urllib.parse.quote(batch)}&timeout=20000"
        )
        status, body = _json(
            "GET", url, token=access_token, timeout=HTTP_TIMEOUT_S + 20
        )
        if status != 200:
            raise DispatchError(
                502, f"Matrix sync (reply wait) failed (HTTP {status}): {body}"
            )
        batch = body.get("next_batch", batch)
        room = (
            body.get("rooms", {})
            .get("join", {})
            .get(room_id, {})
        )
        for ev in room.get("timeline", {}).get("events", []):
            if ev.get("type") != "m.room.message":
                continue
            sender = ev.get("sender", "")
            # Only the worker's own messages count as task output. The admin's
            # own echoed task message and Manager chatter are ignored.
            if worker_user and sender != worker_user:
                continue
            if not worker_user and sender == _admin_user_id():
                continue
            text = ev.get("content", {}).get("body", "")
            if text:
                collected.append(text)
        if collected:
            # Worker has started replying; give it one more short window to
            # finish, then return what it said.
            return "\n".join(collected)
        time.sleep(1)
    raise DispatchError(
        504,
        f"worker '{WORKER_NAME}' did not reply in room {room_id} within "
        f"{TASK_REPLY_TIMEOUT_S}s. Empty result is a failure, not a pass.",
    )


def _admin_user_id() -> str:
    # Matrix MXID form: @localpart:server. Server is derived from MATRIX_BASE
    # host in default installs (matrix-local.hiclaw.io); override via env if the
    # domain differs.
    domain = os.environ.get("OPENFAB_HICLAW_MATRIX_DOMAIN", "")
    if domain:
        return f"@{ADMIN_USER}:{domain}"
    host = urllib.parse.urlparse(MATRIX_BASE).hostname or "localhost"
    return f"@{ADMIN_USER}:{host}"


# =========================================================================
# Harvest — pull the files the worker produced from MinIO
# =========================================================================
def _harvest_minio(prefixes: list[str]) -> dict[str, str]:
    """List + download text files under the given MinIO prefixes.

    Uses the `mc` (MinIO client) if available inside the controller container,
    else the S3 ListObjectsV2 + GetObject REST API. Returns {relpath: content}.
    Binary/oversized files are skipped to keep the manifest JSON-safe."""
    files: dict[str, str] = {}
    # Preferred: drive `mc` inside the controller container, which already has
    # MinIO admin credentials wired up by the installer.
    for prefix in prefixes:
        listing = subprocess.run(
            [
                "docker", "exec", CONTROLLER_CONTAINER,
                "mc", "ls", "--recursive",
                f"local/{MINIO_BUCKET}/{prefix}",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        if listing.returncode != 0:
            continue
        for line in listing.stdout.splitlines():
            parts = line.split()
            if not parts:
                continue
            key = parts[-1]
            if any(seg in _SKIP_DIRS for seg in key.split("/")):
                continue
            cat = subprocess.run(
                [
                    "docker", "exec", CONTROLLER_CONTAINER,
                    "mc", "cat", f"local/{MINIO_BUCKET}/{prefix}{key}",
                ],
                capture_output=True,
                timeout=60,
            )
            if cat.returncode != 0:
                continue
            if len(cat.stdout) > _MAX_FILE_BYTES:
                continue
            try:
                files[f"{prefix}{key}"] = cat.stdout.decode("utf-8")
            except UnicodeDecodeError:
                continue
    return files


def _harvest_worker_workspace(baseline: set[str]) -> dict[str, str]:
    """Read the files the worker wrote into its container workspace.

    The copaw/openclaw runtime writes task output into its local workspace
    (verified: greet.py landed at .../workspaces/default/greet.py) and only
    mirrors to MinIO on a later flush. Harvesting the workspace directly closes
    the loop synchronously. `baseline` is the set of paths that already existed
    before the task (agent scaffolding: AGENTS.md, agent.json, skills, etc.) so
    we return only NEW files the task produced."""
    container = WORKER_CONTAINER.format(worker=WORKER_NAME)
    workspace = WORKER_WORKSPACE.format(worker=WORKER_NAME)
    listing = subprocess.run(
        ["docker", "exec", container, "sh", "-c",
         f"find {workspace} -type f -not -path '*/.*' 2>/dev/null"],
        capture_output=True, text=True, timeout=60,
    )
    if listing.returncode != 0:
        return {}
    files: dict[str, str] = {}
    # Agent scaffolding we never want in a build manifest.
    scaffold = {
        "AGENTS.md", "SOUL.md", "agent.json", "chats.json", "jobs.json",
        "skill.json", "MEMORY.md", "memory.md",
    }
    for abspath in listing.stdout.splitlines():
        abspath = abspath.strip()
        if not abspath:
            continue
        rel = abspath[len(workspace):].lstrip("/")
        if rel in baseline or rel.split("/")[-1] in scaffold:
            continue
        if any(seg in _SKIP_DIRS for seg in rel.split("/")):
            continue
        cat = subprocess.run(
            ["docker", "exec", container, "cat", abspath],
            capture_output=True, timeout=60,
        )
        if cat.returncode != 0 or len(cat.stdout) > _MAX_FILE_BYTES:
            continue
        try:
            files[rel] = cat.stdout.decode("utf-8")
        except UnicodeDecodeError:
            continue
    return files


def _workspace_baseline() -> set[str]:
    """Snapshot the worker workspace's existing files BEFORE the task, so the
    post-task harvest can subtract scaffolding and return only new output."""
    container = WORKER_CONTAINER.format(worker=WORKER_NAME)
    workspace = WORKER_WORKSPACE.format(worker=WORKER_NAME)
    out = subprocess.run(
        ["docker", "exec", container, "sh", "-c",
         f"find {workspace} -type f -not -path '*/.*' 2>/dev/null"],
        capture_output=True, text=True, timeout=60,
    )
    if out.returncode != 0:
        return set()
    return {
        line.strip()[len(workspace):].lstrip("/")
        for line in out.stdout.splitlines()
        if line.strip()
    }


# Worker reply often contains an inline file manifest. We parse two shapes:
#   (a) fenced code blocks ```path\n<contents>``` (Hermes/OpenClaw print style)
#   (b) copaw tool-call echo: 🔧 **write_file**\n```\n{"file_path":..,"content":..}\n```
_FENCE_RE = re.compile(
    r"```(?:[a-zA-Z0-9_+.-]*\s+)?(?P<path>[^\n`]+?)\n(?P<body>.*?)```",
    re.DOTALL,
)
_WRITE_FILE_RE = re.compile(
    r'write_file\W+```[a-z]*\s*(?P<json>\{.*?"file_path".*?\})\s*```',
    re.DOTALL | re.IGNORECASE,
)


def _harvest_reply(reply: str) -> dict[str, str]:
    files: dict[str, str] = {}
    # (b) copaw write_file tool-call echoes — most reliable for that runtime.
    for m in _WRITE_FILE_RE.finditer(reply):
        try:
            obj = json.loads(m.group("json"))
        except json.JSONDecodeError:
            continue
        path = str(obj.get("file_path", "")).strip().lstrip("./")
        content = obj.get("content", "")
        if path and isinstance(content, str):
            files[path] = content
    # (a) generic path-headed fences.
    for m in _FENCE_RE.finditer(reply):
        path = m.group("path").strip()
        if path.lower().startswith("write_file"):
            continue  # already handled by (b)
        if "/" in path or re.search(r"\.[A-Za-z0-9]{1,8}$", path):
            files.setdefault(path.lstrip("./"), m.group("body"))
    return files


# =========================================================================
# Orchestration — one OpenFab dispatch end to end
# =========================================================================
def _build_intent_message(req: dict) -> str:
    lines = [f"@{WORKER_NAME} TASK: {req.get('intent', '')}"]
    language = req.get("language") or ""
    target_dir = req.get("target_dir") or "."
    acceptance = req.get("acceptance") or []
    if language:
        lines.append(f"Primary language: {language}")
    if target_dir not in (".", ""):
        lines.append(f"Lay the files out under: {target_dir}/")
    if acceptance:
        lines.append("Acceptance checks (make these shell commands pass):")
        lines.extend(f"  - {c}" for c in acceptance)
    lines.append(
        "Write the complete files to your shared workspace (MinIO) and, in your "
        "reply, list each file you created as a fenced block headed by its path."
    )
    return "\n".join(lines)


def run_dispatch(req: dict) -> dict:
    """Drive the REAL HiClaw stack for one OpenFab task and return {files,notes}."""
    # 1) control plane: ensure Manager + Worker, get the worker's room.
    token = _controller_token()
    _ensure_manager(token)
    _ensure_worker(token)
    room_id, worker_user = _wait_worker_room(token)

    # Let the worker's Matrix sync loop pass its initial catch-up window before
    # we post — messages delivered during catch-up are suppressed (R14: a
    # dropped task is a real failure, so we avoid the race rather than hide it).
    time.sleep(WORKER_SETTLE_S)
    baseline = _workspace_baseline()

    # 2) data plane: log in, join the room, deliver the intent, await the reply.
    access = _matrix_login()
    _matrix_join(access, room_id)
    since = _matrix_sync_since(access)
    _matrix_send(access, room_id, _build_intent_message(req), worker_user)
    reply = _matrix_wait_reply(access, room_id, since, worker_user)

    # 3) harvest, in order of authority:
    #    (a) the worker's container workspace (where it writes first),
    #    (b) MinIO (after the worker mirrors output back),
    #    (c) the worker's chat reply (tool-call echo / fenced blocks).
    files = _harvest_worker_workspace(baseline)
    source = "worker container workspace"
    if not files:
        files = _harvest_minio(
            [f"agents/{WORKER_NAME}/", "shared/tasks/", "shared/"]
        )
        source = "MinIO"
    if not files:
        files = _harvest_reply(reply)
        source = "worker reply (tool-call / fenced blocks)"

    if not files:
        # Vacuous-success guard (R14): no files == real failure.
        raise DispatchError(
            500,
            "HiClaw worker produced no files (workspace, MinIO, and reply all "
            f"empty). Worker '{WORKER_NAME}' reply was: {reply[:600]!r}",
        )

    notes = (
        f"HiClaw {WORKER_RUNTIME} worker '{WORKER_NAME}' (Matrix room {room_id}, "
        f"mxid {worker_user or 'n/a'}) executed the task via the real "
        f"Manager/Worker Matrix coordination; {len(files)} file(s) harvested "
        f"from {source}. Worker reply head: {reply.strip()[:300]}"
    )
    return {"files": files, "notes": notes}


# =========================================================================
# HTTP surface (OpenFab native dispatch contract)
# =========================================================================
class Handler(BaseHTTPRequestHandler):
    def _send(self, status: int, obj: dict) -> None:
        raw = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:  # noqa: N802 (stdlib naming)
        if self.path == "/health":
            self._send(
                200,
                {
                    "status": "ok",
                    "base": "hiclaw",
                    "worker": WORKER_NAME,
                    "runtime": WORKER_RUNTIME,
                    "controller": _controller_base(),
                    "matrix": MATRIX_BASE,
                },
            )
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self) -> None:  # noqa: N802
        if self.path not in ("/dispatch", "/"):
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        try:
            req = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError as exc:
            self._send(400, {"error": f"invalid JSON: {exc}"})
            return
        try:
            self._send(200, run_dispatch(req))
        except DispatchError as exc:
            # Surface, don't swallow (R5). Non-2xx => OpenFab records a failed
            # native run instead of accepting an empty manifest.
            self._send(exc.status, {"error": exc.message})
        except Exception as exc:  # last-resort guard, still surfaced
            self._send(500, {"error": f"hiclaw native dispatch failed: {exc}"})

    def log_message(self, *_args) -> None:  # quieter logs
        return


def main() -> None:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(
        f"[openfab-hiclaw] native base on http://{HOST}:{PORT}/dispatch "
        f"-> controller {_controller_base()} | matrix {MATRIX_BASE} | "
        f"worker {WORKER_NAME}/{WORKER_RUNTIME}",
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
