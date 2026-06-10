//! OpenFab library crate.
//!
//! The same modules the binary uses, exposed so that integration tests — and, in Phase 1,
//! OpenFab's own self-development (PRD §6) — can drive the real Core API
//! (`use openfab::core::provenance::...`, etc.). The `openfab` binary is a thin shell over
//! `cli::run`.

pub mod adapters;
pub mod cli;
pub mod core;
pub mod ops;
pub mod ports;
pub mod runstate;
pub mod server;
pub mod spec_cycle;
