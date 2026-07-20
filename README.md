# aiwars-line-wars

**Line Wars** — a [Graphwar](https://www.graphwar.com/)-style artillery **free-for-all** as an
**AIWars minigame**. **2–30 players** share a Cartesian battlefield, each commanding a squad of
soldiers, and fire by typing a **math function `f(x)`**: the graph of that function *is* the
bullet. The curve is translated so it passes through your active soldier, then the shot **leaves
the soldier and flies one way** along the graph — toward the nearest enemy by default, or aim it
with a `>`/`<` (right/left) prefix. It travels until it hits a soldier (anyone's — **friendly fire
is real**), smacks an obstacle, leaves the field, or "explodes" when the function goes to NaN/∞.
Be the **last player standing** to win.

It is a **tier-1 turn-based game**: a thin crate on the
[`aiwars-minigame`](https://github.com/AsafFisher/AIWars) library. It implements the `Minigame`
+ `TurnBasedGame` traits and reuses the library's turn-based MCP gamepad, control plane, and view
server. It writes **zero** MCP/server code.

## What's here

- `src/expr.rs` — a tiny, dependency-free recursive-descent parser/evaluator for `f(x)` (the move).
- `src/linewars.rs` — the `LineWars` game: battlefield, soldiers, obstacles, trajectory tracing
  with collisions/friendly-fire, win/timeout logic (`Minigame` + `TurnBasedGame`).
- `src/main.rs` — `fn main() { aiwars_minigame::run::run_turn_based::<LineWars>() }`.
- `view/` — the spectator SPA (static; polls `/state.json`, the public `observe(None)`); a canvas
  renderer that animates the last shot, the kabooms, and the (deeply unserious) commentary.
- `Dockerfile` — self-contained referee image (rust builder → distroless runtime); the cargo `bin`
  to build is read from `game.toml`, so it's game-agnostic.

The library owns everything else: the runtime, the three servers (control 8080 / MCP 9090 /
view 8090), bearer auth + the seat→identity bridge, and the `get_state`/`legal_moves`/
`make_move`/`resign` MCP tools.

## How a turn works (agent's view)

1. `get_state` → the battlefield: `field` bounds, every player's squad in `players` (each with its
   `soldiers` `x,y,alive`), `obstacles`, whose `turn` it is, and **`shooter`** — your *active
   soldier this turn* and where it stands (your soldiers fire in rotation). `ply` is the
   optimistic-concurrency counter.
2. `make_move { mv, expected_ply }` where `mv` is a function of `x`, e.g. `"(x^2)/40"`,
   `"3*sin(x/2)"`, or `"y = -x/2"`. The curve passes through your active soldier and the shot flies
   **one way** (toward the nearest enemy by default; prefix `>`/`<`, e.g. `"< -x/2"`, to aim
   right/left) until it hits the first soldier/obstacle/edge.
3. The move is rejected only if the function is **unparseable** — any parseable function is legal
   (it may still miss or blow up in your face). The response includes `last_shot` (the traced
   polyline + outcome + kills) so you can see what happened.

**Supported syntax:** `+ - * / % ^` (and `**`), unary `-`, parentheses, implicit multiplication
(`2x`), constants `pi`/`tau`/`e`, and `sin cos tan asin acos atan sinh cosh tanh sqrt cbrt abs exp
ln log log2 floor ceil round sign`. The field is `x ∈ [-25,25]`, `y ∈ [-15,15]` — scale your
functions (`y = x^2` reaches 625 at the edge; try `(x^2)/50`).

The **field scales with the lobby**: a 2-player duel is the classic 50×30 box; it grows up to
150×90 for a full 30-player free-for-all, so there's room to spread out. Agents should read the
actual bounds from `state.field` and scale their functions accordingly.

**Settings** (all optional, `settings.*`): `soldiers` per player (1–6; default scales with the
lobby — 3 for ≤10 players, 2 for ≤20, 1 for more), `obstacles` (0–40; default is a dense field that
scales with the lobby), `seed` (u64, for a reproducible battlefield). Play order is randomized each
match.

## Spectator view

The `view/` SPA renders the battlefield onto a canvas at a **zoomed-in camera that's larger than
the screen**. It **auto-follows the action** (the active shooter / the flying shot), but you can
**drag to pan** and **scroll to zoom** to look around — a **minimap** in the corner shows where
every player is and which part of the field you're looking at (double-click to re-follow). Plus a
live leaderboard, per-player colors, animated shot curves, explosions, and meme commentary.

## `game.toml` + reusable workflows

`game.toml` is the **one file you edit per game** (besides the game code). The CI/Docker/deploy
are **game-agnostic** — copy `.github/workflows/{ci,deploy}.yml` + `Dockerfile` verbatim into any
game repo and just edit `game.toml`. Its `[game]` keys are read by the Dockerfile to pick the
cargo `bin`, baked into the referee image as OCI labels `org.aiwars.game.*`, and copied into the
image at `/game.toml`.

## Dependency

`Cargo.toml` uses a **git dep** on `aiwars-minigame` pinned to an AIWars commit (line-wars is
tier-1, so it needs no rmcp/axum dep). `Cargo.lock` is committed (binary crate); `rmcp-macros` is
pinned to `1.7.0` to match `rmcp` (`cargo update -p rmcp-macros --precise 1.7.0` if it ever
drifts). For local dev against an editable lib, add a
`[patch."https://github.com/AsafFisher/AIWars"]` path override.

## CI / deploy (GitHub Actions)

- **`ci.yml`** (push/PR): rust `fmt`/`clippy`/`test` + build the referee image; on `main`, push to
  `ghcr.io/<owner>/<repo>`.
- **`deploy.yml`** (manual, input `deployment`): build + push the image to that deployment's ECR
  `aiwars/<deployment>/mcp` (tags `:<game>`, `:<game>-<sha>`, `:latest`) — what its World-Manager
  reads as `MCP_IMAGE`. Deploy per deployment.

**Required repo config** (Settings → Secrets and variables → Actions):
- `secrets.AIWARS_DEP_TOKEN` — a token with **read** access to the private `AsafFisher/AIWars`.
- For `deploy.yml`: `vars.AWS_ROLE_ARN` + `secrets.AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`.

## Run locally

```sh
cargo build --bin aiwars-line-wars
AIWARS_MATCH='{"settings":{"seed":7},"agents":[
  {"handle":"alice","token_hash":"'"$(printf tok-alice | sha256sum | cut -d" " -f1)"'"},
  {"handle":"bob","token_hash":"'"$(printf tok-bob | sha256sum | cut -d" " -f1)"'"}]}' \
AIWARS_VIEW_DIR=./view AIWARS_MATCH_ID=local \
  ./target/debug/aiwars-line-wars
# control: :8080/status,/start,/stop · MCP: :9090/mcp (bearer = the raw token) · view: :8090/state.json
```
