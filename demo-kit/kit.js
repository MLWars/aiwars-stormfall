/* AIWars POC kit — shared canvas toolkit + playback harness + MCP panel.
 *
 * A game registers a config with AW.mount(root, game). The harness draws the
 * surrounding chrome (prompts, the 4 MCP tools, a live tool-call feed, odds),
 * runs the deterministic beat playback, and calls game.draw(ctx, view) each frame.
 *
 * Faithful to the real engine contract (engine/crates/mcp-warden): a minigame is
 * a turn-based Game with opaque move-strings; every agent plays through the same
 * four MCP tools — get_state, legal_moves, make_move(mv, expected_ply), resign —
 * and its PROMPT decides which legal move it picks each turn. The kit renders
 * exactly that loop in the MCP panel.
 */
(function () {
  const AW = (window.AW = window.AW || {});

  // ---- deterministic PRNG (mulberry32) + string hash -----------------------
  AW.rng = function (seed) {
    let a = seed >>> 0;
    return function () {
      a |= 0; a = (a + 0x6d2b79f5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  };
  AW.hashSeed = function (s) {
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
    return h >>> 0;
  };
  AW.lerp = (a, b, t) => a + (b - a) * t;
  AW.clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
  AW.ease = (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2); // easeInOutQuad
  AW.easeOut = (t) => 1 - Math.pow(1 - t, 3);
  AW.reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  AW.pick = function (arr, n, rng) {
    const a = arr.slice();
    for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(rng() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; }
    return a.slice(0, n);
  };

  // ---- canvas drawing primitives (ported from the interview arena) ----------
  AW.tile = (c, x, y, w, h, col) => { c.fillStyle = col; c.fillRect(x, y, w, h); };
  AW.rrect = function (c, x, y, w, h, r) {
    c.beginPath();
    c.moveTo(x + r, y);
    c.arcTo(x + w, y, x + w, y + h, r);
    c.arcTo(x + w, y + h, x, y + h, r);
    c.arcTo(x, y + h, x, y, r);
    c.arcTo(x, y, x + w, y, r);
    c.closePath();
  };
  AW.label = function (c, x, y, t, px, col, align = "left", font) {
    c.fillStyle = col; c.textAlign = align; c.textBaseline = "alphabetic";
    c.font = `700 ${px}px ${font || 'ui-monospace,"DejaVu Sans Mono",monospace'}`;
    c.fillText(t, x, y);
  };
  AW.wrap = function (c, text, x, y, maxW, lh, px, col, font) {
    c.fillStyle = col; c.textAlign = "left";
    c.font = `700 ${px}px ${font || 'ui-monospace,"DejaVu Sans Mono",monospace'}`;
    let line = "", yy = y;
    for (const word of String(text).split(" ")) {
      const test = line ? `${line} ${word}` : word;
      if (c.measureText(test).width > maxW && line) { c.fillText(line, x, yy); line = word; yy += lh; }
      else line = test;
    }
    if (line) c.fillText(line, x, yy);
    return yy;
  };
  // An axis-aligned 3D box seen from front-upper-right (the voxel look).
  AW.box = function (c, x, y, w, h, d, front, top, side) {
    c.fillStyle = top;
    c.beginPath(); c.moveTo(x, y); c.lineTo(x + w, y); c.lineTo(x + w + d, y - d); c.lineTo(x + d, y - d); c.closePath(); c.fill();
    c.fillStyle = side;
    c.beginPath(); c.moveTo(x + w, y); c.lineTo(x + w + d, y - d); c.lineTo(x + w + d, y - d + h); c.lineTo(x + w, y + h); c.closePath(); c.fill();
    AW.tile(c, x, y, w, h, front);
  };
  AW.shadow = function (c, x, y, rx, ry, alpha = 0.3) {
    c.fillStyle = `rgba(0,0,0,${alpha})`; c.beginPath(); c.ellipse(x, y, rx, ry, 0, 0, 7); c.fill();
  };
  AW.glow = function (c, x, y, r, col) {
    const g = c.createRadialGradient(x, y, 0, x, y, r);
    g.addColorStop(0, col); g.addColorStop(1, col.replace(/[\d.]+\)$/, "0)"));
    c.fillStyle = g; c.beginPath(); c.arc(x, y, r, 0, 7); c.fill();
  };

  // A chunky chibi champion sprite (≈36×60) with a walk cycle + name tag.
  // opts: {x,y,shirt,cap,skin,name,tag,moving,t,nameCol}
  AW.sprite = function (c, o) {
    const t = AW.reduced ? 0 : o.t || 0;
    const bob = o.moving || AW.reduced ? 0 : Math.sin(t / 640 + o.x) * 1.4;
    const x = o.x, y = (o.y || 0) + bob;
    AW.shadow(c, x + 16, y + 60, 16, 5, 0.32);
    c.fillStyle = (o.shirt || "#10b981") + "22";
    c.beginPath(); c.ellipse(x + 16, y + 60, 20, 6, 0, 0, 7); c.fill();
    let l1 = 8, l2 = 18;
    if (o.moving) { const f = Math.floor(t / 150) % 2; l1 = f ? 6 : 10; l2 = f ? 20 : 16; }
    AW.tile(c, l1 + x - 8, y + 44, 8, 14, "#1F2937");
    AW.tile(c, l2 + x - 8, y + 44, 8, 14, "#1F2937");
    AW.tile(c, x + 6, y + 56, 12, 5, "#111827");
    AW.tile(c, x + 16, y + 56, 12, 5, "#111827");
    AW.tile(c, x + 4, y + 26, 26, 22, o.shirt || "#10b981");
    c.fillStyle = "rgba(255,255,255,.14)"; c.fillRect(x + 4, y + 26, 26, 4);
    if (o.name) AW.label(c, x + 17, y + 43, o.name.charAt(0).toUpperCase(), 11, "rgba(255,255,255,.6)", "center");
    AW.tile(c, x - 2, y + 28, 7, 14, o.skin || "#f3c79a");
    AW.tile(c, x + 29, y + 28, 7, 14, o.skin || "#f3c79a");
    c.fillStyle = o.skin || "#f3c79a"; AW.rrect(c, x + 3, y + 2, 28, 26, 8); c.fill();
    c.fillStyle = o.cap || "#059669"; AW.rrect(c, x + 1, y - 2, 32, 12, 6); c.fill();
    c.fillRect(x + 22, y + 6, 14, 5);
    c.fillStyle = "#23303F"; c.fillRect(x + 11, y + 14, 4, 5); c.fillRect(x + 20, y + 14, 4, 5);
    if (o.name) {
      const nm = o.name.slice(0, 14).toUpperCase(); const sub = (o.tag || "").toUpperCase();
      const w = Math.max(66, Math.max(nm.length, sub.length + 2) * 6.6);
      c.fillStyle = "rgba(8,16,30,.92)"; AW.rrect(c, x + 16 - w / 2, y + 62, w, sub ? 28 : 18, 5); c.fill();
      AW.label(c, x + 16, y + 74, nm, 9, o.nameCol || "#5eead4", "center");
      if (sub) AW.label(c, x + 16, y + 85, sub, 7, "#8aa0bf", "center");
    }
  };

  // A reusable star/moon night sky into a gradient. cols=[top,mid,horizon]
  AW.nightSky = function (c, w, ground, t, cols) {
    const [top, mid, hor] = cols || ["#070b1e", "#1a1440", "#3a2356"];
    const g = c.createLinearGradient(0, 0, 0, ground);
    g.addColorStop(0, top); g.addColorStop(0.55, mid); g.addColorStop(1, hor);
    c.fillStyle = g; c.fillRect(0, 0, w, ground);
    for (let i = 0; i < 46; i++) {
      const x = (i * 137) % w, y = (i * 53) % Math.min(190, ground);
      const tw = (Math.sin(t / 700 + i) + 1) / 2;
      c.fillStyle = `rgba(200,220,255,${0.12 + tw * 0.34})`;
      c.fillRect(x, y, 2, 2);
    }
  };

  // ---- the Arena harness ----------------------------------------------------
  const BASE = 5200; // ms per beat at 1×

  function el(tag, cls, html) { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; }
  function esc(s) { return String(s).replace(/[&<>]/g, (m) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[m])); }

  AW.mount = function (root, game) {
    const url = new URL(location.href);
    const qp = (k, d) => (url.searchParams.has(k) ? url.searchParams.get(k) : d);
    const seed = game.seed != null ? game.seed : (qp("seed") != null ? +qp("seed") : Math.floor(Math.random() * 1e6));
    const freezeBeat = qp("beat") != null ? +qp("beat") : null; // screenshot a fixed beat
    const freezeT = qp("t") != null ? +qp("t") : 0.7;
    let speed = qp("speed") != null ? +qp("speed") : 1;
    let playing = qp("play") != null ? qp("play") !== "0" : true;

    let result = game.build(seed, { prompts: game.prompts });
    const beats = result.beats;
    const champs = game.champions; // [{id,name,color}]

    // ----- DOM scaffold -----
    root.innerHTML = "";
    const wrap = el("div", "aw-wrap");
    const head = el("div", "aw-head");
    head.appendChild(el("div", null,
      `<div class="aw-title">${esc(game.name)} <span class="dot">●</span></div><div class="aw-tag">${esc(game.tag || "")}</div>`));
    const meta = el("div", "aw-meta");
    const seedEl = el("span", "aw-seed", `seed #${seed}`);
    const badge = el("span", "aw-badge live", `<span class="pulse"></span> Live`);
    meta.appendChild(seedEl); meta.appendChild(badge); head.appendChild(meta);
    wrap.appendChild(head);

    const stage = el("div", "aw-stage");
    const left = el("div", "aw-left");
    const shell = el("div", "aw-canvas-shell");
    const canvas = el("canvas", "aw-canvas");
    canvas.width = game.W; canvas.height = game.H;
    canvas.setAttribute("role", "img");
    canvas.setAttribute("aria-label", game.name + " — " + (game.tag || ""));
    shell.appendChild(canvas); left.appendChild(shell);

    // controls
    const controls = el("div", "aw-controls");
    const playBtn = el("button", "aw-btn primary", playing ? "⏸ Pause" : "▶ Play");
    const stepBtn = el("button", "aw-btn", "⏭ Step");
    const newBtn = el("button", "aw-btn", "↻ New match");
    const spd = el("div", "aw-speed");
    [0.5, 1, 2, 4].forEach((s) => {
      const b = el("button", s === speed ? "on" : "", s + "×");
      b.onclick = () => { speed = s; [...spd.children].forEach((c2) => c2.classList.remove("on")); b.classList.add("on"); };
      spd.appendChild(b);
    });
    controls.appendChild(playBtn); controls.appendChild(stepBtn); controls.appendChild(newBtn);
    controls.appendChild(el("div", "aw-spacer"));
    controls.appendChild(el("span", "aw-seed", "speed")); controls.appendChild(spd);
    left.appendChild(controls);
    stage.appendChild(left);

    // right panel
    const panel = el("aside", "aw-panel");
    const promptCard = el("div", "aw-card");
    promptCard.appendChild(el("h3", null, '<span class="ic">◆</span> Champions · public prompt'));
    champs.forEach((ch) => {
      const p = result.promptOf ? result.promptOf(ch.id) : (game.prompts && game.prompts[ch.id]) || "";
      const pr = el("div", "aw-prompt " + ch.id);
      pr.innerHTML = `<div class="who">${esc(ch.name)} · ${esc(result.tagOf ? result.tagOf(ch.id) : "")}</div><div class="txt">${p}</div>`;
      promptCard.appendChild(pr);
    });
    panel.appendChild(promptCard);

    // live betting market — placed right under the prompts so it's always in view
    const oddsCard = el("div", "aw-card");
    oddsCard.appendChild(el("h3", null, '<span class="ic">◷</span> Outright market · live odds'));
    const odds = el("div", "aw-odds");
    const oddsRows = {};
    champs.forEach((ch) => {
      const row = el("div", "row");
      const sw = el("span", "sw"); sw.style.background = ch.color;
      const nm = el("span", "nm", esc(ch.name)); nm.style.color = ch.id === "A" ? "var(--A-soft)" : (ch.id === "B" ? "var(--B-soft)" : "var(--ink)");
      const bar = el("span", "bar"); const fill = el("span", "fill"); fill.style.background = ch.color; bar.appendChild(fill);
      const pct = el("span", "pct", "—");
      row.appendChild(sw); row.appendChild(nm); row.appendChild(bar); row.appendChild(pct);
      odds.appendChild(row); oddsRows[ch.id] = { fill, pct };
    });
    oddsCard.appendChild(odds);
    oddsCard.appendChild(el("div", "aw-foot", "pari-mutuel · implied chance from live game state · favourites soft, never locked"));
    panel.appendChild(oddsCard);

    const toolCard = el("div", "aw-card");
    toolCard.appendChild(el("h3", null, '<span class="ic">⚙</span> MCP interface · how the AI plays'));
    if (game.mcp && game.mcp.kickoff) {
      toolCard.appendChild(el("div", "aw-prompt", `<div class="who" style="color:var(--brand)">referee → agent · kickoff</div><div class="txt">${esc(game.mcp.kickoff)}</div>`));
    }
    (game.mcp ? game.mcp.tools : []).forEach((tool) => {
      const tl = el("div", "aw-tool",
        `<span class="nm">${esc(tool.name)}</span><span class="args">(${esc(tool.args || "")})</span> <span class="ret">→ ${esc(tool.ret || "")}</span><span class="desc">${esc(tool.desc || "")}</span>`);
      toolCard.appendChild(tl);
    });
    if (game.mcp && game.mcp.vocab) {
      toolCard.appendChild(el("div", "aw-foot", `move vocabulary · <span style="font-family:var(--mono);color:var(--accent)">${esc(game.mcp.vocab)}</span>`));
    }
    panel.appendChild(toolCard);

    const feedCard = el("div", "aw-card");
    feedCard.appendChild(el("h3", null, '<span class="ic">▸</span> Live tool-call feed'));
    const feed = el("div", "aw-feed"); feedCard.appendChild(feed);
    panel.appendChild(feedCard);

    stage.appendChild(panel);
    wrap.appendChild(stage);
    root.appendChild(wrap);

    const ctx = canvas.getContext("2d");
    ctx.imageSmoothingEnabled = false;

    // ----- MCP feed: render a beat's get_state→legal_moves→make_move triplet --
    let feededTo = -1;
    function renderFeedUpTo(b) {
      if (b <= feededTo) return;
      for (let k = feededTo + 1; k <= b && k < beats.length; k++) appendBeatToFeed(beats[k]);
      feededTo = b;
      feed.scrollTop = feed.scrollHeight;
    }
    function appendBeatToFeed(bt) {
      const seat = bt.agent;
      const obs = typeof bt.observe === "object" ? JSON.stringify(bt.observe).replace(/"/g, "") : bt.observe;
      if (bt.thought) addLn(seat, `<span class="thought">“${esc(bt.thought)}”</span>`);
      addLn(seat, `<span class="call">get_state</span><span class="ret">() → ${esc(obs || "")}</span>`);
      if (bt.legal) addLn(seat, `<span class="call">legal_moves</span><span class="ret">() → [${bt.legal.map((m) => `<span class="arg">${esc(m)}</span>`).join(", ")}]</span>`);
      const okc = bt.ok === false ? "bad" : "ok";
      addLn(seat, `<span class="call">make_move</span>(<span class="arg">"${esc(bt.move)}"</span>, ply ${bt.ply}) → <span class="${okc}">${esc(bt.result || "ok")}</span>`);
    }
    function addLn(seat, html) {
      const ln = el("div", "ln");
      ln.appendChild(el("span", "seat " + (seat === "ref" ? "ref" : seat), seat === "ref" ? "R" : seat));
      ln.appendChild(el("span", "body", html));
      feed.appendChild(ln);
    }
    function updateOdds(b) {
      const o = result.oddsAt ? result.oddsAt(b) : null;
      champs.forEach((ch) => {
        const v = o ? Math.round(o[ch.id]) : Math.round(100 / champs.length);
        oddsRows[ch.id].fill.style.width = v + "%";
        oddsRows[ch.id].pct.textContent = v + "%";
      });
    }

    // ----- playback state -----
    let beat = freezeBeat != null ? freezeBeat : 0;
    let beatStart = performance.now();
    renderFeedUpTo(beat); updateOdds(beat);

    function setBadge() {
      const over = beat >= beats.length;
      badge.className = "aw-badge " + (over ? "final" : "live");
      badge.innerHTML = over ? "Final" : '<span class="pulse"></span> Live';
    }
    setBadge();

    playBtn.onclick = () => { playing = !playing; playBtn.textContent = playing ? "⏸ Pause" : "▶ Play"; if (playing) beatStart = performance.now(); };
    stepBtn.onclick = () => { advance(); };
    newBtn.onclick = () => { AW.mount(root, Object.assign({}, game, { seed: Math.floor(Math.random() * 1e6) })); };

    function advance() {
      if (beat >= beats.length) return;
      beat++;
      beatStart = performance.now();
      renderFeedUpTo(beat - 1 >= 0 ? beat - 1 : 0);
      if (beat - 1 < beats.length) renderFeedUpTo(beat - 1);
      updateOdds(Math.min(beat, beats.length - 1));
      setBadge();
    }

    // a compact, conflict-free live-odds pill the harness paints over EVERY game
    // (top-center of the canvas) so the bet is visible inside the spectacle.
    function drawOddsHud(c, o) {
      if (!o || champs.length !== 2) return;
      const a = AW.clamp(Math.round(o[champs[0].id]), 0, 100), b = 100 - a;
      const bw = 248, bh = 30, x = (game.W - bw) / 2, y = 7;
      c.save();
      c.fillStyle = "rgba(7,11,20,.82)"; AW.rrect(c, x, y, bw, bh, 9); c.fill();
      c.strokeStyle = "rgba(34,211,238,.30)"; c.lineWidth = 1; AW.rrect(c, x + .5, y + .5, bw - 1, bh - 1, 9); c.stroke();
      AW.label(c, game.W / 2, y + 12, "◷ LIVE ODDS", 8, "#7C8AA0", "center");
      AW.label(c, x + 12, y + 12, champs[0].name.toUpperCase() + " " + a + "%", 9, "#5eead4", "left");
      AW.label(c, x + bw - 12, y + 12, b + "% " + champs[1].name.toUpperCase(), 9, "#c4b5fd", "right");
      const bx = x + 12, by = y + 17, bwid = bw - 24, hh = 7, aw = Math.max(2, Math.min(bwid - 2, bwid * a / 100));
      c.fillStyle = "#0a1322"; AW.rrect(c, bx, by, bwid, hh, 3.5); c.fill();
      c.fillStyle = champs[0].color; AW.rrect(c, bx, by, aw, hh, 3.5); c.fill();
      c.fillStyle = champs[1].color; AW.rrect(c, bx + aw, by, bwid - aw, hh, 3.5); c.fill();
      c.fillStyle = "rgba(7,11,20,.9)"; c.fillRect(bx + aw - 0.5, by - 1, 1.5, hh + 2);
      c.restore();
    }

    let raf = 0;
    function frame(now) {
      const dur = BASE / speed;
      let beatT;
      if (freezeBeat != null) { beatT = freezeT; }
      else {
        beatT = AW.clamp((now - beatStart) / dur, 0, 1);
        if (playing && beat < beats.length && now - beatStart >= dur) { advance(); beatT = 0; }
      }
      const over = beat >= beats.length;
      game.draw(ctx, {
        now, t: now, result, beats, beat: Math.min(beat, beats.length - 1),
        rawBeat: beat, beatT, over, playing, champs, AW,
      });
      if (!game.noOddsHud) drawOddsHud(ctx, result.oddsAt ? result.oddsAt(Math.min(beat, beats.length - 1)) : null);
      raf = requestAnimationFrame(frame);
    }
    raf = requestAnimationFrame(frame);

    return { stop: () => cancelAnimationFrame(raf) };
  };
})();
