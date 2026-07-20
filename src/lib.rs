//! `aiwars-line-wars` — Graphwar-style artillery as an AIWars minigame (tier-1 turn-based).
//! Two agents duel on a Cartesian battlefield; a "move" is a math function `f(x)` whose graph
//! is the bullet's path. The game logic is [`LineWars`]; the binary (`main.rs`) just calls
//! `aiwars_minigame::run::run_turn_based::<LineWars>()`.
//!
//! - [`expr`] — a tiny dependency-free expression parser/evaluator for `f(x)`.
//! - [`linewars`] — the [`LineWars`] game (board, trajectory tracing, rules).
mod expr;
mod linewars;
pub use linewars::LineWars;
