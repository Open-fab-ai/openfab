//! Ports — the two pluggable seams of the fab (PRD §3).
//!
//! `BasePort`  : the swappable agent runtime/collaboration base (down axis).
//! `ForgePort` : the swappable git host (across axis).
//!
//! Ports depend on `core` types (e.g. `TaskCard`) — never the reverse. Adapters in
//! `adapters/` implement these traits; Core orchestrates against the traits only, so
//! it never names a concrete base or forge (the base-independence golden rule).

pub mod base;
pub mod forge;
