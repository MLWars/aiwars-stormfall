# AIWars POC kit — builder API

Each game is a **standalone HTML page** that includes the shared kit and calls
`AW.mount(root, GAME)`. The kit draws all the chrome (title, the champions'
prompts, the 4 MCP tools, a live tool-call feed, live odds, playback controls)
and runs the deterministic beat playback. **You write two things:** a
`build(seed, opts)` engine and a `draw(ctx, view)` renderer. Study the worked
reference: `pocs/games/getaway/game.js` (engine + full canvas scene).

## File layout for a new game `foo`
```
pocs/games/foo/index.html   # copy getaway's, swap the title + window.FOO
pocs/games/foo/game.js      # window.FOO = { ...config, build, draw }
```
`index.html` includes `../../kit/style.css`, `../../kit/kit.js`, `game.js`, then
`AW.mount(document.getElementById("root"), window.FOO)`.

## The GAME config object
```js
window.FOO = {
  id, name, W, H,                 // canvas pixel size (≈ 780×560 like Getaway)
  tag,                            // one-line tagline under the title
  champions: [{id:"A",name:"…",color:"#10b981"}, {id:"B",name:"…",color:"#8b5cf6"}],
  prompts: { A: "default prompt A", B: "default prompt B" },
  mcp: {
    kickoff: "referee→agent system prompt framing the game + tools",
    tools: [ {name, args, ret, desc}, … ],   // get_state, legal_moves, make_move, resign
    vocab: "move:thing · other:thing",        // the legal_moves vocabulary
  },
  build, draw,
}
```

## build(seed, opts) → result
Deterministic. `opts.prompts = {A,B}`. Must return:
```js
{
  seed, winner,                    // winner: "A" | "B" | null (draw)
  beats: [ beat, … ],
  promptOf: (id) => htmlString,    // the prompt with doctrine keywords <b>bolded</b>
  tagOf:   (id) => "doctrine tag", // e.g. "alley ghost"
  oddsAt:  (b) => ({A:pct,B:pct}), // live implied odds 0..100 at beat index b
  names: {A,B},
}
```
Each **beat** (one agent's turn, faithful to get_state→legal_moves→make_move):
```js
{
  ply, agent: "A"|"B"|"ref",
  thought: "short reasoning shown in the feed",
  observe: {…} | "string",         // get_state() return (kept short)
  legal: ["move:x","move:y", …],   // legal_moves() return
  move: "move:x",                  // the make_move() the prompt chose
  ok: true, result: "ok · …",      // make_move() return summary (ok:false on a fail)
  events: ["human journal line"],  // shown in dispatcher / narration
  state: { …whatever draw needs… },// snapshot to render this beat
}
```
The LAST beat should be `agent:"ref"`, `move:"resolve"` with the final result line.

**Prompt-is-king is mandatory:** parse each prompt for doctrine keywords (see
`doctrine()`/`KW` in getaway) and let that choose which legal move the agent
takes. Add a hidden seeded twist so two identical prompts don't always resolve
the same way (keeps odds live). Bold the matched keywords in `promptOf`.

## draw(ctx, view) → void
`view = { now, t, result, beats, beat, rawBeat, beatT (0..1 within the beat),
over, playing, champs, AW }`. Interpolate positions with `view.beatT`
(`AW.easeOut`) for smooth motion; freeze on `over` and show a finish overlay.

## Drawing toolkit (all on `AW`, ctx first)
`AW.tile(c,x,y,w,h,col)` · `AW.rrect(c,x,y,w,h,r)` (then fill/stroke) ·
`AW.box(c,x,y,w,h,depth,front,top,side)` iso voxel box ·
`AW.label(c,x,y,text,px,col,align?,font?)` · `AW.wrap(c,text,x,y,maxW,lh,px,col,font?)` ·
`AW.sprite(c,{x,y,shirt,cap,skin,name,tag,moving,t,nameCol})` chibi champion ·
`AW.shadow(c,x,y,rx,ry,alpha)` · `AW.glow(c,x,y,r,"rgba(r,g,b,a)")` radial ·
`AW.nightSky(c,w,ground,t,[top,mid,hor])` stars+gradient.
Helpers: `AW.rng(seed)`→()=>0..1 · `AW.hashSeed(str)` · `AW.lerp,clamp,ease,easeOut` ·
`AW.pick(arr,n,rng)` · `AW.reduced` (prefers-reduced-motion — disable ambient motion).
Palette tokens: A `#10b981`/soft `#5eead4`, B `#8b5cf6`/soft `#c4b5fd`,
brand `#22d3ee`, win `#34d399`, danger `#fb5d5d`. **No remote assets** — draw
everything procedurally. Respect `AW.reduced`.

## Screenshot loop (how to verify + iterate)
URL params freeze a frame: `?seed=7&beat=5&t=0.6&play=0`. Shoot with:
```
cd <scratchpad>/shots
CHROME=$(ls /opt/pw-browsers/chromium-*/chrome-linux/chrome | head -1)
CHROME_BIN="$CHROME" node shotgame.mjs <abs path to your index.html> foo 1240 900 \
  "seed=7&beat=4&t=0.55&play=0" "seed=7&beat=99&t=0.85&play=0"
```
It prints `--- PAGE ERRORS ---` (and exits 2) if your JS throws — fix those
first. Then **Read the PNGs** and iterate until the scene is genuinely
impressive (dense, animated, legible; the prompt's effect visible; a dramatic
finish). `beat=99` clamps to the final beat → the finish overlay.
