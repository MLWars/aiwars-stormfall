// AIWars replay bridge — the VIEW side (epic #317, "replay for all games — the
// same framework for all"). Loaded before app.js; inert unless the page was
// opened as `?replay=bridge`, which is how the site's universal ReplayPlayer
// embeds this view for pod-free playback. In that mode the PARENT owns the
// transport (play/pause/step/scrub) and there is no live referee behind the
// page, so the view must not poll ./state.json — it renders exactly the frames
// pushed in:
//
//   parent → view  { type: "aiwars:hello", nonce, mode: "replay" }   (may repeat)
//   view  → parent { type: "aiwars:ready", nonce, replay: true }
//   parent → view  { type: "aiwars:frame", nonce, state, seq, i, n }
//
// Messages are pinned to the hello's source window + nonce, mirroring the seat
// bridge's discipline; "*" is safe because a recorded PUBLIC state is all that
// ever rides this channel. app.js consumes this via window.AIWARS_REPLAY:
// `active` gates its poller off, `onFrame(apply)` routes each pushed state
// through the same ingest path the poller used.
window.AIWARS_REPLAY = (function () {
  const active = new URLSearchParams(location.search).get("replay") === "bridge";
  let cb = null;
  let bridge = null; // { nonce, post } once the parent's hello lands
  if (active) {
    window.addEventListener("message", (e) => {
      const d = e.data;
      if (!d || typeof d !== "object" || typeof d.nonce !== "string") return;
      if (d.type === "aiwars:hello" && d.mode === "replay") {
        const src = e.source;
        if (!src) return;
        bridge = { nonce: d.nonce, post: (m) => src.postMessage(Object.assign({ nonce: d.nonce }, m), "*") };
        bridge.post({ type: "aiwars:ready", replay: true });
        return;
      }
      if (!bridge || d.nonce !== bridge.nonce) return;
      if (d.type === "aiwars:frame" && d.state && typeof d.state === "object" && cb) cb(d.state);
    });
  }
  return { active: active, onFrame: function (f) { cb = f; } };
})();
