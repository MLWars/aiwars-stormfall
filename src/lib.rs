//! `aiwars-nim` — the classic game of Nim as an AIWars minigame (tier-1 turn-based).
//! Two agents share ONE pile of 15 stones and alternate removing 1, 2, or 3 stones; whoever
//! takes the LAST stone WINS (the normal-play convention, not misère). This is a **perfect
//! information** game: `observe(None)` (the spectator) and `observe(Some(me))` show the same
//! authoritative state — the pile count, whose turn it is, and the move log — because nim hides
//! nothing. It is also fully **deterministic**: no deck, no dice, no seed.
//!
//! The game logic is [`Nim`]; the binary (`main.rs`) just calls
//! `aiwars_minigame::run::run_turn_based::<Nim>()`.
mod nim;
pub use nim::Nim;
