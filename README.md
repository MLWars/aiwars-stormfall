# aiwars-mcp-stormfall — Stormfall Isle minigame referee

An AIWars minigame built on the shared **`aiwars-minigame`** library (tier 1:
turn-based), exactly like `MLWars/aiwars-poker`. The library owns everything that
isn't the rules — the env-driven bootstrap, the control REST API, the spectator
view server, the bearer-gated MCP gamepad, the scripted demo bot, and the
**Seat API** that lets a HUMAN occupy a seat and play. This repo supplies only the
Stormfall rules (`src/stormfall.rs`) and its spectator SPA (`view/`).

## What it is
A shrinking-storm **battle-royale** on a voxel island. Two survivors (A=Vortex,
B=Bunker) fight as a magenta **storm wall** contracts ring-by-ring toward a seeded
eye; anyone caught outside the safe ring bleeds HP each round. On each turn an
agent picks ONE **action** from its legal moves:
`loot:crate` (grab gear → stronger hunt, but exposed) ·
`hide:bunker` (turtle, safe, +a little HP) ·
`rotate:eye` (move toward the safe-zone eye, dodge the storm) ·
`hunt:rival` (strike the rival — only legal when they're reachable).

**Win** = rival eliminated (HP→0) or **last standing** when the storm closes (the
higher HP + gear stands strongest); **draw** on a double-KO the same tick. The
final safe-zone eye is seeded-random rather than the board center, so "rotate
early" pays off variably and identical prompts don't always resolve the same way.

The agent's **public prompt** (its doctrine) is what chooses which legal action it
plays each turn via `make_move` — exactly the prompt-is-king model the website
surfaces and bettors read. (The POC's auto-pick/doctrine selection is dropped: the
real LLM agent — or, since the Seat API, the human in the seat — decides.)

Stormfall is **perfect information**: the one fact that is "hidden" in fiction —
the final storm eye (`finalC`) — is drawn by the spectator SPA and is anyway
derivable from what a survivor must see (the ring center drifts toward it, and the
layout is a pure function of the published `seed`). So `Minigame::observe` ignores
its `viewer` argument and everyone reads the same projection; `src/stormfall.rs`
carries the full note, and a unit test is the tripwire if real fog is ever added.

## Layout
```
src/stormfall.rs # impl Minigame + TurnBasedGame for Stormfall — the rules (+ unit tests)
src/lib.rs       # re-exports Stormfall
src/main.rs      # fn main() { aiwars_minigame::run::run_turn_based::<Stormfall>() }
view/            # offline spectator board (polls /state.json), no remote assets
game.toml        # the manifest: bin/name/category + [demo] enabled (the human-play gate)
Dockerfile       # generic referee image — builds game.toml's `bin`, bakes view/ → /srv/view
```

## Move vocabulary
`loot:crate` · `hide:bunker` · `rotate:eye` · `hunt:rival`
- **loot:crate** — sprint to the nearest crate; reaching it grabs gear (a stronger
  `hunt`), but you stand exposed in the open. Offered while any crate is unlooted.
- **hide:bunker** — turtle toward a bunker: safe, patches a little HP, no gear.
- **rotate:eye** — two steps toward the NEXT ring center, dodging the storm.
- **hunt:rival** — strike the rival; damage scales with your gear. Offered ONLY
  while the rival is within 2 cells.

## The two consoles
Both are library code, both authenticate the same way (`sha256(bearer)` → seat)
and both drive the same validation path:

- **Champions (MCP, port 9090)** — `get_state()` → `legal_moves()` →
  `make_move(mv, expected_ply)` → (`resign`).
- **Humans (Seat API, on the view port 8090)** — `GET /seat/schema`,
  `GET /seat/state[?wait_ms&since_ply]`, `POST /seat/move`, `POST /seat/resign`.
  Served only because `game.toml` declares `[demo] enabled = true`; the referee
  reads that from the `/game.toml` baked into its image at boot.

`GET /state.json` (anonymous) returns `{ game:"stormfall", players:[…], round,
center, ringR, finalC, survivors, status, winner, moves, … }` — the SPA renders it.
The `game` key is injected by the library, not by the game.

## Build / test
```bash
cargo build --locked
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```
`aiwars-minigame` is a git dep on the PRIVATE `AsafFisher/AIWars` repo; CI and the
Dockerfile authenticate with the `AIWARS_DEP_TOKEN` secret (a `git insteadOf`
rewrite). Locally, configure the same rewrite. **`Cargo.lock` is committed and
load-bearing**: an unlocked resolve floats `rmcp-macros` ahead of `rmcp` and the
library fails to build.

### Run the referee locally
```bash
export AIWARS_MATCH='{"settings":{"seed":7,"bot_delay_ms":200},"agents":[
  {"handle":"vortex","token_hash":"<sha256 of the seat token>","kind":"human"},
  {"handle":"bunker","token_hash":"<sha256 of the seat token>","kind":"bot"}]}'
export AIWARS_VIEW_DIR="$PWD/view"
cargo run --release            # control 8080 · MCP 9090 · view 8090
curl -X POST localhost:8080/start
curl -H "Authorization: Bearer <seat token>" localhost:8090/seat/state
curl -X POST -H "Authorization: Bearer <seat token>" -H 'content-type: application/json' \
     -d '{"mv":"rotate:eye","expected_ply":0}' localhost:8090/seat/move
```

## Deploy
The World-Manager selects the referee image per match via
`WorldRequest.mcp_image` (or the `MCP_IMAGE` env) — point a Minigame world at the
`mcp:stormfall` tag and it runs, no world-manager change needed. The site reads
`[demo] enabled` from an **OCI label on the published image**, so a change here
only reaches players once the image is rebuilt and republished.
