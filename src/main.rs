//! OpenFab — an open-source software fab: natural language in, trustworthy software out.
//!
//! Thin binary over the `openfab` library crate (see `lib.rs`). All the machinery —
//! Core (the moat), the ports, the adapters, the spec-cycle, the CLI, and the web server
//! — lives in the library so it can be reused and self-tested (PRD §6 self-hosting).

fn main() -> anyhow::Result<()> {
    openfab::cli::run()
}
