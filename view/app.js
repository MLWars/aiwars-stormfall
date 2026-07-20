// AIWars nim — spectator SPA + optional SEAT MODE (epic #317 human play).
//
// Polls ./state.json (same origin → the view gateway forwards it to this match's referee)
// ~1×/sec and redraws the pile. Dispatches on `data.game` — the only renderer here is nim
// (draws from `data.pile` / `data.players`). A new game = a new renderer branch.
//
// REPLAY MODE (?replay): instead of polling, fetch the recorded frame sequence once and play
// the frames through the SAME renderer, with transport controls. Bare `?replay`/`?replay=1`
// fetches ./replay.json (pod mode); any other value is a same-origin manifest URL; `?replay=
// bridge` lets the site's universal ReplayPlayer push frames over the postMessage bridge.
//
// SEAT MODE (epic #317): when the site's play page embeds this view it opens the standard
// postMessage bridge (hello/ready + nonce). The PARENT holds the seat credential and long-polls
// the referee's Seat API; it pushes each private SeatState in here and we post move REQUESTS
// (button taps) back out. This frame never sees the token — it can only ask the parent to act
// for its own seat. Spectator (and replay) behaviour is byte-identical until a handshake happens.

const el = (id) => document.getElementById(id);

function esc(s) {
  return String(s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );
}

// ---- the pile ----------------------------------------------------------------------
// A stable set of pebble nodes: growing appends fresh stones (with a pop), shrinking scales
// the removed ones away then drops them. Stable so live/replay/seat all animate cleanly, and
// a replay rewind (pile grows again) just re-adds stones.
let pileNodes = [];

function drawPile(n) {
  const box = el("pile");
  n = Math.max(0, n | 0);
  while (pileNodes.length < n) {
    const s = document.createElement("div");
    s.className = "stone pop";
    box.appendChild(s);
    pileNodes.push(s);
  }
  while (pileNodes.length > n) {
    const s = pileNodes.pop();
    s.classList.remove("pop");
    s.classList.add("gone");
    // Drop the node after its exit transition; a bounded timeout, never an event we might miss.
    setTimeout(() => s.remove(), 260);
  }
}

function renderPlate(id, p, data, over, winnerHandle) {
  const seat = el(id);
  const nm = seat.querySelector(".nm");
  nm.textContent = (p && p.handle) || "—";
  const isTurn = !over && !!p && data.to_act === p.handle;
  const isWin = over && !!p && winnerHandle && p.handle === winnerHandle;
  seat.classList.toggle("turn", isTurn);
  seat.classList.toggle("win", isWin);
}

// Nicely label a move-string for a seat-mode action button.
function labelMove(mv) {
  if (mv.startsWith("take:")) return `Take ${mv.slice(5)}`;
  return mv;
}

// The one renderer. `ctx` is null for spectator/replay, or { heroSeat, turn, pending } in seat mode.
function renderNim(data, ctx) {
  const players = data.players || [];
  if (players.length < 2) return;
  const botSeat = ctx && ctx.heroSeat != null ? ctx.heroSeat : 0;
  const topSeat = 1 - botSeat;
  const over = data.status === "over";
  const winner = data.winner || null;

  el("stones").textContent = `${data.pile ?? "—"} stones`;
  renderPlate("seat-top", players[topSeat], data, over, winner);
  renderPlate("seat-bot", players[botSeat], data, over, winner);
  drawPile(data.pile ?? 0);

  // Seat-mode action bar: the TAKE buttons for the seat's legal moves, only on its turn.
  const actions = el("actions");
  actions.innerHTML = "";
  actions.classList.remove("on");
  const myTurn = ctx && ctx.turn && ctx.turn.your_turn && !ctx.pending && !over;
  if (myTurn) {
    actions.classList.add("on");
    for (const mv of ctx.turn.moves || []) {
      const b = document.createElement("button");
      b.textContent = labelMove(mv);
      b.onclick = () => submitMove(mv);
      actions.appendChild(b);
    }
  }

  // Endgame banner + status line.
  const fin = el("fin");
  const status = el("status");
  const lastLog = Array.isArray(data.log) && data.log.length ? data.log[data.log.length - 1] : "";
  if (over) {
    fin.classList.add("on");
    fin.querySelector(".big").textContent = winner ? `${winner} wins` : "Draw";
    fin.querySelector(".sub").textContent = lastLog;
    status.innerHTML = winner
      ? `<span class="win">Game over — ${esc(winner)} took the last stone.</span>`
      : `<span class="win">Game over — draw.</span>`;
    return;
  }
  fin.classList.remove("on");

  const log = lastLog ? ` · <span class="off">${esc(lastLog)}</span>` : "";
  if (ctx) {
    const errTxt = seatErr ? ` · <span class="off">${esc(seatErr)}</span>` : "";
    if (ctx.pending) {
      status.innerHTML = `<span class="sent">Move sent…</span>${log}`;
    } else if (ctx.turn && ctx.turn.your_turn) {
      status.innerHTML = `<span class="you">Your move.</span>${log}${errTxt}`;
    } else {
      status.innerHTML = `Waiting for <b>${esc(data.to_act || "opponent")}</b>…${log}${errTxt}`;
    }
  } else {
    status.innerHTML = data.to_act
      ? `<b>${esc(data.to_act)}</b> to move${log}`
      : `${esc(lastLog)}`;
  }
}

// ---- Seat mode (epic #317): the standard AIWars view bridge ------------------------

let seat = null; // { nonce, post } once the parent's hello lands
let seatData = null; // last pushed SeatState { you, turn, status, state, game_over }
let seatErr = null; // last rejected-action message (cleared on the next state)
let pending = null; // the move just posted over the bridge — held as a "move sent" cue
let pendTimer = null; // bounded fallback so a dropped reply can't wedge the board sending

let bridge = null; // replay bridge { nonce, post } once the parent's replay hello lands

function drawSeat() {
  if (!seatData || !seatData.state) return;
  renderNim(seatData.state, {
    heroSeat: seatData.you ? seatData.you.seat : null,
    turn: seatData.turn,
    pending,
  });
}

window.addEventListener("message", (e) => {
  const d = e.data;
  if (!d || typeof d !== "object" || typeof d.nonce !== "string") return;
  if (d.type === "aiwars:hello") {
    // Bridge replay: the parent (the site's ReplayPlayer) opens with mode:"replay" then drives
    // playback by pushing frames; `replay: true` is the capability its probe waits for.
    if (BRIDGE && d.mode === "replay") {
      const src = e.source;
      if (!src) return;
      bridge = { nonce: d.nonce, post: (m) => src.postMessage(Object.assign({ nonce: d.nonce }, m), "*") };
      bridge.post({ type: "aiwars:ready", replay: true });
      return;
    }
    // Replay playback is a pure recording — never let a bridge handshake turn it into a seat.
    if (REPLAY_SRC) return;
    const src = e.source;
    if (!src) return;
    seat = { nonce: d.nonce, post: (m) => src.postMessage(Object.assign({ nonce: d.nonce }, m), "*") };
    // Every legal move is a button here → advertise full in-view controls so the play page can
    // retire its duplicate move chips for this game.
    seat.post({ type: "aiwars:ready", controls: "full" });
    return;
  }
  // Bridge frames: render the pushed recording through the live renderer.
  if (bridge && d.nonce === bridge.nonce) {
    if (d.type === "aiwars:frame" && d.state && typeof d.state === "object") {
      if (d.state.game === "nim") renderNim(d.state, null);
    }
    return;
  }
  if (!seat || d.nonce !== seat.nonce) return;
  if (d.type === "aiwars:state" && d.state && typeof d.state === "object") {
    const prevPly = seatData && seatData.turn ? seatData.turn.ply : null;
    seatData = d.state;
    seatErr = null;
    const mine = seatData.turn && seatData.turn.your_turn;
    const ply = seatData.turn ? seatData.turn.ply : null;
    // Our submitted move settled once the world advanced (ply moved) or it's no longer our turn.
    if (pending && (ply !== prevPly || !mine)) clearPending();
    if (seatData.state && seatData.state.game === "nim") drawSeat();
    return;
  }
  if (d.type === "aiwars:action_result") {
    // Any reply ends the "move sent" state; a rejection surfaces the referee's message.
    clearPending();
    if (d.ok === false) seatErr = String(d.message || d.error || "move rejected");
    drawSeat();
  }
});

// Submit a move over the seat bridge and enter the "move sent" state until the referee replies
// (aiwars:action_result) or the next aiwars:state lands. A bounded fallback (house law) makes
// sure a dropped reply can't wedge the board sending.
function submitMove(mv) {
  if (!seat) return;
  seat.post({ type: "aiwars:action", mv }); // the parent holds the token and acts for us
  pending = mv;
  if (pendTimer) clearTimeout(pendTimer);
  pendTimer = setTimeout(() => {
    pendTimer = null;
    if (!pending) return;
    seatErr = "no reply from the referee — try again";
    pending = null;
    drawSeat();
  }, 10000);
  drawSeat();
}

function clearPending() {
  pending = null;
  if (pendTimer) {
    clearTimeout(pendTimer);
    pendTimer = null;
  }
}

// ---- live polling ------------------------------------------------------------------
async function tick() {
  // Seat mode renders from the parent's pushed private states; keep the poll as a dormant
  // fallback only (it would overwrite seat affordances with public data).
  if (seat && seatData) return;
  try {
    const res = await fetch("./state.json", { cache: "no-store" });
    if (!res.ok) throw new Error("HTTP " + res.status);
    const data = await res.json();
    if (data.game === "nim") renderNim(data, null);
    else el("status").innerHTML = `<span class="off">unsupported game: ${esc(data.game || "?")}</span>`;
  } catch (e) {
    el("status").innerHTML = `<span class="off">waiting for match… (${esc(e.message || e)})</span>`;
  }
}

// ---- Replay mode -------------------------------------------------------------------
function replaySource() {
  const v = new URLSearchParams(location.search).get("replay");
  if (v === null) return null; // not in replay mode
  if (v === "" || v === "1") return "./replay.json"; // pod mode
  if (v === "bridge") return "bridge"; // parent-driven playback, no fetch
  if (v.startsWith("/") && !v.includes("//") && !v.includes(":")) return v; // same-origin only
  return "./replay.json";
}
const REPLAY_SRC = replaySource();
const BRIDGE = REPLAY_SRC === "bridge";

const replay = {
  frames: [],
  at: -1,
  playing: false,
  timer: null,
  stepMs: 1200,
};

function showFrame(i) {
  if (!replay.frames.length) return;
  replay.at = Math.max(0, Math.min(i, replay.frames.length - 1));
  renderNim(replay.frames[replay.at].state, null);
  el("pos").textContent = `${replay.at + 1}/${replay.frames.length}`;
  const seek = el("seek");
  seek.max = String(replay.frames.length - 1);
  seek.value = String(replay.at);
}

function setPlaying(on) {
  if (on && replay.at >= replay.frames.length - 1) replay.at = -1;
  replay.playing = on && replay.frames.length > 1;
  el("play").textContent = replay.playing ? "⏸" : "▶";
  clearInterval(replay.timer);
  if (replay.playing) {
    replay.timer = setInterval(() => {
      if (replay.at >= replay.frames.length - 1) setPlaying(false);
      else showFrame(replay.at + 1);
    }, replay.stepMs);
  }
}

async function loadReplay() {
  try {
    const res = await fetch(REPLAY_SRC, { cache: "no-store" });
    if (!res.ok) throw new Error("HTTP " + res.status);
    const manifest = await res.json();
    if (manifest.game !== "nim") {
      el("status").innerHTML = `<span class="off">unsupported game: ${esc(manifest.game || "?")}</span>`;
      return;
    }
    replay.frames = (manifest.frames || []).filter((f) => f && f.state);
    if (!replay.frames.length) throw new Error("no frames yet");
    el("controls").style.display = "flex";
    showFrame(0);
    setPlaying(true);
  } catch (e) {
    el("status").innerHTML = `<span class="off">waiting for replay… (${esc(e.message || e)})</span>`;
    setTimeout(loadReplay, 2000);
  }
}

function bindControls() {
  el("play").onclick = () => setPlaying(!replay.playing);
  el("prev").onclick = () => {
    setPlaying(false);
    showFrame(replay.at - 1);
  };
  el("next").onclick = () => {
    setPlaying(false);
    showFrame(replay.at + 1);
  };
  el("first").onclick = () => {
    setPlaying(false);
    showFrame(0);
  };
  el("last").onclick = () => {
    setPlaying(false);
    showFrame(replay.frames.length - 1);
  };
  el("seek").oninput = (e) => {
    setPlaying(false);
    showFrame(+e.target.value);
  };
  el("speed").onchange = (e) => {
    replay.stepMs = +e.target.value;
    if (replay.playing) setPlaying(true);
  };
}

if (BRIDGE) {
  el("status").innerHTML = `<span class="off">replay loading…</span>`;
} else if (REPLAY_SRC) {
  bindControls();
  loadReplay();
} else {
  tick();
  setInterval(tick, 1000);
}
