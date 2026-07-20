//! The classic game of Nim — a tier-1 turn-based, PERFECT-information AIWars minigame.
//!
//! Exactly two agents share ONE pile of 15 stones and alternate removing 1, 2, or 3 stones.
//! Whoever takes the LAST stone WINS — the normal-play convention, not misère. The match is
//! guaranteed to end within 15 plies (every move removes at least one stone from a pile of 15).
//!
//! Moves are opaque strings — `"take:1"`, `"take:2"`, `"take:3"`.
//! [`legal_moves`](TurnBasedGame::legal_moves) offers only the amounts that fit the pile (so at
//! two stones it lists `take:1`/`take:2` and never `take:3`); `apply` validates the same bound
//! and every rejected move leaves the game completely unchanged.
//!
//! Nim hides NOTHING — [`observe`](Minigame::observe) is the same authoritative state for the
//! spectator and either player: the pile count, whose turn it is, the move log, the winner. It is
//! also fully DETERMINISTIC: no deck, no dice, no seed. `new` ignores `settings` entirely.

use aiwars_minigame::{AgentId, MatchError, Minigame, Outcome, TurnBasedGame};
use serde_json::{json, Value};

const PLAYERS: usize = 2;
const START_PILE: u32 = 15;
const MAX_TAKE: u32 = 3;
const LOG_CAP: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Playing,
    Done,
}

/// A game of Nim: one pile, two seats, alternating turns.
pub struct Nim {
    players: Vec<AgentId>, // seat-indexed, length 2
    pile: u32,             // stones remaining
    to_act: usize,
    ply: u32,
    phase: Phase,
    result: Option<usize>, // the winning seat once resolved (nim never draws in normal play)
    log: Vec<String>,
}

impl Nim {
    fn name_of(&self, seat: usize) -> &str {
        &self.players[seat].0
    }

    fn seat_of(&self, agent: &AgentId) -> Option<usize> {
        self.players.iter().position(|p| p == agent)
    }

    fn push_log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
        let n = self.log.len();
        if n > LOG_CAP {
            self.log.drain(0..n - LOG_CAP);
        }
    }

    /// The largest legal take right now — the pile, capped at three.
    fn max_take(&self) -> u32 {
        self.pile.min(MAX_TAKE)
    }

    // ----- move validation (pure — never mutates) -----------------------------------------

    /// Parse + validate `mv` for the seat on the clock, WITHOUT touching any state. Every
    /// illegal move returns `Err` here, which is what makes a rejected `apply` perfectly inert.
    fn parse_take(&self, mv: &str) -> Result<u32, MatchError> {
        let Some(rest) = mv.strip_prefix("take:") else {
            return Err(MatchError::Rejected(format!(
                "unknown move '{mv}' — expected take:1, take:2, or take:3"
            )));
        };
        let n: u32 = rest
            .parse()
            .map_err(|_| MatchError::Rejected(format!("malformed move '{mv}' — use take:<1-3>")))?;
        if !(1..=MAX_TAKE).contains(&n) {
            return Err(MatchError::Rejected(format!(
                "take {n} is out of range — take 1, 2, or 3 stones"
            )));
        }
        if n > self.pile {
            return Err(MatchError::Rejected(format!(
                "take {n} exceeds the {} stone(s) left — take at most {}",
                self.pile,
                self.max_take()
            )));
        }
        Ok(n)
    }
}

impl Minigame for Nim {
    fn new(agents: &[AgentId], _settings: &Value) -> Result<Self, MatchError> {
        if agents.len() != PLAYERS {
            return Err(MatchError::WrongPlayerCount {
                want: 2..=2,
                got: agents.len(),
            });
        }
        // Nim is DETERMINISTIC: one fixed pile, no shuffle, no dice — so `settings` is ignored
        // (there is no seed to honour). Seat 0 always opens; both sides see the whole pile.
        let mut g = Nim {
            players: agents.to_vec(),
            pile: START_PILE,
            to_act: 0,
            ply: 0,
            phase: Phase::Playing,
            result: None,
            log: Vec::new(),
        };
        g.push_log(format!(
            "── Nim — {} stones. Take 1-3 each turn; take the last stone to win. {} moves first.",
            START_PILE,
            g.name_of(0)
        ));
        Ok(g)
    }

    fn name(&self) -> &'static str {
        "nim"
    }

    fn instructions(&self) -> String {
        "AIWars Nim. Two players share ONE pile of 15 stones and take turns removing stones. \
         On your turn you must take 1, 2, or 3 stones; whoever takes the LAST stone WINS (normal \
         play, not misère). Call get_state each turn and read `pile` (stones left), `to_act` \
         (whose turn), and `moves` (your EXACT legal moves this turn — only the amounts that fit \
         the pile). Play with make_move, mv = one of: \"take:1\", \"take:2\", \"take:3\". Pass \
         expected_ply = the ply you saw. It is perfect information: nothing is hidden."
            .into()
    }

    fn observe(&self, viewer: Option<&AgentId>) -> Value {
        let me = viewer.and_then(|a| self.seat_of(a));
        let over = self.phase == Phase::Done;

        let players: Vec<Value> = (0..PLAYERS)
            .map(|s| {
                json!({
                    "handle": self.players[s].0,
                    "turn": !over && self.to_act == s,
                })
            })
            .collect();

        let to_act = if over {
            Value::Null
        } else {
            Value::String(self.players[self.to_act].0.clone())
        };
        let winner = match self.result {
            Some(s) => Value::String(self.players[s].0.clone()),
            None => Value::Null,
        };

        let mut v = json!({
            "game": "nim",
            "pile": self.pile,
            "start_pile": START_PILE,
            "max_take": MAX_TAKE,
            "to_act": to_act,
            "ply": self.ply,
            "players": players,
            "moves": if over { Vec::new() } else { self.legal_moves() },
            "log": self.log,
            "status": if over { "over" } else { "playing" },
            "winner": winner,
        });

        // Per-agent view: nim is perfect information, so a player's projection adds nothing to
        // the public state but a convenience turn hint (there are no hidden cards to reveal).
        if let Some(me) = me {
            let obj = v.as_object_mut().unwrap();
            obj.insert("hero".into(), json!(me));
            obj.insert("your_turn".into(), json!(!over && self.to_act == me));
        }
        v
    }

    fn outcome(&self) -> Option<Outcome> {
        self.result.map(|s| Outcome::Win(self.players[s].clone()))
    }

    /// A wall-clock timeout DRAWS. Nim has no material lead — both sides always control the same
    /// (nothing) — so `None` (draw) is the neutral verdict. We deliberately do NOT encode the
    /// perfect-play rule (the player to move loses iff `pile % 4 == 0`) here: that would settle a
    /// timeout on the POSITION, punishing a stalling agent for the board rather than for stalling.
    fn timeout_leader(&self) -> Option<AgentId> {
        None
    }
}

impl TurnBasedGame for Nim {
    fn turn_agent(&self) -> AgentId {
        self.players[self.to_act].clone()
    }

    fn ply(&self) -> u32 {
        self.ply
    }

    fn legal_moves(&self) -> Vec<String> {
        if self.phase == Phase::Done {
            return Vec::new();
        }
        // Only the amounts that fit the pile: at two stones this is take:1/take:2, never take:3.
        (1..=self.max_take()).map(|n| format!("take:{n}")).collect()
    }

    fn apply(&mut self, agent: &AgentId, mv: &str) -> Result<(), MatchError> {
        if self.phase == Phase::Done {
            return Err(MatchError::GameOver);
        }
        let me = self
            .seat_of(agent)
            .ok_or_else(|| MatchError::Rejected("not a player".into()))?;
        // Defensive: the match wrapper already checks turn ownership + ply, but keep the game
        // honest if driven directly.
        if self.to_act != me {
            return Err(MatchError::Rejected("not your turn".into()));
        }
        // Validate WITHOUT mutating — a rejected move leaves the game completely unchanged.
        let n = self.parse_take(mv)?;

        // --- committed, mutating path (validation has passed) ---
        self.ply += 1;
        self.pile -= n;
        self.push_log(format!(
            "{} takes {} stone{} — {} left.",
            self.name_of(me),
            n,
            if n == 1 { "" } else { "s" },
            self.pile
        ));
        if self.pile == 0 {
            // The taker of the LAST stone wins (normal play). The match ends here.
            self.result = Some(me);
            self.phase = Phase::Done;
            self.push_log(format!(
                "{} took the last stone and wins!",
                self.name_of(me)
            ));
        } else {
            self.to_act = 1 - me;
        }
        Ok(())
    }

    fn resign(&mut self, agent: &AgentId) {
        // A forfeit loses the whole MATCH (like a chess resign, and like the platform's
        // forfeit-on-fuel-exhaustion): the opponent is awarded the win. No-op once resolved.
        if self.phase == Phase::Done {
            return;
        }
        if let Some(s) = self.seat_of(agent) {
            let w = 1 - s;
            self.push_log(format!(
                "{} forfeits — {} wins the match.",
                self.name_of(s),
                self.name_of(w)
            ));
            self.result = Some(w);
            self.phase = Phase::Done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwars_minigame::{RefereeMatch, TurnBasedMatch};

    fn two() -> Vec<AgentId> {
        vec![AgentId("alice".into()), AgentId("bob".into())]
    }

    /// A fresh game, pile 15, seat 0 to move.
    fn game() -> Nim {
        Nim::new(&two(), &json!({})).unwrap()
    }

    /// A full snapshot of every mutable field — for the illegal-move inertness assertion.
    fn snapshot(g: &Nim) -> String {
        format!(
            "pile={} to_act={} ply={} phase={:?} result={:?} log={:?}",
            g.pile, g.to_act, g.ply, g.phase, g.result, g.log
        )
    }

    #[test]
    fn rejects_bad_player_count() {
        assert!(matches!(
            Nim::new(&[AgentId("solo".into())], &json!({})),
            Err(MatchError::WrongPlayerCount { got: 1, .. })
        ));
    }

    /// Nim takes no seed: construction ignores `settings` and is fully deterministic — an empty
    /// object, an explicit seed, and JSON null all produce the identical opening state.
    #[test]
    fn new_needs_no_seed_and_is_deterministic() {
        let a = Nim::new(&two(), &json!({})).unwrap();
        let b = Nim::new(&two(), &Value::Null).unwrap();
        let c = Nim::new(&two(), &json!({ "seed": 7 })).unwrap();
        assert_eq!(a.pile, START_PILE);
        assert_eq!(a.to_act, 0, "seat 0 always opens");
        assert_eq!(snapshot(&a), snapshot(&b));
        assert_eq!(snapshot(&a), snapshot(&c), "a seed changes nothing");
    }

    #[test]
    fn opening_offers_take_one_two_three() {
        let g = game();
        assert_eq!(g.pile, 15);
        assert_eq!(
            g.legal_moves(),
            vec![
                "take:1".to_string(),
                "take:2".to_string(),
                "take:3".to_string()
            ]
        );
    }

    /// Near the bottom the menu shrinks to only the amounts that fit the pile.
    #[test]
    fn legal_moves_are_capped_by_the_pile() {
        let mut g = game();
        g.pile = 2;
        assert_eq!(
            g.legal_moves(),
            vec!["take:1".to_string(), "take:2".to_string()],
            "two stones ⇒ never offer take:3"
        );
        g.pile = 1;
        assert_eq!(g.legal_moves(), vec!["take:1".to_string()]);
    }

    /// Every illegal move is rejected AND leaves the game byte-for-byte unchanged
    /// (validate-before-mutate). Includes the out-of-range bounds and an overdraw.
    #[test]
    fn illegal_moves_are_inert() {
        let mut g = game();
        g.pile = 2; // so take:3 is an overdraw (in range, but more than the pile holds)
        let actor = g.players[g.to_act].clone();
        let wrong = g.players[1 - g.to_act].clone();
        let before = snapshot(&g);
        for (who, mv) in [
            (&actor, "take:0"),   // below the range
            (&actor, "take:4"),   // above the range
            (&actor, "take:3"),   // overdraw — only 2 left
            (&actor, "take:-1"),  // malformed (not a u32)
            (&actor, "take:abc"), // malformed
            (&actor, "grab:1"),   // unknown verb
            (&actor, "teleport"), // unknown
            (&wrong, "take:1"),   // not your turn
        ] {
            assert!(g.apply(who, mv).is_err(), "'{mv}' should be rejected");
            assert_eq!(snapshot(&g), before, "'{mv}' must not mutate state");
        }
    }

    /// A scripted full game: both sides take one stone at a time, so seat 0 removes stones
    /// 1, 3, …, 15 — the last one — and wins. Once over the game is inert.
    #[test]
    fn last_stone_wins_scripted() {
        let mut g = game();
        let a = g.players[0].clone();
        let b = g.players[1].clone();
        for i in 0..15u32 {
            assert_eq!(g.phase, Phase::Playing);
            let who = if i % 2 == 0 { &a } else { &b };
            g.apply(who, "take:1").unwrap();
        }
        assert_eq!(g.pile, 0);
        assert_eq!(g.phase, Phase::Done);
        assert_eq!(g.outcome(), Some(Outcome::Win(a.clone())));
        assert_eq!(g.observe(None)["winner"], json!("alice"));
        // Over ⇒ no moves, and further moves are refused with GameOver.
        assert!(g.legal_moves().is_empty());
        assert_eq!(g.apply(&b, "take:1"), Err(MatchError::GameOver));
    }

    /// The other seat can also take the last stone: a scripted line summing to 15 in six moves,
    /// so seat 1 clears the pile and wins.
    #[test]
    fn either_seat_can_win() {
        let mut g = game();
        let a = g.players[0].clone();
        let b = g.players[1].clone();
        // 3 + 3 + 1 + 2 + 3 + 3 = 15, alternating a,b,a,b,a,b → seat 1 (bob) takes the last.
        for (who, mv) in [
            (&a, "take:3"),
            (&b, "take:3"),
            (&a, "take:1"),
            (&b, "take:2"),
            (&a, "take:3"),
            (&b, "take:3"),
        ] {
            g.apply(who, mv).unwrap();
        }
        assert_eq!(g.pile, 0);
        assert_eq!(g.outcome(), Some(Outcome::Win(b.clone())));
    }

    #[test]
    fn observe_is_full_information() {
        let g = game();
        let public = g.observe(None);
        assert_eq!(public["game"], "nim");
        assert_eq!(public["pile"], 15);
        assert_eq!(public["status"], "playing");
        assert_eq!(public["to_act"], json!("alice"));
        assert_eq!(public["moves"].as_array().unwrap().len(), 3);
        let handles: Vec<&str> = public["players"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap())
            .collect();
        assert_eq!(handles, vec!["alice", "bob"]);
        // A player's private projection carries the same pile (nothing hidden) + a turn hint.
        let a_view = g.observe(Some(&g.players[0]));
        assert_eq!(a_view["pile"], 15);
        assert_eq!(a_view["your_turn"], json!(true));
        assert_eq!(
            g.observe(Some(&g.players[1]))["your_turn"],
            json!(false),
            "it is not seat 1's turn at the open"
        );
    }

    #[test]
    fn timeout_is_a_neutral_draw() {
        // No material lead in nim ⇒ a wall-clock timeout draws (leader is None).
        assert_eq!(game().timeout_leader(), None);
    }

    #[test]
    fn instructions_teach_the_moves_and_the_rule() {
        let s = game().instructions();
        assert!(s.contains("take:1") && s.contains("take:3"));
        assert!(s.to_lowercase().contains("last stone"));
    }

    #[test]
    fn resign_awards_the_opponent_the_match() {
        let mut m = TurnBasedMatch::new::<Nim>(two(), &json!({})).unwrap();
        m.start();
        let st = m.state_json();
        // Seat 0 = alice, seat 1 = bob (construction order).
        m.resign(0);
        assert!(m.is_resolved());
        let r = m.result().unwrap();
        assert_eq!(r.outcome, "Winner");
        assert_eq!(r.winner.as_deref(), Some("bob"));
        // The pre-resign public state named the game.
        assert_eq!(st["game"], "nim");
    }

    /// Drive a whole match via the turn-based wrapper — always take the first legal move — and
    /// assert it resolves to a winner within the 15-ply cap (nim never draws in normal play).
    #[test]
    fn a_full_match_resolves() {
        let mut m = TurnBasedMatch::new::<Nim>(two(), &json!({})).unwrap();
        m.start();
        let order: Vec<String> = two().iter().map(|a| a.0.clone()).collect();
        for _ in 0..100 {
            if m.is_resolved() {
                break;
            }
            let st = m.state_json();
            let actor = st["to_act"].as_str().unwrap().to_string();
            let seat = order.iter().position(|h| *h == actor).unwrap();
            let ply = st["ply"].as_u64().unwrap() as u32;
            let moves = m.turn_info(seat)["moves"].as_array().unwrap().clone();
            let mv = moves.first().and_then(|m| m.as_str()).unwrap().to_string();
            m.make_move(seat, &mv, ply).unwrap();
        }
        assert!(m.is_resolved(), "nim must resolve within the ply cap");
        let r = m.result().unwrap();
        assert_eq!(r.outcome, "Winner", "nim never draws in normal play");
        assert!(
            m.state_json()["ply"].as_u64().unwrap() <= START_PILE as u64,
            "a game of 15 stones lasts at most 15 plies"
        );
    }

    /// The shipped game.toml must parse and its hold must validate — green CI implies a bootable
    /// manifest (a typo in `[settings]` would otherwise crashloop every pod).
    #[test]
    fn game_toml_is_loadable() {
        let settings = aiwars_minigame::settings::manifest_settings_at("game.toml").unwrap();
        aiwars_minigame::settings::validate_hold(&settings).unwrap();
    }
}
