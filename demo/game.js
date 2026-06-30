/* Stormfall Isle — a battle-royale on a shrinking-storm ISO VOXEL ISLAND
 * floating on shimmering water. Two survivors (A=Vortex, B=Bunker) plus a few
 * NPC bots get picked off as a glowing magenta-violet STORM WALL contracts
 * ring-by-ring each round. Anyone outside the safe ring bleeds HP (rising embers).
 *
 * Each turn an agent reads its PUBLIC PROMPT doctrine and picks ONE move:
 *   loot:crate   — grab gear (stronger attack) but stand exposed
 *   hide:bunker  — turtle, safe, no gear gained
 *   rotate:eye   — move toward the safe-zone center (avoid storm damage)
 *   hunt:rival   — attack the rival (needs them reachable)
 *
 * Win = rival eliminated (HP→0) or last standing when the storm closes.
 * Draw if both die the same tick.  HIDDEN TWIST: the final safe-zone center is
 * seeded-random, so "rotate early" pays off variably — identical prompts don't
 * always resolve the same way.
 *
 * Faithful to the engine Game-trait model: turn-based, opaque move-strings, the
 * agent plays via get_state → legal_moves → make_move(mv, expected_ply) → resign.
 */
(function () {
  const A = window.AW;
  const W = 780, H = 560;

  // ---- ISO island grid -----------------------------------------------------
  // logical board is GN x GN cells; we project to an iso diamond on screen.
  const GN = 9;                 // grid cells per side
  const ROUNDS = 5;             // storm contractions
  const TILE = 46, TH = 23;     // iso tile half-width / half-height
  const ISO_CX = W * 0.40, ISO_CY = 196; // diamond top-center anchor
  function iso(gx, gy) {
    return {
      x: ISO_CX + (gx - gy) * TILE,
      y: ISO_CY + (gx + gy) * TH,
    };
  }
  const MID = (GN - 1) / 2;

  // ---- doctrine: parse the public prompt into a survival policy ------------
  const KW = {
    looter:  ["loot", "gear", "speed", "fast", "aggressive", "fight", "loot first", "rush", "weapon", "crate", "kill", "hunt", "attack"],
    turtle:  ["hide", "turtle", "bunker", "safe", "patient", "defensive", "wait", "survive", "passive", "cover", "camp"],
    rotator: ["rotate", "early", "edge", "zone", "center", "reposition", "circle", "storm", "anticipate", "eye"],
  };
  function doctrine(prompt) {
    const p = (prompt || "").toLowerCase();
    const sc = { looter: 0, turtle: 0, rotator: 0 };
    for (const k in KW) for (const w of KW[k]) if (p.includes(w)) sc[k]++;
    let kind = "balanced", best = 0;
    for (const k in sc) if (sc[k] > best) { best = sc[k]; kind = k; }
    if (best === 0) return { kind: "balanced", tag: "wildcard", aggro: 0.5, rotate: 0.5, greed: 0.5 };
    const tag = { looter: "loot raider", turtle: "bunker rat", rotator: "zone reader" }[kind];
    const prof = {
      looter:  { aggro: 0.85, rotate: 0.35, greed: 0.9 },
      turtle:  { aggro: 0.2,  rotate: 0.5,  greed: 0.2 },
      rotator: { aggro: 0.4,  rotate: 0.95, greed: 0.4 },
    }[kind];
    return Object.assign({ kind, tag }, prof);
  }
  function highlight(prompt) {
    let h = (prompt || "").replace(/[&<>]/g, (m) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[m]));
    const all = [].concat(KW.looter, KW.turtle, KW.rotator).sort((a, b) => b.length - a.length);
    const seen = new Set();
    for (const k of all) {
      if (seen.has(k)) continue; seen.add(k);
      h = h.replace(new RegExp("\\b(" + k.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ")\\b", "ig"), "<b>$1</b>");
    }
    return h;
  }

  const DEF_A = "Loot fast and hunt. Grab every crate for gear, then fight — rush the rival aggressive while my attack is strongest. Speed and kills, not hiding.";
  const DEF_B = "Hide and survive. Turtle in the bunker, stay safe and patient, only rotate to the zone center when the storm forces me. Let them burn; I outlast.";

  // ---- the deterministic engine --------------------------------------------
  function build(seed, opts) {
    const rng = A.rng(seed >>> 0);
    const prompts = {
      A: (opts.prompts && opts.prompts.A) || DEF_A,
      B: (opts.prompts && opts.prompts.B) || DEF_B,
    };
    const doc = { A: doctrine(prompts.A), B: doctrine(prompts.B) };

    // HIDDEN TWIST: final safe-zone center is seeded-random (not the board center).
    // Each round the ring shrinks toward this point. "rotate early" pays off only
    // if you read where it lands — so identical prompts diverge by seed.
    const finalCx = 2 + Math.floor(rng() * (GN - 4)); // 2..GN-3
    const finalCy = 2 + Math.floor(rng() * (GN - 4));

    // ring radius per round (Chebyshev distance from the shrinking center).
    // round 0 sits just inside the island edge so the wall hugs the coast.
    const ringR = [3.6, 2.8, 2.0, 1.3, 0.8, 0.4];
    // center drifts from board-mid toward finalC across rounds
    function centerAt(round) {
      const f = A.clamp(round / ROUNDS, 0, 1);
      return { cx: A.lerp(MID, finalCx, f), cy: A.lerp(MID, finalCy, f) };
    }
    function inRing(gx, gy, round) {
      const c = centerAt(round);
      const d = Math.max(Math.abs(gx - c.cx), Math.abs(gy - c.cy));
      return d <= ringR[round] + 0.001;
    }

    // crate positions (seeded), some gear value each
    const crates = [];
    {
      const cr = A.rng(seed * 71 + 13);
      const n = 9;
      for (let i = 0; i < n; i++) {
        const gx = 1 + Math.floor(cr() * (GN - 2));
        const gy = 1 + Math.floor(cr() * (GN - 2));
        crates.push({ gx, gy, gear: 2 + Math.floor(cr() * 3), taken: false });
      }
    }

    // bunker tiles (safe spots) — a couple of fixed-ish spots
    const bunkers = [
      { gx: 1, gy: GN - 2 }, { gx: GN - 2, gy: 1 }, { gx: GN - 2, gy: GN - 2 },
    ];

    // combatants: A, B and a few NPC bots
    function mk(id, gx, gy, hp, isNPC, name, col) {
      return { id, gx, gy, hp, maxhp: hp, gear: 0, alive: true, isNPC, name, col, deathRound: -1, lastMove: null };
    }
    const A0 = mk("A", 1, 1, 100, false, "Vortex", "#10b981");
    const B0 = mk("B", GN - 2, GN - 2, 100, false, "Bunker", "#8b5cf6");
    const npcs = [];
    {
      const nr = A.rng(seed * 311 + 7);
      const spots = [[GN - 2, 1], [1, GN - 2], [Math.floor(MID), 1], [1, Math.floor(MID)]];
      const cols = ["#f59e0b", "#f43f5e", "#38bdf8", "#a3e635"];
      const nm = ["Drone-7", "Husk", "Scrap", "Echo"];
      const count = 3;
      for (let i = 0; i < count; i++) {
        npcs.push(mk("N" + i, spots[i][0], spots[i][1], 55 + Math.floor(nr() * 25), true, nm[i], cols[i]));
      }
    }
    const all = [A0, B0].concat(npcs);
    const ent = { A: A0, B: B0 };

    const beats = [];
    const oddsHist = [];
    let ply = 1, winner = undefined, winReason = "survive", done = false;

    function aliveCount() { return all.filter((e) => e.alive).length; }
    function nearestCrate(e) {
      let best = null, bd = 1e9;
      for (const c of crates) if (!c.taken) {
        const d = Math.abs(c.gx - e.gx) + Math.abs(c.gy - e.gy);
        if (d < bd) { bd = d; best = c; }
      }
      return best;
    }
    function stepToward(e, tx, ty) {
      // move one cell (king-move) toward target
      const dx = Math.sign(tx - e.gx), dy = Math.sign(ty - e.gy);
      e.gx = A.clamp(e.gx + dx, 0, GN - 1);
      e.gy = A.clamp(e.gy + dy, 0, GN - 1);
    }
    function reachable(e, t) {
      return Math.max(Math.abs(e.gx - t.gx), Math.abs(e.gy - t.gy)) <= 2 && t.alive;
    }

    // odds from HP + gear + zone safety
    function snapOdds() {
      const score = (e) => {
        if (!e.alive) return 0.0001;
        return e.hp * 1.0 + e.gear * 6 + 8;
      };
      const sa = score(A0), sb = score(B0);
      // NPCs dilute slightly but A/B are the market
      let a = sa, b = sb;
      const s = a + b || 1; a = (a / s) * 100; b = (b / s) * 100;
      return { A: a, B: b };
    }
    oddsHist.push(snapOdds());

    function nameOf(id) { return id === "A" ? "Vortex" : id === "B" ? "Bunker" : (ent[id] ? ent[id].name : id); }

    // storm damage applied to everyone outside the ring at start of a round
    function applyStorm(round, journal) {
      for (const e of all) {
        if (!e.alive) continue;
        if (!inRing(e.gx, e.gy, round)) {
          const dmg = 14 + round * 3;
          e.hp = Math.max(0, e.hp - dmg);
          if (e.hp <= 0 && e.alive) { e.alive = false; e.deathRound = round; if (journal) journal.push(`${e.name} is caught in the storm wall — eliminated.`); }
        }
      }
    }

    // pick a legal move for A/B by doctrine, with a touch of seeded jitter
    function chooseMove(id, round, legal) {
      const d = doc[id], me = ent[id], op = ent[id === "A" ? "B" : "A"];
      const jr = A.rng(seed * 53 + ply * 17 + (id === "A" ? 1 : 2));
      const j = jr();
      const outOfRing = !inRing(me.gx, me.gy, round);
      const canHunt = legal.includes("hunt:rival");
      const canLoot = legal.includes("loot:crate");

      // storm forces a rotate when badly outside
      if (outOfRing && legal.includes("rotate:eye")) {
        // rotators & turtles always obey; looters obey if low HP or really exposed
        if (d.kind !== "looter" || me.hp < 45 || j < 0.5) return "rotate:eye";
      }
      // looter: loot until geared, then hunt
      if (d.kind === "looter") {
        if (canHunt && me.gear >= 2 && (op.hp <= me.hp + 20 || j < 0.7)) return "hunt:rival";
        if (canLoot && me.gear < 4) return "loot:crate";
        if (canHunt) return "hunt:rival";
      }
      if (d.kind === "turtle") {
        if (legal.includes("hide:bunker") && me.hp > 30 && !outOfRing) return "hide:bunker";
        if (canHunt && op.hp < 25) return "hunt:rival"; // finish a wounded rival
        if (legal.includes("rotate:eye")) return "rotate:eye";
      }
      if (d.kind === "rotator") {
        if (legal.includes("rotate:eye") && (j < d.rotate || outOfRing)) return "rotate:eye";
        if (canLoot && me.gear < 2) return "loot:crate";
        if (canHunt && me.gear >= 1 && op.hp < me.hp) return "hunt:rival";
      }
      // balanced / fallback: weigh
      if (outOfRing && legal.includes("rotate:eye")) return "rotate:eye";
      if (canLoot && me.gear < 3 && j < 0.6) return "loot:crate";
      if (canHunt && me.gear >= 1) return "hunt:rival";
      if (legal.includes("hide:bunker")) return "hide:bunker";
      return legal[0];
    }

    function legalFor(id, round) {
      const me = ent[id], op = ent[id === "A" ? "B" : "A"];
      const L = [];
      if (nearestCrate(me)) L.push("loot:crate");
      L.push("hide:bunker");
      L.push("rotate:eye");
      if (reachable(me, op)) L.push("hunt:rival");
      return L;
    }

    // NPC act (simple): move toward center, sometimes loot; they mostly get
    // attrited by the storm. They are flavour, not market.
    function npcAct(round) {
      const c = centerAt(round);
      for (const n of npcs) {
        if (!n.alive) continue;
        if (!inRing(n.gx, n.gy, round)) { stepToward(n, c.cx, c.cy); }
        else { const cr = nearestCrate(n); if (cr) { stepToward(n, cr.gx, cr.gy); if (n.gx === cr.gx && n.gy === cr.gy) { cr.taken = true; n.gear += cr.gear; } } }
      }
    }

    function snapState(mover) {
      return {
        round: snapState._round || 0,
        ents: all.map((e) => ({ id: e.id, gx: e.gx, gy: e.gy, hp: e.hp, maxhp: e.maxhp, gear: e.gear, alive: e.alive, isNPC: e.isNPC, name: e.name, col: e.col, lastMove: e.lastMove })),
        crates: crates.map((c) => ({ gx: c.gx, gy: c.gy, gear: c.gear, taken: c.taken })),
        bunkers: bunkers.slice(),
        center: centerAt(snapState._round || 0),
        ringR: ringR[snapState._round || 0],
        ringPrev: ringR[Math.max(0, (snapState._round || 0) - 0)],
        mover,
        finalC: { cx: finalCx, cy: finalCy },
        survivors: aliveCount(),
      };
    }
    snapState._round = 0;

    function doResolve() {
      // determine winner among A/B
      if (A0.alive && !B0.alive) { winner = "A"; winReason = winReason || "survive"; }
      else if (B0.alive && !A0.alive) { winner = "B"; }
      else if (!A0.alive && !B0.alive) {
        if (A0.deathRound === B0.deathRound) { winner = null; winReason = "double"; }
        else { winner = A0.deathRound > B0.deathRound ? "A" : "B"; winReason = "survive"; }
      } else {
        // both alive at the end → higher (hp + gear) wins
        const sa = A0.hp + A0.gear * 6, sb = B0.hp + B0.gear * 6;
        winner = sa === sb ? null : sa > sb ? "A" : "B";
        winReason = winner == null ? "double" : "standing";
      }
    }

    // ===== main loop: each round = storm tick + each survivor acts =====
    outer:
    for (let round = 0; round < ROUNDS && !done; round++) {
      snapState._round = round;
      const journal = [];
      // storm contracts: a ref beat announcing the new ring + damage
      if (round > 0) {
        applyStorm(round, journal);
        const c = centerAt(round);
        beats.push({
          ply: ply++, agent: "ref", move: "storm:contract",
          thought: "The storm wall closes another ring.",
          observe: { round, ring: ringR[round].toFixed(1), center: "(" + c.cx.toFixed(1) + "," + c.cy.toFixed(1) + ")", survivors: aliveCount() },
          legal: null, ok: true,
          result: "ring → " + ringR[round].toFixed(1) + " · survivors " + aliveCount(),
          events: journal.length ? journal.slice() : ["The magenta storm wall contracts — the safe zone shrinks."],
          state: snapState(null),
        });
        oddsHist.push(snapOdds());
        if (!A0.alive && !B0.alive) { done = true; break; }
        if (winner === undefined && (!A0.alive || !B0.alive) && round >= 1) {
          // a storm kill can decide it if only one of A/B remains and they're isolated
          if (!A0.alive && B0.alive) { winner = "B"; winReason = "survive"; done = true; break; }
          if (!B0.alive && A0.alive) { winner = "A"; winReason = "survive"; done = true; break; }
        }
      }
      npcAct(round);

      for (const id of ["A", "B"]) {
        if (done) break;
        const me = ent[id], op = ent[id === "A" ? "B" : "A"];
        if (!me.alive) continue;
        const legal = legalFor(id, round);
        const mv = chooseMove(id, round, legal);
        me.lastMove = mv;
        let ok = true, result = "", evt = "", hit = null;

        if (mv === "loot:crate") {
          const cr = nearestCrate(me);
          if (cr) { stepToward(me, cr.gx, cr.gy); if (me.gx === cr.gx && me.gy === cr.gy) { cr.taken = true; me.gear += cr.gear; result = "ok · +" + cr.gear + " gear (atk " + me.gear + ")"; evt = `${me.name} cracks a crate — +${cr.gear} gear, attack now ${me.gear}.`; } else { result = "ok · advancing to crate"; evt = `${me.name} sprints toward a loot crate, exposed in the open.`; } }
          else { ok = false; result = "no crate left"; evt = `${me.name} finds no crates left.`; }
        } else if (mv === "hide:bunker") {
          // move toward nearest bunker, small heal/no exposure
          let bb = null, bd = 1e9;
          for (const b of bunkers) { const d = Math.abs(b.gx - me.gx) + Math.abs(b.gy - me.gy); if (d < bd) { bd = d; bb = b; } }
          if (bb) stepToward(me, bb.gx, bb.gy);
          me.hp = Math.min(me.maxhp, me.hp + 4);
          result = "ok · turtled (hp " + Math.round(me.hp) + ")";
          evt = `${me.name} hunkers in the bunker — patching up, gaining no gear.`;
        } else if (mv === "rotate:eye") {
          const c = centerAt(Math.min(round + 1, ROUNDS)); // rotate toward NEXT ring center
          stepToward(me, c.cx, c.cy);
          stepToward(me, c.cx, c.cy);
          result = "ok · rotated toward zone";
          evt = `${me.name} rotates toward the eye of the storm.`;
        } else if (mv === "hunt:rival") {
          if (reachable(me, op)) {
            const base = 16 + me.gear * 7;
            const jr = A.rng(seed * 97 + ply * 31 + (id === "A" ? 5 : 9));
            const dmg = Math.round(base * (0.8 + jr() * 0.5));
            op.hp = Math.max(0, op.hp - dmg);
            result = "ok · hit " + op.name + " for " + dmg;
            evt = `${me.name} strikes ${op.name} for ${dmg}${op.hp <= 0 ? " — ELIMINATED!" : " (hp " + Math.round(op.hp) + ")"}.`;
            // record the strike so the renderer can pop a floating damage number
            // on the target (combat visible in-world, not only in the ticker).
            hit = { target: op.id, gx: op.gx, gy: op.gy, dmg, kill: op.hp <= 0 };
            if (op.hp <= 0 && op.alive) { op.alive = false; op.deathRound = round; }
          } else { ok = false; result = "rival out of range"; evt = `${me.name} lunges but the rival is out of reach.`; }
        }

        const d = doc[id];
        const thought = d.kind === "looter" ? "Gear up, then end them — aggression wins."
          : d.kind === "turtle" ? "Stay safe, let the storm and rivals do the work."
          : d.kind === "rotator" ? "Read the zone, rotate early, hold the center."
          : "Read the board and take the highest-value move.";

        beats.push({
          ply: ply++, agent: id,
          thought,
          observe: {
            round, hp: Math.round(me.hp) + "/" + me.maxhp, gear: me.gear,
            in_zone: inRing(me.gx, me.gy, round) ? "yes" : "NO-storm!",
            rival_hp: Math.round(op.hp), survivors: aliveCount(),
          },
          legal,
          move: mv, ok, result,
          events: [evt],
          state: Object.assign(snapState(id), { hit }),
        });
        oddsHist.push(snapOdds());

        // win checks
        if (!op.alive && (op.id === "A" || op.id === "B")) {
          // only A/B elimination ends the market
          if (me.id === "A" || me.id === "B") {
            if (!A0.alive && !B0.alive) { winner = null; winReason = "double"; }
            else { winner = me.id; winReason = "elim"; }
            done = true; break outer;
          }
        }
        if (!A0.alive && !B0.alive) { winner = null; winReason = "double"; done = true; break outer; }
      }
    }

    if (winner === undefined) doResolve();

    function finalLine() {
      if (winner == null) return "Double knockout — both survivors fell the same tick. Draw.";
      const wn = nameOf(winner), ln = nameOf(winner === "A" ? "B" : "A");
      if (winReason === "elim") return `${wn} eliminates ${ln} — VICTORY ROYALE.`;
      if (winReason === "survive") return `${ln} is consumed by the storm — ${wn} is the last survivor.`;
      if (winReason === "standing") return `Storm closed. ${wn} stands strongest (HP + gear) — VICTORY ROYALE.`;
      return `${wn} takes the isle.`;
    }

    snapState._round = Math.min(ROUNDS, snapState._round);
    beats.push({
      ply: ply++, agent: "ref", move: "resolve", legal: null,
      thought: "The storm has closed. Resolving the isle.",
      observe: { winner: winner == null ? "draw" : nameOf(winner), reason: winReason, survivors: aliveCount() },
      ok: true,
      result: winner == null ? "draw — double KO" : nameOf(winner) + " wins · " + winReason,
      events: [finalLine()],
      state: Object.assign(snapState(null), { final: true }),
    });

    return {
      seed, beats, winner, winReason,
      names: { A: "Vortex", B: "Bunker" },
      promptOf: (id) => highlight(prompts[id]),
      tagOf: (id) => doc[id].tag,
      oddsAt: (b) => oddsHist[Math.min(b, oddsHist.length - 1)] || { A: 50, B: 50 },
      _doc: doc, _finalC: { cx: finalCx, cy: finalCy },
    };
  }

  // ====== RENDER =============================================================
  // interpolate an entity's screen pos between its prev beat cell and current.
  function entPos(res, beat, beatT, id) {
    let cur = null, prev = null;
    for (let k = 0; k <= beat; k++) {
      const st = res.beats[k].state; if (!st) continue;
      const e = st.ents.find((x) => x.id === id);
      if (!e) continue;
      // track movement only when this beat is the mover (or storm)
      prev = cur; cur = { e, mover: st.mover };
    }
    if (!cur) return null;
    // find previous distinct position
    let from = null;
    for (let k = beat - 1; k >= 0; k--) {
      const st = res.beats[k].state; if (!st) continue;
      const e = st.ents.find((x) => x.id === id); if (!e) continue;
      from = e; break;
    }
    const to = cur.e;
    const f = from || to;
    const active = res.beats[beat] && res.beats[beat].state && res.beats[beat].state.mover === id && !res.beats[beat].state.final;
    const tt = active ? A.easeOut(beatT) : 1;
    const gx = A.lerp(f.gx, to.gx, tt), gy = A.lerp(f.gy, to.gy, tt);
    const p = iso(gx, gy);
    return { x: p.x, y: p.y, gx, gy, e: to, moving: active && beatT < 0.95 && (f.gx !== to.gx || f.gy !== to.gy) };
  }

  function tileTop(gx, gy) { const p = iso(gx, gy); return { x: p.x, y: p.y }; }

  function draw(ctx, v) {
    const t = v.t, res = v.result, beat = v.beat, bt = res.beats[beat];
    const st = bt && bt.state ? bt.state : null;
    const round = st ? st.round : 0;

    skyDusk(ctx, t, beat, res.beats.length);
    water(ctx, t);
    islandBase(ctx, t);
    // floor tiles + storm-consumed purple tint
    floorTiles(ctx, res, st, round, t);
    ruins(ctx, res.seed, t);
    palms(ctx, res.seed, t);
    cratesDraw(ctx, st, t);
    bunkersDraw(ctx, st);

    // storm wall (drawn around the ring), then entities, then embers outside
    stormWall(ctx, st, round, t, v);

    // draw entities sorted by gy (painter's order)
    const drawn = [];
    if (st) {
      for (const e of st.ents) {
        if (e.id === "A" || e.id === "B") {
          const pc = entPos(res, beat, v.beatT, e.id);
          if (pc) drawn.push({ pc, e, sort: pc.y });
        } else {
          const p = iso(e.gx, e.gy);
          drawn.push({ pc: { x: p.x, y: p.y, gx: e.gx, gy: e.gy, e, moving: false }, e, sort: p.y });
        }
      }
    }
    drawn.sort((a, b) => a.sort - b.sort);
    const champPlates = [];
    for (const d of drawn) {
      if (d.e.isNPC) npcSprite(ctx, d.pc, d.e, t);
      else { championSprite(ctx, d.pc, d.e, t, res); if (d.e.alive) champPlates.push({ pc: d.pc, e: d.e }); }
    }
    // champion name plates LAST, with collision-aware offsetting so the A and B
    // tags never z-fight when the survivors stack vertically on the iso grid.
    drawChampPlates(ctx, champPlates, res);
    // floating combat damage numbers (hunt:rival hits) on top of sprites
    damageNumbers(ctx, res, beat, v, t);

    embers(ctx, st, round, t);
    hud(ctx, res, st, t);
    survivorsBadge(ctx, st);
    eyeMarker(ctx, res, st, v, t);
    dispatcher(ctx, res, bt, v);

    if (v.over) finishOverlay(ctx, res, t);
    vignette(ctx);
  }

  // --- sky: day → dusk sweep based on progress ------------------------------
  function skyDusk(ctx, t, beat, total) {
    const f = A.clamp(beat / Math.max(1, total - 1), 0, 1);
    const top = mix("#7fc7ff", "#1b1340", f);
    const mid = mix("#bfe6ff", "#6d2a86", f);
    const hor = mix("#ffe9c2", "#e8557e", f);
    const g = ctx.createLinearGradient(0, 0, 0, H * 0.62);
    g.addColorStop(0, top); g.addColorStop(0.55, mid); g.addColorStop(1, hor);
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // sun/moon descending
    const sx = W * 0.78, sy = A.lerp(70, 150, f);
    A.glow(ctx, sx, sy, 90, f < 0.5 ? "rgba(255,236,170,0.45)" : "rgba(255,150,120,0.4)");
    ctx.fillStyle = mix("#fff6cf", "#ffd0a0", f); ctx.beginPath(); ctx.arc(sx, sy, 26, 0, 7); ctx.fill();
    // far stars appearing at dusk
    if (f > 0.45 && !A.reduced) {
      for (let i = 0; i < 40; i++) {
        const x = (i * 137) % W, y = (i * 53) % 170;
        const tw = (Math.sin(t / 700 + i) + 1) / 2;
        ctx.fillStyle = `rgba(220,230,255,${(f - 0.45) * (0.2 + tw * 0.5)})`;
        ctx.fillRect(x, y, 2, 2);
      }
    }
    // distant clouds
    for (let i = 0; i < 5; i++) {
      const cx = (i * 190 + t * 0.006) % (W + 200) - 100;
      const cy = 60 + (i % 3) * 30;
      ctx.fillStyle = `rgba(255,255,255,${0.10 - f * 0.06})`;
      ctx.beginPath(); ctx.ellipse(cx, cy, 60, 16, 0, 0, 7); ctx.fill();
    }
  }
  function mix(a, b, f) {
    const pa = hx(a), pb = hx(b);
    const r = Math.round(A.lerp(pa[0], pb[0], f)), g = Math.round(A.lerp(pa[1], pb[1], f)), bl = Math.round(A.lerp(pa[2], pb[2], f));
    return `rgb(${r},${g},${bl})`;
  }
  function hx(h) { h = h.replace("#", ""); return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]; }

  // --- shimmering water around the island -----------------------------------
  function water(ctx, t) {
    const g = ctx.createLinearGradient(0, H * 0.5, 0, H);
    g.addColorStop(0, "#0d4a6e"); g.addColorStop(1, "#06243e");
    ctx.fillStyle = g; ctx.fillRect(0, H * 0.5, W, H * 0.5);
    if (A.reduced) return;
    for (let i = 0; i < 34; i++) {
      const y = H * 0.5 + i * 8;
      const a = 0.05 + 0.06 * (Math.sin(t / 600 + i) * 0.5 + 0.5);
      ctx.strokeStyle = `rgba(120,210,255,${a})`; ctx.lineWidth = 1.4;
      ctx.beginPath();
      for (let x = 0; x <= W; x += 30) {
        const yy = y + Math.sin(t / 500 + i + x / 90) * 3;
        x === 0 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy);
      }
      ctx.stroke();
    }
  }

  // --- the floating iso island base (thick voxel slab) ----------------------
  function islandBase(ctx, t) {
    // diamond outline of the GN x GN board, expanded by 0.5 tile
    const c0 = iso(-0.6, -0.6), c1 = iso(GN - 0.4, -0.6), c2 = iso(GN - 0.4, GN - 0.4), c3 = iso(-0.6, GN - 0.4);
    const depth = 56;
    // soft glow under island
    A.glow(ctx, (c0.x + c2.x) / 2, (c0.y + c2.y) / 2 + 30, 320, "rgba(40,120,160,0.18)");
    // sand/dirt sides (left + right faces of the slab)
    ctx.fillStyle = "#7a5a34";
    ctx.beginPath(); ctx.moveTo(c3.x, c3.y); ctx.lineTo(c2.x, c2.y); ctx.lineTo(c2.x, c2.y + depth); ctx.lineTo(c3.x, c3.y + depth); ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#5e4427";
    ctx.beginPath(); ctx.moveTo(c2.x, c2.y); ctx.lineTo(c1.x, c1.y); ctx.lineTo(c1.x, c1.y + depth); ctx.lineTo(c2.x, c2.y + depth); ctx.closePath(); ctx.fill();
    // craggy bottom edge
    ctx.fillStyle = "#3c2c19";
    ctx.beginPath();
    ctx.moveTo(c3.x, c3.y + depth);
    for (let x = c3.x; x <= c2.x; x += 26) ctx.lineTo(x, c3.y + depth + 6 + Math.sin(x) * 5);
    ctx.lineTo(c2.x, c2.y + depth);
    ctx.lineTo(c1.x, c1.y + depth);
    for (let x = c2.x; x <= c1.x; x += 26) ctx.lineTo(x, A.lerp(c2.y, c1.y, (x - c2.x) / (c1.x - c2.x)) + depth + 6 + Math.cos(x) * 5);
    ctx.closePath(); ctx.fill();
  }

  // grid-cell color helpers
  function cellBase(gx, gy, seed) {
    const r = A.rng(seed * 17 + gx * 31 + gy * 7)();
    // ring of sand near the border, grass inside
    const edge = (gx <= 0 || gy <= 0 || gx >= GN - 1 || gy >= GN - 1);
    if (edge) return r < 0.5 ? "#d9c089" : "#cdb37a"; // sand
    const g = r < 0.5 ? "#3f9e54" : (r < 0.8 ? "#349049" : "#46a85d");
    return g;
  }

  function floorTiles(ctx, res, st, round, t) {
    const seed = res.seed;
    const center = st ? st.center : { cx: MID, cy: MID };
    const ringR = st ? st.ringR : GN;
    for (let gy = 0; gy < GN; gy++) for (let gx = 0; gx < GN; gx++) {
      const p = iso(gx, gy);
      // diamond tile (top face)
      const col = cellBase(gx, gy, seed);
      diamond(ctx, p.x, p.y, col);
      // bevel highlight (top-left edges)
      ctx.strokeStyle = "rgba(255,255,255,0.12)"; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(p.x - TILE, p.y); ctx.lineTo(p.x, p.y - TH); ctx.lineTo(p.x + TILE, p.y); ctx.stroke();
      ctx.strokeStyle = "rgba(0,0,0,0.16)";
      ctx.beginPath(); ctx.moveTo(p.x - TILE, p.y); ctx.lineTo(p.x, p.y + TH); ctx.lineTo(p.x + TILE, p.y); ctx.stroke();
      // ground scatter detail (rocks, grass tufts, dirt) — keep the board dense
      // even on the bright opening frame. Seeded & deterministic per cell.
      groundDetail(ctx, gx, gy, p, seed, t);
      // consumed-by-storm DEAD ZONE (outside current ring): strongly darken +
      // desaturate so the shrinking safe zone is unmistakable at a glance.
      const d = Math.max(Math.abs(gx - center.cx), Math.abs(gy - center.cy));
      if (d > ringR + 0.001) {
        // 1) heavy dark/desaturate wash (multiply-ish): kills the bright greens
        diamondFill(ctx, p.x, p.y, "rgba(18,8,30,0.62)");
        // 2) purple consumed haze on top, pulsing
        const pulse = 0.30 + 0.16 * (Math.sin(t / 360 + gx + gy) * 0.5 + 0.5);
        diamondFill(ctx, p.x, p.y, `rgba(150,40,205,${A.reduced ? 0.34 : pulse})`);
        // 3) creeping inner-edge brightening on the very first dead ring so the
        //    boundary between safe & consumed pops
        const edgeBand = d <= ringR + 1.0;
        if (edgeBand) diamondFill(ctx, p.x, p.y, `rgba(214,80,255,${A.reduced ? 0.18 : 0.14 + 0.12 * (Math.sin(t / 220 + gx - gy) * 0.5 + 0.5)})`);
        // faint lightning crackle on consumed tiles
        if (!A.reduced && ((gx * 7 + gy * 13 + Math.floor(t / 150)) % 11) < 1) {
          ctx.strokeStyle = "rgba(235,180,255,0.85)"; ctx.lineWidth = 1.5;
          ctx.beginPath(); ctx.moveTo(p.x - 7, p.y - 7); ctx.lineTo(p.x + 3, p.y - 1); ctx.lineTo(p.x - 4, p.y + 4); ctx.lineTo(p.x + 5, p.y + 8); ctx.stroke();
        }
      }
    }
  }

  // small seeded ground props so the board never reads as flat empty grass.
  // Drawn under the storm wash so consumed tiles still darken them over.
  function groundDetail(ctx, gx, gy, p, seed, t) {
    const edge = (gx <= 0 || gy <= 0 || gx >= GN - 1 || gy >= GN - 1);
    const r = A.rng(seed * 1009 + gx * 131 + gy * 37);
    const a = r(), b = r(), c = r();
    // a couple of dirt/scuff patches per tile to break up flat color
    if (a < 0.55) {
      ctx.fillStyle = edge ? "rgba(150,120,70,0.22)" : "rgba(20,60,28,0.22)";
      const ox = (b - 0.5) * TILE * 0.7, oy = (c - 0.5) * TH * 0.7;
      ctx.beginPath(); ctx.ellipse(p.x + ox, p.y + oy, 9 + b * 7, 4 + c * 3, 0, 0, 7); ctx.fill();
    }
    // rocks on some tiles (with a contact shadow)
    if (a > 0.78) {
      const ox = (b - 0.5) * TILE * 0.5, oy = (c - 0.5) * TH * 0.4;
      const rs = 3 + b * 3;
      A.shadow(ctx, p.x + ox + 1, p.y + oy + rs * 0.5, rs + 2, rs * 0.5 + 1, 0.22);
      ctx.fillStyle = "#7d7568";
      ctx.beginPath(); ctx.moveTo(p.x + ox - rs, p.y + oy); ctx.lineTo(p.x + ox, p.y + oy - rs); ctx.lineTo(p.x + ox + rs, p.y + oy); ctx.lineTo(p.x + ox, p.y + oy + rs * 0.6); ctx.closePath(); ctx.fill();
      ctx.fillStyle = "#9a9286"; ctx.beginPath(); ctx.moveTo(p.x + ox - rs, p.y + oy); ctx.lineTo(p.x + ox, p.y + oy - rs); ctx.lineTo(p.x + ox, p.y + oy); ctx.closePath(); ctx.fill();
    } else if (a > 0.5 && !edge) {
      // grass tufts (a few little blades) on interior tiles
      const ox = (b - 0.5) * TILE * 0.6, oy = (c - 0.5) * TH * 0.5;
      const sway = A.reduced ? 0 : Math.sin(t / 700 + gx * 2 + gy) * 1.5;
      ctx.strokeStyle = "#2e7d3f"; ctx.lineWidth = 1.4; ctx.lineCap = "round";
      for (let k = -1; k <= 1; k++) {
        ctx.beginPath(); ctx.moveTo(p.x + ox + k * 3, p.y + oy + 2);
        ctx.lineTo(p.x + ox + k * 3 + sway * (0.4 + k * 0.2), p.y + oy - 5 - b * 2); ctx.stroke();
      }
      ctx.lineCap = "butt";
    }
  }
  function diamond(ctx, x, y, col) {
    ctx.fillStyle = col;
    ctx.beginPath(); ctx.moveTo(x, y - TH); ctx.lineTo(x + TILE, y); ctx.lineTo(x, y + TH); ctx.lineTo(x - TILE, y); ctx.closePath(); ctx.fill();
  }
  function diamondFill(ctx, x, y, col) {
    ctx.fillStyle = col;
    ctx.beginPath(); ctx.moveTo(x, y - TH); ctx.lineTo(x + TILE, y); ctx.lineTo(x, y + TH); ctx.lineTo(x - TILE, y); ctx.closePath(); ctx.fill();
  }

  // --- a small ruined stone landmark (broken pillars + slab) ----------------
  function ruins(ctx, seed, t) {
    const rr = A.rng(seed * 521 + 19);
    // place toward one interior corner, deterministic
    const gx = 2 + Math.floor(rr() * 2), gy = 2 + Math.floor(rr() * 2);
    const p = iso(gx, gy);
    A.shadow(ctx, p.x, p.y + 6, 30, 11, 0.26);
    // cracked stone base slab (iso voxel)
    A.box(ctx, p.x - 22, p.y - 6, 44, 8, 6, "#6b6358", "#827a6d", "#4d473e");
    // three broken pillars of varying height
    const pill = [[-15, 26], [4, 16], [16, 30]];
    for (const [dx, ph] of pill) {
      const bx = p.x + dx, by = p.y - 4;
      A.box(ctx, bx - 5, by - ph, 10, ph, 5, "#8a8174", "#a59c8c", "#605849");
      // jagged broken top
      ctx.fillStyle = "#4d473e";
      ctx.beginPath(); ctx.moveTo(bx - 5, by - ph); ctx.lineTo(bx, by - ph - 3); ctx.lineTo(bx + 5, by - ph + 1); ctx.lineTo(bx + 10, by - ph - 5); ctx.lineTo(bx + 10, by - ph + 2); ctx.lineTo(bx - 5, by - ph + 2); ctx.closePath(); ctx.fill();
      // moss
      ctx.fillStyle = "rgba(60,140,70,0.5)"; ctx.fillRect(bx - 5, by - 4, 4, 4);
    }
    // a fallen capstone chunk on the ground
    ctx.fillStyle = "#736a5d";
    ctx.beginPath(); ctx.ellipse(p.x + 26, p.y + 4, 8, 4, 0.3, 0, 7); ctx.fill();
  }

  // --- swaying palm trees on a few seeded cells -----------------------------
  function palms(ctx, seed, t) {
    const pr = A.rng(seed * 211 + 3);
    const spots = [];
    for (let i = 0; i < 12; i++) {
      const gx = Math.floor(pr() * GN), gy = Math.floor(pr() * GN);
      spots.push([gx, gy]);
    }
    // sort by gy so they layer with ground
    spots.sort((a, b) => (a[0] + a[1]) - (b[0] + b[1]));
    for (const [gx, gy] of spots) {
      const p = iso(gx, gy);
      const sway = A.reduced ? 0 : Math.sin(t / 900 + gx + gy) * 4;
      // trunk
      ctx.strokeStyle = "#7a5230"; ctx.lineWidth = 4; ctx.lineCap = "round";
      ctx.beginPath(); ctx.moveTo(p.x, p.y - 2);
      ctx.quadraticCurveTo(p.x + sway * 0.5, p.y - 22, p.x + sway, p.y - 40); ctx.stroke();
      // fronds
      const fx = p.x + sway, fy = p.y - 40;
      ctx.strokeStyle = "#2f8a45"; ctx.lineWidth = 3;
      for (let a = 0; a < 6; a++) {
        const ang = (a / 6) * Math.PI * 2 + sway * 0.02;
        ctx.beginPath(); ctx.moveTo(fx, fy);
        ctx.quadraticCurveTo(fx + Math.cos(ang) * 10, fy + Math.sin(ang) * 6 - 4, fx + Math.cos(ang) * 22, fy + Math.sin(ang) * 12);
        ctx.stroke();
      }
      ctx.fillStyle = "#caa24b"; ctx.beginPath(); ctx.arc(fx, fy, 3, 0, 7); ctx.fill(); // coconuts
    }
  }

  // --- loot crates with floating gear icon ----------------------------------
  function cratesDraw(ctx, st, t) {
    if (!st) return;
    for (const c of st.crates) {
      const p = iso(c.gx, c.gy);
      if (c.taken) {
        // opened crate husk
        ctx.globalAlpha = 0.5;
        A.box(ctx, p.x - 9, p.y - 8, 18, 10, 5, "#5b4630", "#6e5638", "#3f3020");
        ctx.globalAlpha = 1;
        continue;
      }
      A.shadow(ctx, p.x, p.y + 8, 14, 5, 0.28);
      A.box(ctx, p.x - 11, p.y - 12, 22, 14, 7, "#b5862f", "#d6a23c", "#8c6622");
      // banded crate straps
      ctx.fillStyle = "#6b4e1e"; ctx.fillRect(p.x - 11, p.y - 7, 22, 2); ctx.fillRect(p.x - 2, p.y - 12, 2, 14);
      // floating gear icon bobbing above
      const bob = A.reduced ? 0 : Math.sin(t / 400 + c.gx) * 3;
      A.glow(ctx, p.x, p.y - 26 + bob, 14, "rgba(255,210,90,0.4)");
      drawGearIcon(ctx, p.x, p.y - 26 + bob, "#ffd23c");
    }
  }
  function drawGearIcon(ctx, x, y, col) {
    ctx.save(); ctx.translate(x, y);
    ctx.fillStyle = col;
    for (let i = 0; i < 8; i++) { ctx.rotate(Math.PI / 4); ctx.fillRect(-1.6, -7, 3.2, 4); }
    ctx.beginPath(); ctx.arc(0, 0, 4.5, 0, 7); ctx.fill();
    ctx.fillStyle = "#7a5a10"; ctx.beginPath(); ctx.arc(0, 0, 1.8, 0, 7); ctx.fill();
    ctx.restore();
  }

  // --- bunkers (safe sandbag spots) -----------------------------------------
  function bunkersDraw(ctx, st) {
    if (!st) return;
    for (const b of st.bunkers) {
      const p = iso(b.gx, b.gy);
      A.shadow(ctx, p.x, p.y + 8, 18, 6, 0.22);
      // sandbag ring
      ctx.fillStyle = "#6f6242";
      for (let a = 0; a < 7; a++) {
        const ang = (a / 7) * Math.PI * 2;
        ctx.beginPath(); ctx.ellipse(p.x + Math.cos(ang) * 16, p.y + Math.sin(ang) * 8 + 2, 6, 4, 0, 0, 7); ctx.fill();
      }
      ctx.fillStyle = "#8a7a52";
      for (let a = 0; a < 7; a++) {
        const ang = (a / 7) * Math.PI * 2 + 0.4;
        ctx.beginPath(); ctx.ellipse(p.x + Math.cos(ang) * 12, p.y + Math.sin(ang) * 6 - 3, 5, 3.5, 0, 0, 7); ctx.fill();
      }
      A.label(ctx, p.x, p.y + 2, "▣", 9, "rgba(220,210,170,0.5)", "center");
    }
  }

  // --- the contracting storm wall (magenta-violet curtain + lightning) ------
  function stormWall(ctx, st, round, t, v) {
    if (!st) return;
    const c = st.center, R = st.ringR;
    // outer area beyond the ring: a translucent magenta curtain mask over the
    // consumed tiles is already drawn; here we draw the glowing WALL boundary.
    // Build the iso polygon of the ring edge (square in grid -> diamond on iso).
    const corners = [
      [c.cx - R, c.cy - R], [c.cx + R, c.cy - R], [c.cx + R, c.cy + R], [c.cx - R, c.cy + R],
    ].map(([gx, gy]) => iso(gx, gy));
    // glowing curtain rising from the wall
    ctx.save();
    ctx.beginPath();
    corners.forEach((p, i) => i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y));
    ctx.closePath();
    // wall stroke (pulsing)
    const pulse = A.reduced ? 0.6 : 0.5 + 0.4 * (Math.sin(t / 220) * 0.5 + 0.5);
    ctx.lineWidth = 5;
    ctx.strokeStyle = `rgba(214,80,255,${pulse})`;
    ctx.shadowColor = "rgba(200,70,255,0.9)"; ctx.shadowBlur = 18;
    ctx.stroke();
    ctx.lineWidth = 2; ctx.strokeStyle = `rgba(255,200,255,${pulse})`; ctx.shadowBlur = 0; ctx.stroke();
    ctx.restore();
    // lightning glyphs along the wall
    if (!A.reduced) {
      for (let s = 0; s < 4; s++) {
        const ph = (t / 700 + s * 0.25) % 1;
        const seg = Math.floor(ph * 4) % 4;
        const a = corners[seg], b = corners[(seg + 1) % 4];
        const lf = (ph * 4) % 1;
        const lx = A.lerp(a.x, b.x, lf), ly = A.lerp(a.y, b.y, lf);
        ctx.strokeStyle = "rgba(240,190,255,0.85)"; ctx.lineWidth = 1.6;
        ctx.beginPath();
        ctx.moveTo(lx, ly - 22);
        ctx.lineTo(lx + 4, ly - 12); ctx.lineTo(lx - 3, ly - 6); ctx.lineTo(lx + 3, ly + 2);
        ctx.stroke();
      }
    }
  }

  // --- rising embers on tiles outside the ring ------------------------------
  function embers(ctx, st, round, t) {
    if (!st || A.reduced) return;
    const c = st.center, R = st.ringR;
    for (let i = 0; i < 60; i++) {
      // distribute embers around the consumed border
      const ang = (i / 60) * Math.PI * 2;
      const rr = R + 0.6 + (i % 3) * 0.5;
      const gx = c.cx + Math.cos(ang) * rr, gy = c.cy + Math.sin(ang) * rr;
      if (gx < -1 || gx > GN || gy < -1 || gy > GN) continue;
      const p = iso(gx, gy);
      const ph = ((t / 22 + i * 40) % 60) / 60;
      const ey = p.y - ph * 26;
      ctx.fillStyle = `rgba(${230},${110 + i % 60},${255},${(1 - ph) * 0.7})`;
      ctx.fillRect(p.x + Math.sin(t / 200 + i) * 3, ey, 2, 2);
    }
  }

  // --- floating combat damage numbers on hunt:rival hits --------------------
  function damageNumbers(ctx, res, beat, v, t) {
    const bt = res.beats[beat];
    if (!bt || !bt.state || !bt.state.hit) return;
    const hit = bt.state.hit;
    const p = iso(hit.gx, hit.gy);
    // progress: on a live beat use beatT; once we're past this beat it's done.
    // (renderer only calls this for the *current* beat, so beatT is the clock.)
    const prog = v.over ? 1 : A.clamp(v.beatT, 0, 1);
    const rise = A.easeOut(prog) * 40;
    const alpha = prog < 0.12 ? prog / 0.12 : A.clamp(1 - (prog - 0.12) / 0.88, 0, 1);
    if (alpha <= 0.02) return;
    const cx = p.x, cy = p.y - 58 - rise;
    // impact spark burst at the strike point (early in the beat)
    if (prog < 0.4 && !A.reduced) {
      const sa = (1 - prog / 0.4);
      A.glow(ctx, p.x, p.y - 30, 26, `rgba(255,90,110,${0.35 * sa})`);
      ctx.strokeStyle = `rgba(255,150,160,${0.8 * sa})`; ctx.lineWidth = 2;
      for (let k = 0; k < 6; k++) {
        const a = (k / 6) * Math.PI * 2 + prog * 3;
        const r0 = 8 + prog * 14, r1 = 16 + prog * 22;
        ctx.beginPath(); ctx.moveTo(p.x + Math.cos(a) * r0, p.y - 30 + Math.sin(a) * r0 * 0.6);
        ctx.lineTo(p.x + Math.cos(a) * r1, p.y - 30 + Math.sin(a) * r1 * 0.6); ctx.stroke();
      }
    }
    // the number itself — big, red, kills shout louder
    ctx.save();
    ctx.globalAlpha = alpha;
    const txt = "-" + hit.dmg;
    const px = hit.kill ? 26 : 22;
    // outline for legibility over any background
    ctx.font = `800 ${px}px ui-monospace,"DejaVu Sans Mono",monospace`;
    ctx.textAlign = "center"; ctx.textBaseline = "alphabetic";
    ctx.lineWidth = 4; ctx.strokeStyle = "rgba(0,0,0,0.85)"; ctx.strokeText(txt, cx, cy);
    ctx.fillStyle = hit.kill ? "#ff4d5e" : "#ff8a4c"; ctx.fillText(txt, cx, cy);
    if (hit.kill) {
      ctx.lineWidth = 3; ctx.strokeStyle = "rgba(0,0,0,0.85)"; ctx.font = `800 13px ui-monospace,monospace`;
      ctx.strokeText("ELIMINATED", cx, cy + 16); ctx.fillStyle = "#ff4d5e"; ctx.fillText("ELIMINATED", cx, cy + 16);
    }
    ctx.restore();
  }

  // --- champion sprite (chibi) — body only; name plate drawn in a later pass -
  function championSprite(ctx, pc, e, t, res) {
    if (!e.alive) { gravestone(ctx, pc, e); return; }
    const shirt = e.col, soft = e.id === "A" ? "#5eead4" : "#c4b5fd";
    const cap = e.id === "A" ? "#059669" : "#6d28d9";
    A.sprite(ctx, {
      x: pc.x - 16, y: pc.y - 56, shirt, cap, skin: "#f3c79a",
      moving: pc.moving, t, nameCol: soft, // no name/tag here → no inline plate
    });
    // HP pips above head
    hpPips(ctx, pc.x, pc.y - 64, e);
    // gear chips (small weapon glints) if geared
    if (e.gear > 0) {
      for (let i = 0; i < Math.min(e.gear, 5); i++) {
        ctx.fillStyle = "#ffd23c";
        ctx.fillRect(pc.x - 14 + i * 6, pc.y - 70, 4, 4);
      }
      // weapon aura
      A.glow(ctx, pc.x, pc.y - 30, 22, "rgba(255,210,60," + Math.min(0.3, e.gear * 0.06) + ")");
    }
  }
  // Draw both champions' name/tag plates with collision avoidance: when the two
  // plates would overlap, push the lower (nearer-camera) one further down and
  // fade it slightly so the labels never z-fight.
  function drawChampPlates(ctx, plates, res) {
    // baseline plate top y for each = sprite foot (pc.y) + 6
    const items = plates.map((p) => ({
      x: p.pc.x, y: p.pc.y + 6, e: p.e,
      soft: p.e.id === "A" ? "#5eead4" : "#c4b5fd",
    })).sort((a, b) => a.y - b.y); // top-most first
    const PLATE_H = 30, PLATE_W = 92;
    for (let i = 0; i < items.length; i++) {
      const it = items[i];
      let alpha = 1, dy = 0;
      // if a previously-placed plate is close in x and above, drop this one
      for (let j = 0; j < i; j++) {
        const pj = items[j];
        if (Math.abs(pj.x - it.x) < PLATE_W && (it.y + dy) - pj.placedY < PLATE_H) {
          dy = (pj.placedY + PLATE_H) - it.y;
          alpha = 0.82;
        }
      }
      it.placedY = it.y + dy;
      champPlate(ctx, it.x, it.placedY, it.e, res, it.soft, alpha);
    }
  }
  function champPlate(ctx, x, y, e, res, soft, alpha) {
    const nm = e.name.toUpperCase();
    const sub = (res.tagOf ? res.tagOf(e.id) : "").toUpperCase();
    const w = Math.max(86, Math.max(nm.length, sub.length + 2) * 7.2);
    ctx.save();
    ctx.globalAlpha = alpha;
    // connector stub from sprite foot to plate when it was pushed down
    ctx.fillStyle = "rgba(8,16,30,0.94)"; A.rrect(ctx, x - w / 2, y, w, sub ? 28 : 18, 6); ctx.fill();
    ctx.strokeStyle = soft + "55"; ctx.lineWidth = 1; A.rrect(ctx, x - w / 2 + 0.5, y + 0.5, w - 1, (sub ? 28 : 18) - 1, 6); ctx.stroke();
    A.label(ctx, x, y + 12, nm, 9, soft, "center");
    if (sub) A.label(ctx, x, y + 23, sub, 7, "#8aa0bf", "center");
    ctx.restore();
  }
  function doctagAbbrev(res, id) {
    const tag = res.tagOf ? res.tagOf(id) : "";
    return tag;
  }
  function hpPips(ctx, x, y, e) {
    const w = 36, h = 5, frac = A.clamp(e.hp / e.maxhp, 0, 1);
    ctx.fillStyle = "rgba(8,16,30,0.85)"; A.rrect(ctx, x - w / 2 - 1, y - 1, w + 2, h + 2, 3); ctx.fill();
    ctx.fillStyle = frac > 0.5 ? "#34d399" : frac > 0.25 ? "#f59e0b" : "#fb5d5d";
    A.rrect(ctx, x - w / 2, y, Math.max(2, w * frac), h, 2); ctx.fill();
  }
  function npcSprite(ctx, pc, e, t) {
    if (!e.alive) { gravestone(ctx, pc, e); return; }
    // smaller, dimmer bot
    ctx.save();
    A.shadow(ctx, pc.x, pc.y + 6, 10, 4, 0.28);
    ctx.fillStyle = e.col; A.rrect(ctx, pc.x - 8, pc.y - 22, 16, 18, 5); ctx.fill();
    ctx.fillStyle = "rgba(255,255,255,0.15)"; ctx.fillRect(pc.x - 8, pc.y - 22, 16, 4);
    // head
    ctx.fillStyle = "#cbd5e1"; A.rrect(ctx, pc.x - 6, pc.y - 33, 12, 12, 4); ctx.fill();
    ctx.fillStyle = "#0f172a"; ctx.fillRect(pc.x - 3, pc.y - 28, 6, 3); // visor
    ctx.fillStyle = "#22d3ee"; ctx.fillRect(pc.x - 2, pc.y - 28, 4, 2);
    ctx.restore();
    // tiny hp bar
    const frac = A.clamp(e.hp / e.maxhp, 0, 1);
    ctx.fillStyle = "rgba(8,16,30,0.8)"; ctx.fillRect(pc.x - 11, pc.y - 40, 22, 4);
    ctx.fillStyle = frac > 0.4 ? "#9ca3af" : "#fb5d5d"; ctx.fillRect(pc.x - 10, pc.y - 39, 20 * frac, 2);
    A.label(ctx, pc.x, pc.y - 44, e.name.toUpperCase(), 6, "rgba(180,190,210,0.7)", "center");
  }
  function gravestone(ctx, pc, e) {
    A.shadow(ctx, pc.x, pc.y + 6, 12, 5, 0.3);
    ctx.fillStyle = "#3a4252";
    A.rrect(ctx, pc.x - 9, pc.y - 22, 18, 24, 8); ctx.fill();
    ctx.fillStyle = "#4a5568"; A.rrect(ctx, pc.x - 7, pc.y - 20, 14, 12, 6); ctx.fill();
    ctx.fillStyle = "#1f2733"; ctx.fillRect(pc.x - 1, pc.y - 16, 2, 8); ctx.fillRect(pc.x - 4, pc.y - 13, 8, 2);
    A.label(ctx, pc.x, pc.y + 12, e.name.toUpperCase(), 7, "rgba(160,170,190,0.7)", "center");
  }

  // --- HUD: per-survivor card -----------------------------------------------
  function hud(ctx, res, st, t) {
    const ents = st ? st.ents : [];
    const rows = [
      ["A", res.names.A, "#10b981", "#5eead4"],
      ["B", res.names.B, "#8b5cf6", "#c4b5fd"],
    ];
    ctx.fillStyle = "rgba(8,14,26,0.84)"; A.rrect(ctx, 12, 12, 250, 86, 10); ctx.fill();
    ctx.strokeStyle = "rgba(124,90,246,0.4)"; ctx.lineWidth = 1; A.rrect(ctx, 12.5, 12.5, 249, 85, 10); ctx.stroke();
    A.label(ctx, 24, 30, "STORMFALL ISLE", 10, "#c4b5fd", "left");
    rows.forEach(([id, nm, col, soft], i) => {
      const e = ents.find((x) => x.id === id) || { hp: 0, maxhp: 100, gear: 0, alive: false };
      const y = 46 + i * 26;
      A.label(ctx, 24, y + 6, nm.toUpperCase(), 10, e.alive ? soft : "#6b7280", "left");
      bar(ctx, 96, y, 92, 7, e.hp / e.maxhp, e.hp > 30 ? col : "#fb5d5d", "#0a1322");
      A.label(ctx, 196, y + 6, e.alive ? Math.round(e.hp) + "hp" : "DEAD", 9, e.alive ? soft : "#fb5d5d", "left");
      // gear icons
      for (let g = 0; g < Math.min(e.gear, 5); g++) { ctx.fillStyle = "#ffd23c"; ctx.fillRect(238 + g * 4, y - 1, 3, 7); }
    });
  }
  function bar(ctx, x, y, w, h, frac, col, bg) {
    ctx.fillStyle = bg; A.rrect(ctx, x, y, w, h, h / 2); ctx.fill();
    ctx.fillStyle = col; A.rrect(ctx, x, y, Math.max(2, w * A.clamp(frac, 0, 1)), h, h / 2); ctx.fill();
  }
  function survivorsBadge(ctx, st) {
    const n = st ? st.survivors : 5;
    const x = W - 132, y = 14;
    ctx.fillStyle = "rgba(8,14,26,0.84)"; A.rrect(ctx, x, y, 120, 42, 10); ctx.fill();
    ctx.strokeStyle = "rgba(244,63,94,0.5)"; ctx.lineWidth = 1; A.rrect(ctx, x + 0.5, y + 0.5, 119, 41, 10); ctx.stroke();
    A.label(ctx, x + 12, y + 18, "SURVIVORS", 9, "#94a3b8", "left");
    A.label(ctx, x + 12, y + 36, String(n), 18, "#f43f5e", "left");
    A.label(ctx, x + 108, y + 36, "alive", 8, "#64748b", "right");
  }

  // --- the safe-zone eye: current ring center + a REVEALED hidden FINAL eye --
  // The final safe-zone center is seeded-random. We render a faint cyan DRIFT
  // TRAIL from the board middle toward that final eye, plus a distinct crosshair
  // marker on the final eye itself — so a viewer can literally see why "rotate
  // early" pays off (it's the core hidden uncertainty).
  function eyeMarker(ctx, res, st, v, t) {
    if (!st) return;
    const fc = st.finalC || { cx: MID, cy: MID };
    const fp = iso(fc.cx, fc.cy);
    const midP = iso(MID, MID);

    // 1) DRIFT TRAIL — dotted path from board-mid to the hidden final eye.
    if (!A.reduced && (fc.cx !== MID || fc.cy !== MID)) {
      const segs = 16;
      for (let i = 0; i <= segs; i++) {
        const f = i / segs;
        const x = A.lerp(midP.x, fp.x, f), y = A.lerp(midP.y, fp.y, f);
        // a travelling brightness wave along the trail
        const wave = (Math.sin(t / 320 - f * 5) * 0.5 + 0.5);
        ctx.fillStyle = `rgba(94,234,212,${0.10 + wave * 0.30 * f})`;
        const r = 1.4 + wave * 1.6 * f;
        ctx.beginPath(); ctx.arc(x, y, r, 0, 7); ctx.fill();
      }
      // a small arrowhead near the final eye
      const ah = Math.atan2(fp.y - midP.y, fp.x - midP.x);
      const tipx = A.lerp(midP.x, fp.x, 0.86), tipy = A.lerp(midP.y, fp.y, 0.86);
      ctx.fillStyle = "rgba(94,234,212,0.55)";
      ctx.save(); ctx.translate(tipx, tipy); ctx.rotate(ah);
      ctx.beginPath(); ctx.moveTo(7, 0); ctx.lineTo(-4, -4); ctx.lineTo(-4, 4); ctx.closePath(); ctx.fill();
      ctx.restore();
    }

    // 2) FINAL EYE marker — distinct cyan crosshair + soft glow, always faintly
    //    visible (the destination the storm is collapsing toward).
    const fpulse = A.reduced ? 0.5 : 0.42 + 0.30 * (Math.sin(t / 300) * 0.5 + 0.5);
    A.glow(ctx, fp.x, fp.y, 26, `rgba(94,234,212,${0.10 + fpulse * 0.10})`);
    ctx.strokeStyle = `rgba(94,234,212,${fpulse})`; ctx.lineWidth = 1.6;
    ctx.beginPath(); ctx.arc(fp.x, fp.y, 9, 0, 7); ctx.stroke();
    // crosshair ticks
    ctx.beginPath();
    ctx.moveTo(fp.x - 14, fp.y); ctx.lineTo(fp.x - 6, fp.y);
    ctx.moveTo(fp.x + 6, fp.y); ctx.lineTo(fp.x + 14, fp.y);
    ctx.moveTo(fp.x, fp.y - 11); ctx.lineTo(fp.x, fp.y - 5);
    ctx.moveTo(fp.x, fp.y + 5); ctx.lineTo(fp.x, fp.y + 11);
    ctx.stroke();
    ctx.fillStyle = `rgba(94,234,212,${fpulse})`;
    ctx.beginPath(); ctx.arc(fp.x, fp.y, 2, 0, 7); ctx.fill();
    A.label(ctx, fp.x, fp.y - 18, "◎ FINAL EYE", 8, "rgba(140,245,225,0.9)", "center");

    // 3) CURRENT ring center — dashed, dimmer (where the storm sits THIS round).
    const c = st.center;
    const p = iso(c.cx, c.cy);
    if (Math.abs(c.cx - fc.cx) > 0.15 || Math.abs(c.cy - fc.cy) > 0.15) {
      const pulse = A.reduced ? 0.3 : 0.22 + 0.18 * (Math.sin(t / 350) * 0.5 + 0.5);
      ctx.strokeStyle = `rgba(168,200,230,${pulse})`; ctx.lineWidth = 1.2;
      ctx.setLineDash([4, 4]); ctx.lineDashOffset = -(t / 40) % 8;
      ctx.beginPath(); ctx.arc(p.x, p.y, 14, 0, 7); ctx.stroke(); ctx.setLineDash([]);
    }
  }

  function dispatcher(ctx, res, bt, v) {
    const h = 44, y = H - h;
    ctx.fillStyle = "rgba(5,9,16,0.92)"; ctx.fillRect(0, y, W, h);
    ctx.strokeStyle = "rgba(124,90,246,0.45)"; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(0, y + 0.5); ctx.lineTo(W, y + 0.5); ctx.stroke();
    A.label(ctx, 16, y + 18, "⚡ STORM-CAST", 10, "#c4b5fd", "left");
    let line = "Two survivors on a sinking storm isle. Read each prompt — who loots and hunts, who turtles and outlasts?";
    if (bt && bt.events && bt.events[0]) line = bt.events[0];
    if (v.over && res.beats.length) line = res.beats[res.beats.length - 1].events[0];
    A.wrap(ctx, line, 118, y + 18, W - 138, 14, 12, "#e9d5ff", "ui-monospace,monospace");
  }

  function finishOverlay(ctx, res, t) {
    ctx.fillStyle = "rgba(3,6,12,0.58)"; ctx.fillRect(0, 0, W, H);
    const draw = res.winner == null;
    const col = draw ? "#9aa2b6" : res.winner === "A" ? "#34d399" : "#a855f7";
    A.glow(ctx, W / 2, H / 2 - 20, 240, (draw ? "rgba(154,162,182," : res.winner === "A" ? "rgba(52,211,153," : "rgba(168,85,247,") + "0.2)");
    const ln = res.winner === "A" ? "B" : "A";
    const title = draw ? "DOUBLE KO" : "VICTORY ROYALE";
    const sub = draw ? "Both survivors fell the same tick"
      : res.winReason === "elim" ? res.names[res.winner] + " eliminates " + res.names[ln]
      : res.winReason === "survive" ? res.names[res.winner] + " outlasts the storm"
      : res.names[res.winner] + " stands strongest as the storm closes";
    // confetti rain
    if (!draw && !A.reduced) {
      const cols = ["#34d399", "#a855f7", "#ffd23c", "#22d3ee", "#fb7185", "#ffffff"];
      for (let i = 0; i < 90; i++) {
        const seedx = (i * 97) % W;
        const fall = ((t / 8 + i * 60) % (H + 40)) - 20;
        const sway = Math.sin(t / 300 + i) * 14;
        ctx.fillStyle = cols[i % cols.length];
        ctx.save(); ctx.translate(seedx + sway, fall); ctx.rotate((t / 200 + i) % 7);
        ctx.fillRect(-3, -3, 6, 6); ctx.restore();
      }
    }
    A.label(ctx, W / 2, H / 2 - 18, title, draw ? 38 : 40, col, "center", "ui-monospace,monospace");
    A.label(ctx, W / 2, H / 2 + 16, sub, 16, "#f1edff", "center");
    A.label(ctx, W / 2, H / 2 + 42, draw ? "draw" : res.names[res.winner] + " takes Stormfall Isle", 11, "#a8b3cf", "center");
  }

  function vignette(ctx) {
    const g = ctx.createRadialGradient(W / 2, H / 2, H * 0.32, W / 2, H / 2, H * 0.82);
    g.addColorStop(0, "rgba(0,0,0,0)"); g.addColorStop(1, "rgba(0,0,0,0.5)");
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
  }

  window.STORMFALL = {
    id: "stormfall", name: "Stormfall Isle", W, H,
    tag: "Battle-royale on a shrinking-storm voxel isle. Loot crates for gear, hunt your rival, or turtle the bunker — but the magenta storm wall contracts each round. Last survivor takes the royale.",
    champions: [
      { id: "A", name: "Vortex", color: "#10b981" },
      { id: "B", name: "Bunker", color: "#8b5cf6" },
    ],
    prompts: { A: DEF_A, B: DEF_B },
    mcp: {
      kickoff: "You are a survivor on Stormfall Isle, a refereed battle-royale played entirely through your tools. Each turn: get_state, legal_moves, then make_move with one action and the current ply. A magenta storm wall contracts every round toward a hidden safe-zone eye — stay in the ring, gear up, and be the last one standing. Win.",
      tools: [
        { name: "get_state", args: "", ret: "{round, hp, gear, in_zone, rival_hp, survivors}", desc: "Read the isle: your HP, gear, whether you're inside the safe ring, the rival's HP, survivors left." },
        { name: "legal_moves", args: "", ret: "[move, …], ply", desc: "Actions available from here — loot, hide, rotate, and hunt only if the rival is reachable." },
        { name: "make_move", args: "move, expected_ply", desc: "Take one action. loot:crate gains gear but exposes you; hide:bunker is safe; rotate:eye chases the zone; hunt:rival attacks.", ret: "new state | error" },
        { name: "resign", args: "", ret: "forfeit", desc: "Give up the isle." },
      ],
      vocab: "loot:crate · hide:bunker · rotate:eye · hunt:rival",
    },
    build, draw,
  };
})();
