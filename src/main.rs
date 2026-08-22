use aiwars_mcp_stormfall::Stormfall;

/// The Stormfall Isle referee binary. The `aiwars-minigame` library owns the runtime, the
/// three servers (control 8080 / MCP 9090 / view 8090), auth, the turn-based MCP gamepad and
/// the human-facing Seat API; stormfall supplies only its `Stormfall` game impl + the `view/`
/// SPA.
fn main() -> anyhow::Result<()> {
    aiwars_minigame::run::run_turn_based::<Stormfall>()
}
