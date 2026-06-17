# Multi-base comparison — one spec, three agent bases

**Claim under test:** OpenFab's Core is *base-agnostic* — the same machine-checkable spec
runs identically on any agent base, and every base produces a verifiable product.

## Setup

- **Shared contract:** [`specs/demo-temp-converter.spec.yaml`](../specs/demo-temp-converter.spec.yaml)
  — a CLI temperature converter (c2f/f2c/c2k/k2c + `--selftest`), with **6 acceptance checks**.
- **Identical everything** except the base: `--forge local --gate solo --draft`, each into an
  isolated repo, dispatched through the *same* OpenFab `ops` pipeline.
- **Bases / models:**
  - `claude` — the local `claude` CLI (native).
  - `agent-chat` — **native**, via its own LLM-client wire format → Ollama Cloud `qwen3-coder:480b`.
  - `agentscope` — **native**, a genuine AgentScope **ReAct agent** (Write/Bash tools) → Ollama Cloud `gpt-oss:120b`.

## Result

| Base | Backing model | Runtime | Acceptance | `app/convert.py` | Time |
|---|---|---|---|---|---|
| **claude** | claude CLI | native | ✅ **6 / 6** | 54 lines | 14.6 s |
| **agent-chat** | qwen3-coder:480b (cloud) | native | ✅ **6 / 6** | 85 lines | **10.8 s** |
| **agentscope** | gpt-oss:120b (cloud, ReAct) | native | ✅ **6 / 6** | 115 lines | 31.7 s |

**All three pass the identical 6 acceptance checks** — the base-agnostic Core is proven.
What differs is *style and mechanism*, not correctness:

- **agent-chat** (single-shot codegen) was the **fastest** and produced compact, direct code.
- **agentscope** (a multi-step **ReAct loop** — the agent calls Write/Bash tools itself) wrote
  the **most thorough** file (full module docstring + a richer `--selftest`), at the cost of
  more wall-clock time (multiple LLM + tool round-trips).
- **claude** sat in the middle — concise and quick.

## Why this matters

The product changes with the base; the **trust pipeline does not**. Each run — whatever the
base — flows through the same spec → generate → sandboxed acceptance → (on promote) signed
in-toto/SLSA provenance + N-of-M gate. The provenance honestly records *which* base and
runtime produced the code, so you can swap bases freely and still get portable, verifiable
software. That is the swap surface OpenFab exists to provide.

## Honesty notes (R14)

- `agent-chat` native here drives agent-chat's **LLM-client** path (pointed at the cloud
  model); it does not exercise its tmux multi-agent orchestration (a separate `orchestrate`
  mode that needs a CLI agent like codex). Badged `native`, recorded truthfully.
- `agentscope` is a **real** AgentScope ReAct agent doing genuine tool-using orchestration.
- Reproduce: `integrations/launch-cloud-demo.sh` to bring the stack up, then
  `openfab run --spec specs/demo-temp-converter.spec.yaml --repo /tmp/cmp-<base> --base <base> --forge local --gate solo --draft`.
