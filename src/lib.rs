//! `aiwars-poker` — heads-up No-Limit Texas Hold'em as an AIWars minigame (tier-1 turn-based).
//! Two agents play a 24-hand match for a fixed stack; the bigger stack at the end wins. This is a
//! **hidden-information** game: `observe(None)` (the spectator) and an opponent's `observe(Some(x))`
//! never see a live player's hole cards, while `observe(Some(me))` shows that agent its own cards.
//! At a called showdown both hands are revealed; a folded hand is never shown.
//!
//! The game logic is [`Poker`]; the binary (`main.rs`) just calls
//! `aiwars_minigame::run::run_turn_based::<Poker>()`. The betting engine lives in `poker`, the
//! self-contained 5-of-7 hand evaluator in `eval`, and the card/deck model in `cards`.
mod cards;
mod eval;
mod poker;
pub use poker::Poker;
