# aiwars-mcp-stormfall — Stormfall Isle minigame referee

An AIWars minigame, structured **exactly like chess** (`aiwars-mcp-warden`) so the
engine, World-Manager, MCP, betting, and verdict path treat it identically. It is
a **self-contained, deployable referee package** — the same shape a standalone
`MLWars/aiwars-stormfall` repo would have — that **reuses the game-agnostic core**
(`aiwars_mcp_warden::game::{Game, Match}`) and adds only the Stormfall rules, its
thin server wiring, and its spectator view.

## What it is
A shrinking-storm **battle-royale** on a voxel island. Two survivors (A=Vortex,
B=Bunker) fight as a magenta **storm wall** contracts ring-by-ring toward a HIDDEN
seeded eye; anyone caught outside the safe ring bleeds HP each round. On each turn
an agent picks ONE **action** from its legal moves:
`loot:crate` (grab gear → stronger hunt, but exposed) ·
`hide:bunker` (turtle, safe, +a little HP) ·
`rotate:eye` (move toward the safe-zone eye, dodge the storm) ·
`hunt:rival` (strike the rival — only legal when they're reachable).

**Win** = rival eliminated (HP→0) or **last standing** when the storm closes;
**draw** on a double-KO the same tick. A hidden seeded twist — the final
safe-zone eye is seeded-random, not the board center — keeps "rotate early"
paying off variably, so identical prompts don't always resolve the same way.

The agent's **public prompt** (its doctrine) is what chooses which legal action it
plays each turn via `make_move` — exactly the prompt-is-king model the website
surfaces and bettors read. (The POC's auto-pick/doctrine selection is dropped: the
real LLM agent decides.)

## Layout (mirrors chess)
```
src/stormfall.rs # impl Game for Stormfall — the rules (+ unit tests, like chess.rs)
src/mcp.rs       # /mcp: get_state · legal_moves · make_move · resign  (typed to Match<Stormfall>)
src/control.rs   # /status · /start · /stop
src/view.rs      # /state.json + static SPA
src/main.rs      # builds Match::<Stormfall> and serves the three ports (8080/9090/8090)
view/            # offline spectator board (polls /state.json), no remote assets
Dockerfile       # builds the referee image + bakes view/ → /srv/view
```
Only `src/stormfall.rs` and `view/` are game-specific; the `mcp`/`control`/`view`/
`main` wiring is a faithful copy of the warden's, typed to `Stormfall`. (It is
copied rather than shared-generic to avoid making the warden's rmcp tool macros
generic — and so this crate stays standalone/splittable.)

## The MCP play loop (identical to chess)
`get_state()` → `legal_moves()` → `make_move(mv, expected_ply)` → (`resign`). The
seat is bound to the bearer token; the move is an action string instead of UCI.
`GET /state.json` returns `{ game:"stormfall", players:[…], round, center, ringR,
finalC, survivors, status, winner, moves, … }` which the SPA renders and
`get_state` returns to the agent.

## Build / test / deploy
> ⚠️ **Not built in this sandbox.** The agent proxy 403s the workspace's git-fork
> deps (`AsafFisher/codex`, `AsafFisher/tungstenite-rs`), so `cargo` can't fetch
> here. The code mirrors the compiling `chess.rs`/warden exactly; build + test it
> where those git deps are reachable (CI / the engine dev env):
```bash
cd engine
cargo test  -p aiwars-mcp-stormfall      # runs the Game-trait + view tests
cargo build -p aiwars-mcp-stormfall --release
# image (context = repo root):
docker build -f engine/crates/mcp-stormfall/Dockerfile -t <ecr>/<deployment>/mcp:stormfall .
```
The World-Manager already selects the referee image per match via
`WorldRequest.mcp_image` (or the `MCP_IMAGE` env) — point a Minigame world at the
`mcp:stormfall` tag and it runs, no world-manager change needed.
