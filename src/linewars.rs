//! Line Wars — a Graphwar-style artillery **free-for-all** for AIWars (tier-1 turn-based).
//!
//! 2–30 players share one Cartesian battlefield, each commanding a squad of soldiers scattered
//! across the field. On your turn your *active soldier* fires by graphing a function `f(x)` you
//! submit: the curve is translated vertically so it passes through that soldier, then the shot
//! LEAVES the soldier and flies in ONE direction along the graph (toward the nearest enemy by
//! default; prefix `>`/`<` to aim right/left). It travels until it
//!
//!   * hits a soldier (anyone's — including your own: friendly fire is real) → kill,
//!   * hits an obstacle                                                      → blocked,
//!   * leaves the field (x or y range)                                       → it flew off, or
//!   * "explodes" (the function goes to NaN/∞, e.g. `sqrt(x)` over negative x).
//!
//! Last player with a soldier standing wins. Soldiers fire in rotation, so the state always tells
//! you which of yours is up and where it is.
//!
//! Identities are [`AgentId`]s (never seat indices). `players[i]` is the i-th squad; play order is
//! randomized at construction.

use aiwars_minigame::{AgentId, MatchError, Minigame, Outcome, TurnBasedGame};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Value};

use crate::expr::Expr;

// ---- battlefield constants -------------------------------------------------

/// How close the curve must pass to a soldier's center to destroy it (world units, fixed).
const HIT_RADIUS: f64 = 0.7;
/// Keep every Nth traced point for the spectator polyline (collisions still use every step).
const SAMPLE_EVERY: usize = 6;
/// Integration steps per direction — the trace STEP scales as `width / TRACE_STEPS`, so cost
/// stays bounded no matter how big the field grows.
const TRACE_STEPS: f64 = 2500.0;
/// Field is `2*half_x` wide × `2*half_y` tall, with `half_y = FIELD_RATIO * half_x` (a 5:3 box).
const FIELD_RATIO: f64 = 0.6;

const MAX_PLAYERS: usize = 30;

/// The match's rectangular battlefield (grows with the lobby — see [`field_for`]).
#[derive(Clone, Copy, Debug)]
struct Bounds {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

impl Bounds {
    fn width(&self) -> f64 {
        self.x_max - self.x_min
    }
}

/// Field bounds for an `n`-player lobby: a 2-player duel is the classic 50×30 box; it grows with
/// the player count (up to 150×90) so big free-for-alls have room to spread out and explore.
fn field_for(n: usize) -> Bounds {
    let half_x = (20.0 + 2.5 * n as f64).clamp(25.0, 75.0);
    let half_y = half_x * FIELD_RATIO;
    Bounds {
        x_min: -half_x,
        x_max: half_x,
        y_min: -half_y,
        y_max: half_y,
    }
}

// ---- data ------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Soldier {
    x: f64,
    y: f64,
    alive: bool,
}

#[derive(Clone, Debug)]
struct Obstacle {
    x: f64,
    y: f64,
    r: f64,
}

/// What a shot ended up doing — drives the spectator label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ShotOutcome {
    OffField,
    Blocked,
    Exploded,
    FriendlyFire,
    HitEnemy,
}

impl ShotOutcome {
    fn as_str(self) -> &'static str {
        match self {
            ShotOutcome::HitEnemy => "hit_enemy",
            ShotOutcome::FriendlyFire => "friendly_fire",
            ShotOutcome::Blocked => "blocked",
            ShotOutcome::OffField => "off_field",
            ShotOutcome::Exploded => "exploded",
        }
    }
}

/// How one direction of a shot ended (used only to compute the overall label).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndKind {
    Blocked,
    OffField,
    Exploded,
}

/// A recorded shot — enough for the view to animate the curve and show the result.
#[derive(Clone, Debug)]
struct Shot {
    by_player: usize,
    by_handle: String,
    func: String,
    from: (f64, f64),
    /// Ordered polyline starting at the muzzle (`muzzle_index = 0`) and flying one way.
    points: Vec<(f64, f64)>,
    muzzle_index: usize,
    outcome: ShotOutcome,
    kills: Vec<(usize, usize)>, // (player, soldier index) destroyed by this shot
}

/// An N-player Line Wars match (free-for-all).
pub struct LineWars {
    /// Squads in (randomized) play order; `players[i]` is the i-th squad's identity.
    players: Vec<AgentId>,
    /// `soldiers[p]` — player p's squad (dead soldiers stay with `alive=false`).
    soldiers: Vec<Vec<Soldier>>,
    /// Players who forfeited (their soldiers are all dead); for display.
    resigned: Vec<bool>,
    /// The battlefield extent (scales with the lobby size).
    bounds: Bounds,
    /// Trajectory integration step in x (= `bounds.width() / TRACE_STEPS`).
    step: f64,
    obstacles: Vec<Obstacle>,
    /// Whose turn (index into `players`).
    turn: usize,
    /// Per-player rotation pointer to the soldier that fires next; kept on an alive soldier for
    /// the player to move.
    shooter: Vec<usize>,
    ply: u32,
    last_shot: Option<Shot>,
}

impl LineWars {
    fn player_of(&self, agent: &AgentId) -> Option<usize> {
        self.players.iter().position(|p| p == agent)
    }

    fn alive_count(&self, p: usize) -> usize {
        self.soldiers[p].iter().filter(|s| s.alive).count()
    }

    fn squads_standing(&self) -> usize {
        (0..self.players.len())
            .filter(|&p| self.alive_count(p) > 0)
            .count()
    }

    /// The next alive soldier index for player `p` at or after `from` (wrapping), or `None`.
    fn next_alive(&self, p: usize, from: usize) -> Option<usize> {
        let n = self.soldiers[p].len();
        if n == 0 {
            return None;
        }
        (0..n)
            .map(|k| (from + k) % n)
            .find(|&i| self.soldiers[p][i].alive)
    }

    /// The next player (at or after `from`, wrapping) that still has a soldier, or `None`.
    fn next_player_alive(&self, from: usize) -> Option<usize> {
        let n = self.players.len();
        (0..n)
            .map(|k| (from + k) % n)
            .find(|&p| self.alive_count(p) > 0)
    }

    /// Keep the player-to-move's shooter pointer on an alive soldier (no-op if they're wiped).
    fn normalize_shooter(&mut self) {
        if let Some(i) = self.next_alive(self.turn, self.shooter[self.turn]) {
            self.shooter[self.turn] = i;
        }
    }

    /// Trace `expr` fired by `soldiers[p][idx]` in direction `dir` (±1). Pure — returns the
    /// [`Shot`], a polyline from the muzzle outward that stops at the first soldier/obstacle/edge.
    fn trace(&self, p: usize, idx: usize, func: &str, expr: &Expr, dir: f64) -> Shot {
        let s = &self.soldiers[p][idx];
        let (xs, ys) = (s.x, s.y);

        let mut shot = Shot {
            by_player: p,
            by_handle: self.players[p].0.clone(),
            func: func.to_string(),
            from: (round3(xs), round3(ys)),
            // The polyline STARTS at the muzzle (the firing soldier) and goes one way.
            points: vec![(round3(xs), round3(ys))],
            muzzle_index: 0,
            outcome: ShotOutcome::OffField,
            kills: Vec::new(),
        };

        let f0 = expr.eval(xs);
        if !f0.is_finite() {
            shot.outcome = ShotOutcome::Exploded; // explodes right at the muzzle
            return shot;
        }

        // The shot leaves the soldier and flies in ONE direction (Graphwar-style).
        let muzzle = (xs, ys, ys - f0);
        let mut path = Vec::new();
        let (kill, end) = self.trace_dir((p, idx), muzzle, expr, dir.signum(), &mut path);
        shot.points.extend(path);

        shot.outcome = if let Some((kp, ki)) = kill {
            shot.kills.push((kp, ki));
            if kp == p {
                ShotOutcome::FriendlyFire
            } else {
                ShotOutcome::HitEnemy
            }
        } else {
            match end {
                EndKind::Blocked => ShotOutcome::Blocked,
                EndKind::Exploded => ShotOutcome::Exploded,
                EndKind::OffField => ShotOutcome::OffField,
            }
        };
        shot
    }

    /// Which way this shot flies: an explicit `>`/`<` (right/left) override the agent gave, else
    /// toward the nearest enemy soldier (so a bare function "fires forward" at an opponent).
    fn shot_dir(&self, p: usize, idx: usize, explicit: Option<f64>) -> f64 {
        if let Some(d) = explicit {
            return d;
        }
        let (xs, ys) = (self.soldiers[p][idx].x, self.soldiers[p][idx].y);
        let mut best: Option<(f64, f64)> = None; // (dist2, enemy_x)
        for (q, squad) in self.soldiers.iter().enumerate() {
            if q == p {
                continue;
            }
            for sol in squad.iter().filter(|s| s.alive) {
                let d2 = (sol.x - xs).powi(2) + (sol.y - ys).powi(2);
                if best.map(|(bd, _)| d2 < bd).unwrap_or(true) {
                    best = Some((d2, sol.x));
                }
            }
        }
        match best {
            Some((_, ex)) if ex < xs => -1.0,
            _ => 1.0, // toward the nearest enemy (or default right if somehow none)
        }
    }

    /// Trace one direction (`dir` = ±1) from the muzzle, collecting sampled points into `out`.
    /// Returns the first soldier hit (if any) and how that side ended.
    fn trace_dir(
        &self,
        shooter: (usize, usize),
        muzzle: (f64, f64, f64),
        expr: &Expr,
        dir: f64,
        out: &mut Vec<(f64, f64)>,
    ) -> (Option<(usize, usize)>, EndKind) {
        let (xs, ys, c) = muzzle;
        let mut prev = (xs, ys);
        let mut x = xs;
        let mut step_n: usize = 0;
        loop {
            x += dir * self.step;
            if !(self.bounds.x_min..=self.bounds.x_max).contains(&x) {
                return (None, EndKind::OffField);
            }
            let y = expr.eval(x) + c;
            if !y.is_finite() {
                return (None, EndKind::Exploded);
            }
            if !(self.bounds.y_min..=self.bounds.y_max).contains(&y) {
                return (None, EndKind::OffField);
            }
            let cur = (x, y);

            if self
                .obstacles
                .iter()
                .any(|o| dist_point_seg(o.x, o.y, prev.0, prev.1, cur.0, cur.1) <= o.r)
            {
                out.push((round3(cur.0), round3(cur.1)));
                return (None, EndKind::Blocked);
            }
            if let Some(hit) = self.first_soldier_hit(shooter.0, shooter.1, prev, cur) {
                out.push((round3(cur.0), round3(cur.1)));
                return (Some(hit), EndKind::OffField);
            }

            step_n += 1;
            if step_n.is_multiple_of(SAMPLE_EVERY) {
                out.push((round3(cur.0), round3(cur.1)));
            }
            prev = cur;
        }
    }

    /// The nearest alive soldier (any player, excluding the shooter itself) within [`HIT_RADIUS`]
    /// of the segment prev→cur.
    fn first_soldier_hit(
        &self,
        sp: usize,
        si: usize,
        prev: (f64, f64),
        cur: (f64, f64),
    ) -> Option<(usize, usize)> {
        let mut best: Option<((usize, usize), f64)> = None;
        for (p, squad) in self.soldiers.iter().enumerate() {
            for (i, sol) in squad.iter().enumerate() {
                if !sol.alive || (p == sp && i == si) {
                    continue;
                }
                let d = dist_point_seg(sol.x, sol.y, prev.0, prev.1, cur.0, cur.1);
                if d <= HIT_RADIUS && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some(((p, i), d));
                }
            }
        }
        best.map(|(hit, _)| hit)
    }

    fn status_str(&self) -> &'static str {
        match self.outcome() {
            None => "playing",
            Some(Outcome::Win(_)) => "win",
            Some(Outcome::Draw) => "draw",
            Some(_) => "resolved", // Outcome is #[non_exhaustive]
        }
    }
}

impl Minigame for LineWars {
    fn new(agents: &[AgentId], settings: &Value) -> Result<Self, MatchError> {
        let n = agents.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(MatchError::WrongPlayerCount {
                want: 2..=MAX_PLAYERS,
                got: n,
            });
        }
        // Squad size: shrinks as the lobby grows so the field doesn't drown in soldiers.
        let default_soldiers = if n <= 10 {
            3
        } else if n <= 20 {
            2
        } else {
            1
        };
        let soldiers_per = settings
            .get("soldiers")
            .and_then(|v| v.as_u64())
            .map(|s| s.clamp(1, 6) as usize)
            .unwrap_or(default_soldiers);
        // A nice dense field of cover — scales up with the (bigger) field for more players.
        let default_obstacles = (9 + n).clamp(10, 28);
        let n_obstacles = settings
            .get("obstacles")
            .and_then(|v| v.as_u64())
            .map(|o| o.clamp(0, 40) as usize)
            .unwrap_or(default_obstacles);
        let seed = settings
            .get("seed")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(rand::random::<u64>);
        let mut rng = StdRng::seed_from_u64(seed);

        // The battlefield scales with the lobby size.
        let bounds = field_for(n);
        let step = bounds.width() / TRACE_STEPS;

        // Randomized play order (fairness / anti-positional-bias).
        let mut players: Vec<AgentId> = agents.to_vec();
        shuffle(&mut players, &mut rng);

        let obstacles = gen_obstacles(&mut rng, n_obstacles, &bounds);

        // Scatter every soldier across the field, then deal them out round-robin to squads.
        let total = n * soldiers_per;
        let positions = gen_positions(&mut rng, total, &obstacles, &bounds);
        let mut soldiers: Vec<Vec<Soldier>> = vec![Vec::with_capacity(soldiers_per); n];
        for (k, (x, y)) in positions.into_iter().take(total).enumerate() {
            soldiers[k % n].push(Soldier { x, y, alive: true });
        }

        Ok(Self {
            players,
            soldiers,
            resigned: vec![false; n],
            bounds,
            step,
            obstacles,
            turn: 0,
            shooter: vec![0; n],
            ply: 0,
            last_shot: None,
        })
    }

    fn name(&self) -> &'static str {
        "line-wars"
    }

    fn instructions(&self) -> String {
        "AIWars Line Wars — a Graphwar-style artillery free-for-all (2–30 players). Call get_state \
         to see the battlefield: the field bounds, every player's squad (soldiers with x,y,alive), \
         the obstacles, whose turn it is, and `shooter` — YOUR active soldier this turn (your \
         soldiers fire in rotation). A move is a math function of x; call make_move with mv = the \
         function, e.g. \"(x^2)/40\", \"3*sin(x/2)\", or \"y = -x/2\", and expected_ply = the ply you \
         observed. The curve is translated vertically so it passes through your active soldier, then \
         the shot LEAVES the soldier and flies in ONE direction along the graph — toward the nearest \
         enemy by default. To aim the other way, prefix the move with a direction: \">\" (or \
         \"right:\") fires toward +x, \"<\" (or \"left:\") toward -x, e.g. \"< -x/2\". The shot travels \
         until it hits a soldier (anyone's — friendly fire kills your own too!), an obstacle, or \
         leaves the field; a shot that goes to NaN/∞ explodes harmlessly. Be the last player with a \
         soldier standing to win. The field GROWS with the lobby, so always read the \
         actual bounds from state.field (x_min/x_max/y_min/y_max) and scale your functions to fit — \
         e.g. a parabola through the field is roughly (x^2)/x_max. Supported: + - * / % ^ (and **), \
         unary -, parentheses, implicit multiplication (2x), constants pi/tau/e, and functions \
         sin cos tan asin acos atan sinh cosh tanh sqrt cbrt abs exp ln log log2 floor ceil round \
         sign. resign to forfeit. Your seat is your bearer token; you cannot act as another player."
            .into()
    }

    // Perfect-information: the board is identical for everyone. The ONLY difference is the
    // `last_shot` projection — the spectator (viewer = None) gets the full traced polyline to
    // animate the curve; an agent (viewer = Some) gets a compact summary (func/outcome/kills +
    // impact point) WITHOUT the hundreds of polyline points, so its get_state stays small and
    // readable (the points are pure rendering data, redundant to the function it already has).
    fn observe(&self, viewer: Option<&AgentId>) -> Value {
        let lean = viewer.is_some();
        let winner = match self.outcome() {
            Some(Outcome::Win(AgentId(h))) => Value::String(h),
            _ => Value::Null,
        };

        let players: Vec<Value> = self
            .players
            .iter()
            .enumerate()
            .map(|(p, id)| {
                json!({
                    "index": p,
                    "handle": id.0,
                    "alive": self.alive_count(p),
                    "total": self.soldiers[p].len(),
                    "resigned": self.resigned[p],
                    "soldiers": self.soldiers[p].iter().enumerate().map(|(i, s)| json!({
                        "index": i, "x": round3(s.x), "y": round3(s.y), "alive": s.alive
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        // Current leader = unique max alive count (for the timeout tiebreak + display).
        let leader = self.leader_index().map(|p| self.players[p].0.clone());

        let resolved = self.outcome().is_some();
        let shooter = if resolved {
            Value::Null
        } else {
            let s = &self.soldiers[self.turn][self.shooter[self.turn]];
            json!({
                "player": self.turn,
                "handle": self.players[self.turn].0,
                "index": self.shooter[self.turn],
                "x": round3(s.x),
                "y": round3(s.y),
            })
        };

        json!({
            "game": "line-wars",
            "field": {
                "x_min": round3(self.bounds.x_min), "x_max": round3(self.bounds.x_max),
                "y_min": round3(self.bounds.y_min), "y_max": round3(self.bounds.y_max)
            },
            "player_count": self.players.len(),
            "players": players,
            "turn": self.turn,
            "turn_handle": self.players[self.turn].0,
            "ply": self.ply,
            "obstacles": self.obstacles.iter().map(|o| json!({
                "x": round3(o.x), "y": round3(o.y), "r": round3(o.r)
            })).collect::<Vec<_>>(),
            "shooter": shooter,
            "squads_standing": self.squads_standing(),
            "status": self.status_str(),
            "winner": winner,
            "leader": leader.map(Value::String).unwrap_or(Value::Null),
            "last_shot": self.last_shot.as_ref().map(|s| shot_json(s, lean)).unwrap_or(Value::Null),
        })
    }

    fn outcome(&self) -> Option<Outcome> {
        match self.squads_standing() {
            0 => Some(Outcome::Draw),
            1 => {
                let p = self.next_player_alive(0).expect("one squad standing");
                Some(Outcome::Win(self.players[p].clone()))
            }
            _ => None,
        }
    }

    fn timeout_leader(&self) -> Option<AgentId> {
        self.leader_index().map(|p| self.players[p].clone())
    }
}

impl TurnBasedGame for LineWars {
    fn turn_agent(&self) -> AgentId {
        self.players[self.turn].clone()
    }

    fn ply(&self) -> u32 {
        self.ply
    }

    // The move space is the set of all functions — not enumerable. Per the trait contract, an
    // empty list means "submit a move and I'll validate it".
    fn legal_moves(&self) -> Vec<String> {
        Vec::new()
    }

    fn apply(&mut self, agent: &AgentId, mv: &str) -> Result<(), MatchError> {
        if self.outcome().is_some() {
            return Err(MatchError::GameOver);
        }
        let p = self
            .player_of(agent)
            .ok_or_else(|| MatchError::Rejected("not a player in this match".into()))?;
        if p != self.turn {
            return Err(MatchError::Rejected("not your turn".into()));
        }
        // An optional leading `>`/`<` (or `right:`/`left:`) aims the shot; the rest is the function.
        let (explicit_dir, func_str) = split_direction(mv);
        let expr = Expr::parse(func_str)
            .map_err(|e| MatchError::Rejected(format!("bad function: {e}")))?;
        let idx = self.shooter[p];
        let dir = self.shot_dir(p, idx, explicit_dir);

        let shot = self.trace(p, idx, mv.trim(), &expr, dir);
        for &(kp, ki) in &shot.kills {
            self.soldiers[kp][ki].alive = false;
        }
        self.last_shot = Some(shot);

        // Advance this player's rotation to their next alive soldier for their NEXT turn.
        if let Some(next) = self.next_alive(p, (idx + 1) % self.soldiers[p].len()) {
            self.shooter[p] = next;
        }
        self.ply += 1;

        // Pass the turn to the next player that still has a soldier.
        if let Some(next) = self.next_player_alive((p + 1) % self.players.len()) {
            self.turn = next;
        }
        self.normalize_shooter();
        Ok(())
    }

    fn resign(&mut self, agent: &AgentId) {
        if self.outcome().is_some() {
            return;
        }
        if let Some(p) = self.player_of(agent) {
            self.resigned[p] = true;
            for s in &mut self.soldiers[p] {
                s.alive = false; // forfeiting wipes your squad
            }
            // If the quitter was on the clock, hand the turn to the next survivor.
            if p == self.turn {
                if let Some(next) = self.next_player_alive((p + 1) % self.players.len()) {
                    self.turn = next;
                    self.normalize_shooter();
                }
            }
        }
    }
}

// ---- helpers ---------------------------------------------------------------

impl LineWars {
    /// The unique player with the most alive soldiers, or `None` on a tie for the lead.
    fn leader_index(&self) -> Option<usize> {
        let counts: Vec<usize> = (0..self.players.len())
            .map(|p| self.alive_count(p))
            .collect();
        let max = *counts.iter().max()?;
        if max == 0 {
            return None;
        }
        let leaders: Vec<usize> = counts
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c == max)
            .map(|(p, _)| p)
            .collect();
        if leaders.len() == 1 {
            Some(leaders[0])
        } else {
            None
        }
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Split an optional leading aim direction off a move. Accepts `>`/`<` or `right:`/`r:` /
/// `left:`/`l:` (case-insensitive); returns `(+1/-1 if given, the remaining function text)`.
fn split_direction(mv: &str) -> (Option<f64>, &str) {
    let t = mv.trim_start();
    if let Some(r) = t.strip_prefix('>') {
        return (Some(1.0), r);
    }
    if let Some(r) = t.strip_prefix('<') {
        return (Some(-1.0), r);
    }
    let lower = t.to_ascii_lowercase();
    for (pfx, d) in [("right:", 1.0), ("left:", -1.0), ("r:", 1.0), ("l:", -1.0)] {
        if lower.starts_with(pfx) {
            return (Some(d), &t[pfx.len()..]);
        }
    }
    (None, t)
}

/// `lean = true` → the agent projection: the useful summary (func/outcome/kills) plus where the
/// shot ENDED (`impact`), but NOT the heavy polyline. `lean = false` → the spectator projection:
/// the full `points` polyline so the canvas can animate the curve.
fn shot_json(s: &Shot, lean: bool) -> Value {
    let kills = s
        .kills
        .iter()
        .map(|(p, i)| json!({ "player": p, "index": i }))
        .collect::<Vec<_>>();
    if lean {
        // The last traced point is where the shot stopped (a wall/obstacle/soldier) — useful
        // feedback for the agent ("my shot ended at (3.7,-3.9)"), the polyline is not.
        let impact = s
            .points
            .last()
            .map(|(x, y)| json!([x, y]))
            .unwrap_or(Value::Null);
        json!({
            "by_player": s.by_player,
            "by_handle": s.by_handle,
            "func": s.func,
            "from": [s.from.0, s.from.1],
            "impact": impact,
            "outcome": s.outcome.as_str(),
            "kills": kills,
        })
    } else {
        json!({
            "by_player": s.by_player,
            "by_handle": s.by_handle,
            "func": s.func,
            "from": [s.from.0, s.from.1],
            "points": s.points.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>(),
            "muzzle_index": s.muzzle_index,
            "outcome": s.outcome.as_str(),
            "kills": kills,
        })
    }
}

/// Distance from point (px,py) to segment (ax,ay)-(bx,by).
fn dist_point_seg(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f64::EPSILON {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Fisher–Yates shuffle with the seeded rng.
fn shuffle<T>(v: &mut [T], rng: &mut StdRng) {
    for i in (1..v.len()).rev() {
        let j = rng.random_range(0..=i);
        v.swap(i, j);
    }
}

fn gen_obstacles(rng: &mut StdRng, n: usize, b: &Bounds) -> Vec<Obstacle> {
    // Keep rocks inside the central ~78% of the field so they don't crowd the spawn fringes.
    let (rx, ry) = (b.x_max * 0.82, b.y_max * 0.82);
    let mut obs: Vec<Obstacle> = Vec::new();
    let mut attempts = 0;
    while obs.len() < n && attempts < n * 120 {
        attempts += 1;
        let r = rng.random_range(1.1..2.6);
        let o = Obstacle {
            x: rng.random_range(-rx..rx),
            y: rng.random_range(-ry..ry),
            r,
        };
        if obs
            .iter()
            .all(|p| ((p.x - o.x).powi(2) + (p.y - o.y).powi(2)).sqrt() > p.r + o.r + 0.9)
        {
            obs.push(o);
        }
    }
    obs
}

/// Scatter `total` soldier positions across the field: a jittered grid (guarantees separation and
/// no starvation), obstacle-filtered and shuffled, with a random top-up if the grid came up short.
fn gen_positions(
    rng: &mut StdRng,
    total: usize,
    obstacles: &[Obstacle],
    b: &Bounds,
) -> Vec<(f64, f64)> {
    // Spawn inside a 2-unit margin from the walls.
    let (x0, x1, y0, y1) = (b.x_min + 2.0, b.x_max - 2.0, b.y_min + 2.0, b.y_max - 2.0);
    let (fw, fh) = (x1 - x0, y1 - y0);
    // Choose a grid a bit larger than needed so obstacle drops still leave enough cells.
    let want = (total as f64 * 1.6).ceil().max(1.0);
    let cols = ((want * (fw / fh)).sqrt().ceil() as usize).max(1);
    let rows = ((want / cols as f64).ceil() as usize).max(1);
    let (cw, ch) = (fw / cols as f64, fh / rows as f64);

    let mut pts: Vec<(f64, f64)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let cx = x0 + (c as f64 + 0.5) * cw + rng.random_range(-0.35..0.35) * cw;
            let cy = y0 + (r as f64 + 0.5) * ch + rng.random_range(-0.35..0.35) * ch;
            if clears_obstacles(cx, cy, obstacles) {
                pts.push((round3(cx), round3(cy)));
            }
        }
    }
    shuffle(&mut pts, rng);

    // Top-up (best effort) if obstacle drops left us short.
    let mut attempts = 0;
    while pts.len() < total && attempts < total * 80 {
        attempts += 1;
        let x = rng.random_range(x0..x1);
        let y = rng.random_range(y0..y1);
        if clears_obstacles(x, y, obstacles)
            && pts
                .iter()
                .all(|&(px, py)| ((px - x).powi(2) + (py - y).powi(2)).sqrt() > 1.6)
        {
            pts.push((round3(x), round3(y)));
        }
    }
    pts
}

fn clears_obstacles(x: f64, y: f64, obstacles: &[Obstacle]) -> bool {
    obstacles
        .iter()
        .all(|o| ((o.x - x).powi(2) + (o.y - y).powi(2)).sqrt() > o.r + 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwars_minigame::{RefereeMatch, TurnBasedMatch};

    use std::cell::RefCell;
    thread_local! {
        // Records construction (seat) order so `seat_of` can map a handle → its seat index.
        // The public state lists players in randomized PLAY order, but seats (what make_move
        // takes) follow the order agents were passed to the constructor.
        static ORDER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn ids(names: &[&str]) -> Vec<AgentId> {
        names.iter().map(|n| AgentId((*n).into())).collect()
    }

    fn build(agents: Vec<AgentId>, settings: Value) -> TurnBasedMatch {
        ORDER.with(|o| *o.borrow_mut() = agents.iter().map(|a| a.0.clone()).collect());
        TurnBasedMatch::new::<LineWars>(agents, &settings).unwrap()
    }

    fn seat_of(handle: &str) -> usize {
        ORDER.with(|o| o.borrow().iter().position(|h| h == handle).unwrap())
    }

    #[test]
    fn rejects_too_few_or_too_many_players() {
        assert!(matches!(
            TurnBasedMatch::new::<LineWars>(ids(&["solo"]), &json!({})),
            Err(MatchError::WrongPlayerCount { got: 1, .. })
        ));
        let many: Vec<AgentId> = (0..31).map(|i| AgentId(format!("p{i}"))).collect();
        assert!(matches!(
            TurnBasedMatch::new::<LineWars>(many, &json!({})),
            Err(MatchError::WrongPlayerCount { got: 31, .. })
        ));
    }

    #[test]
    fn accepts_up_to_30_players_and_deals_squads() {
        let agents: Vec<AgentId> = (0..30).map(|i| AgentId(format!("p{i}"))).collect();
        let lw = LineWars::new(&agents, &json!({ "seed": 1 })).unwrap();
        assert_eq!(lw.players.len(), 30);
        // 30 players → 1 soldier each by default, everyone placed.
        assert!(lw.soldiers.iter().all(|s| s.len() == 1));
        assert_eq!(lw.squads_standing(), 30);
    }

    #[test]
    fn seed_is_deterministic() {
        let a = build(ids(&["alice", "bob", "carol"]), json!({ "seed": 42 }));
        let b = build(ids(&["alice", "bob", "carol"]), json!({ "seed": 42 }));
        assert_eq!(a.state_json()["players"], b.state_json()["players"]);
        assert_eq!(a.state_json()["obstacles"], b.state_json()["obstacles"]);
    }

    #[test]
    fn a_legal_function_advances_ply_and_passes_turn() {
        let mut m = build(ids(&["alice", "bob", "carol"]), json!({ "seed": 7 }));
        m.start();
        let turn0 = m.state_json()["turn"].as_u64().unwrap() as usize;
        let mover = m.state_json()["turn_handle"].as_str().unwrap().to_string();
        let seat = seat_of(&mover);
        let st = m.make_move(seat, "0", 0).unwrap();
        assert_eq!(st["ply"], 1);
        assert_ne!(st["turn"].as_u64().unwrap() as usize, turn0); // turn passed on
        assert!(st["last_shot"].is_object());
    }

    #[test]
    fn unparseable_function_is_rejected_without_advancing() {
        let mut m = build(ids(&["alice", "bob"]), json!({ "seed": 7 }));
        m.start();
        let mover = m.state_json()["turn_handle"].as_str().unwrap().to_string();
        let seat = seat_of(&mover);
        assert!(m.make_move(seat, "2 +* x", 0).is_err());
        assert_eq!(m.state_json()["ply"], 0);
    }

    #[test]
    fn wrong_turn_is_rejected() {
        let mut m = build(ids(&["alice", "bob"]), json!({ "seed": 7 }));
        m.start();
        let mover = m.state_json()["turn_handle"].as_str().unwrap().to_string();
        let other = if mover == "alice" { "bob" } else { "alice" };
        let seat = seat_of(other);
        assert!(m.make_move(seat, "x", 0).is_err());
    }

    #[test]
    fn direct_hit_eliminates_and_wins_1v1() {
        // 1 soldier each, no obstacles → aim a straight line through the opponent. Translation
        // removes the constant term, so the line is just slope*x. The shot auto-fires toward the
        // nearest enemy (the only one), so no explicit direction is needed.
        let mut lw = LineWars::new(
            &ids(&["alice", "bob"]),
            &json!({ "seed": 3, "soldiers": 1, "obstacles": 0 }),
        )
        .unwrap();
        let me = lw.players[lw.turn].clone();
        let mine = &lw.soldiers[lw.turn][0];
        let other = 1 - lw.turn;
        let foe = &lw.soldiers[other][0];
        let slope = (foe.y - mine.y) / (foe.x - mine.x);
        lw.apply(&me, &format!("{slope}*x")).unwrap();
        assert!(
            !lw.soldiers[other][0].alive,
            "the opponent should be destroyed"
        );
        match lw.outcome() {
            Some(Outcome::Win(w)) => assert_eq!(w, me),
            o => panic!("expected a win, got {o:?}"),
        }
    }

    #[test]
    fn shot_is_one_way_and_starts_at_the_muzzle() {
        // A flat shot's polyline must begin AT the firing soldier and extend in a single
        // direction (no points on the far side of the muzzle).
        let mut lw = LineWars::new(
            &ids(&["alice", "bob"]),
            &json!({ "seed": 9, "obstacles": 0 }),
        )
        .unwrap();
        let me = lw.players[lw.turn].clone();
        let idx = lw.shooter[lw.turn];
        let (mx, _) = (lw.soldiers[lw.turn][idx].x, lw.soldiers[lw.turn][idx].y);
        // Aim explicitly right, regardless of where the nearest enemy is.
        lw.apply(&me, "> 0").unwrap();
        let sh = lw.last_shot.as_ref().unwrap();
        assert_eq!(sh.muzzle_index, 0, "polyline starts at the muzzle");
        assert_eq!(sh.from.0, round3(mx));
        assert!(
            sh.points.iter().all(|&(x, _)| x >= round3(mx) - 1e-6),
            "an explicit right-shot only has points at or to the right of the muzzle"
        );
    }

    #[test]
    fn direction_prefix_parses() {
        assert_eq!(split_direction(">sin(x)"), (Some(1.0), "sin(x)"));
        assert_eq!(split_direction("< -x/2"), (Some(-1.0), " -x/2"));
        assert_eq!(split_direction("LEFT: x^2"), (Some(-1.0), " x^2"));
        assert_eq!(split_direction("r: 3*x"), (Some(1.0), " 3*x"));
        assert_eq!(split_direction("sin(x)"), (None, "sin(x)"));
    }

    #[test]
    fn resign_forfeits_squad_and_can_decide_winner() {
        let mut m = build(ids(&["alice", "bob"]), json!({ "seed": 7 }));
        m.start();
        let quitter = m.state_json()["turn_handle"].as_str().unwrap().to_string();
        let seat = seat_of(&quitter);
        m.resign(seat);
        let res = m.result().unwrap();
        assert_eq!(res.outcome, "Winner");
        assert_ne!(res.winner.as_deref(), Some(quitter.as_str())); // the other player wins
    }

    #[test]
    fn agent_view_omits_polyline_spectator_keeps_it() {
        let mut m = build(ids(&["alice", "bob"]), json!({ "seed": 7, "obstacles": 0 }));
        m.start();
        let mover = m.state_json()["turn_handle"].as_str().unwrap().to_string();
        let seat = seat_of(&mover);
        m.make_move(seat, "0", 0).unwrap();

        // Spectator (public) view keeps the full polyline for animation.
        let pub_shot = &m.state_json()["last_shot"];
        assert!(pub_shot["points"].is_array(), "spectator view keeps points");

        // Agent (private) view drops points, keeps the compact summary + impact.
        let agent_shot = &m.private_state(seat)["last_shot"];
        assert!(agent_shot["points"].is_null(), "agent view has no polyline");
        assert!(agent_shot["func"].is_string());
        assert!(agent_shot["outcome"].is_string());
        assert!(agent_shot.get("impact").is_some());
    }

    #[test]
    fn observe_has_required_keys() {
        let m = build(
            ids(&["alice", "bob", "carol", "dave"]),
            json!({ "seed": 5 }),
        );
        let st = m.state_json();
        for k in [
            "game",
            "field",
            "players",
            "player_count",
            "turn",
            "obstacles",
            "status",
        ] {
            assert!(st.get(k).is_some(), "missing key {k}");
        }
        assert_eq!(st["game"], "line-wars");
        assert_eq!(st["player_count"], 4);
        assert_eq!(st["players"].as_array().unwrap().len(), 4);
    }
}
