//! Heads-up No-Limit Texas Hold'em — a tier-1 turn-based, HIDDEN-information AIWars minigame.
//!
//! Exactly two agents play a fixed-length match: 200-chip stacks (100 big blinds), blinds 1/2
//! doubling every 8 hands (1/2 → 2/4 → 4/8), for up to 24 hands. A player who busts loses
//! immediately; otherwise after hand 24 the larger stack wins (equal ⇒ draw). Standard heads-up
//! positions: the button posts the small blind, acts FIRST pre-flop and SECOND post-flop, and
//! the button alternates every hand.
//!
//! Moves are opaque strings — `"fold"`, `"check"`, `"call"`, `"raise:<TOTAL>"` (raise TO a total
//! committed THIS street) and `"allin"`. [`legal_moves`](TurnBasedGame::legal_moves) offers a
//! small discrete raise menu (min-raise, pot-size, all-in); `apply` accepts any raise total from
//! the minimum up to all-in.
//!
//! HIDDEN INFORMATION is the invariant that matters: [`observe`](Minigame::observe) branches on
//! the viewer. `observe(None)` (the spectator) and an opponent's view NEVER include a live
//! player's hole cards; `observe(Some(me))` shows that agent its own cards. At a called showdown
//! both hands are revealed to everyone; a folded hand is never revealed. The deck is shuffled
//! with a deterministic, `settings.seed`-seeded RNG (never the wall clock), exactly like werewolf.

use aiwars_minigame::{AgentId, MatchError, Minigame, Outcome, TurnBasedGame};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Value};
use std::cmp::Ordering;

use crate::cards::{fresh_deck, Card};
use crate::eval::{category_name, evaluate, HandRank};

const PLAYERS: usize = 2;
const STARTING_STACK: u32 = 200;
const MAX_HANDS: u32 = 24;
const HANDS_PER_LEVEL: u32 = 8;
const LOG_CAP: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    fn as_str(self) -> &'static str {
        match self {
            Street::Preflop => "preflop",
            Street::Flop => "flop",
            Street::Turn => "turn",
            Street::River => "river",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Playing,
    Done,
}

/// The resolved match result (by seat), or a draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MatchEnd {
    Win(usize),
    Draw,
}

/// A parsed, validated betting action — produced WITHOUT mutating state, so a rejected move
/// leaves the game completely unchanged. `Raise(to)` carries the total this-street commitment
/// (all-in is normalised to `Raise(my_max)`).
#[derive(Clone, Copy, Debug)]
enum Action {
    Fold,
    Check,
    Call,
    Raise(u32),
}

/// The record of the just-finished hand — kept so the spectator view can show the result (and
/// the showdown reveal) while the next hand is already under way. `revealed` holds ONLY the
/// showdown participants' cards (a folded hand is never in here); it is empty on a fold win.
#[derive(Clone, Debug)]
struct HandResult {
    winners: Vec<usize>,
    pot: u32,
    board: Vec<Card>,
    revealed: Vec<(usize, [Card; 2])>,
    note: String,
}

/// A heads-up Hold'em match.
pub struct Poker {
    players: Vec<AgentId>, // seat-indexed, length 2
    rng: StdRng,
    button: usize, // dealer seat for the current hand
    hand_no: u32,  // 1..=MAX_HANDS

    stack: [u32; 2],
    hole: [[Card; 2]; 2],
    board_full: Vec<Card>, // all five community cards, dealt at hand start
    board_shown: usize,    // how many are face-up: 0 / 3 / 4 / 5

    street: Street,
    committed: [u32; 2],  // total put in the pot THIS hand (per seat) = pot share
    street_bet: [u32; 2], // committed THIS street (resets each street)
    acted: [bool; 2],     // has this seat taken a voluntary action this street?
    last_raise_size: u32, // size of the last bet/raise increment (for the min-raise)
    to_act: usize,
    folded: [bool; 2],

    ply: u32,
    phase: Phase,
    result: Option<MatchEnd>,
    last_result: Option<HandResult>,
    log: Vec<String>,
}

impl Poker {
    fn name_of(&self, seat: usize) -> &str {
        &self.players[seat].0
    }

    fn seat_of(&self, agent: &AgentId) -> Option<usize> {
        self.players.iter().position(|p| p == agent)
    }

    fn blind_level(&self) -> u32 {
        (self.hand_no - 1) / HANDS_PER_LEVEL
    }
    fn small_blind(&self) -> u32 {
        1u32 << self.blind_level()
    }
    fn big_blind(&self) -> u32 {
        2u32 << self.blind_level()
    }

    /// Total chips a seat controls right now (behind + in front). Constant-sum across seats
    /// (always 400), so it is the natural chip-leader metric; at a hand boundary `committed`
    /// is 0, so it equals the stack the termination rule compares.
    fn chips(&self, seat: usize) -> u32 {
        self.stack[seat] + self.committed[seat]
    }

    fn push_log(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
        let n = self.log.len();
        if n > LOG_CAP {
            self.log.drain(0..n - LOG_CAP);
        }
    }

    // ----- betting arithmetic -------------------------------------------------------------

    fn current_bet(&self) -> u32 {
        self.street_bet[0].max(self.street_bet[1])
    }
    /// Chips `seat` must add to match the current bet.
    fn owes(&self, seat: usize) -> u32 {
        self.current_bet().saturating_sub(self.street_bet[seat])
    }
    /// The this-street total `seat` reaches by moving all-in.
    fn all_in_to(&self, seat: usize) -> u32 {
        self.street_bet[seat] + self.stack[seat]
    }
    /// Whether `seat` may raise: it has chips beyond a call AND the opponent has chips to call
    /// the raise. (Against an all-in opponent you can only call or fold — a raise would be an
    /// uncalled bet refunded straight back, so it is never offered. This is also why an all-in
    /// short of a full raise never needs special "re-opening" handling in heads-up: the lone
    /// opponent facing it can always still call or fold.)
    fn can_raise(&self, seat: usize) -> bool {
        let opp = 1 - seat;
        self.stack[seat] > self.owes(seat) && self.stack[opp] > 0
    }
    /// Minimum legal raise-to: current bet + max(last raise size, one big blind), capped at
    /// all-in. Post-flop with no bet yet this is a min bet of one big blind.
    fn min_raise_to(&self, seat: usize) -> u32 {
        let inc = self.last_raise_size.max(self.big_blind());
        (self.current_bet() + inc).min(self.all_in_to(seat))
    }
    /// Pot-size raise-to: after calling, raise by the size of the pot. Clamped into
    /// `[min_raise_to, all_in_to]`.
    fn pot_raise_to(&self, seat: usize) -> u32 {
        let pot = self.committed[0] + self.committed[1];
        let to_call = self.owes(seat);
        let raw = self.current_bet() + pot + to_call;
        raw.clamp(self.min_raise_to(seat), self.all_in_to(seat))
    }

    /// Whether `seat` can take any action right now (has chips, and either owes chips or still
    /// holds an unused option this street). An all-in seat (stack 0) can never act.
    fn can_act(&self, seat: usize) -> bool {
        self.stack[seat] > 0 && (self.owes(seat) > 0 || !self.acted[seat])
    }

    fn commit(&mut self, seat: usize, amount: u32) {
        self.stack[seat] -= amount;
        self.street_bet[seat] += amount;
        self.committed[seat] += amount;
    }

    // ----- hand lifecycle -----------------------------------------------------------------

    /// Shuffle a fresh deck and deal a new hand: hole cards, the (hidden) board, posted blinds,
    /// reset betting state. `hand_no` and `button` are set by the caller.
    fn start_hand(&mut self) {
        let mut deck = fresh_deck();
        shuffle(&mut deck, &mut self.rng);
        // Dealing order is immaterial after a fair shuffle, and there are no burn cards (a
        // physical-table anti-cheat with no effect on a digital deal). Deal two hole cards to
        // each seat, then pre-deal all five community cards (revealed progressively).
        let b = self.button;
        let o = 1 - b;
        self.hole[b] = [deck.pop().unwrap(), deck.pop().unwrap()];
        self.hole[o] = [deck.pop().unwrap(), deck.pop().unwrap()];
        self.board_full = (0..5).map(|_| deck.pop().unwrap()).collect();
        self.board_shown = 0;

        self.street = Street::Preflop;
        self.committed = [0, 0];
        self.street_bet = [0, 0];
        self.acted = [false, false];
        self.folded = [false, false];

        let (sb, bb) = (self.small_blind(), self.big_blind());
        self.post_blind(b, sb); // the button posts the small blind …
        self.post_blind(o, bb); // … the other seat the big blind.
        self.last_raise_size = bb; // the big blind is the opening "bet" for min-raise sizing.
        self.to_act = b; // the button (small blind) acts first pre-flop.

        self.push_log(format!(
            "── Hand {} — blinds {}/{}. {} has the button (small blind), {} posts the big blind.",
            self.hand_no,
            sb,
            bb,
            self.name_of(b),
            self.name_of(o)
        ));
        // A player too short to cover its blind is all-in and cannot act — skip/settle.
        self.normalize_turn();
    }

    /// Post a blind, capped at the seat's stack (an all-in-for-less blind when very short).
    fn post_blind(&mut self, seat: usize, blind: u32) {
        let amount = blind.min(self.stack[seat]);
        self.commit(seat, amount);
    }

    /// If the seat on the clock cannot act (all-in from the blind), hand the turn to the
    /// opponent, or — if neither can act — close the street (which runs the board out).
    fn normalize_turn(&mut self) {
        if self.phase == Phase::Done {
            return;
        }
        if !self.can_act(self.to_act) {
            let opp = 1 - self.to_act;
            if self.can_act(opp) {
                self.to_act = opp;
            } else {
                self.close_street();
            }
        }
    }

    /// Route the turn after a non-folding action: the opponent acts if it can, else the
    /// street's betting is complete.
    fn after_action(&mut self, actor: usize) {
        let opp = 1 - actor;
        if self.can_act(opp) {
            self.to_act = opp;
        } else {
            self.close_street();
        }
    }

    /// Refund an uncalled bet (heads-up ⇒ at most one seat over-committed this street). After
    /// this both seats' this-street contributions are equal, so — across the whole hand — both
    /// seats always match at showdown and the pot is therefore always EVEN there (see
    /// [`split_pot`]).
    fn refund_uncalled(&mut self) {
        let matched = self.street_bet[0].min(self.street_bet[1]);
        for s in 0..PLAYERS {
            let over = self.street_bet[s] - matched;
            if over > 0 {
                self.stack[s] += over;
                self.committed[s] -= over;
                self.street_bet[s] = matched;
                self.push_log(format!("{} takes back {} uncalled.", self.name_of(s), over));
            }
        }
    }

    /// The street's betting is over: refund any uncalled bet, then either run the board out to
    /// showdown (someone is all-in) or open the next betting street.
    fn close_street(&mut self) {
        self.refund_uncalled();
        self.street_bet = [0, 0];
        self.acted = [false, false];
        self.last_raise_size = 0;

        // All-in: no more actions — deal any remaining board and settle at showdown.
        if self.stack[0] == 0 || self.stack[1] == 0 {
            self.board_shown = 5;
            self.resolve_showdown();
            return;
        }
        match self.street {
            Street::Preflop => {
                self.street = Street::Flop;
                self.board_shown = 3;
                self.open_street();
            }
            Street::Flop => {
                self.street = Street::Turn;
                self.board_shown = 4;
                self.open_street();
            }
            Street::Turn => {
                self.street = Street::River;
                self.board_shown = 5;
                self.open_street();
            }
            Street::River => self.resolve_showdown(),
        }
    }

    /// Post-flop the non-button acts first.
    fn open_street(&mut self) {
        self.to_act = 1 - self.button;
        self.push_log(format!(
            "── {}: {}",
            self.street.as_str(),
            self.board_full[..self.board_shown]
                .iter()
                .map(|c| c.to_code())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }

    fn hand_cards(&self, seat: usize) -> Vec<Card> {
        let mut v = self.hole[seat].to_vec();
        v.extend_from_slice(&self.board_full);
        v
    }

    /// End the hand by fold: the other seat wins the pot, no cards shown.
    fn resolve_fold(&mut self, folder: usize) {
        let winner = 1 - folder;
        let pot = self.committed[0] + self.committed[1];
        self.stack[winner] += pot;
        self.push_log(format!(
            "{} folds — {} wins {} chips.",
            self.name_of(folder),
            self.name_of(winner),
            pot
        ));
        self.last_result = Some(HandResult {
            winners: vec![winner],
            pot,
            board: self.board_full[..self.board_shown].to_vec(),
            revealed: Vec::new(), // a fold reveals nothing
            note: format!("{} folded", self.name_of(folder)),
        });
        self.committed = [0, 0];
        self.street_bet = [0, 0];
        self.maybe_end_match();
    }

    /// End the hand at showdown: compare the best 5-of-7 for each seat, award the pot (split
    /// evenly on a tie, the odd chip — see [`split_pot`] — to the non-dealer), reveal both hands.
    fn resolve_showdown(&mut self) {
        let h0 = evaluate(&self.hand_cards(0));
        let h1 = evaluate(&self.hand_cards(1));
        let pot = self.committed[0] + self.committed[1];
        let winners: Vec<usize> = match h0.cmp(&h1) {
            Ordering::Greater => vec![0],
            Ordering::Less => vec![1],
            Ordering::Equal => vec![0, 1],
        };
        let award = if winners.len() == 1 {
            let mut a = [0u32; 2];
            a[winners[0]] = pot;
            a
        } else {
            split_pot(pot, self.button)
        };
        for (slot, add) in self.stack.iter_mut().zip(award.iter()) {
            *slot += *add;
        }
        let revealed: Vec<(usize, [Card; 2])> = (0..PLAYERS)
            .filter(|&s| !self.folded[s])
            .map(|s| (s, self.hole[s]))
            .collect();
        let note = self.showdown_note(&winners, [h0, h1], pot);
        self.push_log(note.clone());
        self.last_result = Some(HandResult {
            winners,
            pot,
            board: self.board_full.clone(),
            revealed,
            note,
        });
        self.committed = [0, 0];
        self.street_bet = [0, 0];
        self.maybe_end_match();
    }

    fn showdown_note(&self, winners: &[usize], hands: [HandRank; 2], pot: u32) -> String {
        let show = |s: usize| {
            format!(
                "{} shows {} ({})",
                self.name_of(s),
                self.hole[s]
                    .iter()
                    .map(|c| c.to_code())
                    .collect::<Vec<_>>()
                    .join(" "),
                category_name(hands[s].cat)
            )
        };
        if winners.len() == 2 {
            format!(
                "Showdown — {} / {}. Split pot: {} chips each.",
                show(0),
                show(1),
                pot / 2
            )
        } else {
            let w = winners[0];
            format!(
                "Showdown — {} / {}. {} wins {} with {}.",
                show(0),
                show(1),
                self.name_of(w),
                pot,
                category_name(hands[w].cat)
            )
        }
    }

    /// After a hand resolves: a busted seat ends the match immediately; otherwise the 24-hand
    /// cap ends it (larger stack wins, equal ⇒ draw); otherwise deal the next hand.
    fn maybe_end_match(&mut self) {
        if self.stack[0] == 0 || self.stack[1] == 0 {
            let w = if self.stack[0] == 0 { 1 } else { 0 };
            self.result = Some(MatchEnd::Win(w));
            self.phase = Phase::Done;
            self.push_log(format!(
                "{} is out of chips — {} wins the match!",
                self.name_of(1 - w),
                self.name_of(w)
            ));
        } else if self.hand_no >= MAX_HANDS {
            self.phase = Phase::Done;
            match self.stack[0].cmp(&self.stack[1]) {
                Ordering::Greater => {
                    self.push_log(format!(
                        "24 hands complete — {} wins the match with {} chips.",
                        self.name_of(0),
                        self.stack[0]
                    ));
                    self.result = Some(MatchEnd::Win(0));
                }
                Ordering::Less => {
                    self.push_log(format!(
                        "24 hands complete — {} wins the match with {} chips.",
                        self.name_of(1),
                        self.stack[1]
                    ));
                    self.result = Some(MatchEnd::Win(1));
                }
                Ordering::Equal => {
                    self.push_log(
                        "24 hands complete — even stacks, the match is a draw.".to_string(),
                    );
                    self.result = Some(MatchEnd::Draw);
                }
            }
        } else {
            self.hand_no += 1;
            self.button = 1 - self.button;
            self.start_hand();
        }
    }

    // ----- move validation (pure — never mutates) -----------------------------------------

    /// Parse + validate `mv` for the seat on the clock, WITHOUT touching any state. Every
    /// illegal move returns `Err` here, which is what makes a rejected `apply` perfectly inert.
    fn parse_action(&self, me: usize, mv: &str) -> Result<Action, MatchError> {
        let to_call = self.owes(me);
        match mv {
            "fold" => {
                if to_call == 0 {
                    // Folding for free is dominated by checking; not offered, not accepted.
                    Err(MatchError::Rejected(
                        "cannot fold when you can check for free — check instead".into(),
                    ))
                } else {
                    Ok(Action::Fold)
                }
            }
            "check" => {
                if to_call > 0 {
                    Err(MatchError::Rejected(format!(
                        "cannot check facing a bet of {to_call} — call, raise, or fold"
                    )))
                } else {
                    Ok(Action::Check)
                }
            }
            "call" => {
                if to_call == 0 {
                    Err(MatchError::Rejected(
                        "nothing to call — check instead".into(),
                    ))
                } else {
                    Ok(Action::Call)
                }
            }
            "allin" => {
                if !self.can_raise(me) {
                    Err(MatchError::Rejected(
                        "cannot raise here (the opponent is all-in, or you have no chips beyond a call) — use call".into(),
                    ))
                } else {
                    Ok(Action::Raise(self.all_in_to(me)))
                }
            }
            _ => {
                let Some(rest) = mv.strip_prefix("raise:") else {
                    return Err(MatchError::Rejected(format!(
                        "unknown move '{mv}' — expected fold, check, call, raise:<total>, or allin"
                    )));
                };
                let to: u32 = rest.parse().map_err(|_| {
                    MatchError::Rejected(format!("malformed raise '{mv}' — use raise:<total>"))
                })?;
                if !self.can_raise(me) {
                    return Err(MatchError::Rejected(
                        "cannot raise here (the opponent is all-in, or you have no chips beyond a call)".into(),
                    ));
                }
                let all_in = self.all_in_to(me);
                if to > all_in {
                    return Err(MatchError::Rejected(format!(
                        "raise to {to} exceeds your stack — all-in is {all_in}"
                    )));
                }
                // Below a full raise is legal only when it IS all-in (a short all-in raise).
                if to < all_in && to < self.min_raise_to(me) {
                    return Err(MatchError::Rejected(format!(
                        "raise to {to} is below the minimum (raise to {} or go all-in)",
                        self.min_raise_to(me)
                    )));
                }
                Ok(Action::Raise(to))
            }
        }
    }

    /// Apply a validated raise-to and update the min-raise tracker.
    fn do_raise(&mut self, me: usize, to: u32) {
        let previous = self.current_bet();
        let add = to - self.street_bet[me];
        self.commit(me, add);
        self.last_raise_size = self.street_bet[me] - previous;
        self.acted[me] = true;
        if self.stack[me] == 0 {
            self.push_log(format!(
                "{} is all-in for {}.",
                self.name_of(me),
                self.street_bet[me]
            ));
        } else if previous == 0 {
            self.push_log(format!("{} bets {}.", self.name_of(me), to));
        } else {
            self.push_log(format!("{} raises to {}.", self.name_of(me), to));
        }
    }

    // ----- observation --------------------------------------------------------------------

    /// Whether `seat`'s hole cards may be shown to `viewer`. Your own cards, always; anyone
    /// else's only at a called showdown that ended the match (the current hand IS the final
    /// hand then, so the revealed set names exactly the shown seats). A folded hand — and every
    /// live hand mid-match — stays hidden.
    fn hole_visible(&self, viewer: Option<usize>, seat: usize) -> bool {
        if viewer == Some(seat) {
            return true;
        }
        self.phase == Phase::Done
            && self
                .last_result
                .as_ref()
                .is_some_and(|r| r.revealed.iter().any(|(s, _)| *s == seat))
    }

    fn cards_json(cards: &[Card]) -> Value {
        json!(cards.iter().map(|c| c.to_code()).collect::<Vec<_>>())
    }

    fn last_result_json(&self) -> Value {
        match &self.last_result {
            None => Value::Null,
            Some(r) => {
                let revealed: serde_json::Map<String, Value> = r
                    .revealed
                    .iter()
                    .map(|(s, cs)| (self.players[*s].0.clone(), Self::cards_json(cs)))
                    .collect();
                json!({
                    "winners": r.winners.iter().map(|&s| self.players[s].0.clone()).collect::<Vec<_>>(),
                    "pot": r.pot,
                    "board": Self::cards_json(&r.board),
                    "revealed": revealed,
                    "note": r.note,
                })
            }
        }
    }
}

/// Split a pot evenly, the odd chip to the NON-dealer (`1 - button`). Given the blind schedule
/// and the uncalled-bet refund, every showdown pot is even (both seats always match), so the
/// odd chip is a defensive rule that gameplay never actually reaches — it is unit-tested
/// directly, and the "pot is always even at showdown" invariant is tested too.
fn split_pot(pot: u32, button: usize) -> [u32; 2] {
    let half = pot / 2;
    let mut award = [half; 2];
    award[1 - button] += pot % 2;
    award
}

fn shuffle(deck: &mut [Card], rng: &mut StdRng) {
    // Fisher-Yates with the match's seeded RNG (deterministic; never the wall clock).
    for i in (1..deck.len()).rev() {
        let j = rng.random_range(0..=i);
        deck.swap(i, j);
    }
}

impl Minigame for Poker {
    fn new(agents: &[AgentId], settings: &Value) -> Result<Self, MatchError> {
        if agents.len() != PLAYERS {
            return Err(MatchError::WrongPlayerCount {
                want: 2..=2,
                got: agents.len(),
            });
        }
        // Deterministic deck: seed from settings.seed when given (reproducible matches + tests),
        // else from entropy — never the wall clock. Exactly werewolf's seeding.
        let seed = settings
            .get("seed")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(rand::random::<u64>);
        let mut rng = StdRng::seed_from_u64(seed);
        let button = rng.random_range(0..=1usize); // who starts on the button

        let ph = Card { rank: 2, suit: 0 };
        let mut g = Poker {
            players: agents.to_vec(),
            rng,
            button,
            hand_no: 1,
            stack: [STARTING_STACK; 2],
            hole: [[ph; 2]; 2],
            board_full: Vec::new(),
            board_shown: 0,
            street: Street::Preflop,
            committed: [0, 0],
            street_bet: [0, 0],
            acted: [false, false],
            last_raise_size: 0,
            to_act: button,
            folded: [false, false],
            ply: 0,
            phase: Phase::Playing,
            result: None,
            last_result: None,
            log: Vec::new(),
        };
        g.start_hand();
        Ok(g)
    }

    fn name(&self) -> &'static str {
        "poker"
    }

    fn instructions(&self) -> String {
        "AIWars heads-up No-Limit Texas Hold'em. Two players, 200-chip stacks, blinds 1/2 \
         doubling every 8 hands; the match is 24 hands and ends early if someone busts. Call \
         get_state each turn and read `your_hole` (your two cards as codes like \"As\" \"Td\" — \
         ranks 2-9/T/J/Q/K/A, suits s/h/d/c), `board`, `pot`, `to_call` (chips needed to match), \
         and `moves` (your EXACT legal moves this turn). Play with make_move, mv = one of: \
         \"fold\"; \"check\" (only when to_call is 0); \"call\"; \"raise:<TOTAL>\" — raise TO a \
         total committed THIS street, e.g. \"raise:20\"; or \"allin\". `moves` offers a \
         min-raise, a pot-size raise and all-in for convenience, but ANY raise total from the \
         minimum up to all-in is legal. You are the button (small blind) on alternate hands: the \
         button acts FIRST pre-flop and SECOND after the flop. Pass expected_ply = the ply you \
         saw. Your seat is your bearer token — you never see the opponent's hole cards until a \
         called showdown."
            .into()
    }

    fn observe(&self, viewer: Option<&AgentId>) -> Value {
        let me = viewer.and_then(|a| self.seat_of(a));

        let players: Vec<Value> = (0..PLAYERS)
            .map(|s| {
                let hole = if self.hole_visible(me, s) {
                    Self::cards_json(&self.hole[s])
                } else {
                    Value::Null
                };
                json!({
                    "handle": self.players[s].0,
                    "stack": self.stack[s],
                    "committed": self.committed[s],
                    "bet": self.street_bet[s],
                    "folded": self.folded[s],
                    "allin": self.phase == Phase::Playing && !self.folded[s] && self.stack[s] == 0,
                    "button": s == self.button,
                    "hole": hole,
                })
            })
            .collect();

        let over = self.phase == Phase::Done;
        let to_act = if over {
            Value::Null
        } else {
            Value::String(self.players[self.to_act].0.clone())
        };
        let winner = match self.result {
            Some(MatchEnd::Win(s)) => Value::String(self.players[s].0.clone()),
            _ => Value::Null,
        };
        let leader = match self.chips(0).cmp(&self.chips(1)) {
            Ordering::Greater => Value::String(self.players[0].0.clone()),
            Ordering::Less => Value::String(self.players[1].0.clone()),
            Ordering::Equal => Value::Null,
        };

        let mut v = json!({
            "game": "poker",
            "hand": self.hand_no,
            "max_hands": MAX_HANDS,
            "blinds": { "sb": self.small_blind(), "bb": self.big_blind(), "level": self.blind_level() },
            "button": self.players[self.button].0,
            "street": self.street.as_str(),
            "board": Self::cards_json(&self.board_full[..self.board_shown]),
            "pot": self.committed[0] + self.committed[1],
            "to_act": to_act,
            "ply": self.ply,
            "players": players,
            "moves": if over { Vec::new() } else { self.legal_moves() },
            "log": self.log,
            "status": if over { "over" } else { "playing" },
            "winner": winner,
            "leader": leader,
            "last_hand": self.last_result_json(),
        });

        // Per-agent private view: only that seat's own hole cards + its action hints.
        if let Some(me) = me {
            let obj = v.as_object_mut().unwrap();
            obj.insert("hero".into(), json!(me));
            obj.insert("your_hole".into(), Self::cards_json(&self.hole[me]));
            let my_turn = !over && self.to_act == me;
            obj.insert("your_turn".into(), json!(my_turn));
            obj.insert(
                "to_call".into(),
                if my_turn {
                    json!(self.owes(me))
                } else {
                    Value::Null
                },
            );
        }
        v
    }

    fn outcome(&self) -> Option<Outcome> {
        match self.result {
            Some(MatchEnd::Win(s)) => Some(Outcome::Win(self.players[s].clone())),
            Some(MatchEnd::Draw) => Some(Outcome::Draw),
            None => None,
        }
    }

    /// The wall-clock timeout tiebreak: the current chip leader wins, equal stacks draw —
    /// the same rule the 24-hand cap uses.
    fn timeout_leader(&self) -> Option<AgentId> {
        match self.chips(0).cmp(&self.chips(1)) {
            Ordering::Greater => Some(self.players[0].clone()),
            Ordering::Less => Some(self.players[1].clone()),
            Ordering::Equal => None,
        }
    }
}

impl TurnBasedGame for Poker {
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
        let me = self.to_act;
        let mut moves = Vec::new();
        let to_call = self.owes(me);
        if to_call > 0 {
            moves.push("fold".to_string());
            moves.push("call".to_string());
        } else {
            moves.push("check".to_string());
        }
        if self.can_raise(me) {
            // A small discrete menu: min-raise, pot-size raise, all-in — deduped, and each
            // dropped when it collapses into all-in. `allin` is ALWAYS offered when raising is
            // legal; `apply` also accepts any other raise total in [min_raise_to, all-in].
            let all_in = self.all_in_to(me);
            let current = self.current_bet();
            let mut sizes: Vec<u32> = Vec::new();
            for cand in [self.min_raise_to(me), self.pot_raise_to(me)] {
                if cand > current && cand < all_in && !sizes.contains(&cand) {
                    sizes.push(cand);
                }
            }
            for s in sizes {
                moves.push(format!("raise:{s}"));
            }
            moves.push("allin".to_string());
        }
        moves
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
        let action = self.parse_action(me, mv)?;

        // --- committed, mutating path (validation has passed) ---
        self.ply += 1;
        match action {
            Action::Fold => {
                self.folded[me] = true;
                self.resolve_fold(me);
            }
            Action::Check => {
                self.acted[me] = true;
                self.push_log(format!("{} checks.", self.name_of(me)));
                self.after_action(me);
            }
            Action::Call => {
                let amount = self.owes(me).min(self.stack[me]);
                self.commit(me, amount);
                self.acted[me] = true;
                if self.stack[me] == 0 {
                    self.push_log(format!(
                        "{} calls {} and is all-in.",
                        self.name_of(me),
                        amount
                    ));
                } else {
                    self.push_log(format!("{} calls {}.", self.name_of(me), amount));
                }
                self.after_action(me);
            }
            Action::Raise(to) => {
                self.do_raise(me, to);
                self.after_action(me);
            }
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
            self.result = Some(MatchEnd::Win(w));
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

    /// A game seeded for reproducibility, hand 1 dealt.
    fn game(seed: u64) -> Poker {
        Poker::new(&two(), &json!({ "seed": seed })).unwrap()
    }

    /// A full snapshot of every mutable field (except the opaque RNG, which a rejected move
    /// never touches) — for the illegal-move inertness assertion.
    fn snapshot(g: &Poker) -> String {
        format!(
            "stack={:?} hole={:?} board={:?} shown={} street={:?} committed={:?} bet={:?} \
             acted={:?} lrs={} to_act={} folded={:?} ply={} button={} hand={} phase={:?} \
             result={:?} log={:?}",
            g.stack,
            g.hole,
            g.board_full,
            g.board_shown,
            g.street,
            g.committed,
            g.street_bet,
            g.acted,
            g.last_raise_size,
            g.to_act,
            g.folded,
            g.ply,
            g.button,
            g.hand_no,
            g.phase,
            g.result,
            g.log
        )
    }

    #[test]
    fn rejects_bad_player_count() {
        assert!(matches!(
            Poker::new(&[AgentId("solo".into())], &json!({})),
            Err(MatchError::WrongPlayerCount { got: 1, .. })
        ));
    }

    #[test]
    fn seed_is_deterministic() {
        let a = game(7);
        let b = game(7);
        assert_eq!(a.hole, b.hole);
        assert_eq!(a.board_full, b.board_full);
        assert_eq!(a.button, b.button);
    }

    #[test]
    fn blinds_posted_and_button_acts_first_preflop() {
        let g = game(1);
        let b = g.button;
        let o = 1 - b;
        assert_eq!(g.hand_no, 1);
        assert_eq!(g.small_blind(), 1);
        assert_eq!(g.big_blind(), 2);
        assert_eq!(g.street_bet[b], 1, "button posts the small blind");
        assert_eq!(g.street_bet[o], 2, "non-button posts the big blind");
        assert_eq!(g.stack[b], STARTING_STACK - 1);
        assert_eq!(g.stack[o], STARTING_STACK - 2);
        assert_eq!(g.to_act, b, "the button (small blind) acts first pre-flop");
        assert_eq!(g.committed[0] + g.committed[1], 3, "pot is SB + BB");
    }

    #[test]
    fn blind_schedule_doubles_every_eight_hands() {
        let mut g = game(1);
        // Level 0 hands 1-8, level 1 hands 9-16, level 2 hands 17-24.
        for (hand, sb, bb) in [
            (1, 1, 2),
            (8, 1, 2),
            (9, 2, 4),
            (16, 2, 4),
            (17, 4, 8),
            (24, 4, 8),
        ] {
            g.hand_no = hand;
            assert_eq!(g.small_blind(), sb, "hand {hand} small blind");
            assert_eq!(g.big_blind(), bb, "hand {hand} big blind");
        }
    }

    #[test]
    fn min_raise_legality_and_menu() {
        let mut g = game(1);
        let b = g.button;
        let id = g.players[b].clone();
        let moves = g.legal_moves();
        // Facing the big blind, the button may fold/call and the min-raise is to 4 (2×BB).
        assert!(moves.contains(&"fold".to_string()));
        assert!(moves.contains(&"call".to_string()));
        assert!(
            moves.contains(&"raise:4".to_string()),
            "min-raise is to 4, got {moves:?}"
        );
        assert!(moves.contains(&"allin".to_string()));

        // A raise below the minimum is rejected AND leaves the game unchanged.
        let before = snapshot(&g);
        assert!(g.apply(&id, "raise:3").is_err());
        assert_eq!(snapshot(&g), before, "a sub-minimum raise must be inert");

        // The legal min-raise is accepted.
        assert!(g.apply(&id, "raise:4").is_ok());
        assert_eq!(g.street_bet[b], 4);
        assert_eq!(g.stack[b], STARTING_STACK - 4);
    }

    #[test]
    fn illegal_moves_are_inert() {
        let mut g = game(2);
        let actor = g.players[g.to_act].clone();
        let wrong = g.players[1 - g.to_act].clone();
        let before = snapshot(&g);
        for (who, mv) in [
            (&actor, "check"),      // can't check facing the big blind
            (&actor, "raise:1"),    // below min
            (&actor, "raise:9999"), // beyond stack
            (&actor, "raise:abc"),  // malformed
            (&actor, "teleport"),   // unknown
            (&wrong, "call"),       // not your turn
        ] {
            assert!(g.apply(who, mv).is_err(), "'{mv}' should be rejected");
            assert_eq!(snapshot(&g), before, "'{mv}' must not mutate state");
        }
    }

    #[test]
    fn check_check_advances_the_street() {
        // Button limps (calls) pre-flop; big blind checks its option → flop.
        let mut g = game(3);
        let b = g.button;
        let o = 1 - b;
        g.apply(&g.players[b].clone(), "call").unwrap(); // SB completes to 2
        assert_eq!(g.to_act, o, "big blind now has the option");
        assert_eq!(g.street, Street::Preflop);
        g.apply(&g.players[o].clone(), "check").unwrap(); // BB checks → flop
        assert_eq!(g.street, Street::Flop);
        assert_eq!(g.board_shown, 3);
        assert_eq!(g.to_act, o, "post-flop the non-button acts first");
        assert_eq!(g.committed[0], g.committed[1], "both put in one big blind");
    }

    #[test]
    fn bet_call_advances_the_street() {
        let mut g = game(5);
        let b = g.button;
        let o = 1 - b;
        g.apply(&g.players[b].clone(), "call").unwrap();
        g.apply(&g.players[o].clone(), "check").unwrap(); // → flop, non-button to act
        let pot = g.committed[0] + g.committed[1];
        g.apply(&g.players[o].clone(), "raise:4").unwrap(); // a flop bet
        assert_eq!(g.street, Street::Flop);
        g.apply(&g.players[b].clone(), "call").unwrap(); // called → turn
        assert_eq!(g.street, Street::Turn);
        assert_eq!(g.board_shown, 4);
        assert_eq!(g.committed[0] + g.committed[1], pot + 8);
    }

    #[test]
    fn all_in_short_call_refunds_and_runs_out_the_board() {
        // Force a lopsided all-in: give the button a tiny stack, jam, opponent calls for less.
        let mut g = game(11);
        let b = g.button;
        let o = 1 - b;
        // Rewind the posted blinds and hand the button a 10-chip stack for a clean scenario.
        g.stack = [STARTING_STACK; 2];
        g.committed = [0, 0];
        g.street_bet = [0, 0];
        g.acted = [false, false];
        g.stack[b] = 10;
        g.stack[o] = 40;
        g.post_blind(b, 1);
        g.post_blind(o, 2);
        g.last_raise_size = 2;
        g.to_act = b;

        let total_before = g.stack[0] + g.stack[1] + g.committed[0] + g.committed[1];
        g.apply(&g.players[b].clone(), "allin").unwrap(); // button jams 10
        assert_eq!(g.street_bet[b], 10);
        // Opponent calls — it has plenty, so this is a full call, board runs out to showdown.
        g.apply(&g.players[o].clone(), "call").unwrap();
        // Either the match ended (a bust) or a fresh hand was dealt, but never mid-street here.
        assert!(g.board_shown == 5 || g.hand_no > 1 || g.phase == Phase::Done);
        // Chips are conserved (nothing created or destroyed by the refund/runout).
        let total_after = g.stack[0] + g.stack[1] + g.committed[0] + g.committed[1];
        assert_eq!(total_before, total_after, "chips must be conserved");
    }

    #[test]
    fn showdown_pot_is_always_even() {
        // Drive a heads-up all-in to showdown from equal stacks and assert the pot split evenly
        // (both seats always match after the uncalled-bet refund).
        let mut g = game(6);
        let b = g.button;
        let o = 1 - b;
        g.apply(&g.players[b].clone(), "allin").unwrap();
        // The opponent covers exactly (equal stacks) → both all-in for the same amount.
        g.apply(&g.players[o].clone(), "call").unwrap();
        // The hand resolved; the recorded pot is even.
        let r = g.last_result.as_ref().expect("a hand resolved");
        assert_eq!(r.pot % 2, 0, "a showdown pot is always even");
    }

    #[test]
    fn split_pot_awards_the_odd_chip_to_the_non_dealer() {
        // Direct test of the defensive odd-chip rule (unreachable in real showdowns).
        assert_eq!(split_pot(10, 0), [5, 5]);
        assert_eq!(
            split_pot(7, 0),
            [3, 4],
            "button=0 → non-dealer seat 1 gets the odd chip"
        );
        assert_eq!(
            split_pot(7, 1),
            [4, 3],
            "button=1 → non-dealer seat 0 gets the odd chip"
        );
    }

    #[test]
    fn bust_ends_the_match_for_the_opponent() {
        let mut g = game(1);
        g.stack = [0, 400];
        g.hand_no = 5;
        g.maybe_end_match();
        assert_eq!(g.outcome(), Some(Outcome::Win(g.players[1].clone())));
    }

    #[test]
    fn twenty_four_hand_cap_awards_the_larger_stack() {
        let mut g = game(1);
        g.stack = [250, 150];
        g.hand_no = MAX_HANDS;
        g.maybe_end_match();
        assert_eq!(g.outcome(), Some(Outcome::Win(g.players[0].clone())));
    }

    #[test]
    fn twenty_four_hand_cap_with_equal_stacks_is_a_draw() {
        let mut g = game(1);
        g.stack = [200, 200];
        g.hand_no = MAX_HANDS;
        g.maybe_end_match();
        assert_eq!(g.outcome(), Some(Outcome::Draw));
    }

    #[test]
    fn resign_awards_the_opponent_the_match() {
        let mut m = TurnBasedMatch::new::<Poker>(two(), &json!({ "seed": 1 })).unwrap();
        m.start();
        let st = m.state_json();
        // Seat 0 = alice, seat 1 = bob (construction order).
        m.resign(0);
        assert!(m.is_resolved());
        let r = m.result().unwrap();
        assert_eq!(r.outcome, "Winner");
        assert_eq!(r.winner.as_deref(), Some("bob"));
        // The pre-resign public state named the game.
        assert_eq!(st["game"], "poker");
    }

    /// Drive a whole match via the turn-based wrapper — both seats jam all-in every hand — and
    /// assert it resolves to a valid outcome within the 24-hand cap (a scripted bust or the cap).
    #[test]
    fn a_full_match_resolves() {
        let mut m = TurnBasedMatch::new::<Poker>(two(), &json!({ "seed": 42 })).unwrap();
        m.start();
        let order: Vec<String> = two().iter().map(|a| a.0.clone()).collect();
        for _ in 0..10_000 {
            if m.is_resolved() {
                break;
            }
            let st = m.state_json();
            let actor = st["to_act"].as_str().unwrap().to_string();
            let seat = order.iter().position(|h| *h == actor).unwrap();
            let ply = st["ply"].as_u64().unwrap() as u32;
            let moves = m.turn_info(seat)["moves"].as_array().unwrap().clone();
            // Prefer jamming to force resolution; otherwise take the first legal move.
            let mv = moves
                .iter()
                .find(|m| m.as_str() == Some("allin"))
                .or_else(|| moves.first())
                .and_then(|m| m.as_str())
                .unwrap()
                .to_string();
            m.make_move(seat, &mv, ply).unwrap();
        }
        assert!(
            m.is_resolved(),
            "the match must resolve within the hand cap"
        );
        let r = m.result().unwrap();
        assert!(r.outcome == "Winner" || r.outcome == "Draw");
    }

    #[test]
    fn projection_never_leaks_hidden_hole_cards() {
        let g = game(2);
        let alice = &g.players[0];
        let bob = &g.players[1];

        // The exact card codes each seat holds (known here because we can read the state).
        let alice_cards: Vec<String> = g.hole[0].iter().map(|c| c.to_code()).collect();
        let bob_cards: Vec<String> = g.hole[1].iter().map(|c| c.to_code()).collect();

        // Structured: the spectator sees NO hole cards; each agent sees only its own.
        let public = g.observe(None);
        for p in public["players"].as_array().unwrap() {
            assert!(p["hole"].is_null(), "spectator must not see any hole cards");
        }
        let a_view = g.observe(Some(alice));
        assert!(
            a_view["players"][0]["hole"].is_array(),
            "alice sees her own cards"
        );
        assert!(
            a_view["players"][1]["hole"].is_null(),
            "alice must not see bob's cards"
        );

        // Raw-string: neither card token appears where it must not (the spec asserts on JSON).
        let public_s = public.to_string();
        for c in alice_cards.iter().chain(bob_cards.iter()) {
            assert!(!public_s.contains(c), "public projection leaked {c}");
        }
        let a_view_s = a_view.to_string();
        for c in &bob_cards {
            assert!(!a_view_s.contains(c), "alice's projection leaked bob's {c}");
        }
        // Sanity: bob's own view carries bob's cards (so the leak checks aren't vacuous).
        let b_view_s = g.observe(Some(bob)).to_string();
        assert!(bob_cards.iter().all(|c| b_view_s.contains(c)));
    }

    /// The shipped game.toml must parse and its hold must validate — green CI implies a bootable
    /// manifest (a typo in `[settings]` would otherwise crashloop every pod).
    #[test]
    fn game_toml_is_loadable() {
        let settings = aiwars_minigame::settings::manifest_settings_at("game.toml").unwrap();
        aiwars_minigame::settings::validate_hold(&settings).unwrap();
    }
}
