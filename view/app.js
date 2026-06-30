/* Stormfall Isle spectator board. Polls ./state.json (the referee's live game
 * state) and renders the battle-royale: an iso voxel island with a contracting
 * magenta storm wall, two survivors (loot/hide/rotate/hunt), loot crates, bunkers,
 * the hidden seeded "final eye", HP/gear bars and live odds. Read-only and offline
 * — everything is drawn procedurally (no remote assets), like the chess board's
 * app.js. Dispatches on data.game so the same SPA shape generalises. */
(function () {
  const W = 780, H = 560;
  const cv = document.getElementById("c"), ctx = cv.getContext("2d");
  ctx.imageSmoothingEnabled = false;
  const statusEl = document.getElementById("status");

  // ---- iso island grid (mirrors pocs/games/stormfall/game.js projection) ----
  const GN = 9;
  const TILE = 40, TH = 20;
  const ISO_CX = W * 0.40, ISO_CY = 168;
  function iso(gx, gy) { return { x: ISO_CX + (gx - gy) * TILE, y: ISO_CY + (gx + gy) * TH }; }
  const MID = (GN - 1) / 2;

  const lerp = (a, b, t) => a + (b - a) * t;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  let data = null;            // latest state.json

  async function tick() {
    try {
      const r = await fetch("./state.json", { cache: "no-store" });
      const j = await r.json();
      if (j.game !== "stormfall") {
        statusEl.innerHTML = `<span class="off">unsupported game: ${j.game || "?"}</span>`;
        data = null;
      } else {
        data = j;
        const p = data.players;
        const live = (e) => e.alive ? `${e.hp}hp·g${e.gear}` : "DEAD";
        statusEl.textContent = data.winner
          ? `Final — ${data.winner} wins (${data.win_reason}).`
          : `Live · round ${data.round + 1}/${data.rounds} · ${p[0].handle} ${live(p[0])} vs ${p[1].handle} ${live(p[1])} · ${data.survivors} alive`;
      }
    } catch (e) {
      statusEl.innerHTML = `<span class="off">waiting for referee…</span>`;
    }
  }
  setInterval(tick, 1000); tick();

  // ---- drawing -------------------------------------------------------------
  function sky(t) {
    const g = ctx.createLinearGradient(0, 0, 0, H * 0.62);
    g.addColorStop(0, "#1b1340"); g.addColorStop(0.55, "#3a1f52"); g.addColorStop(1, "#6d2a86");
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    for (let i = 0; i < 40; i++) {
      const x = (i * 137) % W, y = (i * 53) % 150, tw = (Math.sin(t / 700 + i) + 1) / 2;
      ctx.fillStyle = `rgba(220,210,255,${0.10 + tw * 0.28})`; ctx.fillRect(x, y, 2, 2);
    }
    const sx = W * 0.78, sy = 120;
    glow(sx, sy, 80, "rgba(255,150,120,0.34)");
    ctx.fillStyle = "#ffd0a0"; ctx.beginPath(); ctx.arc(sx, sy, 22, 0, 7); ctx.fill();
  }
  function water(t) {
    const g = ctx.createLinearGradient(0, H * 0.5, 0, H);
    g.addColorStop(0, "#0d4a6e"); g.addColorStop(1, "#06243e");
    ctx.fillStyle = g; ctx.fillRect(0, H * 0.5, W, H * 0.5);
    for (let i = 0; i < 26; i++) {
      const y = H * 0.5 + i * 9;
      const a = 0.04 + 0.06 * (Math.sin(t / 600 + i) * 0.5 + 0.5);
      ctx.strokeStyle = `rgba(120,210,255,${a})`; ctx.lineWidth = 1.2;
      ctx.beginPath();
      for (let x = 0; x <= W; x += 36) { const yy = y + Math.sin(t / 500 + i + x / 90) * 3; x ? ctx.lineTo(x, yy) : ctx.moveTo(x, yy); }
      ctx.stroke();
    }
  }
  function islandBase() {
    const c1 = iso(GN - 0.4, -0.6), c2 = iso(GN - 0.4, GN - 0.4), c3 = iso(-0.6, GN - 0.4);
    const depth = 48;
    glow((c1.x + c3.x) / 2, (c1.y + c3.y) / 2 + 26, 300, "rgba(40,120,160,0.16)");
    ctx.fillStyle = "#7a5a34"; ctx.beginPath(); ctx.moveTo(c3.x, c3.y); ctx.lineTo(c2.x, c2.y); ctx.lineTo(c2.x, c2.y + depth); ctx.lineTo(c3.x, c3.y + depth); ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#5e4427"; ctx.beginPath(); ctx.moveTo(c2.x, c2.y); ctx.lineTo(c1.x, c1.y); ctx.lineTo(c1.x, c1.y + depth); ctx.lineTo(c2.x, c2.y + depth); ctx.closePath(); ctx.fill();
  }
  function diamond(x, y, col) {
    ctx.fillStyle = col;
    ctx.beginPath(); ctx.moveTo(x, y - TH); ctx.lineTo(x + TILE, y); ctx.lineTo(x, y + TH); ctx.lineTo(x - TILE, y); ctx.closePath(); ctx.fill();
  }
  function cellBase(gx, gy) {
    const edge = (gx <= 0 || gy <= 0 || gx >= GN - 1 || gy >= GN - 1);
    const r = ((gx * 31 + gy * 17) % 10) / 10;
    if (edge) return r < 0.5 ? "#d9c089" : "#cdb37a";
    return r < 0.5 ? "#3f9e54" : (r < 0.8 ? "#349049" : "#46a85d");
  }
  function floorTiles(t) {
    const center = data ? data.center : { cx: MID, cy: MID };
    const ringR = data ? data.ringR : GN;
    for (let gy = 0; gy < GN; gy++) for (let gx = 0; gx < GN; gx++) {
      const p = iso(gx, gy);
      diamond(p.x, p.y, cellBase(gx, gy));
      ctx.strokeStyle = "rgba(255,255,255,0.10)"; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(p.x - TILE, p.y); ctx.lineTo(p.x, p.y - TH); ctx.lineTo(p.x + TILE, p.y); ctx.stroke();
      const d = Math.max(Math.abs(gx - center.cx), Math.abs(gy - center.cy));
      if (d > ringR + 0.001) {
        diamond(p.x, p.y, "rgba(18,8,30,0.60)");
        const pulse = 0.28 + 0.16 * (Math.sin(t / 360 + gx + gy) * 0.5 + 0.5);
        diamond(p.x, p.y, `rgba(150,40,205,${pulse})`);
      }
    }
  }
  function cratesDraw(t) {
    if (!data) return;
    for (const c of data.crates) {
      const p = iso(c.gx, c.gy);
      if (c.taken) { ctx.globalAlpha = 0.5; box(p.x - 8, p.y - 7, 16, 9, "#5b4630", "#6e5638", "#3f3020"); ctx.globalAlpha = 1; continue; }
      shadow(p.x, p.y + 7, 12, 4, 0.26);
      box(p.x - 10, p.y - 11, 20, 12, "#b5862f", "#d6a23c", "#8c6622");
      const bob = Math.sin(t / 400 + c.gx) * 3;
      glow(p.x, p.y - 24 + bob, 12, "rgba(255,210,90,0.36)");
      gearIcon(p.x, p.y - 24 + bob, "#ffd23c");
    }
  }
  function bunkersDraw() {
    if (!data) return;
    for (const b of data.bunkers) {
      const p = iso(b.gx, b.gy);
      shadow(p.x, p.y + 7, 16, 5, 0.2);
      ctx.fillStyle = "#6f6242";
      for (let a = 0; a < 7; a++) { const ang = (a / 7) * Math.PI * 2; ctx.beginPath(); ctx.ellipse(p.x + Math.cos(ang) * 14, p.y + Math.sin(ang) * 7 + 2, 5, 3.5, 0, 0, 7); ctx.fill(); }
      ctx.fillStyle = "#8a7a52";
      for (let a = 0; a < 7; a++) { const ang = (a / 7) * Math.PI * 2 + 0.4; ctx.beginPath(); ctx.ellipse(p.x + Math.cos(ang) * 11, p.y + Math.sin(ang) * 5 - 3, 4.5, 3, 0, 0, 7); ctx.fill(); }
      label(p.x, p.y + 3, "▣", 9, "rgba(220,210,170,0.5)", "center");
    }
  }
  function stormWall(t) {
    if (!data) return;
    const c = data.center, R = data.ringR;
    const corners = [[c.cx - R, c.cy - R], [c.cx + R, c.cy - R], [c.cx + R, c.cy + R], [c.cx - R, c.cy + R]].map(([gx, gy]) => iso(gx, gy));
    ctx.save();
    ctx.beginPath(); corners.forEach((p, i) => i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)); ctx.closePath();
    const pulse = 0.5 + 0.4 * (Math.sin(t / 220) * 0.5 + 0.5);
    ctx.lineWidth = 5; ctx.strokeStyle = `rgba(214,80,255,${pulse})`;
    ctx.shadowColor = "rgba(200,70,255,0.9)"; ctx.shadowBlur = 18; ctx.stroke();
    ctx.lineWidth = 2; ctx.strokeStyle = `rgba(255,200,255,${pulse})`; ctx.shadowBlur = 0; ctx.stroke();
    ctx.restore();
  }
  function eyeMarker(t) {
    if (!data) return;
    const fc = data.finalC, fp = iso(fc.cx, fc.cy), midP = iso(MID, MID);
    if (fc.cx !== MID || fc.cy !== MID) {
      const segs = 14;
      for (let i = 0; i <= segs; i++) {
        const f = i / segs, x = lerp(midP.x, fp.x, f), y = lerp(midP.y, fp.y, f);
        const wave = (Math.sin(t / 320 - f * 5) * 0.5 + 0.5);
        ctx.fillStyle = `rgba(94,234,212,${0.08 + wave * 0.28 * f})`;
        ctx.beginPath(); ctx.arc(x, y, 1.3 + wave * 1.4 * f, 0, 7); ctx.fill();
      }
    }
    const fpulse = 0.42 + 0.3 * (Math.sin(t / 300) * 0.5 + 0.5);
    glow(fp.x, fp.y, 24, `rgba(94,234,212,${0.10 + fpulse * 0.10})`);
    ctx.strokeStyle = `rgba(94,234,212,${fpulse})`; ctx.lineWidth = 1.6;
    ctx.beginPath(); ctx.arc(fp.x, fp.y, 8, 0, 7); ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(fp.x - 13, fp.y); ctx.lineTo(fp.x - 5, fp.y); ctx.moveTo(fp.x + 5, fp.y); ctx.lineTo(fp.x + 13, fp.y);
    ctx.moveTo(fp.x, fp.y - 10); ctx.lineTo(fp.x, fp.y - 4); ctx.moveTo(fp.x, fp.y + 4); ctx.lineTo(fp.x, fp.y + 10); ctx.stroke();
    label(fp.x, fp.y - 16, "◎ FINAL EYE", 8, "rgba(140,245,225,0.9)", "center");
  }

  function entsDraw(t) {
    if (!data) return;
    const drawn = data.ents.map((e) => { const p = iso(e.gx, e.gy); return { e, x: p.x, y: p.y, sort: p.y }; });
    drawn.sort((a, b) => a.sort - b.sort);
    for (const d of drawn) {
      if (d.e.isNPC) npcSprite(d.x, d.y, d.e);
      else championSprite(d.x, d.y, d.e, t);
    }
    // champion name plates last
    for (const d of drawn) if (!d.e.isNPC && d.e.alive) champPlate(d.x, d.y + 6, d.e);
  }
  function championSprite(x, y, e, t) {
    if (!e.alive) { gravestone(x, y, e); return; }
    const soft = e.id === "A" ? "#5eead4" : "#c4b5fd";
    shadow(x, y + 6, 13, 5, 0.3);
    // body
    ctx.fillStyle = e.col; rrect(x - 9, y - 26, 18, 22, 6); ctx.fill();
    ctx.fillStyle = "rgba(255,255,255,0.16)"; ctx.fillRect(x - 9, y - 26, 18, 5);
    // head
    ctx.fillStyle = "#f3c79a"; rrect(x - 7, y - 38, 14, 13, 5); ctx.fill();
    ctx.fillStyle = e.id === "A" ? "#059669" : "#6d28d9"; rrect(x - 8, y - 40, 16, 6, 3); ctx.fill();
    // hp pips
    hpPips(x, y - 47, e);
    if (e.gear > 0) {
      for (let i = 0; i < Math.min(e.gear, 5); i++) { ctx.fillStyle = "#ffd23c"; ctx.fillRect(x - 12 + i * 6, y - 53, 4, 4); }
      glow(x, y - 18, 20, "rgba(255,210,60," + Math.min(0.3, e.gear * 0.06) + ")");
    }
    // last-move ring
    if (e.lastMove) { label(x, y - 58, moveGlyph(e.lastMove), 9, soft, "center"); }
  }
  function moveGlyph(m) {
    return m === "loot:crate" ? "▣ loot" : m === "hide:bunker" ? "⛨ hide" : m === "rotate:eye" ? "◎ rotate" : m === "hunt:rival" ? "✶ hunt" : "";
  }
  function champPlate(x, y, e) {
    const soft = e.id === "A" ? "#5eead4" : "#c4b5fd";
    const nm = e.name.toUpperCase();
    const w = Math.max(56, nm.length * 8);
    ctx.fillStyle = "rgba(8,16,30,0.94)"; rrect(x - w / 2, y, w, 16, 5); ctx.fill();
    ctx.strokeStyle = soft + "55"; ctx.lineWidth = 1; rrect(x - w / 2 + 0.5, y + 0.5, w - 1, 15, 5); ctx.stroke();
    label(x, y + 11, nm, 8, soft, "center");
  }
  function hpPips(x, y, e) {
    const w = 32, h = 4, frac = clamp(e.hp / e.maxhp, 0, 1);
    ctx.fillStyle = "rgba(8,16,30,0.85)"; rrect(x - w / 2 - 1, y - 1, w + 2, h + 2, 3); ctx.fill();
    ctx.fillStyle = frac > 0.5 ? "#34d399" : frac > 0.25 ? "#f59e0b" : "#fb5d5d";
    rrect(x - w / 2, y, Math.max(2, w * frac), h, 2); ctx.fill();
  }
  function npcSprite(x, y, e) {
    if (!e.alive) { gravestone(x, y, e); return; }
    shadow(x, y + 5, 9, 4, 0.28);
    ctx.fillStyle = e.col; rrect(x - 7, y - 19, 14, 16, 5); ctx.fill();
    ctx.fillStyle = "#cbd5e1"; rrect(x - 5, y - 29, 10, 11, 4); ctx.fill();
    ctx.fillStyle = "#22d3ee"; ctx.fillRect(x - 2, y - 25, 4, 2);
    const frac = clamp(e.hp / e.maxhp, 0, 1);
    ctx.fillStyle = "rgba(8,16,30,0.8)"; ctx.fillRect(x - 10, y - 35, 20, 4);
    ctx.fillStyle = frac > 0.4 ? "#9ca3af" : "#fb5d5d"; ctx.fillRect(x - 9, y - 34, 18 * frac, 2);
    label(x, y - 38, e.name.toUpperCase(), 6, "rgba(180,190,210,0.7)", "center");
  }
  function gravestone(x, y, e) {
    shadow(x, y + 5, 11, 5, 0.3);
    ctx.fillStyle = "#3a4252"; rrect(x - 8, y - 20, 16, 22, 7); ctx.fill();
    ctx.fillStyle = "#4a5568"; rrect(x - 6, y - 18, 12, 11, 5); ctx.fill();
    ctx.fillStyle = "#1f2733"; ctx.fillRect(x - 1, y - 14, 2, 7); ctx.fillRect(x - 4, y - 11, 8, 2);
    label(x, y + 12, e.name.toUpperCase(), 7, "rgba(160,170,190,0.7)", "center");
  }

  // ---- HUD ----------------------------------------------------------------
  function hud() {
    if (!data) return;
    const p = data.players;
    const rows = [[p[0], "#10b981", "#5eead4"], [p[1], "#8b5cf6", "#c4b5fd"]];
    ctx.fillStyle = "rgba(8,14,26,0.84)"; rrect(12, 46, 250, 86, 10); ctx.fill();
    ctx.strokeStyle = "rgba(168,85,247,0.4)"; ctx.lineWidth = 1; rrect(12.5, 46.5, 249, 85, 10); ctx.stroke();
    label(24, 64, "STORMFALL ISLE", 10, "#c4b5fd", "left");
    rows.forEach(([e, col, soft], i) => {
      const y = 80 + i * 26;
      label(24, y + 6, (e.name || e.handle).toUpperCase().slice(0, 10), 10, e.alive ? soft : "#6b7280", "left");
      bar(96, y, 92, 7, e.hp / e.maxhp, e.hp > 30 ? col : "#fb5d5d");
      label(196, y + 6, e.alive ? Math.round(e.hp) + "hp" : "DEAD", 9, e.alive ? soft : "#fb5d5d", "left");
      for (let g = 0; g < Math.min(e.gear, 5); g++) { ctx.fillStyle = "#ffd23c"; ctx.fillRect(238 + g * 4, y - 1, 3, 7); }
    });
  }
  function survivorsBadge() {
    const n = data ? data.survivors : 5;
    const x = W - 132, y = 46;
    ctx.fillStyle = "rgba(8,14,26,0.84)"; rrect(x, y, 120, 42, 10); ctx.fill();
    ctx.strokeStyle = "rgba(244,63,94,0.5)"; ctx.lineWidth = 1; rrect(x + 0.5, y + 0.5, 119, 41, 10); ctx.stroke();
    label(x + 12, y + 18, "SURVIVORS", 9, "#94a3b8", "left");
    label(x + 12, y + 36, String(n), 18, "#f43f5e", "left");
    label(x + 108, y + 36, "alive", 8, "#64748b", "right");
  }
  // top-center LIVE ODDS pill — kept clear of the HUD/survivors cards (which sit
  // lower, at y≈46) and any other top labels.
  function oddsPill() {
    const p = data ? data.players : [{ handle: "A" }, { handle: "B" }];
    const a = data ? clamp(data.odds_a, 0, 1) : 0.5;
    const pa = Math.round(a * 100), pb = 100 - pa;
    const bw = 248, x = (W - bw) / 2, yy = 7;
    ctx.fillStyle = "rgba(7,11,20,0.86)"; rrect(x, yy, bw, 30, 9); ctx.fill();
    label(W / 2, yy + 12, "◷ LIVE ODDS", 8, "#7C8AA0", "center");
    label(x + 12, yy + 12, p[0].handle.toUpperCase().slice(0, 8) + " " + pa + "%", 9, "#5eead4", "left");
    label(x + bw - 12, yy + 12, pb + "% " + p[1].handle.toUpperCase().slice(0, 8), 9, "#c4b5fd", "right");
    const aw = Math.max(2, (bw - 24) * a);
    ctx.fillStyle = "#10b981"; rrect(x + 12, yy + 18, aw, 7, 3); ctx.fill();
    ctx.fillStyle = "#8b5cf6"; rrect(x + 12 + aw, yy + 18, bw - 24 - aw, 7, 3); ctx.fill();
  }
  function dispatcher() {
    const h = 42, y = H - h;
    ctx.fillStyle = "rgba(5,9,16,0.92)"; ctx.fillRect(0, y, W, h);
    ctx.strokeStyle = "rgba(168,85,247,0.45)"; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(0, y + 0.5); ctx.lineTo(W, y + 0.5); ctx.stroke();
    label(16, y + 18, "⚡ STORM-CAST", 10, "#c4b5fd", "left");
    let line = "Two survivors on a sinking storm isle — who loots and hunts, who turtles and outlasts?";
    if (data && !data.winner) {
      const p = data.players;
      line = `Round ${data.round + 1}/${data.rounds} · ${data.to_move} to act · ${p[0].name} ${p[0].alive ? p[0].hp + "hp" : "down"} vs ${p[1].name} ${p[1].alive ? p[1].hp + "hp" : "down"}.`;
    } else if (data && data.winner) {
      line = data.win_reason === "double" ? "Double knockout — both survivors fell the same tick. Draw."
        : data.win_reason === "elim" ? `${data.winner} eliminates the rival — VICTORY ROYALE.`
        : `${data.winner} takes Stormfall Isle (${data.win_reason}).`;
    }
    wrap(line, 118, y + 18, W - 138, 14, 12, "#e9d5ff");
  }
  function finishOverlay() {
    if (!data || (!data.winner && data.status !== "double")) return;
    if (!data.winner && !["double"].includes(data.status)) return;
    ctx.fillStyle = "rgba(3,6,12,0.58)"; ctx.fillRect(0, 0, W, H);
    const draw = !data.winner;
    const col = draw ? "#9aa2b6" : data.players[0].handle === data.winner ? "#34d399" : "#a855f7";
    const title = draw ? "DOUBLE KO" : "VICTORY ROYALE";
    glow(W / 2, H / 2 - 16, 220, draw ? "rgba(154,162,182,0.2)" : "rgba(168,85,247,0.2)");
    label(W / 2, H / 2 - 12, title, draw ? 36 : 40, col, "center");
    const sub = draw ? "Both survivors fell the same tick"
      : data.win_reason === "elim" ? data.winner + " eliminates the rival"
      : data.win_reason === "survive" ? data.winner + " outlasts the storm"
      : data.winner + " stands strongest as the storm closes";
    label(W / 2, H / 2 + 18, sub, 16, "#f1edff", "center");
  }

  function frame(t) {
    sky(t); water(t); islandBase(); floorTiles(t);
    cratesDraw(t); bunkersDraw();
    stormWall(t);
    entsDraw(t);
    eyeMarker(t);
    hud(); survivorsBadge(); oddsPill(); dispatcher();
    finishOverlay();
    vignette();
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  // ---- tiny helpers --------------------------------------------------------
  function rrect(x, y, w, h, r) { ctx.beginPath(); ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r); ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath(); }
  function label(x, y, s, px, c, al) { ctx.fillStyle = c; ctx.textAlign = al || "left"; ctx.font = `700 ${px}px ui-monospace,monospace`; ctx.fillText(s, x, y); }
  function bar(x, y, w, h, f, c) { ctx.fillStyle = "#0a1322"; rrect(x, y, w, h, h / 2); ctx.fill(); ctx.fillStyle = c; rrect(x, y, Math.max(2, w * clamp(f, 0, 1)), h, h / 2); ctx.fill(); }
  function glow(x, y, r, c) { const g = ctx.createRadialGradient(x, y, 0, x, y, r); g.addColorStop(0, c); g.addColorStop(1, "rgba(0,0,0,0)"); ctx.fillStyle = g; ctx.beginPath(); ctx.arc(x, y, r, 0, 7); ctx.fill(); }
  function shadow(x, y, rx, ry, a) { ctx.fillStyle = `rgba(0,0,0,${a})`; ctx.beginPath(); ctx.ellipse(x, y, rx, ry, 0, 0, 7); ctx.fill(); }
  function box(x, y, w, h, top, light, dark) { ctx.fillStyle = dark; ctx.fillRect(x, y + 3, w, h); ctx.fillStyle = top; ctx.fillRect(x, y, w, h); ctx.fillStyle = light; ctx.fillRect(x, y, w, 3); }
  function gearIcon(x, y, col) { ctx.save(); ctx.translate(x, y); ctx.fillStyle = col; for (let i = 0; i < 8; i++) { ctx.rotate(Math.PI / 4); ctx.fillRect(-1.6, -6, 3.2, 3.6); } ctx.beginPath(); ctx.arc(0, 0, 4, 0, 7); ctx.fill(); ctx.fillStyle = "#7a5a10"; ctx.beginPath(); ctx.arc(0, 0, 1.6, 0, 7); ctx.fill(); ctx.restore(); }
  function wrap(text, x, y, maxw, lh, px, c) {
    ctx.fillStyle = c; ctx.textAlign = "left"; ctx.font = `700 ${px}px ui-monospace,monospace`;
    const words = String(text).split(" "); let line = "", yy = y;
    for (const w of words) { const test = line ? line + " " + w : w; if (ctx.measureText(test).width > maxw && line) { ctx.fillText(line, x, yy); line = w; yy += lh; } else line = test; }
    if (line) ctx.fillText(line, x, yy);
  }
  function vignette() { const g = ctx.createRadialGradient(W / 2, H / 2, H * 0.32, W / 2, H / 2, H * 0.82); g.addColorStop(0, "rgba(0,0,0,0)"); g.addColorStop(1, "rgba(0,0,0,0.5)"); ctx.fillStyle = g; ctx.fillRect(0, 0, W, H); }
})();
