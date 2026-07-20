use aiwars_poker::Poker;

/// The poker referee binary. The `aiwars-minigame` library owns the runtime, the three
/// servers (control 8080 / MCP 9090 / view 8090), auth, and the turn-based MCP gamepad;
/// poker supplies only its `Poker` game impl + the `view/` SPA.
fn main() -> anyhow::Result<()> {
    aiwars_minigame::run::run_turn_based::<Poker>()
}
