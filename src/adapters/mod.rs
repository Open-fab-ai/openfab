//! Adapters — concrete implementations of the ports (PRD §7 `adapters/`).
//!
//! Bases:  `base_claude` (LLM via the claude CLI) · `base_framework` (AgentScope/HiClaw/
//! agent-chat/OpenHands, LLM-backed). Every artifact comes from the LLM — no hardcoded apps.
//! Forges: `forge_local_git` (local repo) · `forge_github` (real, gated).
//! `sandbox` is shared infrastructure OpenFab supplies when a base lacks one.
//!
//! Adapters may depend on `core` and `ports`; `core` must never depend on adapters.

pub mod agent_spec;
pub mod base_claude;
pub mod base_framework;
pub mod bridge_client;
pub mod forge_github;
pub mod forge_local_git;
pub mod forge_rest;
pub mod llm_backend;
pub mod registry;
pub mod sandbox;
