// LINE WARS — read-only spectator SPA (Graphwar-style artillery free-for-all for AIWars).
//
// Polls ./state.json (same origin → the view gateway forwards it to this match's referee)
// ~1×/sec and renders the battlefield onto a <canvas>. The world is BIGGER than the screen:
// a pan/zoom camera auto-follows the action (the active shooter / the flying shot), and you can
// DRAG to look around and SCROLL to zoom — a minimap in the corner shows where everyone is.
// Dispatches on `data.game === "line-wars"`.

const el = (id) => document.getElementById(id);
const canvas = el("field");
const ctx = canvas.getContext("2d");
const GOLD = "#ffd23f";

// Distinct per-player color via the golden-angle hue spread (stable for index 0..29).
function colorFor(i) {
  return `hsl(${((i * 137.508) % 360).toFixed(1)}, 85%, 62%)`;
}

const MEMES = {
  hit_enemy: ["GET REKT", "BOOM 💥", "HEADSHOT", "+1 FRAG", "CALCULATED", "MATH CHECKS OUT", "NO SCOPE", "DOUBLE KILL?"],
  friendly_fire: ["TEAM KILL 💀", "OOPS", "SKILL ISSUE", "OWN GOAL", "WHY THO", "FRIENDLY FIRE!", "BRO…"],
  blocked: ["BLOCKED, L", "OOF, ROCK", "ABSORBED", "DENIED", "TANKY", "WALL SAYS NO"],
  off_field: ["AIR BALL", "MISSED LOL", "INTO THE VOID", "BIG WHIFF", "OUT OF BOUNDS", "TRY AGAIN"],
  exploded: ["IT EXPLODED 🤡", "DOMAIN ERROR", "NaN MOMENT", "SQRT OF SADNESS", "LMAO", "DIV BY SKILL"],
};

let data = null;
let view = { x0: -25, x1: 25, y0: -15, y1: 15 }; // field bounds (from state)
let dpr = 1;

// Camera: cx/cy = world point at screen center; zoom multiplies the fit-to-screen scale, so
// zoom>1 means the world is bigger than the viewport (you pan to see the rest).
const cam = { cx: 0, cy: 0, zoom: 1.9, follow: true, inited: false };
const ZOOM_MIN = 0.85, ZOOM_MAX = 7;

let shotKey = null, shotAnim = null;
let bursts = [], floats = [], pulse = 0;

// ---- camera / transform ----------------------------------------------------

function fit() {
  const r = canvas.getBoundingClientRect();
  dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.max(1, Math.round(r.width * dpr));
  canvas.height = Math.max(1, Math.round(r.height * dpr));
}

function baseScale() {
  const pad = 10 * dpr;
  return Math.min((canvas.width - 2 * pad) / (view.x1 - view.x0),
                  (canvas.height - 2 * pad) / (view.y1 - view.y0));
}

function tx() {
  const scale = baseScale() * cam.zoom;
  const W = canvas.width, H = canvas.height;
  return { scale, X: (wx) => W / 2 + (wx - cam.cx) * scale, Y: (wy) => H / 2 - (wy - cam.cy) * scale };
}

// canvas client px → world coords
function screenToWorld(clientX, clientY) {
  const r = canvas.getBoundingClientRect();
  const px = (clientX - r.left) * dpr, py = (clientY - r.top) * dpr;
  const t = tx();
  return { x: cam.cx + (px - canvas.width / 2) / t.scale, y: cam.cy - (py - canvas.height / 2) / t.scale };
}

function clampCam() {
  const mx = (view.x1 - view.x0) * 0.3, my = (view.y1 - view.y0) * 0.3;
  cam.cx = Math.max(view.x0 - mx, Math.min(view.x1 + mx, cam.cx));
  cam.cy = Math.max(view.y0 - my, Math.min(view.y1 + my, cam.cy));
}

// ---- drawing ---------------------------------------------------------------

function niceStep(span) {
  const raw = span / 12, pow = Math.pow(10, Math.floor(Math.log10(raw)));
  for (const m of [1, 2, 2.5, 5, 10]) if (raw <= m * pow) return m * pow;
  return 10 * pow;
}

function drawGrid(t) {
  const W = canvas.width, H = canvas.height;
  const gs = niceStep(view.x1 - view.x0);
  ctx.lineWidth = 1 * dpr;
  for (let gx = Math.ceil(view.x0 / gs) * gs; gx <= view.x1; gx += gs) {
    const px = t.X(gx);
    ctx.strokeStyle = Math.abs(gx) < 1e-6 ? "rgba(255,255,255,.20)" : "rgba(255,255,255,.06)";
    ctx.beginPath(); ctx.moveTo(px, t.Y(view.y0)); ctx.lineTo(px, t.Y(view.y1)); ctx.stroke();
  }
  for (let gy = Math.ceil(view.y0 / gs) * gs; gy <= view.y1; gy += gs) {
    const py = t.Y(gy);
    ctx.strokeStyle = Math.abs(gy) < 1e-6 ? "rgba(255,255,255,.20)" : "rgba(255,255,255,.06)";
    ctx.beginPath(); ctx.moveTo(t.X(view.x0), py); ctx.lineTo(t.X(view.x1), py); ctx.stroke();
  }
  // field border
  ctx.strokeStyle = "rgba(255,255,255,.18)"; ctx.lineWidth = 2 * dpr;
  ctx.strokeRect(t.X(view.x0), t.Y(view.y1), (view.x1 - view.x0) * t.scale, (view.y1 - view.y0) * t.scale);
}

function drawObstacles(t) {
  for (const o of data.obstacles || []) {
    const px = t.X(o.x), py = t.Y(o.y), r = o.r * t.scale;
    const g = ctx.createRadialGradient(px - r * .3, py - r * .3, r * .2, px, py, r);
    g.addColorStop(0, "#5a5f6e"); g.addColorStop(1, "#23262f");
    ctx.fillStyle = g;
    ctx.beginPath(); ctx.arc(px, py, r, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = "#0008"; ctx.lineWidth = 2 * dpr; ctx.stroke();
    if (r > 12 * dpr) {
      ctx.font = `${r * 1.1}px serif`; ctx.textAlign = "center"; ctx.textBaseline = "middle";
      ctx.fillText("🪨", px, py + r * .05);
    }
  }
}

function drawSoldiers(t) {
  const sh = data.shooter;
  for (const pl of data.players || []) {
    const color = colorFor(pl.index);
    for (const s of pl.soldiers || []) {
      const px = t.X(s.x), py = t.Y(s.y), r = Math.max(5 * dpr, 0.85 * t.scale);
      const active = !!sh && sh.player === pl.index && sh.index === s.index && s.alive;

      if (active) {
        const ring = r + (3 + 2 * Math.sin(pulse * 6)) * dpr;
        ctx.strokeStyle = GOLD; ctx.lineWidth = 3 * dpr;
        ctx.shadowColor = GOLD; ctx.shadowBlur = 16 * dpr;
        ctx.beginPath(); ctx.arc(px, py, ring, 0, Math.PI * 2); ctx.stroke();
        ctx.shadowBlur = 0;
      }

      ctx.beginPath(); ctx.arc(px, py, r, 0, Math.PI * 2);
      ctx.fillStyle = s.alive ? color : "#2a2a2e";
      ctx.globalAlpha = s.alive ? 0.9 : 0.5; ctx.fill(); ctx.globalAlpha = 1;
      ctx.lineWidth = 2.2 * dpr; ctx.strokeStyle = s.alive ? "#0007" : "#555"; ctx.stroke();

      if (!s.alive) {
        ctx.strokeStyle = "#aaa"; ctx.lineWidth = 2 * dpr; const d = r * 0.5;
        ctx.beginPath();
        ctx.moveTo(px - d, py - d); ctx.lineTo(px + d, py + d);
        ctx.moveTo(px + d, py - d); ctx.lineTo(px - d, py + d); ctx.stroke();
      } else if (r > 9 * dpr) {
        ctx.font = `${r * 1.1}px serif`; ctx.textAlign = "center"; ctx.textBaseline = "middle";
        ctx.fillText("😎", px, py + r * .08);
      } else {
        ctx.fillStyle = "#0009";
        ctx.beginPath(); ctx.arc(px, py, r * .28, 0, Math.PI * 2); ctx.fill();
      }

      if (active && r > 7 * dpr) {
        ctx.font = `900 ${11 * dpr}px Impact, "Arial Black", sans-serif`;
        ctx.fillStyle = GOLD; ctx.textAlign = "center";
        ctx.fillText("▼ UP", px, py - r - 11 * dpr + Math.sin(pulse * 5) * 2 * dpr);
      }
    }
  }
}

function drawShot(t) {
  if (!shotAnim) return;
  const pts = shotAnim.points;
  if (pts.length < 2) return;
  const m = shotAnim.muzzle;
  const lHead = Math.round(m - shotAnim.t * m);
  const rHead = Math.round(m + shotAnim.t * (pts.length - 1 - m));

  ctx.lineJoin = "round"; ctx.lineCap = "round";
  ctx.strokeStyle = shotAnim.color; ctx.globalAlpha = 0.35;
  ctx.lineWidth = 9 * dpr; ctx.shadowColor = shotAnim.color; ctx.shadowBlur = 18 * dpr;
  strokeRange(t, pts, lHead, rHead);
  ctx.globalAlpha = 1; ctx.strokeStyle = "#fff"; ctx.lineWidth = 2.5 * dpr; ctx.shadowBlur = 8 * dpr;
  strokeRange(t, pts, lHead, rHead);
  ctx.shadowBlur = 0;

  if (!shotAnim.done) {
    for (const h of [lHead, rHead]) {
      const p = pts[Math.max(0, Math.min(pts.length - 1, h))];
      ctx.fillStyle = "#fff"; ctx.shadowColor = shotAnim.color; ctx.shadowBlur = 16 * dpr;
      ctx.beginPath(); ctx.arc(t.X(p[0]), t.Y(p[1]), 4 * dpr, 0, Math.PI * 2); ctx.fill();
      ctx.shadowBlur = 0;
    }
  }
}

function strokeRange(t, pts, a, b) {
  a = Math.max(0, a); b = Math.min(pts.length - 1, b);
  if (b <= a) return;
  ctx.beginPath();
  ctx.moveTo(t.X(pts[a][0]), t.Y(pts[a][1]));
  for (let i = a + 1; i <= b; i++) ctx.lineTo(t.X(pts[i][0]), t.Y(pts[i][1]));
  ctx.stroke();
}

function drawBursts(t) {
  for (const b of bursts) {
    const r = (6 + b.age * 90) * dpr, a = Math.max(0, 1 - b.age * 1.6);
    ctx.globalAlpha = a; ctx.strokeStyle = GOLD; ctx.lineWidth = 3 * dpr;
    ctx.beginPath(); ctx.arc(t.X(b.x), t.Y(b.y), r, 0, Math.PI * 2); ctx.stroke();
    ctx.globalAlpha = Math.min(1, a + 0.2);
    ctx.font = `${Math.min(44, 16 + b.age * 60) * dpr}px serif`;
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.fillText("💥", t.X(b.x), t.Y(b.y));
    ctx.globalAlpha = 1;
  }
}

function drawFloats(t) {
  for (const f of floats) {
    const a = Math.max(0, 1 - f.age * 0.8);
    ctx.globalAlpha = a;
    ctx.font = `900 ${20 * dpr}px Impact, "Arial Black", sans-serif`;
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    const px = t.X(f.x), py = t.Y(f.y) - f.age * 40 * dpr;
    ctx.lineWidth = 4 * dpr; ctx.strokeStyle = "#000a"; ctx.strokeText(f.text, px, py);
    ctx.fillStyle = f.color; ctx.fillText(f.text, px, py);
    ctx.globalAlpha = 1;
  }
}

// Corner minimap: the whole field shrunk, every soldier as a dot, plus the current viewport box.
function drawMinimap(t) {
  const W = canvas.width, H = canvas.height;
  const fw = view.x1 - view.x0, fh = view.y1 - view.y0;
  const mw = Math.min(W * 0.26, 210 * dpr), mh = mw * (fh / fw);
  const mx = W - mw - 10 * dpr, my = 10 * dpr;
  const MX = (wx) => mx + (wx - view.x0) / fw * mw;
  const MY = (wy) => my + (view.y1 - wy) / fh * mh;

  ctx.globalAlpha = 0.82; ctx.fillStyle = "#0b0d14"; ctx.strokeStyle = "#ffffff22";
  ctx.lineWidth = 1 * dpr;
  roundRect(mx, my, mw, mh, 6 * dpr); ctx.fill(); ctx.stroke();
  ctx.globalAlpha = 1;

  for (const o of data.obstacles || []) {
    ctx.fillStyle = "#ffffff18";
    ctx.beginPath(); ctx.arc(MX(o.x), MY(o.y), Math.max(1.5, o.r / fw * mw), 0, Math.PI * 2); ctx.fill();
  }
  for (const pl of data.players || []) {
    for (const s of pl.soldiers || []) {
      if (!s.alive) continue;
      ctx.fillStyle = colorFor(pl.index);
      ctx.beginPath(); ctx.arc(MX(s.x), MY(s.y), 2.2 * dpr, 0, Math.PI * 2); ctx.fill();
    }
  }
  // viewport rectangle
  const halfW = canvas.width / 2 / t.scale, halfH = canvas.height / 2 / t.scale;
  ctx.strokeStyle = GOLD; ctx.lineWidth = 1.5 * dpr;
  const rx = MX(cam.cx - halfW), ry = MY(cam.cy + halfH);
  ctx.strokeRect(rx, ry, (2 * halfW) / fw * mw, (2 * halfH) / fh * mh);
}

function roundRect(x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath();
}

function drawHud2() {
  // pan/zoom hint, lower-left
  ctx.globalAlpha = 0.5; ctx.fillStyle = "#cfd6e6";
  ctx.font = `${11 * dpr}px ui-sans-serif, system-ui, sans-serif`;
  ctx.textAlign = "left"; ctx.textBaseline = "bottom";
  ctx.fillText(cam.follow ? "drag to look · scroll to zoom" : "drag to look · scroll to zoom · dblclick to re-follow",
    10 * dpr, canvas.height - 8 * dpr);
  ctx.globalAlpha = 1;
}

function drawWinner() {
  if (!data || data.status !== "win" || !data.winner) return;
  const W = canvas.width, H = canvas.height;
  ctx.fillStyle = "rgba(6,7,12,.66)"; ctx.fillRect(0, 0, W, H);
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  const win = (data.players || []).find((p) => p.handle === data.winner);
  ctx.font = `${64 * dpr}px serif`; ctx.fillText("🏆", W / 2, H / 2 - 52 * dpr);
  ctx.font = `900 ${Math.min(50, W / dpr / 12) * dpr}px Impact, "Arial Black", sans-serif`;
  ctx.lineWidth = 6 * dpr; ctx.strokeStyle = "#000b";
  ctx.strokeText(`${data.winner} WINS`, W / 2, H / 2 + 8 * dpr);
  ctx.fillStyle = win ? colorFor(win.index) : "#fff"; ctx.fillText(`${data.winner} WINS`, W / 2, H / 2 + 8 * dpr);
  ctx.font = `900 ${20 * dpr}px Impact, "Arial Black", sans-serif`;
  ctx.fillStyle = "#fff"; ctx.fillText("gg ez 📈", W / 2, H / 2 + 48 * dpr);
}

// ---- main render loop ------------------------------------------------------

let lastTs = 0;
function frame(ts) {
  const dt = Math.min(0.05, (ts - lastTs) / 1000 || 0); lastTs = ts; pulse += dt;
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  if (data && data.game === "line-wars") {
    // auto-follow the action unless the viewer took manual control
    if (cam.follow) {
      const tgt = followTarget();
      if (tgt) { cam.cx += (tgt.x - cam.cx) * Math.min(1, dt * 4); cam.cy += (tgt.y - cam.cy) * Math.min(1, dt * 4); clampCam(); }
    }
    const t = tx();
    drawGrid(t);
    drawObstacles(t);
    drawShot(t);
    drawSoldiers(t);
    drawBursts(t);
    drawFloats(t);
    drawMinimap(t);
    drawHud2();
    drawWinner();

    if (shotAnim && !shotAnim.done) {
      shotAnim.t += dt / 0.8;
      if (shotAnim.t >= 1) { shotAnim.t = 1; shotAnim.done = true; onShotLanded(); }
    }
    bursts.forEach((b) => (b.age += dt)); bursts = bursts.filter((b) => b.age < 0.9);
    floats.forEach((f) => (f.age += dt)); floats = floats.filter((f) => f.age < 1.5);
  }
  requestAnimationFrame(frame);
}

// What the camera tracks: the flying shot's muzzle during a shot, else the active shooter.
function followTarget() {
  if (shotAnim && !shotAnim.done && data.last_shot && data.last_shot.from)
    return { x: data.last_shot.from[0], y: data.last_shot.from[1] };
  if (data.shooter) return { x: data.shooter.x, y: data.shooter.y };
  return null;
}

function onShotLanded() {
  const sh = data && data.last_shot;
  if (!sh) return;
  for (const k of sh.kills || []) {
    const pl = (data.players || [])[k.player];
    const sol = pl && pl.soldiers && pl.soldiers[k.index];
    if (sol) bursts.push({ x: sol.x, y: sol.y, age: 0 });
  }
  const list = MEMES[sh.outcome] || ["NICE"];
  const text = list[(data.ply || 0) % list.length];
  floats.push({ x: sh.from[0], y: sh.from[1], text, color: sh.outcome === "hit_enemy" ? GOLD : colorFor(sh.by_player) });
  el("meme").textContent = text;
  el("meme").style.color = sh.outcome === "hit_enemy" ? GOLD : "#fff";
}

// ---- DOM leaderboard -------------------------------------------------------

function renderBoard() {
  const players = (data.players || []).slice().sort((a, b) => b.alive - a.alive || a.index - b.index);
  el("board").innerHTML = players.map((p) => {
    const out = p.alive === 0, active = data.turn === p.index && data.status === "playing";
    return `<span class="chip ${active ? "active" : ""} ${out ? "out" : ""}">
      <span class="dot" style="color:${colorFor(p.index)}">●</span>
      <span class="nm">${escapeHtml(p.handle)}${p.resigned ? " 🏳️" : ""}</span>
      <span class="ct">${p.alive}/${p.total}</span></span>`;
  }).join("");
}

function renderHud() {
  renderBoard();
  el("ply").textContent = "ply " + (data.ply ?? 0);
  const sh = data.last_shot;
  el("chalk").innerHTML = sh
    ? `${escapeHtml(sh.by_handle)} fired&nbsp; f(x) = <b>${escapeHtml(sh.func)}</b>`
    : `f(x) = <b>awaiting first shot…</b>`;
  const st = el("status");
  if (data.status === "win") st.textContent = `🏆 ${data.winner} wins`;
  else if (data.status === "draw") st.textContent = "🤝 draw — mutual annihilation";
  else st.textContent = `${data.turn_handle} to fire · ${data.squads_standing} squads standing`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

// ---- pan / zoom input ------------------------------------------------------

let drag = null;
canvas.addEventListener("pointerdown", (e) => {
  drag = { x: e.clientX, y: e.clientY, cx: cam.cx, cy: cam.cy };
  cam.follow = false;
  canvas.setPointerCapture(e.pointerId);
});
canvas.addEventListener("pointermove", (e) => {
  if (!drag) return;
  const t = tx();
  cam.cx = drag.cx - (e.clientX - drag.x) * dpr / t.scale;
  cam.cy = drag.cy + (e.clientY - drag.y) * dpr / t.scale;
  clampCam();
});
const endDrag = () => { drag = null; };
canvas.addEventListener("pointerup", endDrag);
canvas.addEventListener("pointercancel", endDrag);
canvas.addEventListener("dblclick", () => { cam.follow = true; });
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const before = screenToWorld(e.clientX, e.clientY);
  const factor = Math.exp(-e.deltaY * 0.0012);
  cam.zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, cam.zoom * factor));
  const after = screenToWorld(e.clientX, e.clientY);
  cam.cx += before.x - after.x; cam.cy += before.y - after.y;
  cam.follow = false; clampCam();
}, { passive: false });

// ---- polling ---------------------------------------------------------------

function ingest(next) {
  data = next;
  if (data.field) {
    view = { x0: data.field.x_min, x1: data.field.x_max, y0: data.field.y_min, y1: data.field.y_max };
    if (!cam.inited) { cam.cx = (view.x0 + view.x1) / 2; cam.cy = (view.y0 + view.y1) / 2; cam.inited = true; }
  }
  const sh = data.last_shot;
  const key = sh ? `${data.ply}:${sh.outcome}:${sh.func}` : null;
  if (key && key !== shotKey) {
    shotKey = key;
    const pts = (sh.points && sh.points.length >= 2) ? sh.points : [sh.from, sh.from];
    const m = Math.max(0, Math.min(pts.length - 1, sh.muzzle_index || 0));
    shotAnim = { points: pts, muzzle: m, color: colorFor(sh.by_player), t: 0, done: false };
  }
  renderHud();
}

async function poll() {
  try {
    const res = await fetch("./state.json", { cache: "no-store" });
    if (res.ok) ingest(await res.json());
    else el("status").innerHTML = `<span class="off">referee says ${res.status}</span>`;
  } catch (e) {
    el("status").innerHTML = `<span class="off">offline — retrying…</span>`;
  }
}

window.addEventListener("resize", fit);
fit();
poll();
setInterval(poll, 900);
requestAnimationFrame(frame);
