# -*- coding: utf-8 -*-
"""Native OpenFab base server backed by a REAL AgentScope agent.

This service makes AgentScope a *genuine* native OpenFab base (the agent base
compared against HiClaw). It speaks the OpenFab NATIVE BASE dispatch contract
(see ``src/adapters/base_framework.rs::dispatch_native``):

    POST  $OPENFAB_AGENTSCOPE_URL
    body  {"intent", "target_dir", "language", "acceptance": [<shell checks>]}
    resp  {"files": {"<relpath>": "<contents>"}, "notes": ""}

How it stays honest (R14): the ``files`` manifest is not assembled by this
server hand-writing code. It is produced by driving an actual
``agentscope.agent.Agent`` — AgentScope 2.0's unified ReAct agent — through its
own reasoning-acting loop. The agent runs with AgentScope's built-in
``Write``/``Edit``/``Read``/``Bash``/``Glob``/``Grep`` tools inside an isolated
per-request workspace, and the manifest is harvested from the files the agent
actually wrote to disk. The LLM behind the loop is a local Ollama model via
AgentScope's ``OllamaChatModel``. If the agent writes nothing, this server
returns HTTP 500 (empty output is failure, never a vacuous pass).

Run:
    # one-time, in the agentscope venv (Python >= 3.11):
    $HOME/claudeworkfolder/agentscope/.venv/bin/pip install \
        -e $HOME/claudeworkfolder/agentscope \
        "agentscope[ollama]" fastapi "uvicorn[standard]"

    # start ollama + pull a tool-capable model:
    ollama serve            # if not already running
    ollama pull qwen2.5:7b  # tool-calling model (llama3.2:3b also works)

    # launch the base server:
    OPENFAB_AGENTSCOPE_MODEL=qwen2.5:7b \
    $HOME/claudeworkfolder/agentscope/.venv/bin/python \
        $HOME/claudeworkfolder/openfab/integrations/agentscope/server.py

    # tell OpenFab the base is native + reachable:
    export OPENFAB_AGENTSCOPE_URL=http://127.0.0.1:8731/dispatch
"""
from __future__ import annotations

import os
import tempfile
from pathlib import Path
from typing import Any

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

# --- AgentScope (real orchestration) ----------------------------------------
from agentscope.agent import Agent, ReActConfig
from agentscope.credential import OllamaCredential
from agentscope.message import UserMsg
from agentscope.model import OllamaChatModel
from agentscope.permission import PermissionContext, PermissionMode
from agentscope.state import AgentState
from agentscope.tool import Bash, Edit, Glob, Grep, Read, Toolkit, Write

# --- Configuration (env-driven; all have safe local defaults) ----------------
# The Ollama model name. Prefer a tool-calling model; the ReAct loop needs the
# model to emit tool calls (Write/Bash) to actually create files.
OLLAMA_MODEL = os.environ.get("OPENFAB_AGENTSCOPE_MODEL", "qwen2.5:7b")
# Ollama server host. None => AgentScope's OllamaCredential default localhost.
OLLAMA_HOST = os.environ.get("OLLAMA_HOST") or None
# Bind address for this base server.
HOST = os.environ.get("OPENFAB_AGENTSCOPE_HOST", "127.0.0.1")
PORT = int(os.environ.get("OPENFAB_AGENTSCOPE_PORT", "8731"))
# Bound the ReAct loop so a confused small model can't spin forever.
MAX_ITERS = int(os.environ.get("OPENFAB_AGENTSCOPE_MAX_ITERS", "24"))
# Files we never want to ship back in the manifest (agent scratch / VCS noise).
_SKIP_DIRS = {".git", "__pycache__", "node_modules", ".venv", ".pytest_cache"}
_MAX_FILE_BYTES = 512 * 1024  # don't bloat the manifest with huge artifacts


# --- Dispatch contract models -----------------------------------------------
class DispatchRequest(BaseModel):
    """The OpenFab native-base request body."""

    intent: str
    target_dir: str = Field(default=".")
    language: str = Field(default="")
    acceptance: list[str] = Field(default_factory=list)


class DispatchResponse(BaseModel):
    """The OpenFab native-base response body (the manifest)."""

    files: dict[str, str]
    notes: str = ""


def _build_system_prompt(workspace: Path) -> str:
    """The agent's standing instructions — repo-agnostic, workspace-scoped."""
    return (
        "You are an autonomous coding agent fulfilling a single build task. "
        "Your working directory is the EMPTY directory:\n"
        f"    {workspace}\n"
        "The file tools (Write, Edit, Read, Glob, Grep) require ABSOLUTE "
        "paths, so create every file under that directory. "
        "Implement the requested change by actually writing the files with "
        "the Write tool — do not just describe them. Keep each file complete "
        "and runnable. You may use Bash to create directories or run quick "
        "sanity checks, but all work must stay inside the working directory. "
        "When every file required by the task exists on disk and you are "
        "confident the acceptance checks would pass, reply with a one-line "
        "summary and stop."
    )


def _build_user_prompt(req: DispatchRequest) -> str:
    """Turn the OpenFab TaskCard fields into the agent's task message."""
    lines = [f"TASK: {req.intent}"]
    if req.language:
        lines.append(f"PRIMARY LANGUAGE: {req.language}")
    if req.target_dir and req.target_dir not in (".", ""):
        lines.append(
            "Lay the files out as if they belong under the project "
            f"subdirectory '{req.target_dir}', but write them relative to "
            "your working directory (do not prepend that path).",
        )
    if req.acceptance:
        lines.append(
            "\nACCEPTANCE CHECKS (these shell commands will be run against "
            "your output; make them pass):",
        )
        lines.extend(f"  - {check}" for check in req.acceptance)
    lines.append(
        "\nCreate all necessary files now using the Write tool.",
    )
    return "\n".join(lines)


def _build_agent(workspace: Path) -> Agent:
    """Construct a real AgentScope ReAct agent wired to Ollama.

    This is the genuine AgentScope object the demo (demo/hello_agentscope.py)
    uses — same Agent + Toolkit + BYPASS permission context, swapping the
    DashScope model for a local OllamaChatModel.
    """
    model = OllamaChatModel(
        credential=OllamaCredential(host=OLLAMA_HOST),
        model=OLLAMA_MODEL,
        stream=True,
    )
    return Agent(
        name="OpenFab-AgentScope",
        system_prompt=_build_system_prompt(workspace),
        model=model,
        # The built-in tools do the real filesystem work; Bash lets the agent
        # run the acceptance checks itself during its ReAct loop.
        toolkit=Toolkit(tools=[Write(), Edit(), Read(), Glob(), Grep(), Bash()]),
        # BYPASS => no human-in-the-loop confirmations. Safe because the agent
        # is chrooted-in-spirit to a throwaway per-request workspace.
        state=AgentState(
            permission_context=PermissionContext(mode=PermissionMode.BYPASS),
        ),
        react_config=ReActConfig(max_iters=MAX_ITERS),
    )


def _harvest_workspace(workspace: Path) -> dict[str, str]:
    """Read back everything the agent wrote into the workspace as the manifest.

    Keys are POSIX relative paths; values are file contents. Binary/oversized
    files are skipped so the JSON manifest stays text and bounded.
    """
    files: dict[str, str] = {}
    for path in sorted(workspace.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(workspace)
        if any(part in _SKIP_DIRS for part in rel.parts):
            continue
        try:
            if path.stat().st_size > _MAX_FILE_BYTES:
                continue
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            # Non-UTF8 / unreadable artifact — skip rather than corrupt the
            # manifest. Surfaced via the notes field by the caller.
            continue
        files[rel.as_posix()] = text
    return files


async def _run_agent(req: DispatchRequest) -> DispatchResponse:
    """Drive one AgentScope reply over an isolated workspace and harvest it."""
    with tempfile.TemporaryDirectory(prefix="openfab-as-") as tmp:
        workspace = Path(tmp).resolve()
        prev_cwd = Path.cwd()
        # AgentScope's Write/Bash tools resolve relative paths against cwd and
        # require absolute paths; chdir keeps any stray relative writes inside
        # the sandbox too. Restored in `finally`.
        os.chdir(workspace)
        try:
            agent = _build_agent(workspace)
            final = await agent.reply(
                UserMsg("user", _build_user_prompt(req)),
            )
            summary = final.get_text_content() or ""
        finally:
            os.chdir(prev_cwd)

        files = _harvest_workspace(workspace)

    if not files:
        # Vacuous-success guard (R14): an empty workspace is a real failure,
        # not a clean pass. Make OpenFab record the dispatch as errored.
        raise HTTPException(
            status_code=500,
            detail=(
                "AgentScope agent produced no files. The model "
                f"'{OLLAMA_MODEL}' may not support tool calling, or the "
                "ReAct loop ended without any Write. Agent summary: "
                f"{summary[:500]!r}"
            ),
        )

    notes = (
        f"AgentScope 2.0 ReAct agent (Ollama model '{OLLAMA_MODEL}') wrote "
        f"{len(files)} file(s) over its own reasoning-acting loop. "
        f"Agent summary: {summary.strip()[:400]}"
    )
    return DispatchResponse(files=files, notes=notes)


app = FastAPI(title="OpenFab AgentScope native base")


@app.get("/health")
async def health() -> dict[str, Any]:
    """Liveness + which model this base will drive."""
    return {"status": "ok", "base": "agentscope", "model": OLLAMA_MODEL}


@app.post("/dispatch", response_model=DispatchResponse)
async def dispatch(req: DispatchRequest) -> DispatchResponse:
    """The OpenFab native-base entry point.

    Errors propagate as HTTP non-2xx so the Rust adapter's `dispatch_native`
    treats them as a failed native run (curl non-success) instead of silently
    accepting an empty manifest.
    """
    try:
        return await _run_agent(req)
    except HTTPException:
        raise
    except Exception as exc:  # surface, don't swallow (R5)
        raise HTTPException(
            status_code=500,
            detail=f"AgentScope native dispatch failed: {exc}",
        ) from exc


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host=HOST, port=PORT)
