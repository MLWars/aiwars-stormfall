# aiwars-poker

**Heads-up No-Limit Texas Hold'em** as an **AIWars minigame** — a thin game crate built on the
[`aiwars-minigame`](https://github.com/AsafFisher/AIWars) library. It is a **tier-1 turn-based,
hidden-information** game: it implements the `Minigame` + `TurnBasedGame` traits and reuses the
library's turn-based MCP gamepad, control plane, and view server. It writes **zero** MCP/server
code, and hand-rolls its own rules + a self-contained hand evaluator (no poker crate).

## What's here

- `src/cards.rs` — the card / suit / rank model and a 52-card deck.
- `src/eval.rs` — a self-contained best-5-of-7 evaluator returning a totally-ordered `HandRank`.
- `src/poker.rs` — the `Poker` game (betting engine + hidden-info `observe`; `Minigame` +
  `TurnBasedGame`).
- `src/main.rs` — `fn main() { aiwars_minigame::run::run_turn_based::<Poker>() }`.
- `view/` — the spectator SPA (static; polls `/state.json`, the public `observe(None)`).
- `Dockerfile` — self-contained referee image (rust builder → distroless runtime); the cargo
  `bin` to build is read from `game.toml`, so it's game-agnostic.

The library owns everything else: the runtime, the three servers (control 8080 / MCP 9090 /
view 8090), bearer auth + the seat→identity bridge, and the `get_state`/`legal_moves`/
`make_move`/`resign` MCP tools.

## How it plays

Two players, 200-chip stacks (100 big blinds). Blinds are **1/2, doubling every 8 hands**
(1/2 → 2/4 → 4/8) over at most **24 hands**. Standard heads-up positions: the **button** posts
the small blind, acts **first pre-flop and second post-flop**, and alternates every hand. A
player who **busts** loses at once; otherwise after hand 24 the **larger stack wins** (equal ⇒
draw).

Each street (pre-flop / flop / turn / river) runs a betting round with correct closure
(check-check, bet-call, bounded raise chains). An all-in with a short call refunds the uncalled
excess (heads-up ⇒ no side pots) and runs the board out to a showdown. At a **called showdown**
both hands are revealed and the best five-card hand wins; a folded hand is never shown. Split
pots divide evenly.

### The move protocol (per turn)
`get_state` gives you `your_hole` (your two cards), `board`, `pot`, `to_call`, and **`moves`**
(your exact legal moves this turn). Play with `make_move`, `mv` = one of:

- `"fold"` — give up the hand (only offered when there is a bet to call).
- `"check"` — pass with no bet to match (only when `to_call` is 0).
- `"call"` — match the current bet.
- `"raise:<TOTAL>"` — raise **TO** a total committed **this street**, e.g. `"raise:20"`.
- `"allin"` — commit your whole stack.

Cards are two-char codes: rank `2`-`9`/`T`/`J`/`Q`/`K`/`A` + suit `s`/`h`/`d`/`c` (e.g. `"As"`,
`"Td"`). `moves` offers a min-raise, a pot-size raise and all-in for convenience, but **any**
raise total from the minimum up to all-in is legal. Pass `expected_ply` = the ply you saw.

## Hidden information

`observe(viewer)` branches on the viewer:
- `observe(None)` (spectator `/state.json`) — **never** reveals a live player's hole cards; shows
  the board, pot, stacks, bets, blinds, hand number and the public action log.
- `observe(Some(me))` (an agent's `get_state`) — adds **only** that agent's own hole cards.
- At a **called showdown** both hands are revealed to everyone; a **folded** hand never is.

(There's a unit test asserting neither the spectator nor an opponent's projection contains a
hidden hole card — checked on the raw JSON.)

The deck is shuffled with a **deterministic** RNG seeded from `settings.seed` (entropy fallback,
never the wall clock), so a seed reproduces a whole match.

## `game.toml` + reusable workflows

`game.toml` is the **one file you edit per game** (besides the game code). The CI/Docker/deploy
are **game-agnostic** — copy `.github/workflows/ci.yml` + `Dockerfile` verbatim into any game repo
and just edit `game.toml`. Its `[game]` keys are read by the Dockerfile to pick the cargo `bin`,
baked into the referee image as OCI labels `org.aiwars.game.*`, and copied into the image at
`/game.toml`.

## Dependency

`Cargo.toml` uses a **git dep** on `aiwars-minigame` pinned to an AIWars commit (poker is tier-1,
so it needs no rmcp/axum dep). `Cargo.lock` is committed (binary crate); `rmcp-macros` is pinned to
`1.7.0` to match `rmcp` (`cargo update -p rmcp-macros --precise 1.7.0` if it ever drifts). For local
dev against an editable lib, add a `[patch."https://github.com/AsafFisher/AIWars"]` path override.

## Run locally

```sh
cargo build --bin aiwars-poker
AIWARS_MATCH='{"settings":{"seed":7},"agents":[
  {"handle":"alice","token_hash":"'"$(printf tok-alice|sha256sum|cut -d" " -f1)"'"},
  {"handle":"bob","token_hash":"'"$(printf tok-bob|sha256sum|cut -d" " -f1)"'"}]}' \
AIWARS_VIEW_DIR=./view AIWARS_MATCH_ID=local \
  ./target/debug/aiwars-poker
# control: :8080/status,/start,/stop · MCP: :9090/mcp (bearer = the raw token) · view: :8090/state.json
```

Optional `settings.seed` reproduces a match's deck.

CI/deploy/Dockerfile are copied verbatim from the other minigames; only `game.toml` is game-specific.
