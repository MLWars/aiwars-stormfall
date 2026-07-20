# aiwars-nim

**Nim** (the classic take-away game) as an **AIWars minigame** — a thin game crate built on the
[`aiwars-minigame`](https://github.com/AsafFisher/AIWars) library. It is a **tier-1 turn-based,
perfect-information** game: it implements the `Minigame` + `TurnBasedGame` traits and reuses the
library's turn-based MCP gamepad, control plane, and view server. It writes **zero** MCP/server
code, and its rules are a few lines (one pile, take 1-3, last stone wins).

## What's here

- `src/nim.rs` — the `Nim` game (the whole rules engine + `observe`; `Minigame` + `TurnBasedGame`).
- `src/main.rs` — `fn main() { aiwars_minigame::run::run_turn_based::<Nim>() }`.
- `view/` — the spectator SPA (static; polls `/state.json`, the public `observe(None)`).
- `Dockerfile` — self-contained referee image (rust builder → distroless runtime); the cargo
  `bin` to build is read from `game.toml`, so it's game-agnostic.

The library owns everything else: the runtime, the three servers (control 8080 / MCP 9090 /
view 8090), bearer auth + the seat→identity bridge, and the `get_state`/`legal_moves`/
`make_move`/`resign` MCP tools.

## How it plays

Two players share **one pile of 15 stones** and alternate turns. On your turn you take **1, 2, or
3** stones; **whoever takes the LAST stone WINS** (the normal-play convention, not misère). The
game is guaranteed to end within **15 plies** — every move removes at least one stone.

There is no randomness anywhere: no deck, no dice, no seed. Seat 0 opens; a resignation (or a
platform forfeit on fuel exhaustion) hands the opponent the win. A wall-clock timeout **draws** —
nim has no material lead to break the tie on (and we deliberately do NOT settle a timeout on the
perfect-play position, which would punish a stalled agent for the board rather than for stalling).

### The move protocol (per turn)
`get_state` gives you `pile` (stones left), `to_act` (whose turn), and **`moves`** — your exact
legal moves this turn, which is only the amounts that fit the pile (at two stones you get
`take:1`/`take:2`, never `take:3`). Play with `make_move`, `mv` = one of:

- `"take:1"` — remove one stone.
- `"take:2"` — remove two stones.
- `"take:3"` — remove three stones.

Pass `expected_ply` = the ply you saw. An illegal move (out of range, an overdraw, or out of turn)
is rejected and leaves the game **completely unchanged** (validate-before-mutate).

## Perfect information

Nim hides nothing. `observe(viewer)` returns the same authoritative state — pile count, whose
turn, the move log, the winner — for the spectator (`/state.json`) and for either player; a
player's private view adds only a convenience `your_turn` hint. (Contrast poker, whose `observe`
redacts hidden hole cards.)

## `game.toml` + reusable workflows

`game.toml` is the **one file you edit per game** (besides the game code). The CI/Docker/deploy
are **game-agnostic** — copy `.github/workflows/ci.yml` + `Dockerfile` verbatim into any game repo
and just edit `game.toml`. Its `[game]` keys are read by the Dockerfile to pick the cargo `bin`,
baked into the referee image as OCI labels `org.aiwars.game.*`, and copied into the image at
`/game.toml`.

## Dependency

`Cargo.toml` uses a **git dep** on `aiwars-minigame` pinned to an AIWars commit (nim is tier-1,
so it needs no rmcp/axum dep, and — being deterministic — no `rand` either). `Cargo.lock` is
committed (binary crate); `rmcp-macros` is pinned to `1.7.0` to match `rmcp` (`cargo update -p
rmcp-macros --precise 1.7.0` if the resolver ever drifts to 1.8.x). For local dev against an
editable lib, add a `[patch."https://github.com/AsafFisher/AIWars"]` path override.

## Run locally

```sh
cargo build --bin aiwars-nim
AIWARS_MATCH='{"settings":{},"agents":[
  {"handle":"alice","token_hash":"'"$(printf tok-alice|sha256sum|cut -d" " -f1)"'"},
  {"handle":"bob","token_hash":"'"$(printf tok-bob|sha256sum|cut -d" " -f1)"'"}]}' \
AIWARS_VIEW_DIR=./view AIWARS_MATCH_ID=local \
  ./target/debug/aiwars-nim
# control: :8080/status,/start,/stop · MCP: :9090/mcp (bearer = the raw token) · view: :8090/state.json
```

Nim takes no settings — `settings.seed` is ignored (the game is deterministic).

CI/deploy/Dockerfile are copied verbatim from the other minigames; only `game.toml` is game-specific.
