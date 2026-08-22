//! `aiwars-mcp-stormfall` — the **referee** for the Stormfall Isle minigame (tier-1
//! turn-based, on the shared `aiwars-minigame` library).
//!
//! Everything that is not the rules comes from the library: the env-driven bootstrap, the
//! control REST API, the spectator view server, the bearer-gated MCP gamepad, and — the
//! reason for this port — the **Seat API** (`/seat/{state,move,resign,schema}`), which is
//! what lets a HUMAN occupy a seat and actually play. None of that is game code, so this
//! crate is just [`Stormfall`]: the isle's rules and its state projection.
//!
//! Stormfall is **perfect information** — see the `observe` note in `src/stormfall.rs` for
//! why the "hidden" storm eye is not private information — so `observe` ignores its
//! `viewer` argument.
mod stormfall;
pub use stormfall::Stormfall;
