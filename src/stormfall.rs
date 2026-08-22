//! Stormfall Isle — a turn-based battle-royale, refereed on the shared `aiwars-minigame`
//! library (tier 1: `Minigame` + `TurnBasedGame`).
//!
//! Two survivors fight on a shrinking-storm voxel island. On each of its turns an
//! agent picks ONE action from its legal moves:
//! - `loot:crate` — sprint to the nearest crate; if reached, grab gear (a stronger
//!   `hunt`) — but you stand exposed in the open.
//! - `hide:bunker` — turtle toward a bunker: safe, patches a little HP, no gear.
//! - `rotate:eye` — move two steps toward the safe-zone EYE (dodge the storm).
//! - `hunt:rival` — strike the rival (only legal when they're reachable).
//!
//! Between rounds a magenta STORM WALL contracts ring-by-ring toward a seeded eye,
//! draining HP from anyone caught outside the safe ring. Eliminate the rival (HP→0)
//! or be the last one standing to win; a double-KO on the same tick is a draw. Same
//! seed ⇒ identical eye + crate layout (deterministic / replayable).
//!
//! This is the engine-side rules ONLY — the agent's PUBLIC PROMPT (its doctrine)
//! is what chooses which legal action it plays each turn, via `make_move`. The
//! POC's auto-pick/doctrine selection is dropped: the real LLM agent (or, since
//! this port, the human in the seat) decides.

use serde_json::{json, Value};

use aiwars_minigame::{AgentId, MatchError, Minigame, Outcome, TurnBasedGame};

const GN: i32 = 9; // grid cells per side
const ROUNDS: u32 = 5; // storm contractions
const MID: f64 = (GN as f64 - 1.0) / 2.0;
/// Ring radius (Chebyshev) per round — index by round, last entry repeated for
/// the final resolve. Mirrors the POC's `ringR`.
const RING_R: [f64; 6] = [3.6, 2.8, 2.0, 1.3, 0.8, 0.4];
/// Exactly two survivors land on the isle.
const PLAYERS: usize = 2;

/// Deterministic per-key PRNG mix (mulberry32-ish), matching the POC engine's
/// shape so the web demo and the referee agree on a seed's layout.
fn rng_u32(mut a: u32) -> u32 {
    a = a.wrapping_add(0x6d2b79f5);
    let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
    t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
    t ^ (t >> 14)
}
/// A 0..1 float from a (seed, slot) tuple.
fn frac(seed: u64, slot: u32) -> f64 {
    let mixed = (seed as u32)
        .wrapping_mul(977)
        .wrapping_add(slot.wrapping_mul(131))
        .wrapping_add(0x9e37_79b9);
    (rng_u32(mixed) as f64) / (u32::MAX as f64)
}

fn clampf(v: f64, lo: f64, hi: f64) -> f64 {
    v.clamp(lo, hi)
}
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, hi)
}
fn signum(d: i32) -> i32 {
    (d > 0) as i32 - (d < 0) as i32
}

/// A loot crate at a grid cell, worth `gear` when first reached.
#[derive(Clone)]
struct Crate {
    gx: i32,
    gy: i32,
    gear: u32,
    taken: bool,
}

/// A combatant — the two real agents are `is_npc == false`; the rest are seeded
/// flavour bots that the storm mostly attrits (they never win the market).
#[derive(Clone)]
struct Ent {
    id: &'static str,
    gx: i32,
    gy: i32,
    hp: i32,
    maxhp: i32,
    gear: u32,
    alive: bool,
    is_npc: bool,
    name: String,
    col: &'static str,
    death_round: i32,
    last_move: Option<String>,
}
impl Ent {
    fn score(&self) -> i32 {
        self.hp + self.gear as i32 * 6
    }
}

/// The two-player Stormfall game. The survivors are entity indices 0 (A) and 1 (B),
/// index-aligned with `players`; NPC husks follow at indices 2..
pub struct Stormfall {
    /// The survivors by IDENTITY, in seat order. The library bridges an auth-resolved seat
    /// to its `AgentId` before calling us, so the game never handles seat indices from
    /// outside.
    players: Vec<AgentId>,
    ents: Vec<Ent>,
    crates: Vec<Crate>,
    bunkers: [(i32, i32); 3],
    to_move: usize,
    /// Whether each agent (0,1) has already acted in the CURRENT round.
    acted: [bool; PLAYERS],
    round: u32,
    ply: u32,
    seed: u64,
    final_cx: f64,
    final_cy: f64,
    resigned_by: Option<usize>,
    winner_idx: Option<usize>,
    win_reason: &'static str,
    resolved: bool,
}

impl Stormfall {
    /// The seat holding `agent`, or `None` for an id that never landed on the isle.
    fn seat_of(&self, agent: &AgentId) -> Option<usize> {
        self.players.iter().position(|p| p == agent)
    }

    /// The current ring center for `round` (drifts from board-mid toward the
    /// final eye across rounds).
    fn center_at(&self, round: u32) -> (f64, f64) {
        let f = clampf(round as f64 / ROUNDS as f64, 0.0, 1.0);
        (lerp(MID, self.final_cx, f), lerp(MID, self.final_cy, f))
    }

    fn ring_r(round: u32) -> f64 {
        RING_R[(round as usize).min(RING_R.len() - 1)]
    }

    fn in_ring(&self, gx: i32, gy: i32, round: u32) -> bool {
        let (cx, cy) = self.center_at(round);
        let d = (gx as f64 - cx).abs().max((gy as f64 - cy).abs());
        d <= Self::ring_r(round) + 0.001
    }

    fn alive_count(&self) -> usize {
        self.ents.iter().filter(|e| e.alive).count()
    }

    fn nearest_crate(&self, gx: i32, gy: i32) -> Option<usize> {
        let mut best = None;
        let mut bd = i32::MAX;
        for (i, c) in self.crates.iter().enumerate() {
            if c.taken {
                continue;
            }
            let d = (c.gx - gx).abs() + (c.gy - gy).abs();
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    }

    fn reachable(&self, from: usize, to: usize) -> bool {
        let a = &self.ents[from];
        let b = &self.ents[to];
        b.alive && (a.gx - b.gx).abs().max((a.gy - b.gy).abs()) <= 2
    }

    fn step_toward(e: &mut Ent, tx: f64, ty: f64) {
        let dx = signum((tx.round() as i32) - e.gx);
        let dy = signum((ty.round() as i32) - e.gy);
        e.gx = clampi(e.gx + dx, 0, GN - 1);
        e.gy = clampi(e.gy + dy, 0, GN - 1);
    }

    /// Legal actions for agent `idx` (0 or 1) at the current round.
    fn legal_for(&self, idx: usize) -> Vec<String> {
        let me = &self.ents[idx];
        let mut l = Vec::new();
        if self.nearest_crate(me.gx, me.gy).is_some() {
            l.push("loot:crate".to_string());
        }
        l.push("hide:bunker".to_string());
        l.push("rotate:eye".to_string());
        let op = 1 - idx;
        if self.reachable(idx, op) {
            l.push("hunt:rival".to_string());
        }
        l
    }

    /// NPCs act once per round: drift toward center or loot the nearest crate.
    fn npc_act(&mut self) {
        let round = self.round;
        let (cx, cy) = self.center_at(round);
        let n = self.ents.len();
        for i in PLAYERS..n {
            if !self.ents[i].alive {
                continue;
            }
            let (gx, gy) = (self.ents[i].gx, self.ents[i].gy);
            if !self.in_ring(gx, gy, round) {
                Self::step_toward(&mut self.ents[i], cx, cy);
            } else if let Some(ci) = self.nearest_crate(gx, gy) {
                let (tgx, tgy) = (self.crates[ci].gx, self.crates[ci].gy);
                Self::step_toward(&mut self.ents[i], tgx as f64, tgy as f64);
                if self.ents[i].gx == tgx && self.ents[i].gy == tgy {
                    self.crates[ci].taken = true;
                    self.ents[i].gear += self.crates[ci].gear;
                }
            }
        }
    }

    /// Apply storm damage to everyone outside `round`'s ring (start of a round).
    fn apply_storm(&mut self, round: u32) {
        let dmg = 14 + round as i32 * 3;
        let n = self.ents.len();
        for i in 0..n {
            if !self.ents[i].alive {
                continue;
            }
            let (gx, gy) = (self.ents[i].gx, self.ents[i].gy);
            if !self.in_ring(gx, gy, round) {
                self.ents[i].hp = (self.ents[i].hp - dmg).max(0);
                if self.ents[i].hp <= 0 && self.ents[i].alive {
                    self.ents[i].alive = false;
                    self.ents[i].death_round = round as i32;
                }
            }
        }
    }

    /// Both agents (0,1) have acted (or are dead) in this round → contract the
    /// storm into the next ring and let the NPCs shuffle. Resolves if the storm
    /// closes out the match.
    fn maybe_advance_round(&mut self) {
        if self.resolved {
            return;
        }
        let pending = |i: usize, s: &Self| s.ents[i].alive && !s.acted[i];
        if pending(0, self) || pending(1, self) {
            return; // someone still owes a move this round
        }
        if self.round + 1 >= ROUNDS {
            // storm has fully closed → final resolution
            self.try_resolve_final();
            return;
        }
        self.round += 1;
        self.acted = [false, false];
        self.apply_storm(self.round);
        self.npc_act();
        // a storm tick can decide the match (one survivor consumed)
        self.check_elim();
        if !self.resolved {
            // point to_move at the first agent still alive this round
            if self.ents[0].alive {
                self.to_move = 0;
            } else if self.ents[1].alive {
                self.to_move = 1;
            }
        }
    }

    /// If exactly one of A/B is dead, the survivor wins; if both dead, draw/closer.
    fn check_elim(&mut self) {
        if self.resolved {
            return;
        }
        let a = &self.ents[0];
        let b = &self.ents[1];
        if !a.alive && !b.alive {
            if a.death_round == b.death_round {
                self.winner_idx = None;
                self.win_reason = "double";
            } else {
                self.winner_idx = Some(if a.death_round > b.death_round { 0 } else { 1 });
                self.win_reason = "survive";
            }
            self.resolved = true;
        } else if !a.alive {
            self.winner_idx = Some(1);
            self.win_reason = "survive";
            self.resolved = true;
        } else if !b.alive {
            self.winner_idx = Some(0);
            self.win_reason = "survive";
            self.resolved = true;
        }
    }

    /// Who stands strongest RIGHT NOW: the higher `hp + gear·6`, the same measure the
    /// storm's final resolution uses. `None` on a dead level. A fallen survivor stands
    /// for nothing (both fallen ⇒ level ⇒ `None`).
    fn leader(&self) -> Option<usize> {
        let score = |i: usize| {
            if self.ents[i].alive {
                self.ents[i].score()
            } else {
                i32::MIN
            }
        };
        let (sa, sb) = (score(0), score(1));
        if sa == sb {
            None
        } else {
            Some(if sa > sb { 0 } else { 1 })
        }
    }

    /// Final resolution after the storm has fully closed (both A/B still alive):
    /// the higher (hp + gear) stands strongest.
    fn try_resolve_final(&mut self) {
        if self.resolved {
            return;
        }
        match self.leader() {
            None => {
                self.winner_idx = None;
                self.win_reason = "double";
            }
            Some(i) => {
                self.winner_idx = Some(i);
                self.win_reason = "standing";
            }
        }
        self.resolved = true;
    }

    fn try_resolve(&mut self) {
        if self.resolved {
            return;
        }
        if let Some(r) = self.resigned_by {
            self.winner_idx = Some(1 - r);
            self.win_reason = "resign";
            self.resolved = true;
            return;
        }
        self.check_elim();
    }

    fn status_str(&self) -> &'static str {
        if self.resigned_by.is_some() {
            "resigned"
        } else if self.resolved {
            self.win_reason
        } else {
            "playing"
        }
    }

    /// Live win odds for A from HP + gear (the betting market). Mirrors the POC's
    /// `snapOdds`.
    fn odds_a(&self) -> f64 {
        let score = |e: &Ent| -> f64 {
            if !e.alive {
                0.0001
            } else {
                e.hp as f64 + e.gear as f64 * 6.0 + 8.0
            }
        };
        let sa = score(&self.ents[0]);
        let sb = score(&self.ents[1]);
        let s = sa + sb;
        if s <= 0.0 {
            0.5
        } else {
            sa / s
        }
    }
}

impl Minigame for Stormfall {
    fn new(agents: &[AgentId], settings: &Value) -> Result<Self, MatchError> {
        if agents.len() != PLAYERS {
            return Err(MatchError::WrongPlayerCount {
                want: 2..=2,
                got: agents.len(),
            });
        }
        let seed = settings.get("seed").and_then(|v| v.as_u64()).unwrap_or(1);

        // The seeded final safe-zone eye (not the board center) — the isle's twist.
        let final_cx = 2.0 + (frac(seed, 1) * (GN as f64 - 4.0)).floor();
        let final_cy = 2.0 + (frac(seed, 2) * (GN as f64 - 4.0)).floor();

        // seeded crate layout
        let mut crates = Vec::new();
        for i in 0..9u32 {
            let gx = 1
                + (frac(seed.wrapping_mul(71).wrapping_add(13), i * 3 + 1) * (GN as f64 - 2.0))
                    as i32;
            let gy = 1
                + (frac(seed.wrapping_mul(71).wrapping_add(13), i * 3 + 2) * (GN as f64 - 2.0))
                    as i32;
            let gear = 2 + (frac(seed.wrapping_mul(71).wrapping_add(13), i * 3 + 3) * 3.0) as u32;
            crates.push(Crate {
                gx,
                gy,
                gear,
                taken: false,
            });
        }

        let bunkers = [(1, GN - 2), (GN - 2, 1), (GN - 2, GN - 2)];

        let mk = |id, gx, gy, hp, is_npc, name: &str, col| Ent {
            id,
            gx,
            gy,
            hp,
            maxhp: hp,
            gear: 0,
            alive: true,
            is_npc,
            name: name.to_string(),
            col,
            death_round: -1,
            last_move: None,
        };
        let mut ents = vec![
            mk("A", 1, 1, 100, false, "Vortex", "#10b981"),
            mk("B", GN - 2, GN - 2, 100, false, "Bunker", "#8b5cf6"),
        ];
        // a few seeded NPC husks (flavour, not the market)
        let spots = [(GN - 2, 1), (1, GN - 2), (GN / 2, 1)];
        let cols = ["#f59e0b", "#f43f5e", "#38bdf8"];
        let nm = ["Drone-7", "Husk", "Scrap"];
        for i in 0..3usize {
            let hp =
                55 + (frac(seed.wrapping_mul(311).wrapping_add(7), i as u32 + 1) * 25.0) as i32;
            ents.push(mk(
                ["N0", "N1", "N2"][i],
                spots[i].0,
                spots[i].1,
                hp,
                true,
                nm[i],
                cols[i],
            ));
        }

        Ok(Self {
            players: agents.to_vec(),
            ents,
            crates,
            bunkers,
            to_move: 0,
            acted: [false, false],
            round: 0,
            ply: 0,
            seed,
            final_cx,
            final_cy,
            resigned_by: None,
            winner_idx: None,
            win_reason: "playing",
            resolved: false,
        })
    }

    fn name(&self) -> &'static str {
        "stormfall"
    }

    fn instructions(&self) -> String {
        "AIWars Stormfall Isle referee. Two survivors fight on a shrinking-storm voxel island. \
         Read the state each turn: `players` (yours carries your handle) has your \
         HP/gear/position and whether you are `in_zone`, `center`+`ringR` are the safe ring right now, \
         `round` counts the storm's contractions, and `moves` lists your EXACT legal actions. \
         Play with make_move, mv = one of: \"loot:crate\" — sprint to the nearest crate and grab \
         gear (a stronger hunt), but you stand exposed; \"hide:bunker\" — turtle toward a bunker, \
         safe, patches a little HP; \"rotate:eye\" — two steps toward the safe-zone eye, dodging \
         the storm; \"hunt:rival\" — strike the rival, legal ONLY while they are reachable. Pass \
         expected_ply = the ply you saw. Each round the storm wall contracts toward the eye and \
         drains the HP of anyone outside the ring. Eliminate the rival (HP to 0) or be the last \
         one standing when the storm closes; if you both stand, the higher HP + gear wins, and a \
         double-KO on the same tick is a draw. resign forfeits the isle to your rival. Your seat \
         is your bearer token; you cannot act as your rival."
            .into()
    }

    /// Stormfall is **perfect information**, so `viewer` is deliberately ignored: survivor and
    /// spectator read the same projection. The isle's one "hidden" fact is the FINAL storm eye
    /// (`finalC`), and it is hidden in FICTION only:
    ///
    /// - the spectator SPA draws it (the drift trail from board-mid plus the crosshair on the
    ///   eye is the whole point of the view), so it belongs in the public projection; and
    /// - a survivor can derive it from what it must be able to see anyway — the visible ring
    ///   `center` drifts `lerp(mid, finalC, round/ROUNDS)`, so a single contraction pins the eye
    ///   exactly, and the layout is a pure function of the published `seed`.
    ///
    /// Redacting `finalC` from a survivor while publishing the seed and the drifting center
    /// would be decoration, not secrecy. Nothing else here is per-player private either: HP,
    /// gear, positions, crates and the NPC husks are all on the board — there is no per-seat
    /// secret one player could read off another. If a variant ever adds real fog (an unseen
    /// rival position, a face-down loot card), THIS is the hook: branch on `viewer` and keep
    /// the `None` (public) branch free of it.
    ///
    /// (The library injects the `"game"` key, so it is not set here.)
    fn observe(&self, _viewer: Option<&AgentId>) -> Value {
        let h = |i: usize| self.players[i].0.clone();
        let winner = self
            .winner_idx
            .filter(|_| self.resolved)
            .map(h)
            .map(Value::String)
            .unwrap_or(Value::Null);

        let (cx, cy) = self.center_at(self.round);
        let ent_json = |e: &Ent| {
            json!({
                "id": e.id,
                "gx": e.gx,
                "gy": e.gy,
                "hp": e.hp,
                "maxhp": e.maxhp,
                "gear": e.gear,
                "alive": e.alive,
                "isNPC": e.is_npc,
                "name": e.name,
                "col": e.col,
                "lastMove": e.last_move.clone().map(Value::String).unwrap_or(Value::Null),
            })
        };
        let player_json = |i: usize| {
            let e = &self.ents[i];
            json!({
                "handle": h(i),
                "id": e.id,
                "name": e.name,
                "hp": e.hp,
                "maxhp": e.maxhp,
                "gear": e.gear,
                "alive": e.alive,
                "gx": e.gx,
                "gy": e.gy,
                "in_zone": self.in_ring(e.gx, e.gy, self.round),
                "last_move": e.last_move.clone().map(Value::String).unwrap_or(Value::Null),
            })
        };

        json!({
            "grid": GN,
            "rounds": ROUNDS,
            "round": self.round,
            "seed": self.seed,
            "to_move": h(self.to_move),
            "to_move_idx": self.to_move,
            "ply": self.ply,
            "status": self.status_str(),
            "winner": winner,
            "win_reason": if self.resolved { self.win_reason } else { "" },
            "survivors": self.alive_count(),
            "odds_a": self.odds_a(),
            "center": { "cx": cx, "cy": cy },
            "ringR": Self::ring_r(self.round),
            "finalC": { "cx": self.final_cx, "cy": self.final_cy },
            "moves": self.legal_moves(),
            "players": [player_json(0), player_json(1)],
            "ents": self.ents.iter().map(ent_json).collect::<Vec<_>>(),
            "crates": self.crates.iter().map(|c| json!({
                "gx": c.gx, "gy": c.gy, "gear": c.gear, "taken": c.taken
            })).collect::<Vec<_>>(),
            "bunkers": self.bunkers.iter().map(|&(gx, gy)| json!({ "gx": gx, "gy": gy }))
                .collect::<Vec<_>>(),
        })
    }

    fn outcome(&self) -> Option<Outcome> {
        if !self.resolved {
            return None;
        }
        Some(match self.winner_idx {
            Some(i) => Outcome::Win(self.players[i].clone()),
            None => Outcome::Draw,
        })
    }

    /// The wall-clock timeout tiebreak: whoever stands strongest right now — the same
    /// `hp + gear·6` rule the closed storm resolves on. Dead level ⇒ `None` ⇒ a timeout draws.
    fn timeout_leader(&self) -> Option<AgentId> {
        self.leader().map(|i| self.players[i].clone())
    }
}

impl TurnBasedGame for Stormfall {
    fn turn_agent(&self) -> AgentId {
        self.players[self.to_move].clone()
    }

    fn ply(&self) -> u32 {
        self.ply
    }

    fn legal_moves(&self) -> Vec<String> {
        if self.resolved {
            return Vec::new();
        }
        self.legal_for(self.to_move)
    }

    fn apply(&mut self, agent: &AgentId, mv: &str) -> Result<(), MatchError> {
        if self.resolved {
            return Err(MatchError::GameOver);
        }
        let seat = self
            .seat_of(agent)
            .ok_or_else(|| MatchError::Rejected("not a survivor on this isle".into()))?;
        // Defensive: `TurnBasedMatch` already polices turn order (`TurnError::NotYourTurn`)
        // and the ply, but keep the game honest if it is ever driven directly.
        if self.to_move != seat {
            return Err(MatchError::Rejected("not your turn".into()));
        }
        let legal = self.legal_for(seat);
        if !legal.iter().any(|m| m == mv) {
            return Err(MatchError::Rejected(format!(
                "'{mv}' is not available here"
            )));
        }

        // --- committed, mutating path (validation has passed) ---
        let round = self.round;
        let op = 1 - seat;

        match mv {
            "loot:crate" => {
                let (gx, gy) = (self.ents[seat].gx, self.ents[seat].gy);
                if let Some(ci) = self.nearest_crate(gx, gy) {
                    let (tgx, tgy) = (self.crates[ci].gx, self.crates[ci].gy);
                    Self::step_toward(&mut self.ents[seat], tgx as f64, tgy as f64);
                    if self.ents[seat].gx == tgx && self.ents[seat].gy == tgy {
                        self.crates[ci].taken = true;
                        self.ents[seat].gear += self.crates[ci].gear;
                    }
                }
            }
            "hide:bunker" => {
                let (mut bb, mut bd) = ((0i32, 0i32), i32::MAX);
                for &(bx, by) in self.bunkers.iter() {
                    let d = (bx - self.ents[seat].gx).abs() + (by - self.ents[seat].gy).abs();
                    if d < bd {
                        bd = d;
                        bb = (bx, by);
                    }
                }
                Self::step_toward(&mut self.ents[seat], bb.0 as f64, bb.1 as f64);
                let mh = self.ents[seat].maxhp;
                self.ents[seat].hp = (self.ents[seat].hp + 4).min(mh);
            }
            "rotate:eye" => {
                // rotate toward the NEXT ring center (read the storm early)
                let (cx, cy) = self.center_at((round + 1).min(ROUNDS));
                Self::step_toward(&mut self.ents[seat], cx, cy);
                Self::step_toward(&mut self.ents[seat], cx, cy);
            }
            "hunt:rival" => {
                // damage scales with gear; a hidden seeded jitter per strike.
                let base = 16.0 + self.ents[seat].gear as f64 * 7.0;
                let j = frac(
                    self.seed
                        .wrapping_mul(97)
                        .wrapping_add(self.ply as u64 * 31),
                    if seat == 0 { 5 } else { 9 },
                );
                let dmg = (base * (0.8 + j * 0.5)).round() as i32;
                self.ents[op].hp = (self.ents[op].hp - dmg).max(0);
                if self.ents[op].hp <= 0 && self.ents[op].alive {
                    self.ents[op].alive = false;
                    self.ents[op].death_round = round as i32;
                }
            }
            // Unreachable: `legal_for` above is the only source of move strings.
            _ => return Err(MatchError::Rejected(format!("'{mv}' is not an action"))),
        }

        self.ents[seat].last_move = Some(mv.to_string());
        self.acted[seat] = true;
        self.ply += 1;

        // an elimination from this strike can end the match immediately.
        self.try_resolve();
        if self.resolved {
            return Ok(());
        }

        // advance the turn: prefer the other agent if it still owes a move.
        if self.ents[op].alive && !self.acted[op] {
            self.to_move = op;
        } else {
            // both have acted (or op is dead) → contract the storm into next round
            self.maybe_advance_round();
        }
        Ok(())
    }

    fn resign(&mut self, agent: &AgentId) {
        if self.resolved {
            return;
        }
        if let Some(seat) = self.seat_of(agent) {
            self.resigned_by = Some(seat);
            self.try_resolve();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwars_minigame::{RefereeMatch, TurnBasedMatch, TurnError};
    use serde_json::json;

    fn survivors() -> Vec<AgentId> {
        vec![AgentId("vortex".into()), AgentId("bunker".into())]
    }

    /// A started two-seat match on a fixed seed.
    fn started(seed: u64) -> TurnBasedMatch {
        let mut m =
            TurnBasedMatch::new::<Stormfall>(survivors(), &json!({ "seed": seed })).unwrap();
        m.start();
        m
    }

    #[test]
    fn rejects_wrong_player_count() {
        for n in [1usize, 3] {
            let ids: Vec<AgentId> = (0..n).map(|i| AgentId(format!("p{i}"))).collect();
            match Stormfall::new(&ids, &json!({})) {
                Err(MatchError::WrongPlayerCount { want, got }) => {
                    assert_eq!(want, 2..=2);
                    assert_eq!(got, n);
                }
                Err(e) => panic!("expected WrongPlayerCount for {n} players, got {e}"),
                Ok(_) => panic!("expected WrongPlayerCount for {n} players, got a built game"),
            }
        }
    }

    #[test]
    fn first_move_advances_ply_and_passes_turn() {
        let mut m = started(7);
        assert_eq!(m.state_json()["ply"], 0);
        assert_eq!(m.state_json()["to_move_idx"], 0);
        let legal = m.turn_info(0)["moves"].as_array().unwrap().len();
        assert!(legal >= 3, "at least loot/hide/rotate available");
        let st = m.make_move(0, "rotate:eye", 0).unwrap();
        assert_eq!(st["ply"], 1);
        assert_eq!(st["to_move_idx"], 1, "turn passes to the rival");
    }

    /// The library injects the `game` key the spectator SPA dispatches on — the game itself
    /// must not (and no longer does) set it.
    #[test]
    fn public_state_carries_the_library_injected_game_key() {
        let m = started(7);
        assert_eq!(m.state_json()["game"], "stormfall");
        assert!(
            !Stormfall::new(&survivors(), &json!({}))
                .unwrap()
                .observe(None)
                .as_object()
                .unwrap()
                .contains_key("game"),
            "the game must not set `game` itself — the library owns that key"
        );
    }

    /// The turn-order policing this port handed to the library: a move by the WRONG agent is
    /// refused with `TurnError::NotYourTurn`, and nothing changes.
    #[test]
    fn move_by_the_wrong_agent_is_refused() {
        let mut m = started(7);
        let before = m.state_json();
        assert_eq!(
            m.make_move(1, "rotate:eye", 0).unwrap_err(),
            TurnError::NotYourTurn
        );
        assert_eq!(
            m.state_json(),
            before,
            "no state change on an out-of-turn move"
        );
    }

    /// The game's OWN defensive check, driven directly (no match wrapper): an illegal mover is
    /// `Rejected` now that `MatchError::NotYourTurn` is gone.
    #[test]
    fn game_driven_directly_also_refuses_the_wrong_agent() {
        let mut g = Stormfall::new(&survivors(), &json!({ "seed": 7 })).unwrap();
        assert!(matches!(
            g.apply(&AgentId("bunker".into()), "rotate:eye"),
            Err(MatchError::Rejected(_))
        ));
        assert!(matches!(
            g.apply(&AgentId("nobody".into()), "rotate:eye"),
            Err(MatchError::Rejected(_))
        ));
        assert_eq!(g.ply(), 0);
    }

    #[test]
    fn illegal_move_rejected_without_change() {
        let mut m = started(7);
        let before = m.state_json();
        match m.make_move(0, "fly:away", 0).unwrap_err() {
            TurnError::Core(MatchError::Rejected(msg)) => assert!(msg.contains("fly:away")),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert_eq!(m.state_json(), before, "no state change on a rejected move");
    }

    #[test]
    fn stale_ply_rejected() {
        let mut m = started(7);
        assert_eq!(
            m.make_move(0, "rotate:eye", 9).unwrap_err(),
            TurnError::StalePly
        );
    }

    /// The rule that makes the isle a chase: you cannot strike a rival you have not closed on.
    #[test]
    fn hunt_is_illegal_until_the_rival_is_reachable() {
        let mut m = started(7);
        // Opposite corners at drop-in — 6 cells apart, well past the strike range of 2.
        assert!(
            !m.turn_info(0)["moves"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "hunt:rival"),
            "hunt must not be offered across the isle"
        );
        match m.make_move(0, "hunt:rival", 0).unwrap_err() {
            TurnError::Core(MatchError::Rejected(msg)) => assert!(msg.contains("hunt:rival")),
            other => panic!("expected Rejected, got {other:?}"),
        }
        // Walk them into each other and the action appears.
        let mut g = Stormfall::new(&survivors(), &json!({ "seed": 7 })).unwrap();
        g.ents[1].gx = 2;
        g.ents[1].gy = 2;
        assert!(g.legal_for(0).iter().any(|m| m == "hunt:rival"));
        assert!(
            g.apply(&AgentId("vortex".into()), "hunt:rival").is_ok(),
            "a reachable rival can be struck"
        );
        assert!(g.ents[1].hp < 100, "the strike lands damage");
    }

    /// The storm is the clock: once BOTH survivors have acted the ring contracts.
    #[test]
    fn the_ring_contracts_once_both_have_acted() {
        let mut m = started(7);
        assert_eq!(m.state_json()["round"], 0);
        m.make_move(0, "rotate:eye", 0).unwrap();
        assert_eq!(
            m.state_json()["round"],
            0,
            "one move does not move the storm"
        );
        let st = m.make_move(1, "rotate:eye", 1).unwrap();
        assert_eq!(st["round"], 1, "both acted → the storm contracts");
        assert!(
            st["ringR"].as_f64().unwrap() < RING_R[0],
            "the safe ring shrank"
        );
    }

    /// Drive a whole battle-royale the way a client does, preferring the strike so it
    /// resolves decisively. It must finish with a concrete result.
    #[test]
    fn a_full_game_resolves_to_winner_or_draw() {
        let mut m = started(7);
        let mut guard = 0;
        while !m.is_resolved() && guard < 256 {
            let seat = m.state_json()["to_move_idx"].as_u64().unwrap() as usize;
            let ply = m.state_json()["ply"].as_u64().unwrap() as u32;
            let moves = m.turn_info(seat)["moves"].as_array().unwrap().clone();
            let mv = moves
                .iter()
                .map(|v| v.as_str().unwrap())
                .find(|s| *s == "hunt:rival")
                .unwrap_or_else(|| moves[0].as_str().unwrap())
                .to_string();
            let _ = m.make_move(seat, &mv, ply);
            guard += 1;
        }
        assert!(m.is_resolved(), "match must resolve as the storm closes");
        let result = m.result().expect("resolved match has a result");
        assert!(result.outcome == "Winner" || result.outcome == "Draw");
        assert!(m.state_json()["moves"].as_array().unwrap().is_empty());
    }

    #[test]
    fn resign_awards_opponent() {
        let mut m = started(3);
        let st = m.resign(0);
        assert_eq!(st["status"], "resigned");
        assert!(m.is_resolved());
        let result = m.result().unwrap();
        assert_eq!(result.outcome, "Winner");
        assert_eq!(result.winner.as_deref(), Some("bunker"));
    }

    #[test]
    fn outcome_names_the_winner_by_identity() {
        let mut g = Stormfall::new(&survivors(), &json!({ "seed": 3 })).unwrap();
        assert_eq!(g.outcome(), None);
        g.resign(&AgentId("vortex".into()));
        assert_eq!(g.outcome(), Some(Outcome::Win(AgentId("bunker".into()))));
    }

    #[test]
    fn timeout_leader_is_whoever_stands_strongest() {
        let mut g = Stormfall::new(&survivors(), &json!({ "seed": 7 })).unwrap();
        assert_eq!(g.timeout_leader(), None, "dead level at the drop");
        // Close on the rival and land a strike — the wound puts Vortex ahead.
        g.ents[1].gx = 2;
        g.ents[1].gy = 2;
        g.apply(&AgentId("vortex".into()), "hunt:rival").unwrap();
        assert!(g.ents[1].hp < g.ents[0].hp);
        assert_eq!(g.timeout_leader(), Some(AgentId("vortex".into())));
    }

    #[test]
    fn same_seed_same_layout() {
        let a = started(42);
        let b = started(42);
        assert_eq!(a.state_json()["finalC"], b.state_json()["finalC"]);
        assert_eq!(a.state_json()["crates"], b.state_json()["crates"]);
        assert_eq!(a.state_json()["moves"], b.state_json()["moves"]);
    }

    /// Stormfall is perfect information: the private projection a survivor's console reads is
    /// byte-identical to the public one the spectator gets. If a variant ever adds real fog,
    /// this test is the tripwire that says "project per viewer now".
    #[test]
    fn every_viewer_reads_the_same_isle() {
        let g = Stormfall::new(&survivors(), &json!({ "seed": 7 })).unwrap();
        let public = g.observe(None);
        assert_eq!(g.observe(Some(&AgentId("vortex".into()))), public);
        assert_eq!(g.observe(Some(&AgentId("bunker".into()))), public);
        // ...and the eye the spectator SPA draws is in that one projection.
        assert!(public["finalC"]["cx"].is_number());
    }

    /// The whole point of the port: the seat payload a HUMAN's browser reads carries its turn
    /// info (`your_turn` + `moves`) alongside the private projection.
    #[test]
    fn seat_state_carries_turn_info_for_a_human_console() {
        let m = started(7);
        let s = m.seat_state(0);
        assert_eq!(s["you"]["handle"], "vortex");
        assert_eq!(s["turn"]["your_turn"], true);
        assert!(s["turn"]["moves"].as_array().unwrap().len() >= 3);
        assert_eq!(s["state"]["to_move"], "vortex");
        assert_eq!(m.seat_state(1)["turn"]["your_turn"], false);
    }
}
